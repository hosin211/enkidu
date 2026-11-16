//! Live: one workspace per project runs in the project's dev environment, and the dev environment runs from the
//! project's main folder. Going Live copies the workspace's changes into the main folder: the files in its changes
//! list (what `git status` shows: modified, new and deleted files), never ignored files, submodules, or the files a new
//! workspace copies from the main folder (`.env` and the like). While it is Live, each new change is copied as it
//! happens (the workspace watcher calls `follow`); a file that stops being a change (Claude committed it, or it was put
//! back) goes back to the main folder's own version. Stop puts every one of those files back the way the main folder
//! had it. The project's "on" command runs after the copy and "off" after putting the files back (to restart a server).
//!
//! Nothing in the main folder is lost. Before Live first writes over a file, the main folder's own version is saved
//! and Stop brings it back; a file changed in the main folder while Live is set aside in `kept/` before Live touches it
//! again. What Live did is on disk (`live/<project>/` in the app's data folder), so a restart never loses track of it.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use crate::events::{Event, Events};

use crate::db::Workspace;
use crate::git;
use crate::script;
use crate::shellenv::shell_quote;
use crate::state::Shared;
use crate::workspace::script_env;

/// One copy at a time: going Live, stopping, and following a change never write the main folder together.
static COPYING: Mutex<()> = Mutex::new(());

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LiveOutcome {
    pub project_id: String,
    pub live_workspace_id: Option<String>,
    /// What happened to the main folder, in a sentence or two
    pub note: String,
    /// What the project's on and off commands printed
    pub output: String,
}

/// Fills `{path}`, `{name}`, `{branch}` and `{main}` in a command, shell-quoted.
pub fn expand(command: &str, main: &Path, ws: &Workspace) -> String {
    command
        .replace("{path}", &shell_quote(&ws.path))
        .replace("{name}", &shell_quote(&ws.name))
        .replace("{branch}", &shell_quote(&ws.branch))
        .replace("{main}", &shell_quote(&main.to_string_lossy()))
}

fn run_step(label: &str, command: &str, main: &Path, ws: &Workspace, out: &mut String) -> Result<()> {
    if command.trim().is_empty() {
        return Ok(());
    }
    let result = script::run(
        &expand(command, main, ws),
        main,
        &script_env(main, Path::new(&ws.path), &ws.name, &ws.branch),
        Duration::from_secs(10 * 60),
        None,
    );
    out.push_str(&format!("$ {label} ({})\n{}\n", ws.name, result.output.trim_end()));
    if !result.ok {
        let why = if result.timed_out { "timed out".to_string() } else { format!("exited with {:?}", result.code) };
        bail!("The Live {label} command {why}");
    }
    Ok(())
}

pub fn set_live(app: &Events, shared: &Shared, project_id: &str, workspace_id: Option<&str>) -> Result<LiveOutcome> {
    let project = shared.db.project(project_id)?.ok_or_else(|| anyhow!("That project is gone"))?;
    let settings = shared.project_settings(project_id)?;
    let main = PathBuf::from(&project.path);
    let store = Store::new(&shared.data_dir, project_id);
    let current = project.live_workspace_id.clone();
    let mut output = String::new();
    let mut notes = Vec::new();

    if current.as_deref() == workspace_id {
        return Ok(LiveOutcome { project_id: project_id.into(), live_workspace_id: current, note: String::new(), output });
    }
    let target = match workspace_id {
        Some(id) => {
            let ws = shared.db.workspace(id)?.ok_or_else(|| anyhow!("That workspace is gone"))?;
            if ws.project_id != project_id {
                bail!("That workspace belongs to another project");
            }
            // the main folder opened as a workspace: Live would copy it onto itself
            let same = |p: &str| PathBuf::from(p).canonicalize().unwrap_or_else(|_| PathBuf::from(p));
            if same(&ws.path) == same(&project.path) {
                bail!("{} is the main folder itself, so there is nothing to copy into it", ws.name);
            }
            Some(ws)
        }
        None => None,
    };

    let finish = |live: Option<String>, note: String, output: String| -> Result<LiveOutcome> {
        shared.db.set_live(project_id, live.as_deref())?;
        let outcome = LiveOutcome { project_id: project_id.into(), live_workspace_id: live, note, output };
        app.send(Event::LiveChanged(outcome.clone()));
        Ok(outcome)
    };

    // the main folder goes back first, then the old workspace's "off" command
    if current.is_some() {
        let back = put_back(&store, &main).map_err(|e| anyhow!("The main folder couldn't be put back: {e:#}"))?;
        if let Some(text) = back.describe(&main, &store) {
            notes.push(text);
        }
        // the copy is gone, so nothing is Live anymore, whatever "off" does
        shared.db.set_live(project_id, None)?;
        if let Some(old) = current.as_deref().and_then(|id| shared.db.workspace(id).ok().flatten()) {
            if let Err(e) = run_step("off", &settings.live.off, &main, &old, &mut output) {
                let _ = finish(None, notes.join(" "), output.clone());
                return Err(anyhow!("{e}\n{output}"));
            }
        }
    }
    let Some(ws) = target else {
        return finish(None, notes.join(" "), output);
    };

    let copied = copy_in(&store, &main, &ws, &settings.copy_files).map_err(|e| {
        // whatever got copied goes back, so the main folder is never half one workspace
        let _ = put_back(&store, &main);
        anyhow!("{}'s changes couldn't be copied into the main folder: {e:#}", ws.name)
    })?;
    notes.push(copied.describe_copy(&main));
    if let Some(text) = copied.describe_kept(&store) {
        notes.push(text);
    }
    // Live from here, so a change made while the "on" command runs is followed too
    shared.db.set_live(project_id, Some(&ws.id))?;
    match run_step("on", &settings.live.on, &main, &ws, &mut output) {
        Ok(()) => finish(Some(ws.id.clone()), notes.join(" "), output),
        Err(e) => {
            // the dev environment didn't take it: the main folder goes back and nothing is Live
            let _ = put_back(&store, &main);
            let _ = finish(None, String::new(), output.clone());
            Err(anyhow!("{e}\n{output}"))
        }
    }
}

