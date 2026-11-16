//! The changes island's plain logic (item 3.2d): which files' diffs are read and kept, and what a file's header says.
//!
//! A file's diff is read once per version of the file (the change's signature) and kept, so going back to a
//! workspace shows it at once. When a file changes again, what was read for it stays on screen until the new diff
//! is there (DESIGN.md, Principle 9: instant first, fresh second).

use std::collections::HashMap;

use enkidu_core::git::Change;

/// What reading a file's diff gave.
#[derive(Clone, Debug, PartialEq)]
pub enum Read<T> {
    /// The parsed diff.
    Lines(T),
    Binary,
    TooLarge,
    /// Git's own words for why it couldn't be read.
    Failed(String),
}

struct Entry<T> {
    change: Change,
    /// What is shown for the file: read for `shown_sig`, which is an older version while a new one is read.
    shown: Option<Read<T>>,
    shown_sig: Option<String>,
    /// The signature a read is under way for.
    reading: Option<String>,
}

/// The changed files of one workspace and what was read for each.
pub struct FileSet<T> {
    order: Vec<String>,
    entries: HashMap<String, Entry<T>>,
}

impl<T> Default for FileSet<T> {
    fn default() -> Self {
        FileSet { order: Vec::new(), entries: HashMap::new() }
    }
}

impl<T> FileSet<T> {
    /// Git's list of changed files. Files that are gone from it are forgotten; a file whose signature moved keeps
    /// what was read for it until it is read again. True when the files or their order changed.
    pub fn follow(&mut self, changes: &[Change]) -> bool {
        let order: Vec<String> = changes.iter().map(|c| c.path.clone()).collect();
        let moved = order != self.order;
        self.entries.retain(|path, _| changes.iter().any(|c| &c.path == path));
        for change in changes {
            match self.entries.get_mut(&change.path) {
                Some(entry) => entry.change = change.clone(),
                None => {
                    let entry = Entry { change: change.clone(), shown: None, shown_sig: None, reading: None };
                    self.entries.insert(change.path.clone(), entry);
                }
            }
        }
        self.order = order;
        moved
    }

    /// The files to read now, at most `room` of them: those whose current version was neither read nor is being
    /// read, `first` (the file on screen) before the others, then in the list's order. They count as being read
    /// from here on.
    pub fn take_to_read(&mut self, room: usize, first: Option<&str>) -> Vec<Change> {
        let wanted = |entry: &Entry<T>| {
            entry.shown_sig.as_deref() != Some(entry.change.sig.as_str())
                && entry.reading.as_deref() != Some(entry.change.sig.as_str())
        };
        let first = first.filter(|path| self.entries.get(*path).is_some_and(wanted));
        let paths: Vec<String> = first
            .into_iter()
            .map(str::to_owned)
            .chain(self.order.iter().filter(|path| Some(path.as_str()) != first).cloned())
            .filter(|path| self.entries.get(path).is_some_and(wanted))
            .take(room)
            .collect();
        paths
            .into_iter()
            .filter_map(|path| {
                let entry = self.entries.get_mut(&path)?;
                entry.reading = Some(entry.change.sig.clone());
                Some(entry.change.clone())
            })
            .collect()
    }

    /// Whether the file at `path` still has to be read as it is now (it is neither read nor being read).
    pub fn wants_read(&self, path: &str) -> bool {
        self.entries.get(path).is_some_and(|entry| {
            entry.shown_sig.as_deref() != Some(entry.change.sig.as_str())
                && entry.reading.as_deref() != Some(entry.change.sig.as_str())
        })
    }

    /// How many reads are under way.
    pub fn reading(&self) -> usize {
        self.entries.values().filter(|entry| entry.reading.is_some()).count()
    }

    /// A read came back. False when it is for a file that is gone or for a version that is no longer the newest one
    /// asked for (then nothing changes: the newer read is under way or will be taken next).
    pub fn read(&mut self, path: &str, sig: &str, read: Read<T>) -> bool {
        let Some(entry) = self.entries.get_mut(path) else {
            return false;
        };
        if entry.reading.as_deref() == Some(sig) {
            entry.reading = None;
        }
        if entry.change.sig != sig {
            return false;
        }
        entry.shown = Some(read);
        entry.shown_sig = Some(sig.to_string());
        true
    }

