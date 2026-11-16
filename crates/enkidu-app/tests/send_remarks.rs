//! Item 3.6: "Type 3 remarks into Claude". The blue bar at the foot of the changes types every remark of the workspace
//! into Claude's prompt as one message and never sends it: you read it there and press Enter. With the fake Claude,
//! which records every prompt that was sent to it.

mod common;

use std::path::PathBuf;
use std::time::Duration;

use common::{App, click_line, kept, line_number, said, until_kept};
use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{AppContext as _, TestAppContext};

const CODE: &str =
    "def total(items):\n    result = 0\n    for item in items:\n        result += item.price\n    return result\n";

/// A second file, new, with more lines than `CODE` so its line numbers can be told from the first file's.
const NOTES: &str = "# Notes\n\nTotals:\n\n- per order\n- per day\n- per item\n- per shop\n";

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

fn until_sent(cx: &mut TestAppContext, app: &App, count: usize) -> Vec<String> {
    for _ in 0..800 {
        cx.run_until_parked();
        if sent(app).len() >= count {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    sent(app)
}

/// The sample project with one workspace where `src/app.py` was rewritten. With `live`, its Claude (the fake) is up and
/// waits at its prompt; without, a terminal that runs nothing stands in.
fn start(cx: &mut TestAppContext, live: bool) -> (App, PathBuf, String) {
    let make = |shared: &std::sync::Arc<enkidu_core::state::Shared>, project: &str, _: &std::path::Path| {
        App::make(shared, project, "stock-fix", "new", "stock-fix", Some("development"))
    };
    let app = match live {
        true => common::start_live(cx, enkidu_testkit::shop, make),
        false => common::start_on(cx, enkidu_testkit::shop, make),
    };
    if live {
        app.until_claude_started(cx, 1);
        assert!(app.until_terminal(cx, "\n>"), "Claude's prompt");
    }
    let (_, _, folder) = app.on_screen(cx).expect("on screen");
    let folder = PathBuf::from(folder);
    std::fs::write(folder.join("src/app.py"), CODE).unwrap();
    let ws = app.frame.read_with(cx, |f, _| f.current_workspace().unwrap().id.clone());
    assert!(app.until(cx, |w| line_number(w, "Line 5, after").is_some()), "the diff never showed");
    (app, folder, ws)
}

/// Writes a remark on a line through the diff, as you would; `count` is how many the workspace has then.
fn remark_on(cx: &mut TestAppContext, app: &App, ws: &str, line: &str, words: &str, count: usize) {
    click_line(cx, app, line);
    assert!(app.until(cx, |w| w.try_find("remark-composer").is_some()), "no composer on {line}");
    app.typed(cx, words);
    app.click(cx, "save-remark");
    assert_eq!(until_kept(cx, app, ws, count).len(), count, "the remark on {line} was never saved");
    assert!(app.until(cx, |w| w.try_find("remark-composer").is_none()), "the composer stayed");
}

/// What the last toast says: its title and the line under it.
fn toast(cx: &mut TestAppContext, app: &App) -> (Option<String>, Option<String>) {
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        (said(w, "toast"), said(w, "toast-detail"))
    })
    .unwrap()
}

