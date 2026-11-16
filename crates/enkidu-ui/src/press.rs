//! A press is answered (DESIGN.md, "Motion": "A press: scale 0.97, 120 ms: the button heard you"). What is pressed is
//! drawn 3 % smaller for as long as the mouse is down on it, and comes back when it is let go, each over 120 ms.
//!
//! GPUI scales pictures only, never an element, so the box itself gets smaller: its size, with as much margin
//! added and as much padding taken as it lost, so nothing beside it moves and what it holds stays where it was.
//! The press is told to the box as it is laid out, by a wrapper that adds no node of its own.
//!
//! A box that shrinks under the pointer could lose the click: the mouse comes up outside it. So it never shrinks
//! past where the pointer went down (three pixels are left around it).

use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

use gpui_kit::{
    AbsoluteLength, AnyElement, App, Bounds, CursorStyle, DefiniteLength, DispatchPhase, Element, ElementId,
    GlobalElementId, Hitbox, InspectorElementId, InteractiveElement, Interactivity, IntoElement, LayoutId, Length,
    MouseButton, MouseDownEvent, MouseUpEvent, ParentElement, Pixels, Point, Size, StatefulInteractiveElement,
    StyleRefinement, Styled, Window, px,
};

use crate::tokens::{DUR_PRESS, EASE_OUT, PRESS_SCALE, bezier};

/// The pointer keeps this much of the box around it, so the mouse comes up inside what it went down on.
const KEPT_AROUND_THE_POINTER: f32 = 3.;

/// Makes `inner` answer a press. It needs an id: the press is kept with the element, between frames.
pub fn pressable<E>(inner: E) -> Press<E>
where
    E: Element<PrepaintState = Option<Hitbox>> + InteractiveElement + Styled,
{
    assert!(Element::id(&inner).is_some(), "what answers a press needs an id");
    Press { inner }
}

/// An element that answers a press. It forwards everything to what it holds and adds no layout node.
///
/// A control that says it can't be clicked now doesn't answer: that is the plain arrow, `cursor_default()`, which every
/// button that is off sets with its dimming. Something that takes clicks under the plain arrow (a card) sets no cursor
/// at all.
pub struct Press<E> {
    inner: E,
}

impl<E: Styled> Press<E> {
    /// It says it can't be clicked now.
    fn off(&mut self) -> bool {
        self.inner.style().mouse_cursor == Some(CursorStyle::Arrow)
    }
}

/// A press as it goes: kept with the element.
#[derive(Default)]
struct Motion {
    down: Cell<bool>,
    /// How far in it was (0 at rest, 1 fully in) when the mouse last went down or came up, and when that was.
    from: Cell<f32>,
    since: Cell<Option<Instant>>,
    /// The smallest scale the pointer's place allows.
    floor: Cell<f32>,
    /// Its size at rest, as last drawn.
    rest: Cell<Option<Size<Pixels>>>,
}

impl Motion {
    /// How far in it is: 0 at rest, 1 fully in.
    fn amount(&self, now: Instant) -> f32 {
        amount_at(self.down.get(), self.from.get(), self.since.get().map(|since| now.saturating_duration_since(since)))
    }

    fn moving(&self, now: Instant) -> bool {
        self.since.get().is_some_and(|since| now.saturating_duration_since(since) < DUR_PRESS)
    }

    fn turn(&self, down: bool, now: Instant) {
        self.from.set(self.amount(now));
        self.since.set(Some(now));
        self.down.set(down);
    }
}

/// How far in a press is, `elapsed` after the mouse went down (or came up) at `from`.
fn amount_at(down: bool, from: f32, elapsed: Option<std::time::Duration>) -> f32 {
    let target = if down { 1. } else { 0. };
    let Some(elapsed) = elapsed else { return target };
    let progress = elapsed.as_secs_f32() / DUR_PRESS.as_secs_f32();
    match progress {
        p if p <= 0. => from,
        p if p >= 1. => target,
        p => from + (target - from) * bezier(EASE_OUT, p),
    }
}

