//! The card in front of the board (DESIGN.md, "The card in front of the board"): a dialog over the page. Its header
//! says what it is (the type, the key, Hide, Open in Jira, the title, the tabs); on the left is the tab's story, on the
//! right how it stands, always in the same order: Jira, each pull request, the Claude it goes to. What it says is
//! worked out in `enkidu_model::work::card`; here it is drawn.

use std::collections::HashMap;
use std::rc::Rc;

use enkidu_core::work::model::Snapshot;
use enkidu_model::frame::Frame;
use enkidu_model::work::card::{
    CardModel, ItemRow, JIRA_NOT_READ, JiraBox, JiraFacts, PrFacts, TIMELINE_SHOWN, Tab, item_rows,
};
use enkidu_model::work::page::CardHead;
use enkidu_model::work::pr::Checks;
use enkidu_model::work::prtab::PrTab;
use enkidu_model::work::rich::{Block, jira_doc};
use enkidu_ui::pieces::changes_seen;
use enkidu_ui::pieces::{Observed, mono};
use enkidu_ui::tips::Tipped as _;
use enkidu_ui::tokens::{self as theme, alpha, tone};
use enkidu_ui::work::{SourceLook, checks_dot, pr_glyph, priority_icon, tinted, type_icon};
use gpui_kit::base::{ElementExt as _, TestSupportExt as _};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input::TextareaState;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Context, Div, Entity, FocusHandle, FontWeight, IntoElement, Pixels, Render, Role, ScrollHandle,
    SharedString, Subscription, WeakEntity, Window, actions, div, linear_color_stop, linear_gradient, px, svg,
};

use crate::stores::ProgressLog;
use crate::terminals::Terminals;
use crate::toast;
use crate::work::board::{ink, pr_state};
use crate::work::composer::TakenOffer;
use crate::work::drafts::Drafts;
use crate::work::page::WorkPage;
use crate::work::pr_tab::{Opened, pr_tab};
use crate::work::report::ReportsMade;
use crate::work::rich::rich;
use crate::work::send::Way;
use crate::work::shots::Shots;
use crate::work::store::{ItemState, Work};

/// The card's own keys work while the keyboard is in it.
pub const KEYS: &str = "WorkCard";
/// Its plain keys (j, k and the arrows, x, n, e, z, ↩) work only while no text field has the keyboard: in a field
/// they are letters.
pub const PLAIN_KEYS: &str = "WorkCard && !Input && !WorkShot";

actions!(work_card, [NextItem, PrevItem, ToggleItem, ItemNote, ItemDone, ItemSnooze, OpenItem, TellClaude, SendTask]);

/// How wide the card is with a story on its left, and with the facts alone.
const WIDE: f32 = 1040.;
const NARROW: f32 = 560.;
/// The column of facts at its right.
const FACTS: f32 = 316.;
/// A description is at most this wide (the old 68 characters), and a long one shows this much of its start: less
/// when something needs you under it. One that is only a little longer than that is shown whole.
pub(super) const TASK_WIDTH: f32 = 548.;
const TASK_SHORT: f32 = 168.;
const TASK_TALL: f32 = 440.;
const TASK_SLACK: f32 = 48.;
/// How much of the window the card leaves free on each side, and above and below (a twentieth of its height).
const SIDE: f32 = 20.;
const ABOVE: f32 = 0.05;

/// A card's model, with the store's count when it was made: it is made again only when the index or its cards
/// changed.
pub(super) struct Made {
    model: Rc<CardModel>,
    /// The ticket's description, read into blocks (none: no description, or an empty one).
    task: Option<Rc<Vec<Block>>>,
    /// What waits on you on the card, in its order.
    pub(super) items: Rc<Vec<ItemRow>>,
    /// What is going on in each PR that has a tab, by the PR's place in the index.
    prs: Vec<(usize, Rc<PrTab>)>,
    /// Claude's reports on it, how it ended first.
    pub(super) reports: Option<Rc<ReportsMade>>,
    /// The store's count of its cards' builds when this was made.
    revision: u64,
}

impl Made {
    fn of(work: &Work, id: &str) -> Option<Made> {
        // read where it is: nothing here keeps the index (it is changed in place when you mark something done)
        let index: &Snapshot = work.snapshot.as_deref()?;
        let group = work.groups.iter().find(|group| group.id == id)?;
        let model = Rc::new(CardModel::of(group, index, work.now()));
        let described = group.ticket(index).and_then(|ticket| ticket.description.as_ref());
        let task = described.map(jira_doc).filter(|blocks| !blocks.is_empty()).map(Rc::new).or_else(|| {
            // a task of your own: what you wrote about it, read as Markdown (lists, code, links)
            let notes = group.own(index).map(|task| task.notes.as_str()).filter(|notes| !notes.is_empty())?;
            Some(Rc::new(enkidu_model::work::markdown::blocks_of(notes)))
        });
        let items = Rc::new(item_rows(group, index, work.now()));
        let prs = (group.prs.iter())
            .filter(|view| view.of(index).details.created_at.is_some())
            .map(|view| (view.pr, Rc::new(PrTab::of(view.of(index), work.now()))))
            .collect();
        let reports = ReportsMade::of(group, index, work.built_from().0, work.now()).map(Rc::new);
        Some(Made { model, task, items, prs, reports, revision: work.revision })
    }

    fn still(&self, work: &Work) -> bool {
        work.revision == self.revision
    }
}

