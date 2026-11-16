//! The signal lamp: one light for what Claude is doing, in every tab, the palette and the status line (DESIGN.md,
//! "Color" and "Motion"). Idle is a hollow ring; working is amber with a ring turning around it; needs you is red with
//! a ping going out; done is green. A change of state pops the lamp once.
//!
//! The motion is cheap on purpose (item 1.11). GPUI draws the whole window again for any motion, and in
//! phase 0 one turning lamp at 120 frames a second cost 19 to 22 % of one core. Here the ring and the ping step
//! [`LAMP_FPS`] times a second, on one clock shared by every lamp, so any number of lamps cost one redraw per step.

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::{
    Animation, AnimationExt as _, BoxShadow, ElementId, IntoElement, Rgba, SharedString, Transformation, div,
    percentage, point, px, svg,
};

use crate::tokens::{self, EASE_POP, POP, alpha, bezier, hsla};

/// How many times a second a lamp's ring or ping moves on.
pub const LAMP_FPS: f32 = 15.0;
/// The ring turns once in this time.
pub const RING_TURN: Duration = Duration::from_millis(950);

/// What Claude is doing in a workspace.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Status {
    Idle,
    Working,
    NeedsYou,
    Done,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Idle => "Idle",
            Status::Working => "Working",
            Status::NeedsYou => "Needs you",
            Status::Done => "Done",
        }
    }

    pub fn color(self) -> Rgba {
        match self {
            Status::Idle => tokens::LAMP_OFF,
            Status::Working => tokens::LAMP_AMBER,
            Status::NeedsYou => tokens::LAMP_RED,
            Status::Done => tokens::LAMP_GREEN,
        }
    }

    /// The core's word for it (`idle`, `working`, `needs_you`, `done`).
    pub fn from_core(word: &str) -> Status {
        match word {
            "working" => Status::Working,
            "needs_you" => Status::NeedsYou,
            "done" => Status::Done,
            _ => Status::Idle,
        }
    }

    /// The core's word for it, as the frame keeps it.
    pub fn core_word(self) -> &'static str {
        match self {
            Status::Idle => "idle",
            Status::Working => "working",
            Status::NeedsYou => "needs_you",
            Status::Done => "done",
        }
    }

    /// The light's own width for a lamp `size` wide.
    pub fn dot(self, size: f32) -> f32 {
        (size
            * match self {
                Status::Working => 0.42,
                Status::Idle => 0.58,
                _ => 0.5,
            })
        .round()
    }
}

/// Where the ping is at `t` of its 1.7 s round: its scale and opacity.
pub fn ping_at(t: f32) -> (f32, f32) {
    let grow = (t / 0.75).clamp(0.0, 1.0);
    let eased = bezier([0.0, 0.0, 0.2, 1.0], grow);
    (1.0 + 1.8 * eased, 0.55 * (1.0 - eased))
}

/// A lamp `size` pixels wide. `id` names its motion. `pops` counts the changes of state seen so far: each new count
/// pops the lamp once (0 never pops, so the first state shown stays still).
pub fn lamp(id: impl Into<ElementId>, status: Status, size: f32, pops: u32) -> impl IntoElement {
    let id: ElementId = id.into();
    let color = status.color();
    let dot = status.dot(size);
    let name = |what: &str| ElementId::NamedChild(Arc::new(id.clone()), SharedString::from(what.to_string()));

    let light = div().flex_none().size(px(dot)).rounded_full().map(|light| match status {
        Status::Idle => light.border_1().border_color(color),
        _ => light.bg(color).shadow(vec![BoxShadow {
            color: hsla(alpha(color, 0.55)),
            offset: point(px(0.), px(0.)),
            blur_radius: px(dot * 1.1),
            spread_radius: px(0.),
            inset: false,
        }]),
    });
    // a new state: the light grows from 0.3 of its size with a small overshoot, 460 ms
    let light = if pops > 0 {
        light
            .with_animation(
                name(&format!("pop-{pops}")),
                Animation::new(POP).with_easing(|t| bezier(EASE_POP, t)),
                move |light, t| light.size(px(dot * (0.3 + 0.7 * t))),
            )
            .into_any_element()
    } else {
        light.into_any_element()
    };

    div()
        .id(id.clone())
        .relative()
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .size(px(size))
        .when(status == Status::NeedsYou, |this| {
            this.child(div().absolute().size(px(dot)).rounded_full().bg(color).with_animation(
                name("ping"),
                Animation::new(tokens::PING_EVERY).repeat_synced().with_max_fps(LAMP_FPS),
                move |ping, t| {
                    let (scale, opacity) = ping_at(t);
                    let side = dot * scale;
                    ping.size(px(side)).left(px((size - side) / 2.0)).top(px((size - side) / 2.0)).opacity(opacity)
                },
            ))
        })
        .when(status == Status::Working, |this| {
            this.child(
                svg()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size(px(size))
                    .path("icons/lamp-ring.svg")
                    .text_color(color)
                    .with_animation(
                        name("ring"),
                        Animation::new(RING_TURN).repeat_synced().with_max_fps(LAMP_FPS),
                        |ring, t| ring.with_transformation(Transformation::rotate(percentage(t))),
                    ),
            )
        })
        .child(light)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_core_s_words_and_the_lamp_s_look() {
        assert_eq!(Status::from_core("working"), Status::Working);
        assert_eq!(Status::from_core("needs_you"), Status::NeedsYou);
        assert_eq!(Status::from_core("done"), Status::Done);
        assert_eq!(Status::from_core("idle"), Status::Idle);
        assert_eq!(Status::from_core("anything else"), Status::Idle);
        // the sizes at 14 px: working 6, idle 8, the others 7
        assert_eq!((Status::Working.dot(14.), Status::Idle.dot(14.), Status::Done.dot(14.)), (6., 8., 7.));
        assert_eq!(Status::NeedsYou.label(), "Needs you");
    }

    /// Has a label for each state.
    #[test]
    fn has_a_label_for_each_state() {
        let labels = [Status::Idle, Status::Working, Status::NeedsYou, Status::Done].map(Status::label);
        assert_eq!(labels, ["Idle", "Working", "Needs you", "Done"]);
    }

    #[test]
    fn the_ping_goes_out_and_fades_by_three_quarters_then_rests() {
        let (scale, opacity) = ping_at(0.0);
        assert!((scale - 1.0).abs() < 1e-3 && (opacity - 0.55).abs() < 1e-3);
        let (scale, opacity) = ping_at(0.75);
        assert!((scale - 2.8).abs() < 1e-3 && opacity.abs() < 1e-3);
        assert_eq!(ping_at(0.9), ping_at(1.0), "nothing moves in the rest of the round");
        let (half, _) = ping_at(0.375);
        assert!(half > 1.9, "fast at first: {half}");
    }
}
