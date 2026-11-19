//! The app's stores (item 1.15): one per subject. The first is the frame's:
//! projects, workspaces, their lamps, and what is on screen.
//!
//! Instant first, fresh second (DESIGN.md, Principle 9): what the database remembers shows as soon as one call off
//! the drawing thread has read it, with where you were last time; then the core's events keep it fresh. Events that
//! come while that first read runs wait their turn, so none is lost.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use enkidu_core::events::Event;
use enkidu_core::git::Change;
use enkidu_model::changes::ChangeLists;
use enkidu_model::frame::{Frame, View};
use enkidu_model::search::Recent;
use enkidu_model::ui::Ui;
use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedReceiver, unbounded};
use gpui_kit::{App, AppContext as _, AsyncApp, Entity, Global};

use crate::bridge::Bridge;
use crate::remarks::Remarks;
use crate::terminals::{Feeds, Terminals};
use crate::work::drafts::Drafts;
use crate::work::store::Work;

/// The core's news of the steps of making a workspace, in order (the New workspace dialog reads what it hasn't yet).
#[derive(Default)]
pub struct ProgressLog(pub Vec<enkidu_core::workspace::Progress>);

/// The stores, and the running core they come from.
pub struct Stores {
    pub frame: Entity<Frame>,
    pub progress: Entity<ProgressLog>,
    /// Every workspace's terminals, as the core runs them.
    pub terminals: Entity<Terminals>,
    /// Every workspace's changed files.
    pub changes: Entity<ChangeLists>,
    /// How the window is laid out, as it was left.
    pub ui: Entity<Ui>,
    /// Every workspace's remarks that weren't sent to Claude yet.
    pub remarks: Entity<Remarks>,
    /// What Search keeps (the tab used last, what was typed) and the files opened lately in each workspace.
    pub search: Entity<crate::search::SearchStore>,
    /// What the core said on its own that the window should say in a toast (Live set an edit aside, or couldn't copy).
    pub notices: Entity<Notices>,
    /// The Work index: your tickets and PRs, the Claudes behind them, what waits on you, how each source stands.
    pub work: Entity<Work>,
    /// What you wrote about each card of the Work page, kept until it goes.
    pub drafts: Entity<Drafts>,
    /// Each project's own icon, for its button on the rail.
    pub icons: Entity<Icons>,
    core: Option<Arc<enkidu_core::Core>>,
}

/// Each project's own icon, by project: the picture its settings name, or the one found in its folder. A project
/// that has none shows its letter.
#[derive(Default)]
pub struct Icons {
    pub of: HashMap<String, PathBuf>,
    /// The projects the icons were last looked for, in their order.
    asked: Vec<String>,
}

/// Toasts that wait for the window: news that came from the core, not from something you pressed.
#[derive(Default)]
pub struct Notices(pub Vec<Notice>);

/// One of them: what kind of news it is, its title, and what is under it.
#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    pub kind: NoticeKind,
    pub title: String,
    pub detail: String,
}

/// Something to know, something that was done, or something that went wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeKind {
    Note,
    Done,
    Warning,
}

impl Global for Stores {}

/// Where the stores keep what must outlive a launch.
#[derive(Clone, Debug, Default)]
pub struct Places {
    /// The project on screen and each project's workspace you had.
    pub view: Option<PathBuf>,
    /// The workspaces that have a Claude, which come back at the next launch.
    pub sessions: Option<PathBuf>,
    /// Every workspace's changed files as last seen, shown from the first frame of the next launch.
    pub changes: Option<PathBuf>,
    /// How the window is laid out (the diff's layout, and more with each screen).
    pub ui: Option<PathBuf>,
    /// The files opened lately in each workspace, which Search offers before anything is typed.
    pub recent: Option<PathBuf>,
    /// What you wrote about each card of the Work page and didn't send yet.
    pub drafts: Option<PathBuf>,
}

