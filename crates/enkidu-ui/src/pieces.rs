//! Our own pieces (DESIGN.md; item 1.11): the island, the buttons, chips and kbd, the tick for counts,
//! the project badge and the file icons. The lamp is in `lamp.rs`, the gliding tab pill in `tab_pill.rs`.
//!
//! Every piece that takes a click gets an id and an accessibility label, so tests find it. GPUI can't scale a div, so
//! where a scale would be the usual answer (a press to 0.97, a badge popping in) these pieces change fill or size
//! instead.

use std::sync::Arc;

use gpui_kit::base::ElementExt as _;
use gpui_kit::base::ObservedElement;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    AbsoluteLength, Animation, AnimationExt as _, App, BoxShadow, Div, ElementId, FontWeight, IntoElement, Rgba, Role,
    SharedString, Stateful, Window, div, linear_color_stop, linear_gradient, point, px, svg,
};

use crate::press::pressable;
use crate::tips::Tipped as _;
use crate::tokens::*;

/// A piece that takes a click: a div with an id, registered for tests and the accessibility tree.
pub type Observed = ObservedElement<Stateful<Div>>;

/// A piece that takes a click and answers the press: drawn 3 % smaller while the mouse is down on it (`press`).
pub type Pressed = ObservedElement<crate::press::Press<Stateful<Div>>>;

/// Code in our look: JetBrains Mono with its ligatures off, so `->` stays `->` (DESIGN.md, "Type"). Every mono text
/// outside the terminal (branch names, the diff, paths) goes through this; the terminal sets the same itself.
pub fn mono<E: Styled>(el: E) -> E {
    el.font_family(MONO_FONT).font_features(gpui_kit::FontFeatures::disable_ligatures())
}

/// A one-line text field as dialogs have it: 32 px tall with 13 px text (DESIGN.md: "32 (inputs in dialogs)"). The
/// library's own field follows the window's base size and comes out at 26 px.
pub fn text_field(
    state: &gpui_kit::Entity<gpui_kit::component::input::InputState>,
) -> gpui_kit::component::input::Input {
    let mut field = gpui_kit::component::input::Input::new(state).text_size(TEXT);
    // the field's own `h` is for fields of several lines only, and it hides the style's: the style is set itself
    field.style().size.height = Some(INPUT_HEIGHT.into());
    field
}

/// A field of several lines in a dialog. `code` sets it for a script, an environment or a list of paths: the code face
/// at 12 px on 18 px lines. Otherwise the window's face at 13 px.
pub fn text_area(
    state: &gpui_kit::Entity<gpui_kit::component::input::TextareaState>,
    code: bool,
) -> gpui_kit::component::input::Textarea {
    let area = gpui_kit::component::input::Textarea::new(state);
    if code {
        mono(area).text_size(MONO_TEXT).line_height(px(18.))
    } else {
        area.text_size(TEXT).line_height(TEXT_LINE)
    }
}

/// Words with code in them, the code between backticks: one text that wraps, the code in the code face with its
/// ligatures off, in `code`'s color.
pub fn with_code(
    words: &str,
    color: impl Into<gpui_kit::Hsla>,
    code: impl Into<gpui_kit::Hsla>,
) -> gpui_kit::StyledText {
    sentence(&[Part { words, color: color.into(), strong: false }], code)
}

/// One part of a [`sentence`]: its words (code between backticks), its color, and whether it is in a label's weight.
pub struct Part<'a> {
    pub words: &'a str,
    pub color: gpui_kit::Hsla,
    pub strong: bool,
}

/// A sentence of several parts, each in its own color, as one text that wraps ("Jira can't be read: twg isn't signed
/// in. Run `twg login --force`."): a strong part in a label's weight, code in the code face in `code`'s color.
pub fn sentence(parts: &[Part], code: impl Into<gpui_kit::Hsla>) -> gpui_kit::StyledText {
    let code = code.into();
    let mut said = String::new();
    let mut runs = Vec::new();
    for part in parts {
        for (ix, piece) in part.words.split('`').enumerate() {
            if piece.is_empty() {
                continue;
            }
            let is_code = ix % 2 == 1;
            said.push_str(piece);
            runs.push(gpui_kit::TextRun {
                len: piece.len(),
                font: match is_code {
                    true => gpui_kit::Font {
                        features: gpui_kit::FontFeatures::disable_ligatures(),
                        ..gpui_kit::font(MONO_FONT)
                    },
                    false if part.strong => gpui_kit::Font { weight: WEIGHT_LABEL, ..gpui_kit::font(UI_FONT) },
                    false => gpui_kit::font(UI_FONT),
                },
                color: if is_code { code } else { part.color },
                // code in a sentence is a chip (DESIGN.md): the code face on a faint fill
                background_color: is_code.then(|| HOVER.into()),
                underline: None,
                strikethrough: None,
            });
        }
    }
    gpui_kit::StyledText::new(said).with_runs(runs)
}

/// A border `width` pixels wide on every side (GPUI's own steps are whole pixels).
pub fn border_width<E: Styled>(mut el: E, width: f32) -> E {
    let w: AbsoluteLength = px(width).into();
    let edges = &mut el.style().border_widths;
    edges.top = Some(w);
    edges.right = Some(w);
    edges.bottom = Some(w);
    edges.left = Some(w);
    el
}

