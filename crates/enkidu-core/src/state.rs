//! Shared app state.

use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use crate::events::{Event, Events};

use crate::db::Db;
use crate::hooks::HookServer;
use crate::pty::PtyManager;
use crate::search::{FileLists, LastText, Running};
use crate::symbols::SymbolIndexes;
use crate::settings::{self, ProjectSettings, UserSettings};
use crate::watcher::ChangesHub;

pub const STATUSES: [&str; 4] = ["idle", "working", "needs_you", "done"];

pub struct Shared {
    pub data_dir: PathBuf,
    pub db: Db,
    pub user: Mutex<UserSettings>,
    pub statuses: Mutex<HashMap<String, String>>,
    pub hooks: Mutex<Option<HookServer>>,
    pub pty: PtyManager,
    pub changes: Arc<ChangesHub>,
    /// Search: tracked-file lists cached per workspace, running text searches, the last one per workspace (to narrow
    /// the next), and the symbol indexes of the workspaces searched lately.
    pub file_lists: FileLists,
    pub text_running: Running,
    pub last_text: Mutex<HashMap<String, LastText>>,
    pub symbols: SymbolIndexes,
    /// The language servers that run, one for each workspace asked lately (go to definition).
    pub servers: crate::lsp::Servers,
    /// The Work index: tickets, PRs and the sessions behind them.
    pub work: crate::work::Hub,
    pub started: Instant,
}

impl Shared {
    pub fn new(data_dir: PathBuf, started: Instant) -> Result<Self> {
        std::fs::create_dir_all(&data_dir)?;
        let db = Db::open(&data_dir.join("enkidu.db"))?;
        let user: UserSettings = settings::load(&settings::user_settings_path(&data_dir)).unwrap_or_default();
        Ok(Self {
            data_dir,
            db,
            user: Mutex::new(user),
            statuses: Mutex::new(HashMap::new()),
            hooks: Mutex::new(None),
            pty: PtyManager::default(),
            changes: Arc::new(ChangesHub::default()),
            file_lists: FileLists::default(),
            text_running: Running::default(),
            last_text: Mutex::new(HashMap::new()),
            symbols: SymbolIndexes::default(),
            servers: crate::lsp::Servers::default(),
            work: crate::work::Hub::default(),
            started,
        })
    }

    pub fn user_settings(&self) -> UserSettings {
        self.user.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn save_user_settings(&self, value: UserSettings) -> Result<()> {
        value.validate()?;
        settings::save(&settings::user_settings_path(&self.data_dir), &value)?;
        *self.user.lock().unwrap_or_else(|e| e.into_inner()) = value;
        Ok(())
    }

    pub fn project_settings(&self, project_id: &str) -> Result<ProjectSettings> {
        settings::load(&settings::project_settings_path(&self.data_dir, project_id))
    }

    pub fn save_project_settings(&self, project_id: &str, value: &ProjectSettings) -> Result<()> {
        value.validate()?;
        settings::save(&settings::project_settings_path(&self.data_dir, project_id), value)
    }

    pub fn statuses(&self) -> HashMap<String, String> {
        self.statuses.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn status(&self, workspace_id: &str) -> Option<String> {
        self.statuses.lock().unwrap_or_else(|e| e.into_inner()).get(workspace_id).cloned()
    }

    pub fn set_status(&self, app: &Events, workspace_id: &str, status: &str) {
        if !STATUSES.contains(&status) {
            return;
        }
        let changed = {
            let mut map = self.statuses.lock().unwrap_or_else(|e| e.into_inner());
            map.insert(workspace_id.to_string(), status.to_string()).as_deref() != Some(status)
        };
        if changed {
            app.send(Event::StatusChanged { workspace_id: workspace_id.to_string(), status: status.to_string() });
        }
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.data_dir.join("logs")
    }
}
