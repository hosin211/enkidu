//! Item 3.10: a merge in progress shows only your own work (DESIGN.md, "The changes are what isn't committed yet").
//! While a merge is on, git stages everything the merge brings in, so to git it is all uncommitted; none of it is your
//! work. The list compares each file with what the merge makes of it on its own: it shows the conflicts (against HEAD,
//! markers and all) and what you or Claude change on top. The first test runs through the whole app.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::{App, said};
use enkidu_testkit::git;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, Window};

/// The sample project with the workspace "stock-fix" on a new branch; its folder and its id.
fn start(cx: &mut TestAppContext) -> (App, PathBuf, String) {
    let app = common::start_on(cx, enkidu_testkit::shop, |shared, project, _| {
        App::make(shared, project, "stock-fix", "new", "stock-fix", Some("development"))
    });
    let (_, _, folder) = app.on_screen(cx).expect("on screen");
    let ws = app.frame.read_with(cx, |f, _| f.current_workspace().unwrap().id.clone());
    (app, PathBuf::from(folder), ws)
}

/// What the app's store has as changed in the workspace, as the core's watcher told it: "path kind", by path. (Asking
/// the core itself would make the list known there, and the watcher only sends news.)
fn listed(cx: &mut TestAppContext, ws: &str) -> String {
    cx.run_until_parked();
    let mut files: Vec<String> = cx.update(|cx| {
        let lists = enkidu_app::stores::Stores::changes(cx);
        let list = lists.read(cx).of(ws).cloned().unwrap_or_default();
        list.iter().map(|change| format!("{} {}", change.path, change.kind)).collect()
    });
    files.sort();
    files.join(", ")
}

/// What every element with `id` says, top to bottom.
fn all(window: &Window, id: &'static str) -> Vec<String> {
    window.find_all(id).iter().map(|found| found.label().unwrap_or_default().to_string()).collect()
}

/// The rows of the Changed files panel: each file and what happened to it.
fn rows(window: &Window) -> Vec<String> {
    (0..40usize)
        .filter_map(|ix| window.try_find(("change-row", ix)))
        .map(|row| row.label().unwrap_or_default().to_string())
        .collect()
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

/// A git command that may fail (a merge that stops on a conflict does); whether it succeeded.
fn try_git(dir: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .output()
        .expect("git runs")
        .status
        .success()
}

/// Shows only your own work while a merge is on, not what the merge brings in.
#[gpui_kit::test]
fn shows_only_your_own_work_while_a_merge_is_on_not_what_the_merge_brings_in(cx: &mut TestAppContext) {
    let (app, folder, ws) = start(cx);
    // another branch moves on: two files of its own
    git(&folder, &["checkout", "-q", "-b", "incoming"]);
    std::fs::write(folder.join("src/incoming_a.py"), "a = 1\n").unwrap();
    std::fs::write(folder.join("src/incoming_b.py"), "b = 2\n").unwrap();
    git(&folder, &["add", "src"]);
    git(&folder, &["commit", "-qm", "incoming work"]);
    git(&folder, &["checkout", "-q", "stock-fix"]);
    std::fs::write(folder.join("notes.md"), "mine\n").unwrap();
    git(&folder, &["merge", "-q", "--no-commit", "--no-ff", "incoming"]);
    // to git, the merge's two files are uncommitted like your own
    assert_eq!(git(&folder, &["status", "--porcelain"]).lines().count(), 3);

    // an edit of yours during the merge: it shows, and the two files the merge brings in don't
    std::fs::write(folder.join("src/app.py"), "def total(items):\n    return sum(items) or 0\n").unwrap();
    let mut seen = String::new();
    for _ in 0..1500 {
        seen = listed(cx, &ws);
        if seen == "notes.md untracked, src/app.py modified" {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(seen, "notes.md untracked, src/app.py modified", "the list showed what the merge brings in");

    // the same on screen: the changes, the Changed files panel, the tab and the status line count your two files
    assert!(app.until(cx, |w| rows(w) == ["src/app.py", "notes.md"]), "the panel: {:?}", rows_now(cx, &app));
    assert!(app.until(cx, |w| all(w, "file-head") == ["src/app.py", "notes.md"]), "the diffs' headers");
    assert!(
        app.until(cx, |w| has_line(w, "    return sum(items) or 0", "Line 2, after, added")),
        "your edit's diff never showed"
    );
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        assert_eq!(said(w, "changes-count").as_deref(), Some("2 files"));
        assert_eq!(said(w, "rail-count").as_deref(), Some("2"));
        assert_eq!(all(w, "row-kind"), ["Modified", "New file"]);
        // your edit against what the merge makes of the file: one line out, one in
        assert!(has_line(w, "    return sum(items)", "Line 2, before, removed"));
        assert_eq!(
            (all(w, "row-added"), all(w, "row-removed")),
            (vec!["+1".to_string(), "+1".into()], vec!["\u{2212}1".to_string()])
        );
        // nothing of the merge's own
        assert!(lines(w).iter().all(|(text, _)| text != "a = 1" && text != "b = 2"));
    })
    .unwrap();

    // called off: as before the merge
    git(&folder, &["checkout", "--", "src/app.py"]);
    git(&folder, &["merge", "--abort"]);
    std::fs::remove_file(folder.join("notes.md")).unwrap();
    assert!(app.until(cx, |w| w.try_find("island-changes").is_none() && rows(w).is_empty()), "the list didn't empty");
    assert_eq!(listed(cx, &ws), "");
}

fn rows_now(cx: &mut TestAppContext, app: &App) -> Vec<String> {
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        rows(w)
    })
    .unwrap()
}

