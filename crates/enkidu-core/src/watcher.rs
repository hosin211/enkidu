//! Keeps every workspace's list of changes fresh. One file watcher covers all workspaces; when files change, a worker
//! thread asks git for the new list and pushes it to the UI only when it differs from the last one. The workspace on
//! screen refreshes within a fraction of a second, the others a little later, so switching is always instant.

use notify::{RecursiveMode, Watcher};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use crate::events::{Event, Events};

use crate::git::{self, Change};

/// How long to wait after the last file event before asking git, for the workspace on screen and for the others.
const QUICK: Duration = Duration::from_millis(150);
const CALM: Duration = Duration::from_millis(1200);

/// Changed files remembered per workspace for the symbol index; past this it just re-checks every file.
const MAX_CHANGED: usize = 5000;

/// Folders that git never tracks in practice; their churn (installs, caches) never changes the list.
const NOISE: [&str; 8] = ["node_modules", "__pycache__", ".pytest_cache", ".mypy_cache", ".ruff_cache", ".venv", ".tox", ".DS_Store"];

struct Entry {
    path: PathBuf,
    git_dir: Option<PathBuf>,
}

enum Msg {
    Dirty(String),
}

/// The file watcher's callback only ever takes `entries`, and nothing holds `entries` while the watcher is being
/// changed: on macOS changing what is watched waits for the callback thread, so sharing one lock could deadlock.
#[derive(Default)]
pub struct ChangesHub {
    entries: Mutex<HashMap<String, Entry>>,
    focused: Mutex<Option<String>>,
    watcher: Mutex<Option<notify::RecommendedWatcher>>,
    cache: Mutex<HashMap<String, Vec<Change>>>,
    tx: Mutex<Option<Sender<Msg>>>,
    /// Per workspace: a counter that goes up on every file event, and the files changed since the symbol index last
    /// asked (None once there were too many to list).
    changed: Mutex<HashMap<String, (u64, Option<HashSet<String>>)>>,
    /// Per workspace: the files changed since its language server was last told (None once there were too many to
    /// list). Kept apart from the symbol index's list: each of the two takes its own.
    for_servers: Mutex<HashMap<String, Option<HashSet<String>>>>,
    /// Told after every look at a workspace's files (Live copies the change into the main folder).
    after: Mutex<Option<AfterRefresh>>,
}

type AfterRefresh = Arc<dyn Fn(&Events, &str) + Send + Sync>;

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn is_noise(path: &Path) -> bool {
    path.components().any(|c| NOISE.iter().any(|n| c.as_os_str() == *n))
}

