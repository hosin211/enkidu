//! The terminal on screen. [`TerminalView`] takes keys, the mouse and the wheel and turns them into bytes for the
//! program (`enkidu_term`); [`TerminalGrid`] draws the terminal's cells from a [`Screen`] copy taken once a frame.
//!
//! Nothing here waits on the program: output is parsed on the terminal's own thread, typing goes through its
//! writer queue, and the parser only wakes this view when there is something new to draw.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use enkidu_term::{
    Button, Cell as ScreenCell, CursorKind, Damage, GridSize, KeyAction, Link, Mods, MouseAction, Rgb, Screen,
    SelectKind, Spawn, Terminal, WheelDelta, dropped_paths,
};
use futures::StreamExt as _;
use futures::channel::mpsc::UnboundedReceiver;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Bounds, ClipboardItem, Context, DispatchPhase, Element, ElementId, ElementInputHandler, EntityInputHandler,
    ExternalPaths, FocusHandle, Focusable, Font, FontFeatures, FontStyle, FontWeight, GlobalElementId, Hsla,
    InspectorElementId, KeyDownEvent, KeyUpEvent, LayoutId, LineLayout, Modifiers, ModifiersChangedEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, Render, ScrollDelta, ScrollEvent,
    ShapedLine, StrikethroughStyle, Style, Subscription, Task, TextAlign, TextRun, UTF16Selection, UnderlineStyle,
    Window, div, fill, outline, point, px, relative, size,
};

use crate::keymap;
use enkidu_ui::tokens::{self as theme, hsla};

/// The terminal's name for key bindings. A key the app binds outside must not be taken from the program in here:
/// `lib.rs` unbinds what gpui-kit's root binds for everything (Tab, Shift-Tab), and a test keeps the list empty.
pub const KEYS: &str = "Terminal";

/// How often the history moves while a selection is dragged past the terminal's edge.
const EDGE_TICK: std::time::Duration = std::time::Duration::from_millis(50);

/// How long a blinking cursor stays on, then off (xterm.js's pace).
const BLINK: std::time::Duration = std::time::Duration::from_millis(600);

/// A view of a new terminal running `spawn`, woken whenever it has output to draw.
pub fn spawn_view(spawn: Spawn, cx: &mut App) -> anyhow::Result<gpui_kit::Entity<TerminalView>> {
    let (tx, rx) = futures::channel::mpsc::unbounded::<()>();
    let wake: enkidu_term::Wake = Arc::new(move || {
        let _ = tx.unbounded_send(());
    });
    let terminal = Terminal::spawn(spawn, GridSize::new(120, 36), enkidu_term::Palette::enkidu(), wake)?;
    Ok(cx.new(|cx| TerminalView::new(Arc::new(terminal), Some(rx), cx)))
}

/// Where the grid was drawn, so the mouse can find its cell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridLayout {
    pub origin: Point<Pixels>,
    pub cell_width: Pixels,
    pub line_height: Pixels,
    pub cols: usize,
    pub rows: usize,
}

impl GridLayout {
    /// Whether a point is on the grid.
    pub fn contains(&self, position: Point<Pixels>) -> bool {
        let (x, y) = (position.x - self.origin.x, position.y - self.origin.y);
        x >= px(0.) && y >= px(0.) && x < self.cell_width * self.cols as f32 && y < self.line_height * self.rows as f32
    }

    /// The cell under a point, and whether the point is on the cell's right half.
    pub fn cell_at(&self, position: Point<Pixels>) -> (usize, usize, bool) {
        let x = ((position.x - self.origin.x) / self.cell_width).max(0.0);
        let y = ((position.y - self.origin.y) / self.line_height).max(0.0);
        let col = (x.floor() as usize).min(self.cols.saturating_sub(1));
        let row = (y.floor() as usize).min(self.rows.saturating_sub(1));
        (row, col, x.fract() >= 0.5)
    }
}

pub struct TerminalView {
    terminal: Arc<Terminal>,
    focus: FocusHandle,
    layout: Rc<Cell<Option<GridLayout>>>,
    /// The last copy of the screen, brought up to date each frame with the rows that changed.
    screen: Rc<RefCell<Screen>>,
    /// The rows drawn in the last two frames, so an unchanged row isn't shaped again.
    rows: Rc<RefCell<RowCache>>,
    selecting: bool,
    /// While a selection is dragged past the top or bottom edge: the lines the history moves each tick (toward the
    /// history when positive), the column the pointer is in, and the tick itself.
    edge_scroll: i32,
    edge_col: (usize, bool),
    edge_tick: Option<Task<()>>,
    /// An input method's composition, not yet typed: shown at the cursor, underlined (Japanese, Chinese, Korean,
    /// accents held down, voice).
    marked: Option<String>,
    /// Where the input method's own caret is in the composition, in UTF-16 units.
    marked_caret: Option<Range<usize>>,
    /// The blinking cursor is drawn now (it is hidden every other 600 ms while the terminal has the keyboard).
    cursor_on: bool,
    /// Turns the cursor on and off while the terminal has the keyboard.
    blink: Option<Task<()>>,
    /// The link under the pointer while ⌘ is held: underlined, and opened by a click.
    hovered_link: Option<Link>,
    /// What the program last heard about the keyboard (`CSI I`, `CSI O`), and who tells it.
    reported_focus: bool,
    focus_watch: Vec<Subscription>,
    _wake: Option<Task<()>>,
}

