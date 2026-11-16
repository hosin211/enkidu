//! Creating and removing workspaces: one git worktree each.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::db::{now_ms, Project, Workspace};
use crate::git;
use crate::script;
use crate::settings::ProjectSettings;

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CreateRequest {
    pub project_id: String,
    pub name: String,
    /// "new" (new branch from `base`), "existing" (check out `branch` as it is), or "copy" (new branch starting where
    /// the branch `base` is now: that branch is checked out in another folder, and git checks a branch out once)
    pub mode: String,
    pub branch: String,
    pub base: Option<String>,
    /// Resume this Claude session in the new workspace (one that ran outside Enkidu) instead of a new one
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub project_id: String,
    pub step: String,
    /// running, done, warning, skipped or error
    pub state: String,
    pub detail: String,
}

pub struct CreateEnv<'a> {
    pub workspaces_root: PathBuf,
    pub project: &'a Project,
    pub settings: &'a ProjectSettings,
    pub log_dir: PathBuf,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Created {
    pub workspace: Workspace,
    pub warnings: Vec<String>,
}

/// The branch asked for is checked out in another folder, and git checks a branch out in one folder only. The UI
/// offers that folder as a workspace, or a new branch starting where this one is.
#[derive(Debug)]
pub struct BranchTaken {
    pub branch: String,
    pub path: PathBuf,
}

impl std::fmt::Display for BranchTaken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} is checked out in {}", self.branch, self.path.display())
    }
}

impl std::error::Error for BranchTaken {}

pub fn validate_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name.starts_with(|c: char| c.is_ascii_alphanumeric())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if !ok {
        bail!("Use letters, numbers, dots, dashes or underscores (start with a letter or number, 64 at most)");
    }
    Ok(())
}

/// A folder-safe version of the project's name.
pub fn project_slug(name: &str) -> String {
    let slug: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '-' })
        .collect();
    if slug.trim_matches('-').is_empty() {
        "project".into()
    } else {
        slug
    }
}

pub fn workspace_path(root: &Path, project: &Project, name: &str) -> PathBuf {
    root.join(project_slug(&project.name)).join(name)
}

pub fn script_env(main: &Path, ws_path: &Path, name: &str, branch: &str) -> Vec<(String, String)> {
    vec![
        ("ENKIDU_WORKSPACE_PATH".into(), ws_path.to_string_lossy().into()),
        ("ENKIDU_MAIN_PATH".into(), main.to_string_lossy().into()),
        ("ENKIDU_WORKSPACE_NAME".into(), name.into()),
        ("ENKIDU_BRANCH".into(), branch.into()),
    ]
}

