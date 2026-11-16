//! The window's frame (DESIGN.md, "Layout"): the projects row with the traffic lights in it, the workspace tabs of
//! the project on screen, one island holding the workspace's terminals, and the status line. Everything but the
//! terminals comes from the frame's store (`enkidu_model::frame::Frame`): the live one in the app, a sample one for
//! pictures. The terminals come from theirs (`crate::terminals::Terminals`): the core's in the app, given ones for
//! pictures and tests.

use std::sync::Arc;

use enkidu_model::changes::ChangeLists;
use enkidu_model::frame::Frame;
use enkidu_model::search::Target;
use enkidu_model::ui::{self, Mode, Panel, Ui};
use enkidu_term::{GridSize, Palette, Terminal};
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    App, ClickEvent, Context, CursorStyle, DragMoveEvent, Entity, FocusHandle, ModifiersChangedEvent, Render,
    StyleRefinement, Window, div, px,
};

use crate::changes_island::{ChangesIsland, Source};
use crate::demo::{self, Tab};
use crate::projects_row::ProjectsRow;
use crate::right_edge::{ToolPanel, ToolRail};
use crate::search::SearchStore;
use crate::status_line::StatusLine;
use crate::tabs_row::WorkspaceTabs;
use crate::terminal_area::TerminalArea;
use crate::terminal_view::{TerminalText, TerminalView};
use crate::terminals::Terminals;
use crate::work::page::WorkPage;
use crate::work::store::Work;
use enkidu_ui::pieces::{rise, slide_in};
use enkidu_ui::tokens as theme;

pub struct Shell {
    frame: Entity<Frame>,
    projects: Entity<ProjectsRow>,
    tabs: Entity<WorkspaceTabs>,
    status: Entity<StatusLine>,
    /// The window's own focus, for when there is no terminal to hold the keyboard (the keys for tabs still work).
    focus: FocusHandle,
    /// Whether a workspace was on screen when last looked at: when none is left, the window takes the keyboard.
    shown: bool,
    /// The island: the terminals of the workspace on screen.
    area: Entity<TerminalArea>,
    /// The terminals given to a window made for pictures and tests (none in the app, whose terminals the core runs).
    given: Vec<Entity<TerminalView>>,
    /// The changes island: the diffs of the workspace on screen. None in a window without stores (some pictures).
    changes: Option<Entity<ChangesIsland>>,
    /// The right edge: the bar that opens the panels, and the panel. None in a window without a changes island.
    edge: Option<(Entity<ToolRail>, Entity<ToolPanel>)>,
    /// How the workspace is laid out, as it was left.
    ui: Entity<Ui>,
    /// The space between the two islands that is being dragged.
    dragging: Option<Gap>,
    /// What Search needs here: the changes it lists, what it keeps, and where its answers come from. None in a
    /// window that has nothing to search.
    search: Option<(Entity<ChangeLists>, Entity<SearchStore>, crate::search::Source)>,
    /// Shift tapped twice opens Search (DESIGN.md, "Search"): the taps so far, and the hearing of every other key.
    taps: enkidu_model::search::ShiftTaps,
    /// The Work page, which takes the place of the tabs and islands while it is open (`Frame::work_open`). None in
    /// a window without a Work index (some pictures).
    work: Option<Entity<WorkPage>>,
    /// Whether the Work page was on screen when last drawn: as it comes or goes, the keyboard goes with it.
    work_shown: bool,
    _keys: gpui_kit::Subscription,
    /// Hears when the keyboard is nowhere (see `keyboard_home`).
    _lost: gpui_kit::Subscription,
    /// What arrived while the window was open, for its motion (DESIGN.md, "Motion").
    arrivals: Arrivals,
    /// How often this window was drawn. A test reads it to see that nothing asks for a frame while nothing happens.
    drawn: u64,
}

/// The things that arrive on this screen and move as they do: the changes rise in when Claude's first edit lands,
/// a panel slides in from the right edge when it is opened. What is there as the window opens, or when another
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
    /// The panel that was open when last drawn.
    panel: Option<Option<Panel>>,
    /// How many times a panel was opened, or another one took its place.
    slides: u32,
    /// The panel that was opened last is still on screen (as for `rising`).
    sliding: bool,
}

