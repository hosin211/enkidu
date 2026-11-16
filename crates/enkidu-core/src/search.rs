//! Text search across a workspace's files ("Find in files"). See SEARCH.md for why it works this way.
//!
//! No index: like VS Code (which runs ripgrep) it scans the files git knows about, in parallel, with Rust's regex
//! engine, the one inside ripgrep. Its literal prefilters and SIMD make a scan of a few hundred MB take a few hundred
//! ms once the files are in the OS file cache, results are always fresh (Claude's edits included), and memory stays
//! flat: one file buffer per thread and at most MAX_LINES result lines. A trigram index would cost about 20% of the
//! source size on disk, RAM to use it, and an update on every edit; it only pays off for many GB of code.

use anyhow::{anyhow, Result};
use regex::bytes::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Instant;

use crate::git;

/// Files bigger than this (bundles, data dumps) are skipped and counted.
pub const MAX_FILE_BYTES: u64 = 1_000_000;
/// The search stops after this many result lines (VS Code's own cap is also in the thousands).
pub const MAX_LINES: usize = 1000;
/// Lines listed per file; the rest of a file's matches are counted.
pub const MAX_PER_FILE: usize = 50;
/// A long line is shown as a window around the match.
const WINDOW_BEFORE: usize = 60;
const WINDOW_AFTER: usize = 180;
/// Files per unit of work: small enough to spread evenly, big enough to keep the queue cheap.
const CHUNK: usize = 32;

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct TextQuery {
    pub pattern: String,
    /// None: smart case (ignore case unless the pattern has a capital letter).
    pub case_sensitive: Option<bool>,
    #[serde(default)]
    pub whole_word: bool,
    #[serde(default)]
    pub regex: bool,
    /// Only the files of this kind are read: what stands after the last dot of their name, as `py` (item 10.5).
    /// None: every file.
    #[serde(default)]
    pub kind: Option<String>,
}

