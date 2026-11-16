//! Item 3.13: the whole-app runs of phase 3. The real wiring (the core, its folder watch, the hook server, the stores,
//! the window) on the sample project (`enkidu_testkit::shop`) with the fake Claude, driven through the window. Each run
//! starts from its own copy, so the runs need no order: what a run needs to find there (Claude's first edits, the
//! second workspace) is made at its start. The eighth run, the merge, is in `merge.rs`.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{App, click_line, said, until_kept};
use enkidu_core::state::Shared;
use enkidu_testkit::git;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Pixels, SharedString, TestAppContext, Window};

/// What every run starts from: the project copies `.env` into every workspace and runs its setup script there (it
/// writes `setup-ran.txt`, a new file, so every workspace starts with one change). "stock-fix" is on a new branch from
/// development and, with `second`, "remote-only" on origin's branch. Every workspace's Claude is the fake one, waiting
/// at its prompt; stock-fix is on screen. Gives its folder.
fn start(cx: &mut TestAppContext, second: bool) -> (App, PathBuf) {
    let app = common::start_live(cx, enkidu_testkit::shop, |shared: &Arc<Shared>, project: &str, home: &Path| {
        let mut settings = shared.project_settings(project).unwrap();
        settings.copy_files = vec![".env".into(), ".claude/settings.local.json".into()];
        settings.setup_script = "echo \"$ENKIDU_BRANCH\" > setup-ran.txt".into();
        // Live's own commands leave a line in a file, and Claude in the Live workspace is told a word to look for
        let state = home.join("live-state.txt");
        settings.live.on = format!("echo on {{path}} > '{}'", state.display());
        settings.live.off = format!("echo off > '{}'", state.display());
        settings.live.message = "LIVE-MESSAGE-E2E".into();
        shared.save_project_settings(project, &settings).unwrap();
        App::make(shared, project, "stock-fix", "new", "stock-fix", Some("development"));
        if second {
            App::make(shared, project, "remote-only", "existing", "feature/remote-only", None);
        }
    });
    app.until_claude_started(cx, if second { 2 } else { 1 });
    app.click(cx, ("tab", 0usize));
    assert!(app.until_frame(cx, |f| f.current_workspace().is_some_and(|w| w.name == "stock-fix")));
    assert!(app.until_terminal(cx, "\n>"), "Claude's prompt");
    let (_, folder) = workspace(cx, &app, "stock-fix");
    assert!(
        app.until(cx, |w| changed(w) == ["setup-ran.txt"]),
        "the setup script's file: {:?}",
        look(cx, &app, |w| changed(w))
    );
    (app, folder)
}

/// A workspace's id and its folder.
fn workspace(cx: &mut TestAppContext, app: &App, name: &str) -> (String, PathBuf) {
    app.frame
        .read_with(cx, |f, _| f.workspaces.iter().find(|w| w.name == name).map(|w| (w.id.clone(), w.path.clone())))
        .map(|(id, path)| (id, PathBuf::from(path)))
        .unwrap_or_else(|| panic!("no workspace {name}"))
}

fn look<R>(cx: &mut TestAppContext, app: &App, at: impl FnOnce(&mut Window) -> R) -> R {
    cx.run_until_parked();
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        at(w)
    })
    .unwrap()
}

/// The rows of the Changed files panel: each changed file's path.
fn changed(window: &Window) -> Vec<String> {
    rows(window, "change-row")
}

fn rows(window: &Window, id: &'static str) -> Vec<String> {
    (0..40usize)
        .filter_map(|ix| window.try_find((id, ix)))
        .map(|row| row.label().unwrap_or_default().to_string())
        .collect()
}

/// Clicks a changed file's row in the Changed files panel.
fn click_changed(cx: &mut TestAppContext, app: &App, path: &str) {
    let ix = look(cx, app, |w| {
        (0..40usize)
            .find(|ix| w.try_find(("change-row", *ix)).is_some_and(|row| row.label() == Some(path)))
            .unwrap_or_else(|| panic!("{path} isn't among the changed files: {:?}", rows(w, "change-row")))
    });
    app.click(cx, ("change-row", ix));
}

/// The Files panel's rows in view, top to bottom: "dir path" for a folder (with "open" when it is), the path for a
/// file.
fn files(window: &Window) -> Vec<String> {
    let mut found: Vec<(Pixels, String)> = Vec::new();
    for ix in 0..200usize {
        if let Some(row) = window.try_find(("folder-row", ix)) {
            let open = if row.description() == Some("open") { " open" } else { "" };
            found.push((row.bounds().top(), format!("dir {}{open}", row.label().unwrap_or_default())));
        }
        if let Some(row) = window.try_find(("file-row", ix)) {
            found.push((row.bounds().top(), row.label().unwrap_or_default().to_string()));
        }
    }
    found.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    found.into_iter().map(|(_, said)| said).collect()
}

/// Clicks the Files panel's row that says `label` (a file's path, or a folder's).
fn click_file_row(cx: &mut TestAppContext, app: &App, label: &str) {
    let id = look(cx, app, |w| {
        (0..200usize)
            .flat_map(|ix| [("folder-row", ix), ("file-row", ix)])
            .find(|id| w.try_find(*id).is_some_and(|row| row.label() == Some(label)))
            .unwrap_or_else(|| panic!("no row says {label:?}; the rows: {:?}", files(w)))
    });
    app.click(cx, id);
    cx.run_until_parked();
}

/// What every element with `id` says, top to bottom.
fn all(window: &Window, id: &'static str) -> Vec<String> {
    window.find_all(id).iter().map(|found| found.label().unwrap_or_default().to_string()).collect()
}

/// Whether a line that says `text` is on screen as the line `which` ("Line 2, after, added"; "Line 2" in a file).
fn has_line(window: &Window, text: &str, which: &str) -> bool {
    window.find_all("source").iter().any(|line| line.label() == Some(text) && line.description() == Some(which))
}

/// How the workspace is laid out, from where its two islands are: "stack" (the changes above the terminal), "side"
/// (beside it, the terminal on the left), "terminal" or "review" (one of them alone).
fn mode(window: &Window) -> &'static str {
    let changes = window.try_find("island-changes").map(|island| island.bounds());
    let terminal = window.try_find("island-claude").map(|island| island.bounds());
    match (changes, terminal) {
        (Some(changes), Some(terminal)) if changes.bottom() <= terminal.top() => "stack",
        (Some(changes), Some(terminal)) if terminal.right() <= changes.left() => "side",
        (Some(_), Some(_)) => "one over the other",
        (None, Some(_)) => "terminal",
        (Some(_), None) => "review",
        (None, None) => "nothing",
    }
}

