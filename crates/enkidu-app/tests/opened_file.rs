//! Items 3.7 and 3.5e: a file opened where the diffs are, in a real window on real git. Read only, every line once
//! under its own band; "Show changes" for a changed file; Escape or the band's first button goes back. A press on a
//! line number writes a remark there, as in a diff, also on lines no diff shows.

mod common;

use std::path::PathBuf;

use common::{App, at, click_line, kept, line_number, press, said, until_kept};
use enkidu_app::changes_island::ChangesIsland;
use enkidu_testkit::git;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext, Window};

const CODE: &str =
    "def total(items):\n    result = 0\n    for item in items:\n        result += item.price\n    return result\n";

/// A file that is committed, so no diff shows it.
const GUIDE: &str = "# Guide\n\nHow totals work.\n\n1. Sum the prices.\n2. Add the tax.\n3. Round once, at the end.\n";

/// The sample project with one workspace: `docs/guide.md` is committed there, and `src/app.py` was rewritten to `CODE`
/// and isn't.
fn start(cx: &mut TestAppContext) -> (App, PathBuf, String) {
    let app = common::start_on(cx, enkidu_testkit::shop, |shared, project, _| {
        App::make(shared, project, "stock-fix", "new", "stock-fix", Some("development"))
    });
    let (_, _, folder) = app.on_screen(cx).expect("on screen");
    let folder = PathBuf::from(folder);
    std::fs::create_dir_all(folder.join("docs")).unwrap();
    std::fs::write(folder.join("docs/guide.md"), GUIDE).unwrap();
    git(&folder, &["add", "docs/guide.md"]);
    git(&folder, &["commit", "-q", "-m", "the guide"]);
    std::fs::write(folder.join("src/app.py"), CODE).unwrap();
    let ws = app.frame.read_with(cx, |f, _| f.current_workspace().unwrap().id.clone());
    assert!(app.until(cx, |w| line_number(w, "Line 5, after").is_some()), "the diff never showed");
    // git may have been asked while the guide was being committed: wait for the list as it is now
    assert!(app.until(cx, |w| all(w, "file-head") == ["src/app.py"]), "the list of changes never settled");
    (app, folder, ws)
}

fn island(cx: &mut TestAppContext, app: &App) -> Entity<ChangesIsland> {
    app.shell.read_with(cx, |shell, _| shell.changes().cloned()).expect("the island")
}

/// Opens a file as the Files panel or Search will: by its path, at a line when one is given.
fn open(cx: &mut TestAppContext, app: &App, path: &str, line: Option<usize>) {
    let island = island(cx, app);
    cx.update(|cx| island.update(cx, |island, cx| island.open_file(path, line, cx)));
    assert!(app.until(cx, |w| said(w, "file-view").as_deref() == Some(path)), "{path} never opened");
}

/// The lines on screen: what each says and which line it is.
fn lines(window: &Window) -> Vec<(String, String)> {
    window
        .find_all("source")
        .iter()
        .map(|line| (line.label().unwrap_or_default().to_string(), line.description().unwrap_or_default().to_string()))
        .collect()
}

/// What every element with `id` says, top to bottom.
fn all(window: &Window, id: &'static str) -> Vec<String> {
    window.find_all(id).iter().map(|found| found.label().unwrap_or_default().to_string()).collect()
}