/// Called after the watcher saw a workspace's files change: when it is Live, the change goes into the main folder.
pub fn follow(app: &Events, shared: &Shared, workspace_id: &str) {
    let Ok(Some(ws)) = shared.db.workspace(workspace_id) else { return };
    let Ok(Some(project)) = shared.db.project(&ws.project_id) else { return };
    if project.live_workspace_id.as_deref() != Some(workspace_id) {
        return;
    }
    let store = Store::new(&shared.data_dir, &project.id);
    // another copy is being made right now (going Live, stopping, another Live workspace's change): this one waits
    // for it. It used to be skipped, on the thought that whoever copies reads the workspace itself; but that copy can
    // be another project's, or have read this workspace before the change, and then the change stayed out of the
    // main folder until the workspace's next one. What is Live is read again below, once the lock is ours
    let _copying = COPYING.lock().unwrap_or_else(|e| e.into_inner());
    let Ok(Some(mut state)) = store.load() else { return };
    if state.workspace_id != workspace_id {
        return;
    }
    let patterns = shared.project_settings(&project.id).map(|s| s.copy_files).unwrap_or_default();
    let main = PathBuf::from(&project.path);
    let note = match changed_files(Path::new(&ws.path), &patterns)
        .and_then(|files| sync(&store, &mut state, &main, Path::new(&ws.path), &files))
    {
        Ok(done) => done.describe_kept(&store).map(|text| (false, text)),
        Err(e) => Some((true, format!("Live couldn't copy {}'s latest change into the main folder: {e:#}", ws.name))),
    };
    if let Some((error, text)) = note {
        app.send(Event::LiveNote { project_id: project.id, error, text });
    }
}

/// The workspace's changes, as its changes list shows them: the files it modified, added or deleted, and its new files
/// (what `git status` lists, renames as a deletion and a new file). Left out: ignored files, submodules (each is a
/// checkout of its own), and the files new workspaces copy from the main folder. While a merge is on, what the merge
/// brings in from the other branch isn't the workspace's work and stays out, as in its changes list.
pub fn changed_files(ws: &Path, from_main: &[String]) -> Result<Vec<String>> {
    let raw = git::git_ok(ws, &["status", "--porcelain=v1", "-z", "--untracked-files=all", "--ignore-submodules=all", "--no-renames"])?;
    let own = git::own_work_in_merge(ws);
    let mut files: Vec<String> = raw
        .split('\0')
        .filter(|e| e.len() > 3)
        .filter(|e| own.as_ref().is_none_or(|own| e.starts_with("??") || own.contains(&e[3..])))
        .map(|e| e[3..].to_string())
        .collect();
    // a file the merge changed and you put back as HEAD has it: not in git status, but your doing
    files.extend(own.into_iter().flatten());
    files.retain(|f| safe(f) && !copied_from_main(f, from_main));
    files.sort();
    files.dedup();
    Ok(files)
}

/// A path inside the repo (git never gives anything else, but the main folder is too precious to trust that).
fn safe(rel: &str) -> bool {
    !rel.is_empty() && Path::new(rel).components().all(|c| matches!(c, Component::Normal(_)))
}

/// One of the project's "Files to copy" (or inside one of its folders): the main folder's own files, which a new
/// workspace gets a copy of and Live never sends back.
fn copied_from_main(rel: &str, patterns: &[String]) -> bool {
    let options = glob::MatchOptions { require_literal_separator: true, ..Default::default() };
    patterns.iter().map(|p| p.trim().trim_end_matches('/')).filter(|p| !p.is_empty()).any(|p| {
        let pattern = glob::Pattern::new(p).ok();
        // the path itself, or any folder it is in
        std::iter::once(rel)
            .chain(rel.match_indices('/').map(|(i, _)| &rel[..i]))
            .any(|part| part == p || pattern.as_ref().is_some_and(|g| g.matches_with(part, options)))
    })
}

/// Where Live keeps what it needs to put the main folder back: `live/<project>/` in the app's data folder.
struct Store {
    root: PathBuf,
}

