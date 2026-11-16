//! Everything git, through the git CLI so the user's own git config applies.

use anyhow::{anyhow, bail, Result};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

use crate::redact::redact_urls;

/// Git's empty tree, used as the base for repos that have no commit yet.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
/// Diffs larger than this are not sent to the UI.
const MAX_DIFF_BYTES: usize = 1_500_000;

pub struct GitOutput {
    pub ok: bool,
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub fn git(dir: &Path, args: &[&str]) -> Result<GitOutput> {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("LC_ALL", "C")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .map_err(|e| anyhow!("Can't run git: {e}"))?;
    Ok(GitOutput {
        ok: out.status.success(),
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

/// Runs git and returns stdout, or an error with git's message (URLs removed).
pub fn git_ok(dir: &Path, args: &[&str]) -> Result<String> {
    let out = git(dir, args)?;
    if out.ok {
        Ok(out.stdout)
    } else {
        let msg = redact_urls(out.stderr.trim());
        bail!("{}", if msg.is_empty() { format!("git {} failed", args.first().unwrap_or(&"")) } else { msg })
    }
}

fn abs(dir: &Path, p: &str) -> PathBuf {
    let path = PathBuf::from(p.trim());
    let joined = if path.is_absolute() { path } else { dir.join(path) };
    joined.canonicalize().unwrap_or(joined)
}

/// The repo's root folder, or an error when `dir` is not in a git repo.
pub fn toplevel(dir: &Path) -> Result<PathBuf> {
    let out = git(dir, &["rev-parse", "--show-toplevel"])?;
    if !out.ok {
        bail!("{} is not a git repository", dir.display());
    }
    Ok(abs(dir, &out.stdout))
}

/// True for a repo's main folder, false for a linked worktree.
pub fn is_main_worktree(dir: &Path) -> Result<bool> {
    let git_dir = git_ok(dir, &["rev-parse", "--git-dir"])?;
    let common = git_ok(dir, &["rev-parse", "--git-common-dir"])?;
    Ok(abs(dir, &git_dir) == abs(dir, &common))
}

/// The worktree's private git folder (HEAD and index live there).
pub fn git_dir(dir: &Path) -> Result<PathBuf> {
    Ok(abs(dir, &git_ok(dir, &["rev-parse", "--git-dir"])?))
}

pub fn has_origin(dir: &Path) -> bool {
    git(dir, &["remote"]).map(|o| o.stdout.lines().any(|l| l.trim() == "origin")).unwrap_or(false)
}

pub fn fetch_origin(dir: &Path) -> Result<()> {
    git_ok(dir, &["fetch", "origin", "--prune", "--quiet"]).map(|_| ())
}

fn refs(dir: &Path, prefix: &str) -> Vec<String> {
    git(dir, &["for-each-ref", "--format=%(refname)", prefix])
        .map(|o| {
            o.stdout
                .lines()
                .filter_map(|l| l.trim().strip_prefix(prefix).map(|s| s.trim_start_matches('/').to_string()))
                .filter(|b| !b.is_empty() && b != "HEAD")
                .collect()
        })
        .unwrap_or_default()
}

pub fn local_branches(dir: &Path) -> Vec<String> {
    refs(dir, "refs/heads")
}

/// Branches on origin, without the `origin/` prefix.
pub fn remote_branches(dir: &Path) -> Vec<String> {
    refs(dir, "refs/remotes/origin")
}

pub fn local_branch_exists(dir: &Path, branch: &str) -> bool {
    git(dir, &["show-ref", "--verify", "--quiet", &format!("refs/heads/{branch}")]).map(|o| o.ok).unwrap_or(false)
}

pub fn remote_branch_exists(dir: &Path, branch: &str) -> bool {
    git(dir, &["show-ref", "--verify", "--quiet", &format!("refs/remotes/origin/{branch}")])
        .map(|o| o.ok)
        .unwrap_or(false)
}

/// origin's default branch (from origin/HEAD), e.g. "main".
pub fn origin_default_branch(dir: &Path) -> Option<String> {
    let out = git(dir, &["symbolic-ref", "--quiet", "--short", "refs/remotes/origin/HEAD"]).ok()?;
    out.ok.then(|| out.stdout.trim().trim_start_matches("origin/").to_string()).filter(|s| !s.is_empty())
}

pub fn current_branch(dir: &Path) -> Option<String> {
    let out = git(dir, &["symbolic-ref", "--quiet", "--short", "HEAD"]).ok()?;
    out.ok.then(|| out.stdout.trim().to_string()).filter(|s| !s.is_empty())
}

/// The base new workspaces start from: the configured one, else development, else origin's default.
pub fn default_base(dir: &Path, configured: &str) -> Option<String> {
    let exists = |b: &str| local_branch_exists(dir, b) || remote_branch_exists(dir, b);
    let configured = configured.trim();
    if !configured.is_empty() && exists(configured) {
        return Some(configured.to_string());
    }
    if exists("development") {
        return Some("development".into());
    }
    origin_default_branch(dir)
        .filter(|b| exists(b))
        .or_else(|| ["main", "master"].into_iter().find(|b| exists(b)).map(String::from))
        .or_else(|| current_branch(dir))
}

pub fn valid_branch_name(dir: &Path, branch: &str) -> bool {
    !branch.trim().is_empty() && git(dir, &["check-ref-format", "--branch", branch]).map(|o| o.ok).unwrap_or(false)
}

/// New branch from `base` (origin's copy when there is one), without tracking the base.
pub fn worktree_add_new(main: &Path, path: &Path, branch: &str, base: &str) -> Result<String> {
    let start = if remote_branch_exists(main, base) { format!("origin/{base}") } else { base.to_string() };
    let path = path.to_string_lossy();
    git_ok(main, &["worktree", "add", "--no-track", "-b", branch, &path, &start])?;
    Ok(start)
}

/// Checks out an existing branch. A branch that only exists on origin gets a local tracking branch.
pub fn worktree_add_existing(main: &Path, path: &Path, branch: &str) -> Result<()> {
    let path = path.to_string_lossy();
    if local_branch_exists(main, branch) {
        git_ok(main, &["worktree", "add", &path, branch])?;
    } else if remote_branch_exists(main, branch) {
        let remote = format!("origin/{branch}");
        git_ok(main, &["worktree", "add", "--track", "-b", branch, &path, &remote])?;
    } else {
        bail!("There is no branch called \"{branch}\" here or on origin");
    }
    Ok(())
}

/// A new branch starting where `from` is now: its own tip (the folder it is checked out in may have commits it hasn't
/// pushed), else origin's copy. Without tracking, like every new branch.
pub fn worktree_add_from(main: &Path, path: &Path, branch: &str, from: &str) -> Result<String> {
    let start = if local_branch_exists(main, from) {
        from.to_string()
    } else if remote_branch_exists(main, from) {
        format!("origin/{from}")
    } else {
        bail!("There is no branch called \"{from}\" here or on origin");
    };
    git_ok(main, &["worktree", "add", "--no-track", "-b", branch, &path.to_string_lossy(), &start])?;
    Ok(start)
}

/// The folder that has `branch` checked out (the main folder or any worktree of the repo, Enkidu's or not), if
/// one has: git checks a branch out in one folder only.
pub fn checked_out_in(main: &Path, branch: &str) -> Option<PathBuf> {
    let out = git_ok(main, &["worktree", "list", "--porcelain", "-z"]).ok()?;
    let want = format!("branch refs/heads/{branch}");
    let mut folder = None;
    for field in out.split('\0') {
        if let Some(path) = field.strip_prefix("worktree ") {
            folder = Some(path);
        } else if field == want {
            return folder.map(|p| abs(main, p));
        }
    }
    None
}

/// True when `dir` is a checkout of the same repo as `main`: its main folder or one of its worktrees.
pub fn same_repo(main: &Path, dir: &Path) -> bool {
    let common = |d: &Path| git_ok(d, &["rev-parse", "--git-common-dir"]).ok().map(|c| abs(d, &c));
    matches!((common(main), common(dir)), (Some(a), Some(b)) if a == b)
}

pub fn worktree_remove(main: &Path, path: &Path, force: bool) -> Result<()> {
    let p = path.to_string_lossy();
    if path.exists() {
        let mut args = vec!["worktree", "remove"];
        if force {
            args.push("--force");
        }
        args.push(&p);
        git_ok(main, &args)?;
    }
    git_ok(main, &["worktree", "prune"])?;
    Ok(())
}

pub fn has_head(dir: &Path) -> bool {
    git(dir, &["rev-parse", "--verify", "--quiet", "HEAD"]).map(|o| o.ok).unwrap_or(false)
}

// Deserialize is the app's: it keeps each workspace's changes as last seen in a file (item 3.1).
#[derive(Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    pub path: String,
    pub old_path: Option<String>,
    /// modified, added, deleted, renamed, untracked or conflicted
    pub kind: String,
    pub staged: bool,
    pub additions: Option<u64>,
    pub deletions: Option<u64>,
    /// Changes whenever the file's diff may have changed (HEAD, size, modification time); the UI keys its
    /// diff cache on it.
    pub sig: String,
}

/// A merge that stopped before its commit (on a conflict, or `git merge --no-commit`). Git stages everything the merge
/// brings in from the other branch, so to git all of it is uncommitted: it can be 722 files when a main branch is
/// merged into a PR branch. None of that is your work. While a merge is on, the changes list compares each file with
/// what the merge makes of it on its own, so only the conflicts and what you (or Claude) change on top show; a file
/// with a conflict compares with HEAD, as without the merge.
#[derive(Clone, Debug)]
struct Merging {
    /// The tree `git merge-tree` writes for HEAD and MERGE_HEAD: both sides merged, with nothing of yours
    tree: String,
    /// The files the merge couldn't merge on its own, resolved since or not
    conflicted: HashSet<String>,
}

/// The merge on in this folder, if any. Worked out once per merge (HEAD and MERGE_HEAD don't move while it is on).
fn merging(dir: &Path) -> Option<Merging> {
    let theirs = git(dir, &["rev-parse", "-q", "--verify", "MERGE_HEAD"]).ok().filter(|o| o.ok)?.stdout.trim().to_string();
    // an octopus merge names more than one: it compares with HEAD as usual
    let file = git_ok(dir, &["rev-parse", "--git-path", "MERGE_HEAD"]).ok()?;
    let heads = std::fs::read_to_string(abs(dir, &file)).ok()?;
    if heads.lines().filter(|l| !l.trim().is_empty()).count() != 1 {
        return None;
    }
    let ours = git_ok(dir, &["rev-parse", "--verify", "HEAD"]).ok()?.trim().to_string();
    static MERGES: OnceLock<Mutex<HashMap<String, Merging>>> = OnceLock::new();
    let key = format!("{ours}:{theirs}");
    let cache = MERGES.get_or_init(Default::default);
    if let Some(known) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return Some(known.clone());
    }
    let out = git(dir, &["merge-tree", "--write-tree", "--name-only", "--no-messages", "-z", &ours, &theirs]).ok()?;
    // 0: merged on its own, 1: with conflicts; anything else is an error (a git older than 2.38)
    if !matches!(out.code, 0 | 1) {
        return None;
    }
    let mut parts = out.stdout.split('\0');
    let tree = parts.next()?.trim().to_string();
    if tree.len() < 40 || !tree.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let found = Merging { tree, conflicted: parts.filter(|p| !p.is_empty()).map(str::to_owned).collect() };
    let mut known = cache.lock().unwrap_or_else(|e| e.into_inner());
    if known.len() > 32 {
        known.clear();
    }
    known.insert(key, found.clone());
    Some(found)
}

/// What differs between a tree and the folder, as git's change letters: path → (kind, old path).
fn kinds_against(dir: &Path, tree: &str) -> Result<HashMap<String, (String, Option<String>)>> {
    // a submodule is a checkout of its own: its commit moving with the merge isn't work done here
    let raw = git_ok(dir, &["diff", tree, "--name-status", "-z", "-M", "--no-ext-diff", "--ignore-submodules=all"])?;
    let mut parts = raw.split('\0').filter(|s| !s.is_empty());
    let mut kinds = HashMap::new();
    while let Some(letter) = parts.next() {
        // a rename or a copy names its source first
        let first = letter.chars().next();
        let source = if matches!(first, Some('R' | 'C')) { parts.next().map(str::to_owned) } else { None };
        let Some(path) = parts.next() else { break };
        let kind = match first {
            Some('R') => "renamed",
            Some('A' | 'C') => "added",
            Some('D') => "deleted",
            Some('U') => "conflicted",
            _ => "modified",
        };
        // a copy's source is still there: only a rename carries the old path
        kinds.insert(path.to_string(), (kind.to_string(), if kind == "renamed" { source } else { None }));
    }
    Ok(kinds)
}

/// While a merge is on, the files that are your own work in it (see `Merging`): the conflicts and the files that
/// differ from what the merge makes of them, a rename by both its names. None when no merge is on.
pub fn own_work_in_merge(dir: &Path) -> Option<HashSet<String>> {
    let m = merging(dir)?;
    let kinds = kinds_against(dir, &m.tree).ok()?;
    let mut own = m.conflicted;
    for (path, (_, old)) in kinds {
        own.extend(old);
        own.insert(path);
    }
    Some(own)
}

/// What a file's diff compares with: HEAD (git's empty tree before the first commit); while a merge is on, what the
/// merge makes of the file on its own, unless the merge couldn't.
fn diff_base(dir: &Path, path: &str) -> String {
    match merging(dir) {
        Some(m) if !m.conflicted.contains(path) => m.tree,
        _ if has_head(dir) => "HEAD".into(),
        _ => EMPTY_TREE.into(),
    }
}

/// Uncommitted changes: staged + unstaged + untracked, compared with HEAD. While a merge is on, only your own work
/// (see `Merging`).
pub fn changes(dir: &Path) -> Result<Vec<Change>> {
    // enkidu: git is asked three things that don't depend on each other (what changed, how many lines, where HEAD and a
    // merge stand). On a big repo each of the first two walks the whole index (about 100 ms for 40,000 files), so
    // they are asked at the same time: a look takes as long as the slowest, not their sum (item 3.16).
    let (raw, stats, state) = std::thread::scope(|scope| {
        let stats = scope.spawn(|| numstat(dir, if has_head(dir) { "HEAD" } else { EMPTY_TREE }));
        let state = scope.spawn(|| {
            let head = git(dir, &["rev-parse", "--verify", "--quiet", "HEAD"])
                .ok()
                .filter(|o| o.ok)
                .map(|o| o.stdout.trim().chars().take(12).collect::<String>())
                .unwrap_or_default();
            (merging(dir), head)
        });
        let raw = git_ok(dir, &["status", "--porcelain=v1", "-z", "--untracked-files=all"]);
        (raw, stats.join().unwrap_or_default(), state.join().unwrap_or((None, String::new())))
    });
    let raw = raw?;
    let (merge, head) = state;
    let mut parts = raw.split('\0').filter(|s| !s.is_empty());
    let mut list = Vec::new();
    while let Some(entry) = parts.next() {
        if entry.len() < 4 {
            continue;
        }
        let (code, path) = entry.split_at(3);
        let x = code.as_bytes()[0] as char;
        let y = code.as_bytes()[1] as char;
        let old_path = if x == 'R' || x == 'C' { parts.next().map(String::from) } else { None };
        let kind = match (x, y) {
            ('?', '?') => "untracked",
            ('!', '!') => continue,
            ('D', 'D') | ('A', 'U') | ('U', _) | (_, 'U') | ('A', 'A') => "conflicted",
            (_, 'D') | ('D', _) => "deleted",
            ('R', _) => "renamed",
            ('A', _) | ('C', _) => "added",
            _ => "modified",
        };
        list.push(Change {
            path: path.to_string(),
            old_path,
            kind: kind.to_string(),
            staged: !matches!(x, ' ' | '?'),
            additions: None,
            deletions: None,
            sig: String::new(),
        });
    }
    if let Some(m) = &merge {
        // only what differs from the merge's own result, the conflicts, and new files
        let mut yours = kinds_against(dir, &m.tree)?;
        list.retain_mut(|c| {
            if c.kind == "untracked" || c.kind == "conflicted" || m.conflicted.contains(&c.path) {
                yours.remove(&c.path);
                return true;
            }
            match yours.remove(&c.path) {
                Some((kind, old)) => {
                    c.kind = kind;
                    c.old_path = old;
                    true
                }
                None => false,
            }
        });
        // a file the merge changed and you put back as HEAD had it: git status doesn't list it, but it is yours
        for (path, (kind, old_path)) in yours {
            list.push(Change { path, old_path, kind, staged: false, additions: None, deletions: None, sig: String::new() });
        }
    }
    let merged = merge.as_ref().map(|m| numstat(dir, &m.tree)).unwrap_or_default();
    // a diff's base moves when a merge starts or ends: so do the signatures
    let base = match &merge {
        Some(m) => format!("{head}+{}", &m.tree[..12]),
        None => head,
    };
    for change in &mut list {
        let against_merge = merge.as_ref().is_some_and(|m| !m.conflicted.contains(&change.path));
        if change.kind == "untracked" {
            if let Some(lines) = count_lines(&dir.join(&change.path)) {
                change.additions = Some(lines);
                change.deletions = Some(0);
            }
        } else if let Some((a, d)) = if against_merge { merged.get(&change.path) } else { stats.get(&change.path) } {
            change.additions = *a;
            change.deletions = *d;
        }
        change.sig = file_sig(&base, &dir.join(&change.path), change);
    }
    // as a tree of their folders lists them: the list, the Changed files tree and the stacked diffs run the same way
    list.sort_by(|a, b| crate::names::in_tree_order(&a.path, &b.path));
    Ok(list)
}

fn file_sig(head: &str, file: &Path, c: &Change) -> String {
    let stamp = file
        .symlink_metadata()
        .ok()
        .map(|m| {
            let mtime = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            format!("{mtime}.{}", m.len())
        })
        .unwrap_or_else(|| "gone".into());
    format!("{head}:{stamp}:{}:{}:{}", c.kind, c.additions.unwrap_or(0), c.deletions.unwrap_or(0))
}

/// Both versions of a changed file, for the side-by-side view: as committed (HEAD) and as it is now.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileVersions {
    pub old: Option<String>,
    pub new: Option<String>,
    pub binary: bool,
    pub too_large: bool,
}

enum Content {
    Text(String),
    Binary,
    TooLarge,
    Missing,
}

fn content(bytes: Vec<u8>) -> Content {
    if bytes.len() > MAX_DIFF_BYTES {
        Content::TooLarge
    } else if bytes.contains(&0) {
        Content::Binary
    } else {
        Content::Text(String::from_utf8_lossy(&bytes).into_owned())
    }
}

/// A file as `base` (a commit or tree) has it.
fn base_content(dir: &Path, base: &str, path: &str) -> Result<Content> {
    let spec = format!("{base}:{path}");
    let size = git(dir, &["cat-file", "-s", &spec])?;
    if !size.ok {
        return Ok(Content::Missing);
    }
    if size.stdout.trim().parse::<usize>().unwrap_or(0) > MAX_DIFF_BYTES {
        return Ok(Content::TooLarge);
    }
    // raw bytes: git's stdout is not always UTF-8
    let out = Command::new("git")
        .args(["cat-file", "blob", &spec])
        .current_dir(dir)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .map_err(|e| anyhow!("Can't run git: {e}"))?;
    Ok(if out.status.success() { content(out.stdout) } else { Content::Missing })
}

fn worktree_content(file: &Path) -> Content {
    match file.metadata() {
        Ok(m) if m.is_file() && m.len() as usize > MAX_DIFF_BYTES => Content::TooLarge,
        Ok(m) if m.is_file() => std::fs::read(file).map(content).unwrap_or(Content::Missing),
        _ => Content::Missing,
    }
}

pub fn file_versions(dir: &Path, path: &str, old_path: Option<&str>, kind: &str) -> Result<FileVersions> {
    let base = diff_base(dir, path);
    let old = if matches!(kind, "added" | "untracked") || base == EMPTY_TREE {
        Content::Missing
    } else {
        base_content(dir, &base, old_path.unwrap_or(path))?
    };
    let new = if kind == "deleted" { Content::Missing } else { worktree_content(&dir.join(path)) };
    let binary = matches!(old, Content::Binary) || matches!(new, Content::Binary);
    let too_large = matches!(old, Content::TooLarge) || matches!(new, Content::TooLarge);
    let text = |c: Content| match c {
        Content::Text(s) if !binary && !too_large => Some(s),
        _ => None,
    };
    Ok(FileVersions { old: text(old), new: text(new), binary, too_large })
}

/// A file the UI asked for, as a path inside the worktree. Refuses absolute paths, `..`, and symlinks that lead
/// out of the worktree, so the UI can only ever read the workspace's own files.
pub fn inside(dir: &Path, rel: &str) -> Result<PathBuf> {
    let path = Path::new(rel);
    if rel.is_empty() || !path.components().all(|c| matches!(c, Component::Normal(_))) {
        bail!("That isn't a file in this workspace");
    }
    let joined = dir.join(path);
    if let (Ok(real), Ok(root)) = (joined.canonicalize(), dir.canonicalize()) {
        if !real.starts_with(&root) {
            bail!("That isn't a file in this workspace");
        }
    }
    Ok(joined)
}

/// Most files the Files panel lists; past this the list is cut (and says so).
pub const MAX_LISTED_FILES: usize = 50_000;

/// The workspace's tracked files, straight from git's own index (Enkidu keeps no index of its own), sorted, without
/// submodules. Fast even on a huge repo: it reads the index and never walks the folders. New files that aren't
/// ignored come from the changes list the watcher already keeps (`git status` walks the folders once for both), and
/// files deleted but not committed are still in the index; the UI adds the first and hides the second.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileListing {
    /// Changes whenever the list does, so the UI can ask "anything new?" without the list coming back.
    pub version: String,
    pub files: Vec<String>,
    pub truncated: bool,
    /// The list is the one the UI already has (`files` is then empty).
    pub unchanged: bool,
}