impl TerminalView {
    /// A view of `terminal`. `wake` is the receiving end of the terminal's wake callback: each message asks for a
    /// frame.
    pub fn new(terminal: Arc<Terminal>, wake: Option<UnboundedReceiver<()>>, cx: &mut Context<Self>) -> Self {
        let task = wake.map(|mut wake| {
            cx.spawn(async move |this, cx| {
                while wake.next().await.is_some() {
                    if this.update(cx, |_, cx| cx.notify()).is_err() {
                        break;
                    }
                }
            })
        });
        TerminalView {
            terminal,
            focus: cx.focus_handle(),
            layout: Rc::new(Cell::new(None)),
            screen: Rc::new(RefCell::new(Screen::default())),
            rows: Rc::new(RefCell::new(RowCache::default())),
            selecting: false,
            edge_scroll: 0,
            edge_col: (0, false),
            edge_tick: None,
            marked: None,
            marked_caret: None,
            cursor_on: true,
            blink: None,
            hovered_link: None,
            reported_focus: false,
            focus_watch: Vec::new(),
            _wake: task,
        }
    }

    pub fn terminal(&self) -> &Arc<Terminal> {
        &self.terminal
    }

    /// Where the grid was last drawn.
    pub fn layout(&self) -> Option<GridLayout> {
        self.layout.get()
    }

    /// How many rows the last frame shaped (the others came from the rows drawn before).
    pub fn rows_shaped(&self) -> usize {
        self.rows.borrow().shaped
    }

    /// The input method's composition shown at the cursor, if one is under way.
    pub fn marked_text(&self) -> Option<&str> {
        self.marked.as_deref()
    }

    /// The link ⌘ and a click would open now.
    pub fn hovered_link(&self) -> Option<&Link> {
        self.hovered_link.as_ref()
    }

    /// Underlines the link under `position` while ⌘ is held, and forgets it otherwise.
    fn hover_link(&mut self, position: Point<Pixels>, command: bool, cx: &mut Context<Self>) {
        let link = self.layout.get().filter(|layout| command && layout.contains(position)).and_then(|layout| {
            let (row, col, _) = layout.cell_at(position);
            self.terminal.link_at(row, col)
        });
        if link != self.hovered_link {
            self.hovered_link = link;
            cx.notify();
        }
    }

    fn modifiers_changed(&mut self, event: &ModifiersChangedEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.hover_link(window.mouse_position(), event.modifiers.platform, cx);
    }

    /// Tells the program whether it has the keyboard (the view focused in the active window), when that changed, and
    /// blinks the cursor while it does.
    fn report_focus(&mut self, window: &Window, cx: &mut Context<Self>) {
        let focused = self.focus.is_focused(window) && window.is_window_active();
        if focused != self.reported_focus {
            self.reported_focus = focused;
            self.terminal.focus(focused);
        }
        if focused && self.blink.is_none() {
            self.restart_blink(cx);
        } else if !focused && self.blink.is_some() {
            self.blink = None;
            self.cursor_on = true;
            cx.notify();
        }
    }