fn look<R>(cx: &mut TestAppContext, app: &App, at: impl FnOnce(&mut Window) -> R) -> R {
    cx.run_until_parked();
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        at(w)
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_file_opens_where_the_diffs_are_and_escape_goes_back(cx: &mut TestAppContext) {
    let (app, folder, _) = start(cx);
    // the diff's header has the button that opens the whole file
    assert_eq!(look(cx, &app, |w| said(w, "open-file")).as_deref(), Some("Open src/app.py"));
    app.click(cx, "open-file");
    assert!(app.until(cx, |w| said(w, "file-view").as_deref() == Some("src/app.py")), "the file never opened");
    assert!(app.until(cx, |w| line_number(w, "Line 5").is_some()), "its lines never showed");
    look(cx, &app, |w| {
        // its own band in place of "Changes": the way back, the file, what happened to it, its lines
        assert!(w.try_find("diff-view").is_none(), "the diffs stayed");
        // the Changed files panel still says how many files changed
        assert_eq!(said(w, "changes-count").as_deref(), Some("1 file"));
        assert_eq!(said(w, "file-band").as_deref(), Some("src/app.py"));
        assert_eq!(said(w, "close-file").as_deref(), Some("Back to the changes"));
        assert_eq!(said(w, "file-kind").as_deref(), Some("Modified"));
        assert_eq!(said(w, "file-lines").as_deref(), Some("5 lines"));
        assert!(w.try_find("file-remarks").is_none(), "a count of no remarks");
        assert!(w.try_find("show-changes").is_some());
        // every line once, as the file is now, named by its number alone
        let want: Vec<(String, String)> =
            CODE.lines().enumerate().map(|(ix, line)| (line.to_string(), format!("Line {}", ix + 1))).collect();
        assert_eq!(lines(w), want);
        // and no header row of a diff over them
        assert!(w.try_find("file-added").is_none() && w.try_find("split-before").is_none());
    });

    // read only: keys change nothing
    app.typed(cx, "x");
    app.press(cx, "backspace");
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(folder.join("src/app.py")).unwrap(), CODE);

    // Escape goes back to the diffs, as they were
    app.press(cx, "escape");
    assert!(app.until(cx, |w| w.try_find("file-view").is_none() && w.try_find("diff-view").is_some()), "Escape");
    assert!(app.until(cx, |w| line_number(w, "Line 5, after").is_some()), "the diff is gone");
    assert_eq!(look(cx, &app, |w| said(w, "changes-count")).as_deref(), Some("1 file"));

    // the band's first button does the same
    app.click(cx, "open-file");
    assert!(app.until(cx, |w| w.try_find("file-view").is_some()));
    app.click(cx, "close-file");
    assert!(app.until(cx, |w| w.try_find("file-view").is_none() && w.try_find("diff-view").is_some()), "the button");

    // and "Show changes" brings the file's diff back
    app.click(cx, "open-file");
    assert!(app.until(cx, |w| w.try_find("show-changes").is_some()));
    app.click(cx, "show-changes");
    assert!(app.until(cx, |w| w.try_find("file-view").is_none() && all(w, "file-head") == ["src/app.py"]));
}

#[gpui_kit::test]
fn a_file_that_claude_edits_while_it_is_open_shows_its_new_lines(cx: &mut TestAppContext) {
    let (app, folder, _) = start(cx);
    app.click(cx, "open-file");
    assert!(app.until(cx, |w| line_number(w, "Line 5").is_some()), "its lines never showed");
    // two more lines on disk: the watcher sees the file change, and the opened file follows
    std::fs::write(folder.join("src/app.py"), format!("{CODE}\nprint(total([]))\n")).unwrap();
    assert!(app.until(cx, |w| said(w, "file-lines").as_deref() == Some("7 lines")), "the file wasn't read again");
    assert!(app.until(cx, |w| lines(w).last() == Some(&("print(total([]))".to_string(), "Line 7".to_string()))));
    // the edit is undone: no change is left, so nothing says what happened to it and there is no diff to show
    git(&folder, &["checkout", "-q", "--", "src/app.py"]);
    assert!(app.until(cx, |w| said(w, "file-lines").as_deref() == Some("2 lines")), "the committed text never came");
    look(cx, &app, |w| {
        assert!(w.try_find("file-kind").is_none() && w.try_find("show-changes").is_none());
        // nothing is left to review, so leaving the file closes it
        assert_eq!(said(w, "close-file").as_deref(), Some("Close the file"));
    });
    // closed: Claude's terminal has the workspace again
    app.click(cx, "close-file");
    assert!(app.until(cx, |w| w.try_find("island-changes").is_none()), "the island stayed with nothing in it");
}

