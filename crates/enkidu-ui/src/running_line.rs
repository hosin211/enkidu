//! While Claude works, the hairline under its terminal tabs runs amber (DESIGN.md, "Motion"): a faint amber line with a
//! band of light crossing it, left to right, every 1.9 s. It steps on the lamps' clock, so it costs no frame of its own
//! while a lamp turns.

use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::{Animation, AnimationExt as _, Div, ElementId, div, linear_color_stop, linear_gradient, px, relative};

use crate::lamp::LAMP_FPS;
use crate::pieces::Observed;
use crate::tokens::{EASE_IN_OUT, LAMP_AMBER, alpha, bezier};
use gpui_kit::base::TestSupportExt as _;

/// One crossing of the light.
pub const RUN: Duration = Duration::from_millis(1900);

/// The light's width, as a share of the line's.
pub const LIGHT: f32 = 0.34;

/// Where the light's left edge is at `t` of a crossing, as a share of the line's width: it starts just off the left end
/// and ends just off the right one, eased in and out.
pub fn light_at(t: f32) -> f32 {
    LIGHT * (-1.0 + 4.0 * bezier(EASE_IN_OUT, t))
}

/// The running line: put it in a parent that is `relative()`; it lies on the parent's bottom edge. `id` names its
/// motion and is what tests find.
pub fn running_line(id: impl Into<ElementId>) -> Observed {
    let id: ElementId = id.into();
    let half = |from: f32, to: f32| -> Div {
        div().h_full().w(relative(0.5)).bg(linear_gradient(
            90.,
            linear_color_stop(alpha(LAMP_AMBER, from), 0.),
            linear_color_stop(alpha(LAMP_AMBER, to), 1.),
        ))
    };
    div()
        .id(id.clone())
        .test_support()
        .aria_label("Claude is working")
        .absolute()
        .left_0()
        .right_0()
        .bottom(px(-1.))
        .h(px(1.))
        .overflow_hidden()
        .bg(alpha(LAMP_AMBER, 0.22))
        .child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .w(relative(LIGHT))
                .flex()
                .child(half(0., 1.))
                .child(half(1., 0.))
                .with_animation(id, Animation::new(RUN).repeat_synced().with_max_fps(LAMP_FPS), |light, t| {
                    light.left(relative(light_at(t)))
                }),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_light_crosses_the_line_from_end_to_end() {
        assert!((light_at(0.0) + LIGHT).abs() < 1e-3, "it starts just off the left end");
        assert!((light_at(1.0) - 3.0 * LIGHT).abs() < 1e-3, "and ends just off the right one");
        // eased in and out: slow at both ends
        assert!(light_at(0.1) - light_at(0.0) < 0.02);
        assert!(light_at(1.0) - light_at(0.9) < 0.02);
        let mut last = light_at(0.0);
        for step in 1..=20 {
            let now = light_at(step as f32 / 20.0);
            assert!(now >= last, "it only moves right");
            last = now;
        }
    }
}