impl Places {
    /// `view.json`, `sessions.json`, `changes.json`, `ui.json`, `recent.json` and `drafts.json` in a data folder.
    pub fn in_dir(dir: &Path) -> Places {
        Places {
            view: Some(dir.join("view.json")),
            sessions: Some(dir.join("sessions.json")),
            changes: Some(dir.join("changes.json")),
            ui: Some(dir.join("ui.json")),
            recent: Some(dir.join("recent.json")),
            drafts: Some(dir.join("drafts.json")),
        }
    }
}

impl Stores {
    pub fn frame(cx: &App) -> Entity<Frame> {
        cx.global::<Stores>().frame.clone()
    }

    pub fn progress(cx: &App) -> Entity<ProgressLog> {
        cx.global::<Stores>().progress.clone()
    }

    pub fn terminals(cx: &App) -> Entity<Terminals> {
        cx.global::<Stores>().terminals.clone()
    }

    pub fn changes(cx: &App) -> Entity<ChangeLists> {
        cx.global::<Stores>().changes.clone()
    }

    pub fn ui(cx: &App) -> Entity<Ui> {
        cx.global::<Stores>().ui.clone()
    }

    pub fn remarks(cx: &App) -> Entity<Remarks> {
        cx.global::<Stores>().remarks.clone()
    }

    pub fn search(cx: &App) -> Entity<crate::search::SearchStore> {
        cx.global::<Stores>().search.clone()
    }

    pub fn work(cx: &App) -> Entity<Work> {
        cx.global::<Stores>().work.clone()
    }

    /// A project's own icon, where one was found for it (none in a window made without the stores).
    pub fn icon_of(project: &str, cx: &App) -> Option<PathBuf> {
        cx.try_global::<Stores>().and_then(|stores| stores.icons.read(cx).of.get(project).cloned())
    }

    /// Looks for every project's icon again, off the drawing thread: as the projects are read, when one is added or
    /// removed, and after a project's settings were saved.
    pub fn read_icons(cx: &mut App) {
        let Some(icons) = cx.try_global::<Stores>().map(|stores| stores.icons.clone()) else { return };
        let answer = Bridge::ask(cx, enkidu_core::commands::project_icons);
        cx.spawn(async move |cx: &mut AsyncApp| {
            let Ok(Ok(found)) = answer.await else { return };
            let found: HashMap<String, PathBuf> =
                found.into_iter().map(|(id, path)| (id, PathBuf::from(path))).collect();
            cx.update(|cx| {
                icons.update(cx, |icons, cx| {
                    if icons.of != found {
                        icons.of = found;
                        cx.notify();
                    }
                })
            });
        })
        .detach();
    }

    /// Asks git for a workspace's changes again, as when it comes on screen. Mostly nothing changed, and the list shown
    /// is kept. The workspace on screen changed (none: an empty screen). The core's watcher is told, so it looks at
    /// that one's files a fraction of a second after an edit and at the others a little later, and git is asked for its
    /// changes now.
    pub fn on_screen(workspace: Option<&str>, cx: &mut App) {
        if cx.try_global::<Stores>().is_none() {
            return;
        }
        let id = workspace.map(String::from);
        Bridge::call(cx, move |shared| enkidu_core::commands::watch_workspace(shared, id), |_, _| {});
        if let Some(id) = workspace {
            Self::refresh_changes(id, cx);
        }
    }

    /// Asks git for a workspace's changes now. One question at a time for a workspace: a visit while one is out is
    /// answered by one more question when that one ends (going back and forth between two big workspaces started a
    /// git for every visit, over thirty at once in the load test).
    pub fn refresh_changes(workspace: &str, cx: &mut App) {
        let Some(lists) = cx.try_global::<Stores>().map(|stores| stores.changes.clone()) else { return };
        let asking = cx.default_global::<Asking>();
        if !asking.one.wish(workspace) {
            return;
        }
        asking.asked += 1;
        let id = workspace.to_string();
        let answer = Bridge::ask(cx, {
            let id = id.clone();
            move |shared| enkidu_core::commands::get_changes(shared, id)
        });
        cx.spawn(async move |cx: &mut AsyncApp| {
            // the watcher keeps trying; a failed read leaves the last good list
            let files = answer.await;
            cx.update(|cx| {
                if let Ok(Ok(files)) = files {
                    lists.update(cx, |lists, cx| {
                        if lists.set(&id, files) {
                            cx.notify();
                        }
                    });
                }
                // answered or not, the question is over: one more for the visits meanwhile
                if cx.default_global::<Asking>().one.ended(&id) {
                    Stores::refresh_changes(&id, cx);
                }
            });
        })
        .detach();
    }
}