/// What Live has done to the main folder, saved after every change.
#[derive(Serialize, Deserialize, Default, Debug, PartialEq)]
struct Copied {
    workspace_id: String,
    /// Per file (its path in the repo): whether the main folder had it before Live (that version is in `before/`), and
    /// the fingerprint of what Live last put there ("gone": Live deleted it)
    files: BTreeMap<String, FileState>,
    /// Folders Live made in the main folder for new files; they go again once empty
    dirs: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct FileState {
    had: bool,
    wrote: String,
}

impl Store {
    fn new(data_dir: &Path, project_id: &str) -> Self {
        Self { root: data_dir.join("live").join(project_id) }
    }

    fn state_file(&self) -> PathBuf {
        self.root.join("copied.json")
    }

    fn before(&self, rel: &str) -> PathBuf {
        self.root.join("before").join(rel)
    }

    fn kept_dir(&self) -> PathBuf {
        self.root.join("kept")
    }

    fn load(&self) -> Result<Option<Copied>> {
        match std::fs::read_to_string(self.state_file()) {
            Ok(text) => Ok(Some(serde_json::from_str(&text).context("Live's record of the main folder is unreadable")?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn save(&self, state: &Copied) -> Result<()> {
        std::fs::create_dir_all(&self.root)?;
        let tmp = self.root.join("copied.json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(state)?)?;
        std::fs::rename(&tmp, self.state_file())?;
        Ok(())
    }

    /// Done with this copy: the record and the saved versions go; what was set aside in `kept/` stays.
    fn clear(&self) {
        let _ = std::fs::remove_file(self.state_file());
        let _ = std::fs::remove_dir_all(self.root.join("before"));
    }
}

/// A file as it is on disk.
#[derive(Debug, PartialEq)]
enum Version {
    File { bytes: Vec<u8>, mode: u32 },
    Link(PathBuf),
    Gone,
}

fn read_version(path: &Path) -> Result<Version> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Version::Gone),
        Err(e) => Err(anyhow!("Can't read {}: {e}", path.display())),
        Ok(m) if m.file_type().is_symlink() => Ok(Version::Link(std::fs::read_link(path)?)),
        Ok(m) if m.is_file() => Ok(Version::File {
            bytes: std::fs::read(path).with_context(|| format!("Can't read {}", path.display()))?,
            mode: m.permissions().mode(),
        }),
        Ok(_) => bail!("{} is a folder", path.display()),
    }
}

/// Tells two versions apart (FNV-1a over the bytes, the size, and whether it runs).
fn fingerprint(v: &Version) -> String {
    match v {
        Version::Gone => "gone".into(),
        Version::Link(target) => format!("link:{}", target.display()),
        Version::File { bytes, mode } => {
            let hash = bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3));
            format!("file:{hash:016x}:{}:{}", bytes.len(), if mode & 0o111 != 0 { "x" } else { "-" })
        }
    }
}

/// Puts a version in place: written next to it first and renamed over, so the dev server never reads half a file.
fn write_version(path: &Path, v: &Version) -> Result<()> {
    let name = path.file_name().ok_or_else(|| anyhow!("{} has no file name", path.display()))?.to_string_lossy().to_string();
    match v {
        Version::Gone => match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(anyhow!("Can't remove {}: {e}", path.display())),
            _ => Ok(()),
        },
        Version::File { bytes, mode } => {
            let tmp = path.with_file_name(format!(".{name}.enkidu-live"));
            std::fs::write(&tmp, bytes).with_context(|| format!("Can't write {}", path.display()))?;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(*mode))?;
            std::fs::rename(&tmp, path).with_context(|| format!("Can't write {}", path.display()))?;
            Ok(())
        }
        Version::Link(target) => {
            let tmp = path.with_file_name(format!(".{name}.enkidu-live"));
            let _ = std::fs::remove_file(&tmp);
            std::os::unix::fs::symlink(target, &tmp)?;
            std::fs::rename(&tmp, path).with_context(|| format!("Can't write {}", path.display()))?;
            Ok(())
        }
    }
}

/// What one round of copying did.
#[derive(Debug, Default, PartialEq)]
struct Done {
    /// Files the main folder now has from the workspace
    copied: usize,
    /// Of those, the ones whose own version the main folder had changed before Live (they come back on Stop)
    own: Vec<String>,
    /// Files put back as the main folder had them
    restored: usize,
    /// Files changed in the main folder while Live, set aside before Live wrote over them
    kept: Vec<String>,
    kept_in: Option<PathBuf>,
}

fn home_relative(p: &Path) -> String {
    let home = crate::shellenv::home_dir();
    match p.strip_prefix(&home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => p.display().to_string(),
    }
}

fn some_files(list: &[String]) -> String {
    let shown: Vec<&str> = list.iter().take(3).map(String::as_str).collect();
    let more = list.len().saturating_sub(shown.len());
    if more > 0 { format!("{} and {more} more", shown.join(", ")) } else { shown.join(", ") }
}

fn files(n: usize) -> String {
    if n == 1 { "1 file".into() } else { format!("{n} files") }
}

