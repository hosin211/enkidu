//! Item 1.17: the workspace tabs. Each tab says what its workspace waits for ("Needs you", Live, the number of changed
//! files); with little room the tabs drop their extra words, then shrink, then scroll with a faded edge, and the tab
//! on screen is kept in view; ⌘1 to 9, ⌃⇥ and ⌃⇧⇥, ⇧⌘] and ⇧⌘[ pick and cycle them; with no workspaces there is no row.

use enkidu_app::shell::Shell;
use enkidu_core::db::{Project, Workspace};
use enkidu_core::events::Event;
use enkidu_model::frame::{Changes, Frame, View};
use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, Entity, InputEvent as _, Modifiers, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, ScrollDelta, TestAppContext, Window, WindowBounds, WindowOptions, point, px,
    size,
};

fn files(count: usize) -> Changes {
    Changes { files: count, ..Default::default() }
}

fn workspace(ix: usize, name: &str) -> Workspace {
    Workspace {
        id: format!("w{ix}"),
        project_id: "p1".into(),
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

/// The project "shop" with a workspace for each name, the first on screen.
fn frame(names: &[&str]) -> Frame {
    let project = Project {
        id: "p1".into(),
        name: "shop".into(),
        path: "/code/shop".into(),
        live_workspace_id: None,
        created_at: 1,
    };
    let workspaces = names.iter().enumerate().map(|(ix, name)| workspace(ix, name)).collect();
    Frame::load(vec![project], workspaces, Default::default(), &View::default())
}

fn open(cx: &mut TestAppContext, frame: Frame, width: f32) -> (AnyWindowHandle, Entity<Shell>, Entity<Frame>) {
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

/// Draws a few frames, so what is measured in one (the room, the tabs' places) is used in the next.
fn settle(cx: &mut TestAppContext, window: AnyWindowHandle) {
    for _ in 0..3 {
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| window.render_frame(cx)).unwrap();
    }
}

fn look<R>(cx: &mut TestAppContext, window: AnyWindowHandle, at: impl FnOnce(&mut Window) -> R) -> R {
    cx.update_window(window, |_, window, _| at(window)).unwrap()
}

fn words(window: &Window, ix: usize) -> Option<String> {
    window.try_find(("tab-words", ix)).and_then(|w| w.label().map(String::from))
}

fn width(window: &Window, id: impl Into<gpui_kit::ElementId>) -> Pixels {
    window.find(id).bounds().size.width
}

fn compact(cx: &mut TestAppContext, shell: &Entity<Shell>) -> bool {
    shell.read_with(cx, |shell, cx| shell.tabs().read(cx).is_compact(cx))
}

fn selected(cx: &mut TestAppContext, frame: &Entity<Frame>) -> Option<String> {
    frame.read_with(cx, |f, _| f.current_workspace().map(|w| w.name.clone()))
}

#[gpui_kit::test]
fn each_tab_says_what_its_workspace_waits_for(cx: &mut TestAppContext) {
    let mut start = frame(&["stock-fix", "sh-21309", "pr-2175", "docs"]);
    start.changed.extend([("w0".to_string(), files(5)), ("w1".to_string(), files(3)), ("w2".to_string(), files(2))]);
    start.statuses.insert("w0".into(), "working".into());
    start.statuses.insert("w1".into(), "needs_you".into());
    start.projects[0].live_workspace_id = Some("w2".into());
    let (window, _, frame) = open(cx, start, 1360.);
    settle(cx, window);
    look(cx, window, |w| {
        assert_eq!(words(w, 0).as_deref(), Some("5 changed files"));
        assert_eq!(words(w, 1).as_deref(), Some("Needs you"), "needing you comes before its changes");
        assert_eq!(words(w, 2).as_deref(), Some("Live"), "Live comes before its changes");
        assert_eq!(words(w, 3), None, "nothing about nothing");
        assert_eq!(w.find(("tab", 0usize)).label(), Some("stock-fix"));
        assert_eq!(w.find(("tab", 0usize)).selected(), Some(true));
        assert_eq!(w.find("tab-strip").label(), Some("Workspaces in shop"));
        assert!(w.try_find_by_label("New workspace in shop").is_some(), "New workspace names its project");
    });

    // the core's news changes the words
    frame.update(cx, |f, cx| {
        f.apply(&Event::ChangesUpdated { workspace_id: "w3".into(), files: vec![change("README.md")] });
        f.apply(&Event::StatusChanged { workspace_id: "w1".into(), status: "working".into() });
        cx.notify();
    });
    settle(cx, window);
    look(cx, window, |w| {
        assert_eq!(words(w, 3).as_deref(), Some("1 changed file"));
        assert_eq!(words(w, 1).as_deref(), Some("3 changed files"), "answered: its count shows again");
    });
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
fn with_little_room_tabs_drop_their_words_then_shrink_then_scroll(cx: &mut TestAppContext) {
    let long: Vec<String> = (1..=5).map(|n| format!("a-rather-long-workspace-name-{n}")).collect();
    let names: Vec<&str> = long.iter().map(String::as_str).collect();
    let mut start = frame(&names);
    start.changed.insert("w0".into(), files(5));
    start.projects[0].live_workspace_id = Some("w1".into());
    start.statuses.insert("w2".into(), "needs_you".into());
    // (1400 px: the Live button is 100 px wide, 8 px more than the pill that stood in for it, and these five names had
    // their room in 1360 px by less than that)
    let (window, shell, frame) = open(cx, start, 1400.);
    settle(cx, window);
    assert!(!compact(cx, &shell), "five tabs have their room in 1400 px");
    let (full_live, full_count, full_tabs) = look(cx, window, |w| {
        let tabs: Vec<Pixels> = (0..5usize).map(|ix| width(w, ("tab", ix))).collect();
        (width(w, ("tab-words", 1usize)), width(w, ("tab-words", 0usize)), tabs)
    });

    // narrower: the words go first (Live keeps its bars, the count its number), then the tabs shrink, no scrolling
    cx.simulate_window_resize(window, size(px(900.), px(860.)));
    settle(cx, window);
    assert!(compact(cx, &shell), "in 900 px they are compact");
    look(cx, window, |w| {
        assert_eq!(words(w, 1).as_deref(), Some("Live"), "still said, for whoever can't see color");
        assert!(width(w, ("tab-words", 1usize)) < full_live, "Live keeps only its bars");
        assert!(width(w, ("tab-words", 0usize)) < full_count, "the count keeps only its number");
        for (ix, full) in full_tabs.iter().enumerate() {
            let now = width(w, ("tab", ix));
            assert!(now < *full && now >= px(112.), "tab {ix} shrank from {full:?} to {now:?}, not under 112 px");
        }
        assert!(w.try_find("tab-fade-left").is_none() && w.try_find("tab-fade-right").is_none(), "all fit");
    });

    // nine tabs can't fit even at 112 px: the strip scrolls, and fades on the side that has more
    frame.update(cx, |f, cx| {
        for ix in 5..9 {
            f.apply(&Event::WorkspaceAdded(workspace(ix, &format!("more-{ix}"))));
        }
        cx.notify();
    });
    settle(cx, window);
    look(cx, window, |w| {
        assert_eq!(width(w, ("tab", 8usize)), px(112.), "shrunk as far as a tab goes");
        assert!(width(w, ("tab", 2usize)) > px(112.), "a tab that needs you keeps room for its lamp, name and words");
        for ix in 0..9usize {
            let tab = w.find(("tab", ix)).bounds();
            if let Some(words) = w.try_find(("tab-words", ix)) {
                assert!(words.bounds().right() <= tab.right(), "tab {ix}'s words stay inside it");
            }
            if ix < 8 {
                let next = w.find(("tab", ix + 1)).bounds();
                assert!(tab.right() <= next.left(), "tab {ix} ends before the next begins: {tab:?} {next:?}");
            }
        }
        assert!(w.try_find("tab-fade-right").is_some(), "more tabs to the right");
        assert!(w.try_find("tab-fade-left").is_none());
    });
    let strip = look(cx, window, |w| w.find("workspace-tabs").bounds());
    cx.update_window(window, |_, w, cx| {
        w.scroll("workspace-tabs", ScrollDelta::Pixels(point(px(-2000.), px(0.))), cx);
    })
    .unwrap();
    settle(cx, window);
    look(cx, window, |w| {
        assert!(w.try_find("tab-fade-left").is_some(), "scrolled to the end: more tabs to the left");
        assert!(w.try_find("tab-fade-right").is_none());
        let last = w.find(("tab", 8usize)).bounds();
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
        assert!((pill.left() - second.left()).abs() < px(1.), "the pill sits under it: {pill:?} {second:?}");
    });
}

#[gpui_kit::test]
fn the_keys_pick_and_cycle_the_tabs(cx: &mut TestAppContext) {
    let (window, shell, frame) = open(cx, frame(&["one", "two", "three"]), 1360.);
    settle(cx, window);
    let terminal = shell.read_with(cx, |s, cx| s.terminal().read(cx).terminal().clone());
    let press = |cx: &mut TestAppContext, key: &str| {
        cx.update_window(window, |_, w, cx| w.press(key, cx)).unwrap();
        settle(cx, window);
        selected(cx, &frame).unwrap()
    };
    assert_eq!(press(cx, "cmd-2"), "two");
    assert_eq!(press(cx, "cmd-9"), "two", "there is no ninth tab");
    assert_eq!(press(cx, "ctrl-tab"), "three");
    assert_eq!(press(cx, "ctrl-tab"), "one", "around from the last");
    assert_eq!(press(cx, "ctrl-shift-tab"), "three", "and back");
    assert_eq!(press(cx, "cmd-}"), "one", "⇧⌘], as the Mac reports it");
    assert_eq!(press(cx, "cmd-{"), "three", "⇧⌘[");
    assert_eq!(press(cx, "cmd-1"), "one");
    assert!(terminal.sent().is_empty(), "the keys never reach the program: {:?}", terminal.sent());
}

#[gpui_kit::test]
fn a_project_without_workspaces_has_no_tabs_row(cx: &mut TestAppContext) {
    let (window, _, _) = open(cx, frame(&[]), 1360.);
    settle(cx, window);
    look(cx, window, |w| {
        assert!(w.try_find("tab-strip").is_none());
        assert!(w.try_find("new-workspace").is_none(), "the empty screen has that button (1.20)");
    });
}

#[gpui_kit::test]
fn a_click_on_the_frame_leaves_the_keyboard_with_the_terminal(cx: &mut TestAppContext) {
    let (window, shell, frame) = open(cx, frame(&["one", "two", "three"]), 1360.);
    settle(cx, window);
    let terminal = shell.read_with(cx, |s, cx| s.terminal().read(cx).terminal().clone());
    // another tab, the status line, the projects row, the empty middle of the tabs row (the tab already on screen is
    // the one click that takes the keyboard: see the arrows test)
    for (target, key) in [("tab", "a"), ("status-line", "b"), ("projects-row", "c"), ("tabs-row", "d")] {
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
    let (window, shell, frame) = open(cx, frame(&["one", "two", "three"]), 1360.);
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
    // Enter puts the tab on screen, and its terminal takes the keyboard back
    cx.update_window(window, |_, w, cx| w.press("enter", cx)).unwrap();
    settle(cx, window);
    assert_eq!(selected(cx, &frame).as_deref(), Some("three"));
    assert_eq!(focused(cx), Vec::<usize>::new());
    cx.update_window(window, |_, w, cx| w.press("x", cx)).unwrap();
    settle(cx, window);
    assert_eq!(terminal.sent(), b"x", "only the key typed after Enter reached the program");
}

/// The tabs' names, in the order they stand in the row.
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
fn drag_word(cx: &mut TestAppContext, window: AnyWindowHandle, ix: usize) -> String {
    look(cx, window, |w| w.find(("tab", ix)).description().unwrap_or_default().to_string())
}

/// The middle of a tab.
fn middle(cx: &mut TestAppContext, window: AnyWindowHandle, ix: usize) -> gpui_kit::Point<Pixels> {
    look(cx, window, |w| w.find(("tab", ix)).bounds().center())
}

/// The pointer goes down in the middle of one tab and moves to the middle of another in a few steps, as a hand
/// drags: the button is still down when this returns.
fn drag_tab_over(cx: &mut TestAppContext, window: AnyWindowHandle, from: usize, to: usize) -> gpui_kit::Point<Pixels> {
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

/// Item 18.4: a tab is
/// dragged onto the tab whose place it takes. While it is held its place in the row dims and the tab under the
/// pointer shows on which side it would land; let go there, the order is the frame's, and the keys follow it.
#[gpui_kit::test]
fn a_tab_is_dragged_to_anothers_place(cx: &mut TestAppContext) {
    let (window, _, frame) = open(cx, frame(&["one", "two", "three", "four"]), 1360.);
    assert_eq!(names(cx, window), ["one", "two", "three", "four"]);
    // to the right: held, and the tab under the pointer says it lands after it
    let over = drag_tab_over(cx, window, 0, 2);
    assert_eq!(drag_word(cx, window, 0), "held");
    assert_eq!(drag_word(cx, window, 2), "lands after");
    assert_eq!(drag_word(cx, window, 1), "", "only the tab under the pointer says where");
    assert_eq!(names(cx, window), ["one", "two", "three", "four"], "nothing moved yet");
    // the pointer rests on that tab with the other one held: its tooltip stays away
    cx.executor().advance_clock(std::time::Duration::from_millis(900));
    settle(cx, window);
    look(cx, window, |w| assert!(w.try_find("tip").is_none(), "no tooltip over what is dragged"));
    let_go(cx, window, over);
    assert_eq!(names(cx, window), ["two", "three", "one", "four"]);
    assert!((0..4).all(|ix| drag_word(cx, window, ix).is_empty()), "nothing is held any more");
    // the workspace on screen is the one it was, in its new place
    assert_eq!(selected(cx, &frame).as_deref(), Some("one"));
    look(cx, window, |w| assert_eq!(w.find(("tab", 2usize)).selected(), Some(true)));
    // to the left: it lands before the tab under the pointer
    let over = drag_tab_over(cx, window, 3, 0);
    assert_eq!((drag_word(cx, window, 3), drag_word(cx, window, 0)), ("held".to_string(), "lands before".to_string()));
    let_go(cx, window, over);
    assert_eq!(names(cx, window), ["four", "two", "three", "one"]);
    // let go beside the tabs: nothing moves, and nothing stays held
    let over = drag_tab_over(cx, window, 1, 2);
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
    assert_eq!(drag_word(cx, window, 2), "", "off the row, it would land nowhere");
    let_go(cx, window, beside);
    assert_eq!(names(cx, window), ["four", "two", "three", "one"]);
    assert!((0..4).all(|ix| drag_word(cx, window, ix).is_empty()));
    // a click is still a click: it puts the tab on screen and moves nothing
    cx.update_window(window, |_, w, cx| w.click(("tab", 1usize), cx)).unwrap();
    assert_eq!(selected(cx, &frame).as_deref(), Some("two"));
    assert_eq!(names(cx, window), ["four", "two", "three", "one"]);
    // the keys follow your order
    cx.update_window(window, |_, w, cx| w.press("cmd-1", cx)).unwrap();
    assert_eq!(selected(cx, &frame).as_deref(), Some("four"));
    cx.update_window(window, |_, w, cx| w.press("ctrl-tab", cx)).unwrap();
    assert_eq!(selected(cx, &frame).as_deref(), Some("two"));
    // and it is what is kept of where you were
    let order = frame.read_with(cx, |frame, _| frame.view().order);
    assert_eq!(order["p1"], ["w3", "w1", "w2", "w0"]);
}

/// A press with the right button in the middle of a tab: its menu.
fn right_click(cx: &mut TestAppContext, window: AnyWindowHandle, ix: usize) {
    let at = middle(cx, window, ix);
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
fn click(cx: &mut TestAppContext, window: AnyWindowHandle, id: impl Into<gpui_kit::ElementId>) {
    let id = id.into();
    settle(cx, window);
    cx.update_window(window, |_, w, cx| w.click(id, cx)).unwrap();
    settle(cx, window);
    cx.executor().advance_clock(std::time::Duration::from_millis(300));
    settle(cx, window);
}

/// Item 18.5: Hide workspace in the workspace's menu, and on a
/// right-click on any tab; the hidden ones behind their button after the tabs, whose menu brings one back to its
/// place and on screen; with every workspace hidden the row keeps that button and the screen says so.
#[gpui_kit::test]
fn a_workspace_is_hidden_from_its_menu_and_brought_back_from_the_button_after_the_tabs(cx: &mut TestAppContext) {
    let mut start = frame(&["one", "two", "three"]);
    start.statuses.insert("w1".into(), "needs_you".into());
    let (window, _, frame) = open(cx, start, 1360.);
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
        let (last, add) = (w.find(("tab", 1usize)).bounds(), w.find("new-workspace").bounds());
        assert!(last.right() <= button.bounds().left() && button.bounds().right() <= add.left(), "after the tabs");
    });
    // a right-click on a tab that is not on screen: the same menu, for that tab's workspace
    right_click(cx, window, 1);
    click(cx, window, "hide-workspace");
    assert_eq!(names(cx, window), ["two"]);
    assert_eq!(selected(cx, &frame).as_deref(), Some("two"), "what is on screen stays");
    look(cx, window, |w| assert_eq!(w.find("hidden-workspaces").label(), Some("2 hidden workspaces")));
    // hidden, it is quiet: the keys pass it by
    cx.update_window(window, |_, w, cx| w.press("ctrl-tab", cx)).unwrap();
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
    // every workspace hidden: the row keeps the button and New workspace, nothing that acts on a workspace, and
    // the screen says so
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
