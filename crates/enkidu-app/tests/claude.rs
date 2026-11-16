//! Item 2.11: Claude in a workspace, with the whole wiring: the core starts it (your args and env from Settings, the
//! hooks through `--settings`, its conversation and name), enkidu-term shows it, and the island's band has its tab,
//! shells beside it, New terminal, the running line, and the way back after it exits. The Claude here is the fake one
//! (`enkidu-testkit/fakes/fake-claude.mjs`), never the real `claude`; a shell tab runs `/bin/sh`.

mod common;

use common::{App, said};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};

fn start(cx: &mut TestAppContext, settings: impl FnOnce(&mut enkidu_core::settings::UserSettings)) -> App {
    common::start_live(
        cx,
        |home| enkidu_testkit::repo_with_origin(home, "shop"),
        |shared, project, _| {
            settings(&mut shared.user.lock().unwrap());
            App::make(shared, project, "stock-fix", "new", "stock-fix", Some("main"));
        },
    )
}

fn tabs(cx: &mut TestAppContext, app: &App) -> Vec<(String, bool)> {
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        (0..8usize)
            .map_while(|ix| w.try_find(("term-tab", ix)))
            .map(|t| (t.label().unwrap_or_default().to_string(), t.selected() == Some(true)))
            .collect()
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_workspace_on_screen_starts_its_claude_with_your_args_and_env_the_hooks_and_its_conversation(
    cx: &mut TestAppContext,
) {
    let app = start(cx, |user| {
        user.claude.args = vec!["--model".into(), "opus".into()];
        user.claude.env.insert("FAKE_FLAVOR".into(), "vanilla".into());
    });
    let started = app.until_claude_started(cx, 1);
    let ws = app.frame.read_with(cx, |f, _| f.current_workspace().cloned()).expect("on screen");
    let argv: Vec<String> = started["argv"].as_array().unwrap().iter().map(|a| a.as_str().unwrap().into()).collect();
    // Claude's own flags first: the hooks, a conversation of its own, the workspace's name; then yours
    assert_eq!(argv[0], "--settings");
    assert_eq!(argv[2..6], ["--session-id", ws.session_id.as_str(), "--name", "stock-fix"]);
    assert_eq!(argv[6], "--append-system-prompt");
    assert_eq!(argv[8..], ["--model", "opus"]);
    assert_eq!(started["resumed"], false);
    // it runs in the workspace's folder, with your variable and the app's own
    let cwd = std::path::PathBuf::from(started["cwd"].as_str().unwrap()).canonicalize().unwrap();
    assert_eq!(cwd, std::path::PathBuf::from(&ws.path).canonicalize().unwrap());
    let env = &started["env"];
    assert_eq!(env["FAKE_FLAVOR"], "vanilla");
    assert_eq!(env["ENKIDU_WORKSPACE"], ws.id.as_str());
    assert_eq!(env["ENKIDU_SESSION"], ws.session_id.as_str());
    assert!(env["ENKIDU_API"].as_str().is_some_and(|api| api.starts_with("http://127.0.0.1:")));
    assert_eq!(env["ENKIDU_TOKEN"], "(set)");
    assert_eq!(env["ENKIDU_BRANCH"], "stock-fix");
    // the hooks in that settings file all tell this app's own hook server, about this workspace
    let settings: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&argv[1]).unwrap()).unwrap();
    let hooks = settings["hooks"].as_object().expect("hooks");
    assert!(hooks.contains_key("Stop") && hooks.contains_key("UserPromptSubmit"), "{:?}", hooks.keys());
    let server = env["ENKIDU_API"].as_str().unwrap();
    for (event, groups) in hooks {
        for group in groups.as_array().unwrap() {
            for hook in group["hooks"].as_array().unwrap() {
                let command = hook["command"].as_str().unwrap();
                assert!(command.contains(server) && command.contains(&ws.id), "{event}: {command}");
            }
        }
    }
    // its tab, and what it printed, are on screen
    assert_eq!(tabs(cx, &app), [("Claude".to_string(), true)]);
    assert!(app.until_terminal(cx, &format!("Fake Claude · session {} · new", ws.session_id)));
    assert!(app.until_terminal(cx, "args: --settings --session-id"), "{}", app.terminal_text(cx));
}