impl Done {
    fn describe_copy(&self, main: &Path) -> String {
        let main = home_relative(main);
        let mut text = if self.copied == 0 {
            format!("Nothing to copy yet: its changes go into {main} as they come.")
        } else {
            format!("Copied {} with its changes into {main}; new changes follow while it is Live.", files(self.copied))
        };
        if !self.own.is_empty() {
            text.push_str(&format!(" The main folder's own edits to {} come back when you stop.", some_files(&self.own)));
        }
        text
    }

    fn describe_kept(&self, store: &Store) -> Option<String> {
        let dir = self.kept_in.clone().unwrap_or_else(|| store.kept_dir());
        (!self.kept.is_empty()).then(|| {
            format!("{} changed in the main folder while Live, so that version is kept in {}.", some_files(&self.kept), home_relative(&dir))
        })
    }

    /// For Stop.
    fn describe(&self, main: &Path, store: &Store) -> Option<String> {
        let back = (self.restored > 0).then(|| format!("{} is back as it was ({} put back).", home_relative(main), files(self.restored)));
        let kept = self.describe_kept(store);
        let text = [back, kept].into_iter().flatten().collect::<Vec<_>>().join(" ");
        (!text.is_empty()).then_some(text)
    }
}

/// Going Live: the record starts (or carries on, after a restart) and the workspace's changes go in.
fn copy_in(store: &Store, main: &Path, ws: &Workspace, from_main: &[String]) -> Result<Done> {
    let _copying = COPYING.lock().unwrap_or_else(|e| e.into_inner());
    let mut state = store.load()?.unwrap_or_default();
    if state.workspace_id != ws.id && !state.files.is_empty() {
        bail!("The main folder still holds another workspace's copy; stop Live first");
    }
    state.workspace_id = ws.id.clone();
    let changed = changed_files(Path::new(&ws.path), from_main)?;
    sync(store, &mut state, main, Path::new(&ws.path), &changed)
}

/// Stopping: every file Live wrote goes back as the main folder had it.
fn put_back(store: &Store, main: &Path) -> Result<Done> {
    let _copying = COPYING.lock().unwrap_or_else(|e| e.into_inner());
    let Some(mut state) = store.load()? else { return Ok(Done::default()) };
    let done = sync(store, &mut state, main, Path::new(""), &[])?;
    store.clear();
    Ok(done)
}

/// The files among `paths` that the main folder has changed since its last commit (its own edits), asked of git in
/// one go. Each path is taken as it is written, never as a pattern.
fn own_edits(main: &Path, paths: &[&str]) -> std::collections::HashSet<String> {
    let mut found = std::collections::HashSet::new();
    // a few hundred paths a call, so the command line never gets too long
    for some in paths.chunks(200) {
        let mut args = vec!["--literal-pathspecs", "diff", "--name-only", "--no-renames", "-z", "HEAD", "--"];
        args.extend(some.iter().copied());
        if let Ok(out) = git::git(main, &args) {
            found.extend(out.stdout.split('\0').filter(|path| !path.is_empty()).map(String::from));
        }
    }
    found
}