pub fn create(env: &CreateEnv, req: &CreateRequest, progress: &dyn Fn(&str, &str, &str)) -> Result<Created> {
    let main = PathBuf::from(&env.project.path);
    let name = req.name.trim();
    let branch = req.branch.trim();
    let mut warnings = Vec::new();

    progress("check", "running", "Checking the name and branch");
    validate_name(name)?;
    let path = workspace_path(&env.workspaces_root, env.project, name);
    if path.exists() {
        bail!("{} already exists", path.display());
    }
    let new_branch = match req.mode.as_str() {
        "new" | "copy" => true,
        "existing" => false,
        other => bail!("Unknown mode \"{other}\""),
    };
    if !git::valid_branch_name(&main, branch) {
        bail!("\"{branch}\" isn't a valid branch name");
    }
    if new_branch && git::local_branch_exists(&main, branch) {
        bail!("The branch \"{branch}\" already exists. Pick \"Existing branch\" to work on it.");
    }
    if !new_branch {
        // a folder deleted by hand still holds its branch for git until it is pruned
        let _ = git::git_ok(&main, &["worktree", "prune"]);
        if let Some(path) = git::checked_out_in(&main, branch) {
            return Err(BranchTaken { branch: branch.to_string(), path }.into());
        }
    }
    progress("check", "done", "");

    if git::has_origin(&main) {
        progress("fetch", "running", "Fetching from origin");
        match git::fetch_origin(&main) {
            Ok(()) => progress("fetch", "done", ""),
            Err(e) => {
                let msg = format!("Couldn't fetch from origin, using the branches already here ({e})");
                progress("fetch", "warning", &msg);
                warnings.push(msg);
            }
        }
    } else {
        progress("fetch", "skipped", "This repo has no origin");
    }

    progress("worktree", "running", "Creating the worktree");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("Can't create {}", parent.display()))?;
    }
    let base = if req.mode == "copy" {
        let from = req.base.as_deref().map(str::trim).filter(|b| !b.is_empty()).context("Which branch should it start from?")?;
        let start = git::worktree_add_from(&main, &path, branch, from)?;
        progress("worktree", "done", &format!("New branch {branch} from {start}"));
        Some(from.to_string())
    } else if new_branch {
        let base = req
            .base
            .clone()
            .filter(|b| !b.trim().is_empty())
            .or_else(|| git::default_base(&main, &env.settings.default_base))
            .context("Pick a base branch")?;
        let start = git::worktree_add_new(&main, &path, branch, base.trim())?;
        progress("worktree", "done", &format!("New branch {branch} from {start}"));
        Some(base.trim().to_string())
    } else {
        git::worktree_add_existing(&main, &path, branch)?;
        progress("worktree", "done", &format!("Checked out {branch}"));
        None
    };

    let (copied, missing) = copy_files(&main, &path, &env.settings.copy_files);
    let detail = if copied.is_empty() { "Nothing to copy".to_string() } else { format!("Copied {}", copied.join(", ")) };
    progress("copy", if copied.is_empty() { "skipped" } else { "done" }, &detail);
    let _ = missing;

    let id = uuid::Uuid::new_v4().to_string();
    let script = env.settings.setup_script.trim();
    if script.is_empty() {
        progress("setup", "skipped", "No setup script");
    } else {
        progress("setup", "running", "Running the setup script");
        let log = env.log_dir.join(format!("setup-{id}.log"));
        let result = script::run(
            script,
            &path,
            &script_env(&main, &path, name, branch),
            Duration::from_secs(15 * 60),
            Some(&log),
        );
        if result.ok {
            progress("setup", "done", "");
        } else {
            let why = if result.timed_out {
                "timed out after 15 minutes".to_string()
            } else {
                format!("exited with {}", result.code.map(|c| c.to_string()).unwrap_or_else(|| "a signal".into()))
            };
            let msg = format!("The setup script {why}. The workspace is ready, check the script.\n{}", result.output);
            progress("setup", "warning", &msg);
            warnings.push(msg);
        }
    }

    let workspace = Workspace {
        id,
        project_id: env.project.id.clone(),
        name: name.to_string(),
        path: path.canonicalize().unwrap_or(path).to_string_lossy().into(),
        branch: branch.to_string(),
        base,
        session_id: req.session_id.clone().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        transcript_path: None,
        claude_config_dir: None,
        created_at: now_ms(),
        adopted: false,
    };
    progress("done", "done", "");
    Ok(Created { workspace, warnings })
}

/// A folder name as a workspace name: what `validate_name` allows, not one of `taken` (else with -2, -3…).
pub fn free_name(folder: &str, taken: &[String]) -> String {
    let cleaned: String = folder.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '-' }).collect();
    let mut base: String = cleaned.trim_start_matches(|c: char| !c.is_ascii_alphanumeric()).chars().take(58).collect();
    if base.is_empty() {
        base = "workspace".into();
    }
    let mut name = base.clone();
    for n in 2.. {
        if !taken.iter().any(|t| t == &name) {
            break;
        }
        name = format!("{base}-{n}");
    }
    name
}

