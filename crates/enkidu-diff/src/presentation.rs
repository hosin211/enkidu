//! Presentation prepared on a background thread: syntax captures, as
//! theme-independent highlight names or an application highlighter's styles,
//! and inline changes between paired lines.
use std::{ops::Range, sync::Arc};

use gpui::{HighlightStyle, SharedString};
use gpui_kit::base::input::HighlightStyleResolver;
use similar::{ChangeTag, TextDiff};

use super::{DiffFile, DiffSide, document::SourceLine};

/// The unit inline changes are compared and emphasized in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DiffInlineUnit {
    /// Words and punctuation; changed words separated only by whitespace are
    /// emphasized as one run.
    #[default]
    Word,
    /// Grapheme clusters.
    Character,
}

/// Settings that determine prepared presentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PresentationOptions {
    pub inline_unit: Option<DiffInlineUnit>,
    pub inline_max_line_length: usize,
    pub syntax_max_line_length: usize,
}

/// enkidu: the original kept gpui-kit's own highlighters here, by language. They are private to gpui-component, and
/// Enkidu brings its own highlighter (an IDE's colors, or Vesper's), so a file without one is shown without syntax colors.
#[derive(Default)]
pub(crate) struct SyntaxHighlighters(());

/// The application's highlighter, set with
/// [`DiffState::with_syntax_highlighter`](super::DiffState::with_syntax_highlighter).
pub(crate) type DiffSyntaxHighlighter = Arc<dyn Fn(&str, &str) -> Vec<(Range<usize>, HighlightStyle)> + Send + Sync>;

/// What a syntax span is painted with.
#[derive(Clone, Copy)]
enum SyntaxPaint {
    /// A style the application's highlighter returned.
    Style(HighlightStyle),
}

/// Prepared presentation for one file.
#[derive(Default)]
pub(crate) struct FilePresentation {
    syntax: [LineSpans<SyntaxPaint>; 2],
    inline: [LineSpans<()>; 2],
}

/// Byte ranges relative to each line's source start, with a payload.
struct LineSpans<T> {
    /// `starts[ix]..starts[ix + 1]` index the spans of line `ix`.
    starts: Vec<u32>,
    spans: Vec<(Range<u32>, T)>,
}

impl<T> Default for LineSpans<T> {
    fn default() -> Self {
        Self { starts: Vec::new(), spans: Vec::new() }
    }
}

impl<T> LineSpans<T> {
    fn from_lines(per_line: Vec<Vec<(Range<u32>, T)>>) -> Self {
        let mut spans = Self::default();
        spans.starts.reserve(per_line.len() + 1);
        spans.starts.push(0);
        for line in per_line {
            spans.spans.extend(line);
            spans.starts.push(spans.spans.len() as u32);
        }
        spans
    }

    fn line(&self, ix: usize) -> &[(Range<u32>, T)] {
        match (self.starts.get(ix), self.starts.get(ix + 1)) {
            (Some(start), Some(end)) => &self.spans[*start as usize..*end as usize],
            _ => &[],
        }
    }
}

impl FilePresentation {
    /// Blocking; run it on a background thread. `custom` replaces the
    /// built-in highlighters when the application set one.
    pub(crate) fn prepare(
        file: &DiffFile,
        options: PresentationOptions,
        _highlighters: &mut SyntaxHighlighters,
        custom: Option<&DiffSyntaxHighlighter>,
    ) -> Self {
        let mut presentation = Self::default();
        for side in [DiffSide::Original, DiffSide::Modified] {
            let per_line =
                syntax_spans(file, side, options.syntax_max_line_length, &mut |language, text| match custom {
                    // A range that splits a character, such as one counted in
                    // characters rather than bytes, cannot be painted.
                    Some(highlight) => highlight(language, text)
                        .into_iter()
                        .filter(|(range, _)| text.is_char_boundary(range.start) && text.is_char_boundary(range.end))
                        .map(|(range, style)| (range, SyntaxPaint::Style(style)))
                        .collect(),
                    // enkidu: no highlighter of its own (see `SyntaxHighlighters`)
                    None => Vec::new(),
                });
            presentation.syntax[side as usize] = LineSpans::from_lines(per_line);
        }
        if let Some(unit) = options.inline_unit {
            presentation.inline = inline_changes(file, unit, options.inline_max_line_length);
        }
        presentation
    }

