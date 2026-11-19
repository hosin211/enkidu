//! How a remark looks (item 3.5c): the card under its lines, and the composer where one is written. Blue is the
//! remarks' color everywhere: it is the color of what is yours (DESIGN.md, Principle 3).

use std::rc::Rc;

use enkidu_ui::pieces::{
    ButtonKind, ButtonSize, Observed, button, button_with, line_over, mono, rise_in_place, text_area,
};
use enkidu_ui::tips::Tipped as _;
use enkidu_ui::tokens::{self as theme, alpha, hsla};
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::input::{Escape, Textarea, TextareaState};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, BoxShadow, Entity, Focusable as _, FontWeight, Rgba, SharedString, Window, div, point, px, svg,
};

/// What a press does, from inside a row of the diff.
pub type Act = Rc<dyn Fn(&mut Window, &mut App)>;

/// A remark's card: where it is, what it says, and what became of its code.
pub struct CardView {
    pub id: String,
    /// "Line 4", "Lines 4–6": where its code is now.
    pub label: String,
    /// It is on lines the change removed.
    pub removed: bool,
    /// One line or more (for "removed line" or "removed lines").
    pub many: bool,
    /// Its code is no longer in the file; the code it was written on is shown with it.
    pub outdated: bool,
    pub snippet: String,
    pub body: String,
}

/// "line 4" as a card's first words say it: "Line 4".
pub fn capitalized(words: &str) -> String {
    let mut letters = words.chars();
    match letters.next() {
        Some(first) => first.to_uppercase().chain(letters).collect(),
        None => String::new(),
    }
}

/// A small square button holding an icon, on a card.
fn card_button(id: &'static str, icon: &'static str, label: &'static str, hot: Rgba) -> Observed {
    div()
        .id(id)
        .test_support()
        .role(gpui_kit::Role::Button)
        .aria_label(label)
        .size(theme::CHIP)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(theme::RADIUS_TAG)
        .cursor_pointer()
        .group("card-button")
        .hover(|s| s.bg(theme::HOVER))
        // a press here is the button's, not the start of a text selection in the code around the card
        .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| gpui_kit::base::GlobalState::suppress_text_selection(cx))
        .child(
            svg()
                .path(icon)
                .size(theme::MARK)
                .text_color(theme::INK_3)
                .group_hover("card-button", move |s| s.text_color(hot)),
        )
}

/// One remark under its lines, in the diff or in an opened file.
pub fn card(view: &CardView, edit: Act, delete: Act) -> AnyElement {
    let said = div()
        .h(theme::CHIP)
        .flex()
        .items_center()
        .gap(px(8.))
        .text_size(theme::TEXT_SMALL)
        .text_color(theme::INK_3)
        .child(div().font_weight(FontWeight::MEDIUM).text_color(theme::INK_2).child(view.label.clone()))
        .when(view.removed, |row| row.child(if view.many { "removed lines" } else { "removed line" }))
        .when(view.outdated, |row| {
            row.child(
                div()
                    .id("remark-outdated")
                    .test_support()
                    .aria_label("Code changed since")
                    .h(theme::CHIP)
                    .flex()
                    .items_center()
                    .px(px(8.))
                    .rounded_full()
                    .bg(alpha(theme::LAMP_AMBER, 0.15))
                    .text_size(theme::TEXT_STATUS)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::LAMP_AMBER)
                    .child("Code changed since"),
            )
        })
        .child(div().flex_1())
        .child(
            // the card's own controls show while the pointer is on the card
            div()
                .flex()
                .gap(px(4.))
                .opacity(0.)
                .group_hover("remark", |s| s.opacity(1.))
                .child(
                    card_button("edit-remark", "icons/pencil.svg", "Edit remark", theme::INK)
                        .on_click(move |_, window, cx| edit(window, cx)),
                )
                .child(
                    card_button("delete-remark", "icons/trash.svg", "Delete remark", theme::LAMP_RED)
                        .on_click(move |_, window, cx| delete(window, cx)),
                ),
        );
    div()
        .id(SharedString::from(format!("remark-{}", view.id)))
        .test_support()
        .aria_label(SharedString::from(view.body.clone()))
        .group("remark")
        .relative()
        .max_w(px(680.))
        .rounded(theme::RADIUS_CARD)
        .bg(theme::SHEET)
        .border_1()
        .border_color(theme::LINE)
        .py(px(8.))
        .px(px(12.))
        // the remarks' blue, down its left edge
        .child(div().absolute().top(px(8.)).bottom(px(8.)).left_0().w(px(2.)).rounded_full().bg(theme::ACCENT))
        .child(said)
        .when(view.outdated, |card| {
            card.child(
                mono(div())
                    .mt(px(4.))
                    .max_h(px(48.))
                    .overflow_hidden()
                    .text_size(px(11.))
                    .line_height(px(16.))
                    .text_color(theme::INK_3)
                    .child(view.snippet.clone()),
            )
        })
        .child(
            div()
                .mt(px(4.))
                .text_size(theme::TEXT)
                .line_height(theme::TEXT_LINE)
                .text_color(theme::INK)
                // (its words can be selected and copied)
                .child(enkidu_ui::selectable::words(
                    SharedString::from(format!("remark-words-{}", view.id)),
                    view.body.clone(),
                )),
        )
        .into_any_element()
}

