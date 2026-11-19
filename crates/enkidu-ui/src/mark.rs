//! Enkidu's mark, as the empty screens draw it (DESIGN.md, "The mark" and "Motion": "the mark's two wedges are
//! pressed in one after the other, then the second one lights blue"; "seen rarely, so it may take its time"): two
//! cuneiform wedges standing side by side, their heads up. It is the icon's own drawing (`bundle/icon.svg`), in the
//! window's gray, with the second wedge in the icon's blue.
//!
//! GPUI's svg element paints in one color and can't be animated part by part, so the wedges are drawn as paths, at
//! the icon's own coordinates scaled to the width asked for.

use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::{
    Animation, AnimationExt as _, Bounds, ElementId, IntoElement, PathBuilder, Pixels, Point, Rgba, Window, canvas,
    div, point, px,
};

use crate::tokens::{EASE_OUT, INK_3, MARK_BLUE, alpha, bezier};

/// A wedge: its head pressed in at (`x`, `y`), its tail running `len` down. `head` is the head's width, `neck` how
/// far down the head reaches, `stem` and `tip` the tail's width at the neck and at its end.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wedge {
    pub x: f32,
    pub y: f32,
    pub len: f32,
    pub head: f32,
    pub neck: f32,
    pub stem: f32,
    pub tip: f32,
}

impl Wedge {
    /// Its six corners, from the head's left one down the left side and up the right, `pressed` of the way in (0 to
    /// 1: it grows down out of its head).
    pub fn corners(&self, pressed: f32) -> [(f32, f32); 6] {
        let at = |along: f32, across: f32| (self.x + across, self.y + along * pressed);
        [
            at(0., -self.head / 2.),
            at(self.neck, -self.stem / 2.),
            at(self.len, -self.tip / 2.),
            at(self.len, self.tip / 2.),
            at(self.neck, self.stem / 2.),
            at(0., self.head / 2.),
        ]
    }
}

/// Where the icon's mark begins on the icon's 1,024 px canvas, and its size: the drawing's view here.
pub const ICON_ORIGIN: (f32, f32) = (318., 262.);
const VIEW: (f32, f32) = (388., 500.);

const fn stand(x: f32) -> Wedge {
    Wedge { x, y: 0., len: 500., head: 172., neck: 156., stem: 60., tip: 34. }
}

/// The two wedges, left to right, in the view.
pub const WEDGES: [Wedge; 2] = [stand(86.), stand(302.)];

/// A wedge is pressed in over 0.42 s, the second one starts 0.36 s after the first; the second one lights over
/// 0.6 s from 0.94 s on.
pub const PRESS: Duration = Duration::from_millis(420);
pub const BETWEEN: Duration = Duration::from_millis(360);
pub const LIGHT_FROM: Duration = Duration::from_millis(940);
pub const LIGHT_FOR: Duration = Duration::from_millis(600);
const WHOLE: Duration = Duration::from_millis(1540);

/// Where the motion is `elapsed` in: how far each wedge is pressed in (0, then up to 1, one after the other), and
/// the second one's light (0, then up to 1).
pub fn mark_at(elapsed: Duration) -> ([f32; 2], f32) {
    // the curve's ends exactly: the solver gives 1e-9 for 0
    let ease = |t: f32| {
        if t <= 0. {
            0.
        } else if t >= 1. {
            1.
        } else {
            bezier(EASE_OUT, t)
        }
    };
    let since = |start: Duration, lasts: Duration| (elapsed.as_secs_f32() - start.as_secs_f32()) / lasts.as_secs_f32();
    let pressed = [0u32, 1].map(|ix| ease(since(BETWEEN * ix, PRESS)));
    (pressed, ease(since(LIGHT_FROM, LIGHT_FOR)))
}

/// The drawing, `width` pixels wide (its height follows), playing its motion once from when it is first shown.
pub fn mark(id: impl Into<ElementId>, width: f32) -> impl IntoElement {
    div().flex_none().w(px(width)).h(px(width * VIEW.1 / VIEW.0)).with_animation(
        id.into(),
        Animation::new(WHOLE),
        |frame, t| {
            let (pressed, light) = mark_at(WHOLE.mul_f32(t));
            frame.child(
                canvas(|_, _, _| {}, move |bounds, _, window, _| paint(bounds, pressed, light, window)).size_full(),
            )
        },
    )
}

