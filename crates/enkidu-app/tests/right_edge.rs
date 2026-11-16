//! Items 3.9b and 3.9c: the right edge of a workspace, in a real window on real git. A bar of two buttons (Changed
//! files with their number, Files); each opens its panel beside the bar at full height, and a second click, ⌥⌘1 or
//! ⌥⌘2 closes it. The panel's width drags, is remembered, and resets on a double click. Changed files is a tree of the
//! folders the changed files are in (items 29.4 and 29.10: a click brings a file's diff into view, a folder closes and
//! opens, all of them at once, and a filter keeps one kind of file), then the files that have remarks only.

mod common;

use std::path::PathBuf;
use std::time::Duration;

use common::{App, click_line, line_number, said, until_kept};
use enkidu_app::changes_island::ChangesIsland;
use enkidu_model::ui::{Panel, Ui};
use enkidu_testkit::git;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Bounds, Entity, InputEvent as _, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Point, TestAppContext, Window, point, px,
};

const CODE: &str =
    "def total(items):\n    result = 0\n    for item in items:\n        result += item.price\n    return result\n";

fn make(shared: &std::sync::Arc<enkidu_core::state::Shared>, project: &str, _: &std::path::Path) {
    App::make(shared, project, "stock-fix", "new", "stock-fix", Some("development"))
}

/// The sample project with one workspace where `src/app.py` was rewritten.
fn start(cx: &mut TestAppContext) -> (App, PathBuf, String) {
    let app = common::start_on(cx, enkidu_testkit::shop, make);
    let (_, _, folder) = app.on_screen(cx).expect("on screen");
    let folder = PathBuf::from(folder);
    std::fs::write(folder.join("src/app.py"), CODE).unwrap();
    let ws = app.frame.read_with(cx, |f, _| f.current_workspace().unwrap().id.clone());
    assert!(app.until(cx, |w| line_number(w, "Line 5, after").is_some()), "the diff never showed");
    (app, folder, ws)
}

fn ui(cx: &mut TestAppContext, app: &App) -> Ui {
    let ui: Entity<Ui> = app.shell.read_with(cx, |shell, _| shell.ui().clone());
    ui.read_with(cx, |ui, _| ui.clone())
}

fn island(cx: &mut TestAppContext, app: &App) -> Entity<ChangesIsland> {
    app.shell.read_with(cx, |shell, _| shell.changes().cloned()).expect("the island")
}

fn look<R>(cx: &mut TestAppContext, app: &App, at: impl FnOnce(&mut Window) -> R) -> R {
    cx.run_until_parked();
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        at(w)
    })
    .unwrap()
}

fn bounds(window: &Window, id: &'static str) -> Option<Bounds<Pixels>> {
    window.try_find(id).map(|found| found.bounds())
}

/// The panel that is open, by its title.
fn panel(window: &Window) -> Option<String> {
    said(window, "panel-band")
}

