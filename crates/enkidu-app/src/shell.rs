//! The window (DESIGN.md, "Layout"): a frame that holds one stage. On the frame stand the top row (the project on
//! screen and its workspaces as tabs), the rail at the left (Work, the projects), the files' column at the right and
//! the status line at the foot; on the stage the workspace on screen: Claude's terminals and the changes. Everything
//! but the terminals comes from the frame's store (`enkidu_model::frame::Frame`): the live one in the app, a sample
//! one for pictures.
//! The terminals come from theirs (`crate::terminals::Terminals`): the core's in the app, given ones for pictures
//! and tests.

use std::collections::HashMap;
use std::sync::Arc;

use enkidu_model::changes::ChangeLists;
use enkidu_model::frame::Frame;
use enkidu_model::search::Target;
use enkidu_model::ui::{self, Mode, Panel, Ui};
use enkidu_term::{GridSize, Palette, Terminal};
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    App, ClickEvent, Context, CursorStyle, DragMoveEvent, Entity, FocusHandle, KeyDownEvent, ModifiersChangedEvent,
    Render, ScrollHandle, StyleRefinement, Window, deferred, div, px,
};

use crate::changes_island::{ChangesIsland, Source};
use crate::demo::{self, Tab};
use crate::rail::Rail;
use crate::right_edge::ToolPanel;
use crate::search::SearchStore;
use crate::status_line::StatusLine;
use crate::switcher::{self, Card};
use crate::tabs_row::{TabsRow, now_ms};
use crate::terminal_area::TerminalArea;
use crate::terminal_view::{TerminalText, TerminalView};
use crate::terminals::Terminals;
use crate::work::page::WorkPage;
use crate::work::store::Work;
use enkidu_ui::pieces::{key_edge, rise, slide_in, stage};
use enkidu_ui::tokens as theme;

pub struct Shell {
    frame: Entity<Frame>,
    /// The rail: Work, the projects, the commands and Settings.
    rail: Entity<Rail>,
    /// The top row: the project on screen, its workspaces as tabs, and what acts on the one on screen.
    tabs: Entity<TabsRow>,
    /// The status line: the workspace on screen in words.
    status: Entity<StatusLine>,
    /// The window's own focus, for when there is no terminal to hold the keyboard (the app's keys still work).
    focus: FocusHandle,
    /// Whether a workspace was on screen when last looked at: when none is left, the window takes the keyboard.
    shown: bool,
    /// Claude's pane: the terminals of the workspace on screen.
    area: Entity<TerminalArea>,
    /// The terminals given to a window made for pictures and tests (none in the app, whose terminals the core runs).
    given: Vec<Entity<TerminalView>>,
    /// The changes island: the diffs of the workspace on screen. None in a window without stores (some pictures).
    changes: Option<Entity<ChangesIsland>>,
    /// The files' column at the right of the stage: Changes and Files. None in a window without a changes island.
    edge: Option<Entity<ToolPanel>>,
    /// How the workspace is laid out, as it was left.
    ui: Entity<Ui>,
    /// The line between two parts that is being dragged.
    dragging: Option<Gap>,
    /// What Search needs here: the changes it lists, what it keeps, and where its answers come from. None in a
    /// window that has nothing to search.
    search: Option<(Entity<ChangeLists>, Entity<SearchStore>, crate::search::Source)>,
    /// Shift tapped twice opens Search (DESIGN.md, "Search"): the taps so far, and the hearing of every other key.
    taps: enkidu_model::search::ShiftTaps,
    /// The Work page, which takes the top row and the stage while it is open (`Frame::work_open`).
    /// None in a window without a Work index (some pictures).
    work: Option<Entity<WorkPage>>,
    /// Whether the Work page was on screen when last drawn: as it comes or goes, the keyboard goes with it.
    work_shown: bool,
    /// With the lists hidden, the files can stand over the stage: they take the keyboard as they come, so Esc
    /// closes them, and what had the keyboard gets it back as they go.
    over_focus: FocusHandle,
    over_back: Option<FocusHandle>,
    over_shown: bool,
    /// The terminals, by workspace: the switcher reads the end of each Claude's from them.
    terminals: Entity<Terminals>,
    /// The Work index, for a workspace's pull request in the switcher. None in a window without one.
    work_index: Option<Entity<Work>>,
    /// The workspaces in the order they were last on screen, the latest first.
    recent: Vec<String>,
    /// The state each workspace was last seen in, and since when (the state it had when first seen has no time).
    stood: HashMap<String, (String, Option<i64>)>,
    /// The switcher while ⌃ is held: its cards, and the one the blue edge is on. It has the keyboard while it is
    /// there, and what had it before gets it back when you stay.
    switcher: Option<(Vec<Card>, usize)>,
    switch_focus: FocusHandle,
    switch_back: Option<FocusHandle>,
    switch_scroll: ScrollHandle,
    _keys: gpui_kit::Subscription,
    /// Hears when the keyboard is nowhere (see `keyboard_home`).
    _lost: gpui_kit::Subscription,
    /// What arrived while the window was open, for its motion (DESIGN.md, "Motion").
    arrivals: Arrivals,
    /// How often this window was drawn. A test reads it to see that nothing asks for a frame while nothing happens.
    drawn: u64,
}

/// The things that arrive on this screen and move as they do: the changes rise in when Claude's first edit lands,
/// the files' column slides in from the right edge when it is opened. What is there as the window opens, or when another
/// workspace comes on screen, stays still.
#[derive(Default)]
struct Arrivals {
    /// The workspace last drawn, and whether it had something to review.
    reviewing: Option<(String, bool)>,
    /// How many times the changes arrived in the workspace on screen.
    rises: u32,
    /// The changes that arrived last are still on screen: they keep their motion. Once they left the screen (another
    /// workspace, the terminal alone), they come back still: nothing arrived then.
    rising: bool,
    /// Whether the panel was open when last drawn.
    panel: Option<bool>,
    /// How many times the panel was opened.
    slides: u32,
    /// The panel, opened while you watched, is still on screen (as for `rising`).
    sliding: bool,
}

/// The line between two parts of a workspace: it is where they are resized (DESIGN.md, "Layout": "The line between
/// two parts drags to resize and remembers; double-click it to reset").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gap {
    /// Between the terminal and the changes beside it: it sets the changes' width.
    Beside,
    /// Between the changes and the terminal under them: it sets the terminal's height.
    Under,
    /// Between the stage and the files' column at its right: it sets the column's width.
    Panel,
}

/// What follows the pointer while a line is dragged: nothing, the parts themselves move.
struct Nothing;