    /// Theme styles for one line's syntax, as display ranges.
    pub(crate) fn syntax_highlights(
        &self,
        line: &SourceLine,
        side: DiffSide,
        ix: usize,
        _theme: &dyn HighlightStyleResolver,
    ) -> Vec<(Range<usize>, HighlightStyle)> {
        self.syntax[side as usize]
            .line(ix)
            .iter()
            .map(|(range, paint)| {
                let SyntaxPaint::Style(style) = paint;
                (line.display_range(range.start as usize..range.end as usize), *style)
            })
            .collect()
    }

    /// Changed runs within one line, as display ranges.
    pub(crate) fn inline_changes<'a>(
        &'a self,
        line: &'a SourceLine,
        side: DiffSide,
        ix: usize,
    ) -> impl Iterator<Item = Range<usize>> + 'a {
        self.inline[side as usize]
            .line(ix)
            .iter()
            .map(|(range, _)| line.display_range(range.start as usize..range.end as usize))
    }
}

/// Parses each hunk of one side independently, so an unterminated comment or
/// string in one hunk cannot affect the next.
fn syntax_spans(
    file: &DiffFile,
    side: DiffSide,
    max_line_length: usize,
    highlight: &mut dyn FnMut(&SharedString, &str) -> Vec<(Range<usize>, SyntaxPaint)>,
) -> Vec<Vec<(Range<u32>, SyntaxPaint)>> {
    let lines = file.lines(side);
    let mut per_line: Vec<Vec<(Range<u32>, SyntaxPaint)>> = (0..lines.len()).map(|_| Vec::new()).collect();
    let language = match (file.language_override(), file.side_path(side)) {
        (Some(language), _) => language.clone(),
        (None, Some(path)) => detected_language(path),
        (None, None) => return per_line,
    };
    if language.as_str() == "text" || lines.is_empty() {
        return per_line;
    }
    let ranges = if file.hunks().is_empty() {
        vec![0..lines.len()]
    } else {
        file.hunks().iter().map(|hunk| hunk.lines(side)).collect()
    };
    let source = file.source(side);
    for range in ranges.into_iter().filter(|range| !range.is_empty()) {
        let base = lines[range.start].source().start;
        let text = &source[base..lines[range.end - 1].source().end];
        let hunk_lines = &lines[range.clone()];
        for (capture, paint) in highlight(&language, text) {
            let capture = capture.start + base..capture.end + base;
            let first = hunk_lines.partition_point(|line| line.source().end <= capture.start);
            for (offset, line) in hunk_lines[first..].iter().enumerate() {
                let line_start = line.source().start;
                if line_start >= capture.end {
                    break;
                }
                if line.content_end() - line_start > max_line_length {
                    continue;
                }
                let start = capture.start.max(line_start);
                let end = capture.end.min(line.content_end());
                if start < end {
                    per_line[range.start + first + offset]
                        .push(((start - line_start) as u32..(end - line_start) as u32, paint));
                }
            }
        }
    }
    per_line
}

/// Compares each deleted line with the added line paired with it.
fn inline_changes(file: &DiffFile, unit: DiffInlineUnit, max_line_length: usize) -> [LineSpans<()>; 2] {
    let mut sides: [Vec<Vec<(Range<u32>, ())>>; 2] = [
        (0..file.lines_count(DiffSide::Original)).map(|_| Vec::new()).collect(),
        (0..file.lines_count(DiffSide::Modified)).map(|_| Vec::new()).collect(),
    ];
    for pair in file.pairs() {
        let (true, Some(old_ix), Some(new_ix)) = (pair.is_changed(), pair.original(), pair.modified()) else {
            continue;
        };
        let old = file.lines(DiffSide::Original)[old_ix].text();
        let new = file.lines(DiffSide::Modified)[new_ix].text();
        if old.len() > max_line_length || new.len() > max_line_length || old == new {
            continue;
        }
        let (old_runs, new_runs) = changed_runs(old, new, unit);
        sides[0][old_ix] = old_runs.into_iter().map(|range| (range, ())).collect();
        sides[1][new_ix] = new_runs.into_iter().map(|range| (range, ())).collect();
    }
    let [original, modified] = sides;
    [LineSpans::from_lines(original), LineSpans::from_lines(modified)]
}

