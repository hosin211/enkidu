//! The card's Claude box (DESIGN.md, "The card in front of the board"): which Claude the message goes to. Each Claude
//! linked to the card is a row you can pick (its workspace or where it ran, its lamp, its strongest link in words, Open
//! for one in a workspace, and a way to undo a link that is wrong), the likeliest first; then New Claude, New workspace
//! when the card has one already, and any other workspace's Claude of the project. Your pick is kept with the card's
//! draft. What it says is worked out in `enkidu_model::work::claudes::claude_box`.

use enkidu_model::work::claudes::{ClaudeBox, ClaudeChoice, card_subjects, claude_box};
use enkidu_ui::lamp::{Status, lamp};
use enkidu_ui::pieces::{MenuTrigger, mono};
use enkidu_ui::tips::Tipped as _;
use enkidu_ui::tokens::{self as theme};
use enkidu_ui::work::tinted;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context, Div, Role, SharedString, Toggled, Window, div, px, svg};

use crate::work::card::{Card, fact_box};
use crate::work::drafts::Drafts;
use crate::work::store::Work;

/// A round mark: picked or not.
fn radio(on: bool) -> Div {
    div()
        .mt(px(3.))
        .size(px(12.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded_full()
        .border_1()
        .border_color(if on { theme::ACCENT } else { theme::RULE_STRONG })
        .when(on, |mark| mark.child(div().size(px(6.)).rounded_full().bg(theme::ACCENT)))
}

/// A row you can pick: its round mark, then what it is on two lines.
fn option(id: SharedString, label: String, said: String, picked: bool) -> enkidu_ui::pieces::Pressed {
    enkidu_ui::press::pressable(div().id(id))
        .test_support()
        .role(Role::RadioButton)
        .aria_label(SharedString::from(label))
        .aria_description(SharedString::from(said))
        .aria_toggled(if picked { Toggled::True } else { Toggled::False })
        .min_w_0()
        .flex_1()
        .flex()
        .items_start()
        .gap(px(8.))
        .rounded(px(7.))
        .px(px(6.))
        .py(px(4.))
        .cursor_pointer()
        .map(|row| if picked { row.bg(theme::SELECTED) } else { row.hover(|s| s.bg(theme::HOVER)) })
        .child(radio(picked))
}

/// The second line of a row: quiet, on one line.
fn under(line: &str) -> Div {
    div().mt(px(2.)).truncate().text_size(px(11.5)).text_color(theme::INK_3).child(line.to_string())
}

impl Card {
    /// The Claude box as it stands now, with your pick. None when there is nothing to choose.
    pub fn claudes(&self, cx: &gpui_kit::App) -> Option<ClaudeBox> {
        let work = self.work.read(cx);
        let index = work.snapshot.as_deref()?;
        let group = work.groups.iter().find(|group| group.id == self.id)?;
        let (workspaces, statuses) = work.built_from();
        let chosen = self.draft(cx).claude.as_deref().map(ClaudeChoice::read);
        claude_box(group, index, workspaces, statuses, chosen.as_ref(), work.now())
    }

    /// You pick the Claude the card's message goes to: kept with the card's draft, shown on the board's card.
    pub(super) fn pick_claude(&mut self, choice: &str, cx: &mut Context<Self>) {
        // another Claude picked: what was said about a branch no longer applies
        self.taken = None;
        let choice = choice.to_string();
        Drafts::change(&self.drafts, &self.id, cx, |draft| draft.claude = Some(choice));
    }

    /// This session isn't the card's Claude: its links to the card go (and stay gone); if it was your pick, the card
    /// is back with the likeliest one.
    fn unlink(&mut self, session: &str, window: &mut Window, cx: &mut Context<Self>) {
        let subjects = {
            let work = self.work.read(cx);
            let group = work.groups.iter().find(|group| group.id == self.id);
            group.zip(work.snapshot.as_deref()).map(|(group, index)| card_subjects(group, index)).unwrap_or_default()
        };
        if self.draft(cx).claude.as_deref() == Some(session) {
            Drafts::change(&self.drafts, &self.id, cx, |draft| draft.claude = None);
        }
        Work::unlink(&self.work, subjects, session, window.window_handle(), cx);
    }

    /// Open: the card closes and the workspace comes on screen, in place of the Work page.
    pub(super) fn go_to_workspace(&mut self, workspace: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.leave(window, cx);
        let (id, workspace) = (self.id.clone(), workspace.to_string());
        let _ = self.page.update(cx, |page, cx| page.left_for_workspace(&id, &workspace, cx));
    }

    /// The box, drawn. `the` is [`Card::claudes`].
    pub(super) fn claude_box(&self, the: &ClaudeBox, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let mut rows: Vec<AnyElement> = Vec::new();
        for row in &the.rows {
            let (pick, open, unlink) = (row.session.clone(), row.workspace.clone(), row.session.clone());
            let lit = row.status.as_deref().map(Status::from_core);
            let said = match lit {
                Some(status) => format!("{}, {}", row.line, status.label().to_lowercase()),
                None => row.line.clone(),
            };
            let id = |what: &str| SharedString::from(format!("{what}:{}", row.session));
            let main = option(id("claude-option"), row.name.clone(), said, row.picked)
                // the line at the card's foot brings the keyboard here, onto the Claude picked
                .when(row.picked, |main| {
                    let lit = self.claude_focus.is_focused(window);
                    main.track_focus(&self.claude_focus).when(lit, |main| {
                        main.shadow(vec![gpui_kit::BoxShadow {
                            color: gpui_kit::Hsla::from(theme::ACCENT),
                            offset: gpui_kit::point(px(0.), px(0.)),
                            blur_radius: px(0.),
                            spread_radius: px(1.),
                            inset: false,
                        }])
                    })
                })
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .text_size(px(12.5))
                                .child(tinted("icons/sparkle-filled.svg", theme::CLAUDE, 12.))
                                .child(
                                    div().min_w_0().flex_1().truncate().text_color(theme::INK).child(row.name.clone()),
                                )
                                .children(lit.map(|status| lamp(id("claude-lamp"), status, 12., 0))),
                        )
                        .child(under(&row.line)),
                )
                .on_click(cx.listener(move |card, _, _, cx| card.pick_claude(&pick, cx)));
            let undo = row.unlinkable.then(|| {
                let group = id("claude-unlink");
                div()
                    .id(group.clone())
                    .test_support()
                    .role(Role::Button)
                    .aria_label("Unlink")
                    .mt(px(2.))
                    .size(px(22.))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .group(group.clone())
                    .hover(|s| s.bg(theme::HOVER))
                    .tipped("Not this task's Claude: unlink it", None)
                    .child(
                        svg()
                            .path("icons/unlink.svg")
                            .size(px(12.))
                            .text_color(theme::INK_3)
                            .group_hover(group, |s| s.text_color(theme::LAMP_RED)),
                    )
                    .on_click(cx.listener(move |card, _, window, cx| card.unlink(&unlink, window, cx)))
            });
            let go = open.map(|workspace| {
                div()
                    .id(id("detail-go-workspace"))
                    .test_support()
                    .role(Role::Button)
                    .aria_label(SharedString::from(format!("Open {}", row.name)))
                    .mt(px(2.))
                    .flex_none()
                    .rounded(px(6.))
                    .px(px(6.))
                    .py(px(2.))
                    .text_size(px(12.))
                    .text_color(theme::INK_3)
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::HOVER).text_color(theme::ACCENT))
                    .child("Open")
                    .on_click(cx.listener(move |card, _, window, cx| card.go_to_workspace(&workspace, window, cx)))
            });
            rows.push(
                div().flex().items_start().gap(px(4.)).child(main).children(undo).children(go).into_any_element(),
            );
        }
        // a new one: a new conversation in the card's workspace, or a new workspace
        let new_row = |choice: &'static str, title: &'static str, icon: &'static str, picked: bool, line: &str| {
            option(SharedString::from(format!("claude-option:{choice}")), title.to_string(), line.to_string(), picked)
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .text_size(px(12.5))
                                .child(tinted(icon, theme::INK_3, 12.))
                                .child(div().text_color(theme::INK).child(title)),
                        )
                        .child(under(line)),
                )
                .on_click(cx.listener(move |card, _, _, cx| card.pick_claude(choice, cx)))
                .into_any_element()
        };
        rows.push(new_row("new", "New Claude", "icons/plus.svg", the.new_claude.picked, &the.new_claude.line));
        if let Some(fresh) = &the.new_workspace {
            rows.push(new_row("fresh", "New workspace", "icons/square-plus.svg", fresh.picked, &fresh.line));
        }
        // any other workspace's Claude, for a card whose links are wrong or missing
        if !the.others.is_empty() {
            let (others, card) = (the.others.clone(), cx.entity().downgrade());
            rows.push(
                MenuTrigger::new("claude-other", "Another workspace's Claude…")
                    .w_full()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .rounded(px(7.))
                    .px(px(6.))
                    .py(px(4.))
                    .text_size(px(12.5))
                    .text_color(theme::INK_3)
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::HOVER).text_color(theme::INK))
                    .child(div().w(px(12.)).flex_none())
                    .child("Another workspace's Claude…")
                    .menu(gpui_kit::Anchor::TopLeft, move |menu, _, _| {
                        let mut menu = menu.min_w(px(240.)).label("Send it to the Claude in");
                        for (session, name, branch) in others.clone() {
                            let (card, pick) = (card.clone(), session.clone());
                            let id = SharedString::from(format!("claude-other-option:{session}"));
                            let item = PopupMenuItem::element(move |_, _| {
                                div()
                                    .id(id.clone())
                                    .test_support()
                                    .aria_label(SharedString::from(name.clone()))
                                    .aria_description(SharedString::from(branch.clone()))
                                    .w_full()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .text_color(theme::INK)
                                    .child(tinted("icons/sparkle-filled.svg", theme::CLAUDE, 12.))
                                    .child(div().min_w_0().flex_1().truncate().child(name.clone()))
                                    .child(
                                        mono(div())
                                            .flex_none()
                                            .max_w(px(120.))
                                            .truncate()
                                            .pl(px(12.))
                                            .text_size(px(11.))
                                            .text_color(theme::INK_3)
                                            .child(branch.clone()),
                                    )
                            });
                            menu = menu.item(item.on_click(move |_, _, cx| {
                                let _ = card.update(cx, |card, cx| card.pick_claude(&pick, cx));
                            }));
                        }
                        menu
                    })
                    .into_any_element(),
            );
        }
        let said = match the.rows.iter().find(|row| row.picked) {
            Some(row) => format!("The message goes to the Claude in {}", row.name),
            None if the.new_workspace.as_ref().is_some_and(|row| row.picked) => {
                "The message goes to a new workspace".into()
            }
            None => "The message goes to a new Claude".to_string(),
        };
        fact_box("detail-sessions", "Claude", said, None)
            .child(
                div()
                    .id("claude-options")
                    .role(Role::RadioGroup)
                    .aria_label("The Claude your message goes to")
                    .mx(px(-6.))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .children(rows),
            )
            .into_any_element()
    }
}
