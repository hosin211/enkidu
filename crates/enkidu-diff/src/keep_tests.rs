//! enkidu: item 3.2b. A new list of files keeps the place on screen, what was unfolded, and what was prepared
//! for the files that didn't change. (The original reset all three; see `DiffState::set_files`.)

use std::sync::{Arc, Mutex};

use gpui::{
    AppContext as _, Context, Entity, IntoElement, ListOffset, ParentElement as _, Pixels, Render, Styled as _,
    TestAppContext, VisualTestContext, Window, div, px,
};

use super::{
    Diff, DiffFile, DiffLinePosition, DiffSide, DiffState,
    document::fixture,
    state::{DisplayRow, FoldExpansion},
};

struct Host {
    state: Entity<DiffState>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(400.)).h(px(200.)).child(Diff::new(&self.state).size_full())
    }
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        _ = window.draw(cx);
    });
}

/// A file of `count` lines; the lines in `changed` differ from the original.
fn file(name: &str, count: usize, changed: &[usize]) -> DiffFile {
    with_added(name, count, changed, 0, 0)
}

/// The same, with `added` new lines put in before line `at`.
fn with_added(name: &str, count: usize, changed: &[usize], at: usize, added: usize) -> DiffFile {
    let original: String = (1..=count).map(|n| format!("line {n}\n")).collect();
    let mut modified = String::new();
    for n in 1..=count {
        if n == at {
            modified.extend((1..=added).map(|k| format!("added {k}\n")));
        }
        modified.push_str(&if changed.contains(&n) { format!("changed {n}\n") } else { format!("line {n}\n") });
    }
    fixture::modified(name, &original, &modified)
}

fn show(
    cx: &mut TestAppContext,
    build: impl FnOnce(&mut Context<DiffState>) -> DiffState,
) -> (Entity<DiffState>, &mut VisualTestContext) {
    cx.update(crate::init_for_tests);
    let state = cx.new(build);
    let (_, cx) = cx.add_window_view({
        let state = state.clone();
        move |_, _| Host { state }
    });
    // The body records its size on the first paint and lays out again.
    for _ in 0..3 {
        draw(cx);
    }
    (state, cx)
}

/// What is at the top of the view: the file, the kind of row, the line numbers it shows (before, after), and how far
/// the row is scrolled past the top.
#[derive(Debug, PartialEq)]
struct Top {
    file: String,
    row: &'static str,
    lines: (Option<usize>, Option<usize>),
    offset: Pixels,
}

fn top(state: &Entity<DiffState>, cx: &mut VisualTestContext) -> (Top, usize) {
    cx.update(|_, cx| {
        let state = state.read(cx);
        let at = state.list().logical_scroll_top();
        let row = &state.rows()[at.item_ix];
        let kind = match row {
            DisplayRow::File(_) => "header",
            DisplayRow::Code { .. } => "line",
            DisplayRow::Fold { .. } => "fold",
            _ => "other",
        };
        let lines = state.row_lines(at.item_ix).unwrap_or((None, None));
        let file = state.files()[row.file()].path().to_string();
        (Top { file, row: kind, lines, offset: at.offset_in_item }, at.item_ix)
    })
}

fn reading(file: &str, line: usize, offset: f32) -> Top {
    Top { file: file.into(), row: "line", lines: (Some(line), Some(line)), offset: px(offset) }
}

/// Scrolls so that `line` of `file` is at the top, `offset` pixels past it.
fn read_at(state: &Entity<DiffState>, cx: &mut VisualTestContext, file: &str, line: usize, offset: f32) {
    cx.update(|_, cx| {
        state.update(cx, |state, cx| {
            state.scroll_to_line(DiffLinePosition::new(file.to_string(), DiffSide::Modified, line), cx);
            let item_ix = state.list().logical_scroll_top().item_ix;
            state.list().scroll_to(ListOffset { item_ix, offset_in_item: px(offset) });
        })
    });
    draw(cx);
}

