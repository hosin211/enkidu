//! enkidu: the view's words. The original read them from gpui-component's translations (`t!("Diff.…")`). These are
//! Enkidu's own, in English, under the same keys, so the copied code still calls `t!`.

use std::borrow::Cow;
use std::fmt::Display;

/// The words for `key`, with each `%{name}` replaced by its value. A count of one gets the singular.
pub(crate) fn say(key: &str, args: &[(&str, &dyn Display)]) -> Cow<'static, str> {
    let one = args.iter().any(|(name, value)| *name == "count" && value.to_string() == "1");
    let words = match (key, one) {
        ("Diff.BinaryChanges", _) => "Binary file, not shown.",
        ("Diff.NoTextChanges", _) => "No line changes: only the file's name or mode changed.",
        ("Diff.NoChanges", _) => "No changes",
        ("Diff.EmptyFile", _) => "Empty file",
        ("Diff.AddedFile", _) => "New file",
        ("Diff.DeletedFile", _) => "Deleted",
        ("Diff.RenamedFile", _) => "Renamed",
        ("Diff.CopiedFile", _) => "Copied",
        ("Diff.ConflictedFile", _) => "Conflict",
        ("Diff.HiddenLines", true) => "1 line not in the diff",
        ("Diff.HiddenLines", false) => "%{count} lines not in the diff",
        ("Diff.ExpandAbove", true) => "Show 1 line above",
        ("Diff.ExpandAbove", false) => "Show %{count} lines above",
        ("Diff.ExpandBelow", true) => "Show 1 line below",
        ("Diff.ExpandBelow", false) => "Show %{count} lines below",
        ("Diff.CollapseFile", _) => "Hide %{name}",
        ("Diff.ExpandFile", _) => "Show %{name}",
        ("Diff.AddAnnotation", _) => "Remark on line %{line}",
        ("Diff.CurrentChange", _) => "Current change",
        ("Diff.IncomingChange", _) => "Incoming change",
        ("Diff.BaseChange", _) => "Common ancestor",
        ("Diff.AcceptCurrent", _) => "Accept current",
        ("Diff.AcceptIncoming", _) => "Accept incoming",
        ("Diff.AcceptBoth", _) => "Accept both",
        ("Diff.AcceptedCurrent", _) => "Accepted the current change",
        ("Diff.AcceptedIncoming", _) => "Accepted the incoming change",
        ("Diff.AcceptedBoth", _) => "Accepted both changes",
        ("Diff.UndoResolution", _) => "Undo",
        ("Diff.Before", _) => "Before",
        ("Diff.BeforeNew", _) => "Before: a new file",
        ("Diff.After", _) => "After",
        ("Diff.AfterDeleted", _) => "After: deleted",
        ("Diff.Original", _) => "before",
        ("Diff.Modified", _) => "after",
        ("Diff.ExpandLines", true) | ("Diff.UnchangedLines", true) => "Show 1 unchanged line",
        ("Diff.ExpandLines", false) | ("Diff.UnchangedLines", false) => "Show %{count} unchanged lines",
        ("Diff.SourceLine", _) | ("Diff.SelectLine", _) => "Line %{line}, %{side}",
        // enkidu: in a file shown whole, which has one side
        ("Diff.LineOfFile", _) => "Line %{line}",
        // enkidu: after a line the application marked (the line a file was opened at, what a search found on it)
        ("Diff.Marked", _) => ", marked",
        ("Diff.MarkedWords", _) => ", marked: %{words}",
        // enkidu: the marks of what differs from the last commit, beside a line of a file shown whole
        ("Diff.MarkAdded", _) => "Line %{line} is new",
        ("Diff.MarkChanged", _) => "Line %{line} is changed",
        ("Diff.MarkRemoved", _) => "Lines were removed after line %{line}",
        ("Diff.MarkRemovedAbove", _) => "Lines were removed above line 1",
        ("Diff.AddedLine", _) => "Line %{line}, %{side}, added",
        ("Diff.RemovedLine", _) => "Line %{line}, %{side}, removed",
        ("Diff.NoNewline", _) => "No newline at end of file",
        ("Diff.Viewer", _) => "Changes in %{name}",
        ("Diff.ViewerFiles", true) => "Changes in 1 file",
        ("Diff.ViewerFiles", false) => "Changes in %{count} files",
        // a key the copied code never asks for
        _ => "",
    };
    if args.is_empty() || !words.contains("%{") {
        return Cow::Borrowed(words);
    }
    let mut text = words.to_string();
    for (name, value) in args {
        text = text.replace(&format!("%{{{name}}}"), &value.to_string());
    }
    Cow::Owned(text)
}

/// The original's `t!`, with its call shapes: a key, then `name = value` pairs.
macro_rules! t {
    ($key:expr $(, $name:ident = $value:expr)* $(,)?) => {
        $crate::words::say($key, &[$((stringify!($name), &$value as &dyn ::std::fmt::Display)),*])
    };
}
pub(crate) use t;

#[cfg(test)]
mod tests {
    #[test]
    fn the_words_take_their_values_and_know_one_from_many() {
        assert_eq!(t!("Diff.UnchangedLines", count = 12), "Show 12 unchanged lines");
        assert_eq!(t!("Diff.UnchangedLines", count = 1), "Show 1 unchanged line");
        assert_eq!(t!("Diff.SelectLine", side = t!("Diff.Modified"), line = 7), "Line 7, after");
        assert_eq!(t!("Diff.CollapseFile", name = "src/stock.py"), "Hide src/stock.py");
        assert_eq!(t!("Diff.NoNewline"), "No newline at end of file");
        // every key the copied code asks for has words
        let code = [include_str!("lib.rs"), include_str!("state.rs"), include_str!("selection.rs")].concat();
        let mut keys = 0;
        for part in code.split("\"Diff.").skip(1) {
            let key = format!("Diff.{}", part.split('"').next().unwrap());
            assert_ne!(super::say(&key, &[]), "", "{key}");
            keys += 1;
        }
        assert!(keys >= 36, "{keys}");
    }
}
