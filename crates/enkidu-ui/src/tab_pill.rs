//! The pill behind the chosen tab of a row: it glides to a newly chosen tab in 220 ms on our ease-out curve
//! (DESIGN.md, "Motion": "The tab pill, segmented thumbs | glide 220 ms to the new choice"). Behind the workspace on
//! screen its edge is signal blue: the mark of where you are (DESIGN.md, "States").
//!
//! GPUI knows where the tabs are only once a frame is laid out, so each tab reports its place as it is drawn and the
//! pill goes there; until the places are known (the very first frame) the chosen tab draws the pill itself.

use std::sync::Arc;

use gpui_kit::base::{ElementExt as _, TestSupportExt as _, Transition, transition};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Bounds, BoxShadow, Div, ElementId, Pixels, Point, Rgba, ScrollHandle, SharedString, Window, div, point, px,
};

use crate::pieces::Observed;
use crate::tokens::*;

/// Where one row's tabs were last drawn.
#[derive(Default)]
struct Places {
    row: Option<Point<Pixels>>,
    tabs: Vec<Bounds<Pixels>>,
}

/// What the pill says (DESIGN.md, "States").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pill {
    /// This one is chosen: a half of a switch, a tab of a dialog. The chosen fill with a hairline edge.
    Picked,
    /// This is where you are: the workspace on screen. The chosen fill with an edge in signal blue, which turns
    /// gray while the window is behind another app.
    OnScreen,
}

/// The soft shadow the pill casts, and its edge: a ring just inside it (an inset shadow without blur), so the edge
/// is never cut where the row scrolls, and nothing of it is painted under the pill.
fn pill_shadow(look: Pill, window_in_front: bool) -> Vec<BoxShadow> {
    let (edge, width) = match (look, window_in_front) {
        (Pill::Picked, _) => (ISLAND_EDGE, px(1.)),
        (Pill::OnScreen, true) => (ACCENT, ON_SCREEN_EDGE),
        (Pill::OnScreen, false) => (BEHIND, ON_SCREEN_EDGE),
    };
    vec![
        BoxShadow {
            color: hsla(Rgba { r: 0., g: 0., b: 0., a: 0.35 }),
            offset: point(px(0.), px(1.)),
            blur_radius: px(3.),
            spread_radius: px(0.),
            inset: false,
        },
        BoxShadow {
            color: hsla(edge),
            offset: point(px(0.), px(0.)),
            blur_radius: px(0.),
            spread_radius: width,
            inset: true,
        },
    ]
}

/// What a tab of the row is: something drawn that the row can measure and, until it knows where the tabs are, give
/// the pill's own look. A plain piece is one, and one that answers a press.
pub trait Tab: Styled + ParentElement + IntoElement {}
impl<T: Styled + ParentElement + IntoElement> Tab for T {}

/// A row of tabs with the pill behind the one at `selected`: the chosen one of a switch. `id` keys the pill's motion
/// and its memory of where the tabs are; the pill itself is `<id>-pill`. Call it while rendering, where `window`
/// keeps element state.
pub fn tab_row<T: Tab>(id: &str, selected: usize, tabs: Vec<T>, window: &mut Window, cx: &mut App) -> Div {
    build(id, selected, tabs, None, Pill::Picked, window, cx)
}

/// The same row with the pill's look said: for a row that doesn't scroll and still says where you are.
pub fn tab_row_of<T: Tab>(
    look: Pill,
    id: &str,
    selected: usize,
    tabs: Vec<T>,
    window: &mut Window,
    cx: &mut App,
) -> Div {
    build(id, selected, tabs, None, look, window, cx)
}

