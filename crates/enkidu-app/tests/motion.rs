//! Item 3.11: this screen's motion (DESIGN.md, "Motion"), in a real window with the clock in the test's hands. The
//! changes rise in when Claude's first edit lands; a panel slides in from the right edge; a new changed file's row
//! flashes once; +/− counts tick into place when they change, never on first show; the bar's count pops; a remark's
//! box rises. None of it with "Reduce motion": everything is in place at once.
//!
//! The other whole-app tests run with reduced motion (see `common`); these turn it off, look at the first frame of
//! each motion, then move the clock past it.

mod common;

use std::path::PathBuf;
use std::time::Duration;

use common::{App, click_line, line_number, said};
use enkidu_testkit::git;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Bounds, InputEvent as _, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    Pixels, Point, TestAppContext, Window, px,
};

const CODE: &str =
    "def total(items):\n    result = 0\n    for item in items:\n        result += item.price\n    return result\n";

fn make(shared: &std::sync::Arc<enkidu_core::state::Shared>, project: &str, _: &std::path::Path) {
    App::make(shared, project, "stock-fix", "new", "stock-fix", Some("development"))
}

/// The sample project with one workspace and nothing to review yet. `moves` is whether things move.
fn start(cx: &mut TestAppContext, moves: bool) -> (App, PathBuf) {
    let app = common::start_on(cx, enkidu_testkit::shop, make);
    cx.update(|cx| cx.set_reduce_motion(!moves));
    let (_, _, folder) = app.on_screen(cx).expect("on screen");
    assert!(app.until(cx, |w| w.try_find("island-claude").is_some()));
    (app, PathBuf::from(folder))
}

fn look<R>(cx: &mut TestAppContext, app: &App, at: impl FnOnce(&mut Window) -> R) -> R {
    cx.run_until_parked();
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        at(w)
    })
    .unwrap()
}

fn bounds(cx: &mut TestAppContext, app: &App, id: &'static str) -> Bounds<Pixels> {
    look(cx, app, |w| w.find(id).bounds())
}

/// Moves the clock past any motion (the longest here is the flash, 1.8 s) and draws.
fn settle(cx: &mut TestAppContext, app: &App, by: u64) {
    cx.executor().advance_clock(Duration::from_millis(by));
    cx.run_until_parked();
    look(cx, app, |_| ());
}

/// Waits for `id` to come on screen without drawing the frames in between at later clock times: the first frame it
/// is found in is the first frame of its motion.
fn appears(cx: &mut TestAppContext, app: &App, id: &'static str) -> Bounds<Pixels> {
    assert!(app.until(cx, |w| w.try_find(id).is_some()), "{id} never came on screen");
    look(cx, app, |w| w.find(id).bounds())
}

#[gpui_kit::test]
fn the_changes_rise_in_when_claudes_first_edit_lands(cx: &mut TestAppContext) {
    let (app, folder) = start(cx, true);
    // the first edit: the island is 6 px under its place, then in place 220 ms later
    std::fs::write(folder.join("src/app.py"), CODE).unwrap();
    let first = appears(cx, &app, "island-changes");
    // it eases out: most of the way is done early
    settle(cx, &app, 30);
    let halfway = bounds(cx, &app, "island-changes");
    settle(cx, &app, 300);
    let place = bounds(cx, &app, "island-changes");
    assert_eq!(first.top() - place.top(), px(6.), "it starts 6 px under its place");
    assert!(halfway.top() > place.top() && halfway.top() < first.top(), "{first:?} {halfway:?} {place:?}");
    assert_eq!((first.size, first.left()), (place.size, place.left()), "only its place moves");
    settle(cx, &app, 300);
    assert_eq!(bounds(cx, &app, "island-changes"), place, "and it stays there");

    // the changes go (the edit is undone) and come back: they rise again
    git(&folder, &["checkout", "-q", "--", "src/app.py"]);
    assert!(app.until(cx, |w| w.try_find("island-changes").is_none()), "the island stayed");
    std::fs::write(folder.join("src/app.py"), CODE).unwrap();
    let again = appears(cx, &app, "island-changes");
    assert_eq!(again.top() - place.top(), px(6.), "the second arrival rises too");
    settle(cx, &app, 300);

    // put away and brought back by you (⌥⌘B twice): nothing arrived, so nothing rises
    app.press(cx, "alt-cmd-b");
    assert!(app.until(cx, |w| w.try_find("island-changes").is_none()));
    app.press(cx, "alt-cmd-b");
    assert_eq!(appears(cx, &app, "island-changes"), place, "your own switch doesn't make it rise");

    // a window that opens with changes already there shows them in place
    let app = common::restart(cx, app);
    cx.update(|cx| cx.set_reduce_motion(false));
    assert_eq!(appears(cx, &app, "island-changes").top(), place.top(), "it rose at launch");
}

#[gpui_kit::test]
fn another_workspace_coming_on_screen_shows_its_changes_in_place(cx: &mut TestAppContext) {
    let app = common::start_on(cx, enkidu_testkit::shop, |shared, project, _| {
        App::make(shared, project, "one", "new", "one", Some("development"));
        App::make(shared, project, "two", "new", "two", Some("development"));
    });
    cx.update(|cx| cx.set_reduce_motion(false));
    // the second workspace has changes; the first, on screen, has none
    app.click(cx, ("tab", 1usize));
    let (_, _, folder) = app.on_screen(cx).expect("on screen");
    std::fs::write(PathBuf::from(folder).join("src/app.py"), CODE).unwrap();
    appears(cx, &app, "island-changes");
    settle(cx, &app, 300);
    let place = bounds(cx, &app, "island-changes");
    app.click(cx, ("tab", 0usize));
    assert!(app.until(cx, |w| w.try_find("island-changes").is_none()), "the first workspace has nothing to review");
    // back to the second: its changes are where they were, at once
    app.click(cx, ("tab", 1usize));
    assert_eq!(appears(cx, &app, "island-changes"), place, "switching workspaces made the changes rise");
}

