//! Settings live in plain TOML files: one for the user, and one per project, all inside the
//! app's data folder (nothing is written into the user's repos).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::shellenv::expand_tilde;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct UserSettings {
    pub workspaces_dir: String,
    pub claude: UserClaude,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct UserClaude {
    pub command: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct ProjectSettings {
    /// Branch new workspaces start from. Empty means: development if it exists, else origin's default.
    pub default_base: String,
    /// Files or globs (relative to the main folder) copied into every new workspace when missing there.
    pub copy_files: Vec<String>,
    pub setup_script: String,
    pub live: LiveSettings,
    pub claude: ProjectClaude,
    pub work: WorkSettings,
    pub rail: RailSettings,
}

/// The project's button on the rail: its own icon, or its letter.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct RailSettings {
    /// Show the project's own icon when it has one. Off: its letter.
    pub icon: bool,
    /// Another picture than the one found in the project's folder: a path from that folder, or a whole one. Empty: the
    /// one found there.
    pub picture: String,
}

impl Default for RailSettings {
    fn default() -> Self {
        Self { icon: true, picture: String::new() }
    }
}

/// The Work index for this project: its PRs on GitHub and its tickets in Jira.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct WorkSettings {
    /// Read this project's PRs from GitHub (through `gh`).
    pub github: bool,
    /// The `gh` account that reads the repo; empty means the first one that can.
    pub github_account: String,
    /// Jira project keys whose tickets belong here (`SH`); none means Jira is off for this project.
    pub jira_keys: Vec<String>,
    /// Jira statuses that mean the ticket came back to you; none means: moved back to an earlier status.
    pub back_to_you: Vec<String>,
    /// Checks that never count as a failed pipeline.
    pub ignored_checks: Vec<String>,
    /// Minutes between reads.
    pub every_minutes: u32,
    /// Workflow: new failing checks on a PR whose session is in a workspace go to that session.
    pub send_failed_checks: bool,
    /// Workflow: every morning, a read-only Claude in the main folder writes one line per ticket.
    pub morning_summary: bool,
    /// When the morning summary runs (local time, "09:00").
    pub morning_at: String,
}