    /// The files in the list's order, each with what there is to show for it (nothing while its first read is under
    /// way).
    pub fn files(&self) -> impl Iterator<Item = (&Change, Option<&Read<T>>)> {
        self.order.iter().filter_map(|path| self.entries.get(path)).map(|entry| (&entry.change, entry.shown.as_ref()))
    }

    /// The change of the file at `path`.
    pub fn change(&self, path: &str) -> Option<&Change> {
        self.entries.get(path).map(|entry| &entry.change)
    }

    /// What was read for the file at `path`, and the version it was read for (its change's signature then).
    pub fn read_of(&self, path: &str) -> Option<(&str, &Read<T>)> {
        let entry = self.entries.get(path)?;
        Some((entry.shown_sig.as_deref()?, entry.shown.as_ref()?))
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }
}

/// Whether an opened file of this kind of change carries marks of what differs from the last commit (item 10.3).
/// A file that is all new has none: every line would be marked, and its badge says it already.
pub fn marks_its_changes(kind: &str) -> bool {
    !matches!(kind, "untracked" | "added" | "deleted")
}

/// A path as a header shows it: the folder, then the file's name.
pub fn split_path(path: &str) -> (&str, &str) {
    match path.rfind('/') {
        Some(at) => (&path[..at], &path[at + 1..]),
        None => ("", path),
    }
}

/// What happened to a file, in a word (a change's `kind`, as the core names it).
pub fn kind_label(kind: &str) -> &'static str {
    match kind {
        "added" => "Added",
        "deleted" => "Deleted",
        "renamed" => "Renamed",
        "untracked" => "New file",
        "conflicted" => "Conflict",
        _ => "Modified",
    }
}

/// The letter that says what happened to a file, as git writes it.
pub fn kind_letter(kind: &str) -> &'static str {
    match kind {
        "added" => "A",
        "deleted" => "D",
        "renamed" => "R",
        "untracked" => "U",
        "conflicted" => "!",
        _ => "M",
    }
}

/// The letter's color as (hue, chroma) at the file icons' lightness: modified tan, new green, deleted red, renamed
/// blue (DESIGN.md, "Icons").
pub fn kind_hue(kind: &str) -> (f32, f32) {
    match kind {
        "added" | "untracked" => (155., 0.14),
        "deleted" => (22., 0.15),
        "renamed" => (255., 0.13),
        "conflicted" => (22., 0.17),
        _ => (80., 0.1),
    }
}

/// "1 file", "5 files".
pub fn files_words(count: usize) -> String {
    format!("{count} {}", if count == 1 { "file" } else { "files" })
}

/// What reading a file to open it gave (item 3.7).
#[derive(Clone, Debug, PartialEq)]
pub enum Opened<T> {
    /// Its lines, and how many.
    Text(T, usize),
    Empty,
    /// It was deleted, or never was.
    Missing,
    Binary {
        size: u64,
    },
    TooLarge {
        size: u64,
    },
    /// The core's own words for why it couldn't be read.
    Failed(String),
}

impl<T> Opened<T> {
    /// What stands in place of the lines when there are none to show.
    pub fn note(&self) -> Option<String> {
        match self {
            Opened::Text(..) => None,
            Opened::Empty => Some("This file is empty.".into()),
            Opened::Missing => Some("This file isn't there any more.".into()),
            Opened::Binary { size } => Some(format!("A binary file ({}), not shown.", size_words(*size))),
            Opened::TooLarge { size } => Some(format!("Too large to show here ({}).", size_words(*size))),
            Opened::Failed(why) => Some(why.clone()),
        }
    }

    /// How many lines it has, when it has lines to show.
    pub fn lines(&self) -> Option<usize> {
        match self {
            Opened::Text(_, lines) => Some(*lines),
            _ => None,
        }
    }
}

/// A file's size as a person says it: "512 bytes", "12 KB", "1.2 MB".
pub fn size_words(bytes: u64) -> String {
    const KB: f64 = 1024.;
    if bytes < 1024 {
        format!("{bytes} bytes")
    } else if bytes < 1024 * 1024 {
        format!("{} KB", (bytes as f64 / KB).round())
    } else {
        format!("{:.1} MB", (bytes as f64 / KB / KB * 10.).round() / 10.)
    }
}