/// A merge that stops on a conflict: the conflicted file shows, against HEAD, with git's markers; what the merge
/// brought in without a conflict doesn't.
#[gpui_kit::test]
fn a_conflict_shows_against_head_with_its_markers(cx: &mut TestAppContext) {
    let (app, folder, ws) = start(cx);
    // the other branch changes the total and adds a file
    git(&folder, &["checkout", "-q", "-b", "incoming"]);
    std::fs::write(folder.join("src/app.py"), "def total(items):\n    return sum(items) + 1\n").unwrap();
    std::fs::write(folder.join("src/incoming.py"), "theirs = True\n").unwrap();
    git(&folder, &["add", "src"]);
    git(&folder, &["commit", "-qm", "incoming work"]);
    // and so does yours, on the same line
    git(&folder, &["checkout", "-q", "stock-fix"]);
    std::fs::write(folder.join("src/app.py"), "def total(items):\n    return sum(items) + 2\n").unwrap();
    git(&folder, &["commit", "-qam", "mine"]);
    assert!(!try_git(&folder, &["merge", "-q", "incoming"]), "the merge was to stop on a conflict");

    let mut seen = String::new();
    for _ in 0..1500 {
        seen = listed(cx, &ws);
        if seen == "src/app.py conflicted" {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(seen, "src/app.py conflicted", "the conflict alone is yours to deal with");
    assert!(app.until(cx, |w| rows(w) == ["src/app.py"]), "the panel: {:?}", rows_now(cx, &app));
    assert!(app.until(cx, |w| all(w, "row-kind") == ["Conflict"]));
    // against HEAD (your side), with the markers git wrote into the file
    assert!(
        app.until(cx, |w| lines(w).iter().any(|(text, _)| text == "<<<<<<< HEAD")),
        "the markers never showed: {:?}",
        cx.update_window(app.window, |_, w, _| lines(w)).unwrap()
    );
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        let text: Vec<String> = lines(w).into_iter().map(|(text, _)| text).collect();
        assert!(text.contains(&"    return sum(items) + 1".to_string()), "their side: {text:?}");
        assert!(text.contains(&"    return sum(items) + 2".to_string()), "your side: {text:?}");
        assert!(text.iter().any(|line| line.starts_with(">>>>>>> ")), "the end marker: {text:?}");
        assert!(!text.contains(&"theirs = True".to_string()), "what the merge brought in without a conflict");
    })
    .unwrap();

    // resolved and committed: nothing is left
    std::fs::write(folder.join("src/app.py"), "def total(items):\n    return sum(items) + 3\n").unwrap();
    git(&folder, &["commit", "-qam", "merged"]);
    assert!(app.until(cx, |w| w.try_find("island-changes").is_none() && rows(w).is_empty()), "the list didn't empty");
}
