//! The Work index (WORK-INDEX.md): your tickets (Jira), PRs (GitHub) and the Claude sessions behind them, linked many
//! to many, and what waits on you. Read-only towards GitHub and Jira; no token is ever stored, printed or logged.

pub mod github;
pub mod hookev;
pub mod jira;
pub mod json;
pub mod links;
pub mod api;
pub mod claudehooks;
pub mod eventlog;
pub mod flows;
pub mod live;
pub mod model;
pub mod parse;
pub mod reports;
pub mod run;
pub mod store;
pub mod transcripts;
pub mod rules;
pub mod watch;
pub mod workflows;

use anyhow::Result;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use crate::events::{Event, Events};

use crate::db::{now_ms, Db, Workspace};
use crate::settings::WorkSettings;
use crate::shellenv::{expand_tilde, home_dir};
use crate::state::Shared;
use crate::workspace::project_slug;
use links::ProjectInfo;
use model::{Pr, Snapshot};
use store::SessionSeen;

/// How far back the transcript index looks.
const TRANSCRIPT_DAYS: i64 = 60;
/// How often saved sessions and the event log are checked for new lines.
pub const TRANSCRIPT_EVERY: std::time::Duration = std::time::Duration::from_secs(60);

/// That, or what end-to-end tests set (`ENKIDU_SCAN_EVERY_MS`).
fn scan_every() -> std::time::Duration {
    std::env::var("ENKIDU_SCAN_EVERY_MS").ok().and_then(|v| v.parse().ok()).map(std::time::Duration::from_millis).unwrap_or(TRANSCRIPT_EVERY)
}

#[derive(Default)]
pub struct Hub {
    /// Each project's GitHub repo (`owner/name`), read from its origin remote once. The URL itself is never kept.
    repos: Mutex<HashMap<String, Option<String>>>,
    /// One transcript scan at a time.
    scanning: Mutex<()>,
    /// Projects whose GitHub and Jira should be read again now (Refresh, a push, Claude finished a turn), each with the
    /// number of its latest request, so a request made while a read runs isn't cleared by that read.
    wanted: Mutex<HashMap<String, u64>>,
    asked: std::sync::atomic::AtomicU64,
    /// The GitHub repo of folders sessions ran in, read once each (gone folders are None).
    folders: Mutex<HashMap<String, Option<String>>>,
    /// Wakes the watcher when a read is wanted.
    woken: Mutex<bool>,
    wake: std::sync::Condvar,
    /// Projects being read right now.
    checking: Mutex<HashSet<String>>,
    failures: Mutex<HashMap<String, u32>>,
    /// Who twg reads Jira as: (account id, name), asked once.
    pub jira_me: Mutex<Option<(String, String)>>,
    /// The programs this core runs in place of `gh` and `twg` (a test gives each of its cores fakes of its own).
    sources: Mutex<Option<watch::Sources>>,
    /// The watchers' threads end at their next turn (a test's core is done).
    stopped: std::sync::atomic::AtomicBool,
}

impl Hub {
    /// The programs the watchers run: the ones given to this core, else `gh` and `twg` (or what `ENKIDU_GH` and
    /// `ENKIDU_TWG` name), found on your login shell's PATH.
    pub fn sources(&self) -> watch::Sources {
        if let Some(given) = self.sources.lock().unwrap_or_else(|e| e.into_inner()).clone() {
            return given;
        }
        watch::Sources::from_env(&crate::shellenv::login_path())
    }

    /// Gives this core its own programs in place of `gh` and `twg`. Tests do, so each core has fakes with a mode and a
    /// log of their own (the environment's names are one for the whole process).
    pub fn use_sources(&self, sources: watch::Sources) {
        *self.sources.lock().unwrap_or_else(|e| e.into_inner()) = Some(sources);
    }

    /// Ends the watchers' threads at their next turn, and wakes the one that sleeps.
    pub fn stop(&self) {
        self.stopped.store(true, std::sync::atomic::Ordering::SeqCst);
        let mut woken = self.woken.lock().unwrap_or_else(|e| e.into_inner());
        *woken = true;
        self.wake.notify_all();
    }

