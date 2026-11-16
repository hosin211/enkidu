//! The Changed files panel's plain logic (items 29.3 and 29.10): the changed files as a tree of the folders they are
//! in. Each folder says how many changed files it holds, a folder that only holds one folder shares its row
//! ("src/orders"), a closed folder hides what is under it, and a filter keeps the files of one kind.
//!
//! The list comes in the order such a tree lists it (`ChangeLists` keeps it so), so the rows are made in one walk down
//! the list: a folder's row stands before the first file under it, and the files keep their places in the list.

use std::collections::{HashMap, HashSet};

use enkidu_core::git::Change;

/// One row of the tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    Folder {
        /// "src", or a chain of single folders: "src/orders".
        label: String,
        /// The folder the row closes and opens.
        path: String,
        depth: usize,
        open: bool,
        /// How many of the listed files it holds, open or closed.
        files: usize,
    },
    /// A changed file: its place in the list.
    File { change: usize, depth: usize },
}

/// Which folders of the tree are closed. Every folder is open (after "Collapse all": closed) except the ones clicked
/// since, so a folder that turns up later stands as the others do.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Folds {
    /// "Collapse all" was the last word for every folder.
    all_closed: bool,
    /// The folders clicked since: each stands the other way round.
    turned: HashSet<String>,
}

impl Folds {
    pub fn closed(&self, folder: &str) -> bool {
        self.all_closed != self.turned.contains(folder)
    }

    /// A folder closes, or opens again.
    pub fn turn(&mut self, folder: &str) {
        if !self.turned.remove(folder) {
            self.turned.insert(folder.to_string());
        }
    }

    /// "Collapse all": every folder closes, the ones inside closed folders too.
    pub fn close_all(&mut self) {
        *self = Folds { all_closed: true, turned: HashSet::new() };
    }

    /// "Expand all": every folder opens.
    pub fn open_all(&mut self) {
        *self = Folds::default();
    }
}

/// Whether `path` is somewhere under `folder`.
pub fn holds(folder: &str, path: &str) -> bool {
    path.as_bytes().get(..folder.len()) == Some(folder.as_bytes()) && path.as_bytes().get(folder.len()) == Some(&b'/')
}

/// The kind of a file, as a filter names it: what stands after the last dot of its name, in small letters ("py" for
/// "src/App.PY"). A name without a dot has none.
pub fn kind_of(path: &str) -> Option<String> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let (_, kind) = name.rsplit_once('.')?;
    (!kind.is_empty()).then(|| kind.to_lowercase())
}

/// The kinds of file among the changes, each with how many files have it: the most common first, then A to Z.
pub fn kinds(changes: &[Change]) -> Vec<(String, usize)> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for change in changes {
        if let Some(kind) = kind_of(&change.path) {
            *counts.entry(kind).or_default() += 1;
        }
    }
    let mut kinds: Vec<(String, usize)> = counts.into_iter().collect();
    kinds.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    kinds
}

/// The tree's rows, top to bottom: `changes` under their folders, each folder closed or open as `folds` says. With a
/// `kind`, only the files of that kind and the folders that hold any.
pub fn rows(changes: &[Change], folds: &Folds, kind: Option<&str>) -> Vec<Row> {
    // the files that are listed, by their places in the list
    let listed: Vec<usize> = (0..changes.len())
        .filter(|ix| kind.is_none_or(|kind| kind_of(&changes[*ix].path).as_deref() == Some(kind)))
        .collect();
    let mut rows = Vec::with_capacity(listed.len() + listed.len() / 4);
    walk(changes, &listed, 0, 0, folds, &mut rows);
    rows
}