    /// Shows the cursor and starts its blinking over, as typing does: it never vanishes under your fingers.
    fn restart_blink(&mut self, cx: &mut Context<Self>) {
        self.cursor_on = true;
        self.blink = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(BLINK).await;
                let blinked = this.update(cx, |this, cx| {
                    this.cursor_on = !this.cursor_on;
                    // a frame only when a blinking cursor is on screen
                    if this.screen.borrow().cursor.is_some_and(|c| c.blinking) {
                        cx.notify();
                    }
                });
                if blinked.is_err() {
                    break;
                }
            }
        }));
    }

    /// Whether the cursor is drawn now (a steady cursor always is).
    pub fn cursor_shown(&self) -> bool {
        self.cursor_on || !self.screen.borrow().cursor.is_some_and(|c| c.blinking)
    }

    /// Hears the keyboard come and go, once the view is in a window.
    fn watch_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus_watch.is_empty() {
            return;
        }
        // where things stand now, unsaid: a program hears changes from here on
        self.reported_focus = self.focus.is_focused(window) && window.is_window_active();
        self.focus_watch = vec![
            cx.on_focus_in(&self.focus, window, |this, window, cx| this.report_focus(window, cx)),
            cx.on_focus_out(&self.focus, window, |this, _, window, cx| this.report_focus(window, cx)),
            cx.observe_window_activation(window, |this, window, cx| this.report_focus(window, cx)),
        ];
    }

    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        if keystroke.modifiers.platform {
            // Cmd belongs to the app: ⌘C and ⌘V come here as the Copy and Paste actions (the Edit menu's too).
            return;
        }
        let Some((key, mods)) = keymap::terminal_key(keystroke) else { return };
        let action = if event.is_held { KeyAction::Repeat } else { KeyAction::Press };
        if self.terminal.key_action(&key, mods, action) {
            if self.blink.is_some() {
                self.restart_blink(cx);
            }
            cx.stop_propagation();
        }
    }

    /// A key coming up: only a program that asked for the kitty protocol's event types hears it.
    fn key_up(&mut self, event: &KeyUpEvent, _: &mut Window, _: &mut Context<Self>) {
        if event.keystroke.modifiers.platform {
            return;
        }
        if let Some((key, mods)) = keymap::terminal_key(&event.keystroke) {
            self.terminal.key_action(&key, mods, KeyAction::Release);
        }
    }

    /// ⌘C or Edit > Copy: the selection to the clipboard. With nothing selected the action goes on to the app.
    fn copy(&mut self, _: &crate::Copy, _: &mut Window, cx: &mut Context<Self>) {
        match self.terminal.selection_text() {
            Some(text) => cx.write_to_clipboard(ClipboardItem::new_string(text)),
            None => cx.propagate(),
        }
    }

    /// ⌘V or Edit > Paste: the clipboard's text goes to the program as one block (bracketed when it asked).
    fn paste(&mut self, _: &crate::Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.terminal.paste(&text);
        }
    }

    /// Files dropped from Finder: their paths go in as one paste, escaped as a shell reads them, and the terminal
    /// takes the keyboard.
    fn dropped(&mut self, paths: &ExternalPaths, window: &mut Window, cx: &mut Context<Self>) {
        self.terminal.paste(&dropped_paths(paths.paths()));
        self.focus.focus(window, cx);
    }

    /// The trackpad's pixels or the mouse wheel's lines: the terminal turns them into lines (`enkidu_term::wheel`).
    fn scroll(&mut self, event: &ScrollEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(layout) = self.layout.get() else { return };
        let delta = match event.delta {
            ScrollDelta::Pixels(pixels) => WheelDelta::Pixels(pixels.y / px(1.0)),
            ScrollDelta::Lines(lines) => WheelDelta::Lines(lines.y),
        };
        let (row, col, _) = layout.cell_at(event.position);
        let line_height = layout.line_height / px(1.0);
        if self.terminal.scroll_wheel(delta, line_height, row, col, mods(&event.modifiers)) != 0 {
            cx.notify();
        }
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
        let Some(layout) = self.layout.get() else { return };
        let Some(button) = button(event.button) else { return };
        let (row, col, right_half) = layout.cell_at(event.position);
        if event.modifiers.platform
            && button == Button::Left
            && let Some(link) = self.terminal.link_at(row, col)
        {
            // ⌘ and a click on a link opens it, whoever has the mouse
            cx.open_url(&link.url);
            return;
        }
        if self.terminal.mouse(MouseAction::Press(button), row, col, mods(&event.modifiers)) {
            // the program takes the mouse
            return;
        }
        if button != Button::Left {
            return;
        }
        let kind = match event.click_count {
            2 => SelectKind::Word,
            n if n >= 3 => SelectKind::Line,
            _ => SelectKind::Simple,
        };
        self.terminal.select_start(row, col, right_half, kind);
        self.selecting = true;
        cx.notify();
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.hover_link(event.position, event.modifiers.platform, cx);
        let Some(layout) = self.layout.get() else { return };
        let (row, col, _) = layout.cell_at(event.position);
        if self.selecting {
            if event.pressed_button == Some(MouseButton::Left) {
                self.drag_selection(event.position, cx);
            }
            return;
        }
        let held = event.pressed_button.and_then(button);
        self.terminal.mouse(MouseAction::Move(held), row, col, mods(&event.modifiers));
    }

    fn mouse_up(&mut self, event: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selecting {
            if let (Some(layout), Some(button)) = (self.layout.get(), button(event.button)) {
                let (row, col, _) = layout.cell_at(event.position);
                self.terminal.mouse(MouseAction::Release(button), row, col, mods(&event.modifiers));
            }
            return;
        }
        self.end_selection(cx);
    }

    /// The pointer moved while a selection is dragged, maybe outside the terminal: the selection goes to the cell
    /// nearest to it. Past the top or bottom edge the history scrolls and the selection goes on with it, faster the
    /// farther out the pointer is.
    fn drag_selection(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(layout) = self.layout.get() else { return };
        let (row, col, right_half) = layout.cell_at(position);
        self.terminal.select_to(row, col, right_half);
        self.edge_col = (col, right_half);
        let top = layout.origin.y;
        let bottom = top + layout.line_height * layout.rows as f32;
        let lines = |past: Pixels| 1 + ((past / layout.line_height) as i32).min(8);
        self.edge_scroll = if position.y < top {
            lines(top - position.y)
        } else if position.y > bottom {
            -lines(position.y - bottom)
        } else {
            0
        };
        if self.edge_scroll == 0 {
            self.edge_tick = None;
        } else if self.edge_tick.is_none() {
            self.edge_tick = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(EDGE_TICK).await;
                    let going = this.update(cx, |this, cx| {
                        let Some(layout) = this.layout.get().filter(|_| this.selecting && this.edge_scroll != 0) else {
                            return false;
                        };
                        this.terminal.scroll(this.edge_scroll);
                        let row = if this.edge_scroll > 0 { 0 } else { layout.rows.saturating_sub(1) };
                        this.terminal.select_to(row, this.edge_col.0, this.edge_col.1);
                        cx.notify();
                        true
                    });
                    if !going.unwrap_or(false) {
                        break;
                    }
                }
            }));
        }
        cx.notify();
    }

    /// The button came up: the selection stands as it is (a click without a drag selects nothing).
    fn end_selection(&mut self, cx: &mut Context<Self>) {
        self.selecting = false;
        self.edge_scroll = 0;
        self.edge_tick = None;
        if self.terminal.selection_text().is_none() {
            self.terminal.select_clear();
        }
        cx.notify();
    }

    /// A move or a release outside the terminal while a selection is dragged (the terminal itself hears only what
    /// happens over it).
    fn dragged_outside(&mut self, position: Point<Pixels>, released: bool, cx: &mut Context<Self>) {
        if !self.selecting || self.layout.get().is_none_or(|layout| layout.contains(position)) {
            return;
        }
        if released { self.end_selection(cx) } else { self.drag_selection(position, cx) }
    }
}

