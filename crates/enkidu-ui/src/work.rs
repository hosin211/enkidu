//! Small pieces of the Work page (DESIGN.md, "The Work page"): a source's lamp, a status's ring, a ticket's type and
//! priority, a PR's glyph, the badge of what waits on you, and a card's own surface.

use std::time::Duration;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, BoxShadow, Div, ElementId, FontWeight, IntoElement, PathBuilder, Pixels,
    Point, Rgba, SharedString, Stateful, Svg, canvas, div, point, px, svg,
};

use crate::lamp::LAMP_FPS;
use crate::pieces::{Observed, border_width};
use crate::tokens::{self, EASE_POP, alpha, bezier, hsla, tone};

/// How a source (GitHub, Jira) stands, as its lamp shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceLook {
    /// Read: green.
    Ok,
    /// Can't be read: red.
    Error,
    /// Being read now: amber, breathing.
    Reading,
    /// Not read yet, or not read for any project: hollow.
    Off,
}

/// One breath of a lamp that is reading.
pub const BREATH: Duration = Duration::from_secs(2);

/// How bright a reading lamp is at `t` of its breath: full, half in the middle, full again.
pub fn breath_at(t: f32) -> f32 {
    0.75 + 0.25 * (t * std::f32::consts::TAU).cos()
}

/// A source's lamp, `size` px wide: green read, amber reading, red can't read, hollow off. `id` names its motion.
pub fn source_lamp(id: impl Into<ElementId>, look: SourceLook, size: f32) -> impl IntoElement {
    div().flex_none().size(px(size)).child(lamp_dot(id, look))
}

/// A PR's checks as a dot, `size` px wide, in a lamp's colors (green passed, amber running, red failing). It pops
/// once each time it flips while you look at it (DESIGN.md, "Motion with a job"): `flips` counts them, 0 is still.
/// The row around it keeps its place while it pops.
pub fn checks_dot(id: impl Into<ElementId>, look: SourceLook, size: f32, flips: u32) -> AnyElement {
    let id: ElementId = id.into();
    let part = |name: String| ElementId::NamedChild(std::sync::Arc::new(id.clone()), SharedString::from(name));
    let stands = match look {
        SourceLook::Ok => "passed",
        SourceLook::Reading => "running",
        SourceLook::Error => "failing",
        SourceLook::Off => "none",
    };
    let dot = div()
        .id(id.clone())
        .test_support()
        .aria_label(stands)
        .flex_none()
        .size(px(size))
        .child(lamp_dot(part("lamp".into()), look));
    let place = div().flex_none().size(px(size)).flex().items_center().justify_center();
    match flips {
        0 => place.child(dot).into_any_element(),
        _ => place
            .child(dot.with_animation(
                part(format!("flip-{flips}")),
                Animation::new(tokens::DUR_SLOW).with_easing(|t| bezier(EASE_POP, t)),
                move |dot, t| dot.size(px(size * (0.4 + 0.6 * t))).opacity(t.clamp(0., 1.)),
            ))
            .into_any_element(),
    }
}

/// The lamp itself, as large as what holds it.
fn lamp_dot(id: impl Into<ElementId>, look: SourceLook) -> AnyElement {
    let dot = div().size_full().rounded_full();
    match look {
        SourceLook::Ok => dot.bg(tokens::LAMP_GREEN).into_any_element(),
        SourceLook::Error => dot.bg(tokens::LAMP_RED).into_any_element(),
        SourceLook::Off => border_width(dot.border_color(tokens::LAMP_OFF), 1.5).into_any_element(),
        // it breathes in steps, on the one clock every lamp shares, so it costs what a lamp costs
        SourceLook::Reading => dot
            .bg(tokens::LAMP_AMBER)
            .with_animation(id, Animation::new(BREATH).repeat_synced().with_max_fps(LAMP_FPS), |dot, t| {
                dot.opacity(breath_at(t))
            })
            .into_any_element(),
    }
}

/// How a status is drawn at the head of its column.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StatusRing {
    /// To do: a hollow ring.
    Todo,
    /// In progress: a ring that fills as the work moves right, by this much of a turn (0 to 1).
    Progress(f32),
    /// On hold: a ring with a pause in it.
    Hold,
    /// Done: a disc with a check.
    Done,
}

