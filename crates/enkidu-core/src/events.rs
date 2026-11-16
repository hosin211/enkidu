//! What the core tells the app as things happen: each is a typed value the app receives (item 1.6).
//!
//! The core calls [`Events::send`] from whatever thread the news comes from (a terminal's reader, the file watcher,
//! the hook server, a workflow). The app's side must only hand the event on to its drawing thread, never block.

use std::sync::Arc;

use crate::db::Workspace;
use crate::git::Change;
use crate::live::LiveOutcome;
use crate::pty::TerminalInfo;
use crate::work::workflows::Run;
use crate::workspace::Progress;

/// One piece of news for the app.
#[derive(Debug, Clone)]
pub enum Event {
    /// A workspace's Claude changed state: idle, working, needs_you or done.
    StatusChanged { workspace_id: String, status: String },
    /// A step of making a workspace, as it happens.
    WorkspaceProgress(Progress),
    /// A workspace's changed files, after the watcher looked again.
    ChangesUpdated { workspace_id: String, files: Vec<Change> },
    /// Text a terminal printed, and where it sits in that terminal's buffer.
    PtyOutput { id: String, data: String, start: u64, end: u64 },
    /// A terminal's program ended.
    PtyExit { id: String, code: Option<i32> },
    /// A workflow started a terminal; the window shows it.
    PtySpawned(TerminalInfo),
    /// Live was turned on or off, or moved.
    LiveChanged(LiveOutcome),
    /// Something to say about Live in a project: what it copied, or what went wrong.
    LiveNote { project_id: String, error: bool, text: String },
    /// The Work index changed: read it again.
    WorkChanged,
    /// A workspace changed, its branch for one.
    WorkspaceUpdated(Workspace),
    /// A workflow made a workspace.
    WorkspaceAdded(Workspace),
    /// A workflow's run ended.
    WorkflowRan(Run),
}

/// Where the core sends its events: the app gives one when it starts the core. Cheap to clone and to share between
/// threads.
#[derive(Clone)]
pub struct Events(Arc<dyn Fn(Event) + Send + Sync>);

impl Events {
    pub fn new(to: impl Fn(Event) + Send + Sync + 'static) -> Self {
        Self(Arc::new(to))
    }

    /// Events that go nowhere, for tests and tools that don't listen.
    pub fn none() -> Self {
        Self::new(|_| {})
    }

    pub fn send(&self, event: Event) {
        (self.0)(event)
    }
}

impl std::fmt::Debug for Events {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Events")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn events_reach_whoever_listens_from_any_thread() {
        let got = Arc::new(Mutex::new(Vec::new()));
        let events = {
            let got = got.clone();
            Events::new(move |e| got.lock().unwrap().push(format!("{e:?}")))
        };
        let other = events.clone();
        std::thread::spawn(move || other.send(Event::WorkChanged)).join().unwrap();
        events.send(Event::StatusChanged { workspace_id: "w1".into(), status: "working".into() });
        Events::none().send(Event::WorkChanged);
        assert_eq!(
            *got.lock().unwrap(),
            ["WorkChanged", "StatusChanged { workspace_id: \"w1\", status: \"working\" }"]
        );
    }
}
