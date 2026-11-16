//! Item 1.21: the palette's first form. ⌘K (or Commands) opens it over every workspace of every project and the
//! actions that work; typing narrows it; ↑ and ↓ move around the ends; Enter or a click goes there and closes it;
//! Esc or ⌘K closes it.

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

fn ws(id: &str, project_id: &str, name: &str, branch: &str) -> Workspace {
    Workspace {
        id: id.into(),
        project_id: project_id.into(),
        name: name.into(),
        path: format!("/ws/{name}"),
        branch: branch.into(),
        base: None,
        session_id: String::new(),
        transcript_path: None,
        claude_config_dir: None,
        created_at: 1,
        adopted: false,
    }
}

fn open(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Frame>) {
    let frame = Frame::load(
        vec![project("p1", "shop"), project("p2", "tools")],
        vec![
            ws("w1", "p1", "stock-fix", "fix/stock-double-count"),
            ws("w2", "p1", "sh-21309", "feature/SH-21309-returns"),
            ws("w3", "p2", "lint", "chore/lint"),
        ],
        Default::default(),
        &View::default(),
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

fn act(cx: &mut TestAppContext, window: AnyWindowHandle, f: impl FnOnce(&mut Window, &mut gpui_kit::App)) {
    cx.update_window(window, |_, w, cx| f(w, cx)).unwrap();
    cx.run_until_parked();
}

/// The rows shown, by name, and which one is picked.
fn rows(cx: &mut TestAppContext, window: AnyWindowHandle) -> (Vec<String>, Option<usize>) {
    look(cx, window, |w| {
        let all: Vec<_> = (0..20usize).map_while(|ix| w.try_find(("palette-row", ix))).collect();
        let names = all.iter().map(|r| r.label().unwrap_or_default().to_string()).collect();
        (names, all.iter().position(|r| r.selected() == Some(true)))
    })
}

fn on_screen(cx: &mut TestAppContext, frame: &Entity<Frame>) -> Option<String> {
    frame.read_with(cx, |f, _| f.current_workspace().map(|w| w.name.clone()))
}

#[gpui_kit::test]
fn cmd_k_lists_every_workspace_then_the_actions(cx: &mut TestAppContext) {
    let (window, _) = open(cx);
    act(cx, window, |w, cx| w.press("cmd-k", cx));
    let (names, picked) = rows(cx, window);
    assert_eq!(
        names,
        [
            "stock-fix",
            "sh-21309",
            "lint",
            "New workspace",
            "Go live with stock-fix",
            "Next workspace",
            "Previous workspace",
            "Add project",
            "Settings"
        ]
    );
    assert_eq!(picked, Some(0));
    // ⌘K again closes it
    act(cx, window, |w, cx| w.press("cmd-k", cx));
    assert!(look(cx, window, |w| w.try_find("command-palette").is_none()));
    // and Commands opens it too
    act(cx, window, |w, cx| w.click("palette", cx));
    assert!(look(cx, window, |w| w.try_find("command-palette").is_some()), "Commands opens it");
}

#[gpui_kit::test]
fn typing_narrows_it_and_enter_goes_there(cx: &mut TestAppContext) {
    let (window, frame) = open(cx);
    act(cx, window, |w, cx| w.press("cmd-k", cx));
    act(cx, window, |w, cx| w.input("lint", cx));
    assert_eq!(rows(cx, window).0, ["lint"]);
    act(cx, window, |w, cx| w.press("enter", cx));
    assert!(look(cx, window, |w| w.try_find("command-palette").is_none()), "it closed");
    assert_eq!(on_screen(cx, &frame).as_deref(), Some("lint"), "the other project's workspace is on screen");
    // nothing that matches says so
    act(cx, window, |w, cx| w.press("cmd-k", cx));
    act(cx, window, |w, cx| w.input("zzzz", cx));
    assert_eq!(
        look(cx, window, |w| w.try_find("palette-empty").and_then(|e| e.label().map(String::from))).as_deref(),
        Some("Nothing matches")
    );
    act(cx, window, |w, cx| w.press("escape", cx));
    assert!(look(cx, window, |w| w.try_find("command-palette").is_none()), "Esc closed it");
}

#[gpui_kit::test]
fn arrows_move_around_the_ends_and_a_click_goes_there(cx: &mut TestAppContext) {
    let (window, frame) = open(cx);
    act(cx, window, |w, cx| w.press("cmd-k", cx));
    act(cx, window, |w, cx| w.press("down", cx));
    assert_eq!(rows(cx, window).1, Some(1));
    act(cx, window, |w, cx| w.press("up", cx));
    act(cx, window, |w, cx| w.press("up", cx));
    assert_eq!(rows(cx, window).1, Some(8), "up from the first is the last");
    act(cx, window, |w, cx| w.press("down", cx));
    assert_eq!(rows(cx, window).1, Some(0), "and down from the last is the first");
    act(cx, window, |w, cx| w.click(("palette-row", 1usize), cx));
    assert!(look(cx, window, |w| w.try_find("command-palette").is_none()));
    assert_eq!(on_screen(cx, &frame).as_deref(), Some("sh-21309"));
}

#[gpui_kit::test]
fn new_workspace_from_the_palette_opens_its_dialog(cx: &mut TestAppContext) {
    let (window, _) = open(cx);
    act(cx, window, |w, cx| w.press("cmd-k", cx));
    act(cx, window, |w, cx| w.input("new work", cx));
    assert_eq!(rows(cx, window).0.first().map(String::as_str), Some("New workspace"));
    act(cx, window, |w, cx| w.press("enter", cx));
    assert!(
        look(cx, window, |w| w.try_find("command-palette").is_none() && w.try_find("new-workspace-dialog").is_some())
    );
}