/// Item 3.5e: remarks on an opened file's lines, also where no diff shows the line.
#[gpui_kit::test]
fn a_remark_is_written_on_any_line_of_an_opened_file(cx: &mut TestAppContext) {
    let (app, _, ws) = start(cx);
    // a file without changes, opened as the Files panel will open it
    open(cx, &app, "docs/guide.md", None);
    assert!(app.until(cx, |w| line_number(w, "Line 7").is_some()), "its lines never showed");
    look(cx, &app, |w| {
        assert_eq!(said(w, "file-lines").as_deref(), Some("7 lines"));
        assert!(w.try_find("file-kind").is_none() && w.try_find("show-changes").is_none(), "it has no changes");
        assert_eq!(said(w, "close-file").as_deref(), Some("Back to the changes"));
    });

    // a press on line 6's number: the composer under it, with the keyboard
    click_line(cx, &app, "Line 6");
    assert!(app.until(cx, |w| said(w, "remark-composer").as_deref() == Some("Line 6")), "no composer");
    app.typed(cx, "Which tax?");
    app.click(cx, "save-remark");
    let saved = until_kept(cx, &app, &ws, 1);
    assert_eq!(saved.len(), 1, "the remark was never saved");
    let remark = &saved[0];
    assert_eq!(
        (remark.path.as_str(), remark.side.as_str(), remark.start_line, remark.end_line, remark.snippet.as_str()),
        ("docs/guide.md", "new", 6, 6, "2. Add the tax.")
    );
    let card = gpui_kit::SharedString::from(format!("remark-{}", remark.id));
    assert!(app.until(cx, |w| w.try_find(card.clone()).is_some() && w.try_find("remark-composer").is_none()));
    look(cx, &app, |w| {
        // the card is right under its line, and the band, the bar and the tab count it
        let (card, line) = (w.find(card.clone()).bounds(), at_line(w, "Line 6"));
        assert!(card.top() >= line && card.top() - line < gpui_kit::px(40.), "{card:?} under {line:?}");
        assert_eq!(said(w, "file-remarks").as_deref(), Some("1 remark"));
        assert_eq!(said(w, "review-bar").as_deref(), Some("1 remark not sent yet"));
        assert_eq!(said(w, ("tab-words", 0usize)).as_deref(), Some("1 remark not sent yet"));
    });

    // a drag from line 1 to line 3: a remark on the three lines
    let (from, to) = (at(cx, &app, "Line 1"), at(cx, &app, "Line 3"));
    press(cx, &app, from, to, false);
    assert!(app.until(cx, |w| said(w, "remark-composer").as_deref() == Some("Lines 1\u{2013}3")), "no range");
    app.typed(cx, "Say who reads this");
    app.press(cx, "cmd-enter");
    let saved = until_kept(cx, &app, &ws, 2);
    assert_eq!(saved.len(), 2, "the second remark was never saved");
    let range = saved.iter().find(|remark| remark.start_line == 1).expect("the range");
    assert_eq!((range.end_line, range.snippet.as_str()), (3, "# Guide\n\nHow totals work."));
    assert!(app.until(cx, |w| said(w, "file-remarks").as_deref() == Some("2 remarks")));

    // back in the diffs, the file is listed under them for its remarks, and opens at the first of them
    app.press(cx, "escape");
    assert!(app.until(cx, |w| all(w, "file-head") == ["src/app.py", "docs/guide.md"]), "not listed under the diffs");
    look(cx, &app, |w| {
        assert_eq!(all(w, "file-unchanged"), ["No changes"]);
        assert_eq!(all(w, "open-file"), ["Open src/app.py", "Open docs/guide.md"]);
    });
    let button = look(cx, &app, |w| w.find_all("open-file")[1].bounds().center());
    press(cx, &app, button, button, false);
    assert!(app.until(cx, |w| said(w, "file-view").as_deref() == Some("docs/guide.md")), "Open file did nothing");
    assert!(app.until(cx, |w| w.try_find(card.clone()).is_some()), "its remarks aren't in the file");

    // a remark is deleted there, as in a diff: the first of the two, on lines 1 to 3
    let bins =
        look(cx, &app, |w| w.find_all("delete-remark").iter().map(|bin| bin.bounds().center()).collect::<Vec<_>>());
    assert_eq!(bins.len(), 2);
    press(cx, &app, bins[0], bins[0], false);
    let left = until_kept(cx, &app, &ws, 1);
    assert_eq!(left.iter().map(|remark| remark.start_line).collect::<Vec<_>>(), [6], "the wrong one went");
    assert!(app.until(cx, |w| said(w, "file-remarks").as_deref() == Some("1 remark")));
}