/// Types the remarks into Claude's prompt without pressing Enter, gives Claude the keyboard, then clears them.
#[gpui_kit::test]
fn types_the_remarks_into_claudes_prompt_without_pressing_enter_gives_claude_the_keyboard_then_clears_them(
    cx: &mut TestAppContext,
) {
    let (app, folder, ws) = start(cx, true);
    // nothing to send: no bar
    cx.update_window(app.window, |_, w, _| assert!(w.try_find("review-bar").is_none())).unwrap();
    remark_on(cx, &app, &ws, "Line 4, after", "Sum the prices with sum()", 1);
    assert!(app.until(cx, |w| said(w, "review-bar").as_deref() == Some("1 remark not sent yet")));
    cx.update_window(app.window, |_, w, _| {
        assert_eq!(said(w, "send-remarks").as_deref(), Some("Type 1 remark into Claude"));
    })
    .unwrap();
    // a remark on a second file
    // (in a folder that stands before src, so its diff is the first one: the files are stacked as the tree of changed
    // files lists them)
    std::fs::create_dir_all(folder.join("docs")).unwrap();
    std::fs::write(folder.join("docs/notes.md"), NOTES).unwrap();
    assert!(app.until(cx, |w| line_number(w, "Line 7, after").is_some()), "the second file's diff never showed");
    remark_on(cx, &app, &ws, "Line 7, after", "Per item of what?", 2);
    assert!(app.until(cx, |w| said(w, "review-bar").as_deref() == Some("2 remarks on 2 files not sent yet")));
    let written = kept(&app, &ws);

    // a note that goes with them
    app.click(cx, "add-note");
    assert!(app.until(cx, |w| w.try_find("remarks-note").is_some() && w.try_find("add-note").is_none()));
    app.typed(cx, "Then run the tests");
    app.click(cx, "send-remarks");

    // one message in Claude's prompt, and Claude's terminal has the keyboard: Enter is right there
    assert!(app.until_terminal(cx, "Also: Then run the tests"), "not in the prompt: {}", app.terminal_text(cx));
    assert!(app.until(cx, |w| w.find("terminal").focused() == Some(true)), "the keyboard isn't on Claude");
    // they are Claude's now: gone from the database, the bar, the tab and the status line
    assert!(until_kept(cx, &app, &ws, 0).is_empty(), "the remarks were not cleared");
    assert!(app.until(cx, |w| w.try_find("review-bar").is_none()), "the bar stayed");
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("status-remarks").is_none());
        assert_ne!(said(w, ("tab-words", 0usize)).as_deref(), Some("2 remarks not sent yet"));
        assert!(w.try_find("remarks-note").is_none(), "the note went with them");
    })
    .unwrap();
    assert_eq!(
        toast(cx, &app),
        (Some("2 remarks in Claude's prompt".into()), Some("Press Enter there to send them.".into()))
    );
    // and nothing was sent: no Enter was pressed for you
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(sent(&app), Vec::<String>::new(), "nothing was sent");

    // you press Enter: Claude has them, as one message
    app.press(cx, "enter");
    let got = until_sent(cx, &app, 1);
    let want = enkidu_model::remarks::message(&written, Some("Then run the tests"));
    assert_eq!(got.as_slice(), std::slice::from_ref(&want), "what Claude got");
    assert!(
        want.starts_with(
            "Review remarks on the code in this workspace. Please address each one.\n\n1. docs/notes.md:7\n"
        )
    );
    assert!(want.contains("\n\n2. src/app.py:4\n   >         result += item.price\n   Sum the prices with sum()\n\n"));
    assert!(want.ends_with("\n\nAlso: Then run the tests"), "{want}");
}

/// Starts Claude when it isn't running, with the remarks to type into its prompt once it is up.
#[gpui_kit::test]
fn starts_claude_when_it_isnt_running_with_the_remarks_to_type_into_its_prompt_once_it_is_up(cx: &mut TestAppContext) {
    let (app, _, ws) = start(cx, true);
    remark_on(cx, &app, &ws, "Line 4, after", "Sum the prices with sum()", 1);
    // Claude ends
    app.type_in_terminal(cx, "exit\r");
    assert!(app.until(cx, |w| w.try_find("restart-terminal").is_some()), "Claude never ended");
    // ⌘↩ with the keyboard in the changes (it went back to the diff when the remark was saved)
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        assert_ne!(w.find("terminal").focused(), Some(true));
    })
    .unwrap();
    app.press(cx, "cmd-enter");
    let started = app.until_claude_started(cx, 2);
    let argv: Vec<&str> = started["argv"].as_array().unwrap().iter().map(|a| a.as_str().unwrap()).collect();
    assert!(argv.iter().all(|a| !a.contains("Sum the prices")), "never a first message, that would send it: {argv:?}");
    // once it is up, the remarks are in its prompt, unsent, and they are no longer kept here
    assert!(app.until_terminal(cx, "Sum the prices with sum()"), "{}", app.terminal_text(cx));
    assert!(until_kept(cx, &app, &ws, 0).is_empty(), "the remarks were not cleared");
    assert_eq!(toast(cx, &app).0.as_deref(), Some("1 remark in Claude's prompt"));
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(sent(&app), Vec::<String>::new(), "nothing was sent");
    assert!(app.until(cx, |w| w.find("terminal").focused() == Some(true)), "the keyboard isn't on Claude");
    app.press(cx, "enter");
    let got = until_sent(cx, &app, 1);
    assert_eq!(got.len(), 1, "Claude never got them");
    assert!(
        got[0].contains("1. src/app.py:4\n   >         result += item.price\n   Sum the prices with sum()"),
        "{got:?}"
    );
}

/// When Claude can't start, nothing is lost: the remarks stay, and a toast says why.
#[gpui_kit::test]
fn the_remarks_stay_when_claude_cant_start(cx: &mut TestAppContext) {
    let (app, _, ws) = start(cx, true);
    remark_on(cx, &app, &ws, "Line 4, after", "Sum the prices with sum()", 1);
    app.type_in_terminal(cx, "exit\r");
    assert!(app.until(cx, |w| w.try_find("restart-terminal").is_some()), "Claude never ended");
    // the claude command in Settings names nothing
    app.shared.user.lock().unwrap().claude.command = "no-such-claude".into();
    app.click(cx, "send-remarks");
    assert!(app.until(cx, |w| said(w, "toast").as_deref() == Some("Couldn't send the remarks")), "no toast");
    assert_eq!(
        toast(cx, &app).1.as_deref(),
        Some("Can't find \"no-such-claude\". Set the claude command in Settings.")
    );
    cx.run_until_parked();
    assert_eq!(kept(&app, &ws).len(), 1, "the remark is still kept");
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        assert_eq!(said(w, "review-bar").as_deref(), Some("1 remark not sent yet"));
        // and the button is ready again
        assert_eq!(said(w, "send-remarks").as_deref(), Some("Type 1 remark into Claude"));
    })
    .unwrap();
    assert_eq!(app.claude_starts().len(), 1, "no second Claude started");
}