#[gpui_kit::test]
fn the_panel_slides_in_from_the_right_edge(cx: &mut TestAppContext) {
    let (app, _) = start(cx, true);
    // as the window opened, the panel is in its place
    let place = bounds(cx, &app, "island-panel");
    // closed, and opened again: it comes from 14 px to the right
    app.press(cx, "alt-cmd-1");
    assert!(app.until(cx, |w| w.try_find("island-panel").is_none()));
    app.press(cx, "alt-cmd-1");
    let first = appears(cx, &app, "island-panel");
    assert_eq!(first.left() - place.left(), px(14.), "it starts 14 px to the right");
    assert_eq!((first.top(), first.size), (place.top(), place.size));
    settle(cx, &app, 300);
    assert_eq!(bounds(cx, &app, "island-panel"), place);
    // its other tab just shows: the panel is there already, so nothing arrives
    app.press(cx, "alt-cmd-2");
    assert!(app.until(cx, |w| said(w, "panel-band").as_deref() == Some("Files")));
    assert_eq!(bounds(cx, &app, "island-panel"), place, "the other tab made the panel slide");
    // and so does the other tab's key: closed, then open again from 14 px to the right
    app.press(cx, "alt-cmd-2");
    assert!(app.until(cx, |w| w.try_find("island-panel").is_none()));
    app.press(cx, "alt-cmd-2");
    assert_eq!(appears(cx, &app, "island-panel").left() - place.left(), px(14.), "the other key");
    settle(cx, &app, 300);
    assert_eq!(bounds(cx, &app, "island-panel"), place);
}

#[gpui_kit::test]
fn a_new_changed_files_row_flashes_once_and_the_tabs_count_ticks(cx: &mut TestAppContext) {
    let (app, folder) = start(cx, true);
    std::fs::write(folder.join("src/app.py"), CODE).unwrap();
    assert!(app.until(cx, |w| w.try_find(("change-row", 0usize)).is_some()), "the file was never listed");
    settle(cx, &app, 2000);
    let place = bounds(cx, &app, "panel-count");
    // a file that turns up while the panel is on screen lights up; the one that was there doesn't
    std::fs::write(folder.join("notes.md"), "# Notes\n").unwrap();
    assert!(app.until(cx, |w| w.try_find(("change-row", 1usize)).is_some()), "the new file was never listed");
    let (flashes, new_row, old_row) = look(cx, &app, |w| {
        let flashes: Vec<Bounds<Pixels>> = w.find_all("row-flash").iter().map(|flash| flash.bounds()).collect();
        // the tree lists a folder's file before the top's own: notes.md is the second file
        (flashes, w.find(("change-row", 1usize)), w.find(("change-row", 0usize)))
    });
    assert_eq!(new_row.label(), Some("notes.md"));
    assert_eq!(flashes, [new_row.bounds()], "the new file's row, and only it: {:?}", old_row.bounds());
    // the count on the tab went from 1 to 2: it ticks into place from 4 px above
    let ticking = bounds(cx, &app, "panel-count");
    assert_eq!(place.top() - ticking.top(), px(4.), "the count didn't tick: {ticking:?}");
    settle(cx, &app, 400);
    assert_eq!(bounds(cx, &app, "panel-count").top(), place.top());
    assert_eq!(look(cx, &app, |w| said(w, "panel-count")).as_deref(), Some("2"));
    // 1.8 s later it has settled: the row is drawn without the light
    settle(cx, &app, 1900);
    assert!(look(cx, &app, |w| w.find_all("row-flash").is_empty()), "the flash stayed");

    // a file that turns up in a closed folder (item 29.4): the folder's row lights up, for it stands for what it holds
    let folder_says = |w: &Window| w.find(("change-folder", 0usize)).description().map(String::from);
    assert_eq!(look(cx, &app, |w| w.find(("change-folder", 0usize)).label().map(String::from)).as_deref(), Some("src"));
    app.click(cx, ("change-folder", 0usize));
    assert!(app.until(cx, |w| folder_says(w).as_deref() == Some("closed, 1 file")), "the folder didn't close");
    std::fs::write(folder.join("src/extra.py"), "EXTRA = 1\n").unwrap();
    assert!(app.until(cx, |w| folder_says(w).as_deref() == Some("closed, 2 files")), "the new file was never counted");
    let (flashes, closed_folder) = look(cx, &app, |w| {
        let flashes: Vec<Bounds<Pixels>> = w.find_all("row-flash").iter().map(|flash| flash.bounds()).collect();
        (flashes, w.find(("change-folder", 0usize)).bounds())
    });
    assert_eq!(flashes, [closed_folder], "the closed folder's row, and only it");
    settle(cx, &app, 1900);
    assert!(look(cx, &app, |w| w.find_all("row-flash").is_empty()), "the folder's flash stayed");

    // the panel closed and opened again: what it lists then was there, nothing lights up
    std::fs::write(folder.join("later.md"), "later\n").unwrap();
    app.press(cx, "alt-cmd-1");
    assert!(app.until(cx, |w| w.try_find("island-panel").is_none()));
    assert!(app.until(cx, |w| said(w, "changes-count").as_deref() == Some("4 files")), "the fourth file never came");
    app.press(cx, "alt-cmd-1");
    assert!(app.until(cx, |w| w.try_find(("change-row", 2usize)).is_some()));
    assert!(look(cx, &app, |w| w.find_all("row-flash").is_empty()), "a file that was there when the panel opened");
}