#[gpui_kit::test]
fn a_shell_opens_beside_claude_in_the_workspaces_folder_and_closes(cx: &mut TestAppContext) {
    let app = start(cx, |_| {});
    app.until_claude_started(cx, 1);
    app.click(cx, "term-add");
    assert!(app.until(cx, |w| w.try_find("new-shell").is_some()), "the menu opened");
    cx.update_window(app.window, |_, w, _| {
        assert!(w.try_find("start-claude").is_none(), "Claude runs: only a shell is offered");
    })
    .unwrap();
    app.click(cx, "new-shell");
    assert!(app.until(cx, |w| w.try_find(("term-tab", 1usize)).is_some()));
    assert_eq!(tabs(cx, &app), [("Claude".to_string(), false), ("Shell".to_string(), true)]);
    // the shell runs in the workspace's folder and knows its branch
    app.type_in_terminal(cx, "echo here:$PWD:$ENKIDU_BRANCH\r");
    let folder = app.workspaces.join("shop").join("stock-fix").canonicalize().unwrap();
    assert!(app.until_terminal(cx, &format!("here:{}:stock-fix", folder.display())), "{}", app.terminal_text(cx));
    // a second shell: both are numbered
    app.click(cx, "term-add");
    assert!(app.until(cx, |w| w.try_find("new-shell").is_some()));
    app.click(cx, "new-shell");
    assert!(app.until(cx, |w| w.try_find(("term-tab", 2usize)).is_some()));
    let now = tabs(cx, &app);
    assert_eq!(now.iter().map(|t| t.0.as_str()).collect::<Vec<_>>(), ["Claude", "Shell 1", "Shell 2"]);
    // a click on Claude's tab puts it back on screen
    app.click(cx, ("term-pick", 0usize));
    assert!(app.until(cx, |w| w.find(("term-tab", 0usize)).selected() == Some(true)));
    assert!(app.until_terminal(cx, "Fake Claude"));
    // closing a shell takes its tab away; the one left is "Shell" again
    cx.update_window(app.window, |_, w, _| {
        assert_eq!(said(w, ("term-close", 2usize)).as_deref(), Some("Close Shell 2"));
    })
    .unwrap();
    app.click(cx, ("term-close", 2usize));
    assert!(app.until(cx, |w| w.try_find(("term-tab", 2usize)).is_none()));
    assert_eq!(tabs(cx, &app), [("Claude".to_string(), true), ("Shell".to_string(), false)]);
}

#[gpui_kit::test]
fn a_claude_that_exited_says_so_and_resumes_its_conversation(cx: &mut TestAppContext) {
    let app = start(cx, |_| {});
    let first = app.until_claude_started(cx, 1);
    assert!(app.until_terminal(cx, "\n>"), "its prompt");
    // a prompt, so there is a conversation to come back to; then Claude ends
    app.type_in_terminal(cx, "add a test\r");
    assert!(app.until_terminal(cx, "Done."), "{}", app.terminal_text(cx));
    app.type_in_terminal(cx, "exit\r");
    assert!(app.until(cx, |w| said(w, ("term-tab", 0usize)).as_deref() == Some("Claude exited")));
    cx.update_window(app.window, |_, w, _| {
        assert_eq!(said(w, "terminal-exited").as_deref(), Some("Claude exited with code 0."));
        assert_eq!(said(w, "restart-terminal").as_deref(), Some("Resume Claude"));
    })
    .unwrap();
    app.click(cx, "restart-terminal");
    let second = app.until_claude_started(cx, 2);
    assert_eq!(second["resumed"], true);
    assert_eq!(second["sessionId"], first["sessionId"], "the same conversation");
    let argv: Vec<&str> = second["argv"].as_array().unwrap().iter().map(|a| a.as_str().unwrap()).collect();
    assert!(argv.windows(2).any(|w| w[0] == "--resume" && w[1] == first["sessionId"].as_str().unwrap()), "{argv:?}");
    assert!(app.until(cx, |w| said(w, ("term-tab", 0usize)).as_deref() == Some("Claude")), "one tab, running again");
    assert_eq!(tabs(cx, &app).len(), 1);
    assert!(app.until_terminal(cx, "· resumed"));
}

