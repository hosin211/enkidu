//! enkidu: the view's measures stand on a 4 px grid, whatever a rem is: text starts 16 px from the view's edge, the
//! line numbers of a diff end at 48 and 88, the sign of a changed line stands in the next 24, the code starts at
//! 112; side by side, each half has one column of numbers. (The original sized all of it in rems.)

use gpui::{
    AppContext as _, Bounds, Context, ElementId, Entity, IntoElement, ParentElement as _, Pixels, Render, Styled as _,
    TestAppContext, VisualTestContext, Window, div, px,
};

use gpui_kit::test::TestWindowExt as _;

use super::{Diff, DiffMode, DiffState, HEADER_HEIGHT, document::fixture, number_width};

const WIDTH: f32 = 640.;

struct Host {
    state: Entity<DiffState>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        // long lines wrap, as they do in the app: a line of code is then as wide as its row lets it be
        div().w(px(WIDTH)).h(px(240.)).child(Diff::new(&self.state).soft_wrap(true).border_0().size_full())
    }
}

/// A file of twelve lines whose line 2 differs, shown whole, in a view 640 px wide without an edge of its own.
fn show(cx: &mut TestAppContext, mode: DiffMode) -> &mut VisualTestContext {
    cx.update(crate::init_for_tests);
    let original: String = (1..=12).map(|n| format!("line {n}\n")).collect();
    let modified = original.replace("line 2\n", "changed 2\n");
    let file = fixture::modified("a.txt", &original, &modified);
    let state = cx.new(|cx| DiffState::new([file], cx).with_context_lines(None).with_mode(mode));
    let (_, cx) = cx.add_window_view(move |_, _| Host { state });
    // The body records its size on the first paint and lays out again.
    for _ in 0..3 {
        cx.run_until_parked();
        cx.update(|window, cx| {
            _ = window.draw(cx);
        });
    }
    cx
}

fn is(id: &ElementId, name: &str) -> bool {
    match id {
        ElementId::Name(own) | ElementId::NamedInteger(own, _) => own.as_ref() == name,
        _ => false,
    }
}

/// Where the pieces named `name` stand that are inside a piece named `inside`, in the first row that has one.
fn in_first_row(cx: &mut VisualTestContext, inside: &str, name: &str) -> Vec<Bounds<Pixels>> {
    let mut all: Vec<Bounds<Pixels>> = cx.update(|window, cx| {
        window.render_frame(cx);
        gpui_kit::base::test_support::snapshots(window)
            .into_iter()
            .filter(|piece| piece.visible() && piece.path().last().is_some_and(|id| is(id, name)))
            .filter(|piece| piece.path().iter().any(|id| is(id, inside)))
            .map(|piece| piece.bounds())
            .collect()
    });
    all.sort_by(|a, b| (a.top(), a.left()).partial_cmp(&(b.top(), b.left())).unwrap());
    let top = all.first().map(|first| first.top());
    all.into_iter().filter(|piece| Some(piece.top()) == top).collect()
}

#[gpui::test]
fn the_numbers_of_a_diff_end_at_48_and_88_and_the_code_starts_at_112(cx: &mut TestAppContext) {
    let cx = show(cx, DiffMode::Unified);
    // the first line is unchanged: it has its number before and after, each in a column of its own
    let old = in_first_row(cx, "old-gutter", "line");
    let new = in_first_row(cx, "new-gutter", "line");
    assert_eq!((old.len(), new.len()), (1, 1));
    assert_eq!((old[0].left(), old[0].right()), (px(8.), px(48.)), "the first column of numbers");
    assert_eq!((new[0].left(), new[0].right()), (px(48.), px(88.)), "the second");
    // the sign's 24 px, then the code, which ends 16 px before the view does
    let code = in_first_row(cx, "line-row", "source");
    assert_eq!((code[0].left(), code[0].right()), (px(112.), px(WIDTH - 16.)));
    // the file's header is a bar inside a pane, and its arrow stands where text starts
    let bar = cx.update(|window, cx| {
        window.render_frame(cx);
        (window.find(("file-bar", 0usize)).bounds(), window.find("collapse-file").bounds())
    });
    assert_eq!(bar.0.size.height, HEADER_HEIGHT);
    assert_eq!(HEADER_HEIGHT, px(32.));
    assert_eq!((bar.1.left(), bar.1.size.width, bar.1.size.height), (px(16.), px(16.), px(16.)));
}

#[gpui::test]
fn side_by_side_each_half_has_its_numbers_to_48_and_its_code_from_72(cx: &mut TestAppContext) {
    let cx = show(cx, DiffMode::Split);
    let half = px(WIDTH / 2.);
    let (old, new) = (in_first_row(cx, "old-gutter", "line"), in_first_row(cx, "new-gutter", "line"));
    assert_eq!((old[0].left(), old[0].right()), (px(8.), px(48.)));
    assert_eq!((new[0].left(), new[0].right()), (half + px(8.), half + px(48.)));
    let code = in_first_row(cx, "line-row", "source");
    assert_eq!(code.len(), 2, "one line of code in each half");
    assert_eq!((code[0].left(), code[1].left()), (px(72.), half + px(72.)));
    // each half's code ends 16 px before the half does (the left half has the line between them as its last pixel)
    assert_eq!((code[0].right(), code[1].right()), (half - px(17.), px(WIDTH - 16.)));
}

#[test]
fn a_column_of_numbers_is_40_px_until_a_longer_number_needs_more_in_steps_of_4() {
    // code at 12 px: four digits fit in 40 px, five need 48
    assert_eq!([1, 2, 3, 4].map(|digits| number_width(px(12.), digits)), [px(40.); 4]);
    assert_eq!(number_width(px(12.), 5), px(48.));
    assert_eq!(number_width(px(12.), 6), px(56.));
    // larger code needs it sooner
    assert_eq!(number_width(px(18.), 3), px(44.));
    for digits in 1..9 {
        for size in [9., 12., 13., 16., 22.] {
            let width = f32::from(number_width(px(size), digits));
            assert_eq!(width % 4., 0., "{digits} digits at {size} px");
        }
    }
}