/// Byte ranges that differ between two lines, on each side.
pub(crate) fn changed_runs(old: &str, new: &str, unit: DiffInlineUnit) -> (Vec<Range<u32>>, Vec<Range<u32>>) {
    let diff = match unit {
        DiffInlineUnit::Word => TextDiff::from_unicode_words(old, new),
        DiffInlineUnit::Character => TextDiff::from_graphemes(old, new),
    };
    // enkidu: a rule of our own in place of the original's ratio of 0.25. Lines that share only a word or two are a
    // rewrite, not an edit, and marking words there is noise: what the two lines share (each shared stretch without the
    // blanks around it) must be half of the shorter line and three tenths of the longer one.
    let (mut same, mut stretch) = (0usize, String::new());
    for change in diff.iter_all_changes() {
        if change.tag() == ChangeTag::Equal {
            stretch.push_str(change.value());
        } else {
            same += stretch.trim().len();
            stretch.clear();
        }
    }
    same += stretch.trim().len();
    let (shorter, longer) = (old.trim().len().min(new.trim().len()), old.trim().len().max(new.trim().len()));
    if (same as f32) < shorter as f32 * 0.5 || (same as f32) < longer as f32 * 0.3 {
        return (Vec::new(), Vec::new());
    }
    let (mut old_runs, mut new_runs) = (Vec::new(), Vec::new());
    let (mut old_offset, mut new_offset) = (0usize, 0usize);
    for change in diff.iter_all_changes() {
        let len = change.value().len();
        match change.tag() {
            ChangeTag::Equal => {
                old_offset += len;
                new_offset += len;
            }
            ChangeTag::Delete => {
                push_run(&mut old_runs, old_offset..old_offset + len, old, unit);
                old_offset += len;
            }
            ChangeTag::Insert => {
                push_run(&mut new_runs, new_offset..new_offset + len, new, unit);
                new_offset += len;
            }
        }
    }
    (old_runs, new_runs)
}

/// Extends the previous run when only whitespace separates it from `range`.
fn push_run(runs: &mut Vec<Range<u32>>, range: Range<usize>, text: &str, unit: DiffInlineUnit) {
    if let Some(last) = runs.last_mut() {
        let gap = &text[last.end as usize..range.start];
        if gap.is_empty() || unit == DiffInlineUnit::Word && gap.trim().is_empty() {
            last.end = range.end as u32;
            return;
        }
    }
    runs.push(range.start as u32..range.end as u32);
}

