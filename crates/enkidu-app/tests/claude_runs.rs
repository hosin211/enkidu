//! Item 2.17: the whole-app runs of phase 2, on the sample project (`enkidu_testkit::shop`) with the fake Claude. The
//! two runs about the lamps are in `lamps.rs`. Each run starts from its own copy, so the runs need no order.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use common::{App, said};
use enkidu_core::state::Shared;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, ScrollDelta, TestAppContext, point, px};

/// What every run starts from: your Claude args and env in the user's settings, the project's own in its settings, and
/// the workspace "stock-fix" on a new branch from development.
fn start(cx: &mut TestAppContext, user_args: &'static [&'static str]) -> App {
    let app = common::start_live(cx, enkidu_testkit::shop, move |shared: &Arc<Shared>, project: &str, _: &Path| {
        {
            let mut user = shared.user.lock().unwrap();
            user.claude.args = user_args.iter().map(|a| a.to_string()).collect();
            user.claude.env.insert("FAKE_GREETING".into(), "user env reached claude".into());
        }
        let mut settings = shared.project_settings(project).unwrap();
        settings.claude.args = vec!["--permission-mode".into(), "acceptEdits".into()];
        settings.claude.env.insert("FAKE_PROJECT_ENV".into(), "project env reached claude".into());
        shared.save_project_settings(project, &settings).unwrap();
        App::make(shared, project, "stock-fix", "new", "stock-fix", Some("development"));
    });
    app.until_claude_started(cx, 1);
    app
}

fn argv(run: &serde_json::Value) -> Vec<String> {
    run["argv"].as_array().unwrap().iter().map(|a| a.as_str().unwrap().to_string()).collect()
}