fn paint(bounds: Bounds<Pixels>, pressed: [f32; 2], light: f32, window: &mut Window) {
    let scale = bounds.size.width / px(VIEW.0);
    let to = |(x, y): (f32, f32)| -> Point<Pixels> {
        point(bounds.origin.x + px(x * scale), bounds.origin.y + px(y * scale))
    };
    let mut fill = |wedge: &Wedge, pressed: f32, color: Rgba| {
        if pressed <= 0. || color.a <= 0. {
            return;
        }
        let corners = wedge.corners(pressed);
        let mut path = PathBuilder::fill();
        path.move_to(to(corners[0]));
        for corner in &corners[1..] {
            path.line_to(to(*corner));
        }
        path.close();
        if let Ok(path) = path.build() {
            window.paint_path(path, color);
        }
    };
    let last = WEDGES.len() - 1;
    for (ix, wedge) in WEDGES.iter().enumerate() {
        // pressed into the window's gray; the second one then lights in the icon's own blue: the one beside you
        let lit = if ix == last { light } else { 0. };
        fill(wedge, pressed[ix], alpha(INK_3, 0.55 * pressed[ix] * (1. - lit)));
        fill(wedge, pressed[ix], alpha(MARK_BLUE, lit));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wedges_are_pressed_in_one_after_the_other_then_the_second_one_lights() {
        assert_eq!(mark_at(Duration::ZERO), ([0., 0.], 0.), "nothing yet");
        let (pressed, light) = mark_at(Duration::from_millis(400));
        assert!(pressed[0] > 0.9 && pressed[0] < 1., "the first is nearly in: {pressed:?}");
        assert!(pressed[1] > 0. && pressed[1] < pressed[0], "the second has begun: {pressed:?}");
        assert_eq!(light, 0., "the light waits");
        let (pressed, light) = mark_at(BETWEEN + PRESS);
        assert_eq!((pressed, light), ([1., 1.], 0.), "both are in before the light comes");
        let (pressed, light) = mark_at(Duration::from_millis(1240));
        assert_eq!(pressed, [1., 1.]);
        assert!(light > 0. && light < 1., "the second one is lighting: {light}");
        assert_eq!(mark_at(Duration::from_millis(1540)), ([1., 1.], 1.), "and then it rests, lit");
    }

    /// A wedge grows down out of its head: the head's two corners stay where they are, the others come down.
    #[test]
    fn a_wedge_grows_down_out_of_its_head() {
        let wedge = WEDGES[0];
        let (half, whole) = (wedge.corners(0.5), wedge.corners(1.));
        assert_eq!((half[0], half[5]), (whole[0], whole[5]), "the head's corners don't move");
        assert_eq!((half[0].1, half[5].1), (0., 0.), "and they are the wedge's top");
        for ix in 1..5 {
            assert_eq!(half[ix].0, whole[ix].0, "a corner comes straight down");
            assert_eq!(half[ix].1 * 2., whole[ix].1, "half pressed is half as far down");
        }
        assert_eq!(whole[2].1, 500., "pressed in, the tail ends 500 down");
    }

    /// The window's mark and the icon are one drawing: every wedge here is a path of `bundle/icon.svg`, corner
    /// for corner.
    #[test]
    fn it_is_the_icons_own_drawing() {
        let icon = include_str!("../../../bundle/icon.svg");
        let paths: Vec<&str> =
            icon.lines().filter_map(|l| l.trim().strip_prefix("<path d=\"")?.strip_suffix("\"/>")).collect();
        assert_eq!(paths.len(), WEDGES.len(), "the icon has as many wedges as the mark");
        for (wedge, path) in WEDGES.iter().zip(paths) {
            let ours = wedge.corners(1.).map(|(x, y)| format!("{} {}", x + ICON_ORIGIN.0, y + ICON_ORIGIN.1)).join("L");
            assert_eq!(format!("M{ours}Z"), path);
        }
        // and the view holds them all
        for wedge in WEDGES {
            for (x, y) in wedge.corners(1.) {
                assert!((0. ..=VIEW.0).contains(&x) && (0. ..=VIEW.1).contains(&y), "{x}, {y}");
            }
        }
        // the second wedge's blue is the icon's own
        assert!(icon.contains("fill=\"#4F7DFF\""), "the icon's second wedge is the mark's blue");
        assert_eq!(MARK_BLUE, crate::tokens::hex(0x4f7dff));
    }
}