/// Opens a folder Enkidu didn't make as a workspace: a checkout of the project's repo that has the branch you
/// wanted (a folder another tool made, a worktree of your own, or the main folder). Nothing in the folder changes: no
/// files are copied, no setup script runs, and removing the workspace later only forgets it.
/// It takes the name you gave (when it is a free, valid name), else the folder's.
pub fn adopt(project: &Project, taken: &[String], folder: &Path, session_id: Option<String>, name: Option<&str>) -> Result<Workspace> {
    if !folder.is_dir() {
        bail!("{} isn't there any more", folder.display());
    }
    let root = git::toplevel(folder)?;
    if !git::same_repo(Path::new(&project.path), &root) {
        bail!("{} isn't a checkout of {}", root.display(), project.name);
    }
    let branch = git::current_branch(&root).with_context(|| format!("{} isn't on a branch", root.display()))?;
    let folder_name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let name = match name.map(str::trim) {
        Some(n) if validate_name(n).is_ok() && !taken.iter().any(|t| t == n) => n.to_string(),
        _ => free_name(&folder_name, taken),
    };
    Ok(Workspace {
        id: uuid::Uuid::new_v4().to_string(),
        project_id: project.id.clone(),
        name,
        path: root.to_string_lossy().into(),
        branch,
        base: None,
        session_id: session_id.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        transcript_path: None,
        claude_config_dir: None,
        created_at: now_ms(),
        adopted: true,
    })
}

/// Copies each listed file or glob from the main folder when the workspace doesn't have it yet.
/// Returns (copied, not found).
pub fn copy_files(main: &Path, ws: &Path, patterns: &[String]) -> (Vec<String>, Vec<String>) {
    let mut copied = Vec::new();
    let mut missing = Vec::new();
    for pattern in patterns.iter().map(|p| p.trim()).filter(|p| !p.is_empty()) {
        if pattern.starts_with('/') || pattern.split('/').any(|part| part == "..") {
            missing.push(pattern.to_string());
            continue;
        }
        let sources: Vec<PathBuf> = if pattern.contains(['*', '?', '[']) {
            glob::glob(&main.join(pattern).to_string_lossy())
                .map(|paths| paths.filter_map(|p| p.ok()).collect())
                .unwrap_or_default()
        } else {
            let p = main.join(pattern);
            if p.exists() {
                vec![p]
            } else {
                vec![]
            }
        };
        if sources.is_empty() {
            missing.push(pattern.to_string());
        }
        for src in sources {
            let Ok(rel) = src.strip_prefix(main) else { continue };
            if copy_tree(&src, &ws.join(rel)) {
                copied.push(rel.to_string_lossy().into_owned());
            }
        }
    }
    (copied, missing)
}

