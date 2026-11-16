//! Item 3.9d: the Files panel, in a real window on real git. The workspace as git sees it (tracked and new files,
//! ignored ones left out): folders first, single-folder chains on one row, an amber dot on folders that hold
//! changes, the change letter on changed files; a filter that ranks file names first; 300 files a folder with "Show
//! all"; and a folder of 50,000 files that draws like one of fifty.

mod common;

use std::path::{Path, PathBuf};
use std::time::Instant;

use common::{App, said};
use enkidu_app::shell::Shell;
use enkidu_testkit::git;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Bounds, Pixels, ScrollDelta, TestAppContext, Window, WindowBounds, WindowOptions, point, px, size,
};

fn make(shared: &std::sync::Arc<enkidu_core::state::Shared>, project: &str, _: &Path) {
    App::make(shared, project, "stock-fix", "new", "stock-fix", Some("development"))
}

fn write(folder: &Path, path: &str, text: &str) {
    let file = folder.join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, text).unwrap();
}

/// The sample project with one workspace, where a few more files are committed, `src/app.py` is edited and `notes.md`
/// is new. Its `.env` is ignored. The Files panel is open.
fn start(cx: &mut TestAppContext) -> (App, PathBuf) {
    let app = common::start_on(cx, enkidu_testkit::shop, make);
    let (_, _, folder) = app.on_screen(cx).expect("on screen");
    let folder = PathBuf::from(folder);
    for path in [
        "README.md",
        "a/b/c/deep.txt",
        "docs/stock-notes.md",
        "file10.txt",
        "file9.txt",
        "src/orders/stock.py",
        "src/orders/serializers.py",
        "src/orders/tests/test_stock.py",
    ] {
        write(&folder, path, "kept\n");
    }
    git(&folder, &["add", "-A"]);
    git(&folder, &["commit", "-q", "-m", "more files"]);
    write(&folder, ".env", "SECRET=1\n");
    write(&folder, "src/app.py", "def total(items):\n    return sum(item.price for item in items)\n");
    write(&folder, "notes.md", "# Notes\n");
    assert!(app.until(cx, |w| said(w, "changes-count").as_deref() == Some("2 files")), "the changes never came");
    app.click(cx, "rail-files");
    assert!(app.until(cx, |w| said(w, "panel-band").as_deref() == Some("Files")), "Files never opened");
    assert!(app.until(cx, |w| !rows(w).is_empty()), "no files were listed");
    (app, folder)
}

fn look<R>(cx: &mut TestAppContext, app: &App, at: impl FnOnce(&mut Window) -> R) -> R {
    cx.run_until_parked();
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        at(w)
    })
    .unwrap()
}

/// The rows in view, top to bottom: "dir path" for a folder (with "open" when it is), the path for a file, and what
/// "Show all" says.
fn rows(window: &Window) -> Vec<String> {
    let mut found: Vec<(Pixels, String)> = Vec::new();
    for ix in 0..800usize {
        if let Some(row) = window.try_find(("folder-row", ix)) {
            let open = if row.description() == Some("open") { " open" } else { "" };
            found.push((row.bounds().top(), format!("dir {}{open}", row.label().unwrap_or_default())));
        }
        for id in ["file-row", "show-all"] {
            if let Some(row) = window.try_find((id, ix)) {
                found.push((row.bounds().top(), row.label().unwrap_or_default().to_string()));
            }
        }
    }
    found.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    found.into_iter().map(|(_, said)| said).collect()
}

/// Clicks the row that says `label` (a file's path, or a folder's).
fn click_row(cx: &mut TestAppContext, app: &App, label: &str) {
    let id = look(cx, app, |w| {
        (0..800usize)
            .flat_map(|ix| [("folder-row", ix), ("file-row", ix), ("show-all", ix)])
            .find(|id| w.try_find(*id).is_some_and(|row| row.label() == Some(label)))
            .unwrap_or_else(|| panic!("no row says {label:?}; the rows: {:?}", rows(w)))
    });
    app.click(cx, id);
    cx.run_until_parked();
}

/// How many rows the panel has, in view or not.
fn row_count(cx: &mut TestAppContext, shell: &gpui_kit::Entity<Shell>) -> usize {
    shell.read_with(cx, |shell, cx| shell.files_pane(cx).expect("the Files panel").read(cx).row_count())
}

