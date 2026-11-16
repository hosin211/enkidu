//! The Files panel's plain logic (item 3.9a): the tree built once per listing (folders first, both A to Z, numbers in
//! number order), single-folder chains on one row, the filter that ranks file names first, and which files the
//! workspace has (git's, plus the new ones, less the deleted ones).

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use enkidu_core::git::Change;
/// Names as a reader orders them (the core orders the changed files with it too).
pub use enkidu_core::names::by_name;

/// A folder's children, by name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FolderNode {
    /// The folders inside.
    pub dirs: Vec<String>,
    /// The files inside.
    pub files: Vec<String>,
}

/// Every folder with its children; "" is the workspace's root.
pub type Tree = HashMap<String, FolderNode>;

/// Every folder with its children, from a list of file paths.
pub fn build_tree(paths: &[String]) -> Tree {
    let mut tree = Tree::new();
    tree.insert(String::new(), FolderNode::default());
    for path in paths {
        let mut dir = String::new();
        let mut parts = path.split('/').peekable();
        while let Some(part) = parts.next() {
            if parts.peek().is_none() {
                tree.entry(dir.clone()).or_default().files.push(part.to_string());
                break;
            }
            let child = if dir.is_empty() { part.to_string() } else { format!("{dir}/{part}") };
            if !tree.contains_key(&child) {
                tree.entry(dir).or_default().dirs.push(part.to_string());
                tree.insert(child.clone(), FolderNode::default());
            }
            dir = child;
        }
    }
    for node in tree.values_mut() {
        node.dirs.sort_by(|a, b| by_name(a, b));
        node.files.sort_by(|a, b| by_name(a, b));
    }
    tree
}

/// A folder that only holds one folder is shown together with it ("src/orders"), so deep paths don't cost a click
/// a level. The label, and the folder the row really opens.
pub fn compact_folder(tree: &Tree, dir: &str, name: &str) -> (String, String) {
    let mut path = if dir.is_empty() { name.to_string() } else { format!("{dir}/{name}") };
    let mut label = name.to_string();
    while let Some(only) = tree.get(&path).filter(|node| node.files.is_empty() && node.dirs.len() == 1) {
        label = format!("{label}/{}", only.dirs[0]);
        path = format!("{path}/{}", only.dirs[0]);
    }
    (label, path)
}

/// How well a path matches what was typed: a file name match beats a folder match beats letters in order; -1 for
/// no match.
pub fn match_score(path: &str, query: &str) -> f64 {
    let lower = path.to_lowercase();
    let name_start = lower.rfind('/').map_or(0, |at| at + 1);
    score_lower(&lower, name_start, &query.trim().to_lowercase())
}

fn score_lower(path: &str, name_start: usize, query: &str) -> f64 {
    if query.is_empty() {
        return 0.;
    }
    let name_length = (path.len() - name_start) as f64;
    match path[name_start..].find(query) {
        // the name starts with it; the whole name is the best there is
        Some(0) if path.len() - name_start == query.len() => return 500.,
        Some(0) => return 400. - name_length / 100.,
        Some(_) => return 300. - name_length / 100.,
        None => {}
    }
    if path.contains(query) {
        return 200. - path.len() as f64 / 1000.;
    }
    // its letters, in order, anywhere in the path
    let mut from = 0;
    for letter in query.chars().filter(|letter| *letter != ' ') {
        match path[from..].find(letter) {
            Some(at) => from += at + letter.len_utf8(),
            None => return -1.,
        }
    }
    100. - path.len() as f64 / 1000.
}

/// What a filter found: the best matches, best first, and how many matched in all.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Found {
    pub matches: Vec<String>,
    pub total: usize,
}

/// A workspace's files, ready to be filtered: lower-case copies and where each file's name starts, made once per
/// listing (a big repo has tens of thousands of paths).
#[derive(Clone, Debug, Default)]
pub struct Files {
    paths: Vec<String>,
    lower: Vec<String>,
    name_start: Vec<usize>,
}

