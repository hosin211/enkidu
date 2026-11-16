//! The projects row and the tabs row with given data (item 1.22). A PR's chip on its tab needs the Work data of phase 5
//! and is tested there.

use enkidu_app::shell::Shell;
use enkidu_core::db::{Project, Workspace};
use enkidu_model::frame::{Frame, View};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, Entity, TestAppContext, Window, WindowBounds, WindowOptions, px, size,
};

fn project(id: &str, name: &str) -> Project {
    Project { id: id.into(), name: name.into(), path: format!("/code/{name}"), live_workspace_id: None, created_at: 1 }
}

fn ws(id: &str, project_id: &str, name: &str) -> Workspace {
    Workspace {
        id: id.into(),
        project_id: project_id.into(),
        name: name.into(),
        path: format!("/ws/{name}"),
        branch: format!("feature/{name}"),
        base: Some("development".into()),
        session_id: format!("s{id}"),
        transcript_path: None,
        claude_config_dir: None,
        created_at: 1,
        adopted: false,
    }
}

/// What these tests start from: shop on screen with stock-fix working and export waiting for you; enkidu with polish
/// and search both waiting.
fn setup(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Frame>) {
    let statuses = [("w1", "working"), ("w2", "needs_you"), ("w3", "needs_you"), ("w4", "needs_you")]
        .into_iter()
        .map(|(id, s)| (id.to_string(), s.to_string()))
        .collect();
    let view = View {
        project_id: Some("p1".into()),
        selected: [("p1".to_string(), "w1".to_string()), ("p2".to_string(), "w3".to_string())].into(),
        work: false,
        ..Default::default()
    };
    let frame = Frame::load(
        vec![project("p1", "shop"), project("p2", "enkidu")],
        vec![ws("w1", "p1", "stock-fix"), ws("w2", "p1", "export"), ws("w3", "p2", "polish"), ws("w4", "p2", "search")],
        statuses,
        &view,
    );
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

fn tab_names(w: &Window) -> Vec<String> {
    (0..9usize).map_while(|ix| w.try_find(("tab", ix))).map(|t| t.label().unwrap_or_default().to_string()).collect()
}

#[gpui_kit::test]
fn lists_the_projects_by_name_with_a_red_count_on_another_project_when_its_workspaces_need_you(
    cx: &mut TestAppContext,
) {
    let (window, _) = setup(cx);
    look(cx, window, |w| {
        assert_eq!(w.find(("project", 0usize)).label(), Some("shop: project menu"), "shop is on screen: its menu");
        assert_eq!(w.find(("project", 1usize)).label(), Some("enkidu"));
        // the project on screen shows its questions in its own tabs, not as a count
        assert!(w.try_find(("needs-you", 0usize)).is_none());
        assert_eq!(w.find(("needs-you", 1usize)).label(), Some("2 workspaces need you"));
        // no search field in the top row: the keyboard opens Search
        assert!(w.try_find("open-search").is_none());
    });
}

#[gpui_kit::test]
fn shows_the_projects_own_workspaces_as_tabs_says_who_needs_you_and_switches_project_on_a_click(
    cx: &mut TestAppContext,
) {
    let (window, frame) = setup(cx);
    look(cx, window, |w| {
        assert_eq!(tab_names(w), ["stock-fix", "export"]);
        assert_eq!(w.find(("tab-words", 1usize)).label(), Some("Needs you"));
    });
    // ⌘1 to ⌘9 follow these tabs
    cx.update_window(window, |_, w, cx| w.press("cmd-2", cx)).unwrap();
    assert_eq!(frame.read_with(cx, |f, _| f.current_workspace().map(|w| w.name.clone())).as_deref(), Some("export"));
    cx.update_window(window, |_, w, cx| w.press("cmd-3", cx)).unwrap();
    assert_eq!(
        frame.read_with(cx, |f, _| f.current_workspace().map(|w| w.name.clone())).as_deref(),
        Some("export"),
        "there is no third tab"
    );
    // a click on the other project puts it on screen, with its own tabs
    cx.update_window(window, |_, w, cx| w.click(("project", 1usize), cx)).unwrap();
    assert_eq!(frame.read_with(cx, |f, _| f.current_project.clone()).as_deref(), Some("p2"));
    look(cx, window, |w| assert_eq!(tab_names(w), ["polish", "search"]));
    cx.update_window(window, |_, w, cx| w.press("cmd-1", cx)).unwrap();
    assert_eq!(frame.read_with(cx, |f, _| f.current_workspace().map(|w| w.name.clone())).as_deref(), Some("polish"));
}