pub struct Card {
    pub(super) page: WeakEntity<WorkPage>,
    pub(super) work: Entity<Work>,
    /// What you wrote about each card, kept until it goes.
    pub(super) drafts: Entity<Drafts>,
    /// The card's id (its group's).
    pub(super) id: String,
    pub(super) made: Made,
    /// The thing that needs you the keyboard was moved to, and the place it had: the one that takes its place is
    /// next (the first as the card opens).
    pub(super) item: Option<String>,
    pub(super) item_at: usize,
    /// The item the keyboard just moved to is brought into view as it is drawn.
    pub(super) reveal: bool,
    /// Each item's instruction field, and the hearing of what is typed in it.
    pub(super) notes: HashMap<String, (Entity<TextareaState>, Subscription)>,
    /// Where the left side is scrolled to.
    pub(super) story_scroll: ScrollHandle,
    /// The window's workspaces and its terminals, for sending (none in a window that runs nothing).
    pub(super) frame: Option<Entity<Frame>>,
    pub(super) terminals: Option<Entity<Terminals>>,
    /// The core's news of the steps of making a workspace.
    pub(super) progress: Option<Entity<ProgressLog>>,
    /// The field for what Claude should do about the task, and the hearing of what is typed in it.
    pub(super) note: Option<(Entity<TextareaState>, Subscription)>,
    /// The whole message's field, while it is shown: "See the message" was pressed, or you edited it.
    pub(super) whole: Option<(Entity<TextareaState>, Subscription)>,
    pub(super) whole_open: bool,
    /// The message last put into that field by the card itself (not by you).
    pub(super) whole_put: String,
    /// The message is on its way into Claude's prompt (a workspace may be being made for it): from which of the
    /// core's steps on, and by which way out of a taken branch.
    pub(super) sending: bool,
    pub(super) way: Option<Way>,
    pub(super) steps_from: usize,
    /// The card's new workspace wants a branch another folder has: what to do is asked.
    pub(super) taken: Option<TakenOffer>,
    /// Where the right side is scrolled to, and the keyboard's place on the Claude picked.
    pub(super) facts_scroll: ScrollHandle,
    pub(super) claude_focus: FocusHandle,
    /// The tab you chose; none: the card's first.
    tab: Option<Tab>,
    /// The whole timeline, after "Show all".
    all_events: bool,
    /// The whole description, after "Show the whole description".
    task_open: bool,
    /// How tall the whole description is (known once it was drawn): a long one shows its start.
    task_height: Option<Pixels>,
    /// What you opened in the PR's tab on screen.
    pub(crate) pr_opened: Opened,
    /// The screenshots of Claude's reports, and the one shown as large as the window.
    pub(super) shots: Entity<Shots>,
    /// The earlier reports, after "Earlier reports".
    pub(super) all_reports: bool,
    pub(super) focus: FocusHandle,
    /// The dialog is closing (or closed): nothing closes it twice.
    left: bool,
}

/// The stores a card stands on: the index, what you wrote, and for sending the window's workspaces and terminals.
pub struct CardStores {
    pub work: Entity<Work>,
    pub drafts: Entity<Drafts>,
    pub frame: Option<Entity<Frame>>,
    pub terminals: Option<Entity<Terminals>>,
}

/// Opens the card `id` in front of the board, the keyboard on it. None when no such card is read.
pub fn open(
    page: &Entity<WorkPage>,
    stores: CardStores,
    id: &str,
    window: &mut Window,
    cx: &mut App,
) -> Option<Entity<Card>> {
    let made = Made::of(stores.work.read(cx), id)?;
    let view = cx.new(|cx| Card::new(page, stores, id, made, window, cx));
    let shown = view.clone();
    window.open_dialog(cx, move |dialog, window, cx| {
        // a twentieth of the window down, 1,040 px wide with a story (560 with the facts alone), or the window less 20
        // px on each side. The builder runs with every frame, so it follows both
        let room = window.viewport_size();
        let width = if shown.read(cx).made.model.story { WIDE } else { NARROW };
        let (on_cancel, on_close) = (shown.clone(), shown.clone());
        crate::dialog_parts::surface(dialog)
            .w(px(width.min(f32::from(room.width) - 2. * SIDE)))
            .margin_top(px(f32::from(room.height) * ABOVE))
            .child(shown.clone())
            // ↩ is the card's own key, never the dialog's OK
            .on_ok(|_, _, _| false)
            // Esc lets go of a field first, then closes
            .on_cancel(move |_, window, cx| on_cancel.update(cx, |card, cx| card.escape(window, cx)))
            .on_close(move |_, _, cx| on_close.update(cx, |card, cx| card.closed(cx)))
    });
    // the keyboard is on the card itself, not on a field of it
    let focus = view.read(cx).focus.clone();
    window.defer(cx, move |window, cx| focus.focus(window, cx));
    Some(view)
}

impl Card {
    fn new(
        page: &Entity<WorkPage>,
        stores: CardStores,
        id: &str,
        made: Made,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let CardStores { work, drafts, frame, terminals } = stores;
        cx.observe_in(page, window, |card, page, window, cx| card.follow(&page, window, cx)).detach();
        // what had the keyboard is gone (a menu of the card that closed, a field of an item that was marked done):
        // the card itself takes it, so its keys go on working
        cx.on_focus_lost(window, |card, window, cx| {
            if !card.left {
                card.focus.focus(window, cx);
            }
        })
        .detach();
        // what you write shows as you write it
        cx.observe(&drafts, |_, _, cx| cx.notify()).detach();
        // a screenshot shows once it is read
        let shots = cx.new(Shots::new);
        cx.observe(&shots, |_, _, cx| cx.notify()).detach();
        // the steps of making the card's workspace show beside its button
        let progress = cx.try_global::<crate::stores::Stores>().map(|stores| stores.progress.clone());
        if let Some(progress) = &progress {
            cx.observe(progress, |card, _, cx| {
                if card.sending {
                    cx.notify();
                }
            })
            .detach();
        }
        Card {
            page: page.downgrade(),
            work,
            drafts,
            frame,
            terminals,
            progress,
            note: None,
            whole: None,
            whole_open: false,
            whole_put: String::new(),
            sending: false,
            way: None,
            steps_from: 0,
            taken: None,
            facts_scroll: ScrollHandle::new(),
            claude_focus: cx.focus_handle(),
            id: id.to_string(),
            made,
            item: None,
            item_at: 0,
            reveal: false,
            notes: HashMap::new(),
            story_scroll: ScrollHandle::new(),
            tab: None,
            all_events: false,
            task_open: false,
            task_height: None,
            pr_opened: Opened::default(),
            shots,
            all_reports: false,
            focus: cx.focus_handle(),
            left: false,
        }
    }