fn set_files(state: &Entity<DiffState>, cx: &mut VisualTestContext, files: Vec<DiffFile>) {
    cx.update(|_, cx| state.update(cx, |state, cx| state.set_files(files, cx)));
}

/// Whether `line` of the original of `file` has a row of its own (it isn't inside a fold).
fn shown(state: &Entity<DiffState>, cx: &mut VisualTestContext, file: &str, line: usize) -> bool {
    cx.update(|_, cx| {
        let state = state.read(cx);
        let position = DiffLinePosition::new(file.to_string(), DiffSide::Original, line);
        state.source_row(&position).is_some_and(|row| matches!(state.rows()[row], DisplayRow::Code { .. }))
    })
}

#[gpui::test]
fn another_files_change_leaves_the_line_being_read_at_the_top(cx: &mut TestAppContext) {
    let (a, b, c) = (file("a.txt", 30, &[15]), file("b.txt", 200, &[100]), file("c.txt", 20, &[5]));
    let (state, cx) = show(cx, |cx| DiffState::new([a, b.clone(), c.clone()], cx).with_context_lines(None));
    read_at(&state, cx, "b.txt", 120, 7.);
    let (before, row_before) = top(&state, cx);
    assert_eq!(before, reading("b.txt", 120, 7.));
    // Claude adds five lines to a.txt, which is above the place
    set_files(&state, cx, vec![with_added("a.txt", 30, &[15], 3, 5), b.clone(), c.clone()]);
    let (after, row_after) = top(&state, cx);
    assert_eq!(after, before, "the original went back to the top of the first file");
    assert_eq!(row_after, row_before + 5, "the same line is five rows further down the list");
    // and it stays there once the view is laid out and drawn again
    draw(cx);
    assert_eq!(top(&state, cx).0, before);
    // a file added before it, and one after
    let (new, last) = (file("a2.txt", 12, &[2]), file("d.txt", 9, &[9]));
    set_files(&state, cx, vec![with_added("a.txt", 30, &[15], 3, 5), new, b, c, last]);
    draw(cx);
    assert_eq!(top(&state, cx).0, before);
}

#[gpui::test]
fn the_file_being_read_changing_above_the_place_keeps_the_same_line_at_the_top(cx: &mut TestAppContext) {
    let (a, b) = (file("a.txt", 30, &[15]), file("b.txt", 200, &[100]));
    let (state, cx) = show(cx, |cx| DiffState::new([a.clone(), b], cx).with_context_lines(None));
    read_at(&state, cx, "b.txt", 120, 7.);
    // four lines go in near the top of b.txt: what was line 120 is line 124 now, and it is still the original's 120
    set_files(&state, cx, vec![a.clone(), with_added("b.txt", 200, &[100], 10, 4)]);
    draw(cx);
    let moved = Top { file: "b.txt".into(), row: "line", lines: (Some(120), Some(124)), offset: px(7.) };
    assert_eq!(top(&state, cx).0, moved);
    // the line being read is itself changed: its old text is still the row at the top (one column shows it first)
    set_files(&state, cx, vec![a, with_added("b.txt", 200, &[100, 120], 10, 4)]);
    draw(cx);
    let changed = Top { file: "b.txt".into(), row: "line", lines: (Some(120), None), offset: px(7.) };
    assert_eq!(top(&state, cx).0, changed);
}