/// The modifiers held, as the terminal names them.
fn mods(m: &Modifiers) -> Mods {
    Mods { shift: m.shift, alt: m.alt, ctrl: m.control, cmd: m.platform }
}

/// A mouse button the terminal reports; the back and forward buttons it doesn't.
fn button(b: MouseButton) -> Option<Button> {
    match b {
        MouseButton::Left => Some(Button::Left),
        MouseButton::Middle => Some(Button::Middle),
        MouseButton::Right => Some(Button::Right),
        MouseButton::Navigate(_) => None,
    }
}

/// The input method's side (macOS calls these while the terminal has the keyboard): a composition is held and drawn at
/// the cursor until the input method commits it, and committed text goes to the program as typed. Keys a program reads
/// (Enter, Esc, Ctrl, Option as Meta) still come through `key_down` first; only an input method in use takes printable
/// keys before it.
impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let marked = self.marked.as_deref()?;
        let (from, to) = (utf16_prefix(marked, range.start).len(), utf16_prefix(marked, range.end).len());
        let text = &marked[from..to.max(from)];
        *adjusted = Some(range.start..range.start + text.encode_utf16().count());
        Some(text.to_string())
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        let range = match &self.marked {
            Some(marked) => self.marked_caret.clone().unwrap_or_else(|| {
                let end = marked.encode_utf16().count();
                end..end
            }),
            None => 0..0,
        };
        Some(UTF16Selection { range, reversed: false })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|marked| 0..marked.encode_utf16().count())
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked_caret = None;
        if self.marked.take().is_some() {
            cx.notify();
        }
    }

    fn replace_text_in_range(&mut self, _: Option<Range<usize>>, text: &str, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        self.marked_caret = None;
        if !text.is_empty() {
            self.terminal.send(text.as_bytes().to_vec());
        }
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        caret: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked = (!text.is_empty()).then(|| text.to_string());
        self.marked_caret = caret;
        cx.notify();
    }

    /// Where the input method shows its candidates: under the composition, at the cursor's cell.
    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.layout.get()?;
        let (row, col) = self.screen.borrow().cursor_cell?;
        let before = self
            .marked
            .as_deref()
            .map_or(0, |marked| unicode_width::UnicodeWidthStr::width(utf16_prefix(marked, range_utf16.start)));
        let origin = point(
            layout.origin.x + layout.cell_width * (col + before) as f32,
            layout.origin.y + layout.line_height * row as f32,
        );
        Some(Bounds::new(origin, size(layout.cell_width, layout.line_height)))
    }

    fn character_index_for_point(&mut self, _: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        None
    }
}

impl Focusable for TerminalView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.watch_focus(window, cx);
        let focused = self.focus.is_focused(window);
        div()
            .id("terminal")
            .test_support()
            .aria_label("Terminal")
            .track_focus(&self.focus)
            .key_context(KEYS)
            .size_full()
            .map(|d| if self.hovered_link.is_some() { d.cursor_pointer() } else { d.cursor_text() })
            .on_modifiers_changed(cx.listener(Self::modifiers_changed))
            .on_key_down(cx.listener(Self::key_down))
            .on_key_up(cx.listener(Self::key_up))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::paste))
            .on_scroll(cx.listener(Self::scroll))
            .on_drop(cx.listener(Self::dropped))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::mouse_down))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up(MouseButton::Middle, cx.listener(Self::mouse_up))
            .on_mouse_up(MouseButton::Right, cx.listener(Self::mouse_up))
            .child(TerminalGrid {
                terminal: self.terminal.clone(),
                layout: self.layout.clone(),
                screen: self.screen.clone(),
                rows: self.rows.clone(),
                link: self.hovered_link.clone(),
                cursor_on: self.cursor_on,
                selecting: self.selecting,
                marked: self.marked.clone(),
                view: cx.entity(),
                focus: self.focus.clone(),
                focused,
            })
    }
}

/// The font sizes and cell size of the terminal.
struct Metrics {
    font_size: Pixels,
    cell_width: Pixels,
    line_height: Pixels,
}

fn mono(bold: bool, italic: bool) -> Font {
    Font {
        family: theme::MONO_FONT.into(),
        // Ligatures off, so `->` stays `->` (DESIGN.md, "Type").
        features: FontFeatures::disable_ligatures(),
        fallbacks: None,
        weight: if bold { FontWeight::BOLD } else { FontWeight::NORMAL },
        style: if italic { FontStyle::Italic } else { FontStyle::Normal },
    }
}

/// The size of the terminal's text when it isn't the usual one (item 8.4): the shell keeps it in step
/// with what is remembered. Without it (a picture, most tests) the text is DESIGN.md's 12.5 px.
pub struct TerminalText(pub Pixels);

impl gpui_kit::Global for TerminalText {}

/// The size of the terminal's text in this app.
pub fn text_size(cx: &App) -> Pixels {
    cx.try_global::<TerminalText>().map_or(theme::TERMINAL_TEXT, |text| text.0)
}

/// A terminal cell's width and height in this window at the usual size (pictures and tests find cells with it).
pub fn cell_size(window: &Window) -> (Pixels, Pixels) {
    let m = metrics(window, theme::TERMINAL_TEXT);
    (m.cell_width, m.line_height)
}

