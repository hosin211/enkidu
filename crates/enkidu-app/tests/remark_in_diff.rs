//! Item 3.5c: remarks in the diff, in a real window on real git. A press on a line number writes a remark under that
//! line, a drag over line numbers or Shift and a click makes it a range; the card can be edited and deleted; the
//! file's header and the workspace's tab count it; a remark whose code is gone says so, above its file's lines.

mod common;

use std::path::PathBuf;

use common::{App, at, click_line, kept, line_number, press, said, until_kept};
use enkidu_core::db::Remark;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Pixels, TestAppContext, Window};

const CODE: &str =
    "def total(items):\n    result = 0\n    for item in items:\n        result += item.price\n    return result\n";

/// The sample project with one workspace, where `src/app.py` was rewritten to `CODE`.
fn start(cx: &mut TestAppContext) -> (App, PathBuf, String) {
    let app = common::start_on(cx, enkidu_testkit::shop, |shared, project, _| {
        App::make(shared, project, "stock-fix", "new", "stock-fix", Some("development"))
    });
    let (_, _, folder) = app.on_screen(cx).expect("on screen");
    let folder = PathBuf::from(folder);
    std::fs::write(folder.join("src/app.py"), CODE).unwrap();
    let ws = app.frame.read_with(cx, |f, _| f.current_workspace().unwrap().id.clone());
    assert!(app.until(cx, |w| line_number(w, "Line 5, after").is_some()), "the diff never showed");
    (app, folder, ws)
}

/// What every remark card on screen says, top to bottom.
fn cards(window: &Window, app_remarks: &[Remark]) -> Vec<String> {
    let mut found: Vec<(Pixels, String)> = app_remarks
        .iter()
        .filter_map(|remark| window.try_find(gpui_kit::SharedString::from(format!("remark-{}", remark.id))))
        .map(|card| (card.bounds().top(), card.label().unwrap_or_default().to_string()))
        .collect();
    found.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    found.into_iter().map(|(_, said)| said).collect()
}