/// The scale of a box pressed `amount` far in, never under `floor`.
fn scale_at(amount: f32, floor: f32) -> f32 {
    (1. - (1. - PRESS_SCALE) * amount).max(floor).min(1.)
}

/// The smallest scale that leaves the pointer inside a box of `rest` size around `center`.
fn floor_for(rest: Size<Pixels>, center: Point<Pixels>, pointer: Point<Pixels>) -> f32 {
    let needs = |half: Pixels, from_center: Pixels| {
        let half = f32::from(half);
        if half <= 0. { 1. } else { ((f32::from(from_center).abs() + KEPT_AROUND_THE_POINTER) / half).min(1.) }
    };
    needs(rest.width / 2., pointer.x - center.x).max(needs(rest.height / 2., pointer.y - center.y)).max(PRESS_SCALE)
}

fn pixels(length: &DefiniteLength, rem: Pixels) -> Option<Pixels> {
    match length {
        DefiniteLength::Absolute(AbsoluteLength::Pixels(pixels)) => Some(*pixels),
        DefiniteLength::Absolute(AbsoluteLength::Rems(rems)) => Some(rems.to_pixels(rem)),
        DefiniteLength::Fraction(_) => None,
    }
}

/// Makes the box `scale` of its `rest` size and keeps its place: the margin grows by what the box lost on each
/// side, and the padding gives up as much, so what it holds doesn't move.
fn squeeze(style: &mut StyleRefinement, rest: Size<Pixels>, scale: f32, rem: Pixels) {
    let lost = (rest.width * (1. - scale) / 2., rest.height * (1. - scale) / 2.);
    let (width, height) = (Length::from(rest.width - lost.0 * 2.), Length::from(rest.height - lost.1 * 2.));
    for size in [&mut style.size, &mut style.min_size, &mut style.max_size] {
        size.width = Some(width);
        size.height = Some(height);
    }
    style.flex_grow = Some(0.);
    style.flex_shrink = Some(0.);
    let margin = &mut style.margin;
    for (side, by) in [
        (&mut margin.left, lost.0),
        (&mut margin.right, lost.0),
        (&mut margin.top, lost.1),
        (&mut margin.bottom, lost.1),
    ] {
        match side {
            None => *side = Some(by.into()),
            // a margin that takes what is left (`ml_auto`) keeps doing that
            Some(Length::Auto) => {}
            Some(Length::Definite(length)) => {
                if let Some(now) = pixels(length, rem) {
                    *side = Some((now + by).into());
                }
            }
        }
    }
    let padding = &mut style.padding;
    for (side, by) in [
        (&mut padding.left, lost.0),
        (&mut padding.right, lost.0),
        (&mut padding.top, lost.1),
        (&mut padding.bottom, lost.1),
    ] {
        if let Some(now) = side.as_ref().and_then(|length| pixels(length, rem)) {
            *side = Some((now - by).max(px(0.)).into());
        }
    }
}

impl<E> Press<E> {
    fn motion(id: Option<&GlobalElementId>, window: &mut Window) -> Option<Rc<Motion>> {
        let id = id?;
        Some(window.with_element_state(id, |motion: Option<Rc<Motion>>, _| {
            let motion = motion.unwrap_or_default();
            (motion.clone(), motion)
        }))
    }
}