/// The row of the workspaces: its pill says where you are ([`Pill::OnScreen`]), and it scrolls sideways when its
/// tabs don't fit (`scroll` follows it; the row's own id is `id`). A newly chosen tab is scrolled into view;
/// scrolling by hand is left alone until the next choice.
pub fn scrolling_tab_row<T: Tab>(
    id: &str,
    selected: usize,
    tabs: Vec<T>,
    scroll: &ScrollHandle,
    window: &mut Window,
    cx: &mut App,
) -> Observed {
    build(id, selected, tabs, Some(scroll.clone()), Pill::OnScreen, window, cx)
        .id(SharedString::from(id.to_string()))
        .overflow_x_scroll()
        .track_scroll(scroll)
        .test_support()
}

fn build<T: Tab>(
    id: &str,
    selected: usize,
    tabs: Vec<T>,
    scroll: Option<ScrollHandle>,
    look: Pill,
    window: &mut Window,
    cx: &mut App,
) -> Div {
    let row_id = ElementId::Name(SharedString::from(id.to_string()));
    // the blue edge of where you are rests while the window is behind another app
    let shadow = pill_shadow(look, window.is_window_active());
    let part = |what: &str| ElementId::NamedChild(Arc::new(row_id.clone()), SharedString::from(what.to_string()));
    // the pill has an id of its own (`<id>-pill`), so tests can find where it is
    let pill_id = SharedString::from(format!("{id}-pill"));
    let places = window.use_keyed_state(part("places"), cx, |_, _| Places::default());
    let known = {
        let places = places.read(cx);
        match (places.row, places.tabs.get(selected)) {
            (Some(row), Some(tab)) if tab.size.width > px(0.) => {
                Some(Bounds { origin: point(tab.origin.x - row.x, tab.origin.y - row.y), size: tab.size })
            }
            _ => None,
        }
    };
    let count = tabs.len();
    if let Some(scroll) = &scroll {
        // the row's children are the pill (once its place is known) and then the tabs
        let revealed = window.use_keyed_state(part("revealed"), cx, |_, _| None::<usize>);
        if *revealed.read(cx) != Some(selected) {
            revealed.update(cx, |revealed, _| *revealed = Some(selected));
            scroll.scroll_to_item(selected + usize::from(known.is_some()));
        }
    }
    let mut row = div().relative().flex().items_center().gap(px(2.));
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
                .rounded(TAB_RADIUS)
                // (the fill as it is seen on the frame: every row of tabs stands on the frame's color)
                .bg(SELECTED_ON_FRAME)
                .shadow(shadow.clone()),
        );
    }
    for (ix, tab) in tabs.into_iter().enumerate() {
        let tab =
            if known.is_none() && ix == selected { tab.bg(SELECTED_ON_FRAME).shadow(shadow.clone()) } else { tab };
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
    /// says: blue for where you are, gray while the window is behind another app, a hairline for a switch.
    #[test]
    fn the_pills_edge_says_where_you_are() {
        let edge = |look, in_front| {
            let shadows = pill_shadow(look, in_front);
            let rings: Vec<&BoxShadow> = shadows.iter().filter(|shadow| shadow.inset).collect();
            assert_eq!(rings.len(), 1, "one edge, inside the pill");
            assert_eq!(rings[0].blur_radius, px(0.), "a ring, not a glow");
            (Rgba::from(rings[0].color), rings[0].spread_radius)
        };
        let near = |a: Rgba, b: Rgba| [a.r - b.r, a.g - b.g, a.b - b.b, a.a - b.a].iter().all(|d| d.abs() < 0.01);
        let (blue, width) = edge(Pill::OnScreen, true);
        assert!(near(blue, ACCENT) && width == ON_SCREEN_EDGE, "{blue:?} {width:?}");
        let (gray, width) = edge(Pill::OnScreen, false);
        assert!(near(gray, BEHIND) && width == ON_SCREEN_EDGE, "{gray:?}");
        for in_front in [true, false] {
            let (hairline, width) = edge(Pill::Picked, in_front);
            assert!(near(hairline, ISLAND_EDGE) && width == px(1.), "a switch has no blue: {hairline:?}");
        }
    }
}