#[gpui::test]
fn a_header_at_the_top_stays_and_a_file_that_is_gone_gives_its_place_to_the_next(cx: &mut TestAppContext) {
    let (a, b, c) = (file("a.txt", 30, &[15]), file("b.txt", 200, &[100]), file("c.txt", 20, &[5]));
    let (state, cx) = show(cx, |cx| DiffState::new([a.clone(), b.clone(), c.clone()], cx).with_context_lines(None));
    cx.update(|_, cx| state.update(cx, |state, cx| state.scroll_to_file("c.txt", cx)));
    draw(cx);
    let header = |file: &str| Top { file: file.into(), row: "header", lines: (Some(1), Some(1)), offset: px(0.) };
    assert_eq!(top(&state, cx).0, header("c.txt"));
    set_files(&state, cx, vec![with_added("a.txt", 30, &[15], 3, 5), b.clone(), c.clone()]);
    draw(cx);
    assert_eq!(top(&state, cx).0, header("c.txt"));
    // reading b.txt when its changes are committed away: the file that took its place is at the top
    read_at(&state, cx, "b.txt", 120, 7.);
    set_files(&state, cx, vec![a.clone(), c]);
    draw(cx);
    assert_eq!(top(&state, cx).0, header("c.txt"));
    // nothing left but one file: the view is on it, not past the end
    set_files(&state, cx, vec![a]);
    draw(cx);
    assert_eq!(top(&state, cx).0.file, "a.txt");
    // and no files at all, then files again
    set_files(&state, cx, Vec::new());
    draw(cx);
    set_files(&state, cx, vec![b]);
    draw(cx);
    assert_eq!(top(&state, cx).0, header("b.txt"));
}

#[gpui::test]
fn what_was_unfolded_stays_unfolded(cx: &mut TestAppContext) {
    let (a, b, c) = (file("a.txt", 30, &[15]), file("b.txt", 200, &[100]), file("c.txt", 20, &[5]));
    let (state, cx) = show(cx, |cx| DiffState::new([a, b.clone(), c.clone()], cx));
    // three lines around the change show; the rest of b.txt is folded
    assert!(shown(&state, cx, "b.txt", 98) && !shown(&state, cx, "b.txt", 50) && !shown(&state, cx, "b.txt", 180));
    cx.update(|_, cx| {
        state.update(cx, |state, cx| {
            let fold = state
                .rows()
                .iter()
                .find_map(|row| match row {
                    DisplayRow::Fold { file: 1, pairs } => Some(pairs.clone()),
                    _ => None,
                })
                .expect("b.txt's first fold");
            state.expand_fold(1, fold, FoldExpansion::All, cx);
        })
    });
    assert!(shown(&state, cx, "b.txt", 50));
    // another file changes: b.txt is the very same, and so is what was unfolded in it
    let a2 = with_added("a.txt", 30, &[15], 3, 5);
    set_files(&state, cx, vec![a2.clone(), b, c.clone()]);
    assert!(shown(&state, cx, "b.txt", 50) && shown(&state, cx, "b.txt", 1) && !shown(&state, cx, "b.txt", 180));
    // b.txt itself changes further down: its unfolded part is found again by the original's line numbers
    set_files(&state, cx, vec![a2.clone(), file("b.txt", 200, &[100, 150]), c.clone()]);
    assert!(shown(&state, cx, "b.txt", 50) && shown(&state, cx, "b.txt", 1), "folded again");
    assert!(shown(&state, cx, "b.txt", 150) && !shown(&state, cx, "b.txt", 180) && !shown(&state, cx, "b.txt", 125));
    // lines put in above it move every row of the file: still found
    set_files(&state, cx, vec![a2, with_added("b.txt", 200, &[100, 150], 2, 6), c]);
    assert!(shown(&state, cx, "b.txt", 50) && shown(&state, cx, "b.txt", 96) && !shown(&state, cx, "b.txt", 180));
}

