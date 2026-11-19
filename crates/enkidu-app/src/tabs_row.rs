//! The top row (DESIGN.md, "Layout": "The top row is the project on screen"): after the window's three lights the
//! project's name, which opens its menu; its workspaces as tabs, each with its lamp, its name and what it waits for;
//! the workspaces put away, and New workspace; and at the right what acts on the workspace on screen: the layout, Go
//! live and its menu.
//!
//! With little room the tabs shorten their names, each down to what it must show; when even that doesn't fit, the
//! row scrolls and fades at the cut edge. The tab on screen stands on the pill with the blue edge, which glides to
//! the next one you pick, and is always scrolled into view.
//!
//! The tabs stand in your own order, and a workspace can be put away (DESIGN.md, "Your own order, and workspaces
//! put away"). A tab is dragged onto the tab whose place it takes. "Hide workspace" is in the workspace's menu,
//! which a right-click on any tab opens for that tab's workspace. The ones put away stand behind a small button
//! after the tabs, whose menu brings one back. What is arranged is the frame's (`Frame::move_tab`,
//! `hide_workspace`, `show_workspace`) and is kept with where you were.
//!
//! With the lists hidden (⌘S) the row also keeps what must not be lost: the button that brings them back, and the
//! other projects' workspaces as small dots.

use std::collections::HashMap;

use enkidu_core::db::Workspace;
use enkidu_model::frame::Frame;
use enkidu_model::live::LiveState;
use enkidu_model::ui::{Layout, Mode, Ui};
use enkidu_model::work::pr::{PrLamp, TabPr, tab_chip};
use enkidu_model::work::when::since_words;
use enkidu_ui::lamp::{Status, lamp};
use enkidu_ui::live::live_badge;
use enkidu_ui::pieces::{MenuTrigger, Observed, Pressed, border_width, icon_button, menu_item, tabular};
use enkidu_ui::segmented::{segment_sized, segmented};
use enkidu_ui::tab_pill::scrolling_tab_row;
use enkidu_ui::tips::Tipped as _;
use enkidu_ui::tokens::{self as theme, alpha};
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::prelude::*;
use gpui_kit::{
    Action, AnyElement, BoxShadow, Context, DismissEvent, DragMoveEvent, Entity, FocusHandle, Focusable as _, Font,
    FontWeight, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, Pixels, Point, Render, Role, ScrollHandle,
    SharedString, TextRun, Window, anchored, deferred, div, linear_color_stop, linear_gradient, point, px, svg,
};

use crate::work::store::Work;

/// A path as you'd say it (enkidu-model's rule): a home folder becomes `~`.
pub use enkidu_model::text::short_path;

pub struct TabsRow {
    frame: Entity<Frame>,
    /// The Work index: a workspace's pull request shows on its tab. None in a window without one.
    work: Option<Entity<Work>>,
    /// How the workspace is laid out: the switch at the right shows it and changes it.
    ui: Entity<Ui>,
    scroll: ScrollHandle,
    /// Each tab's hold on the keyboard, by workspace: a click on a tab gives it the keyboard, and then ← and → move
    /// it.
    focus: HashMap<String, FocusHandle>,
    /// The pointer is on the Live button: Live turns into Stop.
    live_pointed: bool,
    /// The tab that is being dragged along the row: its workspace, and its place when the drag began.
    held: Option<(String, usize)>,
    /// The place of the tab the pointer is over while one is dragged: where the held one would land.
    drop_at: Option<usize>,
    /// The menu a right-click on a tab opened, and where the pointer was.
    menu: Option<(Entity<PopupMenu>, Point<Pixels>)>,
}

/// A workspace's tab while it is dragged along the row: what the tabs it passes over are told.
#[derive(Clone)]
struct DraggedTab {
    id: String,
    name: SharedString,
    status: Status,
}

/// The tab that follows the pointer while it is dragged: its lamp and its name, lifted off the row.
struct TabGhost {
    name: SharedString,
    status: Status,
}

impl Render for TabGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .h(theme::CONTROL)
            .max_w(TAB_MAX)
            .pl(px(8.))
            .pr(px(12.))
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded_full()
            .bg(theme::SHEET)
            .border_1()
            .border_color(theme::LINE_STRONG)
            .shadow(enkidu_ui::pieces::float_shadow())
            .text_size(theme::TEXT)
            .font_weight(theme::WEIGHT_TITLE)
            .text_color(theme::INK)
            .child(div().flex_none().child(lamp("tab-ghost-lamp", self.status, 16., 0)))
            .child(div().truncate().child(self.name.clone()))
    }
}

/// Now, in milliseconds since 1970: when a workspace was put away.
pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_millis() as i64)
}

/// The action that picks the tab at `ix` (⌘1 for the first), for the first nine.
pub fn tab_action(ix: usize) -> Option<Box<dyn Action>> {
    Some(match ix {
        0 => Box::new(crate::SelectTab1),
        1 => Box::new(crate::SelectTab2),
        2 => Box::new(crate::SelectTab3),
        3 => Box::new(crate::SelectTab4),
        4 => Box::new(crate::SelectTab5),
        5 => Box::new(crate::SelectTab6),
        6 => Box::new(crate::SelectTab7),
        7 => Box::new(crate::SelectTab8),
        8 => Box::new(crate::SelectTab9),
        _ => return None,
    })
}

