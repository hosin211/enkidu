//! Item 5.1b on the whole wiring: the window's Work store on a real core whose watcher reads GitHub and Jira through
//! the fake `gh` and the fake `twg` (never the real ones, never the network). The index is read at launch and again
//! when the core says it changed, one question at a time; Refresh makes the core read both sources now; a source
//! that can't be read keeps its data and says what to do.

mod common;

use std::collections::HashMap;

use enkidu_app::stores::Stores;
use enkidu_app::work::store::Work;
use enkidu_core::events::Event;
use enkidu_core::work::model::Snapshot;
use enkidu_model::work::{self, Lamp, Source};
use gpui_kit::TestAppContext;

/// The app on the sample project, whose origin says it is the repository the recorded answers are about, with its Jira
/// projects set.
fn start(cx: &mut TestAppContext) -> common::App {
    common::start_on(cx, enkidu_testkit::shop_on_github, |shared, project, _| common::reads_jira(shared, project))
}

/// How many times the store asked the core for the index, and how many answers differed from what it showed.
fn counts(cx: &mut TestAppContext) -> (u64, u64) {
    cx.run_until_parked();
    cx.update(|cx| {
        let store = Stores::work(cx);
        let store = store.read(cx);
        (store.asked, store.changed)
    })
}

fn state(work: &Snapshot, source: &str) -> String {
    work.health.iter().find(|h| h.source == source).map(|h| h.state.clone()).unwrap_or_default()
}