    /// The screenshots of the card's reports (a test reads how far they are).
    pub fn shots(&self) -> &Entity<Shots> {
        &self.shots
    }

    /// The card's id.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// What the card says now.
    pub fn model(&self) -> &CardModel {
        &self.made.model
    }

    /// The tab on screen.
    pub fn tab(&self) -> Option<&Tab> {
        self.made.model.tab(self.tab.as_ref())
    }

    /// Another card takes this one's place in the same dialog. A card opens on its first tab, also the same card
    /// opened again.
    pub(crate) fn show(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(made) = Made::of(self.work.read(cx), id) {
            self.id = id.to_string();
            self.made = made;
            self.tab = None;
            self.all_events = false;
            self.all_reports = false;
            // the other card's screenshots go with it
            self.shots.update(cx, |shots, cx| shots.forget(window, cx));
            self.task_open = false;
            self.task_height = None;
            self.pr_opened = Opened::default();
            self.item = None;
            self.item_at = 0;
            self.reveal = false;
            self.notes.clear();
            self.story_scroll.set_offset(Default::default());
            // what you tell Claude is the other card's now
            self.note = None;
            self.whole = None;
            self.whole_open = false;
            self.taken = None;
            self.facts_scroll.set_offset(Default::default());
            cx.notify();
        }
    }

    /// The page changed: the card follows the index; one that left the board (its ticket is no longer yours) closes.
    fn follow(&mut self, page: &Entity<WorkPage>, window: &mut Window, cx: &mut Context<Self>) {
        if self.left {
            return;
        }
        if page.read(cx).card() != Some(self.id.as_str()) {
            self.left = true;
            window.close_dialog(cx);
            return;
        }
        let work = self.work.read(cx);
        if !self.made.still(work)
            && let Some(made) = Made::of(work, &self.id)
        {
            self.made = made;
            cx.notify();
        }
    }

    /// The task of your own this card is, as the index has it now.
    fn own_task(&self, cx: &App) -> Option<enkidu_core::work::model::OwnTask> {
        let number = self.made.model.own.as_ref()?.number;
        self.work.read(cx).snapshot.as_ref()?.own_tasks.iter().find(|task| task.id == number).cloned()
    }