/// A status as a small ring, `size` px wide, in its hue (none: gray): hollow to do, filling as the work moves right,
/// paused on hold, a check when done.
pub fn status_ring(ring: StatusRing, hue: Option<f32>, size: f32) -> impl IntoElement {
    let color = hue.map_or(tokens::INK_3, |hue| tone(hue, 0.14));
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            // the old drawing's 14 by 14 view
            let scale = bounds.size.width / px(14.);
            let to = |x: f32, y: f32| -> Point<Pixels> {
                point(bounds.origin.x + px(x * scale), bounds.origin.y + px(y * scale))
            };
            let circle = |path: &mut PathBuilder, r: f32| {
                let radii = point(px(r * scale), px(r * scale));
                path.move_to(to(7. + r, 7.));
                path.arc_to(radii, px(0.), false, true, to(7. - r, 7.));
                path.arc_to(radii, px(0.), false, true, to(7. + r, 7.));
                path.close();
            };
            let mut paint = |path: PathBuilder, color: Rgba| {
                if let Ok(path) = path.build() {
                    window.paint_path(path, color);
                }
            };
            if ring == StatusRing::Done {
                let mut disc = PathBuilder::fill();
                circle(&mut disc, 6.);
                paint(disc, color);
                let mut check = PathBuilder::stroke(px(1.7 * scale));
                check.move_to(to(4.3, 7.2));
                check.line_to(to(6.1, 9.0));
                check.line_to(to(9.7, 5.2));
                paint(check, tokens::SHEET);
                return;
            }
            let mut edge = PathBuilder::stroke(px(1.5 * scale));
            circle(&mut edge, 5.4);
            paint(edge, color);
            match ring {
                StatusRing::Progress(fill) if fill > 0. => {
                    // a wedge from twelve o'clock, clockwise
                    let r = 3.2;
                    let angle = fill.clamp(0., 0.999) * std::f32::consts::TAU;
                    let mut wedge = PathBuilder::fill();
                    wedge.move_to(to(7., 7.));
                    wedge.line_to(to(7., 7. - r));
                    let radii = point(px(r * scale), px(r * scale));
                    wedge.arc_to(radii, px(0.), fill > 0.5, true, to(7. + r * angle.sin(), 7. - r * angle.cos()));
                    wedge.close();
                    paint(wedge, color);
                }
                StatusRing::Hold => {
                    for x in [5., 7.7] {
                        let mut bar = PathBuilder::fill();
                        bar.add_polygon(&[to(x, 4.6), to(x + 1.3, 4.6), to(x + 1.3, 9.4), to(x, 9.4)], true);
                        paint(bar, color);
                    }
                }
                _ => {}
            }
        },
    )
    .flex_none()
    .size(px(size))
}

/// An icon in a color of its own.
pub fn tinted(icon: &'static str, color: Rgba, size: f32) -> Svg {
    svg().path(icon).flex_none().size(px(size)).text_color(color)
}

/// A ticket's type as Jira colors it: a bug red, a task blue, a story green, a sub-task, an epic violet, a problem
/// amber; anything else a gray ring. `kind` is one of bug, task, story, subtask, epic, problem.
pub fn type_icon(kind: &str, size: f32) -> Svg {
    let (icon, hue) = match kind {
        "bug" => ("icons/bug.svg", Some(25.)),
        "task" => ("icons/square-check.svg", Some(235.)),
        "story" => ("icons/bookmark.svg", Some(150.)),
        "subtask" => ("icons/corner-down-right.svg", Some(215.)),
        "epic" => ("icons/zap.svg", Some(300.)),
        "problem" => ("icons/triangle-alert.svg", Some(55.)),
        _ => ("icons/circle.svg", None),
    };
    tinted(icon, hue.map_or(tokens::INK_3, |hue| tone(hue, 0.15)), size)
}

/// A priority worth a mark: high ones point up in orange (1) or red (2), low ones down in blue (-1, -2).
pub fn priority_icon(level: i8, size: f32) -> Svg {
    let (icon, color) = match level {
        2.. => ("icons/chevrons-up.svg", tokens::LAMP_RED),
        1 => ("icons/chevron-up.svg", tone(50., 0.16)),
        -1 | 0 => ("icons/chevron-down.svg", tone(240., 0.1)),
        _ => ("icons/chevrons-down.svg", tone(240., 0.1)),
    };
    tinted(icon, color, size)
}

/// A PR's state as GitHub draws it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrGlyph {
    Open,
    Draft,
    Merged,
    Closed,
}

pub fn pr_glyph(state: PrGlyph, size: f32) -> Svg {
    let (icon, color) = match state {
        PrGlyph::Open => ("icons/git-pull-request.svg", tokens::LAMP_GREEN),
        PrGlyph::Draft => ("icons/git-pull-request-draft.svg", tokens::INK_3),
        PrGlyph::Merged => ("icons/git-merge.svg", tone(300., 0.14)),
        PrGlyph::Closed => ("icons/git-pull-request-closed.svg", tokens::INK_3),
    };
    tinted(icon, color, size)
}

/// The color of a PR's state in words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ink {
    Red,
    Amber,
    Green,
    Violet,
    Muted,
}

impl Ink {
    pub fn color(self) -> Rgba {
        match self {
            Ink::Red => tokens::LAMP_RED,
            Ink::Amber => tokens::LAMP_AMBER,
            Ink::Green => tokens::LAMP_GREEN,
            Ink::Violet => tone(300., 0.14),
            Ink::Muted => tokens::INK_3,
        }
    }
}