#[gpui::test]
fn a_file_that_did_not_change_keeps_what_was_prepared_for_it(cx: &mut TestAppContext) {
    // files of a language (a .txt is shown without colors, and the highlighter isn't asked)
    let (a, b, c) = (file("a.py", 30, &[15]), file("b.py", 200, &[100]), file("c.py", 20, &[5]));
    // the application's highlighter, noting the first line of every text it is given
    let asked = Arc::new(Mutex::new(Vec::<String>::new()));
    let (state, cx) = show(cx, {
        let (asked, files) = (asked.clone(), [a, b.clone(), c.clone()]);
        move |cx| {
            DiffState::new(files, cx).with_syntax_highlighter(move |_, text| {
                asked.lock().unwrap().push(text.lines().next().unwrap_or_default().to_owned());
                Vec::new()
            })
        }
    });
    let prepared = |cx: &mut VisualTestContext| cx.update(|_, cx| state.read(cx).presentations().to_vec());
    let first = prepared(cx);
    assert!(first.iter().all(Option::is_some));
    // each side of each file once
    assert_eq!(std::mem::take(&mut *asked.lock().unwrap()).len(), 6);
    // a.txt changes: only a.txt is prepared again, both its sides
    set_files(&state, cx, vec![with_added("a.py", 30, &[15], 1, 2), b, c]);
    assert!(prepared(cx)[0].is_none(), "not prepared yet: that is the background's work");
    draw(cx);
    let second = prepared(cx);
    assert_eq!(*asked.lock().unwrap(), ["line 1", "added 1"]);
    assert!(second.iter().all(Option::is_some));
    for unchanged in [1, 2] {
        assert!(Arc::ptr_eq(first[unchanged].as_ref().unwrap(), second[unchanged].as_ref().unwrap()));
    }
    assert!(!Arc::ptr_eq(first[0].as_ref().unwrap(), second[0].as_ref().unwrap()));
}

/// enkidu: item 3.7. A line someone was sent to comes to the middle of the view, not to its top edge, in a
/// file shown whole (an opened file); a line near the start stays where the start of the file puts it.
#[gpui::test]
fn a_line_someone_is_sent_to_comes_to_the_middle_of_the_view(cx: &mut TestAppContext) {
    let text: String = (1..=300).map(|n| format!("line {n}\n")).collect();
    let (state, cx) = show(cx, |cx| DiffState::new([DiffFile::unchanged("notes.txt", &text)], cx));
    let place = |state: &Entity<DiffState>, cx: &mut VisualTestContext, line: usize| {
        cx.update(|_, cx| {
            state.update(cx, |state, cx| {
                state.scroll_to_line_centered(DiffLinePosition::new("notes.txt", DiffSide::Modified, line), cx)
            })
        });
        draw(cx);
        cx.update(|_, cx| {
            let state = state.read(cx);
            let row = state.source_row(&DiffLinePosition::new("notes.txt", DiffSide::Modified, line)).expect("a row");
            let (view, at) = (state.list().viewport_bounds(), state.list().bounds_for_item(row).expect("in view"));
            ((at.top() - view.top()) / view.size.height, (at.bottom() - view.top()) / view.size.height)
        })
    };
    // line 150: its row is around the middle of the view, with lines above and below it
    let (top_edge, bottom_edge) = place(&state, cx, 150);
    assert!(top_edge > 0.25 && bottom_edge < 0.75, "not in the middle: {top_edge} to {bottom_edge}");
    let (above, _) = top(&state, cx);
    assert!(above.lines.1.is_some_and(|line| line < 150), "nothing above it: {above:?}");
    // line 2: the file starts at the top, and the line is in view under its first line
    let (top_edge, bottom_edge) = place(&state, cx, 2);
    assert!(top_edge >= 0. && bottom_edge <= 1., "not in view: {top_edge} to {bottom_edge}");
    assert_eq!(top(&state, cx).1, 0, "the file doesn't start at the top");
    // a line the file doesn't have: nothing moves
    cx.update(|_, cx| {
        state.update(cx, |state, cx| {
            state.scroll_to_line_centered(DiffLinePosition::new("notes.txt", DiffSide::Modified, 900), cx)
        })
    });
    draw(cx);
    assert_eq!(top(&state, cx).1, 0);
}