#[gpui_kit::test]
fn counts_tick_into_place_when_they_change_never_on_first_show(cx: &mut TestAppContext) {
    let (app, folder) = start(cx, true);
    std::fs::write(folder.join("src/app.py"), CODE).unwrap();
    // first show: +4 on the file's header and on its row, each in its place
    assert!(app.until(cx, |w| said(w, "file-added").as_deref() == Some("+4")), "the diff's header never showed");
    settle(cx, &app, 400);
    let (header, row) = (bounds(cx, &app, "file-added"), bounds(cx, &app, "row-added"));
    assert_eq!(look(cx, &app, |w| w.find("file-added").bounds()), header);

    // one more line: +5 starts 4 px above its place and comes down in 220 ms
    std::fs::write(folder.join("src/app.py"), format!("{CODE}\n")).unwrap();
    assert!(app.until(cx, |w| said(w, "file-added").as_deref() == Some("+5")), "the count never changed");
    let (ticking, ticking_row) = (bounds(cx, &app, "file-added"), bounds(cx, &app, "row-added"));
    assert_eq!(header.top() - ticking.top(), px(4.), "the header's count starts 4 px up");
    assert_eq!(row.top() - ticking_row.top(), px(4.), "the row's count starts 4 px up");
    settle(cx, &app, 300);
    assert_eq!(bounds(cx, &app, "file-added").top(), header.top());
    assert_eq!(bounds(cx, &app, "row-added").top(), row.top());

    // the same number drawn again doesn't move
    app.press(cx, "alt-cmd-l");
    assert!(app.until(cx, |w| w.try_find("layout-divider").is_some()));
    app.press(cx, "alt-cmd-l");
    settle(cx, &app, 300);
    assert_eq!(bounds(cx, &app, "row-added").top(), row.top());
}

#[gpui_kit::test]
fn a_remarks_box_rises_as_it_opens(cx: &mut TestAppContext) {
    let (app, folder) = start(cx, true);
    std::fs::write(folder.join("src/app.py"), CODE).unwrap();
    assert!(app.until(cx, |w| line_number(w, "Line 5, after").is_some()), "the diff never showed");
    settle(cx, &app, 400);
    click_line(cx, &app, "Line 4, after");
    let first = appears(cx, &app, "remark-composer");
    settle(cx, &app, 300);
    let place = bounds(cx, &app, "remark-composer");
    assert_eq!(first.top() - place.top(), px(6.), "it starts 6 px under its place");
    // it takes what is typed all the same
    app.typed(cx, "Sum the prices with sum()");
    assert_eq!(bounds(cx, &app, "remark-composer").top(), place.top());
}

/// With "Reduce motion" on, each of these is in its place from the first frame.
#[gpui_kit::test]
fn nothing_moves_with_reduced_motion(cx: &mut TestAppContext) {
    let (app, folder) = start(cx, false);
    // the changes arrive
    std::fs::write(folder.join("src/app.py"), CODE).unwrap();
    let first = appears(cx, &app, "island-changes");
    settle(cx, &app, 400);
    assert_eq!(bounds(cx, &app, "island-changes"), first, "the changes rose");
    // a panel is opened
    let place = bounds(cx, &app, "island-panel");
    app.press(cx, "alt-cmd-1");
    assert!(app.until(cx, |w| w.try_find("island-panel").is_none()));
    app.press(cx, "alt-cmd-1");
    assert_eq!(appears(cx, &app, "island-panel"), place, "the panel slid");
    // a count that changes
    assert!(app.until(cx, |w| said(w, "file-added").as_deref() == Some("+4")));
    let (count, on_the_tab) = (bounds(cx, &app, "file-added"), bounds(cx, &app, "panel-count"));
    std::fs::write(folder.join("src/app.py"), format!("{CODE}\n")).unwrap();
    assert!(app.until(cx, |w| said(w, "file-added").as_deref() == Some("+5")), "the count never changed");
    assert_eq!(bounds(cx, &app, "file-added").top(), count.top(), "the count ticked");
    // a new changed file, and the count on the panel's tab
    std::fs::write(folder.join("notes.md"), "# Notes\n").unwrap();
    assert!(app.until(cx, |w| w.try_find(("change-row", 1usize)).is_some()), "the new file was never listed");
    assert!(look(cx, &app, |w| w.find_all("row-flash").is_empty()), "a row flashed");
    assert_eq!(bounds(cx, &app, "panel-count").top(), on_the_tab.top(), "the tab's count ticked");
    // a remark's box (once the new file's diff is read: its lines arriving above would move everything under them,
    // which is no motion of the box)
    assert!(
        app.until(cx, |w| w.find_all("source").iter().any(|line| line.label() == Some("# Notes"))),
        "the new file's diff was never read"
    );
    assert!(app.until(cx, |w| line_number(w, "Line 4, after").is_some()));
    click_line(cx, &app, "Line 4, after");
    let first = appears(cx, &app, "remark-composer");
    settle(cx, &app, 400);
    assert_eq!(bounds(cx, &app, "remark-composer").top(), first.top(), "the remark's box rose");
}

