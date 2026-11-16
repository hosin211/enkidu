//! Item 2.13: text for Claude is typed into its prompt and never sent; you read it there and press Enter. Also for a
//! Claude that is just starting: it starts without the text on its command line (Claude would send a first message
//! given there), and the core types it in once the session is up. With the fake Claude, which records every prompt
//! that was sent to it.

mod common;

use std::time::Duration;

use common::{App, said};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};

/// The prompts that were sent to a fake Claude, oldest first.
fn sent(app: &App) -> Vec<String> {
    let log = format!("{}.prompts", app.claude_log.display());
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter_map(|said| said["prompt"].as_str().map(String::from))
        .collect()
}

fn until_sent(cx: &mut TestAppContext, app: &App, want: &[&str]) -> bool {
    for _ in 0..800 {
        cx.run_until_parked();
        if sent(app) == want {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

fn type_into_claude(cx: &mut TestAppContext, workspace: &str, text: &str) {
    cx.update(|cx| {
        // what became of it isn't awaited here: the tests read Claude's terminal
        drop(
            enkidu_app::stores::Stores::terminals(cx)
                .update(cx, |terminals, cx| terminals.type_into_claude(workspace, text, cx)),
        );
    });
}

fn workspace(cx: &mut TestAppContext, app: &App, name: &str) -> String {
    app.frame.read_with(cx, |f, _| f.workspaces.iter().find(|w| w.name == name).map(|w| w.id.clone())).expect(name)
}

fn two_workspaces(cx: &mut TestAppContext) -> App {
    let app = common::start_live(
        cx,
        |home| enkidu_testkit::repo_with_origin(home, "shop"),
        |shared, project, home| {
            App::make(shared, project, "one", "new", "one", Some("main"));
            App::make(shared, project, "two", "new", "two", Some("main"));
            // "two" had no Claude when the app last ran: none starts there until it is asked for
            common::only_these_had_claude(shared, home, &["one"]);
        },
    );
    app.click(cx, ("tab", 0usize));
    app.until_claude_started(cx, 1);
    assert!(app.until_terminal(cx, "\n>"), "Claude's prompt");
    app
}

#[gpui_kit::test]
fn text_for_a_running_claude_goes_into_its_prompt_and_waits_for_your_enter(cx: &mut TestAppContext) {
    let app = two_workspaces(cx);
    let one = workspace(cx, &app, "one");
    type_into_claude(cx, &one, "Fix the totals in stock.py\nand add a test");
    assert!(app.until_terminal(cx, "and add a test"), "it is in the prompt: {}", app.terminal_text(cx));
    // Claude's terminal has the keyboard, so Enter is right there
    assert!(app.until(cx, |w| w.find("terminal").focused() == Some(true)));
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(sent(&app), Vec::<String>::new(), "nothing was sent");
    // you press Enter: now Claude has it, as one message
    app.press(cx, "enter");
    assert!(app.until_terminal(cx, "[Pasted text +2 lines]"));
    assert!(until_sent(cx, &app, &["Fix the totals in stock.py\nand add a test"]), "{:?}", sent(&app));
}

#[gpui_kit::test]
fn a_claude_that_is_just_starting_gets_the_text_once_it_is_up_never_as_a_first_message(cx: &mut TestAppContext) {
    let app = two_workspaces(cx);
    // the second workspace was never opened: no Claude runs there
    let two = workspace(cx, &app, "two");
    type_into_claude(cx, &two, "Look at the returns first");
    let started = app.until_claude_started(cx, 2);
    let argv: Vec<&str> = started["argv"].as_array().unwrap().iter().map(|a| a.as_str().unwrap()).collect();
    assert!(argv.iter().all(|a| !a.contains("Look at the returns")), "never on Claude's command line: {argv:?}");
    assert_eq!(started["env"]["ENKIDU_WORKSPACE"], two.as_str());
    // on its screen, the text is in the prompt once Claude is up
    app.press(cx, "cmd-2");
    assert!(app.until_terminal(cx, "Look at the returns first"), "{}", app.terminal_text(cx));
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(sent(&app), Vec::<String>::new(), "nothing was sent");
    app.press(cx, "enter");
    assert!(until_sent(cx, &app, &["Look at the returns first"]), "{:?}", sent(&app));
}

#[gpui_kit::test]
fn typing_into_claude_puts_its_tab_on_screen(cx: &mut TestAppContext) {
    let app = two_workspaces(cx);
    let one = workspace(cx, &app, "one");
    // a shell is on screen
    app.click(cx, "term-add");
    assert!(app.until(cx, |w| w.try_find("new-shell").is_some()));
    app.click(cx, "new-shell");
    assert!(app.until(cx, |w| w.try_find(("term-tab", 1usize)).is_some_and(|t| t.selected() == Some(true))));
    type_into_claude(cx, &one, "Then run the tests");
    assert!(app.until(cx, |w| w.find(("term-tab", 0usize)).selected() == Some(true)), "Claude's tab came on screen");
    assert!(app.until_terminal(cx, "Then run the tests"));
    cx.update_window(app.window, |_, w, _| assert_eq!(said(w, ("term-tab", 0usize)).as_deref(), Some("Claude")))
        .unwrap();
    assert_eq!(sent(&app), Vec::<String>::new());
}