#[gpui_kit::test]
fn files_is_the_workspace_as_git_sees_it(cx: &mut TestAppContext) {
    let (app, folder) = start(cx);
    // folders first (a chain of single folders on one row), then files, each A to Z with numbers in order. The new
    // file is there before it is committed; the ignored `.env` is not
    assert_eq!(
        look(cx, &app, |w| rows(w)),
        ["dir a/b/c", "dir docs", "dir src", ".gitignore", "file9.txt", "file10.txt", "notes.md", "README.md"]
    );
    look(cx, &app, |w| {
        assert_eq!(said(w, "files-count").as_deref(), Some("11 files"));
        // the folder that holds a changed file has the dot, and the new file its letter
        let (dot, src) = (w.find("folder-changed").bounds(), w.find(("folder-row", 2usize)).bounds());
        assert!(dot.top() >= src.top() && dot.bottom() <= src.bottom(), "the dot isn't on src: {dot:?} {src:?}");
        assert_eq!(said(w, "row-kind").as_deref(), Some("New file"));
    });

    // a folder opens under its row, one step in; its folders first
    click_row(cx, &app, "src");
    assert_eq!(
        look(cx, &app, |w| rows(w))[..5],
        ["dir a/b/c", "dir docs", "dir src open", "dir src/orders", "src/app.py"]
    );
    look(cx, &app, |w| {
        let (folder, child) = (w.find(("folder-row", 2usize)).bounds(), w.find(("folder-row", 3usize)).bounds());
        assert_eq!(child.top(), folder.bottom(), "a folder's rows come right under it");
        let kinds: Vec<String> = w.find_all("row-kind").iter().filter_map(|k| k.label().map(String::from)).collect();
        assert_eq!(kinds, ["Modified", "New file"], "the changed file has its letter");
    });
    click_row(cx, &app, "src/orders");
    assert_eq!(
        look(cx, &app, |w| rows(w))[3..8],
        [
            "dir src/orders open",
            "dir src/orders/tests",
            "src/orders/serializers.py",
            "src/orders/stock.py",
            "src/app.py"
        ]
    );

    // a click on a file opens it where the diffs are, and marks its row
    click_row(cx, &app, "src/orders/stock.py");
    assert!(app.until(cx, |w| said(w, "file-view").as_deref() == Some("src/orders/stock.py")), "it never opened");
    look(cx, &app, |w| {
        let marked: Vec<String> = (0..40usize)
            .filter_map(|ix| w.try_find(("file-row", ix)))
            .filter(|row| row.selected() == Some(true))
            .filter_map(|row| row.label().map(String::from))
            .collect();
        assert_eq!(marked, ["src/orders/stock.py"]);
    });

    // files come and go on disk: the tree follows without a press of anything
    write(&folder, "src/orders/returns.py", "new\n");
    std::fs::remove_file(folder.join("README.md")).unwrap();
    assert!(
        app.until(cx, |w| {
            let rows = rows(w);
            rows.contains(&"src/orders/returns.py".to_string()) && !rows.contains(&"README.md".to_string())
        }),
        "the tree didn't follow the folder"
    );
    assert_eq!(look(cx, &app, |w| said(w, "files-count")).as_deref(), Some("11 files"), "one came, one went");

    // a folder closes again, and "Read the files again" keeps what is open
    click_row(cx, &app, "src/orders");
    assert!(app.until(cx, |w| !rows(w).contains(&"src/orders/stock.py".to_string())));
    app.click(cx, "files-refresh");
    cx.run_until_parked();
    assert!(app.until(cx, |w| rows(w).contains(&"dir src open".to_string())), "the open folder closed");
}

