//! Item 3.2d: the changes island on the whole wiring. The core's watcher sees files change in the workspace's folder,
//! git gives each file's diff off the drawing thread, and the island shows them stacked under their headers, side by
//! side or in one column; with nothing to review it isn't there.

mod common;

use std::path::PathBuf;
use std::time::Duration;

use common::{App, said};
use enkidu_diff::{DiffLinePosition, DiffSide, DiffState};
use enkidu_testkit::git;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext, Window};

/// The sample project with the workspace "stock-fix" on a new branch; its folder.
fn start(cx: &mut TestAppContext) -> (App, PathBuf) {
    let app = common::start_on(cx, enkidu_testkit::shop, |shared, project, _| {
        App::make(shared, project, "stock-fix", "new", "stock-fix", Some("development"))
    });
    let (_, _, folder) = app.on_screen(cx).expect("on screen");
    (app, PathBuf::from(folder))
}

/// What every element with `id` says, top to bottom.
fn all(window: &Window, id: &'static str) -> Vec<String> {
    window.find_all(id).iter().map(|found| found.label().unwrap_or_default().to_string()).collect()
}

/// The diff's lines on screen: what each says and which line it is.
fn lines(window: &Window) -> Vec<(String, String)> {
    window
        .find_all("source")
        .iter()
        .map(|line| (line.label().unwrap_or_default().to_string(), line.description().unwrap_or_default().to_string()))
        .collect()
}

fn has_line(window: &Window, text: &str, which: &str) -> bool {
    lines(window).iter().any(|(said, line)| said == text && line == which)
}

/// The view state of the diffs on screen.
fn diff(cx: &mut TestAppContext, app: &App) -> Entity<DiffState> {
    app.shell.read_with(cx, |shell, cx| shell.changes().expect("the island").read(cx).diff(cx)).expect("a review")
}

