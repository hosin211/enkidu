//! Enkidu's diff view (item 3.2a). This crate is gpui-kit's own diff view, copied from
//! `gpui-component` 0.9.1 (`src/diff`, Apache-2.0, Copyright 2024 - 2026 Longbridge; see `NOTICE` and
//! `LICENSE-APACHE`), because two things Enkidu needs can't be added from outside it: a new list of files reset
//! its scroll to the top (ours changes every few seconds while Claude works), and a file's header scrolled away with
//! its lines. What differs from the original is marked `enkidu:` in the code. To see all of it, format a copy of the
//! original the way this repository is formatted, then compare (`mod.rs` there is `lib.rs` here):
//!
//! ```sh
//! cp -R ~/.cargo/registry/src/index.crates.io-*/gpui-component-0.9.1/src/diff /tmp/kit-diff
//! chmod -R u+w /tmp/kit-diff && rustfmt --edition 2024 --config-path rustfmt.toml /tmp/kit-diff/*.rs
//! diff -ru /tmp/kit-diff crates/enkidu-diff/src
//! ```
//!
//! The original's own words follow.
//!
//! Readonly, virtualized patch display inspired by Diffs from Pierre.
//! Parse an externally supplied patch into [`DiffFile`]s, retain one
//! [`DiffState`] for all of them in the owner, and build [`Diff`] during rendering.

// enkidu: style lints the original doesn't follow; the copied code stays as it was written.
#![allow(
    clippy::single_range_in_vec_init,
    clippy::type_complexity,
    clippy::collapsible_match,
    clippy::match_like_matches_macro
)]

// enkidu: the copied code names GPUI `gpui`, as its own crate does; here that is gpui-kit, which re-exports it.
extern crate gpui_kit as gpui;

mod conflict;
mod decoration;
mod document;
// enkidu: ours, for the header that stays in view
mod late;
#[cfg(test)]
mod links_tests;
#[cfg(test)]
mod marks_tests;
mod parser;
#[cfg(test)]
mod parser_tests;
mod presentation;
mod selection;
mod state;
// enkidu: the original's strings came from gpui-component's translations; ours are plain English, in one place.
mod words;
pub(crate) use words::t;

pub use conflict::{DiffConflict, DiffConflictResolution};
pub use decoration::{DiffDecoration, DiffGutterMarker, DiffMark, DiffName};
pub use document::{
    DiffChangeKind, DiffChangeMark, DiffFile, DiffFileStatus, DiffLinePosition, DiffLineRange, DiffSide,
};
pub use parser::{DiffParseError, DiffParseOptions};
pub use presentation::DiffInlineUnit;
pub use state::{DiffEvent, DiffState};

/// enkidu: the language a file at `path` is colored as (`python`, `vue`, `text`…), as the view names it for the
/// application's highlighter: for code shown outside a diff, such as a line of Find in files.
pub fn language_of(path: &str) -> gpui::SharedString {
    presentation::detected_language(path)
}

use std::{collections::HashMap, rc::Rc, sync::Arc};

use gpui::{
    AnyElement, App, Axis, ClickEvent, ClipboardItem, Context, ElementId, Entity, Focusable as _, HighlightStyle, Hsla,
    InteractiveElement as _, IntoElement, KeyBinding, ListOffset, MouseButton, ParentElement as _, Pixels, RenderOnce,
    SharedString, StatefulInteractiveElement as _, StyleRefinement, Styled, TextRun, Window, div, list, point,
    prelude::FluentBuilder as _, px,
};
use gpui_kit::base::TestSupportExt as _;

use actions::*;
use conflict::ConflictPart;
use decoration::{ChangeMarks, Decorations, GutterMarkers, Marks};
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Copy, SelectAll},
    scroll::{ScrollableMask, Scrollbar},
    v_flex,
};
use presentation::FilePresentation;
use selection::{CodeText, SelectionLayer};
use state::{DisplayRow, FoldExpansion, LayoutKey, SelectedLines};

// enkidu: the key context of this view (gpui-kit binds "Diff" to its own actions).
const KEYS: &str = "EnkiduDiff";

// Private: bindings serve the focused code body only.
mod actions {
    // enkidu: our own namespace; gpui-kit registers `diff::…` for its own view
    gpui::actions!(
        enkidu_diff,
        [ScrollUp, ScrollDown, ScrollLeft, ScrollRight, PageUp, PageDown, First, Last, ExtendUp, ExtendDown]
    );
}

/// Binds the view's keys. Call it once, after `gpui_kit::init`.
pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("up", ScrollUp, Some(KEYS)),
        KeyBinding::new("down", ScrollDown, Some(KEYS)),
        KeyBinding::new("left", ScrollLeft, Some(KEYS)),
        KeyBinding::new("right", ScrollRight, Some(KEYS)),
        KeyBinding::new("pageup", PageUp, Some(KEYS)),
        KeyBinding::new("pagedown", PageDown, Some(KEYS)),
        KeyBinding::new("home", First, Some(KEYS)),
        KeyBinding::new("end", Last, Some(KEYS)),
        KeyBinding::new("shift-up", ExtendUp, Some(KEYS)),
        KeyBinding::new("shift-down", ExtendDown, Some(KEYS)),
        KeyBinding::new(if cx.platform_info().is_mac() { "cmd-c" } else { "ctrl-c" }, Copy, Some(KEYS)),
        KeyBinding::new(if cx.platform_info().is_mac() { "cmd-a" } else { "ctrl-a" }, SelectAll, Some(KEYS)),
    ]);
}

/// How a [`Diff`] presents the two sides of each file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum DiffMode {
    /// The original and modified source side by side.
    Split,
    /// One column, with deleted lines before added lines.
    #[default]
    Unified,
}

/// Which part of a row is emphasized while the pointer is over it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum DiffHoverHighlight {
    #[default]
    None,
    /// The whole line.
    Line,
    /// The line-number gutter.
    LineNumber,
    /// The line and its line number.
    Both,
}

/// How the start of each hunk is marked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum DiffHunkSeparator {
    /// The `@@` header, including the enclosing scope Git reports.
    #[default]
    Metadata,
    /// The count of lines the patch does not supply before the hunk.
    LineInfo,
    /// A thin divider.
    Simple,
}

/// How changed lines are marked beside the code.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum DiffChangeIndicator {
    /// `+` and `−` signs.
    #[default]
    Signs,
    /// A colored bar.
    Bars,
    /// No marker.
    None,
}

/// Application-owned content attached to a source line or to a whole file.
///
/// The stable ID belongs to the annotation itself, while the target uses the
/// file path and, for a line, its side and one-based source line, independent
/// of folding.
#[derive(Clone)]
pub struct DiffAnnotation {
    id: ElementId,
    path: SharedString,
    position: Option<DiffLinePosition>,
}

impl DiffAnnotation {
    /// Attaches content beneath a source line.
    pub fn line(id: impl Into<ElementId>, position: DiffLinePosition) -> Self {
        Self { id: id.into(), path: position.path().clone(), position: Some(position) }
    }
    /// Attaches content beneath the header of the file at `path`.
    pub fn file(id: impl Into<ElementId>, path: impl Into<SharedString>) -> Self {
        Self { id: id.into(), path: path.into(), position: None }
    }
    pub fn id(&self) -> &ElementId {
        &self.id
    }
    /// The [`DiffFile::path`] of the annotated file.
    pub fn path(&self) -> &SharedString {
        &self.path
    }
    /// The annotated line, or `None` for a file annotation.
    pub fn position(&self) -> Option<&DiffLinePosition> {
        self.position.as_ref()
    }
}

type FileRenderer = Rc<dyn Fn(&DiffFile, &mut Window, &mut App) -> AnyElement>;
/// enkidu: see [`Diff::render_notice`].
type NoticeRenderer = Rc<dyn Fn(&DiffFile, &mut Window, &mut App) -> Option<AnyElement>>;
type AnnotationRenderer = Rc<dyn Fn(&DiffAnnotation, &mut Window, &mut App) -> AnyElement>;
type MarkerRenderer = Rc<dyn Fn(&DiffGutterMarker, &mut Window, &mut App) -> AnyElement>;
type RangeHandler = Rc<dyn Fn(&DiffLineRange, &mut Window, &mut App)>;
type ClickHandler = Rc<dyn Fn(&DiffLinePosition, &ClickEvent, &mut Window, &mut App)>;
type HoverHandler = Rc<dyn Fn(Option<&DiffLinePosition>, &mut Window, &mut App)>;

#[derive(Default)]
struct HeaderSlots {
    content: Option<FileRenderer>,
    prefix: Option<FileRenderer>,
    title_suffix: Option<FileRenderer>,
    suffix: Option<FileRenderer>,
}

#[derive(Default)]
struct Annotations {
    items: Vec<DiffAnnotation>,
    lines: HashMap<DiffLinePosition, Vec<usize>>,
    files: HashMap<SharedString, Vec<usize>>,
}

