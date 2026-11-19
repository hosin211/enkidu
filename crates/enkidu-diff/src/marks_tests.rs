//! enkidu: the marks of what differs in a file, as an editor shows them beside the line numbers (items
//! 10.1 and 10.2): which stretches a diff gives, and that the view draws them on their lines.

use gpui::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _, TestAppContext,
    VisualTestContext, Window, div, px,
};

use super::{
    Diff, DiffChangeKind, DiffChangeKind::*, DiffChangeMark, DiffFile, DiffState, decoration::ChangeMarks,
    document::fixture,
};

fn triples(file: &DiffFile) -> Vec<(DiffChangeKind, usize, usize)> {
    file.change_marks().into_iter().map(|mark| (mark.kind, mark.start, mark.end)).collect()
}

fn marks(before: &str, after: &str) -> Vec<(DiffChangeKind, usize, usize)> {
    triples(&fixture::modified("file.py", before, after))
}

#[test]
fn a_run_of_new_lines_is_added_of_replaced_lines_changed_and_of_lost_lines_removed() {
    let before = "a\nb\nc\nd\ne\nf\ng\n";
    // b is rewritten and one more line comes with it, two lines come new after d, f goes
    let after = "a\nB\nB2\nc\nd\nx\ny\ne\ng\n";
    assert_eq!(marks(before, after), [(Changed, 2, 3), (Added, 6, 7), (Removed, 8, 8)]);
}

#[test]
fn lines_removed_at_the_very_start_stand_before_the_first_line_and_at_the_end_after_the_last() {
    assert_eq!(marks("a\nb\nc\n", "b\nc\n"), [(Removed, 0, 0)]);
    assert_eq!(marks("a\nb\nc\n", "a\nb\n"), [(Removed, 2, 2)]);
}

#[test]
fn more_lines_replaced_by_fewer_is_one_changed_stretch() {
    assert_eq!(marks("a\nb\nc\nd\ne\n", "a\nX\ne\n"), [(Changed, 2, 2)]);
}

#[test]
fn a_new_file_is_added_from_its_first_line_to_its_last_and_a_file_shown_whole_has_no_marks() {
    let new = fixture::document(None, Some(("new.py", "one\ntwo\nthree\n")));
    assert_eq!(new.change_marks(), [DiffChangeMark { kind: Added, start: 1, end: 3 }]);
    assert!(DiffFile::unchanged("whole.py", "one\ntwo\n").change_marks().is_empty());
    assert!(fixture::modified("same.py", "one\n", "one\n").change_marks().is_empty());
}

#[test]
fn a_usual_patch_gives_its_marks_by_the_files_own_line_numbers() {
    // git's own patch for two changes far down a file, three lines of context around each
    let patch = "diff --git a/f.py b/f.py\n--- a/f.py\n+++ b/f.py\n@@ -40,7 +40,8 @@ def f():\n l40\n l41\n l42\n-old43\n+new43\n+new44\n l44\n l45\n l46\n@@ -90,6 +91,5 @@\n l90\n l91\n l92\n-gone93\n l94\n l95\n";
    let file = DiffFile::parse(patch).unwrap().remove(0);
    assert_eq!(triples(&file), [(Changed, 43, 44), (Removed, 93, 93)]);
}

#[test]
fn the_marks_of_a_file_are_found_by_line() {
    let marks = ChangeMarks::new([(
        "file.py".into(),
        vec![
            DiffChangeMark { kind: Removed, start: 0, end: 0 },
            DiffChangeMark { kind: Changed, start: 2, end: 3 },
            DiffChangeMark { kind: Added, start: 6, end: 7 },
            DiffChangeMark { kind: Removed, start: 8, end: 8 },
        ],
    )]);
    let at = |line| marks.at(&"file.py".into(), line);
    // the stripe of a line, and whether lines were removed right after it
    assert_eq!(at(1), (None, false));
    assert_eq!(at(2), (Some(Changed), false));
    assert_eq!(at(3), (Some(Changed), false));
    assert_eq!(at(4), (None, false));
    assert_eq!(at(6), (Some(Added), false));
    assert_eq!(at(7), (Some(Added), false));
    assert_eq!(at(8), (None, true));
    assert_eq!(at(9), (None, false));
    assert!(marks.removed_before_first(&"file.py".into()));
    assert_eq!(marks.at(&"other.py".into(), 2), (None, false));
    assert!(!marks.removed_before_first(&"other.py".into()));
    assert!(ChangeMarks::default().is_empty());
}

struct Host {
    state: Entity<DiffState>,
    marks: Vec<DiffChangeMark>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(600.)).h(px(400.)).child(
            Diff::new(&self.state)
                .size_full()
                .header_visible(false)
                .change_marks([("file.py".into(), self.marks.clone())]),
        )
    }
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        _ = window.draw(cx);
    });
}

/// A file shown whole, as an opened file is, with the marks its diff gave: each marked line has its stripe, in the
/// lane beside its number, as tall as the row; a line without a mark has none.
#[gpui::test]
fn the_view_draws_a_stripe_beside_each_marked_line(cx: &mut TestAppContext) {
    cx.update(crate::init_for_tests);
    let text = "one\ntwo\nthree\nfour\nfive\nsix\n";
    let state = cx.new(|cx| DiffState::new([DiffFile::unchanged("file.py", text)], cx));
    let marks = vec![
        DiffChangeMark { kind: Changed, start: 2, end: 3 },
        DiffChangeMark { kind: Removed, start: 4, end: 4 },
        DiffChangeMark { kind: Added, start: 6, end: 6 },
    ];
    let (_, cx) = cx.add_window_view({
        let state = state.clone();
        move |_, _| Host { state, marks }
    });
    for _ in 0..3 {
        draw(cx);
    }
    let mut found =
        |what: &str, line: usize| cx.debug_bounds(Box::leak(format!("change-{what}-{line}").into_boxed_str()));
    assert!(found("mark", 1).is_none() && found("mark", 4).is_none() && found("mark", 5).is_none());
    let two = found("mark", 2).expect("line 2");
    let three = found("mark", 3).expect("line 3");
    let six = found("mark", 6).expect("line 6");
    // one under the other, each as tall as its row and 4 px wide
    assert_eq!(two.left(), three.left());
    assert_eq!(two.bottom(), three.top());
    assert_eq!((two.size.width, six.size.width), (px(4.), px(4.)));
    assert!(two.size.height > px(10.));
    // where lines went, a small mark at the foot of the line they stood after
    let after_four = found("gone", 4).expect("lines were removed after line 4");
    assert!(found("gone", 2).is_none() && found("gone", 6).is_none());
    assert!(after_four.top() > three.bottom() && after_four.size.height < px(4.));
    assert_eq!(after_four.left(), two.left());
}
