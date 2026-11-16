//! enkidu: links in the code (item 10.10): with ⌘ held, the name under the pointer is told to the
//! application; a name it calls a link is drawn as one, and a click on it with ⌘ held is told and selects nothing.

use std::{cell::RefCell, rc::Rc};

use gpui::{
    AppContext as _, Context, Entity, IntoElement, Modifiers, MouseButton, ParentElement as _, Pixels, Point, Render,
    Styled as _, TestAppContext, VisualTestContext, Window, div, point, px,
};

use super::{Diff, DiffEvent, DiffFile, DiffLinePosition, DiffName, DiffSide, DiffState};

struct Host {
    state: Entity<DiffState>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(700.)).h(px(300.)).child(Diff::new(&self.state).size_full().header_visible(false))
    }
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        _ = window.draw(cx);
    });
}

/// A point of the window on the name that starts at byte `start` of a line, a little inside it.
fn on_name(state: &Entity<DiffState>, cx: &mut VisualTestContext, line: usize, start: usize) -> Point<Pixels> {
    cx.update(|_, cx| {
        let geometry = state.read(cx).geometry().clone();
        let geometry = geometry.borrow();
        let (_, bounds) = geometry.painted_lines().into_iter().find(|(number, _)| *number == line).expect("the line");
        let y = bounds.center().y;
        let mut x = bounds.left() + px(1.);
        while x < bounds.right() {
            if geometry.name_at(point(x, y)).is_some_and(|name| name.range.start == start) {
                return point(x + px(1.), y);
            }
            x += px(1.);
        }
        panic!("no name starts at byte {start} of line {line}");
    })
}

fn at(line: usize) -> DiffLinePosition {
    DiffLinePosition::new("file.py", DiffSide::Modified, line)
}

