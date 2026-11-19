//! "Needs you" in the card's Overview (DESIGN.md, "The card in front of the board"): each thing that waits on you as a
//! sentence a person would say, the words quoted under it, a box that keeps it in the message to Claude or leaves it
//! out, a field for how Claude should handle it, and Open, Snooze and Done. In front of a card the keyboard is on one
//! of them: j and k (the arrows) move, x leaves it out or puts it back, n writes its instruction, e marks it done, z
//! snoozes it for a day, ↩ opens where it was said.

use enkidu_model::work::card::{ItemKind, ItemRow, current_item, step_item};
use enkidu_model::work::compose::Draft;
use enkidu_ui::tips::Tipped as _;
use enkidu_ui::tokens as theme;
use enkidu_ui::work::tinted;
use gpui_kit::base::{ElementExt as _, TestSupportExt as _};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::input::{InputEvent, Position, Textarea, TextareaState};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Bounds, Context, Focusable as _, Pixels, Role, SharedString, Window, div, px, svg};

use crate::work::card::{Card, ItemDone, ItemSnooze, OpenItem, ToggleItem, heading, open_in_browser};
use crate::work::drafts::Drafts;
use crate::work::store::{ItemState, Work};

/// What an instruction field asks.
pub const NOTE_ASKS: &str = "How should Claude handle this one? (optional)";

fn kind_icon(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::Remark | ItemKind::ProbablyFixed => "icons/message-square.svg",
        ItemKind::Question => "icons/message-circle-question-mark.svg",
        ItemKind::ChangesRequested => "icons/git-pull-request-arrow.svg",
        ItemKind::PipelineFailed => "icons/circle-x.svg",
        ItemKind::Conflict => "icons/triangle-alert.svg",
        ItemKind::QaReplied => "icons/reply.svg",
        ItemKind::Mentioned => "icons/at-sign.svg",
        ItemKind::BackToYou => "icons/undo-2.svg",
        ItemKind::Attachment => "icons/paperclip.svg",
    }
}

/// A quiet square button under an item: its words and its key in the tooltip.
fn square(
    id: SharedString,
    label: &'static str,
    icon: &'static str,
    key: Box<dyn gpui_kit::Action>,
) -> enkidu_ui::pieces::Pressed {
    enkidu_ui::press::pressable(div().id(id.clone()))
        .test_support()
        .role(Role::Button)
        .aria_label(label)
        .size(theme::CONTROL)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(theme::RADIUS_CONTROL)
        .border_1()
        .border_color(theme::LINE_STRONG)
        .cursor_pointer()
        .group(id.clone())
        .hover(|s| s.bg(theme::HOVER))
        .tipped(label, Some(key))
        .child(
            svg().path(icon).size(theme::ICON).text_color(theme::INK_3).group_hover(id, |s| s.text_color(theme::INK)),
        )
}

impl Card {
    /// What you wrote about this card.
    pub(super) fn draft(&self, cx: &gpui_kit::App) -> Draft {
        self.drafts.read(cx).of(&self.id)
    }

    /// The place of the item the keyboard is on.
    pub(super) fn current(&self) -> Option<usize> {
        let items = &self.made.items;
        let moved_to = self.item.as_ref().and_then(|id| items.iter().position(|item| &item.id == id));
        current_item(items.len(), moved_to, self.item_at)
    }

    /// How many things need you on the card.
    pub fn needs_count(&self) -> usize {
        self.made.items.len()
    }

    /// The item the keyboard is on.
    pub fn current_item(&self) -> Option<&ItemRow> {
        self.current().and_then(|at| self.made.items.get(at))
    }