/// Which workspaces git is being asked about because they came on screen, and how many questions that made.
#[derive(Default)]
struct Asking {
    one: enkidu_model::changes::OneAtATime,
    asked: u64,
}

/// How many times git was asked for a workspace's changes because it came on screen (tests count them).
pub fn changes_asked(cx: &mut App) -> u64 {
    cx.default_global::<Asking>().asked
}

impl gpui_kit::Global for Asking {}

/// The app's own places, in its data folder.
pub fn places() -> Places {
    Places::in_dir(&enkidu_core::places::data_dir())
}

pub fn load_view(path: &Path) -> Option<View> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn save_view(path: &Path, view: &View) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string(view).map_err(std::io::Error::other)?)?;
    std::fs::rename(tmp, path)
}

/// Starts the core in the app's own data folder and the stores on it. The core's news reaches the stores on the
/// drawing thread; the app shuts the core down as it quits.
pub fn start(cx: &mut App) -> anyhow::Result<Entity<Frame>> {
    let (tx, events) = unbounded::<Event>();
    // a terminal's output goes straight to its parser, never through the drawing thread
    let feeds = Arc::new(Feeds::default());
    let sink = crate::terminals::sink(tx, feeds.clone());
    let core = Arc::new(enkidu_core::start(sink.clone())?);
    Bridge::install(cx, core.shared.clone(), sink);
    let mut stores = install(events, feeds, places(), cx);
    stores.core = Some(core);
    let frame = stores.frame.clone();
    cx.set_global(stores);
    cx.on_app_quit(|cx| {
        let core = cx.try_global::<Stores>().and_then(|s| s.core.clone());
        async move {
            if let Some(core) = core {
                core.shutdown();
            }
        }
    })
    .detach();
    Ok(frame)
}