impl<E> IntoElement for Press<E>
where
    E: Element<PrepaintState = Option<Hitbox>> + InteractiveElement + Styled,
{
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl<E> Element for Press<E>
where
    E: Element<PrepaintState = Option<Hitbox>> + InteractiveElement + Styled,
{
    type RequestLayoutState = E::RequestLayoutState;
    type PrepaintState = Option<Hitbox>;

    fn id(&self) -> Option<ElementId> {
        Element::id(&self.inner)
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        self.inner.source_location()
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        // reduced motion: nothing moves
        if !self.off()
            && !cx.reduce_motion()
            && let Some(motion) = Self::motion(id, window)
            && let Some(rest) = motion.rest.get()
        {
            let scale = scale_at(motion.amount(cx.background_executor().now()), motion.floor.get());
            if scale < 1. {
                squeeze(self.inner.style(), rest, scale, window.rem_size());
            }
        }
        self.inner.request_layout(id, inspector, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        if let Some(motion) = Self::motion(id, window) {
            // its own size is the one it has with no press on it
            let now = cx.background_executor().now();
            if !motion.down.get() && !motion.moving(now) {
                motion.since.set(None);
                motion.rest.set(Some(bounds.size));
            }
        }
        self.inner.prepaint(id, inspector, bounds, layout, window, cx)
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut Self::RequestLayoutState,
        hitbox: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.inner.paint(id, inspector, bounds, layout, hitbox, window, cx);
        if self.off() || cx.reduce_motion() {
            return;
        }
        let (Some(motion), Some(hitbox)) = (Self::motion(id, window), hitbox.clone()) else { return };
        if motion.moving(cx.background_executor().now()) {
            window.request_animation_frame();
        }
        let center = bounds.center();
        window.on_mouse_event({
            let motion = motion.clone();
            move |event: &MouseDownEvent, phase, window, cx| {
                let on_it = phase == DispatchPhase::Bubble && event.button == MouseButton::Left;
                if on_it && !motion.down.get() && hitbox.is_hovered(window) {
                    let rest = motion.rest.get().unwrap_or(hitbox.bounds.size);
                    motion.floor.set(floor_for(rest, center, event.position));
                    motion.turn(true, cx.background_executor().now());
                    window.refresh();
                }
            }
        });
        window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
            // wherever it comes up: a press that left the button ends too
            if phase == DispatchPhase::Capture && event.button == MouseButton::Left && motion.down.get() {
                motion.turn(false, cx.background_executor().now());
                window.refresh();
            }
        });
    }

    fn a11y_role(&self) -> Option<gpui_kit::Role> {
        self.inner.a11y_role()
    }

    fn write_a11y_info(&self, node: &mut gpui_kit::accesskit::Node) {
        self.inner.write_a11y_info(node);
    }

    fn a11y_synthetic_children(&mut self, paint: &mut Self::PrepaintState, builder: &mut gpui_kit::A11ySubtreeBuilder) {
        self.inner.a11y_synthetic_children(paint, builder);
    }
}

impl<E: Styled> Styled for Press<E> {
    fn style(&mut self) -> &mut StyleRefinement {
        self.inner.style()
    }
}

impl<E: InteractiveElement> InteractiveElement for Press<E> {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.inner.interactivity()
    }
}

impl<E: StatefulInteractiveElement> StatefulInteractiveElement for Press<E> {}

impl<E: ParentElement> ParentElement for Press<E> {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.inner.extend(elements);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{point, size};
    use std::time::Duration;

    #[test]
    fn a_press_goes_in_over_120_ms_and_comes_back_as_fast() {
        // DESIGN.md's row: "A press (`press::pressable`) | scale 0.97, 120 ms"
        assert_eq!((PRESS_SCALE, DUR_PRESS), (0.97, Duration::from_millis(120)));
        let ms = |ms| Some(Duration::from_millis(ms));
        // down: from rest to fully in, most of the way early (the strong ease-out), there at 120 ms and after
        assert_eq!(amount_at(true, 0., ms(0)), 0.);
        assert!(amount_at(true, 0., ms(40)) > 0.7, "{}", amount_at(true, 0., ms(40)));
        assert_eq!((amount_at(true, 0., ms(120)), amount_at(true, 0., ms(900))), (1., 1.));
        assert_eq!(scale_at(amount_at(true, 0., ms(120)), PRESS_SCALE), 0.97);
        // up: back from where it was, also when it was let go halfway in
        assert_eq!((amount_at(false, 1., ms(0)), amount_at(false, 1., ms(120))), (1., 0.));
        assert!(amount_at(false, 0.5, ms(40)) < 0.15 && amount_at(false, 0.5, ms(120)) == 0.);
        // nothing going on: where it is
        assert_eq!((amount_at(false, 0.4, None), amount_at(true, 0.4, None)), (0., 1.));
    }