impl Files {
    pub fn new(paths: Vec<String>) -> Files {
        let lower: Vec<String> = paths.iter().map(|path| path.to_lowercase()).collect();
        let name_start = lower.iter().map(|path| path.rfind('/').map_or(0, |at| at + 1)).collect();
        Files { paths, lower, name_start }
    }

    pub fn paths(&self) -> &[String] {
        &self.paths
    }

    pub fn len(&self) -> usize {
        self.paths.len()
    }

    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    /// The kinds of file there are (what stands after the last dot of a name, in small letters), each with how
    /// many files have it: the most common first, at most `limit`. What the filter of Find in files offers (item
    /// 10.5).
    pub fn kinds(&self, limit: usize) -> Vec<(String, usize)> {
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for (path, name_start) in self.lower.iter().zip(&self.name_start) {
            if let Some((_, kind)) = path[*name_start..].rsplit_once('.')
                && !kind.is_empty()
            {
                *counts.entry(kind).or_default() += 1;
            }
        }
        let mut kinds: Vec<(String, usize)> =
            counts.into_iter().map(|(kind, count)| (kind.to_string(), count)).collect();
        kinds.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        kinds.truncate(limit);
        kinds
    }

    /// The files matching a filter, best first, at most `limit`, and how many matched in all. Only the best
    /// `limit` are kept while scanning, so one letter typed over a huge repo doesn't sort every path.
    pub fn filter(&self, query: &str, limit: usize) -> Found {
        let query = query.trim().to_lowercase();
        if query.is_empty() || limit == 0 {
            return Found::default();
        }
        // better: the higher score, then the shorter path, then A to Z
        let before = |a: &(usize, f64), b: &(usize, f64)| -> bool {
            if a.1 != b.1 {
                return a.1 > b.1;
            }
            let (left, right) = (&self.lower[a.0], &self.lower[b.0]);
            if left.len() != right.len() {
                return left.len() < right.len();
            }
            by_name(&self.paths[a.0], &self.paths[b.0]) == Ordering::Less
        };
        let mut best: Vec<(usize, f64)> = Vec::with_capacity(limit + 1);
        let mut total = 0;
        for (ix, lower) in self.lower.iter().enumerate() {
            let score = score_lower(lower, self.name_start[ix], &query);
            if score < 0. {
                continue;
            }
            total += 1;
            let item = (ix, score);
            if best.len() == limit && !best.last().is_some_and(|worst| before(&item, worst)) {
                continue;
            }
            let at = best.partition_point(|kept| !before(&item, kept));
            best.insert(at, item);
            best.truncate(limit);
        }
        Found { matches: best.into_iter().map(|(ix, _)| self.paths[ix].clone()).collect(), total }
    }
}

/// Where the typed text sits in a file name, to mark it there (none when it only matched the folders): the bytes
/// from and to.
pub fn name_match(name: &str, query: &str) -> Option<(usize, usize)> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return None;
    }
    // case folding can change a letter's length in bytes: only a match that maps back onto the name is marked
    let lower = name.to_lowercase();
    let at = lower.find(&query)?;
    (lower.len() == name.len() && name.is_char_boundary(at) && name.is_char_boundary(at + query.len()))
        .then_some((at, at + query.len()))
}

/// The folders above a path, outermost first: "a/b/c.py" gives ["a", "a/b"].
pub fn parent_folders(path: &str) -> Vec<String> {
    let mut folders = Vec::new();
    for (at, _) in path.match_indices('/') {
        folders.push(path[..at].to_string());
    }
    folders
}

/// The workspace's files: the tracked ones git listed, plus the new files from the changes (git's status already
/// walked the folders for those), less the files that were deleted and not committed yet.
pub fn visible_files(listed: &[String], changes: &[Change]) -> Vec<String> {
    let deleted: HashSet<&str> =
        changes.iter().filter(|change| change.kind == "deleted").map(|change| change.path.as_str()).collect();
    let tracked: HashSet<&str> = listed.iter().map(String::as_str).collect();
    let mut files: Vec<String> = listed.iter().filter(|path| !deleted.contains(path.as_str())).cloned().collect();
    let added: Vec<&Change> =
        changes.iter().filter(|c| c.kind == "untracked" && !tracked.contains(c.path.as_str())).collect();
    if !added.is_empty() {
        files.extend(added.into_iter().map(|change| change.path.clone()));
        files.sort();
    }
    files
}

