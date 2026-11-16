//! Remarks on lines (item 3.5a), with their tests. A remark is kept by the core until it is sent to Claude
//! (`enkidu_core::db::Remark`): the file, the side ("old" for lines the change removed, "new" otherwise), the lines,
//! the code they held when it was written, and what you said.

use std::hash::{BuildHasher as _, Hasher as _};

use enkidu_core::db::Remark;

/// At most this many lines of code are quoted under a remark in the message to Claude.
const QUOTE_LINES: usize = 6;

/// A new remark's id: the moment it was made, then sixteen random hex digits.
pub fn new_id(now_ms: i64) -> String {
    let random = std::hash::RandomState::new().build_hasher().finish();
    let mut at = String::new();
    let mut left = now_ms.max(0) as u64;
    loop {
        at.insert(0, char::from_digit((left % 36) as u32, 36).unwrap_or('0'));
        left /= 36;
        if left == 0 {
            break;
        }
    }
    format!("r{at}{random:016x}")
}

/// The remarks in the order they are sent and listed: by file (as the changed files are listed: in every folder its
/// folders first, then its files, by name), the lines a change removed first, then by line, then by when they were
/// written.
pub fn sorted(remarks: &[Remark]) -> Vec<Remark> {
    let mut sorted = remarks.to_vec();
    sorted.sort_by(|a, b| {
        enkidu_core::names::in_tree_order(&a.path, &b.path)
            .then_with(|| (a.side != "old").cmp(&(b.side != "old")))
            .then_with(|| a.start_line.cmp(&b.start_line))
            .then_with(|| a.created_at.cmp(&b.created_at))
    });
    sorted
}

/// "line 4", "lines 4–6".
pub fn line_label(start_line: i64, end_line: i64) -> String {
    if start_line == end_line { format!("line {start_line}") } else { format!("lines {start_line}\u{2013}{end_line}") }
}

/// The message Claude gets when you send your remarks: where, the code, and what you said. Remarks can be on the
/// changes or on any file opened in Enkidu, so the opening line names neither.
pub fn message(remarks: &[Remark], note: Option<&str>) -> String {
    let mut parts = vec!["Review remarks on the code in this workspace. Please address each one.".to_string()];
    for (ix, remark) in sorted(remarks).iter().enumerate() {
        let place = if remark.start_line == remark.end_line {
            format!("{}:{}", remark.path, remark.start_line)
        } else {
            format!("{}:{}-{}", remark.path, remark.start_line, remark.end_line)
        };
        let removed = if remark.side == "old" { " (lines removed by the change)" } else { "" };
        let mut lines = vec![format!("{}. {place}{removed}", ix + 1)];
        let code: Vec<&str> = remark.snippet.split('\n').collect();
        lines.extend(code.iter().take(QUOTE_LINES).map(|line| format!("   > {line}").trim_end().to_string()));
        if code.len() > QUOTE_LINES {
            lines.push("   > \u{2026}".to_string());
        }
        lines.extend(remark.body.trim().split('\n').map(|line| format!("   {line}").trim_end().to_string()));
        parts.push(lines.join("\n"));
    }
    if let Some(note) = note.map(str::trim).filter(|note| !note.is_empty()) {
        parts.push(format!("Also: {note}"));
    }
    parts.join("\n\n")
}

/// Where a remark's lines are now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Located {
    pub start_line: i64,
    pub end_line: i64,
    /// The code the remark was written on is no longer in the file.
    pub outdated: bool,
}