    /// Before the card is drawn: the place of the item the keyboard is on is remembered (so the one that takes its
    /// place is next), and each item that goes with the message has its instruction field.
    pub(super) fn settle_needs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let items = self.made.items.clone();
        if let Some(at) = self.item.as_ref().and_then(|id| items.iter().position(|item| &item.id == id)) {
            self.item_at = at;
        }
        let draft = self.draft(cx);
        self.notes.retain(|id, _| items.iter().any(|item| &item.id == id) && draft.includes(id));
        for item in items.iter().filter(|item| draft.includes(&item.id)) {
            if self.notes.contains_key(&item.id) {
                continue;
            }
            let words = draft.note_for(&item.id).to_string();
            let field =
                cx.new(|cx| TextareaState::new(window, cx).auto_grow(1, 5).placeholder(NOTE_ASKS).default_value(words));
            let id = item.id.clone();
            let heard =
                cx.subscribe_in(&field, window, move |card, field, event: &InputEvent, window, cx| match event {
                    InputEvent::Change => {
                        let words = field.read(cx).value().to_string();
                        card.set_note(&id, &words, cx);
                    }
                    // ⌘↩ sends the card's message from here too (the line break it typed first is taken back)
                    InputEvent::PressEnter { secondary: true, .. } => {
                        let words = field.read(cx).value().to_string();
                        let kept = words.strip_suffix('\n').unwrap_or(&words).to_string();
                        field.update(cx, |field, cx| field.set_value(kept.clone(), window, cx));
                        card.set_note(&id, &kept, cx);
                        card.send(None, window, cx);
                    }
                    _ => {}
                });
            self.notes.insert(item.id.clone(), (field, heard));
        }
    }

    /// Your instruction for an item, as you type it: kept with the card's draft.
    fn set_note(&mut self, item: &str, words: &str, cx: &mut Context<Self>) {
        Drafts::change(&self.drafts, &self.id, cx, |draft| {
            let mut say = draft.items.remove(item).unwrap_or_default();
            say.note = Some(words.to_string()).filter(|words| !words.is_empty());
            if say != Default::default() {
                draft.items.insert(item.to_string(), say);
            }
        });
    }

    /// The box of an item: it goes with the message to Claude, or is left out of it.
    pub(super) fn toggle_include(&mut self, item: &str, cx: &mut Context<Self>) {
        Drafts::change(&self.drafts, &self.id, cx, |draft| {
            let out = !draft.includes(item);
            let mut say = draft.items.remove(item).unwrap_or_default();
            // left out: marked so; put back: as if never touched
            say.include = (!out).then_some(false);
            if say != Default::default() {
                draft.items.insert(item.to_string(), say);
            }
        });
    }

    /// The keyboard goes to an item (a click on it).
    pub(super) fn select_item(&mut self, item: &str, cx: &mut Context<Self>) {
        if self.item.as_deref() != Some(item) {
            self.item = Some(item.to_string());
            cx.notify();
        }
    }

    /// j or k: one item on or back, staying at the ends; it is brought into view.
    pub(super) fn step_item(&mut self, by: i32, cx: &mut Context<Self>) {
        let items = self.made.items.clone();
        if let Some(to) = step_item(items.len(), self.current(), by) {
            self.item = Some(items[to].id.clone());
            self.item_at = to;
            self.reveal = true;
            cx.notify();
        }
    }

    /// Does something with the item the keyboard is on.
    pub(super) fn act_on_item(&mut self, cx: &mut Context<Self>, act: fn(&mut Card, &str, &mut Context<Card>)) {
        if let Some(id) = self.current_item().map(|item| item.id.clone()) {
            act(self, &id, cx);
        }
    }

    /// n: the keyboard goes into the instruction field of the item it is on (one left out has none).
    pub(super) fn write_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let field = self.current_item().and_then(|item| self.notes.get(&item.id)).map(|(field, _)| field.clone());
        if let Some(field) = field {
            self.reveal = true;
            // the cursor goes after what you wrote before
            let words = field.read(cx).value().to_string();
            let end = Position {
                line: words.matches('\n').count() as u32,
                character: words.rsplit('\n').next().map_or(0, |last| last.encode_utf16().count() as u32),
            };
            field.update(cx, |field, cx| field.set_cursor_position(end, window, cx));
            cx.notify();
        }
    }

    /// Marks an item done, or snoozes it for a day. The page answers at once; the core keeps it.
    pub(super) fn set_item(&mut self, item: &str, state: ItemState, window: &mut Window, cx: &mut Context<Self>) {
        // the keyboard stays where it was: the item that takes this one's place is next
        if let Some(at) = self.made.items.iter().position(|row| row.id == item)
            && self.current() == Some(at)
        {
            self.item = None;
            self.item_at = at;
        }
        Work::item_set(&self.work, item, state, window.window_handle(), cx);
    }

    /// e or z: the item the keyboard is on.
    pub(super) fn set_current(&mut self, state: ItemState, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.current_item().map(|item| item.id.clone()) {
            self.set_item(&id, state, window, cx);
        }
    }

    /// ↩: where the item the keyboard is on was said, in the browser.
    pub(super) fn open_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(url) = self.current_item().and_then(|item| item.url.clone()) {
            open_in_browser(&url, window, cx);
        }
    }

    /// The item the keyboard moved to is drawn at `bounds`: the left side scrolls just enough to show it whole, with
    /// 8 px around it where there is the room. In a small window an item can be nearly as high as what is in view:
    /// then the room around it is what is left, and an item higher than the view shows its head.
    fn bring_into_view(&mut self, bounds: Bounds<Pixels>, cx: &mut Context<Self>) {
        if !std::mem::take(&mut self.reveal) {
            return;
        }
        let view = self.story_scroll.bounds();
        let mut offset = self.story_scroll.offset();
        let spare = view.size.height - bounds.size.height;
        let room = px(8.).min((spare / 2.).max(px(0.)));
        let (top, bottom) = (view.top() + room, view.bottom() - room);
        if bounds.top() < top || spare < px(0.) {
            offset.y += top - bounds.top();
        } else if bounds.bottom() > bottom {
            offset.y -= bounds.bottom() - bottom;
        } else {
            return;
        }
        self.story_scroll.set_offset(offset);
        cx.notify();
    }

    /// One thing that waits on you.
    fn item_card(
        &self,
        item: &ItemRow,
        selected: bool,
        draft: &Draft,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = &item.id;
        let named = |what: &str| SharedString::from(format!("{what}:{id}"));
        let include = draft.includes(id);
        let color = if item.kind.severe() { theme::LAMP_RED } else { theme::ACCENT };
        // the box that keeps it in the message, or leaves it out
        let (toggled, box_words, box_tip) = match include {
            true => (gpui_kit::Toggled::True, "In the message to Claude", "Leave it out of the message to Claude"),
            false => {
                (gpui_kit::Toggled::False, "Left out of the message to Claude", "Put it back in the message to Claude")
            }
        };
        let pick = id.clone();
        let include_box = div()
            .id(named("item-include"))
            .test_support()
            .role(Role::CheckBox)
            .aria_label(box_words)
            .aria_toggled(toggled)
            .mt(px(1.))
            .size(px(16.))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(theme::RADIUS_SMALL)
            .border_1()
            .cursor_pointer()
            .map(|mark| match include {
                // ticked: blue with a white sign, as every box that is ticked
                true => mark.border_color(theme::ACCENT_STRONG).bg(theme::ACCENT_STRONG).child(tinted(
                    "icons/check.svg",
                    theme::white(1.0),
                    12.,
                )),
                false => mark.border_color(theme::LINE_STRONG).hover(|s| s.border_color(theme::INK_3)),
            })
            .tipped(box_tip, Some(Box::new(ToggleItem)))
            .on_click(cx.listener(move |card, _, _, cx| {
                cx.stop_propagation();
                card.toggle_include(&pick, cx);
            }));
        let head = div()
            .flex()
            .items_start()
            .gap(px(8.))
            .child(tinted(kind_icon(item.kind), color, 16.).mt(px(1.)))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .text_size(px(13.))
                    .line_height(px(18.))
                    .font_weight(gpui_kit::FontWeight::MEDIUM)
                    .text_color(theme::INK)
                    .child(item.sentence.clone()),
            )
            .when(item.bot, |head| {
                head.child(
                    div()
                        .flex_none()
                        .rounded(px(4.))
                        .bg(theme::THUMB)
                        .px(px(4.))
                        .text_size(theme::CAPTION)
                        .line_height(px(18.))
                        .text_color(theme::INK_3)
                        .child("bot"),
                )
            })
            .child(
                div()
                    .flex_none()
                    .text_size(theme::TEXT_SMALL)
                    .line_height(px(18.))
                    .text_color(theme::INK_3)
                    .child(item.when.clone()),
            );
        let quote = item.detail.clone().map(|detail| {
            div()
                .mt(px(8.))
                .ml(px(8.))
                .border_l_2()
                .border_color(theme::LINE_STRONG)
                .pl(px(12.))
                .text_size(px(12.5))
                .line_height(px(18.))
                .text_color(theme::INK_2)
                .child(detail)
        });
        // how Claude should handle it: only for one that goes with the message
        let note = self.notes.get(id).filter(|_| include).map(|(field, _)| {
            let lit = field.read(cx).focus_handle(cx).is_focused(window);
            div()
                .id(named("item-note"))
                .test_support()
                .mt(px(8.))
                .ml(px(8.))
                .min_h(theme::DIALOG_CONTROL)
                .flex()
                .items_center()
                .rounded(theme::RADIUS_CONTROL)
                .border_1()
                .border_color(if lit { theme::KEY_EDGE } else { theme::LINE_STRONG })
                // with the field's own 4 px, its words start 8 px in
                .px(px(4.))
                // a press in the field is the field's own: the card behind it doesn't take it
                .on_click(|_, _, cx| cx.stop_propagation())
                .child(
                    div().min_w_0().flex_1().child(
                        Textarea::new(field)
                            .xsmall()
                            .appearance(false)
                            .aria_label(NOTE_ASKS)
                            .text_size(theme::TEXT)
                            .line_height(theme::TEXT_LINE),
                    ),
                )
        });
        let (open_id, snooze_id, done_id) = (id.clone(), id.clone(), id.clone());
        let url = item.url.clone();
        let acts = div()
            .mt(px(8.))
            .ml(px(8.))
            .flex()
            .items_center()
            .gap(px(8.))
            .children(url.map(|url| {
                square(
                    named("detail-item-open"),
                    "Open where it was said",
                    "icons/external-link.svg",
                    Box::new(OpenItem),
                )
                .on_click(cx.listener(move |card, _, window, cx| {
                    cx.stop_propagation();
                    card.select_item(&open_id, cx);
                    open_in_browser(&url, window, cx);
                }))
            }))
            .child(
                square(named("detail-snooze"), "Snooze for a day", "icons/alarm-clock.svg", Box::new(ItemSnooze))
                    .on_click(cx.listener(move |card, _, window, cx| {
                        cx.stop_propagation();
                        card.set_item(&snooze_id, ItemState::Snoozed, window, cx);
                    })),
            )
            .child(square(named("detail-done"), "Done", "icons/check.svg", Box::new(ItemDone)).on_click(cx.listener(
                move |card, _, window, cx| {
                    cx.stop_propagation();
                    card.set_item(&done_id, ItemState::Done, window, cx);
                },
            )));
        let select = id.clone();
        let card = cx.entity().downgrade();
        let said = match &item.detail {
            Some(detail) => format!("{}: {detail}", item.sentence),
            None => item.sentence.clone(),
        };
        div()
            .id(named("detail-item"))
            .test_support()
            .aria_label(SharedString::from(said))
            .aria_description(SharedString::from(item.when.clone()))
            .aria_selected(selected)
            .relative()
            .rounded(theme::RADIUS_CARD)
            .border_1()
            .bg(theme::PANE)
            .py(px(12.))
            .px(px(16.))
            // the keyboard is on it: the blue edge
            .border_color(if selected { theme::KEY_EDGE } else { theme::LINE })
            .when(!include, |row| row.opacity(0.55))
            .on_click(cx.listener(move |card, _, _, cx| card.select_item(&select, cx)))
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(8.))
                    .child(include_box)
                    .child(div().min_w_0().flex_1().child(head).children(quote).children(note).child(acts)),
            )
            .when(selected, |row| {
                // moved to with the keyboard, it comes into view (what is measured is what the item holds: its edge
                // stands 1 px around that)
                row.on_prepaint(move |bounds, _, cx| {
                    let _ = card.update(cx, |card, cx| card.bring_into_view(bounds.dilate(px(1.)), cx));
                })
            })
            .into_any_element()
    }

    /// "Needs you", with its count: every item of the card, in the card's order.
    pub(super) fn needs(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let items = self.made.items.clone();
        if items.is_empty() {
            return None;
        }
        let draft = self.draft(cx);
        let current = self.current();
        let rows: Vec<AnyElement> = (items.iter().enumerate())
            .map(|(at, item)| self.item_card(item, current == Some(at), &draft, window, cx))
            .collect();
        Some(
            div()
                .id("task-needs")
                .test_support()
                .aria_label("Needs you")
                .aria_description(SharedString::from(items.len().to_string()))
                .flex_none()
                .child(heading("Needs you", Some(items.len())))
                .child(div().flex().flex_col().gap(px(8.)).children(rows))
                .into_any_element(),
        )
    }
}
