//! The top row and the rail with given data (DESIGN.md, "Layout": "The top row is the project on screen", "The rail
//! is everything you have"). The project on screen has its workspaces as tabs; each tab says what its workspace waits
//! for ("Needs you", Live, the number of changed files); ⌘1 to 9 pick the tabs, ⇧⌘] and ⇧⌘[ go along the row and
//! around its ends; a click on a project on the rail goes to it; ⌃⇥ is the switcher, which goes by last use and shows
//! every workspace as a card while ⌃ is held; a tab is dragged to another's place, and a workspace is put away and
//! brought back. A pull request's dot on a tab needs the Work data and is tested with the Work page.

use enkidu_app::shell::Shell;
use enkidu_core::db::{Project, Workspace};
use enkidu_core::events::Event;
use enkidu_model::frame::{Changes, Frame, View};
use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, ElementId, Entity, InputEvent as _, Modifiers, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, ScrollDelta, TestAppContext, Window, WindowBounds,
    WindowOptions, point, px, size,
};

fn files(count: usize) -> Changes {
    Changes { files: count, ..Default::default() }
}

fn project(id: &str, name: &str) -> Project {
    Project { id: id.into(), name: name.into(), path: format!("/code/{name}"), live_workspace_id: None, created_at: 1 }
}

fn workspace_of(project: &str, ix: usize, name: &str) -> Workspace {
    Workspace {
        id: format!("w{ix}"),
        project_id: project.into(),
        name: name.into(),
        path: format!("/ws/{name}"),
        branch: format!("feature/{name}"),
        base: Some("main".into()),
        session_id: format!("s{ix}"),
        transcript_path: None,
        claude_config_dir: None,
        created_at: ix as i64,
        adopted: false,
    }
}

fn workspace(ix: usize, name: &str) -> Workspace {
    workspace_of("p1", ix, name)
}

/// The project "shop" with a workspace for each name, the first on screen.
fn frame(names: &[&str]) -> Frame {
    let workspaces = names.iter().enumerate().map(|(ix, name)| workspace(ix, name)).collect();
    Frame::load(vec![project("p1", "shop")], workspaces, Default::default(), &View::default())
}

/// Two projects: shop on screen with stock-fix working and export waiting for you; enkidu with polish and search
/// both waiting.
fn two_projects() -> Frame {
    let statuses = [("w0", "working"), ("w1", "needs_you"), ("w2", "needs_you"), ("w3", "needs_you")]
        .into_iter()
        .map(|(id, status)| (id.to_string(), status.to_string()))
        .collect();
    Frame::load(
        vec![project("p1", "shop"), project("p2", "enkidu")],
        vec![
            workspace_of("p1", 0, "stock-fix"),
            workspace_of("p1", 1, "export"),
            workspace_of("p2", 2, "polish"),
            workspace_of("p2", 3, "search"),
        ],
        statuses,
        &View::default(),
    )
}

