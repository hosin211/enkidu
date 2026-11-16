//! "Tell Claude", the bottom of the card, always in view (DESIGN.md, "Tell Claude"): where the message goes, a field
//! for what Claude should do about the task, and what goes with your words, as chips. "See the message" shows it whole,
//! word for word as it goes; edit it there and your text is what goes ("Undo my edits" brings back the one made for
//! you). The one accent button types it into that Claude's prompt and takes you there, the keyboard on it: nothing is
//! ever sent for you. With no Claude on the card yet, it starts a workspace.

use enkidu_model::text::short_path;
use enkidu_model::work::claudes::{ClaudeChoice, SendTarget, card_subjects, task_target};
use enkidu_model::work::compose::{
    Draft, Said, message_of, note_asks, note_subject, send_button, sent_note, sent_toast, target_kind, target_said,
    target_words, with_it,
};
use enkidu_ui::pieces::{ButtonKind, ButtonSize, button, button_with, mono};
use enkidu_ui::tips::Tipped as _;
use enkidu_ui::tokens::{self as theme, alpha};
use enkidu_ui::work::tinted;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::input::{InputEvent, InputState, Position, Textarea, TextareaState};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, AsyncWindowContext, Context, Entity, Focusable as _, Font, FontWeight, Role, SharedString, StyledText,
    TextRun, WeakEntity, Window, div, px,
};

use crate::bridge::Bridge;
use crate::toast;
use crate::work::card::Card;
use crate::work::drafts::Drafts;
use crate::work::send::{Outcome, Taken, Way, deliver};

/// The branch the card's new workspace wants is checked out in another folder: what was found, and the new branch
/// offered from it.
pub(super) struct TakenOffer {
    pub taken: Taken,
    pub new_branch: Entity<InputState>,
}

/// Where the cursor is after `words`.
fn end_of(words: &str) -> Position {
    Position {
        line: words.matches('\n').count() as u32,
        character: words.rsplit('\n').next().map_or(0, |last| last.encode_utf16().count() as u32),
    }
}

impl Card {
    /// Where the card's message goes now: to the Claude picked in its Claude box.
    pub fn target(&self, cx: &gpui_kit::App) -> SendTarget {
        let work = self.work.read(cx);
        let (Some(index), Some(group)) = (work.snapshot.as_deref(), work.groups.iter().find(|g| g.id == self.id))
        else {
            return SendTarget::None;
        };
        let (workspaces, statuses) = work.built_from();
        let chosen = self.draft(cx).claude.as_deref().map(ClaudeChoice::read);
        task_target(group, index, workspaces, statuses, chosen.as_ref())
    }

    /// The message as it goes: the one you edited, else the one made from the card and your words.
    pub fn message(&self, cx: &gpui_kit::App) -> String {
        let work = self.work.read(cx);
        match (work.snapshot.as_deref(), work.groups.iter().find(|g| g.id == self.id)) {
            (Some(index), Some(group)) => message_of(group, index, &self.draft(cx)),
            _ => String::new(),
        }
    }

    /// What the whole message's field holds, while it is shown.
    pub fn whole_text(&self, cx: &gpui_kit::App) -> Option<String> {
        self.whole.as_ref().map(|(field, _)| field.read(cx).value().to_string())
    }

    /// The message is on its way into Claude's prompt.
    pub fn is_sending(&self) -> bool {
        self.sending
    }

