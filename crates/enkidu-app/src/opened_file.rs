//! How an opened file looks around its lines (item 3.7): the band that takes the place of "Changes" while a file is
//! open, and what stands in place of the lines when there are none to show. The lines themselves are drawn by the diff
//! view (`enkidu-diff`), as one file without changes; the island (`changes_island`) holds the file and its remarks.

use enkidu_model::remarks::remarks_words;
use enkidu_model::review::{back_words, close_words, lines_words, split_path};
use enkidu_ui::pieces::{ButtonKind, ButtonSize, Observed, button_with, file_icon, line_under};
use enkidu_ui::tips::Tipped as _;
use enkidu_ui::tokens as theme;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, FontWeight, Rgba, SharedString, div, px, svg};

use crate::changes_island::{KEYS, glyph};
use crate::remark_views::Act;

/// What the band says of the opened file.
pub struct BandView {
    pub path: String,
    /// What happened to it ("modified", "untracked"…), when it is one of the changed files.
    pub kind: Option<String>,
    /// How many lines it has, when it has lines to show.
    pub lines: Option<usize>,
    /// How many remarks wait on its lines.
    pub remarks: usize,
    /// There are changes or remarks to go back to. Without any, leaving the file gives Claude the room again, and
    /// the button says so.
    pub has_review: bool,
    /// A ⌘-click led here (item 10.11): where the band's first button goes back to, a file's name or "the changes".
    pub back_to: Option<String>,
    /// Your typing goes to the file: its name says so, underlined in blue (DESIGN.md, "States").
    pub typing: bool,
}

/// What the band's buttons do.
pub struct BandActs {
    pub close: Act,
    /// The file closes and its diff comes into view (only for a changed file).
    pub show_changes: Act,
    pub copy_path: Act,
    pub reveal: Act,
}

/// A square button holding an icon, in the band.
fn tool(id: &'static str, icon: &'static str, label: impl Into<SharedString>, ink: Rgba) -> Observed {
    div()
        .id(id)
        .test_support()
        .role(gpui_kit::Role::Button)
        .aria_label(label.into())
        .size(theme::CONTROL)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(theme::RADIUS_CONTROL)
        .cursor_pointer()
        .group("band-tool")
        .hover(|s| s.bg(theme::HOVER))
        .child(
            svg().path(icon).size(theme::ICON).text_color(ink).group_hover("band-tool", |s| s.text_color(theme::INK)),
        )
}

/// The band of an opened file: the way back, the file (its icon, name and folder, what happened to it, its lines
/// and its remarks), then "Show changes" for a changed one, its path to copy, and where it is in Finder.
pub fn band(view: &BandView, acts: BandActs) -> AnyElement {
    let BandActs { close, show_changes, copy_path, reveal } = acts;
    let (dir, name) = split_path(&view.path);
    let (dir, name) = (dir.to_string(), name.to_string());
    // after a ⌘-click the way back is to where it was made (⌘[); else to the changes, or the file just closes
    let leave: SharedString = match &view.back_to {
        Some(to) => back_words(to).into(),
        None => close_words(view.has_review).into(),
    };
    let back = if view.has_review || view.back_to.is_some() { "icons/arrow-left.svg" } else { "icons/x.svg" };
    let key: Box<dyn gpui_kit::Action> =
        if view.back_to.is_some() { Box::new(crate::GoBack) } else { Box::new(crate::CloseFile) };
    let full = view.path.clone();
    div()
        .id("file-band")
        .test_support()
        .aria_label(SharedString::from(view.path.clone()))
        .h(theme::BAR)
        .flex_none()
        .flex()
        .items_center()
        .gap(px(8.))
        // (a button's box stands 12 px from the edge, so its sign stands at 16)
        .px(px(12.))
        .relative()
        .child(line_under())
        .child(
            tool("close-file", back, leave.clone(), theme::INK_2)
                .tipped_in(leave, key, KEYS)
                .on_click(move |_, window, cx| close(window, cx)),
        )
        .child(file_icon(&view.path, 16.))
        .child(
            div()
                .id("file-head")
                .test_support()
                .aria_label(SharedString::from(view.path.clone()))
                .aria_description(if view.typing { "has the keyboard" } else { "" })
                .tipped(full.clone(), None)
                .h_full()
                .flex()
                .min_w_0()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .flex_shrink_0()
                        .max_w_full()
                        .truncate()
                        .text_size(theme::PANEL_TITLE)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::INK)
                        .child(name),
                )
                .when(!dir.is_empty(), |head| {
                    head.child(
                        div().min_w_0().truncate().text_size(theme::TEXT_SMALL).text_color(theme::INK_3).child(dir),
                    )
                }),
        )
        .children(view.kind.as_deref().map(glyph))
        .children(view.lines.map(|lines| {
            let words = lines_words(lines);
            div()
                .id("file-lines")
                .test_support()
                .aria_label(SharedString::from(words.clone()))
                .flex_none()
                .text_size(px(12.))
                .text_color(theme::INK_3)
                .child(words)
        }))
        // the remarks on this file that weren't sent yet, in their blue
        .when(view.remarks > 0, |band| {
            let words = remarks_words(view.remarks);
            band.child(
                div()
                    .id("file-remarks")
                    .test_support()
                    .aria_label(SharedString::from(words.clone()))
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(4.))
                    .h(theme::CHIP)
                    .px(px(8.))
                    .rounded_full()
                    .bg(theme::REMARK_BAND)
                    .text_size(theme::TEXT_STATUS)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::ACCENT)
                    .child(svg().path("icons/message-square.svg").size(theme::MARK).text_color(theme::ACCENT))
                    .child(words),
            )
        })
        .child(div().flex_1())
        .when(view.kind.is_some(), |band| {
            band.child(
                button_with(
                    "show-changes",
                    Some("icons/file-diff.svg"),
                    "Show changes",
                    None,
                    ButtonKind::Secondary,
                    ButtonSize::Small,
                )
                .on_click(move |_, window, cx| show_changes(window, cx)),
            )
        })
        .child(
            tool("copy-path", "icons/copy.svg", "Copy path", theme::INK_3)
                .tipped("Copy path", None)
                .on_click(move |_, window, cx| copy_path(window, cx)),
        )
        .child(
            tool("reveal-file", "icons/folder-open.svg", "Show in Finder", theme::INK_3)
                .tipped("Show in Finder", None)
                .on_click(move |_, window, cx| reveal(window, cx)),
        )
        .into_any_element()
}

/// What stands in place of the lines: why there are none ("This file is empty.", "A binary file (12 KB), not
/// shown.").
pub fn note(words: String) -> AnyElement {
    div()
        .id("file-note")
        .test_support()
        .aria_label(SharedString::from(words.clone()))
        .w_full()
        .px(px(16.))
        .py(px(24.))
        .flex()
        .justify_center()
        .text_size(px(12.))
        .text_color(theme::INK_3)
        .child(words)
        .into_any_element()
}
