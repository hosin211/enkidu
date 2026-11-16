//! Item 2.14: at launch every workspace's Claude comes back, not only the one on screen, and after a restart each one
//! resumes its conversation. All of it on the whole wiring with the fake Claude; the last test restarts the app on the
//! same folders.

mod common;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use common::{App, said};
use enkidu_core::state::Shared;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};

/// The ids of the workspaces named, as the database has them.
fn ids(shared: &Shared, names: &[&str]) -> Vec<String> {
    let all = shared.db.workspaces().unwrap();
    names.iter().map(|name| all.iter().find(|w| &w.name == name).expect(name).id.clone()).collect()
}

/// Makes workspaces and, as the app last left them: which one was on screen, and which had a Claude.
fn launch(
    cx: &mut TestAppContext,
    names: &'static [&'static str],
    on_screen: Option<&'static str>,
    kept: Option<&'static [&'static str]>,
    more: impl FnOnce(&Arc<Shared>, &Path),
) -> App {
    common::start_live(
        cx,
        |home| enkidu_testkit::repo_with_origin(home, "shop"),
        |shared, project, home| {
            for name in names {
                App::make(shared, project, name, "new", name, Some("main"));
            }
            let data = home.join("data");
            if let Some(name) = on_screen {
                let id = ids(shared, &[name]).remove(0);
                let view = serde_json::json!({ "project_id": project, "selected": { project: id } });
                std::fs::write(data.join("view.json"), view.to_string()).unwrap();
            }
            if let Some(kept) = kept {
                // a workspace that is gone since is still in the list
                let mut list: Vec<String> =
                    kept.iter().filter(|n| names.contains(n)).map(|n| ids(shared, &[n]).remove(0)).collect();
                list.extend(kept.iter().filter(|n| !names.contains(n)).map(|n| n.to_string()));
                std::fs::write(data.join("sessions.json"), serde_json::to_string(&list).unwrap()).unwrap();
            }
            more(shared, home);
        },
    )
}

/// The workspaces whose fake Claude started, in the order they did.
fn started(app: &App) -> Vec<String> {
    app.claude_starts().iter().map(|s| s["env"]["ENKIDU_WORKSPACE_NAME"].as_str().unwrap_or("?").to_string()).collect()
}

/// The list the app keeps of the workspaces that have a Claude, as names, sorted.
fn kept(app: &App) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(app.home.path().join("data").join("sessions.json")).ok()?;
    let list: Vec<String> = serde_json::from_str(&text).ok()?;
    let all = app.shared.db.workspaces().unwrap();
    let mut names: Vec<String> =
        list.iter().map(|id| all.iter().find(|w| &w.id == id).map_or_else(|| id.clone(), |w| w.name.clone())).collect();
    names.sort();
    Some(names)
}