#[gpui_kit::test]
fn a_claude_that_cannot_start_says_why_and_starts_once_it_can(cx: &mut TestAppContext) {
    let app = start(cx, |user| user.claude.command = "/nowhere/claude".into());
    assert!(app.until(cx, |w| w.try_find("claude-error").is_some()));
    cx.update_window(app.window, |_, w, _| {
        assert_eq!(
            said(w, "claude-error").as_deref(),
            Some("Can't find \"/nowhere/claude\". Set the claude command in Settings.")
        );
        assert!(w.try_find(("term-tab", 0usize)).is_none(), "no tab for a Claude that never ran");
    })
    .unwrap();
    assert!(app.claude_starts().is_empty());
    // "Open Claude settings" (item 4.3b) opens Settings on the Claude page, where the command is put right
    let fake = app.home.path().join("bin").join("claude");
    app.click(cx, "open-claude-settings");
    assert!(
        app.until(cx, |w| w.try_find("settings-dialog").is_some_and(|dialog| dialog.description() == Some("Claude"))),
        "Open Claude settings opened {:?}",
        cx.update_window(app.window, |_, w, _| w
            .try_find("settings-dialog")
            .map(|d| d.description().map(String::from)))
            .unwrap()
    );
    assert!(app.until(cx, |w| w.try_find("claude-command").is_some()));
    app.click(cx, "claude-command");
    app.press(cx, "cmd-a");
    app.typed(cx, &fake.to_string_lossy());
    app.click(cx, "save-settings");
    assert!(app.until(cx, |w| w.try_find("settings-dialog").is_none()), "Save left Settings open");
    assert_eq!(app.shared.user_settings().claude.command, fake.to_string_lossy());
    // nothing starts by itself: Try again starts it
    assert!(app.claude_starts().is_empty());
    app.click(cx, "try-claude-again");
    app.until_claude_started(cx, 1);
    assert!(app.until(cx, |w| w.try_find("claude-error").is_none() && w.try_find(("term-tab", 0usize)).is_some()));
}

#[gpui_kit::test]
fn the_line_under_the_tabs_runs_while_claude_works(cx: &mut TestAppContext) {
    let app = start(cx, |_| {});
    app.until_claude_started(cx, 1);
    let ws = app.frame.read_with(cx, |f, _| f.current_workspace().unwrap().id.clone());
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("claude-activity").is_none(), "idle: nothing runs");
    })
    .unwrap();
    app.shared.set_status(&app.sink, &ws, "working");
    assert!(app.until(cx, |w| w.try_find("claude-activity").is_some()));
    app.shared.set_status(&app.sink, &ws, "needs_you");
    assert!(app.until(cx, |w| w.try_find("claude-activity").is_none()), "it stops when Claude waits for you");
}

#[gpui_kit::test]
fn with_every_terminal_closed_the_island_offers_a_shell_or_claude(cx: &mut TestAppContext) {
    let app = start(cx, |_| {});
    app.until_claude_started(cx, 1);
    assert!(app.until(cx, |w| w.try_find(("term-close", 0usize)).is_some()));
    app.click(cx, ("term-close", 0usize));
    assert!(app.until(cx, |w| w.try_find("no-terminal").is_some()));
    // New terminal now offers Claude too
    app.click(cx, "term-add");
    assert!(app.until(cx, |w| w.try_find("start-claude").is_some() && w.try_find("new-shell").is_some()));
    app.press(cx, "escape");
    app.click(cx, "empty-start-claude");
    app.until_claude_started(cx, 2);
    assert!(app.until(cx, |w| w.try_find("no-terminal").is_none() && w.try_find(("term-tab", 0usize)).is_some()));
}