    pub fn stopped(&self) -> bool {
        self.stopped.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// The GitHub repo (`owner/name`) of any folder, from its `origin`; cached.
    pub fn folder_repo(&self, folder: &str) -> Option<String> {
        if let Some(known) = self.folders.lock().unwrap_or_else(|e| e.into_inner()).get(folder) {
            return known.clone();
        }
        let path = Path::new(folder);
        let repo = if path.is_dir() { github::repo_slug(path) } else { None };
        self.folders.lock().unwrap_or_else(|e| e.into_inner()).insert(folder.to_string(), repo.clone());
        repo
    }

    pub fn repo(&self, project_id: &str, path: &str) -> Option<String> {
        let mut repos = self.repos.lock().unwrap_or_else(|e| e.into_inner());
        repos.entry(project_id.to_string()).or_insert_with(|| github::repo_slug(Path::new(path))).clone()
    }

    /// Asks for a read of this project's sources as soon as possible, and wakes the watcher.
    pub fn want(&self, project_id: &str) {
        let n = self.asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        self.wanted.lock().unwrap_or_else(|e| e.into_inner()).insert(project_id.to_string(), n);
        let mut woken = self.woken.lock().unwrap_or_else(|e| e.into_inner());
        *woken = true;
        self.wake.notify_all();
    }

    /// The projects wanted now, with their latest request (they stay wanted until a read that started after it).
    pub fn wanted(&self) -> HashMap<String, u64> {
        self.wanted.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// A read of the project ended. `seen` is the request it started with: one made while it ran stays wanted, so it
    /// gets a read of its own (a Refresh pressed during a read isn't lost).
    pub fn read_done(&self, project_id: &str, seen: Option<u64>) {
        let mut wanted = self.wanted.lock().unwrap_or_else(|e| e.into_inner());
        if wanted.get(project_id).copied() == seen {
            wanted.remove(project_id);
        }
    }

    /// Sleeps until `max` passes or someone wants a read.
    pub fn sleep(&self, max: std::time::Duration) {
        let woken = self.woken.lock().unwrap_or_else(|e| e.into_inner());
        let (mut woken, _) = self.wake.wait_timeout_while(woken, max, |w| !*w).unwrap_or_else(|e| e.into_inner());
        *woken = false;
    }

    pub fn set_checking(&self, project_id: &str, on: bool) {
        let mut c = self.checking.lock().unwrap_or_else(|e| e.into_inner());
        if on {
            c.insert(project_id.to_string());
        } else {
            c.remove(project_id);
        }
    }

    pub fn checking(&self) -> bool {
        !self.checking.lock().unwrap_or_else(|e| e.into_inner()).is_empty()
    }

    /// Failed reads in a row, per project and source (for the backoff).
    pub fn failures(&self, project_id: &str, source: &str) -> u32 {
        *self.failures.lock().unwrap_or_else(|e| e.into_inner()).get(&format!("{project_id}/{source}")).unwrap_or(&0)
    }

    pub fn set_failures(&self, project_id: &str, source: &str, n: u32) {
        self.failures.lock().unwrap_or_else(|e| e.into_inner()).insert(format!("{project_id}/{source}"), n);
    }
}

/// What the link rules need to know about every project.
pub fn projects(shared: &Shared) -> Vec<ProjectInfo> {
    let root = shared.user_settings().workspaces_root();
    shared
        .db
        .projects()
        .unwrap_or_default()
        .into_iter()
        .map(|p| {
            let settings = shared.project_settings(&p.id).unwrap_or_default();
            ProjectInfo {
                workspaces_dir: root.join(project_slug(&p.name)).to_string_lossy().to_string(),
                repo: shared.work.repo(&p.id, &p.path),
                base: settings.default_base.trim().to_string(),
                jira_keys: settings.work.jira_keys.clone(),
                id: p.id,
                path: p.path,
            }
        })
        .collect()
}

/// Every Claude folder sessions can be saved in: `~/.claude`, the `CLAUDE_CONFIG_DIR` of the settings, and any
/// other `~/.claude*` folder with saved sessions (an alias of yours for a second account).
pub fn config_dirs(shared: &Shared) -> Vec<PathBuf> {
    // tests name their own folders (`a:b`), so they never read your real sessions
    if let Ok(list) = std::env::var("ENKIDU_CLAUDE_DIRS") {
        return list.split(':').filter(|s| !s.trim().is_empty()).map(|s| expand_tilde(s.trim())).filter(|d| d.join("projects").is_dir()).collect();
    }
    let mut dirs = vec![home_dir().join(".claude")];
    let mut add = |env: &BTreeMap<String, String>| {
        if let Some(d) = env.get("CLAUDE_CONFIG_DIR") {
            let p = expand_tilde(d);
            if !dirs.contains(&p) {
                dirs.push(p);
            }
        }
    };
    add(&shared.user_settings().claude.env);
    for p in shared.db.projects().unwrap_or_default() {
        if let Ok(s) = shared.project_settings(&p.id) {
            add(&s.claude.env);
        }
    }
    if let Ok(entries) = std::fs::read_dir(home_dir()) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with(".claude") && e.path().join("projects").is_dir() && !dirs.contains(&e.path()) {
                dirs.push(e.path());
            }
        }
    }
    dirs.retain(|d| d.join("projects").is_dir());
    dirs
}

/// Every workspace's own Claude session, with the workspace's branch.
pub fn sync_workspaces(shared: &Shared) -> Result<()> {
    for ws in shared.db.workspaces()? {
        workspace_session(&shared.db, shared, &ws)?;
    }
    Ok(())
}