/// Item 4.6: a menu grows out of its trigger. It fades in while it slides its last 6 px from the trigger's edge, in
/// 160 ms (GPUI can't scale a subtree); with reduced motion it is in place at once.
#[gpui_kit::test]
fn a_menu_slides_out_of_its_trigger_as_it_opens(cx: &mut TestAppContext) {
    let (app, _) = start(cx, true);
    app.click(cx, "ws-menu");
    // the first frame it is on screen in: 6 px above where it ends up, on its way down
    let first = appears(cx, &app, "popup-menu");
    // (an eighth of its time: the curve is steep at the start, so by half time it is all but there)
    settle(cx, &app, 20);
    let midway = bounds(cx, &app, "popup-menu");
    settle(cx, &app, 300);
    let there = bounds(cx, &app, "popup-menu");
    assert_eq!(there.top() - first.top(), px(6.), "it slid 6 px down from its trigger");
    assert!(midway.top() > first.top() && midway.top() < there.top(), "{first:?} {midway:?} {there:?}");
    assert_eq!((first.left(), first.size), (there.left(), there.size), "only down");
    // it hangs under its trigger's right corner
    let trigger = bounds(cx, &app, "ws-menu");
    assert!(
        there.top() >= trigger.bottom() && (there.right() - trigger.right()).abs() < px(2.),
        "{there:?} {trigger:?}"
    );
    // dismissed, it fades for 110 ms where it stood ("out in 110"), and nothing in it can be pressed any more
    let remove = bounds(cx, &app, "remove-workspace").center();
    app.press(cx, "escape");
    settle(cx, &app, 60);
    let leaving = bounds(cx, &app, "popup-menu");
    assert!(
        near(leaving.left(), there.left()) && near(leaving.top(), there.top()) && near(leaving.right(), there.right()),
        "it moved as it left: {leaving:?}, was {there:?}"
    );
    common::press(cx, &app, remove, remove, false);
    settle(cx, &app, 60);
    assert!(look(cx, &app, |w| w.try_find("popup-menu").is_none()), "it stayed past its 110 ms");
    assert!(look(cx, &app, |w| w.try_find("remove-dialog").is_none()), "a menu that was leaving took a press");
    // opened again, it moves again
    app.click(cx, "ws-menu");
    let again = appears(cx, &app, "popup-menu");
    assert_eq!(again.top(), first.top(), "the second opening starts where the first did");
    settle(cx, &app, 300);
    app.press(cx, "escape");
    settle(cx, &app, 150);
    assert!(look(cx, &app, |w| w.try_find("popup-menu").is_none()));

    // with reduced motion it is where it stays at once
    cx.update(|cx| cx.set_reduce_motion(true));
    app.click(cx, "ws-menu");
    let still = appears(cx, &app, "popup-menu");
    assert_eq!(still.top(), there.top(), "nothing moves with reduced motion");
    // and it is gone at once
    app.press(cx, "escape");
    assert!(look(cx, &app, |w| w.try_find("popup-menu").is_none()), "it lingered with reduced motion");
}

/// The mouse goes down at `at` and stays down (`mouse_up` lets it go).
fn mouse_down(cx: &mut TestAppContext, app: &App, at: Point<Pixels>) {
    let modifiers = Modifiers::default();
    cx.update_window(app.window, |_, w, cx| {
        w.dispatch_event(MouseMoveEvent { position: at, pressed_button: None, modifiers }.to_platform_input(), cx);
        w.render_frame(cx);
        let down =
            MouseDownEvent { button: MouseButton::Left, position: at, modifiers, click_count: 1, first_mouse: false };
        w.dispatch_event(down.to_platform_input(), cx);
    })
    .unwrap();
}

fn mouse_up(cx: &mut TestAppContext, app: &App, at: Point<Pixels>) {
    let (modifiers, left) = (Modifiers::default(), MouseButton::Left);
    cx.update_window(app.window, |_, w, cx| {
        w.dispatch_event(
            MouseMoveEvent { position: at, pressed_button: Some(left), modifiers }.to_platform_input(),
            cx,
        );
        w.render_frame(cx);
        w.dispatch_event(
            MouseUpEvent { button: left, position: at, modifiers, click_count: 1 }.to_platform_input(),
            cx,
        );
    })
    .unwrap();
}

/// Within half a pixel: what is laid out lands on whole pixels.
fn near(a: Pixels, b: Pixels) -> bool {
    (a - b).abs() <= px(0.5)
}

/// Where `id` is in the New workspace dialog once the dialog has all it shows: its branches are read off the
/// drawing thread (and fetched), and what they bring moves its buttons.
fn at_rest(cx: &mut TestAppContext, app: &App, id: &'static str) -> Bounds<Pixels> {
    let read = |w: &Window| matches!(said(w, "ws-base-hint").as_deref(), Some("Fresh from origin" | "Local branches"));
    assert!(app.until(cx, read), "the branches were never read");
    let mut last = bounds(cx, app, id);
    let mut same = 0;
    for _ in 0..600 {
        std::thread::sleep(Duration::from_millis(5));
        let now = bounds(cx, app, id);
        same = if now == last { same + 1 } else { 0 };
        last = now;
        if same >= 20 {
            break;
        }
    }
    last
}