/// One line with a match, as shown: the line (or a window of it), its number, and where the matches are, in chars.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LineHit {
    pub line: u32,
    pub text: String,
    pub ranges: Vec<[u32; 2]>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileHits {
    pub path: String,
    pub lines: Vec<LineHit>,
    /// Lines with matches in this file past the ones listed.
    pub more: u32,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TextSummary {
    pub lines: usize,
    pub files: usize,
    /// Files read.
    pub searched: usize,
    /// Files skipped as binary or too large.
    pub skipped: usize,
    /// More lines matched than MAX_LINES.
    pub truncated: bool,
    /// Every file with a match, listed or not.
    pub matched_files: usize,
    /// A newer search replaced this one.
    pub cancelled: bool,
    pub ms: u64,
}

pub fn compile(q: &TextQuery) -> Result<Regex> {
    if q.pattern.is_empty() {
        return Err(anyhow!("Type something to search for"));
    }
    let mut pattern = if q.regex { q.pattern.clone() } else { regex::escape(&q.pattern) };
    if q.whole_word {
        pattern = format!(r"\b(?:{pattern})\b");
    }
    let ignore_case = match q.case_sensitive {
        Some(sensitive) => !sensitive,
        None => !q.pattern.chars().any(char::is_uppercase),
    };
    RegexBuilder::new(&pattern)
        .case_insensitive(ignore_case)
        .multi_line(true)
        .size_limit(32 << 20)
        .build()
        .map_err(|e| anyhow!("Not a valid regular expression: {}", e.to_string().lines().last().unwrap_or("")))
}

enum Outcome {
    Hits(Vec<LineHit>, u32),
    /// Matched, lines not wanted (the display cap was reached).
    Matched,
    NoMatch,
    Skipped,
    Missing,
}

/// A finished scan: what was shown, and every file that matched (indices into the list searched).
pub struct Scan {
    pub summary: TextSummary,
    pub matched: Vec<usize>,
}

/// Scans `files` (relative to `root`, in the order given) in parallel and hands each finished stretch of results to
/// `emit` in that same order, so results arrive sorted and the first ones show while the rest are still read. After
/// MAX_LINES the scan goes on without collecting lines, only noting which files match: the total is then exact, and a
/// longer query typed next only has to read those files.
pub fn search_files(root: &Path, files: &[&str], re: &Regex, cancel: &AtomicBool, mut emit: impl FnMut(Vec<FileHits>)) -> Scan {
    let started = Instant::now();
    // Past about 8 threads the file system, not the CPU, is the limit: more threads contend in the kernel and get slower
    // (measured on a 42,000-file repo: 4 threads 1.0 s, 8 threads 0.86 s, 10 threads 1.17 s, 16 threads 1.28 s).
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).saturating_sub(1).clamp(1, 8);
    let chunks = files.len().div_ceil(CHUNK);
    let next = AtomicUsize::new(0);
    let want_lines = AtomicBool::new(true);
    let mut summary = TextSummary::default();
    let mut matched = Vec::new();

    std::thread::scope(|scope| {
        let (tx, rx) = mpsc::channel::<(usize, Vec<FileHits>, Vec<usize>, usize, usize)>();
        for _ in 0..threads {
            let tx = tx.clone();
            let (next, want_lines) = (&next, &want_lines);
            scope.spawn(move || {
                let mut buf = Vec::with_capacity(256 * 1024);
                loop {
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let c = next.fetch_add(1, Ordering::Relaxed);
                    if c >= chunks {
                        break;
                    }
                    let (mut found, mut hit, mut searched, mut skipped) = (Vec::new(), Vec::new(), 0, 0);
                    let lines = want_lines.load(Ordering::Relaxed);
                    for i in c * CHUNK..((c + 1) * CHUNK).min(files.len()) {
                        match search_one(&root.join(files[i]), re, &mut buf, lines) {
                            Outcome::Hits(found_lines, more) => {
                                searched += 1;
                                if !found_lines.is_empty() {
                                    hit.push(i);
                                    found.push(FileHits { path: files[i].to_owned(), lines: found_lines, more });
                                }
                            }
                            Outcome::Matched => {
                                searched += 1;
                                hit.push(i);
                            }
                            Outcome::NoMatch => searched += 1,
                            Outcome::Skipped => skipped += 1,
                            Outcome::Missing => {}
                        }
                    }
                    if tx.send((c, found, hit, searched, skipped)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);

        // Chunks finish out of order; hold them until every earlier one is in.
        let mut waiting: BTreeMap<usize, Vec<FileHits>> = BTreeMap::new();
        let mut want = 0;
        for (c, found, hit, searched, skipped) in rx {
            summary.searched += searched;
            summary.skipped += skipped;
            matched.extend(hit);
            waiting.insert(c, found);
            while let Some(found) = waiting.remove(&want) {
                want += 1;
                if summary.truncated {
                    continue;
                }
                let mut batch = Vec::new();
                for mut f in found {
                    let room = MAX_LINES - summary.lines;
                    if f.lines.len() > room {
                        f.more += (f.lines.len() - room) as u32;
                        f.lines.truncate(room);
                    }
                    summary.lines += f.lines.len();
                    summary.files += 1;
                    batch.push(f);
                    if summary.lines >= MAX_LINES {
                        summary.truncated = true;
                        want_lines.store(false, Ordering::Relaxed);
                        break;
                    }
                }
                if !batch.is_empty() {
                    emit(batch);
                }
            }
        }
    });
    matched.sort_unstable();
    summary.matched_files = matched.len();
    summary.cancelled = cancel.load(Ordering::Relaxed);
    summary.ms = started.elapsed().as_millis() as u64;
    Scan { summary, matched }
}

fn search_one(path: &Path, re: &Regex, buf: &mut Vec<u8>, want_lines: bool) -> Outcome {
    let Ok(mut file) = std::fs::File::open(path) else { return Outcome::Missing };
    let size = file.metadata().map(|m| if m.is_file() { m.len() } else { u64::MAX }).unwrap_or(u64::MAX);
    if size == u64::MAX {
        return Outcome::Missing;
    }
    if size > MAX_FILE_BYTES {
        return Outcome::Skipped;
    }
    buf.clear();
    if file.read_to_end(buf).is_err() {
        return Outcome::Missing;
    }
    if memchr::memchr(0, &buf[..buf.len().min(8192)]).is_some() {
        return Outcome::Skipped;
    }
    let text = buf.as_slice();
    if !want_lines {
        return if re.is_match(text) { Outcome::Matched } else { Outcome::NoMatch };
    }
    let mut lines: Vec<LineHit> = Vec::new();
    let mut more = 0u32;
    let mut line_no = 1u32;
    let mut counted = 0usize;
    // the listed line: where it starts, where its shown part starts, and whether "…" was put in front
    let mut current: Option<(usize, usize, u32)> = None;
    for m in re.find_iter(text) {
        let (start, end) = (m.start(), m.end());
        if start == end {
            continue; // an empty match (like ^) marks nothing
        }
        line_no += memchr::memchr_iter(b'\n', &text[counted..start]).count() as u32;
        counted = start;
        let line_start = memchr::memrchr(b'\n', &text[..start]).map_or(0, |i| i + 1);
        let line_end = memchr::memchr(b'\n', &text[start..]).map_or(text.len(), |i| start + i);
        if let Some((listed, window, lead)) = current {
            if listed == line_start {
                // another match on the same line: mark it too, when it is in the shown part
                if let Some(hit) = lines.last_mut().filter(|_| window != usize::MAX) {
                    if let Some(range) = range_in(text, window, lead, &hit.text, start, end.min(line_end)) {
                        hit.ranges.push(range);
                    }
                }
                continue;
            }
        }
        if lines.len() >= MAX_PER_FILE {
            more += 1;
            current = Some((line_start, usize::MAX, 0));
            continue;
        }
        let (hit, window, lead) = make_hit(text, line_start, line_end, start, end.min(line_end), line_no);
        current = Some((line_start, window, lead));
        lines.push(hit);
    }
    Outcome::Hits(lines, more)
}

/// Steps back to the start of a UTF-8 character.
fn char_start(text: &[u8], mut i: usize) -> usize {
    while i > 0 && i < text.len() && (text[i] & 0xC0) == 0x80 {
        i -= 1;
    }
    i
}

fn chars(bytes: &[u8]) -> u32 {
    String::from_utf8_lossy(bytes).chars().count() as u32
}

/// The shown text of a line: without its indentation, and only a window around the match when the line is long.
/// Also returns where the shown part starts and how many chars were put in front of it ("…").
fn make_hit(text: &[u8], line_start: usize, line_end: usize, start: usize, end: usize, line: u32) -> (LineHit, usize, u32) {
    let mut line_end = line_end;
    if line_end > line_start && text[line_end - 1] == b'\r' {
        line_end -= 1;
    }
    let mut from = line_start;
    while from < start && (text[from] == b' ' || text[from] == b'\t') {
        from += 1;
    }
    let mut to = line_end;
    let mut prefix = false;
    if start - from > WINDOW_BEFORE + 20 {
        from = char_start(text, start - WINDOW_BEFORE);
        prefix = true;
    }
    let mut suffix = false;
    if to > end && to - end > WINDOW_AFTER + 20 {
        to = char_start(text, end + WINDOW_AFTER);
        suffix = true;
    }
    let lead = u32::from(prefix);
    let shown = String::from_utf8_lossy(&text[from..to]);
    let text_out = format!("{}{shown}{}", if prefix { "…" } else { "" }, if suffix { "…" } else { "" });
    let s = lead + chars(&text[from..start]);
    let e = s + chars(&text[start..end.min(to)]);
    (LineHit { line, text: text_out, ranges: vec![[s, e]] }, from, lead)
}

/// Where a later match on the same line sits in the shown text, if it is shown at all.
fn range_in(text: &[u8], window: usize, lead: u32, shown: &str, start: usize, end: usize) -> Option<[u32; 2]> {
    if start < window {
        return None;
    }
    let s = lead + chars(&text[window..start]);
    let e = s + chars(&text[start..end]);
    if e as usize > shown.chars().count() {
        return None;
    }
    Some([s, e])
}

/// What stands after the last dot of a file's name, in small letters: its kind, as the filter of Find in files
/// names it. A name without a dot has none.
pub fn kind_of(path: &str) -> Option<String> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let (_, kind) = name.rsplit_once('.')?;
    (!kind.is_empty()).then(|| kind.to_lowercase())
}

/// The files of one kind, in the order given; all of them without a kind to keep to.
pub fn of_kind<'a>(files: Vec<&'a str>, kind: Option<&str>) -> Vec<&'a str> {
    match kind.map(str::to_lowercase) {
        Some(kind) => files.into_iter().filter(|path| kind_of(path).as_deref() == Some(kind.as_str())).collect(),
        None => files,
    }
}

/// The files of a workspace's submodules as they were last listed.
#[derive(Default)]
struct Submodules {
    /// The stamp of `.gitmodules` when the submodules were named.
    named: (u64, u128),
    /// Each one: its folder, its git index file with its stamp, and what git tracks there.
    modules: Vec<(String, PathBuf, (u64, u128), Vec<String>)>,
    /// The watcher's change counter when their new files were listed.
    generation: Option<u64>,
    new: Vec<String>,
    all: Arc<Vec<String>>,
}

/// The tracked files of each workspace, read from git's index and kept until the index file itself changes, so
/// typing a query doesn't ask git again for every keystroke.
#[derive(Default)]
pub struct FileLists {
    cache: Mutex<HashMap<PathBuf, (PathBuf, (u64, u128), Arc<Vec<String>>)>>,
    submodules: Mutex<HashMap<PathBuf, Submodules>>,
}

fn stamp(index: &Path) -> (u64, u128) {
    std::fs::metadata(index)
        .map(|m| {
            let mtime = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos());
            (m.len(), mtime)
        })
        .unwrap_or((0, 0))
}