/// The tracked files from git's index: sorted, without submodules.
pub fn tracked_files(dir: &Path) -> Result<Vec<String>> {
    let out = git_ok(dir, &["ls-files", "-z", "--cached"])?;
    let submodules = submodule_paths(dir);
    let mut files: Vec<String> = out
        .split('\0')
        .filter(|p| !p.is_empty() && !submodules.iter().any(|s| s == p))
        .map(str::to_owned)
        .collect();
    files.sort_unstable();
    files.dedup();
    Ok(files)
}

pub fn list_files(dir: &Path, known: Option<&str>, in_submodules: &[String]) -> Result<FileListing> {
    let mut files = tracked_files(dir)?;
    // the files inside the submodules, where they sort (item 10.7)
    if !in_submodules.is_empty() {
        files.extend(in_submodules.iter().cloned());
        files.sort_unstable();
        files.dedup();
    }
    let truncated = files.len() > MAX_LISTED_FILES;
    files.truncate(MAX_LISTED_FILES);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(&files, &mut hasher);
    let version = format!("{:016x}", std::hash::Hasher::finish(&hasher));
    if known == Some(version.as_str()) {
        return Ok(FileListing { version, files: Vec::new(), truncated, unchanged: true });
    }
    Ok(FileListing { version, files, truncated, unchanged: false })
}