/// Where a remark is written.
pub struct ComposerView {
    /// "Line 4", "Lines 4–6".
    pub label: String,
    pub input: Entity<TextareaState>,
    /// "Add remark" for a new one, "Save" for one whose words are changed.
    pub save: &'static str,
    /// It was sent to the database and the answer isn't back.
    pub busy: bool,
}

/// A ring of one color around a box, `width` wide.
fn ring(color: Rgba, width: f32) -> BoxShadow {
    BoxShadow {
        color: hsla(color),
        offset: point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: px(width),
        inset: false,
    }
}

/// The composer, under the lines the remark is about. ⌘↩ saves (the island hears the text field), Escape cancels.
pub fn composer(view: &ComposerView, save: Act, cancel: Act, cx: &App) -> AnyElement {
    let ready = !view.busy && !view.input.read(cx).value().trim().is_empty();
    let (on_escape, on_cancel) = (cancel.clone(), cancel);
    let field = view.input.clone();
    let add = button("save-remark", view.save, ButtonKind::Primary, ButtonSize::Small);
    let composer = div()
        .id("remark-composer")
        .test_support()
        .aria_label(SharedString::from(view.label.clone()))
        .max_w(px(680.))
        .rounded(theme::RADIUS_CARD)
        .bg(theme::SHEET)
        .px(px(8.))
        .py(px(12.))
        // a blue edge and a faint glow: the keyboard is here
        .shadow(vec![ring(theme::ACCENT, 1.), ring(alpha(theme::ACCENT, 0.16), 4.)])
        .on_action(move |_: &Escape, window, cx| on_escape(window, cx))
        // a press anywhere on the composer keeps the keyboard in its text field (a press outside a text field would
        // otherwise give it to the diff around it)
        .on_mouse_down(gpui_kit::MouseButton::Left, move |_, window, cx| {
            gpui_kit::base::GlobalState::suppress_text_selection(cx);
            window.prevent_default();
            field.update(cx, |field, cx| field.focus(window, cx));
        })
        .child(
            // 4 px in: where the text field's own words start
            div()
                .px(px(4.))
                .pb(px(8.))
                .text_size(theme::TEXT_SMALL)
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::INK_2)
                .child(view.label.clone()),
        )
        .child(text_area(&view.input, false).xsmall().appearance(false).aria_label("What should Claude do here?"))
        .child(
            div()
                .mt(px(8.))
                .px(px(4.))
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .flex_1()
                        .text_size(theme::TEXT_SMALL)
                        .text_color(theme::INK_3)
                        .child("⌘↩ to save, Esc to cancel"),
                )
                .child(
                    button("cancel-remark", "Cancel", ButtonKind::Ghost, ButtonSize::Small)
                        .on_click(move |_, window, cx| on_cancel(window, cx)),
                )
                .child(match ready {
                    true => add.on_click(move |_, window, cx| save(window, cx)),
                    // nothing to save yet, or it is being saved: it neither looks nor acts pressable
                    false => add.opacity(0.45).cursor_default(),
                }),
        );
    // it rises into place as it comes on screen (DESIGN.md, "Motion")
    rise_in_place("composer-rise", composer)
}