/// Makes the main folder hold the workspace's version of each of `changed`, and puts back the files Live wrote
/// before that the workspace no longer changes. With nothing changed, everything goes back.
fn sync(store: &Store, state: &mut Copied, main: &Path, ws: &Path, changed: &[String]) -> Result<Done> {
    let mut done = Done::default();
    let stamp = || {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        store.kept_dir().join(format!("{secs}"))
    };
    // a file the main folder changed since Live last wrote it is set aside before anything touches it
    let set_aside = |rel: &str, now: &Version, done: &mut Done| -> Result<()> {
        if *now == Version::Gone {
            return Ok(());
        }
        let dir = done.kept_in.get_or_insert_with(stamp).clone();
        let to = dir.join(rel);
        std::fs::create_dir_all(to.parent().unwrap_or(&dir))?;
        write_version(&to, now)?;
        done.kept.push(rel.to_string());
        Ok(())
    };

    // which of the files Live is about to write for the first time hold an edit of the main folder's own. Git is asked
    // once for all of them, not once a file: in a big repository each question takes some 20 ms, and going Live with
    // 425 changed files among 40,000 took 8.9 s of them (the load test, 13 Nov)
    let first_time: Vec<&str> =
        changed.iter().filter(|rel| !state.files.contains_key(*rel)).map(String::as_str).collect();
    let own_edits = own_edits(main, &first_time);

    for rel in changed {
        let theirs = match read_version(&ws.join(rel)) {
            Ok(v) => v,
            // a folder where a file was (a submodule, say) is not something to copy
            Err(_) => continue,
        };
        let target = main.join(rel);
        let ours = read_version(&target).with_context(|| format!("The main folder can't take {rel}"))?;
        match state.files.get(rel).cloned() {
            None => {
                // the first time Live writes this file: the main folder's own version is saved to come back on Stop
                let had = ours != Version::Gone;
                if had {
                    let saved = store.before(rel);
                    std::fs::create_dir_all(saved.parent().unwrap_or(&store.root))?;
                    write_version(&saved, &ours)?;
                    // differs from the main folder's last commit: its own edit, not just the file as committed
                    if own_edits.contains(rel.as_str()) {
                        done.own.push(rel.clone());
                    }
                }
                state.files.insert(rel.clone(), FileState { had, wrote: fingerprint(&ours) });
                store.save(state)?;
            }
            Some(entry) if entry.wrote != fingerprint(&ours) => set_aside(rel, &ours, &mut done)?,
            Some(_) => {}
        }
        if fingerprint(&ours) != fingerprint(&theirs) {
            if theirs != Version::Gone {
                make_dirs(main, rel, state)?;
            }
            write_version(&target, &theirs)?;
        }
        if let Some(entry) = state.files.get_mut(rel) {
            entry.wrote = fingerprint(&theirs);
        }
        if theirs != Version::Gone {
            done.copied += 1;
        }
    }

    let gone_back: Vec<String> = state.files.keys().filter(|k| !changed.contains(k)).cloned().collect();
    for rel in gone_back {
        let entry = state.files[&rel].clone();
        let target = main.join(&rel);
        let now = read_version(&target).with_context(|| format!("The main folder can't take {rel} back"))?;
        if entry.wrote != fingerprint(&now) {
            set_aside(&rel, &now, &mut done)?;
        }
        let original = if entry.had { read_version(&store.before(&rel))? } else { Version::Gone };
        if original != Version::Gone {
            make_dirs(main, &rel, state)?;
        }
        write_version(&target, &original)?;
        let _ = std::fs::remove_file(store.before(&rel));
        state.files.remove(&rel);
        done.restored += 1;
        store.save(state)?;
    }

    // folders Live made go once nothing of Live's is in them (and nothing else either: remove_dir only takes empty ones)
    state.dirs.sort_by_key(|d| std::cmp::Reverse(d.matches('/').count()));
    let mut left = Vec::new();
    for dir in std::mem::take(&mut state.dirs) {
        let prefix = format!("{dir}/");
        let in_use = state.files.keys().any(|f| f.starts_with(&prefix));
        if in_use || std::fs::remove_dir(main.join(&dir)).is_err() && main.join(&dir).exists() {
            left.push(dir);
        }
    }
    state.dirs = left;
    store.save(state)?;
    Ok(done)
}

