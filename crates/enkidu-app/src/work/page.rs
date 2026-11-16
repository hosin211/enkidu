//! The Work page itself (DESIGN.md, "The Work page"): it takes the place of the workspace's tabs and islands, under the
//! projects row and over the status line. A header row, then the board; a source that can't be read says so first;
//! before anything was read, it says what is connected and offers to check.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use enkidu_core::db::Workspace;
use enkidu_core::work::flows::Flow;
use enkidu_model::frame::Frame;
use enkidu_model::ui::Ui;
use enkidu_model::work::board::Direction;
use enkidu_model::work::flows::new_flow;
use enkidu_model::work::page::{BoardView, Choices, Quiet, board_view, moved_cards};
use enkidu_model::work::{self, Broken};
use enkidu_ui::pieces::{
    ButtonKind, ButtonSize, Part, button, button_with, island, lit_edge, raised, rise_in_place, sentence, with_code,
};
use enkidu_ui::tips::Tipped as _;
use enkidu_ui::tokens::{self as theme, alpha};
use enkidu_ui::work::SourceLook;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Context, Entity, FocusHandle, FontWeight, IntoElement, ListAlignment, ListState, Render,
    ScrollHandle, SharedString, Subscription, TextAlign, Window, actions, div, linear_color_stop, linear_gradient,
    list, px, svg,
};

use crate::terminals::Terminals;
use crate::work::board;
use crate::work::card::{self, Card};
use crate::work::drafts::Drafts;
use crate::work::flow_editor::{self, FlowEditor};
use crate::work::flows_page::FlowsPage;
use crate::work::store::Work;

/// The space between the window's edge and the page's body, on each side.
const GUTTER: f32 = 8.;

/// The page's own keys work while the keyboard is on it.
pub const KEYS: &str = "WorkPage";
/// The board's plain keys (the arrows and h j k l, ↩, X, /) work only while no text field has the keyboard: in the
/// filter they are letters.
pub const BOARD_KEYS: &str = "WorkPage && !Input";
/// In the filter's field ↓ leaves it for the board's first card.
pub const FILTER_KEYS: &str = "WorkPage > Input";

actions!(work_page, [Back, FocusFilter, MoveUp, MoveDown, MoveLeft, MoveRight, OpenCard, PickCard, FilterDown]);

/// What a press on a card asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    /// A plain click: the card opens in front of the board.
    Open,
    /// Its round box, ⌘ with a click, or a click while you pick: the card is picked, or let go.
    Pick,
    /// ⇧ with a click: every card from the last one picked to this one.
    PickRun,
}

pub struct WorkPage {
    frame: Entity<Frame>,
    pub(crate) work: Entity<Work>,
    /// What the window keeps of your choices here (Needs you).
    ui: Entity<Ui>,
    /// What you wrote about each card, kept until it goes.
    drafts: Entity<Drafts>,
    /// The window's terminals: a card's message is typed into a Claude's prompt there.
    terminals: Option<Entity<Terminals>>,
    focus: FocusHandle,
    /// The board as it is shown: built when the index, the lamps or your choices change, never while drawing.
    pub(crate) board: Rc<BoardView>,
    /// Each column's list (by the column's id): it draws only the cards in view, and keeps where it is scrolled to.
    lists: HashMap<String, ListState>,
    /// The cards you hid, in place of the board.
    show_hidden: bool,
    /// The filter's text.
    pub(crate) query: String,
    /// Jira's boards were asked for ahead of the board menu (once, when the page first shows that Jira is read).
    pub(crate) boards_asked: bool,
    /// The filter's field, made when it is first drawn, and the hearing of what is typed in it.
    pub(crate) filter: Option<(Entity<InputState>, Subscription)>,
    /// The card the keyboard is on.
    selected: Option<String>,
    /// The cards you picked (to hide them, or unhide them), in the order you picked them.
    picked: Vec<String>,
    /// The card open in front of the board, and its dialog.
    card: Option<String>,
    card_view: Option<Entity<Card>>,
    /// The workflow dialog, while it is open.
    flow_editor: Option<Entity<FlowEditor>>,
    /// The Workflows page, and whether it stands in the board's place now. It keeps which workflow or run it shows
    /// while the board is back.
    flows: Entity<FlowsPage>,
    flows_open: bool,
    /// Where the board is scrolled to, sideways.
    scroll: ScrollHandle,
    /// The cards that just moved to another column: each glows once. `moves` counts the times some did (the glow's
    /// name, and what its clearing checks).
    moved: Rc<HashSet<String>>,
    moves: u64,
}

