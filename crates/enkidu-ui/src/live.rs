//! Live's marks (DESIGN.md, "Live"): three playing bars; on a workspace's tab and in the palette, the small green
//! pill with the bars and the word "Live". The bars step on the lamps' clock. And the play button in the top row, at
//! one width in each of its five states.

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::base::{TestSupportExt as _, Transition, transition};
use gpui_kit::prelude::*;
use gpui_kit::{
    Animation, AnimationExt as _, App, ElementId, FontWeight, IntoElement, Role, SharedString, Transformation, Window,
    div, percentage, px, svg,
};

use crate::lamp::LAMP_FPS;
use crate::pieces::{Observed, Pressed};
use crate::press::pressable;
use crate::tokens::{
    CHIP, CONTROL, DUR_QUICK, HOVER, INK, INK_2, INK_3, LAMP_GREEN, LINE_STRONG, MARK, SOFT, TEXT_SMALL, TEXT_STATUS,
    alpha, bezier,
};

/// CSS's `ease-in-out`, the old bars' curve.
const CSS_EASE_IN_OUT: [f32; 4] = [0.42, 0.0, 0.58, 1.0];

/// Each bar: its cycle, where in it the bar starts and its height at rest.
const BARS: [(Duration, f32, f32); 3] = [
    (Duration::from_millis(1100), 0.0, 0.55),
    (Duration::from_millis(850), 400.0 / 850.0, 1.0),
    (Duration::from_millis(1300), 750.0 / 1300.0, 0.75),
];

/// A bar's height at `t` of its cycle, as a share of its height at rest: 0.4 at both ends, 1 in the middle.
pub fn bar_at(t: f32) -> f32 {
    let half = if t < 0.5 { t * 2.0 } else { 2.0 - t * 2.0 };
    0.4 + 0.6 * bezier(CSS_EASE_IN_OUT, half)
}

/// The three bars, `height` pixels tall. `id` names their motion.
pub fn live_bars(id: impl Into<ElementId>, height: f32) -> impl IntoElement {
    let id: ElementId = id.into();
    let bars = BARS.iter().enumerate().map(|(ix, &(cycle, start, rest))| {
        div().flex_none().w(px(2.5)).h(px(height * rest)).rounded(px(1.)).bg(LAMP_GREEN).with_animation(
            ElementId::NamedChild(Arc::new(id.clone()), SharedString::from(format!("bar-{ix}"))),
            Animation::new(cycle).repeat_synced().with_max_fps(LAMP_FPS),
            move |bar, t| bar.h(px(height * rest * bar_at((t + start) % 1.0))),
        )
    });
    div().flex_none().h(px(height)).flex().items_end().gap(px(2.)).children(bars)
}

/// Live on a row: the green pill with the bars and the word, or with `compact` the bars alone. It is labelled
/// "Live" either way, never color alone.
pub fn live_badge(id: impl Into<ElementId>, compact: bool) -> Observed {
    let id: ElementId = id.into();
    let bars = ElementId::NamedChild(Arc::new(id.clone()), SharedString::from("bars"));
    div()
        .id(id)
        .test_support()
        .aria_label("Live")
        .flex_none()
        .h(CHIP)
        .flex()
        .items_center()
        .gap(px(4.))
        .rounded_full()
        .bg(alpha(LAMP_GREEN, 0.14))
        .px(px(8.))
        .text_size(TEXT_STATUS)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(LAMP_GREEN)
        .child(live_bars(bars, 9.))
        .when(!compact, |pill| pill.child("Live"))
}

/// How the Live button looks: its five states (DESIGN.md, "Live").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveLook {
    /// A hairline pill with the green triangle: "Go live".
    Off,
    /// Green, with a spinner: its changes are being copied in.
    Starting,
    /// A green pill with the three playing bars; pointed at, a square and "Stop".
    Live,
    /// A hairline pill with a spinner: the main folder is being put back.
    Stopping,
    /// Dimmed: Live is switching in another workspace.
    Waiting,
}

/// The Live button's one width: "Go live", "Starting…", "Live" and "Stop" all fit, and nothing beside it moves when
/// Live changes.
pub const LIVE_BUTTON_WIDTH: f32 = 92.;