/// How many files a workspace changed, in words (the label of the count on its tab).
pub fn changed_words(changed: usize) -> String {
    if changed == 1 { "1 changed file".into() } else { format!("{changed} changed files") }
}

/// How many remarks a workspace has that weren't sent to Claude yet, in words (the label of the count on its tab).
pub fn unsent_words(remarks: usize) -> String {
    format!("{} not sent yet", enkidu_model::remarks::remarks_words(remarks))
}

/// How strong a done lamp is on a tab that is not on screen (DESIGN.md, "States": the three ranks in the top row).
const DONE_ELSEWHERE: f32 = 0.55;

/// A tab's name is cut to no less than this, and a tab is never wider than [`TAB_MAX`].
const NAME_MIN: Pixels = px(28.);
/// What is kept of a long name when the row is tight.
const NAME_KEPT: Pixels = px(64.);
const TAB_MAX: Pixels = px(256.);

/// How wide `text` is drawn in the UI face at `size` and `weight`, its figures tabular as a tab's counts are: a
/// whole number of pixels, never less than the words take.
fn text_width(window: &Window, text: &str, size: Pixels, weight: FontWeight) -> Pixels {
    let font = Font { weight, features: enkidu_ui::pieces::tabular_figures(), ..gpui_kit::font(theme::UI_FONT) };
    drawn_width(window, text, size, font)
}

/// How wide a workspace's name is drawn at `weight`: its figures are the face's own, each as wide as it is.
fn name_width(window: &Window, name: &str, weight: FontWeight) -> Pixels {
    drawn_width(window, name, theme::TEXT, Font { weight, ..gpui_kit::font(theme::UI_FONT) })
}

fn drawn_width(window: &Window, text: &str, size: Pixels, font: Font) -> Pixels {
    let run = TextRun { len: text.len(), font, ..Default::default() };
    px(f32::from(window.text_system().layout_line(text, size, &[run], None).width).ceil())
}

/// What a tab says after its name, as it is laid out: the pieces, and how wide they are together with the 8 px
/// between them (0 when it says nothing).
struct Said {
    parts: Vec<AnyElement>,
    width: Pixels,
}

/// A workspace's pull request on its tab: a dot in the color of its state (its checks, or merged, or closed). Its
/// tooltip has the words.
fn pr_dot(ix: usize, pr: &TabPr) -> AnyElement {
    let (color, lamp) = match pr.lamp {
        PrLamp::Red => (theme::LAMP_RED, "red"),
        PrLamp::Amber => (theme::LAMP_AMBER, "amber"),
        PrLamp::Green => (theme::LAMP_GREEN, "green"),
        PrLamp::Violet => (theme::oklch(0.72, 0.14, 300.), "violet"),
        PrLamp::Gray => (alpha(theme::INK_3, 0.6), "gray"),
    };
    let title = SharedString::from(pr.title.clone());
    div()
        .id(("tab-pr", ix))
        .test_support()
        .aria_label(title.clone())
        .aria_description(if pr.waiting { "waits on you" } else { "nothing waits" })
        .aria_value(lamp)
        .flex_none()
        // (8 px above and under the dot are the pointer's: a tooltip needs more than 6 px to rest on)
        .py(px(8.))
        .tipped(title, None)
        .child(div().size(px(6.)).rounded_full().bg(color))
        .into_any_element()
}

/// What a tab says after its name. What needs you says only that; else Live, the remarks you wrote and haven't
/// sent, and the number of changed files, each there only when it has something to say. Then its pull request.
fn waits(
    ix: usize,
    status: Status,
    (live, remarks, changed): (bool, usize, usize),
    pr: Option<&TabPr>,
    window: &Window,
) -> Said {
    let small = theme::TEXT_SMALL;
    let mut parts: Vec<AnyElement> = Vec::new();
    let mut widths: Vec<Pixels> = Vec::new();
    if status == Status::NeedsYou {
        widths.push(text_width(window, status.label(), small, FontWeight::SEMIBOLD));
        parts.push(
            div()
                .id(("tab-words", ix))
                .test_support()
                .aria_label(status.label())
                .flex_none()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::LAMP_RED)
                .child(status.label())
                .into_any_element(),
        );
    } else {
        if live {
            // Live's pill: 8 px in, its three bars (2.5 px each, 2 px apart), 4 px, the word, 8 px out
            widths.push(px(8. + 11.5 + 4. + 8.) + text_width(window, "Live", theme::TEXT_STATUS, FontWeight::SEMIBOLD));
            parts.push(live_badge(("tab-live", ix), false).into_any_element());
        }
        // what you wrote is blue
        if remarks > 0 {
            let count = remarks.to_string();
            widths.push(px(12. + 4.) + text_width(window, &count, small, FontWeight::SEMIBOLD));
            parts.push(
                tabular(div())
                    .id(("tab-remarks", ix))
                    .test_support()
                    .aria_label(unsent_words(remarks))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::ACCENT)
                    .child(
                        svg().path("icons/message-square.svg").flex_none().size(theme::MARK).text_color(theme::ACCENT),
                    )
                    .child(count)
                    .into_any_element(),
            );
        }
        if changed > 0 {
            let count = changed.to_string();
            widths.push(px(12. + 4.) + text_width(window, &count, small, FontWeight::NORMAL));
            parts.push(
                tabular(div())
                    .id(("tab-changed", ix))
                    .test_support()
                    .aria_label(changed_words(changed))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(svg().path("icons/file-diff.svg").flex_none().size(theme::MARK).text_color(theme::INK_3))
                    .child(count)
                    .into_any_element(),
            );
        }
    }
    if let Some(pr) = pr {
        widths.push(px(6.));
        parts.push(pr_dot(ix, pr));
    }
    let width = widths.iter().fold(px(0.), |sum, width| sum + *width) + px(8.) * widths.len().saturating_sub(1) as f32;
    Said { parts, width }
}