#[gpui_kit::test]
fn each_workspace_keeps_its_own_claude_and_the_one_on_screen_has_the_keyboard(cx: &mut TestAppContext) {
    let app = common::start_live(
        cx,
        |home| enkidu_testkit::repo_with_origin(home, "shop"),
        |shared, project, _| {
            App::make(shared, project, "one", "new", "one", Some("main"));
            App::make(shared, project, "two", "new", "two", Some("main"));
        },
    );
    // at launch each workspace's Claude starts, in a conversation of its own
    app.until_claude_started(cx, 2);
    let session = |name: &str| {
        let starts = app.claude_starts();
        let start = starts.iter().find(|s| s["env"]["ENKIDU_WORKSPACE_NAME"] == name).expect(name);
        start["sessionId"].as_str().unwrap().to_string()
    };
    let (one, two) = (session("one"), session("two"));
    assert_ne!(one, two);
    app.click(cx, ("tab", 0usize));
    assert!(app.until_terminal(cx, &format!("session {one}")), "{}", app.terminal_text(cx));
    // the second workspace comes on screen: its own Claude, and the keyboard is in its terminal
    app.press(cx, "cmd-2");
    assert!(app.until_terminal(cx, &format!("session {two}")));
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        assert_eq!(w.find("terminal").focused(), Some(true), "the terminal on screen has the keyboard");
    })
    .unwrap();
    // back to the first: the same Claude, nothing started again
    app.press(cx, "cmd-1");
    assert!(app.until_terminal(cx, &format!("session {one}")));
    assert_eq!(app.claude_starts().len(), 2);
}

#[gpui_kit::test]
fn starting_claude_is_said_only_when_the_wait_is_long(cx: &mut TestAppContext) {
    let app = common::start_live(
        cx,
        |home| enkidu_testkit::repo_with_origin(home, "shop"),
        |shared, project, home| {
            App::make(shared, project, "one", "new", "one", Some("main"));
            App::make(shared, project, "two", "new", "two", Some("main"));
            // "two" had no Claude when the app last ran: it starts when the workspace is opened
            common::only_these_had_claude(shared, home, &["one"]);
        },
    );
    app.click(cx, ("tab", 0usize));
    app.until_claude_started(cx, 1);
    // the core is held up (as on its first start, while it asks the login shell for PATH): the next Claude waits
    let (release, held) = std::sync::mpsc::channel::<()>();
    let (holding, ready) = std::sync::mpsc::channel::<()>();
    let shared = app.shared.clone();
    let holder = std::thread::spawn(move || {
        let _hooks = shared.hooks.lock().unwrap();
        holding.send(()).unwrap();
        let _ = held.recv();
    });
    ready.recv().unwrap();
    app.press(cx, "cmd-2");
    assert!(app.until(cx, |w| w.try_find(("term-tab", 0usize)).is_none()), "the other workspace: no terminal yet");
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("starting-claude").is_none(), "a short wait says nothing");
    })
    .unwrap();
    cx.executor().advance_clock(std::time::Duration::from_millis(800));
    assert!(app.until(cx, |w| said(w, "starting-claude").as_deref() == Some("Starting Claude…")));
    release.send(()).unwrap();
    holder.join().unwrap();
    app.until_claude_started(cx, 2);
    assert!(app.until(cx, |w| w.try_find("starting-claude").is_none() && w.try_find(("term-tab", 0usize)).is_some()));
}

