//! enkidu: item 3.12. Nine tests of what a diff must come to: read, paired side by side, the gaps between hunks,
//! word marks, and a line split by its colors and its marks. Where this crate works in a way of its own, the test
//! says how.

use std::ops::Range;
use std::sync::Arc;

use gpui::{HighlightStyle, Hsla};
use gpui_kit::base::input::HighlightStyleResolver;

use super::document::Hunk;
use super::presentation::{
    DiffSyntaxHighlighter, FilePresentation, PresentationOptions, SyntaxHighlighters, changed_runs,
};
use super::{DiffFile, DiffInlineUnit, DiffSide};

/// The diff of these tests: two hunks, with the counts in their headers as git writes them.
const DIFF: &str = "diff --git a/app.py b/app.py
index 1111111..2222222 100644
--- a/app.py
+++ b/app.py
@@ -3,3 +3,4 @@ def total(items):
 a = 1
-b = 2
+b = 3
+c = 4
 d = 5
@@ -20 +21 @@
-x
+y
\\ No newline at end of file
";

const OPTIONS: PresentationOptions = PresentationOptions {
    inline_unit: Some(DiffInlineUnit::Word),
    inline_max_line_length: 1000,
    syntax_max_line_length: 1000,
};

fn parsed() -> DiffFile {
    let mut files = DiffFile::parse(DIFF).expect("the diff of these tests is read");
    assert_eq!(files.len(), 1);
    files.remove(0)
}

/// A hunk's lines in the order the patch has them: what each is, its number before and after, and its text.
fn lines(file: &DiffFile, hunk: &Hunk) -> Vec<(&'static str, Option<usize>, Option<usize>, String)> {
    let (before, after) = (hunk.lines(DiffSide::Original), hunk.lines(DiffSide::Modified));
    file.patch_lines()
        .iter()
        .filter_map(|line| {
            let (old, new) = (line.line(DiffSide::Original), line.line(DiffSide::Modified));
            let here = old.is_some_and(|ix| before.contains(&ix)) || new.is_some_and(|ix| after.contains(&ix));
            if !here {
                return None;
            }
            let kind = match (old, new) {
                (Some(_), Some(_)) => "context",
                (Some(_), None) => "del",
                _ => "add",
            };
            let text = match new {
                Some(ix) => file.lines(DiffSide::Modified)[ix].text().to_string(),
                None => file.lines(DiffSide::Original)[old?].text().to_string(),
            };
            Some((
                kind,
                old.map(|ix| file.line_number(DiffSide::Original, ix)),
                new.map(|ix| file.line_number(DiffSide::Modified, ix)),
                text,
            ))
        })
        .collect()
}

/// Reading a patch: numbers old and new lines per hunk.
#[test]
fn numbers_old_and_new_lines_per_hunk() {
    let file = parsed();
    let hunks = file.hunks();
    assert_eq!(hunks.len(), 2);
    // a hunk keeps its whole header line, with what follows the second `@@`
    assert_eq!(hunks[0].label().as_ref(), "@@ -3,3 +3,4 @@ def total(items):");
    assert_eq!(
        lines(&file, &hunks[0]),
        [
            ("context", Some(3), Some(3), "a = 1".to_string()),
            ("del", Some(4), None, "b = 2".to_string()),
            ("add", None, Some(4), "b = 3".to_string()),
            ("add", None, Some(5), "c = 4".to_string()),
            ("context", Some(5), Some(6), "d = 5".to_string()),
        ]
    );
    assert_eq!(hunks[1].label().as_ref(), "@@ -20 +21 @@", "no function is named there");
    let second: Vec<_> = lines(&file, &hunks[1]).into_iter().map(|(kind, old, new, _)| (kind, old, new)).collect();
    // "\ No newline at end of file" isn't a line: the last line it follows is known to end without one
    assert_eq!(second, [("del", Some(20), None), ("add", None, Some(21))]);
    assert_eq!(file.source(DiffSide::Modified).chars().last(), Some('y'), "the new file's last line has no newline");
    assert!(file.source(DiffSide::Original).ends_with("x\n"), "the old file's last line has one");
}