    /// Edit, on a task of your own: its dialog, in front of the card.
    fn edit_own(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(task), Some(frame)) = (self.own_task(cx), self.frame.clone()) else { return };
        crate::work::own_task::open(self.work.clone(), &frame, Some(task), None, None, window, cx);
    }

    /// A state pressed in the box of a task of your own: it is in that state at once, and its card moves to that
    /// column.
    fn set_own_state(&mut self, state: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(task) = self.own_task(cx).filter(|task| task.state != state) else { return };
        let draft = enkidu_core::commands::OwnTaskDraft {
            id: Some(task.id),
            project_id: task.project_id.clone(),
            title: task.title.clone(),
            notes: task.notes.clone(),
            state: state.to_string(),
        };
        let keeping = Work::save_own_task(&self.work, draft, task.project.clone(), cx);
        cx.spawn_in(window, async move |_, cx: &mut gpui_kit::AsyncWindowContext| {
            if let Err(why) = keeping.await {
                let _ = cx.update(|window, cx| toast::warning(window, "Couldn't change the task", why, cx));
            }
        })
        .detach();
    }

    /// Esc: the keyboard leaves a field for the card first (your words stay); from the card itself it closes.
    fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        // a screenshot shown large closes first, not the card under it
        if self.shots.read(cx).is_open() {
            self.shots.update(cx, |shots, cx| shots.close(window, cx));
            return false;
        }
        if self.focus.is_focused(window) || !self.focus.contains_focused(window, cx) {
            return true;
        }
        self.focus.focus(window, cx);
        false
    }

    /// The dialog closed by itself (Esc, a click beside it): the page has no open card now.
    fn closed(&mut self, cx: &mut Context<Self>) {
        self.left = true;
        let id = self.id.clone();
        let _ = self.page.update(cx, |page, cx| page.card_closed(&id, cx));
    }

    /// The page closes the card (it knows it did).
    pub(crate) fn leave(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.left {
            self.left = true;
            window.close_dialog(cx);
        }
    }

    /// Closes the card.
    pub(crate) fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.left {
            window.close_dialog(cx);
            self.closed(cx);
        }
    }

    /// Hide, or Unhide: the card leaves the view it was in, so it closes too.
    fn hide(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let hidden = self.made.model.hidden;
        Work::hide(&self.work, vec![self.id.clone()], !hidden, window.window_handle(), cx);
        self.close(window, cx);
    }

    fn pick_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        // a PR's tab starts as it is first seen, each time
        if self.tab.as_ref() != Some(&tab) {
            self.pr_opened = Opened::default();
        }
        self.tab = Some(tab);
        cx.notify();
    }

    /// What it is, Hide, Open in Jira and Close; the title; the tabs when there are two or more.
    fn header(&self, cx: &mut Context<Self>) -> Div {
        let model = &self.made.model;
        let what = match &model.head {
            CardHead::Ticket { kind, type_name, key, .. } => div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(type_icon(kind.name(), 14.))
                .children(type_name.clone().map(|name| div().text_size(px(12.5)).text_color(theme::INK_3).child(name)))
                .child(
                    mono(div())
                        .id("task-key")
                        .test_support()
                        .aria_label(SharedString::from(key.clone()))
                        .text_size(px(12.5))
                        .text_color(theme::INK_2)
                        .child(key.clone()),
                ),
            CardHead::Pr { state, name } => {
                div().flex().items_center().gap(px(8.)).child(pr_glyph(pr_state(*state), 14.)).child(
                    mono(div())
                        .id("task-key")
                        .test_support()
                        .aria_label(SharedString::from(name.clone()))
                        .text_size(px(12.5))
                        .text_color(theme::INK_2)
                        .child(name.clone()),
                )
            }
            // a task of your own: signal blue is what is yours; then the project it is for
            CardHead::Own { number, project } => div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(tinted("icons/user-round.svg", theme::ACCENT, 14.))
                .child(
                    div()
                        .id("task-key")
                        .test_support()
                        .aria_label(SharedString::from(format!("Your task {number}")))
                        .text_size(px(12.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme::ACCENT)
                        .child("Your task"),
                )
                .child(div().text_size(px(12.5)).text_color(theme::INK_3).child(project.clone())),
        };
        // its words and its state are yours to change: Edit opens its dialog in front of the card
        let edit = model.own.as_ref().map(|_| {
            quiet_button("task-edit", "Edit")
                .tipped("Change its words, or delete it", None)
                .child(svg().path("icons/pencil.svg").flex_none().size(px(12.)).text_color(theme::INK_3))
                .child("Edit")
                .on_click(cx.listener(|card, _, window, cx| card.edit_own(window, cx)))
        });
        let hidden = model.hidden;
        let (hide_icon, hide_words, hide_tip) = match hidden {
            true => ("icons/eye.svg", "Unhide", "Put it back on the board"),
            false => ("icons/eye-off.svg", "Hide", "Take it off the board (Hidden, in the header, shows it again)"),
        };
        let hide = quiet_button("task-hide", hide_words)
            .tipped(hide_tip, None)
            .child(svg().path(hide_icon).flex_none().size(px(12.)).text_color(theme::INK_3))
            .child(hide_words)
            .on_click(cx.listener(|card, _, window, cx| card.hide(window, cx)));
        // a PR without a ticket has its link in its box below: never twice
        let jira = model.jira_url.clone().map(|url| open_link("detail-open", "Open in Jira", url));
        let close = div()
            .id("task-close")
            .test_support()
            .role(Role::Button)
            .aria_label("Close")
            .ml(px(4.))
            .size(px(28.))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(px(7.))
            .cursor_pointer()
            .hover(|s| s.bg(theme::HOVER))
            .child(svg().path("icons/x.svg").size(px(15.)).text_color(theme::INK_3))
            .on_click(cx.listener(|card, _, window, cx| card.close(window, cx)));
        let first = div()
            .h(px(24.))
            .flex()
            .items_center()
            .gap(px(8.))
            .child(what)
            .when(hidden, |line| {
                line.child(
                    div()
                        .id("task-hidden")
                        .test_support()
                        .aria_label("Hidden")
                        .rounded(px(5.))
                        .bg(theme::HOVER)
                        .px(px(6.))
                        .py(px(1.))
                        .text_size(px(11.))
                        .text_color(theme::INK_3)
                        .child("Hidden"),
                )
            })
            .child(div().flex_1())
            .children(edit)
            .child(hide)
            .children(jira)
            .child(close);
        let title = div()
            .id("task-title")
            .test_support()
            .aria_label(SharedString::from(model.title.clone()))
            .mt(px(8.))
            .max_w(px(780.))
            .text_size(px(19.))
            .line_height(px(26.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme::INK)
            .child(model.title.clone());
        let current = self.tab().cloned();
        let tabs = (model.tabs.len() > 1).then(|| {
            div()
                .id("task-tabs")
                .role(Role::TabList)
                .aria_label("Show")
                .mt(px(12.))
                .flex()
                .items_center()
                .gap(px(2.))
                .children(model.tabs.iter().map(|tab| {
                    let on = current.as_ref() == Some(tab);
                    let pick = tab.clone();
                    div()
                        .id(tab_id(tab))
                        .test_support()
                        .role(Role::Tab)
                        .aria_label(SharedString::from(tab.label()))
                        .aria_selected(on)
                        .h(px(26.))
                        .flex()
                        .flex_none()
                        .items_center()
                        .rounded(px(8.))
                        .px(px(10.))
                        .text_size(px(12.5))
                        .cursor_pointer()
                        .map(|tab| match on {
                            true => {
                                tab.bg(theme::SELECTED).font_weight(FontWeight::MEDIUM).text_color(theme::INK).child(
                                    div().absolute().top_0().left(px(6.)).right(px(6.)).h(px(1.)).bg(theme::HIGHLIGHT),
                                )
                            }
                            false => tab.text_color(theme::INK_3).hover(|s| s.bg(theme::HOVER).text_color(theme::INK)),
                        })
                        .relative()
                        .child(tab.label())
                        .on_click(cx.listener(move |card, _, _, cx| card.pick_tab(pick.clone(), cx)))
                }))
        });
        div().flex_none().px(px(24.)).pt(px(18.)).pb(px(12.)).child(first).child(title).children(tabs)
    }

    /// The timeline: what happened to the ticket and its PRs, the newest first; the first eight, then "Show all".
    fn activity(&self, cx: &mut Context<Self>) -> AnyElement {
        let timeline = &self.made.model.timeline;
        let shown = if self.all_events { timeline.len() } else { timeline.len().min(TIMELINE_SHOWN) };
        let rows = timeline.iter().take(shown).enumerate().map(|(n, (when, text))| {
            div()
                .id(("timeline-row", n))
                .test_support()
                .aria_label(SharedString::from(format!("{when}: {text}")))
                .flex()
                .gap(px(12.))
                .text_size(px(12.5))
                .line_height(px(18.))
                .child(div().w(px(76.)).flex_none().text_color(theme::INK_3).child(when.clone()))
                .child(div().min_w_0().flex_1().text_color(theme::INK_2).child(text.clone()))
        });
        let more = (timeline.len() > shown).then(|| {
            let words = format!("Show all {}", timeline.len());
            div()
                .id("timeline-more")
                .test_support()
                .role(Role::Button)
                .aria_label(SharedString::from(words.clone()))
                .mt(px(8.))
                .text_size(px(12.))
                .text_color(theme::INK_3)
                .cursor_pointer()
                .hover(|s| s.text_color(theme::ACCENT))
                .child(words)
                .on_click(cx.listener(|card, _, _, cx| {
                    card.all_events = true;
                    cx.notify();
                }))
        });
        div()
            .id("detail-timeline")
            .test_support()
            .aria_label("Activity")
            .child(div().flex().flex_col().gap(px(6.)).children(rows))
            .children(more)
            .into_any_element()
    }

    /// The left side: the story of the tab on screen.
    fn story(&self, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        match self.tab() {
            Some(Tab::Activity) => vec![self.activity(cx)],
            Some(Tab::Overview) => self.overview(window, cx),
            Some(Tab::Pr { pr, .. }) => {
                let tab = self.made.prs.iter().find(|(at, _)| at == pr).map(|(_, tab)| tab.clone());
                tab.map(|tab| pr_tab(&tab, self.pr_opened, window, cx)).unwrap_or_default()
            }
            None => Vec::new(),
        }
    }

    /// Overview is the story: how Claude says it ended (what you open the task for after a night's run), what the
    /// task is, then what needs you. A long description shows its start, so both are in view.
    fn overview(&self, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut sections: Vec<AnyElement> = Vec::new();
        sections.extend(self.reports(window, cx));
        if let Some(blocks) = &self.made.task {
            sections.push(self.task(blocks, window, cx));
        }
        sections.extend(self.needs(window, cx));
        sections
    }

    /// "The task": Jira's description in the page's own type; a long one folded to its start, fading out, with a
    /// button for the rest.
    fn task(&self, blocks: &[Block], window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let limit = if self.made.items.is_empty() { TASK_TALL } else { TASK_SHORT };
        // until it was drawn once its height isn't known: it starts as a long one does, without the button
        let long = self.task_height.is_some_and(|height| height > px(limit + TASK_SLACK));
        let folded = !self.task_open && (long || self.task_height.is_none());
        let card = cx.entity().downgrade();
        let whole =
            div().relative().flex_none().max_w(px(TASK_WIDTH)).child(rich("jira-doc", blocks, window, cx)).on_prepaint(
                move |bounds, _, cx| {
                    let _ = card.update(cx, |card, cx| {
                        if card.task_height != Some(bounds.size.height) {
                            card.task_height = Some(bounds.size.height);
                            cx.notify();
                        }
                    });
                },
            );
        let fade = (folded && long).then(|| {
            let (clear, solid) = (alpha(theme::RAISED, 0.), theme::RAISED);
            div().absolute().left_0().right_0().bottom_0().h(px(56.)).bg(linear_gradient(
                180.,
                linear_color_stop(clear, 0.),
                linear_color_stop(solid, 1.),
            ))
        });
        let more = long.then(|| {
            let words = if self.task_open { "Show less" } else { "Show the whole description" };
            div()
                .id("task-description-more")
                .test_support()
                .role(Role::Button)
                .aria_label(words)
                .mt(px(6.))
                .flex()
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme::INK_3)
                        .cursor_pointer()
                        .hover(|s| s.text_color(theme::ACCENT))
                        .child(words),
                )
                .on_click(cx.listener(|card, _, _, cx| {
                    card.task_open = !card.task_open;
                    cx.notify();
                }))
        });
        div()
            .id("task-description")
            .test_support()
            .aria_label("The task")
            .aria_description(if folded && long { "Its start" } else { "All of it" })
            .flex_none()
            .child(heading("The task", None))
            .child(
                div()
                    .id("task-description-text")
                    .test_support()
                    .relative()
                    .when(folded, |text| text.max_h(px(limit)).overflow_hidden())
                    .child(whole)
                    .children(fade),
            )
            .children(more)
            .into_any_element()
    }

    /// The right side, always in this order: Jira, each pull request (or that there is none yet), Claude.
    fn facts(&self, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let model = &self.made.model;
        let mut boxes: Vec<AnyElement> = Vec::new();
        match &model.jira {
            Some(JiraBox::Read(jira)) => boxes.push(jira_box(jira)),
            Some(JiraBox::NotRead) => boxes.push(
                fact_box("task-jira", "Jira", JIRA_NOT_READ.to_string(), None)
                    .child(div().text_size(px(12.5)).line_height(px(18.)).text_color(theme::INK_3).child(JIRA_NOT_READ))
                    .into_any_element(),
            ),
            None => {}
        }
        // a task of your own: its state, which is yours to set, the project it is for, and when you made it
        if let Some(own) = &model.own {
            let card = cx.entity().downgrade();
            let states = crate::work::own_task::state_row(
                "task-own-state",
                &own.state,
                move |state, window, cx| {
                    let _ = card.update(cx, |card, cx| card.set_own_state(state, window, cx));
                },
                window,
                cx,
            );
            let state_words = enkidu_model::work::board::own_state(&own.state).1;
            let said = format!("State {state_words}, project {}, made {}", own.project, own.made);
            let (project, project_value) = fact("task-own-project", "Project", &own.project);
            let (made, made_value) = fact("task-own-made", "Made", &own.made);
            boxes.push(
                fact_box("task-own", "Your task", said, None)
                    .child(div().id("task-own-states").test_support().aria_label(state_words).pb(px(4.)).child(states))
                    .child(project.child(project_value.child(own.project.clone())))
                    .child(made.child(made_value.child(own.made.clone())))
                    .into_any_element(),
            );
        }
        // (a task of your own has no pull request to wait for)
        if model.prs.is_empty() && model.own.is_none() {
            let words = "No pull request yet.";
            boxes.push(
                fact_box("task-pr", "Pull request", words.to_string(), None)
                    .child(
                        div()
                            .id("task-no-pr")
                            .test_support()
                            .aria_label(words)
                            .text_size(px(12.5))
                            .line_height(px(18.))
                            .text_color(theme::INK_2)
                            .child(words),
                    )
                    .into_any_element(),
            );
        }
        for pr in &model.prs {
            boxes.push(pr_box(pr, window, cx));
        }
        // which Claude the message goes to: there when one is linked to the card or the project has a workspace
        if let Some(claudes) = self.claudes(cx) {
            boxes.push(self.claude_box(&claudes, window, cx));
        }
        boxes
    }
}