/// The bar at the foot of the changes while remarks wait to be sent (item 3.6).
pub struct BarView {
    /// "3 remarks on 2 files not sent yet".
    pub waiting: String,
    pub count: usize,
    /// "Add note" was pressed: a note for Claude goes with the remarks.
    pub note_open: bool,
    /// Where that note is typed.
    pub note: Option<Entity<TextareaState>>,
    /// "Discard" was pressed: Keep them, or Delete them.
    pub confirming: bool,
    /// They are on their way into Claude's prompt.
    pub sending: bool,
}

/// What the bar's buttons do.
pub struct BarActs {
    pub add_note: Act,
    /// Escape in a note without words.
    pub close_note: Act,
    pub discard: Act,
    pub keep: Act,
    pub delete: Act,
    pub send: Act,
}

/// The bar: how many remarks wait, then Add note, Discard, and the blue button that types them into Claude's
/// prompt. Faintly blue, the remarks' color, so it reads as theirs.
pub fn bar(view: &BarView, acts: BarActs, window: &Window, cx: &App) -> AnyElement {
    let BarActs { add_note, close_note, discard, keep, delete, send } = acts;
    let medium = |id: &'static str, label: String, kind: ButtonKind| button(id, label, kind, ButtonSize::Small);
    let note = view.note.as_ref().map(|note| {
        let state = note.read(cx);
        let (lit, empty) = (state.focus_handle(cx).is_focused(window), state.value().is_empty());
        div()
            .id("remarks-note")
            .test_support()
            .mb(px(8.))
            .rounded(theme::RADIUS_CONTROL)
            .bg(theme::WELL)
            .shadow(vec![ring(if lit { theme::ACCENT } else { theme::LINE_STRONG }, 1.)])
            // Escape closes a note without words; one with words stays
            .on_action(move |_: &Escape, window, cx| {
                if empty {
                    close_note(window, cx);
                }
            })
            .child(Textarea::new(note).appearance(false).aria_label("A note for Claude that goes with the remarks"))
    });
    let said = div()
        .flex_1()
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(8.))
        .text_size(theme::TEXT_SMALL)
        .text_color(theme::INK_2)
        .child(div().flex_none().size(px(8.)).rounded_full().bg(theme::ACCENT))
        .child(div().min_w_0().truncate().child(view.waiting.clone()));
    let buttons: Vec<AnyElement> = if view.confirming {
        vec![
            medium("keep-remarks", "Keep".into(), ButtonKind::Ghost)
                .on_click(move |_, window, cx| keep(window, cx))
                .into_any_element(),
            medium("confirm-discard", format!("Delete {}", view.count), ButtonKind::Danger)
                .on_click(move |_, window, cx| delete(window, cx))
                .into_any_element(),
        ]
    } else {
        let label =
            if view.sending { "Typing\u{2026}".to_string() } else { enkidu_model::remarks::send_words(view.count) };
        let type_them =
            button_with("send-remarks", Some("icons/send.svg"), label, None, ButtonKind::Accent, ButtonSize::Small)
                .tipped_in(
                    "Types the remarks into Claude's prompt; you press Enter there to send them",
                    Box::new(crate::TypeRemarks),
                    crate::changes_island::KEYS,
                );
        let add = (!view.note_open).then(|| {
            medium("add-note", "Add note".into(), ButtonKind::Ghost)
                .on_click(move |_, window, cx| add_note(window, cx))
                .into_any_element()
        });
        add.into_iter()
            .chain([
                medium("discard-remarks", "Discard".into(), ButtonKind::Ghost)
                    .on_click(move |_, window, cx| discard(window, cx))
                    .into_any_element(),
                match view.sending {
                    false => type_them.on_click(move |_, window, cx| send(window, cx)).into_any_element(),
                    // on their way: it neither looks nor acts pressable
                    true => type_them.opacity(0.6).cursor_default().into_any_element(),
                },
            ])
            .collect()
    };
    div()
        .id("review-bar")
        .test_support()
        .aria_label(SharedString::from(view.waiting.clone()))
        .relative()
        .flex_none()
        .pl(theme::INSET)
        .pr(px(12.))
        .py(px(8.))
        .child(line_over())
        .children(note)
        .child(div().flex().items_center().gap(px(8.)).child(said).children(buttons))
        .into_any_element()
}