fn shadow(color: Rgba, y: f32, blur: f32, spread: f32) -> BoxShadow {
    BoxShadow {
        color: hsla(color),
        offset: point(px(0.), px(y)),
        blur_radius: px(blur),
        spread_radius: px(spread),
        inset: false,
    }
}

const BLACK: Rgba = Rgba { r: 0., g: 0., b: 0., a: 1. };

/// An island: a pane one step lighter than the frame, with 12 px corners and a 1 px edge. The island the keyboard is
/// in gets a signal-blue edge and a faint glow (DESIGN.md, Principles 1 and 2). Give it [`lit_edge`] as its last
/// child.
pub fn island(id: impl Into<ElementId>, focused: bool) -> Observed {
    div()
        .id(id)
        .test_support()
        .relative()
        .flex()
        .flex_col()
        .overflow_hidden()
        .rounded(ISLAND_RADIUS)
        .bg(CANVAS)
        .border_1()
        .border_color(if focused { ISLAND_EDGE_FOCUS } else { ISLAND_EDGE })
        .shadow(if focused {
            vec![shadow(alpha(ACCENT, 0.10), 0., 12., 0.), shadow(alpha(BLACK, 0.35), 1., 3., 0.)]
        } else {
            vec![shadow(alpha(BLACK, 0.35), 1., 3., 0.)]
        })
}

/// An island's lit top edge, drawn over its content so a terminal can't cover it: the island's last child.
pub fn lit_edge() -> Div {
    div().absolute().top_0().left(px(10.)).right(px(10.)).h(px(1.)).bg(HIGHLIGHT)
}

/// A floating layer (a menu, a toast, a remark card): raised, its own edge and a soft shadow.
pub fn raised(id: impl Into<ElementId>) -> Observed {
    div()
        .id(id)
        .test_support()
        .relative()
        .rounded(ISLAND_RADIUS)
        .bg(RAISED)
        .border_1()
        .border_color(white(0.085))
        .shadow(vec![shadow(alpha(BLACK, 0.35), 2., 6., 0.), shadow(alpha(BLACK, 0.6), 12., 32., -6.)])
}

/// What a button does, which sets its look.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonKind {
    /// White, lit from above like a key.
    Primary,
    /// Signal blue: the button that types into Claude ("Open in Claude", "Type remarks into Claude").
    Accent,
    /// A hairline on a faint fill.
    Secondary,
    /// Words only, until you point at it.
    Ghost,
    /// Red, for what deletes.
    Danger,
}

/// A button's size: 24, 28 or 32 px high (DESIGN.md, "Space, size, shape").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonSize {
    Small,
    Medium,
    Large,
}

/// A button in our look. Every one answers a press: its fill steps down and it is drawn 3 % smaller (`press`).
pub fn button(id: impl Into<ElementId>, label: impl Into<SharedString>, kind: ButtonKind, size: ButtonSize) -> Pressed {
    button_with(id, None, label, None, kind, size)
}

/// A button with an icon before its words (Lucide, 14 px) and a key after them, dimmed ("New workspace ⌘N").
pub fn button_with(
    id: impl Into<ElementId>,
    icon: Option<&'static str>,
    label: impl Into<SharedString>,
    key: Option<&'static str>,
    kind: ButtonKind,
    size: ButtonSize,
) -> Pressed {
    button_drawn(id.into(), icon.map(|icon| (icon, None)), label.into(), key, kind, size)
}

/// A button whose icon keeps a color of its own (Claude's coral sparkle, a shell's green terminal).
pub fn button_tinted(
    id: impl Into<ElementId>,
    icon: &'static str,
    tint: Rgba,
    label: impl Into<SharedString>,
    kind: ButtonKind,
    size: ButtonSize,
) -> Pressed {
    button_drawn(id.into(), Some((icon, Some(tint))), label.into(), None, kind, size)
}