/// The stores on a core the [`Bridge`] already reaches: first what the database remembers (with the view from
/// `view`), then `events` in order. Where you are is kept in `view` a moment after it changes. The stores come back
/// without the core that runs them (`start` holds it).
pub fn install(mut events: UnboundedReceiver<Event>, feeds: Arc<Feeds>, places: Places, cx: &mut App) -> Stores {
    let frame = cx.new(|_| Frame::default());
    let progress = cx.new(|_| ProgressLog::default());
    let changes = cx.new(|_| ChangeLists::default());
    let lists = changes.clone();
    let Places { view, sessions, changes: kept_changes, ui: kept_ui, recent: kept_recent, drafts: kept_drafts } =
        places;
    // a few bytes, read before the first frame so the window opens laid out as it was left
    let ui = cx.new(|_| {
        kept_ui
            .as_deref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map_or_else(Ui::default, |text| Ui::read(&text))
    });
    let terminals = cx.new(|_| Terminals::live(feeds, sessions));
    let notices = cx.new(|_| Notices::default());
    let said = notices.clone();
    let work = cx.new(|_| Work::default());
    let index = work.clone();
    // its cards follow the workspaces and their lamps
    crate::work::store::follow_frame(&work, &frame, cx);
    // and the clock: every half minute the ages on the page move on and a snoozed item whose time came wakes, without
    // any news from the core
    let ticking = work.downgrade();
    cx.spawn(async move |cx: &mut AsyncApp| {
        loop {
            cx.background_executor().timer(crate::work::store::TICK).await;
            let Some(work) = ticking.upgrade() else { break };
            cx.update(|cx| {
                work.update(cx, |work, cx| {
                    if work.snapshot.is_some() {
                        work.regroup();
                        cx.notify();
                    }
                })
            });
        }
    })
    .detach();
    let shown = terminals.clone();
    let log = progress.clone();
    let remembered = view.as_deref().and_then(load_view).unwrap_or_default();
    // one call off the drawing thread reads the database and the changes as last seen
    let first = Bridge::ask(cx, {
        let path = kept_changes.clone();
        move |shared| {
            let last_seen: HashMap<String, Vec<Change>> = path
                .and_then(|path| std::fs::read_to_string(path).ok())
                .and_then(|text| serde_json::from_str(&text).ok())
                .unwrap_or_default();
            (enkidu_core::commands::get_state(shared), last_seen)
        }
    });
    let store = frame.clone();
    cx.spawn(async move |cx: &mut AsyncApp| {
        match first.await {
            Ok((Ok(state), last_seen)) => {
                cx.update(|cx| {
                    // instant first: every workspace's changes as last seen, with what the watcher already knows on top,
                    // so the tabs' counts are right from the first frame
                    let ids: Vec<String> = state.workspaces.iter().map(|w| w.id.clone()).collect();
                    let loaded = ChangeLists::load(last_seen, state.changes, &ids);
                    store.update(cx, |frame, cx| {
                        *frame = Frame::load(state.projects, state.workspaces, state.statuses, &remembered);
                        for id in &ids {
                            if let Some(files) = loaded.of(id) {
                                frame.changed.insert(id.clone(), enkidu_model::frame::Changes::of(files));
                            }
                        }
                        cx.notify();
                    });
                    lists.update(cx, |lists, cx| {
                        *lists = loaded;
                        cx.notify();
                    });
                    // the Work index as the core's database has it; its watchers bring what is new
                    Work::read(&index, cx);
                });
            }
            Ok((Err(err), _)) => {
                eprintln!("Enkidu: the projects could not be read: {}", err.message);
                // nothing more will come: show what there is (no project), not a blank window
                cx.update(|cx| {
                    store.update(cx, |frame, cx| {
                        frame.loaded = true;
                        cx.notify();
                    })
                });
            }
            Err(_) => return,
        }
        while let Some(event) = events.next().await {
            cx.update(|cx| {
                if let Event::WorkspaceProgress(step) = &event {
                    log.update(cx, |log, cx| {
                        log.0.push(step.clone());
                        cx.notify();
                    });
                }
                store.update(cx, |frame, cx| {
                    if frame.apply(&event) {
                        cx.notify();
                    }
                });
                if let Event::ChangesUpdated { workspace_id, files } = &event {
                    lists.update(cx, |lists, cx| {
                        if lists.set(workspace_id, files.clone()) {
                            cx.notify();
                        }
                    });
                }
                // Live while it follows a workspace: an edit of the main folder's own was set aside, or a change couldn't
                // be copied. The window says it
                if let Event::LiveNote { error, text, .. } = &event {
                    said.update(cx, |notices, cx| {
                        let title = enkidu_model::live::note_title(*error).to_string();
                        let kind = if *error { NoticeKind::Warning } else { NoticeKind::Note };
                        notices.0.push(Notice { kind, title, detail: text.clone() });
                        cx.notify();
                    });
                }
                // the Work index changed (a read ended, a hook linked a session, you hid a card)
                if let Event::WorkChanged = &event {
                    Work::read(&index, cx);
                }
                // the morning summary ended: how, in a toast (Settings keeps the latest runs)
                if let Event::WorkflowRan(run) = &event
                    && let Some((kind, title, detail)) = enkidu_model::work::runs::ran_toast(run)
                {
                    said.update(cx, |notices, cx| {
                        let kind = match kind {
                            enkidu_model::work::runs::RanKind::Done => NoticeKind::Done,
                            enkidu_model::work::runs::RanKind::Note => NoticeKind::Note,
                            enkidu_model::work::runs::RanKind::Failed => NoticeKind::Warning,
                        };
                        notices.0.push(Notice { kind, title: title.to_string(), detail });
                        cx.notify();
                    });
                }
                // who shows what a workflow did (a project's page in Settings) hears of every piece of that news,
                // also of news that leaves the index as it was
                if matches!(event, Event::WorkChanged | Event::WorkflowRan(_)) {
                    index.update(cx, |_, cx| cx.emit(crate::work::store::WorkNews));
                }
                shown.update(cx, |terminals, cx| terminals.apply(&event, cx));
            });
        }
    })
    .detach();
    if let Some(path) = view {
        keep_view(&frame, path, cx);
    }
    if let Some(path) = kept_changes {
        keep_changes(&changes, path, cx);
    }
    if let Some(path) = kept_ui {
        keep_ui(&ui, path, cx);
    }
    // each workspace's remarks, read from the database as the workspace becomes known
    let remarks = cx.new(|cx| Remarks::new(frame.clone(), terminals.clone(), true, cx));
    // the files opened lately: a few lines, read before the first frame like the layout
    let search = cx.new(|_| crate::search::SearchStore {
        recent: kept_recent
            .as_deref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map_or_else(Recent::default, |text| Recent::read(&text)),
        ..Default::default()
    });
    if let Some(path) = kept_recent {
        keep_recent(&search, &frame, path, cx);
    }
    // what you wrote about a card and didn't send: a few lines, read before the first frame
    let drafts = cx.new(|_| {
        kept_drafts
            .as_deref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map_or_else(Drafts::default, |text| Drafts::read(&text))
    });
    if let Some(path) = kept_drafts {
        keep_drafts(&drafts, path, cx);
    }
    // each project's own icon: looked for whenever the projects themselves change
    let icons = cx.new(|_| Icons::default());
    cx.observe(&frame, {
        let icons = icons.clone();
        move |frame, cx| {
            let projects: Vec<String> = frame.read(cx).projects.iter().map(|project| project.id.clone()).collect();
            if icons.read(cx).asked != projects {
                icons.update(cx, |icons, _| icons.asked = projects);
                Stores::read_icons(cx);
            }
        }
    })
    .detach();
    Stores { frame, progress, terminals, changes, ui, remarks, search, notices, work, drafts, icons, core: None }
}