fn open_sized(
    cx: &mut TestAppContext,
    frame: Frame,
    (width, height): (f32, f32),
) -> (AnyWindowHandle, Entity<Shell>, Entity<Frame>) {
    cx.update(|cx| {
        enkidu_app::init(cx);
        let frame = cx.new(|_| frame);
        let on = frame.clone();
        let (window, shell) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Default::default(),
                    size: size(px(width), px(height)),
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
    open_sized(cx, frame, (1360., 860.))
}

/// Draws a few frames, so what is measured in one is used in the next.
fn settle(cx: &mut TestAppContext, window: AnyWindowHandle) {
    for _ in 0..3 {
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| window.render_frame(cx)).unwrap();
    }
}

fn look<R>(cx: &mut TestAppContext, window: AnyWindowHandle, at: impl FnOnce(&mut Window) -> R) -> R {
    cx.update_window(window, |_, window, _| at(window)).unwrap()
}

/// What a piece of a tab says ("tab-words", "tab-live", "tab-changed", "tab-remarks").
fn says(window: &Window, what: &'static str, ix: usize) -> Option<String> {
    window.try_find((what, ix)).and_then(|piece| piece.label().map(String::from))
}

fn selected(cx: &mut TestAppContext, frame: &Entity<Frame>) -> Option<String> {
    frame.read_with(cx, |f, _| f.current_workspace().map(|w| w.name.clone()))
}

fn change(path: &str) -> enkidu_core::git::Change {
    enkidu_core::git::Change {
        path: path.into(),
        old_path: None,
        kind: "modified".into(),
        staged: false,
        additions: Some(1),
        deletions: Some(0),
        sig: "s".into(),
    }
}

#[gpui_kit::test]
fn each_tab_says_what_its_workspace_waits_for(cx: &mut TestAppContext) {
    let mut start = frame(&["stock-fix", "sh-21309", "pr-2175", "docs"]);
    start.changed.extend([("w0".to_string(), files(5)), ("w1".to_string(), files(3)), ("w2".to_string(), files(2))]);
    start.statuses.insert("w0".into(), "working".into());
    start.statuses.insert("w1".into(), "needs_you".into());
    start.projects[0].live_workspace_id = Some("w2".into());
    start.remarks.insert("w3".into(), 2);
    let (window, _, frame) = open(cx, start);
    settle(cx, window);
    look(cx, window, |w| {
        assert_eq!(says(w, "tab-changed", 0).as_deref(), Some("5 changed files"));
        // what needs you says only that
        assert_eq!(says(w, "tab-words", 1).as_deref(), Some("Needs you"));
        assert_eq!(says(w, "tab-changed", 1), None, "needing you comes before its changes");
        // Live, and its changes beside it
        assert_eq!(says(w, "tab-live", 2).as_deref(), Some("Live"));
        assert_eq!(says(w, "tab-changed", 2).as_deref(), Some("2 changed files"));
        // the remarks you wrote and haven't sent
        assert_eq!(says(w, "tab-remarks", 3).as_deref(), Some("2 remarks not sent yet"));
        assert_eq!(says(w, "tab-changed", 3), None, "nothing about nothing");
        assert_eq!(w.find(("tab", 0usize)).label(), Some("stock-fix"));
        assert_eq!(w.find(("tab", 0usize)).selected(), Some(true));
        assert_eq!(w.find(("tab", 1usize)).selected(), Some(false));
        assert_eq!(w.find("tab-strip").label(), Some("Workspaces in shop"));
        assert!(w.try_find_by_label("New workspace in shop").is_some(), "New workspace names its project");
        // the row is 44 px high and starts under the window's edge; a tab is a pill of 28 px in its middle, and
        // what it says stays inside it, 12 px before its end
        let (row, tab) = (w.find("tabs-row").bounds(), w.find(("tab", 0usize)).bounds());
        assert_eq!((row.top(), row.size.height), (px(0.), px(44.)));
        assert_eq!((tab.size.height, tab.top()), (px(28.), px(8.)));
        assert_eq!(w.find(("tab-changed", 0usize)).bounds().right(), tab.right() - px(12.));
        // the tabs stand 4 px apart, and the + after the last one
        let next = w.find(("tab", 1usize)).bounds();
        assert_eq!(next.left() - tab.right(), px(4.));
        assert!(w.find("new-workspace").bounds().left() >= w.find(("tab", 3usize)).bounds().right());
        // the pill with the blue edge stands under the tab on screen
        let pill = w.find("workspace-tabs-pill").bounds();
        assert_eq!((pill.left(), pill.size.width), (tab.left(), tab.size.width));
    });

    // the core's news changes what the rows say
    frame.update(cx, |f, cx| {
        f.apply(&Event::ChangesUpdated { workspace_id: "w3".into(), files: vec![change("README.md")] });
        f.apply(&Event::StatusChanged { workspace_id: "w1".into(), status: "working".into() });
        cx.notify();
    });
    settle(cx, window);
    look(cx, window, |w| {
        assert_eq!(says(w, "tab-changed", 3).as_deref(), Some("1 changed file"));
        assert_eq!(says(w, "tab-changed", 1).as_deref(), Some("3 changed files"), "answered: its count shows again");
        assert_eq!(says(w, "tab-words", 1), None);
    });
}

#[gpui_kit::test]
fn the_top_row_is_the_project_on_screen_and_the_rail_goes_to_another(cx: &mut TestAppContext) {
    let (window, _, frame) = open(cx, two_projects());
    settle(cx, window);
    look(cx, window, |w| {
        // the name of the project on screen opens its menu; the rail has every project, the one on screen picked
        assert_eq!(w.find("project-menu").label(), Some("shop: project menu"));
        assert_eq!(
            (w.find(("project", 0usize)).label(), w.find(("project", 0usize)).selected()),
            (Some("shop"), Some(true))
        );
        assert_eq!(
            (w.find(("project", 1usize)).label(), w.find(("project", 1usize)).selected()),
            (Some("enkidu"), Some(false))
        );
        // the tabs are the workspaces of the project on screen, and only those
        assert_eq!(w.find(("tab", 0usize)).label(), Some("stock-fix"));
        assert_eq!(w.find(("tab", 1usize)).label(), Some("export"));
        assert!(w.try_find(("tab", 2usize)).is_none());
        // who needs you is said on its own tab; a project whose tabs are not on screen counts them in red
        assert_eq!(says(w, "tab-words", 1).as_deref(), Some("Needs you"));
        assert_eq!(w.find(("project-waiting", 1usize)).label(), Some("2 workspaces need you"));
        assert!(w.try_find(("project-waiting", 0usize)).is_none(), "shop's own tabs say it");
        // the rail: 56 px wide between the top row and the status line, its buttons 40 px, 8 px from its edges
        let (rail, shop, enkidu) =
            (w.find("rail").bounds(), w.find(("project", 0usize)).bounds(), w.find(("project", 1usize)).bounds());
        assert_eq!(
            (rail.left(), rail.top(), rail.size.width, rail.bottom()),
            (px(0.), px(44.), px(56.), px(860. - 28.))
        );
        assert_eq!((shop.left(), shop.size.width, shop.size.height), (px(8.), px(40.), px(40.)));
        assert_eq!(enkidu.top() - shop.bottom(), px(8.));
        // top to bottom: the projects, Add project; at the foot the commands and Settings
        let tops: Vec<Pixels> = [
            shop.top(),
            enkidu.top(),
            w.find("add-project").bounds().top(),
            w.find("palette").bounds().top(),
            w.find("settings").bounds().top(),
        ]
        .into();
        assert!(tops.windows(2).all(|pair| pair[0] < pair[1]), "{tops:?}");
        assert_eq!(w.find("settings").bounds().bottom(), rail.bottom() - px(8.));
        // one + for a workspace, in the top row, and no search field anywhere: the keyboard opens Search
        assert!(w.try_find("new-workspace").is_some());
        assert!(w.try_find("open-search").is_none());
        // the stage starts where the rail ends, under the top row, and ends 8 px before the window's edge and at
        // the status line
        let (stage, status) = (w.find("stage").bounds(), w.find("status-line").bounds());
        assert_eq!(
            (stage.left(), stage.top(), stage.right(), stage.bottom()),
            (px(56.), px(44.), px(1352.), status.top())
        );
        assert_eq!((status.size.height, status.bottom()), (px(28.), px(860.)));
    });
    // ⌘1 to ⌘9 follow the tabs
    cx.update_window(window, |_, w, cx| w.press("cmd-2", cx)).unwrap();
    assert_eq!(selected(cx, &frame).as_deref(), Some("export"));
    cx.update_window(window, |_, w, cx| w.press("cmd-3", cx)).unwrap();
    assert_eq!(selected(cx, &frame).as_deref(), Some("export"), "shop has no third tab");
    // a click on the other project on the rail puts it on screen, on the workspace it had
    settle(cx, window);
    cx.update_window(window, |_, w, cx| w.click(("project", 1usize), cx)).unwrap();
    assert_eq!(frame.read_with(cx, |f, _| f.current_project.clone()).as_deref(), Some("p2"));
    assert_eq!(selected(cx, &frame).as_deref(), Some("polish"));
    settle(cx, window);
    look(cx, window, |w| {
        // now enkidu's workspaces are the tabs, and shop counts its own that needs you
        assert_eq!(w.find("project-menu").label(), Some("enkidu: project menu"));
        assert_eq!(w.find(("tab", 0usize)).label(), Some("polish"));
        assert_eq!(w.find(("tab", 0usize)).selected(), Some(true));
        assert_eq!(w.find(("tab", 1usize)).label(), Some("search"));
        assert_eq!(w.find(("project", 1usize)).selected(), Some(true));
        assert_eq!(w.find(("project-waiting", 0usize)).label(), Some("1 workspace needs you"));
        assert!(w.try_find(("project-waiting", 1usize)).is_none());
    });
    cx.update_window(window, |_, w, cx| w.press("cmd-2", cx)).unwrap();
    assert_eq!(selected(cx, &frame).as_deref(), Some("search"));
}

#[gpui_kit::test]
fn the_keys_pick_the_tabs_and_go_along_the_row_and_around_its_ends(cx: &mut TestAppContext) {
    let (window, shell, frame) = open(cx, frame(&["one", "two", "three"]));
    settle(cx, window);
    let terminal = shell.read_with(cx, |s, cx| s.terminal().read(cx).terminal().clone());
    let press = |cx: &mut TestAppContext, key: &str| {
        cx.update_window(window, |_, w, cx| w.press(key, cx)).unwrap();
        settle(cx, window);
        selected(cx, &frame).unwrap()
    };
    assert_eq!(press(cx, "cmd-2"), "two");
    assert_eq!(press(cx, "cmd-9"), "two", "there is no ninth tab");
    // ⇧⌘] and ⇧⌘[ (as the Mac reports them) go along the row
    assert_eq!(press(cx, "cmd-}"), "three");
    assert_eq!(press(cx, "cmd-}"), "one", "around from the last");
    assert_eq!(press(cx, "cmd-{"), "three", "and back");
    assert_eq!(press(cx, "cmd-1"), "one");
    assert!(terminal.sent().is_empty(), "the keys never reach the program: {:?}", terminal.sent());
}

#[gpui_kit::test]
fn a_project_without_workspaces_leaves_new_workspace_to_the_screen_beside_it(cx: &mut TestAppContext) {
    let (window, _, _) = open(cx, frame(&[]));
    settle(cx, window);
    look(cx, window, |w| {
        assert_eq!(w.find("project-menu").label(), Some("shop: project menu"));
        assert!(w.try_find(("tab", 0usize)).is_none());
        assert!(w.try_find("first-workspace").is_some(), "the empty screen has the button");
        assert!(w.try_find("new-workspace").is_none(), "so the top row has none: no button twice on a screen");
        assert!(w.try_find("ws-menu").is_none() && w.try_find("workspace-layout").is_none(), "nothing acts on one");
        assert!(w.try_find("tabs-rule").is_none(), "and no line after the project's name: nothing follows it");
        // the empty screen stands on the stage
        let (stage, screen) = (w.find("stage").bounds(), w.find("empty-workspaces").bounds());
        assert!(screen.left() >= stage.left() && screen.right() <= stage.right(), "{screen:?} on {stage:?}");
    });
}

#[gpui_kit::test]
fn a_click_on_the_window_s_parts_leaves_the_keyboard_with_the_terminal(cx: &mut TestAppContext) {
    let (window, shell, frame) = open(cx, frame(&["one", "two", "three"]));
    settle(cx, window);
    // (with tabs, a line stands between the project's name and them)
    look(cx, window, |w| assert!(w.try_find("tabs-rule").is_some()));
    let terminal = shell.read_with(cx, |s, cx| s.terminal().read(cx).terminal().clone());
    // another tab, the top row's empty middle, the rail's, the status line's (the tab already on screen is the one
    // click that takes the keyboard: see the arrows test)
    for (target, key) in [("tab", "a"), ("tabs-row", "b"), ("rail", "c"), ("status-line", "d")] {
        cx.update_window(window, |_, w, cx| match target {
            "tab" => w.click(("tab", 1usize), cx),
            other => w.click(other, cx),
        })
        .unwrap();
        settle(cx, window);
        cx.update_window(window, |_, w, cx| w.press(key, cx)).unwrap();
        settle(cx, window);
    }
    assert_eq!(selected(cx, &frame).as_deref(), Some("two"));
    assert_eq!(terminal.sent(), b"abcd", "every key after a click reached the program");
}

#[gpui_kit::test]
fn arrows_move_between_the_tabs_and_enter_puts_one_on_screen(cx: &mut TestAppContext) {
    let (window, shell, frame) = open(cx, frame(&["one", "two", "three"]));
    settle(cx, window);
    let terminal = shell.read_with(cx, |s, cx| s.terminal().read(cx).terminal().clone());
    let focused = |cx: &mut TestAppContext| {
        look(cx, window, |w| (0..3usize).filter(|ix| w.find(("tab", *ix)).focused() == Some(true)).collect::<Vec<_>>())
    };
    // a click on the tab on screen gives it the keyboard
    cx.update_window(window, |_, w, cx| w.click(("tab", 0usize), cx)).unwrap();
    settle(cx, window);
    assert_eq!(focused(cx), [0]);
    for (key, at) in [("right", 1), ("right", 2), ("right", 0), ("left", 2)] {
        cx.update_window(window, |_, w, cx| w.press(key, cx)).unwrap();
        settle(cx, window);
        assert_eq!(focused(cx), [at], "after {key}");
    }
    assert_eq!(selected(cx, &frame).as_deref(), Some("one"), "moving the keyboard picks nothing");
    // Enter puts the tab's workspace on screen, and its terminal takes the keyboard back
    cx.update_window(window, |_, w, cx| w.press("enter", cx)).unwrap();
    settle(cx, window);
    assert_eq!(selected(cx, &frame).as_deref(), Some("three"));
    assert_eq!(focused(cx), Vec::<usize>::new());
    cx.update_window(window, |_, w, cx| w.press("x", cx)).unwrap();
    settle(cx, window);
    assert_eq!(terminal.sent(), b"x", "only the key typed after Enter reached the program");
}

/// With little room the tabs shorten, each down to what it must show; when even that doesn't fit the row scrolls,
/// fades at the edge it is cut at, and the tab of a workspace that comes on screen is brought into view.
#[gpui_kit::test]
fn with_little_room_the_tabs_shorten_then_the_row_scrolls(cx: &mut TestAppContext) {
    let long: Vec<String> = (1..=5).map(|n| format!("a-rather-long-workspace-name-{n}")).collect();
    let names: Vec<&str> = long.iter().map(String::as_str).collect();
    let mut start = frame(&names);
    start.changed.insert("w0".into(), files(5));
    start.projects[0].live_workspace_id = Some("w1".into());
    start.statuses.insert("w2".into(), "needs_you".into());
    let (window, _, frame) = open_sized(cx, start, (1700., 860.));
    settle(cx, window);
    let width = |w: &Window, ix: usize| w.find(("tab", ix)).bounds().size.width;
    let full: Vec<Pixels> = look(cx, window, |w| (0..5usize).map(|ix| width(w, ix)).collect());
    look(cx, window, |w| {
        assert!(w.try_find("tab-fade-left").is_none() && w.try_find("tab-fade-right").is_none(), "all fit");
        assert!(full.iter().all(|tab| *tab <= px(256.)), "a tab is never wider than 256 px: {full:?}");
    });

    // narrower: the names shorten, nothing scrolls yet, and what a tab says is still said. The tab on screen keeps
    // its name whole: the others give way
    cx.simulate_window_resize(window, size(px(1300.), px(860.)));
    settle(cx, window);
    look(cx, window, |w| {
        assert_eq!(says(w, "tab-live", 1).as_deref(), Some("Live"));
        assert_eq!(says(w, "tab-words", 2).as_deref(), Some("Needs you"));
        assert_eq!(width(w, 0), full[0], "the tab on screen keeps its width");
        for (ix, full) in full.iter().enumerate().skip(1) {
            let now = width(w, ix);
            assert!(now < *full && now >= px(112.), "tab {ix} went from {full:?} to {now:?}, not under 112 px");
        }
        assert!(w.try_find("tab-fade-left").is_none() && w.try_find("tab-fade-right").is_none(), "all fit");
        // what acts on the workspace keeps its room at the right
        assert_eq!(w.find("ws-menu").bounds().right(), px(1300. - 12.));
    });

    // twelve tabs can't fit even at their least: the row scrolls, and fades on the side that has more
    frame.update(cx, |f, cx| {
        for ix in 5..12 {
            f.apply(&Event::WorkspaceAdded(workspace(ix, &format!("more-{ix}"))));
        }
        cx.notify();
    });
    settle(cx, window);
    look(cx, window, |w| {
        assert_eq!(width(w, 11), px(112.), "shortened as far as a tab goes");
        assert!(width(w, 2) > px(112.), "a tab that needs you keeps room for its lamp, its name and its words");
        for ix in 0..12usize {
            let tab = w.find(("tab", ix)).bounds();
            if let Some(words) = w.try_find(("tab-words", ix)) {
                assert!(words.bounds().right() <= tab.right(), "tab {ix}'s words stay inside it");
            }
            if ix < 11 {
                let next = w.find(("tab", ix + 1)).bounds();
                assert!(tab.right() <= next.left(), "tab {ix} ends before the next begins: {tab:?} {next:?}");
            }
        }
        assert!(w.try_find("tab-fade-right").is_some(), "more tabs to the right");
        assert!(w.try_find("tab-fade-left").is_none());
    });
    let strip = look(cx, window, |w| w.find("tab-strip").bounds());
    cx.update_window(window, |_, w, cx| {
        w.scroll("workspace-tabs", ScrollDelta::Pixels(point(px(-4000.), px(0.))), cx);
    })
    .unwrap();
    settle(cx, window);
    look(cx, window, |w| {
        assert!(w.try_find("tab-fade-left").is_some(), "scrolled to the end: more tabs to the left");
        assert!(w.try_find("tab-fade-right").is_none());
        let last = w.find(("tab", 11usize)).bounds();
        assert!(last.right() <= strip.right() + px(1.), "the last tab is in view");
    });

    // ⌘2 picks the second tab, scrolled away to the left: it comes into view, with the pill under it
    look(cx, window, |w| assert!(w.find(("tab", 1usize)).bounds().left() < strip.left(), "out of view first"));
    cx.update_window(window, |_, w, cx| w.press("cmd-2", cx)).unwrap();
    settle(cx, window);
    assert_eq!(selected(cx, &frame).as_deref(), Some("a-rather-long-workspace-name-2"));
    // the pill glides there in 220 ms
    cx.executor().advance_clock(std::time::Duration::from_millis(300));
    settle(cx, window);
    look(cx, window, |w| {
        let second = w.find(("tab", 1usize)).bounds();
        assert!(
            second.left() >= strip.left() - px(1.),
            "the tab on screen is scrolled into view: {second:?} in {strip:?}"
        );
        let pill = w.find("workspace-tabs-pill").bounds();
        assert!((pill.left() - second.left()).abs() < px(1.), "the pill stands under it: {pill:?} {second:?}");
        // the rail keeps its width in a small window, and its foot stays in view
        assert_eq!(w.find("rail").bounds().size.width, px(56.));
        assert!(w.find("settings").bounds().bottom() <= px(860.));
    });
}

/// The tabs' names, in the order they stand in the top row.
fn names(cx: &mut TestAppContext, window: AnyWindowHandle) -> Vec<String> {
    settle(cx, window);
    look(cx, window, |w| {
        (0..12usize)
            .filter_map(|n| w.try_find(("tab", n)))
            .map(|tab| tab.label().unwrap_or_default().to_string())
            .collect()
    })
}

/// What a tab says of a drag: "held", "lands before", "lands after", or nothing.
fn drag_word(cx: &mut TestAppContext, window: AnyWindowHandle, id: impl Into<ElementId>) -> String {
    let id = id.into();
    look(cx, window, |w| w.find(id).description().unwrap_or_default().to_string())
}

/// The middle of a piece.
fn middle(cx: &mut TestAppContext, window: AnyWindowHandle, id: impl Into<ElementId>) -> gpui_kit::Point<Pixels> {
    let id = id.into();
    look(cx, window, |w| w.find(id).bounds().center())
}

/// The pointer goes down in the middle of one tab and moves to the middle of another in a few steps, as a hand
/// drags: the button is still down when this returns.
fn drag_over(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    from: impl Into<ElementId>,
    to: impl Into<ElementId>,
) -> gpui_kit::Point<Pixels> {
    let (start, end) = (middle(cx, window, from), middle(cx, window, to));
    cx.update_window(window, |_, w, cx| {
        let (modifiers, left) = (Modifiers::default(), MouseButton::Left);
        w.dispatch_event(MouseMoveEvent { position: start, pressed_button: None, modifiers }.to_platform_input(), cx);
        let down = MouseDownEvent { button: left, position: start, modifiers, click_count: 1, first_mouse: false };
        w.dispatch_event(down.to_platform_input(), cx);
        for step in 1..=5 {
            let part = step as f32 / 5.;
            let position = point(start.x + (end.x - start.x) * part, start.y + (end.y - start.y) * part);
            let moved = MouseMoveEvent { position, pressed_button: Some(left), modifiers };
            w.dispatch_event(moved.to_platform_input(), cx);
            w.render_frame(cx);
        }
    })
    .unwrap();
    settle(cx, window);
    end
}

/// The button comes up where the pointer is.
fn let_go(cx: &mut TestAppContext, window: AnyWindowHandle, at: gpui_kit::Point<Pixels>) {
    cx.update_window(window, |_, w, cx| {
        let up =
            MouseUpEvent { button: MouseButton::Left, position: at, modifiers: Modifiers::default(), click_count: 1 };
        w.dispatch_event(up.to_platform_input(), cx);
        w.render_frame(cx);
    })
    .unwrap();
    settle(cx, window);
}

/// Item 18.4: a tab is dragged onto the tab whose place it takes. While it is held its place in the row dims and the
/// tab under the pointer shows on which side it would land; let go there, the order is the frame's, and the keys
/// follow it.
#[gpui_kit::test]
fn a_tab_is_dragged_to_anothers_place(cx: &mut TestAppContext) {
    let (window, _, frame) = open(cx, frame(&["one", "two", "three", "four"]));
    assert_eq!(names(cx, window), ["one", "two", "three", "four"]);
    // along the row: held, and the tab under the pointer says it lands after it
    let over = drag_over(cx, window, ("tab", 0usize), ("tab", 2usize));
    assert_eq!(drag_word(cx, window, ("tab", 0usize)), "held");
    assert_eq!(drag_word(cx, window, ("tab", 2usize)), "lands after");
    assert_eq!(drag_word(cx, window, ("tab", 1usize)), "", "only the tab under the pointer says where");
    assert_eq!(names(cx, window), ["one", "two", "three", "four"], "nothing moved yet");
    // the pointer rests on that tab with the other one held: its tooltip stays away
    cx.executor().advance_clock(std::time::Duration::from_millis(900));
    settle(cx, window);
    look(cx, window, |w| assert!(w.try_find("tip").is_none(), "no tooltip over what is dragged"));
    let_go(cx, window, over);
    assert_eq!(names(cx, window), ["two", "three", "one", "four"]);
    assert!((0..4usize).all(|ix| drag_word(cx, window, ("tab", ix)).is_empty()), "nothing is held any more");
    // the workspace on screen is the one it was, in its new place
    assert_eq!(selected(cx, &frame).as_deref(), Some("one"));
    look(cx, window, |w| assert_eq!(w.find(("tab", 2usize)).selected(), Some(true)));
    // back along the row: it lands before the tab under the pointer
    let over = drag_over(cx, window, ("tab", 3usize), ("tab", 0usize));
    assert_eq!(drag_word(cx, window, ("tab", 3usize)), "held");
    assert_eq!(drag_word(cx, window, ("tab", 0usize)), "lands before");
    let_go(cx, window, over);
    assert_eq!(names(cx, window), ["four", "two", "three", "one"]);
    // let go under the row: nothing moves, and nothing stays held
    let over = drag_over(cx, window, ("tab", 1usize), ("tab", 2usize));
    let beside = point(over.x, over.y + px(300.));
    cx.update_window(window, |_, w, cx| {
        let moved = MouseMoveEvent {
            position: beside,
            pressed_button: Some(MouseButton::Left),
            modifiers: Modifiers::default(),
        };
        w.dispatch_event(moved.to_platform_input(), cx);
        w.render_frame(cx);
    })
    .unwrap();
    settle(cx, window);
    assert_eq!(drag_word(cx, window, ("tab", 2usize)), "", "off the row, it would land nowhere");
    let_go(cx, window, beside);
    assert_eq!(names(cx, window), ["four", "two", "three", "one"]);
    assert!((0..4usize).all(|ix| drag_word(cx, window, ("tab", ix)).is_empty()));
    // a click is still a click: it puts the tab's workspace on screen and moves nothing
    cx.update_window(window, |_, w, cx| w.click(("tab", 1usize), cx)).unwrap();
    assert_eq!(selected(cx, &frame).as_deref(), Some("two"));
    assert_eq!(names(cx, window), ["four", "two", "three", "one"]);
    // the keys follow your order
    cx.update_window(window, |_, w, cx| w.press("cmd-1", cx)).unwrap();
    assert_eq!(selected(cx, &frame).as_deref(), Some("four"));
    cx.update_window(window, |_, w, cx| w.press("cmd-}", cx)).unwrap();
    assert_eq!(selected(cx, &frame).as_deref(), Some("two"));
    // and it is what is kept of where you were
    let order = frame.read_with(cx, |frame, _| frame.view().order);
    assert_eq!(order["p1"], ["w3", "w1", "w2", "w0"]);
}

/// A press with the right button in the middle of a tab: its menu.
fn right_click(cx: &mut TestAppContext, window: AnyWindowHandle, ix: usize) {
    let at = middle(cx, window, ("tab", ix));
    cx.update_window(window, |_, w, cx| {
        let (modifiers, right) = (Modifiers::default(), MouseButton::Right);
        w.dispatch_event(MouseMoveEvent { position: at, pressed_button: None, modifiers }.to_platform_input(), cx);
        let down = MouseDownEvent { button: right, position: at, modifiers, click_count: 1, first_mouse: false };
        w.dispatch_event(down.to_platform_input(), cx);
        w.dispatch_event(
            MouseUpEvent { button: right, position: at, modifiers, click_count: 1 }.to_platform_input(),
            cx,
        );
        w.render_frame(cx);
    })
    .unwrap();
    settle(cx, window);
}

/// A click, and then the moment a menu takes to go (it fades for 110 ms where it stood, its items still drawn).
fn click(cx: &mut TestAppContext, window: AnyWindowHandle, id: impl Into<ElementId>) {
    let id = id.into();
    settle(cx, window);
    cx.update_window(window, |_, w, cx| w.click(id, cx)).unwrap();
    settle(cx, window);
    cx.executor().advance_clock(std::time::Duration::from_millis(300));
    settle(cx, window);
}

/// Item 18.5: Hide workspace in the workspace's menu, and on a right-click on any tab; the hidden ones behind their
/// button after the tabs, whose menu brings one back to its place and on screen; with every workspace hidden the
/// top row keeps that button and the +, and the screen says so.
#[gpui_kit::test]
fn a_workspace_is_hidden_from_its_menu_and_brought_back_from_the_button_after_the_tabs(cx: &mut TestAppContext) {
    let mut start = frame(&["one", "two", "three"]);
    start.statuses.insert("w1".into(), "needs_you".into());
    let (window, _, frame) = open(cx, start);
    assert_eq!(names(cx, window), ["one", "two", "three"]);
    look(cx, window, |w| assert!(w.try_find("hidden-workspaces").is_none(), "nothing about nothing"));
    // the workspace on screen, from its menu: Hide workspace stands above Remove workspace…
    click(cx, window, "ws-menu");
    look(cx, window, |w| {
        let (hide, remove) = (w.find("hide-workspace").bounds(), w.find("remove-workspace").bounds());
        assert_eq!(w.find("hide-workspace").label(), Some("Hide workspace"));
        assert!(hide.bottom() <= remove.top(), "{hide:?} above {remove:?}");
    });
    click(cx, window, "hide-workspace");
    assert_eq!(names(cx, window), ["two", "three"]);
    assert_eq!(selected(cx, &frame).as_deref(), Some("two"), "the tab that takes its place shows");
    // the button after the tabs: how many, and its menu lists them
    look(cx, window, |w| {
        let button = w.find("hidden-workspaces");
        assert_eq!((button.label(), button.value()), (Some("1 hidden workspace"), Some("1")));
        let (last, new) = (w.find(("tab", 1usize)).bounds(), w.find("new-workspace").bounds());
        assert!(last.right() <= button.bounds().left() && button.bounds().right() <= new.left(), "after the tabs");
    });
    // a right-click on a tab that is not on screen: the same menu, for that tab's workspace
    right_click(cx, window, 1);
    click(cx, window, "hide-workspace");
    assert_eq!(names(cx, window), ["two"]);
    assert_eq!(selected(cx, &frame).as_deref(), Some("two"), "what is on screen stays");
    look(cx, window, |w| assert_eq!(w.find("hidden-workspaces").label(), Some("2 hidden workspaces")));
    // hidden, it is quiet: the keys pass it by
    cx.update_window(window, |_, w, cx| w.press("cmd-}", cx)).unwrap();
    assert_eq!(selected(cx, &frame).as_deref(), Some("two"));
    // the menu: each with its name and since when; a click brings it back to its place, and on screen
    click(cx, window, "hidden-workspaces");
    look(cx, window, |w| {
        let rows: Vec<(String, String, String)> = (0..4usize)
            .filter_map(|n| w.try_find(("hidden-workspace", n)))
            .map(|row| {
                let said = |words: Option<&str>| words.unwrap_or_default().to_string();
                (said(row.label()), said(row.description()), said(row.value()))
            })
            .collect();
        let row = |name: &str, id: &str| (name.to_string(), "since today".to_string(), id.to_string());
        assert_eq!(rows, [row("one", "w0"), row("three", "w2")]);
    });
    click(cx, window, ("hidden-workspace", 1usize));
    assert_eq!(names(cx, window), ["two", "three"]);
    assert_eq!(selected(cx, &frame).as_deref(), Some("three"));
    look(cx, window, |w| assert_eq!(w.find("hidden-workspaces").label(), Some("1 hidden workspace")));
    // every workspace hidden: the top row keeps the button and the +, nothing acts on a workspace, and the screen
    // says so
    for _ in 0..2 {
        click(cx, window, "ws-menu");
        click(cx, window, "hide-workspace");
    }
    assert!(names(cx, window).is_empty());
    assert_eq!(selected(cx, &frame), None);
    look(cx, window, |w| {
        assert_eq!(w.find("hidden-workspaces").label(), Some("3 hidden workspaces"));
        assert!(w.try_find("new-workspace").is_some());
        assert!(w.try_find("ws-menu").is_none(), "no workspace is on screen: nothing acts on one");
        assert_eq!(w.find("empty-hidden").label(), Some("Every workspace in shop is hidden"));
        assert!(w.try_find("empty-workspaces").is_none());
    });
    // one comes back: first in the menu, it stands first among the tabs again
    click(cx, window, "hidden-workspaces");
    click(cx, window, ("hidden-workspace", 0usize));
    assert_eq!(names(cx, window), ["one"]);
    assert_eq!(selected(cx, &frame).as_deref(), Some("one"));
    look(cx, window, |w| assert!(w.try_find("empty-hidden").is_none() && w.try_find("ws-menu").is_some()));
    // what is hidden is kept with where you were
    let hidden = frame.read_with(cx, |frame, _| frame.view().hidden);
    let mut ids: Vec<&str> = hidden.keys().map(String::as_str).collect();
    ids.sort();
    assert_eq!(ids, ["w1", "w2"]);
}

/// ⌃ is let go: no modifier is held any more.
fn let_go_of_ctrl(cx: &mut TestAppContext, window: AnyWindowHandle) {
    cx.update_window(window, |_, w, cx| {
        let event = gpui_kit::ModifiersChangedEvent { modifiers: Modifiers::default(), ..Default::default() };
        w.dispatch_event(event.to_platform_input(), cx);
    })
    .unwrap();
    settle(cx, window);
}

/// The switcher's cards: each one's name, what it says of Claude, and whether the blue edge is on it.
fn cards(cx: &mut TestAppContext, window: AnyWindowHandle) -> Vec<(String, String, bool)> {
    settle(cx, window);
    look(cx, window, |w| {
        (0..12usize)
            .filter_map(|n| w.try_find(("switch-card", n)))
            .map(|card| {
                let said = |words: Option<&str>| words.unwrap_or_default().to_string();
                (said(card.label()), said(card.description()), card.selected() == Some(true))
            })
            .collect()
    })
}

/// The switcher (DESIGN.md, "Layout"): ⌃⇥ goes to the workspace used before this one. Held, it shows every
/// workspace as a card, the most lately used first; each ⌃⇥ moves the blue edge on, ⇧ back; letting go of ⌃ goes
/// there, Esc stays, a click on a card goes to it.
#[gpui_kit::test]
fn ctrl_tab_goes_by_last_use_and_held_shows_every_workspace_as_a_card(cx: &mut TestAppContext) {
    let (window, shell, frame) = open(cx, two_projects());
    settle(cx, window);
    let terminal = shell.read_with(cx, |s, cx| s.terminal().read(cx).terminal().clone());
    let press = |cx: &mut TestAppContext, key: &str| {
        cx.update_window(window, |_, w, cx| w.press(key, cx)).unwrap();
        settle(cx, window);
    };
    // stock-fix is on screen; then the other project's search (the rail, then ⌘2), then shop's export: export was
    // used last, search before it
    click(cx, window, ("project", 1usize));
    press(cx, "cmd-2");
    assert_eq!(selected(cx, &frame).as_deref(), Some("search"));
    frame.update(cx, |frame, cx| {
        frame.select_workspace("w1");
        cx.notify();
    });
    settle(cx, window);
    assert_eq!(selected(cx, &frame).as_deref(), Some("export"));
    assert!(cards(cx, window).is_empty(), "no switcher until ⌃⇥");

    // ⌃⇥, ⌃ still held: every workspace of every project as a card, by last use; the blue edge is on the one used
    // before, and nothing has moved yet
    press(cx, "ctrl-tab");
    let card = |name: &str, says: &str, edge: bool| (name.to_string(), says.to_string(), edge);
    assert_eq!(
        cards(cx, window),
        [
            card("export", "Needs you", false),
            card("search", "Needs you", true),
            card("polish", "Needs you", false),
            card("stock-fix", "Working", false)
        ]
    );
    assert_eq!(selected(cx, &frame).as_deref(), Some("export"));
    look(cx, window, |w| {
        // four in a row, 12 px apart, each 176 px high, inside a sheet that floats over the stage
        let (sheet, first, second) = (
            w.find("switcher").bounds(),
            w.find(("switch-card", 0usize)).bounds(),
            w.find(("switch-card", 1usize)).bounds(),
        );
        assert_eq!(
            (first.size.height, second.left() - first.right(), first.top(), second.top()),
            (px(176.), px(12.), sheet.top() + px(17.), sheet.top() + px(17.))
        );
        assert_eq!(sheet.top(), px(104.));
        assert_eq!(w.find(("switch-card", 0usize)).value(), Some("w1"));
    });
    // ⌃⇥ again: on; ⌃⇧⇥: back
    press(cx, "ctrl-tab");
    assert_eq!(cards(cx, window).iter().position(|card| card.2), Some(2));
    press(cx, "ctrl-shift-tab");
    assert_eq!(cards(cx, window).iter().position(|card| card.2), Some(1));
    // ⌃ is let go: search comes on screen, and the switcher is gone
    let_go_of_ctrl(cx, window);
    assert_eq!(selected(cx, &frame).as_deref(), Some("search"));
    assert!(cards(cx, window).is_empty());
    // a quick ⌃⇥: back to the one before
    press(cx, "ctrl-tab");
    let_go_of_ctrl(cx, window);
    assert_eq!(selected(cx, &frame).as_deref(), Some("export"));
    // ⌃⇧⇥ opens on the one used longest ago; Esc stays, and ⌃ let go afterwards changes nothing
    press(cx, "ctrl-shift-tab");
    assert_eq!(cards(cx, window).iter().position(|card| card.2), Some(3));
    press(cx, "escape");
    assert!(cards(cx, window).is_empty());
    let_go_of_ctrl(cx, window);
    assert_eq!(selected(cx, &frame).as_deref(), Some("export"));
    // a click on a card goes to it, whichever project it is in
    press(cx, "ctrl-tab");
    click(cx, window, ("switch-card", 3usize));
    assert_eq!(selected(cx, &frame).as_deref(), Some("stock-fix"));
    assert!(cards(cx, window).is_empty());
    assert!(terminal.sent().is_empty(), "none of it reaches the program: {:?}", terminal.sent());
}

/// With one workspace there is nothing to switch to: ⌃⇥ shows nothing.
#[gpui_kit::test]
fn with_one_workspace_ctrl_tab_shows_nothing(cx: &mut TestAppContext) {
    let (window, _, frame) = open(cx, frame(&["alone"]));
    settle(cx, window);
    cx.update_window(window, |_, w, cx| w.press("ctrl-tab", cx)).unwrap();
    assert!(cards(cx, window).is_empty());
    assert_eq!(selected(cx, &frame).as_deref(), Some("alone"));
}

/// With the lists hidden (⌘S) the rail is away: the other projects' workspaces stay in the top row as small dots after
/// the tabs, and a click on one goes there.
#[gpui_kit::test]
fn with_the_lists_hidden_the_other_projects_workspaces_are_dots_after_the_tabs(cx: &mut TestAppContext) {
    let (window, _, frame) = open(cx, two_projects());
    settle(cx, window);
    look(cx, window, |w| assert!(w.try_find("lamp-dots").is_none(), "the rail says it while it is there"));
    cx.update_window(window, |_, w, cx| w.press("cmd-s", cx)).unwrap();
    settle(cx, window);
    look(cx, window, |w| {
        assert!(w.try_find("rail").is_none());
        // shop's tabs stay as they are; enkidu's two workspaces are dots after them and after the +
        assert_eq!(w.find(("tab", 1usize)).label(), Some("export"));
        let (first, second) = (w.find(("lamp-dot", 0usize)), w.find(("lamp-dot", 1usize)));
        assert_eq!(
            (first.label(), first.description(), first.value()),
            (Some("polish"), Some("Needs you"), Some("enkidu"))
        );
        assert_eq!(second.label(), Some("search"));
        assert!(w.try_find(("lamp-dot", 2usize)).is_none(), "the project on screen has its tabs");
        let (new, dot) = (w.find("new-workspace").bounds(), first.bounds());
        assert!(dot.left() > new.right() && dot.size.width == px(8.), "{dot:?} after {new:?}");
        assert!(second.bounds().right() < w.find("workspace-layout").bounds().left());
    });
    // a click on a dot goes to that workspace, with its project
    cx.update_window(window, |_, w, cx| w.click(("lamp-dot", 1usize), cx)).unwrap();
    settle(cx, window);
    assert_eq!(frame.read_with(cx, |f, _| f.current_project.clone()).as_deref(), Some("p2"));
    assert_eq!(selected(cx, &frame).as_deref(), Some("search"));
    look(cx, window, |w| {
        // now enkidu's are the tabs, and shop's are the dots
        assert_eq!(w.find(("tab", 1usize)).label(), Some("search"));
        assert_eq!(w.find(("lamp-dot", 0usize)).label(), Some("stock-fix"));
        assert_eq!(w.find(("lamp-dot", 1usize)).description(), Some("Needs you"));
    });
}
