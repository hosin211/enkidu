//! Item 4.1: Search, in a real window on real git and the real core (its list of files, its index of definitions,
//! its scan of the files). One popup with three tabs, opened from the keyboard: Files (⌘P, ⇧⌘O), Symbols (⌘O),
//! Text (⇧⌘F); ⇧⇧ opens the tab used last. A result opens where the diffs are, at its line.

mod common;

use std::path::{Path, PathBuf};

use common::{App, said};
use enkidu_testkit::git;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Capslock, InputEvent as _, Modifiers, ModifiersChangedEvent, TestAppContext, Window};

fn write(folder: &Path, path: &str, text: &str) {
    let file = folder.join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, text).unwrap();
}

const STOCK: &str = "from decimal import Decimal\n\n\nclass StockLevelView:\n    def get_queryset(self):\n        return \
                     StockItem.objects.all()\n\n    def total(self):\n        return sum(item.stock for item in \
                     self.items)\n\n\ndef restock(order):\n    for line in order.lines:\n        line.item.stock += \
                     line.quantity\n";

/// The sample project with one workspace, where a few more files are committed, `src/app.py` is edited and `notes.md`
/// is new. Its `.env` is ignored.
fn start(cx: &mut TestAppContext) -> (App, PathBuf, String) {
    let app = common::start_on(cx, enkidu_testkit::shop, |shared, project, _| {
        App::make(shared, project, "stock-fix", "new", "stock-fix", Some("development"))
    });
    let (_, _, folder) = app.on_screen(cx).expect("on screen");
    let folder = PathBuf::from(folder);
    for path in ["README.md", "docs/stock-notes.md", "src/orders/serializers.py", "src/orders/tests/test_stock.py"] {
        write(&folder, path, "kept\n");
    }
    write(&folder, "src/orders/stock.py", STOCK);
    git(&folder, &["add", "-A"]);
    git(&folder, &["commit", "-q", "-m", "more files"]);
    write(&folder, ".env", "SECRET=1\n");
    write(&folder, "src/app.py", "def total(items):\n    return sum(item.price for item in items)\n");
    write(&folder, "notes.md", "# Notes\n");
    let ws = app.frame.read_with(cx, |f, _| f.current_workspace().unwrap().id.clone());
    assert!(app.until(cx, |w| said(w, "changes-count").as_deref() == Some("2 files")), "the changes never came");
    (app, folder, ws)
}

fn look<R>(cx: &mut TestAppContext, app: &App, at: impl FnOnce(&mut Window) -> R) -> R {
    cx.run_until_parked();
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        at(w)
    })
    .unwrap()
}

/// The popup's rows, top to bottom: a heading in brackets, a row that opens as what it says, with a star when it is
/// the one Enter opens.
fn rows(window: &Window) -> Vec<String> {
    let mut found = Vec::new();
    for ix in 0..120usize {
        if let Some(row) = window.try_find(("search-label", ix)) {
            found.push(format!("[{}]", row.label().unwrap_or_default()));
        }
        if let Some(row) = window.try_find(("search-row", ix)) {
            let star = if row.selected() == Some(true) { " *" } else { "" };
            found.push(format!("{}{star}", row.label().unwrap_or_default()));
        }
    }
    found
}

/// What stands at the right of each line of Find in files: its file's name and its number.
fn places(window: &Window) -> Vec<String> {
    (0..120usize)
        .filter_map(|ix| window.try_find(("search-place", ix)))
        .map(|place| place.label().unwrap_or_default().to_string())
        .collect()
}

/// The tab on screen, when the popup is open.
fn tab(window: &Window) -> Option<String> {
    window.try_find("search-popup").and_then(|popup| popup.description().map(String::from))
}

fn status(window: &Window) -> Option<String> {
    said(window, "search-status")
}

fn note(window: &Window) -> Option<String> {
    said(window, "search-note")
}

/// Search asks a moment after the last key, by the clock a test holds: the moment passes.
fn moment(cx: &mut TestAppContext) {
    cx.executor().advance_clock(std::time::Duration::from_millis(250));
    cx.run_until_parked();
}

/// Empties the popup's field, as ⌘A and Backspace do.
fn clear(cx: &mut TestAppContext, app: &App) {
    app.press(cx, "cmd-a");
    moment(cx);
    app.press(cx, "backspace");
    moment(cx);
}

