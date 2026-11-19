//! The pill behind the chosen one of a row: the thumb of a switch with words, and the blue edge of the workspace's tab
//! that is on screen. It glides to a newly chosen one in 220 ms on our ease-out curve (DESIGN.md, "Motion": "A
//! switch's thumb | glides 220 ms to the new choice", "The tab on screen | its blue edge glides to the tab you pick").
//!
//! GPUI knows where the tabs are only once a frame is laid out, so each one reports its place as it is drawn and
//! the pill goes there; until the places are known (the very first frame) the chosen one draws the pill itself.

use std::sync::Arc;

use gpui_kit::base::{ElementExt as _, TestSupportExt as _, Transition, transition};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Bounds, BoxShadow, Div, ElementId, Pixels, Point, Rgba, Role, ScrollHandle, SharedString, Window, div, point,
    px,
};

use crate::pieces::Observed;
use crate::tokens::*;

/// One answer of a switch with words (a dialog's "Above the terminal | Beside the terminal"): 28 px high with 12 px
/// inside, bright when it is the one chosen. Who uses it says what a click does.
pub fn word_segment(id: impl Into<ElementId>, words: impl Into<SharedString>, chosen: bool) -> Observed {
    let words = words.into();
    div()
        .id(id)
        .test_support()
        .role(Role::Tab)
        .aria_label(words.clone())
        .aria_selected(chosen)
        .h(px(28.))
        .px(px(12.))
        .flex()
        .items_center()
        .rounded_full()
        .text_size(TEXT)
        .font_weight(WEIGHT_LABEL)
        .text_color(if chosen { INK } else { INK_2 })
        .cursor_pointer()
        .child(words)
}

/// The switch itself, as high as a control in a dialog: its answers in a track with a clear edge, 2 px around them,
/// the chosen fill gliding under the one at `selected` (`<id>-pill`). A switch is a pill.
pub fn word_switch(id: &str, selected: usize, segments: Vec<Observed>, window: &mut Window, cx: &mut App) -> Div {
    let edge = BoxShadow {
        color: hsla(LINE),
        offset: point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: px(1.),
        inset: true,
    };
    div().flex_none().p(px(2.)).rounded_full().shadow(vec![edge]).child(tab_row(id, selected, segments, window, cx))
}

/// Where one row's tabs were last drawn.
#[derive(Default)]
struct Places {
    row: Option<Point<Pixels>>,
    tabs: Vec<Bounds<Pixels>>,
}

/// What the pill says (DESIGN.md, "States").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pill {
    /// This one is chosen: a half of a switch. The chosen fill, flat.
    Picked,
    /// This is where you are: the workspace on screen. The stage's black with a blue edge, which turns gray while
    /// the window is behind another app.
    OnScreen,
}

/// The pill's fill, and its edge: a ring just inside it (an inset shadow without blur), so the edge is never cut
/// where the row scrolls, and nothing of it is painted under the pill.
fn pill_look(look: Pill, window_in_front: bool) -> (Rgba, Vec<BoxShadow>) {
    let ring = |color: Rgba| BoxShadow {
        color: hsla(color),
        offset: point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: ON_SCREEN_EDGE,
        inset: true,
    };
    match (look, window_in_front) {
        (Pill::Picked, _) => (SELECTED, Vec::new()),
        (Pill::OnScreen, true) => (WELL, vec![ring(HERE)]),
        (Pill::OnScreen, false) => (WELL, vec![ring(BEHIND)]),
    }
}

/// What a tab of the row is: something drawn that the row can measure and, until it knows where the tabs are, give
/// the pill's own look. A plain piece is one, and one that answers a press.
pub trait Tab: Styled + ParentElement + IntoElement {}
impl<T: Styled + ParentElement + IntoElement> Tab for T {}

/// A switch with the thumb behind the half at `selected`: the chosen fill, flat (DESIGN.md, "States": Picked). `id`
/// keys the thumb's motion and its memory of where the halves are; the thumb itself is `<id>-pill`. Call it while
/// rendering, where `window` keeps element state.
pub fn tab_row<T: Tab>(id: &str, selected: usize, tabs: Vec<T>, window: &mut Window, cx: &mut App) -> Div {
    build(id, Some(selected), tabs, None, Pill::Picked, 0., window, cx)
}

/// The row of the workspaces: its pill says where you are ([`Pill::OnScreen`]), and it scrolls sideways when its
/// tabs don't fit (`scroll` follows it; the row's own id is `id`). A newly chosen tab is scrolled into view;
/// scrolling by hand is left alone until the next choice. With no tab on screen (`selected` is none) there is no
/// pill.
pub fn scrolling_tab_row<T: Tab>(
    id: &str,
    selected: Option<usize>,
    tabs: Vec<T>,
    scroll: &ScrollHandle,
    window: &mut Window,
    cx: &mut App,
) -> Observed {
    build(id, selected, tabs, Some(scroll.clone()), Pill::OnScreen, 4., window, cx)
        .id(SharedString::from(id.to_string()))
        .overflow_x_scroll()
        .track_scroll(scroll)
        .test_support()
}