impl FileLists {
    pub fn tracked(&self, dir: &Path) -> Result<Arc<Vec<String>>> {
        let known = self.cache.lock().unwrap_or_else(|e| e.into_inner()).get(dir).cloned();
        if let Some((index, was, list)) = &known {
            if stamp(index) == *was {
                return Ok(list.clone());
            }
        }
        let index = match &known {
            Some((index, _, _)) => index.clone(),
            None => {
                let p = git::git_ok(dir, &["rev-parse", "--git-path", "index"])?;
                let p = PathBuf::from(p.trim());
                if p.is_absolute() { p } else { dir.join(p) }
            }
        };
        let before = stamp(&index);
        let list = Arc::new(git::tracked_files(dir)?);
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).insert(dir.to_path_buf(), (index, before, list.clone()));
        Ok(list)
    }

    /// The files inside the workspace's submodules, each from the workspace's root, sorted: those each submodule's
    /// git tracks, and its new files that it doesn't ignore (item 10.7). A workspace without `.gitmodules` costs one
    /// look at the folder. What a submodule tracks is asked again when its own index file changes; its new files
    /// when the watcher's change counter (`generation`) moved, which any file event in the workspace does.
    pub fn in_submodules(&self, dir: &Path, generation: u64) -> Arc<Vec<String>> {
        let named = stamp(&dir.join(".gitmodules"));
        let mut all = self.submodules.lock().unwrap_or_else(|e| e.into_inner());
        if named == (0, 0) {
            all.remove(dir);
            return Arc::default();
        }
        let known = all.entry(dir.to_path_buf()).or_default();
        let mut changed = false;
        if known.named != named {
            known.named = named;
            known.modules = git::submodules(dir).into_iter().map(|(path, index)| (path, index, (0, 0), Vec::new())).collect();
            known.generation = None;
            changed = true;
        }
        for (path, index, was, tracked) in &mut known.modules {
            let now = stamp(index);
            if now != *was || now == (0, 0) {
                *tracked = git::submodule_tracked(dir, path);
                *was = now;
                changed = true;
            }
        }
        if known.generation != Some(generation) {
            let new: Vec<String> = known.modules.iter().flat_map(|(path, ..)| git::submodule_new(dir, path)).collect();
            changed |= new != known.new;
            known.new = new;
            known.generation = Some(generation);
        }
        if changed {
            let mut files: Vec<String> = known.modules.iter().flat_map(|(_, _, _, tracked)| tracked.iter().cloned()).collect();
            files.extend(known.new.iter().cloned());
            files.sort_unstable();
            files.dedup();
            known.all = Arc::new(files);
        }
        known.all.clone()
    }
}