/// The rows of one folder: `listed` are the files under it, and `inside` is where their paths go on after it.
fn walk(changes: &[Change], listed: &[usize], inside: usize, depth: usize, folds: &Folds, rows: &mut Vec<Row>) {
    let mut at = 0;
    while at < listed.len() {
        let path = changes[listed[at]].path.as_str();
        let Some(cut) = path[inside..].find('/') else {
            rows.push(Row::File { change: listed[at], depth });
            at += 1;
            continue;
        };
        // a folder: every file that follows and is under it
        let mut folder = inside + cut;
        let under = |from: usize, folder: usize| {
            from + listed[from..].iter().take_while(|ix| holds(&path[..folder], &changes[**ix].path)).count()
        };
        let end = under(at, folder);
        // a folder that only holds one folder shares its row with it
        while let Some(cut) = path[folder + 1..].find('/') {
            let next = folder + 1 + cut;
            if under(at, next) != end {
                break;
            }
            folder = next;
        }
        let open = !folds.closed(&path[..folder]);
        rows.push(Row::Folder {
            label: path[inside..folder].to_string(),
            path: path[..folder].to_string(),
            depth,
            open,
            files: end - at,
        });
        if open {
            walk(changes, &listed[at..end], folder + 1, depth + 1, folds, rows);
        }
        at = end;
    }
}

#[cfg(test)]
mod tests {
    use enkidu_core::names::in_tree_order;

    use super::*;
    use crate::filetree::{self, Tree};

    fn changes(paths: &[&str]) -> Vec<Change> {
        let mut list: Vec<Change> = paths
            .iter()
            .map(|path| Change {
                path: path.to_string(),
                old_path: None,
                kind: "modified".into(),
                staged: false,
                additions: Some(1),
                deletions: Some(0),
                sig: String::new(),
            })
            .collect();
        list.sort_by(|a, b| in_tree_order(&a.path, &b.path));
        list
    }