/// The Live button. `words` are what it says and `state` its state in a word (both for tests and the accessibility
/// tree too); `pointed`: the pointer is on it, which turns Live into Stop and grows Go live's triangle. Give it its
/// `on_click`, `on_hover` and tooltip.
pub fn live_button(
    id: impl Into<ElementId>,
    look: LiveLook,
    words: &'static str,
    state: &'static str,
    pointed: bool,
    window: &mut Window,
    cx: &mut App,
) -> Pressed {
    let id: ElementId = id.into();
    let part = |name: &'static str| ElementId::NamedChild(Arc::new(id.clone()), SharedString::from(name));
    let green = LAMP_GREEN;
    // it answers a press while it can be pressed: off or Live
    let pill = pressable(div().id(id.clone()))
        .test_support()
        .role(Role::Button)
        .aria_label(words)
        .aria_description(state)
        // pressed while it is Live
        .aria_toggled(if look == LiveLook::Live { gpui_kit::Toggled::True } else { gpui_kit::Toggled::False })
        .flex_none()
        .h(CONTROL)
        .w(px(LIVE_BUTTON_WIDTH))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .rounded_full()
        .border_1()
        .text_size(TEXT_SMALL)
        .font_weight(FontWeight::SEMIBOLD);
    // a clear edge for the calm states; a green one while it is Live or going Live
    let spinner = |color: gpui_kit::Rgba| {
        svg().flex_none().size(MARK).path("icons/loader-circle.svg").text_color(color).with_animation(
            part("spinner"),
            Animation::new(Duration::from_secs(1)).repeat_synced().with_max_fps(LAMP_FPS),
            |sign, t| sign.with_transformation(Transformation::rotate(percentage(t))),
        )
    };
    // the triangle turns green and grows a quarter when pointed at (DESIGN.md, "Live"); at rest it is gray: green
    // is for what is Live
    let play = |grown: bool, color: gpui_kit::Rgba, window: &mut Window, cx: &mut App| {
        let size = transition(part("play"), if grown { 12. } else { 10. }, Transition::new(DUR_QUICK), window, cx);
        div()
            .flex_none()
            .size(MARK)
            .flex()
            .items_center()
            .justify_center()
            .child(svg().flex_none().size(px(size)).path("icons/play-fill.svg").text_color(color))
    };
    match look {
        LiveLook::Off => pill
            .border_color(LINE_STRONG)
            .text_color(if pointed { INK } else { INK_2 })
            .cursor_pointer()
            .hover(|s| s.bg(HOVER))
            .active(|s| s.bg(SOFT))
            .child(play(pointed, if pointed { green } else { INK_2 }, window, cx))
            .child(words),
        LiveLook::Waiting => pill
            .border_color(LINE_STRONG)
            .text_color(INK_3)
            .opacity(0.6)
            .cursor_default()
            .child(play(false, INK_3, window, cx))
            .child(words),
        LiveLook::Starting => pill
            .border_color(alpha(green, 0.38))
            .bg(alpha(green, 0.10))
            .text_color(green)
            .cursor_default()
            .child(spinner(green))
            .child(words),
        LiveLook::Stopping => {
            pill.border_color(LINE_STRONG).text_color(INK_2).cursor_default().child(spinner(INK_2)).child(words)
        }
        LiveLook::Live => pill
            .border_color(alpha(green, 0.38))
            .bg(alpha(green, 0.14))
            .text_color(green)
            .cursor_pointer()
            .hover(|s| s.bg(alpha(green, 0.22)))
            .active(|s| s.bg(alpha(green, 0.28)))
            .map(|pill| match pointed {
                // pointing at it turns it into Stop, in the same green
                true => pill.child(svg().flex_none().size(px(8.)).path("icons/square-fill.svg").text_color(green)),
                false => pill.child(live_bars(part("bars"), 10.)),
            })
            .child(words),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bar_breathes_between_two_fifths_and_its_full_height() {
        assert!((bar_at(0.0) - 0.4).abs() < 1e-4);
        assert!((bar_at(1.0) - 0.4).abs() < 1e-4);
        assert!((bar_at(0.5) - 1.0).abs() < 1e-4);
        assert!(bar_at(0.25) > 0.4 && bar_at(0.25) < 1.0);
        assert!((bar_at(0.25) - bar_at(0.75)).abs() < 1e-4, "it falls as it rose");
    }
}