impl Render for Nothing {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

impl Shell {
    /// A window on `frame` and `terminals`. The terminal on screen takes the keyboard.
    pub fn on(
        frame: Entity<Frame>,
        terminals: Entity<Terminals>,
        given: Vec<Entity<TerminalView>>,
        split: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Shell {
        Shell::build(frame, terminals, given, split, None, None, window, cx)
    }

    /// The same; `layout` is how the window is laid out when it isn't the stores' (a window made for pictures), and
    /// `work` the Work index when it isn't the stores'.
    #[allow(clippy::too_many_arguments)]
    fn build(
        frame: Entity<Frame>,
        terminals: Entity<Terminals>,
        given: Vec<Entity<TerminalView>>,
        split: bool,
        layout: Option<Entity<Ui>>,
        work: Option<Entity<Work>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Shell {
        let focus = cx.focus_handle();
        // the terminal takes the keyboard when a workspace is on screen (the island sees to that); otherwise (an empty
        // screen, or the database not read yet) the window does, so ⌘N and the rest still work
        let shown = frame.read(cx).current_workspace().is_some();
        if !shown {
            focus.focus(window, cx);
        }
        // the marks of where you are and where your typing goes rest while the window is behind another app
        // (DESIGN.md, "States"): the whole window is drawn again as it goes behind or comes in front
        cx.observe_window_activation(window, |_, window, _| window.refresh()).detach();
        // each time a workspace comes on screen, the core is told (its watcher looks at that one first) and git is
        // asked for its changes (mostly nothing changed)
        let mut on_screen = frame.read(cx).current_workspace().map(|w| w.id.clone());
        crate::stores::Stores::on_screen(on_screen.as_deref(), cx);
        cx.observe(&frame, move |_, frame, cx| {
            let now = frame.read(cx).current_workspace().map(|w| w.id.clone());
            if now != on_screen {
                on_screen = now;
                crate::stores::Stores::on_screen(on_screen.as_deref(), cx);
            }
        })
        .detach();
        cx.observe_in(&frame, window, |this: &mut Shell, frame, window, cx| {
            let now = frame.read(cx).current_workspace().is_some();
            if now != this.shown {
                this.shown = now;
                if !now {
                    // an empty screen: the window itself takes the keyboard, so ⌘N and the rest still work
                    this.focus.focus(window, cx);
                }
            }
            cx.notify();
        })
        .detach();
        // how the window is laid out: the stores', kept across launches; a window without stores has its own
        let ui = layout
            .or_else(|| cx.try_global::<crate::stores::Stores>().map(|stores| stores.ui.clone()))
            .unwrap_or_else(|| cx.new(|_| Ui::default()));
        // the terminal's text is the size that was left, and follows it (item 8.4)
        Self::size_terminal_text(&ui, cx);
        cx.observe(&ui, |_, ui, cx| {
            Self::size_terminal_text(&ui, cx);
            cx.notify();
        })
        .detach();
        // the Work index: the stores', or the one given to a window made for pictures and tests
        let work = work.or_else(|| cx.try_global::<crate::stores::Stores>().map(|stores| stores.work.clone()));
        let rail = cx.new(|cx| Rail::new(frame.clone(), work.clone(), cx));
        let tabs = cx.new(|cx| TabsRow::new(frame.clone(), ui.clone(), work.clone(), cx));
        // what you wrote about each card: the stores', kept across launches; a window without stores has its own
        let drafts = (cx.try_global::<crate::stores::Stores>().map(|stores| stores.drafts.clone()))
            .unwrap_or_else(|| cx.new(|_| crate::work::drafts::Drafts::default()));
        let (typed_into, kept_terminals, work_index) = (terminals.clone(), terminals.clone(), work.clone());
        let work = work.map(|work| {
            cx.new(|cx| WorkPage::new(frame.clone(), work, ui.clone(), drafts.clone(), Some(typed_into), cx))
        });
        // the status line says the workspace on screen in words, and on the Work page how each source was read
        let status = cx.new(|cx| StatusLine::new(frame.clone(), work_index.clone().zip(work.clone()), cx));
        let area = cx.new(|cx| TerminalArea::new(frame.clone(), terminals, split, focus.clone(), window, cx));
        // the island's terminals come and go (Claude starts, a tab closes): the frame follows who holds the keyboard
        cx.observe(&area, |_, _, cx| cx.notify()).detach();
        // the changes island, on the live stores: it comes and goes with the workspace's changes
        let stores =
            cx.try_global::<crate::stores::Stores>().map(|stores| (stores.changes.clone(), stores.remarks.clone()));
        let changes = stores.map(|(lists, remarks)| {
            let (ui, kept) = (ui.clone(), (lists.clone(), remarks.clone()));
            let island = cx.new(|cx| ChangesIsland::new(frame.clone(), lists, ui, remarks, Source::Core, window, cx));
            cx.observe(&island, |_, _, cx| cx.notify()).detach();
            (island, kept)
        });
        let edge = changes.as_ref().map(|(island, (lists, remarks))| {
            Shell::right_edge(&frame, lists, &ui, remarks, island, crate::files_pane::Source::Core, cx)
        });
        let changes = changes.map(|(island, _)| island);
        // Search on the app's stores: a picked result opens where the diffs are
        let search = cx
            .try_global::<crate::stores::Stores>()
            .map(|stores| (stores.changes.clone(), stores.search.clone(), crate::search::Source::Core));
        if let Some((_, store, _)) = &search {
            Shell::follow_search(store, changes.as_ref(), window, cx);
        }
        // any other key between two taps of Shift, or while Shift is down, means they aren't the two clean taps that
        // open Search. Keys are heard before anything acts on them, also the ones a text field turns into its own
        // commands (Shift and an arrow selects there)
        let (shell, here) = (cx.weak_entity(), window.window_handle());
        let keys = cx.intercept_keystrokes(move |event, window, cx| {
            if window.window_handle() == here && !event.keystroke.is_modifier_key() {
                let _ = shell.update(cx, |shell: &mut Shell, _| shell.taps.other());
            }
        });
        // what the core says on its own (Live's notes) is said in a toast here, where there is a window to say it in
        if let Some(notices) = cx.try_global::<crate::stores::Stores>().map(|stores| stores.notices.clone()) {
            cx.observe_in(&notices, window, |_, notices, window, cx| {
                for notice in notices.update(cx, |notices, _| std::mem::take(&mut notices.0)) {
                    match notice.kind {
                        crate::stores::NoticeKind::Warning => {
                            crate::toast::warning(window, notice.title, notice.detail, cx)
                        }
                        crate::stores::NoticeKind::Done => crate::toast::done(window, notice.title, notice.detail, cx),
                        crate::stores::NoticeKind::Note => crate::toast::note(window, notice.title, notice.detail, cx),
                    }
                }
            })
            .detach();
        }
        // what had the keyboard left the screen and nothing took it (a dialog closed and gave it back to something
        // that is gone by now, a terminal ended): the island on screen takes it, or no key of the app would work
        // until the next click
        let lost = cx.on_focus_lost(window, |this: &mut Shell, window, cx| this.keyboard_home(window, cx));
        Shell {
            frame,
            rail,
            tabs,
            status,
            focus,
            shown,
            area,
            given,
            changes,
            edge,
            ui,
            dragging: None,
            arrivals: Arrivals::default(),
            search,
            taps: Default::default(),
            work,
            work_shown: false,
            over_focus: cx.focus_handle(),
            over_back: None,
            over_shown: false,
            terminals: kept_terminals,
            work_index,
            recent: Vec::new(),
            stood: HashMap::new(),
            switcher: None,
            switch_focus: cx.focus_handle(),
            switch_back: None,
            switch_scroll: ScrollHandle::new(),
            _keys: keys,
            _lost: lost,
            drawn: 0,
        }
    }

    /// For pictures and screen tests: the window on `frame` with a given Work index (`enkidu_app::work::store::Work::given`)
    /// and a terminal that runs nothing.
    pub fn with_work(frame: Entity<Frame>, work: Entity<Work>, window: &mut Window, cx: &mut Context<Self>) -> Shell {
        let terminal = Arc::new(Terminal::kept(GridSize::new(120, 36), Palette::enkidu()));
        let view = cx.new(|cx| TerminalView::new(terminal, None, cx));
        let sample = cx.new(|_| Terminals::sample(vec![view.clone()]));
        Shell::build(frame, sample, vec![view], false, None, Some(work), window, cx)
    }

    /// For pictures: these projects show these icons on the rail.
    pub fn show_icons(&self, icons: HashMap<String, std::path::PathBuf>, cx: &mut App) {
        self.rail.update(cx, |rail, cx| rail.show_icons(icons, cx));
    }

    /// How often this window was drawn since it opened: every frame draws its root.
    pub fn drawn(&self) -> u64 {
        self.drawn
    }

    /// The Work page's view, in a window that has one.
    pub fn work_page(&self) -> Option<&Entity<WorkPage>> {
        self.work.as_ref()
    }

    /// The Work page is on screen, in the top row and on the stage.
    pub fn work_open(&self, cx: &App) -> bool {
        self.work.is_some() && self.frame.read(cx).work_open
    }

    /// A result picked in Search opens in this window; the files that are opened are remembered in Search's store.
    fn follow_search(
        store: &Entity<SearchStore>,
        island: Option<&Entity<ChangesIsland>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(island) = island {
            let store = store.clone();
            island.update(cx, |island, _| island.remember_in(store));
        }
        cx.observe_in(store, window, |this: &mut Shell, store, window, cx| {
            if let Some((workspace, target)) = store.update(cx, |store, _| store.asked.take()) {
                this.go_to(&workspace, target, window, cx);
            }
        })
        .detach();
    }

    /// Opens a file of the workspace on screen where the diffs are, at a line when one is named (a result of
    /// Search). Its folders open in the Files panel, so it is there in the tree afterwards.
    fn go_to(&mut self, workspace: &str, target: Target, _: &mut Window, cx: &mut Context<Self>) {
        let on_screen = self.frame.read(cx).current_workspace().is_some_and(|ws| ws.id == workspace);
        let (true, Some(island)) = (on_screen, self.changes.clone()) else {
            return;
        };
        if let Some(files) = self.files_pane(cx) {
            files.update(cx, |files, cx| files.reveal(&target.path, cx));
        }
        island.update(cx, |island, cx| island.open_at(&target.path, target.line, target.shown, cx));
    }

    /// Opens Search on a tab, or on the one used last. Nothing opens without a workspace on screen.
    fn open_search(&mut self, tab: Option<enkidu_model::search::Tab>, window: &mut Window, cx: &mut Context<Self>) {
        let Some((lists, store, source)) = self.search.clone() else {
            return;
        };
        let parts = crate::search::Parts { frame: self.frame.clone(), lists, store, ui: self.ui.clone(), source };
        crate::search::open(parts, tab, window, cx);
    }

    /// For a picture: sample tabs, with Claude (the first of `terminals`) exited with `code`.
    pub fn with_exited_claude(
        tabs: Vec<Tab>,
        terminals: Vec<Entity<TerminalView>>,
        code: i32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Shell {
        let frame = cx.new(|_| demo::frame(&tabs, 0));
        let sample = cx.new(|_| Terminals::sample(terminals.clone()).sample_exited(0, code));
        Shell::on(frame, sample, terminals, false, window, cx)
    }

    /// For a picture: sample tabs, and no terminal because Claude couldn't start.
    pub fn with_claude_error(tabs: Vec<Tab>, why: &str, window: &mut Window, cx: &mut Context<Self>) -> Shell {
        let frame = cx.new(|_| demo::frame(&tabs, 0));
        let sample = cx.new(|_| Terminals::sample(Vec::new()).sample_error(why));
        Shell::on(frame, sample, Vec::new(), false, window, cx)
    }

    /// For pictures: sample tabs, and the workspace on screen with `changes` to review, each with its diff.
    pub fn with_changes(
        tabs: Vec<Tab>,
        terminals: Vec<Entity<TerminalView>>,
        changes: Vec<(enkidu_core::git::Change, String)>,
        layout: enkidu_model::ui::DiffLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Shell {
        Shell::with_changes_and_files(tabs, terminals, changes, demo::files(), layout, window, cx)
    }

    /// The same, with the files git would list in the workspace (for the Files panel).
    pub fn with_changes_and_files(
        tabs: Vec<Tab>,
        terminals: Vec<Entity<TerminalView>>,
        changes: Vec<(enkidu_core::git::Change, String)>,
        files: Vec<String>,
        layout: enkidu_model::ui::DiffLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Shell {
        let frame = cx.new(|_| demo::frame(&tabs, 0));
        let ui = Ui { diff_layout: layout, ..Ui::default() };
        Shell::reviewing(frame, None, terminals, changes, files, ui, window, cx)
    }

    /// For the picture of the whole window: `frame` with a given Work index, so the Work tab has its count and the
    /// tabs their pull requests, and the workspace on screen with `changes` to review, laid out as `ui` says.
    pub fn with_work_and_changes(
        frame: Entity<Frame>,
        work: Entity<Work>,
        terminals: Vec<Entity<TerminalView>>,
        changes: Vec<(enkidu_core::git::Change, String)>,
        ui: Ui,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Shell {
        Shell::reviewing(frame, Some(work), terminals, changes, demo::files(), ui, window, cx)
    }

    /// The window on `frame` with `changes` to review in the workspace on screen, each with its diff, and `files`
    /// as the ones git would list there.
    #[allow(clippy::too_many_arguments)]
    fn reviewing(
        frame: Entity<Frame>,
        work: Option<Entity<Work>>,
        terminals: Vec<Entity<TerminalView>>,
        mut changes: Vec<(enkidu_core::git::Change, String)>,
        files: Vec<String>,
        ui: Ui,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Shell {
        let sample = cx.new(|_| Terminals::sample(terminals.clone()));
        let ui = cx.new(|_| ui);
        let mut shell =
            Shell::build(frame.clone(), sample.clone(), terminals, false, Some(ui.clone()), work, window, cx);
        // git's list comes in the order a tree of its folders lists it
        changes.sort_by(|(a, _), (b, _)| enkidu_core::names::in_tree_order(&a.path, &b.path));
        let workspace = frame.read(cx).current_workspace().map(|w| w.id.clone()).unwrap_or_default();
        // the tab and the status line count what the island shows
        let list: Vec<enkidu_core::git::Change> = changes.iter().map(|(change, _)| change.clone()).collect();
        frame.update(cx, |frame, _| frame.changed.insert(workspace.clone(), enkidu_model::frame::Changes::of(&list)));
        let lists = cx.new(|_| {
            let mut lists = enkidu_model::changes::ChangeLists::default();
            lists.set(&workspace, changes.iter().map(|(change, _)| change.clone()).collect());
            lists
        });
        let patches = changes.into_iter().map(|(change, patch)| (change.path, patch)).collect();
        // remarks kept in the window only: there is no database behind a picture
        let remarks = cx.new(|cx| crate::remarks::Remarks::new(frame.clone(), sample, false, cx));
        let island = cx.new(|cx| {
            let (lists, ui, remarks) = (lists.clone(), ui.clone(), remarks.clone());
            ChangesIsland::new(frame.clone(), lists, ui, remarks, Source::Given(patches), window, cx)
        });
        cx.observe(&island, |_, _, cx| cx.notify()).detach();
        // Search on sample answers: the same files, and two files opened lately
        let store = cx.new(|_| {
            let mut store = SearchStore::default();
            for path in ["src/orders/views.py", "src/orders/models.py"] {
                store.recent.opened(&workspace, path);
            }
            store
        });
        let given = crate::search::Source::Given(std::rc::Rc::new(demo::search(files.clone())));
        Shell::follow_search(&store, Some(&island), window, cx);
        shell.search = Some((lists.clone(), store, given));
        // and the files that git would list there
        let files = crate::files_pane::Source::Given(files);
        shell.edge = Some(Shell::right_edge(&frame, &lists, &ui, &remarks, &island, files, cx));
        shell.changes = Some(island);
        shell
    }

    /// A window with sample tabs (the measuring plans' and the pictures'): `selected` is on screen, and `terminals`
    /// stand in every workspace, the first as Claude.
    pub fn new(
        tabs: Vec<Tab>,
        selected: usize,
        terminals: Vec<Entity<TerminalView>>,
        split: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Shell {
        let frame = cx.new(|_| demo::frame(&tabs, selected));
        let sample = cx.new(|_| Terminals::sample(terminals.clone()));
        Shell::on(frame, sample, terminals, split, window, cx)
    }

    /// The window as the app opens it: the frame's live store, and each workspace's terminals as the core runs them
    /// (its Claude starts when the workspace comes on screen).
    pub fn live(frame: Entity<Frame>, window: &mut Window, cx: &mut Context<Self>) -> Shell {
        let terminals = crate::stores::Stores::terminals(cx);
        // at launch every workspace's Claude comes back, once the database was read (asked for once). A workspace
        // that is put away is left asleep: its Claude starts when it is brought back and comes on screen
        let bring_back = |frame: &Entity<Frame>, terminals: &Entity<Terminals>, cx: &mut App| {
            let frame = frame.read(cx);
            if frame.loaded {
                let all = frame.workspaces.iter().filter(|w| !frame.is_hidden(&w.id)).map(|w| w.id.clone()).collect();
                let on_screen = frame.current_workspace().map(|w| w.id.clone());
                terminals.update(cx, |terminals, cx| terminals.bring_back(all, on_screen, cx));
            }
        };
        bring_back(&frame, &terminals, cx);
        cx.observe(&frame, {
            let terminals = terminals.clone();
            move |_, frame, cx| bring_back(&frame, &terminals, cx)
        })
        .detach();
        // the first launch (item 21.6; DESIGN.md, "Setup"): with no project yet, and Setup never shown on this Mac,
        // Settings opens on Setup by itself, once the database was read. Whoever has a project already is past it
        let decided = std::rc::Rc::new(std::cell::Cell::new(false));
        let welcome = move |frame: &Entity<Frame>, window: &mut Window, cx: &mut App| {
            if decided.get() || !frame.read(cx).loaded {
                return;
            }
            decided.set(true);
            let ui = crate::stores::Stores::ui(cx);
            if ui.read(cx).setup_seen {
                return;
            }
            ui.update(cx, |ui, cx| {
                ui.setup_seen = true;
                cx.notify();
            });
            if frame.read(cx).projects.is_empty() {
                let frame = frame.clone();
                window.defer(cx, move |window, cx| crate::settings::open_welcome(frame, window, cx));
            }
        };
        welcome(&frame, window, cx);
        cx.observe_in(&frame, window, move |_, frame, window, cx| welcome(&frame, window, cx)).detach();
        Shell::on(frame, terminals, Vec::new(), false, window, cx)
    }

    /// The window on `frame` with a terminal that runs nothing, the same in every workspace: for screen tests on the
    /// live stores that are not about terminals.
    pub fn kept(frame: Entity<Frame>, window: &mut Window, cx: &mut Context<Self>) -> Shell {
        let terminal = Arc::new(Terminal::kept(GridSize::new(120, 36), Palette::enkidu()));
        let view = cx.new(|cx| TerminalView::new(terminal, None, cx));
        let sample = cx.new(|_| Terminals::sample(vec![view.clone()]));
        Shell::on(frame, sample, vec![view], false, window, cx)
    }

    /// The window with sample data and a terminal that runs nothing: for pictures and screen tests.
    pub fn demo(window: &mut Window, cx: &mut Context<Self>) -> Shell {
        let terminal = Arc::new(Terminal::kept(GridSize::new(120, 36), Palette::enkidu()));
        terminal.feed(&demo::terminal_output());
        let view = cx.new(|cx| TerminalView::new(terminal, None, cx));
        Shell::new(demo::tabs(), 0, vec![view], false, window, cx)
    }

    pub fn frame(&self) -> &Entity<Frame> {
        &self.frame
    }

    /// Where the workspace on screen is among the rows of the project on screen.
    pub fn selected(&self, cx: &App) -> usize {
        let frame = self.frame.read(cx);
        let current = frame.current_workspace().map(|w| w.id.clone());
        frame.tabs().iter().position(|w| Some(&w.id) == current.as_ref()).unwrap_or(0)
    }

    /// The first terminal given to a window made for pictures and tests.
    pub fn terminal(&self) -> &Entity<TerminalView> {
        &self.given[0]
    }

    pub fn terminals(&self) -> &[Entity<TerminalView>] {
        &self.given
    }

    /// The Files tab's view, in a window that has the files' column.
    pub fn files_pane(&self, cx: &App) -> Option<Entity<crate::files_pane::FilesPane>> {
        self.edge.as_ref().map(|panel| panel.read(cx).files().clone())
    }

    /// The changes island, in a window on the stores.
    pub fn changes(&self) -> Option<&Entity<ChangesIsland>> {
        self.changes.as_ref()
    }

    /// The files' column of a window that has a changes island.
    fn right_edge(
        frame: &Entity<Frame>,
        lists: &Entity<enkidu_model::changes::ChangeLists>,
        ui: &Entity<Ui>,
        remarks: &Entity<crate::remarks::Remarks>,
        island: &Entity<ChangesIsland>,
        files: crate::files_pane::Source,
        cx: &mut Context<Self>,
    ) -> Entity<ToolPanel> {
        cx.new(|cx| {
            ToolPanel::new(frame.clone(), lists.clone(), ui.clone(), remarks.clone(), island.clone(), files, cx)
        })
    }

    /// How the window is laid out.
    /// The keyboard goes to the island on screen: the terminal there, or the changes while they have the whole
    /// workspace. With no terminal the window itself holds it. Never while a dialog is open: its keys are its own.
    fn keyboard_home(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use gpui_kit::component::WindowExt as _;
        if window.has_active_dialog(cx) {
            return;
        }
        // while the Work page is on screen the keyboard's home is there
        if let Some(page) = self.work.clone().filter(|_| self.frame.read(cx).work_open) {
            page.update(cx, |page, cx| page.take_keyboard(window, cx));
            return;
        }
        let reviewing = self.changes.as_ref().filter(|island| island.read(cx).reviewing(cx)).cloned();
        let mode = self.ui.read(cx).mode(reviewing.is_none());
        match reviewing {
            Some(changes) if mode == Mode::Review => {
                changes.update(cx, |changes, cx| changes.take_keyboard(window, cx))
            }
            _ => self.area.update(cx, |area, cx| area.take_keyboard(window, cx)),
        }
    }

    pub fn ui(&self) -> &Entity<Ui> {
        &self.ui
    }

    /// The layout changes: `change` says how (a key, the switch).
    /// The terminal's text at the size that is remembered.
    fn size_terminal_text(ui: &Entity<Ui>, cx: &mut App) {
        let size = px(ui.read(cx).terminal_size);
        if crate::terminal_view::text_size(cx) != size {
            cx.set_global(TerminalText(size));
        }
    }

    /// ⌘+, ⌘− and ⌘0 (item 8.4): the text of what has the keyboard a size up, a size down (`step`), or back at its
    /// usual size (no step). A terminal's when a terminal has the keyboard, as in every terminal; else the code's,
    /// in the diffs and in an opened file.
    fn resize_text(&mut self, step: Option<bool>, window: &mut Window, cx: &mut Context<Self>) {
        let terminal = self.area.read(cx).has_keyboard(window, cx);
        self.lay_out(cx, |ui| {
            match (terminal, step) {
                (true, Some(up)) => ui.step_terminal_size(up),
                (true, None) => ui.set_terminal_size(ui::TERMINAL_SIZE),
                (false, Some(up)) => ui.step_code_size(up),
                (false, None) => ui.set_code_size(ui::CODE_SIZE),
            };
        });
    }

    fn lay_out(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut Ui)) {
        self.ui.update(cx, |ui, cx| {
            let was = ui.clone();
            change(ui);
            if *ui != was {
                cx.notify();
            }
        });
    }

    /// The line being dragged moved: the part on its far side gets what the pointer asks for, within what each part
    /// keeps. `event.bounds` are the two parts together.
    fn drag_gap(&mut self, event: &DragMoveEvent<Gap>, window: &mut Window, cx: &mut Context<Self>) {
        let gap = *event.drag(cx);
        let (at, room) = (event.event.position, event.bounds);
        self.dragging = Some(gap);
        // the panel's line is measured against the whole workspace, not the two parts (see `drag_panel_gap`)
        if gap == Gap::Panel {
            return;
        }
        self.lay_out(cx, |ui| match gap {
            Gap::Beside => {
                ui.review_width = ui::dragged_review_width((room.right() - at.x).into(), room.size.width.into())
            }
            _ => {
                ui.terminal_height = ui::dragged_terminal_height((room.bottom() - at.y).into(), room.size.height.into())
            }
        });
        // the pointer leaves the line within a pixel or two: it keeps saying what it is doing
        let cursor = if gap == Gap::Beside { CursorStyle::ResizeLeftRight } else { CursorStyle::ResizeUpDown };
        cx.set_active_drag_cursor_style(cursor, window);
        cx.notify();
    }

    /// The line between the stage and the files' column moved: the column gets what the pointer asks for, between
    /// 200 and 520 px. `event.bounds` are the stage and the column together.
    fn drag_panel_gap(&mut self, event: &DragMoveEvent<Gap>, window: &mut Window, cx: &mut Context<Self>) {
        if *event.drag(cx) != Gap::Panel {
            return;
        }
        let from_right = event.bounds.right() - event.event.position.x;
        self.dragging = Some(Gap::Panel);
        self.lay_out(cx, |ui| ui.right_panel_width = ui::dragged_panel_width(from_right.into()));
        cx.set_active_drag_cursor_style(CursorStyle::ResizeLeftRight, window);
        cx.notify();
    }

    /// The line between two parts: 1 px that is seen, and 4 px on either side of it that take the pointer. It drags
    /// to resize the part on its far side, and a double click puts that part back to its first size. Pointing at it
    /// shows the line that is about to move, in blue. Between the stage and the files' column the stage's own edge
    /// is the line: there the pointer has 4 px on either side of where the two meet.
    fn gap(&self, gap: Gap, cx: &mut Context<Self>) -> impl IntoElement {
        let moving = self.dragging == Some(gap) && cx.has_active_drag();
        let lit = div()
            .absolute()
            .bg(theme::ACCENT)
            .map(|lit| match gap {
                Gap::Panel => lit.top_0().bottom_0().left(px(3.)).w(px(2.)),
                Gap::Beside => lit.top_0().bottom_0().left(px(3.)).w(px(3.)),
                Gap::Under => lit.left_0().right_0().top(px(3.)).h(px(3.)),
            })
            .opacity(if moving { 1. } else { 0. })
            .group_hover("gap", |lit| lit.opacity(1.));
        let handle = div()
            .id(if gap == Gap::Panel { "panel-divider" } else { "layout-divider" })
            .test_support()
            .aria_label(match gap {
                Gap::Beside => "Resize changes",
                Gap::Under => "Resize terminal",
                Gap::Panel => "Resize panel",
            })
            .absolute()
            .map(|handle| match gap {
                Gap::Panel => handle.top_0().bottom_0().left(px(-4.)).w(px(8.)).cursor_col_resize(),
                Gap::Beside => handle.top_0().bottom_0().left(px(-4.)).w(px(9.)).cursor_col_resize(),
                Gap::Under => handle.left_0().right_0().top(px(-4.)).h(px(9.)).cursor_row_resize(),
            })
            // what lies under it (a terminal's text, a diff) doesn't hear a press that is meant for the line
            .occlude()
            .group("gap")
            .child(lit)
            .on_drag(gap, |_, _, _, cx| cx.new(|_| Nothing))
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                if event.click_count() == 2 {
                    this.lay_out(cx, |ui| match gap {
                        Gap::Beside => ui.review_width = ui::REVIEW_WIDTH,
                        Gap::Under => ui.terminal_height = ui::TERMINAL_HEIGHT,
                        Gap::Panel => ui.right_panel_width = ui::PANEL_WIDTH,
                    });
                }
            }));
        div()
            .relative()
            .flex_none()
            .map(|line| match gap {
                // between the stage and the files' column the stage's own edge is the line: this takes no room
                Gap::Panel => line.w(px(0.)).h_full(),
                Gap::Beside => line.w(px(ui::DIVIDER)).h_full().bg(theme::LINE),
                Gap::Under => line.h(px(ui::DIVIDER)).w_full().bg(theme::LINE),
            })
            // drawn after the parts beside it, so the pointer finds it above both
            .child(deferred(handle))
    }

    /// What stands between the top row and the status line while a workspace's project is on screen: the stage with
    /// `body` on it, and at its right the files' column, while it is open and a workspace is on screen. The rail
    /// stands over the room left for it at the left; with the lists hidden the stage has the window's width.
    fn workspace_row(
        &self,
        body: gpui_kit::AnyElement,
        workspace_shown: bool,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let ui = self.ui.read(cx);
        let hidden = ui.lists_hidden;
        let panel = self.edge.as_ref().filter(|_| workspace_shown);
        // with the lists hidden there is no column at the right: the files stand over the stage while asked for
        let open = panel.is_some() && !hidden && ui.right_panel.is_some();
        let over = panel.is_some() && hidden && ui.files_over;
        let width = ui.right_panel_width as f32;
        let turn = if self.arrivals.sliding { self.arrivals.slides } else { 0 };
        let row = div()
            .id("workspace-row")
            .test_support()
            .relative()
            .flex_1()
            .min_h_0()
            .flex()
            .pl(if hidden { theme::STAGE_MARGIN } else { theme::RAIL })
            // where no column stands, the stage ends 8 px before the window's edge
            .when(!open, |row| row.pr(theme::STAGE_MARGIN))
            .on_drag_move(cx.listener(Self::drag_panel_gap))
            .child(stage("stage", body).flex_1().min_w_0().h_full());
        // the column is never more than 40% of the window
        let row = match panel.filter(|_| open) {
            Some(panel) => row.child(self.gap(Gap::Panel, cx)).child(
                div().w(px(width)).max_w(gpui_kit::relative(0.4)).h_full().flex_none().child(slide_in(
                    "panel-slide",
                    turn,
                    panel.clone(),
                )),
            ),
            None => row,
        };
        // over the stage: a sheet at its right, 8 px inside it. Esc takes it away
        let row = match panel.filter(|_| over) {
            Some(panel) => row.child(
                div()
                    .id("files-over")
                    .test_support()
                    .aria_label("The files, over the stage")
                    .track_focus(&self.over_focus)
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                        if event.keystroke.key == "escape" && !event.keystroke.modifiers.modified() {
                            cx.stop_propagation();
                            this.lay_out(cx, |ui| {
                                ui.close_files_over();
                            });
                        }
                    }))
                    .absolute()
                    .top(px(8.))
                    .right(theme::STAGE_MARGIN + px(8.))
                    .bottom(px(8.))
                    .w(px(FILES_OVER))
                    .max_w(gpui_kit::relative(0.6))
                    .occlude()
                    .overflow_hidden()
                    .rounded(theme::RADIUS_DIALOG)
                    .bg(theme::SHEET)
                    .border_1()
                    .border_color(theme::LINE_STRONG)
                    .shadow(enkidu_ui::pieces::float_shadow())
                    .child(panel.clone()),
            ),
            None => row,
        };
        row.into_any_element()
    }

    /// A pane of the stage: what it holds, and over it the blue edge while your typing goes there (DESIGN.md,
    /// "States": Keyboard). `corners` are the pane's own, clockwise from the top left.
    fn pane(holds: impl IntoElement, typing: bool, corners: [f32; 4]) -> gpui_kit::Div {
        div().relative().child(holds).when(typing, |pane| {
            pane.child(key_edge(corners).id("key-edge").test_support().aria_label("Your typing goes here"))
        })
    }

    /// The terminal on screen in Claude's pane, if the workspace on screen has one.
    pub fn terminal_on_screen(&self, cx: &App) -> Option<Entity<TerminalView>> {
        self.area.read(cx).active_view(cx)
    }

    /// ⌘1 to ⌘9 put that tab of the project on screen there; ⇧⌘] the next tab of the row and ⇧⌘[ the one before,
    /// around its ends.
    fn pick_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        self.frame.update(cx, |frame, cx| {
            if frame.select_tab(index) {
                cx.notify();
            }
        })
    }