fn workspace_session(db: &Db, shared: &Shared, ws: &Workspace) -> Result<()> {
    let config_dir = ws
        .transcript_path
        .as_deref()
        .and_then(hookev::config_dir_of)
        .or_else(|| {
            let env = crate::settings::claude_launch(&shared.user_settings(), &shared.project_settings(&ws.project_id).unwrap_or_default()).env;
            env.get("CLAUDE_CONFIG_DIR").map(|d| expand_tilde(d).to_string_lossy().to_string())
        })
        .unwrap_or_else(|| home_dir().join(".claude").to_string_lossy().to_string());
    db.work_session_seen(&SessionSeen {
        id: ws.session_id.clone(),
        config_dir,
        cwd: ws.path.clone(),
        title: Some(ws.name.clone()),
        workspace_id: Some(ws.id.clone()),
        first_at: ws.created_at,
        last_at: ws.created_at,
        source: "workspace".into(),
    })?;
    db.work_session_branch(&ws.session_id, &ws.branch, "branch", "worked", ws.created_at)?;
    Ok(())
}

pub fn relink(shared: &Shared) -> Result<usize> {
    links::relink(&shared.db, &projects(shared), now_ms(), &|folder| shared.work.folder_repo(folder))
}

pub fn changed(app: &Events) {
    app.send(Event::WorkChanged);
}

/// Reads what was added to the saved sessions since the last scan, then links. Returns how many files were read.
pub fn scan_transcripts(shared: &Shared) -> Result<transcripts::Stats> {
    let _one = shared.work.scanning.lock().unwrap_or_else(|e| e.into_inner());
    let since = now_ms() - TRANSCRIPT_DAYS * 86_400_000;
    let stats = transcripts::scan(&shared.db, &config_dirs(shared), since)?;
    Ok(stats)
}

/// A workspace was made: its session and branch join the index at once.
pub fn workspace_created(app: &Events, shared: &Shared, ws: &Workspace) {
    if workspace_session(&shared.db, shared, ws).is_ok() && relink(shared).is_ok() {
        changed(app);
    }
}

/// The workspace's folder is on another branch now (Claude or you checked one out in it): the workspace follows, and so
/// do its links (a workspace on a PR's branch is that PR's). A detached HEAD keeps the branch it had.
pub fn follow_branch(app: &Events, shared: &Shared, workspace_id: &str) {
    let Ok(Some(ws)) = shared.db.workspace(workspace_id) else { return };
    let Some(now) = crate::git::current_branch(Path::new(&ws.path)) else { return };
    if now == ws.branch || shared.db.set_branch(&ws.id, &now).is_err() {
        return;
    }
    app.send(Event::WorkspaceUpdated(Workspace { branch: now, ..ws }));
    if relink(shared).is_ok() {
        changed(app);
    }
}

/// A workspace was removed: its links stay in the history, marked removed.
pub fn workspace_removed(app: &Events, shared: &Shared, workspace_id: &str) {
    if shared.db.work_remove_links_of(&format!("workspace:{workspace_id}")).is_ok() {
        changed(app);
    }
}

/// A hook from a Claude in a workspace. Returns what to tell Claude (SessionStart only).
pub fn on_hook(app: &Events, shared: &Shared, ws: &Workspace, body: &str) -> String {
    let Some(p) = hookev::read_payload(body) else { return String::new() };
    let branch_of = |cwd: &str| crate::git::current_branch(Path::new(cwd));
    let outcome = match hookev::apply(&shared.db, Some(ws), &p, now_ms(), &branch_of) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("work index: a hook couldn't be saved: {e:#}");
            return String::new();
        }
    };
    if outcome.refresh {
        shared.work.want(&ws.project_id);
    }
    let matters = p.event != "PostToolUse" || !p.ops.is_empty();
    if matters && relink(shared).is_ok() {
        changed(app);
    }
    if p.event == "SessionStart" {
        session_context(&shared.db, &p.session_id).unwrap_or_default()
    } else {
        String::new()
    }
}

/// You marked an item done, snoozed it (by default for a day), or opened it again. Goes into its history.
pub fn set_item(db: &Db, id: &str, state: &str, snoozed_until: Option<i64>, now: i64) -> Result<()> {
    anyhow::ensure!(matches!(state, "open" | "done" | "snoozed"), "An item is open, done or snoozed");
    let mut item = db.work_items()?.into_iter().find(|i| i.id == id).ok_or_else(|| anyhow::anyhow!("That item is gone"))?;
    let subject = item.ticket.clone().or_else(|| item.pr.clone()).unwrap_or_default();
    item.state = state.into();
    item.snoozed_until = (state == "snoozed").then(|| snoozed_until.unwrap_or(now + 86_400_000));
    if state == "open" {
        item.closed_at = None;
    }
    db.work_put_item(&item)?;
    let what = match state {
        "done" => "Marked done",
        "snoozed" => "Snoozed",
        _ => "Opened again",
    };
    db.work_event(&subject, &format!("item-{state}"), &format!("{what}: {}", item.title), now)?;
    Ok(())
}