fn button_drawn(
    id: ElementId,
    icon: Option<(&'static str, Option<Rgba>)>,
    label: SharedString,
    key: Option<&'static str>,
    kind: ButtonKind,
    size: ButtonSize,
) -> Pressed {
    let fg = match kind {
        ButtonKind::Primary => ON_INK,
        ButtonKind::Accent | ButtonKind::Danger => white(1.0),
        ButtonKind::Secondary => INK,
        ButtonKind::Ghost => INK_2,
    };
    let (height, pad, text, radius) = match size {
        ButtonSize::Small => (SMALL_CONTROL_HEIGHT, px(8.), TEXT_SMALL, px(6.)),
        ButtonSize::Medium => (CONTROL_HEIGHT, px(10.), TEXT, RADIUS_CONTROL),
        ButtonSize::Large => (INPUT_HEIGHT, px(14.), TEXT, RADIUS_CONTROL),
    };
    let base = pressable(div().id(id))
        .test_support()
        .role(Role::Button)
        .aria_label(label.clone())
        .h(height)
        .px(pad)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .rounded(radius)
        .text_size(text)
        .font_weight(WEIGHT_LABEL)
        .cursor_pointer();
    let lit =
        |top: Rgba, bottom: Rgba| linear_gradient(180., linear_color_stop(top, 0.), linear_color_stop(bottom, 1.));
    let styled = match kind {
        ButtonKind::Primary => base
            .bg(lit(white(1.0), hex(0xe2e5ec)))
            .text_color(ON_INK)
            .shadow(vec![shadow(white(0.18), 0., 0., 1.), shadow(alpha(BLACK, 0.45), 1., 2., 0.)])
            .hover(|s| s.bg(white(1.0)))
            .active(|s| s.bg(hex(0xd8dbe2))),
        ButtonKind::Accent => base
            .bg(ACCENT_STRONG)
            .text_color(white(1.0))
            .shadow(vec![shadow(alpha(ACCENT, 0.35), 0., 0., 1.), shadow(alpha(BLACK, 0.4), 1., 2., 0.)])
            .hover(|s| s.bg(hex(0x5c88ff)))
            .active(|s| s.bg(hex(0x4472f0))),
        ButtonKind::Secondary => border_width(base.bg(white(0.055)).border_color(RULE_STRONG), 1.)
            .text_color(INK)
            .hover(|s| s.bg(white(0.09)))
            .active(|s| s.bg(white(0.12))),
        ButtonKind::Ghost => base.text_color(INK_2).hover(|s| s.bg(HOVER).text_color(INK)).active(|s| s.bg(SOFT)),
        ButtonKind::Danger => base
            .bg(LAMP_RED)
            .text_color(white(1.0))
            .shadow(vec![shadow(alpha(BLACK, 0.4), 1., 2., 0.)])
            .hover(|s| s.bg(hex(0xff7378)))
            .active(|s| s.bg(hex(0xe85a60))),
    };
    styled
        .children(icon.map(|(icon, tint)| svg().path(icon).flex_none().size(px(14.)).text_color(tint.unwrap_or(fg))))
        .child(label)
        .children(key.map(|key| div().ml(px(4.)).text_color(alpha(fg, 0.6)).child(key)))
}

/// A square button holding one icon, labelled for tests and the accessibility tree (26 px, as in the top rows).
pub fn icon_button(id: impl Into<ElementId>, icon: &'static str, label: impl Into<SharedString>) -> Pressed {
    pressable(div().id(id))
        .test_support()
        .role(Role::Button)
        .aria_label(label.into())
        .size(PROJECT_TAB_HEIGHT)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(RADIUS_CONTROL)
        .cursor_pointer()
        .hover(|s| s.bg(HOVER))
        .active(|s| s.bg(SOFT))
        .child(svg().path(icon).size(px(15.)).text_color(INK_3))
}

/// One item of our menus: a 14 px icon, then the words, 8 px apart, in `color` (INK, or LAMP_RED for what removes
/// something, INK_3 while it can't be chosen). Its `id` and words let a test find and click it.
pub fn menu_item(
    id: &'static str,
    icon: &'static str,
    words: impl Into<SharedString>,
    color: Rgba,
) -> gpui_kit::component::menu::PopupMenuItem {
    let words: SharedString = words.into();
    gpui_kit::component::menu::PopupMenuItem::element(move |_, _| {
        div()
            .id(id)
            .test_support()
            .aria_label(words.clone())
            .flex()
            .items_center()
            .gap(px(8.))
            .text_color(color)
            .child(svg().path(icon).flex_none().size(px(14.)).text_color(color))
            .child(words.clone())
    })
}

/// A menu item whose icon keeps a color of its own, its words in INK ("New shell", "Start Claude").
pub fn menu_item_tinted(
    id: &'static str,
    icon: &'static str,
    tint: Rgba,
    words: impl Into<SharedString>,
) -> gpui_kit::component::menu::PopupMenuItem {
    let words: SharedString = words.into();
    gpui_kit::component::menu::PopupMenuItem::element(move |_, _| {
        div()
            .id(id)
            .test_support()
            .aria_label(words.clone())
            .flex()
            .items_center()
            .gap(px(8.))
            .text_color(INK)
            .child(svg().path(icon).flex_none().size(px(14.)).text_color(tint))
            .child(words.clone())
    })
}

/// A menu item with its key on the right ("New workspace" and ⌘N), drawn as [`menu_item`].
pub fn menu_item_with_key(
    id: &'static str,
    icon: &'static str,
    words: impl Into<SharedString>,
    key: &'static str,
    color: Rgba,
) -> gpui_kit::component::menu::PopupMenuItem {
    let words: SharedString = words.into();
    gpui_kit::component::menu::PopupMenuItem::element(move |_, _| {
        div()
            .id(id)
            .test_support()
            .aria_label(words.clone())
            .aria_keyshortcuts(key)
            .w_full()
            .flex()
            .items_center()
            .gap(px(8.))
            .text_color(color)
            .child(svg().path(icon).flex_none().size(px(14.)).text_color(color))
            .child(div().flex_1().child(words.clone()))
            .child(div().flex_none().text_size(TEXT_SMALL).text_color(INK_3).child(key))
    })
}

/// A piece that opens a menu when clicked (a project's tab, a workspace's actions). gpui-kit's dropdown menu needs its
/// trigger to know whether its menu is open; open, it takes the pressed fill.
#[derive(IntoElement)]
pub struct MenuTrigger {
    base: Observed,
    open: bool,
    /// What its tooltip says, while its menu is closed.
    tip: Option<SharedString>,
}

impl MenuTrigger {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self { base: div().id(id).test_support().role(Role::Button).aria_label(label.into()), open: false, tip: None }
    }

    /// A tooltip, shown while the menu is closed.
    pub fn tip(mut self, words: impl Into<SharedString>) -> Self {
        self.tip = Some(words.into());
        self
    }
}

