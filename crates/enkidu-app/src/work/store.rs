//! The Work store (item 5.1b): the core's Work index as it was last read, for the page, the Work tab's count and each
//! workspace tab's PR; and its cards (the groups of `enkidu_model::work::groups`), built from the index, the workspaces
//! and their lamps.
//!
//! Instant first, fresh second: the index is the core's own database, so the first read at launch shows what was
//! known when the app last ran; the core's watchers then read GitHub and Jira and say when something changed. Each
//! piece of that news asks for the index again, one question at a time: news that comes while a question is out is
//! answered by one more question when that one ends. The index is read and compared off the drawing thread, and the
//! views hear only of an index that differs from the one they show.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use enkidu_core::commands::OwnTaskDraft;
use enkidu_core::db::Workspace;
use enkidu_core::work::flows::Flow;
use enkidu_core::work::model::{Board, BoardPick, OwnTask, Snapshot};
use enkidu_model::frame::Frame;
use enkidu_model::work::groups::{Group, build_groups};
use gpui_kit::{AnyWindowHandle, App, AppContext as _, AsyncApp, Entity, Task};

use crate::bridge::Bridge;
use crate::toast;

/// What you say of something that waits on you: it is done, or it can wait a day.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemState {
    Done,
    Snoozed,
}

/// The core said something about Work (the index changed, a workflow's run ended): told to whoever subscribes, also
/// when the index stays as it is (the store's observers hear only of an index that differs).
pub struct WorkNews;

impl gpui_kit::EventEmitter<WorkNews> for Work {}

/// How often the page's clock moves on: its ages ("read 2 min ago"), and the snoozed items whose time came.
pub const TICK: std::time::Duration = std::time::Duration::from_secs(30);

/// The Work index in the window.
#[derive(Default)]
pub struct Work {
    /// The index as the core last gave it; none until the first read came back.
    pub snapshot: Option<Arc<Snapshot>>,
    /// Its cards: one per ticket, or per PR that names no ticket, of every project and every board, the ones you hid
    /// too, worst first. They name their tickets, PRs and items by their place in `snapshot`.
    pub groups: Arc<Vec<Group>>,
    /// Counts every time the cards were built again (a new index, a lamp, the clock): who shows something made
    /// from them keeps the number, and makes it again when the number moved on.
    pub revision: u64,
    /// The workspaces and their lamps the cards were built from.
    workspaces: Vec<Workspace>,
    statuses: HashMap<String, String>,
    /// A time that stands still, for pictures and tests of the views (none: the clock).
    clock: Option<i64>,
    /// Jira's boards are being read for the board menu (it says so while it has none to show).
    pub boards_loading: bool,
    /// What the failing-checks workflow queued is being sent (`work::queued::run`): one round at a time.
    pub(crate) sending_queued: bool,
    /// A question is out.
    asking: bool,
    /// News came while it was: one more question when it ends.
    again: bool,
    /// How many times the core was asked for the index, and how many of the answers differed from what was shown
    /// (tests count them).
    pub asked: u64,
    pub changed: u64,
}

impl Work {
    /// An index that is given, not read, at a time that stands still: for pictures and for tests of the views.
    pub fn given(snapshot: Snapshot, frame: &Frame, now: i64) -> Work {
        let mut work = Work { snapshot: Some(Arc::new(snapshot)), clock: Some(now), ..Default::default() };
        if !work.follow(frame) {
            work.regroup();
        }
        work
    }

    /// The workspaces and their lamps the cards were built from (what the board's faces are drawn from too).
    pub fn built_from(&self) -> (&[Workspace], &HashMap<String, String>) {
        (&self.workspaces, &self.statuses)
    }

    /// Now, in milliseconds: what ages are counted from, and when a snoozed item wakes.
    pub fn now(&self) -> i64 {
        self.clock.unwrap_or_else(enkidu_core::db::now_ms)
    }

