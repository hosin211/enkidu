//! Item 3.8: the three layouts and their keys, in a real window on real git. The changes above the terminal, beside
//! it, or away (the switch in the tabs row, ⌥⌘L, ⌥⌘B); the changes alone (⇧⌘E, the button at the end of their band).
//! The space between the two islands drags to resize, is remembered across a restart, and goes back to its first
//! size on a double click. With nothing to review the terminal has the whole workspace, whatever was chosen.

mod common;

use std::path::PathBuf;
use std::time::Duration;

use common::{App, click_line, line_number, said, until_kept};
use enkidu_model::ui::{Layout, Mode, Ui};
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

/// The sample project with one workspace where `src/app.py` was rewritten: there is something to review.
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

fn look<R>(cx: &mut TestAppContext, app: &App, at: impl FnOnce(&mut Window) -> R) -> R {
    cx.run_until_parked();
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        at(w)
    })
    .unwrap()
}

/// Where the two islands are, when they are on screen: the changes, then Claude's terminal.
fn islands(window: &Window) -> (Option<Bounds<Pixels>>, Option<Bounds<Pixels>>) {
    (
        window.try_find("island-changes").map(|island| island.bounds()),
        window.try_find("island-claude").map(|island| island.bounds()),
    )
}

/// Which of the switch's three is chosen: its label.
fn chosen(window: &Window) -> Option<String> {
    ["layout-side", "layout-stack", "layout-terminal"]
        .iter()
        .filter_map(|id| window.try_find(*id))
        .find(|segment| segment.selected() == Some(true))
        .and_then(|segment| segment.label().map(String::from))
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

/// A double click, as the mouse sends it: two presses, the second counted as the second.
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

#[gpui_kit::test]
fn the_switch_and_the_keys_put_the_changes_above_beside_or_away(cx: &mut TestAppContext) {
    let (app, _, _) = start(cx);
    // as the app starts: the changes above the terminal, 8 px apart, the terminal 320 px high
    look(cx, &app, |w| {
        let (Some(changes), Some(terminal)) = islands(w) else { panic!("both islands are on screen") };
        assert_eq!(terminal.top() - changes.bottom(), px(8.), "8 px between the islands");
        assert_eq!((changes.left(), changes.right()), (terminal.left(), terminal.right()));
        assert_eq!(terminal.size.height, px(320.));
        assert_eq!(chosen(w).as_deref(), Some("Changes above the terminal"));
        assert_eq!(said(w, "layout-divider").as_deref(), Some("Resize terminal"));
    });

    // the switch: beside the terminal, which is on the left; the changes 540 px wide
    app.click(cx, "layout-side");
    assert!(app.until(cx, |w| chosen(w).as_deref() == Some("Changes beside the terminal")));
    look(cx, &app, |w| {
        let (Some(changes), Some(terminal)) = islands(w) else { panic!("both islands are on screen") };
        assert_eq!(changes.left() - terminal.right(), px(8.), "8 px between the islands");
        assert_eq!((changes.top(), changes.bottom()), (terminal.top(), terminal.bottom()));
        assert_eq!(changes.size.width, px(540.));
        assert_eq!(said(w, "layout-divider").as_deref(), Some("Resize changes"));
    });
    assert_eq!(ui(cx, &app).layout, Layout::Side);
    assert!(kept_layout(cx, &app, r#""layout":"side""#).contains(r#""layout":"side""#), "not kept");

    // ⌥⌘L: back above, and beside again
    app.press(cx, "alt-cmd-l");
    assert!(app.until(cx, |w| chosen(w).as_deref() == Some("Changes above the terminal")), "⌥⌘L");
    app.press(cx, "alt-cmd-l");
    assert!(app.until(cx, |w| chosen(w).as_deref() == Some("Changes beside the terminal")), "⌥⌘L again");

    // ⌥⌘B: the terminal alone, with the whole workspace; again, the changes are back beside it
    let room = look(cx, &app, |w| w.find("workspace-body").bounds());
    app.press(cx, "alt-cmd-b");
    assert!(app.until(cx, |w| chosen(w).as_deref() == Some("Terminal only")), "⌥⌘B");
    look(cx, &app, |w| {
        let (changes, Some(terminal)) = islands(w) else { panic!("the terminal is on screen") };
        assert!(changes.is_none(), "the changes are put away");
        assert_eq!(terminal, room, "the terminal has the whole workspace");
        assert!(w.try_find("layout-divider").is_none());
    });
    app.press(cx, "alt-cmd-b");
    assert!(app.until(cx, |w| chosen(w).as_deref() == Some("Changes beside the terminal")), "⌥⌘B again");

    // the switch's third choice does the same, and its second brings the changes back above
    app.click(cx, "layout-terminal");
    assert!(app.until(cx, |w| w.try_find("island-changes").is_none()));
    app.click(cx, "layout-stack");
    assert!(app.until(cx, |w| chosen(w).as_deref() == Some("Changes above the terminal")));
    assert_eq!(ui(cx, &app).chosen(), Mode::Stack);
}

#[gpui_kit::test]
fn the_changes_take_the_whole_workspace_and_give_it_back(cx: &mut TestAppContext) {
    let (app, _, _) = start(cx);
    let room = look(cx, &app, |w| w.find("workspace-body").bounds());
    // the keyboard is in the terminal, as it is while you work with Claude
    app.click(cx, "terminal");
    assert!(app.until(cx, |w| w.find("terminal").focused() == Some(true)));
    assert_eq!(look(cx, &app, |w| said(w, "maximize-review")).as_deref(), Some("Review in full width"));

    // ⇧⌘E: the changes alone; none of the switch's three is chosen; the keyboard is in the changes
    app.press(cx, "cmd-shift-e");
    assert!(app.until(cx, |w| w.try_find("island-claude").is_none()), "⇧⌘E");
    look(cx, &app, |w| {
        assert_eq!(islands(w).0, Some(room), "the changes have the whole workspace");
        assert_eq!(chosen(w), None);
        assert_eq!(said(w, "maximize-review").as_deref(), Some("Back to the terminal"));
    });
    let in_changes = |cx: &mut TestAppContext| {
        let island = app.shell.read_with(cx, |shell, _| shell.changes().cloned()).expect("the island");
        cx.update_window(app.window, |_, w, cx| island.read(cx).has_keyboard(w, cx)).unwrap()
    };
    assert!(in_changes(cx), "the keyboard stayed in a terminal that isn't on screen");

    // ⇧⌘E again: both islands as before
    app.press(cx, "cmd-shift-e");
    assert!(app.until(cx, |w| w.try_find("island-claude").is_some() && w.try_find("island-changes").is_some()));
    assert_eq!(look(cx, &app, |w| chosen(w)).as_deref(), Some("Changes above the terminal"));

    // the button at the end of the band does the same
    app.click(cx, "maximize-review");
    assert!(app.until(cx, |w| w.try_find("island-claude").is_none()), "the button");
    // from there ⌥⌘B is the terminal alone, with the keyboard
    app.press(cx, "alt-cmd-b");
    assert!(app.until(cx, |w| w.try_find("island-changes").is_none() && w.try_find("island-claude").is_some()));
    assert!(app.until(cx, |w| w.find("terminal").focused() == Some(true)), "the keyboard isn't in the terminal");
    // and ⇧⌘E from there is the changes alone again
    app.press(cx, "cmd-shift-e");
    assert!(app.until(cx, |w| w.try_find("island-claude").is_none() && w.try_find("island-changes").is_some()));
    app.click(cx, "maximize-review");
    assert!(app.until(cx, |w| w.try_find("island-claude").is_some() && w.try_find("island-changes").is_some()));
}

#[gpui_kit::test]
fn the_space_between_the_islands_drags_is_remembered_and_resets_on_a_double_click(cx: &mut TestAppContext) {
    let (app, _, _) = start(cx);
    let (room, gap) = look(cx, &app, |w| (w.find("workspace-body").bounds(), w.find("layout-divider").bounds()));
    assert_eq!(gap.size.height, px(8.));
    let at = gap.center();
    let up = |by: f32| point(at.x, at.y - px(by));

    // above the terminal: 100 px up, and the terminal is 100 px higher
    drag(cx, &app, at, &[up(30.), up(60.), up(100.)]);
    assert_eq!(ui(cx, &app).terminal_height, 420);
    assert_eq!(look(cx, &app, |w| islands(w).1.map(|terminal| terminal.size.height)), Some(px(420.)));
    assert!(kept_layout(cx, &app, r#""terminalHeight":420"#).contains(r#""terminalHeight":420"#), "not kept");

    // all the way up: the changes keep 120 px; all the way down: the terminal keeps 140
    let gap = look(cx, &app, |w| w.find("layout-divider").bounds().center());
    drag(cx, &app, gap, &[point(gap.x, gap.y - px(40.)), point(gap.x, room.top() - px(50.))]);
    let most = f32::from(room.size.height) as u32 - 8 - 120;
    assert_eq!(ui(cx, &app).terminal_height, most);
    assert_eq!(look(cx, &app, |w| islands(w).0.map(|changes| changes.size.height)), Some(px(120.)));
    let gap = look(cx, &app, |w| w.find("layout-divider").bounds().center());
    drag(cx, &app, gap, &[point(gap.x, gap.y + px(40.)), point(gap.x, room.bottom() + px(50.))]);
    assert_eq!(ui(cx, &app).terminal_height, 140);

    // a double click: back to 320
    let gap = look(cx, &app, |w| w.find("layout-divider").bounds().center());
    double_click(cx, &app, gap);
    assert_eq!(ui(cx, &app).terminal_height, 320);

    // beside the terminal: 60 px to the left, and the changes are 60 px wider
    app.press(cx, "alt-cmd-l");
    assert!(app.until(cx, |w| said(w, "layout-divider").as_deref() == Some("Resize changes")));
    let gap = look(cx, &app, |w| w.find("layout-divider").bounds());
    assert_eq!(gap.size.width, px(8.));
    let at = gap.center();
    drag(cx, &app, at, &[point(at.x - px(20.), at.y), point(at.x - px(60.), at.y)]);
    assert_eq!(ui(cx, &app).review_width, 600);
    assert_eq!(look(cx, &app, |w| islands(w).0.map(|changes| changes.size.width)), Some(px(600.)));
    // far to the right: the changes keep 340 px
    let gap = look(cx, &app, |w| w.find("layout-divider").bounds().center());
    drag(cx, &app, gap, &[point(gap.x + px(40.), gap.y), point(room.right() + px(80.), gap.y)]);
    assert_eq!(ui(cx, &app).review_width, 340);
    // far to the left: the terminal keeps 360
    let gap = look(cx, &app, |w| w.find("layout-divider").bounds().center());
    drag(cx, &app, gap, &[point(gap.x - px(40.), gap.y), point(room.left() - px(80.), gap.y)]);
    assert_eq!(ui(cx, &app).review_width, f32::from(room.size.width) as u32 - 8 - 360);
    assert_eq!(look(cx, &app, |w| islands(w).1.map(|terminal| terminal.size.width)), Some(px(360.)));

    // 500 px, and a restart: the window opens laid out as it was left
    let gap = look(cx, &app, |w| w.find("layout-divider").bounds().center());
    let want = room.right() - px(504.);
    drag(cx, &app, gap, &[point(gap.x + px(30.), gap.y), point(want, gap.y)]);
    assert_eq!(ui(cx, &app).review_width, 500);
    let kept = kept_layout(cx, &app, r#""reviewWidth":500"#);
    assert!(kept.contains(r#""layout":"side""#) && kept.contains(r#""reviewWidth":500"#), "not kept: {kept}");
    let app = common::restart(cx, app);
    assert!(app.until(cx, |w| line_number(w, "Line 5, after").is_some()), "the diff never showed again");
    let again = ui(cx, &app);
    assert_eq!((again.layout, again.review_width, again.terminal_height), (Layout::Side, 500, 320));
    look(cx, &app, |w| {
        let (Some(changes), Some(terminal)) = islands(w) else { panic!("both islands are on screen") };
        assert_eq!(changes.size.width, px(500.));
        assert!(terminal.right() < changes.left(), "the terminal is on the left");
    });
    // a double click there: back to 540
    let gap = look(cx, &app, |w| w.find("layout-divider").bounds().center());
    double_click(cx, &app, gap);
    assert_eq!(ui(cx, &app).review_width, 540);
}

#[gpui_kit::test]
fn with_nothing_to_review_the_terminal_has_the_whole_workspace_whatever_was_chosen(cx: &mut TestAppContext) {
    let app = common::start_on(cx, enkidu_testkit::shop, make);
    let (_, _, folder) = app.on_screen(cx).expect("on screen");
    let folder = PathBuf::from(folder);
    assert!(app.until(cx, |w| w.try_find("island-claude").is_some()));
    let whole = look(cx, &app, |w| w.find("island-claude").bounds());
    // beside is chosen, and then the changes alone: with nothing to review the terminal still has it all
    app.click(cx, "layout-side");
    assert!(app.until(cx, |w| chosen(w).as_deref() == Some("Changes beside the terminal")));
    assert_eq!(look(cx, &app, |w| islands(w)), (None, Some(whole)));
    app.press(cx, "cmd-shift-e");
    cx.run_until_parked();
    assert_eq!(look(cx, &app, |w| islands(w)), (None, Some(whole)), "an empty island took the workspace");
    app.press(cx, "cmd-shift-e");

    // the first edit brings the changes where the layout puts them: beside the terminal
    std::fs::write(folder.join("src/app.py"), CODE).unwrap();
    assert!(app.until(cx, |w| w.try_find("island-changes").is_some()), "the changes never came");
    look(cx, &app, |w| {
        let (Some(changes), Some(terminal)) = islands(w) else { panic!("both islands are on screen") };
        assert!(terminal.right() < changes.left(), "the changes are beside the terminal");
    });
    // it is undone: the terminal has the workspace again
    enkidu_testkit::git(&folder, &["checkout", "-q", "--", "src/app.py"]);
    assert!(app.until(cx, |w| islands(w) == (None, Some(whole))), "the changes stayed with nothing in them");
}

/// Sending remarks while the changes have the whole workspace brings Claude's terminal back: Enter is pressed there.
#[gpui_kit::test]
fn remarks_typed_into_claude_bring_its_terminal_back(cx: &mut TestAppContext) {
    let app = common::start_live(cx, enkidu_testkit::shop, make);
    app.until_claude_started(cx, 1);
    assert!(app.until_terminal(cx, "\n>"), "Claude's prompt");
    let (_, _, folder) = app.on_screen(cx).expect("on screen");
    std::fs::write(PathBuf::from(folder).join("src/app.py"), CODE).unwrap();
    let ws = app.frame.read_with(cx, |f, _| f.current_workspace().unwrap().id.clone());
    assert!(app.until(cx, |w| line_number(w, "Line 5, after").is_some()), "the diff never showed");
    app.press(cx, "cmd-shift-e");
    assert!(app.until(cx, |w| w.try_find("island-claude").is_none()), "⇧⌘E");
    click_line(cx, &app, "Line 4, after");
    assert!(app.until(cx, |w| w.try_find("remark-composer").is_some()), "no composer");
    app.typed(cx, "Sum the prices with sum()");
    app.click(cx, "save-remark");
    assert_eq!(until_kept(cx, &app, &ws, 1).len(), 1, "the remark was never saved");
    app.click(cx, "send-remarks");
    assert!(app.until(cx, |w| w.try_find("island-claude").is_some()), "Claude's terminal didn't come back");
    assert!(app.until_terminal(cx, "Sum the prices with sum()"), "{}", app.terminal_text(cx));
    assert!(app.until(cx, |w| w.find("terminal").focused() == Some(true)), "the keyboard isn't on Claude");
    assert!(!ui(cx, &app).review_maximized);
}

/// The palette has the layouts while a workspace is on screen, and says which one is on screen.
#[gpui_kit::test]
fn the_palette_changes_the_layout_too(cx: &mut TestAppContext) {
    let (app, _, _) = start(cx);
    let pick = |cx: &mut TestAppContext, words: &str, want: &str| {
        app.press(cx, "cmd-k");
        assert!(app.until(cx, |w| w.try_find("command-palette").is_some()), "the palette never opened");
        app.typed(cx, words);
        assert!(app.until(cx, |w| said(w, ("palette-row", 0usize)).as_deref() == Some(want)), "no row says {want}");
        let current = look(cx, &app, |w| w.try_find(("palette-current", 0usize)).is_some());
        app.press(cx, "enter");
        assert!(app.until(cx, |w| w.try_find("command-palette").is_none()), "the palette stayed");
        current
    };
    // above is the current one; picking beside puts the changes beside
    assert!(pick(cx, "above the", "Changes above the terminal"), "the layout on screen doesn't say Current");
    assert!(!pick(cx, "beside", "Changes beside the terminal"));
    assert!(app.until(cx, |w| chosen(w).as_deref() == Some("Changes beside the terminal")));
    assert!(pick(cx, "beside", "Changes beside the terminal"), "beside doesn't say Current now");
    // the terminal alone, and the changes back
    pick(cx, "terminal only", "Terminal only");
    assert!(app.until(cx, |w| w.try_find("island-changes").is_none()));
    pick(cx, "show the changes", "Show the changes again");
    assert!(app.until(cx, |w| w.try_find("island-changes").is_some()));
    // the changes alone, and back
    pick(cx, "full size", "Changes in full size");
    assert!(app.until(cx, |w| w.try_find("island-claude").is_none()));
    pick(cx, "back to the", "Back to the terminal");
    assert!(app.until(cx, |w| w.try_find("island-claude").is_some()));
    // the diffs in one column, and side by side again
    pick(cx, "one column", "Diffs in one column");
    assert!(app.until(cx, |w| w.try_find("diff-unified").is_some_and(|one| one.selected() == Some(true))));
    pick(cx, "side by side", "Diffs before and after, side by side");
    assert!(app.until(cx, |w| w.try_find("diff-split").is_some_and(|two| two.selected() == Some(true))));
}

/// Item 8.4: ⌘+ and ⌘− resize the text of what
/// has the keyboard, and ⌘0 puts it back. The code's while the changes have the keyboard, with its lines keeping their
/// share of height; the terminal's while a terminal has it, as in every terminal.
#[gpui_kit::test]
fn the_keys_resize_the_code_or_the_terminals_text_whichever_has_the_keyboard(cx: &mut TestAppContext) {
    let (app, _, _) = start(cx);
    // how far apart two lines of code stand
    let rows = |cx: &mut TestAppContext| {
        look(cx, &app, |w| line_number(w, "Line 5, after").unwrap().y - line_number(w, "Line 4, after").unwrap().y)
    };
    let sizes = |cx: &mut TestAppContext| {
        let ui = ui(cx, &app);
        (ui.code_size, ui.terminal_size)
    };
    let terminal_text = |cx: &mut TestAppContext| cx.update(|cx| enkidu_app::terminal_view::text_size(cx));
    assert_eq!((sizes(cx), rows(cx), terminal_text(cx)), ((12, 12.5), px(20.), px(12.5)), "as DESIGN.md's Type says");

    // the changes alone: they have the keyboard
    app.click(cx, "terminal");
    assert!(app.until(cx, |w| w.find("terminal").focused() == Some(true)));
    app.press(cx, "cmd-shift-e");
    assert!(app.until(cx, |w| w.try_find("island-claude").is_none()), "⇧⌘E");
    app.press(cx, "cmd-=");
    assert_eq!(sizes(cx), (13, 12.5), "⌘= (⌘+ without Shift): the code, not the terminal");
    assert_eq!(rows(cx), px(22.));
    app.press(cx, "cmd-+");
    assert_eq!((sizes(cx), rows(cx)), ((14, 12.5), px(23.)));
    for _ in 0..3 {
        app.press(cx, "cmd--");
    }
    assert_eq!((sizes(cx), rows(cx)), ((11, 12.5), px(18.)), "⌘− three times");
    // the ends hold
    for _ in 0..30 {
        app.press(cx, "cmd--");
    }
    assert_eq!((sizes(cx), rows(cx)), ((9, 12.5), px(15.)));
    for _ in 0..30 {
        app.press(cx, "cmd-=");
    }
    assert_eq!((sizes(cx), rows(cx)), ((24, 12.5), px(40.)));
    app.press(cx, "cmd-0");
    assert_eq!((sizes(cx), rows(cx)), ((12, 12.5), px(20.)), "⌘0: the usual size");

    // both islands again, the keyboard in the terminal: now the keys are the terminal's
    app.press(cx, "cmd-shift-e");
    assert!(app.until(cx, |w| w.try_find("island-claude").is_some()));
    app.click(cx, "terminal");
    assert!(app.until(cx, |w| w.find("terminal").focused() == Some(true)));
    app.press(cx, "cmd-=");
    assert_eq!((sizes(cx), terminal_text(cx)), ((12, 13.5), px(13.5)), "the terminal's text, not the code");
    assert_eq!(rows(cx), px(20.));
    app.press(cx, "cmd--");
    app.press(cx, "cmd--");
    assert_eq!((sizes(cx), terminal_text(cx)), ((12, 11.5), px(11.5)));
    // the terminal still draws, on a grid of the new size
    look(cx, &app, |w| assert!(w.try_find("terminal").is_some()));
    app.press(cx, "cmd-0");
    assert_eq!((sizes(cx), terminal_text(cx)), ((12, 12.5), px(12.5)));

    // the usual way there: the keyboard is in Claude's terminal, a click on a changed file in the panel at the right
    // takes the keyboard to its diff, so the keys are the code's
    assert!(app.until(cx, |w| w.find("terminal").focused() == Some(true)));
    assert!(app.until(cx, |w| w.try_find(("change-row", 0usize)).is_some()), "the Changed files panel");
    app.click(cx, ("change-row", 0usize));
    let in_changes = |cx: &mut TestAppContext| {
        let island = app.shell.read_with(cx, |shell, _| shell.changes().cloned()).expect("the island");
        cx.update_window(app.window, |_, w, cx| {
            w.render_frame(cx);
            island.read(cx).has_keyboard(w, cx)
        })
        .unwrap()
    };
    assert!(in_changes(cx), "a click on a changed file left the keyboard in the terminal");
    app.press(cx, "cmd-=");
    assert_eq!((sizes(cx), terminal_text(cx)), ((13, 12.5), px(12.5)), "the code you are looking at, not the terminal");
    app.press(cx, "cmd-0");
    assert_eq!(sizes(cx), (12, 12.5));
}