/// enkidu: item 3.7. A file opened at a line is centered on it although the view had no height yet when it was asked.
#[gpui::test]
fn a_line_asked_for_before_the_view_was_laid_out_is_centered_once_it_is(cx: &mut TestAppContext) {
    let text: String = (1..=300).map(|n| format!("line {n}\n")).collect();
    cx.update(crate::init_for_tests);
    let state = cx.new(|cx| {
        let mut state = DiffState::new([DiffFile::unchanged("notes.txt", &text)], cx);
        state.scroll_to_line_centered(DiffLinePosition::new("notes.txt", DiffSide::Modified, 150), cx);
        state
    });
    let (_, cx) = cx.add_window_view({
        let state = state.clone();
        move |_, _| Host { state }
    });
    for _ in 0..4 {
        draw(cx);
    }
    let (top_edge, bottom_edge) = cx.update(|_, cx| {
        let state = state.read(cx);
        let row = state.source_row(&DiffLinePosition::new("notes.txt", DiffSide::Modified, 150)).expect("a row");
        let (view, at) = (state.list().viewport_bounds(), state.list().bounds_for_item(row).expect("in view"));
        ((at.top() - view.top()) / view.size.height, (at.bottom() - view.top()) / view.size.height)
    });
    assert!(top_edge > 0.25 && bottom_edge < 0.75, "not in the middle: {top_edge} to {bottom_edge}");
}

/// enkidu: item 3.16. A file that comes or goes moves every file after it by one place in the list; the rows
/// after it are still the same rows, and only the file's own rows are given to the list again.
#[test]
fn a_file_that_comes_or_goes_leaves_the_rows_of_the_files_after_it() {
    use crate::state::{DisplayRow, changed_middle};
    let code = |file: usize, line: usize| DisplayRow::Code {
        file,
        original: Some(line),
        modified: Some(line),
        changed: false,
    };
    // a file's rows: its header, the names of its sides, its lines
    let file = |at: usize, lines: usize| {
        let mut rows = vec![DisplayRow::File(at), DisplayRow::Sides(at)];
        rows.extend((0..lines).map(|line| code(at, line)));
        rows
    };
    // a list of files, each with so many lines
    let list = |lines: &[usize]| lines.iter().enumerate().flat_map(|(at, lines)| file(at, *lines)).collect::<Vec<_>>();

    // three files of 2, 3 and 4 lines: 15 rows
    let old = list(&[2, 3, 4]);
    assert_eq!(old.len(), 15);
    // a file of one line comes first: three rows are new, and the twelve rows after them are kept though each of
    // their files moved one place down the list
    assert_eq!(changed_middle(&old, &list(&[1, 2, 3, 4]), 3, 4), (3, 12));
    // one comes in the middle: the same three rows
    assert_eq!(changed_middle(&old, &list(&[2, 1, 3, 4]), 3, 4), (7, 8));
    // the first file goes: its four rows go, the other eleven are kept
    assert_eq!(changed_middle(&old, &list(&[3, 4]), 3, 2), (4, 7));
    // nothing changed: all of it is kept (the start counts first)
    assert_eq!(changed_middle(&old, &list(&[2, 3, 4]), 3, 3), (15, 0));
    // a file got one more line: one row is new
    assert_eq!(changed_middle(&old, &list(&[2, 4, 4]), 3, 3), (9, 6));
    // counted from the start alone, a new first file would leave nothing at the end
    let (new, from_the_start) = (list(&[1, 2, 3, 4]), |a: &DisplayRow, b: &DisplayRow| a == b);
    assert_eq!(old.iter().rev().zip(new.iter().rev()).take_while(|(a, b)| from_the_start(a, b)).count(), 0);
    // from an empty list, and to one
    assert_eq!(changed_middle(&[], &list(&[2, 3]), 0, 2), (0, 0));
    assert_eq!(changed_middle(&old, &[], 3, 0), (0, 0));
}