/// The least of a name a tab shows: its whole name when that is short, else [`NAME_KEPT`] of it (some eight
/// letters and the dots), so a tab in a tight row can still be told from the next one.
fn name_min(name: Pixels) -> Pixels {
    name.min(NAME_KEPT).max(NAME_MIN)
}

/// The narrowest a tab may get: 112 px, or its padding, lamp, name cut to `name` px ([`name_min`]) and words when
/// they need more, so a narrow tab never runs into the next one. A row whose tabs can't get that scrolls.
fn tab_min_width(name: Pixels, said: Pixels) -> Pixels {
    let after = if said > px(0.) { px(8.) + said } else { px(0.) };
    (px(8. + 16. + 8.) + name + after + px(12.)).max(px(112.))
}

/// The edge where the row is cut: the frame's color fading out over 28 px.
fn fade(id: &'static str, left: bool) -> Observed {
    let (solid, clear) = (linear_color_stop(theme::PANE, 0.), linear_color_stop(alpha(theme::PANE, 0.), 1.));
    let (from, to) = if left {
        (solid, clear)
    } else {
        (linear_color_stop(alpha(theme::PANE, 0.), 0.), linear_color_stop(theme::PANE, 1.))
    };
    div()
        .id(id)
        .test_support()
        .absolute()
        .top_0()
        .bottom_0()
        .w(px(28.))
        .when(left, |d| d.left_0())
        .when(!left, |d| d.right_0())
        .bg(linear_gradient(90., from, to))
}

/// The ring on a tab that has the keyboard (DESIGN.md, "States": "a card or a row the arrow keys are on has a blue
/// edge"): 2 px of blue standing 2 px inside the tab, so it is never taken for the blue edge of the tab on screen
/// (that tab keeps its edge, and has the ring inside it as well).
///
/// The rings are inset shadows without blur, painted one over the other: inside the tab, so the row never cuts
/// them where it scrolls, and nothing is painted under the tab.
fn focus_ring(on_screen: bool) -> Vec<BoxShadow> {
    let ring = |color: gpui_kit::Rgba, reach: f32| BoxShadow {
        color: color.into(),
        offset: point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: px(reach),
        inset: true,
    };
    let edge = f32::from(theme::ON_SCREEN_EDGE);
    match on_screen {
        // over the pill: the ring, the stage's black as the gap, and the pill's blue edge again
        true => vec![ring(theme::ACCENT, edge + 4.), ring(theme::WELL, edge + 2.), ring(theme::ACCENT, edge)],
        // the ring, and the row's own color as the gap
        false => vec![ring(theme::ACCENT, 4.), ring(theme::PANE, 2.)],
    }
}

/// A workspace that is put away, as the menu of the hidden ones lists it.
#[derive(Clone)]
struct HiddenRow {
    id: String,
    name: SharedString,
    status: Status,
    /// "since Mon 2 Nov".
    since: SharedString,
}

/// One line of that menu: its lamp, its name, and since when it is put away.
fn hidden_row(at: usize, row: HiddenRow) -> PopupMenuItem {
    PopupMenuItem::element(move |_, _| {
        div()
            .id(("hidden-workspace", at))
            .test_support()
            .aria_label(row.name.clone())
            .aria_description(row.since.clone())
            .aria_value(SharedString::from(row.id.clone()))
            .w_full()
            .flex()
            .items_center()
            .gap(px(8.))
            .map(enkidu_ui::pieces::in_menu)
            .text_color(theme::INK)
            .child(div().flex_none().child(lamp(("hidden-lamp", at), row.status, 16., 0)))
            .child(div().min_w_0().flex_1().truncate().child(row.name.clone()))
            .child(
                div()
                    .flex_none()
                    .pl(px(12.))
                    .text_size(theme::TEXT_STATUS)
                    .text_color(theme::INK_3)
                    .child(row.since.clone()),
            )
    })
}

/// The hairline between two groups of the row.
fn rule() -> gpui_kit::Div {
    div().flex_none().h(px(16.)).w(px(1.)).bg(theme::LINE_STRONG)
}

impl TabsRow {
    pub fn new(frame: Entity<Frame>, ui: Entity<Ui>, work: Option<Entity<Work>>, cx: &mut Context<Self>) -> Self {
        cx.observe(&frame, |_, _, cx| cx.notify()).detach();
        cx.observe(&ui, |_, _, cx| cx.notify()).detach();
        if let Some(work) = &work {
            cx.observe(work, |_, _, cx| cx.notify()).detach();
        }
        Self {
            frame,
            work,
            ui,
            scroll: ScrollHandle::new(),
            focus: HashMap::new(),
            live_pointed: false,
            held: None,
            drop_at: None,
            menu: None,
        }
    }