fn metrics(window: &Window, font_size: Pixels) -> Metrics {
    let text = window.text_system();
    let id = text.resolve_font(&mono(false, false));
    let cell_width = text.advance(id, font_size, 'm').map(|s| s.width).unwrap_or(px(7.5));
    let height = text.ascent(id, font_size) + text.descent(id, font_size).abs();
    let line_height = (height * theme::TERMINAL_LINE_SCALE).floor();
    // A text system that measures nothing (a test window) still gets a usable grid.
    let cell_width = if cell_width < px(1.) { px(7.5) } else { cell_width };
    let line_height = if line_height < px(1.) { px(20.) } else { line_height };
    Metrics { font_size, cell_width, line_height }
}

/// Draws the terminal's cells: backgrounds, the selection, the text a row at a time snapped to the grid, the
/// cursor.
pub struct TerminalGrid {
    terminal: Arc<Terminal>,
    layout: Rc<Cell<Option<GridLayout>>>,
    screen: Rc<RefCell<Screen>>,
    rows: Rc<RefCell<RowCache>>,
    /// The link under the pointer while ⌘ is held, underlined.
    link: Option<Link>,
    /// A blinking cursor is in its shown half.
    cursor_on: bool,
    /// A selection is being dragged: moves and the release outside the terminal are heard too.
    selecting: bool,
    /// An input method's composition, drawn at the cursor's cell.
    marked: Option<String>,
    /// The view, which takes the input method's text while the terminal has the keyboard.
    view: gpui_kit::Entity<TerminalView>,
    focus: FocusHandle,
    focused: bool,
}

pub struct PreparedGrid {
    /// When this frame's work on the grid began (for the flood measurement).
    started: std::time::Instant,
    line_height: Pixels,
    rects: Vec<(Bounds<Pixels>, Hsla)>,
    lines: Vec<(Point<Pixels>, ShapedLine)>,
    /// An input method's composition: its background and its text, drawn over the row.
    composition: Option<Composition>,
    cursor: Option<PaintQuad>,
}

impl IntoElement for TerminalGrid {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TerminalGrid {
    type RequestLayoutState = ();
    type PrepaintState = PreparedGrid;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> PreparedGrid {
        let started = std::time::Instant::now();
        let m = metrics(window, text_size(cx));
        let cols = ((bounds.size.width / m.cell_width).floor() as usize).clamp(2, u16::MAX as usize);
        let rows = ((bounds.size.height / m.line_height).floor() as usize).clamp(1, u16::MAX as usize);
        self.terminal.resize(GridSize {
            cols: cols as u16,
            rows: rows as u16,
            cell_width: (m.cell_width / px(1.0)).round() as u16,
            cell_height: (m.line_height / px(1.0)).round() as u16,
        });
        self.terminal.take_changed();
        let mut screen = self.screen.borrow_mut();
        let damage = self.terminal.update(&mut screen);
        let layout = GridLayout {
            origin: bounds.origin,
            cell_width: m.cell_width,
            line_height: m.line_height,
            cols: screen.cols,
            rows: screen.rows,
        };
        self.layout.set(Some(layout));
        // while composing, the composition stands where the cursor would
        let cursor_on = self.cursor_on && self.marked.is_none();
        let mut rows = self.rows.borrow_mut();
        let mut prepared = prepare(&screen, &damage, &mut rows, &layout, &m, self.focused, cursor_on, window);
        if self.marked.is_some() {
            prepared.cursor = None;
            if let (Some(text), Some((row, col))) = (&self.marked, screen.cursor_cell) {
                prepared.composition = Some(composition(text, row, col, &layout, &m, window));
            }
        }
        if let Some(link) = &self.link {
            prepared.rects.extend(link_underline(&screen, &layout, link));
        }
        prepared.started = started;
        prepared
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        prepared: &mut PreparedGrid,
        window: &mut Window,
        cx: &mut App,
    ) {
        for (bounds, color) in &prepared.rects {
            window.paint_quad(fill(*bounds, *color));
        }
        for (origin, line) in &prepared.lines {
            let _ = line.paint(*origin, prepared.line_height, TextAlign::Left, None, window, cx);
        }
        if let Some(((bounds, background), (origin, line))) = prepared.composition.take() {
            window.paint_quad(fill(bounds, background));
            let _ = line.paint(origin, prepared.line_height, TextAlign::Left, None, window, cx);
        }
        if self.selecting {
            // a selection dragged past the terminal's edge goes on, and ends where the button comes up
            let view = self.view.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                if phase == DispatchPhase::Bubble && event.pressed_button == Some(MouseButton::Left) {
                    view.update(cx, |view, cx| view.dragged_outside(event.position, false, cx));
                }
            });
            let view = self.view.clone();
            window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
                    view.update(cx, |view, cx| view.dragged_outside(event.position, true, cx));
                }
            });
        }
        // the input method's text comes here while the terminal has the keyboard
        window.handle_input(&self.focus, ElementInputHandler::new(bounds, self.view.clone()), cx);
        if let Some(cursor) = prepared.cursor.take() {
            window.paint_quad(cursor);
        }
        crate::measure::painted(&self.terminal, prepared.started.elapsed());
    }
}

fn color(c: Rgb) -> Hsla {
    gpui_kit::Rgba { r: f32::from(c.0) / 255.0, g: f32::from(c.1) / 255.0, b: f32::from(c.2) / 255.0, a: 1.0 }.into()
}

/// An input method's composition as drawn: its background, and its text where it starts.
type Composition = ((Bounds<Pixels>, Hsla), (Point<Pixels>, ShapedLine));

