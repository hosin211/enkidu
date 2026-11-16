use std::collections::HashMap;

use gpui::{ElementId, Hsla, SharedString};

use super::{DiffLinePosition, DiffLineRange, DiffSide};

/// A background that marks source lines in place, without changing the
/// selection, copying or layout.
///
/// Decorations paint above the change background and below inline emphasis,
/// search highlights and text; a selected line shows the selection instead.
/// Where several decorations cover a line, the last supplied one is shown.
#[derive(Clone)]
pub struct DiffDecoration {
    range: DiffLineRange,
    background: Option<Hsla>,
}

impl DiffDecoration {
    /// Marks the lines of `range`.
    ///
    /// A range that ends on the other side covers its start side from
    /// `start` onwards and its end side up to `end`.
    pub fn new(range: DiffLineRange) -> Self {
        Self { range, background: None }
    }
    /// Sets the background, which defaults to a translucent warning color.
    pub fn with_background(mut self, background: impl Into<Hsla>) -> Self {
        self.background = Some(background.into());
        self
    }
    pub fn range(&self) -> &DiffLineRange {
        &self.range
    }
    /// The background set with [`Self::with_background`].
    pub fn background(&self) -> Option<Hsla> {
        self.background
    }

    fn contains(&self, side: DiffSide, line: usize) -> bool {
        let range = &self.range;
        if range.is_single_side() {
            side == range.side() && (range.start()..=range.end()).contains(&line)
        } else if side == range.side() {
            line >= range.start()
        } else {
            line <= range.end()
        }
    }
}

/// An interactive marker in the lane between the line numbers and the code,
/// such as a comment or diagnostic indicator that opens a details panel.
///
/// The application renders it with [`super::Diff::render_gutter_marker`],
/// usually as a small button, and owns the panel and its focus.
#[derive(Clone)]
pub struct DiffGutterMarker {
    id: ElementId,
    position: DiffLinePosition,
    label: SharedString,
}

impl DiffGutterMarker {
    /// Places a marker beside a source line.
    pub fn new(id: impl Into<ElementId>, position: DiffLinePosition) -> Self {
        Self { id: id.into(), position, label: SharedString::default() }
    }
    /// Sets the accessible label that the rendered marker should announce.
    pub fn with_label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = label.into();
        self
    }
    pub fn id(&self) -> &ElementId {
        &self.id
    }
    pub fn position(&self) -> &DiffLinePosition {
        &self.position
    }
    pub fn label(&self) -> &SharedString {
        &self.label
    }
}

/// Decorations by file path, in supplied order.
#[derive(Default)]
pub(super) struct Decorations {
    files: HashMap<SharedString, Vec<DiffDecoration>>,
}

impl Decorations {
    pub(super) fn new(decorations: impl IntoIterator<Item = DiffDecoration>) -> Self {
        let mut index = Self::default();
        for decoration in decorations {
            index.files.entry(decoration.range.path().clone()).or_default().push(decoration);
        }
        index
    }

    /// The decoration shown on a line: the last supplied one covering it.
    pub(super) fn at(&self, position: &DiffLinePosition) -> Option<&DiffDecoration> {
        self.files
            .get(position.path())?
            .iter()
            .rev()
            .find(|decoration| decoration.contains(position.side(), position.line()))
    }
}

/// Gutter markers by line, keeping the first supplied for each line.
#[derive(Default)]
pub(super) struct GutterMarkers {
    lines: HashMap<DiffLinePosition, DiffGutterMarker>,
}

impl GutterMarkers {
    pub(super) fn new(markers: impl IntoIterator<Item = DiffGutterMarker>) -> Self {
        let mut index = Self::default();
        for marker in markers {
            index.lines.entry(marker.position.clone()).or_insert(marker);
        }
        index
    }

    pub(super) fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub(super) fn at(&self, position: &DiffLinePosition) -> Option<&DiffGutterMarker> {
        self.lines.get(position)
    }
}

/// enkidu: a name in the code, as the pointer finds it while ⌘ is held (item 10.10): the line it is on, its
/// stretch of the line's text, and the byte the pointer is at (all in bytes of the line's text). The application
/// may say it is a link ([`super::DiffState::set_link`]); the view then draws it as one and tells the click.
#[derive(Clone, Debug, PartialEq)]
pub struct DiffName {
    pub position: DiffLinePosition,
    pub range: std::ops::Range<usize>,
    pub column: usize,
}

/// enkidu: a stretch of one source line marked in place, such as what a search found on the line that was opened. It
/// is painted like the words changed inside a line: above the line's own background, under the text.
#[derive(Clone)]
pub struct DiffMark {
    position: DiffLinePosition,
    /// The bytes of the line's text that are marked.
    range: std::ops::Range<usize>,
    background: Hsla,
}

impl DiffMark {
    pub fn new(position: DiffLinePosition, range: std::ops::Range<usize>, background: impl Into<Hsla>) -> Self {
        Self { position, range, background: background.into() }
    }
}

/// Marks by line, in supplied order.
#[derive(Default)]
pub(super) struct Marks {
    lines: HashMap<DiffLinePosition, Vec<(std::ops::Range<usize>, Hsla)>>,
}

impl Marks {
    pub(super) fn new(marks: impl IntoIterator<Item = DiffMark>) -> Self {
        let mut index = Self::default();
        for mark in marks {
            index.lines.entry(mark.position).or_default().push((mark.range, mark.background));
        }
        index
    }

    pub(super) fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub(super) fn at(&self, position: &DiffLinePosition) -> &[(std::ops::Range<usize>, Hsla)] {
        self.lines.get(position).map_or(&[], Vec::as_slice)
    }
}

/// enkidu: what differs in the files shown, as marks beside their line numbers ([`super::Diff::change_marks`]): each
/// file's stretches, in line order.
#[derive(Default)]
pub(super) struct ChangeMarks {
    files: HashMap<SharedString, Vec<super::DiffChangeMark>>,
}

impl ChangeMarks {
    pub(super) fn new(files: impl IntoIterator<Item = (SharedString, Vec<super::DiffChangeMark>)>) -> Self {
        let mut index = Self::default();
        for (path, mut marks) in files {
            if marks.is_empty() {
                continue;
            }
            marks.sort_by_key(|mark| (mark.start, mark.end));
            index.files.insert(path, marks);
        }
        index
    }

    pub(super) fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// A line of the modified side: the stripe it carries (added or changed), and whether lines were removed
    /// right after it.
    pub(super) fn at(&self, path: &SharedString, line: usize) -> (Option<super::DiffChangeKind>, bool) {
        let Some(marks) = self.files.get(path) else {
            return (None, false);
        };
        // the marks that start at or before the line: the one it is in, and a removal standing on it, are among
        // the last few of them (stretches don't overlap; a removal has no length)
        let upto = marks.partition_point(|mark| mark.start <= line);
        let (mut stripe, mut gone) = (None, false);
        for mark in marks[..upto].iter().rev().take(3) {
            match mark.kind {
                super::DiffChangeKind::Removed => gone |= mark.start == line,
                kind if mark.end >= line => stripe = stripe.or(Some(kind)),
                _ => {}
            }
        }
        (stripe, gone)
    }

    /// Lines were removed above the file's first line.
    pub(super) fn removed_before_first(&self, path: &SharedString) -> bool {
        self.files.get(path).is_some_and(|marks| {
            marks.first().is_some_and(|mark| mark.kind == super::DiffChangeKind::Removed && mark.start == 0)
        })
    }
}