/// A section's heading on the left, with the count of what is under it when that says something.
pub(super) fn heading(words: &'static str, count: Option<usize>) -> Div {
    div()
        .mb(px(10.))
        .flex()
        .items_center()
        .gap(px(8.))
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme::INK_3)
        .child(words)
        .children(count.filter(|count| *count > 0).map(|count| {
            enkidu_ui::pieces::tabular(div())
                .rounded_full()
                .bg(theme::HOVER)
                .px(px(6.))
                .text_size(px(11.))
                .font_weight(FontWeight::NORMAL)
                .text_color(theme::INK_2)
                .child(count.to_string())
        }))
}

/// A tab's id on screen: `task-tab:overview`, `task-tab:pr:2414`, `task-tab:activity`.
pub fn tab_id(tab: &Tab) -> SharedString {
    SharedString::from(match tab {
        Tab::Overview => "task-tab:overview".to_string(),
        Tab::Pr { number, .. } => format!("task-tab:pr:{number}"),
        Tab::Activity => "task-tab:activity".to_string(),
    })
}

/// Opens a page of the Work index (a ticket, a PR, a comment, a check's log) in the browser, never in the app. Only
/// web links (https), as the core's own rule says.
pub fn open_in_browser(url: &str, window: &mut Window, cx: &mut App) {
    let ok = url.starts_with("https://") && !url.contains(char::is_whitespace) && url.len() < 4096;
    match ok {
        true => cx.open_url(url),
        false => toast::warning(window, "Couldn't open the link", "Only web links (https) open from here", cx),
    }
}