/// The top of the line number that says `label`.
fn at_line(window: &Window, label: &str) -> gpui_kit::Pixels {
    (0..40usize)
        .flat_map(|ix| window.find_all(("line", ix)))
        .find(|number| number.label() == Some(label))
        .map(|number| number.bounds().top())
        .unwrap_or_else(|| panic!("no line number says {label:?}"))
}

#[gpui_kit::test]
fn a_remark_follows_its_code_in_an_opened_file_and_says_when_the_code_is_gone(cx: &mut TestAppContext) {
    let (app, folder, ws) = start(cx);
    app.click(cx, "open-file");
    assert!(app.until(cx, |w| line_number(w, "Line 5").is_some()), "its lines never showed");
    click_line(cx, &app, "Line 4");
    assert!(app.until(cx, |w| w.try_find("remark-composer").is_some()), "no composer");
    app.typed(cx, "Sum the prices with sum()");
    app.click(cx, "save-remark");
    let saved = until_kept(cx, &app, &ws, 1);
    assert_eq!(saved[0].snippet, "        result += item.price");
    let card = gpui_kit::SharedString::from(format!("remark-{}", saved[0].id));
    assert!(app.until(cx, |w| w.try_find(card.clone()).is_some()));

    // two lines are put in above it: the remark is under its code, now line 6
    std::fs::write(folder.join("src/app.py"), format!("# totals\n\n{CODE}")).unwrap();
    assert!(app.until(cx, |w| said(w, "file-lines").as_deref() == Some("7 lines")), "the file wasn't read again");
    assert!(
        app.until(cx, |w| {
            w.try_find(card.clone()).is_some_and(|card| {
                line_top(w, "Line 6").is_some_and(|line| card.bounds().top() >= line)
                    && line_top(w, "Line 7").is_some_and(|next| card.bounds().top() < next)
            })
        }),
        "the remark didn't follow its code"
    );
    look(cx, &app, |w| assert!(w.try_find("remark-outdated").is_none()));

    // its code is rewritten: it says so, above the file's lines, with the code it was written on
    std::fs::write(folder.join("src/app.py"), "def total(items):\n    return sum(item.price for item in items)\n")
        .unwrap();
    assert!(app.until(cx, |w| said(w, "file-lines").as_deref() == Some("2 lines")), "the file wasn't read again");
    assert!(app.until(cx, |w| said(w, "remark-outdated").as_deref() == Some("Code changed since")), "not outdated");
    look(cx, &app, |w| {
        let (card, first) = (w.find(card.clone()).bounds(), line_top(w, "Line 1").expect("line 1"));
        assert!(card.bottom() <= first, "an outdated remark stands above the lines: {card:?} {first:?}");
        assert_eq!(said(w, "file-remarks").as_deref(), Some("1 remark"));
    });
    assert_eq!(kept(&app, &ws).len(), 1);
}

fn line_top(window: &Window, label: &str) -> Option<gpui_kit::Pixels> {
    (0..40usize)
        .flat_map(|ix| window.find_all(("line", ix)))
        .find(|number| number.label() == Some(label))
        .map(|number| number.bounds().top())
}