    /// Whether the row is cut on the left and on the right (it has more tabs that way, behind a fade).
    pub fn cut(&self) -> (bool, bool) {
        let (offset, max) = (self.scroll.offset(), self.scroll.max_offset());
        (offset.x < px(-1.), max.x + offset.x > px(1.))
    }

    /// A right-click on a tab: the workspace's menu at the pointer, for that tab's workspace (it need not be on
    /// screen). It is drawn in the frame of every menu; when it goes, the keyboard goes back where it was.
    fn open_menu(&mut self, ws: Workspace, at: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let (frame, back) = (self.frame.clone(), window.focused(cx));
        let menu = PopupMenu::build(window, cx, move |menu, _, _| workspace_items(menu, frame, ws));
        menu.focus_handle(cx).focus(window, cx);
        cx.subscribe_in(&menu, window, move |this, menu, _: &DismissEvent, window, cx| {
            // (what was chosen may have opened something that took the keyboard: then it keeps it)
            if menu.focus_handle(cx).contains_focused(window, cx)
                && let Some(back) = &back
            {
                back.focus(window, cx);
            }
            this.menu = None;
            cx.notify();
        })
        .detach();
        self.menu = Some((menu, at));
        cx.notify();
    }

    /// A key on a tab that has the keyboard: ← and → move to the tab before or after (around the ends), Enter and
    /// Space put it on screen.
    fn tab_key(&mut self, ix: usize, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        if keystroke.modifiers.modified() {
            return;
        }
        let tabs: Vec<String> = self.frame.read(cx).tabs().iter().map(|w| w.id.clone()).collect();
        let count = tabs.len();
        if count == 0 || ix >= count {
            return;
        }
        match keystroke.key.as_str() {
            "left" | "right" => {
                let next = if keystroke.key == "right" { (ix + 1) % count } else { (ix + count - 1) % count };
                if let Some(handle) = self.focus.get(&tabs[next]) {
                    handle.focus(window, cx);
                }
                cx.stop_propagation();
            }
            "enter" | "space" => {
                self.frame.update(cx, |frame, cx| {
                    if frame.select_workspace(&tabs[ix]) {
                        cx.notify();
                    }
                });
                cx.stop_propagation();
            }
            _ => {}
        }
    }