/// Where a remark's lines are in the file now (`lines`: the file's lines on the remark's side, none when the file
/// can't be read). Files change while Claude works, so the remark follows its code: the same place if the code is
/// still there, else the nearest place the same code stands, else it is outdated.
pub fn locate(remark: &Remark, lines: Option<&[&str]>) -> Located {
    let here = Located { start_line: remark.start_line, end_line: remark.end_line, outdated: false };
    let Some(lines) = lines else {
        return here;
    };
    let want: Vec<&str> = remark.snippet.split('\n').collect();
    let matches_at = |start: i64| {
        start >= 1
            && start as usize + want.len() - 1 <= lines.len()
            && want.iter().enumerate().all(|(ix, line)| lines[start as usize - 1 + ix] == *line)
    };
    if matches_at(remark.start_line) {
        return here;
    }
    let nearest = (1..=lines.len() as i64)
        .filter(|start| matches_at(*start))
        .min_by_key(|start| ((start - remark.start_line).abs(), *start));
    match nearest {
        Some(start) => {
            Located { start_line: start, end_line: start + (remark.end_line - remark.start_line), outdated: false }
        }
        None => Located { outdated: true, ..here },
    }
}

/// "1 remark", "3 remarks".
pub fn remarks_words(count: usize) -> String {
    format!("{count} {}", if count == 1 { "remark" } else { "remarks" })
}

/// What the bar at the foot of the changes says of the remarks that wait: "3 remarks on 2 files not sent yet" (the
/// files only when they are on more than one).
pub fn waiting_words(remarks: &[Remark]) -> String {
    let mut files: Vec<&str> = remarks.iter().map(|remark| remark.path.as_str()).collect();
    files.sort_unstable();
    files.dedup();
    let on = if files.len() > 1 { format!(" on {} files", files.len()) } else { String::new() };
    format!("{}{on} not sent yet", remarks_words(remarks.len()))
}

/// The button that types them into Claude's prompt: "Type 3 remarks into Claude".
pub fn send_words(count: usize) -> String {
    format!("Type {} into Claude", remarks_words(count))
}