/// Writes a remark on a line with its code.
#[gpui_kit::test]
fn writes_a_remark_on_a_line_with_its_code(cx: &mut TestAppContext) {
    let (app, _, ws) = start(cx);
    // a press on line 4's number: the composer opens under it, named for the line, with the keyboard in it
    click_line(cx, &app, "Line 4, after");
    assert!(app.until(cx, |w| said(w, "remark-composer").as_deref() == Some("Line 4")), "no composer");
    // nothing typed: "Add remark" does nothing
    app.click(cx, "save-remark");
    cx.run_until_parked();
    assert!(kept(&app, &ws).is_empty());
    app.typed(cx, "Sum the prices with sum()");
    app.click(cx, "save-remark");
    let saved = until_kept(cx, &app, &ws, 1);
    assert_eq!(saved.len(), 1, "the remark was never saved");
    let remark = &saved[0];
    assert_eq!(
        (remark.path.as_str(), remark.side.as_str(), remark.start_line, remark.end_line),
        ("src/app.py", "new", 4, 4)
    );
    assert_eq!(remark.snippet, "        result += item.price");
    assert_eq!(remark.body, "Sum the prices with sum()");
    assert_eq!(remark.workspace_id, ws);
    // the composer closed, the card is under the line, and the header, the tab and the status line count it
    assert!(
        app.until(cx, |w| w.try_find("remark-composer").is_none() && cards(w, &saved) == ["Sum the prices with sum()"])
    );
    cx.update_window(app.window, |_, w, _| {
        assert_eq!(said(w, "file-remarks").as_deref(), Some("1 remark"));
        // on its workspace's tab in the top row, and at the foot of the changes
        assert_eq!(said(w, ("tab-remarks", 0usize)).as_deref(), Some("1 remark not sent yet"));
        assert_eq!(said(w, "review-bar").as_deref(), Some("1 remark not sent yet"));
        let card = w.find(gpui_kit::SharedString::from(format!("remark-{}", saved[0].id))).bounds();
        let line = line_number(w, "Line 4, after").unwrap();
        assert!(card.top() > line.y && card.top() - line.y < gpui_kit::px(40.), "the card is right under line 4");
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_drag_over_line_numbers_or_shift_and_a_click_makes_it_a_range(cx: &mut TestAppContext) {
    let (app, _, ws) = start(cx);
    // a drag from line 2 to line 4
    let (from, to) = (at(cx, &app, "Line 2, after"), at(cx, &app, "Line 4, after"));
    press(cx, &app, from, to, false);
    assert!(app.until(cx, |w| said(w, "remark-composer").as_deref() == Some("Lines 2\u{2013}4")), "no range");
    app.typed(cx, "One line with sum() does this");
    // Shift and a click on line 5: the remark being written reaches to there, with what was typed
    let last = at(cx, &app, "Line 5, after");
    press(cx, &app, last, last, true);
    assert!(app.until(cx, |w| said(w, "remark-composer").as_deref() == Some("Lines 2\u{2013}5")));
    // ⌘↩ saves from the keyboard
    cx.update_window(app.window, |_, w, cx| w.click("remark-composer", cx)).unwrap();
    app.press(cx, "cmd-enter");
    let saved = until_kept(cx, &app, &ws, 1);
    assert_eq!(saved.len(), 1, "⌘↩ didn't save");
    assert_eq!((saved[0].start_line, saved[0].end_line), (2, 5));
    assert_eq!(
        saved[0].snippet,
        "    result = 0\n    for item in items:\n        result += item.price\n    return result"
    );
    assert_eq!(saved[0].body, "One line with sum() does this");
    // a remark on a line the change removed is on the old side
    click_line(cx, &app, "Line 2, before");
    assert!(app.until(cx, |w| said(w, "remark-composer").as_deref() == Some("Line 2")));
    app.typed(cx, "Why not keep this?");
    app.click(cx, "save-remark");
    let saved = until_kept(cx, &app, &ws, 2);
    let old = saved.iter().find(|remark| remark.side == "old").expect("a remark on the old side");
    assert_eq!((old.start_line, old.snippet.as_str()), (2, "    return sum(items)"));
    assert!(app.until(cx, |w| said(w, "file-remarks").as_deref() == Some("2 remarks")));
}

#[gpui_kit::test]
fn a_remark_is_cancelled_edited_and_deleted_where_it_stands(cx: &mut TestAppContext) {
    let (app, _, ws) = start(cx);
    // Escape leaves nothing behind
    click_line(cx, &app, "Line 1, after");
    assert!(app.until(cx, |w| w.try_find("remark-composer").is_some()));
    app.typed(cx, "never mind");
    app.press(cx, "escape");
    assert!(app.until(cx, |w| w.try_find("remark-composer").is_none()), "Escape left the composer open");
    assert!(kept(&app, &ws).is_empty());
    // Cancel too
    click_line(cx, &app, "Line 1, after");
    assert!(app.until(cx, |w| w.try_find("remark-composer").is_some()));
    app.click(cx, "cancel-remark");
    assert!(app.until(cx, |w| w.try_find("remark-composer").is_none()));

    click_line(cx, &app, "Line 3, after");
    assert!(app.until(cx, |w| w.try_find("remark-composer").is_some()));
    app.typed(cx, "Name the loop variable");
    app.click(cx, "save-remark");
    let saved = until_kept(cx, &app, &ws, 1);
    assert!(app.until(cx, |w| cards(w, &saved) == ["Name the loop variable"]));
    // its words are changed in place: the card becomes the composer with the words in it
    app.click(cx, "edit-remark");
    assert!(app.until(cx, |w| said(w, "remark-composer").as_deref() == Some("Line 3") && cards(w, &saved).is_empty()));
    app.typed(cx, " for what it holds");
    app.click(cx, "save-remark");
    for _ in 0..600 {
        cx.run_until_parked();
        if kept(&app, &ws).first().is_some_and(|remark| remark.body.ends_with("holds")) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let edited = kept(&app, &ws);
    assert_eq!(edited.len(), 1);
    assert_eq!(edited[0].body, "Name the loop variable for what it holds");
    assert_eq!((edited[0].id.as_str(), edited[0].start_line), (saved[0].id.as_str(), 3), "the same remark");
    assert!(app.until(cx, |w| cards(w, &edited) == ["Name the loop variable for what it holds"]));
    // deleted: gone from the screen, the counts and the database
    app.click(cx, "delete-remark");
    assert!(until_kept(cx, &app, &ws, 0).is_empty());
    assert!(app.until(cx, |w| cards(w, &edited).is_empty() && w.try_find("file-remarks").is_none()));
    cx.update_window(app.window, |_, w, _| {
        assert!(w.try_find("review-bar").is_none() && w.try_find(("tab-remarks", 0usize)).is_none())
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_remark_follows_its_code_and_says_when_the_code_is_gone(cx: &mut TestAppContext) {
    let (app, folder, ws) = start(cx);
    click_line(cx, &app, "Line 4, after");
    assert!(app.until(cx, |w| w.try_find("remark-composer").is_some()));
    app.typed(cx, "Sum the prices with sum()");
    app.click(cx, "save-remark");
    let saved = until_kept(cx, &app, &ws, 1);
    let id = gpui_kit::SharedString::from(format!("remark-{}", saved[0].id));
    assert!(app.until(cx, |w| w.try_find(id.clone()).is_some()));
    // Claude puts two lines in above it: the remark is under its code, which is line 6 now
    std::fs::write(folder.join("src/app.py"), CODE.replace("def total", "# prices in cents\n\ndef total")).unwrap();
    assert!(app.until(cx, |w| line_number(w, "Line 7, after").is_some()), "the new diff never showed");
    assert!(
        app.until(cx, |w| {
            let (card, line) = (w.try_find(id.clone()), line_number(w, "Line 6, after"));
            card.zip(line).is_some_and(|(card, line)| {
                card.bounds().top() > line.y && card.bounds().top() - line.y < gpui_kit::px(40.)
            })
        }),
        "the remark didn't follow its code"
    );
    cx.update_window(app.window, |_, w, _| assert!(w.try_find("remark-outdated").is_none())).unwrap();
    // Claude rewrites the line: the remark says its code changed, above the file's lines, with the code it was on
    std::fs::write(folder.join("src/app.py"), "def total(items):\n    return sum(item.price for item in items)\n")
        .unwrap();
    assert!(app.until(cx, |w| said(w, "remark-outdated").as_deref() == Some("Code changed since")), "never outdated");
    cx.update_window(app.window, |_, w, _| {
        let (card, first) = (w.find(id.clone()).bounds(), line_number(w, "Line 1, after").unwrap());
        assert!(card.bottom() < first.y, "above the file's lines: {card:?} {first:?}");
        assert_eq!(said(w, "file-remarks").as_deref(), Some("1 remark"));
    })
    .unwrap();
    // the database keeps the remark as it was written
    assert_eq!(kept(&app, &ws)[0].start_line, 4);
}

/// What every element with `id` says, top to bottom.
fn all(window: &Window, id: &'static str) -> Vec<String> {
    window.find_all(id).iter().map(|found| found.label().unwrap_or_default().to_string()).collect()
}

/// Item 3.5d: a remark whose file has no changes (any more) is listed under the diffs, and keeps the island there.
#[gpui_kit::test]
fn a_remark_on_a_file_without_changes_is_listed_under_the_diffs_and_keeps_the_island(cx: &mut TestAppContext) {
    use enkidu_testkit::git;
    let (app, folder, ws) = start(cx);
    std::fs::write(folder.join("notes.md"), "# Notes\n").unwrap();
    assert!(app.until(cx, |w| said(w, "changes-count").as_deref() == Some("2 files")));
    click_line(cx, &app, "Line 4, after");
    assert!(app.until(cx, |w| w.try_find("remark-composer").is_some()));
    app.typed(cx, "Sum the prices with sum()");
    app.click(cx, "save-remark");
    let saved = until_kept(cx, &app, &ws, 1);
    assert!(app.until(cx, |w| cards(w, &saved) == ["Sum the prices with sum()"]));
    cx.update_window(app.window, |_, w, _| assert!(w.try_find("file-unchanged").is_none())).unwrap();

    // the file's changes are committed, the remark wasn't sent: the file is listed after the changed one, for its
    // remark, and says it has no changes
    git(&folder, &["add", "src/app.py"]);
    git(&folder, &["commit", "-q", "-m", "the totals"]);
    assert!(app.until(cx, |w| said(w, "changes-count").as_deref() == Some("1 file")));
    assert!(app.until(cx, |w| all(w, "file-head") == ["notes.md", "src/app.py"]), "not listed under the diffs");
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        assert_eq!(all(w, "file-unchanged"), ["No changes"]);
        assert_eq!(all(w, "file-remarks"), ["1 remark"]);
        assert_eq!(cards(w, &saved), ["Sum the prices with sum()"]);
        // its card is under its own header
        let (head, card) = (
            w.find_all("file-head").last().unwrap().bounds(),
            w.find(gpui_kit::SharedString::from(format!("remark-{}", saved[0].id))).bounds(),
        );
        assert!(card.top() > head.bottom() && card.top() - head.bottom() < gpui_kit::px(40.), "{head:?} {card:?}");
    })
    .unwrap();

    // everything is committed: no changed file is left, and the island stays for the remark (no count of nothing)
    git(&folder, &["add", "-A"]);
    git(&folder, &["commit", "-q", "-m", "notes"]);
    assert!(app.until(cx, |w| all(w, "file-head") == ["src/app.py"]), "the island went with the last change");
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("island-changes").is_some() && w.try_find("changes-count").is_none());
        assert_eq!(cards(w, &saved), ["Sum the prices with sum()"]);
        assert_eq!(said(w, ("tab-remarks", 0usize)).as_deref(), Some("1 remark not sent yet"));
    })
    .unwrap();

    // its words can still be changed there
    app.click(cx, "edit-remark");
    assert!(app.until(cx, |w| said(w, "remark-composer").as_deref() == Some("Line 4")));
    app.typed(cx, ", please");
    app.click(cx, "save-remark");
    for _ in 0..600 {
        cx.run_until_parked();
        if kept(&app, &ws).first().is_some_and(|remark| remark.body.ends_with("please")) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(kept(&app, &ws)[0].body, "Sum the prices with sum(), please");
    let edited = kept(&app, &ws);
    assert!(app.until(cx, |w| cards(w, &edited) == ["Sum the prices with sum(), please"]));

    // deleted: nothing left to review, and the island goes
    app.click(cx, "delete-remark");
    assert!(until_kept(cx, &app, &ws, 0).is_empty());
    assert!(app.until(cx, |w| w.try_find("island-changes").is_none()), "the island stayed with nothing in it");
}