impl Default for WorkSettings {
    fn default() -> Self {
        Self {
            github: true,
            github_account: String::new(),
            jira_keys: Vec::new(),
            back_to_you: Vec::new(),
            ignored_checks: Vec::new(),
            every_minutes: 5,
            send_failed_checks: false,
            morning_summary: false,
            morning_at: "09:00".into(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct LiveSettings {
    /// Runs in the main folder after a workspace's changes are copied in (going Live). `{path}` is the workspace.
    pub on: String,
    /// Runs in the main folder after it is put back (Stop, or before another workspace goes Live).
    pub off: String,
    /// Told to Claude in the Live workspace with every prompt.
    pub message: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct ProjectClaude {
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
}

impl Default for UserSettings {
    fn default() -> Self {
        Self {
            workspaces_dir: crate::places::WORKSPACES_DIR.into(),
            claude: UserClaude::default(),
        }
    }
}

impl Default for UserClaude {
    fn default() -> Self {
        Self { command: "claude".into(), args: Vec::new(), env: BTreeMap::new() }
    }
}

impl Default for ProjectSettings {
    fn default() -> Self {
        Self {
            default_base: String::new(),
            copy_files: vec![
                ".env".into(),
                ".env.local".into(),
                ".claude/settings.local.json".into(),
                "CLAUDE.local.md".into(),
            ],
            setup_script: String::new(),
            live: LiveSettings::default(),
            claude: ProjectClaude::default(),
            work: WorkSettings::default(),
            rail: RailSettings::default(),
        }
    }
}

impl Default for LiveSettings {
    fn default() -> Self {
        Self {
            on: String::new(),
            off: String::new(),
            message: "This workspace is Live: your changes here are copied into the project's main folder as you \
                      make them, and the dev environment runs from there, so you can test them there."
                .into(),
        }
    }
}

impl UserSettings {
    pub fn workspaces_root(&self) -> PathBuf {
        expand_tilde(self.workspaces_dir.trim())
    }

    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(!self.workspaces_dir.trim().is_empty(), "The workspaces folder can't be empty");
        anyhow::ensure!(!self.claude.command.trim().is_empty(), "The claude command can't be empty");
        validate_env(&self.claude.env)
    }
}

impl ProjectSettings {
    pub fn validate(&self) -> Result<()> {
        for key in &self.work.jira_keys {
            let ok = key.len() >= 2 && key.len() <= 10 && key.starts_with(|c: char| c.is_ascii_uppercase())
                && key.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
            anyhow::ensure!(ok, "\"{key}\" isn't a Jira project key (like SH)");
        }
        anyhow::ensure!((1..=240).contains(&self.work.every_minutes), "Read every 1 to 240 minutes");
        anyhow::ensure!(
            crate::work::workflows::minutes_of(&self.work.morning_at).is_some(),
            "The morning summary's time is a time of day like 09:00"
        );
        validate_env(&self.claude.env)
    }
}

fn validate_env(env: &BTreeMap<String, String>) -> Result<()> {
    for key in env.keys() {
        let ok = !key.is_empty()
            && !key.starts_with(|c: char| c.is_ascii_digit())
            && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        anyhow::ensure!(ok, "\"{key}\" isn't a valid environment variable name");
    }
    Ok(())
}

/// The claude program, its extra args (user first, then project) and its env (project wins).
pub struct ClaudeLaunch {
    pub command: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
}

pub fn claude_launch(user: &UserSettings, project: &ProjectSettings) -> ClaudeLaunch {
    let mut env = user.claude.env.clone();
    env.extend(project.claude.env.clone());
    ClaudeLaunch {
        command: user.claude.command.trim().to_string(),
        args: user.claude.args.iter().chain(project.claude.args.iter()).cloned().collect(),
        env,
    }
}

pub fn load<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> Result<T> {
    match std::fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text).with_context(|| format!("Can't read settings in {}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(e).with_context(|| format!("Can't open {}", path.display())),
    }
}

pub fn save<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = toml::to_string_pretty(value)?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

pub fn user_settings_path(data_dir: &Path) -> PathBuf {
    data_dir.join("settings.toml")
}

pub fn project_settings_path(data_dir: &Path, project_id: &str) -> PathBuf {
    data_dir.join("projects").join(format!("{project_id}.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_gives_defaults_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        let loaded: UserSettings = load(&path).unwrap();
        assert_eq!(loaded, UserSettings::default());

        let mut changed = loaded.clone();
        changed.workspaces_dir = "~/work/trees".into();
        changed.claude.args = vec!["--model".into(), "opus".into()];
        changed.claude.env.insert("CLAUDE_CONFIG_DIR".into(), "~/.claude-work".into());
        save(&path, &changed).unwrap();
        let again: UserSettings = load(&path).unwrap();
        assert_eq!(again, changed);
    }

    #[test]
    fn partial_files_fill_in_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.toml");
        std::fs::write(&path, "setup_script = \"make setup\"\n[live]\non = \"echo {path}\"\n").unwrap();
        let p: ProjectSettings = load(&path).unwrap();
        assert_eq!(p.setup_script, "make setup");
        assert_eq!(p.live.on, "echo {path}");
        assert!(p.copy_files.contains(&".env".to_string()));
        assert!(!p.live.message.is_empty());
    }

    #[test]
    fn workflows_are_off_until_turned_on_and_their_time_is_checked() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.toml");
        // a project's settings from before the workflows
        std::fs::write(&path, "[work]\ngithub = true\njira_keys = [\"SH\"]\nevery_minutes = 5\n").unwrap();
        let mut p: ProjectSettings = load(&path).unwrap();
        assert_eq!((p.work.send_failed_checks, p.work.morning_summary, p.work.morning_at.as_str()), (false, false, "09:00"));
        p.validate().unwrap();
        p.work.morning_at = "9am".into();
        assert!(p.validate().unwrap_err().to_string().contains("like 09:00"));
        p.work.morning_at = "07:30".into();
        p.work.morning_summary = true;
        save(&path, &p).unwrap();
        let back: ProjectSettings = load(&path).unwrap();
        assert_eq!((back.work.morning_summary, back.work.morning_at.as_str()), (true, "07:30"));
    }

    #[test]
    fn broken_files_are_errors_not_silent_resets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.toml");
        std::fs::write(&path, "workspaces_dir = [").unwrap();
        assert!(load::<UserSettings>(&path).is_err());
    }

    #[test]
    fn launch_merges_args_in_order_and_env_with_project_winning() {
        let mut user = UserSettings::default();
        user.claude.args = vec!["--model".into(), "opus".into()];
        user.claude.env.insert("A".into(), "user".into());
        user.claude.env.insert("B".into(), "user".into());
        let mut project = ProjectSettings::default();
        project.claude.args = vec!["--permission-mode".into(), "acceptEdits".into()];
        project.claude.env.insert("B".into(), "project".into());
        let launch = claude_launch(&user, &project);
        assert_eq!(launch.args, ["--model", "opus", "--permission-mode", "acceptEdits"]);
        assert_eq!(launch.env["A"], "user");
        assert_eq!(launch.env["B"], "project");
    }

    #[test]
    fn settings_from_older_versions_still_load() {
        // v1 had a theme setting; the app is dark only now, and the old key is simply ignored
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        std::fs::write(&path, "theme = \"light\"\nworkspaces_dir = \"~/trees\"\n").unwrap();
        let user: UserSettings = load(&path).unwrap();
        assert_eq!(user.workspaces_dir, "~/trees");
        assert_eq!(user.claude, UserClaude::default());
    }

    #[test]
    fn validation_rejects_bad_values() {
        let mut user = UserSettings::default();
        user.workspaces_dir = "  ".into();
        assert!(user.validate().is_err());
        let mut user = UserSettings::default();
        user.claude.env.insert("BAD-NAME".into(), "x".into());
        assert!(user.validate().is_err());
        assert!(UserSettings::default().validate().is_ok());
    }
}