#[gpui_kit::test]
fn a_workspaces_changes_show_as_diffs_under_their_headers_and_go_with_a_commit(cx: &mut TestAppContext) {
    let (app, folder) = start(cx);
    // nothing changed yet: no island, no words; the terminal has the workspace
    assert!(app.until(cx, |w| w.try_find("island-claude").is_some()));
    cx.update_window(app.window, |_, w, _| assert!(w.try_find("island-changes").is_none())).unwrap();
    // an edit and a new file
    std::fs::write(folder.join("src/app.py"), "def total(items):\n    return sum(i.price for i in items)\n").unwrap();
    std::fs::write(folder.join("notes.md"), "# Notes\n\nCheck the totals.\n").unwrap();
    assert!(app.until(cx, |w| said(w, "changes-count").as_deref() == Some("2 files")), "the band's count");
    // each file under its header, in the order the tree of changed files lists them, with what happened to it and
    // its lines
    assert!(app.until(cx, |w| all(w, "file-head") == ["src/app.py", "notes.md"]));
    // each file's diff is read on its own: wait for both
    assert!(
        app.until(cx, |w| {
            has_line(w, "    return sum(i.price for i in items)", "Line 2, after, added")
                && has_line(w, "# Notes", "Line 1, after, added")
        }),
        "the diffs were never read"
    );
    cx.update_window(app.window, |_, w, _| {
        assert_eq!(all(w, "file-kind"), ["Modified", "New file"]);
        assert_eq!(all(w, "file-added"), ["+1", "+3"]);
        assert_eq!(all(w, "file-removed"), ["\u{2212}1"]);
        assert_eq!(
            (said(w, "changes-added"), said(w, "changes-removed")),
            (Some("+4".into()), Some("\u{2212}1".into()))
        );
        assert!(has_line(w, "    return sum(items)", "Line 2, before, removed"), "{:?}", lines(w));
        assert!(has_line(w, "# Notes", "Line 1, after, added"));
        assert!(has_line(w, "Check the totals.", "Line 3, after, added"));
        // side by side: the two sides are named, and the unchanged line is on both
        assert_eq!(w.find("diff-split").selected(), Some(true));
        assert_eq!(all(w, "split-before"), ["Before", "Before: a new file"]);
        assert_eq!(all(w, "split-after"), ["After", "After"]);
        assert!(
            has_line(w, "def total(items):", "Line 1, before") && has_line(w, "def total(items):", "Line 1, after")
        );
        // the terminal is still there, under the changes
        let (changes, terminal) = (w.find("island-changes").bounds(), w.find("island-claude").bounds());
        assert!(changes.bottom() < terminal.top(), "{changes:?} {terminal:?}");
    })
    .unwrap();

    // one column: no sides to name, the removed line then the added one
    app.click(cx, "diff-unified");
    assert!(app.until(cx, |w| w.try_find("split-before").is_none() && w.find("diff-unified").selected() == Some(true)));
    cx.update_window(app.window, |_, w, _| {
        let said: Vec<String> = lines(w).into_iter().map(|(said, _)| said).collect();
        let at = |text: &str| said.iter().position(|line| line == text).unwrap_or_else(|| panic!("{text}: {said:?}"));
        assert!(at("    return sum(items)") < at("    return sum(i.price for i in items)"));
        assert_eq!(said.iter().filter(|line| *line == "def total(items):").count(), 1);
    })
    .unwrap();
    // the layout is kept for the next launch
    cx.executor().advance_clock(Duration::from_millis(400));
    let kept = app.home.path().join("data").join("ui.json");
    for _ in 0..400 {
        cx.run_until_parked();
        if kept.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let kept = std::fs::read_to_string(&kept).unwrap();
    assert!(kept.starts_with(r#"{"diffLayout":"unified","#), "{kept}");

    // a commit: nothing left to review, the island goes and the terminal has the room again
    git(&folder, &["add", "-A"]);
    git(&folder, &["commit", "-q", "-m", "work"]);
    assert!(app.until(cx, |w| w.try_find("island-changes").is_none()), "the island stayed after the commit");
    cx.update_window(app.window, |_, w, _| assert!(w.try_find("changes-count").is_none())).unwrap();
}

#[gpui_kit::test]
fn a_file_that_changes_again_shows_its_new_diff_and_the_view_keeps_its_place(cx: &mut TestAppContext) {
    let (app, folder) = start(cx);
    let numbered = |count: usize| (1..=count).map(|n| format!("row {n}\n")).collect::<String>();
    std::fs::write(folder.join("long.txt"), numbered(300)).unwrap();
    std::fs::write(folder.join("a.txt"), "first\n").unwrap();
    assert!(
        app.until(cx, |w| has_line(w, "first", "Line 1, after, added") && has_line(w, "row 1", "Line 1, after, added")),
        "never read"
    );
    // reading long.txt far down
    let state = diff(cx, &app);
    cx.update_window(app.window, |_, _, cx| {
        state.update(cx, |state, cx| {
            state.scroll_to_line(DiffLinePosition::new("long.txt", DiffSide::Modified, 200), cx)
        })
    })
    .unwrap();
    assert!(app.until(cx, |w| has_line(w, "row 200", "Line 200, after, added")));
    let top = |cx: &mut TestAppContext| state.read_with(cx, |state, _| state.top_path().map(|path| path.to_string()));
    assert_eq!(top(cx).as_deref(), Some("long.txt"));
    // the file above it grows, and a file is added before it: still there, and the new lines were read
    std::fs::write(folder.join("a.txt"), "first\nsecond\nthird\n").unwrap();
    std::fs::write(folder.join("b.txt"), "between\n").unwrap();
    assert!(app.until(cx, |w| all(w, "file-added").first().map(String::as_str) == Some("+3")
        || said(w, "changes-count").as_deref() == Some("3 files")));
    assert!(app.until(cx, |w| said(w, "changes-count").as_deref() == Some("3 files")));
    cx.run_until_parked();
    assert_eq!(top(cx).as_deref(), Some("long.txt"));
    assert!(app.until(cx, |w| has_line(w, "row 200", "Line 200, after, added")), "the view left its place");
    // the file being read changes too: its new lines show
    std::fs::write(folder.join("long.txt"), numbered(300).replace("row 201\n", "row 201, edited\n")).unwrap();
    assert!(app.until(cx, |w| has_line(w, "row 201, edited", "Line 201, after, added")), "the new diff never showed");
    assert_eq!(top(cx).as_deref(), Some("long.txt"));
}

#[gpui_kit::test]
fn a_file_without_lines_to_show_says_why(cx: &mut TestAppContext) {
    let (app, folder) = start(cx);
    std::fs::write(folder.join("logo.png"), [0x89, b'P', b'N', b'G', 0, 1, 2, 3]).unwrap();
    // more than a diff may hold (3 MB, both versions of a file at the limit)
    let huge: String = (0..90_000).map(|n| format!("{n:06} this line is one of very many in a file\n")).collect();
    assert!(huge.len() > 3_000_000);
    std::fs::write(folder.join("data.csv"), huge).unwrap();
    std::fs::write(folder.join("caf\u{e9}.py"), "x = 1\n").unwrap();
    assert!(app.until(cx, |w| said(w, "changes-count").as_deref() == Some("3 files")));
    assert!(app.until(cx, |w| all(w, "file-note").len() == 2), "the notes never came");
    // a name outside ASCII is read as it is (git quotes it in its diff); each file is read on its own, so wait for it
    assert!(app.until(cx, |w| has_line(w, "x = 1", "Line 1, after, added")), "caf\u{e9}.py was never read");
    cx.update_window(app.window, |_, w, _| {
        assert_eq!(all(w, "file-head"), ["caf\u{e9}.py", "data.csv", "logo.png"]);
        assert_eq!(all(w, "file-note"), ["This change is too large to show here.", "Binary file, not shown."]);
    })
    .unwrap();
}

#[gpui_kit::test]
fn every_changed_file_is_read_however_many_there_are(cx: &mut TestAppContext) {
    let (app, folder) = start(cx);
    // forty new files: more than are read at once (three), and more than fit on screen
    std::fs::create_dir_all(folder.join("notes")).unwrap();
    for n in 0..40 {
        std::fs::write(folder.join(format!("notes/note-{n:02}.md")), format!("note {n}\nits second line\n")).unwrap();
    }
    assert!(app.until(cx, |w| said(w, "changes-count").as_deref() == Some("40 files")));
    assert!(app.until(cx, |w| has_line(w, "note 0", "Line 1, after, added")), "the first file was never read");
    let state = diff(cx, &app);
    // a file that has been read has its two lines; one that hasn't stands in without any
    let read = |cx: &mut TestAppContext| {
        state.read_with(cx, |state, _| state.files().iter().filter(|f| f.additions() == 2).count())
    };
    let mut seen = 0;
    for _ in 0..800 {
        cx.run_until_parked();
        // the island gives the view what was read as it draws
        cx.update_window(app.window, |_, w, cx| w.render_frame(cx)).unwrap();
        seen = read(cx);
        if seen == 40 {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(seen, 40, "reading stopped before the last file");
    // the last one is there to read, far down the list
    cx.update_window(app.window, |_, _, cx| state.update(cx, |state, cx| state.scroll_to_file("notes/note-39.md", cx)))
        .unwrap();
    assert!(app.until(cx, |w| has_line(w, "note 39", "Line 1, after, added")));
}

/// Says when a file is binary.
#[gpui_kit::test]
fn says_when_a_file_is_binary(cx: &mut TestAppContext) {
    let (app, folder) = start(cx);
    std::fs::write(folder.join("x.png"), [0x89u8, b'P', b'N', b'G', 0, 1, 2, 3, 0, 0, 0, 0]).unwrap();
    assert!(app.until(cx, |w| all(w, "file-head") == ["x.png"]), "the file was never listed");
    assert!(
        app.until(cx, |w| said(w, "file-note").as_deref() == Some("Binary file, not shown.")),
        "it says: {:?}",
        cx.update_window(app.window, |_, w, _| said(w, "file-note")).unwrap()
    );
    cx.update_window(app.window, |_, w, _| assert!(lines(w).is_empty(), "a binary file's bytes as lines")).unwrap();
}