fn glow(color: Rgba, blur: f32, spread: f32, y: f32) -> BoxShadow {
    BoxShadow {
        color: hsla(color),
        offset: point(px(0.), px(y)),
        blur_radius: px(blur),
        spread_radius: px(spread),
        inset: false,
    }
}

/// The badge on a card that counts what waits on you: red when something blocks, blue otherwise. It pops in each
/// time its count changes (`id` names it; the count is part of the motion's name).
pub fn needs_badge(id: impl Into<ElementId>, count: usize, blocked: bool, label: impl Into<SharedString>) -> Observed {
    let color = if blocked { tokens::LAMP_RED } else { tokens::ACCENT_STRONG };
    let pill = crate::pieces::tabular(div())
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(color)
        .text_color(tokens::white(1.0))
        .font_weight(FontWeight::BOLD)
        .child(count.to_string())
        .with_animation(
            SharedString::from(format!("needs-pop-{count}")),
            Animation::new(tokens::DUR_SLOW).with_easing(|t| bezier(EASE_POP, t)),
            |pill, t| {
                let scale = 0.5 + 0.5 * t;
                pill.h(px(16. * scale))
                    .min_w(px(16. * scale))
                    .px(px(4. * scale))
                    .text_size(px(11. * scale))
                    .opacity(t.clamp(0., 1.))
            },
        );
    div()
        .id(id)
        .test_support()
        .aria_label(label)
        .flex_none()
        .h(tokens::BADGE)
        .min_w(tokens::BADGE)
        .flex()
        .items_center()
        .justify_center()
        .child(pill)
}

/// A person as their initials on a hue of their own, `size` px across: the same person, the same color, everywhere. The
/// row it is in says who it is.
pub fn person_badge(initials: impl Into<SharedString>, hue: f32, size: f32) -> Div {
    let (top, bottom) = (tokens::oklch(0.66, 0.13, hue), tokens::oklch(0.55, 0.13, hue));
    div()
        .flex_none()
        .size(px(size))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(gpui_kit::linear_gradient(
            180.,
            gpui_kit::linear_color_stop(top, 0.),
            gpui_kit::linear_color_stop(bottom, 1.),
        ))
        .text_size(px((size * 0.45).round()))
        .line_height(px(size))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(tokens::white(1.0))
        .child(initials.into())
}

/// A color on the way from `from` to `to`: `t` of 0 is `from`, 1 is `to`.
fn between(from: Rgba, to: Rgba, t: f32) -> Rgba {
    let at = |a: f32, b: f32| a + (b - a) * t.clamp(0., 1.);
    Rgba { r: at(from.r, to.r), g: at(from.g, to.g), b: at(from.b, to.b), a: at(from.a, to.a) }
}

/// How long a card that moved to another column glows.
pub const CARD_MOVED: std::time::Duration = std::time::Duration::from_millis(1600);

/// A card that just moved to another column, `t` of the way through its glow (DESIGN.md, "Motion with a job": "a
/// card that moved to another column glows once, so you see it move"): it starts tinted signal blue with a blue edge
/// and a faint ring, and settles into a card like the others.
pub fn card_moved<E: Styled>(card: E, t: f32) -> E {
    if t >= 1. {
        return card;
    }
    let tinted = between(tokens::PANE, tokens::ACCENT, 0.16);
    card.bg(between(tinted, tokens::PANE, t))
        .border_color(between(alpha(tokens::ACCENT, 0.55), tokens::LINE, t))
        .shadow(vec![glow(alpha(tokens::ACCENT, 0.12 * (1. - t)), 0., 3., 0.)])
}

/// A card's own surface on the board: a hair above the board's black, with a clear edge and no shadow; the one the
/// keyboard is on has the keyboard's blue edge; a dimmed one comes from a source that can't be read now.
pub fn card_surface(id: impl Into<ElementId>, selected: bool, dimmed: bool) -> Stateful<Div> {
    div()
        .id(id)
        .relative()
        .w_full()
        .flex_none()
        .rounded(tokens::RADIUS_CARD)
        .bg(tokens::PANE)
        .border_1()
        .when(dimmed, |card| card.opacity(0.6))
        .map(|card| match selected {
            true => card.border_color(tokens::KEY_EDGE),
            false => card
                .border_color(tokens::LINE)
                .hover(|card| card.bg(tokens::CARD_HOVER).border_color(tokens::LINE_STRONG)),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reading_lamp_breathes_to_half_and_back() {
        assert!((breath_at(0.) - 1.).abs() < 1e-6);
        assert!((breath_at(0.5) - 0.5).abs() < 1e-6);
        assert!((breath_at(1.) - 1.).abs() < 1e-5);
        assert!(breath_at(0.25) > 0.5 && breath_at(0.25) < 1.);
    }
}