    /// ⌃⇥ or ⌃⇧⇥: the switcher opens with the blue edge on the workspace used before this one (going back, on the
    /// one used longest ago); while it is open, each press moves the edge on. With one workspace there is nothing
    /// to switch to.
    fn switch(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((cards, at)) = &mut self.switcher {
            *at = enkidu_model::switcher::step(*at, cards.len(), forward);
            cx.notify();
            return;
        }
        let cards = {
            let work = self.work_index.as_ref().map(|work| work.read(cx).snapshot.clone());
            switcher::cards(
                self.frame.read(cx),
                &self.recent,
                &self.stood,
                self.terminals.read(cx),
                work.as_ref().and_then(|snapshot| snapshot.as_deref()),
                now_ms(),
                cx,
            )
        };
        if cards.len() < 2 {
            return;
        }
        let at = enkidu_model::switcher::opens_at(cards.len(), forward);
        self.switch_back = window.focused(cx);
        self.switch_focus.focus(window, cx);
        self.switcher = Some((cards, at));
        cx.notify();
    }

    /// The switcher goes. `go`: ⌃ was let go, Enter, or a card was clicked: the workspace with the blue edge comes on
    /// screen. Else (Esc, a click beside the cards) you stay, and what had the keyboard has it again.
    fn switched(&mut self, go: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some((cards, at)) = self.switcher.take() else {
            return;
        };
        let back = self.switch_back.take();
        let target = cards.get(at).filter(|_| go).map(|card| card.id.clone());
        let moved = target.is_some_and(|id| {
            self.frame.update(cx, |frame, cx| {
                let moved = frame.select_workspace(&id);
                if moved {
                    cx.notify();
                }
                moved
            })
        });
        match (moved, back) {
            (false, Some(back)) => back.focus(window, cx),
            _ => self.keyboard_home(window, cx),
        }
        cx.notify();
    }

