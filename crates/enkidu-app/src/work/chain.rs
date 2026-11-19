//! A workflow drawn as a chain (DESIGN.md, "A Jira board … workflows"):
//! its steps stand under each other on a rail, each with a mark on the rail and its words beside
//! it. The workflow dialog draws the plan this way (how it starts, each task, how it ends), and the Workflows page
//! draws what happened the same way (when it started, each task with how it went, when it ended), so a workflow
//! is the same picture before and after.

use enkidu_ui::pieces::Pressed;
use enkidu_ui::tokens as theme;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Div, ElementId, IntoElement, div, px};

/// A step's mark on the rail: how far down its step it stands (level with its first line), and the square it stands
/// in. The rail's column is 24 px wide, with the square in its middle.
pub(crate) const MARK_TOP: f32 = 8.;
pub(crate) const MARK: f32 = 20.;
const COLUMN: f32 = 24.;

/// A step of a chain: its mark on the rail, the rail going on above it and below it, then what the caller adds
/// (its words, a time at the right). Steps stand right under each other, so the rail is one line from the first
/// to the last. `ringed`: the mark stands in a ring (a number, a small glyph); else it is an icon of its own, and
/// the rail comes close to it.
pub(crate) fn chain_step(
    id: impl Into<ElementId>,
    mark: AnyElement,
    ringed: bool,
    above: bool,
    below: bool,
) -> Pressed {
    let near = if ringed { 0. } else { 2. };
    let rail = || div().absolute().left(px(COLUMN / 2. - 0.5)).w(px(1.)).bg(theme::LINE_STRONG);
    enkidu_ui::press::pressable(div().id(id))
        .test_support()
        .relative()
        .flex_none()
        .w_full()
        .min_w_0()
        .flex()
        .gap(px(8.))
        .rounded(theme::RADIUS_CONTROL)
        .px(px(8.))
        .child(
            div()
                .relative()
                .flex_none()
                .w(px(COLUMN))
                .when(above, |column| column.child(rail().top_0().h(px(MARK_TOP + near))))
                .when(below, |column| column.child(rail().top(px(MARK_TOP + MARK - near)).bottom_0()))
                .child(
                    div()
                        .absolute()
                        .top(px(MARK_TOP))
                        .left(px((COLUMN - MARK) / 2.))
                        .size(px(MARK))
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(ringed, |mark| mark.rounded_full().border_1().border_color(theme::LINE_STRONG))
                        .child(mark),
                ),
        )
}

/// What a step says: its first line, and the quieter one under it.
pub(crate) fn chain_words(first: impl IntoElement, second: Option<AnyElement>) -> Div {
    div().min_w_0().flex_1().py(px(8.)).child(first).children(second)
}

/// The step that is shown beside the chain: pressed in, with the blue bar of what is on screen (DESIGN.md,
/// "States"); the others answer the pointer.
pub(crate) fn shown_in_chain(step: Pressed, on: bool, window_in_front: bool) -> Pressed {
    match on {
        true => step.bg(theme::SELECTED).child(enkidu_ui::pieces::on_screen_bar(8., window_in_front)),
        false => step.hover(|s| s.bg(theme::HOVER)),
    }
}
