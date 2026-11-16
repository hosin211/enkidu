//! The commands the UI calls.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use crate::events::{Event, Events};

use crate::db::{now_ms, Project, Remark, Workspace};
use crate::git;
use crate::goto;
use crate::lsp;
use crate::hooks;
use crate::live::{self, LiveOutcome};
use crate::pty::{self, SpawnSpec, TerminalInfo};
use crate::redact::redact_urls;
use crate::search;
use crate::symbols;
use crate::settings::{self, ProjectSettings, UserSettings};
use crate::shellenv::{self, expand_tilde};
use crate::state::Shared;
use crate::workspace::{self, CreateEnv, CreateRequest, Created, Progress};

#[derive(Debug, Serialize)]
pub struct CmdError {
    pub code: String,
    pub message: String,
    /// The folder the error is about ("branch_taken": the one the branch is checked out in)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

impl CmdError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self { code: code.into(), message: message.into(), path: None }
    }
}

impl From<anyhow::Error> for CmdError {
    fn from(e: anyhow::Error) -> Self {
        if let Some(taken) = e.downcast_ref::<workspace::BranchTaken>() {
            let path = taken.path.to_string_lossy().to_string();
            return Self { code: "branch_taken".into(), message: taken.to_string(), path: Some(path) };
        }
        Self::new("error", redact_urls(&format!("{e:#}")))
    }
}

pub type CmdResult<T> = Result<T, CmdError>;

/// Runs a command's work. Every command is a plain function the app calls off its drawing thread (item 1.9), so
/// the work runs where it is called.
fn blocking<T>(f: impl FnOnce() -> CmdResult<T>) -> CmdResult<T> {
    f()
}

fn workspace(shared: &Shared, id: &str) -> CmdResult<Workspace> {
    shared.db.workspace(id)?.ok_or_else(|| CmdError::new("not_found", "That workspace is gone"))
}

fn project(shared: &Shared, id: &str) -> CmdResult<Project> {
    shared.db.project(id)?.ok_or_else(|| CmdError::new("not_found", "That project is gone"))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStateDto {
    pub projects: Vec<Project>,
    pub workspaces: Vec<Workspace>,
    pub statuses: HashMap<String, String>,
    pub settings: UserSettings,
    /// The changes the watcher already knows, per workspace (the others arrive as they are worked out).
    pub changes: HashMap<String, Vec<git::Change>>,
}

/// The window has its first full state: how long the start took, kept in `startup.log` (showing the window is the
/// app's own work now).
pub fn app_ready(state: &Arc<Shared>) -> u128 {
    let ms = state.started.elapsed().as_millis();
    let _ = std::fs::write(state.data_dir.join("startup.log"), format!("{ms}\n"));
    println!("ENKIDU_STARTUP_MS={ms}");
    ms
}

pub fn get_state(state: &Arc<Shared>) -> CmdResult<AppStateDto> {
    let shared = state.clone();
    blocking(move || {
        Ok(AppStateDto {
            projects: shared.db.projects()?,
            workspaces: shared.db.workspaces()?,
            statuses: shared.statuses(),
            settings: shared.user_settings(),
            changes: shared.changes.snapshot(),
        })
    })
}

pub fn add_project(state: &Arc<Shared>, path: String) -> CmdResult<Project> {
    let shared = state.clone();
    blocking(move || {
        let picked = expand_tilde(path.trim());
        let root = git::toplevel(&picked).map_err(|_| CmdError::new("not_git", "That folder isn't a git repository"))?;
        if !git::is_main_worktree(&root)? {
            return Err(CmdError::new("worktree", "That's a linked worktree. Pick the project's main folder."));
        }
        let root_str = root.to_string_lossy().to_string();
        if shared.db.project_by_path(&root_str)?.is_some() {
            return Err(CmdError::new("exists", "That project is already here"));
        }
        let name = root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "project".into());
        let project = Project {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            path: root_str,
            live_workspace_id: None,
            created_at: now_ms(),
        };
        shared.db.insert_project(&project)?;
        shared.save_project_settings(&project.id, &ProjectSettings::default())?;
        Ok(project)
    })
}