#[gpui_kit::test]
fn the_filter_ranks_file_names_first_and_enter_opens_the_best_match(cx: &mut TestAppContext) {
    let (app, folder) = start(cx);
    // typed in the filter: the file whose name starts with it first, then names that hold it, each with its folder
    app.click(cx, "files-filter");
    app.typed(cx, "stock");
    assert!(
        app.until(cx, |w| {
            rows(w) == ["src/orders/stock.py", "docs/stock-notes.md", "src/orders/tests/test_stock.py"]
        }),
        "the filter's matches: {:?}",
        look(cx, &app, |w| rows(w))
    );
    look(cx, &app, |w| assert!(w.try_find("files-note").is_none(), "a note with nothing to say"));

    // Enter opens the best match
    app.press(cx, "enter");
    assert!(app.until(cx, |w| said(w, "file-view").as_deref() == Some("src/orders/stock.py")), "Enter did nothing");

    // the filter cleared with its button: the tree is back, with the folders of the file that was picked open
    app.click(cx, "files-clear");
    assert!(app.until(cx, |w| rows(w).contains(&"dir src/orders open".to_string())), "its folders didn't open");
    look(cx, &app, |w| assert!(w.try_find("files-clear").is_none(), "a button to clear nothing"));

    // nothing matches: it says so
    app.typed(cx, "zzz");
    assert!(app.until(cx, |w| said(w, "files-note").as_deref() == Some("No file matches \u{201c}zzz\u{201d}.")));
    assert!(look(cx, &app, |w| rows(w)).is_empty());
    // Escape clears what was typed
    app.press(cx, "escape");
    assert!(app.until(cx, |w| w.try_find("files-note").is_none() && rows(w).contains(&"dir docs".to_string())));

    // more matches than are shown: it says how many more
    for n in 0..230 {
        write(&folder, &format!("many/part_{n:03}.txt"), "x\n");
    }
    assert!(app.until(cx, |w| said(w, "files-count").as_deref() == Some("241 files")), "the new files never came");
    app.typed(cx, "part_");
    assert!(
        app.until(cx, |w| {
            said(w, "files-note").as_deref() == Some("30 more. Type more of the name to narrow it down.")
        }),
        "{:?}",
        look(cx, &app, |w| said(w, "files-note"))
    );
    assert_eq!(row_count(cx, &app.shell), 200);
    // letters in order find a file too, after the names that hold them
    app.press(cx, "escape");
    app.typed(cx, "sostk");
    assert!(app.until(cx, |w| rows(w).first().map(String::as_str) == Some("src/orders/stock.py")));
}

#[gpui_kit::test]
fn a_folder_shows_300_files_and_the_rest_with_show_all(cx: &mut TestAppContext) {
    let (app, folder) = start(cx);
    for n in 0..350 {
        write(&folder, &format!("many/part_{n:03}.txt"), "x\n");
    }
    assert!(app.until(cx, |w| said(w, "files-count").as_deref() == Some("361 files")), "the new files never came");
    let before = row_count(cx, &app.shell);
    click_row(cx, &app, "many");
    // 300 of its files, and the row that shows the rest
    assert_eq!(row_count(cx, &app.shell), before + 300 + 1);
    // that row is far down: scrolled to, it says how many there are in all
    cx.update_window(app.window, |_, w, cx| {
        w.scroll("files-tree", ScrollDelta::Pixels(point(px(0.), px(-26. * 400.))), cx);
    })
    .unwrap();
    assert!(
        app.until(cx, |w| rows(w).contains(&"Show all 350 files".to_string())),
        "{:?}",
        look(cx, &app, |w| rows(w))
    );
    click_row(cx, &app, "Show all 350 files");
    assert_eq!(row_count(cx, &app.shell), before + 350);
    assert!(app.until(cx, |w| !rows(w).contains(&"Show all 350 files".to_string())));
}