/// A quiet button in the card's header (Hide): its words, an icon before them, lit when pointed at.
fn quiet_button(id: &'static str, label: &'static str) -> enkidu_ui::pieces::Pressed {
    enkidu_ui::press::pressable(div().id(id))
        .test_support()
        .role(Role::Button)
        .aria_label(label)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(4.))
        .rounded(px(6.))
        .px(px(6.))
        .py(px(2.))
        .text_size(px(12.))
        .text_color(theme::INK_3)
        .cursor_pointer()
        .hover(|s| s.bg(theme::HOVER).text_color(theme::INK))
}

/// A link that opens in the browser: its words, then the sign for "outside".
pub(super) fn open_link(id: impl Into<gpui_kit::ElementId>, label: &'static str, url: String) -> Observed {
    let group = SharedString::from(format!("open-link:{url}"));
    div()
        .id(id)
        .test_support()
        .role(Role::Link)
        .aria_label(label)
        .aria_description(SharedString::from(url.clone()))
        .group(group.clone())
        .flex()
        .flex_none()
        .items_center()
        .gap(px(4.))
        .rounded(px(6.))
        .px(px(6.))
        .py(px(2.))
        .text_size(px(12.))
        .text_color(theme::INK_3)
        .cursor_pointer()
        .hover(|s| s.bg(theme::HOVER).text_color(theme::ACCENT))
        .child(label)
        .child(
            svg()
                .path("icons/external-link.svg")
                .flex_none()
                .size(px(11.))
                .text_color(theme::INK_3)
                .group_hover(group, |s| s.text_color(theme::ACCENT)),
        )
        .on_click(move |_, window, cx| open_in_browser(&url, window, cx))
}