/// What is said once they are there: "3 remarks in Claude's prompt".
pub fn typed_words(count: usize) -> String {
    format!("{} in Claude's prompt", remarks_words(count))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remark(change: impl FnOnce(&mut Remark)) -> Remark {
        let mut remark = Remark {
            id: "r".into(),
            workspace_id: "w1".into(),
            path: "src/a.py".into(),
            side: "new".into(),
            start_line: 1,
            end_line: 1,
            snippet: "x = 1".into(),
            body: "Rename x".into(),
            created_at: 1,
        };
        change(&mut remark);
        remark
    }

    /// Writes a message Claude can act on: where, the code, the remark.
    #[test]
    fn writes_a_message_claude_can_act_on_where_the_code_the_remark() {
        let said = message(
            &[
                remark(|r| {
                    (r.id, r.path, r.start_line, r.end_line) = ("b".into(), "src/b.py".into(), 7, 8);
                    (r.snippet, r.body) = ("for i in x:\n    total += i".into(), "Use sum()".into());
                }),
                remark(|r| {
                    (r.id, r.start_line, r.end_line) = ("a".into(), 3, 3);
                    r.body = "Rename x\nto count".into();
                }),
                remark(|r| {
                    (r.id, r.side, r.start_line, r.end_line) = ("c".into(), "old".into(), 10, 10);
                    (r.snippet, r.body) = ("cache.clear()".into(), "Why was this removed?".into());
                }),
            ],
            Some("Run the tests after."),
        );
        assert_eq!(
            said,
            [
                "Review remarks on the code in this workspace. Please address each one.",
                "1. src/a.py:10 (lines removed by the change)\n   > cache.clear()\n   Why was this removed?",
                "2. src/a.py:3\n   > x = 1\n   Rename x\n   to count",
                "3. src/b.py:7-8\n   > for i in x:\n   >     total += i\n   Use sum()",
                "Also: Run the tests after.",
            ]
            .join("\n\n")
        );
        // no note, or a blank one: nothing after the remarks
        let plain = message(&[remark(|_| {})], Some("  "));
        assert!(plain.ends_with("   Rename x") && message(&[remark(|_| {})], None) == plain, "{plain}");
    }

    /// Quotes at most six lines of code.
    #[test]
    fn quotes_at_most_six_lines_of_code() {
        let snippet = (1..=9).map(|n| format!("line {n}")).collect::<Vec<_>>().join("\n");
        let said = message(&[remark(|r| (r.start_line, r.end_line, r.snippet) = (1, 9, snippet))], None);
        assert!(said.contains("   > line 6\n   > \u{2026}"), "{said}");
        assert!(!said.contains("line 7"));
    }

    /// Follows its code when lines move, and says when the code is gone.
    #[test]
    fn follows_its_code_when_lines_move_and_says_when_the_code_is_gone() {
        let r = remark(|r| (r.start_line, r.end_line, r.snippet) = (2, 3, "b\nc".into()));
        let at = |start_line, end_line, outdated| Located { start_line, end_line, outdated };
        assert_eq!(locate(&r, Some(&["a", "b", "c", "d"])), at(2, 3, false));
        // two lines were added above it
        assert_eq!(locate(&r, Some(&["new", "new", "a", "b", "c", "d"])), at(4, 5, false));
        // the nearest copy wins when the code appears twice
        assert_eq!(locate(&r, Some(&["b", "c", "x", "x", "x", "x", "b", "c"])).start_line, 1);
        assert_eq!(locate(&r, Some(&["a", "B", "C"])), at(2, 3, true));
        // without the file there is nothing to check against
        assert!(!locate(&r, None).outdated);
        // a remark past the end of a file that got shorter
        let late = remark(|r| (r.start_line, r.end_line, r.snippet) = (9, 9, "z".into()));
        assert_eq!(locate(&late, Some(&["a", "z"])), at(2, 2, false));
        assert_eq!(locate(&late, Some(&[])), at(9, 9, true));
    }

    /// Sorts by file, removed lines first, then line.
    #[test]
    fn sorts_by_file_removed_lines_first_then_line() {
        let order = sorted(&[
            remark(|r| (r.id, r.path, r.start_line) = ("3".into(), "b.py".into(), 1)),
            remark(|r| (r.id, r.path, r.start_line) = ("2".into(), "a.py".into(), 9)),
            remark(|r| (r.id, r.path, r.start_line, r.side) = ("1".into(), "a.py".into(), 9, "old".into())),
            remark(|r| (r.id, r.path, r.start_line) = ("0".into(), "a.py".into(), 2)),
        ]);
        assert_eq!(order.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["1", "0", "2", "3"]);
        // the files stand as the changed files are listed: a folder's before the top's own (item 29.2)
        let order = sorted(&[
            remark(|r| (r.id, r.path) = ("top".into(), "a.py".into())),
            remark(|r| (r.id, r.path) = ("deep".into(), "src/z.py".into())),
        ]);
        assert_eq!(order.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["deep", "top"]);
        assert_eq!(line_label(4, 4), "line 4");
        assert_eq!(line_label(4, 6), "lines 4\u{2013}6");
        assert_eq!((remarks_words(1), remarks_words(3)), ("1 remark".to_string(), "3 remarks".to_string()));
    }

    /// Item 3.6: what the bar, its button and the toast say.
    #[test]
    fn the_bar_says_how_many_wait_and_on_how_many_files() {
        let one = [remark(|_| {})];
        assert_eq!(waiting_words(&one), "1 remark not sent yet");
        let same_file = [remark(|_| {}), remark(|r| r.start_line = 9)];
        assert_eq!(waiting_words(&same_file), "2 remarks not sent yet");
        let two_files = [remark(|_| {}), remark(|r| r.path = "src/b.py".into()), remark(|r| r.start_line = 9)];
        assert_eq!(waiting_words(&two_files), "3 remarks on 2 files not sent yet");
        assert_eq!(
            (send_words(1), send_words(3)),
            ("Type 1 remark into Claude".into(), "Type 3 remarks into Claude".into())
        );
        assert_eq!(
            (typed_words(1), typed_words(3)),
            ("1 remark in Claude's prompt".into(), "3 remarks in Claude's prompt".into())
        );
    }

    #[test]
    fn a_new_remarks_id_is_its_own() {
        let (a, b) = (new_id(1_762_966_800_000), new_id(1_762_966_800_000));
        assert!(a.starts_with('r') && a.len() > 17 && a != b, "{a} {b}");
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric()));
    }
}