/// An input method's composition at a cell: its text, underlined, over the terminal's own background so the row under
/// it doesn't show through.
fn composition(
    text: &str,
    row: usize,
    col: usize,
    layout: &GridLayout,
    m: &Metrics,
    window: &mut Window,
) -> Composition {
    let palette = enkidu_term::Palette::enkidu();
    let ink = color(palette.foreground);
    let run = TextRun {
        len: text.len(),
        font: mono(false, false),
        color: ink,
        background_color: None,
        underline: Some(UnderlineStyle { thickness: px(1.), color: Some(ink), wavy: false }),
        strikethrough: None,
    };
    let shaped = window.text_system().shape_line(text.to_string().into(), m.font_size, &[run], None);
    let origin =
        point(layout.origin.x + layout.cell_width * col as f32, layout.origin.y + layout.line_height * row as f32);
    let cells = unicode_width::UnicodeWidthStr::width(text).max(1);
    let width = (layout.cell_width * cells as f32).max(shaped.width);
    ((Bounds::new(origin, size(width, layout.line_height)), color(palette.background)), (origin, shaped))
}

/// The part of `text` before `units` UTF-16 units (the input method counts in them).
fn utf16_prefix(text: &str, units: usize) -> &str {
    let mut seen = 0;
    for (at, c) in text.char_indices() {
        if seen >= units {
            return &text[..at];
        }
        seen += c.len_utf16();
    }
    text
}

/// U+200E, which makes a row a left-to-right paragraph.
const LEFT_TO_RIGHT: char = '\u{200e}';

/// The runs of a row's cells that read right to left (Arabic and Hebrew words, with the spaces and digits between
/// them), as (first cell, end cell), by the Unicode bidi rules for a left-to-right paragraph. `text` is the row as
/// shaped, `starts` where each cell's text begins in it.
fn right_to_left(text: &str, starts: &[usize]) -> Vec<(usize, usize)> {
    use unicode_bidi::{BidiClass, Level, ParagraphBidiInfo, bidi_class};
    if !text.chars().any(|c| matches!(bidi_class(c), BidiClass::R | BidiClass::AL)) {
        return Vec::new();
    }
    let info = ParagraphBidiInfo::new(text, Some(Level::ltr()));
    let mut runs = Vec::new();
    let mut open = None;
    for (cell, &start) in starts.iter().enumerate() {
        match (info.levels[start].number() > 0, open) {
            (true, None) => open = Some(cell),
            (false, Some(first)) => {
                runs.push((first, cell));
                open = None;
            }
            _ => {}
        }
    }
    if let Some(first) = open {
        runs.push((first, starts.len()));
    }
    runs
}

/// Puts each glyph of a shaped row in its cells. `starts` holds where each cell's text begins, `rtl` the runs that
/// read right to left. A cell's glyph goes to its column, and marks keep their place on it. A right-to-left run keeps
/// the shape the shaper gave it, its letters joined, set against the right edge of its own cells (squeezed only when
/// it is wider than they are), so it reads right to left and everything after it keeps its column. GPUI's own forced
/// width counts glyphs instead: a ligature such as Arabic lam-alef moved the rest of the row one cell to the left, and a
/// row that started with Arabic was laid out right to left as a whole.
fn on_the_grid(mut line: ShapedLine, starts: &[usize], rtl: &[(usize, usize)], cell: Pixels) -> ShapedLine {
    let cell_of = |index: usize| match starts.binary_search(&index) {
        Ok(c) => (c, true),
        Err(n) => (n.saturating_sub(1), false),
    };
    let run_of = |c: usize| rtl.iter().position(|&(first, end)| (first..end).contains(&c));
    // where the shaper put the first glyph of each cell's letter
    let mut shaped_at = vec![None; starts.len()];
    for glyph in line.runs.iter().flat_map(|run| &run.glyphs) {
        if let (c, true) = cell_of(glyph.index) {
            shaped_at[c].get_or_insert(glyph.position.x);
        }
    }
    // each right-to-left run: where it starts as shaped, where it goes, and how much it is squeezed
    let moves: Vec<(Pixels, Pixels, f32)> = rtl
        .iter()
        .enumerate()
        .map(|(r, &(first, end))| {
            let xs = line.runs.iter().flat_map(|run| &run.glyphs).filter(|g| run_of(cell_of(g.index).0) == Some(r));
            let (start, last) =
                xs.fold((Pixels::MAX, Pixels::MIN), |(lo, hi), g| (lo.min(g.position.x), hi.max(g.position.x)));
            // its right edge: the next thing the shaper put after it
            let edge = line
                .runs
                .iter()
                .flat_map(|run| &run.glyphs)
                .filter(|g| run_of(cell_of(g.index).0) != Some(r) && g.position.x > last)
                .map(|g| g.position.x)
                .fold(line.width, Pixels::min);
            let natural = (edge - start).max(px(0.01));
            let room = cell * (end - first) as f32;
            if natural <= room {
                (start, cell * first as f32 + (room - natural), 1.0)
            } else {
                (start, cell * first as f32, room / natural)
            }
        })
        .collect();
    let mut layout = LineLayout {
        font_size: line.font_size,
        width: cell * starts.len() as f32,
        ascent: line.ascent,
        descent: line.descent,
        runs: line.runs.clone(),
        len: line.len,
    };
    for glyph in layout.runs.iter_mut().flat_map(|run| &mut run.glyphs) {
        let (c, _) = cell_of(glyph.index);
        if let Some(r) = run_of(c) {
            let (start, at, squeeze) = moves[r];
            glyph.position.x = at + (glyph.position.x - start) * squeeze;
        } else if let Some(shaped) = shaped_at[c] {
            glyph.position.x = cell * c as f32 + (glyph.position.x - shaped);
        }
    }
    let laid: &mut Arc<LineLayout> = &mut line;
    *laid = Arc::new(layout);
    line
}