/// File paths may come from another platform. Only the final component
/// determines the language; dots in a directory are not extensions.
///
/// enkidu: the application's highlighter is keyed on these names; `text` is a file shown without colors.
pub(crate) fn detected_language(path: &str) -> SharedString {
    let name = path.rsplit(['/', '\\']).next().unwrap_or("");
    let lower = name.to_lowercase();
    if lower == "dockerfile" || lower.starts_with("dockerfile.") {
        return "dockerfile".into();
    }
    if name == ".env" || name.starts_with(".env.") {
        return "shellscript".into();
    }
    let language = match lower.rsplit_once('.').map(|(_, extension)| extension) {
        Some("py" | "pyi") => "python",
        Some("ts" | "mts" | "cts") => "typescript",
        Some("tsx") => "tsx",
        Some("js" | "mjs" | "cjs") => "javascript",
        Some("jsx") => "jsx",
        Some("vue") => "vue",
        Some("html" | "htm") => "html",
        Some("jinja" | "j2") => "jinja",
        Some("css") => "css",
        Some("scss") => "scss",
        Some("less") => "less",
        Some("json" | "jsonc") => "json",
        Some("yml" | "yaml") => "yaml",
        Some("toml") => "toml",
        Some("ini" | "cfg") => "ini",
        Some("md" | "mdx") => "markdown",
        Some("sh" | "bash" | "zsh") => "shellscript",
        Some("sql") => "sql",
        Some("rs") => "rust",
        Some("go") => "go",
        Some("rb") => "ruby",
        Some("java") => "java",
        Some("swift") => "swift",
        Some("php") => "php",
        Some("xml" | "svg") => "xml",
        Some("graphql" | "gql") => "graphql",
        _ => "text",
    };
    language.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPTIONS: PresentationOptions = PresentationOptions {
        inline_unit: Some(DiffInlineUnit::Word),
        inline_max_line_length: 1000,
        syntax_max_line_length: 1000,
    };

    #[test]
    fn language_detection_uses_filename() {
        for (name, language) in [
            ("src/main.RS", "rust"),
            ("src/components/App.jsx", "jsx"),
            ("C:\\src\\stock.PY", "python"),
            ("web/Stock.vue", "vue"),
            ("deploy/Dockerfile.prod", "dockerfile"),
            (".env.local", "shellscript"),
            ("styles/main.scss", "scss"),
            ("directory.rs/README", "text"),
            ("ts", "text"),
            ("notes.unknown", "text"),
        ] {
            assert_eq!(detected_language(name).as_str(), language);
        }
        let file = DiffFile::parse("--- a/notes\n+++ b/notes\n@@ -1 +1 @@\n-a\n+b\n").unwrap().remove(0);
        assert_eq!(file.language().as_str(), "text");
        assert_eq!(file.with_language("rust").language().as_str(), "rust");
    }

    #[test]
    fn inline_changes_merge_words_across_whitespace_and_skip_unrelated_lines() {
        let (old, new) =
            changed_runs("let total = price * count;", "let total = cost * quantity;", DiffInlineUnit::Word);
        assert_eq!(old, [12..17, 20..25]);
        assert_eq!(new, [12..16, 19..27]);
        let (old, new) = changed_runs("old value here", "new value here", DiffInlineUnit::Word);
        assert_eq!((old, new), (vec![0..3], vec![0..3]));
        // enkidu: two letters in common out of five are a rewrite by our rule (the original marked `x y`)
        let (old, new) = changed_runs("a b c", "a x y c", DiffInlineUnit::Word);
        assert!(old.is_empty() && new.is_empty());
        // enkidu: a line rewritten but for its indent and one word: no words marked
        let (old, new) =
            changed_runs("        item.stock += line.quantity", "        if not line.restocked:", DiffInlineUnit::Word);
        assert!(old.is_empty() && new.is_empty());
        // enkidu: one word put in: marked
        let (old, new) = changed_runs(
            "Orders and stock for the shop floor.",
            "Orders, returns and stock for the shop floor.",
            DiffInlineUnit::Word,
        );
        assert_eq!((old, new), (vec![], vec![6..15]));
        let (old, new) = changed_runs("colour", "color", DiffInlineUnit::Character);
        assert_eq!((old, new), (vec![4..5], vec![]));
        let (old, new) = changed_runs("alpha", "zzzzzzzzz", DiffInlineUnit::Word);
        assert!(old.is_empty() && new.is_empty());
    }

    #[test]
    fn inline_changes_cover_paired_lines_only() {
        let file = DiffFile::parse(
            "--- a/a.txt\n+++ b/a.txt\n@@ -1,2 +1,3 @@\n-let total = price * count;\n same\n+let total = cost * count;\n+added\n",
        )
        .unwrap()
        .remove(0);
        let presentation = FilePresentation::prepare(&file, OPTIONS, &mut SyntaxHighlighters::default(), None);
        let ranges = |side, ix| {
            let line = &file.lines(side)[ix];
            presentation.inline_changes(line, side, ix).collect::<Vec<_>>()
        };
        // The deletion and the first addition are in different change runs,
        // so they are not paired.
        assert!(ranges(DiffSide::Original, 0).is_empty());
        let file = DiffFile::parse(
            "--- a/a.txt\n+++ b/a.txt\n@@ -1 +1,2 @@\n-let total = price * count;\n+let total = cost * count;\n+added\n",
        )
        .unwrap()
        .remove(0);
        let presentation = FilePresentation::prepare(&file, OPTIONS, &mut SyntaxHighlighters::default(), None);
        let ranges = |side, ix| {
            let line = &file.lines(side)[ix];
            presentation.inline_changes(line, side, ix).collect::<Vec<_>>()
        };
        assert_eq!(ranges(DiffSide::Original, 0), [12..17]);
        assert_eq!(ranges(DiffSide::Modified, 0), [12..16]);
        assert!(ranges(DiffSide::Modified, 1).is_empty());
        let off = PresentationOptions { inline_unit: None, ..OPTIONS };
        let presentation = FilePresentation::prepare(&file, off, &mut SyntaxHighlighters::default(), None);
        assert_eq!(presentation.inline_changes(&file.lines(DiffSide::Original)[0], DiffSide::Original, 0).count(), 0);
    }

    #[test]
    fn an_application_highlighter_styles_each_hunk_side_on_its_own() {
        use std::sync::Mutex;

        struct NoTheme;
        impl HighlightStyleResolver for NoTheme {
            fn style(&self, _: &str) -> Option<HighlightStyle> {
                None
            }
        }

        let file = DiffFile::parse(
            "--- a/a.rs\n+++ b/a.rs\n@@ -1,2 +1,2 @@\n-let a = 1;\n+let a = 2;\n /* open\n@@ -10,2 +10,2 @@\n close */\n-let old = 0;\n+let new = 0;\n",
        )
        .unwrap()
        .remove(0);
        let bold = HighlightStyle { font_weight: Some(gpui::FontWeight::BOLD), ..Default::default() };
        let calls = Arc::new(Mutex::new(Vec::new()));
        let highlighter: DiffSyntaxHighlighter = Arc::new({
            let calls = calls.clone();
            move |language, text| {
                calls.lock().unwrap().push((language.to_owned(), text.to_owned()));
                text.match_indices("let").map(|(ix, keyword)| (ix..ix + keyword.len(), bold)).collect()
            }
        });
        let presentation =
            FilePresentation::prepare(&file, OPTIONS, &mut SyntaxHighlighters::default(), Some(&highlighter));

        // Each side of each hunk alone, named by the file's language.
        let calls = std::mem::take(&mut *calls.lock().unwrap());
        assert_eq!(calls.len(), 4);
        assert!(calls.iter().all(|(language, _)| language == "rust"));
        assert!(calls.iter().all(|(_, text)| !(text.contains("open") && text.contains("close"))));
        let styles = |presentation: &FilePresentation, side, ix| {
            presentation.syntax_highlights(&file.lines(side)[ix], side, ix, &NoTheme)
        };
        assert_eq!(styles(&presentation, DiffSide::Modified, 0), [(0..3, bold)]);
        assert_eq!(styles(&presentation, DiffSide::Original, 3), [(0..3, bold)]);
        assert!(styles(&presentation, DiffSide::Modified, 1).is_empty());

        let short = PresentationOptions { syntax_max_line_length: 5, ..OPTIONS };
        let presentation =
            FilePresentation::prepare(&file, short, &mut SyntaxHighlighters::default(), Some(&highlighter));
        assert!(styles(&presentation, DiffSide::Modified, 0).is_empty());

        // `é` is two bytes, so 5..6 splits it: counted in characters, not bytes.
        let file =
            DiffFile::parse("--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-let é = 1;\n+let é = 2;\n").unwrap().remove(0);
        let split: DiffSyntaxHighlighter = Arc::new(move |_, _| vec![(0..3, bold), (5..6, bold), (4..6, bold)]);
        let presentation = FilePresentation::prepare(&file, OPTIONS, &mut SyntaxHighlighters::default(), Some(&split));
        assert_eq!(
            presentation.syntax_highlights(&file.lines(DiffSide::Modified)[0], DiffSide::Modified, 0, &NoTheme),
            [(0..3, bold), (4..6, bold)]
        );
    }
}
