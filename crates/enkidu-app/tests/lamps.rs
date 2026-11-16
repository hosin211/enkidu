//! Item 2.12: the lamps from Claude's hooks, with the fake Claude and the whole wiring (the fake runs the hooks of the
//! `--settings` file the core gave it; they post to the core's hook server; the core's news turns the lamp): working,
//! needs you, done; and idle after Esc or Ctrl+C, for which Claude Code runs no hook at all.

mod common;

use std::time::Duration;

use common::{App, said};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};

fn start(cx: &mut TestAppContext) -> App {
    let app = common::start_live(
        cx,
        |home| enkidu_testkit::repo_with_origin(home, "shop"),
        |shared, project, _| App::make(shared, project, "stock-fix", "new", "stock-fix", Some("main")),
    );
    app.until_claude_started(cx, 1);
    assert!(app.until_terminal(cx, "\n>"), "Claude's prompt");
    app
}

/// Claude's state in the workspace on screen, as the frame has it.
fn status(cx: &mut TestAppContext, app: &App) -> String {
    cx.run_until_parked();
    app.frame.read_with(cx, |f, _| f.status(&f.current_workspace().expect("on screen").id).to_string())
}

fn until_status(cx: &mut TestAppContext, app: &App, want: &str) -> bool {
    let want = want.to_string();
    app.until_frame(cx, move |f| f.current_workspace().is_some_and(|w| f.status(&w.id) == want))
}

/// What the status line says of Claude.
fn line(cx: &mut TestAppContext, app: &App) -> Option<String> {
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        said(w, "status-claude")
    })
    .unwrap()
}

#[gpui_kit::test]
fn shows_working_needs_you_and_done_from_claudes_hooks(cx: &mut TestAppContext) {
    let app = start(cx);
    assert_eq!(status(cx, &app), "idle");
    // Claude asks for permission: the tab and the status line say so, and the line under the tabs stops
    app.type_in_terminal(cx, "ask\r");
    assert!(until_status(cx, &app, "needs_you"), "never needs_you: {}", app.terminal_text(cx));
    assert!(app.until(cx, |w| said(w, ("tab-words", 0usize)).as_deref() == Some("Needs you")));
    assert_eq!(line(cx, &app).as_deref(), Some("Claude needs you"));
    // answered: it works, then its turn ends
    app.type_in_terminal(cx, "y\r");
    assert!(until_status(cx, &app, "done"), "never done: {}", app.terminal_text(cx));
    assert_eq!(line(cx, &app).as_deref(), Some("Claude is done"));
    // the tab no longer asks for you (it may say what Claude changed)
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        assert_ne!(said(w, ("tab-words", 0usize)).as_deref(), Some("Needs you"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn goes_back_to_idle_when_claude_is_interrupted_though_claude_code_runs_no_hook_for_that(cx: &mut TestAppContext) {
    let app = start(cx);
    // working, then quiet with no hook and no key: nothing says Claude stopped, so it still counts as working
    app.type_in_terminal(cx, "spin 1\r");
    assert!(until_status(cx, &app, "working"), "never working");
    assert!(app.until(cx, |w| w.try_find("claude-activity").is_some()), "the line under the tabs runs");
    std::thread::sleep(Duration::from_millis(3500));
    assert_eq!(status(cx, &app), "working");
    // Esc while it works, from the keyboard: once it has stopped printing, the workspace is idle
    app.type_in_terminal(cx, "spin 2\r");
    std::thread::sleep(Duration::from_millis(500));
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        assert_eq!(w.find("terminal").focused(), Some(true));
    })
    .unwrap();
    app.press(cx, "escape");
    assert!(until_status(cx, &app, "idle"), "still working after Esc");
    assert_eq!(line(cx, &app).as_deref(), Some("Claude is idle"));
    cx.update_window(app.window, |_, w, _| assert!(w.try_find("claude-activity").is_none())).unwrap();
}

#[gpui_kit::test]
fn ctrl_c_brings_an_interrupted_claude_back_to_idle_too(cx: &mut TestAppContext) {
    // Claude Code reads Ctrl+C as a key (its terminal is raw). The fake's isn't, so its `claude` here turns the
    // terminal's own Ctrl+C handling off first: the key reaches the fake as a key, as it reaches Claude.
    let app = common::start_live(
        cx,
        |home| enkidu_testkit::repo_with_origin(home, "shop"),
        |shared, project, home| {
            use std::os::unix::fs::PermissionsExt as _;
            let plain = home.join("bin").join("claude");
            let raw = home.join("bin").join("claude-keys");
            let script = std::fs::read_to_string(&plain).unwrap().replace("exec node", "stty -isig\nexec node");
            std::fs::write(&raw, script).unwrap();
            std::fs::set_permissions(&raw, std::fs::Permissions::from_mode(0o755)).unwrap();
            shared.user.lock().unwrap().claude.command = raw.to_string_lossy().into();
            App::make(shared, project, "stock-fix", "new", "stock-fix", Some("main"));
        },
    );
    app.until_claude_started(cx, 1);
    assert!(app.until_terminal(cx, "\n>"));
    app.type_in_terminal(cx, "spin 2\r");
    assert!(until_status(cx, &app, "working"));
    std::thread::sleep(Duration::from_millis(500));
    app.press(cx, "ctrl-c");
    assert!(until_status(cx, &app, "idle"), "still working after Ctrl+C");
    // Claude wasn't ended by it: its tab is as it was
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        assert_eq!(said(w, ("term-tab", 0usize)).as_deref(), Some("Claude"));
    })
    .unwrap();
    assert!(app.shared.pty.running_claude(&app.frame.read_with(cx, |f, _| f.current_workspace().unwrap().id.clone())));
}