/// What says that the set of files itself changed: the changes that add or remove files, not the edits. It is the
/// cue to ask git for the list again, so a busy Claude editing files doesn't make the panel read a big repo over
/// and over.
pub fn file_set_key(changes: &[Change]) -> String {
    let coming_or_going: Vec<String> =
        changes.iter().filter(|c| c.kind != "modified").map(|c| format!("{}:{}", c.kind, c.path)).collect();
    coming_or_going.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Item 10.5: the kinds of file the filter of Find in files offers, the most common first.
    #[test]
    fn the_kinds_of_file_are_counted_by_what_stands_after_the_last_dot() {
        let files = Files::new(
            [
                "Makefile",
                "a.py",
                "src/b.PY",
                "src/c.py",
                "web/Cart.vue",
                "web/x.test.js",
                "y.js",
                ".env",
                "some.dir/README",
            ]
            .map(String::from)
            .to_vec(),
        );
        let kinds = files.kinds(10);
        let said: Vec<String> = kinds.iter().map(|(kind, count)| format!("{kind} {count}")).collect();
        // by how many files, then by name; a name without a dot has no kind, and a dot in a folder's name is not it
        assert_eq!(said, ["py 3", "js 2", "env 1", "vue 1"]);
        assert_eq!(files.kinds(2).len(), 2);
        assert!(Files::new(Vec::new()).kinds(10).is_empty());
    }

    fn paths() -> Vec<String> {
        [
            "README.md",
            "src/orders/stock.py",
            "src/orders/tests/test_stock.py",
            "src/orders/serializers.py",
            "docs/stock-notes.md",
            "package.json",
            "src/legacy/old_sync.py",
            "a/b/c/deep.txt",
            "file10.txt",
            "file9.txt",
        ]
        .map(String::from)
        .to_vec()
    }

    fn node(dirs: &[&str], files: &[&str]) -> FolderNode {
        FolderNode {
            dirs: dirs.iter().map(|d| d.to_string()).collect(),
            files: files.iter().map(|f| f.to_string()).collect(),
        }
    }

    /// Lists folders first, then files, each A to Z with numbers in order.
    #[test]
    fn lists_folders_first_then_files_each_a_to_z_with_numbers_in_order() {
        let tree = build_tree(&paths());
        assert_eq!(
            tree.get(""),
            Some(&node(&["a", "docs", "src"], &["file9.txt", "file10.txt", "package.json", "README.md"]))
        );
        assert_eq!(tree.get("src"), Some(&node(&["legacy", "orders"], &[])));
        assert_eq!(tree.get("src/orders"), Some(&node(&["tests"], &["serializers.py", "stock.py"])));
    }

    /// Shows a chain of single folders as one row.
    #[test]
    fn shows_a_chain_of_single_folders_as_one_row() {
        let tree = build_tree(&paths());
        assert_eq!(compact_folder(&tree, "", "a"), ("a/b/c".to_string(), "a/b/c".to_string()));
        // src holds two folders, so it stays on its own
        assert_eq!(compact_folder(&tree, "", "src"), ("src".to_string(), "src".to_string()));
        assert_eq!(compact_folder(&tree, "src", "orders"), ("orders".to_string(), "src/orders".to_string()));
    }

    /// Finds files by name first, then by folder, then by letters in order.
    #[test]
    fn finds_files_by_name_first_then_by_folder_then_by_letters_in_order() {
        assert!(match_score("src/orders/stock.py", "stock.py") > match_score("docs/stock-notes.md", "stock"));
        assert!(match_score("src/orders/stock.py", "orders") > 0.);
        assert!(match_score("src/orders/stock.py", "sostk") > 0.);
        assert_eq!(match_score("src/orders/stock.py", "zzz"), -1.);
        let files = Files::new(paths());
        let found = files.filter("stock", 200);
        assert_eq!(found.matches[..2], ["src/orders/stock.py", "docs/stock-notes.md"]);
        assert_eq!(found.total, 3);
        assert_eq!(files.filter("py", 2).matches.len(), 2);
        // nothing typed finds nothing (the tree shows instead)
        assert_eq!(files.filter("  ", 200), Found::default());
        // capitals in what was typed don't matter
        assert_eq!(files.filter("README", 5).matches, ["README.md"]);
    }

    /// Stays quick on a repo with 60,000 files.
    #[test]
    fn stays_quick_on_a_repo_with_60_000_files() {
        let mut big = Vec::new();
        for a in 0..60 {
            for b in 0..40 {
                for c in 0..25 {
                    big.push(format!("app{a}/module{b}/file_{c}_{}.py", (a * 7 + b) % 13));
                }
            }
        }
        assert_eq!(big.len(), 60_000);
        let started = std::time::Instant::now();
        let tree = build_tree(&big);
        let tree_ms = started.elapsed().as_secs_f64() * 1000.;
        assert_eq!(tree.get("app3/module7").map(|node| node.files.len()), Some(25));
        let started = std::time::Instant::now();
        let files = Files::new(big);
        let prepared_ms = started.elapsed().as_secs_f64() * 1000.;
        let mut times = Vec::new();
        for query in ["f", "file_3", "mod12/f9", "zzz"] {
            let started = std::time::Instant::now();
            let found = files.filter(query, 50);
            times.push(started.elapsed().as_secs_f64() * 1000.);
            assert!(found.matches.len() <= 50);
        }
        println!("FILES tree={tree_ms:.0}ms prepared={prepared_ms:.0}ms queries={times:.1?}ms");
        // generous bounds: this checks the approach, not the machine
        assert!(tree_ms < 1000., "the tree took {tree_ms:.0} ms");
        assert!(times.iter().all(|ms| *ms < 250.), "a query took too long: {times:?}");
        // the best match comes first even among 60,000
        assert!(files.filter("file_3_9.py", 5).matches[0].ends_with("file_3_9.py"));
    }

    /// Marks the typed text in a file name.
    #[test]
    fn marks_the_typed_text_in_a_file_name() {
        assert_eq!(name_match("stock_levels.py", "LEVEL"), Some((6, 11)));
        assert_eq!(name_match("stock.py", "orders"), None);
        assert_eq!(name_match("stock.py", "  "), None);
    }

    /// Names the folders to open to reach a file.
    #[test]
    fn names_the_folders_to_open_to_reach_a_file() {
        assert_eq!(parent_folders("src/orders/stock.py"), ["src", "src/orders"]);
        assert_eq!(parent_folders("README.md"), Vec::<String>::new());
    }

    fn change(kind: &str, path: &str) -> Change {
        Change {
            path: path.into(),
            old_path: None,
            kind: kind.into(),
            staged: false,
            additions: None,
            deletions: None,
            sig: String::new(),
        }
    }

    /// Which files a workspace has, and the key of that set.
    #[test]
    fn the_workspace_has_gits_files_plus_the_new_ones_less_the_deleted_ones() {
        let listed: Vec<String> = ["README.md", "src/app.py", "src/old.py"].map(String::from).to_vec();
        // edits change nothing, and the very same list comes back in git's order
        let edits = [change("modified", "src/app.py")];
        assert_eq!(visible_files(&listed, &edits), listed);
        assert_eq!(file_set_key(&edits), "");
        // a new file is there before git lists it, a deleted one is gone before the commit
        let changes = [
            change("modified", "src/app.py"),
            change("untracked", "notes.md"),
            change("deleted", "src/old.py"),
            change("untracked", "README.md"),
        ];
        assert_eq!(visible_files(&listed, &changes), ["README.md", "notes.md", "src/app.py"]);
        assert_eq!(file_set_key(&changes), "untracked:notes.md\ndeleted:src/old.py\nuntracked:README.md");
        assert_eq!(visible_files(&[], &[]), Vec::<String>::new());
    }
}