    /// Before the card is drawn: the field for your words is there, and the whole message's field while it is shown
    /// holds the message as it goes.
    pub(super) fn settle_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let draft = self.draft(cx);
        if self.note.is_none() {
            let asks = {
                let work = self.work.read(cx);
                let group = work.groups.iter().find(|g| g.id == self.id);
                group.zip(work.snapshot.as_deref()).map(|(group, index)| note_asks(group, index)).unwrap_or_default()
            };
            let words = draft.note.clone();
            let field =
                cx.new(|cx| TextareaState::new(window, cx).auto_grow(2, 6).placeholder(asks).default_value(words));
            let heard = cx.subscribe_in(&field, window, |card, field, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    let words = field.read(cx).value().to_string();
                    Drafts::change(&card.drafts, &card.id, cx, |draft| draft.note = words);
                }
                // ⌘↩ sends from here (the line break it typed first is taken back)
                InputEvent::PressEnter { secondary: true, .. } => {
                    let words = field.read(cx).value().to_string();
                    let kept = words.strip_suffix('\n').unwrap_or(&words).to_string();
                    field.update(cx, |field, cx| field.set_value(kept.clone(), window, cx));
                    Drafts::change(&card.drafts, &card.id, cx, |draft| draft.note = kept);
                    card.send(None, window, cx);
                }
                _ => {}
            });
            self.note = Some((field, heard));
        }
        // the whole message: there while you look at it, or once you edited it
        if !(self.whole_open || draft.text.is_some()) {
            self.whole = None;
            return;
        }
        let message = self.message(cx);
        match &self.whole {
            None => {
                let (rows, text) = (if draft.text.is_some() { 16 } else { 10 }, message.clone());
                self.whole_put = message;
                let field = cx.new(|cx| TextareaState::new(window, cx).auto_grow(2, rows).default_value(text));
                let heard =
                    cx.subscribe_in(&field, window, |card, field, event: &InputEvent, window, cx| match event {
                        // edited: your text is the message
                        InputEvent::Change => {
                            let text = field.read(cx).value().to_string();
                            Drafts::change(&card.drafts, &card.id, cx, |draft| draft.text = Some(text));
                        }
                        InputEvent::PressEnter { secondary: true, .. } => {
                            // the line break ⌘↩ typed is not an edit
                            let text = field.read(cx).value().to_string();
                            let kept = text.strip_suffix('\n').unwrap_or(&text).to_string();
                            let made = {
                                let unedited = Draft { text: None, ..card.draft(cx) };
                                let work = card.work.read(cx);
                                let group = work.groups.iter().find(|g| g.id == card.id);
                                group
                                    .zip(work.snapshot.as_deref())
                                    .map(|(group, index)| message_of(group, index, &unedited))
                            };
                            let edit = Some(kept.clone()).filter(|kept| made.as_ref() != Some(kept));
                            field.update(cx, |field, cx| field.set_value(kept, window, cx));
                            Drafts::change(&card.drafts, &card.id, cx, |draft| draft.text = edit);
                            card.send(None, window, cx);
                        }
                        _ => {}
                    });
                self.whole = Some((field, heard));
            }
            // not edited: it follows the card and your words as they change. Only what was put there by us is ever
            // put there again: a letter you just typed (heard a moment later) is the start of an edit, never undone
            Some((field, _)) if draft.text.is_none() && self.whole_put != message => {
                let field = field.clone();
                self.whole_put = message.clone();
                field.update(cx, |field, cx| field.set_value(message, window, cx));
            }
            Some(_) => {}
        }
    }

    /// s or c: the keyboard goes to what you tell Claude (the whole message once you edited it), after what is
    /// there.
    pub(super) fn write_to_claude(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let edited = self.draft(cx).text.is_some();
        let field = if edited { self.whole.as_ref() } else { self.note.as_ref() }.map(|(field, _)| field.clone());
        if let Some(field) = field {
            let end = end_of(field.read(cx).value().as_ref());
            field.update(cx, |field, cx| field.set_cursor_position(end, window, cx));
        }
    }

    /// The line that says where the message goes was pressed: the Claude box comes into view, the keyboard on the
    /// Claude picked.
    fn show_claudes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.facts_scroll.scroll_to_bottom();
        self.claude_focus.focus(window, cx);
        cx.notify();
    }

    /// The accent button, or ⌘↩ from anywhere in the card: the message goes into the prompt of the Claude picked,
    /// never sent. `way` is what you picked when its new workspace's branch was checked out in another folder.
    pub(super) fn send(&mut self, way: Option<Way>, window: &mut Window, cx: &mut Context<Self>) {
        if self.sending || (self.taken.is_some() && way.is_none()) {
            return;
        }
        let (Some(frame), Some(terminals)) = (self.frame.clone(), self.terminals.clone()) else { return };
        let target = self.target(cx);
        let (text, draft) = (self.message(cx), self.draft(cx));
        // what the card's timeline and the index are told once it went
        let (subjects, subject) = {
            let work = self.work.read(cx);
            let group = work.groups.iter().find(|g| g.id == self.id);
            match group.zip(work.snapshot.as_deref()) {
                Some((group, index)) => (card_subjects(group, index), note_subject(group, index)),
                None => (Vec::new(), String::new()),
            }
        };
        self.sending = true;
        self.way = way.clone();
        self.steps_from = self.progress.as_ref().map_or(0, |log| log.read(cx).0.len());
        let going = deliver(frame, terminals, target.clone(), text, way, cx);
        cx.spawn_in(window, async move |card: WeakEntity<Self>, cx: &mut AsyncWindowContext| {
            let outcome = going.await;
            let _ = card.update_in(cx, |card, window, cx| {
                card.sending = false;
                card.way = None;
                match outcome {
                    Ok(Outcome::Sent(workspace, warnings)) => {
                        for warning in warnings {
                            toast::warning(window, "Workspace created, with a warning", warning, cx);
                        }
                        // you sent the card's work there: the card goes there next time too, and its timeline says so
                        let (session, said) =
                            (workspace.session_id.clone(), sent_note(&target, &workspace.name, &draft));
                        Bridge::call(
                            cx,
                            move |shared| {
                                let events = enkidu_core::Events::none();
                                let _ =
                                    enkidu_core::commands::work_link_session(events.clone(), shared, subjects, session);
                                let _ = enkidu_core::commands::work_note(events, shared, subject, said);
                            },
                            |(), cx| {
                                if let Some(stores) = cx.try_global::<crate::stores::Stores>() {
                                    let work = stores.work.clone();
                                    crate::work::store::Work::read(&work, cx);
                                }
                            },
                        );
                        // your words are sent; the card stays with the Claude it went to
                        let kept = Draft { claude: Some(workspace.session_id.clone()), ..Draft::default() };
                        card.drafts.update(cx, |drafts, cx| drafts.set(&card.id, kept, cx));
                        let (title, detail) = sent_toast(&target, &workspace.name);
                        toast::done(window, title, detail, cx);
                        // you are taken to that Claude, to read it and press Enter
                        card.leave(window, cx);
                        let (id, to) = (card.id.clone(), workspace);
                        let _ = card.page.update(cx, |page, cx| page.left_for_new_workspace(&id, to, cx));
                    }
                    Ok(Outcome::Taken(taken)) => {
                        let offered = enkidu_model::new_workspace::next_branch(&taken.branch, &[]);
                        let new_branch = cx.new(|cx| InputState::new(window, cx).default_value(offered));
                        card.taken = Some(TakenOffer { taken, new_branch });
                    }
                    Ok(Outcome::Nowhere) => {
                        toast::note(window, "Nowhere to start Claude on this", "The message is on your clipboard.", cx);
                    }
                    Err(why) => toast::warning(window, "Couldn't put it in Claude's prompt", why, cx),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// While the card's new workspace is made: the step the core is on ("Running the setup script").
    fn step(&self, cx: &gpui_kit::App) -> Option<String> {
        if !self.sending {
            return None;
        }
        let project = match self.target(cx) {
            SendTarget::New { project_id, .. } | SendTarget::Outside { project_id, .. } => project_id,
            _ => return None,
        };
        let log = self.progress.as_ref()?.read(cx);
        let running = |step: &&enkidu_core::workspace::Progress| step.project_id == project && step.state == "running";
        log.0
            .iter()
            .skip(self.steps_from)
            .rev()
            .find(running)
            .map(|step| step.detail.clone())
            .filter(|said| !said.is_empty())
    }

    /// The card's new workspace wants a branch another folder has: open that folder, or a new branch from it.
    fn taken_panel(&self, offer: &TakenOffer, cx: &mut Context<Self>) -> AnyElement {
        let taken = &offer.taken;
        let there = self
            .frame
            .as_ref()
            .and_then(|frame| frame.read(cx).workspaces.iter().find(|w| w.path == taken.path).map(|w| w.name.clone()));
        let busy = self.way.clone();
        let open_words = match (&busy, &there) {
            (Some(Way::Open(_)), _) => "Opening…".to_string(),
            (_, Some(name)) => format!("Go to {name}"),
            (_, None) => "Open that folder as a workspace".to_string(),
        };
        let quiet = busy.is_some();
        let dim = |el: enkidu_ui::pieces::Pressed| if quiet { el.opacity(0.6).cursor_default() } else { el };
        let (path, field) = (taken.path.clone(), offer.new_branch.clone());
        div()
            .id("branch-taken")
            .test_support()
            .aria_label(SharedString::from(format!("{} is checked out in {}", taken.branch, short_path(&taken.path))))
            .mt(px(10.))
            .flex()
            .flex_col()
            .gap(px(10.))
            .p(px(14.))
            .rounded(px(10.))
            .bg(alpha(theme::CHROME, 0.7))
            .border_1()
            .border_color(theme::RULE)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_x(px(4.))
                    .text_size(px(12.5))
                    .line_height(px(18.))
                    .text_color(theme::INK_2)
                    .child(mono(div()).text_size(px(12.)).text_color(theme::INK).child(taken.branch.clone()))
                    .child("is checked out in")
                    .child(div().text_color(theme::INK).child(short_path(&taken.path)))
                    .child(". Git checks a branch out in one folder only."),
            )
            .child(div().flex().child(
                dim(button("taken-open", open_words, ButtonKind::Secondary, ButtonSize::Medium)).on_click(cx.listener(
                    move |card, _, window, cx| {
                        if card.way.is_none() {
                            card.send(Some(Way::Open(path.clone())), window, cx);
                        }
                    },
                )),
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .min_w_0()
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(12.5))
                            .text_color(theme::INK_3)
                            .child("Or a new branch from it:"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(enkidu_ui::pieces::text_field(&offer.new_branch).id("taken-branch")),
                    )
                    .child(
                        dim(button(
                            "taken-new-branch",
                            if matches!(busy, Some(Way::Branch(_))) { "Creating…" } else { "Create" },
                            ButtonKind::Secondary,
                            ButtonSize::Medium,
                        ))
                        .on_click(cx.listener(move |card, _, window, cx| {
                            let branch = field.read(cx).value().trim().to_string();
                            let same = card.taken.as_ref().is_some_and(|offer| offer.taken.branch == branch);
                            if card.way.is_none()
                                && !branch.is_empty()
                                && !branch.contains(char::is_whitespace)
                                && !same
                            {
                                card.send(Some(Way::Branch(branch)), window, cx);
                            }
                        })),
                    ),
            )
            .into_any_element()
    }

    /// The card's foot.
    pub(super) fn composer(&self, has_claudes: bool, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let draft = self.draft(cx);
        let target = self.target(cx);
        let edited = draft.text.is_some();
        let chips = {
            let work = self.work.read(cx);
            let group = work.groups.iter().find(|g| g.id == self.id);
            group.zip(work.snapshot.as_deref()).map(|(group, index)| with_it(group, index, &draft)).unwrap_or_default()
        };
        // where it goes, in a sentence: its names brighter, a branch in the code face
        let said = target_said(&target);
        let mut line = String::new();
        let mut runs: Vec<TextRun> = Vec::new();
        for part in &said {
            let (words, color, code) = match part {
                Said::Words(words) => (words, theme::INK_3, false),
                Said::Name(words) => (words, theme::INK_2, false),
                Said::Branch(words) => (words, theme::INK_3, true),
            };
            line.push_str(words);
            runs.push(TextRun {
                len: words.len(),
                font: match code {
                    true => Font {
                        features: gpui_kit::FontFeatures::disable_ligatures(),
                        ..gpui_kit::font(theme::MONO_FONT)
                    },
                    false => gpui_kit::font(theme::UI_FONT),
                },
                color: color.into(),
                background_color: None,
                underline: None,
                strikethrough: None,
            });
        }
        let where_to = div()
            .id("task-target")
            .test_support()
            .aria_label(SharedString::from(target_words(&target)))
            .aria_description(target_kind(&target))
            .min_w_0()
            .rounded(px(4.))
            .line_height(px(17.))
            .child(StyledText::new(line).with_runs(runs))
            .when(has_claudes, |line| {
                // a click shows the Claude box, to pick another
                line.cursor_pointer()
                    .tipped("Pick another Claude", None)
                    .on_click(cx.listener(|card, _, window, cx| card.show_claudes(window, cx)))
            });
        let quiet_link = |id: &'static str, words: &'static str| {
            div()
                .id(id)
                .test_support()
                .role(Role::Button)
                .aria_label(words)
                .flex_none()
                .text_size(px(12.))
                .text_color(theme::INK_3)
                .cursor_pointer()
                .hover(|s| s.text_color(theme::ACCENT))
                .child(words)
        };
        let toggle = match edited {
            true => quiet_link("task-message-reset", "Undo my edits").on_click(cx.listener(|card, _, _, cx| {
                Drafts::change(&card.drafts, &card.id, cx, |draft| draft.text = None);
                // the field holds your text now: the message made for you is put there again
                card.whole_put.clear();
                cx.notify();
            })),
            false => {
                let words = if self.whole_open { "Hide the message" } else { "See the message" };
                quiet_link("task-preview-toggle", words).on_click(cx.listener(|card, _, _, cx| {
                    card.whole_open = !card.whole_open;
                    cx.notify();
                }))
            }
        };
        let first = div()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(8.))
            .text_size(px(12.))
            .child(tinted("icons/sparkle-filled.svg", theme::CLAUDE, 12.))
            .child(div().flex_none().font_weight(FontWeight::SEMIBOLD).text_color(theme::INK_2).child("Tell Claude"))
            .child(where_to)
            .child(div().flex_1())
            .child(toggle);
        // your words about the task (they step aside once you edited the whole message)
        let note = self.note.as_ref().filter(|_| !edited).map(|(field, _)| {
            let lit = field.read(cx).focus_handle(cx).is_focused(window);
            div()
                .id("task-note")
                .test_support()
                .mt(px(8.))
                .min_h(px(44.))
                .rounded(px(10.))
                .border_1()
                .border_color(if lit { alpha(theme::ACCENT, 0.6) } else { theme::RULE_STRONG })
                .bg(theme::CANVAS)
                .px(px(4.))
                .py(px(2.))
                .child(
                    Textarea::new(field)
                        .appearance(false)
                        .aria_label("What Claude should do about the task")
                        .text_size(px(13.))
                        .line_height(px(19.)),
                )
        });
        // the whole message, word for word as it goes to Claude; edit it here and your text is what goes
        let whole = self.whole.as_ref().map(|(field, _)| {
            let lit = field.read(cx).focus_handle(cx).is_focused(window);
            div()
                .id("task-preview")
                .test_support()
                .aria_description(if edited { "edited" } else { "as made for you" })
                .mt(px(8.))
                .rounded(px(10.))
                .border_1()
                .border_color(if lit { alpha(theme::ACCENT, 0.6) } else { theme::RULE })
                .bg(alpha(theme::FRAME, 0.6))
                .px(px(4.))
                .py(px(2.))
                .text_color(if lit { theme::INK } else { theme::INK_2 })
                .child(
                    mono(Textarea::new(field).appearance(false).aria_label("The message to Claude"))
                        .text_size(px(11.5))
                        .line_height(px(17.)),
                )
        });
        let taken = self.taken.as_ref().map(|offer| self.taken_panel(offer, cx));
        let kept: Vec<AnyElement> = chips
            .iter()
            .enumerate()
            .map(|(n, (words, bright))| {
                div()
                    .id(("task-chip", n))
                    .test_support()
                    .aria_label(SharedString::from(words.clone()))
                    .h(px(20.))
                    .flex()
                    .flex_none()
                    .items_center()
                    .rounded(px(6.))
                    .bg(theme::HOVER)
                    .px(px(6.))
                    .text_size(px(11.5))
                    .text_color(if *bright { theme::INK } else { theme::INK_2 })
                    .child(words.clone())
                    .into_any_element()
            })
            .collect();
        let step = self.step(cx).map(|step| {
            div()
                .id("task-step")
                .test_support()
                .aria_label(SharedString::from(step.clone()))
                .min_w_0()
                .truncate()
                .text_size(px(12.))
                .text_color(theme::INK_3)
                .child(step)
        });
        let the = send_button(&target);
        let busy = self.sending || self.taken.is_some();
        let icon = if the.plus { "icons/plus.svg" } else { "icons/sparkle-filled.svg" };
        let label = if self.sending { the.busy } else { the.label };
        let send = button_with("task-send", Some(icon), label, Some("⌘↩"), ButtonKind::Accent, ButtonSize::Medium)
            .aria_description(target_kind(&target))
            .when(busy, |button| button.opacity(0.6))
            .on_click(cx.listener(|card, _, window, cx| card.send(None, window, cx)));
        let said = chips.iter().map(|(words, _)| words.as_str()).collect::<Vec<_>>().join(", ");
        let foot = div()
            .mt(px(10.))
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(div().flex_none().text_size(px(12.)).text_color(theme::INK_3).child("With it:"))
            .child(
                div()
                    .id("task-with")
                    .test_support()
                    .aria_label(SharedString::from(said))
                    .min_w_0()
                    .flex_1()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(6.))
                    .children(kept),
            )
            .children(step)
            .child(send);
        div()
            .id("task-composer")
            .test_support()
            .aria_label("Tell Claude")
            .flex_none()
            .border_t_1()
            .border_color(theme::RULE)
            .bg(alpha(gpui_kit::rgb(0x000000), 0.12))
            .px(px(24.))
            .pb(px(16.))
            .pt(px(12.))
            .child(first)
            .children(note)
            .children(whole)
            .children(taken)
            .child(foot)
            .into_any_element()
    }
}