#[gpui_kit::test]
fn an_opened_file_without_lines_to_show_says_why(cx: &mut TestAppContext) {
    let (app, folder, _) = start(cx);
    std::fs::write(folder.join("empty.txt"), "").unwrap();
    std::fs::write(folder.join("logo.png"), [0x89u8, b'P', b'N', b'G', 0, 1, 2, 3, 0, 0, 0, 0]).unwrap();
    std::fs::write(folder.join("big.txt"), "0123456789abcdef\n".repeat(100_000)).unwrap();
    let note = |cx: &mut TestAppContext, path: &str| {
        open(cx, &app, path, None);
        assert!(app.until(cx, |w| w.try_find("file-note").is_some()), "{path} says nothing");
        look(cx, &app, |w| {
            assert!(w.try_find("source").is_none() && w.try_find("file-lines").is_none(), "{path} shows lines");
            said(w, "file-note").unwrap_or_default()
        })
    };
    assert_eq!(note(cx, "empty.txt"), "This file is empty.");
    assert_eq!(note(cx, "logo.png"), "A binary file (12 bytes), not shown.");
    assert_eq!(note(cx, "big.txt"), "Too large to show here (1.6 MB).");
    assert_eq!(note(cx, "nowhere.py"), "This file isn't there any more.");
    // a file outside the workspace is never read
    let outside = note(cx, "../../outside.txt");
    assert!(!outside.is_empty() && outside != "This file is empty.", "{outside}");
    // and a file that is there shows its lines again
    open(cx, &app, "docs/guide.md", Some(5));
    assert!(app.until(cx, |w| line_number(w, "Line 5").is_some() && w.try_find("file-note").is_none()));
}

/// Where the line that says `text` is drawn (its code, not its number).
fn source_line(cx: &mut TestAppContext, app: &App, text: &str) -> gpui_kit::Bounds<gpui_kit::Pixels> {
    assert!(
        app.until(cx, |w| w.find_all("source").iter().any(|line| line.label().is_some_and(|said| said.trim() == text))),
        "no line says {text:?}: {:?}",
        look(cx, app, |w| lines(w))
    );
    look(cx, app, |w| {
        let lines = w.find_all("source");
        lines.iter().find(|line| line.label().is_some_and(|said| said.trim() == text)).unwrap().bounds()
    })
}

/// What is selected in the window, as ⌘C would copy it.
fn selected(cx: &mut TestAppContext, app: &App) -> String {
    cx.run_until_parked();
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        gpui_kit::base::TextSelection::selected_text(w, cx)
    })
    .unwrap()
}

/// The mouse goes down at `from`, moves to `to` in two steps and comes up there, with frames between, as a hand
/// drags over text.
fn drag(
    cx: &mut TestAppContext,
    app: &App,
    from: gpui_kit::Point<gpui_kit::Pixels>,
    to: gpui_kit::Point<gpui_kit::Pixels>,
) {
    use gpui_kit::{InputEvent as _, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent};
    cx.run_until_parked();
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        let (modifiers, left) = (Modifiers::default(), MouseButton::Left);
        w.dispatch_event(MouseMoveEvent { position: from, pressed_button: None, modifiers }.to_platform_input(), cx);
        w.render_frame(cx);
        let down = MouseDownEvent { button: left, position: from, modifiers, click_count: 1, first_mouse: false };
        w.dispatch_event(down.to_platform_input(), cx);
        w.render_frame(cx);
        let middle = gpui_kit::point((from.x + to.x) / 2., (from.y + to.y) / 2.);
        for position in [middle, to] {
            let moved = MouseMoveEvent { position, pressed_button: Some(left), modifiers };
            w.dispatch_event(moved.to_platform_input(), cx);
            w.render_frame(cx);
        }
        w.dispatch_event(
            MouseUpEvent { button: left, position: to, modifiers, click_count: 1 }.to_platform_input(),
            cx,
        );
        w.render_frame(cx);
    })
    .unwrap();
}

fn copied(cx: &mut TestAppContext) -> Option<String> {
    cx.read_from_clipboard().and_then(|item| item.text())
}

