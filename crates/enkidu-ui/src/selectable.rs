//! Text you can select with the mouse and copy (items 7.5 and 7.5a).
//!
//! GPUI's text can't be selected by itself. gpui-kit keeps one selection for the window (a drag marks it, ⌘C
//! copies it, a press elsewhere lets it go); a text takes part by telling it, as it is drawn, what it says and
//! where its letters are. gpui-kit's own piece for that takes plain words only; this one takes any text that is
//! laid out by GPUI, so words with marks (bold, code, links) can be selected too. Each piece takes part on its
//! own, and a drag goes on from one piece into the next in reading order: down the window, then along the line.
//! The terminal and the diffs have their own selections.

use gpui_kit::base::{TextSelectionHandle, TextSelectionRegistration, TextSelectionRun};
use gpui_kit::{
    AnyElement, App, Bounds, Element, ElementId, GlobalElementId, Hitbox, HitboxBehavior, InspectorElementId,
    IntoElement, LayoutId, Pixels, SharedString, StyledText, TextLayout, Window, fill, point,
};

use crate::tokens::{ACCENT, alpha, hsla};

/// How strong the selection's blue is behind the letters: as in the terminal.
const MARK: f32 = 0.25;

/// Words that can be selected and copied: `styled` is how they are drawn (plain, or with runs).
pub fn selectable(id: impl Into<ElementId>, text: impl Into<SharedString>, styled: StyledText) -> Selectable {
    let layout = styled.layout().clone();
    Selectable { id: id.into(), text: text.into(), layout, drawn: styled.into_any_element(), order: 0 }
}

/// The same for words drawn by something around the text (links that hear a click): `layout` is the text's own,
/// taken from it before it was wrapped, and `drawn` is what draws it.
pub fn selectable_as(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    layout: TextLayout,
    drawn: impl IntoElement,
) -> Selectable {
    Selectable { id: id.into(), text: text.into(), layout, drawn: drawn.into_any_element(), order: 0 }
}

/// Plain words that can be selected and copied, drawn in the face, size and color of where they stand.
pub fn words(id: impl Into<ElementId>, text: impl Into<SharedString>) -> Selectable {
    let text: SharedString = text.into();
    selectable(id, text.clone(), StyledText::new(text))
}

/// One piece of selectable text. It forwards its layout to what draws the text and adds no layout node.
pub struct Selectable {
    id: ElementId,
    text: SharedString,
    layout: TextLayout,
    drawn: AnyElement,
    /// Where it stands in reading order, known once it is placed.
    order: u64,
}

/// A piece's place in reading order from where it is drawn: down the window first, then along the line.
pub fn reading_order(bounds: Bounds<Pixels>) -> u64 {
    let whole = |at: Pixels| f32::from(at).round().clamp(0., 1_000_000.) as u64;
    whole(bounds.top()) * 1_000_000 + whole(bounds.left())
}

impl IntoElement for Selectable {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Selectable {
    type RequestLayoutState = TextSelectionHandle;
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        // its part in the window's selection is kept for as long as it is drawn frame after frame
        let handle = match id {
            Some(id) => window.with_element_state(id, |kept: Option<TextSelectionHandle>, _| {
                let handle = kept.unwrap_or_else(|| TextSelectionHandle::new(self.text.to_string(), cx));
                (handle.clone(), handle)
            }),
            None => TextSelectionHandle::new(self.text.to_string(), cx),
        };
        (self.drawn.request_layout(window, cx), handle)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        handle: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.drawn.prepaint(window, cx);
        self.order = reading_order(bounds);
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let registration = TextSelectionRegistration::new(hitbox.clone(), bounds)
            .with_document_order(self.order)
            .with_text_bounds(vec![bounds])
            .with_rendered_element(handle, window, cx);
        handle.register(registration, window, cx);
        hitbox
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        handle: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let run = TextSelectionRun::new(self.text.clone(), self.layout.clone(), bounds).with_document_order(self.order);
        let projection = handle.update_runs(&[run], cx);
        // what is selected: a blue behind its letters, line by line
        let color = hsla(alpha(ACCENT, MARK));
        for range in projection.ranges().iter().flatten().cloned() {
            for mark in marks(&self.layout, range) {
                window.paint_quad(fill(mark, color));
            }
        }
        self.drawn.paint(window, cx);
    }
}

/// The boxes behind the letters `range` holds: one for each line it touches.
fn marks(layout: &TextLayout, range: std::ops::Range<usize>) -> Vec<Bounds<Pixels>> {
    let (Some(start), Some(end)) = (layout.position_for_index(range.start), layout.position_for_index(range.end))
    else {
        return Vec::new();
    };
    let (text, line) = (layout.bounds(), layout.line_height());
    if start.y == end.y {
        return vec![Bounds::from_corners(start, point(end.x, end.y + line))];
    }
    let mut boxes = vec![Bounds::from_corners(start, point(text.right(), start.y + line))];
    if end.y > start.y + line {
        boxes.push(Bounds::from_corners(point(text.left(), start.y + line), point(text.right(), end.y)));
    }
    boxes.push(Bounds::from_corners(point(text.left(), end.y), point(end.x, end.y + line)));
    boxes
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{px, size};

    #[test]
    fn pieces_are_read_down_the_window_then_along_the_line() {
        let at = |x: f32, y: f32| Bounds::new(point(px(x), px(y)), size(px(100.), px(20.)));
        // a paragraph above another comes first, wherever each starts
        assert!(reading_order(at(400., 100.)) < reading_order(at(20., 130.)));
        // on one line, the one to the left
        assert!(reading_order(at(20., 100.)) < reading_order(at(400., 100.)));
        // scrolled out above the window's top: still an order, never a wrap around
        assert_eq!(reading_order(at(-50., -300.)), 0);
    }
}