impl gpui_kit::base::Selectable for MenuTrigger {
    fn selected(mut self, selected: bool) -> Self {
        self.open = selected;
        self
    }

    fn is_selected(&self) -> bool {
        self.open
    }
}

impl ParentElement for MenuTrigger {
    fn extend(&mut self, elements: impl IntoIterator<Item = gpui_kit::AnyElement>) {
        self.base.extend(elements);
    }
}

impl Styled for MenuTrigger {
    fn style(&mut self) -> &mut gpui_kit::StyleRefinement {
        self.base.style()
    }
}

impl InteractiveElement for MenuTrigger {
    fn interactivity(&mut self) -> &mut gpui_kit::Interactivity {
        self.base.interactivity()
    }
}

// it has an id, so it can say more of itself (a value, a description) as any named piece does
impl gpui_kit::StatefulInteractiveElement for MenuTrigger {}

/// How far a menu slides out of its trigger as it opens, in px.
pub const MENU_SLIDE: f32 = 6.;

impl MenuTrigger {
    /// Opens a menu from this trigger, hung from its `anchor` corner. The menu grows out of its trigger (DESIGN.md,
    /// "Motion": "Menus, tooltips: grow from their trigger, 160 ms"): GPUI can't scale a subtree, so it fades in
    /// while it slides the last 6 px from the trigger's edge. gpui-kit's own dropdown menu opens with no motion and
    /// has no hook for one; this is its wiring (a popover holding a `PopupMenu` that is built when it opens and
    /// closes it when dismissed) with the motion around the menu.
    pub fn menu(
        mut self,
        anchor: gpui_kit::Anchor,
        build: impl Fn(
            gpui_kit::component::menu::PopupMenu,
            &mut Window,
            &mut gpui_kit::Context<gpui_kit::component::menu::PopupMenu>,
        ) -> gpui_kit::component::menu::PopupMenu
        + 'static,
    ) -> MenuFrom {
        let id = self.base.interactivity().element_id.clone().unwrap_or(ElementId::Integer(0));
        MenuFrom { id, anchor, trigger: self, build: std::rc::Rc::new(build) }
    }
}

type MenuBuild = std::rc::Rc<
    dyn Fn(
        gpui_kit::component::menu::PopupMenu,
        &mut Window,
        &mut gpui_kit::Context<gpui_kit::component::menu::PopupMenu>,
    ) -> gpui_kit::component::menu::PopupMenu,
>;

/// A trigger with the menu it opens (see [`MenuTrigger::menu`]).
#[derive(IntoElement)]
pub struct MenuFrom {
    id: ElementId,
    anchor: gpui_kit::Anchor,
    trigger: MenuTrigger,
    build: MenuBuild,
}

/// How long a menu takes to go (DESIGN.md, "Motion": "Menus, tooltips: grow from their trigger, 160 ms; out in 110").
pub const MENU_LEAVE: std::time::Duration = std::time::Duration::from_millis(110);

/// The menu of a trigger while it is open: built once as it opens, forgotten when it is dismissed, so the next
/// opening builds its items afresh.
#[derive(Default)]
struct OpenMenu {
    menu: Option<gpui_kit::Entity<gpui_kit::component::menu::PopupMenu>>,
    /// Where the open menu was last drawn.
    at: std::rc::Rc<std::cell::Cell<gpui_kit::Bounds<gpui_kit::Pixels>>>,
    /// The menu that was just dismissed, fading out where it stood.
    leaving: Option<(gpui_kit::Entity<gpui_kit::component::menu::PopupMenu>, gpui_kit::Bounds<gpui_kit::Pixels>)>,
    /// Counts dismissals, so each one's fade is its own and an old timer ends nothing new.
    turn: usize,
}

