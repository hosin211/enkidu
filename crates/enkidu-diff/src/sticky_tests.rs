//! enkidu: item 3.2c. The header of the file being read stays at the top of the view while its lines scroll
//! under it, and the next file's header pushes it away. (In the original a header was a row like any other.)

use std::{cell::RefCell, rc::Rc};

use gpui::{
    AppContext as _, Bounds, Context, Entity, IntoElement, ParentElement as _, Pixels, Render, Styled as _,
    TestAppContext, VisualTestContext, Window, div, point, px,
};
use gpui_kit::test::TestWindowExt as _;

use super::{
    Diff, DiffEvent, DiffFile, DiffLinePosition, DiffSide, DiffState, HEADER_HEIGHT, document::fixture,
    state::DisplayRow,
};

struct Host {
    state: Entity<DiffState>,
    headers: bool,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(400.)).h(px(200.)).child(Diff::new(&self.state).header_visible(self.headers).size_full())
    }
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        _ = window.draw(cx);
    });
}

/// A file of `count` lines whose line `changed` differs from the original.
fn file(name: &str, count: usize, changed: usize) -> DiffFile {
    let original: String = (1..=count).map(|n| format!("line {n}\n")).collect();
    let modified = original.replace(&format!("line {changed}\n"), &format!("changed {changed}\n"));
    fixture::modified(name, &original, &modified)
}

/// a.txt, b.txt (long) and c.txt, every line shown.
fn show(cx: &mut TestAppContext, headers: bool) -> (Entity<DiffState>, &mut VisualTestContext) {
    cx.update(crate::init_for_tests);
    let files = [file("a.txt", 30, 15), file("b.txt", 200, 100), file("c.txt", 20, 5)];
    let state = cx.new(|cx| DiffState::new(files, cx).with_context_lines(None));
    let (_, cx) = cx.add_window_view({
        let state = state.clone();
        move |_, _| Host { state, headers }
    });
    // The body records its size on the first paint and lays out again.
    for _ in 0..3 {
        draw(cx);
    }
    (state, cx)
}

fn read_at(state: &Entity<DiffState>, cx: &mut VisualTestContext, file: &str, line: usize) {
    cx.update(|_, cx| {
        state.update(cx, |state, cx| {
            state.scroll_to_line(DiffLinePosition::new(file.to_string(), DiffSide::Modified, line), cx)
        })
    });
    draw(cx);
}

/// Where the header of the file at `ix` is drawn, if it is: there is one of each at most.
fn bar(cx: &mut VisualTestContext, ix: usize) -> Option<Bounds<Pixels>> {
    let all = cx.update(|window, cx| {
        window.render_frame(cx);
        window.find_all(("file-bar", ix))
    });
    assert!(all.len() <= 1, "the header of file {ix} is drawn {} times", all.len());
    all.first().map(|bar| bar.bounds())
}

/// The file and the kind of the row at the top of the list.
fn top_row(state: &Entity<DiffState>, cx: &mut VisualTestContext) -> (String, bool) {
    cx.update(|_, cx| {
        let state = state.read(cx);
        let row = &state.rows()[state.list().logical_scroll_top().item_ix];
        (state.files()[row.file()].path().to_string(), matches!(row, DisplayRow::File(_)))
    })
}

#[gpui::test]
fn the_header_of_the_file_being_read_stays_at_the_top(cx: &mut TestAppContext) {
    let (state, cx) = show(cx, true);
    // at the very top the first file's header is where its row is, drawn once
    let first = bar(cx, 0).expect("a.txt's header");
    assert_eq!(first.size.height, HEADER_HEIGHT);
    let stays = cx.debug_bounds("sticky-header").expect("the header that stays");
    assert_eq!((stays.top(), stays.bottom()), (first.top(), first.bottom()));
    // deep in b.txt: its header row is far above, and its header is still at the top of the view
    read_at(&state, cx, "b.txt", 120);
    assert_eq!(top_row(&state, cx), ("b.txt".to_string(), false));
    let pinned = bar(cx, 1).expect("b.txt's header");
    assert_eq!((pinned.top(), pinned.bottom(), pinned.left()), (first.top(), first.bottom(), first.left()));
    assert_eq!(pinned.size.width, first.size.width);
    assert_eq!(bar(cx, 0), None, "a.txt's header scrolled away with its file");
    // further down the same file: the same place
    read_at(&state, cx, "b.txt", 180);
    assert_eq!(bar(cx, 1), Some(pinned));
}

