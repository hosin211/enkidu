//! Item 1.18: the status line says in words, for the workspace on screen, its branch and where it started, what
//! Claude is doing, Live, and its changes; a click on the folder copies its path. With no workspace it says nothing.
//! The top row over it has what acts on the workspace: the layout, Go live and its menu, each a control's size.

use enkidu_app::shell::Shell;
use enkidu_core::db::{Project, Workspace};
use enkidu_core::events::Event;
use enkidu_model::frame::{Changes, Frame, View};
use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, Entity, TestAppContext, Window, WindowBounds, WindowOptions, px, size,
};

fn home() -> String {
    std::env::var("HOME").unwrap()
}

fn workspace(id: &str, name: &str, base: Option<&str>) -> Workspace {
    Workspace {
        id: id.into(),
        project_id: "p1".into(),
        name: name.into(),
        path: format!("{}/enkidu/workspaces/shop/{name}", home()),
        branch: format!("fix/{name}"),
        base: base.map(String::from),
        session_id: format!("s-{id}"),
        transcript_path: None,
        claude_config_dir: None,
        created_at: 1,
        adopted: false,
    }
}

fn frame(workspaces: Vec<Workspace>) -> Frame {
    let project = Project {
        id: "p1".into(),
        name: "shop".into(),
        path: "/code/shop".into(),
        live_workspace_id: None,
        created_at: 1,
    };
    Frame::load(vec![project], workspaces, Default::default(), &View::default())
}

fn open_wide(cx: &mut TestAppContext, frame: Frame, width: f32) -> (AnyWindowHandle, Entity<Shell>, Entity<Frame>) {
    cx.update(|cx| {
        enkidu_app::init(cx);
        let frame = cx.new(|_| frame);
        let on = frame.clone();
        let (window, shell) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Default::default(),
                    size: size(px(width), px(860.)),
                })),
                ..Default::default()
            },
            cx,
            |window, cx| cx.new(|cx| Shell::kept(on, window, cx)),
        )
        .expect("a test window");
        (window, shell, frame)
    })
}

fn open(cx: &mut TestAppContext, frame: Frame) -> (AnyWindowHandle, Entity<Shell>, Entity<Frame>) {
    open_wide(cx, frame, 1360.)
}

fn settle(cx: &mut TestAppContext, window: AnyWindowHandle) {
    for _ in 0..3 {
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| window.render_frame(cx)).unwrap();
    }
}

fn said(window: &Window, id: impl Into<gpui_kit::ElementId>) -> Option<String> {
    window.try_find(id).and_then(|e| e.label().map(String::from))
}

fn news(cx: &mut TestAppContext, frame: &Entity<Frame>, event: Event) {
    frame.update(cx, |f, cx| {
        f.apply(&event);
        cx.notify();
    });
}

fn change(path: &str, additions: u64, deletions: u64) -> enkidu_core::git::Change {
    enkidu_core::git::Change {
        path: path.into(),
        old_path: None,
        kind: "modified".into(),
        staged: false,
        additions: Some(additions),
        deletions: Some(deletions),
        sig: "s".into(),
    }
}