/// What every fake Claude was sent, oldest first: its session and the prompt.
fn sent(app: &App) -> Vec<(String, String)> {
    let log = format!("{}.prompts", app.claude_log.display());
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter_map(|said| Some((said["sessionId"].as_str()?.to_string(), said["prompt"].as_str()?.to_string())))
        .collect()
}

fn until_sent(cx: &mut TestAppContext, app: &App, count: usize) -> Vec<(String, String)> {
    for _ in 0..800 {
        cx.run_until_parked();
        if sent(app).len() >= count {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    sent(app)
}

/// Waits until the Claude on screen has written `words` as a line of `fake-output.txt` (what the fake does with a
/// prompt) and its turn is over.
fn turn_over(cx: &mut TestAppContext, app: &App, folder: &Path, words: &str) {
    let file = folder.join("fake-output.txt");
    let wrote = |_: &enkidu_model::frame::Frame| {
        std::fs::read_to_string(&file).unwrap_or_default().lines().any(|line| line == words)
    };
    assert!(app.until_frame(cx, wrote), "Claude never wrote {words:?}; its terminal:\n{}", app.terminal_text(cx));
    // it was working when it wrote; its Stop hook says when the turn is over
    assert!(
        app.until_frame(cx, |f| f.current_workspace().is_some_and(|w| f.status(&w.id) == "done")),
        "Claude's turn never ended; its terminal:\n{}",
        app.terminal_text(cx)
    );
}

/// Types a prompt into the Claude on screen and presses Enter, as you would, and waits for its turn to end.
fn prompt(cx: &mut TestAppContext, app: &App, folder: &Path, words: &str) {
    app.type_in_terminal(cx, &format!("{words}\r"));
    turn_over(cx, app, folder, words);
}

/// What the app's store has as changed in a workspace, as the core's watcher told it. (Asking the core itself would
/// make the list known there, and the watcher only sends news.)
fn in_store(cx: &mut TestAppContext, ws: &str) -> Vec<String> {
    cx.run_until_parked();
    cx.update(|cx| {
        let lists = enkidu_app::stores::Stores::changes(cx);
        lists.read(cx).of(ws).map(|list| list.iter().map(|change| change.path.clone()).collect()).unwrap_or_default()
    })
}

/// Updates the changes panel as files change, and empties it after a commit.
#[gpui_kit::test]
fn updates_the_changes_panel_as_files_change_and_empties_it_after_a_commit(cx: &mut TestAppContext) {
    let (app, folder) = start(cx, false);
    // Claude's first edit: it asked first, and went on after "y"
    app.type_in_terminal(cx, "ask\r");
    assert!(app.until_terminal(cx, "Allow Bash(make test)?"), "Claude never asked: {}", app.terminal_text(cx));
    app.type_in_terminal(cx, "y\r");
    // its file turns up in the Changed files panel, and a click brings its diff
    assert!(
        app.until(cx, |w| changed(w) == ["fake-output.txt", "setup-ran.txt"]),
        "Claude's file never turned up: {:?}",
        look(cx, &app, |w| changed(w))
    );
    click_changed(cx, &app, "fake-output.txt");
    assert!(app.until(cx, |w| has_line(w, "ask", "Line 1, after, added")), "its diff never showed");
    turn_over(cx, &app, &folder, "ask");

    // Claude edits the file again: the diff follows
    prompt(cx, &app, &folder, "second edit");
    assert!(app.until(cx, |w| has_line(w, "second edit", "Line 2, after, added")), "the diff didn't refresh");
    look(cx, &app, |w| {
        assert!(has_line(w, "ask", "Line 1, after, added"));
        assert_eq!(said(w, "status-changes").as_deref(), Some("2 changed files"));
        assert_eq!(said(w, "rail-count").as_deref(), Some("2"));
        assert_eq!(said(w, "changes-count").as_deref(), Some("2 files"));
        assert_eq!(mode(w), "stack");
    });
    let room = look(cx, &app, |w| w.find("workspace-body").bounds());

    git(&folder, &["add", "-A"]);
    git(&folder, &["commit", "-q", "-m", "wip"]);
    // nothing left to review: the changes go away (no empty pane, no words) and the terminal gets the room
    assert!(app.until(cx, |w| w.try_find("island-changes").is_none()), "the changes stayed after the commit");
    look(cx, &app, |w| {
        assert_eq!(mode(w), "terminal");
        assert_eq!(w.find("island-claude").bounds(), room, "the terminal has the room");
        assert!(changed(w).is_empty(), "the panel still lists {:?}", changed(w));
        assert!(w.try_find("status-changes").is_none() && w.try_find("status-counts").is_none());
        assert!(w.try_find("rail-count").is_none() && w.try_find("changes-count").is_none(), "a count of nothing");
        assert_ne!(said(w, ("tab-words", 0usize)).as_deref(), Some("2 changed files"));
    });
}

/// Keeps every workspace's changes fresh in the background.
#[gpui_kit::test]
fn keeps_every_workspaces_changes_fresh_in_the_background(cx: &mut TestAppContext) {
    let (app, _) = start(cx, true);
    let (remote, remote_folder) = workspace(cx, &app, "remote-only");
    // remote-only is not on screen; its list still learns about the new file
    assert_eq!(app.on_screen(cx).map(|(name, _, _)| name).as_deref(), Some("stock-fix"));
    std::fs::write(remote_folder.join("notes.md"), "background edit\n").unwrap();
    let mut seen = Vec::new();
    for _ in 0..1000 {
        seen = in_store(cx, &remote);
        if seen.iter().any(|path| path == "notes.md") {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(seen.iter().any(|path| path == "notes.md"), "the hidden workspace's changes never arrived: {seen:?}");
    // its tab counts both new files (the setup script's and this one) without ever being opened
    assert!(
        app.until(cx, |w| said(w, ("tab-words", 1usize)).as_deref() == Some("2 changed files")),
        "its tab says {:?}",
        look(cx, &app, |w| said(w, ("tab-words", 1usize)))
    );
    assert_eq!(app.on_screen(cx).map(|(name, _, _)| name).as_deref(), Some("stock-fix"), "it was never opened");
    // and the workspace on screen still shows its own
    look(cx, &app, |w| assert_eq!(changed(w), ["setup-ran.txt"]));
}

/// Clicks a tab and draws until that workspace is on screen with `row` among its changed files: how long that took.
fn switch(cx: &mut TestAppContext, app: &App, tab: usize, name: &str, row: &str) -> Duration {
    look(cx, app, |_| ());
    let clicked = Instant::now();
    cx.update_window(app.window, |_, w, cx| w.click(("tab", tab), cx)).unwrap();
    loop {
        let listed = cx
            .update_window(app.window, |_, w, cx| {
                w.render_frame(cx);
                w.try_find("island-claude").is_some() && changed(w).iter().any(|path| path == row)
            })
            .unwrap();
        let on = app.frame.read_with(cx, |f, _| f.current_workspace().is_some_and(|w| w.name == name));
        if listed && on {
            return clicked.elapsed();
        }
        assert!(clicked.elapsed() < Duration::from_secs(10), "{name} never came on screen with {row} listed");
        cx.run_until_parked();
    }
}

/// Switches workspaces and shows new edits fast.
#[gpui_kit::test]
fn switches_workspaces_and_shows_new_edits_fast(cx: &mut TestAppContext) {
    let (app, _) = start(cx, true);
    let (_, remote_folder) = workspace(cx, &app, "remote-only");
    std::fs::write(remote_folder.join("notes.md"), "background edit\n").unwrap();
    let terminal = |cx: &mut TestAppContext| {
        app.shell.read_with(cx, |shell, cx| shell.terminal_on_screen(cx).map(|view| view.entity_id()))
    };
    // open both once, as you would during a day
    let stock_terminal = terminal(cx).expect("stock-fix's terminal");
    app.click(cx, ("tab", 1usize));
    assert!(app.until(cx, |w| changed(w) == ["notes.md", "setup-ran.txt"]), "{:?}", look(cx, &app, |w| changed(w)));
    assert!(app.until(cx, |w| has_line(w, "background edit", "Line 1, after, added")));
    assert!(app.until_terminal(cx, "\n>"), "remote-only's Claude never came up");
    let remote_terminal = terminal(cx).expect("remote-only's terminal");
    assert_ne!(stock_terminal, remote_terminal);
    app.click(cx, ("tab", 0usize));
    assert!(app.until(cx, |w| changed(w) == ["setup-ran.txt"]));
    std::thread::sleep(Duration::from_millis(300));
    let started = app.claude_starts().len();

    // from the click to the other workspace on screen with its changes listed: the best of three, as a busy Mac
    // makes one timing say little
    let mut switch_time = Duration::MAX;
    for _ in 0..3 {
        switch_time = switch_time.min(switch(cx, &app, 1, "remote-only", "notes.md"));
        // a switch starts no terminal and makes none again: each workspace keeps its own while it is off screen
        assert_eq!(terminal(cx), Some(remote_terminal));
        assert!(app.terminal_text(cx).contains("Fake Claude"), "remote-only's terminal lost its screen");
        switch(cx, &app, 0, "stock-fix", "setup-ran.txt");
        assert_eq!(terminal(cx), Some(stock_terminal));
    }
    switch(cx, &app, 1, "remote-only", "notes.md");
    assert_eq!(app.claude_starts().len(), started, "a switch started a Claude");

    // an edit in the workspace on screen: from writing the file to seeing it in the list
    let written = Instant::now();
    std::fs::write(remote_folder.join("speed.txt"), "fast\n").unwrap();
    loop {
        if look(cx, &app, |w| changed(w).iter().any(|path| path == "speed.txt")) {
            break;
        }
        assert!(written.elapsed() < Duration::from_secs(10), "speed.txt was never listed");
        std::thread::sleep(Duration::from_millis(2));
    }
    let edit_time = written.elapsed();

    // opening that file's diff: it was read in the background, so it is already there or nearly
    let opened = Instant::now();
    click_changed(cx, &app, "speed.txt");
    loop {
        if look(cx, &app, |w| has_line(w, "fast", "Line 1, after, added")) {
            break;
        }
        assert!(opened.elapsed() < Duration::from_secs(10), "speed.txt's diff never showed");
        std::thread::sleep(Duration::from_millis(2));
    }
    let diff_time = opened.elapsed();
    println!(
        "ENKIDU_SPEED switch={:.1}ms edit={}ms diff={}ms",
        switch_time.as_secs_f64() * 1000.,
        edit_time.as_millis(),
        diff_time.as_millis()
    );
    assert!(switch_time < Duration::from_millis(150), "a switch took {switch_time:?}");
    assert!(edit_time < Duration::from_millis(1500), "an edit took {edit_time:?} to be listed");
}

/// Reviews a change side by side and sends a remark to Claude's terminal.
#[gpui_kit::test]
fn reviews_a_change_side_by_side_and_sends_a_remark_to_claudes_terminal(cx: &mut TestAppContext) {
    let (app, folder) = start(cx, false);
    let (ws, _) = workspace(cx, &app, "stock-fix");
    // as the runs before this one left it: Claude's two edits are committed, then it edits the file a third time
    prompt(cx, &app, &folder, "first edit");
    prompt(cx, &app, &folder, "second edit");
    git(&folder, &["add", "-A"]);
    git(&folder, &["commit", "-q", "-m", "wip"]);
    assert!(app.until(cx, |w| w.try_find("island-changes").is_none()), "the commit wasn't seen");
    prompt(cx, &app, &folder, "third edit");
    assert!(app.until(cx, |w| changed(w) == ["fake-output.txt"]), "{:?}", look(cx, &app, |w| changed(w)));

    // the changes in full size, in one column, then before and after side by side
    app.click(cx, "maximize-review");
    assert!(app.until(cx, |w| mode(w) == "review"), "the changes never took the workspace");
    app.click(cx, "diff-unified");
    assert!(
        app.until(cx, |w| w.try_find("split-before").is_none() && has_line(w, "third edit", "Line 3, after, added"))
    );
    app.click(cx, "diff-split");
    assert!(app.until(cx, |w| w.try_find("split-before").is_some() && w.try_find("split-after").is_some()));
    let text = std::fs::read_to_string(folder.join("fake-output.txt")).unwrap();
    let lines: Vec<&str> = text.trim_end().split('\n').collect();
    let last = lines.len();
    assert_eq!((last, lines[last - 1]), (3, "third edit"));
    look(cx, &app, |w| {
        // the line that was added faces nothing on the left, and the two before it are on both sides
        assert!(has_line(w, "third edit", "Line 3, after, added"));
        assert!(has_line(w, "second edit", "Line 2, before") && has_line(w, "second edit", "Line 2, after"));
    });

    // a remark on the last line of the new side
    click_line(cx, &app, &format!("Line {last}, after"));
    assert!(app.until(cx, |w| w.try_find("remark-composer").is_some()), "no composer on the last line");
    app.typed(cx, "Say why this edit was needed");
    app.click(cx, "save-remark");
    // the remark is kept by the database until it is sent
    let saved = until_kept(cx, &app, &ws, 1);
    assert_eq!(saved.len(), 1, "the remark was never saved");
    assert_eq!(
        (saved[0].path.as_str(), saved[0].start_line as usize, saved[0].snippet.as_str()),
        ("fake-output.txt", last, lines[last - 1])
    );
    let card = SharedString::from(format!("remark-{}", saved[0].id));
    assert!(app.until(cx, |w| w.try_find(card.clone()).is_some()), "its card never showed");

    let before = sent(&app).len();
    assert_eq!(before, 3, "the three edits were asked for");
    app.click(cx, "send-remarks");
    // in Claude's prompt, not sent: the keyboard is on Claude, and you press Enter
    assert!(
        app.until_terminal(cx, "Say why this edit was needed"),
        "the remarks never reached Claude's prompt: {}",
        app.terminal_text(cx)
    );
    assert!(app.until(cx, |w| w.find("terminal").focused() == Some(true)), "the keyboard isn't on Claude");
    std::thread::sleep(Duration::from_millis(600));
    cx.run_until_parked();
    assert_eq!(sent(&app).len(), before, "something was sent before Enter");
    app.press(cx, "enter");
    let got = until_sent(cx, &app, before + 1);
    assert!(got.len() > before, "Claude never got the remarks");
    let message = &got.last().unwrap().1;
    assert!(message.contains("Review remarks on the code in this workspace"), "{message}");
    assert!(message.contains(&format!("1. fake-output.txt:{last}")), "{message}");
    assert!(message.contains(&format!("   > {}", lines[last - 1])), "{message}");
    assert!(message.contains("   Say why this edit was needed"), "{message}");
    // sent as one prompt, not line by line
    turn_over(cx, &app, &folder, "Review remarks on the code in this workspace. Please address each one.");
    assert_eq!(sent(&app).len(), before + 1);
    assert!(app.until(cx, |w| w.try_find("review-bar").is_none()), "the bar stayed");
    assert!(until_kept(cx, &app, &ws, 0).is_empty(), "the remark is still in the database");
    // back to the terminal so you can watch Claude work on them
    look(cx, &app, |w| {
        assert_eq!(said(w, "maximize-review").as_deref(), Some("Review in full width"));
        assert_eq!(mode(w), "stack");
    });
}

/// Switches layouts without restarting the terminal.
#[gpui_kit::test]
fn switches_layouts_without_restarting_the_terminal(cx: &mut TestAppContext) {
    let (app, folder) = start(cx, false);
    prompt(cx, &app, &folder, "before the switches");
    assert_eq!(look(cx, &app, |w| mode(w)), "stack");
    // the terminal as it is now: after the switches it must be the same one (a new one would lose Claude's screen)
    let terminal = |cx: &mut TestAppContext| {
        let view = app.shell.read_with(cx, |shell, cx| shell.terminal_on_screen(cx)).expect("a terminal");
        (view.entity_id(), view.read_with(cx, |view, _| Arc::as_ptr(view.terminal())))
    };
    let (kept, started) = (terminal(cx), app.claude_starts().len());
    assert_eq!(started, 1);

    app.click(cx, "layout-side");
    assert!(app.until(cx, |w| mode(w) == "side"), "the switch's first choice: {}", look(cx, &app, |w| mode(w)));
    app.click(cx, "layout-terminal");
    assert!(app.until(cx, |w| mode(w) == "terminal"), "the switch's third choice: {}", look(cx, &app, |w| mode(w)));
    app.press(cx, "alt-cmd-b");
    assert!(app.until(cx, |w| mode(w) == "side"), "⌥⌘B: {}", look(cx, &app, |w| mode(w)));
    app.press(cx, "alt-cmd-l");
    assert!(app.until(cx, |w| mode(w) == "stack"), "⌥⌘L: {}", look(cx, &app, |w| mode(w)));

    // the same terminal with its screen, and Claude was not started again
    assert_eq!(terminal(cx), kept, "the terminal was made again");
    assert_eq!(app.claude_starts().len(), started, "Claude was started again");
    assert!(app.terminal_text(cx).contains("before the switches"), "Claude's screen was lost");
    app.type_in_terminal(cx, "after the switches\r");
    assert!(app.until_terminal(cx, "after the switches"));
    turn_over(cx, &app, &folder, "after the switches");
    // and it is the same conversation that got both
    let got = sent(&app);
    assert_eq!(
        got.iter().map(|(_, prompt)| prompt.as_str()).collect::<Vec<_>>(),
        ["before the switches", "after the switches"]
    );
    assert_eq!(got[0].0, got[1].0, "another session answered");
}

/// Browses the project's files from the right edge and opens one read-only.
#[gpui_kit::test]
fn browses_the_projects_files_from_the_right_edge_and_opens_one_read_only(cx: &mut TestAppContext) {
    let (app, folder) = start(cx, false);
    app.click(cx, "rail-files");
    assert!(app.until(cx, |w| said(w, "panel-band").as_deref() == Some("Files")), "Files never opened");
    assert!(app.until(cx, |w| files(w).contains(&"dir src".to_string())), "{:?}", look(cx, &app, |w| files(w)));
    // what git sees: the ignored .env stays out, though the project copied it into the workspace
    assert_eq!(std::fs::read_to_string(folder.join(".env")).unwrap(), "SECRET=e2e\n");
    look(cx, &app, |w| {
        assert_eq!(files(w), ["dir src", ".gitignore", "setup-ran.txt"]);
    });
    click_file_row(cx, &app, "src");
    click_file_row(cx, &app, "src/app.py");
    assert!(app.until(cx, |w| said(w, "file-view").as_deref() == Some("src/app.py")), "the file never opened");
    assert!(app.until(cx, |w| has_line(w, "def total(items):", "Line 1")), "its lines never showed");
    look(cx, &app, |w| assert!(w.try_find("show-changes").is_none(), "changes of a file that has none"));
    // read only: keys change nothing
    let original = std::fs::read_to_string(folder.join("src/app.py")).unwrap();
    app.typed(cx, "x");
    app.press(cx, "backspace");
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(folder.join("src/app.py")).unwrap(), original);

    // Claude (here: the test) edits the file: the view reads it again and offers its diff
    let edited = "def total(items):\n    return sum(items)\n\n\ndef count(items):\n    return len(items)\n";
    std::fs::write(folder.join("src/app.py"), edited).unwrap();
    assert!(app.until(cx, |w| w.try_find("show-changes").is_some()), "its changes were never offered");
    assert!(app.until(cx, |w| has_line(w, "def count(items):", "Line 5")), "the file wasn't read again");
    app.click(cx, "show-changes");
    assert!(
        app.until(cx, |w| w.try_find("file-view").is_none() && all(w, "file-head").contains(&"src/app.py".to_string())),
        "its diff never came"
    );
    assert!(app.until(cx, |w| has_line(w, "def count(items):", "Line 5, after, added")));

    // the filter finds a file by name
    app.click(cx, "files-filter");
    app.typed(cx, "app");
    assert!(app.until(cx, |w| files(w) == ["src/app.py"]), "the filter's matches: {:?}", look(cx, &app, |w| files(w)));

    // back to the changed files; the edit is there too
    app.click(cx, "rail-changes");
    assert!(app.until(cx, |w| changed(w) == ["src/app.py", "setup-ran.txt"]), "{:?}", look(cx, &app, |w| changed(w)));
    git(&folder, &["checkout", "--", "src/app.py"]);
    assert!(
        app.until(cx, |w| changed(w) == ["setup-ran.txt"]),
        "the undone edit stayed: {:?}",
        look(cx, &app, |w| changed(w))
    );
}

/// Leaves a remark on a line of an opened file and sends it to Claude with the others: another remark waits on a change
/// of Claude's, so "with the others" is checked too.
#[gpui_kit::test]
fn leaves_a_remark_on_a_line_of_an_opened_file_and_sends_it_to_claude_with_the_others(cx: &mut TestAppContext) {
    let (app, folder) = start(cx, false);
    let (ws, _) = workspace(cx, &app, "stock-fix");
    prompt(cx, &app, &folder, "first edit");
    prompt(cx, &app, &folder, "second edit");
    assert!(app.until(cx, |w| has_line(w, "second edit", "Line 2, after, added")), "Claude's edits never showed");
    click_line(cx, &app, "Line 2, after");
    assert!(app.until(cx, |w| w.try_find("remark-composer").is_some()), "no composer in the diff");
    app.typed(cx, "Why a second one?");
    app.click(cx, "save-remark");
    assert_eq!(until_kept(cx, &app, &ws, 1).len(), 1, "the remark in the diff was never saved");

    // a file without changes, opened from the Files panel: the filter's best match, with Enter
    app.click(cx, "rail-files");
    assert!(app.until(cx, |w| said(w, "panel-band").as_deref() == Some("Files")), "Files never opened");
    app.click(cx, "files-filter");
    app.typed(cx, "app");
    assert!(app.until(cx, |w| files(w) == ["src/app.py"]), "the filter's matches: {:?}", look(cx, &app, |w| files(w)));
    app.press(cx, "enter");
    assert!(app.until(cx, |w| said(w, "file-view").as_deref() == Some("src/app.py")), "Enter opened nothing");
    assert!(app.until(cx, |w| has_line(w, "def total(items):", "Line 1")), "its lines never showed");
    let text = std::fs::read_to_string(folder.join("src/app.py")).unwrap();
    let code: Vec<&str> = text.split('\n').collect();
    // pressing a line number opens a remark under that line, as in the diff
    click_line(cx, &app, "Line 2");
    assert!(app.until(cx, |w| w.try_find("remark-composer").is_some()), "no composer in the file");
    app.typed(cx, "Handle an empty list here");
    app.click(cx, "save-remark");
    let saved = until_kept(cx, &app, &ws, 2);
    assert_eq!(saved.len(), 2, "the remark in the file was never saved");
    let mine = saved.iter().find(|remark| remark.path == "src/app.py").expect("the remark on the opened file");
    assert_eq!((mine.start_line as usize, mine.snippet.as_str()), (2, code[1]));
    let card = SharedString::from(format!("remark-{}", mine.id));
    assert!(app.until(cx, |w| w.try_find(card.clone()).is_some()), "its card never showed");
    // the send bar is right there
    assert!(
        app.until(cx, |w| said(w, "review-bar").as_deref() == Some("2 remarks on 2 files not sent yet")),
        "the bar says {:?}",
        look(cx, &app, |w| said(w, "review-bar"))
    );

    // back at the changes it waits with the others: under the diffs, in the Changed files panel and in the status
    // line
    app.click(cx, "close-file");
    app.click(cx, "rail-changes");
    assert!(app.until(cx, |w| rows(w, "remark-file-row") == ["src/app.py"]), "not in the Changed files panel");
    app.click(cx, "maximize-review");
    assert!(
        app.until(cx, |w| all(w, "file-head") == ["fake-output.txt", "setup-ran.txt", "src/app.py"]),
        "not under the diffs: {:?}",
        look(cx, &app, |w| all(w, "file-head"))
    );
    assert!(app.until(cx, |w| w.try_find(card.clone()).is_some()), "its card isn't under the diffs");
    look(cx, &app, |w| {
        assert_eq!(all(w, "file-unchanged"), ["No changes"]);
        assert_eq!(said(w, "status-remarks").as_deref(), Some("2 remarks not sent"));
        assert_eq!(changed(w), ["fake-output.txt", "setup-ran.txt"]);
    });

    let before = sent(&app).len();
    app.click(cx, "send-remarks");
    assert!(
        app.until_terminal(cx, "Handle an empty list here"),
        "the remark never reached Claude's prompt: {}",
        app.terminal_text(cx)
    );
    std::thread::sleep(Duration::from_millis(600));
    cx.run_until_parked();
    // nothing goes before you press Enter
    assert_eq!(sent(&app).len(), before);
    app.type_in_terminal(cx, "\r");
    let got = until_sent(cx, &app, before + 1);
    assert!(got.len() > before, "Claude never got the remarks");
    let message = &got.last().unwrap().1;
    assert!(message.contains("Review remarks on the code in this workspace"), "{message}");
    // both in one message, by file as the changed files are listed: a folder's before the top's own
    assert!(message.contains("1. src/app.py:2"), "{message}");
    assert!(message.contains("2. fake-output.txt:2") && message.contains("   Why a second one?"), "{message}");
    assert!(message.contains(&format!("   > {}", code[1])), "{message}");
    assert!(message.contains("   Handle an empty list here"), "{message}");
    assert_eq!(got.len(), before + 1, "sent as more than one prompt");
    assert!(app.until(cx, |w| w.try_find("review-bar").is_none()), "the bar stayed");
    // and the file without changes is no longer listed, under the diffs or in the panel
    assert!(app.until(cx, |w| rows(w, "remark-file-row").is_empty()), "the panel still lists it");
    assert!(app.until(cx, |w| !all(w, "file-head").contains(&"src/app.py".to_string())), "still under the diffs");
    assert!(until_kept(cx, &app, &ws, 0).is_empty(), "the remarks are still in the database");
}

/// The Live button of the workspace on screen: what it says, its state, and whether it is pressed.
fn live_button(window: &Window) -> (String, String, bool) {
    let button = window.find("live-button");
    (
        button.label().unwrap_or_default().to_string(),
        button.description().unwrap_or_default().to_string(),
        button.checked() == Some(true),
    )
}

/// The toasts on screen: each one's title and what is under it.
fn toasts(window: &Window) -> Vec<(String, String)> {
    let said = |id: &'static str| -> Vec<String> {
        window.find_all(id).iter().map(|found| found.label().unwrap_or_default().to_string()).collect()
    };
    let details = said("toast-detail");
    // a toast of one line has nothing under it
    said("toast")
        .into_iter()
        .enumerate()
        .map(|(ix, title)| (title, details.get(ix).cloned().unwrap_or_default()))
        .collect()
}

/// What is under the toast titled `title`, when it is on screen.
fn toast(window: &Window, title: &str) -> Option<String> {
    toasts(window).into_iter().find(|toast| toast.0 == title).map(|toast| toast.1)
}

/// Takes the pointer off the Live button (a click leaves it there, and Live pointed at says Stop).
fn point_away(cx: &mut TestAppContext, app: &App) {
    cx.update_window(app.window, |_, w, cx| w.hover(("tab", 0usize), cx)).unwrap();
}

/// Goes Live: copies the workspace's changes into the main folder, follows them, and puts the main folder back.
#[gpui_kit::test]
fn goes_live_copies_the_workspaces_changes_into_the_main_folder_follows_them_and_puts_the_main_folder_back(
    cx: &mut TestAppContext,
) {
    let (app, ws) = start(cx, false);
    let main = app.project.clone();
    let state = app.home.path().join("live-state.txt");
    let read = |path: &Path| std::fs::read_to_string(path).unwrap_or_default();
    let main_before = git(&main, &["status", "--porcelain"]);
    // a change in the workspace, and an edit of the main folder's own to the same file
    std::fs::write(ws.join("src/app.py"), "def total(items):\n    return sum(items)  # live v1\n").unwrap();
    std::fs::write(main.join("src/app.py"), "def total(items):\n    return sum(items)  # main's own edit\n").unwrap();
    let main_own = git(&main, &["status", "--porcelain"]);
    assert_ne!(main_own, main_before);

    assert_eq!(look(cx, &app, |w| live_button(w)), ("Go live".into(), "off".into(), false));
    app.click(cx, "live-button");
    // the tab says Live, and the button is the green one, pressed
    assert!(
        app.until(cx, |w| said(w, ("tab-words", 0usize)).as_deref() == Some("Live")),
        "the tab never said Live; the toasts: {:?}",
        look(cx, &app, |w| toasts(w))
    );
    // the pointer is still on the button, so it says what a click would do now; away from it, it says Live
    assert!(
        app.until(cx, |w| live_button(w) == ("Stop".into(), "live".into(), true)),
        "{:?}",
        look(cx, &app, |w| live_button(w))
    );
    point_away(cx, &app);
    assert!(
        app.until(cx, |w| live_button(w) == ("Live".into(), "live".into(), true)),
        "{:?}",
        look(cx, &app, |w| live_button(w))
    );
    // the copy came first, then the project's own command
    assert!(read(&main.join("src/app.py")).contains("# live v1"));
    assert_eq!(read(&state).trim(), format!("on {}", ws.display()));
    // the status line says it in words
    assert_eq!(look(cx, &app, |w| said(w, "status-live")).as_deref(), Some("Live in shop's dev environment"));
    // and Claude in the Live workspace is told
    app.type_in_terminal(cx, "are we live\r");
    assert!(
        app.until_terminal(cx, "[context] LIVE-MESSAGE-E2E"),
        "the Live message didn't reach Claude:\n{}",
        app.terminal_text(cx)
    );
    // what Claude writes in the workspace, and your next edit, follow into the main folder on their own
    let followed = main.join("fake-output.txt");
    assert!(
        app.until_frame(cx, |_| read(&followed).contains("are we live")),
        "Claude's edit never reached the main folder: in the workspace {:?}, in the main folder {:?}, the toasts {:?}, \
         the terminal:\n{}",
        read(&ws.join("fake-output.txt")),
        read(&followed),
        look(cx, &app, |w| toasts(w)),
        app.terminal_text(cx)
    );
    std::fs::write(ws.join("src/app.py"), "def total(items):\n    return sum(items)  # live v2\n").unwrap();
    assert!(
        app.until_frame(cx, |_| read(&main.join("src/app.py")).contains("# live v2")),
        "the edit never followed into the main folder"
    );

    // pointed at, the button says Stop; a click puts the main folder back
    cx.update_window(app.window, |_, w, cx| w.hover("live-button", cx)).unwrap();
    assert!(app.until(cx, |w| live_button(w).0 == "Stop"), "{:?}", look(cx, &app, |w| live_button(w)));
    app.click(cx, "live-button");
    assert!(app.until_frame(cx, |_| read(&state).trim() == "off"), "the off command never ran: {:?}", read(&state));
    assert!(app.until(cx, |w| said(w, ("tab-words", 0usize)).as_deref() != Some("Live")), "the tab still says Live");
    assert!(app.until(cx, |w| live_button(w).1 == "off"), "{:?}", look(cx, &app, |w| live_button(w)));
    assert!(look(cx, &app, |w| w.try_find("status-live").is_none()), "the status line still says Live");
    // the main folder is back as it was, its own edit included, and Claude's file is gone from it
    assert!(read(&main.join("src/app.py")).contains("# main's own edit"));
    assert!(!followed.exists());
    assert_eq!(git(&main, &["status", "--porcelain"]), main_own);
    // and once Live is off, edits stay in the workspace
    std::fs::write(ws.join("src/app.py"), "def total(items):\n    return sum(items)  # live v3\n").unwrap();
    for _ in 0..200 {
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(read(&main.join("src/app.py")).contains("# main's own edit"));
}

/// Switches Live and says what it did to the main folder, then the command output, on the real core: the toast after Go
/// live, the toast after Stop, and the one when Live can't switch.
#[gpui_kit::test]
fn switches_live_and_says_what_it_did_to_the_main_folder_then_the_command_output(cx: &mut TestAppContext) {
    let (app, ws) = start(cx, false);
    let main = app.project.clone();
    app.click(cx, "live-button");
    assert!(app.until(cx, |w| live_button(w).1 == "live"), "{:?}", look(cx, &app, |w| (live_button(w), toasts(w))));
    let detail = look(cx, &app, |w| toast(w, "stock-fix is Live")).expect("a toast after Go live");
    // what went where, then what the project's own command printed
    let place = enkidu_model::text::short_path(&main.to_string_lossy());
    let copied = format!("Copied 1 file with its changes into {place}; new changes follow while it is Live.");
    assert_eq!(detail, format!("{copied}\n\n$ on (stock-fix)"), "the main folder: {place}");
    let _ = &ws;

    // Stop: the main folder is back
    app.click(cx, "live-button");
    assert!(app.until(cx, |w| live_button(w).1 == "off"));
    assert!(app.until(cx, |w| toast(w, "Live stopped").is_some()), "{:?}", look(cx, &app, |w| toasts(w)));
    let detail = look(cx, &app, |w| toast(w, "Live stopped")).unwrap();
    assert!(detail.contains("back as it was"), "{detail}");

    // Live that can't switch says why, and nothing is Live: the on command fails
    let mut settings = app.shared.project_settings(&app.project_id).unwrap();
    settings.live.on = "echo the server is down; exit 3".into();
    app.shared.save_project_settings(&app.project_id, &settings).unwrap();
    app.click(cx, "live-button");
    assert!(app.until(cx, |w| toast(w, "Live didn't switch").is_some()), "{:?}", look(cx, &app, |w| toasts(w)));
    let why = look(cx, &app, |w| toast(w, "Live didn't switch")).unwrap();
    assert!(why.starts_with("The Live on command exited with Some(3)") && why.contains("the server is down"), "{why}");
    assert_eq!(look(cx, &app, |w| live_button(w)), ("Go live".into(), "off".into(), false));
    // and the main folder is as it was (what was copied went back)
    assert!(!main.join("setup-ran.txt").exists());
}

/// Shows which way Live is switching, and ignores a second click meanwhile.
#[gpui_kit::test]
fn shows_which_way_live_is_switching_and_ignores_a_second_click_meanwhile(cx: &mut TestAppContext) {
    let (app, _) = start(cx, true);
    let state = app.home.path().join("live-state.txt");
    let read = || std::fs::read_to_string(&state).unwrap_or_default();
    // the project's own command takes a moment, so the switch can be seen
    let mut settings = app.shared.project_settings(&app.project_id).unwrap();
    settings.live.on = format!("sleep 1; echo on {{name}} >> '{}'", state.display());
    settings.live.off = format!("sleep 1; echo off {{name}} >> '{}'", state.display());
    app.shared.save_project_settings(&app.project_id, &settings).unwrap();

    // Go live: "Starting…" at the same width, and a second click meanwhile does nothing
    let width = look(cx, &app, |w| w.find("live-button").bounds().size.width);
    app.click(cx, "live-button");
    assert_eq!(look(cx, &app, |w| live_button(w)), ("Starting…".into(), "starting".into(), false));
    assert_eq!(look(cx, &app, |w| w.find("live-button").bounds().size.width), width, "the button kept its width");
    app.click(cx, "live-button");
    // the other workspace's button waits meanwhile
    app.press(cx, "cmd-2");
    assert!(app.until(cx, |w| live_button(w).1 == "waiting"), "{:?}", look(cx, &app, |w| live_button(w)));
    app.click(cx, "live-button");
    app.press(cx, "cmd-1");
    assert!(app.until(cx, |w| live_button(w).1 == "live"), "{:?}", look(cx, &app, |w| (live_button(w), toasts(w))));
    assert_eq!(read(), "on stock-fix\n", "one switch, whatever was clicked meanwhile");
    assert_eq!(look(cx, &app, |w| w.find("live-button").bounds().size.width), width);

    // going Live in the other workspace: the one that was Live goes back first (its button says Stopping…), then that
    // one is Live
    app.press(cx, "cmd-2");
    assert!(app.until(cx, |w| live_button(w).1 == "off"));
    app.click(cx, "live-button");
    assert_eq!(look(cx, &app, |w| live_button(w)), ("Starting…".into(), "starting".into(), false));
    app.press(cx, "cmd-1");
    assert!(app.until(cx, |w| live_button(w).1 == "stopping"), "{:?}", look(cx, &app, |w| live_button(w)));
    assert_eq!(look(cx, &app, |w| live_button(w).0), "Stopping…");
    assert!(app.until(cx, |w| live_button(w).1 == "off"), "{:?}", look(cx, &app, |w| (live_button(w), toasts(w))));
    assert_eq!(read(), "on stock-fix\noff stock-fix\non remote-only\n", "the first one's off, then the other one's on");
    assert!(app.until(cx, |w| toast(w, "remote-only is Live").is_some()), "{:?}", look(cx, &app, |w| toasts(w)));
    // the tabs say which one is Live
    let words = look(cx, &app, |w| (said(w, ("tab-words", 0usize)), said(w, ("tab-words", 1usize))));
    assert_eq!(words.1.as_deref(), Some("Live"));
    assert_ne!(words.0.as_deref(), Some("Live"));
    // stopping from the Live one: "Stopping…", then off
    app.press(cx, "cmd-2");
    assert!(app.until(cx, |w| live_button(w).1 == "live"));
    app.click(cx, "live-button");
    assert_eq!(look(cx, &app, |w| live_button(w)), ("Stopping…".into(), "stopping".into(), false));
    assert!(app.until(cx, |w| live_button(w).1 == "off"));
    assert!(app.until(cx, |w| toast(w, "Live stopped").is_some()), "{:?}", look(cx, &app, |w| toasts(w)));
}

/// Items 4.2c and 4.5: the palette's Live command for the workspace on screen, the button's tooltip, and what Live
/// says on its own while it follows a workspace.
#[gpui_kit::test]
fn the_palette_goes_live_the_tooltip_says_what_a_press_does_and_live_says_when_it_keeps_a_main_folder_edit(
    cx: &mut TestAppContext,
) {
    let (app, ws) = start(cx, true);
    let main = app.project.clone();
    let read = |path: &Path| std::fs::read_to_string(path).unwrap_or_default();
    // the tooltip comes a moment after the pointer (the test's clock is moved)
    let tip = |cx: &mut TestAppContext| {
        cx.update_window(app.window, |_, w, cx| {
            w.hover(("tab", 0usize), cx);
            w.hover("live-button", cx);
        })
        .unwrap();
        cx.executor().advance_clock(Duration::from_millis(900));
        let said = look(cx, &app, |w| said(w, "tip"));
        point_away(cx, &app);
        said
    };
    assert_eq!(
        tip(cx).as_deref(),
        Some("Copy stock-fix's changes into shop's main folder, where the dev environment runs, and keep them in step")
    );

    // the palette: "Go live with stock-fix"
    std::fs::write(ws.join("src/app.py"), "def total(items):\n    return sum(items)  # live v1\n").unwrap();
    app.press(cx, "cmd-k");
    assert!(app.until(cx, |w| w.try_find("command-palette").is_some()));
    app.typed(cx, "go live");
    assert!(
        app.until(cx, |w| said(w, ("palette-row", 0usize)).as_deref() == Some("Go live with stock-fix")),
        "{:?}",
        look(cx, &app, |w| said(w, ("palette-row", 0usize)))
    );
    app.press(cx, "enter");
    assert!(app.until(cx, |w| w.try_find("command-palette").is_none() && live_button(w).1 == "live"));
    assert!(app.until_frame(cx, |_| read(&main.join("src/app.py")).contains("# live v1")));
    assert_eq!(
        tip(cx).as_deref(),
        Some("stock-fix's changes are in shop's main folder and follow every edit. Click to put the main folder back.")
    );
    // on the other workspace the tooltip says whose changes go back first
    app.press(cx, "cmd-2");
    assert!(app.until(cx, |w| live_button(w).1 == "off"));
    assert_eq!(
        tip(cx).as_deref(),
        Some(
            "Copy remote-only's changes into shop's main folder, where the dev environment runs, and keep them in \
             step. stock-fix's go back first."
        )
    );
    app.press(cx, "cmd-1");
    assert!(app.until(cx, |w| live_button(w).1 == "live"));

    // an edit of the main folder's own to a file Live copied: when the workspace's next change comes, that version
    // is set aside, and a toast says where
    std::fs::write(main.join("src/app.py"), "def total(items):\n    return 0  # edited in the main folder\n").unwrap();
    std::fs::write(ws.join("src/app.py"), "def total(items):\n    return sum(items)  # live v2\n").unwrap();
    assert!(
        app.until(cx, |w| toast(w, "Kept a main-folder edit").is_some()),
        "no toast about the main folder's edit: {:?}; the main folder has {:?}",
        look(cx, &app, |w| toasts(w)),
        read(&main.join("src/app.py"))
    );
    let where_kept = look(cx, &app, |w| toast(w, "Kept a main-folder edit")).unwrap();
    assert!(
        where_kept.starts_with("src/app.py changed in the main folder while Live, so that version is kept in "),
        "{where_kept}"
    );
    assert!(read(&main.join("src/app.py")).contains("# live v2"), "the workspace's change went in");

    // the palette again: "Stop Live for stock-fix"
    app.press(cx, "cmd-k");
    assert!(app.until(cx, |w| w.try_find("command-palette").is_some()));
    app.typed(cx, "stop live");
    assert!(app.until(cx, |w| said(w, ("palette-row", 0usize)).as_deref() == Some("Stop Live for stock-fix")));
    app.press(cx, "enter");
    assert!(app.until(cx, |w| w.try_find("command-palette").is_none() && live_button(w).1 == "off"));
    assert!(app.until(cx, |w| toast(w, "Live stopped").is_some()), "{:?}", look(cx, &app, |w| toasts(w)));
}

/// Jumps to a workspace from the command palette.
#[gpui_kit::test]
fn jumps_to_a_workspace_from_the_command_palette(cx: &mut TestAppContext) {
    let (app, _) = start(cx, true);
    assert_eq!(app.on_screen(cx).map(|w| w.0).as_deref(), Some("stock-fix"));
    app.press(cx, "cmd-k");
    assert!(app.until(cx, |w| w.try_find("command-palette").is_some()), "⌘K opened nothing");
    app.typed(cx, "remote-only");
    assert!(
        app.until(cx, |w| said(w, ("palette-row", 0usize)).as_deref() == Some("remote-only")),
        "{:?}",
        look(cx, &app, |w| said(w, ("palette-row", 0usize)))
    );
    app.press(cx, "enter");
    assert!(app.until(cx, |w| w.try_find("command-palette").is_none()), "the palette stayed");
    assert!(app.until_frame(cx, |f| f.current_workspace().is_some_and(|w| w.name == "remote-only")));
    // its tab is the one picked, and its own Claude has the keyboard
    look(cx, &app, |w| {
        assert_eq!(w.find(("tab", 1usize)).selected(), Some(true));
        assert_eq!(w.find(("tab", 0usize)).selected(), Some(false));
    });
    assert!(app.until(cx, |w| w.try_find("terminal").is_some_and(|terminal| terminal.focused() == Some(true))));
}

/// Remembers the workspace and the layout after a restart: quitting the app and opening it again brings back the
/// workspace you were in, how it was laid out, and its Claude in its conversation. (The run looks at the second
/// workspace and at layouts the app does not start with, so that remembering is what is tested.)
#[gpui_kit::test]
fn remembers_the_workspace_and_the_layout_after_a_reload(cx: &mut TestAppContext) {
    let (app, _) = start(cx, true);
    app.click(cx, ("tab", 1usize));
    assert!(app.until_frame(cx, |f| f.current_workspace().is_some_and(|w| w.name == "remote-only")));
    assert!(app.until(cx, |w| mode(w) == "stack"), "{}", look(cx, &app, |w| mode(w)));
    // something said to its Claude, so there is a conversation to come back to (one without a word isn't kept)
    let (_, other) = workspace(cx, &app, "remote-only");
    assert!(app.until_terminal(cx, "\n>"), "its Claude's prompt");
    prompt(cx, &app, &other, "before the reload");
    look(cx, &app, |w| assert_eq!(w.find("diff-split").selected(), Some(true)));
    app.click(cx, "layout-side");
    assert!(app.until(cx, |w| mode(w) == "side"));
    app.click(cx, "diff-unified");
    assert!(app.until(cx, |w| w.find("diff-unified").selected() == Some(true)));
    let session = |app: &App| {
        let starts = app.claude_starts();
        let last =
            starts.iter().rev().find(|s| s["env"]["ENKIDU_WORKSPACE_NAME"] == "remote-only").expect("its Claude");
        (last["sessionId"].as_str().unwrap().to_string(), last["resumed"] == true)
    };
    let (conversation, _) = session(&app);
    // where you are and how it is laid out are kept a moment after they changed
    cx.executor().advance_clock(Duration::from_millis(600));
    for _ in 0..30 {
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(10));
    }

    let app = common::restart(cx, app);
    app.until_claude_started(cx, 4);
    assert!(
        app.until_frame(cx, |f| f.current_workspace().is_some_and(|w| w.name == "remote-only")),
        "another workspace is on screen"
    );
    look(cx, &app, |w| assert_eq!(w.find(("tab", 1usize)).selected(), Some(true)));
    assert!(app.until(cx, |w| mode(w) == "side"), "the layout: {}", look(cx, &app, |w| mode(w)));
    assert!(
        app.until(cx, |w| w.try_find("diff-unified").is_some_and(|one| one.selected() == Some(true))),
        "the diff layout"
    );
    // its Claude came back in the same conversation
    assert_eq!(session(&app), (conversation, true));
    assert!(app.until_terminal(cx, "Fake Claude"), "the terminal didn't come back:\n{}", app.terminal_text(cx));
    // (beside the changes the terminal is narrow, and the line wraps before its last word)
    assert!(app.until_terminal(cx, "resumed"), "{}", app.terminal_text(cx));
}