#[gpui_kit::test]
fn removing_a_workspace_ends_its_claude_and_forgets_its_terminals(cx: &mut TestAppContext) {
    let app = start(cx, |_| {});
    app.until_claude_started(cx, 1);
    let ws = app.frame.read_with(cx, |f, _| f.current_workspace().unwrap().id.clone());
    assert!(app.shared.pty.running_claude(&ws));
    app.click(cx, "ws-menu");
    assert!(app.until(cx, |w| w.try_find("remove-workspace").is_some()));
    app.click(cx, "remove-workspace");
    assert!(app.until(cx, |w| w.try_find("confirm-remove").is_some()));
    app.click(cx, "confirm-remove");
    // the fake wrote nothing, so there is nothing uncommitted to ask about
    assert!(app.until_frame(cx, |f| f.workspaces.is_empty()));
    assert!(!app.shared.pty.running_claude(&ws), "its Claude ended with it");
    let kept = cx.update(|cx| enkidu_app::stores::Stores::terminals(cx).read(cx).of(&ws).is_some());
    assert!(!kept, "its terminals are forgotten");
}

/// The keyboard is never nowhere. A dialog gives the keyboard back to what had it when it opened; with no terminal
/// yet (Claude is still starting, as right after the app opens) that is the window itself, which stops holding the
/// keyboard once a terminal is there. The terminal must take it then, or no key of the app works until a click.
#[gpui_kit::test]
fn a_dialog_closed_after_the_terminal_came_leaves_the_keyboard_in_the_terminal(cx: &mut TestAppContext) {
    let app = common::start_live(
        cx,
        |home| enkidu_testkit::repo_with_origin(home, "shop"),
        |shared, project, home| {
            App::make(shared, project, "one", "new", "one", Some("main"));
            App::make(shared, project, "two", "new", "two", Some("main"));
            // "two" had no Claude when the app last ran: it starts when the workspace is opened
            common::only_these_had_claude(shared, home, &["one"]);
        },
    );
    app.click(cx, ("tab", 0usize));
    app.until_claude_started(cx, 1);
    // the core is held up: the next Claude waits, so its workspace is on screen with no terminal yet
    let (release, held) = std::sync::mpsc::channel::<()>();
    let (holding, ready) = std::sync::mpsc::channel::<()>();
    let shared = app.shared.clone();
    let holder = std::thread::spawn(move || {
        let _hooks = shared.hooks.lock().unwrap();
        holding.send(()).unwrap();
        let _ = held.recv();
    });
    ready.recv().unwrap();
    app.press(cx, "cmd-2");
    assert!(app.until(cx, |w| w.try_find(("term-tab", 0usize)).is_none()), "the other workspace: no terminal yet");
    // the palette opens meanwhile
    app.press(cx, "cmd-k");
    assert!(app.until(cx, |w| w.try_find("command-palette").is_some()), "⌘K opened nothing");
    // Claude starts behind it: the palette keeps the keyboard, what is typed goes into its field
    release.send(()).unwrap();
    holder.join().unwrap();
    app.until_claude_started(cx, 2);
    assert!(app.until(cx, |w| w.try_find(("term-tab", 0usize)).is_some()), "the terminal is there");
    app.typed(cx, "settings");
    assert!(
        app.until(cx, |w| said(w, ("palette-row", 0usize)).as_deref() == Some("Settings")),
        "typing with the palette open: its first row says {:?}",
        cx.update_window(app.window, |_, w, _| said(w, ("palette-row", 0usize))).unwrap()
    );
    // closed: the keyboard is in the terminal, and the app's keys go on working
    app.press(cx, "escape");
    assert!(app.until(cx, |w| w.try_find("command-palette").is_none()));
    assert!(
        app.until(cx, |w| w.try_find("terminal").is_some_and(|terminal| terminal.focused() == Some(true))),
        "the terminal has the keyboard after the palette closed"
    );
    app.press(cx, "cmd-,");
    assert!(app.until(cx, |w| w.try_find("settings-dialog").is_some()), "⌘, did nothing after the palette closed");
}