/// What `ui.json` in the app's data folder says, once it says `want` there.
fn kept_layout(cx: &mut TestAppContext, app: &App, want: &str) -> String {
    let file = app.home.path().join("data").join("ui.json");
    for _ in 0..600 {
        // it is written a moment after the last change, by the test's clock
        cx.executor().advance_clock(Duration::from_millis(400));
        cx.run_until_parked();
        if std::fs::read_to_string(&file).is_ok_and(|text| text.contains(want)) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    std::fs::read_to_string(&file).unwrap_or_default()
}

/// A drag with the mouse: down at `from`, through each of `to`, up at the last.
fn drag(cx: &mut TestAppContext, app: &App, from: Point<Pixels>, to: &[Point<Pixels>]) {
    let (left, modifiers) = (MouseButton::Left, Modifiers::default());
    cx.update_window(app.window, |_, w, cx| {
        let hover = MouseMoveEvent { position: from, pressed_button: None, modifiers };
        w.dispatch_event(hover.to_platform_input(), cx);
        let down = MouseDownEvent { button: left, position: from, modifiers, click_count: 1, first_mouse: false };
        w.dispatch_event(down.to_platform_input(), cx);
        w.render_frame(cx);
        for position in to {
            let moved = MouseMoveEvent { position: *position, pressed_button: Some(left), modifiers };
            w.dispatch_event(moved.to_platform_input(), cx);
            w.render_frame(cx);
        }
        let up = MouseUpEvent { button: left, position: *to.last().unwrap_or(&from), modifiers, click_count: 1 };
        w.dispatch_event(up.to_platform_input(), cx);
        w.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn double_click(cx: &mut TestAppContext, app: &App, at: Point<Pixels>) {
    let (left, modifiers) = (MouseButton::Left, Modifiers::default());
    cx.update_window(app.window, |_, w, cx| {
        for click_count in [1, 2] {
            let down = MouseDownEvent { button: left, position: at, modifiers, click_count, first_mouse: false };
            w.dispatch_event(down.to_platform_input(), cx);
            let up = MouseUpEvent { button: left, position: at, modifiers, click_count };
            w.dispatch_event(up.to_platform_input(), cx);
        }
        w.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
}

/// The rows of Changed files, top to bottom: what each says (its path) and whether it is marked.
fn rows(window: &Window, id: &'static str) -> Vec<(String, bool)> {
    (0..40usize)
        .filter_map(|ix| window.try_find((id, ix)))
        .map(|row| (row.label().unwrap_or_default().to_string(), row.selected() == Some(true)))
        .collect()
}

fn paths(window: &Window, id: &'static str) -> Vec<String> {
    rows(window, id).into_iter().map(|(path, _)| path).collect()
}

/// Item 3.9b.
#[gpui_kit::test]
fn the_bar_opens_one_panel_at_a_time_beside_it_and_a_second_click_or_its_key_closes_it(cx: &mut TestAppContext) {
    let (app, _, _) = start(cx);
    // as the app starts: Changed files is open, beside the bar at the window's right edge, at the workspace's height
    look(cx, &app, |w| {
        assert_eq!(panel(w).as_deref(), Some("Changed files"));
        assert_eq!(w.find("rail-changes").selected(), Some(true));
        assert_eq!(w.find("rail-files").selected(), Some(false));
        assert_eq!(said(w, "rail-count").as_deref(), Some("1"), "the changed files' number is on the button");
        let (row, panel, body) =
            (w.find("workspace-row").bounds(), w.find("island-panel").bounds(), w.find("workspace-body").bounds());
        assert_eq!(panel.size.width, px(280.));
        assert_eq!((panel.top(), panel.bottom()), (row.top(), row.bottom()), "full height");
        assert_eq!(row.right() - panel.right(), px(40.), "the bar is 40 px wide, at the edge");
        assert_eq!(panel.left() - body.right(), px(8.), "8 px between the workspace and the panel");
        assert_eq!(said(w, "panel-divider").as_deref(), Some("Resize panel"));
        // the panel says how many files changed, so the changes' own band doesn't
        let count = w.find("changes-count").bounds();
        assert!(count.left() > panel.left(), "the count is on the changes' band");
    });

    // the other button: its panel takes the place
    app.click(cx, "rail-files");
    assert!(app.until(cx, |w| panel(w).as_deref() == Some("Files")), "Files never opened");
    look(cx, &app, |w| {
        assert_eq!(w.find("rail-files").selected(), Some(true));
        assert_eq!(w.find("rail-changes").selected(), Some(false));
        // the count went back to the changes' band
        let (count, panel) = (w.find("changes-count").bounds(), w.find("island-panel").bounds());
        assert!(count.right() < panel.left(), "the count isn't on the changes' band");
    });
    assert!(kept_layout(cx, &app, r#""rightPanel":"files""#).contains(r#""rightPanel":"files""#), "not kept");

    // the same button again: no panel, and the workspace has the room up to the bar
    app.click(cx, "rail-files");
    assert!(app.until(cx, |w| w.try_find("island-panel").is_none()), "the panel stayed");
    look(cx, &app, |w| {
        let (row, body) = (w.find("workspace-row").bounds(), w.find("workspace-body").bounds());
        assert_eq!(row.right() - body.right(), px(40.));
        assert!(w.try_find("panel-divider").is_none());
        assert_eq!((w.find("rail-files").selected(), w.find("rail-changes").selected()), (Some(false), Some(false)));
    });
    assert!(kept_layout(cx, &app, r#""rightPanel":null"#).contains(r#""rightPanel":null"#), "not kept");

    // ⌥⌘1 and ⌥⌘2 open and close them
    app.press(cx, "alt-cmd-1");
    assert!(app.until(cx, |w| panel(w).as_deref() == Some("Changed files")), "⌥⌘1");
    app.press(cx, "alt-cmd-2");
    assert!(app.until(cx, |w| panel(w).as_deref() == Some("Files")), "⌥⌘2");
    app.press(cx, "alt-cmd-2");
    assert!(app.until(cx, |w| w.try_find("island-panel").is_none()), "⌥⌘2 again");
    app.press(cx, "alt-cmd-1");
    assert!(app.until(cx, |w| panel(w).as_deref() == Some("Changed files")), "⌥⌘1 again");
    app.press(cx, "alt-cmd-1");
    assert!(app.until(cx, |w| w.try_find("island-panel").is_none()), "⌥⌘1 a third time");

    // the palette has both
    let pick = |cx: &mut TestAppContext, words: &str, want: &str| {
        app.press(cx, "cmd-k");
        assert!(app.until(cx, |w| w.try_find("command-palette").is_some()), "the palette never opened");
        app.typed(cx, words);
        assert!(app.until(cx, |w| said(w, ("palette-row", 0usize)).as_deref() == Some(want)), "no row says {want}");
        app.press(cx, "enter");
        assert!(app.until(cx, |w| w.try_find("command-palette").is_none()), "the palette stayed");
    };
    pick(cx, "changed files", "Show the changed files");
    assert!(app.until(cx, |w| panel(w).as_deref() == Some("Changed files")));
    pick(cx, "changed files", "Hide the changed files");
    assert!(app.until(cx, |w| w.try_find("island-panel").is_none()));
    pick(cx, "project's files", "Show the project's files");
    assert!(app.until(cx, |w| panel(w).as_deref() == Some("Files")));
    assert_eq!(ui(cx, &app).right_panel, Some(Panel::Files));
}

/// Item 3.9b.
#[gpui_kit::test]
fn the_panels_width_drags_is_remembered_and_resets_on_a_double_click(cx: &mut TestAppContext) {
    let (app, _, _) = start(cx);
    let gap = look(cx, &app, |w| w.find("panel-divider").bounds());
    assert_eq!(gap.size.width, px(8.));
    let at = gap.center();
    // 60 px to the left: the panel is 60 px wider, and the workspace gives the room
    drag(cx, &app, at, &[point(at.x - px(20.), at.y), point(at.x - px(60.), at.y)]);
    assert_eq!(ui(cx, &app).right_panel_width, 340);
    assert_eq!(look(cx, &app, |w| bounds(w, "island-panel").map(|panel| panel.size.width)), Some(px(340.)));
    // far to the left: 520 px at most; far to the right: 200 at least
    let gap = look(cx, &app, |w| w.find("panel-divider").bounds().center());
    drag(cx, &app, gap, &[point(gap.x - px(40.), gap.y), point(px(100.), gap.y)]);
    assert_eq!(ui(cx, &app).right_panel_width, 520);
    let gap = look(cx, &app, |w| w.find("panel-divider").bounds().center());
    drag(cx, &app, gap, &[point(gap.x + px(40.), gap.y), point(gap.x + px(900.), gap.y)]);
    assert_eq!(ui(cx, &app).right_panel_width, 200);
    assert_eq!(look(cx, &app, |w| bounds(w, "island-panel").map(|panel| panel.size.width)), Some(px(200.)));

    // 300 px, and a restart: the panel opens as wide as it was left
    let (gap, edge) = look(cx, &app, |w| (w.find("panel-divider").bounds().center(), w.find("workspace-row").bounds()));
    let want = edge.right() - px(40. + 300. + 4.);
    drag(cx, &app, gap, &[point(gap.x - px(30.), gap.y), point(want, gap.y)]);
    assert_eq!(ui(cx, &app).right_panel_width, 300);
    let kept = kept_layout(cx, &app, r#""rightPanelWidth":300"#);
    assert!(kept.contains(r#""rightPanelWidth":300"#) && kept.contains(r#""rightPanel":"changes""#), "{kept}");
    let app = common::restart(cx, app);
    assert!(app.until(cx, |w| line_number(w, "Line 5, after").is_some()), "the diff never showed again");
    assert_eq!(look(cx, &app, |w| bounds(w, "island-panel").map(|panel| panel.size.width)), Some(px(300.)));
    // a double click: 280 again
    let gap = look(cx, &app, |w| w.find("panel-divider").bounds().center());
    double_click(cx, &app, gap);
    assert_eq!(ui(cx, &app).right_panel_width, 280);
}

/// Item 3.9c.
#[gpui_kit::test]
fn changed_files_lists_every_changed_file_and_a_click_brings_its_diff_into_view(cx: &mut TestAppContext) {
    let (app, folder, ws) = start(cx);
    // two more new files at the top: they stand after the folder, by name
    let long: String = (1..=90).map(|n| format!("note {n}\n")).collect();
    std::fs::write(folder.join("notes.md"), long).unwrap();
    let stripes: String = (1..=90).map(|n| format!("stripe {n}\n")).collect();
    std::fs::write(folder.join("zebra.txt"), stripes).unwrap();
    assert!(app.until(cx, |w| paths(w, "change-row") == ["src/app.py", "notes.md", "zebra.txt"]), "not listed");
    look(cx, &app, |w| {
        assert_eq!(said(w, "changes-count").as_deref(), Some("3 files"));
        assert_eq!(said(w, "rail-count").as_deref(), Some("3"));
        let all = |id: &'static str| -> Vec<String> {
            w.find_all(id).iter().map(|found| found.label().unwrap_or_default().to_string()).collect()
        };
        // each row says what happened to its file and its lines
        assert_eq!(all("row-kind"), ["Modified", "New file", "New file"]);
        assert_eq!(all("row-added"), ["+4", "+90", "+90"]);
        assert_eq!(all("row-removed"), ["\u{2212}1"]);
        // nothing is marked until a file is asked for
        assert!(rows(w, "change-row").iter().all(|(_, marked)| !marked));
    });
    let top = |cx: &mut TestAppContext| {
        let island = island(cx, &app);
        cx.run_until_parked();
        cx.update_window(app.window, |_, w, cx| {
            w.render_frame(cx);
            w.render_frame(cx);
            island.read(cx).diff(cx).and_then(|diff| diff.read(cx).top_path().map(|path| path.to_string()))
        })
        .unwrap()
    };
    // the file at the top, once the diffs that decide it were read (a diff that isn't read yet is a short stand-in,
    // and the place is right only when the ones above it have their lines)
    let at_top = |cx: &mut TestAppContext, want: &str| {
        let mut seen = None;
        for _ in 0..400 {
            seen = top(cx);
            if seen.as_deref() == Some(want) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        seen
    };
    assert_eq!(at_top(cx, "src/app.py").as_deref(), Some("src/app.py"), "the diffs are stacked as the tree lists them");

    // a click on the last one: its diff is at the top of the view, and its row is marked
    app.click(cx, ("change-row", 2usize));
    assert!(app.until(cx, |w| rows(w, "change-row")[2].1), "the row isn't marked");
    assert_eq!(at_top(cx, "zebra.txt").as_deref(), Some("zebra.txt"));
    app.click(cx, ("change-row", 1usize));
    assert!(app.until(cx, |w| rows(w, "change-row")[1].1 && !rows(w, "change-row")[2].1));
    assert_eq!(at_top(cx, "notes.md").as_deref(), Some("notes.md"));

    // with the changes put away, a click brings them back with that file at the top
    app.press(cx, "alt-cmd-b");
    assert!(app.until(cx, |w| w.try_find("island-changes").is_none()), "⌥⌘B");
    app.click(cx, ("change-row", 2usize));
    assert!(app.until(cx, |w| w.try_find("island-changes").is_some()), "the changes didn't come back");
    assert_eq!(at_top(cx, "zebra.txt").as_deref(), Some("zebra.txt"));

    // with a file opened, a click closes it and shows the diff that was asked for
    let opened = island(cx, &app);
    cx.update(|cx| opened.update(cx, |island, cx| island.open_file("src/app.py", None, cx)));
    assert!(app.until(cx, |w| said(w, "file-view").as_deref() == Some("src/app.py")), "the file never opened");
    // the opened file is the one being looked at
    assert!(
        app.until(cx, |w| rows(w, "change-row")[0].1 && !rows(w, "change-row")[2].1),
        "the opened file isn't marked"
    );
    app.click(cx, ("change-row", 1usize));
    assert!(app.until(cx, |w| w.try_find("file-view").is_none() && w.try_find("diff-view").is_some()));
    assert_eq!(at_top(cx, "notes.md").as_deref(), Some("notes.md"));
    assert!(look(cx, &app, |w| rows(w, "change-row")[1].1));

    // a remark on a changed file is counted on its row
    app.click(cx, ("change-row", 0usize));
    assert!(app.until(cx, |w| line_number(w, "Line 4, after").is_some()));
    click_line(cx, &app, "Line 4, after");
    assert!(app.until(cx, |w| w.try_find("remark-composer").is_some()), "no composer");
    app.typed(cx, "Sum the prices with sum()");
    app.click(cx, "save-remark");
    assert_eq!(until_kept(cx, &app, &ws, 1).len(), 1);
    assert!(app.until(cx, |w| said(w, "row-remarks").as_deref() == Some("1")), "the row doesn't count the remark");
}

/// The tree of Changed files, top to bottom: a folder as "▾ src (3 files)" ("▸" when it is closed), a file by its
/// path; each with whether it is marked.
fn tree(window: &Window) -> Vec<(String, bool)> {
    let mut found: Vec<(Pixels, String, bool)> = Vec::new();
    for ix in 0..60usize {
        if let Some(row) = window.try_find(("change-folder", ix)) {
            let said = row.description().unwrap_or_default();
            let (state, count) = said.split_once(", ").unwrap_or((said, ""));
            let sign = if state == "open" { "▾" } else { "▸" };
            let words = format!("{sign} {} ({count})", row.label().unwrap_or_default());
            found.push((row.bounds().top(), words, row.selected() == Some(true)));
        }
        if let Some(row) = window.try_find(("change-row", ix)) {
            found.push((row.bounds().top(), row.label().unwrap_or_default().to_string(), row.selected() == Some(true)));
        }
    }
    found.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    found.into_iter().map(|(_, said, marked)| (said, marked)).collect()
}

fn folders_and_files(window: &Window) -> Vec<String> {
    tree(window).into_iter().map(|(said, _)| said).collect()
}

/// Clicks the row of a folder of Changed files.
fn click_folder(cx: &mut TestAppContext, app: &App, path: &str) {
    let ix = look(cx, app, |w| {
        (0..60usize)
            .find(|ix| w.try_find(("change-folder", *ix)).is_some_and(|row| row.label() == Some(path)))
            .unwrap_or_else(|| panic!("no folder {path} among {:?}", folders_and_files(w)))
    });
    app.click(cx, ("change-folder", ix));
}

/// Whether a button of the tree's tools is there and can be pressed (None when it isn't there).
fn can_press(window: &Window, id: &'static str) -> Option<bool> {
    window.try_find(id).map(|button| button.description() != Some("dimmed"))
}

/// Items 29.4 and 29.10: the changed files under their folders, each folder with how many it holds; a folder closes
/// and opens, all of them at once, and the filter keeps one kind of file.
#[gpui_kit::test]
fn changed_files_is_a_tree_of_folders_that_close_and_open_and_a_filter_keeps_one_kind(cx: &mut TestAppContext) {
    let (app, folder, _) = start(cx);
    // one changed file, at the end of a folder: its folder's row, and the two buttons for it. Every changed file is
    // of one kind, so no filter is offered
    assert!(app.until(cx, |w| folders_and_files(w) == ["▾ src (1 file)", "src/app.py"]), "the first tree");
    look(cx, &app, |w| {
        assert!(w.try_find("changed-kind").is_none(), "a filter with nothing to pick");
        assert_eq!((can_press(w, "changed-expand"), can_press(w, "changed-collapse")), (Some(false), Some(true)));
    });
    // more changes, in folders of every shape
    let write = |path: &str, text: &str| {
        let file = folder.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    };
    write("src/orders/stock.py", "def count():\n    return 1\n");
    write("src/orders/tests/test_stock.py", "def test_count():\n    assert True\n");
    write("docs/guide/intro.md", "# Intro\n");
    write("notes.md", "# Notes\n");
    let whole = [
        // a folder that only holds one folder shares its row with it
        "▾ docs/guide (1 file)",
        "docs/guide/intro.md",
        "▾ src (3 files)",
        "▾ src/orders (2 files)",
        // in a folder its folders first, then its files
        "▾ src/orders/tests (1 file)",
        "src/orders/tests/test_stock.py",
        "src/orders/stock.py",
        "src/app.py",
        "notes.md",
    ];
    assert!(app.until(cx, |w| folders_and_files(w) == whole), "the tree");
    look(cx, &app, |w| {
        assert_eq!(said(w, "changes-count").as_deref(), Some("5 files"));
        let counts: Vec<String> =
            w.find_all("folder-files").iter().map(|found| found.label().unwrap_or_default().to_string()).collect();
        assert_eq!(counts, ["1 file", "3 files", "2 files", "1 file"]);
        // a row is 26 px, and what a folder holds stands further in than the folder
        let row = |path: &str| {
            (0..60usize)
                .filter_map(|ix| w.try_find(("change-row", ix)))
                .find(|row| row.label() == Some(path))
                .map(|row| row.bounds())
                .expect("the file's row")
        };
        assert_eq!(row("src/app.py").size.height, px(26.));
        assert_eq!(row("notes.md").top() - row("src/app.py").top(), px(26.));
    });

    // a folder closes with a click: its row stays with its count, what it holds goes
    click_folder(cx, &app, "src/orders");
    assert!(
        app.until(cx, |w| folders_and_files(w)[2..]
            == ["▾ src (3 files)", "▸ src/orders (2 files)", "src/app.py", "notes.md"]),
        "src/orders didn't close"
    );
    // and opens again
    click_folder(cx, &app, "src/orders");
    assert!(app.until(cx, |w| folders_and_files(w) == whole), "src/orders didn't open");

    // the file on screen is marked; while its folder is closed, the folder is
    let stock = look(cx, &app, |w| {
        (0..60usize)
            .find(|ix| w.try_find(("change-row", *ix)).is_some_and(|row| row.label() == Some("src/orders/stock.py")))
            .expect("stock.py")
    });
    app.click(cx, ("change-row", stock));
    let marked =
        |w: &Window| tree(w).into_iter().filter(|(_, marked)| *marked).map(|(said, _)| said).collect::<Vec<_>>();
    assert!(app.until(cx, |w| marked(w) == ["src/orders/stock.py"]), "the file isn't marked");
    click_folder(cx, &app, "src");
    assert!(app.until(cx, |w| marked(w) == ["▸ src (3 files)"]), "the closed folder doesn't stand for its file");
    click_folder(cx, &app, "src");
    assert!(app.until(cx, |w| marked(w) == ["src/orders/stock.py"]), "open again, the file has its mark back");

    // "Collapse all": every folder closed, the top ones left with their counts
    app.click(cx, "changed-collapse");
    assert!(
        app.until(cx, |w| folders_and_files(w) == ["▸ docs/guide (1 file)", "▸ src (3 files)", "notes.md"]),
        "Collapse all"
    );
    look(cx, &app, |w| {
        assert_eq!((can_press(w, "changed-expand"), can_press(w, "changed-collapse")), (Some(true), Some(false)));
    });
    // a folder opened after that shows its own folders closed
    click_folder(cx, &app, "src");
    assert!(
        app.until(cx, |w| {
            folders_and_files(w)
                == ["▸ docs/guide (1 file)", "▾ src (3 files)", "▸ src/orders (2 files)", "src/app.py", "notes.md"]
        }),
        "a folder opened after Collapse all"
    );
    look(cx, &app, |w| {
        assert_eq!((can_press(w, "changed-expand"), can_press(w, "changed-collapse")), (Some(true), Some(true)));
    });
    // "Expand all": every folder open
    app.click(cx, "changed-expand");
    assert!(app.until(cx, |w| folders_and_files(w) == whole), "Expand all");
    look(cx, &app, |w| {
        assert_eq!((can_press(w, "changed-expand"), can_press(w, "changed-collapse")), (Some(false), Some(true)));
    });

    // the filter: the kinds of file among the changes, the most common first, each with how many have it
    assert_eq!(look(cx, &app, |w| said(w, "changed-kind")).as_deref(), Some("All files"));
    app.click(cx, "changed-kind");
    assert!(app.until(cx, |w| w.try_find("changed-kind-option:md").is_some()), "the filter's menu never opened");
    look(cx, &app, |w| {
        let every = w.find("changed-kind-option:");
        assert_eq!((every.label(), every.description(), every.selected()), (Some("All files"), Some("5"), Some(true)));
        let py = w.find("changed-kind-option:py");
        assert_eq!((py.label(), py.description(), py.selected()), (Some("*.py"), Some("3"), Some(false)));
        assert_eq!(w.find("changed-kind-option:md").description(), Some("2"));
        assert!(py.bounds().top() < w.find("changed-kind-option:md").bounds().top(), "the most common first");
    });
    // one kind: its files with the folders that hold them; the band still counts every changed file
    app.click(cx, "changed-kind-option:md");
    assert!(
        app.until(cx, |w| folders_and_files(w) == ["▾ docs/guide (1 file)", "docs/guide/intro.md", "notes.md"]),
        "the filter didn't keep the .md files"
    );
    look(cx, &app, |w| {
        assert_eq!(said(w, "changed-kind").as_deref(), Some("*.md"));
        assert_eq!(said(w, "changes-count").as_deref(), Some("5 files"));
        assert!(w.try_find("diff-view").is_some(), "the diffs stay");
    });
    // every file again
    app.click(cx, "changed-kind");
    assert!(app.until(cx, |w| w.try_find("changed-kind-option:").is_some()), "the menu never opened again");
    look(cx, &app, |w| assert_eq!(w.find("changed-kind-option:md").selected(), Some(true)));
    app.click(cx, "changed-kind-option:");
    assert!(app.until(cx, |w| folders_and_files(w) == whole), "All files");

    // a kind that no changed file has any more is forgotten: every file is listed, and with one kind left there is
    // no filter
    app.click(cx, "changed-kind");
    assert!(app.until(cx, |w| w.try_find("changed-kind-option:md").is_some()));
    app.click(cx, "changed-kind-option:md");
    assert!(app.until(cx, |w| folders_and_files(w).len() == 3));
    std::fs::remove_file(folder.join("notes.md")).unwrap();
    std::fs::remove_dir_all(folder.join("docs")).unwrap();
    assert!(
        app.until(cx, |w| {
            folders_and_files(w)
                == [
                    "▾ src (3 files)",
                    "▾ src/orders (2 files)",
                    "▾ src/orders/tests (1 file)",
                    "src/orders/tests/test_stock.py",
                    "src/orders/stock.py",
                    "src/app.py",
                ]
        }),
        "the files of the kind that is left aren't listed"
    );
    look(cx, &app, |w| assert!(w.try_find("changed-kind").is_none(), "a filter with nothing to pick"));
}

/// Item 3.9c.
#[gpui_kit::test]
fn files_that_have_remarks_only_are_listed_after_the_changed_ones_and_open_at_their_first_remark(
    cx: &mut TestAppContext,
) {
    let (app, folder, ws) = start(cx);
    // a remark on the changed file, then its change is committed: the file has a remark and no changes
    click_line(cx, &app, "Line 4, after");
    assert!(app.until(cx, |w| w.try_find("remark-composer").is_some()), "no composer");
    app.typed(cx, "Sum the prices with sum()");
    app.click(cx, "save-remark");
    assert_eq!(until_kept(cx, &app, &ws, 1).len(), 1);
    std::fs::write(folder.join("notes.md"), "# Notes\n").unwrap();
    assert!(app.until(cx, |w| paths(w, "change-row") == ["src/app.py", "notes.md"]));
    look(cx, &app, |w| assert!(paths(w, "remark-file-row").is_empty(), "a changed file listed twice"));
    git(&folder, &["add", "src/app.py"]);
    git(&folder, &["commit", "-q", "-m", "the totals"]);
    assert!(app.until(cx, |w| paths(w, "change-row") == ["notes.md"]), "the commit wasn't seen");
    assert!(app.until(cx, |w| paths(w, "remark-file-row") == ["src/app.py"]), "the remark's file isn't listed");
    look(cx, &app, |w| {
        assert_eq!(said(w, "row-remarks").as_deref(), Some("1"));
        assert_eq!(said(w, "changes-count").as_deref(), Some("1 file"), "only changed files are counted");
        // under the changed files, after its heading
        let (changed, remarked) =
            (w.find(("change-row", 0usize)).bounds(), w.find(("remark-file-row", 2usize)).bounds());
        assert!(remarked.top() > changed.bottom() + px(30.), "no heading between them: {changed:?} {remarked:?}");
    });

    // a click opens the file where the changes are, at the remark's line, and marks the row
    app.click(cx, ("remark-file-row", 2usize));
    assert!(app.until(cx, |w| said(w, "file-view").as_deref() == Some("src/app.py")), "the file never opened");
    assert!(app.until(cx, |w| line_number(w, "Line 4").is_some()), "its lines never showed");
    assert!(app.until(cx, |w| rows(w, "remark-file-row") == [("src/app.py".to_string(), true)]), "not marked");

    // everything committed and the remark deleted: the panel keeps its title and nothing under it
    git(&folder, &["add", "-A"]);
    git(&folder, &["commit", "-q", "-m", "notes"]);
    app.click(cx, "delete-remark");
    assert!(until_kept(cx, &app, &ws, 0).is_empty());
    assert!(app.until(cx, |w| paths(w, "change-row").is_empty() && paths(w, "remark-file-row").is_empty()));
    look(cx, &app, |w| {
        assert_eq!(panel(w).as_deref(), Some("Changed files"));
        assert!(w.try_find("changes-count").is_none() && w.try_find("rail-count").is_none(), "a count of nothing");
    });
}