/// One source of one project, read fresh and compared with what gh or twg says directly.
#[derive(serde::Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Checked {
    pub project: String,
    pub source: String,
    pub ok: bool,
    pub compared: Vec<String>,
    pub differences: Vec<live::Difference>,
    pub error: Option<String>,
}

/// One GitHub read of a project, as every caller does it: its failures counted for the backoff, then the failing-checks
/// workflow told about it (it acts only on a full, fresh read).
fn read_github(shared: &Shared, sources: &watch::Sources, project: &ProjectInfo, settings: &WorkSettings) -> model::Health {
    let mut failures = shared.work.failures(&project.id, "github");
    let read_at = now_ms();
    let h = watch::github_pass(&shared.db, &sources.gh, project, settings, &mut failures, read_at);
    shared.work.set_failures(&project.id, "github", failures);
    if let Err(e) = workflows::after_github(&shared.db, &project.id, settings.send_failed_checks, &h, read_at) {
        eprintln!("work index: the failing-checks workflow couldn't look at the read: {e:#}");
    }
    h
}

/// One Jira read of a project.
fn read_jira(shared: &Shared, sources: &watch::Sources, project: &ProjectInfo, settings: &WorkSettings) -> model::Health {
    let mut failures = shared.work.failures(&project.id, "jira");
    let mut me = shared.work.jira_me.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let h = watch::jira_pass(&shared.db, &sources.twg, project, settings, &mut me, &mut failures, now_ms());
    *shared.work.jira_me.lock().unwrap_or_else(|e| e.into_inner()) = me;
    shared.work.set_failures(&project.id, "jira", failures);
    h
}

/// Starts a project's morning summary once for `subject` (the day, or the time when run by hand) on its own thread:
/// the run's id, or None when it already ran for that subject. Tells the window ("workflow-ran") when it ends.
pub fn start_morning(app: &Events, shared: &std::sync::Arc<Shared>, project_id: &str, subject: &str) -> Result<Option<i64>> {
    let Some(id) = shared.db.work_workflow_add(project_id, workflows::MORNING, subject, "running", now_ms())? else {
        return Ok(None);
    };
    changed(app);
    let (app, shared, project_id) = (app.clone(), shared.clone(), project_id.to_string());
    std::thread::Builder::new().name("work-morning".into()).spawn(move || {
        let (state, text) = morning(&app, &shared, &project_id)
            .unwrap_or_else(|e| ("failed".to_string(), crate::redact::redact_urls(&format!("{e:#}"))));
        let _ = shared.db.work_workflow_finish(id, &state, &text, now_ms());
        if let Ok(Some(run)) = shared.db.work_workflow_run(id) {
            app.send(Event::WorkflowRan(run));
        }
        changed(&app);
    })?;
    Ok(Some(id))
}

/// The morning summary: check everything (both sources read now), then, only on full and fresh reads, the read-only
/// Claude in the project's main folder. Returns how it ended, in words.
fn morning(app: &Events, shared: &Shared, project_id: &str) -> Result<(String, String)> {
    let project = projects(shared).into_iter().find(|p| p.id == project_id).ok_or_else(|| anyhow::anyhow!("That project is gone"))?;
    let settings = shared.project_settings(project_id).unwrap_or_default();
    let ws = &settings.work;
    let sources = shared.work.sources();
    let started = now_ms();
    shared.work.set_checking(project_id, true);
    changed(app);
    let github = (ws.github && project.repo.is_some()).then(|| read_github(shared, &sources, &project, ws));
    let jira = (!ws.jira_keys.is_empty()).then(|| read_jira(shared, &sources, &project, ws));
    shared.work.set_checking(project_id, false);
    let _ = relink(shared);
    changed(app);
    if let Err(why) = workflows::morning_ready(github.as_ref(), jira.as_ref(), started) {
        return Ok(("skipped".into(), format!("Didn't run: {why}")));
    }
    let out = morning_claude(shared, &project, &settings)?;
    let notes = shared.db.work_events(5000)?.into_iter().filter(|e| e.kind == "note" && e.at >= started).count();
    let last = |text: &str| text.lines().map(str::trim).filter(|l| !l.is_empty()).next_back().unwrap_or("").chars().take(240).collect::<String>();
    if out.code != 0 {
        let why = last(&out.stderr);
        return Ok(("failed".into(), format!("Claude stopped (exit code {}): {}", out.code, if why.is_empty() { last(&out.stdout) } else { why })));
    }
    let lines = format!("Claude wrote {notes} line{} on the tickets' timelines.", if notes == 1 { "" } else { "s" });
    let said = last(&out.stdout);
    Ok(("done".into(), if said.is_empty() { lines } else { format!("{lines} {said}") }))
}