/// Reading a patch: ignores file headers and counts changes.
#[test]
fn ignores_file_headers_and_counts_changes() {
    let file = parsed();
    for side in [DiffSide::Original, DiffSide::Modified] {
        assert!(file.lines(side).iter().all(|line| !line.text().contains("a/app.py")));
    }
    assert_eq!((file.additions(), file.deletions()), (3, 2));
    // the header says which file it is instead
    assert_eq!(file.path().as_ref(), "app.py");
}

/// Reading a patch: handles new files and empty input.
#[test]
fn handles_new_files_and_empty_input() {
    let file = DiffFile::parse("--- /dev/null\n+++ b/n.txt\n@@ -0,0 +1,2 @@\n+one\n+two\n").unwrap().remove(0);
    let numbers: Vec<usize> = file.side_lines(DiffSide::Modified).map(|(number, _)| number).collect();
    assert_eq!(numbers, [1, 2]);
    assert_eq!(file.side_lines(DiffSide::Original).count(), 0);
    assert!(DiffFile::parse("").unwrap().is_empty());
}

/// Side by side: pairs each removed line with the line that replaced it.
#[test]
fn pairs_each_removed_line_with_the_line_that_replaced_it() {
    let file = parsed();
    let text = |side: DiffSide, ix: Option<usize>| ix.map(|ix| file.lines(side)[ix].text().to_string());
    let rows: Vec<(Option<String>, Option<String>)> = file.pairs()[file.hunks()[0].pairs()]
        .iter()
        .map(|pair| (text(DiffSide::Original, pair.original()), text(DiffSide::Modified, pair.modified())))
        .collect();
    let row = |left: Option<&str>, right: Option<&str>| (left.map(String::from), right.map(String::from));
    assert_eq!(
        rows,
        [
            row(Some("a = 1"), Some("a = 1")),
            row(Some("b = 2"), Some("b = 3")),
            row(None, Some("c = 4")),
            row(Some("d = 5"), Some("d = 5")),
        ]
    );
}

/// Side by side: knows the start of each hunk and the unchanged lines between them.
#[test]
fn knows_the_start_of_each_hunk_and_the_unchanged_lines_between_them() {
    let file = parsed();
    let hunks = file.hunks();
    let starts: Vec<(usize, usize)> = hunks
        .iter()
        .map(|hunk| {
            (
                file.line_number(DiffSide::Original, hunk.lines(DiffSide::Original).start),
                file.line_number(DiffSide::Modified, hunk.lines(DiffSide::Modified).start),
            )
        })
        .collect();
    assert_eq!(starts, [(3, 3), (20, 21)]);
    // the lines the patch leaves out: 1 and 2 before the first hunk, 6 to 19 before the second
    assert_eq!(hunks[0].hidden_lines_before(None), 2);
    assert_eq!(hunks[1].hidden_lines_before(Some(&hunks[0])), 14);
    // what follows the last hunk isn't known from a patch alone. The app asks git for the whole file in one hunk, so
    // nothing is left out there
    let whole =
        DiffFile::parse("--- a/n.txt\n+++ b/n.txt\n@@ -1,3 +1,3 @@\n one\n-two\n+2\n three\n").unwrap().remove(0);
    assert_eq!(whole.hunks()[0].hidden_lines_before(None), 0);
}

/// The marked parts of a line, joined by "|".
fn marked(line: &str, runs: &[Range<u32>]) -> String {
    runs.iter().map(|run| &line[run.start as usize..run.end as usize]).collect::<Vec<_>>().join("|")
}