/// Items 7.5 and 7.5a: the code of a diff, the lines of an opened file and a remark's words can be selected with the
/// mouse, and ⌘C copies them.
#[gpui_kit::test]
fn a_diffs_code_an_opened_files_lines_and_a_remarks_words_can_be_selected_and_copied(cx: &mut TestAppContext) {
    let (app, _, ws) = start(cx);
    let (px, point) = (gpui_kit::px, gpui_kit::point::<gpui_kit::Pixels>);

    // a line of the diff: from inside the space before its first word to its end
    let line = source_line(cx, &app, "result = 0");
    let y = line.center().y;
    drag(cx, &app, point(line.left() + px(20.), y), point(line.right() - px(1.), y));
    assert_eq!(selected(cx, &app).trim(), "result = 0");
    app.press(cx, "cmd-c");
    assert_eq!(copied(cx).as_deref().map(str::trim), Some("result = 0"));
    // the diff has the keyboard now, not the terminal: ⌘C was its own
    assert_eq!(look(cx, &app, |w| w.find("terminal").focused()), Some(false));

    // a line of a file opened where the diffs are, whole: from its end back to its first letter (the first few
    // pixels of a line belong to the button that adds a remark, so a drag starts beside them)
    open(cx, &app, "docs/guide.md", None);
    let line = source_line(cx, &app, "How totals work.");
    let y = line.center().y;
    drag(cx, &app, point(line.left() + px(300.), y), point(line.left(), y));
    assert_eq!(selected(cx, &app).trim(), "How totals work.");
    app.press(cx, "cmd-c");
    assert_eq!(copied(cx).as_deref().map(str::trim), Some("How totals work."));
    // two lines at once, from the middle of one into the next
    let (first, second) = (source_line(cx, &app, "1. Sum the prices."), source_line(cx, &app, "2. Add the tax."));
    drag(cx, &app, point(first.left() + px(22.), first.center().y), point(second.left() + px(50.), second.center().y));
    let two = selected(cx, &app);
    assert!(two.starts_with("Sum the prices.\n2. Add") && !two.contains("tax"), "what was selected: {two:?}");

    // a remark's words, in its card under the line
    click_line(cx, &app, "Line 6");
    assert!(app.until(cx, |w| said(w, "remark-composer").as_deref() == Some("Line 6")), "no composer");
    app.typed(cx, "Which tax is meant here?");
    app.click(cx, "save-remark");
    let saved = until_kept(cx, &app, &ws, 1);
    let card = gpui_kit::SharedString::from(format!("remark-{}", saved[0].id));
    assert!(app.until(cx, |w| w.try_find(card.clone()).is_some() && w.try_find("remark-composer").is_none()));
    let card = look(cx, &app, |w| w.find(card.clone()).bounds());
    // its words are its last line
    let y = card.bottom() - px(16.);
    drag(cx, &app, point(card.left() + px(4.), y), point(card.right() - px(40.), y));
    assert_eq!(selected(cx, &app).trim(), "Which tax is meant here?");
    app.press(cx, "cmd-c");
    assert_eq!(copied(cx).as_deref().map(str::trim), Some("Which tax is meant here?"));
}

/// The marks beside the line numbers of the opened file, top to bottom: what each says.
fn change_marks(window: &Window) -> Vec<String> {
    (0..60usize)
        .flat_map(|line| [window.try_find(("change-mark", line)), window.try_find(("change-gone", line))])
        .flatten()
        .map(|mark| mark.label().unwrap_or_default().to_string())
        .collect()
}