/// Both sources were read (or tried) at `since` or later, and no read is running. (A read can start in the very
/// millisecond the test noted, so the same time counts: `refresh` waits a few milliseconds before it notes one.)
fn read_since(work: &Snapshot, since: i64) -> bool {
    !work.checking && work.health.len() == 2 && work.health.iter().all(|h| h.last_try.is_some_and(|at| at >= since))
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

/// Refresh, then the index once both sources were tried again.
fn refresh(cx: &mut TestAppContext, app: &common::App) -> std::sync::Arc<Snapshot> {
    // a read is told apart from the one before by its time, in milliseconds
    std::thread::sleep(std::time::Duration::from_millis(5));
    let since = now();
    cx.update(Work::refresh);
    app.until_work(cx, |work| read_since(work, since))
}

#[gpui_kit::test]
fn the_index_is_read_at_launch_and_again_when_the_core_says_it_changed(cx: &mut TestAppContext) {
    let app = start(cx);
    // at launch: what the core's database has, which is nothing yet
    let first = app.until_work(cx, |_| true);
    assert!(first.prs.is_empty() && first.tickets.is_empty() && first.health.is_empty(), "{:?}", first.health);
    assert_eq!(counts(cx), (1, 1));
    // the core's watcher reads GitHub and Jira (through the fakes) and says so: the store has what it read
    let fakes = app.start_work();
    let work = app.until_work(cx, |work| read_since(work, 0) && !work.tickets.is_empty());
    assert_eq!(
        (state(&work, "github"), state(&work, "jira")),
        ("ok".to_string(), "ok".to_string()),
        "{:?}",
        work.health
    );
    assert_eq!(work.prs.iter().filter(|pr| pr.state == "OPEN").count(), 6);
    assert_eq!(work.prs.iter().filter(|pr| pr.state == "MERGED").count(), 8);
    // every open ticket of yours (45, from all three pages of the search), and the two done ones your PRs name
    assert_eq!(work.tickets.len(), 47);
    let mut done: Vec<&str> = work.tickets.iter().filter(|t| t.category == "done").map(|t| t.key.as_str()).collect();
    done.sort();
    assert_eq!(done, ["SH-20698", "SH-21254"]);
    assert!(work.items.iter().any(|item| item.kind == "pipeline_failed" && item.state == "open"));
    // each source's lamp, and what the status line says of it
    let github = work::source_health(&work.health, Source::Github).expect("GitHub is read");
    let jira = work::source_health(&work.health, Source::Jira).expect("Jira is read");
    assert_eq!(
        (work::lamp(Some(&github), work.checking), work::lamp(Some(&jira), work.checking)),
        (Lamp::Ok, Lamp::Ok)
    );
    assert_eq!(
        work::status_words(&github, Source::Github, now()),
        ("GitHub read just now as me-dev".to_string(), false)
    );
    assert_eq!(work::status_words(&jira, Source::Jira, now()), ("Jira read just now as Me Dev".to_string(), false));

    // news that changes nothing: one question at a time (the first, and one more for all that came meanwhile), and
    // the views hear nothing, because the index that came back is the one they show
    let (asked, changed) = counts(cx);
    for _ in 0..10 {
        app.sink.send(Event::WorkChanged);
    }
    for _ in 0..100 {
        if counts(cx).0 >= asked + 2 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    std::thread::sleep(std::time::Duration::from_millis(150));
    let (asked_after, changed_after) = counts(cx);
    assert!((asked + 2..=asked + 3).contains(&asked_after), "ten pieces of news asked {} times", asked_after - asked);
    assert_eq!(changed_after, changed, "nothing changed, so nothing was drawn again");
    // nothing was ever written to GitHub or Jira, and no token went on a command line
    let log = fakes.gh_log();
    assert!(!log.is_empty() && !log.contains("fake-token"), "the token travels in the environment only");
    for write in
        [" pr comment", " pr review", " pr merge", " pr edit", " pr close", "-X POST", "--method POST", "mutation"]
    {
        assert!(!log.contains(write), "nothing is ever written to GitHub ({write})");
    }
    for line in fakes.twg_log().lines() {
        let call = line.split(" | ").nth(1).unwrap_or("");
        let reads = ["whoami", "jira workitem query", "jira workitem get"];
        assert!(reads.iter().any(|read| call.starts_with(read)), "only reads: {call}");
    }
}

#[gpui_kit::test]
fn refresh_reads_both_sources_now_and_a_source_that_cant_be_read_keeps_its_data(cx: &mut TestAppContext) {
    let app = start(cx);
    let fakes = app.start_work();
    let good = app.until_work(cx, |work| read_since(work, 0) && !work.tickets.is_empty());
    let projects: HashMap<String, String> = [(app.project_id.clone(), "shop".to_string())].into_iter().collect();
    assert!(work::broken(&good.health, &projects, now()).is_empty());

    // GitHub signed out: its lamp is red, the line says what is wrong and what to do, and what was read stays
    fakes.gh("logged-out");
    let work = refresh(cx, &app);
    assert_eq!(
        (state(&work, "github"), state(&work, "jira")),
        ("error".to_string(), "ok".to_string()),
        "{:?}",
        work.health
    );
    let github = work::source_health(&work.health, Source::Github).unwrap();
    assert_eq!(work::lamp(Some(&github), work.checking), Lamp::Error);
    assert_eq!(work::status_words(&github, Source::Github, now()), ("GitHub can't be read".to_string(), true));
    let lines = work::broken(&work.health, &projects, now());
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!((lines[0].source, lines[0].title.as_str()), (Source::Github, "GitHub can't be read:"));
    assert!(lines[0].error.contains("gh isn't signed in"), "{lines:?}");
    assert!(lines[0].fix.as_deref().unwrap_or("").contains("gh auth login"), "{lines:?}");
    assert!(lines[0].under.starts_with("What you see from GitHub is from its last good read, just now."), "{lines:?}");
    assert_eq!((work.prs.len(), work.tickets.len()), (good.prs.len(), good.tickets.len()), "its data is kept");
    // signed in again
    fakes.gh("ok");
    let work = refresh(cx, &app);
    assert_eq!(state(&work, "github"), "ok");
    assert!(work::broken(&work.health, &projects, now()).is_empty());

    // Jira signed out
    fakes.twg("logged-out");
    let work = refresh(cx, &app);
    assert_eq!(
        (state(&work, "github"), state(&work, "jira")),
        ("ok".to_string(), "error".to_string()),
        "{:?}",
        work.health
    );
    let lines = work::broken(&work.health, &projects, now());
    assert_eq!((lines.len(), lines[0].source, lines[0].title.as_str()), (1, Source::Jira, "Jira can't be read:"));
    assert!(lines[0].error.contains("twg isn't signed in"), "{lines:?}");
    assert!(lines[0].fix.as_deref().unwrap_or("").contains("twg login --force"), "{lines:?}");
    assert_eq!(work.tickets.len(), good.tickets.len(), "its tickets are kept");
    fakes.twg("ok");
    let work = refresh(cx, &app);
    assert_eq!(state(&work, "jira"), "ok");

    // both answer garbage at once, then both come back
    fakes.gh("garbage");
    fakes.twg("garbage");
    let work = refresh(cx, &app);
    assert_eq!(
        (state(&work, "github"), state(&work, "jira")),
        ("error".to_string(), "error".to_string()),
        "{:?}",
        work.health
    );
    let lines = work::broken(&work.health, &projects, now());
    assert_eq!(lines.len(), 2);
    assert!(lines.iter().all(|line| line.error.contains("isn't valid JSON")), "{lines:?}");
    assert_eq!((work.prs.len(), work.tickets.len()), (good.prs.len(), good.tickets.len()), "everything is kept");
    fakes.gh("ok");
    fakes.twg("ok");
    let work = refresh(cx, &app);
    assert_eq!((state(&work, "github"), state(&work, "jira")), ("ok".to_string(), "ok".to_string()));
    assert!(work::broken(&work.health, &projects, now()).is_empty());
    common::never_wrote(&fakes);
}

/// Item 8.5 on the whole wiring: a task of your own made in the window is in the core's database with its timeline;
/// given another state it is the same task; a new core on the same data (a restart) reads it; deleted, it is gone
/// with its timeline.
#[gpui_kit::test]
fn a_task_of_your_own_is_kept_by_the_core_with_its_timeline_and_is_there_after_a_restart(cx: &mut TestAppContext) {
    use enkidu_core::commands::OwnTaskDraft;
    let app = start(cx);
    let _ = app.until_work(cx, |_| true);
    let store = cx.update(|cx| Stores::work(cx));
    let project = app.shared.db.project(&app.project_id).unwrap().expect("the project").name;
    let save = |cx: &mut TestAppContext, id: Option<i64>, title: &str, state: &str| {
        let draft = OwnTaskDraft {
            id,
            project_id: app.project_id.clone(),
            title: title.into(),
            notes: "On the CI runner.".into(),
            state: state.into(),
        };
        cx.update(|cx| Work::save_own_task(&store, draft, project.clone(), cx)).detach();
    };
    save(cx, None, "Try the small stack", "todo");
    let work = app.until_work(cx, |work| work.own_tasks.len() == 1);
    let task = work.own_tasks[0].clone();
    assert_eq!((task.id, task.title.as_str(), task.notes.as_str()), (1, "Try the small stack", "On the CI runner."));
    assert_eq!(
        (task.state.as_str(), task.project_id.as_str(), task.project.as_str()),
        ("todo", app.project_id.as_str(), project.as_str())
    );
    assert_eq!(app.shared.db.work_own_tasks().unwrap(), std::slice::from_ref(&task), "the core's database has it");

    // another state: the same task, and its timeline says both, the newest first
    save(cx, Some(task.id), "Try the small stack", "doing");
    let work = app.until_work(cx, |work| work.own_tasks.first().is_some_and(|task| task.state == "doing"));
    assert_eq!(work.own_tasks.len(), 1);
    let said: Vec<&str> = work.events.iter().filter(|e| e.subject == "own:1").map(|e| e.text.as_str()).collect();
    assert_eq!(said, ["You set it to In progress", "You made this task"]);
    // other words alone say nothing new on the timeline
    save(cx, Some(task.id), "Try the small Docker setup", "doing");
    let work =
        app.until_work(cx, |work| work.own_tasks.first().is_some_and(|task| task.title.ends_with("Docker setup")));
    assert_eq!(work.events.iter().filter(|e| e.subject == "own:1").count(), 2);

    // a restart: a new core on the same data folder reads the same
    let again = enkidu_core::state::Shared::new(app.home.path().join("data"), std::time::Instant::now()).unwrap();
    let kept = again.db.work_own_tasks().unwrap();
    assert_eq!(
        (kept.len(), kept[0].title.as_str(), kept[0].state.as_str()),
        (1, "Try the small Docker setup", "doing")
    );
    drop(again);

    // deleted: gone from the window, from the database, and so is its timeline
    cx.update(|cx| Work::delete_own_task(&store, task.id, cx)).detach();
    // (the window has it gone at once; its timeline goes when the core has answered)
    let gone = |work: &Snapshot| work.own_tasks.is_empty() && work.events.iter().all(|e| e.subject != "own:1");
    let _ = app.until_work(cx, gone);
    assert!(app.shared.db.work_own_tasks().unwrap().is_empty());
    // what can't be kept is said, and nothing is kept: no title
    let answer = std::rc::Rc::new(std::cell::RefCell::new(None));
    let got = answer.clone();
    cx.update(|cx| {
        let draft = OwnTaskDraft {
            id: None,
            project_id: app.project_id.clone(),
            title: "   ".into(),
            notes: String::new(),
            state: "todo".into(),
        };
        let refused = Work::save_own_task(&store, draft, project.clone(), cx);
        cx.spawn(async move |_| *got.borrow_mut() = Some(refused.await)).detach();
    });
    for _ in 0..2000 {
        cx.run_until_parked();
        if answer.borrow().is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(answer.borrow().clone(), Some(Err("Give the task a title".to_string())));
    assert!(app.shared.db.work_own_tasks().unwrap().is_empty());
}