#[gpui::test]
fn the_next_files_header_pushes_it_away(cx: &mut TestAppContext) {
    let (state, cx) = show(cx, true);
    let top = bar(cx, 0).expect("a.txt's header").top();
    // c.txt's header 20 px under the top of the view: the last lines of b.txt are above it
    cx.update(|_, cx| {
        state.update(cx, |state, cx| {
            state.scroll_to_file("c.txt", cx);
            state.list().scroll_by(px(-20.));
            cx.notify();
        })
    });
    draw(cx);
    assert_eq!(top_row(&state, cx), ("b.txt".to_string(), false));
    let (leaving, arriving) = (bar(cx, 1).expect("b.txt's header"), bar(cx, 2).expect("c.txt's header"));
    // b.txt's header ends where c.txt's begins, 12 px of it cut off above
    assert_eq!((leaving.top(), leaving.bottom()), (top - px(12.), top + px(20.)));
    assert_eq!(arriving.top(), top + px(20.));
    assert_eq!(leaving.size.height, HEADER_HEIGHT);
    // c.txt reaches the top: its header is the one that stays, and b.txt's is gone
    cx.update(|_, cx| {
        state.update(cx, |state, cx| {
            state.list().scroll_by(px(25.));
            cx.notify();
        })
    });
    draw(cx);
    assert_eq!(bar(cx, 1), None);
    assert_eq!(bar(cx, 2).expect("c.txt's header").top(), top);
}

#[gpui::test]
fn the_header_that_stays_takes_the_press_and_collapsing_from_it_goes_to_the_file(cx: &mut TestAppContext) {
    let (state, cx) = show(cx, true);
    let events = Rc::new(RefCell::new(Vec::new()));
    let _watch = cx.update(|_, cx| {
        let events = events.clone();
        cx.subscribe(&state, move |_, event: &DiffEvent, _| events.borrow_mut().push(format!("{event:?}")))
    });
    read_at(&state, cx, "b.txt", 120);
    let pinned = bar(cx, 1).expect("b.txt's header");
    // a press on the header, where a line's number is under it: it selects nothing
    cx.simulate_click(point(pinned.left() + px(60.), pinned.center().y), Default::default());
    draw(cx);
    assert!(cx.update(|_, cx| state.read(cx).selected_lines().is_none()));
    assert!(events.borrow().is_empty(), "{:?}", events.borrow());
    // its collapse control, pressed while the file's lines are at the top: the view goes to the file's header
    cx.update(|window, cx| window.within("sticky-header").click("collapse-file", cx));
    draw(cx);
    assert!(cx.update(|_, cx| state.read(cx).is_file_collapsed("b.txt")));
    assert_eq!(*events.borrow(), ["FileCollapsed(\"b.txt\")"]);
    assert_eq!(top_row(&state, cx), ("b.txt".to_string(), true));
    assert_eq!(bar(cx, 1).expect("b.txt's header").top(), pinned.top());
    // the next file is right under it
    assert_eq!(bar(cx, 2).expect("c.txt's header").top(), pinned.bottom());
}

#[gpui::test]
fn without_headers_nothing_stays(cx: &mut TestAppContext) {
    let (state, cx) = show(cx, false);
    read_at(&state, cx, "b.txt", 120);
    assert_eq!(cx.debug_bounds("sticky-header"), None);
    assert_eq!(bar(cx, 1), None);
}