/// The submodules that are checked out here: each one's folder from the workspace's root, and its own git index
/// file (which changes when what it tracks does). A submodule that was never fetched has no files to list.
pub fn submodules(dir: &Path) -> Vec<(String, PathBuf)> {
    submodule_paths(dir)
        .into_iter()
        .filter_map(|path| {
            let folder = dir.join(&path);
            if !folder.join(".git").exists() {
                return None;
            }
            let index = git_ok(&folder, &["rev-parse", "--git-path", "index"]).ok()?;
            let index = PathBuf::from(index.trim());
            Some((path, if index.is_absolute() { index } else { folder.join(index) }))
        })
        .collect()
}

/// The files git tracks in the submodule at `path` (a folder from the workspace's root), each from that root.
pub fn submodule_tracked(dir: &Path, path: &str) -> Vec<String> {
    listed_in(dir, path, &["ls-files", "-z", "--cached"])
}

/// The new files in that submodule: the ones its own git doesn't track and doesn't ignore.
pub fn submodule_new(dir: &Path, path: &str) -> Vec<String> {
    listed_in(dir, path, &["ls-files", "-z", "--others", "--exclude-standard"])
}

fn listed_in(dir: &Path, path: &str, args: &[&str]) -> Vec<String> {
    let Ok(out) = git_ok(&dir.join(path), args) else {
        return Vec::new();
    };
    out.split('\0').filter(|p| !p.is_empty()).map(|p| format!("{path}/{p}")).collect()
}