/// A line under each row of a link's cells, in its text's color, just under the letters.
fn link_underline(screen: &Screen, layout: &GridLayout, link: &Link) -> Vec<(Bounds<Pixels>, Hsla)> {
    link.cells
        .iter()
        .filter(|&&(row, first, last)| row < screen.rows && first <= last && last < screen.cols)
        .map(|&(row, first, last)| {
            let origin = point(
                layout.origin.x + layout.cell_width * first as f32,
                layout.origin.y + layout.line_height * (row + 1) as f32 - px(2.),
            );
            let width = layout.cell_width * (last - first + 1) as f32;
            (Bounds { origin, size: size(width, px(1.)) }, color(screen.row(row)[first].fg))
        })
        .collect()
}

/// What one row draws: its colored runs of cells (backgrounds that aren't the terminal's own, then the selection) as
/// (first column, end column, color), and its text, shaped and placed on the grid.
pub struct RowPaint {
    fills: Vec<(usize, usize, Hsla)>,
    line: Option<ShapedLine>,
}

/// The rows drawn in the last two frames, by what they hold: a row that stayed as it was, or only moved up as output
/// scrolled, is not shaped again. A row is read again only when the terminal says it changed ([`Damage`]).
#[derive(Default)]
pub struct RowCache {
    /// What each row on screen held when it was last read (its cells; the cursor is added to the key apart).
    cells: Vec<u64>,
    now: HashMap<u64, Rc<RowPaint>>,
    before: HashMap<u64, Rc<RowPaint>>,
    /// The font size and cell width the rows were shaped with.
    metrics: Option<(Pixels, Pixels)>,
    /// The rows shaped in the last frame.
    shaped: usize,
}

/// A row's cells, as a number that changes when anything drawn in them does.
fn row_hash(cells: &[ScreenCell]) -> u64 {
    let mut h = DefaultHasher::new();
    for c in cells {
        (c.ch, &c.extra).hash(&mut h);
        (c.fg.0, c.fg.1, c.fg.2, c.bg.0, c.bg.1, c.bg.2).hash(&mut h);
        (c.default_bg, c.bold, c.italic, c.underline, c.strike, c.wide, c.spacer, c.selected).hash(&mut h);
    }
    h.finish()
}

/// Draws one row: its colored runs and its text. `block` is the block cursor on it (its column, and whether it is
/// on a wide character): the text under it takes the background's color.
fn row_paint(
    cells: &[ScreenCell],
    block: Option<(usize, bool)>,
    m: &Metrics,
    cw: Pixels,
    window: &mut Window,
) -> RowPaint {
    // Backgrounds that aren't the terminal's own, then the selection, as runs of cells.
    let mut fills = Vec::new();
    let mut runs_of = |want: &dyn Fn(&ScreenCell) -> Option<Hsla>| {
        let mut col = 0;
        while col < cells.len() {
            let Some(fill_color) = want(&cells[col]) else {
                col += 1;
                continue;
            };
            let start = col;
            while col < cells.len() && want(&cells[col]) == Some(fill_color) {
                col += 1;
            }
            fills.push((start, col, fill_color));
        }
    };
    runs_of(&|c: &ScreenCell| (!c.default_bg).then(|| color(c.bg)));
    runs_of(&|c: &ScreenCell| c.selected.then(|| hsla(theme::TERMINAL_SELECTION)));

    // The text of the row as one line, each glyph in its cells.
    let end =
        cells.iter().rposition(|c| c.ch != ' ' || c.underline || c.strike || c.extra.is_some()).map_or(0, |i| i + 1);
    if end == 0 {
        return RowPaint { fills, line: None };
    }
    // A left-to-right mark first: the row reads left to right whatever its first letter (a row that starts with
    // Arabic would turn around); right-to-left words inside it still read right to left.
    let mut text = String::with_capacity(end + 3);
    text.push(LEFT_TO_RIGHT);
    let mut starts = Vec::with_capacity(end);
    let mut runs: Vec<TextRun> = Vec::new();
    let mut last: Option<(Rgb, bool, bool, bool, bool)> = None;
    for (col, cell) in cells[..end].iter().enumerate() {
        let start = text.len();
        starts.push(start);
        text.push(if cell.spacer || cell.ch == '\0' { ' ' } else { cell.ch });
        if let Some(extra) = &cell.extra {
            text.extend(extra.iter());
        }
        let len = text.len() - start;
        let under_block = block.is_some_and(|(at, wide)| at == col || (wide && at + 1 == col));
        let fg = if under_block { Rgb(0x11, 0x12, 0x17) } else { cell.fg };
        let style = (fg, cell.bold, cell.italic, cell.underline, cell.strike);
        if last == Some(style) {
            if let Some(run) = runs.last_mut() {
                run.len += len;
            }
            continue;
        }
        last = Some(style);
        let ink = color(fg);
        runs.push(TextRun {
            len,
            font: mono(cell.bold, cell.italic),
            color: ink,
            background_color: None,
            underline: cell.underline.then_some(UnderlineStyle { thickness: px(1.), color: Some(ink), wavy: false }),
            strikethrough: cell.strike.then_some(StrikethroughStyle { thickness: px(1.), color: Some(ink) }),
        });
    }
    if let Some(first) = runs.first_mut() {
        first.len += LEFT_TO_RIGHT.len_utf8();
    }
    let shaped = window.text_system().shape_line(text.into(), m.font_size, &runs, None);
    let rtl = right_to_left(&shaped.text, &starts);
    RowPaint { fills, line: Some(on_the_grid(shaped, &starts, &rtl, cw)) }
}