/// Makes the folders a new file needs in the main folder, remembering the ones Live made.
fn make_dirs(main: &Path, rel: &str, state: &mut Copied) -> Result<()> {
    let mut so_far = PathBuf::new();
    let parent = Path::new(rel).parent().map(Path::to_path_buf).unwrap_or_default();
    for part in parent.components() {
        so_far.push(part);
        let dir = main.join(&so_far);
        if !dir.exists() {
            std::fs::create_dir(&dir).with_context(|| format!("Can't make {}", dir.display()))?;
            state.dirs.push(so_far.to_string_lossy().to_string());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn ws_at(path: &Path, base: &str) -> Workspace {
        Workspace {
            id: "w".into(),
            project_id: "p".into(),
            name: "stock-fix".into(),
            path: path.to_string_lossy().to_string(),
            branch: "stock-fix".into(),
            base: Some(base.into()),
            session_id: "s".into(),
            transcript_path: None,
            claude_config_dir: None,
            adopted: false,
            created_at: 0,
        }
    }

    #[test]
    fn expands_placeholders_quoted() {
        let mut ws = ws_at(Path::new("/ws/a b"), "development");
        ws.name = "it's".into();
        ws.branch = "feature/x".into();
        let out = expand("SHOP_SRC={path} docker compose up -d # {name} {branch} {main}", Path::new("/main"), &ws);
        assert_eq!(out, "SHOP_SRC='/ws/a b' docker compose up -d # 'it'\\''s' 'feature/x' '/main'");
    }

    fn run(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git").args(args).current_dir(dir).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    /// A throwaway repo on `development` with a few files, and a workspace (a worktree) on a branch off it.
    fn repo() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let main = tmp.path().join("shop");
        std::fs::create_dir(&main).unwrap();
        run(&main, &["init", "-q", "-b", "development"]);
        run(&main, &["config", "user.email", "t@example.com"]);
        run(&main, &["config", "user.name", "T"]);
        write(&main.join("src/app.py"), "app v0\n");
        write(&main.join("src/stock.py"), "stock v0\n");
        write(&main.join("docs/old.md"), "old\n");
        write(&main.join(".gitignore"), ".env\n");
        run(&main, &["add", "."]);
        run(&main, &["commit", "-q", "-m", "first"]);
        let ws = tmp.path().join("workspaces/stock-fix");
        run(&main, &["worktree", "add", "-q", "-b", "stock-fix", ws.to_str().unwrap(), "development"]);
        (tmp, main, ws)
    }

    #[test]
    fn copies_a_workspace_into_the_main_folder_follows_it_and_puts_the_main_folder_back() {
        let (tmp, main, ws) = repo();
        let store = Store::new(&tmp.path().join("data"), "p");
        let w = ws_at(&ws, "development");
        // the main folder's own state: an edit of its own, and its .env (ignored)
        write(&main.join("src/stock.py"), "stock: main's own edit\n");
        write(&main.join(".env"), "SECRET=main\n");
        let status_before = run(&main, &["status", "--porcelain"]);
        // the workspace: a commit, an edit, a deleted file, a new file, its own .env and a file copied from main
        write(&ws.join("src/app.py"), "app v1\n");
        run(&ws, &["commit", "-q", "-am", "app v1"]);
        write(&ws.join("src/stock.py"), "stock v1\n");
        std::fs::remove_file(ws.join("docs/old.md")).unwrap();
        write(&ws.join("src/new/vouchers.py"), "new v1\n");
        write(&ws.join(".env"), "SECRET=workspace\n");
        write(&ws.join("CLAUDE.local.md"), "from main\n");
        let from_main = vec![".env".to_string(), "CLAUDE.local.md".to_string()];
        assert_eq!(changed_files(&ws, &from_main).unwrap(), ["docs/old.md", "src/new/vouchers.py", "src/stock.py"], "its changes list");

        let done = copy_in(&store, &main, &w, &from_main).unwrap();
        assert_eq!(read(&main.join("src/stock.py")), "stock v1\n", "a modified file goes in");
        assert_eq!(read(&main.join("src/new/vouchers.py")), "new v1\n", "a new one too");
        assert!(!main.join("docs/old.md").exists(), "a file the workspace deleted is gone");
        assert_eq!(read(&main.join("src/app.py")), "app v0\n", "a committed change isn't in the changes list, so it stays out");
        assert_eq!(read(&main.join(".env")), "SECRET=main\n", "ignored files stay the main folder's");
        assert!(!main.join("CLAUDE.local.md").exists(), "the files copied from the main folder never go back");
        assert_eq!((done.copied, done.own.clone()), (2, vec!["src/stock.py".to_string()]));
        assert!(done.describe_copy(&main).contains("main folder's own edits to src/stock.py come back"));

        // while Live: the workspace changes again, and puts one file back as it was
        write(&ws.join("src/new/vouchers.py"), "new v2\n");
        run(&ws, &["checkout", "-q", "--", "src/stock.py"]);
        let mut state = store.load().unwrap().unwrap();
        let done = sync(&store, &mut state, &main, &ws, &changed_files(&ws, &from_main).unwrap()).unwrap();
        assert_eq!(read(&main.join("src/new/vouchers.py")), "new v2\n");
        assert_eq!(read(&main.join("src/stock.py")), "stock: main's own edit\n", "no longer changed: the main folder's own version is back");
        assert_eq!(done.restored, 1);

        // someone edits the copy in the main folder; the workspace's next change sets that edit aside, never loses it
        write(&main.join("src/new/vouchers.py"), "edited in main while Live\n");
        write(&ws.join("src/new/vouchers.py"), "new v3\n");
        let done = sync(&store, &mut state, &main, &ws, &changed_files(&ws, &from_main).unwrap()).unwrap();
        assert_eq!(read(&main.join("src/new/vouchers.py")), "new v3\n");
        assert_eq!(done.kept, vec!["src/new/vouchers.py".to_string()]);
        assert_eq!(read(&done.kept_in.clone().unwrap().join("src/new/vouchers.py")), "edited in main while Live\n");
        assert!(done.describe_kept(&store).unwrap().contains("src/new/vouchers.py changed in the main folder while Live"));

        // Stop: the main folder is exactly as before, its own edit included; Live's folders are gone
        let back = put_back(&store, &main).unwrap();
        assert_eq!(back.restored, 2);
        assert_eq!(read(&main.join("src/app.py")), "app v0\n");
        assert_eq!(read(&main.join("src/stock.py")), "stock: main's own edit\n");
        assert_eq!(read(&main.join("docs/old.md")), "old\n");
        assert!(!main.join("src/new").exists(), "the folder Live made for a new file goes with it");
        assert_eq!(run(&main, &["status", "--porcelain"]), status_before);
        assert!(store.load().unwrap().is_none(), "nothing left to put back");
        assert!(store.kept_dir().exists(), "what was set aside stays");
        // the workspace itself was never touched
        assert_eq!(read(&ws.join("src/app.py")), "app v1\n");
        assert_eq!(read(&ws.join("src/new/vouchers.py")), "new v3\n");
    }

    /// Found by three whole-app Live tests running at once (13 Nov): a change that came while another copy was being
    /// made was skipped, and stayed out of the main folder until the workspace's next change.
    #[test]
    fn a_change_that_comes_while_another_copy_is_made_waits_for_it_and_is_copied() {
        let (tmp, main, ws) = repo();
        let shared = Shared::new(tmp.path().join("data"), std::time::Instant::now()).unwrap();
        let project = crate::db::Project {
            id: "p".into(),
            name: "shop".into(),
            path: main.to_string_lossy().into(),
            live_workspace_id: None,
            created_at: 0,
        };
        shared.db.insert_project(&project).unwrap();
        let w = ws_at(&ws, "development");
        shared.db.insert_workspace(&w).unwrap();
        // it goes Live with one change
        write(&ws.join("src/app.py"), "app v1\n");
        set_live(&Events::none(), &shared, "p", Some("w")).unwrap();
        assert_eq!(read(&main.join("src/app.py")), "app v1\n");
        // another copy holds the lock for a moment (another project going Live, say) while a new change comes
        let (holding, held) = std::sync::mpsc::channel::<()>();
        let other = std::thread::spawn(move || {
            let _copying = COPYING.lock().unwrap_or_else(|e| e.into_inner());
            holding.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(300));
        });
        held.recv().unwrap();
        write(&ws.join("src/app.py"), "app v2\n");
        write(&ws.join("notes.md"), "new\n");
        let asked = std::time::Instant::now();
        follow(&Events::none(), &shared, "w");
        let waited = asked.elapsed();
        other.join().unwrap();
        assert!(waited >= Duration::from_millis(250), "it didn't wait for the other copy: {waited:?}");
        assert_eq!(read(&main.join("src/app.py")), "app v2\n", "the change was skipped");
        assert_eq!(read(&main.join("notes.md")), "new\n");
        // and Stop still puts everything back
        set_live(&Events::none(), &shared, "p", None).unwrap();
        assert_eq!(read(&main.join("src/app.py")), "app v0\n");
        assert!(!main.join("notes.md").exists());
    }

    /// Going Live asks git once which of the files hold an edit of the main folder's own (it asked once a file, 20 ms
    /// each in a big repository). The answer is the same as before, also for names that look like patterns.
    #[test]
    fn the_main_folders_own_edits_are_found_in_one_question_whatever_the_files_are_called() {
        let (tmp, main, ws) = repo();
        // two more files in the last commit, one with a name git would read as a pattern
        write(&main.join("src/[x].py"), "bracket v0\n");
        write(&main.join("src/x.py"), "x v0\n");
        run(&main, &["add", "."]);
        run(&main, &["commit", "-q", "-m", "two more"]);
        run(&ws, &["merge", "-q", "development"]);
        // the main folder's own edits: to app.py, and to x.py (which the workspace doesn't touch)
        write(&main.join("src/app.py"), "app, edited in the main folder\n");
        write(&main.join("src/x.py"), "x, edited in the main folder\n");
        // the workspace changes four files and adds one
        write(&ws.join("src/app.py"), "app v1\n");
        write(&ws.join("src/stock.py"), "stock v1\n");
        write(&ws.join("src/[x].py"), "bracket v1\n");
        write(&ws.join("docs/old.md"), "old v1\n");
        write(&ws.join("docs/new.md"), "new\n");
        assert_eq!(
            own_edits(&main, &["src/app.py", "src/stock.py", "src/[x].py", "docs/old.md", "docs/new.md"]),
            ["src/app.py".to_string()].into_iter().collect(),
            "only app.py: [x].py is its own name, not a pattern that takes x.py in"
        );
        assert!(own_edits(&main, &[]).is_empty());
        let store = Store::new(&tmp.path().join("data"), "p");
        let done = copy_in(&store, &main, &ws_at(&ws, "development"), &[]).unwrap();
        assert_eq!((done.copied, done.own.clone()), (5, vec!["src/app.py".to_string()]));
        assert_eq!(read(&main.join("src/[x].py")), "bracket v1\n");
        assert_eq!(read(&main.join("src/x.py")), "x, edited in the main folder\n", "not the workspace's to touch");
        // Stop brings the main folder's own edit back
        put_back(&store, &main).unwrap();
        assert_eq!(read(&main.join("src/app.py")), "app, edited in the main folder\n");
        assert_eq!(read(&main.join("src/[x].py")), "bracket v0\n");
        assert!(!main.join("docs/new.md").exists());
    }

    #[test]
    fn a_restart_carries_on_a_commit_takes_a_file_back_and_stopping_twice_changes_nothing() {
        let (tmp, main, ws) = repo();
        let store = Store::new(&tmp.path().join("data"), "p");
        let w = ws_at(&ws, "development");
        write(&ws.join("src/app.py"), "app v1\n");
        copy_in(&store, &main, &w, &[]).unwrap();
        // the app restarts: the record is on disk, so the same copy carries on
        write(&ws.join("src/app.py"), "app v2\n");
        let done = copy_in(&store, &main, &w, &[]).unwrap();
        assert_eq!((done.copied, read(&main.join("src/app.py"))), (1, "app v2\n".to_string()));
        // Claude commits it: no longer a change, so the main folder has its own version again
        run(&ws, &["commit", "-q", "-am", "app v2"]);
        let mut state = store.load().unwrap().unwrap();
        let done = sync(&store, &mut state, &main, &ws, &changed_files(&ws, &[]).unwrap()).unwrap();
        assert_eq!((done.restored, read(&main.join("src/app.py"))), (1, "app v0\n".to_string()));
        assert_eq!(put_back(&store, &main).unwrap(), Done::default());
        assert_eq!(put_back(&store, &main).unwrap(), Done::default());
        assert_eq!(run(&main, &["status", "--porcelain"]), "");
    }

    #[test]
    fn copies_only_its_changes_list_even_when_the_main_folder_is_far_behind() {
        let (tmp, main, _) = repo();
        // origin's development moved on (others' work); the main folder's local development didn't
        let old = run(&main, &["rev-parse", "HEAD"]).trim().to_string();
        write(&main.join("src/others.py"), "someone else's work\n");
        run(&main, &["add", "."]);
        run(&main, &["commit", "-qm", "others"]);
        let newer = run(&main, &["rev-parse", "HEAD"]).trim().to_string();
        run(&main, &["update-ref", "refs/remotes/origin/development", &newer]);
        run(&main, &["reset", "-q", "--hard", &old]);
        // the workspace is made from origin's development, as Enkidu makes them
        let ws = tmp.path().join("workspaces/fresh");
        run(&main, &["worktree", "add", "-q", "-b", "fresh", ws.to_str().unwrap(), "origin/development"]);
        write(&ws.join("src/stock.py"), "stock v1\n");
        assert_eq!(changed_files(&ws, &[]).unwrap(), ["src/stock.py"], "its own change, never the others' work");
        let store = Store::new(&tmp.path().join("data"), "p");
        assert_eq!(copy_in(&store, &main, &ws_at(&ws, "development"), &[]).unwrap().copied, 1);
        assert_eq!(read(&main.join("src/stock.py")), "stock v1\n");
        assert!(!main.join("src/others.py").exists());
        assert_eq!(run(&main, &["rev-parse", "HEAD"]).trim(), old, "the main folder's branch stays where it is");
        put_back(&store, &main).unwrap();
        assert_eq!(run(&main, &["status", "--porcelain"]), "");
    }

    #[test]
    fn during_a_merge_copies_only_the_workspaces_own_work() {
        let (_tmp, main, ws) = repo();
        // development moves on in two files; the workspace changed one of them too
        write(&main.join("src/app.py"), "app v1\n");
        write(&main.join("src/stock.py"), "stock v1\n");
        run(&main, &["commit", "-q", "-am", "development moves on"]);
        write(&ws.join("src/stock.py"), "stock: the workspace's\n");
        run(&ws, &["commit", "-q", "-am", "the workspace's stock"]);
        let merge = Command::new("git").args(["merge", "development"]).current_dir(&ws).output().unwrap();
        assert!(!merge.status.success(), "stops on the conflict in stock.py");
        // git's status lists app.py too, staged by the merge: development's work, not the workspace's
        assert!(run(&ws, &["status", "--porcelain"]).contains("src/app.py"));
        assert_eq!(changed_files(&ws, &[]).unwrap(), ["src/stock.py"]);
        write(&ws.join("src/stock.py"), "stock v1, and the workspace's\n");
        write(&ws.join("src/new.py"), "new\n");
        run(&ws, &["add", "src/stock.py"]);
        assert_eq!(changed_files(&ws, &[]).unwrap(), ["src/new.py", "src/stock.py"]);
    }

    #[test]
    fn leaves_submodules_alone() {
        let (tmp, main, _) = repo();
        let lib = tmp.path().join("spa-origin");
        std::fs::create_dir(&lib).unwrap();
        run(&lib, &["init", "-q", "-b", "main"]);
        run(&lib, &["config", "user.email", "t@example.com"]);
        run(&lib, &["config", "user.name", "T"]);
        write(&lib.join("index.js"), "spa\n");
        run(&lib, &["add", "."]);
        run(&lib, &["commit", "-q", "-m", "spa"]);
        run(&main, &["-c", "protocol.file.allow=always", "submodule", "add", "-q", lib.to_str().unwrap(), "spa"]);
        run(&main, &["commit", "-q", "-m", "spa"]);
        // a new workspace has the submodule's folder empty (not checked out), like Enkidu's
        let ws = tmp.path().join("workspaces/two");
        run(&main, &["worktree", "add", "-q", "-b", "two", ws.to_str().unwrap(), "development"]);
        write(&ws.join("src/app.py"), "app v1\n");
        assert_eq!(changed_files(&ws, &[]).unwrap(), ["src/app.py"]);
        // and the main folder's checked-out submodule with a change of its own stays exactly as it is
        write(&main.join("spa/index.js"), "spa: edited in main\n");
        let store = Store::new(&tmp.path().join("data"), "p");
        copy_in(&store, &main, &ws_at(&ws, "development"), &[]).unwrap();
        put_back(&store, &main).unwrap();
        assert_eq!(read(&main.join("spa/index.js")), "spa: edited in main\n");
    }

    #[test]
    fn knows_the_files_new_workspaces_copy_from_the_main_folder() {
        let patterns = vec![".env".to_string(), ".claude/settings.local.json".to_string(), "config/*.json".to_string(), "secrets/".to_string()];
        assert!(copied_from_main(".env", &patterns));
        assert!(copied_from_main(".claude/settings.local.json", &patterns));
        assert!(copied_from_main("config/local.json", &patterns));
        assert!(copied_from_main("secrets/key.pem", &patterns), "inside a listed folder");
        assert!(!copied_from_main("config/sub/local.json", &patterns), "* stays inside its folder");
        assert!(!copied_from_main("src/.env.py", &patterns));
        assert!(!safe("../outside.py") && !safe("/etc/hosts") && safe("src/app.py"));
    }
}