    /// A row in a few words: "2 ▾ orders (2)" for a folder two steps in, open, with two files; "3 stock.py" for a
    /// file.
    fn said(changes: &[Change], rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|row| match row {
                Row::Folder { label, depth, open, files, .. } => {
                    format!("{depth} {} {label} ({files})", if *open { "▾" } else { "▸" })
                }
                Row::File { change, depth } => {
                    format!("{depth} {}", changes[*change].path.rsplit('/').next().unwrap_or_default())
                }
            })
            .collect()
    }

    const SHOP: [&str; 8] = [
        "notes.md",
        "docs/guide/intro.md",
        "src/app.py",
        "src/orders/stock.py",
        "src/orders/tests/test_stock.py",
        "src/orders/tests/test_totals.py",
        "src/billing/v2/api/views.py",
        "src/billing/v2/api/urls.py",
    ];

    #[test]
    fn the_changed_files_stand_under_their_folders_each_folder_with_how_many_it_holds() {
        let list = changes(&SHOP);
        let rows = rows(&list, &Folds::default(), None);
        assert_eq!(
            said(&list, &rows),
            [
                // a folder that only holds one folder shares its row with it
                "0 ▾ docs/guide (1)",
                "1 intro.md",
                "0 ▾ src (6)",
                "1 ▾ billing/v2/api (2)",
                "2 urls.py",
                "2 views.py",
                // a folder with a file of its own beside its folder stays on its own row
                "1 ▾ orders (3)",
                "2 ▾ tests (2)",
                "3 test_stock.py",
                "3 test_totals.py",
                "2 stock.py",
                "1 app.py",
                "0 notes.md",
            ]
        );
        // the files stand in the list's own order, each once
        let files: Vec<usize> = rows
            .iter()
            .filter_map(|row| if let Row::File { change, .. } = row { Some(*change) } else { None })
            .collect();
        assert_eq!(files, (0..list.len()).collect::<Vec<_>>());
        // a folder's row names the folder it closes
        assert!(rows.iter().any(|row| matches!(row, Row::Folder { path, .. } if path == "src/billing/v2/api")));
    }

    #[test]
    fn a_closed_folder_keeps_its_row_and_its_count_and_hides_what_is_under_it() {
        let list = changes(&SHOP);
        let mut folds = Folds::default();
        folds.turn("src/orders");
        folds.turn("docs/guide");
        assert_eq!(
            said(&list, &rows(&list, &folds, None)),
            [
                "0 ▸ docs/guide (1)",
                "0 ▾ src (6)",
                "1 ▾ billing/v2/api (2)",
                "2 urls.py",
                "2 views.py",
                "1 ▸ orders (3)",
                "1 app.py",
                "0 notes.md",
            ]
        );
        // a folder closed above another closed one: one row
        folds.turn("docs/guide");
        folds.turn("src");
        assert_eq!(
            said(&list, &rows(&list, &folds, None)),
            ["0 ▾ docs/guide (1)", "1 intro.md", "0 ▸ src (6)", "0 notes.md"]
        );
        // clicked again, it is open as it was
        folds.turn("src");
        folds.turn("src/orders");
        assert_eq!(rows(&list, &folds, None), rows(&list, &Folds::default(), None));
        // nothing changed: no rows
        assert!(rows(&[], &Folds::default(), None).is_empty());
    }

    /// Item 29.10: "Collapse all" closes every folder, the ones inside closed folders too; "Expand all" opens them.
    #[test]
    fn every_folder_closes_at_once_and_opens_at_once() {
        let list = changes(&SHOP);
        let mut folds = Folds::default();
        folds.turn("src/orders");
        folds.close_all();
        assert_eq!(said(&list, &rows(&list, &folds, None)), ["0 ▸ docs/guide (1)", "0 ▸ src (6)", "0 notes.md"]);
        // one folder opened after that: the folders in it are still closed
        folds.turn("src");
        assert_eq!(
            said(&list, &rows(&list, &folds, None)),
            ["0 ▸ docs/guide (1)", "0 ▾ src (6)", "1 ▸ billing/v2/api (2)", "1 ▸ orders (3)", "1 app.py", "0 notes.md"]
        );
        // a folder that turns up now is closed like the others
        let more: Vec<&str> = SHOP.iter().copied().chain(["tools/lint.sh"]).collect();
        let longer = changes(&more);
        assert!(said(&longer, &rows(&longer, &folds, None)).contains(&"0 ▸ tools (1)".to_string()));
        // "Expand all": every folder open, whatever was clicked before
        folds.open_all();
        assert_eq!(folds, Folds::default());
        assert!(rows(&list, &folds, None).iter().all(|row| !matches!(row, Row::Folder { open: false, .. })));
    }

    /// Item 29.10: the filter keeps the files of one kind, with the folders that hold any and their counts.
    #[test]
    fn a_kind_of_file_keeps_its_files_and_the_folders_that_hold_them() {
        let more: Vec<&str> = SHOP.iter().copied().chain(["src/orders/README.MD", "Makefile", ".gitignore"]).collect();
        let list = changes(&more);
        // the kinds there are, the most common first, then A to Z; a name without a dot has none
        assert_eq!(kinds(&list), [("py".to_string(), 6), ("md".to_string(), 3), ("gitignore".to_string(), 1)]);
        assert_eq!(kind_of("src/App.PY").as_deref(), Some("py"));
        assert_eq!(kind_of("a.b/Makefile"), None, "a dot in a folder's name is not the file's");
        assert_eq!(kind_of("notes."), None);
        // one kind: its files, each folder with how many of them it holds, and chains as that tree has them
        assert_eq!(
            said(&list, &rows(&list, &Folds::default(), Some("md"))),
            ["0 ▾ docs/guide (1)", "1 intro.md", "0 ▾ src/orders (1)", "1 README.MD", "0 notes.md"]
        );
        // the files keep their places in the whole list
        let listed = rows(&list, &Folds::default(), Some("md"));
        let at: Vec<&str> = listed
            .iter()
            .filter_map(|row| if let Row::File { change, .. } = row { Some(list[*change].path.as_str()) } else { None })
            .collect();
        assert_eq!(at, ["docs/guide/intro.md", "src/orders/README.MD", "notes.md"]);
        // a kind nothing has: no rows
        assert!(rows(&list, &Folds::default(), Some("rs")).is_empty());
        // the same kinds Find in files would offer for these paths
        let paths: Vec<String> = list.iter().map(|change| change.path.clone()).collect();
        assert_eq!(kinds(&list), filetree::Files::new(paths).kinds(usize::MAX));
    }

    #[test]
    fn a_folder_holds_what_is_under_it_and_nothing_that_only_starts_like_it() {
        assert!(holds("src", "src/app.py"));
        assert!(holds("src/orders", "src/orders/tests/test_stock.py"));
        assert!(!holds("src", "src"));
        assert!(!holds("src", "src2/app.py"));
        assert!(!holds("src/orders", "src/app.py"));
        // a name in another script, cut anywhere: never a panic
        assert!(holds("目录", "目录/文件.py"));
        assert!(!holds("目", "目录/文件.py"));
        assert!(!holds("src/orders/tests/deeper", "src/é"));
    }

    /// The same tree the Files panel would make of these paths (`filetree`): its folders, its chains, its order.
    fn as_the_files_panel(tree: &Tree, dir: &str, depth: usize, said: &mut Vec<String>) {
        let Some(node) = tree.get(dir) else { return };
        for name in &node.dirs {
            let (label, path) = filetree::compact_folder(tree, dir, name);
            said.push(format!("{depth} {label}"));
            as_the_files_panel(tree, &path, depth + 1, said);
        }
        for name in &node.files {
            said.push(format!("{depth} {name}"));
        }
    }

    #[test]
    fn the_tree_is_the_one_the_files_panel_makes_of_the_same_paths() {
        // 600 files in folders of every shape: one file, many, chains, a file beside its folder's folders
        let mut paths = Vec::new();
        for n in 0..600usize {
            let top = ["src", "docs", "tests", "Apps", "x"][n % 5];
            let path = match n % 7 {
                0 => format!("{top}/file{n}.py"),
                1 => format!("{top}/a/b/c/file{n}.py"),
                2 => format!("{top}/a/b/file{n}.py"),
                3 => format!("{top}/only{}/deep/er/file{n}.py", n % 11),
                4 => format!("file{n}.md"),
                5 => format!("{top}/Zeta{}/file{n}.py", n % 3),
                _ => format!("{top}/a/File{n}.PY"),
            };
            paths.push(path);
        }
        let borrowed: Vec<&str> = paths.iter().map(String::as_str).collect();
        let list = changes(&borrowed);
        let rows = rows(&list, &Folds::default(), None);
        let ours: Vec<String> = rows
            .iter()
            .map(|row| match row {
                Row::Folder { label, depth, .. } => format!("{depth} {label}"),
                Row::File { change, depth } => {
                    format!("{depth} {}", list[*change].path.rsplit('/').next().unwrap_or_default())
                }
            })
            .collect();
        let mut theirs = Vec::new();
        as_the_files_panel(&filetree::build_tree(&paths), "", 0, &mut theirs);
        assert_eq!(ours, theirs);
        // and every folder's count is the number of files under it
        for row in &rows {
            if let Row::Folder { path, files, .. } = row {
                assert_eq!(*files, paths.iter().filter(|file| holds(path, file)).count(), "{path}");
            }
        }
    }

    #[test]
    fn a_list_in_any_order_gives_rows_and_never_a_panic() {
        // not the order the store keeps: rows are made all the same (a folder may then stand twice)
        let list: Vec<Change> = ["b/é.py", "a/x.py", "b/ü/y.py", "a/z.py", "目录/文件.py", "b/é/z.py", "top.py"]
            .iter()
            .flat_map(|path| changes(&[path]))
            .collect();
        let rows = rows(&list, &Folds::default(), None);
        let files = rows.iter().filter(|row| matches!(row, Row::File { .. })).count();
        assert_eq!(files, list.len(), "every file has its row");
    }

    /// How long the rows of a big list take. Run alone: `cargo test --release -p enkidu-model
    /// the_rows_of_five_thousand -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn the_rows_of_five_thousand_changed_files_are_made_in_a_moment() {
        let paths: Vec<String> = (0..5000usize)
            .map(|n| format!("app{}/module{}/sub{}/part{}/file{n}.py", n % 9, n % 37, n % 5, n % 3))
            .collect();
        let borrowed: Vec<&str> = paths.iter().map(String::as_str).collect();
        let list = changes(&borrowed);
        let started = std::time::Instant::now();
        let mut made = 0;
        for _ in 0..20 {
            made = rows(&list, &Folds::default(), None).len();
        }
        let each = started.elapsed() / 20;
        println!("{made} rows of {} changed files in {each:?}", list.len());
        let started = std::time::Instant::now();
        let mut again = list.clone();
        again.sort_by(|a, b| in_tree_order(&a.path, &b.path));
        println!("a list already in order, put in order again: {:?}", started.elapsed());
        assert!(each < std::time::Duration::from_millis(8), "{each:?}");
    }
}