#[gpui_kit::test]
fn the_line_says_where_the_workspace_is_and_what_claude_does(cx: &mut TestAppContext) {
    let mut start = frame(vec![workspace("w1", "stock-double-count", Some("development"))]);
    start.statuses.insert("w1".into(), "working".into());
    start.changed.insert("w1".into(), Changes { files: 5, additions: 13, deletions: 6 });
    start.projects[0].live_workspace_id = Some("w1".into());
    start.remarks.insert("w1".into(), 2);
    let (window, _, frame) = open(cx, start);
    settle(cx, window);
    cx.update_window(window, |_, w, _| {
        assert_eq!(said(w, "status-branch").as_deref(), Some("fix/stock-double-count"));
        assert_eq!(said(w, "status-base").as_deref(), Some("from development"));
        assert_eq!(said(w, "status-claude").as_deref(), Some("Claude is working"));
        assert_eq!(said(w, "status-live").as_deref(), Some("Live in shop's dev environment"));
        assert_eq!(said(w, "status-remarks").as_deref(), Some("2 remarks not sent"));
        assert_eq!(said(w, "status-changes").as_deref(), Some("5 changed files"));
        assert_eq!(said(w, "status-counts").as_deref(), Some("+13 −6"));
        assert_eq!(said(w, "status-path").as_deref(), Some("~/enkidu/workspaces/shop/stock-double-count"));
        // the line is the window's foot, 28 px high; its words start 16 px from the window's edge and its folder
        // ends 16 px before the other
        let (line, branch, path) =
            (w.find("status-line").bounds(), w.find("status-branch-item").bounds(), w.find("status-path").bounds());
        assert_eq!(
            (line.size.height, line.bottom(), line.left(), line.right()),
            (px(28.), px(860.), px(0.), px(1360.))
        );
        assert_eq!((branch.left(), path.right()), (px(16.), px(1360. - 16.)));
        // left to right: the branch, Claude, Live, the remarks; then at the right the changes and the folder
        let lefts: Vec<_> = ["status-branch-item", "status-claude", "status-live", "status-remarks", "status-changes"]
            .into_iter()
            .map(|id| w.find(id).bounds().left())
            .collect();
        assert!(lefts.windows(2).all(|pair| pair[0] < pair[1]), "{lefts:?}");
        assert!(w.find("status-changes").bounds().right() < path.left());
        // what Claude does is said here, and not a second time beside Claude's tabs
        let (words, tabs) = (w.find("status-claude").bounds(), w.find("terminal-tabs").bounds());
        assert!(words.top() >= tabs.bottom(), "{words:?} under {tabs:?}");
        // Live: the button in the top row, and the pill on the workspace's tab
        assert_eq!(said(w, "live-button").as_deref(), Some("Live"));
        assert_eq!(said(w, ("tab-live", 0usize)).as_deref(), Some("Live"));
        assert_eq!(said(w, ("tab-changed", 0usize)).as_deref(), Some("5 changed files"));
    })
    .unwrap();

    // the core's news: Claude needs you, one file is left with two lines added, then none
    news(cx, &frame, Event::StatusChanged { workspace_id: "w1".into(), status: "needs_you".into() });
    news(cx, &frame, Event::ChangesUpdated { workspace_id: "w1".into(), files: vec![change("stock.py", 2, 0)] });
    settle(cx, window);
    cx.update_window(window, |_, w, _| {
        assert_eq!(said(w, "status-claude").as_deref(), Some("Claude needs you"));
        assert_eq!(said(w, "status-changes").as_deref(), Some("1 changed file"));
        assert_eq!(said(w, "status-counts").as_deref(), Some("+2"), "no −0");
        // its tab says only that it needs you
        assert_eq!(said(w, ("tab-words", 0usize)).as_deref(), Some("Needs you"));
        assert_eq!(said(w, ("tab-changed", 0usize)), None);
    })
    .unwrap();
    news(cx, &frame, Event::StatusChanged { workspace_id: "w1".into(), status: "done".into() });
    news(cx, &frame, Event::ChangesUpdated { workspace_id: "w1".into(), files: vec![] });
    news(
        cx,
        &frame,
        Event::LiveChanged(enkidu_core::live::LiveOutcome {
            project_id: "p1".into(),
            live_workspace_id: None,
            note: String::new(),
            output: String::new(),
        }),
    );
    settle(cx, window);
    cx.update_window(window, |_, w, _| {
        assert_eq!(said(w, "status-claude").as_deref(), Some("Claude is done"));
        assert_eq!(said(w, "status-changes"), None, "nothing about nothing");
        assert_eq!(said(w, "status-counts"), None);
        assert_eq!(said(w, "status-live"), None);
        assert_eq!(said(w, ("tab-changed", 0usize)), None);
        assert_eq!(said(w, ("tab-live", 0usize)), None);
        assert_eq!(said(w, "live-button").as_deref(), Some("Go live"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_workspace_on_an_existing_branch_has_no_base(cx: &mut TestAppContext) {
    let (window, _, _) = open(cx, frame(vec![workspace("w1", "review", None)]));
    settle(cx, window);
    cx.update_window(window, |_, w, _| {
        assert_eq!(said(w, "status-branch").as_deref(), Some("fix/review"));
        assert_eq!(said(w, "status-base"), None);
        assert_eq!(said(w, "status-claude").as_deref(), Some("Claude is idle"));
        assert_eq!(said(w, "status-remarks"), None, "nothing about nothing");
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_click_on_the_folder_copies_its_path(cx: &mut TestAppContext) {
    let (window, shell, _) = open(cx, frame(vec![workspace("w1", "stock-double-count", Some("development"))]));
    settle(cx, window);
    let terminal = shell.read_with(cx, |s, cx| s.terminal().read(cx).terminal().clone());
    cx.update_window(window, |_, w, cx| w.click("status-path", cx)).unwrap();
    settle(cx, window);
    let copied = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(copied, Some(format!("{}/enkidu/workspaces/shop/stock-double-count", home())));
    cx.update_window(window, |_, w, _| {
        assert!(w.try_find_by_label("Path copied").is_some(), "a toast says so");
        assert_eq!(said(w, "toast-detail").as_deref(), Some("~/enkidu/workspaces/shop/stock-double-count"));
    })
    .unwrap();
    // and the keyboard is still the terminal's
    cx.update_window(window, |_, w, cx| w.press("x", cx)).unwrap();
    settle(cx, window);
    assert_eq!(terminal.sent(), b"x");
}

#[gpui_kit::test]
fn the_workspace_s_menu_copies_the_folder_s_path(cx: &mut TestAppContext) {
    let (window, shell, _) = open(cx, frame(vec![workspace("w1", "stock-double-count", Some("development"))]));
    settle(cx, window);
    let terminal = shell.read_with(cx, |s, cx| s.terminal().read(cx).terminal().clone());
    cx.update_window(window, |_, w, cx| w.click("ws-menu", cx)).unwrap();
    settle(cx, window);
    cx.update_window(window, |_, w, cx| w.click("ws-copy-path", cx)).unwrap();
    settle(cx, window);
    cx.executor().advance_clock(std::time::Duration::from_millis(300));
    settle(cx, window);
    let copied = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(copied, Some(format!("{}/enkidu/workspaces/shop/stock-double-count", home())));
    cx.update_window(window, |_, w, _| {
        assert!(w.try_find_by_label("Path copied").is_some(), "a toast says so");
        assert_eq!(said(w, "toast-detail").as_deref(), Some("~/enkidu/workspaces/shop/stock-double-count"));
    })
    .unwrap();
    // and the keyboard is the terminal's again
    cx.update_window(window, |_, w, cx| w.press("x", cx)).unwrap();
    settle(cx, window);
    assert_eq!(terminal.sent(), b"x");
}

#[gpui_kit::test]
fn with_no_workspace_the_line_keeps_its_place_and_says_nothing(cx: &mut TestAppContext) {
    let (window, _, _) = open(cx, frame(vec![]));
    settle(cx, window);
    cx.update_window(window, |_, w, _| {
        assert_eq!(w.find("status-line").bounds().size.height, px(28.));
        assert_eq!(w.find("tabs-row").bounds().size.height, px(44.));
        assert_eq!(said(w, "status-branch"), None);
        assert_eq!(said(w, "status-claude"), None);
        assert_eq!(said(w, "status-path"), None);
        assert!(w.try_find("ws-menu").is_none() && w.try_find("live-button").is_none());
    })
    .unwrap();
}

/// The top row is 44 px high; the project's name starts after the window's three lights; what acts on the workspace
/// is a control's 28 px high and keeps its place at the right, in the window's first size and in its smallest.
#[gpui_kit::test]
fn the_top_row_s_parts_keep_their_sizes(cx: &mut TestAppContext) {
    let long = "a-rather-long-name-for-a-workspace-that-fixes-the-stock-and-the-returns-and-more";
    let (window, _, _) = open_wide(cx, frame(vec![workspace("w1", long, Some("development"))]), 1360.);
    settle(cx, window);
    let check = |w: &Window, width: f32| {
        let row = w.find("tabs-row").bounds();
        assert_eq!((row.size.height, row.left(), row.top(), row.size.width), (px(44.), px(0.), px(0.), px(width)));
        let live = w.find("live-button").bounds();
        assert_eq!((live.size.height, live.size.width), (px(28.), px(92.)));
        assert_eq!(w.find("ws-menu").bounds().size, size(px(28.), px(28.)));
        assert_eq!(w.find("layout-side").bounds().size, size(px(24.), px(24.)));
        assert_eq!(w.find("workspace-layout").bounds().size.height, px(28.));
        // the menu's box ends 12 px before the edge, and the three stand 8 px apart
        assert_eq!(row.right() - w.find("ws-menu").bounds().right(), px(12.));
        assert_eq!(w.find("ws-menu").bounds().left() - live.right(), px(8.));
        assert_eq!(live.left() - w.find("workspace-layout").bounds().right(), px(8.));
        // the project's name: its box starts 8 px after the lights' room, so its words stand at 84
        let (name, tab) = (w.find("project-menu").bounds(), w.find(("tab", 0usize)).bounds());
        assert_eq!((name.left(), name.size.height), (px(76.), px(28.)));
        // left to right, nothing over anything
        assert!(name.right() < tab.left() && tab.right() < w.find("new-workspace").bounds().left());
        assert!(w.find("new-workspace").bounds().right() < w.find("workspace-layout").bounds().left());
        tab
    };
    let wide = cx.update_window(window, |_, w, _| check(w, 1360.)).unwrap();
    assert_eq!(wide.size.width, px(256.), "a long name is cut: a tab is 256 px at most");
    cx.simulate_window_resize(window, size(px(900.), px(860.)));
    settle(cx, window);
    let narrow = cx.update_window(window, |_, w, _| check(w, 900.)).unwrap();
    assert_eq!(narrow.size.width, px(256.), "one tab has its room in the smallest window too");
}