    /// A workspace's tab: its lamp, its name, then what it waits for.
    fn tab(
        &self,
        ix: usize,
        ws: &Workspace,
        frame: &Frame,
        on_screen: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Pressed {
        let status = Status::from_core(frame.status(&ws.id));
        let says = (frame.is_live(ws), frame.remarks(&ws.id), frame.changed_files(&ws.id));
        // the workspace's PR, if it has one
        let pr = (self.work.as_ref())
            .and_then(|work| tab_chip(work.read(cx).snapshot.as_deref(), &ws.id, Some(&ws.session_id)));
        let said = waits(ix, status, says, pr.as_ref(), window);
        // (measured in the weight it is drawn in: the tab on screen is semibold, the others as the window's text)
        let weight = if on_screen { theme::WEIGHT_TITLE } else { window.text_style().font_weight };
        let name_kept = name_min(name_width(window, &ws.name, weight));
        let tab_min = tab_min_width(name_kept, said.width);
        let tip = match &ws.base {
            Some(base) => format!("{}, from {base}", ws.branch),
            None => ws.branch.clone(),
        };
        // while a tab is dragged: this one is the one held (its place in the row dims), or the one the pointer is
        // over, which shows where the held one would land: at its right edge when it comes from the left, at its
        // left edge when it comes from the right
        let held = self.held.as_ref().is_some_and(|(held, _)| held == &ws.id);
        let lands = match (&self.held, self.drop_at) {
            (Some((_, from)), Some(at)) if at == ix && *from != ix => Some(*from < ix),
            _ => None,
        };
        let dragged = DraggedTab { id: ws.id.clone(), name: ws.name.clone().into(), status };
        let (this, menu_ws, id) = (cx.entity().downgrade(), ws.clone(), ws.id.clone());
        // it answers a press, as every tab and button does
        enkidu_ui::press::pressable(div().id(("tab", ix)))
            .test_support()
            .role(Role::Tab)
            .aria_label(SharedString::from(ws.name.clone()))
            .aria_selected(on_screen)
            // (what a test reads: whether it is the one held, or where the held one would land beside it)
            .aria_description(match (held, lands) {
                (true, _) => "held",
                (_, Some(true)) => "lands after",
                (_, Some(false)) => "lands before",
                _ => "",
            })
            .relative()
            .when(held, |tab| tab.opacity(0.35))
            .children(lands.map(|after| {
                let line = div().absolute().top(px(4.)).bottom(px(4.)).w(px(2.)).rounded_full().bg(theme::ACCENT);
                if after { line.right(px(-3.)) } else { line.left(px(-3.)) }
            }))
            // dragged along the row, it takes the place of the tab it is let go on
            .on_drag(dragged, move |dragged, _, _, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.held = Some((dragged.id.clone(), ix));
                    cx.notify();
                });
                cx.new(|_| TabGhost { name: dragged.name.clone(), status: dragged.status })
            })
            .on_drag_move(cx.listener(move |this, event: &DragMoveEvent<DraggedTab>, _, cx| {
                let over = event.bounds.contains(&event.event.position);
                if over && this.drop_at != Some(ix) {
                    this.drop_at = Some(ix);
                    cx.notify();
                } else if !over && this.drop_at == Some(ix) {
                    this.drop_at = None;
                    cx.notify();
                }
            }))
            .on_drop(cx.listener(move |this, dragged: &DraggedTab, _, cx| {
                (this.held, this.drop_at) = (None, None);
                this.frame.update(cx, |frame, cx| {
                    if frame.move_tab(&dragged.id, ix) {
                        cx.notify();
                    }
                });
                cx.notify();
            }))
            .track_focus(&self.focus[&ws.id])
            .focus_visible(move |s| s.shadow(focus_ring(on_screen)))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| this.tab_key(ix, event, window, cx)))
            .h(theme::CONTROL)
            // it shortens like a browser's tab down to what it must show; the tab on screen keeps its name whole
            .min_w(tab_min)
            .max_w(TAB_MAX)
            .flex_shrink(if on_screen { 0. } else { 1. })
            .pl(px(8.))
            .pr(px(12.))
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded_full()
            .text_size(theme::TEXT)
            .cursor_pointer()
            .map(|tab| match on_screen {
                true => tab.text_color(theme::INK),
                // what needs you is not quiet
                false => tab
                    .text_color(if status == Status::NeedsYou { theme::INK } else { theme::INK_2 })
                    .hover(|s| s.bg(theme::HOVER).text_color(theme::INK)),
            })
            .tipped(tip, tab_action(ix))
            .child(
                div()
                    .flex_none()
                    // "done" is the least urgent thing a workspace says: quiet on a tab that is not on screen
                    .when(status == Status::Done && !on_screen, |d| d.opacity(DONE_ELSEWHERE))
                    .child(lamp(SharedString::from(format!("tab-lamp-{id}")), status, 16., frame.pops(&ws.id))),
            )
            .child(
                div()
                    .min_w(name_kept)
                    .flex_shrink(1.)
                    .truncate()
                    .when(on_screen, |d| d.font_weight(theme::WEIGHT_TITLE))
                    .child(ws.name.clone()),
            )
            .when(!said.parts.is_empty(), |tab| {
                tab.child(
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .text_size(theme::TEXT_SMALL)
                        .text_color(theme::INK_3)
                        .children(said.parts),
                )
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.frame.update(cx, |frame, cx| {
                    if frame.select_workspace(&id) {
                        cx.notify();
                    }
                })
            }))
            // a right-click: the workspace's menu, for this tab's workspace (it need not be on screen)
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_menu(menu_ws.clone(), event.position, window, cx);
                }),
            )
    }

    /// The workspaces of the project on screen that are put away, behind a small button after the tabs: an eye
    /// with a line through it and their number. Its menu lists them, each with its lamp, its name and since when;
    /// a click brings one back to its place among the tabs, and on screen.
    fn hidden_button(&self, hidden: &[&Workspace], frame: &Frame) -> AnyElement {
        let count = hidden.len();
        let words = match count {
            1 => "1 hidden workspace".to_string(),
            count => format!("{count} hidden workspaces"),
        };
        let now = now_ms();
        let rows: Vec<HiddenRow> = (hidden.iter())
            .map(|ws| HiddenRow {
                id: ws.id.clone(),
                name: ws.name.clone().into(),
                status: Status::from_core(frame.status(&ws.id)),
                since: since_words(frame.hidden.get(&ws.id).copied().unwrap_or(now), now).into(),
            })
            .collect();
        let frame = self.frame.clone();
        MenuTrigger::new("hidden-workspaces", SharedString::from(words.clone()))
            .tip(words)
            .aria_value(SharedString::from(count.to_string()))
            .h(theme::CONTROL)
            .flex_none()
            .flex()
            .items_center()
            .gap(px(4.))
            .px(px(8.))
            .rounded(theme::RADIUS_CONTROL)
            .text_size(theme::TEXT_SMALL)
            .text_color(theme::INK_3)
            .cursor_pointer()
            .hover(|s| s.bg(theme::HOVER).text_color(theme::INK_2))
            .child(svg().path("icons/eye-off.svg").flex_none().size(theme::ICON).text_color(theme::INK_3))
            .child(tabular(div()).child(count.to_string()))
            .menu(gpui_kit::Anchor::TopLeft, move |menu, _, _| {
                let mut menu = menu
                    .min_w(px(280.))
                    .max_h(px(420.))
                    .scrollable(true)
                    .item(enkidu_ui::pieces::menu_label("Hidden workspaces"));
                for (at, row) in rows.iter().enumerate() {
                    let (frame, id) = (frame.clone(), row.id.clone());
                    menu = menu.item(hidden_row(at, row.clone()).on_click(move |_, _, cx| {
                        frame.update(cx, |frame, cx| {
                            if frame.show_workspace(&id) {
                                cx.notify();
                            }
                        })
                    }));
                }
                menu
            })
            .into_any_element()
    }

    /// With the lists hidden: the other projects' workspaces, each as a small dot of its lamp, in the rail's order,
    /// a thin line between two projects. The one that needs you is ringed in red. A click goes there. None while
    /// no other project has a workspace.
    fn lamp_dots(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let frame = self.frame.read(cx);
        let on_screen = frame.current_project.clone();
        let mut dots: Vec<AnyElement> = Vec::new();
        let (mut n, mut groups) = (0usize, 0usize);
        for project in frame.projects.iter().filter(|p| Some(&p.id) != on_screen.as_ref()) {
            let tabs = frame.tabs_of(&project.id);
            if tabs.is_empty() {
                continue;
            }
            if groups > 0 {
                dots.push(div().flex_none().w(px(1.)).h(px(12.)).bg(theme::LINE_STRONG).into_any_element());
            }
            groups += 1;
            for ws in tabs {
                let status = Status::from_core(frame.status(&ws.id));
                let color = status.color();
                let light = div().flex_none().size(px(8.)).rounded_full().map(|light| match status {
                    Status::Idle => border_width(light, 1.5).border_color(theme::INK_3),
                    Status::NeedsYou => light.bg(color).shadow(vec![BoxShadow {
                        color: gpui_kit::Hsla::from(alpha(color, 0.22)),
                        offset: point(px(0.), px(0.)),
                        blur_radius: px(0.),
                        spread_radius: px(3.),
                        inset: false,
                    }]),
                    _ => light.bg(color),
                });
                let id = ws.id.clone();
                let says = format!("{}, {}: {}", project.name, ws.name, status.label().to_lowercase());
                dots.push(
                    div()
                        .id(("lamp-dot", n))
                        .test_support()
                        .role(Role::Button)
                        .aria_label(SharedString::from(ws.name.clone()))
                        .aria_description(status.label())
                        .aria_value(SharedString::from(project.name.clone()))
                        .tipped(says, None)
                        .w(px(8.))
                        .h_full()
                        .flex()
                        .flex_none()
                        .items_center()
                        .cursor_pointer()
                        .child(light)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.frame.update(cx, |frame, cx| {
                                if frame.select_workspace(&id) {
                                    cx.notify();
                                }
                            })
                        }))
                        .into_any_element(),
                );
                n += 1;
            }
        }
        (n > 0).then(|| {
            div()
                .id("lamp-dots")
                .test_support()
                .aria_label("The other projects' workspaces")
                .h_full()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(8.))
                .child(rule().mr(px(4.)))
                .children(dots)
                .into_any_element()
        })
    }

    /// Live for the workspace on screen (DESIGN.md, "Live"): a play button at one width in all its states. A press
    /// takes this workspace Live, or stops it; while Live is switching nothing can be pressed. Its tooltip says what
    /// a press would do.
    fn live_button(&self, ws: &Workspace, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let frame = self.frame.read(cx);
        let state = frame.live_state(ws);
        let project = frame.projects.iter().find(|p| p.id == ws.project_id).map(|p| p.name.clone());
        let tip = SharedString::from(state.tip(&ws.name, project.as_deref(), frame.live_elsewhere(ws)));
        let (project_id, target) = (ws.project_id.clone(), (state != LiveState::Live).then(|| ws.id.clone()));
        let store = self.frame.clone();
        let (pointed, look) = (self.live_pointed, crate::live::look(state));
        enkidu_ui::live::live_button("live-button", look, state.words(pointed), state.name(), pointed, window, cx)
            .on_hover(cx.listener(|this, pointed: &bool, _, cx| {
                if this.live_pointed != *pointed {
                    this.live_pointed = *pointed;
                    cx.notify();
                }
            }))
            // (a long sentence wraps inside the tooltip)
            .tipped(tip.clone(), None)
            .on_click(move |_, window, cx| {
                if state.enabled() {
                    crate::live::switch(store.clone(), project_id.clone(), target.clone(), window, cx);
                }
            })
    }

    /// Where the changes go: beside the terminal, above it, or away (DESIGN.md, "Layout": "Three layouts, switched
    /// from the top row"). With the changes alone on screen (⇧⌘E) none of the three is chosen.
    fn layout_switch(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let chosen = self.ui.read(cx).chosen();
        let choice = |id: &'static str, title: &'static str, key: Box<dyn Action>, on: bool, pick: fn(&mut Ui)| {
            let icon: &'static str = match id {
                "layout-side" => "icons/layout-side.svg",
                "layout-stack" => "icons/layout-stack.svg",
                _ => "icons/layout-terminal.svg",
            };
            segment_sized(id, icon, 16., title, on).tipped(title, Some(key)).on_click(cx.listener(
                move |this, _, _, cx| {
                    this.ui.update(cx, |ui, cx| {
                        let was = ui.clone();
                        pick(ui);
                        if *ui != was {
                            cx.notify();
                        }
                    })
                },
            ))
        };
        let (beside, above) = (Box::new(crate::ToggleSideLayout), Box::new(crate::ToggleSideLayout));
        let segments = vec![
            choice("layout-side", "Changes beside the terminal", beside, chosen == Mode::Side, |ui| {
                ui.show(Layout::Side)
            }),
            choice("layout-stack", "Changes above the terminal", above, chosen == Mode::Stack, |ui| {
                ui.show(Layout::Stack)
            }),
            choice(
                "layout-terminal",
                "Terminal only",
                Box::new(crate::ToggleTerminalOnly),
                chosen == Mode::Terminal,
                Ui::terminal_only,
            ),
        ];
        let at = match chosen {
            Mode::Side => Some(0),
            Mode::Stack => Some(1),
            Mode::Terminal => Some(2),
            Mode::Review => None,
        };
        segmented("workspace-layout", at, segments, window, cx)
    }
}