impl OpenMenu {
    /// The menu was dismissed: the popover is closed at once (the keyboard goes back where it was), and the menu
    /// itself is drawn for 110 ms more, fading, with nothing in it to press.
    fn dismissed(&mut self, cx: &mut gpui_kit::Context<Self>) {
        let Some(menu) = self.menu.take() else { return };
        if cx.reduce_motion() {
            return;
        }
        self.turn += 1;
        let turn = self.turn;
        self.leaving = Some((menu, self.at.get()));
        cx.notify();
        cx.spawn(async move |open, cx| {
            cx.background_executor().timer(MENU_LEAVE).await;
            let _ = open.update(cx, |open, cx| {
                if open.turn == turn {
                    open.leaving = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

impl gpui_kit::RenderOnce for MenuFrom {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        use gpui_kit::Focusable as _;
        use gpui_kit::component::menu::PopupMenu;
        let named = |what: &str| ElementId::Name(SharedString::from(format!("{what}:{:?}", self.id)));
        let open = window.use_keyed_state(named("menu"), cx, |_, _| OpenMenu::default());
        let (build, moving) = (self.build, named("menu-in"));
        // a menu that was just dismissed fades out where it stood, above everything, behind a cover that takes
        // the mouse: nothing in it can be pressed any more
        let trigger = match open.read(cx).leaving.clone() {
            None => self.trigger,
            Some((menu, at)) => {
                let fading =
                    ElementId::Name(SharedString::from(format!("menu-out:{:?}:{}", self.id, open.read(cx).turn)));
                self.trigger.child(
                    gpui_kit::deferred(
                        gpui_kit::anchored().position(at.origin).child(
                            div()
                                .relative()
                                .w(at.size.width)
                                .child(menu)
                                .child(div().absolute().inset_0().occlude())
                                .with_animation(fading, Animation::new(MENU_LEAVE), |menu, t| menu.opacity(1. - t)),
                        ),
                    )
                    .with_priority(gpui_kit::base::POPUP_PRIORITY),
                )
            }
        };
        gpui_kit::component::popover::Popover::new(named("menu-popover"))
            .appearance(false)
            .overlay_closable(false)
            .trigger(trigger)
            .anchor(self.anchor)
            .content(move |_, window, cx| {
                let menu = match open.read(cx).menu.clone() {
                    Some(menu) => menu,
                    None => {
                        let build = build.clone();
                        let menu = PopupMenu::build(window, cx, move |menu, window, cx| build(menu, window, cx));
                        open.update(cx, |open, _| open.menu = Some(menu.clone()));
                        menu.focus_handle(cx).focus(window, cx);
                        // dismissed (a pick, Escape, a click outside): the popover closes and the menu is forgotten.
                        // Weak handles: the listener lives as long as the menu, which `open` owns
                        let (popover, kept) = (cx.entity().downgrade(), open.downgrade());
                        window
                            .subscribe(&menu, cx, move |_, _: &gpui_kit::DismissEvent, window, cx| {
                                if let Some(popover) = popover.upgrade() {
                                    popover.update(cx, |state, cx| state.dismiss(window, cx));
                                }
                                let _ = kept.update(cx, |open, cx| open.dismissed(cx));
                            })
                            .detach();
                        menu
                    }
                };
                let at = open.read(cx).at.clone();
                div().relative().child(menu).on_prepaint(move |bounds, _, _| at.set(bounds)).with_animation(
                    moving.clone(),
                    Animation::new(DUR_QUICK).with_easing(|t| bezier(EASE_OUT, t)),
                    |menu, t| menu.top(px(-MENU_SLIDE * (1. - t))).opacity(t),
                )
            })
    }
}

impl gpui_kit::RenderOnce for MenuTrigger {
    fn render(self, _: &mut gpui_kit::Window, _: &mut gpui_kit::App) -> impl IntoElement {
        let open = self.open;
        self.base
            .when(open, |trigger| trigger.bg(THUMB))
            .when_some(self.tip.filter(|_| !open), |trigger, words| trigger.tipped(words, None))
    }
}

/// A small tag: a word on a tint of its color (a branch's state, "Draft", a ticket's type).
pub fn chip(text: impl Into<SharedString>, color: Rgba) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .h(px(18.))
        .px(px(6.))
        .rounded(RADIUS_TAG)
        .bg(alpha(color, 0.14))
        .text_color(color)
        .text_size(CAPTION)
        .font_weight(WEIGHT_LABEL)
        .child(text.into())
}

/// A key or shortcut as the keyboard writes it (`⌘K`), in a small hairline box.
pub fn kbd(keys: impl Into<SharedString>) -> Div {
    border_width(
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .h(px(18.))
            .min_w(px(18.))
            .px(px(4.))
            .rounded(RADIUS_TAG)
            .bg(white(0.04))
            .border_color(RULE_STRONG)
            .text_color(INK_2)
            .text_size(CAPTION)
            .child(keys.into()),
        1.,
    )
}

/// A switch, on or off: 30 by 18, a dim track with a white thumb when off, a bright track with a dark thumb when on.
/// The thumb glides across in 160 ms when it changes while you look at it; a page that opens shows it where it stands.
/// `id` and `label` name it, and a test reads `checked()`.
pub fn switch(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    on: bool,
    window: &mut Window,
    cx: &mut App,
) -> Observed {
    switch_toned(id.into(), label.into(), on, INK, window, cx)
}

/// The same switch in Live's green while on: for what turns Live on.
pub fn switch_live(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    on: bool,
    window: &mut Window,
    cx: &mut App,
) -> Observed {
    switch_toned(id.into(), label.into(), on, LAMP_GREEN, window, cx)
}

fn switch_toned(
    id: ElementId,
    label: SharedString,
    on: bool,
    track: Rgba,
    window: &mut Window,
    cx: &mut App,
) -> Observed {
    let part = |name: String| ElementId::NamedChild(Arc::new(id.clone()), SharedString::from(name));
    let changes = changes_seen(part("seen".into()), on as u64, window, cx);
    let (off_at, on_at) = (2., 14.);
    let (from, to) = if on { (off_at, on_at) } else { (on_at, off_at) };
    let thumb = div()
        .absolute()
        .top(px(2.))
        .size(px(14.))
        .rounded_full()
        .bg(if on { gpui_kit::rgb(0x000000) } else { gpui_kit::rgb(0xffffff) })
        .shadow(vec![gpui_kit::BoxShadow {
            color: alpha(gpui_kit::rgb(0x000000), 0.4).into(),
            offset: gpui_kit::point(px(0.), px(1.)),
            blur_radius: px(2.),
            spread_radius: px(0.),
            inset: false,
        }]);
    let thumb = match changes {
        0 => thumb.left(px(to)).into_any_element(),
        _ => thumb
            .with_animation(
                part(format!("glide-{changes}")),
                Animation::new(DUR_QUICK).with_easing(|t| bezier(EASE_OUT, t)),
                move |thumb, t| thumb.left(px(from + (to - from) * t)),
            )
            .into_any_element(),
    };
    div()
        .id(id)
        .test_support()
        .role(Role::Switch)
        .aria_label(label)
        .aria_toggled(if on { gpui_kit::Toggled::True } else { gpui_kit::Toggled::False })
        .relative()
        .flex_none()
        .w(px(30.))
        .h(px(18.))
        .rounded_full()
        .bg(if on { track } else { alpha(INK, 0.18) })
        .cursor_pointer()
        .child(thumb)
}

/// Numbers that change (counts, the +/− lines, times) in the tabular figures of the window's face, so a digit that
/// changes doesn't move the ones beside it (DESIGN.md, "Type"). What it holds inherits it.
pub fn tabular<E: Styled>(el: E) -> E {
    el.font_features(tabular_figures())
}

/// The face's feature for tabular figures, for who measures a number before it is drawn (a tab's narrowest width).
pub fn tabular_figures() -> gpui_kit::FontFeatures {
    gpui_kit::FontFeatures(Arc::new(vec![("tnum".into(), 1)]))
}

/// A value that ticks into place when it changes (a count, the +/− lines): it slides down 4 px and brightens in
/// 220 ms. `changes` counts the changes seen; 0 shows the value still, so opening a list or switching workspace
/// stays still and only a real change while you watch moves.
pub fn tick(id: impl Into<ElementId>, value: impl Into<SharedString>, changes: u32) -> gpui_kit::AnyElement {
    ticking(id, div().child(value.into()), changes)
}

/// The same for an element of your own (one a test can find, say): it is the thing that ticks.
pub fn ticking<E>(id: impl Into<ElementId>, what: E, changes: u32) -> gpui_kit::AnyElement
where
    E: Styled + IntoElement + 'static,
{
    // a number that changes keeps its width
    let text = tabular(what.relative());
    if changes == 0 {
        return text.into_any_element();
    }
    let id: ElementId = id.into();
    text.with_animation(
        ElementId::NamedChild(Arc::new(id), SharedString::from(format!("tick-{changes}"))),
        Animation::new(DUR_BASE).with_easing(|t| bezier(EASE_OUT, t)),
        |el, t| el.top(px(-4. * (1. - t))).opacity(0.15 + 0.85 * t),
    )
    .into_any_element()
}

/// How many times `value` changed while the thing at `key` was on screen, for [`tick`]: 0 until it first changes.
/// The count lives only while it is asked for frame after frame, so a list that opens, a row scrolled back into view
/// and another workspace's numbers (give each workspace its own key) all start still. Call it while drawing.
pub fn changes_seen(key: impl Into<ElementId>, value: u64, window: &mut Window, cx: &mut App) -> u32 {
    let seen = window.use_keyed_state(key, cx, |_, _| (value, 0u32));
    seen.update(cx, |seen, _| {
        if seen.0 != value {
            *seen = (value, seen.1 + 1);
        }
        seen.1
    })
}

/// Something that arrives rises into place: 6 px up and in from nothing, in 220 ms (DESIGN.md, "Motion": the changes
/// when Claude's first edit lands, a remark's box). `turn` counts the arrivals: 0 is still, so what was there when
/// the window opened doesn't move. With reduced motion it is in place at once.
pub fn rise(id: impl Into<ElementId>, turn: u32, what: impl IntoElement) -> gpui_kit::AnyElement {
    arrive(id, turn, div().size_full().child(what), |el, t| el.top(px(6. * (1. - t))))
}

/// The same for something that stands in a column and is as tall as what it holds (a remark's box): it rises each
/// time it comes on screen.
pub fn rise_in_place(id: impl Into<ElementId>, what: impl IntoElement) -> gpui_kit::AnyElement {
    arrive(id, 1, div().w_full().child(what), |el, t| el.top(px(6. * (1. - t))))
}

/// A panel that was opened slides in from the right edge: 14 px, in 220 ms. `turn` as for [`rise`].
pub fn slide_in(id: impl Into<ElementId>, turn: u32, what: impl IntoElement) -> gpui_kit::AnyElement {
    arrive(id, turn, div().size_full().child(what), |el, t| el.left(px(14. * (1. - t))))
}

fn arrive(
    id: impl Into<ElementId>,
    turn: u32,
    frame: Div,
    from: impl Fn(Div, f32) -> Div + 'static,
) -> gpui_kit::AnyElement {
    let frame = frame.relative();
    if turn == 0 {
        return frame.into_any_element();
    }
    let id: ElementId = id.into();
    frame
        .with_animation(
            ElementId::NamedChild(Arc::new(id), SharedString::from(format!("arrives-{turn}"))),
            Animation::new(DUR_BASE).with_easing(|t| bezier(EASE_OUT, t)),
            move |el, t| from(el, t).opacity(t),
        )
        .into_any_element()
}

/// The hues of projects, by their place in the top row: far apart, and no yellows (a white letter needs a hue that
/// stays deep at this lightness).
pub const PROJECT_HUES: [f32; 7] = [258., 155., 32., 305., 200., 345., 280.];

/// A project's hue: from its place in the top row, so two projects never share one.
pub fn project_hue(index: usize) -> f32 {
    PROJECT_HUES[index % PROJECT_HUES.len()]
}

/// The badge's letter: the name's first letter or digit, upper case.
pub fn badge_letter(name: &str) -> String {
    name.chars().find(|c| c.is_ascii_alphanumeric()).map_or("?".into(), |c| c.to_ascii_uppercase().to_string())
}

/// A project's badge: its first letter in white on its own color, lit from above like an app icon.
pub fn project_badge(name: &str, hue: f32, size: f32) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(size))
        .rounded(RADIUS_TAG)
        .bg(linear_gradient(
            180.,
            linear_color_stop(oklch(0.64, 0.15, hue), 0.),
            linear_color_stop(oklch(0.52, 0.15, hue), 1.),
        ))
        .shadow(vec![shadow(alpha(oklch(0.3, 0.08, hue), 0.6), 0., 0., 0.5)])
        .text_color(white(1.0))
        .text_size(px((size * 0.58).round()))
        .font_weight(FontWeight::SEMIBOLD)
        .child(badge_letter(name))
}

/// The badge of a project that is not the one on screen: its letter on a gray tile, so the top row keeps its color
/// for the project you are in (DESIGN.md, "States": the ranks of the top rows).
pub fn project_badge_quiet(name: &str, size: f32) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(size))
        .rounded(RADIUS_TAG)
        .bg(white(0.14))
        .text_color(INK_2)
        .text_size(px((size * 0.58).round()))
        .font_weight(FontWeight::SEMIBOLD)
        .child(badge_letter(name))
}

/// The bar at the left of the row of what is on screen (DESIGN.md, "States"): signal blue, gray while the window is
/// behind another app. It stands `inset` px in from the row's top and bottom; the row must be `relative()`.
pub fn on_screen_bar(inset: f32, window_in_front: bool) -> Div {
    div().absolute().top(px(inset)).bottom(px(inset)).left_0().w(SELECTED_BAR).rounded_full().bg(if window_in_front {
        ACCENT
    } else {
        BEHIND
    })
}

/// What a file is, from its name: its glyph and its color.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FileKind {
    /// The icon's path among the app's assets.
    pub icon: &'static str,
    pub color: Rgba,
}