/// Runs the morning summary's Claude: your claude and its environment (the account's folder), never your or the
/// project's Claude flags; `enkidu` reaches the app (no session or workspace: its notes name their ticket).
fn morning_claude(shared: &Shared, project: &ProjectInfo, settings: &crate::settings::ProjectSettings) -> Result<run::Output> {
    let launch = crate::settings::claude_launch(&shared.user_settings(), settings);
    let path_env = crate::shellenv::login_path();
    let program = crate::shellenv::which(&launch.command, &path_env)
        .ok_or_else(|| anyhow::anyhow!(crate::commands::claude_missing(&launch.command)))?;
    let mut env = crate::pty::base_env(&path_env);
    let mut set = |k: &str, v: String| {
        env.retain(|(x, _)| x != k);
        env.push((k.to_string(), v));
    };
    for (k, v) in launch.env {
        set(&k, if v.starts_with("~/") { expand_tilde(&v).to_string_lossy().to_string() } else { v });
    }
    if let Some(server) = shared.hooks.lock().unwrap_or_else(|e| e.into_inner()).clone() {
        set("ENKIDU_API", format!("http://127.0.0.1:{}", server.port));
        set("ENKIDU_TOKEN", server.token);
    }
    set("ENKIDU_WORKFLOW", "morning".into());
    claudehooks::cli_on_path(&mut env);
    workflows::run_headless(&program, &workflows::morning_args(), Path::new(&project.path), &env, workflows::MORNING_PROMPT, workflows::MORNING_LIMIT)
}

/// "Check connections" and `enkidu doctor`: every project's GitHub and Jira read now, then compared field by field with
/// what gh and twg say directly (the live check). Read-only.
pub fn check_connections(app: &Events, shared: &Shared) -> Vec<Checked> {
    let sources = shared.work.sources();
    let mut out = Vec::new();
    let checked = |name: &str, r: live::Report| Checked {
        project: name.to_string(),
        ok: r.ok(),
        source: r.source,
        compared: r.compared,
        differences: r.differences,
        error: r.error,
    };
    for project in projects(shared) {
        let settings = shared.project_settings(&project.id).unwrap_or_default().work;
        let name = shared.db.project(&project.id).ok().flatten().map(|p| p.name).unwrap_or_default();
        let github = settings.github && project.repo.is_some();
        let jira = !settings.jira_keys.is_empty();
        if !github && !jira {
            continue;
        }
        shared.work.set_checking(&project.id, true);
        changed(app);
        if github {
            let h = read_github(shared, &sources, &project, &settings);
            let report = if h.state == "ok" {
                live::check_github(&shared.db, &sources.gh, &project, h.account.as_deref().unwrap_or(""))
            } else {
                live::Report { source: "github".into(), project_id: project.id.clone(), error: h.error.clone(), ..Default::default() }
            };
            out.push(checked(&name, report));
            let _ = relink(shared);
        }
        if jira {
            let h = read_jira(shared, &sources, &project, &settings);
            let report = if h.state == "ok" {
                live::check_jira(&shared.db, &sources.twg, &project, &settings, now_ms())
            } else {
                live::Report { source: "jira".into(), project_id: project.id.clone(), error: h.error.clone(), ..Default::default() }
            };
            out.push(checked(&name, report));
        }
        shared.work.set_checking(&project.id, false);
        let _ = relink(shared);
        changed(app);
    }
    out
}

/// Jira's boards for the choice in the Work page's header: the boards of every project's Jira projects, read now, by
/// name. Kept, so the choice opens at once next time.
pub fn read_boards(shared: &Shared) -> Result<Vec<model::Board>> {
    let sources = shared.work.sources();
    let mut keys: Vec<String> = projects(shared).into_iter().flat_map(|p| p.jira_keys).collect();
    keys.sort();
    keys.dedup();
    let mut all: Vec<model::Board> = Vec::new();
    for key in &keys {
        for b in jira::boards(&sources.twg, "", key)? {
            if !all.iter().any(|x| x.id == b.id) {
                all.push(b);
            }
        }
    }
    all.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.id.cmp(&b.id)));
    shared.db.work_put_boards(&all)?;
    Ok(all)
}

/// You picked a Jira board for the Work page (or none: everything read). The pick is kept at once; then the tickets
/// already read are asked about, so the page shows the board without waiting for a whole read of Jira.
pub fn pick_board(app: &Events, shared: &Shared, board: Option<model::Board>) -> Result<()> {
    shared.db.work_set_board(board.as_ref())?;
    changed(app);
    let Some(board) = board else { return Ok(()) };
    let sources = shared.work.sources();
    let read = snapshot_of(&shared.db)?;
    for project in projects(shared).into_iter().filter(|p| !p.jira_keys.is_empty()) {
        let keys: Vec<String> = read.tickets.iter().filter(|t| read.ticket_projects.get(&t.key) == Some(&project.id)).map(|t| t.key.clone()).collect();
        match watch::board_keys(&sources.twg, &board, &keys) {
            Ok(Some(on)) => shared.db.work_put_board_keys(&board.id, &project.id, &on, now_ms())?,
            // it can't be read this way (the board is gone, a ticket was deleted since the last read, Jira can't be
            // reached): a whole read is asked for, and its health line says what is wrong
            Ok(None) | Err(_) => shared.work.want(&project.id),
        }
    }
    changed(app);
    Ok(())
}