/// Cancels a workspace's running text search when a newer one starts.
#[derive(Default)]
pub struct Running {
    flags: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl Running {
    pub fn begin(&self, workspace_id: &str) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        if let Some(old) = self.flags.lock().unwrap_or_else(|e| e.into_inner()).insert(workspace_id.to_owned(), flag.clone()) {
            old.store(true, Ordering::Relaxed);
        }
        flag
    }

    pub fn cancel(&self, workspace_id: &str) {
        if let Some(flag) = self.flags.lock().unwrap_or_else(|e| e.into_inner()).remove(workspace_id) {
            flag.store(true, Ordering::Relaxed);
        }
    }
}

fn ignores_case(q: &TextQuery) -> bool {
    match q.case_sensitive {
        Some(sensitive) => !sensitive,
        None => !q.pattern.chars().any(char::is_uppercase),
    }
}

/// True when every match of `new` must be in a file that matched `old`: typing more of the same text. Then the
/// new search only reads the files the old one found. Regular expressions and whole-word searches never narrow.
pub fn narrows(old: &TextQuery, new: &TextQuery) -> bool {
    // another kind of file is another set of files
    if old.regex || new.regex || old.whole_word || old.pattern.is_empty() || old.kind != new.kind {
        return false;
    }
    if ignores_case(old) {
        new.pattern.to_lowercase().contains(&old.pattern.to_lowercase())
    } else {
        !ignores_case(new) && new.pattern.contains(&old.pattern)
    }
}

