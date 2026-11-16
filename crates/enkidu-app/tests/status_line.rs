//! Item 1.18: the status line says in words, for the workspace on screen, its branch and where it started, what
//! Claude is doing, Live, and its changes; a click on the folder copies its path. With no workspace it says nothing.

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

fn open(cx: &mut TestAppContext, frame: Frame) -> (AnyWindowHandle, Entity<Shell>, Entity<Frame>) {
    cx.update(|cx| {
        enkidu_app::init(cx);
        let frame = cx.new(|_| frame);
        let on = frame.clone();
        let (window, shell) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Default::default(),
                    size: size(px(1360.), px(860.)),
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

fn settle(cx: &mut TestAppContext, window: AnyWindowHandle) {
    for _ in 0..3 {
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| window.render_frame(cx)).unwrap();
    }
}

fn said(window: &Window, id: &'static str) -> Option<String> {
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
    let (window, _, frame) = open(cx, start);
    settle(cx, window);
    cx.update_window(window, |_, w, _| {
        assert_eq!(said(w, "status-branch").as_deref(), Some("fix/stock-double-count"));
        assert_eq!(said(w, "status-base").as_deref(), Some("from development"));
        assert_eq!(said(w, "status-claude").as_deref(), Some("Claude is working"));
        assert_eq!(said(w, "status-live").as_deref(), Some("Live in shop's dev environment"));
        assert_eq!(said(w, "status-changes").as_deref(), Some("5 changed files"));
        assert_eq!(said(w, "status-counts").as_deref(), Some("+13 −6"));
        assert_eq!(said(w, "status-path").as_deref(), Some("~/enkidu/workspaces/shop/stock-double-count"));
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
    })
    .unwrap();
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
        assert_eq!(said(w, "status-changes"), None, "nothing about nothing");
        assert_eq!(said(w, "status-counts"), None);
        assert_eq!(said(w, "status-live"), None);
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
fn with_no_workspace_the_line_keeps_its_place_and_says_nothing(cx: &mut TestAppContext) {
    let (window, _, _) = open(cx, frame(vec![]));
    settle(cx, window);
    cx.update_window(window, |_, w, _| {
        assert_eq!(w.find("status-line").bounds().size.height, px(26.));
        assert_eq!(said(w, "status-claude"), None);
        assert_eq!(said(w, "status-path"), None);
    })
    .unwrap();
}