/// Item 10.3: an opened file that has changes
/// carries a mark beside each line that differs from the last commit. The marks come from the diff the app reads
/// for its list of changes, and they change in the same frame as the text when Claude edits the file.
#[gpui_kit::test]
fn an_opened_file_marks_what_differs_from_the_last_commit_and_follows_claudes_edits(cx: &mut TestAppContext) {
    let (app, folder, _) = start(cx);
    // the last commit has six lines. Three things differ now: its first line is changed, its comment is gone, and
    // a line is new
    let before = "def total(items):\n    result = 0\n    # the prices\n    for item in items:\n        result += item.price\n    return result\n";
    let now = "def total(items, tax=0):\n    result = 0\n    for item in items:\n        result += item.price\n    result += tax\n    return result\n";
    std::fs::write(folder.join("src/app.py"), before).unwrap();
    git(&folder, &["commit", "-q", "-am", "totals"]);
    std::fs::write(folder.join("src/app.py"), now).unwrap();
    assert!(app.until(cx, |w| line_number(w, "Line 6, after").is_some()), "the diff of the new text never showed");

    open(cx, &app, "src/app.py", None);
    let want = ["Line 1 is changed", "Lines were removed after line 2", "Line 5 is new"];
    assert!(app.until(cx, |w| change_marks(w) == want), "the marks: {:?}", look(cx, &app, |w| change_marks(w)));
    look(cx, &app, |w| {
        assert_eq!(said(w, "file-kind").as_deref(), Some("Modified"));
        assert_eq!(said(w, "file-lines").as_deref(), Some("6 lines"));
        // each stripe stands between its line's number and its code
        let (number, stripe) = (w.find(("line", 0usize)), w.find(("change-mark", 1usize)));
        let code = w.find_all("source")[0].bounds();
        assert!(stripe.bounds().left() >= number.bounds().right(), "the stripe is over the line number");
        assert!(stripe.bounds().right() <= code.left(), "the stripe is over the code");
    });

    // Claude writes two more lines. The text and its marks change together: no frame has one without the other
    std::fs::write(folder.join("src/app.py"), format!("{now}\nprint(total([]))\n")).unwrap();
    let torn = std::cell::Cell::new(false);
    let followed = app.until(cx, |w| {
        let new_text = said(w, "file-lines").as_deref() == Some("8 lines");
        let new_marks = change_marks(w).iter().any(|mark| mark == "Line 8 is new");
        if new_text != new_marks {
            torn.set(true);
        }
        new_text && new_marks
    });
    assert!(followed, "the edit never showed: {:?}", look(cx, &app, |w| change_marks(w)));
    assert!(!torn.get(), "a frame showed the new text with the old marks, or the old text with the new ones");
    let want =
        ["Line 1 is changed", "Lines were removed after line 2", "Line 5 is new", "Line 7 is new", "Line 8 is new"];
    assert_eq!(look(cx, &app, |w| change_marks(w)), want);

    // a file that is all new has no marks: every line would have one, and its badge says what it is
    std::fs::write(folder.join("src/fresh.py"), "def fresh():\n    return 1\n").unwrap();
    assert!(app.until(cx, |w| said(w, "changes-count").as_deref() == Some("2 files")), "the new file isn't listed");
    open(cx, &app, "src/fresh.py", None);
    assert!(app.until(cx, |w| said(w, "file-lines").as_deref() == Some("2 lines")), "the new file never opened");
    look(cx, &app, |w| {
        assert_eq!(said(w, "file-kind").as_deref(), Some("New file"));
        assert_eq!(change_marks(w), Vec::<String>::new());
    });
    // and so has a file without changes
    open(cx, &app, "docs/guide.md", None);
    assert!(app.until(cx, |w| said(w, "file-lines").as_deref() == Some("7 lines")), "the guide never opened");
    look(cx, &app, |w| assert_eq!(change_marks(w), Vec::<String>::new()));

    // the changes are committed: the marks go with them
    open(cx, &app, "src/app.py", None);
    assert!(app.until(cx, |w| !change_marks(w).is_empty()), "back in the changed file");
    git(&folder, &["add", "-A"]);
    git(&folder, &["commit", "-q", "-m", "tax"]);
    assert!(app.until(cx, |w| w.try_find("file-kind").is_none() && change_marks(w).is_empty()), "after the commit");
    assert_eq!(look(cx, &app, |w| said(w, "file-lines")).as_deref(), Some("8 lines"));
}