/// "1 line", "1,234 lines".
pub fn lines_words(count: usize) -> String {
    format!("{} {}", crate::text::thousands(count), if count == 1 { "line" } else { "lines" })
}

/// What the button that leaves an opened file says: back to the changes when there is something to review, else the
/// file closes and Claude has the room again.
pub fn close_words(has_review: bool) -> &'static str {
    if has_review { "Back to the changes" } else { "Close the file" }
}

/// What the same button says after a ⌘-click led to the file (item 10.11): back to where the click was made, a
/// file by its name or "the changes".
pub fn back_words(to: &str) -> String {
    format!("Back to {to}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(path: &str, sig: &str) -> Change {
        Change {
            path: path.into(),
            old_path: None,
            kind: "modified".into(),
            staged: false,
            additions: Some(1),
            deletions: Some(1),
            sig: sig.into(),
        }
    }

    fn paths(changes: &[Change]) -> Vec<&str> {
        changes.iter().map(|c| c.path.as_str()).collect()
    }

    fn shown(set: &FileSet<&'static str>) -> Vec<(String, Option<Read<&'static str>>)> {
        set.files().map(|(change, read)| (change.path.clone(), read.cloned())).collect()
    }

    #[test]
    fn files_are_read_a_few_at_a_time_the_one_on_screen_first() {
        let mut set = FileSet::<&str>::default();
        let list = ["a.py", "b.py", "c.py", "d.py", "e.py"].map(|path| change(path, "1"));
        assert!(set.follow(&list));
        assert!(!set.follow(&list), "the same list again");
        // three at a time; you are looking at d.py
        assert_eq!(paths(&set.take_to_read(3, Some("d.py"))), ["d.py", "a.py", "b.py"]);
        assert_eq!(set.reading(), 3);
        assert_eq!(paths(&set.take_to_read(0, Some("d.py"))), Vec::<&str>::new());
        // one comes back: there is room for one more
        assert!(set.read("a.py", "1", Read::Lines("a's diff")));
        assert_eq!(paths(&set.take_to_read(3 - set.reading(), Some("d.py"))), ["c.py"]);
        assert!(set.read("d.py", "1", Read::Binary));
        assert!(set.read("b.py", "1", Read::Failed("fatal: bad object".into())));
        assert!(set.read("c.py", "1", Read::TooLarge));
        assert_eq!(paths(&set.take_to_read(3, None)), ["e.py"]);
        assert!(set.read("e.py", "1", Read::Lines("e's diff")));
        // everything was read: nothing more to take, whatever is on screen
        assert_eq!(paths(&set.take_to_read(3, Some("b.py"))), Vec::<&str>::new());
        assert_eq!(set.reading(), 0);
        assert_eq!(shown(&set)[0], ("a.py".to_string(), Some(Read::Lines("a's diff"))));
        assert_eq!(shown(&set)[3], ("d.py".to_string(), Some(Read::Binary)));
    }

    #[test]
    fn a_file_that_changes_again_keeps_what_was_read_until_the_new_diff_is_there() {
        let mut set = FileSet::<&str>::default();
        set.follow(&[change("a.py", "1"), change("b.py", "1")]);
        set.take_to_read(3, None);
        set.read("a.py", "1", Read::Lines("a v1"));
        set.read("b.py", "1", Read::Lines("b v1"));
        // Claude edits a.py: its order and b.py are as they were
        assert!(!set.follow(&[change("a.py", "2"), change("b.py", "1")]));
        assert_eq!(shown(&set)[0].1, Some(Read::Lines("a v1")), "the old diff stays on screen");
        assert_eq!(paths(&set.take_to_read(3, None)), ["a.py"]);
        // and again before that read is back: the read that comes back is already old
        set.follow(&[change("a.py", "3"), change("b.py", "1")]);
        assert_eq!(paths(&set.take_to_read(3, None)), ["a.py"], "the newest version is read too");
        assert!(!set.read("a.py", "2", Read::Lines("a v2")), "an answer for a version that is no longer wanted");
        assert_eq!(shown(&set)[0].1, Some(Read::Lines("a v1")));
        assert!(set.read("a.py", "3", Read::Lines("a v3")));
        assert_eq!(shown(&set)[0].1, Some(Read::Lines("a v3")));
        assert_eq!(set.reading(), 0);
        assert_eq!(paths(&set.take_to_read(3, None)), Vec::<&str>::new());
    }

    #[test]
    fn files_that_are_gone_are_forgotten_and_new_ones_take_their_place_in_gits_order() {
        let mut set = FileSet::<&str>::default();
        set.follow(&[change("a.py", "1"), change("c.py", "1")]);
        set.take_to_read(3, None);
        set.read("a.py", "1", Read::Lines("a"));
        // c.py is committed away while its diff is being read, and b.py turns up
        assert!(set.follow(&[change("a.py", "1"), change("b.py", "1")]));
        assert!(!set.read("c.py", "1", Read::Lines("c")), "an answer for a file that is gone");
        assert_eq!(shown(&set), [("a.py".to_string(), Some(Read::Lines("a"))), ("b.py".to_string(), None)]);
        assert_eq!((set.len(), set.reading()), (2, 0));
        assert_eq!(paths(&set.take_to_read(3, None)), ["b.py"]);
        // a commit empties the list
        assert!(set.follow(&[]));
        assert!(set.is_empty() && shown(&set).is_empty());
    }

    #[test]
    fn a_header_says_the_name_first_then_the_folder_and_what_happened() {
        assert_eq!(split_path("src/orders/stock.py"), ("src/orders", "stock.py"));
        assert_eq!(split_path("README.md"), ("", "README.md"));
        let kinds = ["modified", "added", "deleted", "renamed", "untracked", "conflicted"];
        assert_eq!(kinds.map(kind_label), ["Modified", "Added", "Deleted", "Renamed", "New file", "Conflict"]);
        assert_eq!(kinds.map(kind_letter), ["M", "A", "D", "R", "U", "!"]);
        // DESIGN.md, "Icons": "M tan, A and U green, D red, R blue"
        assert_eq!(
            kinds.map(kind_hue),
            [(80., 0.1), (155., 0.14), (22., 0.15), (255., 0.13), (155., 0.14), (22., 0.17)]
        );
        assert_eq!((files_words(1), files_words(5)), ("1 file".to_string(), "5 files".to_string()));
    }

    /// Item 3.7: what an opened file says of itself.
    #[test]
    fn an_opened_file_says_its_lines_or_why_it_has_none() {
        assert_eq!(Opened::Text((), 3).note(), None);
        assert_eq!(Opened::Text((), 3).lines(), Some(3));
        assert_eq!(Opened::<()>::Empty.note().as_deref(), Some("This file is empty."));
        assert_eq!(Opened::<()>::Missing.note().as_deref(), Some("This file isn't there any more."));
        assert_eq!(Opened::<()>::Binary { size: 12_288 }.note().as_deref(), Some("A binary file (12 KB), not shown."));
        assert_eq!(
            Opened::<()>::TooLarge { size: 2_621_440 }.note().as_deref(),
            Some("Too large to show here (2.5 MB).")
        );
        assert_eq!(Opened::<()>::Failed("Permission denied".into()).note().as_deref(), Some("Permission denied"));
        assert_eq!(Opened::<()>::Missing.lines(), None);
        // sizes, rounded
        assert_eq!(size_words(0), "0 bytes");
        assert_eq!(size_words(1023), "1023 bytes");
        assert_eq!(size_words(1024), "1 KB");
        assert_eq!(size_words(2560), "3 KB");
        assert_eq!(size_words(1_048_575), "1024 KB");
        assert_eq!(size_words(1_048_576), "1.0 MB");
        assert_eq!(size_words(1_310_720), "1.3 MB");
        // lines, with the thousands apart
        assert_eq!(lines_words(1), "1 line");
        assert_eq!(lines_words(0), "0 lines");
        assert_eq!(lines_words(999), "999 lines");
        assert_eq!(lines_words(1234), "1,234 lines");
        assert_eq!(lines_words(1_234_567), "1,234,567 lines");
        assert_eq!((close_words(true), close_words(false)), ("Back to the changes", "Close the file"));
    }
}
