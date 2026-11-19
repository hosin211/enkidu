//! A switch: a row of mutually exclusive buttons. The choice sits on a thumb that glides to the next one in 220 ms
//! (DESIGN.md, "Motion": "A switch's thumb | glides 220 ms to the new choice"). This is the one with an icon in each
//! half: a control's 28 px, with 2 px inside it around the thumb, and a pill (DESIGN.md, "Space, size, shape").

use std::sync::Arc;

use gpui_kit::base::{TestSupportExt as _, Transition, transition};
use gpui_kit::prelude::*;
use gpui_kit::{App, BoxShadow, ElementId, Role, SharedString, Window, div, point, px, svg};

use crate::pieces::Observed;
use crate::tokens::*;

/// A segment's width: its 16 px icon and 4 px each side.
const SEGMENT: f32 = 24.;
const HEIGHT: f32 = 24.;
/// The track's padding around its segments.
const PAD: f32 = 2.;

/// One segment: an icon, named by `title` for tests and the accessibility tree. `on` is the chosen one. Give it its
/// `on_click`, then hand the segments to [`segmented`].
pub fn segment(id: impl Into<ElementId>, icon: &'static str, title: impl Into<SharedString>, on: bool) -> Observed {
    segment_sized(id, icon, 16., title, on)
}

/// The same with an icon of `size` px.
pub fn segment_sized(
    id: impl Into<ElementId>,
    icon: &'static str,
    size: f32,
    title: impl Into<SharedString>,
    on: bool,
) -> Observed {
    div()
        .id(id)
        .test_support()
        .role(Role::RadioButton)
        .aria_label(title.into())
        .aria_selected(on)
        .relative()
        .w(px(SEGMENT))
        .h(px(HEIGHT))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded_full()
        .cursor_pointer()
        // an icon is drawn in its own color, not its parent's: it brightens with the segment it is in
        .group("segment")
        .child(
            svg()
                .path(icon)
                .size(px(size))
                .text_color(if on { INK } else { INK_3 })
                .group_hover("segment", |s| s.text_color(INK)),
        )
}

/// The track's edge: a line just inside it, which takes no room.
fn track_edge() -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: hsla(LINE),
        offset: point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: px(1.),
        inset: true,
    }]
}

/// The track with its segments and the thumb under the one at `selected` (none when a view outside the choices has
/// replaced them). `id` keys the thumb's motion; the thumb itself is `<id>-thumb`. Call it while rendering.
pub fn segmented(
    id: &str,
    selected: Option<usize>,
    segments: Vec<Observed>,
    window: &mut Window,
    cx: &mut App,
) -> Observed {
    let track = ElementId::Name(SharedString::from(id.to_string()));
    // (the track is named too, so a test can read the 2 px inside it)
    let mut row = div()
        .id(SharedString::from(id.to_string()))
        .test_support()
        .role(Role::RadioGroup)
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .p(px(PAD))
        .rounded_full()
        .shadow(track_edge());
    if let Some(selected) = selected {
        // every segment is as wide as the next, so the thumb's place needs no measuring
        let x = transition(
            ElementId::NamedChild(Arc::new(track), SharedString::from("thumb")),
            PAD + SEGMENT * selected as f32,
            Transition::new(DUR_BASE).ease(|t| bezier(EASE_OUT, t)),
            window,
            cx,
        );
        row = row.child(
            div()
                .id(SharedString::from(format!("{id}-thumb")))
                .test_support()
                .absolute()
                .left(px(x))
                .top(px(PAD))
                .w(px(SEGMENT))
                .h(px(HEIGHT))
                .rounded_full()
                .bg(THUMB),
        );
    }
    row.children(segments)
}