/// Every key a program in a terminal can be sent: the named keys and the letters, digits and signs, alone and with
/// Shift, Ctrl and Alt. Nothing here holds ⌘: those are the app's.
fn keys_of_a_terminal() -> Vec<String> {
    let named = [
        "tab",
        "enter",
        "escape",
        "backspace",
        "delete",
        "space",
        "up",
        "down",
        "left",
        "right",
        "home",
        "end",
        "pageup",
        "pagedown",
        "insert",
        "f1",
        "f2",
        "f3",
        "f4",
        "f5",
        "f6",
        "f7",
        "f8",
        "f9",
        "f10",
        "f11",
        "f12",
    ];
    let typed = r"abcdefghijklmnopqrstuvwxyz0123456789`-=[]\;',./";
    let mut keys = Vec::new();
    for with in ["", "shift-", "ctrl-", "alt-", "ctrl-shift-", "alt-shift-", "ctrl-alt-"] {
        keys.extend(named.iter().map(|key| format!("{with}{key}")));
        keys.extend(typed.chars().map(|key| format!("{with}{key}")));
    }
    keys
}

/// Tab in a terminal used to move the keyboard to another part of the window. gpui-kit's root binds Tab and Shift-Tab
/// to "the next control" for everything
/// inside it, and GPUI asks the bindings before a key listener: so the shell never completed a name and Claude never
/// changed its mode. Inside a terminal no key without ⌘ may be bound at all, but the two that change the workspace.
#[gpui_kit::test]
fn inside_a_terminal_no_key_is_the_apps_but_those_with_cmd(cx: &mut TestAppContext) {
    let app = start(cx, |_| {});
    app.until_claude_started(cx, 1);
    assert!(app.until(cx, |w| w.find("terminal").focused() == Some(true)), "Claude's terminal has the keyboard");
    let taken: Vec<String> = cx
        .update_window(app.window, |_, w, cx| {
            w.render_frame(cx);
            let held = w.context_stack();
            assert!(held.iter().any(|context| context.contains("Terminal")), "the keys are asked for in a terminal");
            let keymap = cx.key_bindings();
            let keymap = keymap.borrow();
            keys_of_a_terminal()
                .into_iter()
                .filter(|key| {
                    let os = cx.platform_info().os;
                    let key = gpui_kit::Keystroke::parse_with_os(key, os).unwrap_or_else(|_| panic!("{key} is a key"));
                    let (bound, more_to_come) = keymap.bindings_for_input(&[key], &held);
                    !bound.is_empty() || more_to_come
                })
                .collect()
        })
        .unwrap();
    assert_eq!(taken, ["ctrl-tab", "ctrl-shift-tab"], "only the next and the previous workspace");
}

/// The same, end to end, with a real key press and a program that shows what it was given: `cat -vt` writes a tab
/// as ^I and Shift-Tab's three bytes as ^[[Z. The terminal keeps the keyboard.
#[gpui_kit::test]
fn tab_and_shift_tab_reach_the_program_in_the_terminal(cx: &mut TestAppContext) {
    let app = start(cx, |_| {});
    app.until_claude_started(cx, 1);
    app.click(cx, "term-add");
    assert!(app.until(cx, |w| w.try_find("new-shell").is_some()), "the menu opened");
    app.click(cx, "new-shell");
    assert!(app.until(cx, |w| w.try_find(("term-tab", 1usize)).is_some()));
    assert!(app.until(cx, |w| w.find("terminal").focused() == Some(true)), "the new shell has the keyboard");
    // "go" is written apart, so the word on screen is the shell's answer and not the line that was typed
    app.type_in_terminal(cx, "echo g\"\"o; cat -vt\r");
    assert!(app.until_terminal(cx, "\ngo"), "cat runs: {}", app.terminal_text(cx));

    app.press(cx, "tab");
    app.press(cx, "enter");
    assert!(app.until_terminal(cx, "^I"), "Tab came to the program: {}", app.terminal_text(cx));
    assert!(app.until(cx, |w| w.find("terminal").focused() == Some(true)), "and the keyboard stayed");

    app.press(cx, "shift-tab");
    app.press(cx, "enter");
    assert!(app.until_terminal(cx, "^[[Z"), "Shift-Tab came to the program: {}", app.terminal_text(cx));
    assert!(app.until(cx, |w| w.find("terminal").focused() == Some(true)), "and the keyboard stayed");
}