/// A box of facts at the right: its title, something to do at its right, then its facts. `said` is all of it in
/// words.
pub(super) fn fact_box(
    id: impl Into<gpui_kit::ElementId>,
    title: &'static str,
    said: String,
    action: Option<AnyElement>,
) -> Observed {
    div()
        .id(id)
        .test_support()
        .aria_label(title)
        .aria_description(SharedString::from(said))
        .flex_none()
        .rounded(px(12.))
        .border_1()
        .border_color(theme::RULE)
        .bg(alpha(theme::CANVAS, 0.7))
        .px(px(14.))
        .pb(px(12.))
        .pt(px(10.))
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(
            div()
                .mb(px(2.))
                .h(px(24.))
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(12.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::INK_3)
                        .child(title),
                )
                .children(action),
        )
}

/// A fact: what it is, then how it stands.
fn fact(id: impl Into<gpui_kit::ElementId>, label: &'static str, said: &str) -> (Observed, Div) {
    let row = div()
        .id(id)
        .test_support()
        .aria_label(SharedString::from(said.to_string()))
        .min_h(px(24.))
        .flex()
        .items_start()
        .gap(px(12.))
        .text_size(px(12.5))
        .line_height(px(18.))
        .child(div().w(px(68.)).flex_none().pt(px(3.)).text_color(theme::INK_3).child(label));
    let value =
        div().min_w_0().flex_1().flex().flex_wrap().items_center().gap(px(6.)).pt(px(3.)).text_color(theme::INK_2);
    (row, value)
}

/// A Jira status as a pill in the hue of its column's ring (to do and on hold gray, in progress blue, review
/// violet, QA amber, done green): `large` as the card's right column shows it, else small, beside a key.
pub(super) fn status_pill(status: &str, hue: Option<f32>, large: bool) -> Div {
    let (height, radius, side, text) = if large { (24., 7., 10., 12.5) } else { (18., 5., 6., 11.) };
    let pill = div()
        .h(px(height))
        .flex()
        .flex_none()
        .items_center()
        .rounded(px(radius))
        .px(px(side))
        .text_size(px(text))
        .font_weight(FontWeight::MEDIUM)
        .child(status.to_string());
    match hue {
        Some(hue) => pill.bg(alpha(tone(hue, 0.12), 0.12)).text_color(tone(hue, 0.12)),
        None => pill.bg(theme::HOVER).text_color(theme::INK_2),
    }
}

/// Jira's box: the status as a large pill, the type, the priority, who has it, when it last changed.
fn jira_box(jira: &JiraFacts) -> AnyElement {
    let mut said = vec![format!("Status {}", jira.status)];
    let (status, value) = fact("task-status", "Status", &jira.status);
    let mut rows = vec![status.child(value.child(status_pill(&jira.status, jira.hue, true))).into_any_element()];
    if let Some((name, kind)) = &jira.issue_type {
        said.push(format!("Type {name}"));
        let (row, value) = fact("task-type", "Type", name);
        rows.push(row.child(value.child(type_icon(kind.name(), 12.)).child(name.clone())).into_any_element());
    }
    if let Some((name, level)) = &jira.priority {
        said.push(format!("Priority {name}"));
        let (row, value) = fact("task-priority", "Priority", name);
        rows.push(
            row.child(value.children(level.map(|level| priority_icon(level, 12.))).child(name.clone()))
                .into_any_element(),
        );
    }
    said.push(format!("Assignee {}", jira.assignee));
    let (row, value) = fact("task-assignee", "Assignee", &jira.assignee);
    rows.push(row.child(value.child(jira.assignee.clone())).into_any_element());
    said.push(format!("Updated {}", jira.updated));
    let (row, value) = fact("task-updated", "Updated", &jira.updated);
    rows.push(row.child(value.child(jira.updated.clone())).into_any_element());
    fact_box("task-jira", "Jira", said.join(", "), None).children(rows).into_any_element()
}