/// A guard: 50,000 files in one folder (the most git's list holds), all shown. The tree is built off the drawing
/// thread; a frame draws only the rows in view, so scrolling them costs what scrolling fifty costs.
#[gpui_kit::test]
fn fifty_thousand_files_in_one_folder_draw_like_fifty(cx: &mut TestAppContext) {
    let files: Vec<String> = (0..50_000).map(|n| format!("big/module_{:05}.py", n)).collect();
    let shell = cx.update(|cx| {
        enkidu_app::init(cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Default::default(),
                size: size(px(1360.), px(860.)),
            })),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| {
                let terminal = std::sync::Arc::new(enkidu_term::Terminal::kept(
                    enkidu_term::GridSize::new(120, 36),
                    enkidu_term::Palette::enkidu(),
                ));
                let view = cx.new(|cx| enkidu_app::terminal_view::TerminalView::new(terminal, None, cx));
                let shell = Shell::with_changes_and_files(
                    enkidu_app::demo::tabs(),
                    vec![view],
                    enkidu_app::demo::changes(),
                    files,
                    enkidu_model::ui::DiffLayout::Split,
                    window,
                    cx,
                );
                shell.ui().update(cx, |ui, cx| {
                    ui.toggle_panel(enkidu_model::ui::Panel::Files);
                    cx.notify();
                });
                shell
            })
        })
        .expect("a test window")
    });
    let (window, shell) = (shell.0, shell.1);
    let frame = |cx: &mut TestAppContext| {
        cx.run_until_parked();
        cx.update_window(window, |_, w, cx| w.render_frame(cx)).unwrap();
    };
    for _ in 0..3 {
        frame(cx);
    }
    // the one folder, closed, and the folder of the demo's new file
    assert_eq!(row_count(cx, &shell), 2);
    cx.update_window(window, |_, w, cx| w.click(("folder-row", 0usize), cx)).unwrap();
    frame(cx);
    assert_eq!(row_count(cx, &shell), 2 + 300 + 1);
    // "Show all 50,000 files"
    cx.update_window(window, |_, w, cx| {
        w.scroll("files-tree", ScrollDelta::Pixels(point(px(0.), px(-26. * 400.))), cx);
        w.render_frame(cx);
        assert_eq!(said(w, ("show-all", 301usize)).as_deref(), Some("Show all 50,000 files"));
        w.click(("show-all", 301usize), cx);
    })
    .unwrap();
    frame(cx);
    assert_eq!(row_count(cx, &shell), 50_002);

    // scrolled frames: only the rows in view are made, so each takes about what an empty frame takes
    // three rounds, the best one counts: the Mac is often busy with other work
    let mut rounds: Vec<(f64, f64)> = Vec::new();
    for _ in 0..3 {
        let mut times: Vec<f64> = Vec::new();
        for step in 0..40 {
            cx.update_window(window, |_, w, cx| {
                w.scroll("files-tree", ScrollDelta::Pixels(point(px(0.), px(-26. * 37.))), cx);
                let started = Instant::now();
                w.render_frame(cx);
                if step >= 5 {
                    times.push(started.elapsed().as_secs_f64() * 1000.);
                }
            })
            .unwrap();
        }
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        rounds.push((times[times.len() / 2], times[times.len() - 1]));
    }
    rounds.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let (middle, worst) = rounds[0];
    println!("FILES 50,000 in one folder: a scrolled frame {middle:.2} ms in the middle, {worst:.2} ms at worst");
    // this build isn't optimized: the bound only tells a frame that walks every file (tens of milliseconds) from
    // one that draws what is in view
    assert!(middle < 12., "a scrolled frame took {middle:.2} ms in the middle, in the best of three rounds");
    // and far down the list the rows are there
    let seen = cx
        .update_window(window, |_, w, cx| {
            w.render_frame(cx);
            (1_000..12_000usize).find_map(|ix| w.try_find(("file-row", ix)))
        })
        .unwrap();
    assert!(seen.is_some_and(|row| row.label().is_some_and(|path| path.starts_with("big/module_"))));
}