    /// The frame's workspaces or their lamps are no longer the ones the cards were built from (a Claude started
    /// working, a workspace was made): the cards are built again. True when they were.
    pub fn follow(&mut self, frame: &Frame) -> bool {
        if self.workspaces == frame.workspaces && self.statuses == frame.statuses {
            return false;
        }
        (self.workspaces, self.statuses) = (frame.workspaces.clone(), frame.statuses.clone());
        self.regroup();
        true
    }

    /// Builds the cards from the index as it is now (a fraction of a millisecond for an index of usual size, under
    /// 3 ms for one of 520 tickets and 360 PRs: `enkidu_model`'s `a_big_index_is_grouped_in_a_few_milliseconds`).
    pub fn regroup(&mut self) {
        let groups = match &self.snapshot {
            Some(work) => build_groups(work, &self.workspaces, &self.statuses, self.now()),
            None => Vec::new(),
        };
        self.groups = Arc::new(groups);
        self.revision += 1;
    }

    /// Asks the core for the index: at launch, and whenever the core says it changed. In a window without a core (a
    /// picture) nothing is asked.
    pub fn read(store: &Entity<Work>, cx: &mut App) {
        if !cx.has_global::<Bridge>() {
            return;
        }
        let shown = store.update(cx, |work, _| {
            if work.asking {
                work.again = true;
                return None;
            }
            work.asking = true;
            work.asked += 1;
            Some(work.snapshot.clone())
        });
        let Some(shown) = shown else { return };
        let answer = Bridge::ask(cx, move |shared| {
            // a failed read leaves what is shown: the next news asks again, and the health lines say what is wrong
            let read = enkidu_core::commands::work_snapshot(shared).ok()?;
            // most news changes nothing the window shows (a hook that linked nothing new, a read that found the same)
            (shown.as_deref() != Some(&read)).then(|| Arc::new(read))
        });
        let store = store.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            let read = answer.await;
            cx.update(|cx| {
                let again = store.update(cx, |work, cx| {
                    work.asking = false;
                    if let Ok(Some(read)) = read {
                        work.snapshot = Some(read);
                        work.regroup();
                        work.changed += 1;
                        cx.notify();
                    }
                    std::mem::take(&mut work.again)
                });
                // with the index as it is now: what the failing-checks workflow queued goes to its Claude
                crate::work::queued::run(cx);
                if again {
                    Work::read(&store, cx);
                }
            });
        })
        .detach();
    }

    /// Changes the index as the window has it, at once: what you just did shows before the core has kept it (the
    /// core's own index follows with its news).
    fn change(store: &Entity<Work>, cx: &mut App, change: impl FnOnce(&mut Snapshot)) {
        store.update(cx, |work, cx| {
            // changed in place: the index is copied only while someone else still reads the one before (a
            // question to the core that is being compared), never for a view (they keep `revision`, not the index)
            let Some(index) = work.snapshot.as_mut() else { return };
            change(Arc::make_mut(index));
            work.regroup();
            cx.notify();
        });
    }

    /// The board menu opened: Jira's boards are read now (the ones read last show meanwhile). What goes wrong is
    /// said in a toast in `window` (with none, as when the page reads them ahead of the menu, it isn't said).
    pub fn load_boards(store: &Entity<Work>, window: Option<AnyWindowHandle>, cx: &mut App) {
        if !cx.has_global::<Bridge>() || store.read(cx).boards_loading {
            return;
        }
        store.update(cx, |work, cx| {
            work.boards_loading = true;
            cx.notify();
        });
        // the core's news brings the boards themselves, with the index
        let answer =
            Bridge::ask_with_news(cx, |events, shared| enkidu_core::commands::work_boards(events.clone(), shared));
        let store = store.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            let read = answer.await;
            cx.update(|cx| {
                store.update(cx, |work, cx| {
                    work.boards_loading = false;
                    cx.notify();
                })
            });
            if let (Ok(Err(err)), Some(window)) = (read, window) {
                let _ = cx.update_window(window, |_, window, cx| {
                    toast::warning(window, "Couldn't read Jira's boards", err.message, cx)
                });
            }
        })
        .detach();
    }

    /// You picked a Jira board for the page (none: everything read). The page answers at once: its header names
    /// the board, and its cards come when its read lands (the board picked already keeps the ones it shows).
    pub fn pick_board(store: &Entity<Work>, board: Option<Board>, window: AnyWindowHandle, cx: &mut App) {
        let picked = board.clone();
        Work::change(store, cx, move |index| {
            let same = index.board.take().filter(|had| picked.as_ref().is_some_and(|board| board.id == had.board.id));
            index.board = picked.map(|board| BoardPick {
                keys: same.as_ref().and_then(|had| had.keys.clone()),
                read_at: same.as_ref().and_then(|had| had.read_at),
                board,
            });
        });
        if !cx.has_global::<Bridge>() {
            return;
        }
        let answer = Bridge::ask_with_news(cx, |events, shared| {
            enkidu_core::commands::work_set_board(events.clone(), shared, board)
        });
        let store = store.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            let kept = answer.await;
            if let Ok(Err(err)) = kept {
                let _ = cx.update_window(window, |_, window, cx| {
                    toast::warning(window, "Couldn't pick that board", err.message, cx)
                });
            }
            // the index as the core has it now, whatever happened
            cx.update(|cx| Work::read(&store, cx));
        })
        .detach();
    }

    /// Hides cards from the board, or shows them again. The page shows it at once; the core keeps it (in the app's
    /// database, with a line on each card's timeline), and what goes wrong is said in a toast in `window`.
    pub fn hide(store: &Entity<Work>, cards: Vec<String>, hidden: bool, window: AnyWindowHandle, cx: &mut App) {
        if cards.is_empty() {
            return;
        }
        let ids = cards.clone();
        Work::change(store, cx, move |index| {
            index.hidden.retain(|card| !ids.contains(card));
            if hidden {
                index.hidden.extend(ids);
            }
        });
        if !cx.has_global::<Bridge>() {
            return;
        }
        let answer = Bridge::ask_with_news(cx, move |events, shared| {
            enkidu_core::commands::work_hide(events.clone(), shared, cards, hidden)
        });
        let store = store.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            let kept = answer.await;
            if let Ok(Err(err)) = kept {
                let title = if hidden { "Couldn't hide them" } else { "Couldn't show them again" };
                let _ = cx.update_window(window, |_, window, cx| toast::warning(window, title, err.message, cx));
            }
            // the index as the core has it now, whatever happened
            cx.update(|cx| Work::read(&store, cx));
        })
        .detach();
    }

    /// Marks something that waits on you done, or snoozes it for a day. The page answers at once (the item leaves
    /// the card); the core keeps it, and what goes wrong is said in a toast in `window`.
    pub fn item_set(store: &Entity<Work>, item: &str, state: ItemState, window: AnyWindowHandle, cx: &mut App) {
        let (word, until) = match state {
            ItemState::Done => ("done", None),
            ItemState::Snoozed => ("snoozed", Some(store.read(cx).now() + 86_400_000)),
        };
        let id = item.to_string();
        Work::change(store, cx, move |index| {
            for item in index.items.iter_mut().filter(|item| item.id == id) {
                item.state = word.to_string();
                item.snoozed_until = until;
            }
        });
        if !cx.has_global::<Bridge>() {
            return;
        }
        let id = item.to_string();
        // a snooze's end is the core's to say: a day from its own clock
        let answer = Bridge::ask_with_news(cx, move |events, shared| {
            enkidu_core::commands::work_item_set(events.clone(), shared, id, word.to_string(), None)
        });
        let store = store.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            let kept = answer.await;
            if let Ok(Err(err)) = kept {
                let _ = cx.update_window(window, |_, window, cx| {
                    toast::warning(window, "Couldn't change that item", err.message, cx)
                });
            }
            // the index as the core has it now, whatever happened
            cx.update(|cx| Work::read(&store, cx));
        })
        .detach();
    }

    /// A session isn't a card's Claude: its links to the card's ticket and PRs go, at once here, and for good in
    /// the core (the index won't make them again). What goes wrong is said in a toast in `window`.
    pub fn unlink(store: &Entity<Work>, subjects: Vec<String>, session: &str, window: AnyWindowHandle, cx: &mut App) {
        let (gone, of) = (format!("session:{session}"), subjects.clone());
        Work::change(store, cx, move |index| {
            for link in index.links.iter_mut() {
                let between = (of.contains(&link.a) && link.b == gone) || (of.contains(&link.b) && link.a == gone);
                if between {
                    link.removed = true;
                }
            }
        });
        if !cx.has_global::<Bridge>() {
            return;
        }
        let session = session.to_string();
        let answer = Bridge::ask_with_news(cx, move |events, shared| {
            enkidu_core::commands::work_unlink_session(events.clone(), shared, subjects, session)
        });
        let store = store.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            let kept = answer.await;
            if let Ok(Err(err)) = kept {
                let _ = cx.update_window(window, |_, window, cx| {
                    toast::warning(window, "Couldn't unlink it", err.message, cx)
                });
            }
            // the index as the core has it now, whatever happened
            cx.update(|cx| Work::read(&store, cx));
        })
        .detach();
    }

    /// Saves a workflow you made or changed (scheduled when it has a start time, kept otherwise) and hands back the
    /// workflow as it was kept, or why it wasn't. The core keeps it, and its index is read again; a window without a
    /// core (a screen test, a picture) keeps the same workflow in its own index.
    pub fn save_flow(store: &Entity<Work>, flow: Flow, cx: &mut App) -> Task<Result<Flow, String>> {
        if !cx.has_global::<Bridge>() {
            let (now, before) = {
                let work = store.read(cx);
                let before = work.snapshot.as_ref().and_then(|index| index.flows.iter().find(|f| f.id == flow.id));
                (work.now(), before.cloned())
            };
            let kept = enkidu_core::work::flows::as_saved(flow, before.as_ref(), now).map_err(|err| err.to_string());
            if let Ok(kept) = kept.clone() {
                Work::change(store, cx, move |index| {
                    // the newest first, as the core lists them
                    index.flows.retain(|flow| flow.id != kept.id);
                    index.flows.insert(0, kept);
                });
            }
            return Task::ready(kept);
        }
        let answer = Bridge::ask_with_news(cx, move |events, shared| {
            enkidu_core::commands::work_flow_save(events.clone(), shared, flow).map_err(|err| err.message)
        });
        let store = store.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            let kept = answer.await.unwrap_or_else(|_| Err("The app is closing.".into()));
            // the index as the core has it now, whatever happened
            cx.update(|cx| Work::read(&store, cx));
            kept
        })
    }

    /// "Run now", or "Run it again": the workflow starts now, its tasks from the first (the core's runner starts it
    /// at once). What goes wrong is said in a toast in `window`.
    pub fn run_flow_now(store: &Entity<Work>, id: &str, window: AnyWindowHandle, cx: &mut App) {
        if !cx.has_global::<Bridge>() {
            // a window without a core keeps it as the core would: due now, every task waiting again
            let now = store.read(cx).now();
            let id = id.to_string();
            Work::change(store, cx, move |index| {
                let Some(at) = index.flows.iter().position(|flow| flow.id == id) else { return };
                let before = index.flows[at].clone();
                let due = Flow { start_at: Some(now), ..before.clone() };
                if let Ok(kept) = enkidu_core::work::flows::as_saved(due, Some(&before), now) {
                    index.flows[at] = kept;
                }
            });
            return;
        }
        let id = id.to_string();
        let answer = Bridge::ask_with_news(cx, move |events, shared| {
            enkidu_core::commands::work_flow_run_now(events.clone(), shared, id)
        });
        Work::after(store, answer, "Couldn't start the workflow", window, cx);
    }

    /// Stop: a workflow that runs stops after the task that runs (its Claude isn't stopped); one that is due is
    /// kept, without a time.
    pub fn stop_flow(store: &Entity<Work>, id: &str, window: AnyWindowHandle, cx: &mut App) {
        if !cx.has_global::<Bridge>() {
            let id = id.to_string();
            Work::change(store, cx, move |index| {
                for flow in index.flows.iter_mut().filter(|flow| flow.id == id) {
                    match flow.state.as_str() {
                        "scheduled" => (flow.state, flow.start_at) = ("saved".into(), None),
                        "running" => flow.state = "stopped".into(),
                        _ => {}
                    }
                }
            });
            return;
        }
        let id = id.to_string();
        let answer = Bridge::ask_with_news(cx, move |events, shared| {
            enkidu_core::commands::work_flow_stop(events.clone(), shared, id)
        });
        Work::after(store, answer, "Couldn't stop the workflow", window, cx);
    }

    /// Deletes a workflow. Its runs, and Claude's reports, stay.
    pub fn delete_flow(store: &Entity<Work>, id: &str, window: AnyWindowHandle, cx: &mut App) {
        let gone = id.to_string();
        Work::change(store, cx, move |index| index.flows.retain(|flow| flow.id != gone));
        if !cx.has_global::<Bridge>() {
            return;
        }
        let id = id.to_string();
        let answer = Bridge::ask_with_news(cx, move |events, shared| {
            enkidu_core::commands::work_flow_delete(events.clone(), shared, id)
        });
        Work::after(store, answer, "Couldn't delete the workflow", window, cx);
    }

    /// After the core answered something you did to a workflow: what went wrong is said in a toast in `window`, and
    /// the index is read as the core has it now, whatever happened.
    fn after(
        store: &Entity<Work>,
        answer: futures::channel::oneshot::Receiver<enkidu_core::commands::CmdResult<()>>,
        failed: &'static str,
        window: AnyWindowHandle,
        cx: &mut App,
    ) {
        let store = store.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            let kept = answer.await;
            if let Ok(Err(err)) = kept {
                let _ = cx.update_window(window, |_, window, cx| toast::warning(window, failed, err.message, cx));
            }
            cx.update(|cx| Work::read(&store, cx));
        })
        .detach();
    }

    /// Keeps your own words for the kinds of workflow tasks, all of them at once, as their defaults: new tasks of a
    /// kind start from them. The page has them at once; the core keeps them (in the app's database).
    pub fn save_flow_kinds(
        store: &Entity<Work>,
        kinds: BTreeMap<String, String>,
        cx: &mut App,
    ) -> Task<Result<(), String>> {
        let mine = kinds.clone();
        Work::change(store, cx, move |index| index.flow_kinds = mine);
        if !cx.has_global::<Bridge>() {
            return Task::ready(Ok(()));
        }
        let answer = Bridge::ask_with_news(cx, move |events, shared| {
            enkidu_core::commands::work_flow_kinds_save(events.clone(), shared, kinds).map_err(|err| err.message)
        });
        let store = store.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            let kept = answer.await.unwrap_or_else(|_| Err("The app is closing.".into()));
            // the index as the core has it now, whatever happened
            cx.update(|cx| Work::read(&store, cx));
            kept
        })
    }

    /// Keeps what a new workflow starts with (Settings, Workflows; item 8.6): the minutes a task gets and the time it
    /// starts at. The window has it at once; the core keeps it (in the app's database).
    pub fn save_flow_defaults(
        store: &Entity<Work>,
        defaults: enkidu_core::work::flows::FlowDefaults,
        cx: &mut App,
    ) -> Task<Result<(), String>> {
        let mine = defaults.clone();
        Work::change(store, cx, move |index| index.flow_defaults = mine);
        if !cx.has_global::<Bridge>() {
            return Task::ready(Ok(()));
        }
        let answer = Bridge::ask_with_news(cx, move |events, shared| {
            enkidu_core::commands::work_flow_defaults_save(events.clone(), shared, defaults).map_err(|err| err.message)
        });
        let store = store.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            let kept = answer.await.unwrap_or_else(|_| Err("The app is closing.".into()));
            cx.update(|cx| Work::read(&store, cx));
            kept
        })
    }

    /// Saves a task of your own (item 8.5): a new one, or the words and the state of one that is there. Hands back
    /// the task as it was kept, or why it wasn't. The core keeps it, and its index is read again; a window without a
    /// core (a screen test, a picture) keeps it in its own index, under `project`, its project's name.
    pub fn save_own_task(
        store: &Entity<Work>,
        draft: OwnTaskDraft,
        project: String,
        cx: &mut App,
    ) -> Task<Result<OwnTask, String>> {
        if !cx.has_global::<Bridge>() {
            let kept = store.read(cx).own_task_as_kept(&draft, project);
            if let Ok(kept) = kept.clone() {
                Work::change(store, cx, move |index| {
                    match index.own_tasks.iter_mut().find(|task| task.id == kept.id) {
                        Some(task) => *task = kept,
                        None => index.own_tasks.push(kept),
                    }
                });
            }
            return Task::ready(kept);
        }
        let answer = Bridge::ask_with_news(cx, move |events, shared| {
            enkidu_core::commands::work_own_task_save(events.clone(), shared, draft).map_err(|err| err.message)
        });
        let store = store.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            let kept: Result<OwnTask, String> = answer.await.unwrap_or_else(|_| Err("The app is closing.".into()));
            cx.update(|cx| {
                // the window has it at once (who asked may put its card somewhere right away: a workflow's tasks)
                if let Ok(task) = kept.clone() {
                    Work::change(&store, cx, move |index| {
                        match index.own_tasks.iter_mut().find(|own| own.id == task.id) {
                            Some(own) => *own = task,
                            None => index.own_tasks.push(task),
                        }
                    });
                }
                // then the index as the core has it now, whatever happened
                Work::read(&store, cx)
            });
            kept
        })
    }

    /// What a window without a core keeps for a draft: what the core would, numbered after the last one.
    fn own_task_as_kept(&self, draft: &OwnTaskDraft, project: String) -> Result<OwnTask, String> {
        let index = self.snapshot.as_ref().ok_or("The index isn't read yet.")?;
        let (title, notes) = (draft.title.trim(), draft.notes.trim());
        if title.is_empty() {
            return Err("Give the task a title".into());
        }
        let now = self.now();
        let known = draft.id.and_then(|id| index.own_tasks.iter().find(|task| task.id == id));
        Ok(match known {
            Some(known) => OwnTask {
                title: title.into(),
                notes: notes.into(),
                state: draft.state.clone(),
                updated_at: now,
                ..known.clone()
            },
            None => OwnTask {
                id: index.own_tasks.iter().map(|task| task.id).max().unwrap_or(0) + 1,
                project_id: draft.project_id.clone(),
                title: title.into(),
                notes: notes.into(),
                state: draft.state.clone(),
                created_at: now,
                updated_at: now,
                project,
            },
        })
    }

    /// Deletes a task of your own. The board has it gone at once; the core forgets it, and its index is read again.
    pub fn delete_own_task(store: &Entity<Work>, id: i64, cx: &mut App) -> Task<Result<(), String>> {
        Work::change(store, cx, move |index| index.own_tasks.retain(|task| task.id != id));
        if !cx.has_global::<Bridge>() {
            return Task::ready(Ok(()));
        }
        let answer = Bridge::ask_with_news(cx, move |events, shared| {
            enkidu_core::commands::work_own_task_delete(events.clone(), shared, id).map_err(|err| err.message)
        });
        let store = store.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            let gone = answer.await.unwrap_or_else(|_| Err("The app is closing.".into()));
            cx.update(|cx| Work::read(&store, cx));
            gone
        })
    }

    /// Refresh: the core reads every project's GitHub and Jira again now. What it finds comes as its news.
    pub fn refresh(cx: &mut App) {
        if !cx.has_global::<Bridge>() {
            return;
        }
        Bridge::call(cx, enkidu_core::commands::work_refresh, |_, _| {});
    }
}

/// Keeps the store's cards on the frame's workspaces and lamps. Most of the frame's news is about something else (a
/// tab picked, a file changed): then nothing is built and no view hears of it.
pub fn follow_frame(store: &Entity<Work>, frame: &Entity<Frame>, cx: &mut App) {
    let store = store.clone();
    cx.observe(frame, move |frame, cx| {
        store.update(cx, |work, cx| {
            if work.follow(frame.read(cx)) {
                cx.notify();
            }
        });
    })
    .detach();
}