impl WorkPage {
    pub fn new(
        frame: Entity<Frame>,
        work: Entity<Work>,
        ui: Entity<Ui>,
        drafts: Entity<Drafts>,
        terminals: Option<Entity<Terminals>>,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&work, |page, _, cx| page.rebuild(cx)).detach();
        // of what you wrote only the Claude you picked for a card shows on the board
        let mut chosen = drafts.read(cx).chosen();
        cx.observe(&drafts, move |page, drafts, cx| {
            let now = drafts.read(cx).chosen();
            if now != chosen {
                chosen = now;
                page.rebuild(cx);
            }
        })
        .detach();
        // of the layout only Needs you matters here
        let mut needs_you = ui.read(cx).work_needs_you;
        cx.observe(&ui, move |page, ui, cx| {
            let now = ui.read(cx).work_needs_you;
            if now != needs_you {
                needs_you = now;
                page.rebuild(cx);
            }
        })
        .detach();
        let flows = {
            let (page, work) = (cx.entity().downgrade(), work.clone());
            cx.new(|cx| FlowsPage::new(page, work, cx))
        };
        let mut page = WorkPage {
            frame,
            work,
            ui,
            drafts,
            terminals,
            focus: cx.focus_handle(),
            board: Rc::new(BoardView::default()),
            lists: HashMap::new(),
            show_hidden: false,
            query: String::new(),
            boards_asked: false,
            filter: None,
            selected: None,
            picked: Vec::new(),
            card: None,
            card_view: None,
            flow_editor: None,
            flows,
            flows_open: false,
            scroll: ScrollHandle::new(),
            moved: Rc::new(HashSet::new()),
            moves: 0,
        };
        page.rebuild(cx);
        page
    }

    /// The cards that glow now because they just moved to another column (a test reads them).
    pub fn moved(&self) -> Vec<String> {
        let mut moved: Vec<String> = self.moved.iter().cloned().collect();
        moved.sort();
        moved
    }

    /// The board as it is shown now.
    pub fn board(&self) -> &BoardView {
        &self.board
    }

    /// Builds the board from the index, the lamps and your choices (a fraction of a millisecond for an index of usual
    /// size), and draws again.
    pub(crate) fn rebuild(&mut self, cx: &mut Context<Self>) {
        let choices = Choices {
            needs_you: self.ui.read(cx).work_needs_you,
            show_hidden: self.show_hidden,
            query: self.query.clone(),
        };
        let work = self.work.read(cx);
        let board = match &work.snapshot {
            Some(index) => {
                let (workspaces, statuses) = work.built_from();
                let chosen = self.drafts.read(cx).chosen();
                board_view(index, &work.groups, workspaces, statuses, &choices, &chosen, work.now())
            }
            None => BoardView::default(),
        };
        // each column's list follows its cards: the same cards keep their place, changed ones are measured again,
        // and a column whose cards came or went starts from where it was scrolled to
        self.lists.retain(|id, _| board.columns.iter().any(|column| &column.id == id));
        for column in &board.columns {
            let before = self.board.columns.iter().find(|old| old.id == column.id);
            let count = column.cards.len();
            // every card of a column is measured once when its cards come or go, so the column can be scrolled to
            // any place at once; after that only the cards in view are drawn
            let list = (self.lists.entry(column.id.clone()))
                .or_insert_with(|| ListState::new(0, ListAlignment::Top, px(board::OVERDRAW)).measure_all());
            let same_cards = before.is_some_and(|old| {
                old.cards.len() == count && old.cards.iter().zip(&column.cards).all(|(a, b)| a.id == b.id)
            });
            if !same_cards || list.item_count() != count {
                let top = list.logical_scroll_top();
                list.reset(count);
                list.scroll_to(top);
            } else if before.is_some_and(|old| old.cards != column.cards) {
                list.remeasure_items(0..count);
            }
        }
        // a card that stands in another column than before glows once, so you see it move; the glow's time over,
        // it is a card like the others (also when it is scrolled away and back)
        let moved = moved_cards(&self.board, &board);
        if !moved.is_empty() {
            self.moved = Rc::new(moved.into_iter().collect());
            self.moves += 1;
            let turn = self.moves;
            cx.spawn(async move |page: gpui_kit::WeakEntity<Self>, cx: &mut gpui_kit::AsyncApp| {
                cx.background_executor()
                    .timer(enkidu_ui::work::CARD_MOVED + std::time::Duration::from_millis(100))
                    .await;
                let _ = page.update(cx, |page, cx| {
                    if page.moves == turn {
                        page.moved = Rc::new(HashSet::new());
                        cx.notify();
                    }
                });
            })
            .detach();
        }
        self.board = Rc::new(board);
        // a card that left the board (hidden, filtered away, no longer yours) no longer has the keyboard
        if self.selected.as_ref().is_some_and(|id| !self.board.shown_ids().any(|shown| shown == id)) {
            self.selected = None;
        }
        // nor is one that is no longer yours picked, or open
        let cards = &self.board.cards;
        self.picked.retain(|id| cards.contains(id));
        if self.card.as_ref().is_some_and(|id| !cards.contains(id)) {
            // its dialog hears this and closes
            self.card = None;
            self.card_view = None;
        }
        // the last hidden task is back on the board: so are you
        if self.show_hidden && self.board.away == 0 && self.work.read(cx).snapshot.is_some() {
            self.show_hidden = false;
        }
        cx.notify();
    }

    /// The card the keyboard is on.
    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    /// The cards you picked, in the order you picked them.
    pub fn picked(&self) -> &[String] {
        &self.picked
    }

    /// The card open in front of the board.
    pub fn card(&self) -> Option<&str> {
        self.card.as_deref()
    }

    /// Its dialog.
    pub fn card_view(&self) -> Option<&Entity<Card>> {
        self.card_view.as_ref()
    }

    /// What you wrote about each card.
    pub fn drafts(&self) -> &Entity<Drafts> {
        &self.drafts
    }

    /// The workflow dialog, while it is open.
    pub fn flow_editor(&self) -> Option<&Entity<FlowEditor>> {
        self.flow_editor.as_ref()
    }

    /// "Workflow…" in the bar: a workflow for the cards you picked, in the order they are on the board, each to
    /// the Claude it goes to. Its dialog opens; nothing is saved yet.
    pub fn workflow_for_picked(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let order: Vec<&str> = self.board.shown_ids().collect();
        let mut picks = self.picked.clone();
        // a card that isn't on the board now (hidden) comes first
        picks.sort_by_key(|id| order.iter().position(|shown| shown == id).map_or(0, |at| at + 1));
        let flow = {
            let work = self.work.read(cx);
            let Some(index) = work.snapshot.as_deref() else { return };
            let cards: Vec<&enkidu_model::work::groups::Group> =
                picks.iter().filter_map(|id| work.groups.iter().find(|group| &group.id == id)).collect();
            let (workspaces, statuses) = work.built_from();
            let chosen = self.drafts.read(cx).chosen();
            new_flow(&cards, index, workspaces, statuses, &chosen, work.now(), &index.flow_kinds)
        };
        self.edit_flow(flow, picks, window, cx);
    }

    /// Opens the workflow dialog on a workflow: a new one for `picks`, or one you have, to change it.
    pub fn edit_flow(&mut self, flow: Flow, picks: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        if self.flow_editor.is_some() {
            return;
        }
        let (work, drafts) = (self.work.clone(), self.drafts.clone());
        self.flow_editor = Some(flow_editor::open(&cx.entity(), work, drafts, flow, picks, window, cx));
        cx.notify();
    }

    /// The workflow dialog closed.
    pub(crate) fn flow_editor_closed(&mut self, cx: &mut Context<Self>) {
        self.flow_editor = None;
        cx.notify();
    }

    /// A workflow was saved from the dialog: the picks are let go, and the Workflows page shows it next time.
    pub(crate) fn flow_saved(&mut self, id: &str, cx: &mut Context<Self>) {
        self.picked.clear();
        self.flows.update(cx, |flows, cx| flows.select(format!("flow:{id}"), cx));
        cx.notify();
    }

    /// The Workflows page.
    pub fn flows(&self) -> &Entity<FlowsPage> {
        &self.flows
    }

    /// The Workflows page stands in the board's place.
    pub fn flows_open(&self) -> bool {
        self.flows_open
    }

    /// "New task" in the header (item 8.5): the dialog for a task of your own, in one of your projects.
    pub fn new_own_task(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        crate::work::own_task::open(self.work.clone(), &self.frame, None, None, None, window, cx);
    }

    /// The window's projects and workspaces.
    pub(crate) fn frame(&self) -> &Entity<Frame> {
        &self.frame
    }

    /// There is a project a task of your own could be for.
    pub(crate) fn has_projects(&self, cx: &gpui_kit::App) -> bool {
        !self.frame.read(cx).projects.is_empty()
    }

    /// "Workflows" in the header, or Esc there: the Workflows page takes the board's place, or the board is back.
    /// The keyboard goes with it.
    pub fn show_flows(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.flows_open == on {
            return;
        }
        self.flows_open = on;
        if !on {
            self.flows.update(cx, |flows, cx| flows.left(window, cx));
        }
        self.take_keyboard(window, cx);
        cx.notify();
    }

    /// "Open sh-21309-export" on a task of the Workflows page: that workspace comes on screen in place of the Work
    /// page, and the board is back behind it.
    pub(crate) fn left_flows_for_workspace(&mut self, workspace: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.show_flows(false, window, cx);
        self.frame.update(cx, |frame, cx| {
            frame.select_workspace(workspace);
            cx.notify();
        });
    }

    /// Tasks or Hidden in the header: the board, or the cards you hid. The picks are let go.
    pub(crate) fn show_hidden(&mut self, on: bool, cx: &mut Context<Self>) {
        self.show_hidden = on;
        self.picked.clear();
        self.rebuild(cx);
    }

    /// Needs you in the header: only the cards with something waiting on you, or every card. It is remembered. It
    /// is a filter of the board: the board is back.
    pub(crate) fn toggle_needs_you(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_flows(false, window, cx);
        self.ui.update(cx, |ui, cx| {
            ui.work_needs_you = !ui.work_needs_you;
            cx.notify();
        });
    }

    /// The keyboard goes to a card (none: to no card). Its column scrolls to show it, and the board to show its
    /// column; nothing animates.
    pub fn select(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        if let Some((col, row)) = id.as_deref().and_then(|id| self.board.place_of(id)) {
            if let Some(list) = self.board.columns.get(col).and_then(|column| self.lists.get(&column.id)) {
                list.scroll_to_reveal_item(row);
            }
            self.scroll.scroll_to_item(col);
        }
        self.selected = id;
        cx.notify();
    }

    /// An arrow (or h j k l): the keyboard moves from card to card.
    fn step(&mut self, direction: Direction, cx: &mut Context<Self>) {
        if self.flows_open {
            return;
        }
        if let Some(next) = self.board.step(self.selected.as_deref(), direction) {
            self.select(Some(next), cx);
        }
    }

    /// A card was pressed.
    pub fn pressed(&mut self, id: &str, press: Press, window: &mut Window, cx: &mut Context<Self>) {
        match press {
            Press::Open => self.open(id, window, cx),
            Press::Pick => self.pick(id, cx),
            // every card from the last one picked to this one; this one comes last, so the next run starts here
            Press::PickRun => {
                let run = self.board.between(self.picked.last().map(String::as_str), id);
                self.picked.retain(|picked| !run.contains(picked));
                self.picked.extend(run);
                cx.notify();
            }
        }
    }

    /// Picks a card, or lets it go.
    pub fn pick(&mut self, id: &str, cx: &mut Context<Self>) {
        match self.picked.iter().position(|picked| picked == id) {
            Some(at) => {
                self.picked.remove(at);
            }
            None => self.picked.push(id.to_string()),
        }
        cx.notify();
    }

    /// Lets go of every picked card.
    pub fn clear_picks(&mut self, cx: &mut Context<Self>) {
        self.picked.clear();
        cx.notify();
    }

    /// Hide or Unhide in the bar: the picked cards that are on the board leave it (or the hidden ones come back),
    /// and the picks are let go. The core keeps it; the page shows it at once.
    fn hide_picked(&mut self, hidden: bool, window: &mut Window, cx: &mut Context<Self>) {
        let already: Vec<String> =
            self.work.read(cx).snapshot.as_ref().map(|index| index.hidden.clone()).unwrap_or_default();
        let cards: Vec<String> = self.picked.iter().filter(|id| already.contains(id) != hidden).cloned().collect();
        self.picked.clear();
        Work::hide(&self.work, cards, hidden, window.window_handle(), cx);
    }

    /// A card opens in front of the board, as a dialog; with one open already, it takes that one's place.
    pub fn open(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !self.board.cards.iter().any(|card| card == id) {
            return;
        }
        self.selected = Some(id.to_string());
        self.card = Some(id.to_string());
        match &self.card_view {
            Some(view) => view.update(cx, |card, cx| card.show(id, window, cx)),
            None => {
                let stores = card::CardStores {
                    work: self.work.clone(),
                    drafts: self.drafts.clone(),
                    frame: Some(self.frame.clone()),
                    terminals: self.terminals.clone(),
                };
                self.card_view = card::open(&cx.entity(), stores, id, window, cx);
            }
        }
        if self.card_view.is_none() {
            self.card = None;
        }
        cx.notify();
    }

    /// "Open" in the card's Claude box: the card is closed, and the workspace comes on screen in place of this page.
    pub(crate) fn left_for_workspace(&mut self, card: &str, workspace: &str, cx: &mut Context<Self>) {
        self.card_closed(card, cx);
        self.frame.update(cx, |frame, cx| {
            frame.select_workspace(workspace);
            cx.notify();
        });
    }

    /// A card's message went to the Claude of `workspace` (one just made is new to the window): the card is closed,
    /// and that workspace comes on screen in place of this page.
    pub(crate) fn left_for_new_workspace(&mut self, card: &str, workspace: Workspace, cx: &mut Context<Self>) {
        self.card_closed(card, cx);
        self.frame.update(cx, |frame, cx| {
            frame.add_workspace(workspace);
            cx.notify();
        });
    }

    /// The card's dialog closed (Esc, its ×, a click beside it, Hide).
    pub(crate) fn card_closed(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.card.as_deref() == Some(id) {
            self.card = None;
            self.card_view = None;
            cx.notify();
        }
    }

    /// The page takes the keyboard (it came on screen): the Workflows page while it stands in the board's place.
    pub fn take_keyboard(&self, window: &mut Window, cx: &mut App) {
        let keyboard = match self.flows_open {
            true => self.flows.read(cx).keyboard(),
            false => self.focus.clone(),
        };
        keyboard.focus(window, cx);
    }

    pub fn has_keyboard(&self, window: &Window, cx: &App) -> bool {
        self.focus.contains_focused(window, cx)
    }

    /// Esc: it empties the filter, then leaves its field; lets go of the picked cards; leaves Hidden; and with
    /// nothing else to let go of, goes back to the workspace.
    fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.card.take().is_some() {
            // the card in front of the board closes first (its dialog takes Esc itself while it has the keyboard)
            if let Some(view) = self.card_view.take() {
                view.update(cx, |card, cx| card.leave(window, cx));
            }
            cx.notify();
        } else if self.flows_open {
            // the Workflows page has one way out: back to the board
            self.show_flows(false, window, cx);
        } else if self.filtering(window, cx) {
            match self.filter.as_ref().filter(|_| !self.query.is_empty()).map(|(field, _)| field.clone()) {
                Some(field) => {
                    // emptied by us, so nothing tells us: the board is built again here
                    field.update(cx, |field, cx| field.set_value("", window, cx));
                    self.query.clear();
                    self.rebuild(cx);
                }
                None => self.take_keyboard(window, cx),
            }
        } else if !self.picked.is_empty() {
            self.clear_picks(cx);
        } else if self.show_hidden {
            self.show_hidden(false, cx);
        } else {
            self.frame.update(cx, |frame, cx| {
                frame.work_open = false;
                cx.notify();
            });
        }
    }

    /// Each project's name, for the lines that name one.
    pub(crate) fn projects(&self, cx: &App) -> HashMap<String, String> {
        self.frame.read(cx).projects.iter().map(|p| (p.id.clone(), p.name.clone())).collect()
    }
}