#[gpui::test]
fn the_name_under_the_pointer_is_told_while_cmd_is_held_and_a_link_is_clicked(cx: &mut TestAppContext) {
    cx.update(crate::init_for_tests);
    let text = "alpha = beta(gamma)\n    delta.epsilon()\n<row-item />\n";
    let state = cx.new(|cx| DiffState::new([DiffFile::unchanged("file.py", text)], cx));
    let told: Rc<RefCell<Vec<DiffEvent>>> = Rc::default();
    cx.update(|cx| {
        let told = told.clone();
        cx.subscribe(&state, move |_, event: &DiffEvent, _| told.borrow_mut().push(event.clone())).detach();
    });
    let (_, cx) = cx.add_window_view({
        let state = state.clone();
        move |_, _| Host { state }
    });
    for _ in 0..3 {
        draw(cx);
    }
    let cmd = Modifiers::command();
    let take = |told: &Rc<RefCell<Vec<DiffEvent>>>| -> Vec<String> {
        told.borrow_mut()
            .drain(..)
            .map(|event| match event {
                DiffEvent::NamePointed(Some(name)) => format!("pointed {}:{:?}", name.position.line(), name.range),
                DiffEvent::NamePointed(None) => "pointed nothing".into(),
                DiffEvent::NameClicked(name) => format!("clicked {}:{:?}", name.position.line(), name.range),
                other => format!("{other:?}"),
            })
            .collect()
    };
    let beta = on_name(&state, cx, 1, 8);
    let gamma = on_name(&state, cx, 1, 13);
    let epsilon = on_name(&state, cx, 2, 10);

    // without ⌘ a move of the pointer tells nothing
    cx.simulate_mouse_move(beta, None, Modifiers::none());
    assert_eq!(take(&told), Vec::<String>::new());
    // with ⌘: the name, once; moving inside it tells nothing more
    cx.simulate_mouse_move(beta, None, cmd);
    assert_eq!(take(&told), ["pointed 1:8..12"]);
    cx.simulate_mouse_move(beta + point(px(3.), px(0.)), None, cmd);
    assert_eq!(take(&told), Vec::<String>::new());
    // another name, on another line
    cx.simulate_mouse_move(epsilon, None, cmd);
    assert_eq!(take(&told), ["pointed 2:10..17"]);
    // the bracket after it is no name
    cx.simulate_mouse_move(epsilon + point(px(60.), px(0.)), None, cmd);
    assert_eq!(take(&told), ["pointed nothing"]);

    // the application says a name is a link: it is one while the pointer is on it
    cx.simulate_mouse_move(beta, None, cmd);
    assert_eq!(take(&told), ["pointed 1:8..12"]);
    let name = DiffName { position: at(1), range: 8..12, column: 9 };
    cx.update(|_, cx| state.update(cx, |state, cx| state.set_link(Some(name.clone()), cx)));
    assert_eq!(cx.update(|_, cx| state.read(cx).link().cloned()), Some(name.clone()));
    // an answer for a name the pointer isn't on is dropped
    let other = DiffName { position: at(1), range: 13..18, column: 14 };
    cx.update(|_, cx| state.update(cx, |state, cx| state.set_link(Some(other), cx)));
    assert_eq!(cx.update(|_, cx| state.read(cx).link().cloned()), None, "a link the pointer isn't on");
    cx.update(|_, cx| state.update(cx, |state, cx| state.set_link(Some(name.clone()), cx)));
    draw(cx);

    // a click on it with ⌘ held is told, and selects nothing
    cx.simulate_mouse_down(beta, MouseButton::Left, cmd);
    cx.simulate_mouse_up(beta, MouseButton::Left, cmd);
    assert_eq!(take(&told), ["clicked 1:8..12"]);
    assert_eq!(cx.update(|_, cx| state.read(cx).selected_text(cx)), "");
    assert!(cx.update(|_, cx| state.read(cx).selected_lines().is_none()));
    // pressed on the link and let go elsewhere: nothing
    cx.simulate_mouse_down(beta, MouseButton::Left, cmd);
    cx.simulate_mouse_up(gamma, MouseButton::Left, cmd);
    assert_eq!(take(&told), Vec::<String>::new());
    // a click with ⌘ on a name that is no link is an ordinary click
    cx.simulate_mouse_move(gamma, None, cmd);
    assert_eq!(take(&told), ["pointed 1:13..18"]);
    assert_eq!(cx.update(|_, cx| state.read(cx).link().cloned()), None, "the link left with the pointer");
    cx.simulate_mouse_down(gamma, MouseButton::Left, cmd);
    cx.simulate_mouse_up(gamma, MouseButton::Left, cmd);
    assert_eq!(take(&told), Vec::<String>::new());

    // ⌘ let go while the pointer rests on a name: nothing is pointed at any more; pressed again: the name again.
    // The application tells the view (a change of the keys goes to what has the keyboard, which the view may not)
    cx.simulate_mouse_move(beta, None, cmd);
    take(&told);
    cx.update(|_, cx| state.update(cx, |state, cx| state.point_with(false, beta, cx)));
    assert_eq!(take(&told), ["pointed nothing"]);
    cx.update(|_, cx| state.update(cx, |state, cx| state.point_with(true, beta, cx)));
    assert_eq!(take(&told), ["pointed 1:8..12"]);
    // a point outside the view is on no name
    cx.update(|_, cx| state.update(cx, |state, cx| state.point_with(true, point(px(5000.), px(5000.)), cx)));
    assert_eq!(take(&told), ["pointed nothing"]);

    // a link may be wider than the word that was told (a tag's whole name): moving on inside it asks nothing
    let row = on_name(&state, cx, 3, 1);
    let item = on_name(&state, cx, 3, 5);
    cx.simulate_mouse_move(row, None, cmd);
    assert_eq!(take(&told), ["pointed 3:1..4"]);
    let tag = DiffName { position: at(3), range: 1..9, column: 2 };
    cx.update(|_, cx| state.update(cx, |state, cx| state.set_link(Some(tag.clone()), cx)));
    cx.simulate_mouse_move(item, None, cmd);
    assert_eq!(take(&told), Vec::<String>::new());
    assert_eq!(cx.update(|_, cx| state.read(cx).link().cloned()), Some(tag));
    draw(cx);
    cx.simulate_mouse_down(item, MouseButton::Left, cmd);
    cx.simulate_mouse_up(item, MouseButton::Left, cmd);
    assert_eq!(take(&told), ["clicked 3:1..9"]);
}