/// DESIGN.md, "Motion": "A press (`press::pressable`): scale 0.97, 120 ms: the button heard you". GPUI can't scale an
/// element, so the button's box gets 3 % smaller around its middle; what is beside it stays, and the click counts.
#[gpui_kit::test]
fn a_button_is_drawn_smaller_while_the_mouse_is_down_on_it(cx: &mut TestAppContext) {
    let (app, _) = start(cx, true);
    app.press(cx, "cmd-n");
    assert!(app.until(cx, |w| w.try_find("new-workspace-dialog").is_some()), "the dialog never opened");
    settle(cx, &app, 600);
    let rest = at_rest(cx, &app, "ws-cancel");
    let beside = bounds(cx, &app, "create-workspace");
    let dialog = bounds(cx, &app, "new-workspace-dialog");

    // the mouse goes down in its middle: it is still whole in that frame, on its way in 10 ms later (the curve is
    // steep at the start), and 3 % smaller around its middle from 120 ms on
    mouse_down(cx, &app, rest.center());
    assert_eq!(bounds(cx, &app, "ws-cancel"), rest, "the first frame of a press is the button as it was");
    settle(cx, &app, 10);
    let midway = bounds(cx, &app, "ws-cancel");
    settle(cx, &app, 300);
    let pressed = bounds(cx, &app, "ws-cancel");
    assert!(near(pressed.size.width, rest.size.width * 0.97), "{pressed:?} of {rest:?}");
    assert!(near(pressed.size.height, rest.size.height * 0.97), "{pressed:?} of {rest:?}");
    assert!(near(pressed.center().x, rest.center().x) && near(pressed.center().y, rest.center().y));
    assert!(midway.size.width < rest.size.width && midway.size.width > pressed.size.width, "{midway:?}");
    // nothing around it moved
    assert_eq!(bounds(cx, &app, "create-workspace"), beside);
    assert_eq!(bounds(cx, &app, "new-workspace-dialog"), dialog);
    // held longer, it stays as it is
    settle(cx, &app, 2_000);
    assert_eq!(bounds(cx, &app, "ws-cancel"), pressed);

    // the mouse leaves it and comes up elsewhere: no click, and it is whole again 120 ms later
    mouse_up(cx, &app, dialog.center());
    settle(cx, &app, 40);
    let back = bounds(cx, &app, "ws-cancel");
    assert!(back.size.width > pressed.size.width && back.size.width < rest.size.width, "{back:?}");
    settle(cx, &app, 300);
    assert_eq!(bounds(cx, &app, "ws-cancel"), rest);
    assert!(look(cx, &app, |w| w.try_find("new-workspace-dialog").is_some()), "a press that left it is no click");

    // a button that is off doesn't answer (Create workspace, with no name yet)
    mouse_down(cx, &app, beside.center());
    settle(cx, &app, 300);
    assert_eq!(bounds(cx, &app, "create-workspace"), beside);
    mouse_up(cx, &app, beside.center());
    settle(cx, &app, 300);

    // with "Reduce motion" nothing moves
    cx.update(|cx| cx.set_reduce_motion(true));
    mouse_down(cx, &app, rest.center());
    settle(cx, &app, 300);
    assert_eq!(bounds(cx, &app, "ws-cancel"), rest, "nothing moves with reduced motion");
    mouse_up(cx, &app, dialog.center());
    settle(cx, &app, 300);
    cx.update(|cx| cx.set_reduce_motion(false));

    // pressed a pixel from its edge it doesn't shrink away from the pointer, so the mouse comes up on it: a click
    let edge = gpui_kit::point(rest.right() - px(1.), rest.center().y);
    mouse_down(cx, &app, edge);
    settle(cx, &app, 300);
    assert_eq!(bounds(cx, &app, "ws-cancel"), rest, "it would have left the pointer outside");
    mouse_up(cx, &app, edge);
    // (the dialog leaves over a moment of the clock)
    settle(cx, &app, 600);
    assert!(look(cx, &app, |w| w.try_find("new-workspace-dialog").is_none()), "the click at its edge was lost");

    // and a press held in the middle until it is all the way in is a click too
    app.press(cx, "cmd-n");
    assert!(app.until(cx, |w| w.try_find("new-workspace-dialog").is_some()));
    settle(cx, &app, 600);
    let rest = at_rest(cx, &app, "ws-cancel");
    let inside = gpui_kit::point(rest.left() + px(8.), rest.center().y);
    mouse_down(cx, &app, inside);
    settle(cx, &app, 300);
    assert!(bounds(cx, &app, "ws-cancel").size.width < rest.size.width);
    mouse_up(cx, &app, inside);
    settle(cx, &app, 600);
    assert!(look(cx, &app, |w| w.try_find("new-workspace-dialog").is_none()), "the click was lost");
}

/// The pointer goes to `id` (a tooltip comes when it arrives from somewhere else).
fn point_at(cx: &mut TestAppContext, app: &App, id: &'static str) {
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        w.hover(id, cx);
    })
    .unwrap();
}

/// What the tooltip on screen says and where it is (one that is leaving is still drawn).
fn tip(cx: &mut TestAppContext, app: &App) -> Option<(String, Bounds<Pixels>)> {
    look(cx, app, |w| w.try_find("tip").map(|tip| (tip.label().unwrap_or_default().to_string(), tip.bounds())))
}

