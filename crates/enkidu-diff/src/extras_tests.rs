//! enkidu: item 3.2d, the view's part: a file without lines says what the application says of it, and a
//! file shown side by side names its two sides.

use gpui::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _,
    TestAppContext, VisualTestContext, Window, div, px,
};
use gpui_kit::test::TestWindowExt as _;

use super::{Diff, DiffFile, DiffMode, DiffState, document::fixture, state::DisplayRow};

struct Host {
    state: Entity<DiffState>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(600.)).h(px(400.)).child(
            Diff::new(&self.state)
                .render_notice(|file, _, _| {
                    (file.path().as_str() == "big.csv")
                        .then(|| div().debug_selector(|| "said".into()).child("This change is too large to show here."))
                })
                .size_full(),
        )
    }
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
    for _ in 0..3 {
        cx.run_until_parked();
        cx.update(|window, cx| {
            _ = window.draw(cx);
        });
    }
    (state, cx)
}

/// The kinds of the rows, in order: h(eader), n(otice), s(ides), l(ine), f(old), o(ther).
fn kinds(state: &Entity<DiffState>, cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| {
        state
            .read(cx)
            .rows()
            .iter()
            .map(|row| match row {
                DisplayRow::File(_) => 'h',
                DisplayRow::Notice(_) => 'n',
                DisplayRow::Sides(_) => 's',
                DisplayRow::Code { .. } => 'l',
                DisplayRow::Fold { .. } => 'f',
                _ => 'o',
            })
            .collect()
    })
}

#[gpui::test]
fn a_file_without_lines_says_what_the_application_says(cx: &mut TestAppContext) {
    let files = [
        DiffFile::without_lines("big.csv"),
        DiffFile::without_lines("not-read-yet.py"),
        fixture::modified("a.txt", "one\n", "two\n"),
    ];
    let (state, cx) = show(cx, |cx| DiffState::new(files, cx));
    // each has its header and one row under it; the third has its two lines
    assert_eq!(kinds(&state, cx), "hnhnhll");
    assert!(cx.debug_bounds("said").is_some(), "the application's words for big.csv");
    let files = cx.update(|_, cx| state.read(cx).files().iter().map(|f| f.path().to_string()).collect::<Vec<_>>());
    assert_eq!(files, ["big.csv", "not-read-yet.py", "a.txt"]);
    // nothing counted for a file whose lines aren't known
    cx.update(|_, cx| {
        let first = &state.read(cx).files()[0];
        assert_eq!((first.additions(), first.deletions(), first.is_binary()), (0, 0, false));
    });
}

#[gpui::test]
fn a_file_shown_side_by_side_names_its_sides(cx: &mut TestAppContext) {
    let files = [
        fixture::modified("changed.txt", "one\n", "two\n"),
        fixture::document(None, Some(("new.txt", "fresh\n"))),
        fixture::document(Some(("gone.txt", "old\n")), None),
        DiffFile::without_lines("big.csv"),
    ];
    let (state, cx) = show(cx, |cx| DiffState::new(files, cx).with_side_names(true));
    // in one column there are no sides to name
    assert_eq!(kinds(&state, cx), "hllhlhlhn");
    cx.update(|_, cx| state.update(cx, |state, cx| state.set_mode(DiffMode::Split, cx)));
    assert_eq!(kinds(&state, cx), "hslhslhslhn");
    let said = |cx: &mut VisualTestContext, id: &'static str| {
        cx.update(|window, cx| {
            window.render_frame(cx);
            window.find_all(id).iter().map(|side| side.label().unwrap_or_default().to_string()).collect::<Vec<_>>()
        })
    };
    assert_eq!(said(cx, "split-before"), ["Before", "Before: a new file", "Before"]);
    assert_eq!(said(cx, "split-after"), ["After", "After", "After: deleted"]);
    // back in one column they are gone
    cx.update(|_, cx| state.update(cx, |state, cx| state.set_mode(DiffMode::Unified, cx)));
    assert_eq!(kinds(&state, cx), "hllhlhlhn");
    assert_eq!(said(cx, "split-before"), Vec::<String>::new());
}

#[gpui::test]
fn the_file_being_read_gets_its_colors_first(cx: &mut TestAppContext) {
    use std::sync::{Arc, Mutex};
    let line = |file: &str| format!("{file} = 1\n");
    let files = ["a.py", "b.py", "c.py", "d.py"]
        .map(|name| fixture::modified(name, &line(name), &line(name).replace('1', "2")));
    let asked = Arc::new(Mutex::new(Vec::<String>::new()));
    let (state, cx) = show(cx, {
        let asked = asked.clone();
        move |cx| {
            DiffState::new(files, cx).with_syntax_highlighter(move |_, text| {
                // each side of each file: the file's name is the first word of its one line
                asked.lock().unwrap().push(text.split(' ').next().unwrap_or_default().to_string());
                Vec::new()
            })
        }
    });
    // from the top of the list: the files in order
    assert_eq!(
        std::mem::take(&mut *asked.lock().unwrap()),
        ["a.py", "a.py", "b.py", "b.py", "c.py", "c.py", "d.py", "d.py"]
    );
    // reading c.py when everything is prepared again: c.py first, then what is after it, then what is before
    cx.update(|_, cx| {
        state.update(cx, |state, cx| {
            state.scroll_to_file("c.py", cx);
            state.set_syntax_highlighter(
                {
                    let asked = asked.clone();
                    move |_, text| {
                        asked.lock().unwrap().push(text.split(' ').next().unwrap_or_default().to_string());
                        Vec::new()
                    }
                },
                cx,
            );
        })
    });
    cx.run_until_parked();
    assert_eq!(*asked.lock().unwrap(), ["c.py", "c.py", "d.py", "d.py", "a.py", "a.py", "b.py", "b.py"]);
}