pub fn remove_project(state: &Arc<Shared>, project_id: String) -> CmdResult<()> {
    let shared = state.clone();
    blocking(move || {
        if shared.db.workspaces()?.iter().any(|w| w.project_id == project_id) {
            return Err(CmdError::new("has_workspaces", "Remove the project's workspaces first"));
        }
        shared.db.delete_project(&project_id)?;
        // its own tasks go with it (they belong to no other project)
        shared.db.work_delete_own_tasks_of(&project_id)?;
        let _ = std::fs::remove_file(settings::project_settings_path(&shared.data_dir, &project_id));
        Ok(())
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Branches {
    pub local: Vec<String>,
    pub remote: Vec<String>,
    pub default_base: Option<String>,
    pub has_origin: bool,
}

pub fn list_branches(state: &Arc<Shared>, project_id: String) -> CmdResult<Branches> {
    let shared = state.clone();
    blocking(move || {
        let p = project(&shared, &project_id)?;
        let main = PathBuf::from(&p.path);
        let configured = shared.project_settings(&project_id)?.default_base;
        Ok(Branches {
            local: git::local_branches(&main),
            remote: git::remote_branches(&main),
            default_base: git::default_base(&main, &configured),
            has_origin: git::has_origin(&main),
        })
    })
}

pub fn fetch_project(state: &Arc<Shared>, project_id: String) -> CmdResult<()> {
    let shared = state.clone();
    blocking(move || {
        let p = project(&shared, &project_id)?;
        let main = PathBuf::from(&p.path);
        if git::has_origin(&main) {
            git::fetch_origin(&main)?;
        }
        Ok(())
    })
}

pub fn create_workspace(app: Events, state: &Arc<Shared>, req: CreateRequest) -> CmdResult<Created> {
    let shared = state.clone();
    blocking(move || create_workspace_now(&app, &shared, &req))
}

/// Makes a workspace (its worktree, the copied files, the setup script), saved and watched, saying each step.
pub fn create_workspace_now(app: &Events, shared: &Shared, req: &CreateRequest) -> CmdResult<Created> {
    let p = project(shared, &req.project_id)?;
    if shared.db.workspaces()?.iter().any(|w| w.project_id == p.id && w.name == req.name.trim()) {
        return Err(CmdError::new("exists", "This project already has a workspace with that name"));
    }
    let settings = shared.project_settings(&p.id)?;
    let env = CreateEnv {
        workspaces_root: shared.user_settings().workspaces_root(),
        project: &p,
        settings: &settings,
        log_dir: shared.logs_dir(),
    };
    let project_id = p.id.clone();
    let progress = |step: &str, st: &str, detail: &str| {
        app.send(Event::WorkspaceProgress(Progress {
                project_id: project_id.clone(),
                step: step.into(),
                state: st.into(),
                detail: redact_urls(detail),
        }));
    };
    let mut created = workspace::create(&env, req, &progress)?;
    bring_session(shared, &mut created.workspace, req.session_id.as_deref())?;
    shared.db.insert_workspace(&created.workspace)?;
    shared.changes.add(&created.workspace.id, Path::new(&created.workspace.path));
    crate::work::workspace_created(app, shared, &created.workspace);
    Ok(created)
}

/// A session from outside Enkidu comes along into a workspace: its conversation is copied next to the folder,
/// in its own Claude folder (account), so Claude resumes it there (a conversation already there stays as it is).
fn bring_session(shared: &Shared, ws: &mut Workspace, session: Option<&str>) -> CmdResult<()> {
    let Some(session) = session.filter(|s| !s.is_empty()) else { return Ok(()) };
    if let Some((config, from)) = shared.db.work_transcript_of(session)? {
        let to = transcript_in(Path::new(&config), ws);
        if !to.exists() {
            carry_over(Path::new(&from), &to).map_err(|e| CmdError::new("error", format!("Can't bring the conversation over: {e}")))?;
        }
        ws.claude_config_dir = Some(config);
        ws.transcript_path = Some(to.to_string_lossy().to_string());
    }
    Ok(())
}

/// Opens a folder Enkidu didn't make as a workspace (the branch you wanted is checked out there), named `name` when
/// that is free, with a new Claude or the session `session_id`. A folder that already is a workspace is that workspace.
pub fn adopt_workspace(
    app: Events,
    state: &Arc<Shared>,
    project_id: String,
    path: String,
    session_id: Option<String>,
    name: Option<String>,
) -> CmdResult<Created> {
    let shared = state.clone();
    blocking(move || {
        let p = project(&shared, &project_id)?;
        let folder = expand_tilde(path.trim());
        let real = folder.canonicalize().unwrap_or_else(|_| folder.clone());
        let all = shared.db.workspaces()?;
        if let Some(ws) = all.iter().find(|w| Path::new(&w.path).canonicalize().is_ok_and(|w| w == real)) {
            return Ok(Created { workspace: ws.clone(), warnings: Vec::new() });
        }
        let taken: Vec<String> = all.iter().filter(|w| w.project_id == p.id).map(|w| w.name.clone()).collect();
        let mut ws = workspace::adopt(&p, &taken, &folder, session_id.clone(), name.as_deref())?;
        bring_session(&shared, &mut ws, session_id.as_deref())?;
        shared.db.insert_workspace(&ws)?;
        shared.changes.add(&ws.id, Path::new(&ws.path));
        crate::work::workspace_created(&app, &shared, &ws);
        Ok(Created { workspace: ws, warnings: Vec::new() })
    })
}

/// The workspace's Claude starts in another conversation from now on: `session_id` (one a card is linked to, brought
/// over from where it ran), or a new one. The conversation it was in stays saved (`/resume` finds it). Refused while
/// Claude runs there: the window closes an idle one first.
pub fn switch_conversation(state: &Arc<Shared>, workspace_id: String, session_id: Option<String>) -> CmdResult<Workspace> {
    let shared = state.clone();
    blocking(move || {
        let mut ws = workspace(&shared, &workspace_id)?;
        if shared.pty.running_claude(&ws.id) {
            return Err(CmdError::new("claude_running", format!("Claude is running in {}. Close it first.", ws.name)));
        }
        let session = session_id.filter(|s| !s.trim().is_empty());
        ws.session_id = session.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        ws.transcript_path = None;
        bring_session(&shared, &mut ws, session.as_deref())?;
        shared.db.set_conversation(&ws.id, &ws.session_id, ws.transcript_path.as_deref(), ws.claude_config_dir.as_deref())?;
        Ok(ws)
    })
}

/// You sent a card's work to this session: it is linked to the card's ticket and PRs, or to the task of your own the
/// card is ("linked by you"), so the card goes to it next time too.
pub fn work_link_session(app: Events, state: &Arc<Shared>, subjects: Vec<String>, session_id: String) -> CmdResult<()> {
    let shared = state.clone();
    blocking(move || {
        let now = now_ms();
        let session = format!("session:{session_id}");
        for subject in subjects.iter().filter(|s| crate::work::model::is_card(s)) {
            shared.db.work_link(subject, &session, "you", "linked by you", now)?;
        }
        crate::work::changed(&app);
        Ok(())
    })
}

/// Saves a workflow you made or changed (scheduled when it has a start time), and says what it is now.
pub fn work_flow_save(app: Events, state: &Arc<Shared>, flow: crate::work::flows::Flow) -> CmdResult<crate::work::flows::Flow> {
    let shared = state.clone();
    blocking(move || {
        project(&shared, &flow.project_id)?;
        let saved = crate::work::flows::save(&shared.db, flow, now_ms())?;
        crate::work::changed(&app);
        Ok(saved)
    })
}

/// Starts a workflow now (its tasks start over).
pub fn work_flow_run_now(app: Events, state: &Arc<Shared>, id: String) -> CmdResult<()> {
    let shared = state.clone();
    blocking(move || {
        crate::work::flows::run_now(&shared.db, &id, now_ms())?;
        crate::work::changed(&app);
        // the runner starts it now, not on its next look
        crate::work::flows::wake();
        Ok(())
    })
}

/// Stops a workflow: one that runs stops after the task that runs (its Claude isn't stopped), a scheduled one is
/// kept without a time.
pub fn work_flow_stop(app: Events, state: &Arc<Shared>, id: String) -> CmdResult<()> {
    let shared = state.clone();
    blocking(move || {
        crate::work::flows::stop(&app, &shared, &id, now_ms())?;
        crate::work::changed(&app);
        Ok(())
    })
}

pub fn work_flow_delete(app: Events, state: &Arc<Shared>, id: String) -> CmdResult<()> {
    let shared = state.clone();
    blocking(move || {
        crate::work::flows::delete(&app, &shared, &id, now_ms())?;
        crate::work::changed(&app);
        Ok(())
    })
}

/// Your own default words for the kinds of workflow tasks (`came_back`: "Read QA's comment first…"), all of them at
/// once.
pub fn work_flow_kinds_save(app: Events, state: &Arc<Shared>, kinds: std::collections::BTreeMap<String, String>) -> CmdResult<()> {
    let shared = state.clone();
    blocking(move || {
        shared.db.work_set_flow_kinds(&kinds)?;
        crate::work::changed(&app);
        Ok(())
    })
}

/// What a new workflow starts from (Settings, Workflows): the minutes a task gets, and the time of day it starts at.
pub fn work_flow_defaults_save(app: Events, state: &Arc<Shared>, defaults: crate::work::flows::FlowDefaults) -> CmdResult<()> {
    let shared = state.clone();
    blocking(move || {
        shared.db.work_set_flow_defaults(&defaults)?;
        crate::work::changed(&app);
        Ok(())
    })
}

/// One of a report's screenshots, as its bytes (the window shows them as a picture).
pub fn work_report_image(state: &Arc<Shared>, id: i64, name: String) -> CmdResult<Vec<u8>> {
    let shared = state.clone();
    blocking(move || {
        let (bytes, _) = crate::work::reports::image(&shared.db, &shared.data_dir, id, &name)?;
        Ok(bytes)
    })
}

/// This session isn't the card's Claude (a link the index or Claude made was wrong): its links to the card's ticket
/// and PRs go, and stay gone (the index's rules don't make them again; you can link it again by sending to it).
pub fn work_unlink_session(app: Events, state: &Arc<Shared>, subjects: Vec<String>, session_id: String) -> CmdResult<()> {
    let shared = state.clone();
    blocking(move || {
        let now = now_ms();
        let session = format!("session:{session_id}");
        for subject in subjects.iter().filter(|s| crate::work::model::is_card(s)) {
            shared.db.work_unlink(subject, &session, now)?;
        }
        let _ = crate::work::relink(&shared);
        crate::work::changed(&app);
        Ok(())
    })
}

pub fn remove_workspace(app: Events, state: &Arc<Shared>, workspace_id: String, force: bool) -> CmdResult<()> {
    let shared = state.clone();
    blocking(move || {
        let ws = workspace(&shared, &workspace_id)?;
        let p = project(&shared, &ws.project_id)?;
        if p.live_workspace_id.as_deref() == Some(ws.id.as_str()) {
            return Err(CmdError::new("live", "This workspace is Live. Turn Live off first."));
        }
        let path = PathBuf::from(&ws.path);
        // a folder Enkidu didn't make is only forgotten: nothing in it is deleted
        if !ws.adopted && !force && path.exists() && git::is_dirty(&path).unwrap_or(false) {
            return Err(CmdError::new("dirty", "This workspace has uncommitted changes"));
        }
        shared.pty.close_workspace(&ws.id);
        shared.changes.remove(&ws.id);
        shared.symbols.remove(&ws.id);
        shared.servers.stop(&ws.id);
        shared.last_text.lock().unwrap_or_else(|e| e.into_inner()).remove(&ws.id);
        if !ws.adopted {
            git::worktree_remove(Path::new(&p.path), &path, force)?;
        }
        shared.db.delete_workspace(&ws.id)?;
        shared.statuses.lock().unwrap_or_else(|e| e.into_inner()).remove(&ws.id);
        let _ = std::fs::remove_file(shared.data_dir.join("hooks").join(format!("{}.json", ws.id)));
        crate::work::workspace_removed(&app, &shared, &ws.id);
        Ok(())
    })
}

/// The Work index: tickets, PRs, the sessions behind them, what waits on you, and each source's health.
pub fn work_snapshot(state: &Arc<Shared>) -> CmdResult<crate::work::model::Snapshot> {
    let shared = state.clone();
    blocking(move || Ok(crate::work::snapshot(&shared)?))
}

/// Reads every project's GitHub and Jira again now.
pub fn work_refresh(state: &Arc<Shared>) {
    for p in state.db.projects().unwrap_or_default() {
        state.work.want(&p.id);
    }
}

/// Marks an item done, snoozes it until a time, or opens it again; each goes into its history.
pub fn work_item_set(app: Events, state: &Arc<Shared>, id: String, item_state: String, snoozed_until: Option<i64>) -> CmdResult<()> {
    let shared = state.clone();
    blocking(move || {
        Ok(crate::work::set_item(&shared.db, &id, &item_state, snoozed_until, now_ms()).map(|_| crate::work::changed(&app))?)
    })
}

/// What the page sends to save a task of your own: a new one (no id) in a project, or one that is there.
#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OwnTaskDraft {
    pub id: Option<i64>,
    pub project_id: String,
    pub title: String,
    pub notes: String,
    /// `todo`, `doing` or `done`
    pub state: String,
}

/// How a state of a task of your own is said: "To do", "In progress", "Done".
pub fn own_state_words(state: &str) -> &'static str {
    match state {
        "doing" => "In progress",
        "done" => "Done",
        _ => "To do",
    }
}

/// Saves a task of your own (the Work board's New task, and its card): a new one, or the words and the state of one
/// that is there. Its timeline says when you made it and each time you gave it another state.
pub fn work_own_task_save(app: Events, state: &Arc<Shared>, draft: OwnTaskDraft) -> CmdResult<crate::work::model::OwnTask> {
    let shared = state.clone();
    blocking(move || {
        let now = now_ms();
        let before = match draft.id {
            Some(id) => shared.db.work_own_task(id)?,
            None => None,
        };
        let task = shared.db.work_save_own_task(draft.id, &draft.project_id, &draft.title, &draft.notes, &draft.state, now)?;
        match before {
            None => shared.db.work_event(&task.subject(), "own", "You made this task", now)?,
            Some(before) if before.state != task.state => {
                shared.db.work_event(&task.subject(), "own", &format!("You set it to {}", own_state_words(&task.state)), now)?
            }
            Some(_) => {}
        }
        crate::work::changed(&app);
        Ok(task)
    })
}

/// Deletes a task of your own, with what was kept about its card.
pub fn work_own_task_delete(app: Events, state: &Arc<Shared>, id: i64) -> CmdResult<()> {
    let shared = state.clone();
    blocking(move || {
        shared.db.work_delete_own_task(id)?;
        crate::work::changed(&app);
        Ok(())
    })
}

/// Hides cards from the Work board, or shows them again; each card's timeline says so.
pub fn work_hide(app: Events, state: &Arc<Shared>, cards: Vec<String>, hidden: bool) -> CmdResult<()> {
    let shared = state.clone();
    blocking(move || {
        let now = now_ms();
        shared.db.work_set_hidden(&cards, hidden, now)?;
        for card in &cards {
            // the timeline's subject: the ticket's key, or the PR's owner/repo#N
            let subject = card.strip_prefix("ticket:").or_else(|| card.strip_prefix("pr:")).unwrap_or(card);
            shared.db.work_event(subject, "hidden", if hidden { "Hidden from the board" } else { "Back on the board" }, now)?;
        }
        crate::work::changed(&app);
        Ok(())
    })
}

/// Jira's boards, read now, for the choice in the Work page's header.
pub fn work_boards(app: Events, state: &Arc<Shared>) -> CmdResult<Vec<crate::work::model::Board>> {
    let shared = state.clone();
    blocking(move || {
        let boards = crate::work::read_boards(&shared)?;
        crate::work::changed(&app);
        Ok(boards)
    })
}

/// You picked a Jira board for the Work page: only its tickets are on the page. None: everything read.
pub fn work_set_board(app: Events, state: &Arc<Shared>, board: Option<crate::work::model::Board>) -> CmdResult<()> {
    let shared = state.clone();
    blocking(move || Ok(crate::work::pick_board(&app, &shared, board)?))
}

/// "Check connections": every project's GitHub and Jira read now and compared with gh and twg directly.
pub fn work_check(app: Events, state: &Arc<Shared>) -> CmdResult<Vec<crate::work::Checked>> {
    let shared = state.clone();
    blocking(move || Ok(crate::work::check_connections(&app, &shared)))
}

/// The failing checks a workflow queued for sending, handed out once (the window sends them: it owns the terminals).
pub fn work_workflow_take(state: &Arc<Shared>) -> CmdResult<Vec<crate::work::workflows::Run>> {
    let shared = state.clone();
    blocking(move || Ok(shared.db.work_workflow_take(crate::db::now_ms())?))
}

/// How a queued send ended. One that wasn't sent says why on the item's timeline.
pub fn work_workflow_finish(app: Events, state: &Arc<Shared>, id: i64, run_state: String, text: String) -> CmdResult<()> {
    let shared = state.clone();
    blocking(move || {
        let now = crate::db::now_ms();
        let text: String = text.chars().take(500).collect();
        shared.db.work_workflow_finish(id, &run_state, &text, now)?;
        if run_state != "done" {
            if let Some(run) = shared.db.work_workflow_run(id)? {
                let item = shared.db.work_items()?.into_iter().find(|i| i.id == run.subject);
                if let Some(subject) = item.and_then(|i| i.ticket.or(i.pr)) {
                    shared.db.work_event(&subject, "workflow", &format!("The failing-checks workflow didn't send this: {text}"), now)?;
                }
            }
        }
        crate::work::changed(&app);
        Ok(())
    })
}

/// A project's latest workflow runs, newest first (Settings shows them).
pub fn work_workflow_runs(state: &Arc<Shared>, project_id: String) -> CmdResult<Vec<crate::work::workflows::Run>> {
    let shared = state.clone();
    blocking(move || Ok(shared.db.work_workflow_runs(&project_id, 8)?))
}

/// Runs a project's morning summary now (Settings, "Run now"); it ends on its own and says how ("workflow-ran").
pub fn work_morning_now(app: Events, state: &Arc<Shared>, project_id: String) -> CmdResult<()> {
    let shared = state.clone();
    blocking(move || {
        project(&shared, &project_id)?;
        crate::work::start_morning(&app, &shared, &project_id, &format!("by hand {}", crate::db::now_ms()))?;
        Ok(())
    })
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeHooks {
    /// The `enkidu` the hooks run (none: this build has no `enkidu` of its own next to its program)
    pub cli: Option<String>,
    pub command: Option<String>,
    pub files: Vec<crate::work::claudehooks::HookFile>,
    /// What is added, as JSON
    pub adds: Option<String>,
}

/// The hooks for Claude outside Enkidu: which Claude folders have them, and exactly what they add.
pub fn claude_hooks(state: &Arc<Shared>) -> CmdResult<ClaudeHooks> {
    let shared = state.clone();
    blocking(move || {
        use crate::work::claudehooks as ch;
        let cli = ch::cli_path();
        Ok(ClaudeHooks {
            command: cli.as_deref().map(ch::command),
            adds: cli.as_deref().map(|r| serde_json::to_string_pretty(&ch::adds(r)).unwrap_or_default()),
            cli: cli.map(|r| r.to_string_lossy().to_string()),
            files: ch::status(&crate::work::config_dirs(&shared)),
        })
    })
}

/// Adds the hooks to one Claude folder's settings, or takes them out. Only folders Enkidu knows.
pub fn set_claude_hooks(state: &Arc<Shared>, dir: String, on: bool) -> CmdResult<ClaudeHooks> {
    set_claude_hooks_with(state, dir, on, crate::work::claudehooks::cli_path())?;
    claude_hooks(state)
}

/// [`set_claude_hooks`] with the app's own `enkidu` given.
fn set_claude_hooks_with(state: &Arc<Shared>, dir: String, on: bool, cli: Option<PathBuf>) -> CmdResult<()> {
    let shared = state.clone();
    let known = crate::work::config_dirs(&shared).iter().any(|d| d.to_string_lossy() == dir);
    if !known {
        return Err(CmdError::new("unknown", "That isn't a Claude folder Enkidu knows"));
    }
    blocking(move || {
        use crate::work::claudehooks as ch;
        if on {
            let cli = cli.ok_or_else(|| CmdError::new("no_cli", "This build has no enkidu command of its own beside its program, so there is nothing the hooks could run"))?;
            ch::add(Path::new(&dir), &cli)?;
        } else {
            ch::remove(Path::new(&dir))?;
        }
        Ok(())
    })
}

/// A line in the timeline of a ticket or PR ("Sent to Claude in stock-fix: …").
pub fn work_note(app: Events, state: &Arc<Shared>, subject: String, text: String) -> CmdResult<()> {
    let shared = state.clone();
    blocking(move || {
        let text: String = text.chars().take(500).collect();
        shared.db.work_event(subject.trim(), "note", text.trim(), now_ms())?;
        crate::work::changed(&app);
        Ok(())
    })
}

/// Opens a web page from the Work index (a PR, a comment, a ticket, a check's log) in the browser. Only https links.
pub fn open_link(url: String) -> CmdResult<()> {
    let ok = url.starts_with("https://") && !url.contains(char::is_whitespace) && url.len() < 4096;
    if !ok {
        return Err(CmdError::new("bad_link", "Only web links (https) open from here"));
    }
    std::process::Command::new("/usr/bin/open").arg(&url).status().map_err(|e| CmdError::new("error", e.to_string()))?;
    Ok(())
}

pub fn get_project_settings(state: &Arc<Shared>, project_id: String) -> CmdResult<ProjectSettings> {
    let shared = state.clone();
    blocking(move || Ok(shared.project_settings(&project_id)?))
}

pub fn save_project_settings(state: &Arc<Shared>, project_id: String, settings: ProjectSettings) -> CmdResult<()> {
    let shared = state.clone();
    blocking(move || {
        project(&shared, &project_id)?;
        Ok(shared.save_project_settings(&project_id, &settings)?)
    })
}

pub fn save_user_settings(state: &Arc<Shared>, settings: UserSettings) -> CmdResult<()> {
    let shared = state.clone();
    blocking(move || Ok(shared.save_user_settings(settings)?))
}

/// What is said when Claude Code isn't on this Mac at all (the claude command is the usual one, and nothing answers
/// to it): Setup has the way to install it.
pub const CLAUDE_NOT_INSTALLED: &str = "Can't find Claude Code on this Mac. Setup, in Settings, says how to install it.";

/// Why Claude can't start when its program isn't found: Claude Code isn't installed, or the command of your own in
/// Settings names nothing.
pub fn claude_missing(command: &str) -> String {
    if command.trim() == "claude" {
        CLAUDE_NOT_INSTALLED.to_string()
    } else {
        format!("Can't find \"{command}\". Set the claude command in Settings.")
    }
}

/// The folder claude keeps its account and sessions in: CLAUDE_CONFIG_DIR, else ~/.claude.
fn claude_config_dir(env: &std::collections::BTreeMap<String, String>) -> PathBuf {
    env.get("CLAUDE_CONFIG_DIR")
        .map(|d| expand_tilde(d))
        .unwrap_or_else(|| shellenv::home_dir().join(".claude"))
}

/// Where claude saves this workspace's session inside one config folder.
fn transcript_in(config: &Path, ws: &Workspace) -> PathBuf {
    let encoded: String = ws.path.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    config.join("projects").join(encoded).join(format!("{}.jsonl", ws.session_id))
}

#[derive(Debug, PartialEq)]
enum Session {
    New,
    Resume,
    /// Saved under another config folder (another Claude account, before it was switched in Settings), or for
    /// another folder (a conversation resumed in the workspace from the main folder). `--resume` looks where Claude
    /// keeps the workspace folder's conversations, so it's copied there first, and Claude stays in the workspace.
    CarryOver { from: PathBuf, to: PathBuf },
}

/// A conversation Claude saved for this workspace's folder: under its name, or a shortened one with a tail added
/// (Claude shortens very long folder names). A conversation resumed from another folder (`/resume` in the workspace's
/// Claude picking one started in the main folder) is saved under that folder's name instead.
fn saved_for(transcript: &Path, config: &Path, ws: &Workspace) -> bool {
    let Ok(rest) = transcript.strip_prefix(config.join("projects")) else { return false };
    let Some(folder) = rest.components().next().map(|c| c.as_os_str().to_string_lossy().to_string()) else { return false };
    let mine = transcript_in(config, ws);
    let mine = mine.parent().and_then(|d| d.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let shared_start = mine.chars().zip(folder.chars()).take_while(|(a, b)| a == b).count();
    folder == mine || shared_start >= mine.chars().count().min(folder.chars().count()).min(200)
}

/// The conversation the workspace's Claude was last in, when it isn't the workspace's saved one: the hooks record the
/// transcript of every session that reports in, and Claude names a transcript after its session. Catches up with a
/// `/resume` or `/clear` the hooks saw before the workspace learned to follow them.
fn last_conversation(ws: &Workspace) -> Option<String> {
    let last = Path::new(ws.transcript_path.as_deref()?);
    let id = last.file_stem()?.to_string_lossy().to_string();
    (last.exists() && !id.is_empty() && id != ws.session_id).then_some(id)
}

fn find_session(ws: &Workspace, config: &Path, default_config: &Path) -> Session {
    let recorded = ws.transcript_path.as_deref().map(PathBuf::from).filter(|p| p.exists());
    let here = transcript_in(config, ws);
    if here.exists() || recorded.as_ref().is_some_and(|p| saved_for(p, config, ws)) {
        return Session::Resume;
    }
    match recorded.or_else(|| Some(transcript_in(default_config, ws)).filter(|p| p.exists())) {
        Some(from) => Session::CarryOver { from, to: here },
        None => Session::New,
    }
}

/// Copies a saved conversation into the current config folder. The original stays where it was.
fn carry_over(from: &Path, to: &Path) -> std::io::Result<()> {
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let part = to.with_extension("jsonl.part");
    std::fs::copy(from, &part)?;
    std::fs::rename(&part, to)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeCommand {
    program: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
    resume: bool,
}

/// Told to every Claude Enkidu starts, after Claude Code's own system prompt: what `enkidu` is for.
pub const CLI_PROMPT: &str = "You run inside Enkidu, which keeps an index of the user's tickets (Jira), PRs (GitHub) and the Claude \
     sessions behind them. Its `enkidu` command reads and corrects it: `enkidu status` shows this session's PRs, tickets and what waits \
     on the user there; `enkidu items` lists what waits; when this session works on a PR or ticket Enkidu hasn't linked, run \
     `enkidu link pr <number>` or `enkidu link ticket <KEY>` (and `enkidu unlink` for a wrong link); `enkidu note \"...\"` adds a line to the \
     ticket's timeline. When you finish a task the user gave you, or can't go on, report how it ended so it shows on the \
     task in Enkidu: `enkidu report done|review|blocked \"what you changed, how you checked it, what is left\"` (done: \
     nothing needs the user; review: it wants their review; blocked: you couldn't finish), with `--ticket KEY` or `--pr N` \
     when this session works on more than one, and `--image FILE` (repeatable) for screenshots of what you checked; the \
     user often reads these reports later, so make them a clear handover. In a workflow's task, `enkidu update \"how it goes\"` (with `--image FILE`) tells the workflow how it goes along the way, and `enkidu workflow` shows the workflow and your time left. To check your work in the dev environment, \
     `enkidu live on` copies this workspace's changes into the project's main folder, where the dev environment runs (another \
     workspace that was Live goes back first), and `enkidu live off` puts the main folder back. `enkidu --help` lists the rest.";

/// What a terminal Enkidu starts knows about it: where the app's API is, its token, the workspace and session.
fn cli_env(shared: &Shared, ws: &Workspace) -> Vec<(String, String)> {
    let mut env = vec![("ENKIDU_WORKSPACE".to_string(), ws.id.clone()), ("ENKIDU_SESSION".to_string(), ws.session_id.clone())];
    if let Some(server) = shared.hooks.lock().unwrap_or_else(|e| e.into_inner()).clone() {
        env.push(("ENKIDU_API".to_string(), format!("http://127.0.0.1:{}", server.port)));
        env.push(("ENKIDU_TOKEN".to_string(), server.token));
    }
    env
}

/// `starting` is false for the preview in Settings, which must not copy anything.
fn claude_command(shared: &Shared, ws: &Workspace, starting: bool) -> CmdResult<ClaudeCommand> {
    let mut launch = settings::claude_launch(&shared.user_settings(), &shared.project_settings(&ws.project_id)?);
    // a session brought in from outside runs in its own Claude folder (account)
    if let Some(dir) = ws.claude_config_dir.as_deref().filter(|d| !d.is_empty()) {
        launch.env.insert("CLAUDE_CONFIG_DIR".to_string(), dir.to_string());
    }
    let path_env = shellenv::login_path();
    let program = shellenv::which(&launch.command, &path_env)
        .ok_or_else(|| CmdError::new("claude_missing", claude_missing(&launch.command)))?;
    let server = shared
        .hooks
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .ok_or_else(|| CmdError::new("error", "The status server isn't running"))?;
    let settings_file = hooks::write_claude_settings(&shared.data_dir, &server, &ws.id)?;
    let resume = match find_session(ws, &claude_config_dir(&launch.env), &shellenv::home_dir().join(".claude")) {
        Session::New => false,
        Session::Resume => true,
        Session::CarryOver { from, to } => {
            if starting {
                carry_over(&from, &to).map_err(|e| {
                    CmdError::new(
                        "error",
                        format!("Can't bring this workspace's conversation over to {}: {e}", to.display()),
                    )
                })?;
            }
            true
        }
    };
    let mut args = vec![
        "--settings".to_string(),
        settings_file.to_string_lossy().to_string(),
        if resume { "--resume" } else { "--session-id" }.to_string(),
        ws.session_id.clone(),
        "--name".to_string(),
        ws.name.clone(),
        "--append-system-prompt".to_string(),
        CLI_PROMPT.to_string(),
    ];
    args.extend(launch.args);
    let env = launch
        .env
        .into_iter()
        .map(|(k, v)| {
            let v = if v.starts_with("~/") { expand_tilde(&v).to_string_lossy().to_string() } else { v };
            (k, v)
        })
        .collect();
    Ok(ClaudeCommand { program: program.to_string_lossy().to_string(), args, env, resume })
}

/// `draft` is typed into Claude's prompt once it has started, never sent: you press Enter (a message for a Claude that
/// wasn't running).
#[allow(clippy::too_many_arguments)]
pub fn pty_spawn(
    app: Events,
    state: &Arc<Shared>,
    workspace_id: String,
    kind: String,
    cols: u16,
    rows: u16,
    draft: Option<String>,
) -> CmdResult<TerminalInfo> {
    let shared = state.clone();
    blocking(move || spawn_terminal(&app, &shared, &workspace_id, &kind, cols, rows, draft, false))
}

/// Starts a terminal in a workspace: Claude (in the conversation it was last in) or your shell. `draft` goes into
/// Claude's prompt once it has started; Enter follows only with `send` (a scheduled workflow's run).
#[allow(clippy::too_many_arguments)]
pub fn spawn_terminal(
    app: &Events,
    shared: &Arc<Shared>,
    workspace_id: &str,
    kind: &str,
    cols: u16,
    rows: u16,
    draft: Option<String>,
    send: bool,
) -> CmdResult<TerminalInfo> {
    let mut ws = workspace(shared, workspace_id)?;
    // Claude starts in the conversation it was last in (you may have resumed another one in it)
    if kind == "claude" {
        if let Some(last) = last_conversation(&ws) {
            shared.db.set_session_id(&ws.id, &last)?;
            ws.session_id = last;
        }
    }
    let p = project(shared, &ws.project_id)?;
    let path_env = shellenv::login_path();
    let mut env = pty::base_env(&path_env);
    env.extend(workspace::script_env(Path::new(&p.path), Path::new(&ws.path), &ws.name, &ws.branch));
    env.extend(cli_env(shared, &ws));
    crate::work::claudehooks::cli_on_path(&mut env);
    let (program, args, title) = match kind {
        "claude" => {
            if shared.pty.running_claude(&ws.id) {
                return Err(CmdError::new("claude_running", "Claude is already running in this workspace"));
            }
            let cmd = claude_command(shared, &ws, true)?;
            env.extend(cmd.env);
            (PathBuf::from(cmd.program), cmd.args, "Claude".to_string())
        }
        "shell" => (PathBuf::from(shellenv::user_shell()), vec!["-l".to_string()], "Shell".to_string()),
        other => return Err(CmdError::new("error", format!("Unknown terminal kind {other}"))),
    };
    // only Claude gets text typed into its prompt
    let draft = if kind == "claude" { draft } else { None };
    let spec = SpawnSpec {
        workspace_id: ws.id.clone(),
        kind: kind.to_string(),
        title,
        program,
        args,
        cwd: PathBuf::from(&ws.path),
        env,
        cols,
        rows,
        draft,
        send,
    };
    Ok(shared.pty.spawn(app.clone(), shared.clone(), spec)?)
}

pub fn pty_write(app: Events, state: &Arc<Shared>, id: String, data: String) -> CmdResult<()> {
    if let Some(interrupted) = state.pty.write(&id, &data)? {
        pty::settle_after_interrupt(app, state.clone(), interrupted);
    }
    Ok(())
}

pub fn pty_resize(state: &Arc<Shared>, id: String, cols: u16, rows: u16) -> CmdResult<()> {
    Ok(state.pty.resize(&id, cols, rows)?)
}

pub fn pty_close(state: &Arc<Shared>, id: String) {
    state.pty.close(&id);
}

pub fn pty_list(state: &Arc<Shared>, workspace_id: String) -> Vec<TerminalInfo> {
    state.pty.list(&workspace_id)
}

pub fn pty_buffer(state: &Arc<Shared>, id: String) -> Option<pty::Buffered> {
    state.pty.buffer(&id)
}

pub fn get_changes(state: &Arc<Shared>, workspace_id: String) -> CmdResult<Vec<git::Change>> {
    let shared = state.clone();
    blocking(move || {
        let ws = workspace(&shared, &workspace_id)?;
        let files = git::changes(Path::new(&ws.path))?;
        // keep the watcher's copy current, so it only pushes real news afterwards
        shared.changes.store(&ws.id, &files);
        Ok(files)
    })
}

pub fn get_file_versions(
    state: &Arc<Shared>,
    workspace_id: String,
    path: String,
    old_path: Option<String>,
    kind: String,
) -> CmdResult<git::FileVersions> {
    let shared = state.clone();
    blocking(move || {
        let ws = workspace(&shared, &workspace_id)?;
        let dir = Path::new(&ws.path);
        git::inside(dir, &path)?;
        if let Some(old) = &old_path {
            git::inside(dir, old)?;
        }
        Ok(git::file_versions(dir, &path, old_path.as_deref(), &kind)?)
    })
}

/// The workspace's files for the Files panel and Go to file. `known`: the version the UI has, to skip resending it.
pub fn list_files(state: &Arc<Shared>, workspace_id: String, known: Option<String>) -> CmdResult<git::FileListing> {
    let shared = state.clone();
    blocking(move || {
        let ws = workspace(&shared, &workspace_id)?;
        let root = Path::new(&ws.path);
        let nested = shared.file_lists.in_submodules(root, shared.changes.generation(&workspace_id));
        Ok(git::list_files(root, known.as_deref(), &nested)?)
    })
}

/// One file of the workspace, read-only.
pub fn read_file(state: &Arc<Shared>, workspace_id: String, path: String) -> CmdResult<git::FilePreview> {
    let shared = state.clone();
    blocking(move || {
        let ws = workspace(&shared, &workspace_id)?;
        Ok(git::read_file(Path::new(&ws.path), &path)?)
    })
}

pub fn list_remarks(state: &Arc<Shared>, workspace_id: String) -> CmdResult<Vec<Remark>> {
    let shared = state.clone();
    blocking(move || Ok(shared.db.remarks(&workspace_id)?))
}

pub fn save_remark(state: &Arc<Shared>, remark: Remark) -> CmdResult<()> {
    let shared = state.clone();
    blocking(move || {
        workspace(&shared, &remark.workspace_id)?;
        if remark.side != "old" && remark.side != "new" {
            return Err(CmdError::new("error", "A remark is on the old or the new side"));
        }
        if remark.start_line < 1 || remark.end_line < remark.start_line {
            return Err(CmdError::new("error", "A remark needs a valid line range"));
        }
        Ok(shared.db.save_remark(&remark)?)
    })
}

pub fn delete_remark(state: &Arc<Shared>, id: String) -> CmdResult<()> {
    let shared = state.clone();
    blocking(move || Ok(shared.db.delete_remark(&id)?))
}

pub fn clear_remarks(state: &Arc<Shared>, workspace_id: String) -> CmdResult<()> {
    let shared = state.clone();
    blocking(move || Ok(shared.db.clear_remarks(&workspace_id)?))
}

/// Shows a project or workspace folder in Finder.
pub fn reveal_in_finder(state: &Arc<Shared>, path: String) -> CmdResult<()> {
    let shared = state.clone();
    blocking(move || {
        let known = shared.db.projects()?.iter().any(|p| p.path == path) || shared.db.workspaces()?.iter().any(|w| w.path == path);
        if !known || !Path::new(&path).exists() {
            return Err(CmdError::new("not_found", "That folder is gone"));
        }
        std::process::Command::new("open").arg("-R").arg(&path).status().map_err(|e| CmdError::new("error", e.to_string()))?;
        Ok(())
    })
}

/// Find in files: streams batches of results (in path order) through `on_hits` as they are found, and returns the
/// totals. A newer search of the same workspace cancels this one. Typing more of the same text only re-reads the
/// files the previous search found, as long as no file changed in between.
pub fn search_text(
    state: &Arc<Shared>,
    workspace_id: String,
    query: search::TextQuery,
    on_hits: &dyn Fn(Vec<search::FileHits>),
) -> CmdResult<search::TextSummary> {
    let shared = state.clone();
    blocking(move || {
        let ws = workspace(&shared, &workspace_id)?;
        let re = search::compile(&query).map_err(|e| CmdError::new("bad_query", e.to_string()))?;
        let cancel = shared.text_running.begin(&workspace_id);
        let root = Path::new(&ws.path);
        let generation = shared.changes.generation(&workspace_id);
        let tracked = shared.file_lists.tracked(root)?;
        let untracked = shared.changes.untracked(&workspace_id);
        let nested = shared.file_lists.in_submodules(root, generation);
        let narrowed: Option<Vec<String>> =
            shared.last_text.lock().unwrap_or_else(|e| e.into_inner()).get(&workspace_id).and_then(|last| {
                last.candidates(&query, generation, &tracked, &untracked, &nested).map(<[String]>::to_vec)
            });
        let files: Vec<&str> = match &narrowed {
            Some(candidates) => candidates.iter().map(String::as_str).collect(),
            // only the files of the kind asked for are read (item 10.5)
            None => search::of_kind(search::search_list(&tracked, &untracked, &nested), query.kind.as_deref()),
        };
        let scan = search::search_files(root, &files, &re, &cancel, |batch| {
            on_hits(batch);
        });
        if !scan.summary.cancelled {
            let matched = scan.matched.iter().map(|&i| files[i].to_string()).collect();
            let last = search::LastText { query, generation, tracked: tracked.clone(), untracked, nested, matched };
            shared.last_text.lock().unwrap_or_else(|e| e.into_inner()).insert(workspace_id.clone(), last);
        }
        Ok(scan.summary)
    })
}

/// Stops the workspace's running text search (the search popup closed).
pub fn cancel_text_search(state: &Arc<Shared>, workspace_id: String) {
    state.text_running.cancel(&workspace_id);
}

/// Go to symbol: the best definitions for `query`. The first search of a workspace builds its index (about a
/// second for 40,000 files); later ones only re-read the files the watcher saw change.
pub fn search_symbols(state: &Arc<Shared>, workspace_id: String, query: String, limit: Option<usize>) -> CmdResult<symbols::SymbolResults> {
    let shared = state.clone();
    blocking(move || {
        let ws = workspace(&shared, &workspace_id)?;
        let root = Path::new(&ws.path);
        let tracked = shared.file_lists.tracked(root)?;
        let untracked = shared.changes.untracked(&workspace_id);
        let nested = shared.file_lists.in_submodules(root, shared.changes.generation(&workspace_id));
        let files = search::search_list(&tracked, &untracked, &nested);
        let index = shared.symbols.get(&workspace_id, root);
        let changes = shared.changes.clone();
        let id = workspace_id.clone();
        Ok(symbols::search(&index, &files, move |_| changes.take_changed(&id), &query, limit.unwrap_or(60).min(200)))
    })
}

/// Go to definition: the name at a place of a file, and where its language's server says it is defined (phases 12 and
/// 13). The app has no rules of its own for this: a language without a server in the app has no links.
/// None: no name is there, or
/// the file's language has no server.
pub fn go_to_definition(state: &Arc<Shared>, workspace_id: String, ask: goto::Ask) -> CmdResult<Option<goto::Found>> {
    let shared = state.clone();
    blocking(move || {
        if !goto::is_python(&ask.path) {
            return Ok(None);
        }
        let ws = workspace(&shared, &workspace_id)?;
        Ok(by_server(&shared, &workspace_id, Path::new(&ws.path), &ask))
    })
}

/// Files bigger than this aren't given to a language server (the limit search and the rules have too).
const MAX_ASKED_BYTES: u64 = 1_000_000;

/// What the workspace's language server says about the name at a place of a Python file (phases 12
/// and 13). None: there is no name there, or no server on this machine. An answer that says `later`: the server
/// could not be asked yet (the first question starts it; it may be at work on an earlier one), and the app asks
/// again in a moment.
fn by_server(shared: &Arc<Shared>, workspace_id: &str, root: &Path, ask: &goto::Ask) -> Option<goto::Found> {
    let program = lsp::python_program(&shared.data_dir)?;
    let file = git::inside(root, &ask.path).ok()?;
    if std::fs::metadata(&file).ok()?.len() > MAX_ASKED_BYTES {
        return None;
    }
    let text = std::fs::read_to_string(&file).ok()?;
    let line = text.lines().nth(ask.line.checked_sub(1)?)?;
    let (start, end) = goto::name_at(line, ask.column)?;
    let question = lsp::Question {
        path: &ask.path,
        text: &text,
        language: "python",
        line: ask.line - 1,
        character: line[..start].encode_utf16().count(),
    };
    let changed = || shared.changes.take_changed_for_servers(workspace_id);
    let folder = shared.data_dir.join("servers");
    let word = &line[start..end];
    let Some(named) = shared.servers.definition(workspace_id, &program, &folder, root, &changed, &question) else {
        return Some(goto::not_yet(word, (start, end)));
    };
    // several places are listed: each row then says what stands on its line (a method, its class), as the
    // definitions list of Search's Symbols tab knows it. One place is gone to, and needs no words
    let listed = named.iter().filter(|(path, line)| path.as_deref().is_some_and(|p| p != ask.path || *line != ask.line));
    let index = (listed.count() > 1).then(|| definitions_index(shared, workspace_id, root)).flatten();
    let on_line = |path: &str, line: usize| {
        let index = index.as_ref()?.lock().unwrap_or_else(|e| e.into_inner());
        index.in_file(path).into_iter().find(|def| def.line as usize == line)
    };
    Some(goto::from_server(ask, word, (start, end), named, &on_line))
}

/// The workspace's definitions list (the one Search's Symbols tab searches), brought up to date.
fn definitions_index(
    shared: &Arc<Shared>,
    workspace_id: &str,
    root: &Path,
) -> Option<Arc<std::sync::Mutex<symbols::SymbolIndex>>> {
    let tracked = shared.file_lists.tracked(root).ok()?;
    let untracked = shared.changes.untracked(workspace_id);
    let nested = shared.file_lists.in_submodules(root, shared.changes.generation(workspace_id));
    let files = search::search_list(&tracked, &untracked, &nested);
    let index = shared.symbols.get(workspace_id, root);
    {
        let mut held = index.lock().unwrap_or_else(|e| e.into_inner());
        let (changes, id) = (shared.changes.clone(), workspace_id.to_string());
        held.bring_up_to_date(&files, move |_| changes.take_changed(&id));
    }
    Some(index)
}

/// A Python file comes on screen (phase 13): its workspace's language server is started now, and set
/// to work on that file, so it has read what it needs by the time a name is asked about. Nothing is waited for.
pub fn prepare_definitions(state: &Arc<Shared>, workspace_id: String, path: String) {
    if !goto::is_python(&path) {
        return;
    }
    let shared = state.clone();
    std::thread::spawn(move || {
        let Ok(Some(ws)) = shared.db.workspace(&workspace_id) else { return };
        let root = Path::new(&ws.path);
        let Some(program) = lsp::python_program(&shared.data_dir) else { return };
        if shared.servers.held_for(&workspace_id) {
            return;
        }
        let Ok(file) = git::inside(root, &path) else { return };
        if std::fs::metadata(&file).map_or(true, |meta| meta.len() > MAX_ASKED_BYTES) {
            return;
        }
        let Ok(text) = std::fs::read_to_string(&file) else { return };
        let question = lsp::Question { path: &path, text: &text, language: "python", line: 0, character: 0 };
        let changed = || shared.changes.take_changed_for_servers(&workspace_id);
        let folder = shared.data_dir.join("servers");
        let _ = shared.servers.definition(&workspace_id, &program, &folder, root, &changed, &question);
    });
}

/// Builds (or refreshes) the workspace's symbol index in the background, so the first symbol search is quick.
pub fn prepare_symbols(state: &Arc<Shared>, workspace_id: String) {
    let shared = state.clone();
    std::thread::spawn(move || {
        let Ok(Some(ws)) = shared.db.workspace(&workspace_id) else { return };
        let root = Path::new(&ws.path);
        let Ok(tracked) = shared.file_lists.tracked(root) else { return };
        let untracked = shared.changes.untracked(&workspace_id);
        let nested = shared.file_lists.in_submodules(root, shared.changes.generation(&workspace_id));
        let files = search::search_list(&tracked, &untracked, &nested);
        let index = shared.symbols.get(&workspace_id, root);
        let changes = shared.changes.clone();
        let _ = symbols::search(&index, &files, move |_| changes.take_changed(&workspace_id), "", 0);
    });
}

/// Shows one file of a workspace in Finder.
pub fn reveal_file(state: &Arc<Shared>, workspace_id: String, path: String) -> CmdResult<()> {
    let shared = state.clone();
    blocking(move || {
        let ws = workspace(&shared, &workspace_id)?;
        let file = git::inside(Path::new(&ws.path), &path)?;
        if !file.exists() {
            return Err(CmdError::new("not_found", "That file is gone"));
        }
        std::process::Command::new("open").arg("-R").arg(&file).status().map_err(|e| CmdError::new("error", e.to_string()))?;
        Ok(())
    })
}

pub fn get_file_diff(
    state: &Arc<Shared>,
    workspace_id: String,
    path: String,
    old_path: Option<String>,
    untracked: bool,
) -> CmdResult<git::FileDiff> {
    let shared = state.clone();
    blocking(move || {
        let ws = workspace(&shared, &workspace_id)?;
        let dir = Path::new(&ws.path);
        git::inside(dir, &path)?;
        if let Some(old) = &old_path {
            git::inside(dir, old)?;
        }
        Ok(git::file_diff(dir, &path, old_path.as_deref(), untracked)?)
    })
}

/// One file's diff with the whole file in it (`git::file_patch`), for Enkidu's diff view.
pub fn get_file_patch(
    state: &Arc<Shared>,
    workspace_id: String,
    path: String,
    old_path: Option<String>,
    untracked: bool,
) -> CmdResult<git::FileDiff> {
    let shared = state.clone();
    blocking(move || {
        let ws = workspace(&shared, &workspace_id)?;
        let dir = Path::new(&ws.path);
        git::inside(dir, &path)?;
        if let Some(old) = &old_path {
            git::inside(dir, old)?;
        }
        Ok(git::file_patch(dir, &path, old_path.as_deref(), untracked)?)
    })
}

/// Every workspace is watched; the one on screen refreshes fastest.
pub fn watch_workspace(state: &Arc<Shared>, workspace_id: Option<String>) {
    state.changes.focus(workspace_id);
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveRequest {
    pub project_id: String,
    pub workspace_id: Option<String>,
}

pub fn set_live(app: Events, state: &Arc<Shared>, req: LiveRequest) -> CmdResult<LiveOutcome> {
    let shared = state.clone();
    blocking(move || Ok(live::set_live(&app, &shared, &req.project_id, req.workspace_id.as_deref())?))
}

pub fn claude_preview(state: &Arc<Shared>, workspace_id: String) -> CmdResult<String> {
    let shared = state.clone();
    blocking(move || {
        let ws = workspace(&shared, &workspace_id)?;
        let cmd = claude_command(&shared, &ws, false)?;
        let quoted: Vec<String> = cmd.args.iter().map(|a| shell_words::quote(a).into_owned()).collect();
        Ok(format!("{} {}", cmd.program, quoted.join(" ")))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Items 4.4 and 21.4: the hooks go into a Claude folder Enkidu knows (here the account's folder Settings name),
    /// with the file as it was kept beside it, and come out again.
    #[test]
    fn hooks_are_added_to_a_claude_folder_and_taken_out_again() {
        let home = tempfile::tempdir().unwrap();
        let shared = Arc::new(Shared::new(home.path().join("data"), std::time::Instant::now()).unwrap());
        let claude = home.path().join("claude");
        std::fs::create_dir_all(claude.join("projects")).unwrap();
        let settings = claude.join("settings.json");
        std::fs::write(&settings, "{ \"model\": \"opus\" }\n").unwrap();
        let mut user = shared.user_settings();
        user.claude.env.insert("CLAUDE_CONFIG_DIR".into(), claude.to_string_lossy().into());
        shared.save_user_settings(user).unwrap();
        let dir = claude.to_string_lossy().to_string();
        let cli = PathBuf::from("/Applications/Enkidu.app/Contents/MacOS/enkidu");
        set_claude_hooks_with(&shared, dir.clone(), true, Some(cli)).unwrap();
        let written: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        assert!(crate::work::claudehooks::is_added(&written));
        assert_eq!(written["model"], "opus", "what was in the file stays");
        assert_eq!(std::fs::read_to_string(claude.join("settings.json.before-enkidu")).unwrap(), "{ \"model\": \"opus\" }\n");
        // a build without its own enkidu has nothing the hooks could run: it says so
        assert_eq!(set_claude_hooks_with(&shared, dir.clone(), true, None).unwrap_err().code, "no_cli");
        // a folder Enkidu doesn't know is never written into
        let stranger = home.path().join("elsewhere");
        std::fs::create_dir_all(&stranger).unwrap();
        let refused = set_claude_hooks_with(&shared, stranger.to_string_lossy().into(), true, Some("/x/enkidu".into()));
        assert_eq!(refused.unwrap_err().code, "unknown");
        assert!(!stranger.join("settings.json").exists());
        // taken out again: the file says what it said
        set_claude_hooks_with(&shared, dir, false, None).unwrap();
        let back: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(back, serde_json::json!({ "model": "opus" }));
    }

    /// Item 21.7: with the usual command and nothing answering to it, Claude Code isn't installed, and Setup says how;
    /// a command of your own that names nothing is put right in Settings.
    #[test]
    fn a_claude_that_isnt_there_is_said_as_not_installed_or_as_a_command_to_put_right() {
        assert_eq!(claude_missing("claude"), CLAUDE_NOT_INSTALLED);
        assert_eq!(claude_missing(" claude "), CLAUDE_NOT_INSTALLED);
        assert_eq!(claude_missing("/nowhere/claude"), "Can't find \"/nowhere/claude\". Set the claude command in Settings.");
        assert!(CLAUDE_NOT_INSTALLED.contains("Setup"));
    }

    const WS_PATH: &str = "/Users/me/enkidu/workspaces/shop.example/one";
    const SAVED_AS: &str = "projects/-Users-me-enkidu-workspaces-shop-example-one/abc.jsonl";

    fn ws_at(path: &str, transcript: Option<&Path>) -> Workspace {
        Workspace {
            id: "w".into(),
            project_id: "p".into(),
            name: "one".into(),
            path: path.into(),
            branch: "b".into(),
            base: None,
            session_id: "abc".into(),
            transcript_path: transcript.map(|p| p.to_string_lossy().to_string()),
            claude_config_dir: None,
            adopted: false,
            created_at: 0,
        }
    }

    fn save(file: &Path, text: &str) {
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }

    #[test]
    fn the_config_dir_comes_from_the_env_with_a_tilde_expanded() {
        let mut env = std::collections::BTreeMap::new();
        assert_eq!(claude_config_dir(&env), shellenv::home_dir().join(".claude"));
        env.insert("CLAUDE_CONFIG_DIR".to_string(), "~/.claude-work".to_string());
        assert_eq!(claude_config_dir(&env), shellenv::home_dir().join(".claude-work"));
    }

    #[test]
    fn finds_saved_sessions_in_the_claude_config_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let (config, default) = (tmp.path().join("work"), tmp.path().join("personal"));
        let ws = ws_at(WS_PATH, None);
        assert_eq!(find_session(&ws, &config, &default), Session::New);
        save(&config.join(SAVED_AS), "{}");
        assert_eq!(find_session(&ws, &config, &default), Session::Resume);
    }

    #[test]
    fn trusts_the_path_claude_reported_inside_the_config_dir() {
        // claude shortens very long folder names, so the path it reported wins over the computed one
        let tmp = tempfile::tempdir().unwrap();
        let reported = tmp.path().join("projects/-Users-me-a-very-long-path-1a2b/abc.jsonl");
        save(&reported, "{}");
        let ws = ws_at("/Users/me/a/very/long/path", Some(&reported));
        assert_eq!(find_session(&ws, tmp.path(), &tmp.path().join("none")), Session::Resume);
    }

    #[test]
    fn a_conversation_from_another_account_is_copied_over_and_resumed() {
        let tmp = tempfile::tempdir().unwrap();
        let (personal, work) = (tmp.path().join(".claude"), tmp.path().join(".claude-work"));
        let (old, new) = (personal.join(SAVED_AS), work.join(SAVED_AS));
        save(&old, "{\"type\":\"user\"}\n");

        // found through the path the hooks recorded, or in ~/.claude when none was recorded
        for ws in [ws_at(WS_PATH, Some(&old)), ws_at(WS_PATH, None)] {
            let found = find_session(&ws, &work, &personal);
            assert_eq!(found, Session::CarryOver { from: old.clone(), to: new.clone() });
        }
        carry_over(&old, &new).unwrap();
        assert_eq!(std::fs::read_to_string(&new).unwrap(), "{\"type\":\"user\"}\n");
        assert!(old.exists(), "the original stays with the other account");
        assert!(!new.with_extension("jsonl.part").exists());
        assert_eq!(find_session(&ws_at(WS_PATH, Some(&old)), &work, &personal), Session::Resume);
    }

    #[test]
    fn starts_in_the_conversation_the_hooks_saw_last() {
        let tmp = tempfile::tempdir().unwrap();
        let resumed = tmp.path().join("projects/-Users-me-code-shop-example/3d5c2d6e.jsonl");
        // nothing recorded, or the recorded one is the workspace's own: nothing to catch up with
        assert_eq!(last_conversation(&ws_at(WS_PATH, None)), None);
        let own = tmp.path().join("projects/x/abc.jsonl");
        save(&own, "{}");
        assert_eq!(last_conversation(&ws_at(WS_PATH, Some(&own))), None);
        // a transcript that isn't there anymore isn't followed
        assert_eq!(last_conversation(&ws_at(WS_PATH, Some(&resumed))), None);
        save(&resumed, "{}");
        assert_eq!(last_conversation(&ws_at(WS_PATH, Some(&resumed))).as_deref(), Some("3d5c2d6e"));
    }

    #[test]
    fn a_conversation_resumed_from_the_main_folder_is_copied_into_the_workspace() {
        // `/resume` in the workspace's Claude picked a conversation started in the main folder: Claude saves it under
        // the main folder's name, and the hooks recorded that path
        let tmp = tempfile::tempdir().unwrap();
        let work = tmp.path().join(".claude-work");
        let in_main = work.join("projects/-Users-me-code-shop-example/abc.jsonl");
        save(&in_main, "{\"type\":\"user\"}\n");
        let ws = ws_at(WS_PATH, Some(&in_main));
        assert_eq!(find_session(&ws, &work, &tmp.path().join(".claude")), Session::CarryOver { from: in_main, to: work.join(SAVED_AS) });
        // the workspace's own, under its full name or a shortened one, is resumed where it is
        let own = work.join(SAVED_AS);
        assert!(saved_for(&own, &work, &ws));
        let shortened = work.join("projects/-Users-me-enkidu-workspaces-shop-example-one-7f3a/abc.jsonl");
        assert!(saved_for(&shortened, &work, &ws));
        assert!(!saved_for(&work.join("projects/-Users-me-enkidu-workspaces-shop-example-other/abc.jsonl"), &work, &ws));
    }
}