/// Word marks: marks the words that changed in an edited line.
#[test]
fn marks_the_words_that_changed_in_an_edited_line() {
    let (before, after) = ("level.quantity += movement.quantity", "level.quantity = F(quantity) + movement.quantity");
    let (old, new) = changed_runs(before, after, DiffInlineUnit::Word);
    assert!(!old.is_empty() || !new.is_empty(), "an edited line has marks");
    // one sign of "+=" went away, and both quantities stay unmarked: the plus is in the new line too, so it is kept,
    // and the equals sign is marked
    let (gone, came) = (marked(before, &old), marked(after, &new));
    assert_eq!(gone, "=");
    assert!(came.contains("F(quantity)"), "{came:?}");
    assert!(!came.contains("level") && !gone.contains("quantity"), "{gone:?} {came:?}");
}

/// Word marks: leaves rewritten lines unmarked: a shared word is not an edit.
#[test]
fn leaves_rewritten_lines_unmarked_a_shared_word_is_not_an_edit() {
    let rewritten = changed_runs(
        "apply_movement(movement)",
        "for movement in sorted(movements, key=lambda m: m.created_at):",
        DiffInlineUnit::Word,
    );
    assert_eq!(rewritten, (Vec::new(), Vec::new()));
    assert_eq!(changed_runs("same", "same", DiffInlineUnit::Word), (Vec::new(), Vec::new()));
}

/// Word marks: pairs marks for every removed and added line in a hunk.
#[test]
fn pairs_marks_for_every_removed_and_added_line_in_a_hunk() {
    let file = parsed();
    let presentation = FilePresentation::prepare(&file, OPTIONS, &mut SyntaxHighlighters::default(), None);
    let mut changed = Vec::new();
    for side in [DiffSide::Original, DiffSide::Modified] {
        for (ix, line) in file.lines(side).iter().enumerate() {
            if presentation.inline_changes(line, side, ix).next().is_some() {
                changed.push(line.text().to_string());
            }
        }
    }
    // the line that was replaced and the one that replaced it; "c = 4" was only added, and "x" became "y" whole
    assert_eq!(changed, ["b = 2", "b = 3"]);
}

struct NoTheme;

impl HighlightStyleResolver for NoTheme {
    fn style(&self, _: &str) -> Option<HighlightStyle> {
        None
    }
}

/// Word marks: splits a line by its colors and its marks.
#[test]
fn splits_a_line_by_its_colors_and_its_marks() {
    let color = |lightness: f32| HighlightStyle {
        color: Some(Hsla { h: 0., s: 0., l: lightness, a: 1. }),
        ..Default::default()
    };
    let (light, mid, dim) = (color(0.9), color(0.8), color(0.7));
    // the application's highlighter colors "b = 3" in three parts; for another text its ranges don't fit
    // it is given a side's text at once ("b = 3\nö\n"): the first line in three parts, then a range past the end of
    // the text and one that splits the two-byte letter of the second line, neither of which can be painted
    let highlighter: DiffSyntaxHighlighter =
        Arc::new(move |_, _| vec![(0..1, light), (1..4, mid), (4..5, dim), (0..400, light), (6..7, mid)]);
    let file = DiffFile::parse("--- a/a.py\n+++ b/a.py\n@@ -1,2 +1,2 @@\n-b = 2\n+b = 3\n-é\n+ö\n").unwrap().remove(0);
    let presentation =
        FilePresentation::prepare(&file, OPTIONS, &mut SyntaxHighlighters::default(), Some(&highlighter));
    let line = &file.lines(DiffSide::Modified)[0];
    assert_eq!(line.text().as_ref(), "b = 3");
    // its colors, part by part, and its mark on the part that changed: the view lays the mark over the color
    assert_eq!(
        presentation.syntax_highlights(line, DiffSide::Modified, 0, &NoTheme),
        [(0..1, light), (1..4, mid), (4..5, dim)]
    );
    assert_eq!(presentation.inline_changes(line, DiffSide::Modified, 0).collect::<Vec<_>>(), [4..5]);
    // colors that don't fit the text are left out rather than shown wrong
    let other = &file.lines(DiffSide::Modified)[1];
    assert_eq!(other.text().as_ref(), "ö");
    assert!(presentation.syntax_highlights(other, DiffSide::Modified, 1, &NoTheme).is_empty());
}