/// Discard asks first; a note closes with Escape while it has no words.
#[gpui_kit::test]
fn discard_asks_first_and_an_empty_note_closes_with_escape(cx: &mut TestAppContext) {
    let (app, _, ws) = start(cx, false);
    remark_on(cx, &app, &ws, "Line 2, after", "Start from zero?", 1);
    remark_on(cx, &app, &ws, "Line 4, after", "Sum the prices with sum()", 2);
    assert!(app.until(cx, |w| said(w, "review-bar").as_deref() == Some("2 remarks not sent yet")));

    // Discard: Keep or Delete 2, in place of the other buttons
    app.click(cx, "discard-remarks");
    assert!(app.until(cx, |w| said(w, "confirm-discard").as_deref() == Some("Delete 2")));
    cx.update_window(app.window, |_, w, _| {
        assert!(w.try_find("send-remarks").is_none() && w.try_find("add-note").is_none());
        assert!(w.try_find("discard-remarks").is_none());
    })
    .unwrap();
    app.click(cx, "keep-remarks");
    assert!(app.until(cx, |w| w.try_find("send-remarks").is_some() && w.try_find("confirm-discard").is_none()));
    assert_eq!(kept(&app, &ws).len(), 2, "Keep keeps them");

    // a note: Escape leaves it open while it has words, and closes it once it has none
    app.click(cx, "add-note");
    assert!(app.until(cx, |w| w.try_find("remarks-note").is_some() && w.try_find("add-note").is_none()));
    app.typed(cx, "x");
    app.press(cx, "escape");
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("remarks-note").is_some(), "a note with words closed on Escape");
    })
    .unwrap();
    app.press(cx, "backspace");
    app.press(cx, "escape");
    assert!(app.until(cx, |w| w.try_find("remarks-note").is_none() && w.try_find("add-note").is_some()));

    // Delete 2: gone from the database and the bar with them; the changes stay
    app.click(cx, "discard-remarks");
    assert!(app.until(cx, |w| w.try_find("confirm-discard").is_some()));
    app.click(cx, "confirm-discard");
    assert!(until_kept(cx, &app, &ws, 0).is_empty(), "the remarks were not deleted");
    assert!(app.until(cx, |w| w.try_find("review-bar").is_none() && w.try_find("island-changes").is_some()));
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("file-remarks").is_none() && w.try_find("status-remarks").is_none());
    })
    .unwrap();
}

/// The palette (⌘K) has the same action while remarks wait on the workspace on screen.
#[gpui_kit::test]
fn the_palette_types_the_remarks_into_claude_too(cx: &mut TestAppContext) {
    let (app, _, ws) = start(cx, true);
    // without remarks the palette doesn't offer it
    app.press(cx, "cmd-k");
    assert!(app.until(cx, |w| w.try_find("command-palette").is_some()), "the palette never opened");
    app.typed(cx, "remark");
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        assert!(w.try_find_by_label("Type 1 remark into Claude").is_none());
    })
    .unwrap();
    app.press(cx, "escape");
    assert!(app.until(cx, |w| w.try_find("command-palette").is_none()));

    remark_on(cx, &app, &ws, "Line 4, after", "Sum the prices with sum()", 1);
    app.press(cx, "cmd-k");
    assert!(app.until(cx, |w| w.try_find("command-palette").is_some()), "the palette never opened");
    app.typed(cx, "remark");
    assert!(
        app.until(cx, |w| said(w, ("palette-row", 0usize)).as_deref() == Some("Type 1 remark into Claude")),
        "the palette doesn't offer it"
    );
    app.press(cx, "enter");
    assert!(app.until(cx, |w| w.try_find("command-palette").is_none()), "the palette stayed");
    assert!(app.until_terminal(cx, "Sum the prices with sum()"), "not in the prompt: {}", app.terminal_text(cx));
    assert!(until_kept(cx, &app, &ws, 0).is_empty(), "the remarks were not cleared");
    assert_eq!(toast(cx, &app).0.as_deref(), Some("1 remark in Claude's prompt"));
    assert!(app.until(cx, |w| w.find("terminal").focused() == Some(true)), "the keyboard isn't on Claude");
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(sent(&app), Vec::<String>::new(), "nothing was sent");
}