/// The space between the two islands of a workspace: it is where they are resized (DESIGN.md, "Layout": "The space
/// between two islands drags to resize and remembers; double-click it to reset").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gap {
    /// Between the terminal and the changes beside it: it sets the changes' width.
    Beside,
    /// Between the changes and the terminal under them: it sets the terminal's height.
    Under,
    /// Between the workspace and the panel at its right edge: it sets the panel's width.
    Panel,
}

/// What follows the pointer while a gap is dragged: nothing, the islands themselves move.
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
        let projects = cx.new(|cx| ProjectsRow::new(frame.clone(), work.clone(), cx));
        let tabs = cx.new(|cx| WorkspaceTabs::new(frame.clone(), ui.clone(), work.clone(), cx));
        // what you wrote about each card: the stores', kept across launches; a window without stores has its own
        let drafts = (cx.try_global::<crate::stores::Stores>().map(|stores| stores.drafts.clone()))
            .unwrap_or_else(|| cx.new(|_| crate::work::drafts::Drafts::default()));
        let typed_into = terminals.clone();
        let page = work.clone().map(|work| {
            cx.new(|cx| WorkPage::new(frame.clone(), work, ui.clone(), drafts.clone(), Some(typed_into), cx))
        });
        let status = cx.new(|cx| StatusLine::new(frame.clone(), work.zip(page.clone()), cx));
        let work = page;
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
            projects,
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

    /// How often this window was drawn since it opened: every frame draws its root.
    pub fn drawn(&self) -> u64 {
        self.drawn
    }

    /// The Work page's view, in a window that has one.
    pub fn work_page(&self) -> Option<&Entity<WorkPage>> {
        self.work.as_ref()
    }

    /// The Work page is on screen, in place of the workspace's tabs and islands.
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

    pub fn tabs(&self) -> &Entity<WorkspaceTabs> {
        &self.tabs
    }

    /// Where the workspace on screen is among the tabs of the project on screen.
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

    /// The Files panel's view, in a window that has a right edge.
    pub fn files_pane(&self, cx: &App) -> Option<Entity<crate::files_pane::FilesPane>> {
        self.edge.as_ref().map(|(_, panel)| panel.read(cx).files().clone())
    }

    /// The changes island, in a window on the stores.
    pub fn changes(&self) -> Option<&Entity<ChangesIsland>> {
        self.changes.as_ref()
    }

    /// The right edge of a window that has a changes island: its bar and its panel.
    fn right_edge(
        frame: &Entity<Frame>,
        lists: &Entity<enkidu_model::changes::ChangeLists>,
        ui: &Entity<Ui>,
        remarks: &Entity<crate::remarks::Remarks>,
        island: &Entity<ChangesIsland>,
        files: crate::files_pane::Source,
        cx: &mut Context<Self>,
    ) -> (Entity<ToolRail>, Entity<ToolPanel>) {
        let rail = cx.new(|cx| ToolRail::new(frame.clone(), lists.clone(), ui.clone(), cx));
        let panel = cx.new(|cx| {
            ToolPanel::new(frame.clone(), lists.clone(), ui.clone(), remarks.clone(), island.clone(), files, cx)
        });
        (rail, panel)
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

    /// The gap being dragged moved: the island on its far side gets what the pointer asks for, within what each
    /// island keeps. `event.bounds` are the two islands together.
    fn drag_gap(&mut self, event: &DragMoveEvent<Gap>, window: &mut Window, cx: &mut Context<Self>) {
        let gap = *event.drag(cx);
        let (at, room) = (event.event.position, event.bounds);
        self.dragging = Some(gap);
        // the panel's gap is measured against the whole workspace, not the two islands (see `drag_panel_gap`)
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
        // the pointer leaves the gap within a pixel or two: it keeps saying what it is doing
        let cursor = if gap == Gap::Beside { CursorStyle::ResizeLeftRight } else { CursorStyle::ResizeUpDown };
        cx.set_active_drag_cursor_style(cursor, window);
        cx.notify();
    }

    /// The gap beside the panel at the right edge moved: the panel gets what the pointer asks for, between 200 and
    /// 520 px. `event.bounds` are the whole workspace, bar included.
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

    /// The space between the two islands: 8 px that drag to resize the island on their far side, and put it back to
    /// its first size on a double click. Pointing at it shows the line that is about to move.
    fn gap(&self, gap: Gap, cx: &mut Context<Self>) -> impl IntoElement {
        let beside = gap != Gap::Under;
        let moving = self.dragging == Some(gap) && cx.has_active_drag();
        let line = div()
            .absolute()
            .rounded_full()
            .bg(theme::ACCENT)
            .map(|line| match beside {
                true => line.top(px(12.)).bottom(px(12.)).left(px(3.)).w(px(2.)),
                false => line.left(px(12.)).right(px(12.)).top(px(3.)).h(px(2.)),
            })
            .opacity(if moving { 0.8 } else { 0. })
            .group_hover("gap", |line| line.opacity(0.8));
        div()
            .id(if gap == Gap::Panel { "panel-divider" } else { "layout-divider" })
            .test_support()
            .aria_label(match gap {
                Gap::Beside => "Resize changes",
                Gap::Under => "Resize terminal",
                Gap::Panel => "Resize panel",
            })
            .relative()
            .flex_none()
            .map(|space| match beside {
                true => space.w(theme::GAP).h_full().cursor_col_resize(),
                false => space.h(theme::GAP).w_full().cursor_row_resize(),
            })
            .group("gap")
            .child(line)
            .on_drag(gap, |_, _, _, cx| cx.new(|_| Nothing))
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                if event.click_count() == 2 {
                    this.lay_out(cx, |ui| match gap {
                        Gap::Beside => ui.review_width = ui::REVIEW_WIDTH,
                        Gap::Under => ui.terminal_height = ui::TERMINAL_HEIGHT,
                        Gap::Panel => ui.right_panel_width = ui::PANEL_WIDTH,
                    });
                }
            }))
    }

    /// What stands under the tabs: the workspace's islands, and at its right edge the bar with the panel it opened
    /// beside it, at full height (DESIGN.md, "The right edge is a tool bar"). The bar is there while a workspace is on
    /// screen.
    fn workspace_row(
        &self,
        body: gpui_kit::AnyElement,
        workspace_shown: bool,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let Some((rail, panel)) = self.edge.as_ref().filter(|_| workspace_shown) else {
            return div().flex_1().min_h_0().px(theme::GAP).child(body).into_any_element();
        };
        let ui = self.ui.read(cx);
        let (open, width) = (ui.right_panel.is_some(), ui.right_panel_width as f32);
        let turn = if self.arrivals.sliding { self.arrivals.slides } else { 0 };
        let (rail, panel) = (rail.clone(), slide_in("panel-slide", turn, panel.clone()));
        div()
            .id("workspace-row")
            .test_support()
            .flex_1()
            .min_h_0()
            .pl(theme::GAP)
            .flex()
            .on_drag_move(cx.listener(Self::drag_panel_gap))
            .child(div().flex_1().min_w_0().h_full().child(body))
            // the panel is never more than 40% of the workspace
            .when(open, |row| {
                row.child(self.gap(Gap::Panel, cx))
                    .child(div().w(px(width)).max_w(gpui_kit::relative(0.4)).h_full().flex_none().child(panel))
            })
            .child(rail)
            .into_any_element()
    }

    /// The terminal on screen in the island, if the workspace on screen has one.
    pub fn terminal_on_screen(&self, cx: &App) -> Option<Entity<TerminalView>> {
        self.area.read(cx).active_view(cx)
    }

    /// ⌘1 to ⌘9 put that tab of the project on screen there, ⌃⇥ and ⇧⌘] the next one, ⌃⇧⇥ and ⇧⌘[ the one before.
    fn pick_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        self.frame.update(cx, |frame, cx| {
            if frame.select_tab(index) {
                cx.notify();
            }
        })
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
        // no workspaces yet: no tabs row at all (the empty screen under it says what to do)
        let (loaded, has_tabs, all_hidden, project) = {
            let frame = self.frame.read(cx);
            let project = frame.current_project.as_ref().and_then(|id| frame.projects.iter().find(|p| &p.id == id));
            let (tabs, hidden) = (!frame.tabs().is_empty(), !frame.hidden_tabs().is_empty());
            (frame.loaded, tabs, !tabs && hidden, project.map(|project| project.name.clone()))
        };
        // the Work page takes the place of the tabs and the islands; as it comes or goes, the keyboard goes with it
        let work = self.work.clone().filter(|_| self.frame.read(cx).work_open);
        if work.is_some() != self.work_shown {
            self.work_shown = work.is_some();
            self.keyboard_home(window, cx);
        }
        // what fills the island's place: nothing until the database was read, an empty screen, or the island
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
            // not cached: in real windows on 12 Nov, a cached island (it holds the terminal that has the keyboard) gave 0
            // frames a second while a lamp turned, three runs in a row; uncached, the runs before and after gave 24
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
                let island = island.map(|island| rise("changes-rise", turn, island));
                // an island that leaves the screen hands the keyboard to the one that stays
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
                        .child(div().flex_1().min_h(px(ui::MIN_CHANGES_HEIGHT)).child(island))
                        .child(self.gap(Gap::Under, cx))
                        .child(
                            div()
                                .h(px(ui.terminal_height as f32))
                                .min_h(px(ui::MIN_TERMINAL_HEIGHT))
                                .flex_shrink(1.)
                                .child(self.area.clone()),
                        )
                        .into_any_element(),
                    // the changes beside the terminal, which keeps its least width
                    (Mode::Side, Some(island)) => div()
                        .id("workspace-body")
                        .test_support()
                        .size_full()
                        .flex()
                        .on_drag_move(cx.listener(Self::drag_gap))
                        .child(div().flex_1().h_full().min_w(px(ui::MIN_TERMINAL_WIDTH)).child(self.area.clone()))
                        .child(self.gap(Gap::Beside, cx))
                        .child(div().w(px(ui.review_width as f32)).h_full().min_w_0().flex_shrink(1.).child(island))
                        .into_any_element(),
                    // the changes alone; the terminals keep running behind them
                    (Mode::Review, Some(island)) => island,
                    _ => self.area.clone().into_any_element(),
                }
            }
        };
        let island_shown = loaded && has_tabs;
        // a panel that was opened while you watch (or one that took another's place) slides in
        let panel = self.ui.read(cx).right_panel;
        if self.arrivals.panel.is_some_and(|was| was != panel) && panel.is_some() && island_shown {
            (self.arrivals.slides, self.arrivals.sliding) = (self.arrivals.slides + 1, true);
        } else if panel.is_none() || !island_shown {
            self.arrivals.sliding = false;
        }
        self.arrivals.panel = Some(panel);
        let no_terminal = self.area.read(cx).active_view(cx).is_none();
        div()
            .id("shell")
            // only while no terminal is shown: a frame that takes focus would take the keyboard from the terminal on any
            // click
            .when(no_terminal || !island_shown, |shell| shell.track_focus(&self.focus))
            // ⇧⇧: Shift tapped twice opens Search on the tab used last. Only two clean taps count (see
            // `enkidu_model::search::ShiftTaps`), by the clock that tests can hold
            .on_modifiers_changed(cx.listener(|this, event: &ModifiersChangedEvent, window, cx| {
                let held = event.modifiers;
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
            // the panels at the right edge: ⌥⌘1 Changed files, ⌥⌘2 Files
            .on_action(cx.listener(|this, _: &crate::ToggleChangedFiles, _, cx| {
                this.lay_out(cx, |ui| ui.toggle_panel(Panel::Changes))
            }))
            .on_action(
                cx.listener(|this, _: &crate::ToggleFiles, _, cx| this.lay_out(cx, |ui| ui.toggle_panel(Panel::Files))),
            )
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
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::FRAME)
            .text_color(theme::INK)
            .font_family(theme::UI_FONT)
            .text_size(theme::TEXT)
            // each part is a view of its own that reuses its last drawing until it changes, so a turning lamp lays out
            // and draws only its tabs row (item 1.18b); a cached view takes its size from here, not from its contents
            .child(self.projects.clone().cached(row(theme::PROJECTS_ROW)))
            .map(|shell| match work {
                Some(page) => shell.child(page),
                None => shell
                    // the tabs row: there with a tab, and with none while workspaces are hidden (their button is in it)
                    .when(has_tabs || all_hidden, |shell| shell.child(self.tabs.clone().cached(row(theme::TABS_ROW))))
                    .child(self.workspace_row(body, island_shown, cx)),
            })
            .child(self.status.clone().cached(row(theme::STATUS_LINE)))
            // the window's toasts in its bottom right corner, and its tooltip above everything
            .child(crate::toast::layer(window, cx))
            .child(enkidu_ui::tips::layer(window, cx))
    }
}

/// The space a row of the frame takes: the window's width, `height` tall.
fn row(height: gpui_kit::Pixels) -> StyleRefinement {
    StyleRefinement::default().w_full().h(height).flex_none()
}