/// A pull request's box: its number and state, its title, its branch and base, then how it stands (checks, review,
/// a merge conflict, open threads, or when it was merged), and its GitHub link.
fn pr_box(pr: &PrFacts, window: &mut Window, cx: &mut App) -> AnyElement {
    let n = pr.number;
    let id = |what: &str| SharedString::from(format!("{what}:{n}"));
    let state = pr.state.label();
    let mut said = vec![format!("#{n} {state}"), pr.title.clone(), format!("{} into {}", pr.head, pr.base)];
    let first = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(pr_glyph(pr_state(pr.state), 15.))
        .child(
            mono(div())
                .text_size(px(13.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::INK)
                .child(format!("#{n}")),
        )
        .child(
            div()
                .id(id("task-pr-state"))
                .test_support()
                .aria_label(state)
                .rounded(px(6.))
                .bg(theme::HOVER)
                .px(px(6.))
                .py(px(2.))
                .text_size(px(11.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(ink(pr.tone).color())
                .child(state),
        );
    let title =
        div().text_size(px(12.5)).line_height(px(18.)).text_color(theme::INK_2).line_clamp(2).child(pr.title.clone());
    let branch = mono(div())
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(6.))
        .pb(px(4.))
        .text_size(px(11.5))
        .text_color(theme::INK_3)
        .child(tinted("icons/git-branch.svg", theme::INK_3, 11.))
        .child(div().truncate().child(pr.head.clone()))
        .child(tinted("icons/arrow-right.svg", theme::INK_3, 11.))
        .child(div().flex_none().child(pr.base.clone()));
    let mut rows: Vec<AnyElement> = vec![first.into_any_element(), title.into_any_element(), branch.into_any_element()];
    if let Some((checks, words)) = &pr.checks {
        said.push(format!("Checks: {words}"));
        let look = match checks {
            Checks::Failing => SourceLook::Error,
            Checks::Running => SourceLook::Reading,
            _ => SourceLook::Ok,
        };
        let (row, value) = fact(id("task-checks"), "Checks", words);
        let color = if *checks == Checks::Failing { theme::LAMP_RED } else { theme::INK_2 };
        rows.push(
            row.child(
                value
                    // the dot pops once when the checks flip while the card is open
                    .child(checks_dot(
                        id("task-checks-lamp"),
                        look,
                        7.,
                        changes_seen(id("task-checks-seen"), *checks as u64, window, cx),
                    ))
                    .child(div().min_w_0().text_color(color).child(words.clone())),
            )
            .into_any_element(),
        );
    }
    if let Some((words, tone)) = &pr.review {
        said.push(format!("Review: {words}"));
        let (row, value) = fact(id("task-review"), "Review", words);
        let color = match tone {
            enkidu_model::work::board::Tone::Muted => theme::INK_2,
            tone => ink(*tone).color(),
        };
        rows.push(row.child(value.child(div().text_color(color).child(words.clone()))).into_any_element());
    }
    if let Some(words) = &pr.conflict {
        said.push(words.clone());
        let (row, value) = fact(id("task-merge"), "Merge", words);
        rows.push(row.child(value.child(div().text_color(theme::LAMP_RED).child(words.clone()))).into_any_element());
    }
    if let Some(words) = &pr.threads {
        said.push(words.clone());
        let (row, value) = fact(id("task-threads"), "Threads", words);
        rows.push(row.child(value.child(words.clone())).into_any_element());
    }
    if let Some(when) = &pr.merged {
        said.push(format!("Merged {when}"));
        let (row, value) = fact(id("task-merged"), "Merged", when);
        rows.push(row.child(value.child(when.clone())).into_any_element());
    }
    let github = open_link(id("task-pr-open"), "GitHub", pr.url.clone()).into_any_element();
    fact_box(id("task-pr"), "Pull request", said.join(" | "), Some(github)).children(rows).into_any_element()
}

impl Render for Card {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.settle_needs(window, cx);
        self.settle_composer(window, cx);
        self.settle_reports(cx);
        let has_claudes = self.claudes(cx).is_some_and(|the| !the.rows.is_empty());
        let composer = self.composer(has_claudes, window, cx);
        let model = self.made.model.clone();
        let story = model.story;
        // never taller than nine tenths of the window: the header and the foot stay, each side scrolls by itself
        let tallest = f32::from(window.viewport_size().height) * (1. - 2. * ABOVE);
        let facts = div()
            .id("task-facts")
            .test_support()
            .aria_label("How it stands")
            .flex()
            .flex_col()
            .gap(px(12.))
            .px(px(16.))
            .py(px(20.))
            .overflow_y_scroll()
            .track_scroll(&self.facts_scroll)
            .children(self.facts(window, cx))
            .map(|facts| match story {
                true => facts
                    .w(px(FACTS))
                    .flex_none()
                    .border_l_1()
                    .border_color(theme::RULE)
                    .bg(alpha(gpui_kit::rgb(0x000000), 0.12)),
                false => facts.flex_1().min_w_0(),
            });
        let body = div()
            .flex_1()
            .min_h_0()
            .flex()
            .border_t_1()
            .border_color(theme::RULE)
            .when(story, |body| {
                body.child(
                    div()
                        .id("task-story")
                        .test_support()
                        .aria_label(SharedString::from(self.tab().map(Tab::label).unwrap_or_default()))
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(24.))
                        .px(px(24.))
                        .py(px(20.))
                        .overflow_y_scroll()
                        .track_scroll(&self.story_scroll)
                        .children(self.story(window, cx)),
                )
            })
            .child(facts);
        div()
            .id("task-dialog")
            .test_support()
            .aria_label(SharedString::from(model.title.clone()))
            .aria_description(SharedString::from(format!("{}, {}", self.id, if story { "wide" } else { "narrow" })))
            .key_context(KEYS)
            .track_focus(&self.focus)
            // ⌘C on words selected in it (a description, a comment, a report)
            .on_action(|_: &gpui_kit::base::input::Copy, window, cx| {
                if !crate::work::rich::copy_selection(window, cx) {
                    cx.propagate();
                }
            })
            // in front of a card: j and k (the arrows) move through what needs you, x leaves it out of the message
            // or puts it back, n writes its instruction, e marks it done, z snoozes it, ↩ opens where it was said
            .on_action(cx.listener(|card, _: &NextItem, _, cx| card.step_item(1, cx)))
            .on_action(cx.listener(|card, _: &PrevItem, _, cx| card.step_item(-1, cx)))
            .on_action(cx.listener(|card, _: &ToggleItem, _, cx| card.act_on_item(cx, Card::toggle_include)))
            .on_action(cx.listener(|card, _: &ItemNote, window, cx| card.write_note(window, cx)))
            .on_action(cx.listener(|card, _: &ItemDone, window, cx| card.set_current(ItemState::Done, window, cx)))
            .on_action(cx.listener(|card, _: &ItemSnooze, window, cx| card.set_current(ItemState::Snoozed, window, cx)))
            .on_action(cx.listener(|card, _: &OpenItem, window, cx| card.open_current(window, cx)))
            // s or c writes to Claude; ⌘↩ types the message into its prompt
            .on_action(cx.listener(|card, _: &TellClaude, window, cx| card.write_to_claude(window, cx)))
            .on_action(cx.listener(|card, _: &SendTask, window, cx| card.send(None, window, cx)))
            .relative()
            .w_full()
            .max_h(px(tallest))
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(self.header(cx))
            .child(body)
            // what you tell Claude, always in view
            .child(composer)
            .child(crate::dialog_parts::lit_edge())
            // a screenshot as large as the window, over the card
            .child(self.shots.clone())
    }
}