/// A source's lamp as the page draws it: one that wasn't read yet is hollow like one that is off.
pub fn look(lamp: work::Lamp) -> SourceLook {
    match lamp {
        work::Lamp::Ok => SourceLook::Ok,
        work::Lamp::Error => SourceLook::Error,
        work::Lamp::Reading => SourceLook::Reading,
        work::Lamp::Pending | work::Lamp::Off => SourceLook::Off,
    }
}

/// Before the first read: what is connected, and the one action.
fn first_run() -> AnyElement {
    let words = "Enkidu reads your pull requests with `gh` and your Jira tickets with `twg`, with your own logins, \
                 every few minutes, and never writes to either. Jira is read for a project once its Jira project keys \
                 are set in its settings.";
    let title = "Your tickets and PRs, as a board";
    island("work-first-run", false)
        .aria_label(title)
        .size_full()
        .items_center()
        .justify_center()
        .p(px(32.))
        .child(
            div()
                .max_w(px(440.))
                .flex()
                .flex_col()
                .items_center()
                .child(svg().path("icons/inbox.svg").size(px(28.)).text_color(theme::ACCENT))
                .child(
                    div()
                        .mt(px(12.))
                        .text_size(px(17.))
                        .line_height(px(24.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::INK)
                        .child(title),
                )
                .child(
                    div()
                        .mt(px(8.))
                        .text_size(theme::TEXT)
                        .line_height(px(19.))
                        .text_align(TextAlign::Center)
                        .child(with_code(words, theme::INK_2, theme::INK)),
                )
                .child(
                    div().mt(px(16.)).child(
                        button_with(
                            "work-check-now",
                            Some("icons/refresh-cw.svg"),
                            "Check now",
                            None,
                            ButtonKind::Primary,
                            ButtonSize::Large,
                        )
                        .on_click(|_, _, cx| Work::refresh(cx)),
                    ),
                ),
        )
        .child(lit_edge())
        .into_any_element()
}

/// A source that can't be read says so first: what is wrong, what to do, and how old the data under it is.
fn broken_line(line: &Broken) -> AnyElement {
    let said = match &line.fix {
        Some(fix) => format!("{} {} {fix}", line.title, line.error),
        None => format!("{} {}", line.title, line.error),
    };
    let fix = line.fix.as_ref().map(|fix| format!(" {fix}")).unwrap_or_default();
    let id = SharedString::from(format!("broken-{}", line.source.key()));
    div()
        .id(id)
        .test_support()
        .aria_label(SharedString::from(said.replace('`', "")))
        .aria_description(SharedString::from(line.under.clone()))
        .flex()
        .items_start()
        .gap(px(10.))
        .rounded(px(10.))
        .border_1()
        .border_color(alpha(theme::LAMP_RED, 0.25))
        .bg(alpha(theme::LAMP_RED, 0.07))
        .px(px(12.))
        .py(px(8.))
        .text_size(px(12.5))
        .line_height(px(18.))
        .child(svg().path("icons/triangle-alert.svg").flex_none().mt(px(2.)).size(px(14.)).text_color(theme::LAMP_RED))
        .child(
            div()
                .min_w_0()
                .flex_1()
                .flex()
                .flex_col()
                // one text that wraps: the title in a label's weight, then what is wrong and what to do, its
                // commands as code
                .child(sentence(
                    &[
                        Part { words: &line.title, color: theme::INK.into(), strong: true },
                        Part { words: &format!(" {}", line.error), color: theme::INK_2.into(), strong: false },
                        Part { words: &fix, color: theme::INK_2.into(), strong: false },
                    ],
                    theme::INK,
                ))
                .child(div().text_size(px(12.)).text_color(theme::INK_3).child(line.under.clone())),
        )
        .into_any_element()
}

impl WorkPage {
    /// The board: its columns side by side, each as wide as the others, scrolling sideways when the window is too
    /// narrow for them.
    fn board_drawn(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let columns: Vec<AnyElement> = (self.board.columns.iter().enumerate())
            .map(|(ix, column)| {
                let Some(state) = self.lists.get(&column.id).cloned() else { return div().into_any_element() };
                let (shown, selected, picked) = (self.board.clone(), self.selected.clone(), self.picked.clone());
                let (moved, moves) = (self.moved.clone(), self.moves);
                let page = cx.entity().downgrade();
                // a row is made when it comes into view
                let cards = list(state, move |row, _, _| {
                    let Some(face) = shown.columns.get(ix).and_then(|column| column.cards.get(row)) else {
                        return div().into_any_element();
                    };
                    let state = board::CardState {
                        selected: selected.as_deref() == Some(face.id.as_str()),
                        picked: picked.contains(&face.id),
                        picking: !picked.is_empty(),
                        moved: moved.contains(&face.id).then_some(moves),
                    };
                    let (page, id) = (page.clone(), face.id.clone());
                    board::in_list(board::card(face, state, move |press, window, cx| {
                        let _ = page.update(cx, |page, cx| page.pressed(&id, press, window, cx));
                    }))
                })
                .size_full();
                board::column(ix, column, cards.into_any_element(), window, cx)
            })
            .collect();
        // more board to the left or right: that edge fades, so a cut card reads as "scroll", not as broken
        let room = f32::from(window.viewport_size().width) - 2. * GUTTER;
        let (left, right) = board::more(self.board.columns.len(), room, f32::from(self.scroll.offset().x));
        let said = match (left, right) {
            (true, true) => "More to the left and to the right",
            (true, false) => "More to the left",
            (false, true) => "More to the right",
            (false, false) => "",
        };
        let fade = |from_left: bool| {
            let (near, far) = (theme::FRAME, alpha(theme::FRAME, 0.));
            let (first, second) = if from_left { (near, far) } else { (far, near) };
            div()
                .absolute()
                .top_0()
                .bottom(px(8.))
                .w(px(board::FADE))
                .map(|edge| if from_left { edge.left_0() } else { edge.right_0() })
                .bg(linear_gradient(90., linear_color_stop(first, 0.), linear_color_stop(second, 1.)))
        };
        let mut scroller = div()
            .id("work-board")
            .test_support()
            .aria_label("Board")
            .aria_description(said)
            .size_full()
            .flex()
            .gap(px(board::GAP))
            .pb(px(8.))
            .overflow_x_scroll()
            .track_scroll(&self.scroll)
            .children(columns);
        // the board moves sideways only by a sideways swipe: the wheel over a column scrolls that column's cards
        // (GPUI would give an area that scrolls one way the wheel's other way too)
        scroller.style().restrict_scroll_to_axis = Some(true);
        div()
            .relative()
            .size_full()
            .child(scroller)
            .when(left, |board| board.child(fade(true)))
            .when(right, |board| board.child(fade(false)))
            .into_any_element()
    }

    /// What you can do with the cards you picked: hide them, unhide them, or let go of them (Esc). It rises at the
    /// bottom of the board.
    fn pick_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.picked.is_empty() {
            return None;
        }
        let hidden: Vec<String> =
            self.work.read(cx).snapshot.as_ref().map(|index| index.hidden.clone()).unwrap_or_default();
        let to_show = self.picked.iter().filter(|id| hidden.contains(id)).count();
        let to_hide = self.picked.len() - to_show;
        let both = to_hide > 0 && to_show > 0;
        let count = self.picked.len();
        let words = if count == 1 { "1 card picked".to_string() } else { format!("{count} cards picked") };
        let numbered = |word: &str, n: usize| if both { format!("{word} {n}") } else { word.to_string() };
        let bar = raised("pick-bar")
            .aria_label(SharedString::from(words.clone()))
            .occlude()
            .flex()
            .items_center()
            .gap(px(4.))
            .py(px(6.))
            .pl(px(14.))
            .pr(px(6.))
            .child(
                enkidu_ui::pieces::tabular(div())
                    .mr(px(6.))
                    .whitespace_nowrap()
                    .text_size(theme::TEXT)
                    .text_color(theme::INK)
                    .child(words),
            )
            .child(
                button_with(
                    "pick-workflow",
                    Some("icons/calendar-clock.svg"),
                    "Workflow…",
                    None,
                    ButtonKind::Ghost,
                    ButtonSize::Medium,
                )
                .tipped("Claude works on them one after another, from a time you set", None)
                .on_click(cx.listener(|page, _, window, cx| page.workflow_for_picked(window, cx))),
            )
            .when(to_hide > 0, |bar| {
                bar.child(
                    button_with(
                        "pick-hide",
                        Some("icons/eye-off.svg"),
                        numbered("Hide", to_hide),
                        None,
                        ButtonKind::Primary,
                        ButtonSize::Medium,
                    )
                    .on_click(cx.listener(|page, _, window, cx| page.hide_picked(true, window, cx))),
                )
            })
            .when(to_show > 0, |bar| {
                // the only thing to do with them: the white button; beside Hide, the quiet one
                let kind = if to_hide > 0 { ButtonKind::Ghost } else { ButtonKind::Primary };
                bar.child(
                    button_with(
                        "pick-show",
                        Some("icons/eye.svg"),
                        numbered("Unhide", to_show),
                        None,
                        kind,
                        ButtonSize::Medium,
                    )
                    .on_click(cx.listener(|page, _, window, cx| page.hide_picked(false, window, cx))),
                )
            })
            .child(
                button("pick-clear", "Clear", ButtonKind::Ghost, ButtonSize::Medium)
                    .tipped("Let go of them", Some(Box::new(Back)))
                    .on_click(cx.listener(|page, _, _, cx| page.clear_picks(cx))),
            );
        Some(
            // centred over the board, 20 px above its foot; it rises in as the first card is picked
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom(px(20.))
                .child(rise_in_place("pick-bar-rise", div().flex().justify_center().child(bar)))
                .into_any_element(),
        )
    }
}