/// DESIGN.md, "Motion": "Menus, tooltips: grow from their trigger, 160 ms; out in 110; the next tooltip along a toolbar
/// is instant". The wait is 450 ms, the quarter second for the next one 250 ms.
#[gpui_kit::test]
fn a_tooltip_eases_in_under_its_button_after_the_wait_and_fades_as_the_pointer_leaves(cx: &mut TestAppContext) {
    let (app, _) = start(cx, true);
    point_at(cx, &app, "terminal");
    point_at(cx, &app, "layout-side");
    // nothing until the pointer has rested there for 450 ms
    settle(cx, &app, 400);
    assert_eq!(tip(cx, &app), None, "it came before its time");
    // then it eases in: the first frame 3 px nearer its button than where it stays (the clock is moved to the
    // moment it comes and no further: the window is drawn as it comes)
    cx.executor().advance_clock(Duration::from_millis(50));
    let (said, first) = tip(cx, &app).expect("the tooltip never came");
    assert_eq!(said, "Changes beside the terminal");
    settle(cx, &app, 20);
    let midway = tip(cx, &app).expect("still there").1;
    settle(cx, &app, 300);
    let there = tip(cx, &app).expect("still there").1;
    assert_eq!(there.top() - first.top(), px(3.), "it slid 3 px out of its button");
    assert!(midway.top() > first.top() && midway.top() < there.top(), "{first:?} {midway:?} {there:?}");
    assert_eq!((first.left(), first.size), (there.left(), there.size), "only down");
    // under its button, 6 px away, and inside the window
    let button = bounds(cx, &app, "layout-side");
    assert_eq!(there.top(), button.bottom() + px(6.), "{there:?} under {button:?}");
    let window = look(cx, &app, |w| w.viewport_size());
    assert!(there.right() <= window.width - px(8.) && there.left() >= px(8.), "{there:?} in {window:?}");

    // the pointer leaves: it fades for 110 ms and is gone
    point_at(cx, &app, "terminal");
    settle(cx, &app, 60);
    assert!(tip(cx, &app).is_some(), "it went before it had faded");
    settle(cx, &app, 60);
    assert_eq!(tip(cx, &app), None, "it stayed past its 110 ms");

    // back on a button within a quarter of a second: no wait and no motion, as along a bar
    settle(cx, &app, 100);
    point_at(cx, &app, "layout-stack");
    let (said, at_once) = tip(cx, &app).expect("the next one waited");
    assert_eq!(said, "Changes above the terminal");
    settle(cx, &app, 300);
    assert_eq!(tip(cx, &app).map(|(_, bounds)| bounds), Some(at_once), "the next one moved");
    // and from one button straight to the one beside it
    point_at(cx, &app, "layout-side");
    assert_eq!(tip(cx, &app).map(|(said, _)| said).as_deref(), Some("Changes beside the terminal"));

    // later than that, it waits again
    point_at(cx, &app, "terminal");
    settle(cx, &app, 400);
    point_at(cx, &app, "layout-stack");
    settle(cx, &app, 400);
    assert_eq!(tip(cx, &app), None, "it came without its wait");
    settle(cx, &app, 100);
    assert_eq!(tip(cx, &app).map(|(said, _)| said).as_deref(), Some("Changes above the terminal"));

    // a key sends it away, and so does the wheel
    app.press(cx, "shift-f12");
    assert_eq!(tip(cx, &app), None, "a key left it there");
    point_at(cx, &app, "terminal");
    settle(cx, &app, 400);
    point_at(cx, &app, "layout-stack");
    settle(cx, &app, 460);
    assert!(tip(cx, &app).is_some());
    let on_the_button = bounds(cx, &app, "layout-stack").center();
    cx.update_window(app.window, |_, w, cx| {
        let wheel = gpui_kit::ScrollEvent {
            position: on_the_button,
            delta: gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-12.))),
            ..Default::default()
        };
        w.dispatch_event(wheel.to_platform_input(), cx);
    })
    .unwrap();
    assert_eq!(tip(cx, &app), None, "the wheel left it there");

    // with "Reduce motion" it is in its place at once
    cx.update(|cx| cx.set_reduce_motion(true));
    point_at(cx, &app, "terminal");
    settle(cx, &app, 400);
    point_at(cx, &app, "layout-side");
    settle(cx, &app, 460);
    assert_eq!(tip(cx, &app).map(|(_, bounds)| bounds.top()), Some(there.top()), "nothing moves with reduced motion");
}

/// Item 7.2b: a button's tooltip came up over the dialog the button had just opened. A press sends the
/// tooltip away, the one still waiting too, and it says nothing again until the pointer has left the button.
#[gpui_kit::test]
fn a_pressed_button_says_nothing_over_what_it_opened(cx: &mut TestAppContext) {
    let (app, _) = start(cx, true);
    // pressed a moment after the pointer arrived, while its tooltip still waits
    point_at(cx, &app, "terminal");
    point_at(cx, &app, "settings");
    settle(cx, &app, 100);
    app.click(cx, "settings");
    assert!(app.until(cx, |w| w.try_find("settings-dialog").is_some()), "Settings never opened");
    settle(cx, &app, 1_500);
    assert_eq!(tip(cx, &app), None, "the gear's tooltip came up over Settings");
    app.press(cx, "escape");
    settle(cx, &app, 600);
    assert!(look(cx, &app, |w| w.try_find("settings-dialog").is_none()));

    // the pointer is still on the gear: it stays quiet, however long
    point_at(cx, &app, "settings");
    settle(cx, &app, 1_500);
    assert_eq!(tip(cx, &app), None, "it spoke again though the pointer never left");
    // once the pointer has left and come back, it speaks
    point_at(cx, &app, "terminal");
    settle(cx, &app, 600);
    point_at(cx, &app, "settings");
    settle(cx, &app, 500);
    assert_eq!(tip(cx, &app).map(|(said, _)| said).as_deref(), Some("Settings"));

    // pressed while its tooltip is up: gone in that frame, and nothing over the dialog
    app.click(cx, "settings");
    assert!(app.until(cx, |w| w.try_find("settings-dialog").is_some()));
    assert_eq!(tip(cx, &app), None);
    settle(cx, &app, 1_500);
    assert_eq!(tip(cx, &app), None);
}