    fn cycle_tabs(&mut self, step: i32, cx: &mut Context<Self>) {
        self.frame.update(cx, |frame, cx| {
            if frame.cycle_workspace(step) {
                cx.notify();
            }
        })
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::measure::FRAMES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.drawn += 1;
        // no workspaces yet: the empty screen says what to do
        let (loaded, has_tabs, all_hidden, project) = {
            let frame = self.frame.read(cx);
            let project = frame.current_project.as_ref().and_then(|id| frame.projects.iter().find(|p| &p.id == id));
            let (tabs, hidden) = (!frame.tabs().is_empty(), !frame.hidden_tabs().is_empty());
            (frame.loaded, tabs, !tabs && hidden, project.map(|project| project.name.clone()))
        };
        // the Work page takes the top row and the stage; as it comes or goes, the keyboard goes with it
        let work = self.work.clone().filter(|_| self.frame.read(cx).work_open);
        if work.is_some() != self.work_shown {
            self.work_shown = work.is_some();
            self.keyboard_home(window, cx);
        }
        // what stands on the stage: nothing until the database was read, an empty screen, or the workspace's panes
        let body = match (loaded, project) {
            // behind the Work page nothing of the workspace is drawn (its terminals keep running)
            _ if work.is_some() => div().into_any_element(),
            (false, _) => div().into_any_element(),
            (true, None) => crate::empty::no_projects(self.frame.clone(), window).into_any_element(),
            // every workspace of the project is put away: the screen says so, and how one comes back
            (true, Some(project)) if all_hidden => crate::empty::all_hidden(&project).into_any_element(),
            (true, Some(project)) if !has_tabs => {
                crate::empty::no_workspaces(self.frame.clone(), &project).into_any_element()
            }
            // not cached: in real windows, a cached pane (it holds the terminal that has the keyboard) gave 0 frames
            // a second while a lamp turned, three runs in a row; uncached, the runs before and after gave 24
            (true, Some(_)) => {
                // with nothing to review, Claude's terminal has the whole workspace, whatever the layout
                let island = self.changes.as_ref().filter(|island| island.read(cx).reviewing(cx)).cloned();
                let ui = self.ui.read(cx).clone();
                let mode = ui.mode(island.is_none());
                // the changes that arrive while you watch this workspace rise into place
                let on_screen = self.frame.read(cx).current_workspace().map(|ws| ws.id.clone()).unwrap_or_default();
                let now = (on_screen, island.is_some());
                if let Some((workspace, reviewing)) = &self.arrivals.reviewing
                    && *workspace == now.0
                    && !*reviewing
                    && now.1
                {
                    (self.arrivals.rises, self.arrivals.rising) = (self.arrivals.rises + 1, true);
                } else if island.is_none()
                    || mode == Mode::Terminal
                    || self.arrivals.reviewing.as_ref().is_some_and(|(workspace, _)| *workspace != now.0)
                {
                    // they aren't on screen, or these are another workspace's: the next time they show, they are still
                    self.arrivals.rising = false;
                }
                self.arrivals.reviewing = Some(now);
                let turn = if self.arrivals.rising { self.arrivals.rises } else { 0 };
                // the pane your typing goes to has the blue edge; none has it while the window is behind another
                // app. A pane's corners are the stage's, 1 px inside its edge, where it stands in one of them
                let in_front = window.is_window_active();
                let typing_changes =
                    in_front && island.as_ref().is_some_and(|island| island.read(cx).has_keyboard(window, cx));
                let typing_claude = in_front && self.area.read(cx).has_keyboard(window, cx);
                let round = f32::from(theme::RADIUS_STAGE) - 1.;
                let island = island.map(|island| rise("changes-rise", turn, island));
                // a pane that leaves the screen hands the keyboard to the one that stays
                if let Some(changes) = &self.changes {
                    if mode == Mode::Terminal && changes.read(cx).has_keyboard(window, cx) {
                        self.area.update(cx, |area, cx| area.take_keyboard(window, cx));
                    } else if mode == Mode::Review && self.area.read(cx).has_keyboard(window, cx) {
                        changes.update(cx, |changes, cx| changes.take_keyboard(window, cx));
                    }
                }
                match (mode, island) {
                    // the changes above the terminal, which keeps its height while there is room
                    (Mode::Stack, Some(island)) => div()
                        .id("workspace-body")
                        .test_support()
                        .size_full()
                        .flex()
                        .flex_col()
                        .on_drag_move(cx.listener(Self::drag_gap))
                        .child(
                            Self::pane(island, typing_changes, [round, round, 0., 0.])
                                .flex_1()
                                .min_h(px(ui::MIN_CHANGES_HEIGHT)),
                        )
                        .child(self.gap(Gap::Under, cx))
                        .child(
                            Self::pane(self.area.clone(), typing_claude, [0., 0., round, round])
                                .h(px(ui.terminal_height as f32))
                                .min_h(px(ui::MIN_TERMINAL_HEIGHT))
                                .flex_shrink(1.),
                        )
                        .into_any_element(),
                    // Claude at the left, where you type, and the changes beside it, next to their files; the
                    // terminal keeps its least width
                    (Mode::Side, Some(island)) => div()
                        .id("workspace-body")
                        .test_support()
                        .size_full()
                        .flex()
                        .on_drag_move(cx.listener(Self::drag_gap))
                        .child(
                            Self::pane(self.area.clone(), typing_claude, [round, 0., 0., round])
                                .flex_1()
                                .h_full()
                                .min_w(px(ui::MIN_TERMINAL_WIDTH)),
                        )
                        .child(self.gap(Gap::Beside, cx))
                        .child(
                            Self::pane(island, typing_changes, [0., round, round, 0.])
                                .w(px(ui.review_width as f32))
                                .h_full()
                                .min_w_0()
                                .flex_shrink(1.),
                        )
                        .into_any_element(),
                    // the changes alone; the terminals keep running behind them
                    (Mode::Review, Some(island)) => {
                        Self::pane(island, typing_changes, [round; 4]).size_full().into_any_element()
                    }
                    _ => Self::pane(self.area.clone(), typing_claude, [round; 4]).size_full().into_any_element(),
                }
            }
        };
        let island_shown = loaded && has_tabs;
        // the files' column, opened while you watch, slides in (another tab of it just shows)
        let (hidden, over) = {
            let ui = self.ui.read(cx);
            (ui.lists_hidden, ui.lists_hidden && ui.files_over)
        };
        // for the switcher: which workspace was on screen last, and since when each stands as it does
        {
            let frame = self.frame.read(cx);
            if let Some(ws) = frame.current_workspace() {
                enkidu_model::switcher::used(&mut self.recent, &ws.id);
            }
            for ws in &frame.workspaces {
                let status = frame.status(&ws.id);
                match self.stood.get(&ws.id) {
                    Some((was, _)) if was == status => {}
                    Some(_) => {
                        self.stood.insert(ws.id.clone(), (status.to_string(), Some(now_ms())));
                    }
                    None => {
                        self.stood.insert(ws.id.clone(), (status.to_string(), None));
                    }
                }
            }
        }
        let panel = !hidden && self.ui.read(cx).right_panel.is_some();
        // the files over the stage take the keyboard as they come (Esc closes them); as they go, what had the
        // keyboard has it again, unless something else took it meanwhile (a file that was picked)
        let over = over && island_shown && work.is_none();
        if over != self.over_shown {
            self.over_shown = over;
            match over {
                true => {
                    self.over_back = window.focused(cx);
                    self.over_focus.focus(window, cx);
                }
                false => {
                    if self.over_focus.contains_focused(window, cx)
                        && let Some(back) = self.over_back.take()
                    {
                        back.focus(window, cx);
                    }
                    self.over_back = None;
                }
            }
        }
        if self.arrivals.panel == Some(false) && panel && island_shown {
            (self.arrivals.slides, self.arrivals.sliding) = (self.arrivals.slides + 1, true);
        } else if !panel || !island_shown {
            self.arrivals.sliding = false;
        }
        self.arrivals.panel = Some(panel);
        let no_terminal = self.area.read(cx).active_view(cx).is_none();
        div()
            .id("shell")
            // only while no terminal is shown: a window that takes focus would take the keyboard from the terminal on
            // any click
            .when(no_terminal || !island_shown, |shell| shell.track_focus(&self.focus))
            // ⇧⇧: Shift tapped twice opens Search on the tab used last. Only two clean taps count (see
            // `enkidu_model::search::ShiftTaps`), by the clock that tests can hold
            .on_modifiers_changed(cx.listener(|this, event: &ModifiersChangedEvent, window, cx| {
                let held = event.modifiers;
                // ⌃ was let go while the switcher is there: the workspace with the blue edge comes on screen
                if !held.control && this.switcher.is_some() {
                    this.switched(true, window, cx);
                }
                let others = held.control || held.alt || held.platform || held.function;
                let now = cx.background_executor().now();
                if this.taps.modifiers(held.shift, others, now) {
                    this.open_search(None, window, cx);
                }
                // ⌘ went down or up: the code on screen is told, so the name under the pointer becomes a link (item
                // 10.11). The keys are told here because Claude's terminal often has the keyboard
                if let Some(island) = this.changes.clone() {
                    let pointer = window.mouse_position();
                    island.update(cx, |island, cx| island.keys_changed(held.platform, pointer, cx));
                }
            }))
            .capture_any_mouse_down(cx.listener(|this, _, _, _| this.taps.other()))
            .on_action(cx.listener(|this, _: &crate::SelectTab1, _, cx| this.pick_tab(0, cx)))
            .on_action(cx.listener(|this, _: &crate::SelectTab2, _, cx| this.pick_tab(1, cx)))
            .on_action(cx.listener(|this, _: &crate::SelectTab3, _, cx| this.pick_tab(2, cx)))
            .on_action(cx.listener(|this, _: &crate::SelectTab4, _, cx| this.pick_tab(3, cx)))
            .on_action(cx.listener(|this, _: &crate::SelectTab5, _, cx| this.pick_tab(4, cx)))
            .on_action(cx.listener(|this, _: &crate::SelectTab6, _, cx| this.pick_tab(5, cx)))
            .on_action(cx.listener(|this, _: &crate::SelectTab7, _, cx| this.pick_tab(6, cx)))
            .on_action(cx.listener(|this, _: &crate::SelectTab8, _, cx| this.pick_tab(7, cx)))
            .on_action(cx.listener(|this, _: &crate::SelectTab9, _, cx| this.pick_tab(8, cx)))
            .on_action(cx.listener(|this, _: &crate::SwitchNext, window, cx| this.switch(true, window, cx)))
            .on_action(cx.listener(|this, _: &crate::SwitchPrevious, window, cx| this.switch(false, window, cx)))
            .on_action(cx.listener(|this, _: &crate::NextWorkspace, _, cx| this.cycle_tabs(1, cx)))
            .on_action(cx.listener(|this, _: &crate::PreviousWorkspace, _, cx| this.cycle_tabs(-1, cx)))
            .on_action(cx.listener(|this, _: &crate::NewWorkspace, window, cx| {
                crate::new_workspace::open(this.frame.clone(), window, cx)
            }))
            .on_action(cx.listener(|this, _: &crate::TogglePalette, window, cx| {
                crate::palette::open(this.frame.clone(), window, cx)
            }))
            // the layouts (DESIGN.md, "Layout"): ⌥⌘L the changes beside the terminal or above it, ⌥⌘B the terminal alone,
            // ⇧⌘E the changes alone
            .on_action(cx.listener(|this, _: &crate::ToggleSideLayout, _, cx| this.lay_out(cx, Ui::toggle_side)))
            .on_action(
                cx.listener(|this, _: &crate::ToggleTerminalOnly, _, cx| this.lay_out(cx, Ui::toggle_terminal_only)),
            )
            .on_action(cx.listener(|this, _: &crate::ToggleFullReview, _, cx| {
                if this.frame.read(cx).current_workspace().is_some() {
                    this.lay_out(cx, Ui::toggle_full_review);
                }
            }))
            // ⌘+, ⌘− and ⌘0: the text of what has the keyboard, bigger, smaller, or as usual
            .on_action(cx.listener(|this, _: &crate::TextBigger, window, cx| this.resize_text(Some(true), window, cx)))
            .on_action(
                cx.listener(|this, _: &crate::TextSmaller, window, cx| this.resize_text(Some(false), window, cx)),
            )
            .on_action(cx.listener(|this, _: &crate::TextUsualSize, window, cx| this.resize_text(None, window, cx)))
            // the files' column: ⌥⌘1 its Changes, ⌥⌘2 its Files
            .on_action(cx.listener(|this, _: &crate::ToggleChangedFiles, _, cx| {
                this.lay_out(cx, |ui| ui.toggle_panel(Panel::Changes))
            }))
            .on_action(
                cx.listener(|this, _: &crate::ToggleFiles, _, cx| this.lay_out(cx, |ui| ui.toggle_panel(Panel::Files))),
            )
            // ⌘S: the rail and the files' column go, or come back
            .on_action(cx.listener(|this, _: &crate::ToggleLists, _, cx| this.lay_out(cx, Ui::toggle_lists)))
            // Search (DESIGN.md, "Search"): ⇧⌘O or ⌘P its Files, ⌘O its Symbols, ⇧⌘F its Text
            .on_action(cx.listener(|this, _: &crate::SearchFiles, window, cx| {
                this.open_search(Some(enkidu_model::search::Tab::Files), window, cx)
            }))
            .on_action(cx.listener(|this, _: &crate::SearchSymbols, window, cx| {
                this.open_search(Some(enkidu_model::search::Tab::Symbols), window, cx)
            }))
            .on_action(cx.listener(|this, _: &crate::SearchText, window, cx| {
                this.open_search(Some(enkidu_model::search::Tab::Text), window, cx)
            }))
            .on_action(cx.listener(|this, _: &crate::OpenSearch, window, cx| this.open_search(None, window, cx)))
            // the Work page (⌘I): it comes on screen, or the workspace is back
            .on_action(cx.listener(|this, _: &crate::ToggleWork, _, cx| {
                if this.work.is_some() {
                    this.frame.update(cx, |frame, cx| {
                        if frame.toggle_work() {
                            cx.notify();
                        }
                    });
                }
            }))
            // Settings (⌘,)
            .on_action(cx.listener(|this, _: &crate::OpenSettings, window, cx| {
                crate::settings::open_here(this.frame.clone(), enkidu_model::settings::Page::General, window, cx)
            }))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            // (the frame's own black: the stage stands on it)
            .bg(theme::PANE)
            .text_color(theme::INK)
            .font_family(theme::UI_FONT)
            .text_size(theme::TEXT)
            // each part of the frame is a view of its own that reuses its last drawing until it changes, so a turning
            // lamp lays out and draws only the top row and the status line (item 1.18b); a cached view takes its size
            // from here, not from its contents. The Work page draws the top row and the stage itself
            .map(|shell| match work {
                Some(page) => shell.child(page),
                None => shell.child(self.tabs.clone().cached(row(theme::TOP_ROW))).child(self.workspace_row(
                    body,
                    island_shown,
                    cx,
                )),
            })
            .child(self.status.clone().cached(row(theme::STATUS_LINE)))
            // the rail stands at the left, between the top row and the status line, over the room both screens leave
            // for it. With the lists hidden (⌘S) it is away
            .when(!hidden, |shell| {
                shell.child(
                    div().absolute().left_0().top(theme::TOP_ROW).bottom(theme::STATUS_LINE).w(theme::RAIL).child(
                        self.rail.clone().cached(StyleRefinement::default().w(theme::RAIL).h_full().flex_none()),
                    ),
                )
            })
            // the switcher, over everything of the window while ⌃ is held: it has the keyboard, and takes every press
            .children(self.switcher.as_ref().map(|(cards, at)| {
                let this = cx.entity().downgrade();
                // a card shows its project as the rail does: its own icon where it has one
                let icons: HashMap<usize, std::path::PathBuf> = {
                    let rail = self.rail.read(cx);
                    (self.frame.read(cx).projects.iter().enumerate())
                        .filter_map(|(place, project)| Some((place, rail.icon_of(&project.id, cx)?)))
                        .collect()
                };
                div()
                    .id("switcher-layer")
                    .track_focus(&self.switch_focus)
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        cx.stop_propagation();
                        match event.keystroke.key.as_str() {
                            "escape" => this.switched(false, window, cx),
                            "enter" => this.switched(true, window, cx),
                            "right" | "down" => this.switch(true, window, cx),
                            "left" | "up" => this.switch(false, window, cx),
                            _ => {}
                        }
                    }))
                    .absolute()
                    .inset_0()
                    .occlude()
                    // a click beside the cards: you stay
                    .on_click(cx.listener(|this, _, window, cx| this.switched(false, window, cx)))
                    .child(switcher::sheet(cards, *at, &icons, &self.switch_scroll, window, move |n, window, cx| {
                        cx.stop_propagation();
                        let _ = this.update(cx, |this, cx| {
                            if let Some((_, at)) = &mut this.switcher {
                                *at = n;
                            }
                            this.switched(true, window, cx);
                        });
                    }))
            }))
            // the window's toasts in its bottom right corner, and its tooltip above everything
            .child(crate::toast::layer(window, cx))
            .child(enkidu_ui::tips::layer(window, cx))
    }
}

/// How wide the files are while they stand over the stage (the lists are hidden).
const FILES_OVER: f32 = 320.;

/// The space a row of the frame takes: the window's width, `height` tall.
fn row(height: gpui_kit::Pixels) -> StyleRefinement {
    StyleRefinement::default().w_full().h(height).flex_none()
}