const QUIET: Rgba = INK_3;

fn kind(icon: &'static str, hue: f32, chroma: f32) -> FileKind {
    FileKind { icon, color: tone(hue, chroma) }
}

fn code(hue: f32) -> FileKind {
    kind("icons/file-code.svg", hue, 0.13)
}

/// The kind of a file from its path: Python blue, JavaScript yellow, config violet, and so on.
pub fn file_kind(path: &str) -> FileKind {
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    let lock = FileKind { icon: "icons/file-lock.svg", color: QUIET };
    match name.as_str() {
        "dockerfile" | ".dockerignore" => return kind("icons/container.svg", 240., 0.13),
        "makefile" => return kind("icons/file-terminal.svg", 140., 0.07),
        ".gitignore" | ".gitattributes" | ".gitmodules" => return kind("icons/git-branch.svg", 35., 0.13),
        "package-lock.json" | "pnpm-lock.yaml" | "yarn.lock" | "poetry.lock" | "cargo.lock" => return lock,
        _ => {}
    }
    if name == ".env" || name.starts_with(".env.") {
        return kind("icons/file-key.svg", 95., 0.13);
    }
    if name.starts_with("docker-compose") || name.starts_with("compose.") {
        return kind("icons/container.svg", 240., 0.13);
    }
    let ext = match name.rfind('.') {
        Some(dot) if dot > 0 => &name[dot + 1..],
        _ => "",
    };
    match ext {
        "py" | "pyi" | "ipynb" => code(250.),
        "js" | "mjs" | "cjs" => code(95.),
        "jsx" | "tsx" => code(205.),
        "ts" | "mts" => code(235.),
        "vue" => code(160.),
        "svelte" | "swift" => code(40.),
        "html" | "htm" => code(50.),
        "xml" => code(55.),
        "css" => code(300.),
        "scss" | "sass" | "graphql" | "gql" => code(350.),
        "less" => code(270.),
        "rs" => kind("icons/file-code.svg", 45., 0.11),
        "go" => code(215.),
        "java" => code(35.),
        "kt" => code(295.),
        "rb" => code(25.),
        "php" => code(275.),
        "c" | "h" => kind("icons/file-code.svg", 250., 0.07),
        "cpp" | "hpp" => code(0.),
        "cs" => code(305.),
        "dart" => code(220.),
        "lua" => code(265.),
        "lock" => lock,
        "sql" => kind("icons/database.svg", 195., 0.13),
        "json" | "jsonc" => kind("icons/file-braces.svg", 70., 0.13),
        "md" | "mdx" | "rst" => kind("icons/file-text.svg", 250., 0.06),
        "yml" | "yaml" | "toml" => kind("icons/file-cog.svg", 285., 0.13),
        "ini" | "cfg" | "conf" => kind("icons/file-cog.svg", 285., 0.07),
        "sh" | "bash" | "zsh" | "fish" => kind("icons/file-terminal.svg", 140., 0.13),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "avif" | "ico" | "bmp" => kind("icons/file-image.svg", 310., 0.13),
        "svg" => kind("icons/file-image.svg", 80., 0.13),
        "zip" | "gz" | "tgz" | "tar" => kind("icons/file-archive.svg", 70., 0.08),
        "csv" | "tsv" | "xlsx" => kind("icons/file-spreadsheet.svg", 150., 0.13),
        "mp4" | "mov" | "webm" => kind("icons/file-video-camera.svg", 350., 0.13),
        "mp3" | "wav" => kind("icons/file-headphone.svg", 350., 0.13),
        _ => FileKind { icon: "icons/file.svg", color: QUIET },
    }
}