/// DESIGN.md, "Motion": "Dialogs: rise 8 px and fade, 220 ms" and "The palette, Search: none". The library brings a
/// dialog down into its place, not up: the travel, the time and the curve are the design's.
#[gpui_kit::test]
fn a_dialog_fades_in_over_its_last_8_px_and_the_palette_and_search_are_there_at_once(cx: &mut TestAppContext) {
    let (app, _) = start(cx, true);
    app.press(cx, "cmd-n");
    let first = appears(cx, &app, "new-workspace-dialog");
    settle(cx, &app, 20);
    let midway = bounds(cx, &app, "new-workspace-dialog");
    settle(cx, &app, 199);
    let nearly = bounds(cx, &app, "new-workspace-dialog");
    settle(cx, &app, 2);
    let there = bounds(cx, &app, "new-workspace-dialog");
    settle(cx, &app, 400);
    assert_eq!(bounds(cx, &app, "new-workspace-dialog"), there, "it was still moving after 220 ms");
    assert_eq!(there.top() - first.top(), px(8.), "it travels 8 px: {first:?} to {there:?}");
    assert!(midway.top() > first.top() && midway.top() < there.top(), "{first:?} {midway:?} {there:?}");
    assert!(nearly.top() <= there.top() && there.top() - nearly.top() < px(1.), "{nearly:?} {there:?}");
    assert_eq!((first.left(), first.size), (there.left(), there.size), "only down");
    // it leaves over the same time, and is gone after it
    app.press(cx, "escape");
    settle(cx, &app, 100);
    assert!(look(cx, &app, |w| w.try_find("new-workspace-dialog").is_some()), "it was gone at once");
    settle(cx, &app, 200);
    assert!(look(cx, &app, |w| w.try_find("new-workspace-dialog").is_none()), "it stayed past its time");

    // what the keyboard opens many times a day is there at once, and gone at once
    for (key, id) in [("cmd-k", "command-palette"), ("cmd-p", "search-popup")] {
        app.press(cx, key);
        let first = appears(cx, &app, id);
        settle(cx, &app, 400);
        assert_eq!(bounds(cx, &app, id), first, "{id} moved as it opened");
        app.press(cx, "escape");
        assert!(look(cx, &app, |w| w.try_find(id).is_none()), "{id} took time to go");
    }

    // with "Reduce motion" a dialog is in its place at once too
    cx.update(|cx| cx.set_reduce_motion(true));
    app.press(cx, "cmd-n");
    assert_eq!(appears(cx, &app, "new-workspace-dialog").top(), there.top(), "nothing moves with reduced motion");
}

/// DESIGN.md, "Motion": "A switch's thumb: glides 220 ms to the new choice".
#[gpui_kit::test]
fn a_switch_s_thumb_glides_to_the_new_choice(cx: &mut TestAppContext) {
    let (app, _) = start(cx, true);
    settle(cx, &app, 400);
    let from = bounds(cx, &app, "workspace-layout-thumb");
    let (side, stack) = (bounds(cx, &app, "layout-side"), bounds(cx, &app, "layout-stack"));
    assert_eq!(from.left(), stack.left(), "the thumb starts under the layout chosen: changes above the terminal");
    app.click(cx, "layout-side");
    // halfway through its 220 ms it is between the two, past the middle (a strong ease-out)
    settle(cx, &app, 110);
    let midway = bounds(cx, &app, "workspace-layout-thumb");
    let middle = (side.left() + stack.left()) / 2.;
    assert!(midway.left() < middle && midway.left() > side.left(), "{midway:?} between {side:?} and {stack:?}");
    settle(cx, &app, 300);
    let there = bounds(cx, &app, "workspace-layout-thumb");
    assert_eq!((there.left(), there.size), (side.left(), from.size), "it settles under the new choice");

    // with "Reduce motion" it is there at once
    cx.update(|cx| cx.set_reduce_motion(true));
    app.click(cx, "layout-stack");
    assert_eq!(bounds(cx, &app, "workspace-layout-thumb").left(), stack.left(), "nothing moves with reduced motion");
}

/// DESIGN.md, "Motion": "Toasts: rise from the corner (300 ms), leave to the right (180 ms)".
#[gpui_kit::test]
fn a_toast_rises_into_the_corner_and_leaves_to_the_right(cx: &mut TestAppContext) {
    let (app, _) = start(cx, true);
    cx.update_window(app.window, |_, w, cx| enkidu_app::toast::say(w, "shop is already a project here.", cx)).unwrap();
    // the first frame it is on screen in: 14 px under where it stays
    let first = appears(cx, &app, "toast");
    settle(cx, &app, 30);
    let midway = bounds(cx, &app, "toast");
    settle(cx, &app, 269);
    let nearly = bounds(cx, &app, "toast");
    settle(cx, &app, 2);
    let there = bounds(cx, &app, "toast");
    settle(cx, &app, 600);
    assert_eq!(bounds(cx, &app, "toast"), there, "it was still moving after 300 ms");
    assert_eq!(first.top() - there.top(), px(14.), "it rises 14 px: {first:?} to {there:?}");
    assert!(midway.top() < first.top() && midway.top() > there.top(), "{first:?} {midway:?} {there:?}");
    assert!(near(nearly.top(), there.top()), "{nearly:?} {there:?}");
    assert_eq!((first.left(), first.size), (there.left(), there.size), "only up");
    // in the window's bottom right corner: 16 px from its right edge (over the files' column when that is open),
    // and 8 px above the stage's foot, so it never covers the status line
    let window = look(cx, &app, |w| w.viewport_size());
    let card = look(cx, &app, |w| w.find(("toast-card", 0usize)).bounds());
    let (stage, line) = look(cx, &app, |w| (w.find("stage").bounds(), w.find("status-line").bounds()));
    assert_eq!((card.right(), card.bottom()), (window.width - px(16.), window.height - px(36.)), "{card:?}");
    assert_eq!(stage.bottom() - card.bottom(), px(8.), "{stage:?}");
    assert!(card.bottom() < line.top(), "{card:?} over {line:?}");
    assert_eq!(card.size.width, px(360.));

    // closed, it goes to the right for 180 ms and is gone; the frame of the press still has it in its place
    app.click(cx, ("toast-close", 0usize));
    assert_eq!(bounds(cx, &app, "toast"), there);
    settle(cx, &app, 30);
    let leaving = bounds(cx, &app, "toast");
    assert!(leaving.left() > there.left() && leaving.left() < there.left() + px(28.), "{leaving:?} from {there:?}");
    assert_eq!(leaving.top(), there.top(), "only to the right");
    settle(cx, &app, 160);
    assert!(look(cx, &app, |w| w.try_find("toast").is_none()), "it stayed past its 180 ms");

    // a second one comes under the first, and the first stays where it is
    cx.update_window(app.window, |_, w, cx| {
        enkidu_app::toast::say(w, "The first.", cx);
        enkidu_app::toast::say(w, "The second.", cx);
    })
    .unwrap();
    settle(cx, &app, 600);
    let cards = look(cx, &app, |w| {
        let mut cards: Vec<(String, Bounds<Pixels>)> =
            w.find_all("toast").iter().map(|t| (t.label().unwrap_or_default().to_string(), t.bounds())).collect();
        cards.sort_by(|a, b| a.1.top().partial_cmp(&b.1.top()).unwrap());
        cards
    });
    let said: Vec<&str> = cards.iter().map(|(said, _)| said.as_str()).collect();
    assert_eq!(said, ["The first.", "The second."], "one above the other, the newest last");

    // with "Reduce motion" a toast is in its place at once
    cx.update(|cx| cx.set_reduce_motion(true));
    cx.update_window(app.window, |_, w, cx| enkidu_app::toast::good(w, "Settings saved", cx)).unwrap();
    let at_once = look(cx, &app, |w| {
        w.find_all("toast").iter().find(|t| t.label() == Some("Settings saved")).map(|t| t.bounds())
    })
    .expect("the third toast");
    settle(cx, &app, 600);
    let later = look(cx, &app, |w| {
        w.find_all("toast").iter().find(|t| t.label() == Some("Settings saved")).map(|t| t.bounds())
    });
    assert_eq!(later, Some(at_once), "nothing moves with reduced motion");
}