/// Writes what you wrote about the Work page's cards to `path`, off the drawing thread, a moment after it last
/// changed.
fn keep_drafts(drafts: &Entity<Drafts>, path: PathBuf, cx: &mut App) {
    let turn = std::rc::Rc::new(std::cell::Cell::new(0u64));
    cx.observe(drafts, move |drafts, cx| {
        turn.set(turn.get() + 1);
        let (turn, mine, path, text) = (turn.clone(), turn.get(), path.clone(), drafts.read(cx).write());
        cx.spawn(async move |cx: &mut AsyncApp| {
            cx.background_executor().timer(Duration::from_millis(300)).await;
            // only the last change of a burst is written
            if turn.get() != mine {
                return;
            }
            cx.background_spawn(async move {
                let tmp = path.with_extension("json.tmp");
                if let Err(err) = std::fs::write(&tmp, text).and_then(|_| std::fs::rename(tmp, &path)) {
                    eprintln!("Enkidu: the Work page's drafts could not be kept: {err}");
                }
            })
            .detach();
        })
        .detach();
    })
    .detach();
}

/// Writes how the window is laid out to `path`, off the drawing thread, a moment after it last changed.
/// Writes the files opened lately to `path` when they change, off the drawing thread, and forgets the ones of
/// workspaces that are gone.
fn keep_recent(search: &Entity<crate::search::SearchStore>, frame: &Entity<Frame>, path: PathBuf, cx: &mut App) {
    let written = std::rc::Rc::new(std::cell::RefCell::new(search.read(cx).recent.clone()));
    cx.observe(search, move |search, cx| {
        let recent = search.read(cx).recent.clone();
        if *written.borrow() == recent {
            return;
        }
        *written.borrow_mut() = recent.clone();
        let (written, path) = (written.clone(), path.clone());
        cx.spawn(async move |cx: &mut AsyncApp| {
            cx.background_executor().timer(Duration::from_millis(300)).await;
            // only the last change of a burst is written
            if *written.borrow() != recent {
                return;
            }
            let text = recent.write();
            cx.background_spawn(async move {
                let tmp = path.with_extension("json.tmp");
                if let Err(err) = std::fs::write(&tmp, text).and_then(|_| std::fs::rename(tmp, &path)) {
                    eprintln!("Enkidu: the files opened lately could not be kept: {err}");
                }
            })
            .detach();
        })
        .detach();
    })
    .detach();
    let search = search.clone();
    cx.observe(frame, move |frame, cx| {
        let frame = frame.read(cx);
        if !frame.loaded {
            return;
        }
        let known: Vec<String> = frame.workspaces.iter().map(|ws| ws.id.clone()).collect();
        search.update(cx, |search, cx| {
            if search.recent.keep_only(&known) {
                cx.notify();
            }
        });
    })
    .detach();
}

