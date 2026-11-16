//! Item 2.15: `enkidu`, run inside Claude as its Bash tool would run it, reaches the app: it knows its session and
//! workspace from the terminal's own variables, and status, link, note and unlink read and correct this app's index.
//! The fake Claude runs any line that starts with `enkidu` in its own environment; the `enkidu` on its PATH is the
//! app's own, and what it changes shows in this test's own database.

mod common;

use common::App;
use gpui_kit::TestAppContext;

#[gpui_kit::test]
fn enkidu_inside_claude_reads_and_corrects_the_new_apps_index(cx: &mut TestAppContext) {
    let app = common::start_live(
        cx,
        |home| enkidu_testkit::repo_with_origin(home, "shop"),
        |shared, project, _| App::make(shared, project, "stock-fix", "new", "stock-fix", Some("main")),
    );
    app.until_claude_started(cx, 1);
    assert!(app.until_terminal(cx, "\n>"), "Claude's prompt");
    let ws = app.frame.read_with(cx, |f, _| f.current_workspace().cloned()).expect("on screen");
    let session = format!("session:{}", ws.session_id);
    let links = |app: &App| -> Vec<(String, String, String)> {
        app.shared.db.work_links().unwrap().into_iter().map(|l| (l.a, l.b, l.how)).collect()
    };

    // status: enkidu knows where it runs
    app.type_in_terminal(cx, "enkidu status --json\r");
    assert!(app.until_terminal(cx, &format!("\"session\": \"{}\"", ws.session_id)), "{}", app.terminal_text(cx));
    assert!(app.terminal_text(cx).contains(&format!("\"workspace\": \"{}\"", ws.id)));

    // link: this session works on a ticket, said by Claude
    assert!(links(&app).is_empty());
    app.type_in_terminal(cx, "enkidu link ticket SH-21533 --json\r");
    assert!(app.until_terminal(cx, "\"linked\": \"ticket:SH-21533\""), "{}", app.terminal_text(cx));
    assert_eq!(links(&app), [("ticket:SH-21533".to_string(), session.clone(), "claude".to_string())]);

    // note: a line in the ticket's timeline
    app.type_in_terminal(cx, "enkidu note \"checked the totals\" --ticket SH-21533 --json\r");
    assert!(app.until_terminal(cx, "\"noted\": \"SH-21533\""), "{}", app.terminal_text(cx));
    let events = app.shared.db.work_events(50).unwrap();
    assert!(
        events.iter().any(|e| e.subject == "SH-21533" && e.kind == "note" && e.text == "checked the totals"),
        "{events:?}"
    );

    // unlink: a wrong link is undone
    app.type_in_terminal(cx, "enkidu unlink ticket SH-21533 --json\r");
    assert!(app.until_terminal(cx, "\"unlinked\": \"ticket:SH-21533\""), "{}", app.terminal_text(cx));
    assert!(links(&app).is_empty());

    // the enkidu it ran is the app's own
    app.type_in_terminal(cx, "enkidu --help\r");
    assert!(app.until_terminal(cx, "Nothing here writes to GitHub or Jira."));
}