/// Submodule folders (from .gitmodules): another repo's files, not this one's.
fn submodule_paths(dir: &Path) -> Vec<String> {
    if !dir.join(".gitmodules").is_file() {
        return Vec::new();
    }
    git(dir, &["config", "-f", ".gitmodules", "--get-regexp", r"^submodule\..*\.path$"])
        .map(|o| o.stdout.lines().filter_map(|l| l.split_once(' ').map(|(_, p)| p.trim().to_owned())).collect())
        .unwrap_or_default()
}

/// One file as it is on disk, for the read-only view.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FilePreview {
    pub text: Option<String>,
    pub binary: bool,
    pub too_large: bool,
    pub missing: bool,
    pub size: u64,
}

pub fn read_file(dir: &Path, rel: &str) -> Result<FilePreview> {
    let file = inside(dir, rel)?;
    let size = file.metadata().map(|m| m.len()).unwrap_or(0);
    let mut preview = FilePreview { text: None, binary: false, too_large: false, missing: false, size };
    match worktree_content(&file) {
        Content::Text(text) => preview.text = Some(text),
        Content::Binary => preview.binary = true,
        Content::TooLarge => preview.too_large = true,
        Content::Missing => preview.missing = true,
    }
    Ok(preview)
}

/// Lines added and removed per file, from `base` to the folder.
fn numstat(dir: &Path, base: &str) -> HashMap<String, (Option<u64>, Option<u64>)> {
    let Ok(raw) = git_ok(dir, &["diff", base, "--numstat", "-z", "-M", "--no-ext-diff", "--no-color"]) else {
        return HashMap::new();
    };
    let mut map = HashMap::new();
    let mut parts = raw.split('\0');
    while let Some(entry) = parts.next() {
        let mut fields = entry.splitn(3, '\t');
        let (Some(a), Some(d), Some(path)) = (fields.next(), fields.next(), fields.next()) else {
            continue;
        };
        let path = if path.is_empty() {
            // rename: the old and new paths follow as their own fields
            let _old = parts.next();
            parts.next().unwrap_or_default().to_string()
        } else {
            path.to_string()
        };
        map.insert(path, (a.parse().ok(), d.parse().ok()));
    }
    map
}

