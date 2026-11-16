//! Item 1.15: the frame's store shows what the database remembers, with where you were last time, then follows the
//! core's events in order; where you are is kept for next time.

use std::sync::Arc;
use std::time::{Duration, Instant};

use enkidu_app::bridge::Bridge;
use enkidu_core::db::{Project, Workspace};
use enkidu_core::events::{Event, Events};
use enkidu_core::state::Shared;
use futures::channel::mpsc::unbounded;
use gpui_kit::TestAppContext;

fn ws(id: &str, name: &str) -> Workspace {
    Workspace {
        id: id.into(),
        project_id: "p1".into(),
        name: name.into(),
        path: format!("/ws/{name}"),
        branch: format!("feature/{name}"),
        base: Some("development".into()),
        session_id: format!("s{id}"),
        transcript_path: None,
        claude_config_dir: None,
        created_at: 1,
        adopted: false,
    }
}

#[gpui_kit::test]
fn the_frame_shows_what_is_remembered_then_follows_the_core(cx: &mut TestAppContext) {
    let data = tempfile::tempdir().unwrap();
    let shared = Arc::new(Shared::new(data.path().to_path_buf(), Instant::now()).unwrap());
    // the database remembers a project with two workspaces; the core has the first one's Claude working
    let project = Project {
        id: "p1".into(),
        name: "shop".into(),
        path: "/code/shop".into(),
        live_workspace_id: None,
        created_at: 1,
    };
    shared.db.insert_project(&project).unwrap();
    shared.db.insert_workspace(&ws("w1", "one")).unwrap();
    shared.db.insert_workspace(&ws("w2", "two")).unwrap();
    shared.set_status(&Events::none(), "w1", "working");
    // last time, the second workspace was on screen
    let view = data.path().join("view.json");
    std::fs::write(&view, r#"{"project_id":"p1","selected":{"p1":"w2"}}"#).unwrap();
    // news that comes while the first read runs waits its turn
    let (news, events) = unbounded();
    news.unbounded_send(Event::StatusChanged { workspace_id: "w2".into(), status: "needs_you".into() }).unwrap();

    let frame = cx.update(|cx| {
        Bridge::install(cx, shared.clone(), enkidu_core::events::Events::none());
        enkidu_app::stores::install_for_tests(
            events,
            Default::default(),
            enkidu_app::stores::Places { view: Some(view.clone()), ..Default::default() },
            cx,
        )
    });
    cx.executor().allow_parking();
    let settle = |cx: &mut TestAppContext| {
        for _ in 0..200 {
            cx.run_until_parked();
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    for _ in 0..300 {
        cx.run_until_parked();
        if frame.read_with(cx, |f, _| !f.projects.is_empty()) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    cx.run_until_parked();
    frame.read_with(cx, |f, _| {
        assert_eq!(f.projects.len(), 1);
        assert_eq!(f.tabs().iter().map(|w| w.name.as_str()).collect::<Vec<_>>(), ["one", "two"]);
        assert_eq!(f.current_workspace().map(|w| w.name.as_str()), Some("two"), "where you were");
        assert_eq!(f.status("w1"), "working", "the core's own state");
        assert_eq!(f.status("w2"), "needs_you", "the news that came meanwhile, applied after");
    });

    // then the core's news keeps it fresh
    news.unbounded_send(Event::StatusChanged { workspace_id: "w1".into(), status: "done".into() }).unwrap();
    news.unbounded_send(Event::WorkspaceAdded(ws("w3", "task"))).unwrap();
    cx.run_until_parked();
    frame.read_with(cx, |f, _| {
        assert_eq!(f.status("w1"), "done");
        assert_eq!(f.tabs().len(), 3);
    });

    // where you are is kept a moment after it changes
    frame.update(cx, |f, cx| {
        f.select_workspace("w1");
        cx.notify();
    });
    cx.executor().advance_clock(Duration::from_millis(400));
    settle(cx);
    let kept = enkidu_app::stores::load_view(&view).expect("kept");
    assert_eq!(kept.selected.get("p1").map(String::as_str), Some("w1"));
}

/// Item 18.6: your order of the tabs and the workspaces you hid are kept with where you were, a moment after they
/// change, and a `view.json` from before (it has neither) is read as it always was.
#[gpui_kit::test]
fn your_order_and_what_you_hid_are_kept_with_where_you_were(cx: &mut TestAppContext) {
    let data = tempfile::tempdir().unwrap();
    let shared = Arc::new(Shared::new(data.path().to_path_buf(), Instant::now()).unwrap());
    let project = Project {
        id: "p1".into(),
        name: "shop".into(),
        path: "/code/shop".into(),
        live_workspace_id: None,
        created_at: 1,
    };
    shared.db.insert_project(&project).unwrap();
    for (id, name, made) in [("w1", "one", 1), ("w2", "two", 2), ("w3", "three", 3)] {
        shared.db.insert_workspace(&Workspace { created_at: made, ..ws(id, name) }).unwrap();
    }
    // what an earlier build left: where you were, and nothing of an order or of hidden workspaces
    let view = data.path().join("view.json");
    std::fs::write(&view, r#"{"project_id":"p1","selected":{"p1":"w2"},"work":false}"#).unwrap();
    let (_news, events) = unbounded();
    let frame = cx.update(|cx| {
        Bridge::install(cx, shared.clone(), enkidu_core::events::Events::none());
        enkidu_app::stores::install_for_tests(
            events,
            Default::default(),
            enkidu_app::stores::Places { view: Some(view.clone()), ..Default::default() },
            cx,
        )
    });
    cx.executor().allow_parking();
    for _ in 0..300 {
        cx.run_until_parked();
        if frame.read_with(cx, |f, _| !f.projects.is_empty()) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let tabs = |cx: &mut TestAppContext| -> Vec<String> {
        frame.read_with(cx, |f, _| f.tabs().iter().map(|w| w.name.clone()).collect())
    };
    assert_eq!(tabs(cx), ["one", "two", "three"], "as they were made, with nothing hidden");
    // the last tab goes first, and the second workspace is hidden
    frame.update(cx, |f, cx| {
        assert!(f.move_tab("w3", 0) && f.hide_workspace("w2", 5));
        cx.notify();
    });
    assert_eq!(tabs(cx), ["three", "one"]);
    cx.executor().advance_clock(Duration::from_millis(400));
    for _ in 0..200 {
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(5));
    }
    let kept = enkidu_app::stores::load_view(&view).expect("kept");
    assert_eq!(kept.order.get("p1").map(Vec::as_slice), Some(&["w3".to_string(), "w1".into(), "w2".into()][..]));
    assert_eq!(kept.hidden.iter().collect::<Vec<_>>(), [(&"w2".to_string(), &5)]);
    assert_eq!(kept.selected.get("p1").map(String::as_str), Some("w1"), "it was the last tab: the one before it shows");
    // the next time the app starts, it is as you left it
    let again =
        enkidu_model::frame::Frame::load(vec![project], shared.db.workspaces().unwrap(), Default::default(), &kept);
    assert_eq!(again.tabs().iter().map(|w| w.name.as_str()).collect::<Vec<_>>(), ["three", "one"]);
    assert_eq!(again.hidden_tabs().iter().map(|w| w.name.as_str()).collect::<Vec<_>>(), ["two"]);
    assert_eq!(again.current_workspace().map(|w| w.name.as_str()), Some("one"));
}

fn change(path: &str, sig: &str) -> enkidu_core::git::Change {
    enkidu_core::git::Change {
        path: path.into(),
        old_path: None,
        kind: "modified".into(),
        staged: false,
        additions: Some(3),
        deletions: Some(1),
        sig: sig.into(),
    }
}

/// Item 3.1: each workspace's changes are there from the first frame as last seen, then the core's news replaces
/// them, and they are kept for the next launch at most twice a second.
#[gpui_kit::test]
fn the_changes_show_as_last_seen_then_follow_the_core_and_are_kept(cx: &mut TestAppContext) {
    let data = tempfile::tempdir().unwrap();
    let shared = Arc::new(Shared::new(data.path().to_path_buf(), Instant::now()).unwrap());
    let project = Project {
        id: "p1".into(),
        name: "shop".into(),
        path: "/code/shop".into(),
        live_workspace_id: None,
        created_at: 1,
    };
    shared.db.insert_project(&project).unwrap();
    shared.db.insert_workspace(&ws("w1", "one")).unwrap();
    shared.db.insert_workspace(&ws("w2", "two")).unwrap();
    // as the app last left it: one changed file in "one", and a list for a workspace that is gone since
    let kept = data.path().join("changes.json");
    let last_seen = serde_json::json!({ "w1": [change("src/a.py", "1")], "w9-removed": [change("x.py", "1")] });
    std::fs::write(&kept, last_seen.to_string()).unwrap();
    let (news, events) = unbounded();
    let frame = cx.update(|cx| {
        Bridge::install(cx, shared.clone(), Events::none());
        let places = enkidu_app::stores::Places { changes: Some(kept.clone()), ..Default::default() };
        enkidu_app::stores::install_for_tests(events, Default::default(), places, cx)
    });
    cx.executor().allow_parking();
    for _ in 0..300 {
        cx.run_until_parked();
        if frame.read_with(cx, |f, _| f.loaded) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let lists = cx.update(|cx| enkidu_app::stores::Stores::changes(cx));
    // from the first frame, before git said anything
    lists.read_with(cx, |lists, _| {
        assert_eq!(lists.of("w1").map(|l| l.as_slice()), Some([change("src/a.py", "1")].as_slice()));
        assert!(lists.of("w2").is_none() && lists.of("w9-removed").is_none());
    });
    frame.read_with(cx, |f, _| assert_eq!((f.changed_files("w1"), f.changed_files("w2")), (1, 0)));
    let shown = lists.read_with(cx, |lists, _| lists.of("w1").unwrap().clone());

    // git says the same: the list shown is kept; then another file changes
    news.unbounded_send(Event::ChangesUpdated { workspace_id: "w1".into(), files: vec![change("src/a.py", "1")] })
        .unwrap();
    cx.run_until_parked();
    assert!(lists.read_with(cx, |lists, _| Arc::ptr_eq(&shown, lists.of("w1").unwrap())));
    let two = vec![change("src/a.py", "2"), change("src/b.py", "1")];
    news.unbounded_send(Event::ChangesUpdated { workspace_id: "w1".into(), files: two.clone() }).unwrap();
    cx.run_until_parked();
    lists.read_with(cx, |lists, _| assert_eq!(lists.of("w1").unwrap().as_slice(), two.as_slice()));
    frame.read_with(cx, |f, _| assert_eq!(f.changed_files("w1"), 2));

    // kept for the next launch, half a second after the last change
    cx.executor().advance_clock(Duration::from_millis(500));
    for _ in 0..200 {
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(5));
    }
    let written: std::collections::HashMap<String, Vec<enkidu_core::git::Change>> =
        serde_json::from_str(&std::fs::read_to_string(&kept).unwrap()).unwrap();
    assert_eq!(written.get("w1"), Some(&two));
    assert!(!written.contains_key("w9-removed"), "a workspace that is gone is not kept");
}

/// Keeps each workspace's changes from the core's news. Also for a workspace that isn't on screen.
#[gpui_kit::test]
fn keeps_each_workspaces_changes_from_the_cores_news(cx: &mut TestAppContext) {
    let data = tempfile::tempdir().unwrap();
    let shared = Arc::new(Shared::new(data.path().to_path_buf(), Instant::now()).unwrap());
    let project = Project {
        id: "p1".into(),
        name: "shop".into(),
        path: "/code/shop".into(),
        live_workspace_id: None,
        created_at: 1,
    };
    shared.db.insert_project(&project).unwrap();
    shared.db.insert_workspace(&ws("w1", "one")).unwrap();
    shared.db.insert_workspace(&ws("w2", "two")).unwrap();
    let (news, events) = unbounded();
    let frame = cx.update(|cx| {
        Bridge::install(cx, shared.clone(), Events::none());
        enkidu_app::stores::install_for_tests(events, Default::default(), Default::default(), cx)
    });
    cx.executor().allow_parking();
    for _ in 0..300 {
        cx.run_until_parked();
        if frame.read_with(cx, |f, _| f.loaded) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    // "one" is on screen; the push is for "two"
    assert_eq!(frame.read_with(cx, |f, _| f.current_workspace().map(|w| w.id.clone())).as_deref(), Some("w1"));
    let files = vec![change("a.py", "s1")];
    news.unbounded_send(Event::ChangesUpdated { workspace_id: "w2".into(), files: files.clone() }).unwrap();
    cx.run_until_parked();
    let lists = cx.update(|cx| enkidu_app::stores::Stores::changes(cx));
    lists.read_with(cx, |lists, _| {
        assert_eq!(lists.of("w2").map(|list| list.as_slice()), Some(files.as_slice()));
        assert!(lists.of("w1").is_none(), "the other workspace's list is its own");
    });
    // and its tab counts it
    frame.read_with(cx, |f, _| assert_eq!((f.changed_files("w1"), f.changed_files("w2")), (0, 1)));
}