/// What a PR's state is, in a few words: "changes requested, checks failing (SonarCloud), 2 open threads".
pub fn pr_summary(pr: &Pr) -> String {
    let mut parts: Vec<String> = Vec::new();
    match pr.state.as_str() {
        "MERGED" => parts.push("merged".into()),
        "CLOSED" => parts.push("closed".into()),
        _ if pr.draft => parts.push("draft".into()),
        _ => {}
    }
    match pr.review_decision.as_deref() {
        Some("CHANGES_REQUESTED") => parts.push("changes requested".into()),
        Some("APPROVED") => parts.push("approved".into()),
        Some("REVIEW_REQUIRED") if pr.state == "OPEN" => parts.push("waiting for review".into()),
        _ => {}
    }
    if !pr.failing.is_empty() {
        let names: Vec<&str> = pr.failing.iter().map(|c| c.name.as_str()).collect();
        parts.push(format!("checks failing ({})", names.join(", ")));
    } else if pr.running > 0 {
        parts.push("checks running".into());
    }
    if pr.mergeable.as_deref() == Some("CONFLICTING") {
        parts.push("conflicts with its base".into());
    }
    if pr.unresolved > 0 {
        parts.push(format!("{} open review thread{}", pr.unresolved, if pr.unresolved == 1 { "" } else { "s" }));
    }
    if parts.is_empty() {
        parts.push("open".into());
    }
    parts.join(", ")
}

/// What Enkidu tells a starting session about its own work: its PRs, their tickets, and what waits on you.
pub fn session_context(db: &Db, session_id: &str) -> Result<String> {
    let me = format!("session:{session_id}");
    let links = db.work_links()?;
    let prs: HashMap<String, Pr> = db.work_prs()?.into_iter().map(|(p, _)| (format!("pr:{}", p.reference()), p)).collect();
    let mine: Vec<&Pr> = links.iter().filter(|l| l.b == me).filter_map(|l| prs.get(&l.a)).collect();
    if mine.is_empty() {
        return Ok(String::new());
    }
    let mut seen = HashSet::new();
    let tickets: HashMap<String, String> = db.work_tickets()?.into_iter().map(|(t, _)| (t.key.clone(), t.status.clone())).collect();
    let items = db.work_items()?;
    let mut lines = vec!["Enkidu knows this session's work (read-only from GitHub and Jira):".to_string()];
    for pr in mine {
        if !seen.insert(pr.reference()) {
            continue;
        }
        let r = format!("pr:{}", pr.reference());
        let keys: Vec<String> = links
            .iter()
            .filter(|l| l.b == r && l.a.starts_with("ticket:"))
            .map(|l| {
                let key = l.a.trim_start_matches("ticket:");
                match tickets.get(key) {
                    Some(status) => format!("{key} ({status})"),
                    None => key.to_string(),
                }
            })
            .collect();
        let ticket = if keys.is_empty() { String::new() } else { format!(", ticket {}", keys.join(", ")) };
        lines.push(format!("- PR #{} {} → {}{}: {}", pr.number, pr.head, pr.base, ticket, pr_summary(pr)));
        for item in items.iter().filter(|i| i.state == "open" && i.pr.as_deref() == Some(&pr.reference())) {
            lines.push(format!("  - {}", item.title));
        }
    }
    Ok(lines.join("\n"))
}

/// Everything the Work page shows: only the sessions that belong to something.
pub fn snapshot(shared: &Shared) -> Result<Snapshot> {
    let mut s = snapshot_of(&shared.db)?;
    s.checking = shared.work.checking();
    Ok(s)
}