/// The workspace on screen's menu, behind the … at the right of the row; its items are [`workspace_items`].
fn workspace_menu(frame: Entity<Frame>, ws: Workspace) -> impl IntoElement {
    MenuTrigger::new("ws-menu", "Workspace actions")
        .tip("Workspace actions")
        .size(theme::CONTROL)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(theme::RADIUS_CONTROL)
        .cursor_pointer()
        .hover(|s| s.bg(theme::HOVER))
        .child(svg().path("icons/ellipsis.svg").size(theme::ICON).text_color(theme::INK_3))
        .menu(gpui_kit::Anchor::TopRight, move |menu, _, _| workspace_items(menu, frame.clone(), ws.clone()))
}

/// A workspace's menu: its name, Show in Finder, Copy path, Copy branch name, Project settings, then Hide workspace and
/// Remove workspace…, which asks first. The … at the right of the top row opens it for the workspace on screen, a
/// right-click on a tab for that tab's workspace.
pub(crate) fn workspace_items(menu: PopupMenu, frame: Entity<Frame>, ws: Workspace) -> PopupMenu {
    let (path, branch, place) = (ws.path.clone(), ws.branch.clone(), short_path(&ws.path));
    let (hiding, hidden) = (frame.clone(), ws.id.clone());
    let (frame, removed) = (frame.clone(), ws.clone());
    menu.item(enkidu_ui::pieces::menu_label(ws.name.clone()))
        .item(menu_item("ws-show-in-finder", "icons/folder-open.svg", "Show in Finder", theme::INK).on_click({
            let path = path.clone();
            move |_, _, cx| cx.reveal_path(std::path::Path::new(&path))
        }))
        .item(menu_item("ws-copy-path", "icons/copy.svg", "Copy path", theme::INK).on_click({
            let path = path.clone();
            move |_, window, cx| {
                cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(path.clone()));
                crate::toast::note(window, "Path copied", place.clone(), cx);
            }
        }))
        .item(menu_item("ws-copy-branch", "icons/git-branch.svg", "Copy branch name", theme::INK).on_click(
            move |_, window, cx| {
                cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(branch.clone()));
                crate::toast::note(window, "Branch name copied", branch.clone(), cx);
            },
        ))
        .item(
            menu_item("ws-project-settings", "icons/sliders-horizontal.svg", "Project settings", theme::INK).on_click(
                {
                    let (frame, page) = (frame.clone(), enkidu_model::settings::Page::Project(ws.project_id.clone()));
                    move |_, window, cx| crate::settings::open_here(frame.clone(), page.clone(), window, cx)
                },
            ),
        )
        .separator()
        // put away: its tab goes, and nothing else of it (the button after the tabs brings it back)
        .item(menu_item("hide-workspace", "icons/eye-off.svg", "Hide workspace", theme::INK).on_click(
            move |_, _, cx| {
                hiding.update(cx, |frame, cx| {
                    if frame.hide_workspace(&hidden, now_ms()) {
                        cx.notify();
                    }
                })
            },
        ))
        .item(
            menu_item("remove-workspace", "icons/trash.svg", "Remove workspace…", theme::LAMP_RED).on_click(
                move |_, window, cx| crate::remove_workspace::open(frame.clone(), removed.clone(), window, cx),
            ),
        )
}