/// Item 7.5a: what stands under a toast's title can be selected with the mouse and copied, a path or a command's own
/// words.
#[gpui_kit::test]
fn a_toasts_detail_can_be_selected_and_copied(cx: &mut TestAppContext) {
    let (app, _) = start(cx, false);
    let detail = "~/code/shop/stock-fix";
    cx.update_window(app.window, |_, w, cx| enkidu_app::toast::note(w, "Path copied", detail, cx)).unwrap();
    settle(cx, &app, 600);
    let words = bounds(cx, &app, "toast-detail");
    let (from, to) = (
        gpui_kit::point(words.left() + px(1.), words.center().y),
        gpui_kit::point(words.right() - px(1.), words.center().y),
    );
    let (modifiers, left) = (Modifiers::default(), MouseButton::Left);
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        w.dispatch_event(MouseMoveEvent { position: from, pressed_button: None, modifiers }.to_platform_input(), cx);
        w.render_frame(cx);
        let down = MouseDownEvent { button: left, position: from, modifiers, click_count: 1, first_mouse: false };
        w.dispatch_event(down.to_platform_input(), cx);
        w.render_frame(cx);
        for position in [gpui_kit::point((from.x + to.x) / 2., from.y), to] {
            let moved = MouseMoveEvent { position, pressed_button: Some(left), modifiers };
            w.dispatch_event(moved.to_platform_input(), cx);
            w.render_frame(cx);
        }
        w.dispatch_event(
            MouseUpEvent { button: left, position: to, modifiers, click_count: 1 }.to_platform_input(),
            cx,
        );
        w.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    let taken = cx
        .update_window(app.window, |_, w, cx| {
            w.render_frame(cx);
            gpui_kit::base::TextSelection::selected_text(w, cx)
        })
        .unwrap();
    assert_eq!(taken.trim(), detail);
    app.press(cx, "cmd-c");
    assert_eq!(cx.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some(detail));
}

/// How often the window is drawn while `idle` updates of it go by with nothing happening, 100 ms of the clock
/// apart. A test window is drawn after an update only when something asked for a frame.
fn drawn_while_idle(cx: &mut TestAppContext, app: &App, idle: usize) -> u64 {
    settle(cx, app, 2_500);
    settle(cx, app, 2_500);
    let before = app.shell.read_with(cx, |shell, _| shell.drawn());
    for _ in 0..idle {
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        cx.update_window(app.window, |_, _, _| {}).unwrap();
    }
    app.shell.read_with(cx, |shell, _| shell.drawn()) - before
}

/// Smooth first: at rest the window is drawn only for the cursor's blink (about twice a second). What
/// answers a press, what has a tooltip, a toast that stays and text that can be selected must not ask for a frame
/// of their own: one that did would draw with every one of the forty looks, four seconds long.
#[gpui_kit::test]
fn nothing_is_drawn_again_while_nothing_happens(cx: &mut TestAppContext) {
    let (app, _) = start(cx, true);
    const LOOKS: usize = 40;
    // only the blink: far fewer frames than looks
    let blink_only = |drawn: u64, what: &str| {
        println!("IDLE {what}: {drawn} frames in {LOOKS} looks, 100 ms apart");
        assert!(drawn <= 12, "{what}: {drawn} frames in {LOOKS} looks while nothing happens");
    };
    // the frame as it stands: tabs and buttons that answer a press, tooltips on them
    blink_only(drawn_while_idle(cx, &app, LOOKS), "the plain frame");
    // a toast that stays (what went wrong), with a detail that can be selected
    cx.update_window(app.window, |_, w, cx| {
        enkidu_app::toast::warning(w, "Live didn't switch", "git said: the main folder has changes of its own", cx)
    })
    .unwrap();
    blink_only(drawn_while_idle(cx, &app, LOOKS), "a toast that stays");
    // the pointer resting on a button, its tooltip up
    point_at(cx, &app, "terminal");
    point_at(cx, &app, "settings");
    settle(cx, &app, 600);
    assert!(tip(cx, &app).is_some());
    blink_only(drawn_while_idle(cx, &app, LOOKS), "a tooltip that is up");
    // a dialog on screen
    app.press(cx, "cmd-n");
    assert!(app.until(cx, |w| w.try_find("new-workspace-dialog").is_some()));
    at_rest(cx, &app, "ws-cancel");
    blink_only(drawn_while_idle(cx, &app, LOOKS), "a dialog");
}