/// The folder name Claude saves a folder's conversations under.
fn folder_name(path: &Path) -> String {
    path.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

#[gpui_kit::test]
fn starts_claude_with_only_its_own_flags_plus_my_args_and_env(cx: &mut TestAppContext) {
    let app = start(cx, &["--model", "opus"]);
    let run = &app.claude_starts()[0];
    let argv = argv(run);
    assert_eq!(argv[0], "--settings");
    assert!(Path::new(&argv[1]).exists());
    assert_eq!(argv[2], "--session-id");
    assert_eq!(argv[4..6], ["--name", "stock-fix"]);
    // a short note on what enkidu is for, then the user's own args, then the project's
    assert_eq!(argv[6], "--append-system-prompt");
    assert!(argv[7].contains("enkidu status"), "{}", argv[7]);
    assert_eq!(argv[8..], ["--model", "opus", "--permission-mode", "acceptEdits"]);
    let env = &run["env"];
    assert_eq!(env["ENKIDU_TOKEN"], "(set)");
    let api = env["ENKIDU_API"].as_str().unwrap();
    assert!(api.strip_prefix("http://127.0.0.1:").is_some_and(|port| port.parse::<u16>().is_ok()), "{api}");
    assert_eq!(run["resumed"], false);
    assert_eq!(env["FAKE_GREETING"], "user env reached claude");
    assert_eq!(env["FAKE_PROJECT_ENV"], "project env reached claude");
    assert_eq!(env["CLAUDE_CONFIG_DIR"].as_str(), Some(app.home.path().join("claude-config").to_str().unwrap()));
    let ws = app.frame.read_with(cx, |f, _| f.current_workspace().cloned()).expect("on screen");
    assert_eq!(env["ENKIDU_WORKSPACE_PATH"].as_str(), Some(ws.path.as_str()));
    assert!(app.until_terminal(cx, "user env reached claude"), "{}", app.terminal_text(cx));
}

#[gpui_kit::test]
fn scrolls_a_program_that_takes_the_mouse_one_line_per_line_of_trackpad_like_a_native_terminal(
    cx: &mut TestAppContext,
) {
    let app = start(cx, &[]);
    assert!(app.until_terminal(cx, "\n>"));
    app.type_in_terminal(cx, "mouse on\r");
    assert!(app.until_terminal(cx, "Mouse reporting on"));
    // a trackpad swipe of ten lines' height over the terminal, toward the bottom
    let line_height = app.shell.read_with(cx, |shell, cx| {
        shell.terminal_on_screen(cx).expect("a terminal").read(cx).layout().expect("drawn").line_height
    });
    cx.update_window(app.window, |_, w, cx| {
        w.scroll("terminal", ScrollDelta::Pixels(point(px(0.), -(line_height * 10.))), cx);
    })
    .unwrap();
    // the terminal echoes what went to Claude's input: ten wheel-down reports
    let reports = |cx: &mut TestAppContext| app.terminal_text(cx).replace('\n', "").matches("[<65;").count();
    let mut seen = 0;
    for _ in 0..800 {
        seen = reports(cx);
        if seen == 10 {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(seen, 10, "not one wheel report per line:\n{}", app.terminal_text(cx));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(reports(cx), 10);
    app.type_in_terminal(cx, "mouse off\r");
    assert!(app.until_terminal(cx, "Mouse reporting off"));
}

#[gpui_kit::test]
fn resumes_claudes_session_when_claude_starts_again(cx: &mut TestAppContext) {
    let app = start(cx, &["--model", "opus", "--verbose"]);
    assert!(app.until_terminal(cx, "\n>"));
    // a turn, so there is a conversation saved to come back to
    app.type_in_terminal(cx, "add a test\r");
    assert!(app.until_terminal(cx, "Done."));
    app.type_in_terminal(cx, "exit\r");
    assert!(app.until(cx, |w| w.try_find("restart-terminal").is_some()));
    app.click(cx, "restart-terminal");
    app.until_claude_started(cx, 2);
    let runs = app.claude_starts();
    let (first, second) = (&runs[0], &runs[1]);
    assert_eq!(second["resumed"], true);
    let again = argv(second);
    assert_eq!(again[2], "--resume");
    assert_eq!(second["sessionId"], first["sessionId"]);
    // Enkidu's own flags (the name and its note on enkidu), then my args, then the project's
    assert_eq!(again[4..7], ["--name", "stock-fix", "--append-system-prompt"]);
    assert_eq!(again[8..], ["--model", "opus", "--verbose", "--permission-mode", "acceptEdits"]);
}

#[gpui_kit::test]
fn follows_a_conversation_you_resume_in_the_workspace_enkidu_reads_it_and_a_restart_brings_it_back_there(
    cx: &mut TestAppContext,
) {
    let app = start(cx, &[]);
    let config = app.home.path().join("claude-config");
    let main = app.project.canonicalize().unwrap();
    // a conversation started in the main folder (in Warp, say), which Claude saved under the main folder's name
    let saved_in_main = config.join("projects").join(folder_name(&main)).join("from-main-1.jsonl");
    std::fs::create_dir_all(saved_in_main.parent().unwrap()).unwrap();
    std::fs::write(&saved_in_main, "{\"type\":\"user\",\"prompt\":\"started in the main folder\"}\n").unwrap();
    assert!(app.until_terminal(cx, "\n>"));
    app.type_in_terminal(cx, &format!("/resume from-main-1 {}\r", main.display()));
    assert!(app.until_terminal(cx, "Resumed from-main-1"), "{}", app.terminal_text(cx));
    // enkidu in that Claude reads the conversation it is in now, not the one its terminal started with
    app.type_in_terminal(cx, "enkidu status --json\r");
    assert!(
        app.until_terminal(cx, "\"session\": \"from-main-1\""),
        "enkidu still reads the conversation the terminal started with:\n{}",
        app.terminal_text(cx)
    );
    // Claude starts again: the resumed conversation comes back, copied in, so Claude is in the workspace folder again
    let before = app.claude_starts().len();
    app.type_in_terminal(cx, "exit\r");
    assert!(app.until(cx, |w| said(w, "restart-terminal").is_some()));
    app.click(cx, "restart-terminal");
    let again = app.until_claude_started(cx, before + 1);
    assert_eq!(argv(&again)[2..4], ["--resume", "from-main-1"]);
    let ws = app.frame.read_with(cx, |f, _| f.workspaces[0].clone());
    let ws_path = PathBuf::from(&ws.path).canonicalize().unwrap();
    assert_eq!(PathBuf::from(again["cwd"].as_str().unwrap()).canonicalize().unwrap(), ws_path);
    assert_eq!(again["env"]["ENKIDU_SESSION"], "from-main-1");
    let copied: Vec<PathBuf> = std::fs::read_dir(config.join("projects"))
        .unwrap()
        .filter_map(|dir| dir.ok().map(|d| d.path().join("from-main-1.jsonl")))
        .filter(|file| file.exists() && *file != saved_in_main)
        .collect();
    assert_eq!(copied.len(), 1, "copied into the workspace's own folder: {copied:?}");
    assert!(copied[0].to_string_lossy().contains("stock-fix"), "{copied:?}");
    assert!(saved_in_main.exists(), "the original stays where it was");
}
