//! Words for something in progress ("Claude is working") shimmer, as if light passed over them (DESIGN.md, "Motion").
//! GPUI can't paint text with a gradient, so each letter takes the color the old gradient gave it at its place. It
//! steps on the lamps' clock.

use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::{
    Animation, AnimationExt as _, ElementId, HighlightStyle, IntoElement, Rgba, SharedString, StyledText, div,
};

use crate::lamp::LAMP_FPS;
use crate::tokens::{INK, INK_2};

/// One pass of the light.
pub const PASS: Duration = Duration::from_millis(2600);

/// How bright the light makes a letter at `x` (0 where the words start, 1 where they end) at `t` of a pass: the old
/// gradient was 2.6 times the words' width and slid from right to left, so its bright middle crosses from -0.3 to
/// 1.3 and lights 0.312 of the width on each side, fading in a straight line.
pub fn shimmer_at(x: f32, t: f32) -> f32 {
    let center = -0.3 + 1.6 * t;
    (1.0 - (x - center).abs() / 0.312).max(0.0)
}

/// A letter's color at brightness `k`: from `INK_2` toward `INK`.
fn lit(k: f32) -> Rgba {
    Rgba {
        r: INK_2.r + (INK.r - INK_2.r) * k,
        g: INK_2.g + (INK.g - INK_2.g) * k,
        b: INK_2.b + (INK.b - INK_2.b) * k,
        a: 1.0,
    }
}

/// The shimmering words. `id` names their motion.
pub fn shimmer(id: impl Into<ElementId>, words: impl Into<SharedString>) -> impl IntoElement {
    let words: SharedString = words.into();
    div().with_animation(id.into(), Animation::new(PASS).repeat_synced().with_max_fps(LAMP_FPS), move |line, t| {
        let count = words.chars().count().max(1) as f32;
        let colors: Vec<_> = words
            .char_indices()
            .enumerate()
            .map(|(ix, (at, ch))| {
                let color = lit(shimmer_at((ix as f32 + 0.5) / count, t));
                (at..at + ch.len_utf8(), HighlightStyle { color: Some(color.into()), ..Default::default() })
            })
            .collect();
        line.child(StyledText::new(words.clone()).with_highlights(colors))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_light_crosses_the_words_once_a_pass() {
        // at the start and the end of a pass the light's edge only touches the words' ends
        assert!(shimmer_at(0.0, 0.0) < 0.05 && shimmer_at(1.0, 1.0) < 0.05);
        assert_eq!(shimmer_at(0.5, 0.0), 0.0);
        // halfway through, it is on the middle of the words, fully, and fades toward the ends
        assert!((shimmer_at(0.5, 0.5) - 1.0).abs() < 1e-4);
        assert!(shimmer_at(0.35, 0.5) > 0.0 && shimmer_at(0.35, 0.5) < 1.0);
        assert_eq!(shimmer_at(0.0, 0.5), 0.0, "0.5 of the width from the light is dark");
        // the light moves left to right
        assert!(shimmer_at(0.2, 0.3) > shimmer_at(0.2, 0.6));
        assert_eq!(lit(0.0), INK_2);
    }
}