/// The last finished text search of a workspace: what it looked for, which files matched, and the state of the
/// files at the time (any change since makes it useless for narrowing).
pub struct LastText {
    pub query: TextQuery,
    pub generation: u64,
    pub tracked: Arc<Vec<String>>,
    pub untracked: Vec<String>,
    /// The files inside the submodules.
    pub nested: Arc<Vec<String>>,
    pub matched: Vec<String>,
}

impl LastText {
    /// The files a new search has to read, when this one narrows it.
    pub fn candidates(
        &self,
        query: &TextQuery,
        generation: u64,
        tracked: &Arc<Vec<String>>,
        untracked: &[String],
        nested: &Arc<Vec<String>>,
    ) -> Option<&[String]> {
        let same_files = generation == self.generation
            && Arc::ptr_eq(tracked, &self.tracked)
            && untracked == self.untracked.as_slice()
            && Arc::ptr_eq(nested, &self.nested);
        (same_files && narrows(&self.query, query)).then_some(self.matched.as_slice())
    }
}

/// Tracked files, new ones and the files inside the submodules, sorted, without the ones deleted from disk (those
/// just aren't found).
pub fn search_list<'a>(tracked: &'a [String], untracked: &'a [String], nested: &'a [String]) -> Vec<&'a str> {
    let mut all: Vec<&str> = Vec::with_capacity(tracked.len() + untracked.len() + nested.len());
    all.extend(tracked.iter().map(String::as_str));
    all.extend(untracked.iter().map(String::as_str));
    all.extend(nested.iter().map(String::as_str));
    all.sort_unstable();
    all.dedup();
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(pattern: &str) -> TextQuery {
        TextQuery { pattern: pattern.into(), ..Default::default() }
    }

    fn run(root: &Path, files: &[&str], query: TextQuery) -> (Vec<FileHits>, TextSummary) {
        let re = compile(&query).unwrap();
        let mut out = Vec::new();
        let scan = search_files(root, files, &re, &AtomicBool::new(false), |b| out.extend(b));
        (out, scan.summary)
    }

    #[test]
    fn finds_lines_in_order_with_smart_case_words_and_regex() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.py"), "class StockLevel:\n    def save(self):\n        stock = StockLevel()  # stock\n").unwrap();
        std::fs::write(root.join("src/b.py"), "from a import StockLevel\nrestock = 1\n").unwrap();
        std::fs::write(root.join("bin.dat"), [0u8, 1, 2, b's', b't', b'o', b'c', b'k']).unwrap();
        let files = ["bin.dat", "missing.py", "src/a.py", "src/b.py"];

        // lower case ignores case; results come in path order, one row per line with every match marked
        let (hits, summary) = run(root, &files, q("stock"));
        assert_eq!(hits.iter().map(|h| h.path.as_str()).collect::<Vec<_>>(), ["src/a.py", "src/b.py"]);
        assert_eq!(hits[0].lines.iter().map(|l| l.line).collect::<Vec<_>>(), [1, 3]);
        assert_eq!(hits[0].lines[1].text, "stock = StockLevel()  # stock");
        assert_eq!(hits[0].lines[1].ranges, [[0, 5], [8, 13], [24, 29]]);
        assert_eq!((summary.lines, summary.files, summary.skipped), (4, 2, 1));

        // a capital makes it case-sensitive
        let (hits, _) = run(root, &files, q("Stock"));
        assert_eq!(hits.iter().map(|h| h.lines.len()).sum::<usize>(), 3);
        // whole words: "restock" doesn't count
        let (hits, _) = run(root, &files, TextQuery { whole_word: true, ..q("stock") });
        assert_eq!(hits.iter().map(|h| h.lines.len()).sum::<usize>(), 1);
        // a regular expression
        let (hits, _) = run(root, &files, TextQuery { regex: true, ..q(r"def \w+\(") });
        assert_eq!(hits[0].lines[0].line, 2);
        assert!(compile(&TextQuery { regex: true, ..q("(") }).is_err());
    }

    #[test]
    fn long_lines_show_a_window_and_big_results_stop_at_the_cap() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let long = format!("{}needle{}\n", "x".repeat(500), "y".repeat(500));
        std::fs::write(root.join("long.js"), &long).unwrap();
        let (hits, _) = run(root, &["long.js"], q("needle"));
        let line = &hits[0].lines[0];
        assert!(line.text.starts_with('…') && line.text.ends_with('…'));
        let [s, e] = line.ranges[0];
        assert_eq!(line.text.chars().skip(s as usize).take((e - s) as usize).collect::<String>(), "needle");

        // a file with many matching lines lists MAX_PER_FILE and counts the rest
        std::fs::write(root.join("many.txt"), "hit\n".repeat(80)).unwrap();
        let (hits, _) = run(root, &["many.txt"], q("hit"));
        assert_eq!((hits[0].lines.len(), hits[0].more), (MAX_PER_FILE, 30));

        // across files the search stops at MAX_LINES
        let names: Vec<String> = (0..30).map(|i| format!("f{i:02}.txt")).collect();
        for n in &names {
            std::fs::write(root.join(n), "hit\n".repeat(45)).unwrap();
        }
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let (hits, summary) = run(root, &refs, q("hit"));
        assert!(summary.truncated);
        assert_eq!(hits.iter().map(|h| h.lines.len()).sum::<usize>(), MAX_LINES);
        assert_eq!(hits[0].path, "f00.txt", "the first files in order come first");
        // past the cap the scan still counts every file with a match
        assert_eq!(summary.matched_files, 30);
    }

    #[test]
    fn typing_more_of_the_same_text_narrows_to_the_files_found_before() {
        assert!(narrows(&q("stock"), &q("stocklevel")));
        assert!(narrows(&q("stock"), &q("StockLevel")), "any case before covers an exact case now");
        assert!(narrows(&q("Stock"), &q("StockLevel")));
        assert!(!narrows(&q("Stock"), &q("stocklevel")), "exact case before can't cover any case now");
        assert!(!narrows(&q("stock"), &q("level")));
        assert!(!narrows(&TextQuery { regex: true, ..q("st.ck") }, &q("stock")));
        assert!(!narrows(&TextQuery { whole_word: true, ..q("stock") }, &q("stocklevel")));
        assert!(narrows(&q("stock"), &TextQuery { whole_word: true, ..q("stocklevel") }));

        let tracked = Arc::new(vec!["a.py".to_string(), "b.py".to_string()]);
        let nested = Arc::new(Vec::new());
        let last = LastText {
            query: q("stock"),
            generation: 4,
            tracked: tracked.clone(),
            untracked: vec![],
            nested: nested.clone(),
            matched: vec!["a.py".into()],
        };
        assert_eq!(last.candidates(&q("stockl"), 4, &tracked, &[], &nested), Some(&["a.py".to_string()][..]));
        assert_eq!(last.candidates(&q("stockl"), 5, &tracked, &[], &nested), None, "a file changed since");
        assert_eq!(last.candidates(&q("stockl"), 4, &tracked, &["new.py".to_string()], &nested), None, "a new file appeared");
        let other = Arc::new(vec!["spa/a.vue".to_string()]);
        assert_eq!(last.candidates(&q("stockl"), 4, &tracked, &[], &other), None, "the submodules' files changed");
        // another kind of file is another set of files: nothing found among `py` files says what `vue` files hold
        let vue = TextQuery { kind: Some("vue".into()), ..q("stockl") };
        assert_eq!(last.candidates(&vue, 4, &tracked, &[], &nested), None, "another kind of file");
        assert!(narrows(&TextQuery { kind: Some("py".into()), ..q("stock") }, &TextQuery { kind: Some("py".into()), ..q("stockl") }));
    }

    /// Item 10.5: Find in files can keep to one kind of file, named by what stands after the last dot.
    #[test]
    fn a_scan_can_keep_to_one_kind_of_file() {
        assert_eq!(kind_of("src/orders/stock.py").as_deref(), Some("py"));
        assert_eq!(kind_of("web/Cart.VUE").as_deref(), Some("vue"), "small letters");
        assert_eq!(kind_of("archive.tar.gz").as_deref(), Some("gz"), "what stands after the last dot");
        assert_eq!(kind_of(".env").as_deref(), Some("env"));
        assert_eq!(kind_of("Makefile"), None);
        assert_eq!(kind_of("some.dir/Makefile"), None, "a dot in a folder's name is not the file's");
        assert_eq!(kind_of("notes."), None);
        let files = vec!["Makefile", "a.py", "b.vue", "docs/c.PY", "d.pyc"];
        assert_eq!(of_kind(files.clone(), Some("py")), ["a.py", "docs/c.PY"]);
        assert_eq!(of_kind(files.clone(), Some("PY")), ["a.py", "docs/c.PY"]);
        assert_eq!(of_kind(files.clone(), None), files);
        assert!(of_kind(files, Some("rs")).is_empty());

        // only those files are read
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(root.join("a.py"), "stock = 1\n").unwrap();
        std::fs::write(root.join("b.vue"), "<p>stock</p>\n").unwrap();
        std::fs::write(root.join("c.md"), "stock\n").unwrap();
        let all = vec!["a.py", "b.vue", "c.md"];
        let (hits, summary) = run(root, &of_kind(all.clone(), Some("vue")), q("stock"));
        assert_eq!(hits.iter().map(|h| h.path.as_str()).collect::<Vec<_>>(), ["b.vue"]);
        assert_eq!((summary.searched, summary.lines), (1, 1));
        let (hits, summary) = run(root, &of_kind(all, None), q("stock"));
        assert_eq!((hits.len(), summary.searched), (3, 3));
    }

    /// Items 10.7 and 10.8: the files inside a submodule are in the search's list, each from the workspace's root:
    /// what the submodule's own git tracks, and its new files that it doesn't ignore. They are asked for again
    /// only when something changed.
    #[test]
    fn the_files_inside_a_submodule_are_listed_with_its_new_ones_and_without_its_ignored_ones() {
        use crate::git::testutil::run as git;
        let tmp = tempfile::tempdir().unwrap();
        let (_origin, main) = crate::git::testutil::repo_with_origin(tmp.path());
        let lists = FileLists::default();
        // a workspace without submodules has nothing to list
        assert!(lists.in_submodules(&main, 1).is_empty());

        // another repo, added as the submodule `spa`
        let spa = tmp.path().join("spa-source");
        std::fs::create_dir_all(spa.join("src")).unwrap();
        git(&spa, &["init", "-q", "-b", "main"]);
        git(&spa, &["config", "user.email", "t@example.com"]);
        git(&spa, &["config", "user.name", "Test"]);
        std::fs::write(spa.join("src/Cart.vue"), "<template><p>cart</p></template>\n").unwrap();
        std::fs::write(spa.join("package.json"), "{}\n").unwrap();
        std::fs::write(spa.join(".gitignore"), "dist/\n").unwrap();
        git(&spa, &["add", "."]);
        git(&spa, &["commit", "-q", "-m", "the spa"]);
        git(&main, &["-c", "protocol.file.allow=always", "submodule", "add", "-q", spa.to_str().unwrap(), "spa"]);
        let listed = lists.in_submodules(&main, 1);
        assert_eq!(*listed, ["spa/.gitignore", "spa/package.json", "spa/src/Cart.vue"]);
        // the main list still leaves the submodule's own entry out, and nothing is listed twice
        let tracked = lists.tracked(&main).unwrap();
        assert!(!tracked.iter().any(|path| path == "spa"));
        let all = search_list(&tracked, &[], &listed);
        assert!(all.contains(&"spa/src/Cart.vue") && all.contains(&"README.md"));
        assert!(all.windows(2).all(|pair| pair[0] < pair[1]), "sorted, each once");

        // nothing changed: the very same list, without asking git
        assert!(Arc::ptr_eq(&listed, &lists.in_submodules(&main, 1)));
        // a new file in the submodule, and one it ignores: listed when the watcher says something changed
        std::fs::write(main.join("spa/src/New.vue"), "<template />\n").unwrap();
        std::fs::create_dir_all(main.join("spa/dist")).unwrap();
        std::fs::write(main.join("spa/dist/bundle.js"), "x\n").unwrap();
        assert!(Arc::ptr_eq(&listed, &lists.in_submodules(&main, 1)), "the same counter: the list on hand");
        let now = lists.in_submodules(&main, 2);
        assert_eq!(*now, ["spa/.gitignore", "spa/package.json", "spa/src/Cart.vue", "spa/src/New.vue"]);
        // the submodule tracks it now: its own index changed, and the list is the same files
        git(&main.join("spa"), &["add", "src/New.vue"]);
        assert_eq!(*lists.in_submodules(&main, 3), *now);
        // a tracked file removed there is gone from the list
        git(&main.join("spa"), &["rm", "-q", "-f", "package.json"]);
        assert_eq!(*lists.in_submodules(&main, 4), ["spa/.gitignore", "spa/src/Cart.vue", "spa/src/New.vue"]);

        // and the scan reads them: a line in the submodule is found, the ignored bundle is not
        let nested = lists.in_submodules(&main, 4);
        let files = search_list(&tracked, &[], &nested);
        let (hits, _) = run(&main, &files, q("cart"));
        assert_eq!(hits.iter().map(|h| h.path.as_str()).collect::<Vec<_>>(), ["spa/src/Cart.vue"]);
    }

    #[test]
    fn a_newer_search_cancels_the_running_one() {
        let running = Running::default();
        let first = running.begin("w1");
        let second = running.begin("w1");
        assert!(first.load(Ordering::Relaxed));
        assert!(!second.load(Ordering::Relaxed));
        running.cancel("w1");
        assert!(second.load(Ordering::Relaxed));
    }

    /// Run on a big repo: ENKIDU_BIG_REPO=<path> cargo test --release text_search_speed -- --ignored --nocapture
    #[test]
    #[ignore]
    fn text_search_speed() {
        let root = PathBuf::from(std::env::var("ENKIDU_BIG_REPO").expect("set ENKIDU_BIG_REPO"));
        let lists = FileLists::default();
        let t = Instant::now();
        let tracked = lists.tracked(&root).unwrap();
        let list_ms = t.elapsed().as_millis();
        let t = Instant::now();
        let _ = lists.tracked(&root).unwrap();
        let cached_us = t.elapsed().as_micros();
        let files = search_list(&tracked, &[], &[]);
        for (label, query) in [
            ("QuerySet", q("QuerySet")),
            ("def get_queryset", q("def get_queryset")),
            ("import", q("import")),
            ("queryset (any case)", q("queryset")),
            ("class \\w+Admin\\( (regex)", TextQuery { regex: true, ..q(r"class \w+Admin\(") }),
            ("zzqqxx (no match)", q("zzqqxx")),
        ] {
            let re = compile(&query).unwrap();
            let mut first = None;
            let t = Instant::now();
            let scan = search_files(&root, &files, &re, &AtomicBool::new(false), |_| {
                first.get_or_insert(t.elapsed().as_millis());
            });
            let s = scan.summary;
            println!(
                "ENKIDU_TEXT {label}: {} ms total, first results at {:?} ms, {} lines shown, {} files match, read {} skipped {}{}",
                s.ms,
                first,
                s.lines,
                s.matched_files,
                s.searched,
                s.skipped,
                if s.truncated { " (capped)" } else { "" }
            );
        }
        println!("ENKIDU_TEXT file list: {list_ms} ms from git, {cached_us} µs cached, {} files", files.len());
        // typing on: "Query", then "QuerySet" only reads the files "Query" matched
        let first = search_files(&root, &files, &compile(&q("Query")).unwrap(), &AtomicBool::new(false), |_| {});
        let candidates: Vec<&str> = first.matched.iter().map(|&i| files[i]).collect();
        let t = Instant::now();
        let refined = search_files(&root, &candidates, &compile(&q("QuerySet")).unwrap(), &AtomicBool::new(false), |_| {});
        let full = search_files(&root, &files, &compile(&q("QuerySet")).unwrap(), &AtomicBool::new(false), |_| {});
        println!(
            "ENKIDU_TEXT typing on: \"Query\" {} ms over {} files, then \"QuerySet\" {} ms over its {} files ({} match; a full scan takes {} ms and finds {})",
            first.summary.ms,
            files.len(),
            t.elapsed().as_millis().min(refined.summary.ms as u128 + 1),
            candidates.len(),
            refined.summary.matched_files,
            full.summary.ms,
            full.summary.matched_files
        );
    }
}