fn keep_ui(ui: &Entity<Ui>, path: PathBuf, cx: &mut App) {
    let turn = std::rc::Rc::new(std::cell::Cell::new(0u64));
    cx.observe(ui, move |ui, cx| {
        turn.set(turn.get() + 1);
        let (turn, mine, path, text) = (turn.clone(), turn.get(), path.clone(), ui.read(cx).write());
        cx.spawn(async move |cx: &mut AsyncApp| {
            cx.background_executor().timer(Duration::from_millis(300)).await;
            // only the last change of a burst is written
            if turn.get() != mine {
                return;
            }
            cx.background_spawn(async move {
                let tmp = path.with_extension("json.tmp");
                if let Err(err) = std::fs::write(&tmp, text).and_then(|_| std::fs::rename(tmp, &path)) {
                    eprintln!("Enkidu: the layout could not be kept: {err}");
                }
            })
            .detach();
        })
        .detach();
    })
    .detach();
}

/// Writes every workspace's changes to `path` for the next launch, off the drawing thread, at most twice a second.
fn keep_changes(lists: &Entity<ChangeLists>, path: PathBuf, cx: &mut App) {
    let turn = std::rc::Rc::new(std::cell::Cell::new(0u64));
    cx.observe(lists, move |lists, cx| {
        turn.set(turn.get() + 1);
        let (turn, mine, path, lists) = (turn.clone(), turn.get(), path.clone(), lists.clone());
        cx.spawn(async move |cx: &mut AsyncApp| {
            cx.background_executor().timer(Duration::from_millis(500)).await;
            // only the last change of a burst is written
            if turn.get() != mine {
                return;
            }
            let all = cx.update(|cx| lists.read(cx).all());
            cx.background_spawn(async move {
                let written = serde_json::to_string(&all).map_err(std::io::Error::other).and_then(|text| {
                    let tmp = path.with_extension("json.tmp");
                    std::fs::write(&tmp, text)?;
                    std::fs::rename(tmp, &path)
                });
                if let Err(err) = written {
                    eprintln!("Enkidu: the changes could not be kept: {err}");
                }
            })
            .detach();
        })
        .detach();
    })
    .detach();
}

/// Writes where you are to `path`, off the drawing thread, once it has stayed the same for a moment.
fn keep_view(frame: &Entity<Frame>, path: PathBuf, cx: &mut App) {
    let last = std::rc::Rc::new(std::cell::RefCell::new(None::<View>));
    cx.observe(frame, move |frame, cx| {
        let view = frame.read(cx).view();
        if last.borrow().as_ref() == Some(&view) {
            return;
        }
        *last.borrow_mut() = Some(view.clone());
        let (last, path) = (last.clone(), path.clone());
        cx.spawn(async move |cx: &mut AsyncApp| {
            cx.background_executor().timer(Duration::from_millis(300)).await;
            if last.borrow().as_ref() == Some(&view) {
                cx.background_spawn(async move {
                    if let Err(err) = save_view(&path, &view) {
                        eprintln!("Enkidu: where you were could not be kept: {err}");
                    }
                })
                .detach();
            }
        })
        .detach();
    })
    .detach();
}

/// Stores for tests: the frame on a core the test made itself.
pub fn install_for_tests(
    events: UnboundedReceiver<Event>,
    feeds: Arc<Feeds>,
    places: Places,
    cx: &mut App,
) -> Entity<Frame> {
    let stores = install(events, feeds, places, cx);
    let frame = stores.frame.clone();
    cx.set_global(stores);
    frame
}