    #[test]
    fn the_box_never_shrinks_past_the_pointer() {
        let rest = size(px(100.), px(28.));
        let center = point(px(200.), px(100.));
        // in the middle: the whole 3 %
        assert_eq!(floor_for(rest, center, center), PRESS_SCALE);
        assert_eq!(floor_for(rest, center, point(px(230.), px(104.))), PRESS_SCALE);
        // at its right edge: it stays as it is, so the mouse comes up on it
        assert_eq!(floor_for(rest, center, point(px(249.), px(100.))), 1.);
        // 6 px from the edge: the edge may come 3 px closer, no more (44 + 3 of 50)
        assert!((floor_for(rest, center, point(px(244.), px(100.))) - 0.94_f32.max(PRESS_SCALE)).abs() < 1e-3);
        assert!((floor_for(rest, center, point(px(246.), px(100.))) - 0.98).abs() < 1e-3);
        // near the top of a low button the height decides
        assert_eq!(floor_for(rest, center, point(px(200.), px(87.))), 1.);
        assert_eq!(scale_at(1., 0.98), 0.98);
    }

    #[test]
    fn a_squeezed_box_keeps_its_place_and_what_it_holds() {
        let rem = px(13.);
        let mut style = StyleRefinement::default();
        style.padding.left = Some(px(12.).into());
        style.padding.right = Some(px(12.).into());
        style.margin.left = Some(px(4.).into());
        style.margin.right = Some(Length::Auto);
        squeeze(&mut style, size(px(100.), px(28.)), 0.97, rem);
        let of = |length: Option<Length>| match length {
            Some(Length::Definite(length)) => pixels(&length, rem).map(f32::from),
            _ => None,
        };
        let near = |length: Option<Length>, want: f32| of(length).is_some_and(|got| (got - want).abs() < 1e-3);
        // 3 % smaller: 97 by 27.16, held there whatever its own rules were
        assert!(near(style.size.width, 97.) && near(style.size.height, 27.16));
        assert!(near(style.min_size.width, 97.) && near(style.max_size.width, 97.));
        assert!(near(style.min_size.height, 27.16) && near(style.max_size.height, 27.16));
        assert_eq!((style.flex_grow, style.flex_shrink), (Some(0.), Some(0.)));
        // what it lost on each side is margin now (1.5 and 0.42 px), on top of its own; an auto margin stays auto
        assert!(near(style.margin.left, 5.5), "{:?}", style.margin.left);
        assert_eq!(style.margin.right, Some(Length::Auto));
        assert!(near(style.margin.top, 0.42) && near(style.margin.bottom, 0.42));
        // and its padding gave up as much, so its words stay where they were; none is never less than none
        let padded = |length: Option<DefiniteLength>| length.and_then(|length| pixels(&length, rem)).map(f32::from);
        assert!(padded(style.padding.left).is_some_and(|got| (got - 10.5).abs() < 1e-3));
        assert!(padded(style.padding.right).is_some_and(|got| (got - 10.5).abs() < 1e-3));
        assert_eq!((style.padding.top, style.padding.bottom), (None, None));
        let mut bare = StyleRefinement::default();
        bare.padding.left = Some(px(1.).into());
        squeeze(&mut bare, size(px(100.), px(28.)), 0.97, rem);
        assert_eq!(bare.padding.left, Some(px(0.).into()));
    }
}
