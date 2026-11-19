//! Enkidu's core: everything the app does that is not drawing. No GPUI here.
//!
//! The app starts it with [`start`] and hands it an [`Events`] sink: the core's news (a lamp changed, a terminal
//! printed, the Work index changed) arrives there as typed [`Event`]s. What the window asks of the core are the plain
//! functions of [`commands`], called off the drawing thread.
//!
//! Most modules carry `#[rustfmt::skip]` and keep a layout of their own, with lint's style and complexity groups
//! off for them; its correctness, suspicious and performance lints still apply.
//!
//! `places` holds the app's own identity and folders.

pub mod events;
pub mod places;

#[rustfmt::skip]
#[allow(clippy::style, clippy::complexity)]
pub mod commands;
#[rustfmt::skip]
#[allow(clippy::style, clippy::complexity)]
pub mod db;
#[rustfmt::skip]
#[allow(clippy::style, clippy::complexity)]
pub mod git;
pub mod goto;
#[rustfmt::skip]
#[allow(clippy::style, clippy::complexity)]
pub mod hooks;
pub mod icon;
#[rustfmt::skip]
#[allow(clippy::style, clippy::complexity)]
pub mod live;
pub mod lsp;
pub mod names;
pub mod setup;
#[rustfmt::skip]
#[allow(clippy::style, clippy::complexity)]
pub mod pty;
#[rustfmt::skip]
#[allow(clippy::style, clippy::complexity)]
pub mod redact;
#[rustfmt::skip]
#[allow(clippy::style, clippy::complexity)]
pub mod script;
#[rustfmt::skip]
#[allow(clippy::style, clippy::complexity)]
pub mod search;
#[rustfmt::skip]
#[allow(clippy::style, clippy::complexity)]
pub mod settings;
#[rustfmt::skip]
#[allow(clippy::style, clippy::complexity)]
pub mod shellenv;
#[rustfmt::skip]
#[allow(clippy::style, clippy::complexity)]
pub mod state;
#[rustfmt::skip]
#[allow(clippy::style, clippy::complexity)]
pub mod symbols;
#[rustfmt::skip]
#[allow(clippy::style, clippy::complexity)]
pub mod watcher;
#[rustfmt::skip]
#[allow(clippy::style, clippy::complexity)]
pub mod work;
#[rustfmt::skip]
#[allow(clippy::style, clippy::complexity)]
pub mod workspace;

use std::sync::Arc;
use std::time::Instant;

pub use events::{Event, Events};
use state::Shared;

/// The running core, as the app holds it.
pub struct Core {
    pub shared: Arc<Shared>,
}

impl Core {
    /// Ends every terminal's program; the app calls this as it quits.
    pub fn shutdown(&self) {
        self.shared.pty.close_all();
        // and every language server it started
        self.shared.servers.stop_all();
    }
}

/// Starts the core: the data folder, the
/// database and settings, the hook server and its `api.json`, the file watcher, the Work index and the scheduled
/// workflows. Everything slow runs on threads of its own; this returns as soon as they are started.
pub fn start(app: Events) -> anyhow::Result<Core> {
    let started = Instant::now();
    raise_open_files_limit();
    let data_dir = places::data_dir();
    let shared = Arc::new(Shared::new(data_dir, started)?);
    let server = hooks::start(app.clone(), shared.clone())?;
    *shared.hooks.lock().unwrap_or_else(|e| e.into_inner()) = Some(server);
    shared.changes.start(app.clone())?;
    // a Live workspace's changes go on into the main folder as they happen
    let follower = Arc::downgrade(&shared);
    shared.changes.after_refresh(move |app, id| {
        if let Some(shared) = follower.upgrade() {
            work::follow_branch(app, &shared, id);
            live::follow(app, &shared, id);
        }
    });
    // The Work index: the sessions behind your PRs and tickets (saved sessions, every minute).
    work::start(app.clone(), shared.clone());
    work::start_watcher(app.clone(), shared.clone());
    // your scheduled workflows: what is due starts, the task that runs is looked after
    work::flows::start(app.clone(), shared.clone());
    // Ask the login shell for PATH early, so the first terminal opens fast.
    std::thread::spawn(|| {
        shellenv::login_path();
    });
    // Start watching every workspace, so each one's changes are ready before you open it.
    let watched = shared.clone();
    std::thread::spawn(move || {
        for ws in watched.db.workspaces().unwrap_or_default() {
            watched.changes.add(&ws.id, std::path::Path::new(&ws.path));
        }
    });
    Ok(Core { shared })
}