/// The button that brings the lists back while they are hidden: it stands first in the top row, after the window's
/// traffic lights.
pub(crate) fn show_lists_button(ui: Entity<Ui>) -> Pressed {
    icon_button("show-lists", "icons/panel-left.svg", "Show the lists")
        .tipped("Show the lists", Some(Box::new(crate::ToggleLists)))
        .on_click(move |_, _, cx| {
            ui.update(cx, |ui, cx| {
                ui.toggle_lists();
                cx.notify();
            })
        })
}

impl Render for TabsRow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::measure::TAB_ROWS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let frame = self.frame.read(cx).clone();
        let hidden_mode = self.ui.read(cx).lists_hidden;
        let tabs = frame.tabs();
        // the ones that are put away: behind their button, after the tabs
        let hidden = frame.hidden_tabs();
        for ws in &tabs {
            self.focus.entry(ws.id.clone()).or_insert_with(|| cx.focus_handle());
        }
        self.focus.retain(|id, _| tabs.iter().any(|w| &w.id == id));
        // a drag that ended, on a tab or anywhere else: nothing is held any more
        if !cx.has_active_drag() {
            (self.held, self.drop_at) = (None, None);
        }
        // the row starts after the window's traffic lights; a button's box stands 8 px before its words
        let row = div()
            .id("tabs-row")
            .test_support()
            .w_full()
            .h(theme::TOP_ROW)
            .flex_none()
            .flex()
            .items_center()
            .pl(theme::AFTER_LIGHTS - px(8.))
            .pr(px(12.))
            .text_size(theme::TEXT)
            // with the lists hidden: the button that brings them back
            .children(hidden_mode.then(|| div().flex_none().mr(px(4.)).child(show_lists_button(self.ui.clone()))));
        let (at, project) = match (frame.current_project.as_ref())
            .and_then(|id| frame.projects.iter().enumerate().find(|(_, project)| &project.id == id))
        {
            Some((at, project)) if frame.loaded => (at, project.clone()),
            // nothing until the database was read; with no project yet the screen under it has the one Add project
            _ => return row.into_any_element(),
        };
        let current = frame.current_workspace().cloned();
        // the main folder itself has no Live: Live copies a workspace's changes into that folder
        let can_go_live = current.as_ref().is_some_and(|ws| !frame.is_main_folder(ws));
        let selected = current.as_ref().and_then(|ws| tabs.iter().position(|tab| tab.id == ws.id));
        let views: Vec<Pressed> = (tabs.iter().enumerate())
            .map(|(ix, ws)| self.tab(ix, ws, &frame, Some(ix) == selected, window, cx))
            .collect();
        let hidden_button = (!hidden.is_empty()).then(|| self.hidden_button(&hidden, &frame));
        // while the screen under the row says "no workspaces yet", it has the one New workspace button
        let plus = !(tabs.is_empty() && hidden.is_empty());
        let (cut_left, cut_right) = self.cut();
        let strip = scrolling_tab_row("workspace-tabs", selected, views, &self.scroll, window, cx)
            .h_full()
            // a wheel scroll moves the fades at the cut edges
            .on_scroll(cx.listener(|_, _, _, cx| cx.notify()));
        let dots = if hidden_mode { self.lamp_dots(cx) } else { None };
        let new = plus.then(|| {
            let words = SharedString::from(format!("New workspace in {}", project.name));
            let frame = self.frame.clone();
            icon_button("new-workspace", "icons/plus.svg", words.clone())
                .tipped(words, Some(Box::new(crate::NewWorkspace)))
                .on_click(move |_, window, cx| crate::new_workspace::open(frame.clone(), window, cx))
        });
        row.child(crate::rail::project_menu(self.frame.clone(), at, &project, &frame, window))
            // (no line before nothing: a project without a workspace has only its name here)
            .children(plus.then(|| rule().id("tabs-rule").test_support().ml(px(8.)).mr(px(12.))))
            .child(
                div()
                    .id("tab-strip")
                    .test_support()
                    .role(Role::TabList)
                    .aria_label(SharedString::from(format!("Workspaces in {}", project.name)))
                    .relative()
                    .h_full()
                    .min_w_0()
                    .flex_shrink(1.)
                    .flex()
                    .child(strip)
                    .when(cut_left, |d| d.child(fade("tab-fade-left", true)))
                    .when(cut_right, |d| d.child(fade("tab-fade-right", false))),
            )
            .child(div().flex_none().ml(px(4.)).flex().items_center().children(hidden_button).children(new))
            .children(dots.map(|dots| div().flex_none().h_full().ml(px(8.)).child(dots)))
            .child(div().flex_1().min_w(px(12.)))
            // what acts on the workspace on screen: nothing of it while every workspace is put away
            .children(current.map(|ws| {
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(self.layout_switch(window, cx))
                    .children(can_go_live.then(|| self.live_button(&ws, window, cx)))
                    .child(workspace_menu(self.frame.clone(), ws))
            }))
            // a tab's menu, over everything, where the pointer was (kept 8 px inside the window)
            .children(self.menu.clone().map(|(menu, at)| {
                deferred(
                    anchored()
                        .position(at)
                        .snap_to_window_with_margin(px(8.))
                        .child(enkidu_ui::pieces::menu_frame(menu)),
                )
                .with_priority(gpui_kit::base::POPUP_PRIORITY)
            }))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ring of a tab that has the keyboard stands inside the tab, so the scrolling row can't cut it and nothing
    /// is painted under the tab.
    #[test]
    fn the_keyboards_ring_stands_inside_the_tab() {
        for on_screen in [true, false] {
            let rings = focus_ring(on_screen);
            assert!(rings.iter().all(|ring| ring.inset && ring.blur_radius == px(0.)), "{rings:?}");
            assert_eq!(rings.len(), if on_screen { 3 } else { 2 });
        }
        // a tab is never narrower than 112 px, and wider when what it says needs it
        assert_eq!(tab_min_width(NAME_MIN, px(0.)), px(112.));
        assert_eq!(tab_min_width(NAME_MIN, px(60.)), px(8. + 16. + 8. + 28. + 8. + 60. + 12.));
        // a short name is kept whole, a long one keeps 64 px of itself
        assert_eq!((name_min(px(10.)), name_min(px(40.)), name_min(px(200.))), (px(28.), px(40.), px(64.)));
        assert_eq!(tab_min_width(name_min(px(200.)), px(60.)), px(8. + 16. + 8. + 64. + 8. + 60. + 12.));
    }
}
