//! A row of mutually exclusive buttons: the choice sits on a thumb that glides to the next one in 220 ms (DESIGN.md,
//! "Motion": "The tab pill, segmented thumbs | glide 220 ms to the new choice"). This is the small size, with an icon
//! in each segment: 22 px high (DESIGN.md, "Control heights").

use std::sync::Arc;

use gpui_kit::base::{TestSupportExt as _, Transition, transition};
use gpui_kit::prelude::*;
use gpui_kit::{App, BoxShadow, Div, ElementId, Rgba, Role, SharedString, Window, div, point, px, svg};

use crate::pieces::Observed;
use crate::tokens::*;

/// A segment's width: its 13 px icon and 6 px each side, 26 at least.
const SEGMENT: f32 = 26.;
const HEIGHT: f32 = 22.;
/// The track's padding around its segments.
const PAD: f32 = 2.;

/// One segment: an icon, named by `title` for tests and the accessibility tree. `on` is the chosen one. Give it its
/// `on_click`, then hand the segments to [`segmented`].
pub fn segment(id: impl Into<ElementId>, icon: &'static str, title: impl Into<SharedString>, on: bool) -> Observed {
    segment_sized(id, icon, 13., title, on)
}

/// The same with an icon of `size` px (the layout pictures are drawn at 16).
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
        .rounded(px(6.))
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

/// The thumb's look: lifted by a hair of an edge, lit from above, with a soft shadow under it.
fn thumb_shadow() -> Vec<BoxShadow> {
    let shadow = |color: Rgba, y: f32, blur: f32, spread: f32, inset: bool| BoxShadow {
        color: hsla(color),
        offset: point(px(0.), px(y)),
        blur_radius: px(blur),
        spread_radius: px(spread),
        inset,
    };
    vec![
        shadow(white(0.12), 0., 0., 0.5, false),
        shadow(white(0.07), 1., 0., 0., true),
        shadow(Rgba { r: 0., g: 0., b: 0., a: 0.5 }, 1., 2., 0., false),
    ]
}

/// The track with its segments and the thumb under the one at `selected` (none when a view outside the choices has
/// replaced them). `id` keys the thumb's motion; the thumb itself is `<id>-thumb`. Call it while rendering.
pub fn segmented(id: &str, selected: Option<usize>, segments: Vec<Observed>, window: &mut Window, cx: &mut App) -> Div {
    let track = ElementId::Name(SharedString::from(id.to_string()));
    let mut row = div().relative().flex().flex_none().items_center().p(px(PAD)).rounded(RADIUS_CONTROL).bg(HOVER);
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
                .rounded(px(6.))
                .bg(THUMB)
                .shadow(thumb_shadow()),
        );
    }
    row.children(segments)
}