pub fn snapshot_of(db: &Db) -> Result<Snapshot> {
    let links = db.work_links()?;
    let linked: HashSet<&str> = links.iter().flat_map(|l| [l.a.as_str(), l.b.as_str()]).filter_map(|t| t.strip_prefix("session:")).collect();
    let sessions = db.work_sessions()?.into_iter().filter(|s| linked.contains(s.id.as_str())).collect();
    // only what each project's latest read saw: a ticket no longer yours, a PR merged long ago, drop off the page
    // (a failed read changes nothing, so the last good read stays on it)
    let pr_reads = db.work_latest_reads("work_prs")?;
    let ticket_reads = db.work_latest_reads("work_tickets")?;
    let prs = db.work_prs()?.into_iter().filter(|(p, seen)| pr_reads.get(&p.project_id) == Some(seen)).map(|(p, _)| p).collect();
    let rows: Vec<_> = db.work_ticket_rows()?.into_iter().filter(|(_, project, seen)| ticket_reads.get(project) == Some(seen)).collect();
    let ticket_projects = rows.iter().map(|(t, project, _)| (t.key.clone(), project.clone())).collect();
    let tickets = rows.into_iter().map(|(t, _, _)| t).collect();
    Ok(Snapshot {
        health: db.work_health()?,
        prs,
        tickets,
        ticket_projects,
        hidden: db.work_hidden()?,
        boards: db.work_boards()?,
        board: db.work_board_pick()?,
        sessions,
        // what waits (or sleeps) now, and what was done or closed in the last three days (for the timeline)
        items: db
            .work_items()?
            .into_iter()
            .filter(|i| matches!(i.state.as_str(), "open" | "snoozed") || i.closed_at.unwrap_or(i.opened_at) > now_ms() - 3 * 86_400_000)
            .collect(),
        events: db.work_events(300)?,
        reports: db.work_reports(now_ms() - 30 * 86_400_000)?,
        flows: db.work_flows()?,
        flow_kinds: db.work_flow_kinds()?,
        flow_defaults: db.work_flow_defaults()?,
        flow_runs: db.work_flow_runs(60)?,
        own_tasks: db.work_own_tasks()?,
        links,
        checking: false,
    })
}

/// Starts the background work: the workspaces' sessions, then the transcript index, now and every minute.
pub fn start(app: Events, shared: std::sync::Arc<Shared>) {
    let _ = std::thread::Builder::new().name("work-transcripts".into()).spawn(move || while !shared.work.stopped() {
        let _ = sync_workspaces(&shared);
        // what Claude outside Enkidu recorded through `enkidu event` (the hooks added from Settings)
        match eventlog::ingest(&shared.db, &shared.data_dir.join("events.jsonl")) {
            Ok(got) if got.events > 0 => {
                if got.refresh {
                    for p in shared.db.projects().unwrap_or_default() {
                        shared.work.want(&p.id);
                    }
                }
                if relink(&shared).is_ok() {
                    changed(&app);
                }
            }
            Ok(_) => {}
            Err(e) => eprintln!("work index: the event log couldn't be read: {e:#}"),
        }
        match scan_transcripts(&shared) {
            Ok(stats) if stats.read > 0 => {
                if relink(&shared).is_ok() {
                    changed(&app);
                }
            }
            Ok(_) => {}
            Err(e) => eprintln!("work index: the transcript scan failed: {e:#}"),
        }
        std::thread::sleep(scan_every());
    });
}