/// Item 4.1b.
#[gpui_kit::test]
fn files_offers_the_changed_files_finds_a_file_by_name_and_opens_it_where_the_diffs_are(cx: &mut TestAppContext) {
    let (app, _, _) = start(cx);
    // no field on screen for it: the keyboard opens it
    look(cx, &app, |w| assert!(w.try_find("search-popup").is_none() && w.try_find("search-input").is_none()));
    app.press(cx, "cmd-p");
    moment(cx);
    assert!(app.until(cx, |w| tab(w).as_deref() == Some("Files")), "⌘P opened nothing");
    // before anything is typed: the changed files, the first one picked; and how many files there are
    assert!(
        app.until(cx, |w| rows(w) == ["[Changed]", "src/app.py *", "notes.md"]),
        "before typing: {:?}",
        look(cx, &app, |w| rows(w))
    );
    assert!(app.until(cx, |w| status(w).as_deref() == Some("8 files")), "{:?}", look(cx, &app, |w| status(w)));
    look(cx, &app, |w| {
        assert_eq!(note(w), None);
        assert_eq!(w.find(("search-tab", 0usize)).selected(), Some(true));
        assert_eq!(said(w, ("search-tab", 1usize)).as_deref(), Some("Symbols"));
        // the changed files say so, and what happened to them
        assert_eq!(w.find(("search-row", 1usize)).description(), Some("Changed"));
        assert_eq!(w.find_all("search-kind").len(), 2);
        // the popup stands a little above the middle, 680 px wide with its hairline edge
        let popup = w.find("search-popup").bounds();
        assert_eq!(popup.size.width, gpui_kit::px(678.));
        assert!(popup.top() > gpui_kit::px(90.) && popup.top() < gpui_kit::px(120.), "{popup:?}");
    });

    // typed: the file whose name starts with it first, then names that hold it; the ignored `.env` is never there
    app.typed(cx, "stock");
    moment(cx);
    assert!(
        app.until(cx, |w| {
            rows(w) == ["src/orders/stock.py *", "docs/stock-notes.md", "src/orders/tests/test_stock.py"]
        }),
        "the matches: {:?}",
        look(cx, &app, |w| rows(w))
    );
    assert_eq!(look(cx, &app, |w| status(w)).as_deref(), Some("3 of 8 files"));
    // ↓ and ↑ move, and stop at the ends
    app.press(cx, "down");
    moment(cx);
    assert!(app.until(cx, |w| rows(w)[1].ends_with(" *")));
    app.press(cx, "down");
    moment(cx);
    app.press(cx, "down");
    moment(cx);
    assert!(app.until(cx, |w| rows(w)[2].ends_with(" *")), "↓ at the last row: {:?}", look(cx, &app, |w| rows(w)));
    app.press(cx, "up");
    moment(cx);
    app.press(cx, "up");
    moment(cx);
    app.press(cx, "up");
    moment(cx);
    assert!(app.until(cx, |w| rows(w)[0].ends_with(" *")));
    // nothing matches: it says so
    app.typed(cx, "zzz");
    moment(cx);
    assert!(
        app.until(cx, |w| note(w).as_deref() == Some("No file matches \u{201c}stockzzz\u{201d}.")),
        "{:?}",
        look(cx, &app, |w| note(w))
    );
    assert!(look(cx, &app, |w| rows(w)).is_empty());
    for _ in 0..3 {
        app.press(cx, "backspace");
        moment(cx);
    }
    assert!(app.until(cx, |w| rows(w).len() == 3 && note(w).is_none()));

    // Enter opens the row picked where the diffs are, and the popup is gone; the keyboard is in the file
    app.press(cx, "enter");
    moment(cx);
    assert!(app.until(cx, |w| w.try_find("search-popup").is_none()), "the popup stayed");
    assert!(app.until(cx, |w| said(w, "file-view").as_deref() == Some("src/orders/stock.py")), "the file never opened");
    app.press(cx, "escape");
    moment(cx);
    assert!(app.until(cx, |w| w.try_find("file-view").is_none()), "Escape didn't leave the file");

    // ⇧⌘O opens it too. What was typed is there, selected: the matches show, and typing replaces it
    app.press(cx, "cmd-shift-o");
    moment(cx);
    assert!(app.until(cx, |w| tab(w).as_deref() == Some("Files")), "⇧⌘O opened nothing");
    assert!(
        app.until(cx, |w| rows(w).len() == 3),
        "what was typed last time is gone: {:?}",
        look(cx, &app, |w| rows(w))
    );
    app.typed(cx, "app");
    moment(cx);
    assert!(app.until(cx, |w| rows(w) == ["src/app.py *"]), "{:?}", look(cx, &app, |w| rows(w)));
    // emptied: the changed files, then the file that was opened lately
    clear(cx, &app);
    assert!(
        app.until(cx, |w| {
            rows(w) == ["[Changed]", "src/app.py *", "notes.md", "[Opened lately]", "src/orders/stock.py"]
        }),
        "before typing, the second time: {:?}",
        look(cx, &app, |w| rows(w))
    );
    // the pointer picks a row as it moves over it, and a click opens it
    app.click(cx, ("search-row", 4usize));
    moment(cx);
    assert!(
        app.until(cx, |w| said(w, "file-view").as_deref() == Some("src/orders/stock.py")),
        "a click opened nothing"
    );
    look(cx, &app, |w| assert!(w.try_find("search-popup").is_none()));

    // Escape closes the popup and nothing opens
    app.press(cx, "escape");
    moment(cx);
    assert!(app.until(cx, |w| w.try_find("file-view").is_none()));
    app.press(cx, "cmd-p");
    moment(cx);
    assert!(app.until(cx, |w| tab(w).is_some()));
    app.press(cx, "escape");
    moment(cx);
    assert!(app.until(cx, |w| w.try_find("search-popup").is_none()), "Escape left the popup open");
    look(cx, &app, |w| assert!(w.try_find("file-view").is_none()));
}