/// What the page says where the board would be, when there is no card to show.
fn quiet_line(quiet: &Quiet) -> AnyElement {
    let id = match quiet {
        Quiet::NoMatch(_) => "work-no-match",
        Quiet::AllQuiet(_) => "work-all-quiet",
    };
    div()
        .id(id)
        .test_support()
        .aria_label(SharedString::from(quiet.words().to_string()))
        .p(px(24.))
        .text_size(theme::TEXT)
        .text_color(theme::INK_3)
        .child(quiet.words().to_string())
        .into_any_element()
}

impl Render for WorkPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let work = self.work.read(cx);
        let now = work.now();
        let snapshot = work.snapshot.clone();
        let first = work::page::first_run(snapshot.as_deref());
        let broken = snapshot.as_deref().map(|work| work::broken(&work.health, &self.projects(cx), now));
        let broken = broken.unwrap_or_default();
        let body = if first {
            first_run()
        } else if self.flows_open {
            // the Workflows page, in the board's place
            self.flows.clone().into_any_element()
        } else if self.board.has_cards() {
            self.board_drawn(window, cx)
        } else {
            self.board.quiet.as_ref().map_or_else(|| div().into_any_element(), quiet_line)
        };
        let header = self.header(window, cx);
        // while the Workflows page is on screen there is no board to pick on
        let bar = self.pick_bar(cx).filter(|_| !self.flows_open);
        div()
            .id("work-page")
            .test_support()
            .aria_label("Work")
            .key_context(KEYS)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Back, window, cx| this.back(window, cx)))
            // the board's keys rest while the Workflows page stands in its place (that page has its own)
            .on_action(cx.listener(|this, _: &FocusFilter, window, cx| {
                if !this.flows_open {
                    this.focus_filter(window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &MoveUp, _, cx| this.step(Direction::Up, cx)))
            .on_action(cx.listener(|this, _: &MoveDown, _, cx| this.step(Direction::Down, cx)))
            .on_action(cx.listener(|this, _: &MoveLeft, _, cx| this.step(Direction::Left, cx)))
            .on_action(cx.listener(|this, _: &MoveRight, _, cx| this.step(Direction::Right, cx)))
            // ↓ in the filter: the keyboard leaves it for the board
            .on_action(cx.listener(|this, _: &FilterDown, window, cx| {
                this.take_keyboard(window, cx);
                this.step(Direction::Down, cx);
            }))
            .on_action(cx.listener(|this, _: &OpenCard, window, cx| {
                if let Some(id) = this.selected.clone().filter(|_| !this.flows_open) {
                    this.open(&id, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &PickCard, _, cx| {
                if let Some(id) = this.selected.clone().filter(|_| !this.flows_open) {
                    this.pick(&id, cx);
                }
            }))
            .flex_1()
            .min_h_0()
            .w_full()
            .flex()
            .flex_col()
            // the header row, where the workspace's tabs were
            .child(header)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .px(px(GUTTER))
                    .when(!broken.is_empty(), |page| {
                        page.child(
                            div()
                                .id("work-broken")
                                .mb(px(8.))
                                .flex()
                                .flex_col()
                                .gap(px(6.))
                                .children(broken.iter().map(broken_line)),
                        )
                    })
                    .child(div().relative().flex_1().min_h_0().child(body).children(bar)),
            )
    }
}