/// Opens the right-edge panels, files read-only, and a changed file's diff.
#[gpui_kit::test]
fn opens_the_right_edge_panels_files_read_only_and_a_changed_files_diff(cx: &mut TestAppContext) {
    let (app, _) = start(cx);
    // the changes are put away; Files is open, and its own button closes it
    app.press(cx, "alt-cmd-b");
    assert!(app.until(cx, |w| w.try_find("island-changes").is_none()), "⌥⌘B");
    app.click(cx, "rail-files");
    assert!(app.until(cx, |w| w.try_find("island-panel").is_none()), "the panel stayed");
    // a closed panel stays closed after a restart: that is what is kept
    let kept = app.home.path().join("data").join("ui.json");
    for _ in 0..600 {
        cx.executor().advance_clock(std::time::Duration::from_millis(400));
        cx.run_until_parked();
        if std::fs::read_to_string(&kept).is_ok_and(|text| text.contains(r#""rightPanel":null"#)) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(std::fs::read_to_string(&kept).unwrap_or_default().contains(r#""rightPanel":null"#), "not kept");

    // opening a file shows the changes area, where the file appears, read only
    app.click(cx, "rail-files");
    assert!(app.until(cx, |w| !rows(w).is_empty()), "Files never opened again");
    click_row(cx, &app, "src");
    click_row(cx, &app, "src/app.py");
    assert!(app.until(cx, |w| said(w, "file-view").as_deref() == Some("src/app.py")), "the file never opened");
    assert!(look(cx, &app, |w| w.try_find("island-changes").is_some()), "the changes area stayed put away");
    let ui = app.shell.read_with(cx, |shell, cx| shell.ui().read(cx).clone());
    assert!(!ui.review_hidden);

    // picking a changed file leaves the file view and brings its diff, every time
    app.press(cx, "alt-cmd-1");
    assert!(app.until(cx, |w| w.try_find(("change-row", 0usize)).is_some()), "Changed files never opened");
    for _ in 0..2 {
        app.click(cx, ("change-row", 0usize));
        assert!(app.until(cx, |w| w.try_find("file-view").is_none() && w.try_find("diff-view").is_some()));
        assert_eq!(look(cx, &app, |w| w.find(("change-row", 0usize)).selected()), Some(true));
    }
    // (the first of the changed files: a folder's stands before the top's own)
    assert_eq!(
        look(cx, &app, |w| w.find(("change-row", 0usize)).label().map(String::from)).as_deref(),
        Some("src/app.py")
    );

    // a folder that closes keeps what was open inside it
    app.press(cx, "alt-cmd-2");
    assert!(app.until(cx, |w| rows(w).contains(&"dir src open".to_string())), "src isn't open any more");
    click_row(cx, &app, "src/orders");
    assert!(app.until(cx, |w| rows(w).contains(&"dir src/orders open".to_string())));
    click_row(cx, &app, "src");
    assert!(
        app.until(cx, |w| rows(w).contains(&"dir src".to_string())
            && !rows(w).iter().any(|row| row.contains("orders")))
    );
    click_row(cx, &app, "src");
    assert!(app.until(cx, |w| rows(w).contains(&"dir src/orders open".to_string())), "what was open inside closed");
}

/// DESIGN.md, principle 5: "Loading words ("Reading the file…", "Starting Claude…") show only when a wait is long
/// enough to notice (0.4 to 0.8 s); a quick load goes from nothing straight to the result." In a window whose reads
/// never come back: the Files panel says "Reading files…" once 600 ms have passed and an opened file "Reading the
/// file…" once 400 ms have, and neither says anything before.
#[gpui_kit::test]
fn what_takes_long_to_read_says_so_only_after_a_wait(cx: &mut TestAppContext) {
    use enkidu_app::changes_island::STILL_READING;
    let mut changes = enkidu_app::demo::changes();
    changes[0].1 = STILL_READING.to_string();
    let slow = changes[0].0.path.clone();
    let shell = cx.update(|cx| {
        enkidu_app::init(cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Default::default(),
                size: size(px(1360.), px(860.)),
            })),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| {
                let terminal = std::sync::Arc::new(enkidu_term::Terminal::kept(
                    enkidu_term::GridSize::new(120, 36),
                    enkidu_term::Palette::enkidu(),
                ));
                let view = cx.new(|cx| enkidu_app::terminal_view::TerminalView::new(terminal, None, cx));
                let shell = Shell::with_changes_and_files(
                    enkidu_app::demo::tabs(),
                    vec![view],
                    changes,
                    vec![STILL_READING.to_string()],
                    enkidu_model::ui::DiffLayout::Split,
                    window,
                    cx,
                );
                shell.ui().update(cx, |ui, cx| {
                    ui.toggle_panel(enkidu_model::ui::Panel::Files);
                    cx.notify();
                });
                shell
            })
        })
        .expect("a test window")
    });
    let (window, shell) = (shell.0, shell.1);
    let said_now = |cx: &mut TestAppContext, id: &'static str| {
        cx.run_until_parked();
        cx.update_window(window, |_, w, cx| {
            w.render_frame(cx);
            said(w, id)
        })
        .unwrap()
    };
    let wait = |cx: &mut TestAppContext, ms: u64| cx.executor().advance_clock(std::time::Duration::from_millis(ms));

    // the Files panel: nothing while the wait is short, the words once it is long
    assert_eq!(said_now(cx, "files-note"), None);
    wait(cx, 590);
    assert_eq!(said_now(cx, "files-note"), None, "it spoke before 600 ms");
    wait(cx, 20);
    assert_eq!(said_now(cx, "files-note").as_deref(), Some("Reading files…"));

    // a file opened where the diffs are
    let island = shell.read_with(cx, |shell, _| shell.changes().cloned()).expect("the island");
    cx.update(|cx| island.update(cx, |island, cx| island.open_file(&slow, None, cx)));
    assert_eq!(said_now(cx, "file-view").as_deref(), Some(slow.as_str()), "it never opened");
    assert_eq!(said_now(cx, "file-note"), None);
    wait(cx, 390);
    assert_eq!(said_now(cx, "file-note"), None, "it spoke before 400 ms");
    wait(cx, 20);
    assert_eq!(said_now(cx, "file-note").as_deref(), Some("Reading the file…"));
}