/// A themed readonly code-review surface in split or unified mode.
///
/// All files of the supplied state share one virtualized list, each introduced
/// by its header. The component owns its scrolling, so give it a bounded
/// height. Long lines scroll horizontally unless [`Diff::soft_wrap`] is set;
/// file headers and review annotations are explicit slots, and application
/// commands remain owned by the application.
#[derive(IntoElement)]
pub struct Diff {
    state: Entity<DiffState>,
    style: StyleRefinement,
    line_number: bool,
    syntax_highlight: bool,
    header_visible: bool,
    hover_highlight: DiffHoverHighlight,
    hunk_separator: DiffHunkSeparator,
    change_indicator: DiffChangeIndicator,
    change_background: bool,
    soft_wrap: bool,
    header: HeaderSlots,
    annotations: Rc<Annotations>,
    annotation_content: Option<AnnotationRenderer>,
    decorations: Rc<Decorations>,
    marks: Rc<Marks>,
    /// enkidu: see [`Diff::change_marks`].
    change_marks: Rc<ChangeMarks>,
    gutter_markers: Rc<GutterMarkers>,
    gutter_marker_content: Option<MarkerRenderer>,
    on_add_annotation: Option<RangeHandler>,
    on_line_click: Option<ClickHandler>,
    on_line_hover: Option<HoverHandler>,
    /// enkidu: see [`Diff::colors`].
    colors: Option<DiffColors>,
    /// enkidu: see [`Diff::render_notice`].
    notice: Option<NoticeRenderer>,
    /// enkidu: see [`Diff::row_height`].
    row_height: Option<Pixels>,
    /// enkidu: see [`Diff::font_size`].
    font_size: Option<Pixels>,
}

/// enkidu: how tall a file's header is. It is fixed, so the header drawn over the rows (the one that stays in view) and
/// the row it stands in for are the same height.
pub const HEADER_HEIGHT: Pixels = px(36.);

/// enkidu: the view's own colors. The original took them from gpui-kit's theme at fixed strengths
/// ([`DiffColors::of_theme`] gives those); Enkidu sets its own, so the diff matches the rest of the window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DiffColors {
    /// A file header's band. It is opaque: the rows scroll under the header that stays in view.
    pub header: Hsla,
    /// Hairlines: under a header, between two files, between the two sides, around a line note.
    pub rule: Hsla,
    /// An added line's fill, the words added inside it, and its `+`.
    pub added: Hsla,
    pub added_word: Hsla,
    pub added_sign: Hsla,
    /// A removed line's fill, the words removed from it, and its `−`.
    pub removed: Hsla,
    pub removed_word: Hsla,
    pub removed_sign: Hsla,
    /// What stands for lines that aren't shown: folds, hunk headings, the empty side of a row.
    pub gap: Hsla,
    /// The lines that are selected by their numbers.
    pub selected: Hsla,
    /// Behind a line note.
    pub note: Hsla,
    /// The stripes of [`Diff::change_marks`]: beside an added line, beside a changed one, and where lines went.
    pub mark_added: Hsla,
    pub mark_changed: Hsla,
    pub mark_removed: Hsla,
    /// A name that is a link ([`DiffState::set_link`]): its text and the line under it.
    pub link: Hsla,
}

impl DiffColors {
    /// The original's colors: gpui-kit's theme, at the strengths the original used.
    pub fn of_theme(cx: &App) -> Self {
        let theme = cx.theme();
        Self {
            header: theme.background,
            rule: theme.border,
            added: theme.success.opacity(0.12),
            added_word: theme.success.opacity(0.3),
            added_sign: theme.success,
            removed: theme.danger.opacity(0.12),
            removed_word: theme.danger.opacity(0.3),
            removed_sign: theme.danger,
            gap: theme.muted.opacity(0.35),
            selected: theme.selection,
            note: theme.muted.opacity(0.18),
            mark_added: theme.success,
            mark_changed: theme.info,
            mark_removed: theme.danger,
            link: theme.link,
        }
    }
}

fn file_renderer<F, E>(render: F) -> FileRenderer
where
    F: Fn(&DiffFile, &mut Window, &mut App) -> E + 'static,
    E: IntoElement,
{
    Rc::new(move |file, window, cx| render(file, window, cx).into_any_element())
}

impl Diff {
    pub fn new(state: &Entity<DiffState>) -> Self {
        Self {
            state: state.clone(),
            style: StyleRefinement::default(),
            line_number: true,
            syntax_highlight: true,
            header_visible: true,
            hover_highlight: DiffHoverHighlight::default(),
            hunk_separator: DiffHunkSeparator::default(),
            change_indicator: DiffChangeIndicator::default(),
            change_background: true,
            soft_wrap: false,
            header: HeaderSlots::default(),
            annotations: Rc::default(),
            annotation_content: None,
            decorations: Rc::default(),
            marks: Rc::default(),
            change_marks: Rc::default(),
            gutter_markers: Rc::default(),
            gutter_marker_content: None,
            on_add_annotation: None,
            on_line_click: None,
            on_line_hover: None,
            colors: None,
            notice: None,
            row_height: None,
            font_size: None,
        }
    }
    /// enkidu: sets the size of the code and of its line numbers. Default is the theme's mono font size, as in the
    /// original. The view lays its lines out again when it changes, keeping its place.
    pub fn font_size(mut self, size: Pixels) -> Self {
        self.font_size = Some(size);
        self
    }
    /// enkidu: sets how tall a line of code is. Default is 1.6 times the code font's size, as in the original.
    pub fn row_height(mut self, height: Pixels) -> Self {
        self.row_height = Some(height);
        self
    }
    /// enkidu: says what stands under the header of a file that has no lines to show (see
    /// [`DiffFile::without_lines`]): the application knows why, the view doesn't. `None` leaves the view's own
    /// words ("Binary file, not shown.", and so on).
    pub fn render_notice<F, E>(mut self, render: F) -> Self
    where
        F: Fn(&DiffFile, &mut Window, &mut App) -> Option<E> + 'static,
        E: IntoElement,
    {
        self.notice =
            Some(Rc::new(move |file, window, cx| render(file, window, cx).map(IntoElement::into_any_element)));
        self
    }
    /// enkidu: sets the view's own colors. Default is [`DiffColors::of_theme`].
    pub fn colors(mut self, colors: DiffColors) -> Self {
        self.colors = Some(colors);
        self
    }
    /// Shows the original and modified line-number lanes. Default is true.
    pub fn line_number(mut self, value: bool) -> Self {
        self.line_number = value;
        self
    }
    /// Emphasizes syntax once the state has prepared it. Default is true.
    pub fn syntax_highlight(mut self, value: bool) -> Self {
        self.syntax_highlight = value;
        self
    }
    /// Shows a header above each file. Default is true.
    pub fn header_visible(mut self, visible: bool) -> Self {
        self.header_visible = visible;
        self
    }
    /// Emphasizes the line, its number, or both under the pointer. Default is none.
    pub fn hover_highlight(mut self, highlight: DiffHoverHighlight) -> Self {
        self.hover_highlight = highlight;
        self
    }
    /// Marks the start of each hunk. Default is [`DiffHunkSeparator::Metadata`].
    pub fn hunk_separator(mut self, separator: DiffHunkSeparator) -> Self {
        self.hunk_separator = separator;
        self
    }
    /// Marks changed lines beside the code. Default is [`DiffChangeIndicator::Signs`].
    pub fn change_indicator(mut self, indicator: DiffChangeIndicator) -> Self {
        self.change_indicator = indicator;
        self
    }
    /// Tints changed lines. Default is true. Inline changes stay emphasized.
    pub fn change_background(mut self, value: bool) -> Self {
        self.change_background = value;
        self
    }
    /// Wraps long lines at the column width instead of scrolling horizontally.
    /// Default is false.
    pub fn soft_wrap(mut self, value: bool) -> Self {
        self.soft_wrap = value;
        self
    }
    /// Supplies annotations with stable identities and targets.
    ///
    /// Rows re-measure when annotations are added, removed or replaced, and
    /// whenever they are visible, so annotation content may change height freely.
    pub fn annotations(mut self, annotations: impl IntoIterator<Item = DiffAnnotation>) -> Self {
        let mut index = Annotations { items: annotations.into_iter().collect(), ..Annotations::default() };
        for (ix, annotation) in index.items.iter().enumerate() {
            match &annotation.position {
                Some(position) => index.lines.entry(position.clone()).or_default().push(ix),
                None => index.files.entry(annotation.path.clone()).or_default().push(ix),
            }
        }
        self.annotations = Rc::new(index);
        self
    }
    /// Renders each supplied annotation. Application state and commands stay with the owner.
    pub fn render_annotation<F, E>(mut self, render: F) -> Self
    where
        F: Fn(&DiffAnnotation, &mut Window, &mut App) -> E + 'static,
        E: IntoElement,
    {
        self.annotation_content =
            Some(Rc::new(move |annotation, window, cx| render(annotation, window, cx).into_any_element()));
        self
    }
    /// Supplies line backgrounds that mark source ranges, such as diagnostics or
    /// references, without changing the selection.
    pub fn decorations(mut self, decorations: impl IntoIterator<Item = DiffDecoration>) -> Self {
        self.decorations = Rc::new(Decorations::new(decorations));
        self
    }
    /// enkidu: supplies stretches of lines marked in place, such as the matches of a search on the line that was opened.
    pub fn marks(mut self, marks: impl IntoIterator<Item = DiffMark>) -> Self {
        self.marks = Rc::new(Marks::new(marks));
        self
    }
    /// enkidu: supplies, by path, the stretches of a file that differ from the version it is compared with
    /// ([`DiffFile::change_marks`] gives them): the view draws a stripe beside the number of each such line of the
    /// modified side, in [`DiffColors::mark_added`] or [`DiffColors::mark_changed`], and a small mark in
    /// [`DiffColors::mark_removed`] where lines went. For a file shown whole, which shows no change of its own.
    pub fn change_marks(mut self, files: impl IntoIterator<Item = (SharedString, Vec<DiffChangeMark>)>) -> Self {
        self.change_marks = Rc::new(ChangeMarks::new(files));
        self
    }
    /// Supplies markers for the marker lane, shown once both markers and
    /// [`Self::render_gutter_marker`] are set.
    ///
    /// The lane holds one marker per line; when several target the same line,
    /// the first supplied is shown.
    pub fn gutter_markers(mut self, markers: impl IntoIterator<Item = DiffGutterMarker>) -> Self {
        self.gutter_markers = Rc::new(GutterMarkers::new(markers));
        self
    }
    /// Renders each gutter marker within a lane one row high and wide.
    ///
    /// Return a focusable control such as a small ghost button labeled with
    /// [`DiffGutterMarker::label`], so the marker works with the pointer and
    /// keyboard. Presses inside the lane do not start a selection.
    pub fn render_gutter_marker<F, E>(mut self, render: F) -> Self
    where
        F: Fn(&DiffGutterMarker, &mut Window, &mut App) -> E + 'static,
        E: IntoElement,
    {
        self.gutter_marker_content =
            Some(Rc::new(move |marker, window, cx| render(marker, window, cx).into_any_element()));
        self
    }
    /// Replaces the content of each file header, keeping its shell, separator
    /// and collapse control.
    pub fn render_header<F, E>(mut self, render: F) -> Self
    where
        F: Fn(&DiffFile, &mut Window, &mut App) -> E + 'static,
        E: IntoElement,
    {
        self.header.content = Some(file_renderer(render));
        self
    }
    /// Inserts content before the path in the default header.
    pub fn render_header_prefix<F, E>(mut self, render: F) -> Self
    where
        F: Fn(&DiffFile, &mut Window, &mut App) -> E + 'static,
        E: IntoElement,
    {
        self.header.prefix = Some(file_renderer(render));
        self
    }
    /// Inserts compact content immediately after the path in the default header.
    pub fn render_header_title_suffix<F, E>(mut self, render: F) -> Self
    where
        F: Fn(&DiffFile, &mut Window, &mut App) -> E + 'static,
        E: IntoElement,
    {
        self.header.title_suffix = Some(file_renderer(render));
        self
    }
    /// Inserts trailing content after the change statistics in the default header.
    pub fn render_header_suffix<F, E>(mut self, render: F) -> Self
    where
        F: Fn(&DiffFile, &mut Window, &mut App) -> E + 'static,
        E: IntoElement,
    {
        self.header.suffix = Some(file_renderer(render));
        self
    }
    /// Compatibility alias for [`Self::render_annotation`].
    pub fn annotation_content<F, E>(self, render: F) -> Self
    where
        F: Fn(&DiffAnnotation, &mut Window, &mut App) -> E + 'static,
        E: IntoElement,
    {
        self.render_annotation(render)
    }