#[allow(clippy::too_many_arguments)]
fn build<T: Tab>(
    id: &str,
    selected: Option<usize>,
    tabs: Vec<T>,
    scroll: Option<ScrollHandle>,
    look: Pill,
    gap: f32,
    window: &mut Window,
    cx: &mut App,
) -> Div {
    let row_id = ElementId::Name(SharedString::from(id.to_string()));
    // the blue edge of where you are rests while the window is behind another app
    let (fill, edge) = pill_look(look, window.is_window_active());
    let part = |what: &str| ElementId::NamedChild(Arc::new(row_id.clone()), SharedString::from(what.to_string()));
    // the pill has an id of its own (`<id>-pill`), so tests can find where it is
    let pill_id = SharedString::from(format!("{id}-pill"));
    let places = window.use_keyed_state(part("places"), cx, |_, _| Places::default());
    let known = selected.and_then(|selected| {
        let places = places.read(cx);
        match (places.row, places.tabs.get(selected)) {
            (Some(row), Some(tab)) if tab.size.width > px(0.) => {
                Some(Bounds { origin: point(tab.origin.x - row.x, tab.origin.y - row.y), size: tab.size })
            }
            _ => None,
        }
    });
    let count = tabs.len();
    if let (Some(scroll), Some(selected)) = (&scroll, selected) {
        // the row's children are the pill (once its place is known) and then the tabs
        let revealed = window.use_keyed_state(part("revealed"), cx, |_, _| None::<usize>);
        if *revealed.read(cx) != Some(selected) {
            revealed.update(cx, |revealed, _| *revealed = Some(selected));
            scroll.scroll_to_item(selected + usize::from(known.is_some()));
        }
    }
    let mut row = div().relative().flex().items_center().gap(px(gap));
    if let Some(target) = known {
        let at = transition(part("pill"), target, Transition::new(DUR_BASE).ease(|t| bezier(EASE_OUT, t)), window, cx);
        row = row.child(
            div()
                .id(pill_id)
                .test_support()
                .absolute()
                .left(at.origin.x)
                .top(at.origin.y)
                .w(at.size.width)
                .h(at.size.height)
                .rounded_full()
                .bg(fill)
                .shadow(edge.clone()),
        );
    }
    for (ix, tab) in tabs.into_iter().enumerate() {
        let tab = if known.is_none() && Some(ix) == selected { tab.bg(fill).shadow(edge.clone()) } else { tab };
        let places = places.clone();
        // a scrolled row moves the tabs and its own measure (a child too) alike, so the pill needs no correction
        row = row.child(tab.on_prepaint(move |bounds, _, cx| {
            places.update(cx, |places, cx| {
                places.tabs.resize(count, Bounds::default());
                if places.tabs[ix] != bounds {
                    places.tabs[ix] = bounds;
                    cx.notify();
                }
            })
        }));
    }
    row.on_prepaint(move |bounds, _, cx| {
        places.update(cx, |places, cx| {
            if places.row != Some(bounds.origin) {
                places.row = Some(bounds.origin);
                cx.notify();
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pill's edge is a ring inside it (so the scrolling row never cuts it), and its color says what the pill
    /// says: blue for where you are, gray while the window is behind another app, none for a switch.
    #[test]
    fn the_pills_edge_says_where_you_are() {
        let near = |a: Rgba, b: Rgba| [a.r - b.r, a.g - b.g, a.b - b.b, a.a - b.a].iter().all(|d| d.abs() < 0.01);
        let edge = |look, in_front| {
            let (fill, rings) = pill_look(look, in_front);
            assert!(near(fill, WELL), "the tab on screen stands on the stage's black: {fill:?}");
            assert_eq!(rings.len(), 1, "one edge, inside the pill");
            assert!(rings[0].inset && rings[0].blur_radius == px(0.), "a ring, not a glow");
            (Rgba::from(rings[0].color), rings[0].spread_radius)
        };
        let (blue, width) = edge(Pill::OnScreen, true);
        assert!(near(blue, ACCENT) && width == ON_SCREEN_EDGE, "{blue:?} {width:?}");
        let (gray, width) = edge(Pill::OnScreen, false);
        assert!(near(gray, BEHIND) && width == ON_SCREEN_EDGE, "{gray:?}");
        for in_front in [true, false] {
            let (fill, rings) = pill_look(Pill::Picked, in_front);
            assert!(near(fill, SELECTED) && rings.is_empty(), "a switch has no blue: {rings:?}");
        }
    }
}