/// macOS's own cap on files one process may open (OPEN_MAX); asking for more than this fails.
const OPEN_FILES: libc::rlim_t = 10_240;

/// Started from the Dock or Finder (by launchd), the app may have only 256 files open at once, and just after a
/// restart it once ran past that: "Can't open a terminal: failed to openpty … Too many open files". The soft limit goes
/// up to the hard one, at most 10,240; the programs it starts (Claude, shells, git) inherit it. Returns the new limit.
#[rustfmt::skip]
fn raise_open_files_limit() -> Option<libc::rlim_t> {
    let mut limit = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
    // SAFETY: getrlimit and setrlimit only read and write the struct they are given.
    unsafe {
        if libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) != 0 {
            return None;
        }
        let want = limit.rlim_max.min(OPEN_FILES);
        if limit.rlim_cur < want {
            limit.rlim_cur = want;
            if libc::setrlimit(libc::RLIMIT_NOFILE, &limit) != 0 {
                return None;
            }
        }
    }
    Some(limit.rlim_cur)
}

#[cfg(test)]
#[rustfmt::skip]
mod tests {
    use super::*;

    fn open_files_limit() -> libc::rlimit {
        let mut limit = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        // SAFETY: getrlimit only writes the struct it is given.
        assert_eq!(unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) }, 0);
        limit
    }

    const LOW_LIMIT_TEST: &str = "tests::raises_the_open_files_limit_from_launchds_256_for_the_app_and_what_it_starts";

    #[test]
    fn raises_the_open_files_limit_from_launchds_256_for_the_app_and_what_it_starts() {
        // As launchd starts the app: 256 files (the soft limit only). In a process of its own, so the other tests keep
        // theirs: this test runs itself again under that limit.
        if std::env::var_os("ENKIDU_LOW_LIMIT_TEST").is_none() {
            let out = std::process::Command::new("/bin/sh")
                .args(["-c", r#"ulimit -S -n 256 && exec "$0" --exact "$1" --nocapture --test-threads=1"#])
                .arg(std::env::current_exe().unwrap())
                .arg(LOW_LIMIT_TEST)
                .env("ENKIDU_LOW_LIMIT_TEST", "1")
                .output()
                .unwrap();
            let said = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
            assert!(out.status.success() && said.contains("1 passed"), "{said}");
            return;
        }
        let before = open_files_limit();
        assert_eq!(before.rlim_cur, 256);
        let raised = raise_open_files_limit().expect("the limit can be read and raised");
        assert_eq!(raised, before.rlim_max.min(OPEN_FILES), "as far as it goes, and no further than macOS allows");
        assert!(raised > 256);
        assert_eq!(open_files_limit().rlim_cur, raised);
        // a program started from here (Claude, a shell, git) has the same limit
        let out = std::process::Command::new("/bin/sh").args(["-c", "ulimit -S -n"]).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), raised.to_string());
        // asking again changes nothing
        assert_eq!(raise_open_files_limit(), Some(raised));
    }

    #[test]
    fn leaves_a_higher_open_files_limit_alone() {
        let before = open_files_limit();
        let raised = raise_open_files_limit().expect("the limit can be read");
        assert_eq!(raised, before.rlim_cur.max(before.rlim_max.min(OPEN_FILES)));
        assert!(raised >= before.rlim_cur, "never lowered");
    }
}