    /// Compatibility alias for [`Self::render_header`].
    pub fn header<F, E>(self, render: F) -> Self
    where
        F: Fn(&DiffFile, &mut Window, &mut App) -> E + 'static,
        E: IntoElement,
    {
        self.render_header(render)
    }

    /// Compatibility alias for [`Self::render_header_prefix`].
    pub fn header_prefix<F, E>(self, render: F) -> Self
    where
        F: Fn(&DiffFile, &mut Window, &mut App) -> E + 'static,
        E: IntoElement,
    {
        self.render_header_prefix(render)
    }

    /// Compatibility alias for [`Self::render_header_title_suffix`].
    pub fn header_title_suffix<F, E>(self, render: F) -> Self
    where
        F: Fn(&DiffFile, &mut Window, &mut App) -> E + 'static,
        E: IntoElement,
    {
        self.render_header_title_suffix(render)
    }

    /// Compatibility alias for [`Self::render_header_suffix`].
    pub fn header_suffix<F, E>(self, render: F) -> Self
    where
        F: Fn(&DiffFile, &mut Window, &mut App) -> E + 'static,
        E: IntoElement,
    {
        self.render_header_suffix(render)
    }

    /// Shows an add button beside the hovered line and at the end of the
    /// selected lines. The handler receives the selected range when the line
    /// is part of it, or that line alone.
    pub fn on_add_annotation<F>(mut self, handler: F) -> Self
    where
        F: Fn(&DiffLineRange, &mut Window, &mut App) + 'static,
    {
        self.on_add_annotation = Some(Rc::new(handler));
        self
    }
    /// Handles a click on a line's code, outside a text-selection drag.
    pub fn on_line_click<F>(mut self, handler: F) -> Self
    where
        F: Fn(&DiffLinePosition, &ClickEvent, &mut Window, &mut App) + 'static,
    {
        self.on_line_click = Some(Rc::new(handler));
        self
    }
    /// Reports the line under the pointer, or `None` when the pointer leaves it.
    pub fn on_line_hover<F>(mut self, handler: F) -> Self
    where
        F: Fn(Option<&DiffLinePosition>, &mut Window, &mut App) + 'static,
    {
        self.on_line_hover = Some(Rc::new(handler));
        self
    }
}
impl Styled for Diff {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Diff {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = self.state.clone();
        state.update(cx, |state, cx| render_diff(self, state, window, cx))
    }
}