fn copy_tree(src: &Path, dest: &Path) -> bool {
    if src.is_dir() {
        let mut any = false;
        if let Ok(entries) = std::fs::read_dir(src) {
            for entry in entries.flatten() {
                any |= copy_tree(&entry.path(), &dest.join(entry.file_name()));
            }
        }
        any
    } else if dest.exists() {
        false
    } else {
        dest.parent().map(std::fs::create_dir_all);
        std::fs::copy(src, dest).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testutil::repo_with_origin;

    fn project(main: &Path) -> Project {
        Project {
            id: "p1".into(),
            name: "my shop".into(),
            path: main.to_string_lossy().into(),
            live_workspace_id: None,
            created_at: 0,
        }
    }

    #[test]
    fn names_and_slugs() {
        assert!(validate_name("sh-21309_fix.2").is_ok());
        for bad in ["", "-x", "a b", "x/y", ".hidden", &"a".repeat(65)] {
            assert!(validate_name(bad).is_err(), "{bad} should be rejected");
        }
        assert_eq!(project_slug("my shop"), "my-shop");
        assert_eq!(project_slug("shop.example"), "shop.example");
        assert_eq!(project_slug("///"), "project");
    }

    #[test]
    fn creates_a_workspace_with_files_and_setup_script() {
        let tmp = tempfile::tempdir().unwrap();
        let (_origin, main) = repo_with_origin(tmp.path());
        std::fs::write(main.join(".env"), "SECRET=1\n").unwrap();
        std::fs::create_dir_all(main.join(".claude")).unwrap();
        std::fs::write(main.join(".claude/settings.local.json"), "{}").unwrap();
        let project = project(&main);
        let mut settings = ProjectSettings::default();
        settings.setup_script = "echo \"$ENKIDU_BRANCH\" > setup-ran.txt".into();
        let env = CreateEnv {
            workspaces_root: tmp.path().join("ws"),
            project: &project,
            settings: &settings,
            log_dir: tmp.path().join("logs"),
        };
        let steps = std::cell::RefCell::new(Vec::new());
        let req = CreateRequest {
            project_id: "p1".into(),
            name: "one".into(),
            mode: "new".into(),
            branch: "feature/one".into(),
            base: None,
            session_id: None,
        };
        let created = create(&env, &req, &|step, state, _| steps.borrow_mut().push(format!("{step}:{state}"))).unwrap();
        let ws = &created.workspace;
        assert!(created.warnings.is_empty(), "{:?}", created.warnings);
        assert_eq!(ws.base.as_deref(), Some("development"));
        let path = PathBuf::from(&ws.path);
        assert!(path.ends_with("ws/my-shop/one"));
        assert_eq!(std::fs::read_to_string(path.join(".env")).unwrap(), "SECRET=1\n");
        assert!(path.join(".claude/settings.local.json").exists());
        assert_eq!(std::fs::read_to_string(path.join("setup-ran.txt")).unwrap().trim(), "feature/one");
        assert_eq!(git::current_branch(&path).as_deref(), Some("feature/one"));
        assert!(steps.borrow().contains(&"fetch:done".to_string()));
        assert!(steps.borrow().contains(&"setup:done".to_string()));

        // same name again is refused, and so is an existing branch in "new" mode
        assert!(create(&env, &req, &|_, _, _| {}).is_err());
        let again = CreateRequest { name: "two".into(), ..req.clone() };
        let err = create(&env, &again, &|_, _, _| {}).unwrap_err().to_string();
        assert!(err.contains("already exists"), "{err}");

        // an existing branch from origin, and a failing setup script is only a warning
        let failing = ProjectSettings { setup_script: "exit 7".into(), ..settings.clone() };
        let env = CreateEnv { settings: &failing, ..env };
        let existing = CreateRequest {
            project_id: "p1".into(),
            name: "remote".into(),
            mode: "existing".into(),
            branch: "feature/remote-only".into(),
            base: None,
            session_id: None,
        };
        let created = create(&env, &existing, &|_, _, _| {}).unwrap();
        assert_eq!(created.workspace.base, None);
        assert_eq!(created.warnings.len(), 1);
        assert!(created.warnings[0].contains("exited with 7"));
    }

    #[test]
    fn a_branch_checked_out_elsewhere_names_its_folder_and_can_start_a_new_branch_or_be_opened_there() {
        let tmp = tempfile::tempdir().unwrap();
        let (_origin, main) = repo_with_origin(tmp.path());
        let project = project(&main);
        let settings = ProjectSettings::default();
        let env = CreateEnv { workspaces_root: tmp.path().join("ws"), project: &project, settings: &settings, log_dir: tmp.path().join("logs") };
        // the branch is checked out in a folder Enkidu didn't make, as a folder another tool made can be
        let elsewhere = tmp.path().join("elsewhere/harbor");
        git::worktree_add_existing(&main, &elsewhere, "feature/remote-only").unwrap();
        let req = CreateRequest {
            project_id: "p1".into(),
            name: "remote".into(),
            mode: "existing".into(),
            branch: "feature/remote-only".into(),
            base: None,
            session_id: None,
        };
        let err = create(&env, &req, &|_, _, _| {}).unwrap_err();
        let taken = err.downcast_ref::<BranchTaken>().expect("a BranchTaken");
        assert_eq!((taken.branch.as_str(), taken.path.clone()), ("feature/remote-only", elsewhere.canonicalize().unwrap()));
        assert!(!env.workspaces_root.join("my-shop/remote").exists(), "nothing made");

        // a new branch, starting where that one is
        let copy = CreateRequest { mode: "copy".into(), branch: "feature/remote-only-2".into(), base: Some("feature/remote-only".into()), ..req.clone() };
        let created = create(&env, &copy, &|_, _, _| {}).unwrap();
        assert_eq!(created.workspace.base.as_deref(), Some("feature/remote-only"));
        assert!(!created.workspace.adopted);
        let path = PathBuf::from(&created.workspace.path);
        assert_eq!(git::current_branch(&path).as_deref(), Some("feature/remote-only-2"));
        let tip = |dir: &Path, rev: &str| git::git_ok(dir, &["rev-parse", rev]).unwrap();
        assert_eq!(tip(&path, "HEAD"), tip(&elsewhere, "HEAD"));
        let again = CreateRequest { name: "remote-3".into(), ..copy.clone() };
        assert!(create(&env, &again, &|_, _, _| {}).unwrap_err().to_string().contains("already exists"));

        // or that folder itself, as it is
        let ws = adopt(&project, &["harbor".into()], &elsewhere.join("."), Some("s-1".into()), None).unwrap();
        assert_eq!((ws.name.as_str(), ws.branch.as_str(), ws.session_id.as_str(), ws.adopted, ws.base.as_deref()), ("harbor-2", "feature/remote-only", "s-1", true, None));
        assert_eq!(PathBuf::from(&ws.path), elsewhere.canonicalize().unwrap());
        // the name you gave, when it is free
        let named = adopt(&project, &["taken".into()], &elsewhere, None, Some("sh-21349-api")).unwrap();
        assert_eq!(named.name, "sh-21349-api");
        assert_eq!(adopt(&project, &["taken".into()], &elsewhere, None, Some("taken")).unwrap().name, "harbor");
        assert_eq!(adopt(&project, &[], &elsewhere, None, Some("no spaces")).unwrap().name, "harbor");
        // the main folder is the project's too; another repo isn't
        assert_eq!(adopt(&project, &[], &main, None, None).unwrap().branch, "main");
        let other = tmp.path().join("other");
        std::fs::create_dir_all(&other).unwrap();
        git::git_ok(&other, &["init", "-q"]).unwrap();
        assert!(adopt(&project, &[], &other, None, None).unwrap_err().to_string().contains("isn't a checkout of my shop"));
        assert!(adopt(&project, &[], &tmp.path().join("gone"), None, None).is_err());
    }

    #[test]
    fn folder_names_become_free_workspace_names() {
        assert_eq!(free_name("harbor", &[]), "harbor");
        assert_eq!(free_name("harbor", &["harbor".into(), "harbor-2".into()]), "harbor-3");
        assert_eq!(free_name(".worktrees x", &[]), "worktrees-x");
        assert_eq!(free_name("...", &[]), "workspace");
        assert!(validate_name(&free_name(&"ü".repeat(80), &[])).is_ok());
    }

    #[test]
    fn copy_files_skips_existing_missing_and_unsafe_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let main = tmp.path().join("main");
        let ws = tmp.path().join("ws");
        std::fs::create_dir_all(main.join("conf")).unwrap();
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::write(main.join(".env"), "new").unwrap();
        std::fs::write(ws.join(".env"), "keep").unwrap();
        std::fs::write(main.join("conf/a.local"), "a").unwrap();
        std::fs::write(main.join("conf/b.local"), "b").unwrap();
        let patterns: Vec<String> =
            [".env", "conf/*.local", "nope.txt", "../outside", "/etc/hosts"].iter().map(|s| s.to_string()).collect();
        let (copied, missing) = copy_files(&main, &ws, &patterns);
        assert_eq!(std::fs::read_to_string(ws.join(".env")).unwrap(), "keep", "never overwrites");
        assert_eq!(copied, ["conf/a.local", "conf/b.local"]);
        assert_eq!(missing, ["nope.txt", "../outside", "/etc/hosts"]);
    }
}