fn count_lines(path: &Path) -> Option<u64> {
    let meta = path.metadata().ok()?;
    if !meta.is_file() || meta.len() > 2_000_000 {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    if bytes.contains(&0) {
        return None;
    }
    let lines = bytes.iter().filter(|&&b| b == b'\n').count() as u64;
    Some(if bytes.last().is_some_and(|&b| b != b'\n') { lines + 1 } else { lines })
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FileDiff {
    pub path: String,
    pub binary: bool,
    pub too_large: bool,
    pub diff: String,
}

/// The diff of one file against HEAD (including staged and untracked work); while a merge is on, against what the
/// merge makes of it on its own (see `Merging`).
pub fn file_diff(dir: &Path, path: &str, old_path: Option<&str>, untracked: bool) -> Result<FileDiff> {
    file_diff_with(dir, path, old_path, untracked, None)
}

/// The same diff with every line of the file in it, the unchanged ones as context (new in Enkidu: its diff
/// view folds the unchanged lines itself and unfolds them on a click, so it needs them all). One hunk for the file.
pub fn file_patch(dir: &Path, path: &str, old_path: Option<&str>, untracked: bool) -> Result<FileDiff> {
    file_diff_with(dir, path, old_path, untracked, Some(WHOLE_FILE))
}

/// More context lines than a file that is shown can have (`MAX_DIFF_BYTES` holds fewer lines than this).
const WHOLE_FILE: usize = 10_000_000;

fn file_diff_with(dir: &Path, path: &str, old_path: Option<&str>, untracked: bool, context: Option<usize>) -> Result<FileDiff> {
    let unified = context.map(|lines| format!("--unified={lines}"));
    let out = if untracked {
        let mut args = vec!["diff", "--no-index", "--no-color", "--no-ext-diff"];
        args.extend(unified.as_deref());
        args.extend(["--", "/dev/null", path]);
        git(dir, &args)?
    } else {
        let base = diff_base(dir, path);
        let mut args = vec!["diff", base.as_str(), "--no-color", "--no-ext-diff", "-M"];
        args.extend(unified.as_deref());
        args.push("--");
        if let Some(old) = old_path {
            args.push(old);
        }
        args.push(path);
        git(dir, &args)?
    };
    // `git diff --no-index` exits with 1 when the files differ
    if !out.ok && !(untracked && out.code == 1) {
        bail!("{}", redact_urls(out.stderr.trim()));
    }
    // a patch with the whole file in it can hold both versions of a file that is itself within the limit
    let too_large = out.stdout.len() > if context.is_some() { 2 * MAX_DIFF_BYTES } else { MAX_DIFF_BYTES };
    let binary = out.stdout.lines().any(|l| l.starts_with("Binary files ") && l.ends_with(" differ"));
    Ok(FileDiff {
        path: path.to_string(),
        binary,
        too_large,
        diff: if too_large { String::new() } else { out.stdout },
    })
}

/// True when the folder has uncommitted or untracked work.
pub fn is_dirty(dir: &Path) -> Result<bool> {
    Ok(!git_ok(dir, &["status", "--porcelain", "--untracked-files=all"])?.trim().is_empty())
}

#[cfg(test)]
pub mod testutil {
    use super::*;

    pub fn run(dir: &Path, args: &[&str]) {
        let out = git(dir, args).unwrap();
        assert!(out.ok, "git {:?} failed: {}", args, out.stderr);
    }

    /// A bare "origin" plus a clone of it with main + development (+ feature/remote-only on origin).
    pub fn repo_with_origin(root: &Path) -> (PathBuf, PathBuf) {
        let origin = root.join("origin.git");
        let seed = root.join("seed");
        let main = root.join("main");
        std::fs::create_dir_all(&seed).unwrap();
        run(root, &["init", "--bare", "-b", "main", origin.to_str().unwrap()]);
        run(&seed, &["init", "-b", "main"]);
        run(&seed, &["config", "user.email", "t@example.com"]);
        run(&seed, &["config", "user.name", "Test"]);
        std::fs::write(seed.join("README.md"), "hello\n").unwrap();
        std::fs::write(seed.join(".gitignore"), ".env\n").unwrap();
        run(&seed, &["add", "."]);
        run(&seed, &["commit", "-m", "first"]);
        run(&seed, &["branch", "development"]);
        run(&seed, &["branch", "feature/remote-only"]);
        run(&seed, &["remote", "add", "origin", origin.to_str().unwrap()]);
        run(&seed, &["push", "-q", "origin", "main", "development", "feature/remote-only"]);
        run(root, &["clone", "-q", origin.to_str().unwrap(), main.to_str().unwrap()]);
        run(&main, &["config", "user.email", "t@example.com"]);
        run(&main, &["config", "user.name", "Test"]);
        (origin, main.canonicalize().unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::*;
    use super::*;

    /// New in Enkidu: the diff view folds unchanged lines itself, so it is given every line of the file.
    #[test]
    fn a_file_patch_has_every_line_of_the_file_in_one_hunk() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        run(dir, &["init", "-q", "-b", "main"]);
        run(dir, &["config", "user.email", "t@example.com"]);
        run(dir, &["config", "user.name", "Test"]);
        let lines = |changed: &[usize]| -> String {
            (1..=60).map(|n| if changed.contains(&n) { format!("changed {n}\n") } else { format!("line {n}\n") }).collect()
        };
        std::fs::write(dir.join("stock.py"), lines(&[])).unwrap();
        std::fs::write(dir.join("old-name.txt"), "a\nb\nc\nd\ne\nf\ng\nh\n").unwrap();
        run(dir, &["add", "."]);
        run(dir, &["commit", "-q", "-m", "first"]);
        // two changes far apart: git's own diff has two hunks with three lines around each
        std::fs::write(dir.join("stock.py"), lines(&[5, 50])).unwrap();
        let usual = file_diff(dir, "stock.py", None, false).unwrap().diff;
        assert_eq!(usual.matches("\n@@ ").count(), 2, "{usual}");
        assert!(!usual.contains(" line 30\n"));
        // the patch has the whole file in one hunk, the same two changes in it
        let whole = file_patch(dir, "stock.py", None, false).unwrap();
        assert!(!whole.binary && !whole.too_large);
        assert_eq!(whole.diff.matches("\n@@ ").count(), 1, "{}", whole.diff);
        assert!(whole.diff.contains("\n@@ -1,60 +1,60 @@\n"), "{}", whole.diff);
        for n in (1..=60).filter(|n| ![5, 50].contains(n)) {
            assert!(whole.diff.contains(&format!("\n line {n}\n")), "line {n}");
        }
        for n in [5, 50] {
            assert!(whole.diff.contains(&format!("\n-line {n}\n+changed {n}\n")), "{}", whole.diff);
        }
        // a new file is whole either way, and a renamed one names both paths
        std::fs::write(dir.join("new.txt"), "one\ntwo\n").unwrap();
        let new = file_patch(dir, "new.txt", None, true).unwrap().diff;
        assert!(new.contains("\n@@ -0,0 +1,2 @@\n+one\n+two\n"), "{new}");
        run(dir, &["mv", "old-name.txt", "new-name.txt"]);
        std::fs::write(dir.join("new-name.txt"), "a\nb\nc\nd\ne\nf\ng\nH\n").unwrap();
        let renamed = file_patch(dir, "new-name.txt", Some("old-name.txt"), false).unwrap().diff;
        assert!(renamed.contains("rename from old-name.txt\nrename to new-name.txt\n"), "{renamed}");
        assert!(renamed.contains("\n a\n b\n c\n d\n e\n f\n g\n-h\n+H\n"), "{renamed}");
    }

    #[test]
    fn reads_branches_and_default_base() {
        let tmp = tempfile::tempdir().unwrap();
        let (_origin, main) = repo_with_origin(tmp.path());
        assert!(is_main_worktree(&main).unwrap());
        assert_eq!(toplevel(&main.join(".")).unwrap(), main);
        assert!(has_origin(&main));
        fetch_origin(&main).unwrap();
        assert_eq!(local_branches(&main), ["main"]);
        let mut remote = remote_branches(&main);
        remote.sort();
        assert_eq!(remote, ["development", "feature/remote-only", "main"]);
        assert_eq!(default_base(&main, "").as_deref(), Some("development"));
        assert_eq!(default_base(&main, "main").as_deref(), Some("main"));
        assert_eq!(default_base(&main, "nope").as_deref(), Some("development"));
        assert!(valid_branch_name(&main, "feature/ok-1"));
        assert!(!valid_branch_name(&main, "bad..name"));
        assert!(!valid_branch_name(&main, ""));
    }

    #[test]
    fn creates_worktrees_on_new_and_existing_branches() {
        let tmp = tempfile::tempdir().unwrap();
        let (_origin, main) = repo_with_origin(tmp.path());
        let ws_root = tmp.path().join("ws");

        let new_path = ws_root.join("one");
        let start = worktree_add_new(&main, &new_path, "feature/one", "development").unwrap();
        assert_eq!(start, "origin/development");
        assert_eq!(current_branch(&new_path).as_deref(), Some("feature/one"));
        assert!(!is_main_worktree(&new_path).unwrap());
        // no upstream: pushing must not go to development by accident
        assert!(!git(&new_path, &["rev-parse", "--abbrev-ref", "@{upstream}"]).unwrap().ok);

        let remote_path = ws_root.join("two");
        worktree_add_existing(&main, &remote_path, "feature/remote-only").unwrap();
        assert_eq!(current_branch(&remote_path).as_deref(), Some("feature/remote-only"));
        let upstream = git_ok(&remote_path, &["rev-parse", "--abbrev-ref", "@{upstream}"]).unwrap();
        assert_eq!(upstream.trim(), "origin/feature/remote-only");

        let err = worktree_add_existing(&main, &ws_root.join("three"), "main").unwrap_err();
        assert!(err.to_string().contains("already"), "main is checked out in the main folder: {err}");
        assert!(worktree_add_existing(&main, &ws_root.join("four"), "no-such-branch").is_err());

        // where each branch is checked out, so the UI can offer that folder
        assert_eq!(checked_out_in(&main, "main"), Some(main.clone()));
        assert_eq!(checked_out_in(&main, "feature/one"), Some(new_path.canonicalize().unwrap()));
        assert_eq!(checked_out_in(&main, "development"), None);
        assert!(same_repo(&main, &new_path) && same_repo(&new_path, &main));
        assert!(!same_repo(&main, tmp.path()));
        // a new branch where another one is: from its own tip, with a commit origin doesn't have
        std::fs::write(new_path.join("unpushed.txt"), "u\n").unwrap();
        run(&new_path, &["add", "unpushed.txt"]);
        run(&new_path, &["-c", "user.email=t@example.com", "-c", "user.name=T", "commit", "-qm", "not pushed"]);
        let copy = ws_root.join("one-again");
        assert_eq!(worktree_add_from(&main, &copy, "feature/one-2", "feature/one").unwrap(), "feature/one");
        assert_eq!(current_branch(&copy).as_deref(), Some("feature/one-2"));
        assert!(copy.join("unpushed.txt").exists(), "the commit came along");
        assert!(!git(&copy, &["rev-parse", "--abbrev-ref", "@{upstream}"]).unwrap().ok, "no upstream");
        // a branch only origin has (the clone made no local development) starts from origin's copy
        assert_eq!(worktree_add_from(&main, &ws_root.join("five"), "development-2", "development").unwrap(), "origin/development");
        assert!(worktree_add_from(&main, &ws_root.join("six"), "x-2", "no-such-branch").is_err());
        worktree_remove(&main, &copy, true).unwrap();
        worktree_remove(&main, &ws_root.join("five"), true).unwrap();

        std::fs::write(new_path.join("scratch.txt"), "x").unwrap();
        assert!(is_dirty(&new_path).unwrap());
        assert!(worktree_remove(&main, &new_path, false).is_err(), "dirty worktrees need force");
        worktree_remove(&main, &new_path, true).unwrap();
        assert!(!new_path.exists());
        assert!(local_branch_exists(&main, "feature/one"), "removing keeps the branch");
    }

    #[test]
    fn lists_the_files_git_sees_and_reads_one_safely() {
        let tmp = tempfile::tempdir().unwrap();
        let (_origin, main) = repo_with_origin(tmp.path());
        std::fs::create_dir_all(main.join("src/orders")).unwrap();
        std::fs::write(main.join("src/orders/stock.py"), "x = 1\n").unwrap();
        std::fs::write(main.join(".env"), "SECRET=1\n").unwrap();
        std::fs::write(main.join("blob.bin"), [0u8, 1, 2]).unwrap();
        run(&main, &["add", "src/orders/stock.py", "blob.bin"]);
        let listing = list_files(&main, None, &[]).unwrap();
        // the tracked files from the index, sorted (the ignored .env is never there)
        assert_eq!(listing.files, [".gitignore", "README.md", "blob.bin", "src/orders/stock.py"]);
        assert!(!listing.truncated && !listing.unchanged);
        // asking again with the version it has: nothing comes back
        let again = list_files(&main, Some(&listing.version), &[]).unwrap();
        assert!(again.unchanged && again.files.is_empty());
        assert_eq!(again.version, listing.version);
        // a new, untracked file comes from the changes list, not from here
        std::fs::write(main.join("notes.md"), "n\n").unwrap();
        assert!(list_files(&main, Some(&listing.version), &[]).unwrap().unchanged);
        // once added, the index changed, and so does the version
        run(&main, &["add", "notes.md"]);
        let grown = list_files(&main, Some(&listing.version), &[]).unwrap();
        assert!(!grown.unchanged && grown.files.contains(&"notes.md".to_string()));
        // a submodule's folder is not this repo's files
        std::fs::write(main.join(".gitmodules"), "[submodule \"spa\"]\n\tpath = spa\n\turl = ../spa.git\n").unwrap();
        std::fs::create_dir_all(main.join("spa")).unwrap();
        // a real gitlink, the way a submodule sits in the index
        run(&main, &["update-index", "--add", "--cacheinfo", "160000,0123456789abcdef0123456789abcdef01234567,spa"]);
        assert!(git_ok(&main, &["ls-files", "--cached"]).unwrap().lines().any(|l| l == "spa"), "git lists the submodule");
        assert!(!list_files(&main, None, &[]).unwrap().files.iter().any(|f| f == "spa"), "the panel doesn't");

        let p = read_file(&main, "src/orders/stock.py").unwrap();
        assert_eq!(p.text.as_deref(), Some("x = 1\n"));
        assert_eq!(p.size, 6);
        assert!(read_file(&main, "blob.bin").unwrap().binary);
        assert!(read_file(&main, "nope.txt").unwrap().missing);
        // only files inside the workspace
        for bad in ["../outside.txt", "/etc/hosts", "src/../../x", "", "./README.md"] {
            assert!(read_file(&main, bad).is_err(), "{bad} should be refused");
        }
        std::fs::write(tmp.path().join("secret.txt"), "s").unwrap();
        std::os::unix::fs::symlink(tmp.path().join("secret.txt"), main.join("link.txt")).unwrap();
        assert!(read_file(&main, "link.txt").is_err(), "a symlink out of the workspace is refused");
    }

    /// Run on a big repo: ENKIDU_BIG_REPO=<path> cargo test --release list_files_speed -- --ignored --nocapture
    #[test]
    #[ignore]
    fn list_files_speed() {
        let dir = PathBuf::from(std::env::var("ENKIDU_BIG_REPO").expect("set ENKIDU_BIG_REPO"));
        let mut first = Vec::new();
        let mut again = Vec::new();
        let mut count = 0;
        for _ in 0..5 {
            let t = std::time::Instant::now();
            let listing = list_files(&dir, None, &[]).unwrap();
            first.push(t.elapsed().as_millis());
            count = listing.files.len();
            let t = std::time::Instant::now();
            assert!(list_files(&dir, Some(&listing.version), &[]).unwrap().unchanged);
            again.push(t.elapsed().as_millis());
        }
        println!("ENKIDU_LIST files={count} full={first:?}ms unchanged={again:?}ms");
    }

    /// How long one look at a workspace's changes takes (what the watcher does after every edit). Run on a big repo
    /// with many changed files: ENKIDU_BIG_REPO=<path> cargo test --release changes_speed -- --ignored --nocapture
    #[test]
    #[ignore]
    fn changes_speed() {
        let dir = PathBuf::from(std::env::var("ENKIDU_BIG_REPO").expect("set ENKIDU_BIG_REPO"));
        let mut looks = Vec::new();
        let mut count = 0;
        for _ in 0..5 {
            let t = std::time::Instant::now();
            count = changes(&dir).unwrap().len();
            looks.push(t.elapsed().as_millis());
        }
        let t = std::time::Instant::now();
        let merge = merging(&dir).is_some();
        let merging_ms = t.elapsed().as_millis();
        println!("ENKIDU_CHANGES changed={count} look={looks:?}ms (of it merging()={merging_ms}ms, a merge is on: {merge})");
    }

    #[test]
    fn lists_uncommitted_changes_and_diffs() {
        let tmp = tempfile::tempdir().unwrap();
        let (_origin, main) = repo_with_origin(tmp.path());
        std::fs::write(main.join("README.md"), "hello\nworld\n").unwrap();
        std::fs::write(main.join("new.txt"), "a\nb\nc").unwrap();
        std::fs::write(main.join("staged.txt"), "s\n").unwrap();
        run(&main, &["add", "staged.txt"]);
        std::fs::write(main.join(".env"), "SECRET=1\n").unwrap();
        run(&main, &["mv", ".gitignore", "ignore.txt"]);

        let list = changes(&main).unwrap();
        let by_path: HashMap<_, _> = list.iter().map(|c| (c.path.as_str(), c)).collect();
        assert_eq!(by_path["README.md"].kind, "modified");
        assert_eq!(by_path["README.md"].additions, Some(1));
        assert_eq!(by_path["new.txt"].kind, "untracked");
        assert_eq!(by_path["new.txt"].additions, Some(3));
        assert_eq!(by_path["staged.txt"].kind, "added");
        assert!(by_path["staged.txt"].staged);
        assert_eq!(by_path["ignore.txt"].kind, "renamed");
        assert_eq!(by_path["ignore.txt"].old_path.as_deref(), Some(".gitignore"));
        // .gitignore was renamed away, so .env is no longer ignored
        assert_eq!(by_path[".env"].kind, "untracked");

        let d = file_diff(&main, "README.md", None, false).unwrap();
        assert!(d.diff.contains("+world"));
        let u = file_diff(&main, "new.txt", None, true).unwrap();
        assert!(u.diff.contains("+c"));
        assert!(!u.binary);
        std::fs::write(main.join("bin.dat"), [0u8, 1, 2, 3]).unwrap();
        let b = file_diff(&main, "bin.dat", None, true).unwrap();
        assert!(b.binary);
        assert!(file_patch(&main, "bin.dat", None, true).unwrap().binary);

        // Both versions for the side-by-side view.
        let v = file_versions(&main, "README.md", None, "modified").unwrap();
        assert_eq!(v.old.as_deref(), Some("hello\n"));
        assert_eq!(v.new.as_deref(), Some("hello\nworld\n"));
        let n = file_versions(&main, "new.txt", None, "untracked").unwrap();
        assert_eq!((n.old, n.new.as_deref()), (None, Some("a\nb\nc")));
        let r = file_versions(&main, "ignore.txt", Some(".gitignore"), "renamed").unwrap();
        assert!(r.old.is_some(), "a renamed file shows its old content");
        assert!(file_versions(&main, "bin.dat", None, "untracked").unwrap().binary);

        // The signature moves when the file does, and only then.
        let sig = |list: &[Change]| list.iter().find(|c| c.path == "README.md").unwrap().sig.clone();
        let before = sig(&changes(&main).unwrap());
        assert_eq!(before, sig(&changes(&main).unwrap()));
        std::fs::write(main.join("README.md"), "hello\nworld\nagain\n").unwrap();
        assert_ne!(before, sig(&changes(&main).unwrap()));
    }

    #[test]
    fn works_in_a_repo_without_commits() {
        let tmp = tempfile::tempdir().unwrap();
        run(tmp.path(), &["init", "-q", "-b", "main"]);
        std::fs::write(tmp.path().join("a.txt"), "one\n").unwrap();
        run(tmp.path(), &["add", "a.txt"]);
        let list = changes(tmp.path()).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].kind, "added");
        assert_eq!(list[0].additions, Some(1));
        assert!(file_diff(tmp.path(), "a.txt", None, false).unwrap().diff.contains("+one"));
    }

    /// A feature branch, and a development that moved on: a conflict in orders.py, a change in stock.py, a new
    /// file and a deleted one.
    fn diverged(dir: &Path) {
        run(dir, &["init", "-q", "-b", "feature"]);
        run(dir, &["config", "user.email", "t@example.com"]);
        run(dir, &["config", "user.name", "Test"]);
        std::fs::write(dir.join("orders.py"), "a\nb\nc\n").unwrap();
        std::fs::write(dir.join("stock.py"), "x\n").unwrap();
        std::fs::write(dir.join("gone.py"), "old\n").unwrap();
        run(dir, &["add", "."]);
        run(dir, &["commit", "-qm", "base"]);
        run(dir, &["checkout", "-qb", "development"]);
        std::fs::write(dir.join("orders.py"), "a\nB from development\nc\n").unwrap();
        std::fs::write(dir.join("stock.py"), "x\ny from development\n").unwrap();
        std::fs::write(dir.join("reports.py"), "new\n").unwrap();
        run(dir, &["rm", "-q", "gone.py"]);
        run(dir, &["add", "."]);
        run(dir, &["commit", "-qm", "development"]);
        run(dir, &["checkout", "-q", "feature"]);
        std::fs::write(dir.join("orders.py"), "a\nB from the feature\nc\n").unwrap();
        run(dir, &["commit", "-qam", "feature"]);
    }

    #[test]
    fn during_a_merge_only_your_own_work_shows() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        diverged(dir);
        assert!(!git(dir, &["merge", "development"]).unwrap().ok, "the merge stops on the conflict");
        // to git, everything development brings is uncommitted
        assert_eq!(git_ok(dir, &["status", "--porcelain"]).unwrap().lines().count(), 4);

        // the conflict only, against HEAD: its markers show
        let list = changes(dir).unwrap();
        let shown: Vec<_> = list.iter().map(|c| (c.path.as_str(), c.kind.as_str())).collect();
        assert_eq!(shown, [("orders.py", "conflicted")]);
        assert!(file_diff(dir, "orders.py", None, false).unwrap().diff.contains("+<<<<<<<"));

        // resolved, and edits of your own: each against what the merge made of the file
        std::fs::write(dir.join("orders.py"), "a\nB from both\nc\n").unwrap();
        run(dir, &["add", "orders.py"]);
        std::fs::write(dir.join("stock.py"), "x\ny from development\nz from you\n").unwrap();
        std::fs::write(dir.join("notes.md"), "n\n").unwrap();
        let list = changes(dir).unwrap();
        let by_path: HashMap<_, _> = list.iter().map(|c| (c.path.as_str(), c)).collect();
        assert_eq!(by_path.len(), 3, "{list:?}");
        assert_eq!(by_path["notes.md"].kind, "untracked");
        let stock = by_path["stock.py"];
        assert_eq!((stock.kind.as_str(), stock.additions, stock.deletions), ("modified", Some(1), Some(0)));
        let d = file_diff(dir, "stock.py", None, false).unwrap().diff;
        assert!(d.contains("+z from you") && !d.contains("+y from development"), "{d}");
        let v = file_versions(dir, "stock.py", None, "modified").unwrap();
        assert_eq!(v.old.as_deref(), Some("x\ny from development\n"));
        // the resolved conflict still compares with HEAD: your side, then what became of it
        assert_eq!(by_path["orders.py"].kind, "modified");
        let r = file_versions(dir, "orders.py", None, "modified").unwrap();
        assert_eq!((r.old.as_deref(), r.new.as_deref()), (Some("a\nB from the feature\nc\n"), Some("a\nB from both\nc\n")));

        // development's change put back as HEAD had it: git status no longer lists the file, but that is your doing
        run(dir, &["checkout", "HEAD", "--", "stock.py"]);
        let list = changes(dir).unwrap();
        assert!(list.iter().any(|c| c.path == "stock.py" && c.kind == "modified" && c.deletions == Some(1)), "{list:?}");
        run(dir, &["checkout", "MERGE_HEAD", "--", "stock.py"]);

        // committed: HEAD again, and nothing left
        std::fs::remove_file(dir.join("notes.md")).unwrap();
        run(dir, &["commit", "-qm", "merge development"]);
        assert!(changes(dir).unwrap().is_empty());
        assert!(file_diff(dir, "orders.py", None, false).unwrap().diff.is_empty());
    }

    #[test]
    fn a_merge_that_went_through_on_its_own_shows_nothing_until_you_change_something() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        diverged(dir);
        // no conflict this time: the feature leaves orders.py as it was
        run(dir, &["reset", "-q", "--hard", "HEAD~1"]);
        run(dir, &["merge", "-q", "--no-commit", "--no-ff", "development"]);
        assert_eq!(git_ok(dir, &["status", "--porcelain"]).unwrap().lines().count(), 4, "staged, waiting for the commit");
        assert!(changes(dir).unwrap().is_empty());
        std::fs::write(dir.join("reports.py"), "new\nmine\n").unwrap();
        let list = changes(dir).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!((list[0].path.as_str(), list[0].kind.as_str(), list[0].additions), ("reports.py", "modified", Some(1)));
        // the merge called off: no merge, nothing changed
        run(dir, &["reset", "-q", "--hard"]);
        assert!(changes(dir).unwrap().is_empty());
    }
}