/// Item 4.1b: the files opened lately are kept across a restart, the last one first.
#[gpui_kit::test]
fn the_files_opened_lately_are_there_after_a_restart(cx: &mut TestAppContext) {
    let (app, _, ws) = start(cx);
    for name in ["stock.py", "README"] {
        app.press(cx, "cmd-p");
        moment(cx);
        assert!(app.until(cx, |w| tab(w).is_some()));
        clear(cx, &app);
        app.typed(cx, name);
        moment(cx);
        assert!(app.until(cx, |w| rows(w).first().is_some_and(|row| row.contains(name))), "{name} wasn't found");
        app.press(cx, "enter");
        moment(cx);
        assert!(app.until(cx, |w| w.try_find("search-popup").is_none() && w.try_find("file-view").is_some()));
    }
    // kept in the data folder a moment after the last one
    let kept = app.home.path().join("data").join("recent.json");
    let want = format!(r#"{{"{ws}":["README.md","src/orders/stock.py"]}}"#);
    for _ in 0..600 {
        cx.executor().advance_clock(std::time::Duration::from_millis(400));
        cx.run_until_parked();
        if std::fs::read_to_string(&kept).is_ok_and(|text| text == want) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(std::fs::read_to_string(&kept).unwrap_or_default(), want);

    let app = common::restart_as(cx, app, false);
    assert!(app.until(cx, |w| said(w, "changes-count").as_deref() == Some("2 files")), "the changes never came back");
    app.press(cx, "cmd-p");
    moment(cx);
    assert!(app.until(cx, |w| tab(w).as_deref() == Some("Files")), "⌘P opened nothing after the restart");
    assert!(
        app.until(cx, |w| {
            rows(w) == ["[Changed]", "src/app.py *", "notes.md", "[Opened lately]", "README.md", "src/orders/stock.py"]
        }),
        "after the restart: {:?}",
        look(cx, &app, |w| rows(w))
    );
}

/// Whether a line that says `text` is on screen as the line `which` ("Line 8, marked").
fn has_line(window: &Window, text: &str, which: &str) -> bool {
    window.find_all("source").iter().any(|line| line.label() == Some(text) && line.description() == Some(which))
}

/// What each row that opens says about itself, top to bottom.
fn details(window: &Window) -> Vec<String> {
    (0..120usize)
        .filter_map(|ix| window.try_find(("search-row", ix)))
        .map(|row| row.description().unwrap_or_default().to_string())
        .collect()
}

/// Item 4.1c.
#[gpui_kit::test]
fn symbols_finds_a_definition_by_its_letters_in_order_and_opens_the_file_at_its_line(cx: &mut TestAppContext) {
    let (app, _, _) = start(cx);
    app.press(cx, "cmd-o");
    moment(cx);
    assert!(app.until(cx, |w| tab(w).as_deref() == Some("Symbols")), "⌘O opened nothing");
    // until something is typed: how to ask
    assert!(
        app.until(cx, |w| {
            note(w).as_deref()
                == Some("Type a class, function or method name. Letters in order are enough: slv finds StockLevelView.")
        }),
        "{:?}",
        look(cx, &app, |w| note(w))
    );
    // letters in order are enough
    app.typed(cx, "slv");
    moment(cx);
    assert!(app.until(cx, |w| rows(w) == ["StockLevelView *"]), "slv found {:?}", look(cx, &app, |w| rows(w)));
    look(cx, &app, |w| {
        assert_eq!(details(w), ["class, src/orders/stock.py:4"]);
        assert_eq!(note(w), None);
        let said = status(w).unwrap_or_default();
        assert!(said.starts_with("1 found in ") && said.ends_with(" definitions"), "{said}");
        assert_eq!(w.find("search-status").description(), Some("done"));
    });

    // a name that two definitions have: the function before the method, each with its place
    clear(cx, &app);
    app.typed(cx, "total");
    moment(cx);
    assert!(app.until(cx, |w| rows(w) == ["total *", "total"]), "total found {:?}", look(cx, &app, |w| rows(w)));
    assert_eq!(
        look(cx, &app, |w| details(w)),
        ["function, src/app.py:1", "method in StockLevelView, src/orders/stock.py:8"]
    );
    // ↓ and Enter: the file opens where the diffs are, at the definition's line, which is marked
    app.press(cx, "down");
    moment(cx);
    app.press(cx, "enter");
    moment(cx);
    assert!(app.until(cx, |w| w.try_find("search-popup").is_none()), "the popup stayed");
    assert!(app.until(cx, |w| said(w, "file-view").as_deref() == Some("src/orders/stock.py")), "the file never opened");
    assert!(
        app.until(cx, |w| has_line(w, "    def total(self):", "Line 8, marked")),
        "line 8 isn't marked: {:?}",
        look(cx, &app, |w| {
            w.find_all("source")
                .iter()
                .map(|line| line.description().unwrap_or_default().to_string())
                .collect::<Vec<_>>()
        })
    );
    look(cx, &app, |w| assert!(has_line(w, "class StockLevelView:", "Line 4"), "another line is marked too"));
    app.press(cx, "escape");
    moment(cx);
    assert!(app.until(cx, |w| w.try_find("file-view").is_none()));

    // ⌥⌘O opens it too, with what was typed; nothing matches: it says so
    app.press(cx, "alt-cmd-o");
    moment(cx);
    assert!(app.until(cx, |w| tab(w).as_deref() == Some("Symbols") && rows(w).len() == 2), "⌥⌘O");
    app.typed(cx, "zzzz");
    moment(cx);
    assert!(
        app.until(cx, |w| note(w).as_deref() == Some("No definition matches \u{201c}zzzz\u{201d}.")),
        "{:?}",
        look(cx, &app, |w| (note(w), rows(w)))
    );
    // the field emptied: the hint again, and how many definitions the workspace has
    clear(cx, &app);
    assert!(app.until(cx, |w| note(w).is_some_and(|note| note.starts_with("Type a class"))));
    let said = look(cx, &app, |w| status(w)).unwrap_or_default();
    assert!(said.contains(" definitions in ") && said.ends_with(" files"), "{said}");
}

/// Items 4.1d, 10.4, 10.5 and 10.8.
#[gpui_kit::test]
fn text_lists_one_row_for_each_line_with_its_file_and_opens_the_line_with_what_was_found_marked(
    cx: &mut TestAppContext,
) {
    let (app, folder, _) = start(cx);
    app.press(cx, "cmd-shift-f");
    moment(cx);
    assert!(app.until(cx, |w| tab(w).as_deref() == Some("Text")), "⇧⌘F opened nothing");
    assert!(app.until(cx, |w| note(w).is_some_and(|note| note.starts_with("Searches the workspace's files as they"))));
    look(cx, &app, |w| {
        // a line of code with its file's name beside it needs more room than a name
        assert_eq!(w.find("search-popup").bounds().size.width, gpui_kit::px(918.));
        assert_eq!(said(w, "toggle-case").as_deref(), Some("Match case"));
        assert_eq!(w.find("toggle-case").selected(), Some(false));
        // every kind of file, until one is picked
        assert_eq!(said(w, "search-kind").as_deref(), Some("All files"));
    });

    // one row for each line with a match, the first one picked: the line, then its file's name and its number at
    // the right. No heading for the file, no count. Without "Match case" a small letter finds capitals too
    app.typed(cx, "stock");
    moment(cx);
    let all = [
        "class StockLevelView: *",
        "return StockItem.objects.all()",
        "return sum(item.stock for item in self.items)",
        "def restock(order):",
        "line.item.stock += line.quantity",
    ];
    assert!(app.until(cx, |w| rows(w) == all), "stock found {:?}", look(cx, &app, |w| rows(w)));
    assert!(
        app.until(cx, |w| status(w).as_deref() == Some("5 lines in 1 file")),
        "no time is said: {:?}",
        look(cx, &app, |w| status(w))
    );
    look(cx, &app, |w| {
        assert_eq!(places(w), ["stock.py 4", "stock.py 6", "stock.py 9", "stock.py 12", "stock.py 14"]);
        assert_eq!(
            details(w),
            [
                "src/orders/stock.py:4, matched: Stock",
                "src/orders/stock.py:6, matched: Stock",
                "src/orders/stock.py:9, matched: stock",
                "src/orders/stock.py:12, matched: stock",
                "src/orders/stock.py:14, matched: stock",
            ]
        );
        assert_eq!(note(w), None);
        // the file and the number stand at the right end of the row, after the line
        let (row, place) = (w.find(("search-row", 0usize)).bounds(), w.find(("search-place", 0usize)).bounds());
        assert!(place.right() > row.right() - gpui_kit::px(16.) && place.left() > row.center().x);
    });

    // "Match case" by its key: only the small letters
    app.press(cx, "alt-cmd-c");
    moment(cx);
    assert!(app.until(cx, |w| rows(w).len() == 3), "⌥⌘C: {:?}", look(cx, &app, |w| rows(w)));
    look(cx, &app, |w| {
        assert_eq!(w.find("toggle-case").selected(), Some(true));
        assert_eq!(rows(w)[0], "return sum(item.stock for item in self.items) *");
    });
    // "Whole words" by its button: "restock" isn't one
    app.click(cx, "toggle-word");
    moment(cx);
    assert!(
        app.until(cx, |w| {
            rows(w) == ["return sum(item.stock for item in self.items) *", "line.item.stock += line.quantity"]
        }),
        "whole words: {:?}",
        look(cx, &app, |w| rows(w))
    );
    app.press(cx, "alt-cmd-w");
    moment(cx);
    app.press(cx, "alt-cmd-c");
    moment(cx);
    assert!(app.until(cx, |w| rows(w) == all), "the switches off again: {:?}", look(cx, &app, |w| rows(w)));

    // a regular expression, by its key: without it the same text is looked for as it is, and nothing has it
    clear(cx, &app);
    app.typed(cx, "re?stock\\(");
    moment(cx);
    assert!(
        app.until(cx, |w| note(w).as_deref() == Some("Nothing found for \u{201c}re?stock\\(\u{201d}.")),
        "{:?}",
        look(cx, &app, |w| (note(w), rows(w)))
    );
    app.press(cx, "alt-cmd-r");
    moment(cx);
    assert!(
        app.until(cx, |w| rows(w) == ["def restock(order): *"]),
        "the regular expression: {:?}",
        look(cx, &app, |w| rows(w))
    );
    assert_eq!(look(cx, &app, |w| details(w)), ["src/orders/stock.py:12, matched: restock("]);
    // one the engine refuses: its own words, and no lines
    app.typed(cx, "(");
    moment(cx);
    assert!(
        app.until(cx, |w| note(w).is_some_and(|note| note.contains("unclosed group")) && rows(w).is_empty()),
        "a broken pattern: {:?}",
        look(cx, &app, |w| (note(w), rows(w)))
    );
    app.press(cx, "alt-cmd-r");
    moment(cx);

    // ↓ and Enter open the file at the line, and what was found is marked there
    clear(cx, &app);
    app.typed(cx, "stock");
    moment(cx);
    assert!(app.until(cx, |w| rows(w) == all));
    app.press(cx, "down");
    moment(cx);
    app.press(cx, "down");
    moment(cx);
    app.press(cx, "enter");
    moment(cx);
    assert!(app.until(cx, |w| w.try_find("search-popup").is_none()), "the popup stayed");
    assert!(app.until(cx, |w| said(w, "file-view").as_deref() == Some("src/orders/stock.py")), "the file never opened");
    assert!(
        app.until(cx, |w| {
            has_line(w, "        return sum(item.stock for item in self.items)", "Line 9, marked: stock")
        }),
        "line 9 isn't marked: {:?}",
        look(cx, &app, |w| {
            w.find_all("source")
                .iter()
                .map(|line| line.description().unwrap_or_default().to_string())
                .collect::<Vec<_>>()
        })
    );
    app.click(cx, "close-file");
    assert!(app.until(cx, |w| w.try_find("file-view").is_none()), "the file stayed open");
    let _ = folder;
}

/// Items 10.4, 10.5 and 10.8. A new file git doesn't track yet is
/// read, in a new folder too; a file git ignores is not. The line that defines the name that is looked for stands
/// first. The filter keeps the scan to one kind of file.
#[gpui_kit::test]
fn text_reads_the_unversioned_files_puts_the_defining_line_first_and_keeps_to_a_kind_of_file(cx: &mut TestAppContext) {
    let (app, folder, _) = start(cx);
    // two files git has never seen, one of them in a folder it has never seen, and one more line in the ignored
    // `.env`. The new folder sorts before `src`, where the name is defined
    write(&folder, "aaa/first/notes.txt", "restock is called when an order comes back\n");
    write(&folder, "zzz.md", "# restock\n");
    write(&folder, ".env", "SECRET=1\nRESTOCK_TOKEN=restock-me\n");
    assert!(app.until(cx, |w| said(w, "changes-count").as_deref() == Some("4 files")), "the new files aren't listed");

    app.press(cx, "cmd-shift-f");
    moment(cx);
    assert!(app.until(cx, |w| tab(w).as_deref() == Some("Text")), "⇧⌘F opened nothing");
    app.typed(cx, "restock");
    moment(cx);
    // the line that defines restock first, though its file comes after the others by path; then the unversioned
    // files' lines. Nothing of `.env`
    let all = ["def restock(order): *", "restock is called when an order comes back", "# restock"];
    assert!(app.until(cx, |w| rows(w) == all), "restock found {:?}", look(cx, &app, |w| rows(w)));
    look(cx, &app, |w| {
        assert_eq!(places(w), ["stock.py 12", "notes.txt 1", "zzz.md 1"]);
        assert_eq!(details(w)[1], "aaa/first/notes.txt:1, matched: restock");
    });
    assert!(
        app.until(cx, |w| status(w).as_deref() == Some("3 lines in 3 files")),
        "{:?}",
        look(cx, &app, |w| status(w))
    );

    // the filter: the kinds of file this workspace has, the most common first, each with how many files
    app.click(cx, "search-kind");
    moment(cx);
    assert!(app.until(cx, |w| w.try_find("search-kind-option:py").is_some()), "the filter's menu never opened");
    look(cx, &app, |w| {
        let every = w.find("search-kind-option:");
        assert_eq!((every.label(), every.selected()), (Some("All files"), Some(true)));
        let py = w.find("search-kind-option:py");
        assert_eq!((py.label(), py.selected()), (Some("*.py"), Some(false)));
        // the unversioned files' kinds are offered too, an ignored file's kind is not
        assert_eq!(w.find("search-kind-option:txt").description(), Some("1"));
        assert!(w.try_find("search-kind-option:md").is_some());
        assert!(w.try_find("search-kind-option:env").is_none(), "an ignored file's kind is offered");
    });
    app.click(cx, "search-kind-option:txt");
    moment(cx);
    let only = ["restock is called when an order comes back *"];
    assert!(app.until(cx, |w| rows(w) == only), "*.txt found {:?}", look(cx, &app, |w| rows(w)));
    assert!(app.until(cx, |w| status(w).as_deref() == Some("1 line in 1 file")));
    look(cx, &app, |w| {
        assert_eq!(said(w, "search-kind").as_deref(), Some("*.txt"));
        // the keyboard is back in the field: typing goes on
        assert_eq!(w.find("search-input").focused(), Some(true));
    });
    // typing on keeps to that kind
    app.typed(cx, " is");
    moment(cx);
    assert!(app.until(cx, |w| rows(w) == only), "typing on: {:?}", look(cx, &app, |w| rows(w)));
    // a kind without a match says so, with the kind
    app.click(cx, "search-kind");
    moment(cx);
    assert!(app.until(cx, |w| w.try_find("search-kind-option:py").is_some()));
    app.click(cx, "search-kind-option:py");
    moment(cx);
    assert!(
        app.until(cx, |w| note(w).as_deref() == Some("Nothing found for \u{201c}restock is\u{201d} in *.py files.")),
        "{:?}",
        look(cx, &app, |w| (note(w), rows(w)))
    );
    // every file again
    app.click(cx, "search-kind");
    moment(cx);
    assert!(app.until(cx, |w| w.try_find("search-kind-option:").is_some()));
    app.click(cx, "search-kind-option:");
    moment(cx);
    assert!(app.until(cx, |w| rows(w) == only && said(w, "search-kind").as_deref() == Some("All files")));

    // what git ignores is never read
    clear(cx, &app);
    app.typed(cx, "RESTOCK_TOKEN");
    moment(cx);
    assert!(
        app.until(cx, |w| note(w).as_deref() == Some("Nothing found for \u{201c}RESTOCK_TOKEN\u{201d}.")),
        "an ignored file was read: {:?}",
        look(cx, &app, |w| (note(w), rows(w)))
    );
    // and the unversioned file is in the Files tab, by its name
    app.click(cx, ("search-tab", 0usize));
    moment(cx);
    app.press(cx, "cmd-a");
    app.typed(cx, "notes.txt");
    moment(cx);
    assert!(
        app.until(cx, |w| rows(w).first().is_some_and(|row| row == "aaa/first/notes.txt *")),
        "{:?}",
        look(cx, &app, |w| rows(w))
    );
    look(cx, &app, |w| assert!(!rows(w).iter().any(|row| row.contains(".env")), "the ignored file is offered"));
}

/// The modifier keys change, as the keyboard says it.
fn modifiers(cx: &mut TestAppContext, app: &App, held: Modifiers) {
    cx.update_window(app.window, |_, w, cx| {
        let event = ModifiersChangedEvent { modifiers: held, capslock: Capslock { on: false } };
        w.dispatch_event(event.to_platform_input(), cx);
    })
    .unwrap();
    cx.run_until_parked();
}

/// Shift goes down and up again.
fn tap_shift(cx: &mut TestAppContext, app: &App) {
    modifiers(cx, app, Modifiers::shift());
    modifiers(cx, app, Modifiers::default());
}

fn open_now(cx: &mut TestAppContext, app: &App) -> bool {
    look(cx, app, |w| w.try_find("search-popup").is_some())
}

/// Item 4.1e.
#[gpui_kit::test]
fn shift_tapped_twice_opens_the_tab_used_last_and_tab_goes_to_the_next(cx: &mut TestAppContext) {
    let (app, _, _) = start(cx);
    // two clean taps of Shift, as in PyCharm: the first time, Files
    tap_shift(cx, &app);
    assert!(!open_now(cx, &app), "one tap opened it");
    tap_shift(cx, &app);
    assert!(app.until(cx, |w| tab(w).as_deref() == Some("Files")), "⇧⇧ opened nothing");

    // ⇥ goes to the next tab and ⇧⇥ back, around the ends; what was typed goes along
    app.typed(cx, "total");
    moment(cx);
    assert!(app.until(cx, |w| rows(w).is_empty() && note(w).is_some()), "{:?}", look(cx, &app, |w| rows(w)));
    app.press(cx, "tab");
    moment(cx);
    assert!(app.until(cx, |w| tab(w).as_deref() == Some("Symbols") && rows(w) == ["total *", "total"]), "⇥");
    app.press(cx, "tab");
    moment(cx);
    assert!(
        // one row for each line: the two functions named total
        app.until(cx, |w| tab(w).as_deref() == Some("Text") && rows(w).len() == 2),
        "⇥ again: {:?}",
        look(cx, &app, |w| (tab(w), rows(w)))
    );
    app.press(cx, "tab");
    moment(cx);
    assert!(app.until(cx, |w| tab(w).as_deref() == Some("Files")), "⇥ on the last tab");
    app.press(cx, "shift-tab");
    moment(cx);
    assert!(app.until(cx, |w| tab(w).as_deref() == Some("Text")), "⇧⇥ on the first tab");
    // a tab's own button, and the keys that open Search, go to their tab
    app.click(cx, ("search-tab", 0usize));
    moment(cx);
    assert!(app.until(cx, |w| tab(w).as_deref() == Some("Files")));
    app.press(cx, "cmd-shift-f");
    moment(cx);
    assert!(app.until(cx, |w| tab(w).as_deref() == Some("Text")), "⇧⌘F while it is open");
    app.press(cx, "cmd-p");
    moment(cx);
    assert!(app.until(cx, |w| tab(w).as_deref() == Some("Files")), "⌘P while it is open");
    app.press(cx, "cmd-o");
    moment(cx);
    assert!(app.until(cx, |w| tab(w).as_deref() == Some("Symbols")), "⌘O while it is open");
    // the field kept the keyboard through all of it, and on each tab what was typed is selected: typing replaces it
    app.typed(cx, "x");
    moment(cx);
    assert!(
        app.until(cx, |w| note(w).as_deref() == Some("No definition matches \u{201c}x\u{201d}.")),
        "{:?}",
        look(cx, &app, |w| note(w))
    );
    clear(cx, &app);
    app.typed(cx, "total");
    moment(cx);
    assert!(app.until(cx, |w| rows(w) == ["total *", "total"]));
    app.press(cx, "escape");
    moment(cx);
    assert!(app.until(cx, |w| w.try_find("search-popup").is_none()));

    // ⇧⇧ again: the tab used last, with what was typed (and its answers, a moment later)
    tap_shift(cx, &app);
    tap_shift(cx, &app);
    moment(cx);
    assert!(
        app.until(cx, |w| tab(w).as_deref() == Some("Symbols") && rows(w) == ["total *", "total"]),
        "⇧⇧ the second time: {:?}",
        look(cx, &app, |w| (tab(w), rows(w)))
    );
    app.press(cx, "escape");
    moment(cx);
    assert!(app.until(cx, |w| w.try_find("search-popup").is_none()));

    // a Shift held for a capital letter never opens it, nor the tap after it
    modifiers(cx, &app, Modifiers::shift());
    app.press(cx, "shift-a");
    moment(cx);
    modifiers(cx, &app, Modifiers::default());
    tap_shift(cx, &app);
    assert!(!open_now(cx, &app), "a capital letter and a tap opened Search");
    // nor a key typed between two taps
    cx.executor().advance_clock(std::time::Duration::from_secs(1));
    tap_shift(cx, &app);
    app.press(cx, "a");
    moment(cx);
    tap_shift(cx, &app);
    assert!(!open_now(cx, &app), "a key between the taps");
    // nor Shift and a click (a range of lines is picked that way)
    cx.executor().advance_clock(std::time::Duration::from_secs(1));
    modifiers(cx, &app, Modifiers::shift());
    app.click(cx, "terminal");
    moment(cx);
    modifiers(cx, &app, Modifiers::default());
    tap_shift(cx, &app);
    assert!(!open_now(cx, &app), "Shift with a click, then a tap");
    // nor a Shift that was held for a while
    cx.executor().advance_clock(std::time::Duration::from_secs(1));
    modifiers(cx, &app, Modifiers::shift());
    cx.executor().advance_clock(std::time::Duration::from_millis(400));
    modifiers(cx, &app, Modifiers::default());
    tap_shift(cx, &app);
    assert!(!open_now(cx, &app), "a long press, then a tap");
    // nor two taps too far apart
    cx.executor().advance_clock(std::time::Duration::from_secs(1));
    tap_shift(cx, &app);
    cx.executor().advance_clock(std::time::Duration::from_millis(450));
    tap_shift(cx, &app);
    assert!(!open_now(cx, &app), "two taps 450 ms apart");
    // nor Shift with another modifier (⇧⌘ and a key that does nothing here)
    cx.executor().advance_clock(std::time::Duration::from_secs(1));
    for _ in 0..2 {
        modifiers(cx, &app, Modifiers::shift());
        modifiers(cx, &app, Modifiers { shift: true, platform: true, ..Default::default() });
        modifiers(cx, &app, Modifiers::default());
    }
    assert!(!open_now(cx, &app), "Shift and ⌘, twice");
    // and after all that, two clean taps still do
    cx.executor().advance_clock(std::time::Duration::from_secs(1));
    tap_shift(cx, &app);
    tap_shift(cx, &app);
    assert!(app.until(cx, |w| tab(w).as_deref() == Some("Symbols")), "two clean taps after all that");
}

/// Item 4.2a: the palette offers Search's three tabs as commands, and each opens Search on its tab with the keyboard
/// in its field.
#[gpui_kit::test]
fn the_palette_opens_search_on_each_of_its_tabs(cx: &mut TestAppContext) {
    let (app, _, _) = start(cx);
    for (typed, command, on) in [
        ("go to file", "Go to file…", "Files"),
        ("go to symbol", "Go to symbol (class, function, method)…", "Symbols"),
        ("find in files", "Find in files…", "Text"),
    ] {
        app.press(cx, "cmd-k");
        assert!(app.until(cx, |w| w.try_find("command-palette").is_some()), "⌘K opened nothing");
        app.typed(cx, typed);
        assert!(
            app.until(cx, |w| said(w, ("palette-row", 0usize)).as_deref() == Some(command)),
            "{typed}: the first row says {:?}",
            look(cx, &app, |w| said(w, ("palette-row", 0usize)))
        );
        app.press(cx, "enter");
        assert!(
            app.until(cx, |w| w.try_find("command-palette").is_none() && tab(w).as_deref() == Some(on)),
            "{command} opened {:?}",
            look(cx, &app, |w| tab(w))
        );
        // its field has the keyboard
        app.typed(cx, "zzzq");
        moment(cx);
        assert!(
            app.until(cx, |w| note(w).is_some_and(|note| note.contains("zzzq"))),
            "typing after {command}: {:?}",
            look(cx, &app, |w| note(w))
        );
        app.press(cx, "escape");
        assert!(app.until(cx, |w| w.try_find("search-popup").is_none()));
    }
}

/// The project of the two Search runs: the shop with "stock-fix" on a new branch from development, `src/app.py` as it
/// was committed.
fn start_shop(cx: &mut TestAppContext) -> (App, PathBuf, String) {
    let app = common::start_on(cx, enkidu_testkit::shop, |shared, project, _| {
        App::make(shared, project, "stock-fix", "new", "stock-fix", Some("development"))
    });
    let (_, _, folder) = app.on_screen(cx).expect("on screen");
    let ws = app.frame.read_with(cx, |f, _| f.current_workspace().unwrap().id.clone());
    assert!(app.until(cx, |w| w.try_find("island-claude").is_some()), "the workspace never came on screen");
    (app, PathBuf::from(folder), ws)
}

/// Finds a file with Search, opened by Shift twice or ⌘P.
#[gpui_kit::test]
fn finds_a_file_with_search_opened_by_shift_twice_or_cmd_p(cx: &mut TestAppContext) {
    let (app, _, _) = start_shop(cx);
    // two clean taps of Shift, as in PyCharm
    tap_shift(cx, &app);
    tap_shift(cx, &app);
    moment(cx);
    assert!(app.until(cx, |w| tab(w).as_deref() == Some("Files")), "⇧⇧ opened {:?}", look(cx, &app, |w| tab(w)));
    app.typed(cx, "app");
    moment(cx);
    assert!(
        app.until(cx, |w| rows(w).first().map(String::as_str) == Some("src/app.py *")),
        "app found {:?}",
        look(cx, &app, |w| rows(w))
    );
    app.press(cx, "enter");
    moment(cx);
    assert!(app.until(cx, |w| w.try_find("search-popup").is_none()), "the popup stayed");
    assert!(app.until(cx, |w| said(w, "file-view").as_deref() == Some("src/app.py")), "the file never opened");

    // a Shift held for a capital letter never opens it
    modifiers(cx, &app, Modifiers::shift());
    app.press(cx, "shift-a");
    modifiers(cx, &app, Modifiers::default());
    tap_shift(cx, &app);
    moment(cx);
    for _ in 0..15 {
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(!open_now(cx, &app), "a Shift held for a capital letter opened Search");

    // ⌘P too; before typing it lists the file just opened
    app.press(cx, "cmd-p");
    moment(cx);
    assert!(app.until(cx, |w| tab(w).as_deref() == Some("Files")), "⌘P opened nothing");
    clear(cx, &app);
    assert!(
        app.until(cx, |w| rows(w).iter().any(|row| row.trim_end_matches(" *") == "src/app.py")),
        "the file just opened isn't offered: {:?}",
        look(cx, &app, |w| rows(w))
    );
    assert!(look(cx, &app, |w| rows(w)).contains(&"[Opened lately]".to_string()));
    app.press(cx, "escape");
    moment(cx);
    assert!(app.until(cx, |w| w.try_find("search-popup").is_none()), "Escape left Search open");
    app.click(cx, "close-file");
    assert!(app.until(cx, |w| w.try_find("file-view").is_none()), "the file stayed open");
}

/// Goes to a symbol (⌘O) and finds text in files (⇧⌘F), each straight to its line.
#[gpui_kit::test]
fn goes_to_a_symbol_cmd_o_and_finds_text_in_files_shift_cmd_f_each_straight_to_its_line(cx: &mut TestAppContext) {
    let (app, folder, ws) = start_shop(cx);
    // Go to symbol: the function total in src/app.py, from the core's index
    app.press(cx, "cmd-o");
    moment(cx);
    assert!(app.until(cx, |w| tab(w).as_deref() == Some("Symbols")), "⌘O opened nothing");
    app.typed(cx, "total");
    moment(cx);
    assert!(app.until(cx, |w| rows(w) == ["total *"]), "total found {:?}", look(cx, &app, |w| rows(w)));
    assert_eq!(look(cx, &app, |w| details(w)), ["function, src/app.py:1"]);
    app.press(cx, "enter");
    moment(cx);
    assert!(app.until(cx, |w| said(w, "file-view").as_deref() == Some("src/app.py")), "the file never opened");
    assert!(app.until(cx, |w| has_line(w, "def total(items):", "Line 1, marked")), "line 1 isn't marked");

    // Find in files, by the real engine: the line with sum(items)
    app.press(cx, "cmd-shift-f");
    moment(cx);
    assert!(app.until(cx, |w| tab(w).as_deref() == Some("Text")), "⇧⌘F opened nothing");
    app.typed(cx, "sum(items)");
    moment(cx);
    assert!(
        app.until(cx, |w| rows(w) == ["return sum(items) *"]),
        "sum(items) found {:?}",
        look(cx, &app, |w| rows(w))
    );
    assert_eq!(look(cx, &app, |w| details(w)), ["src/app.py:2, matched: sum(items)"]);
    app.press(cx, "enter");
    moment(cx);
    assert!(app.until(cx, |w| w.try_find("search-popup").is_none()), "the popup stayed");
    assert!(
        app.until(cx, |w| has_line(w, "    return sum(items)", "Line 2, marked: sum(items)")),
        "line 2 isn't marked: {:?}",
        look(cx, &app, |w| {
            w.find_all("source").iter().map(|l| l.description().unwrap_or_default().to_string()).collect::<Vec<_>>()
        })
    );

    // a file written just now is found by the next search: nothing is narrowed to the last search's files
    app.press(cx, "cmd-shift-f");
    moment(cx);
    app.typed(cx, "zzfresh");
    moment(cx);
    assert!(
        app.until(cx, |w| note(w).as_deref() == Some("Nothing found for \u{201c}zzfresh\u{201d}.")),
        "{:?}",
        look(cx, &app, |w| (note(w), status(w)))
    );
    write(&folder, "fresh.py", "class ZzFreshThing:\n    note = 'zzfresh here'\n");
    let listed = |cx: &mut TestAppContext| {
        cx.run_until_parked();
        cx.update(|cx| {
            let lists = enkidu_app::stores::Stores::changes(cx);
            lists.read(cx).of(&ws).is_some_and(|files| files.iter().any(|change| change.path == "fresh.py"))
        })
    };
    let mut there = false;
    for _ in 0..1000 {
        if listed(cx) {
            there = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(there, "the new file never showed up in the changes");
    app.typed(cx, " h");
    moment(cx);
    assert!(
        app.until(cx, |w| rows(w) == ["note = 'zzfresh here' *"]),
        "zzfresh h found {:?}",
        look(cx, &app, |w| (rows(w), note(w)))
    );
    // and its class is in Go to symbol, read again because the watcher saw the file
    app.click(cx, ("search-tab", 1usize));
    moment(cx);
    assert!(app.until(cx, |w| tab(w).as_deref() == Some("Symbols")));
    app.press(cx, "cmd-a");
    app.typed(cx, "ZzFresh");
    moment(cx);
    assert!(
        app.until(cx, |w| rows(w) == ["ZzFreshThing *"]),
        "ZzFresh found {:?}",
        look(cx, &app, |w| (rows(w), note(w)))
    );
    assert_eq!(look(cx, &app, |w| details(w)), ["class, fresh.py:1"]);
    app.press(cx, "escape");
    moment(cx);
    assert!(app.until(cx, |w| w.try_find("search-popup").is_none()));
    app.click(cx, "close-file");
    assert!(app.until(cx, |w| w.try_find("file-view").is_none()), "the file stayed open");
}

/// What stands under the rows of Find in files: the file it shows, and the line it is at.
fn preview(window: &Window) -> Option<(String, String)> {
    window.try_find("search-preview").map(|preview| {
        (preview.label().unwrap_or_default().to_string(), preview.description().unwrap_or_default().to_string())
    })
}

fn previews(path: &str, line: &str) -> Option<(String, String)> {
    Some((path.to_string(), line.to_string()))
}

/// Item 10.6: under the rows, the
/// file around the picked line, with what was found marked and the marks of what differs from the last commit. It
/// follows the picked row, and Enter still opens the file.
#[gpui_kit::test]
fn the_file_around_the_picked_line_stands_under_the_rows_and_follows_the_picked_row(cx: &mut TestAppContext) {
    let (app, folder, _) = start(cx);
    app.press(cx, "cmd-shift-f");
    moment(cx);
    assert!(app.until(cx, |w| tab(w).as_deref() == Some("Text")), "⇧⌘F opened nothing");
    assert_eq!(look(cx, &app, |w| preview(w)), None, "a preview of nothing");

    app.typed(cx, "total");
    moment(cx);
    let both = ["def total(items): *", "def total(self):"];
    assert!(app.until(cx, |w| rows(w) == both), "total found {:?}", look(cx, &app, |w| rows(w)));
    moment(cx);
    // the file of the picked line, at that line, which is marked with what was found on it
    assert!(
        app.until(cx, |w| preview(w) == previews("src/app.py", "Line 1")),
        "no preview: {:?}",
        look(cx, &app, |w| preview(w))
    );
    assert!(app.until(cx, |w| has_line(w, "def total(items):", "Line 1, marked: total")), "line 1 isn't marked");
    look(cx, &app, |w| {
        // a changed file: its badge, and the mark beside the line that differs from the last commit
        assert_eq!(said(w, "search-preview-kind").as_deref(), Some("Modified"));
        assert_eq!(said(w, ("change-mark", 2usize)).as_deref(), Some("Line 2 is changed"));
        assert!(w.try_find(("change-mark", 1usize)).is_none());
        // under the rows, over the foot, and tall enough to read around the line
        let (lines, under) = (w.find("search-lines").bounds(), w.find("search-preview").bounds());
        assert!(under.top() >= lines.bottom(), "the preview lies over the rows");
        assert!(under.bottom() <= w.find("search-popup").bounds().bottom());
        assert!(under.size.height > gpui_kit::px(150.), "{:?}", under.size.height);
    });

    // Claude writes the file again while it is previewed: the preview follows, with the new line and its mark
    write(&folder, "src/app.py", "def total(items):\n    return sum(item.price for item in items)\n# rounded later\n");
    // (the watcher sees the write in its own time; the preview then waits its moment, by the clock the test holds)
    let mut followed = false;
    for _ in 0..300 {
        moment(cx);
        // the new line stands right under the changed one: one stretch of changed lines
        if look(cx, &app, |w| said(w, ("change-mark", 3usize)).as_deref() == Some("Line 3 is changed")) {
            followed = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        followed,
        "the preview kept the old text: {:?}",
        look(cx, &app, |w| (preview(w), said(w, ("change-mark", 2usize))))
    );
    assert!(app.until(cx, |w| has_line(w, "# rounded later", "Line 3")), "the new line isn't in the preview");

    // ↓: the other file at its line. A file without changes has no badge and no marks
    app.press(cx, "down");
    moment(cx);
    assert!(
        app.until(cx, |w| preview(w) == previews("src/orders/stock.py", "Line 8")),
        "↓: {:?}",
        look(cx, &app, |w| preview(w))
    );
    assert!(app.until(cx, |w| has_line(w, "    def total(self):", "Line 8, marked: total")), "line 8 isn't marked");
    look(cx, &app, |w| {
        assert!(w.try_find("search-preview-kind").is_none());
        assert!((0..40usize).all(|line| w.try_find(("change-mark", line)).is_none()), "marks in an unchanged file");
    });
    // back and forth without a rest reads nothing on the way: after the rest, the file of the row that is picked
    app.press(cx, "up");
    app.press(cx, "down");
    app.press(cx, "up");
    assert_eq!(look(cx, &app, |w| preview(w)), previews("src/orders/stock.py", "Line 8"), "read without a rest");
    moment(cx);
    assert!(app.until(cx, |w| preview(w) == previews("src/app.py", "Line 1")), "{:?}", look(cx, &app, |w| preview(w)));

    // another line of the file that is shown is there at once, without a read
    clear(cx, &app);
    assert_eq!(look(cx, &app, |w| preview(w)), None, "nothing typed, and a preview");
    app.typed(cx, "stock");
    moment(cx);
    assert!(app.until(cx, |w| rows(w).len() == 5), "stock found {:?}", look(cx, &app, |w| rows(w)));
    moment(cx);
    assert!(app.until(cx, |w| preview(w) == previews("src/orders/stock.py", "Line 4")));
    app.press(cx, "down");
    assert_eq!(look(cx, &app, |w| preview(w)), previews("src/orders/stock.py", "Line 6"), "the same file, at once");
    assert!(app.until(cx, |w| has_line(w, "        return StockItem.objects.all()", "Line 6, marked: Stock")));

    // the other tabs have none
    app.press(cx, "tab");
    moment(cx);
    assert!(app.until(cx, |w| tab(w).as_deref() == Some("Files") && preview(w).is_none()), "a preview in Files");
    app.press(cx, "shift-tab");
    moment(cx);
    assert!(app.until(cx, |w| preview(w) == previews("src/orders/stock.py", "Line 4")), "back in Text");

    // a window too short for six lines of it has none, and the rows have the room
    cx.simulate_window_resize(app.window, gpui_kit::size(gpui_kit::px(1200.), gpui_kit::px(400.)));
    assert!(app.until(cx, |w| preview(w).is_none() && rows(w).len() == 5), "{:?}", look(cx, &app, |w| preview(w)));
    cx.simulate_window_resize(app.window, gpui_kit::size(gpui_kit::px(1360.), gpui_kit::px(860.)));
    assert!(app.until(cx, |w| preview(w).is_some()), "the room is back, the preview isn't");

    // Enter opens the file at the picked line, as before
    app.press(cx, "enter");
    moment(cx);
    assert!(app.until(cx, |w| w.try_find("search-popup").is_none()), "the popup stayed");
    assert!(app.until(cx, |w| said(w, "file-view").as_deref() == Some("src/orders/stock.py")), "the file never opened");
}