/// Starts the watchers: every project's GitHub (and Jira) on its interval, on Refresh, and after a push or a turn.
pub fn start_watcher(app: Events, shared: std::sync::Arc<Shared>) {
    let _ = std::thread::Builder::new().name("work-watcher".into()).spawn(move || while !shared.work.stopped() {
        let sources = shared.work.sources();
        let wanted = shared.work.wanted();
        for project in projects(&shared) {
            let now = now_ms();
            let settings = shared.project_settings(&project.id).unwrap_or_default().work;
            let wants = wanted.contains_key(&project.id);
            let github_due = watch::due(watch::health_of(&shared.db, &project.id, "github").as_ref(), wants, now);
            let jira_due = watch::due(watch::health_of(&shared.db, &project.id, "jira").as_ref(), wants, now);
            if !github_due && !jira_due {
                continue;
            }
            shared.work.set_checking(&project.id, true);
            changed(&app);
            if github_due {
                read_github(&shared, &sources, &project, &settings);
                // new PRs name their tickets before Jira is asked for them
                let _ = relink(&shared);
            }
            if jira_due {
                read_jira(&shared, &sources, &project, &settings);
            }
            shared.work.read_done(&project.id, wanted.get(&project.id).copied());
            shared.work.set_checking(&project.id, false);
            let _ = relink(&shared);
            changed(&app);
        }
        // the morning summary, once a day at its time (or the first time Enkidu runs after it)
        let (day, minutes) = workflows::local_now();
        for project in projects(&shared) {
            let settings = shared.project_settings(&project.id).unwrap_or_default().work;
            if !settings.morning_summary {
                continue;
            }
            let ran = shared.db.work_workflow_ran(&project.id, workflows::MORNING, &day).unwrap_or(true);
            if workflows::morning_due(settings.morning_summary, &settings.morning_at, minutes, ran) {
                if let Err(e) = start_morning(&app, &shared, &project.id, &day) {
                    eprintln!("work index: the morning summary couldn't start: {e:#}");
                }
            }
        }
        shared.work.sleep(std::time::Duration::from_secs(5));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::work::links::tests as l;
    use crate::work::model::Check;
    use crate::work::store::tests::db;

    #[test]
    fn a_refresh_asked_for_while_a_read_runs_gets_a_read_of_its_own() {
        let hub = Hub::default();
        hub.want("p1");
        let seen = hub.wanted(); // the read starts
        hub.want("p1"); // Refresh pressed (or Claude pushed) while it runs
        hub.read_done("p1", seen.get("p1").copied());
        assert!(hub.wanted().contains_key("p1"), "still wanted: the next loop reads again");
        let seen = hub.wanted();
        hub.read_done("p1", seen.get("p1").copied());
        assert!(hub.wanted().is_empty(), "read after the request: done");
        // a read nobody asked for clears nothing asked for during it
        hub.want("p2");
        hub.read_done("p2", None);
        assert!(hub.wanted().contains_key("p2"));
    }

    #[test]
    fn a_core_given_its_own_programs_runs_those_in_place_of_gh_and_twg() {
        let hub = Hub::default();
        let gh = run::Tool::new("node /x/fake-gh.mjs", "/usr/bin");
        let mut twg = run::Tool::new("node /x/fake-twg.mjs", "/usr/bin");
        twg.env = vec![("FAKE_TWG_MODE_FILE".into(), "/x/twg.mode".into())];
        hub.use_sources(watch::Sources { gh, twg });
        let sources = hub.sources();
        assert_eq!((sources.gh.name(), sources.twg.name()), ("fake-gh.mjs".to_string(), "fake-twg.mjs".to_string()));
        assert_eq!(sources.twg.env, [("FAKE_TWG_MODE_FILE".to_string(), "/x/twg.mode".to_string())]);
    }

    #[test]
    fn a_stopped_hub_wakes_the_watcher_that_sleeps() {
        let hub = std::sync::Arc::new(Hub::default());
        assert!(!hub.stopped());
        let sleeper = {
            let hub = hub.clone();
            std::thread::spawn(move || {
                let started = std::time::Instant::now();
                hub.sleep(std::time::Duration::from_secs(30));
                started.elapsed()
            })
        };
        std::thread::sleep(std::time::Duration::from_millis(50));
        hub.stop();
        assert!(hub.stopped());
        let slept = sleeper.join().unwrap();
        assert!(slept < std::time::Duration::from_secs(10), "it slept {slept:?}");
    }

    #[test]
    fn a_starting_session_hears_about_its_prs_and_tickets() {
        let (_d, db) = db();
        let mut pr = l::pr(2395, "feature/SH-21533-totals", "SH-21533 Totals");
        pr.review_decision = Some("CHANGES_REQUESTED".into());
        pr.failing = vec![Check { name: "SonarCloud Code Analysis".into(), state: "CANCELLED".into(), url: None }];
        pr.unresolved = 2;
        db.work_put_prs("p1", &[pr, l::pr(1, "feature/other", "Other")], now_ms()).unwrap();
        l::session(&db, "s1", "/code/shop", "hook");
        db.work_session_branch("s1", "feature/SH-21533-totals", "branch", "worked", 1).unwrap();
        links::relink(&db, &[l::shop()], now_ms(), &|_| None).unwrap();
        let text = session_context(&db, "s1").unwrap();
        assert!(text.contains("PR #2395 feature/SH-21533-totals → development, ticket SH-21533: changes requested, checks failing (SonarCloud Code Analysis), 2 open review threads"), "{text}");
        assert!(!text.contains("#1 "), "only its own PRs");
        assert_eq!(session_context(&db, "nobody").unwrap(), "", "nothing to say: says nothing");
    }

    #[test]
    fn you_mark_items_done_or_snooze_them_and_it_is_remembered() {
        let (_d, db) = db();
        let item = crate::work::model::Item {
            id: "thread:t1".into(), kind: "remark".into(), source: "github".into(), project_id: "p1".into(),
            pr: Some("Org/shop#1".into()), ticket: None, title: "Remark from mira".into(), detail: String::new(), who: None,
            url: None, path: None, line: None, at: "2026-11-05T10:00:00Z".into(), state: "open".into(), opened_at: 1,
            closed_at: None, snoozed_until: None, bot: false,
        };
        db.work_put_item(&item).unwrap();
        set_item(&db, "thread:t1", "snoozed", None, 1_000).unwrap();
        let i = db.work_items().unwrap().pop().unwrap();
        assert_eq!((i.state.as_str(), i.snoozed_until), ("snoozed", Some(1_000 + 86_400_000)), "a day by default");
        set_item(&db, "thread:t1", "done", None, 2_000).unwrap();
        let i = db.work_items().unwrap().pop().unwrap();
        assert_eq!((i.state.as_str(), i.snoozed_until), ("done", None));
        let history: Vec<String> = db.work_events(10).unwrap().into_iter().map(|e| e.text).collect();
        assert_eq!(history, ["Marked done: Remark from mira", "Snoozed: Remark from mira"]);
        assert!(set_item(&db, "thread:t1", "deleted", None, 3_000).is_err());
        assert!(set_item(&db, "nope", "done", None, 3_000).is_err());
    }
}