impl ChangesHub {
    /// Starts the worker thread and the file watcher. Call once, after the app is set up.
    pub fn start(self: &Arc<Self>, app: Events) -> anyhow::Result<()> {
        let (tx, rx) = mpsc::channel::<Msg>();
        *lock(&self.tx) = Some(tx.clone());

        let hub = Arc::downgrade(self);
        let events = tx.clone();
        let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let (Ok(event), Some(hub)) = (res, hub.upgrade()) else { return };
            let entries = lock(&hub.entries);
            for path in &event.paths {
                if is_noise(path) {
                    continue;
                }
                for (id, entry) in entries.iter() {
                    let in_git_dir = entry.git_dir.as_deref().is_some_and(|g| path.starts_with(g));
                    if path.starts_with(&entry.path) || in_git_dir {
                        let _ = events.send(Msg::Dirty(id.clone()));
                        let rel = path.strip_prefix(&entry.path).ok().and_then(|r| r.to_str()).filter(|r| !r.is_empty());
                        hub.note_change(id, if in_git_dir { None } else { rel }, path.is_dir());
                    }
                }
            }
        })?;
        *lock(&self.watcher) = Some(watcher);

        let hub = Arc::downgrade(self);
        std::thread::spawn(move || {
            let mut due: HashMap<String, Instant> = HashMap::new();
            loop {
                let wait = due.values().min().map(|t| t.saturating_duration_since(Instant::now())).unwrap_or(Duration::from_secs(3600));
                match rx.recv_timeout(wait) {
                    Ok(Msg::Dirty(id)) => {
                        let Some(hub) = hub.upgrade() else { return };
                        let delay = if hub.is_focused(&id) { QUICK } else { CALM };
                        let at = Instant::now() + delay;
                        due.entry(id).and_modify(|t| *t = (*t).min(at)).or_insert(at);
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return,
                }
                let now = Instant::now();
                let ready: Vec<String> = due.iter().filter(|(_, t)| **t <= now).map(|(id, _)| id.clone()).collect();
                for id in ready {
                    due.remove(&id);
                    let Some(hub) = hub.upgrade() else { return };
                    hub.refresh(&app, &id);
                }
            }
        });
        Ok(())
    }

    fn is_focused(&self, id: &str) -> bool {
        lock(&self.focused).as_deref() == Some(id)
    }

    /// enkidu: the workspace the app said is on screen, whose files are looked at first.
    pub fn focused(&self) -> Option<String> {
        lock(&self.focused).clone()
    }

    /// Starts watching a workspace and computes its first list in the background.
    pub fn add(&self, id: &str, path: &Path) {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let git_dir = git::git_dir(&path).ok().map(|g| g.canonicalize().unwrap_or(g));
        lock(&self.entries).insert(id.to_string(), Entry { path: path.clone(), git_dir: git_dir.clone() });
        if let Some(w) = lock(&self.watcher).as_mut() {
            let _ = w.watch(&path, RecursiveMode::Recursive);
            if let Some(g) = &git_dir {
                // the index and HEAD live here: commits and staging show up through them
                let _ = w.watch(g, RecursiveMode::NonRecursive);
            }
        }
        self.poke(id);
    }

    pub fn remove(&self, id: &str) {
        let entry = lock(&self.entries).remove(id);
        if let (Some(entry), Some(w)) = (entry, lock(&self.watcher).as_mut()) {
            let _ = w.unwatch(&entry.path);
            if let Some(g) = &entry.git_dir {
                let _ = w.unwatch(g);
            }
        }
        let mut focused = lock(&self.focused);
        if focused.as_deref() == Some(id) {
            *focused = None;
        }
        drop(focused);
        lock(&self.cache).remove(id);
        lock(&self.changed).remove(id);
        lock(&self.for_servers).remove(id);
    }

    /// The workspace on screen refreshes fastest.
    pub fn focus(&self, id: Option<String>) {
        *lock(&self.focused) = id.clone();
        if let Some(id) = id {
            self.poke(&id);
        }
    }

    fn poke(&self, id: &str) {
        if let Some(tx) = lock(&self.tx).as_ref() {
            let _ = tx.send(Msg::Dirty(id.to_string()));
        }
    }

    #[cfg(test)]
    pub fn cached(&self, id: &str) -> Option<Vec<Change>> {
        lock(&self.cache).get(id).cloned()
    }

    /// Every list worked out so far, so the window opens already knowing each workspace's changes.
    pub fn snapshot(&self) -> HashMap<String, Vec<Change>> {
        lock(&self.cache).clone()
    }

    /// New files git doesn't track yet (and doesn't ignore), from the last list of changes.
    pub fn untracked(&self, id: &str) -> Vec<String> {
        lock(&self.cache).get(id).map(|list| list.iter().filter(|c| c.kind == "untracked").map(|c| c.path.clone()).collect()).unwrap_or_default()
    }

    fn note_change(&self, id: &str, rel: Option<&str>, is_dir: bool) {
        {
            let mut for_servers = lock(&self.for_servers);
            let files = for_servers.entry(id.to_string()).or_insert_with(|| Some(HashSet::new()));
            match (rel, files.as_mut()) {
                (Some(rel), Some(list)) if !is_dir && list.len() < MAX_CHANGED => {
                    list.insert(rel.to_string());
                }
                (Some(_), _) => *files = None,
                (None, _) => {}
            }
        }
        let mut changed = lock(&self.changed);
        let entry = changed.entry(id.to_string()).or_insert_with(|| (0, Some(HashSet::new())));
        entry.0 += 1;
        let Some(files) = entry.1.as_mut() else { return };
        match rel {
            // git's own files (index, HEAD) changed: counted, but no file of the workspace to re-read
            None => {}
            Some(rel) if !is_dir && files.len() < MAX_CHANGED => {
                files.insert(rel.to_string());
            }
            // a whole folder, or too many files: the list can't say which, so everything gets checked
            Some(_) => entry.1 = None,
        }
    }

    /// The workspace's change counter: equal counters mean no file changed in between.
    pub fn generation(&self, id: &str) -> u64 {
        lock(&self.changed).get(id).map_or(0, |c| c.0)
    }

    /// The counter, and the files changed since the last call (None: too many, or unknown; check them all).
    pub fn take_changed(&self, id: &str) -> (u64, Option<HashSet<String>>) {
        let mut changed = lock(&self.changed);
        let entry = changed.entry(id.to_string()).or_insert_with(|| (0, Some(HashSet::new())));
        let files = std::mem::replace(&mut entry.1, Some(HashSet::new()));
        (entry.0, files)
    }

    /// The files changed since the workspace's language server last asked (None: too many, or a whole folder; it
    /// has to look at everything again).
    pub fn take_changed_for_servers(&self, id: &str) -> Option<HashSet<String>> {
        let mut for_servers = lock(&self.for_servers);
        let files = for_servers.entry(id.to_string()).or_insert_with(|| Some(HashSet::new()));
        std::mem::replace(files, Some(HashSet::new()))
    }

    /// Stores a freshly computed list. Returns true when it differs from the last one.
    pub fn store(&self, id: &str, files: &[Change]) -> bool {
        let mut cache = lock(&self.cache);
        if cache.get(id).is_some_and(|old| old.as_slice() == files) {
            return false;
        }
        cache.insert(id.to_string(), files.to_vec());
        true
    }

    /// What to call after each look at a workspace's files, on the watcher's thread.
    pub fn after_refresh(&self, f: impl Fn(&Events, &str) + Send + Sync + 'static) {
        *lock(&self.after) = Some(Arc::new(f));
    }

    fn refresh(&self, app: &Events, id: &str) {
        let Some(path) = lock(&self.entries).get(id).map(|e| e.path.clone()) else { return };
        let Ok(files) = git::changes(&path) else { return };
        if self.store(id, &files) {
            app.send(Event::ChangesUpdated { workspace_id: id.to_string(), files });
        }
        let after = lock(&self.after).clone();
        if let Some(after) = after {
            after(app, id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_folders_git_never_tracks() {
        assert!(is_noise(Path::new("/ws/app/node_modules/react/index.js")));
        assert!(is_noise(Path::new("/ws/app/pkg/__pycache__/x.pyc")));
        assert!(!is_noise(Path::new("/ws/app/src/node_modules_docs.md")));
        assert!(!is_noise(Path::new("/ws/app/src/stock.py")));
    }

    #[test]
    fn reports_a_list_only_when_it_changed() {
        let hub = ChangesHub::default();
        let change = |path: &str, sig: &str| Change {
            path: path.into(),
            old_path: None,
            kind: "modified".into(),
            staged: false,
            additions: Some(1),
            deletions: Some(0),
            sig: sig.into(),
        };
        assert!(hub.store("w1", &[change("a.py", "1")]));
        assert!(!hub.store("w1", &[change("a.py", "1")]), "same list: nothing to send");
        assert!(hub.store("w1", &[change("a.py", "2")]), "the file changed again");
        assert_eq!(hub.cached("w1").unwrap()[0].sig, "2");
        hub.remove("w1");
        assert!(hub.cached("w1").is_none());
    }

    #[test]
    fn counts_changes_and_remembers_which_files_changed() {
        let hub = ChangesHub::default();
        assert_eq!(hub.generation("w1"), 0);
        hub.note_change("w1", Some("src/a.py"), false);
        hub.note_change("w1", Some("src/b.py"), false);
        hub.note_change("w1", None, false); // git's own files (index, HEAD): counted, no file to re-read
        assert_eq!(hub.generation("w1"), 3);
        let (generation, files) = hub.take_changed("w1");
        assert_eq!(generation, 3);
        assert_eq!(files.unwrap(), ["src/a.py".to_string(), "src/b.py".to_string()].into());
        // taken: the next call starts empty
        assert!(hub.take_changed("w1").1.unwrap().is_empty());
        // a whole folder changed: the list can't say which files, so everything gets checked
        hub.note_change("w1", Some("src"), true);
        assert!(hub.take_changed("w1").1.is_none());
    }
}