fn render_diff(props: Diff, state: &mut DiffState, window: &mut Window, cx: &mut Context<DiffState>) -> AnyElement {
    state.set_window(window.window_handle().window_id());
    state.ensure_presentation(cx);
    let rem = window.rem_size();
    let font_size = props.font_size.unwrap_or_else(|| rem * (f32::from(cx.theme().mono_font_size) / 16.));
    let font_family = cx.theme().mono_font_family.clone();
    let row_height = props.row_height.unwrap_or(font_size * 1.6);
    let digits = state.files().iter().map(|file| file.line_number_digits()).max().unwrap_or(1);
    let gutter_width = if props.line_number { font_size * 0.7 * digits as f32 + rem } else { px(0.) };
    let mode = state.mode();
    let soft_wrap = props.soft_wrap;
    let marker_lane = !props.gutter_markers.is_empty() && props.gutter_marker_content.is_some();
    let lane_width = if marker_lane { row_height } else { px(0.) };
    let layout = LayoutKey::new(rem, font_size, font_family.clone(), mode, props.line_number, marker_lane, soft_wrap);
    if state.update_layout(layout, row_height) {
        // Wrapped rows take the viewport width instead of their longest line.
        let width = if soft_wrap {
            px(0.)
        } else {
            measure_code_width(state.files(), font_size, &font_family, gutter_width, lane_width, mode, window, cx)
        };
        state.set_content_width(width);
    }
    state.sync_annotations(&props.annotations.items);
    let content_width = state.content_width().max(state.viewport().width);
    let column_width = if mode == DiffMode::Split { content_width / 2. } else { content_width };
    // enkidu: the file at the top of the view. Its header is drawn over the rows, at the top, so it stays in view while
    // its lines scroll under it; the next file's header pushes it away as it arrives.
    let follows_top = props.header_visible;
    let sticky = if follows_top { state.top_file() } else { None };
    let sticky_shift = sticky
        .and_then(|file| state.header_row(file + 1))
        .and_then(|row| state.list().bounds_for_item(row))
        .map_or(px(0.), |next| (next.top() - state.list().viewport_bounds().top() - HEADER_HEIGHT).min(px(0.)));
    let colors = props.colors.unwrap_or_else(|| DiffColors::of_theme(cx));
    let code = Rc::new(CodePresentation {
        colors,
        sticky,
        notice: props.notice,
        files: state.files().to_vec(),
        presentations: state.presentations().to_vec(),
        mode,
        content_width,
        column_width,
        font_size,
        row_height,
        gutter_width,
        rem,
        expansion_lines: state.expansion_lines(),
        line_number: props.line_number,
        syntax_highlight: props.syntax_highlight,
        header_visible: props.header_visible,
        hover_highlight: props.hover_highlight,
        hunk_separator: props.hunk_separator,
        change_indicator: props.change_indicator,
        change_background: props.change_background,
        soft_wrap,
        header: props.header,
        annotations: props.annotations.clone(),
        annotation_content: props.annotation_content,
        decorations: props.decorations,
        marks: props.marks,
        change_marks: props.change_marks,
        link: state.link().cloned(),
        gutter_markers: props.gutter_markers,
        gutter_marker_content: props.gutter_marker_content.filter(|_| marker_lane),
        lane_width,
        on_add_annotation: props.on_add_annotation,
        on_line_click: props.on_line_click,
        on_line_hover: props.on_line_hover,
        selection: state.selection().clone(),
        selected_lines: state.selected_lines(),
        selected: state.selected_span(),
        geometry: state.geometry().clone(),
        state: cx.entity(),
    });
    let rows = state.rows().clone();
    let scrollbar = state.list().clone();
    let horizontal = state.horizontal_scroll().clone();
    let has_rows = !rows.is_empty();
    // enkidu: the header that stays (see `sticky` above), clipped to the body so that a header being pushed away
    // doesn't draw over what is above the view
    let sticky_header = sticky.filter(|_| has_rows).map(|file| {
        let code = code.clone();
        div().absolute().inset_0().overflow_hidden().child(
            div()
                .id("sticky-header")
                .test_support()
                .debug_selector(|| "sticky-header".into())
                .absolute()
                .top(sticky_shift)
                .left_0()
                .right_0()
                // a press on it is not a press on the rows under it
                .occlude()
                // built at layout, when this state is free again (see `late`)
                .child(late::Late::new(move |window, cx| file_header_bar(file, true, &code, window, cx))),
        )
    });
    let label = match state.files() {
        [file] => t!("Diff.Viewer", name = file.path().as_str()).to_string(),
        files => t!("Diff.ViewerFiles", count = files.len()).to_string(),
    };
    let body = div()
        .id("diff-body")
        .relative()
        .min_h_0()
        .min_w_0()
        .flex_1()
        .child(SelectionLayer::new(
            div()
                .id("diff-horizontal")
                .size_full()
                // Preserve offset tracking and clipping without GPUI's
                // single-axis wheel remapping. The sibling mask owns X input.
                .when(!soft_wrap, |this| this.overflow_x_hidden())
                .track_scroll(&horizontal)
                .child(gpui_kit::base::ScrollBounce::new(
                    "diff-bounce",
                    &scrollbar,
                    list(scrollbar.clone(), move |ix, window, cx| {
                        let row = &rows[ix];
                        div()
                            .id(("diff-file", row.file()))
                            .w_full()
                            .child(render_row(row, code.clone(), window, cx))
                            .into_any_element()
                    })
                    .w(content_width)
                    .h_full(),
                ))
                .into_any_element(),
            scrollbar.clone(),
            horizontal.clone(),
            state.selection().clone(),
            state.geometry().clone(),
            cx.entity().downgrade(),
        ))
        .when(has_rows && !soft_wrap, |this| {
            this.child(ScrollableMask::new(Axis::Horizontal, &horizontal).id("diff-horizontal-wheel"))
        })
        .children(sticky_header)
        .when(has_rows, |this| {
            this.child(div().absolute().inset_0().child(Scrollbar::vertical(&scrollbar).viewport_from_layout()))
                .when(!soft_wrap, |this| this.child(Scrollbar::horizontal(&horizontal)))
        })
        // A line-number drag may finish anywhere, including outside the body.
        .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, cx| this.end_line_selection(cx)))
        .on_mouse_up_out(MouseButton::Left, cx.listener(|this, _, _, cx| this.end_line_selection(cx)))
        .on_action(cx.listener(|this, _: &ScrollUp, window, cx| {
            if !body_focused(this, window, cx) {
                return;
            }
            this.list().scroll_by(-window.rem_size() * 1.5);
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &ScrollDown, window, cx| {
            if !body_focused(this, window, cx) {
                return;
            }
            this.list().scroll_by(window.rem_size() * 1.5);
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &ScrollLeft, window, cx| {
            if !body_focused(this, window, cx) {
                return;
            }
            let offset = this.horizontal_scroll().offset();
            this.horizontal_scroll().set_offset(point(offset.x + window.rem_size() * 3., offset.y));
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &ScrollRight, window, cx| {
            if !body_focused(this, window, cx) {
                return;
            }
            let offset = this.horizontal_scroll().offset();
            this.horizontal_scroll().set_offset(point(offset.x - window.rem_size() * 3., offset.y));
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &PageUp, window, cx| {
            if !body_focused(this, window, cx) {
                return;
            }
            this.list().scroll_by(-this.viewport().height * 0.9);
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &PageDown, window, cx| {
            if !body_focused(this, window, cx) {
                return;
            }
            this.list().scroll_by(this.viewport().height * 0.9);
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &First, window, cx| {
            if !body_focused(this, window, cx) {
                return;
            }
            this.list().scroll_to(ListOffset::default());
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &Last, window, cx| {
            if !body_focused(this, window, cx) {
                return;
            }
            this.list().scroll_to_end();
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &ExtendUp, window, cx| {
            if !body_focused(this, window, cx) {
                return;
            }
            this.keyboard_select(-1, true, window, cx)
        }))
        .on_action(cx.listener(|this, _: &ExtendDown, window, cx| {
            if !body_focused(this, window, cx) {
                return;
            }
            this.keyboard_select(1, true, window, cx)
        }))
        .on_action(cx.listener(|this, _: &SelectAll, window, cx| {
            if body_focused(this, window, cx) {
                this.select_all(window, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &Copy, window, cx| {
            if !body_focused(this, window, cx) {
                return;
            }
            let text = this.selected_text(cx);
            if text.is_empty() {
                cx.propagate();
            } else {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
        }))
        .child(
            gpui::canvas(
                {
                    let state = cx.entity();
                    move |bounds, _, cx| {
                        state.update(cx, |state, cx| {
                            // Runs after the list's prepaint, which drops
                            // every row height when its width changes.
                            let rehinted = state.keep_row_height_hint();
                            // The estimate shapes the widest candidates only;
                            // grow to fit any wider line once it is painted.
                            let painted = if soft_wrap {
                                px(0.)
                            } else {
                                let painted = state.geometry().borrow().painted_width();
                                code_width(painted, gutter_width, lane_width, rem, mode)
                            };
                            // enkidu: the list's layout can move its top (when the rows no longer fill the view):
                            // then the header that stays was chosen for the wrong file, so draw again
                            let moved = follows_top && state.top_file() != sticky;
                            // enkidu: a line that was to be centered before the view had a height
                            let centered = state.center_when_laid_out();
                            if state.record_paint(bounds.size, painted) || rehinted || moved || centered {
                                cx.notify();
                            }
                        });
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .size_full(),
        )
        .test_support()
        .track_focus(&state.focus_handle(cx))
        .key_context(KEYS)
        .role(gpui::accesskit::Role::Document)
        .aria_label(label);
    v_flex()
        .id(("diff", cx.entity_id()))
        .size_full()
        .min_w_0()
        .min_h_0()
        .bg(cx.theme().background)
        .text_color(cx.theme().foreground)
        .text_sm()
        .border_1()
        .border_color(cx.theme().border)
        .child(body)
        .refine_style(&props.style)
        .into_any_element()
}

fn body_focused(state: &DiffState, window: &Window, cx: &mut Context<DiffState>) -> bool {
    if state.focus_handle(cx).is_focused(window) {
        true
    } else {
        cx.propagate();
        false
    }
}

struct CodePresentation {
    /// enkidu: the view's colors.
    colors: DiffColors,
    /// enkidu: the file whose header is drawn over the rows, at the top of the view. Its header's own row is then left
    /// blank: it is only ever drawn when it is the row at the top, right under the header that stands in for it.
    sticky: Option<usize>,
    /// enkidu: the application's own words for a file without lines.
    notice: Option<NoticeRenderer>,
    files: Vec<DiffFile>,
    presentations: Vec<Option<Arc<FilePresentation>>>,
    mode: DiffMode,
    content_width: Pixels,
    column_width: Pixels,
    font_size: Pixels,
    row_height: Pixels,
    gutter_width: Pixels,
    rem: Pixels,
    expansion_lines: usize,
    line_number: bool,
    syntax_highlight: bool,
    header_visible: bool,
    hover_highlight: DiffHoverHighlight,
    hunk_separator: DiffHunkSeparator,
    change_indicator: DiffChangeIndicator,
    change_background: bool,
    soft_wrap: bool,
    header: HeaderSlots,
    annotations: Rc<Annotations>,
    annotation_content: Option<AnnotationRenderer>,
    decorations: Rc<Decorations>,
    marks: Rc<Marks>,
    change_marks: Rc<ChangeMarks>,
    /// enkidu: the name that is drawn as a link now.
    link: Option<DiffName>,
    gutter_markers: Rc<GutterMarkers>,
    /// Set only while the marker lane is shown.
    gutter_marker_content: Option<MarkerRenderer>,
    lane_width: Pixels,
    on_add_annotation: Option<RangeHandler>,
    on_line_click: Option<ClickHandler>,
    on_line_hover: Option<HoverHandler>,
    selection: gpui_kit::base::TextSelectionHandle,
    selected_lines: Option<DiffLineRange>,
    selected: Option<SelectedLines>,
    geometry: Rc<std::cell::RefCell<selection::SelectionGeometry>>,
    state: Entity<DiffState>,
}

impl CodePresentation {
    fn position(&self, file: usize, side: DiffSide, ix: usize) -> DiffLinePosition {
        let document = &self.files[file];
        DiffLinePosition::new(document.path().clone(), side, document.line_number(side, ix))
    }

    fn is_selected(&self, file: usize, side: DiffSide, ix: usize) -> bool {
        self.selected.as_ref().is_some_and(|selected| selected.contains(&self.files, file, side, ix))
    }
}

/// Total row width for code of `width` in the current mode.
fn code_width(width: Pixels, gutter_width: Pixels, lane_width: Pixels, rem: Pixels, mode: DiffMode) -> Pixels {
    if mode == DiffMode::Split {
        (width + gutter_width + lane_width + rem * 2.) * 2.
    } else {
        width + gutter_width * 2. + lane_width + rem * 2.
    }
}

/// Estimates content width from the lines widest in display cells, then
/// shapes those candidates with the code font for resolved geometry.
#[allow(clippy::too_many_arguments)]
fn measure_code_width(
    files: &[DiffFile],
    font_size: Pixels,
    family: &SharedString,
    gutter_width: Pixels,
    lane_width: Pixels,
    mode: DiffMode,
    window: &mut Window,
    cx: &App,
) -> Pixels {
    // Glyph widths vary within a font, so shape several candidates rather
    // than trusting the single widest by cell count.
    const CANDIDATES: usize = 8;
    let mut candidates: Vec<(usize, &SharedString)> = Vec::with_capacity(CANDIDATES + 1);
    for file in files {
        for side in [DiffSide::Original, DiffSide::Modified] {
            for line in file.lines(side) {
                let cells = unicode_width::UnicodeWidthStr::width(line.display().as_str());
                if candidates.len() == CANDIDATES && candidates.last().is_some_and(|(widest, _)| *widest >= cells) {
                    continue;
                }
                let ix = candidates.partition_point(|(widest, _)| *widest >= cells);
                candidates.insert(ix, (cells, line.display()));
                candidates.truncate(CANDIDATES);
            }
        }
    }
    let font = gpui::Font { family: family.clone(), ..Default::default() };
    let width = candidates
        .into_iter()
        .map(|(_, text)| {
            window
                .text_system()
                .shape_line(
                    text.clone(),
                    font_size,
                    &[TextRun {
                        len: text.len(),
                        font: font.clone(),
                        color: cx.theme().foreground,
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    }],
                    None,
                )
                .width
        })
        .fold(px(0.), Pixels::max);
    code_width(width, gutter_width, lane_width, window.rem_size(), mode)
}

fn render_row(row: &DisplayRow, code: Rc<CodePresentation>, window: &mut Window, cx: &mut App) -> AnyElement {
    match row {
        DisplayRow::File(file) => render_file_header(*file, &code, window, cx),
        DisplayRow::Notice(file) => {
            match code.notice.as_ref().and_then(|render| render(&code.files[*file], window, cx)) {
                Some(notice) => notice,
                None => render_notice(&code.files[*file], cx),
            }
        }
        DisplayRow::Sides(file) => render_sides(*file, &code, cx),
        DisplayRow::Hunk { file, hunk } => render_hunk_separator(*file, *hunk, &code, cx),
        DisplayRow::Fold { file, pairs } => render_fold(*file, pairs.clone(), &code, cx),
        DisplayRow::Conflict { file, conflict, part } => render_conflict(*file, *conflict, *part, &code, cx),
        DisplayRow::Code { file, original, modified, changed } => {
            if code.mode == DiffMode::Split && !code.files[*file].is_single_column() {
                h_flex()
                    .items_stretch()
                    .w_full()
                    .child(render_cell(
                        *file,
                        DiffSide::Original,
                        *original,
                        *modified,
                        *changed,
                        code.clone(),
                        window,
                        cx,
                    ))
                    .child(render_cell(*file, DiffSide::Modified, *modified, *original, *changed, code, window, cx))
                    .into_any_element()
            } else {
                let (side, ix) = match (original, modified) {
                    (_, Some(modified)) => (DiffSide::Modified, *modified),
                    (Some(original), None) => (DiffSide::Original, *original),
                    (None, None) => unreachable!("a code row shows at least one side"),
                };
                render_unified_cell(*file, side, ix, (*original, *modified), *changed, code, window, cx)
            }
        }
    }
}

fn render_hunk_separator(file: usize, hunk: usize, code: &CodePresentation, cx: &App) -> AnyElement {
    let hunks = code.files[file].hunks();
    let row = h_flex().w_full().px_3().bg(code.colors.gap).text_xs().text_color(cx.theme().muted_foreground);
    match code.hunk_separator {
        DiffHunkSeparator::Metadata => row.h_6().child(hunks[hunk].label().clone()).into_any_element(),
        DiffHunkSeparator::LineInfo => {
            let hidden = hunks[hunk].hidden_lines_before(hunk.checked_sub(1).map(|ix| &hunks[ix]));
            if hidden == 0 {
                return div().into_any_element();
            }
            row.h_6().child(t!("Diff.HiddenLines", count = hidden).to_string()).into_any_element()
        }
        DiffHunkSeparator::Simple => row.h_1().into_any_element(),
    }
}

fn render_fold(file: usize, pairs: std::ops::Range<usize>, code: &CodePresentation, cx: &App) -> AnyElement {
    let document = &code.files[file];
    // A fold bordering the hunk start has no source above it, and one
    // bordering the hunk end has none below; offer only the meaningful sides.
    let hunk = document
        .hunks()
        .iter()
        .find(|hunk| hunk.pairs().contains(&pairs.start))
        .map_or(0..document.pairs().len(), |hunk| hunk.pairs());
    let lines = code.expansion_lines;
    let partial = pairs.len() > lines;
    let expand = |id: (&'static str, usize), expansion: FoldExpansion| {
        let state = code.state.clone();
        let pairs = pairs.clone();
        Button::new(id).ghost().xsmall().w_auto().text_color(cx.theme().muted_foreground).on_click(move |_, _, cx| {
            state.update(cx, |state, cx| state.expand_fold(file, pairs.clone(), expansion, cx))
        })
    };
    h_flex()
        .w_full()
        .h_6()
        .px_2()
        .gap_1()
        .bg(code.colors.gap)
        .when(partial && pairs.start > hunk.start, |this| {
            this.child(
                expand(("expand-down", pairs.start), FoldExpansion::Down)
                    .icon(IconName::ArrowDown)
                    .accessibility_label(t!("Diff.ExpandBelow", count = lines).to_string()),
            )
        })
        .when(partial && pairs.end < hunk.end, |this| {
            this.child(
                expand(("expand-up", pairs.start), FoldExpansion::Up)
                    .icon(IconName::ArrowUp)
                    .accessibility_label(t!("Diff.ExpandAbove", count = lines).to_string()),
            )
        })
        .child(
            expand(("expand", pairs.start), FoldExpansion::All)
                .icon(if partial { IconName::ChevronsUpDown } else { IconName::ChevronDown })
                .label(t!("Diff.UnchangedLines", count = pairs.len()).to_string())
                .accessibility_label(t!("Diff.ExpandLines", count = pairs.len()).to_string()),
        )
        .into_any_element()
}

/// The heading of one part of a conflict, with the resolution commands on
/// the current part; a resolved conflict shows its choice and Undo instead.
fn render_conflict(file: usize, ix: usize, part: ConflictPart, code: &CodePresentation, cx: &App) -> AnyElement {
    let document = &code.files[file];
    let conflict = &document.conflicts()[ix];
    let resolution = code.state.read(cx).conflict_resolution(document.path(), ix);
    let choose = |id: &'static str, label: String, resolution: Option<DiffConflictResolution>| {
        let state = code.state.clone();
        Button::new((id, ix))
            .ghost()
            .xsmall()
            .w_auto()
            .label(label)
            .on_click(move |_, _, cx| state.update(cx, |state, cx| state.choose_conflict(file, ix, resolution, cx)))
    };
    let row =
        h_flex().w_full().h_6().px_3().gap_2().text_xs().text_color(cx.theme().muted_foreground).bg(conflict_tint(
            Some(part),
            cx,
        )
        .opacity(0.25));
    if let Some(resolution) = resolution {
        let label = match resolution {
            DiffConflictResolution::Current => t!("Diff.AcceptedCurrent"),
            DiffConflictResolution::Incoming => t!("Diff.AcceptedIncoming"),
            _ => t!("Diff.AcceptedBoth"),
        };
        return row
            .bg(code.colors.gap)
            .child(label.to_string())
            .child(choose("undo-resolution", t!("Diff.UndoResolution").to_string(), None))
            .into_any_element();
    }
    let title = match part {
        ConflictPart::Current => t!("Diff.CurrentChange"),
        ConflictPart::Base => t!("Diff.BaseChange"),
        ConflictPart::Incoming => t!("Diff.IncomingChange"),
    };
    let label = conflict.label(part).clone();
    row.child(div().font_medium().child(title.to_string()))
        .when(!label.is_empty(), |this| this.child(label))
        .when(part == ConflictPart::Current, |this| {
            this.child(div().flex_1())
                .child(choose(
                    "accept-current",
                    t!("Diff.AcceptCurrent").to_string(),
                    Some(DiffConflictResolution::Current),
                ))
                .child(choose(
                    "accept-incoming",
                    t!("Diff.AcceptIncoming").to_string(),
                    Some(DiffConflictResolution::Incoming),
                ))
                .child(choose("accept-both", t!("Diff.AcceptBoth").to_string(), Some(DiffConflictResolution::Both)))
        })
        .into_any_element()
}

/// The color that marks lines of a conflict part.
fn conflict_tint(part: Option<ConflictPart>, cx: &App) -> Hsla {
    match part {
        Some(ConflictPart::Current) => cx.theme().success,
        Some(ConflictPart::Incoming) => cx.theme().info,
        _ => cx.theme().muted_foreground,
    }
}

/// enkidu: a file's header row. The header itself is [`file_header_bar`]; for the file at the top of the view the bar is
/// drawn over the rows instead (so it stays in view), and this row keeps its place blank.
fn render_file_header(file: usize, code: &CodePresentation, window: &mut Window, cx: &mut App) -> AnyElement {
    if !code.header_visible {
        return div().children(file_annotations(file, code, window, cx)).into_any_element();
    }
    v_flex()
        .id("file-header")
        .w_full()
        .child(if code.sticky == Some(file) {
            div().h(HEADER_HEIGHT).into_any_element()
        } else {
            file_header_bar(file, false, code, window, cx)
        })
        .children(file_annotations(file, code, window, cx))
        .into_any_element()
}

/// The band with a file's collapse control and its header content. enkidu: of a fixed height and opaque (see
/// [`HEADER_HEIGHT`]); in the original it was part of the header row. `stays` is the one drawn over the rows at the
/// top of the view: it is the same band in the same place as the row it stands in for, without the hairline that
/// parts a file from the one above it.
fn file_header_bar(file: usize, stays: bool, code: &CodePresentation, window: &mut Window, cx: &mut App) -> AnyElement {
    let document = &code.files[file];
    let foreground = cx.theme().muted_foreground;
    let collapsed = code.state.read(cx).is_file_collapsed(document.path());
    let content = if let Some(render) = &code.header.content {
        render(document, window, cx)
    } else {
        let name: SharedString = match (document.original_path(), document.modified_path()) {
            (Some(old), Some(new)) if old != new => format!("{old} → {new}").into(),
            _ => document.path().clone(),
        };
        let slot = |render: &Option<FileRenderer>, window: &mut Window, cx: &mut App| {
            render.as_ref().map(|render| render(document, window, cx))
        };
        let prefix = slot(&code.header.prefix, window, cx);
        let title_suffix = slot(&code.header.title_suffix, window, cx);
        let suffix = slot(&code.header.suffix, window, cx);
        let status = match document.status() {
            DiffFileStatus::Added => Some(t!("Diff.AddedFile")),
            DiffFileStatus::Deleted => Some(t!("Diff.DeletedFile")),
            DiffFileStatus::Renamed => Some(t!("Diff.RenamedFile")),
            DiffFileStatus::Copied => Some(t!("Diff.CopiedFile")),
            DiffFileStatus::Conflicted => Some(t!("Diff.ConflictedFile")),
            _ => None,
        };
        let statistics = !document.is_single_column();
        h_flex()
            .w_full()
            .min_w_0()
            .gap_2()
            .children(prefix)
            .child(
                h_flex()
                    .min_w_0()
                    .flex_1()
                    .gap_2()
                    .child(div().min_w_0().truncate().font_medium().child(name))
                    .when_some(title_suffix, |this, suffix| this.child(div().flex_shrink_0().child(suffix))),
            )
            .when(statistics, |this| {
                this.child(div().text_xs().text_color(foreground).child(format!(
                    "+{} −{}",
                    document.additions(),
                    document.deletions()
                )))
            })
            .when_some(status, |this, status| {
                this.child(div().text_xs().text_color(foreground).child(status.to_string()))
            })
            .children(suffix)
            .into_any_element()
    };
    let state = code.state.clone();
    let name = document.path().clone();
    // the hairline between two files is the top line of the lower one's band: the band is as tall with it as without
    let parting = if file > 0 && !stays { code.colors.rule } else { code.colors.header };
    v_flex()
        .id(("file-bar", file))
        .test_support()
        .debug_selector(move || format!("file-bar-{file}"))
        .w_full()
        .min_w_0()
        .h(HEADER_HEIGHT)
        .bg(code.colors.header)
        .child(div().w_full().h(px(1.)).flex_shrink_0().bg(parting))
        .child(
            h_flex()
                .w_full()
                .min_w_0()
                .flex_1()
                .pl_1()
                .pr_3()
                .gap_1()
                .child(
                    Button::new("collapse-file")
                        .ghost()
                        .xsmall()
                        .icon(if collapsed { IconName::ChevronRight } else { IconName::ChevronDown })
                        .accessibility_label(
                            if collapsed {
                                t!("Diff.ExpandFile", name = name.as_str())
                            } else {
                                t!("Diff.CollapseFile", name = name.as_str())
                            }
                            .to_string(),
                        )
                        .on_click(move |_, _, cx| state.update(cx, |state, cx| state.toggle_file_collapsed(file, cx))),
                )
                .child(div().flex_1().min_w_0().child(content)),
        )
        .child(div().w_full().h(px(1.)).flex_shrink_0().bg(code.colors.rule))
        .into_any_element()
}

fn file_annotations(file: usize, code: &CodePresentation, window: &mut Window, cx: &mut App) -> Vec<AnyElement> {
    let path = code.files[file].path();
    let ixs = code.annotations.files.get(path).cloned().unwrap_or_default();
    annotation_elements(&ixs, code, window, cx)
}

/// enkidu: the two sides of a file shown side by side, named over their columns (a new file has nothing before, a
/// deleted one nothing after).
fn render_sides(file: usize, code: &CodePresentation, cx: &App) -> AnyElement {
    let status = code.files[file].status();
    let before = if status == DiffFileStatus::Added { t!("Diff.BeforeNew") } else { t!("Diff.Before") };
    let after = if status == DiffFileStatus::Deleted { t!("Diff.AfterDeleted") } else { t!("Diff.After") };
    let side = |id: &'static str, words: std::borrow::Cow<'static, str>| {
        div()
            .id(id)
            .test_support()
            .aria_label(words.to_string())
            .w(code.column_width)
            .flex_shrink_0()
            .h_6()
            .px_3()
            .flex()
            .items_center()
            .border_b_1()
            .border_color(code.colors.rule)
            .child(words.to_string())
    };
    h_flex()
        .w_full()
        .text_size(px(11.))
        .font_medium()
        .text_color(cx.theme().muted_foreground)
        .child(side("split-before", before).border_r_1())
        .child(side("split-after", after))
        .into_any_element()
}

/// Summarizes a file without source rows: binary, metadata-only or empty.
fn render_notice(file: &DiffFile, cx: &App) -> AnyElement {
    let message = if !file.has_changes() {
        t!("Diff.NoChanges")
    } else if file.is_binary() {
        t!("Diff.BinaryChanges")
    } else if !file.extended_headers().is_empty() {
        t!("Diff.NoTextChanges")
    } else {
        t!("Diff.EmptyFile")
    };
    // enkidu: the words alone (the original listed git's header lines above them)
    div()
        .id("file-note")
        .test_support()
        .aria_label(message.to_string())
        .w_full()
        .px_4()
        .py_4()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(message.to_string())
        .into_any_element()
}

fn render_gutter(file: usize, side: DiffSide, ix: Option<usize>, code: &CodePresentation, cx: &App) -> AnyElement {
    let hover = matches!(code.hover_highlight, DiffHoverHighlight::LineNumber | DiffHoverHighlight::Both);
    div()
        .id(if side == DiffSide::Original { "old-gutter" } else { "new-gutter" })
        .flex_shrink_0()
        .w(code.gutter_width)
        .pr_1()
        .when(hover, |this| this.hover(|this| this.bg(cx.theme().list_hover)))
        .when_some(ix, |this, ix| {
            let position = code.position(file, side, ix);
            let down = code.state.clone();
            let moved = code.state.clone();
            let dragged = position.clone();
            // A plain hit target: line numbers need no button chrome or state.
            this.child(
                div()
                    .id(("line", ix))
                    .test_support()
                    .w_full()
                    .h(code.row_height)
                    .px_1()
                    .flex()
                    .items_center()
                    .justify_end()
                    .text_size(code.font_size)
                    .text_color(cx.theme().muted_foreground)
                    .hover(|this| this.text_color(cx.theme().foreground))
                    .role(gpui::accesskit::Role::Button)
                    // enkidu: a file shown whole has one side, so its line is named by its number alone
                    .aria_label(if code.files[file].is_single_column() {
                        t!("Diff.LineOfFile", line = position.line()).to_string()
                    } else {
                        t!("Diff.SelectLine", side = side_label(side), line = position.line()).to_string()
                    })
                    .child(position.line().to_string())
                    .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                        down.update(cx, |state, cx| {
                            state.begin_line_selection(position.clone(), event.modifiers.shift, window, cx)
                        })
                    })
                    .on_mouse_move(move |event, _, cx| {
                        if event.pressed_button == Some(MouseButton::Left) {
                            moved.update(cx, |state, cx| state.drag_line_selection(dragged.clone(), cx))
                        }
                    }),
            )
        })
        .into_any_element()
}

fn side_label(side: DiffSide) -> std::borrow::Cow<'static, str> {
    if side == DiffSide::Original { t!("Diff.Original") } else { t!("Diff.Modified") }
}

#[allow(clippy::too_many_arguments)]
fn render_cell(
    file: usize,
    side: DiffSide,
    ix: Option<usize>,
    other_ix: Option<usize>,
    changed: bool,
    code: Rc<CodePresentation>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let cell = v_flex()
        .w(code.column_width)
        .min_w_0()
        .flex_shrink_0()
        .when(side == DiffSide::Original, |this| this.border_r_1().border_color(code.colors.rule))
        .when(ix.is_none(), |this| this.bg(code.colors.gap).child(div().h(code.row_height)));
    let Some(ix) = ix else {
        return cell.into_any_element();
    };
    cell.id((if side == DiffSide::Original { "old" } else { "new" }, ix))
        .child(code_line(file, side, ix, changed, code.clone(), [Some((side, Some(ix))), None], window, cx))
        .children(line_extras(file, side, ix, other_ix, changed, &code, window, cx))
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn render_unified_cell(
    file: usize,
    side: DiffSide,
    ix: usize,
    pair: (Option<usize>, Option<usize>),
    changed: bool,
    code: Rc<CodePresentation>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let (original, modified) = pair;
    let single_column = code.files[file].is_single_column();
    let counterpart = code.files[file].lines(side)[ix].counterpart();
    let other_ix = if side == DiffSide::Original { modified } else { original };
    let mut extras = line_extras(file, side, ix, counterpart, changed, &code, window, cx);
    if !changed && let Some(other_ix) = other_ix {
        extras.extend(line_annotations(file, side.other(), other_ix, &code, window, cx));
    }
    let gutters = if single_column {
        [Some((DiffSide::Modified, modified)), None]
    } else {
        [Some((DiffSide::Original, original)), Some((DiffSide::Modified, modified))]
    };
    v_flex()
        .w(if single_column { code.content_width } else { code.column_width })
        .id((if side == DiffSide::Original { "old" } else { "new" }, ix))
        .child(code_line(file, side, ix, changed, code.clone(), gutters, window, cx))
        .children(extras)
        .into_any_element()
}

/// Syntax colors with inline-change and selection emphasis layered over them.
#[allow(clippy::too_many_arguments)]
fn line_highlights(
    file: usize,
    side: DiffSide,
    ix: usize,
    changed: bool,
    selected: bool,
    word: Hsla,
    code: &CodePresentation,
    cx: &App,
) -> Vec<(std::ops::Range<usize>, HighlightStyle)> {
    let line = &code.files[file].lines(side)[ix];
    let presentation = code.presentations.get(file).and_then(Option::as_ref);
    let mut highlights = match presentation {
        Some(presentation) if code.syntax_highlight => {
            presentation.syntax_highlights(line, side, ix, cx.theme().highlight_theme.as_ref())
        }
        _ => Vec::new(),
    };
    if changed && let Some(presentation) = presentation {
        highlights.extend(
            presentation
                .inline_changes(line, side, ix)
                .map(|range| (range, HighlightStyle { background_color: Some(word), ..Default::default() })),
        );
    }
    // enkidu: what the application marked on this line (a search's matches), where the line is long enough for it
    if !code.marks.is_empty() {
        let position = DiffLinePosition::new(code.files[file].path().clone(), side, line.line_number());
        let length = line.text().len();
        highlights.extend(
            code.marks
                .at(&position)
                .iter()
                .filter(|(range, _)| {
                    range.start < range.end
                        && range.end <= length
                        && line.text().is_char_boundary(range.start)
                        && line.text().is_char_boundary(range.end)
                })
                .map(|(range, background)| {
                    let style = HighlightStyle { background_color: Some(*background), ..Default::default() };
                    (line.display_range(range.clone()), style)
                }),
        );
    }
    if selected {
        highlights.push((
            0..line.display().len(),
            HighlightStyle { background_color: Some(code.colors.selected), ..Default::default() },
        ));
    }
    // enkidu: a name the application said is a link, while ⌘ is held over it: in the link's color, underlined
    if let Some(link) = &code.link
        && link.position.side() == side
        && link.position.line() == line.line_number()
        && link.position.path() == code.files[file].path()
        && link.range.start < link.range.end
        && link.range.end <= line.text().len()
        && line.text().is_char_boundary(link.range.start)
        && line.text().is_char_boundary(link.range.end)
    {
        let underline = gpui::UnderlineStyle { thickness: px(1.), color: Some(code.colors.link), wavy: false };
        highlights.push((
            line.display_range(link.range.clone()),
            HighlightStyle { color: Some(code.colors.link), underline: Some(underline), ..Default::default() },
        ));
    }
    highlights
}

/// The add-annotation button, shown on hover and at the end of the selection.
fn add_annotation_button(
    file: usize,
    side: DiffSide,
    ix: usize,
    left: Pixels,
    code: &CodePresentation,
    cx: &App,
) -> Option<AnyElement> {
    let handler = code.on_add_annotation.clone()?;
    let position = code.position(file, side, ix);
    let range = code
        .selected_lines
        .clone()
        .filter(|_| code.is_selected(file, side, ix))
        .unwrap_or_else(|| DiffLineRange::new(position.path().clone(), side, position.line(), position.line()));
    let at_selection_end = code.selected.as_ref().is_some_and(|selected| selected.is_last(&code.files, file, side, ix));
    Some(
        div()
            .id(("add-annotation", ix))
            .test_support()
            .absolute()
            .top_0()
            .left(left)
            .size(code.row_height)
            .flex()
            .items_center()
            .justify_center()
            .rounded(cx.theme().radius)
            .bg(cx.theme().primary)
            .text_color(cx.theme().primary_foreground)
            .when(!at_selection_end, |this| this.opacity(0.).group_hover("diff-line", |this| this.opacity(1.)))
            .role(gpui::accesskit::Role::Button)
            .aria_label(t!("Diff.AddAnnotation", line = position.line()).to_string())
            .child(Icon::new(IconName::Plus).xsmall())
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, window, cx| handler(&range, window, cx))
            .into_any_element(),
    )
}

#[allow(clippy::too_many_arguments)]
fn code_line(
    file: usize,
    side: DiffSide,
    ix: usize,
    changed: bool,
    code: Rc<CodePresentation>,
    gutters: [Option<(DiffSide, Option<usize>)>; 2],
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let document = &code.files[file];
    let line = &document.lines(side)[ix];
    let conflict_part = (document.status() == DiffFileStatus::Conflicted && changed)
        .then(|| {
            document
                .conflicts()
                .iter()
                .find(|conflict| conflict.source_lines().contains(&ix))
                .and_then(|conflict| conflict.part_of(ix))
        })
        .flatten();
    // enkidu: a changed line's three colors (its fill, the words changed inside it, its sign) are the view's own; the
    // original derived all three from one theme color
    let (fill, word, sign) = if conflict_part.is_some() {
        let tint = conflict_tint(conflict_part, cx);
        (tint.opacity(0.12), tint.opacity(0.3), tint)
    } else if side == DiffSide::Original {
        (code.colors.removed, code.colors.removed_word, code.colors.removed_sign)
    } else {
        (code.colors.added, code.colors.added_word, code.colors.added_sign)
    };
    let selected = code.is_selected(file, side, ix)
        || (code.mode == DiffMode::Unified
            && !changed
            && gutters.iter().flatten().any(|(side, ix)| ix.is_some_and(|ix| code.is_selected(file, *side, ix))));
    let highlights = line_highlights(file, side, ix, changed, selected, word, &code, cx);
    // An unchanged Unified row shows one source line for both sides; follow
    // the side a text selection started on so its offsets stay consistent.
    let (text_side, text_ix) = if code.mode == DiffMode::Unified
        && !changed
        && let Some((selected_file, selected_side, _)) = selection::selected_source_range(&code.selection, cx)
        && selected_file == file
        && selected_side != side
        && let Some(other_ix) = line.counterpart()
    {
        (selected_side, other_ix)
    } else {
        (side, ix)
    };
    let position = code.position(file, text_side, text_ix);
    let source_line = &document.lines(text_side)[text_ix];
    // Wrapped text must be one run to break across the column; unwrapped
    // long lines stay chunked to bound shaping and selection projection.
    let ranges: Vec<_> =
        if code.soft_wrap { vec![0..source_line.display().len()] } else { source_line.chunk_ranges().collect() };
    // A block container gives wrapped text a definite width to break at.
    let text = div()
        .map(|this| if code.soft_wrap { this.w_full() } else { this.flex().flex_row().flex_shrink_0() })
        .children(ranges.into_iter().enumerate().map(|(chunk_ix, range)| {
            CodeText::new(
                ("code", chunk_ix).into(),
                document.clone(),
                file,
                text_side,
                text_ix,
                range,
                highlights.clone(),
                code.selection.clone(),
                code.geometry.clone(),
            )
        }));
    let gutters_width =
        if code.line_number { code.gutter_width * gutters.iter().flatten().count() as f32 } else { px(0.) };
    let indicator = match code.change_indicator {
        DiffChangeIndicator::Signs => {
            div().w_4().flex_shrink_0().text_color(if changed { sign } else { cx.theme().muted_foreground }).child(
                match (changed && conflict_part.is_none(), side) {
                    (false, _) => " ",
                    (true, DiffSide::Original) => "−",
                    (true, DiffSide::Modified) => "+",
                },
            )
        }
        DiffChangeIndicator::Bars => {
            div().w_4().h_full().flex_shrink_0().when(changed, |this| this.child(div().w(px(3.)).h_full().bg(sign)))
        }
        DiffChangeIndicator::None => div().w_2().flex_shrink_0(),
    };
    let hover_line = matches!(code.hover_highlight, DiffHoverHighlight::Line | DiffHoverHighlight::Both);
    let add_button = add_annotation_button(file, side, ix, gutters_width + code.lane_width + code.rem / 2., &code, cx);
    // Unified rows of unchanged lines carry both sides; either may be targeted.
    let positions: Vec<_> =
        gutters.iter().flatten().filter_map(|(side, ix)| ix.map(|ix| code.position(file, *side, ix))).collect();
    let decoration = positions
        .iter()
        .find_map(|position| code.decorations.at(position))
        .filter(|_| !selected)
        .map(|decoration| decoration.background().unwrap_or_else(|| cx.theme().warning.opacity(0.2)));
    // enkidu: what differs from the version the file is compared with, as a stripe beside the line's number
    let change_mark = positions
        .iter()
        .find(|position| position.side() == DiffSide::Modified)
        .filter(|_| !code.change_marks.is_empty())
        .map(|position| {
            let (stripe, gone_after) = code.change_marks.at(position.path(), position.line());
            let gone_before = position.line() == 1 && code.change_marks.removed_before_first(position.path());
            (position.line(), stripe, gone_after, gone_before)
        })
        .filter(|(_, stripe, gone_after, gone_before)| stripe.is_some() || *gone_after || *gone_before);
    // after the row's own padding, the line numbers and the marker lane: where the signs of a diff stand
    let mark_left = code.rem / 2. + gutters_width + code.lane_width + px(2.);
    let marker_lane = code.gutter_marker_content.clone().map(|render| {
        let marker = positions.iter().find_map(|position| code.gutter_markers.at(position));
        div()
            .id("marker-lane")
            .flex_shrink_0()
            .size(code.row_height)
            .flex()
            .items_center()
            .justify_center()
            // Keep presses on a marker from starting a text or line selection.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .when_some(marker, |this, marker| this.child(render(marker, window, cx)))
    });
    h_flex()
        .id("line-row")
        .group("diff-line")
        .relative()
        .w_full()
        .px_2()
        .map(|this| {
            if code.soft_wrap {
                this.min_h(code.row_height).items_start().whitespace_normal()
            } else {
                this.h(code.row_height).whitespace_nowrap()
            }
        })
        .font_family(cx.theme().mono_font_family.clone())
        // enkidu: code is read letter by letter: `->` stays `->`
        .font_features(gpui::FontFeatures::disable_ligatures())
        .text_size(code.font_size)
        .line_height(code.row_height)
        .when(changed && code.change_background, |this| this.bg(fill))
        // An overlay, so the change background shows through a translucent fill.
        .when_some(decoration, |this, background| this.child(div().absolute().inset_0().bg(background)))
        .when_some(change_mark, |this, (line, stripe, gone_after, gone_before)| {
            let stripe = stripe.map(|kind| {
                let (color, words) = match kind {
                    DiffChangeKind::Added => (code.colors.mark_added, t!("Diff.MarkAdded", line = line)),
                    _ => (code.colors.mark_changed, t!("Diff.MarkChanged", line = line)),
                };
                div()
                    .id(("change-mark", line))
                    .test_support()
                    .debug_selector(move || format!("change-mark-{line}"))
                    .role(gpui::accesskit::Role::Label)
                    .aria_label(words.to_string())
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(mark_left)
                    .w(px(3.))
                    .bg(color)
            });
            // where lines went: a small mark at the foot of the line they stood after (or at the head of the
            // first line, when they stood above it)
            let gone = |after: bool| {
                let at = if after { line } else { 0 };
                div()
                    .id(("change-gone", at))
                    .test_support()
                    .debug_selector(move || format!("change-gone-{at}"))
                    .role(gpui::accesskit::Role::Label)
                    .aria_label(
                        if after { t!("Diff.MarkRemoved", line = at) } else { t!("Diff.MarkRemovedAbove") }.to_string(),
                    )
                    .absolute()
                    .map(|mark| if after { mark.bottom_0() } else { mark.top_0() })
                    .left(mark_left)
                    .w(px(8.))
                    .h(px(2.))
                    .bg(code.colors.mark_removed)
            };
            this.children(stripe).children(gone_after.then(|| gone(true))).children(gone_before.then(|| gone(false)))
        })
        .when(selected, |this| this.bg(code.colors.selected))
        .when(hover_line && !selected, |this| this.hover(|this| this.bg(cx.theme().list_hover)))
        .when_some(code.on_line_hover.clone(), |this, handler| {
            let position = position.clone();
            this.on_hover(move |hovered, window, cx| handler(hovered.then_some(&position), window, cx))
        })
        .when(code.line_number, |this| {
            this.children(gutters.into_iter().flatten().map(|(side, ix)| render_gutter(file, side, ix, &code, cx)))
        })
        .children(marker_lane)
        .child(indicator)
        .child(
            div()
                .id("source")
                .test_support()
                .map(|this| if code.soft_wrap { this.flex_1().min_w_0() } else { this.flex_shrink_0() })
                .role(gpui::accesskit::Role::Label)
                .aria_label(source_line.text().clone())
                .aria_description({
                    let which = t!(
                        if changed {
                            if side == DiffSide::Original { "Diff.RemovedLine" } else { "Diff.AddedLine" }
                        } else if code.files[file].is_single_column() {
                            // enkidu: a file shown whole has one side
                            "Diff.LineOfFile"
                        } else {
                            "Diff.SourceLine"
                        },
                        side = side_label(text_side),
                        line = position.line()
                    );
                    // enkidu: a line the application marked says so, with the words marked inside it
                    let text = source_line.text();
                    let words: Vec<&str> =
                        (code.marks.at(&position).iter()).filter_map(|(range, _)| text.get(range.clone())).collect();
                    if !words.is_empty() {
                        format!("{which}{}", t!("Diff.MarkedWords", words = words.join(" | ")))
                    } else if decoration.is_some() {
                        format!("{which}{}", t!("Diff.Marked"))
                    } else {
                        which.to_string()
                    }
                })
                .when_some(code.on_line_click.clone(), |this, handler| {
                    let position = position.clone();
                    this.on_click(move |event, window, cx| {
                        // A drag selects text; only a stationary press is a click.
                        if let ClickEvent::Mouse(mouse) = event {
                            let moved = mouse.up.position - mouse.down.position;
                            if moved.x.abs() > px(3.) || moved.y.abs() > px(3.) {
                                return;
                            }
                        }
                        handler(&position, event, window, cx)
                    })
                })
                .child(text),
        )
        .children(add_button)
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn line_extras(
    file: usize,
    side: DiffSide,
    ix: usize,
    other_ix: Option<usize>,
    changed: bool,
    code: &CodePresentation,
    window: &mut Window,
    cx: &mut App,
) -> Vec<AnyElement> {
    let mut extras = Vec::new();
    let document = &code.files[file];
    let line = &document.lines(side)[ix];
    let ending = &document.source(side)[line.content_end()..line.source().end];
    let ending_only = changed && has_line_ending_change(document, side, ix, other_ix);
    if ending.is_empty() || ending_only {
        let label = if ending.is_empty() {
            t!("Diff.NoNewline").to_string()
        } else {
            format!("\\ {}", if ending == "\r\n" { "CRLF" } else { "LF" })
        };
        extras
            .push(div().h_6().px_3().text_xs().text_color(cx.theme().muted_foreground).child(label).into_any_element());
    }
    extras.extend(line_annotations(file, side, ix, code, window, cx));
    extras
}

fn line_annotations(
    file: usize,
    side: DiffSide,
    ix: usize,
    code: &CodePresentation,
    window: &mut Window,
    cx: &mut App,
) -> Vec<AnyElement> {
    let position = code.position(file, side, ix);
    let ixs = code.annotations.lines.get(&position).cloned().unwrap_or_default();
    annotation_elements(&ixs, code, window, cx)
}

fn annotation_elements(ixs: &[usize], code: &CodePresentation, window: &mut Window, cx: &mut App) -> Vec<AnyElement> {
    let Some(render) = &code.annotation_content else {
        return Vec::new();
    };
    if ixs.is_empty() {
        return Vec::new();
    }
    // enkidu: the notes of one line stand together in one band, 8 px apart, between two hairlines (the original gave
    // each its own band, so two notes on a line had a doubled line between them)
    let notes = ixs.iter().map(|ix| {
        let annotation = &code.annotations.items[*ix];
        div().id(annotation.id.clone()).w_full().child(render(annotation, window, cx))
    });
    vec![
        v_flex()
            .w_full()
            .px_3()
            .py_2()
            .gap_2()
            .border_t_1()
            .border_b_1()
            .border_color(code.colors.rule)
            .bg(code.colors.note)
            // enkidu: a note keeps its own presses. Its buttons and its text field say so themselves
            // (`GlobalState::suppress_text_selection`), or the selection would hand the keyboard back to this view
            // a moment later; its words don't, so they can be selected like the code around them
            .children(notes)
            .into_any_element(),
    ]
}

fn has_line_ending_change(file: &DiffFile, side: DiffSide, ix: usize, counterpart: Option<usize>) -> bool {
    counterpart.is_some_and(|counterpart| {
        let other_side = side.other();
        let line = &file.lines(side)[ix];
        let other = &file.lines(other_side)[counterpart];
        line.text() == other.text()
            && file.source(side)[line.content_end()..line.source().end]
                != file.source(other_side)[other.content_end()..other.source().end]
    })
}

/// enkidu: what a test that draws the view needs first (the original's tests called gpui-component's own `init`).
#[cfg(test)]
pub(crate) fn init_for_tests(cx: &mut App) {
    gpui_kit::init(cx);
    init(cx);
}

#[cfg(test)]
mod basics_tests;
#[cfg(test)]
mod copy_tests;
#[cfg(test)]
mod decoration_tests;
#[cfg(test)]
mod extras_tests;
#[cfg(test)]
mod header_tests;
#[cfg(test)]
mod keep_tests;
#[cfg(test)]
mod newline_tests;
#[cfg(test)]
mod review_tests;
#[cfg(test)]
mod scroll_tests;
#[cfg(test)]
mod selection_tests;
#[cfg(test)]
mod sticky_tests;
#[cfg(test)]
mod tests;