fn until_kept(cx: &mut TestAppContext, app: &App, want: &[&str]) -> bool {
    for _ in 0..800 {
        cx.run_until_parked();
        if kept(app).is_some_and(|names| names == want) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

/// Lets a moment pass in which nothing more may start.
fn settle(cx: &mut TestAppContext) {
    for _ in 0..60 {
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[gpui_kit::test]
fn the_first_time_starts_every_workspaces_claude_the_one_on_screen_first_and_remembers_them(cx: &mut TestAppContext) {
    // the core is held at the first step of starting a Claude, so the order they are asked for in can be seen
    let (release, held) = std::sync::mpsc::channel::<()>();
    let app = launch(cx, &["one", "two", "three"], Some("two"), None, move |shared, _| {
        let (holding, ready) = std::sync::mpsc::channel::<()>();
        let shared = shared.clone();
        std::thread::spawn(move || {
            let _hooks = shared.hooks.lock().unwrap();
            holding.send(()).unwrap();
            let _ = held.recv();
        });
        ready.recv().unwrap();
    });
    assert_eq!(app.on_screen(cx).map(|w| w.0).as_deref(), Some("two"));
    settle(cx);
    let all = ids(&app.shared, &["one", "two", "three"]);
    let asked: Vec<bool> = cx.update(|cx| {
        let terminals = enkidu_app::stores::Stores::terminals(cx);
        let terminals = terminals.read(cx);
        all.iter().map(|id| terminals.of(id).is_some()).collect()
    });
    // the one on screen and, after it, the first of the others; the third waits for its turn
    assert_eq!(asked, [true, true, false], "one after another, the one on screen first");
    assert!(started(&app).is_empty());
    release.send(()).unwrap();
    app.until_claude_started(cx, 3);
    let mut names = started(&app);
    names.sort();
    assert_eq!(names, ["one", "three", "two"]);
    assert!(until_kept(cx, &app, &["one", "three", "two"]), "{:?}", kept(&app));
    // once per launch
    let all = ids(&app.shared, &["one", "two", "three"]);
    cx.update(|cx| {
        enkidu_app::stores::Stores::terminals(cx).update(cx, |terminals, cx| terminals.bring_back(all, None, cx))
    });
    settle(cx);
    assert_eq!(started(&app).len(), 3);
}

#[gpui_kit::test]
fn afterwards_brings_back_the_ones_that_had_a_claude_when_the_window_closed_and_no_workspace_that_is_gone(
    cx: &mut TestAppContext,
) {
    let app = launch(cx, &["one", "two", "three"], Some("one"), Some(&["three", "w9-removed", "one"]), |_, _| {});
    app.until_claude_started(cx, 2);
    settle(cx);
    // "two" had no Claude when the app last ran: it gets one when you open it, as before
    let mut names = started(&app);
    names.sort();
    assert_eq!(names, ["one", "three"]);
    assert!(until_kept(cx, &app, &["one", "three"]), "{:?}", kept(&app));
    app.press(cx, "cmd-2");
    app.until_claude_started(cx, 3);
    assert_eq!(started(&app)[2], "two");
}

#[gpui_kit::test]
fn picks_up_a_claude_that_already_runs_instead_of_starting_a_second_one(cx: &mut TestAppContext) {
    let app = launch(cx, &["one", "two"], Some("one"), None, |shared, _| {
        // "one" already has its Claude before the window looks (the core ran it: a workflow, say)
        let one = ids(shared, &["one"]).remove(0);
        let claude = enkidu_core::commands::spawn_terminal(
            &enkidu_core::events::Events::none(),
            shared,
            &one,
            "claude",
            120,
            32,
            None,
            false,
        )
        .expect("Claude started in the core");
        // it has printed before any window looks: that is what the window must show
        for _ in 0..800 {
            if shared.pty.buffer(&claude.id).is_some_and(|kept| kept.data.contains("\n> ")) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    });
    app.until_claude_started(cx, 2);
    settle(cx);
    let mut names = started(&app);
    names.sort();
    assert_eq!(names, ["one", "two"], "one Claude each");
    // the one that ran already is on screen with what it had printed before the window was there
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        assert_eq!(said(w, ("term-tab", 0usize)).as_deref(), Some("Claude"));
        assert!(w.try_find(("term-tab", 1usize)).is_none());
    })
    .unwrap();
    let one = ids(&app.shared, &["one"]).remove(0);
    let kept_by_core = app.shared.pty.claude_terminal(&one).and_then(|id| app.shared.pty.buffer(&id));
    assert!(
        app.until_terminal(cx, "Fake Claude · session"),
        "on screen: {:?}; the core kept: {:?}",
        app.terminal_text(cx),
        kept_by_core
    );
}

#[gpui_kit::test]
fn forgets_nothing_while_the_last_ones_are_still_coming_back_then_follows_what_you_open_and_close(
    cx: &mut TestAppContext,
) {
    let app = launch(cx, &["one", "two"], Some("one"), Some(&["one", "two"]), |_, _| {});
    // while they come back, the list is never written with fewer
    for _ in 0..800 {
        cx.run_until_parked();
        assert_eq!(kept(&app), Some(vec!["one".to_string(), "two".to_string()]));
        if started(&app).len() == 2 {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(started(&app).len(), 2);
    // you close a workspace's Claude: the next launch doesn't start it by itself
    app.press(cx, "cmd-2");
    assert!(app.until(cx, |w| w.try_find(("term-close", 0usize)).is_some()));
    app.click(cx, ("term-close", 0usize));
    assert!(until_kept(cx, &app, &["one"]), "{:?}", kept(&app));
    // a shell alone is not a session to bring back
    assert!(app.until(cx, |w| w.try_find("empty-new-shell").is_some()));
    app.click(cx, "empty-new-shell");
    assert!(app.until(cx, |w| said(w, ("term-tab", 0usize)).as_deref() == Some("Shell")));
    settle(cx);
    assert_eq!(kept(&app), Some(vec!["one".to_string()]));
}

#[gpui_kit::test]
fn marks_a_claude_that_ends_while_its_workspace_isnt_on_screen(cx: &mut TestAppContext) {
    let app = launch(cx, &["one", "two"], Some("one"), None, |_, _| {});
    app.until_claude_started(cx, 2);
    assert!(until_kept(cx, &app, &["one", "two"]));
    let two = ids(&app.shared, &["two"]).remove(0);
    // "two" isn't on screen; its Claude ends
    let terminal = app.shared.pty.claude_terminal(&two).expect("two's Claude");
    app.shared.pty.write(&terminal, "exit\r").unwrap();
    let exited = |cx: &mut TestAppContext| {
        cx.run_until_parked();
        cx.update(|cx| {
            let terminals = enkidu_app::stores::Stores::terminals(cx);
            let terminals = terminals.read(cx);
            terminals.of(&two).and_then(|area| area.tabs.first().map(|t| (t.info.exited, t.info.exit_code)))
        })
    };
    let mut seen = None;
    for _ in 0..800 {
        seen = exited(cx);
        if seen == Some((true, Some(0))) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(seen, Some((true, Some(0))));
    // it still counts as that workspace's session: the next launch starts it again
    settle(cx);
    assert_eq!(kept(&app), Some(vec!["one".to_string(), "two".to_string()]));
    // and its workspace says so when it comes on screen
    app.press(cx, "cmd-2");
    assert!(app.until(cx, |w| said(w, ("term-tab", 0usize)).as_deref() == Some("Claude exited")));
    cx.update_window(app.window, |_, w, _| {
        assert_eq!(said(w, "restart-terminal").as_deref(), Some("Resume Claude"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn after_a_restart_every_workspace_and_its_claude_come_back_resumed(cx: &mut TestAppContext) {
    let app = launch(cx, &["one", "two"], Some("one"), None, |_, _| {});
    app.until_claude_started(cx, 2);
    // a turn in each, so each has a conversation to come back to
    assert!(app.until_terminal(cx, "\n>"));
    app.type_in_terminal(cx, "note one\r");
    assert!(app.until_terminal(cx, "Done."));
    app.press(cx, "cmd-2");
    assert!(app.until_terminal(cx, "\n>"), "{}", app.terminal_text(cx));
    app.type_in_terminal(cx, "note two\r");
    assert!(app.until_terminal(cx, "Done."));
    assert!(until_kept(cx, &app, &["one", "two"]));
    let first: Vec<(String, String)> = app
        .claude_starts()
        .iter()
        .map(|s| {
            (
                s["env"]["ENKIDU_WORKSPACE_NAME"].as_str().unwrap().to_string(),
                s["sessionId"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    // where you are is kept a moment after it changed
    cx.executor().advance_clock(Duration::from_millis(400));
    settle(cx);

    let app = common::restart(cx, app);
    app.until_claude_started(cx, 4);
    let again = app.claude_starts()[2..].to_vec();
    // the workspace you were in is on screen
    assert_eq!(app.on_screen(cx).map(|w| w.0).as_deref(), Some("two"));
    assert_eq!(again.len(), 2);
    for start in &again {
        let name = start["env"]["ENKIDU_WORKSPACE_NAME"].as_str().unwrap();
        let session = &first.iter().find(|(n, _)| n == name).expect(name).1;
        assert_eq!(start["resumed"], true, "{name} resumed");
        assert_eq!(start["sessionId"].as_str(), Some(session.as_str()), "{name}: the same conversation");
        let argv: Vec<&str> = start["argv"].as_array().unwrap().iter().map(|a| a.as_str().unwrap()).collect();
        assert!(argv.windows(2).any(|w| w[0] == "--resume" && w[1] == session), "{argv:?}");
    }
    assert!(app.until_terminal(cx, "· resumed"), "{}", app.terminal_text(cx));
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        assert_eq!(said(w, ("term-tab", 0usize)).as_deref(), Some("Claude"));
    })
    .unwrap();
}

/// Item 18.3: a workspace that is hidden is left asleep at launch. Its Claude does not come back with the others; it
/// starts when the workspace is brought back and comes on screen, and what is hidden is kept with where you were.
#[gpui_kit::test]
fn a_hidden_workspaces_claude_stays_asleep_at_launch_and_starts_when_it_is_brought_back(cx: &mut TestAppContext) {
    let app = launch(cx, &["one", "two", "three"], Some("one"), None, |shared, home| {
        // "two" was hidden when the app last ran
        let two = ids(shared, &["two"]).remove(0);
        let path = home.join("data").join("view.json");
        let mut view: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        view["hidden"] = serde_json::json!({ two: 1 });
        std::fs::write(path, view.to_string()).unwrap();
    });
    app.until_claude_started(cx, 2);
    settle(cx);
    let mut back = started(&app);
    back.sort();
    assert_eq!(back, ["one", "three"], "the hidden one is left asleep");
    assert_eq!(app.tabs(cx), ["one", "three"], "and has no tab");
    // brought back: it is on screen, and its Claude starts
    let two = ids(&app.shared, &["two"]).remove(0);
    app.frame.update(cx, |frame, cx| {
        assert!(frame.show_workspace(&two));
        cx.notify();
    });
    app.until_claude_started(cx, 3);
    assert_eq!(started(&app).last().map(String::as_str), Some("two"));
    assert_eq!(app.on_screen(cx).map(|w| w.0).as_deref(), Some("two"));
    assert_eq!(app.tabs(cx), ["one", "two", "three"], "where it stood");
    // what is hidden is kept a moment after it changes: nothing is, now
    let path = app.home.path().join("data").join("view.json");
    let hidden_kept = || {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        serde_json::from_str::<serde_json::Value>(&text).ok().map(|view| view["hidden"].clone())
    };
    let mut kept = None;
    for _ in 0..400 {
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(50));
        kept = hidden_kept();
        if kept == Some(serde_json::json!({})) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(kept, Some(serde_json::json!({})), "view.json");
}