/// A file's type icon in its color, 14 px unless given.
pub fn file_icon(path: &str, size: f32) -> impl IntoElement {
    let kind = file_kind(path);
    svg().flex_none().path(kind.icon).size(px(size)).text_color(kind.color)
}

/// A folder in the Files tree: quieter than files, a cool steel.
pub fn folder_icon(open: bool, size: f32) -> impl IntoElement {
    let icon = if open { "icons/folder-open.svg" } else { "icons/folder.svg" };
    svg().flex_none().path(icon).size(px(size)).text_color(tone(250., 0.045))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_text_has_its_ligatures_off() {
        let mut text = mono(div());
        let style = text.style();
        assert_eq!(style.text.font_family.as_ref().map(|f| f.as_ref()), Some(MONO_FONT));
        assert_eq!(style.text.font_features.as_ref().and_then(|f| f.is_calt_enabled()), Some(false));
    }

    /// DESIGN.md, "Type": "Numbers that change (counts, +/−) use tabular figures of the UI face". A count that ticks
    /// gets them by itself.
    #[test]
    fn numbers_that_change_get_tabular_figures() {
        let tnum = |el: &mut Div| el.style().text.font_features.as_ref().map(|f| f.tag_value_list().to_vec());
        assert_eq!(tnum(&mut tabular(div())), Some(vec![("tnum".to_string(), 1)]));
        assert_eq!(tnum(&mut div()), None);
        // the window's face stays the window's: only the figures change
        assert_eq!(tabular(div()).style().text.font_family, None);
    }

    #[test]
    fn files_get_their_glyphs_and_colors() {
        assert_eq!(file_kind("src/orders/stock.py"), code(250.));
        assert_eq!(file_kind("web/App.vue").color, tone(160., 0.13));
        assert_eq!(file_kind("README.md"), kind("icons/file-text.svg", 250., 0.06));
        assert_eq!(file_kind("config/settings.TOML"), kind("icons/file-cog.svg", 285., 0.13), "case doesn't matter");
        assert_eq!(file_kind("Dockerfile").icon, "icons/container.svg");
        assert_eq!(file_kind("docker-compose.small.yml").icon, "icons/container.svg");
        assert_eq!(file_kind(".env.local").icon, "icons/file-key.svg");
        assert_eq!(file_kind("pnpm-lock.yaml").color, INK_3, "lock files stay quiet");
        assert_eq!(file_kind(".gitignore").icon, "icons/git-branch.svg");
        assert_eq!(file_kind("Makefile").color, tone(140., 0.07));
        assert_eq!(file_kind("notes").icon, "icons/file.svg");
        assert_eq!(file_kind(".bashrc").icon, "icons/file.svg", "a leading dot is no extension");
        assert_eq!(file_kind("a/b/photo.JPG").icon, "icons/file-image.svg");
    }

    #[test]
    fn projects_take_their_color_from_their_place_and_their_letter_from_their_name() {
        assert_eq!((project_hue(0), project_hue(1), project_hue(7), project_hue(8)), (258., 155., 258., 155.));
        assert_eq!(badge_letter("shop"), "S");
        assert_eq!(badge_letter("_internal-tools"), "I");
        assert_eq!(badge_letter("3d-viewer"), "3");
        assert_eq!(badge_letter("—"), "?");
    }

    #[test]
    fn every_icon_a_piece_names_is_carried() {
        use gpui_kit::AssetSource as _;
        let paths = [
            "x.py",
            "x.json",
            "x.md",
            "x.yml",
            "x.ini",
            "x.sh",
            "x.png",
            "x.zip",
            "x.csv",
            "x.mp4",
            "x.mp3",
            "x.sql",
            "Dockerfile",
            ".env",
            ".gitignore",
            "x.lock",
            "x",
        ];
        for path in paths.iter().map(|p| file_kind(p).icon).chain(["icons/folder.svg", "icons/folder-open.svg"]) {
            let bytes = crate::assets::Assets.load(path).ok().flatten().unwrap_or_default();
            assert!(bytes.starts_with(b"<svg"), "{path} is not carried");
        }
    }
}