/// Turns a screen copy into what to paint. Rows come from `cache` unless `damage` says they changed and their new
/// content wasn't drawn in the last two frames.
#[allow(clippy::too_many_arguments)]
fn prepare(
    screen: &Screen,
    damage: &Damage,
    cache: &mut RowCache,
    layout: &GridLayout,
    m: &Metrics,
    focused: bool,
    cursor_on: bool,
    window: &mut Window,
) -> PreparedGrid {
    // a blinking cursor in its hidden half isn't drawn
    let drawn_cursor = screen.cursor.filter(|c| cursor_on || !c.blinking);
    let cw = layout.cell_width;
    let lh = layout.line_height;
    let at = |row: usize, col: usize| point(layout.origin.x + cw * col as f32, layout.origin.y + lh * row as f32);
    let mut rects = Vec::new();
    let mut lines = Vec::new();

    // A block cursor on a cell draws that cell's text in the background color.
    let block = drawn_cursor.filter(|c| focused && c.kind == CursorKind::Block);

    if cache.metrics != Some((m.font_size, cw)) {
        *cache = RowCache { metrics: Some((m.font_size, cw)), ..RowCache::default() };
    }
    cache.before = std::mem::take(&mut cache.now);
    cache.shaped = 0;
    let fresh = cache.cells.len() != screen.rows;
    if fresh {
        cache.cells = vec![0; screen.rows];
    }
    for row in 0..screen.rows {
        let cells = screen.row(row);
        if fresh || damage.has(row) {
            cache.cells[row] = row_hash(cells);
        }
        let block_here = block.filter(|b| b.row == row).map(|b| (b.col, b.wide));
        let key = {
            let mut h = DefaultHasher::new();
            (cache.cells[row], block_here).hash(&mut h);
            h.finish()
        };
        let paint = match cache.now.get(&key).cloned().or_else(|| cache.before.remove(&key)) {
            Some(paint) => paint,
            None => {
                let paint = row_paint(cells, block_here, m, cw, window);
                cache.shaped += usize::from(paint.line.is_some());
                Rc::new(paint)
            }
        };
        for &(start, end, fill_color) in &paint.fills {
            rects.push((Bounds::new(at(row, start), size(cw * (end - start) as f32, lh)), fill_color));
        }
        if let Some(line) = &paint.line {
            lines.push((at(row, 0), line.clone()));
        }
        cache.now.insert(key, paint);
    }

    let ink = hsla(theme::TERMINAL_CURSOR);
    let mut cursor = None;
    if let Some(c) = drawn_cursor {
        let width = if c.wide { cw * 2.0 } else { cw };
        let origin = at(c.row, c.col);
        let whole = Bounds::new(origin, size(width, lh));
        if block.is_some() {
            // A block goes under its text, which is drawn in the background color.
            rects.push((whole, ink));
        } else if !focused || c.kind == CursorKind::HollowBlock {
            cursor = Some(outline(whole, ink, gpui_kit::BorderStyle::Solid));
        } else if c.kind == CursorKind::Underline {
            cursor = Some(fill(Bounds::new(point(origin.x, origin.y + lh - px(2.)), size(width, px(2.))), ink));
        } else {
            cursor = Some(fill(Bounds::new(origin, size(px(2.), lh)), ink));
        }
    }
    PreparedGrid { started: std::time::Instant::now(), line_height: lh, rects, lines, composition: None, cursor }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_terminal_font_has_ligatures_off() {
        for (bold, italic) in [(false, false), (true, false), (false, true), (true, true)] {
            let font = mono(bold, italic);
            assert_eq!(font.family.as_ref(), "JetBrains Mono");
            assert_eq!(font.features.is_calt_enabled(), Some(false), "calt is off, so -> stays ->");
        }
    }

    /// A row as the view shapes it: the left-to-right mark, then each cell's text; and where each cell starts.
    fn row(cells: &str) -> (String, Vec<usize>) {
        let mut text = String::from(LEFT_TO_RIGHT);
        let mut starts = Vec::new();
        for c in cells.chars() {
            starts.push(text.len());
            text.push(c);
        }
        (text, starts)
    }

    #[test]
    fn finds_the_cells_that_read_right_to_left() {
        let rtl = |cells: &str| {
            let (text, starts) = row(cells);
            right_to_left(&text, &starts)
        };
        assert_eq!(rtl("abcdefgh|ascii"), []);
        assert_eq!(rtl("中文 emoji 🙂"), []);
        // two words and the space between them are one run; the bar after them is not in it
        assert_eq!(rtl("مرحبا بك|arabic"), [(0, 8)]);
        assert_eq!(rtl("ab مرحبا|mixed"), [(3, 8)]);
        // digits after Arabic read with it
        assert_eq!(rtl("عدد 12x|digits"), [(0, 6)]);
        assert_eq!(rtl("a שלום b سلام c"), [(2, 6), (9, 13)]);
    }
}
