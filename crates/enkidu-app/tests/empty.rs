//! Item 1.20: the empty screens. With no project, how to start and Add project; with no workspace in the project on
//! screen, New workspace (and ⌘N, as the window holds the keyboard); nothing at all until the database was read; and
//! when the last workspace goes, the empty screen comes back with the keyboard.

use enkidu_app::shell::Shell;
use enkidu_core::db::{Project, Workspace};
use enkidu_model::frame::{Frame, View};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, Entity, TestAppContext, Window, WindowBounds, WindowOptions, px, size,
};

fn shop() -> Project {
    Project { id: "p1".into(), name: "shop".into(), path: "/code/shop".into(), live_workspace_id: None, created_at: 1 }
}

fn workspace() -> Workspace {
    Workspace {
        id: "w1".into(),
        project_id: "p1".into(),
        name: "stock-fix".into(),
        path: "/ws/stock-fix".into(),
        branch: "fix/stock".into(),
        base: Some("main".into()),
        session_id: "s1".into(),
        transcript_path: None,
        claude_config_dir: None,
        created_at: 1,
        adopted: false,
    }
}

fn open(cx: &mut TestAppContext, frame: Frame) -> (AnyWindowHandle, Entity<Frame>) {
    cx.update(|cx| {
        enkidu_app::init(cx);
        let frame = cx.new(|_| frame);
        let on = frame.clone();
        let (window, _) = gpui_kit::open_window(
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
        (window, frame)
    })
}

fn look<R>(cx: &mut TestAppContext, window: AnyWindowHandle, at: impl FnOnce(&mut Window) -> R) -> R {
    cx.run_until_parked();
    cx.update_window(window, |_, w, cx| {
        w.render_frame(cx);
        at(w)
    })
    .unwrap()
}

fn label(window: &Window, id: &'static str) -> Option<String> {
    window.try_find(id).and_then(|e| e.label().map(String::from))
}

#[gpui_kit::test]
fn nothing_shows_until_the_database_was_read(cx: &mut TestAppContext) {
    let (window, _) = open(cx, Frame::default());
    look(cx, window, |w| {
        assert_eq!(label(w, "empty-projects"), None, "no empty screen before the projects are known");
        assert!(w.try_find("island-claude").is_none());
    });
}

#[gpui_kit::test]
fn with_no_project_the_window_says_how_to_start(cx: &mut TestAppContext) {
    let (window, _) = open(cx, Frame::load(vec![], vec![], Default::default(), &View::default()));
    look(cx, window, |w| {
        assert_eq!(label(w, "empty-projects").as_deref(), Some("Add a project to start"));
        assert_eq!(label(w, "add-first-project").as_deref(), Some("Add project"));
        assert!(w.try_find("island-claude").is_none(), "no terminal");
    });
    cx.update_window(window, |_, w, cx| w.click("add-first-project", cx)).unwrap();
    assert!(cx.did_prompt_for_paths(), "Add project opens the folder picker");
}

#[gpui_kit::test]
fn with_no_workspace_the_window_offers_the_first(cx: &mut TestAppContext) {
    let (window, _) = open(cx, Frame::load(vec![shop()], vec![], Default::default(), &View::default()));
    look(cx, window, |w| {
        assert_eq!(label(w, "empty-workspaces").as_deref(), Some("Start a workspace in shop"));
        assert_eq!(label(w, "status-claude"), None, "the status line says nothing");
    });
    cx.update_window(window, |_, w, cx| w.click("first-workspace", cx)).unwrap();
    assert!(look(cx, window, |w| w.try_find("new-workspace-dialog").is_some()), "New workspace opens the dialog");
    cx.update_window(window, |_, w, cx| w.press("escape", cx)).unwrap();
    assert!(look(cx, window, |w| w.try_find("new-workspace-dialog").is_none()));
    // the window holds the keyboard, so ⌘N works here too
    cx.update_window(window, |_, w, cx| w.press("cmd-n", cx)).unwrap();
    assert!(look(cx, window, |w| w.try_find("new-workspace-dialog").is_some()), "⌘N opens it");
}

#[gpui_kit::test]
fn when_the_last_workspace_goes_the_empty_screen_takes_the_keyboard(cx: &mut TestAppContext) {
    let (window, frame) = open(cx, Frame::load(vec![shop()], vec![workspace()], Default::default(), &View::default()));
    look(cx, window, |w| {
        assert!(w.try_find("island-claude").is_some(), "the workspace's island");
        assert_eq!(label(w, "empty-workspaces"), None);
    });
    frame.update(cx, |f, cx| {
        f.remove_workspace("w1");
        cx.notify();
    });
    look(cx, window, |w| assert_eq!(label(w, "empty-workspaces").as_deref(), Some("Start a workspace in shop")));
    cx.update_window(window, |_, w, cx| w.press("cmd-n", cx)).unwrap();
    assert!(look(cx, window, |w| w.try_find("new-workspace-dialog").is_some()), "⌘N reaches the window");
}
