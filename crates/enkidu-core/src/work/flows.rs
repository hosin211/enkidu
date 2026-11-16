//! Scheduled workflows (the Work page's "Workflows"): tasks you pick, each with the message its Claude gets, run one
//! after another from a time you choose, each for at most `minutes`. A task runs in the Claude you picked for it (its
//! workspace) or in a new workspace made for it; with Live on, the project's main folder follows the task that runs and
//! goes back to what it was at the end. Claude says how a task ended with `enkidu report` (done, wants your review, stuck),
//! with screenshots, and that shows on the task in the morning. A task is over when Claude has reported and its turn has
//! ended (its Stop hook), and that same hook wakes the runner, so Live and the next task follow in the same second. A
//! Claude that stops without a report is asked once; a task whose time runs out is left as it is and the next one
//! starts. A turn cut short by an API error (Claude Code's StopFailure hook: the usage limit, a login that ran out)
//! ends its task with Claude Code's own words, and the run stops there when every later task would hit it too.
//!
//! Unlike everything else in Enkidu, a workflow presses Enter: you wrote and read its messages when you set it up,
//! and it runs while you're away. A run whose start time passed while Enkidu wasn't running doesn't start late on
//! its own (it says it was missed; Run now starts it).

use anyhow::{anyhow, bail, Result};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::process::Child;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
use crate::events::{Event, Events};

use super::reports::Report;
use crate::db::{now_ms, Db, Workspace};
use crate::state::Shared;

/// Stopped after working, without a report: asked after this long.
pub const QUIET_SEEN: i64 = 45_000;
/// Never seen working yet (a new Claude still starting): given this long before it is asked.
pub const QUIET_UNSEEN: i64 = 180_000;
/// Asked for its report and quiet since: the task closes as "no report" after this long.
pub const AFTER_NUDGE: i64 = 240_000;
/// Waiting for you (a question it asked) this long: the task is yours, the next one starts.
pub const ASKED_FOR: i64 = 600_000;
/// A start time this far in the past when Enkidu sees it (it wasn't running, the Mac slept): missed, not started.
pub const MISSED_AFTER: i64 = 30 * 60_000;
/// How often the runner looks when nothing wakes it sooner.
const TICK: Duration = Duration::from_secs(3);

/// Set (and the runner woken) when something it waits for happened: a Claude's turn ended, a report came in, Run now.
static WAKE: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());

/// The runner looks now instead of on its next tick: the end of a task's turn hands over to the next task at once.
pub fn wake() {
    *WAKE.0.lock().unwrap_or_else(|e| e.into_inner()) = true;
    WAKE.1.notify_all();
}

/// Waits until the runner's next look: a tick, or less when it is woken.
fn nap() {
    let mut woken = WAKE.0.lock().unwrap_or_else(|e| e.into_inner());
    if !*woken {
        woken = match WAKE.1.wait_timeout(woken, TICK) {
            Ok((guard, _)) => guard,
            Err(poisoned) => poisoned.into_inner().0,
        };
    }
    *woken = false;
}

/// An API error that ended a Claude's turn, as Claude Code's StopFailure hook tells it.
#[derive(Clone, Debug, PartialEq)]
pub struct Failure {
    /// Claude Code's name for it: rate_limit (the usage limit), authentication_failed, overloaded, server_error...
    pub kind: String,
    /// What Claude Code showed for it ("You've hit your limit · resets 3am")
    pub text: String,
    pub at: i64,
}

/// The errors every later task would run into too (the account's usage limit or login, the model): the run stops at
/// the first. The others (the API overloaded or failing for a while, one conversation grown too long) end only the task.
const STOPS_THE_RUN: [&str; 8] = [
    "rate_limit",
    "authentication_failed",
    "oauth_org_not_allowed",
    "account_on_hold",
    "verification_required",
    "billing_error",
    "cloud_credential_error",
    "model_not_found",
];

/// The last API error that ended a turn, per workspace. Only a task that started before it ends on it.
static FAILURES: Mutex<BTreeMap<String, Failure>> = Mutex::new(BTreeMap::new());

/// A Claude's turn ended on an API error (its StopFailure hook): the workflow task running in its workspace, if there is
/// one, ends on it at the runner's next look, which is now.
pub fn stop_failure(workspace_id: &str, kind: &str, text: &str, now: i64) {
    let kind = if kind.trim().is_empty() { "unknown" } else { kind.trim() };
    let text: String = text.trim().chars().take(400).collect();
    FAILURES.lock().unwrap_or_else(|e| e.into_inner()).insert(workspace_id.into(), Failure { kind: kind.into(), text, at: now });
    wake();
}

fn failure_in(workspace_id: &str) -> Option<Failure> {
    FAILURES.lock().unwrap_or_else(|e| e.into_inner()).get(workspace_id).cloned()
}

fn waiting() -> String {
    "waiting".into()
}

fn saved() -> String {
    "saved".into()
}

fn custom() -> String {
    "custom".into()
}

/// What a new workflow starts from, where you set it yourself (Settings, Workflows; item 8.6): how long
/// a task gets, and the time of day it starts at.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct FlowDefaults {
    /// Minutes for each task, 5 to 480
    pub minutes: u32,
    /// The time of day, as "02:00"
    pub time: String,
}

impl Default for FlowDefaults {
    fn default() -> Self {
        FlowDefaults { minutes: 60, time: "02:00".into() }
    }
}

/// A time of day as a workflow writes it ("02:00", "7:30"): its hour and minute.
pub fn clock(text: &str) -> Option<(u32, u32)> {
    let (hours, minutes) = text.trim().split_once(':')?;
    let digits = |text: &str| !text.is_empty() && text.len() <= 2 && text.bytes().all(|b| b.is_ascii_digit());
    if !digits(hours) || minutes.len() != 2 || !digits(minutes) {
        return None;
    }
    let (hour, minute): (u32, u32) = (hours.parse().ok()?, minutes.parse().ok()?);
    (hour < 24 && minute < 60).then_some((hour, minute))
}

/// The kinds of work a task can be, as `kind` names them
pub const KINDS: [&str; 7] = ["start", "continue", "came_back", "review", "checks", "investigate", "custom"];

/// Where a task runs.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Target {
    /// An Enkidu workspace's Claude, in the conversation it is in when the task starts
    Workspace {
        #[serde(rename = "workspaceId")]
        workspace_id: String,
    },
    /// A new workspace for the card, on `branch` ("new": made from the project's base; "existing": checked out)
    New { name: String, branch: String, mode: String },
    /// An Enkidu workspace's Claude in a new conversation: the workspace's branch and changes, a clean context (the
    /// conversation it was in stays saved)
    Conversation {
        #[serde(rename = "workspaceId")]
        workspace_id: String,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FlowTask {
    /// The card: `ticket:SH-1`, or `pr:owner/repo#2` for a PR without a ticket
    pub subject: String,
    pub title: String,
    pub target: Target,
    /// What its Claude gets, word for word
    pub message: String,
    /// You edited the message: the workflow's instruction no longer changes it
    #[serde(default)]
    pub edited: bool,
    /// The kind of work (start, continue, came_back, review, checks, investigate, custom): what its instruction was
    /// made from, picked from the card's state
    #[serde(default = "custom")]
    pub kind: String,
    /// What to do on this task, its own words (its kind's, as you changed them)
    #[serde(default)]
    pub instruction: String,
    /// Its own most minutes (none: the workflow's)
    #[serde(default)]
    pub minutes: Option<u32>,
    /// waiting, running, then done, review or blocked (its report), or no_report, timed_out, asked, cut_off (an API
    /// error ended Claude's turn), failed, stopped
    #[serde(default = "waiting")]
    pub state: String,
    /// The workspace it ran in
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub started_at: Option<i64>,
    #[serde(default)]
    pub ended_at: Option<i64>,
    /// What happened, in words, when it didn't end with a report
    #[serde(default)]
    pub note: Option<String>,
    /// When its Claude was last seen working
    #[serde(default)]
    pub busy_at: Option<i64>,
    /// When its Claude was asked for the report it hadn't made
    #[serde(default)]
    pub nudged_at: Option<i64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Flow {
    #[serde(default)]
    pub id: String,
    pub project_id: String,
    pub name: String,
    /// What to do, for every task (each task's message is made from it unless you edited the message)
    pub instruction: String,
    /// When it starts (ms); none: kept, not scheduled
    pub start_at: Option<i64>,
    /// The most each task gets
    pub minutes: u32,
    /// The main folder follows the task that runs (Live), and goes back to what it was at the end
    pub live: bool,
    /// saved, scheduled, running, done, stopped, missed
    #[serde(default = "saved")]
    pub state: String,
    /// The task that runs now (its place in `tasks`)
    #[serde(default)]
    pub current: Option<u32>,
    /// The workspace that was Live when it started, put back at the end
    #[serde(default)]
    pub live_before: Option<String>,
    pub tasks: Vec<FlowTask>,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

/// One run of a workflow, kept as it ended: when it ran, how it ended (done, stopped, missed), and the workflow as it
/// was then, each task with how it went (its reports are found by its card and the time it ran).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FlowRun {
    pub id: i64,
    pub flow_id: String,
    pub started_at: Option<i64>,
    pub ended_at: i64,
    pub state: String,
    pub flow: Flow,
}

/// What the runner does about the task that runs.
#[derive(Debug, PartialEq)]
pub enum Step {
    Wait,
    /// Claude stopped without a report: ask it for one, once
    Nudge,
    /// The task is over: how it ended, and what happened when it wasn't a report
    End(String, Option<String>),
    /// An API error ended its Claude's turn before it reported: the task is cut off, with Claude Code's words for why;
    /// `stop_run` when every later task would run into the same error
    CutOff { note: String, stop_run: bool },
}

/// What the task that runs has come to: its report, Claude's state (`status`, from its hooks), whether its Claude
/// still runs, the time it has had, and an API error that ended its turn (`failed`, from its StopFailure hook).
pub fn decide(task: &FlowTask, minutes: u32, now: i64, status: Option<&str>, report: Option<&Report>, alive: bool, failed: Option<&Failure>) -> Step {
    let Some(started) = task.started_at else { return Step::Wait };
    // an error from before the task started belongs to another turn
    let failed = failed.filter(|f| f.at >= started);
    let out_of_time = now - started >= minutes as i64 * 60_000;
    if let Some(r) = report {
        // the report says how it went; the task is over once Claude's turn is over too (its Stop hook), so Live never
        // leaves a Claude still at work and the next task's Claude never works beside it
        if !alive || failed.is_some() || status != Some("working") {
            return Step::End(r.state.clone(), None);
        }
        if out_of_time {
            return Step::End(r.state.clone(), Some(format!("Its {minutes} minutes ran out while Claude was still going after its report")));
        }
        return Step::Wait;
    }
    if let Some(f) = failed {
        let note = if f.text.is_empty() { format!("Claude Code stopped on an API error ({})", f.kind) } else { f.text.clone() };
        return Step::CutOff { note, stop_run: STOPS_THE_RUN.contains(&f.kind.as_str()) };
    }
    if !alive {
        return Step::End("failed".into(), Some("Its Claude closed before it reported (Enkidu may have quit)".into()));
    }
    if out_of_time {
        return Step::End("timed_out".into(), Some(format!("Its {minutes} minutes ran out before Claude reported; it may still be working")));
    }
    let active = task.busy_at.unwrap_or(started).max(started);
    match status {
        Some("working") => Step::Wait,
        Some("needs_you") => {
            if now - active >= ASKED_FOR {
                Step::End("asked".into(), Some("Claude asked you something and waited; answer it in its workspace".into()))
            } else {
                Step::Wait
            }
        }
        // its turn ended (done), or it hasn't started one yet
        _ => match task.nudged_at {
            None => {
                let quiet = if task.busy_at.is_some() { QUIET_SEEN } else { QUIET_UNSEEN };
                if now - active >= quiet {
                    Step::Nudge
                } else {
                    Step::Wait
                }
            }
            Some(asked) => {
                if now - active.max(asked) >= AFTER_NUDGE {
                    Step::End("no_report".into(), Some("Claude stopped without a report, also when asked for one".into()))
                } else {
                    Step::Wait
                }
            }
        },
    }
}

/// Whether a scheduled workflow starts now, waits, or was missed (its time passed long ago, Enkidu wasn't running).
#[derive(Debug, PartialEq)]
pub enum Due {
    Not,
    Start,
    Missed,
}

pub fn due(flow: &Flow, now: i64) -> Due {
    match flow.start_at {
        Some(at) if flow.state == "scheduled" && at <= now => {
            if now - at > MISSED_AFTER {
                Due::Missed
            } else {
                Due::Start
            }
        }
        _ => Due::Not,
    }
}

/// A task's card as `enkidu` names it: `--ticket SH-1`, `--pr 2414`, or `--task 3` for a task of your own.
pub fn cli_flag(subject: &str) -> String {
    if let Some(key) = subject.strip_prefix("ticket:") {
        format!("--ticket {key}")
    } else if let Some(id) = super::model::own_id(subject) {
        format!("--task {id}")
    } else {
        let n = subject.rsplit('#').next().unwrap_or("");
        format!("--pr {n}")
    }
}

/// What a report is about, as `enkidu report` saves it: the ticket key, the PR's reference, or a task of your own whole
/// (`own:3`).
fn report_subject(subject: &str) -> &str {
    subject.strip_prefix("ticket:").or_else(|| subject.strip_prefix("pr:")).unwrap_or(subject)
}

/// A task's card in words: the ticket's key, the PR's reference, or "task 3" for a task of your own.
fn spoken(subject: &str) -> String {
    match super::model::own_id(subject) {
        Some(id) => format!("task {id}"),
        None => report_subject(subject).to_string(),
    }
}

/// The words for a task that stopped without saying how it went.
pub fn nudge_text(flow: &Flow, task: &FlowTask) -> String {
    let what = spoken(&task.subject);
    format!(
        "Enkidu's workflow \"{}\" here: you stopped without saying how {what} went. Report it now with \
         `enkidu report done|review|blocked {} \"what you changed, how you checked it, what is left\"`, and add screenshots \
         of what you checked with `--image FILE` (done: nothing needs the user; review: it wants their review; blocked: \
         you couldn't finish).",
        flow.name,
        cli_flag(&task.subject)
    )
}

/// Checks a workflow you saved: a project, a name, at least one task with a message, sensible minutes.
pub fn check(flow: &Flow) -> Result<()> {
    if flow.name.trim().is_empty() {
        bail!("Give the workflow a name");
    }
    if flow.tasks.is_empty() {
        bail!("Pick at least one task for the workflow");
    }
    if !(5..=480).contains(&flow.minutes) {
        bail!("Each task gets between 5 minutes and 8 hours");
    }
    for t in &flow.tasks {
        if !super::model::is_card(&t.subject) {
            bail!("A task is a ticket, a PR or a task of your own, not {}", t.subject);
        }
        if t.message.trim().is_empty() {
            bail!("The message for {} is empty", spoken(&t.subject));
        }
        if let Target::New { name, branch, mode } = &t.target {
            if name.trim().is_empty() || branch.trim().is_empty() || !matches!(mode.as_str(), "new" | "existing") {
                bail!("The new workspace for {} has no name or branch", spoken(&t.subject));
            }
        }
        if !KINDS.contains(&t.kind.as_str()) {
            bail!("{}: there is no kind of task called {}", spoken(&t.subject), t.kind);
        }
        if t.minutes.is_some_and(|m| !(5..=480).contains(&m)) {
            bail!("{} gets between 5 minutes and 8 hours", spoken(&t.subject));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------------
// Storage

fn flow_from_row(r: &rusqlite::Row) -> rusqlite::Result<Flow> {
    Ok(Flow {
        id: r.get(0)?,
        project_id: r.get(1)?,
        name: r.get(2)?,
        instruction: r.get(3)?,
        start_at: r.get(4)?,
        minutes: r.get::<_, i64>(5)? as u32,
        live: r.get::<_, i64>(6)? != 0,
        state: r.get(7)?,
        current: r.get::<_, Option<i64>>(8)?.map(|c| c as u32),
        live_before: r.get(9)?,
        tasks: Vec::new(),
        created_at: r.get(10)?,
        updated_at: r.get(11)?,
    })
}

fn task_from_row(r: &rusqlite::Row) -> rusqlite::Result<FlowTask> {
    let target: String = r.get(2)?;
    Ok(FlowTask {
        subject: r.get(0)?,
        title: r.get(1)?,
        target: serde_json::from_str(&target).unwrap_or(Target::New { name: String::new(), branch: String::new(), mode: "new".into() }),
        message: r.get(3)?,
        edited: r.get::<_, i64>(4)? != 0,
        state: r.get(5)?,
        workspace_id: r.get(6)?,
        started_at: r.get(7)?,
        ended_at: r.get(8)?,
        note: r.get(9)?,
        busy_at: r.get(10)?,
        nudged_at: r.get(11)?,
        kind: r.get(12)?,
        instruction: r.get(13)?,
        minutes: r.get::<_, Option<i64>>(14)?.map(|m| m as u32),
    })
}

impl Db {
    /// Every workflow with its tasks, the newest first.
    pub fn work_flows(&self) -> Result<Vec<Flow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, project_id, name, instruction, start_at, minutes, live, state, current, live_before, created_at, updated_at
             FROM work_flows ORDER BY created_at DESC, id",
        )?;
        let mut flows: Vec<Flow> = stmt.query_map([], flow_from_row)?.collect::<rusqlite::Result<_>>()?;
        let mut tasks = conn.prepare(
            "SELECT subject, title, target, message, edited, state, workspace_id, started_at, ended_at, note, busy_at, nudged_at, kind,
               instruction, minutes
             FROM work_flow_tasks WHERE flow_id = ?1 ORDER BY position",
        )?;
        for f in &mut flows {
            f.tasks = tasks.query_map(params![f.id], task_from_row)?.collect::<rusqlite::Result<_>>()?;
        }
        Ok(flows)
    }

    pub fn work_flow(&self, id: &str) -> Result<Option<Flow>> {
        Ok(self.work_flows()?.into_iter().find(|f| f.id == id))
    }

    /// Saves a workflow whole: the flow, and its tasks in order.
    pub fn work_put_flow(&self, flow: &Flow) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO work_flows (id, project_id, name, instruction, start_at, minutes, live, state, current, live_before, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(id) DO UPDATE SET project_id = ?2, name = ?3, instruction = ?4, start_at = ?5, minutes = ?6, live = ?7,
               state = ?8, current = ?9, live_before = ?10, updated_at = ?12",
            params![
                flow.id,
                flow.project_id,
                flow.name,
                flow.instruction,
                flow.start_at,
                flow.minutes as i64,
                flow.live as i64,
                flow.state,
                flow.current.map(|c| c as i64),
                flow.live_before,
                flow.created_at,
                flow.updated_at
            ],
        )?;
        tx.execute("DELETE FROM work_flow_tasks WHERE flow_id = ?1", params![flow.id])?;
        for (n, t) in flow.tasks.iter().enumerate() {
            tx.execute(
                "INSERT INTO work_flow_tasks (flow_id, position, subject, title, target, message, edited, state, workspace_id, started_at,
                   ended_at, note, busy_at, nudged_at, kind, instruction, minutes)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
                params![
                    flow.id,
                    n as i64,
                    t.subject,
                    t.title,
                    serde_json::to_string(&t.target)?,
                    t.message,
                    t.edited as i64,
                    t.state,
                    t.workspace_id,
                    t.started_at,
                    t.ended_at,
                    t.note,
                    t.busy_at,
                    t.nudged_at,
                    t.kind,
                    t.instruction,
                    t.minutes.map(|m| m as i64)
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Keeps a run as it ended: the workflow then, its tasks and how each went. It started when its first task did
    /// (a missed run: at its time).
    pub fn work_put_flow_run(&self, flow: &Flow, ended_at: i64) -> Result<i64> {
        let started = flow.tasks.iter().filter_map(|t| t.started_at).min().or(flow.start_at);
        let conn = self.conn();
        conn.execute(
            "INSERT INTO work_flow_runs (flow_id, started_at, ended_at, state, flow) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![flow.id, started, ended_at, flow.state, serde_json::to_string(flow)?],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Workflows that ended before their runs were kept: each kept once as its run, so it is in the list of runs and
    /// running it again doesn't lose how it went. Only while no run is kept yet.
    pub fn work_keep_ended_flows(&self) -> Result<()> {
        let kept: i64 = self.conn().query_row("SELECT COUNT(*) FROM work_flow_runs", [], |r| r.get(0))?;
        if kept > 0 {
            return Ok(());
        }
        for f in self.work_flows()? {
            if matches!(f.state.as_str(), "done" | "stopped" | "missed") {
                let ended = f.tasks.iter().filter_map(|t| t.ended_at).max().unwrap_or(0).max(f.updated_at);
                self.work_put_flow_run(&f, ended)?;
            }
        }
        Ok(())
    }

    /// The runs kept, the newest first.
    pub fn work_flow_runs(&self, limit: usize) -> Result<Vec<FlowRun>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT id, flow_id, started_at, ended_at, state, flow FROM work_flow_runs ORDER BY ended_at DESC, id DESC LIMIT ?1")?;
        let rows = stmt.query_map(params![limit as i64], |r| {
            let flow: String = r.get(5)?;
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<i64>>(2)?, r.get::<_, i64>(3)?, r.get::<_, String>(4)?, flow))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, flow_id, started_at, ended_at, state, flow) = row?;
            // a run whose kept workflow can't be read (a later version changed its shape) is left out, not an error
            if let Ok(flow) = serde_json::from_str::<Flow>(&flow) {
                out.push(FlowRun { id, flow_id, started_at, ended_at, state, flow });
            }
        }
        Ok(out)
    }

    /// Your own default words for each kind of task (`start`, `came_back`…), where you changed them.
    pub fn work_flow_kinds(&self) -> Result<std::collections::BTreeMap<String, String>> {
        let conn = self.conn();
        let text: Option<String> = conn.query_row("SELECT value FROM work_prefs WHERE key = 'flow_kinds'", [], |r| r.get(0)).ok();
        Ok(text.and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default())
    }

    /// What a new workflow starts from: yours where you set it, else the usual (60 minutes a task, at 02:00).
    pub fn work_flow_defaults(&self) -> Result<FlowDefaults> {
        let conn = self.conn();
        let text: Option<String> = conn.query_row("SELECT value FROM work_prefs WHERE key = 'flow_defaults'", [], |r| r.get(0)).ok();
        Ok(text.and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default())
    }

    pub fn work_set_flow_defaults(&self, defaults: &FlowDefaults) -> Result<()> {
        anyhow::ensure!((5..=480).contains(&defaults.minutes), "Each task gets between 5 minutes and 8 hours");
        anyhow::ensure!(clock(&defaults.time).is_some(), "The start time is a time of day, like 02:00");
        self.conn().execute(
            "INSERT INTO work_prefs (key, value) VALUES ('flow_defaults', ?1) ON CONFLICT(key) DO UPDATE SET value = ?1",
            params![serde_json::to_string(defaults)?],
        )?;
        Ok(())
    }

    pub fn work_set_flow_kinds(&self, kinds: &std::collections::BTreeMap<String, String>) -> Result<()> {
        for k in kinds.keys() {
            anyhow::ensure!(KINDS.contains(&k.as_str()), "There is no kind of task called {k}");
        }
        self.conn().execute(
            "INSERT INTO work_prefs (key, value) VALUES ('flow_kinds', ?1) ON CONFLICT(key) DO UPDATE SET value = ?1",
            params![serde_json::to_string(kinds)?],
        )?;
        Ok(())
    }

    pub fn work_delete_flow(&self, id: &str) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM work_flow_tasks WHERE flow_id = ?1", params![id])?;
        tx.execute("DELETE FROM work_flows WHERE id = ?1", params![id])?;
        tx.commit()?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// What you do from the Work page

/// Saves a workflow you made or changed: scheduled when it has a start time, kept otherwise. One that runs can't be
/// changed (stop it first).
pub fn save(db: &Db, flow: Flow, now: i64) -> Result<Flow> {
    let before = if flow.id.is_empty() { None } else { db.work_flow(&flow.id)? };
    let flow = as_saved(flow, before.as_ref(), now)?;
    db.work_put_flow(&flow)?;
    Ok(flow)
}

/// A workflow as it is kept when you save it, given the one kept under its id before (none: it is new): checked,
/// with an id, scheduled when it has a start time, its tasks waiting again. Nothing is written here: `save` does
/// that, and a window without a core (a screen test, a picture) keeps the same workflow in its own index.
pub fn as_saved(mut flow: Flow, before: Option<&Flow>, now: i64) -> Result<Flow> {
    check(&flow)?;
    if before.is_some_and(|b| b.state == "running") {
        bail!("The workflow is running: stop it before you change it");
    }
    if flow.id.is_empty() {
        flow.id = uuid::Uuid::new_v4().to_string();
    }
    flow.created_at = before.map(|b| b.created_at).unwrap_or(now);
    flow.updated_at = now;
    flow.state = if flow.start_at.is_some() { "scheduled".into() } else { "saved".into() };
    flow.current = None;
    flow.live_before = None;
    for t in &mut flow.tasks {
        *t = FlowTask { state: "waiting".into(), workspace_id: None, started_at: None, ended_at: None, note: None, busy_at: None, nudged_at: None, ..t.clone() };
    }
    Ok(flow)
}

/// Starts a workflow now (again, if it ran before): its tasks start over.
pub fn run_now(db: &Db, id: &str, now: i64) -> Result<Flow> {
    let flow = db.work_flow(id)?.ok_or_else(|| anyhow!("That workflow is gone"))?;
    if flow.state == "running" {
        bail!("It is running already");
    }
    save(db, Flow { start_at: Some(now), ..flow }, now)
}

// ---------------------------------------------------------------------------------------------------------------------
// The runner

/// Starts the runner: every few seconds it starts what is due and looks after the task that runs. It ends with the
/// core's other Work loops (`shared.work.stop()`, then `wake()`): a test's core is done, where the app's never is.
pub fn start(app: Events, shared: Arc<Shared>) {
    let _ = std::thread::Builder::new().name("work-flows".into()).spawn(move || {
        while !shared.work.stopped() {
            tick(&app, &shared, now_ms());
            nap();
        }
    });
}

/// Serializes the runner's work with your stop and delete (each changes the same rows).
static RUNNING: Mutex<()> = Mutex::new(());

pub fn tick(app: &Events, shared: &Arc<Shared>, now: i64) {
    let _one = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
    let Ok(flows) = shared.db.work_flows() else { return };
    let mut changed = false;
    for flow in flows {
        match (flow.state.as_str(), due(&flow, now)) {
            (_, Due::Missed) => {
                let mut f = flow;
                f.state = "missed".into();
                f.updated_at = now;
                let _ = shared.db.work_put_flow(&f);
                // kept with the runs: the night it didn't run shows too
                let _ = shared.db.work_put_flow_run(&f, now);
                changed = true;
            }
            (_, Due::Start) => {
                begin(app, shared, flow, now);
                changed = true;
            }
            ("running", _) => changed |= step(app, shared, flow, now),
            _ => {}
        }
    }
    let soon = shared.db.work_flows().unwrap_or_default().iter().any(|f| {
        f.state == "running" || (f.state == "scheduled" && f.start_at.is_some_and(|at| at - now < 24 * 3_600_000))
    });
    keep_awake(soon);
    if changed {
        super::changed(app);
    }
}

fn begin(app: &Events, shared: &Arc<Shared>, mut flow: Flow, now: i64) {
    flow.state = "running".into();
    flow.current = Some(0);
    flow.live_before = shared.db.project(&flow.project_id).ok().flatten().and_then(|p| p.live_workspace_id);
    flow.updated_at = now;
    let _ = shared.db.work_put_flow(&flow);
    step(app, shared, flow, now);
}

/// Looks after the task that runs; true when anything changed.
fn step(app: &Events, shared: &Arc<Shared>, mut flow: Flow, now: i64) -> bool {
    let Some(i) = flow.current.map(|c| c as usize).filter(|&i| i < flow.tasks.len()) else {
        finish(app, shared, flow, now);
        return true;
    };
    match flow.tasks[i].state.as_str() {
        "waiting" => {
            start_task(app, shared, &mut flow, i);
            let _ = shared.db.work_put_flow(&flow);
            if flow.tasks[i].state != "running" {
                advance(app, shared, flow, now);
            }
            true
        }
        "running" => {
            let task = flow.tasks[i].clone();
            let ws = task.workspace_id.clone().unwrap_or_default();
            let status = shared.status(&ws);
            let mut changed = false;
            if status.as_deref() == Some("working") {
                flow.tasks[i].busy_at = Some(now);
                changed = true;
            }
            let report = task.started_at.and_then(|since| report_since(&shared.db, report_subject(&task.subject), since));
            let alive = shared.pty.running_claude(&ws);
            let minutes = flow.tasks[i].minutes.unwrap_or(flow.minutes);
            let failed = failure_in(&ws);
            match decide(&flow.tasks[i], minutes, now, status.as_deref(), report.as_ref(), alive, failed.as_ref()) {
                Step::Wait => {
                    if changed {
                        let _ = shared.db.work_put_flow(&flow);
                    }
                    false
                }
                Step::Nudge => {
                    flow.tasks[i].nudged_at = Some(now);
                    let _ = shared.db.work_put_flow(&flow);
                    if let Some(id) = shared.pty.claude_terminal(&ws) {
                        let text = nudge_text(&flow, &task);
                        let shared = shared.clone();
                        // typing waits a moment before Enter: not on the runner's own time
                        std::thread::spawn(move || {
                            let _ = crate::pty::paste(&shared, &id, &text, true);
                        });
                    }
                    true
                }
                Step::End(state, note) => {
                    end_task(shared, &mut flow, i, &state, note, now);
                    advance(app, shared, flow, now);
                    true
                }
                Step::CutOff { note, stop_run } => {
                    end_task(shared, &mut flow, i, "cut_off", Some(note), now);
                    if stop_run {
                        // the tasks after it would only run into the same error: the run ends here, the main folder
                        // goes back, and the tasks that didn't get their turn stay as they were
                        end_run(app, shared, flow, now, "stopped");
                    } else {
                        advance(app, shared, flow, now);
                    }
                    true
                }
            }
        }
        // already over: on to the next
        _ => {
            advance(app, shared, flow, now);
            true
        }
    }
}

/// Git and GitHub commands that make or send commits: `git commit`, `push`, `merge`, `rebase`, `cherry-pick`, `am`,
/// `revert`, `pull`, and `gh pr create` or `merge`, wherever they stand in a command line (`cd x && git -C y commit`).
static COMMITTING: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(
        r"(?:^|[\s;&|(`])(?:git(?:\s+(?:-[Cc]\s+\S+|--?[A-Za-z][\w-]*(?:=\S+)?))*\s+(commit|push|merge|rebase|cherry-pick|am|revert|pull)\b|gh\s+pr\s+(create|merge)\b)",
    )
    .unwrap()
});

/// What a command would do to the branch, when it makes or sends commits: "commit", "push", "gh pr create"...
pub fn commits(command: &str) -> Option<String> {
    let m = COMMITTING.captures(command)?;
    Some(match (m.get(1), m.get(2)) {
        (Some(git), _) => format!("git {}", git.as_str()),
        (_, Some(gh)) => format!("gh pr {}", gh.as_str()),
        _ => "it".into(),
    })
}

/// Claude in this workspace is about to run a shell command (its PreToolUse hook for Bash). In a workflow's task that
/// runs here, a command that makes or sends commits is refused, with the reason Claude is given: the work stays
/// uncommitted, for you to read in the diff (and for Live, which copies uncommitted changes). Anything else, and every
/// command outside a workflow's task, goes as before.
pub fn tool_guard(db: &Db, workspace_id: &str, command: &str) -> Option<String> {
    let what = commits(command)?;
    let flow = db.work_flows().ok()?.into_iter().find(|f| {
        f.state == "running"
            && f.current.map(|c| c as usize).and_then(|i| f.tasks.get(i)).is_some_and(|t| t.state == "running" && t.workspace_id.as_deref() == Some(workspace_id))
    })?;
    Some(format!(
        "Enkidu's workflow \"{}\" keeps every change uncommitted, for the user to review in the diff in the morning: `{what}` isn't run during a workflow's task. Leave your changes as they are and report with `enkidu report`.",
        flow.name
    ))
}

/// Claude in this workspace stopped (its Stop hook). When it runs a workflow's task, hasn't reported yet and wasn't
/// asked before, it is asked now, by the hook's answer (Claude goes on with it): the words for that, else nothing.
/// `active` is Claude Code's `stop_hook_active` (it already went on because of a stop hook): never asked then. While
/// the runner is busy (making a workspace), nothing is asked here; the runner asks itself later.
pub fn stop_hook(db: &Db, workspace_id: &str, active: bool, now: i64) -> Option<String> {
    if active {
        return None;
    }
    // the runner may be in the middle of a step (typing a message waits a moment before Enter): a short wait, inside
    // the hook's own time limit, then nothing is asked
    let started = std::time::Instant::now();
    let _one = loop {
        match RUNNING.try_lock() {
            Ok(guard) => break guard,
            Err(std::sync::TryLockError::Poisoned(p)) => break p.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) if started.elapsed() < Duration::from_millis(1500) => std::thread::sleep(Duration::from_millis(50)),
            Err(std::sync::TryLockError::WouldBlock) => return None,
        }
    };
    for mut flow in db.work_flows().ok()?.into_iter().filter(|f| f.state == "running") {
        let Some(i) = flow.current.map(|c| c as usize).filter(|&i| i < flow.tasks.len()) else { continue };
        let task = &flow.tasks[i];
        if task.state != "running" || task.workspace_id.as_deref() != Some(workspace_id) || task.nudged_at.is_some() {
            continue;
        }
        let since = task.started_at?;
        if report_since(db, report_subject(&task.subject), since).is_some() {
            return None;
        }
        let reason = nudge_text(&flow, task);
        flow.tasks[i].nudged_at = Some(now);
        let _ = db.work_put_flow(&flow);
        return Some(reason);
    }
    None
}

fn report_since(db: &Db, subject: &str, since: i64) -> Option<Report> {
    // only how it ended counts; an update along the way ends nothing
    db.work_reports(since).ok()?.into_iter().find(|r| r.subject.eq_ignore_ascii_case(subject) && super::reports::STATES.contains(&r.state.as_str()))
}

fn end_task(shared: &Shared, flow: &mut Flow, i: usize, state: &str, note: Option<String>, now: i64) {
    let t = &mut flow.tasks[i];
    t.state = state.to_string();
    t.ended_at = Some(now);
    if note.is_some() {
        t.note = note;
    }
    // the timeline it is written on: the card's own
    let what = report_subject(&t.subject).to_string();
    let words = match state {
        "done" | "review" | "blocked" => super::reports::words(state).to_string(),
        "timed_out" => "Its time ran out".into(),
        "no_report" => "Claude stopped without a report".into(),
        "asked" => "Claude asked you something and waited".into(),
        "cut_off" => format!("Claude was cut off: {}", t.note.clone().unwrap_or_default()),
        "stopped" => "You stopped the workflow".into(),
        _ => format!("Didn't run: {}", t.note.clone().unwrap_or_default()),
    };
    let _ = shared.db.work_event(&what, "workflow", &format!("Workflow \"{}\": {words}", flow.name), now);
}

fn advance(app: &Events, shared: &Arc<Shared>, mut flow: Flow, now: i64) {
    let next = flow.current.map(|c| c + 1).unwrap_or(0);
    flow.current = Some(next);
    flow.updated_at = now;
    let _ = shared.db.work_put_flow(&flow);
    if (next as usize) < flow.tasks.len() {
        step(app, shared, flow, now);
    } else {
        finish(app, shared, flow, now);
    }
}

/// All its tasks are over.
fn finish(app: &Events, shared: &Arc<Shared>, flow: Flow, now: i64) {
    end_run(app, shared, flow, now, "done");
}

/// A run is over (done, or stopped): the main folder goes back to the workspace that was Live before it started, and
/// the run is kept as it ended (each task and how it went), so running the workflow again starts afresh while this run
/// stays to look back at.
fn end_run(app: &Events, shared: &Arc<Shared>, mut flow: Flow, now: i64, state: &str) {
    if flow.live {
        let live = shared.db.project(&flow.project_id).ok().flatten().and_then(|p| p.live_workspace_id);
        if live != flow.live_before {
            let back = flow.live_before.clone().filter(|w| shared.db.workspace(w).ok().flatten().is_some());
            let _ = crate::live::set_live(app, shared, &flow.project_id, back.as_deref());
        }
    }
    flow.state = state.into();
    flow.current = None;
    flow.updated_at = now;
    let _ = shared.db.work_put_flow(&flow);
    let _ = shared.db.work_put_flow_run(&flow, now);
}

/// You stopped a workflow: one that runs stops after the task that runs (its Claude isn't stopped; the task says you
/// stopped it), and the main folder goes back; a scheduled one is kept, not scheduled.
pub fn stop(app: &Events, shared: &Arc<Shared>, id: &str, now: i64) -> Result<()> {
    let _one = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
    let mut flow = shared.db.work_flow(id)?.ok_or_else(|| anyhow!("That workflow is gone"))?;
    match flow.state.as_str() {
        "running" => {
            if let Some(i) = flow.current.map(|c| c as usize).filter(|&i| i < flow.tasks.len()) {
                if matches!(flow.tasks[i].state.as_str(), "waiting" | "running") {
                    end_task(shared, &mut flow, i, "stopped", Some("You stopped the workflow".into()), now);
                }
            }
            end_run(app, shared, flow, now, "stopped");
        }
        "scheduled" => {
            flow.state = "saved".into();
            flow.start_at = None;
            flow.updated_at = now;
            shared.db.work_put_flow(&flow)?;
        }
        _ => {}
    }
    Ok(())
}

/// Deletes a workflow (one that runs is stopped first).
pub fn delete(app: &Events, shared: &Arc<Shared>, id: &str, now: i64) -> Result<()> {
    stop(app, shared, id, now)?;
    let _one = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
    shared.db.work_delete_flow(id)
}

/// Starts a task: its workspace (made when it's a new one), the card linked to that Claude, Live moved to it, and its
/// message typed in and sent.
fn start_task(app: &Events, shared: &Arc<Shared>, flow: &mut Flow, i: usize) {
    let task = flow.tasks[i].clone();
    let mut notes: Vec<String> = Vec::new();
    let result = (|| -> Result<Workspace> {
        let ws = workspace_for(app, shared, &flow.project_id, &task.target)?;
        // Live is the project's: a workflow's tasks are all of its project
        if ws.project_id != flow.project_id {
            bail!("{} is a workspace of another project than the workflow's", ws.name);
        }
        if shared.status(&ws.id).as_deref() == Some("needs_you") && shared.pty.running_claude(&ws.id) {
            bail!("Claude in {} was waiting for an answer from you; nothing was typed into it", ws.name);
        }
        // a new conversation in its workspace: the workspace's branch and changes, a clean context
        let ws = if matches!(task.target, Target::Conversation { .. }) { fresh_conversation(shared, &ws)? } else { ws };
        // it runs from now, saved before Claude hears of it: its hooks (no commits, the report it owes) know the task
        // from Claude's very first step
        let t = &mut flow.tasks[i];
        t.state = "running".into();
        t.workspace_id = Some(ws.id.clone());
        t.started_at = Some(now_ms());
        // the new workspace made for it is its workspace from now on: running the workflow again goes on there
        t.target = Target::Workspace { workspace_id: ws.id.clone() };
        shared.db.work_put_flow(flow)?;
        // the card goes to this Claude (its report finds the ticket; the card shows it)
        shared.db.work_link(&task.subject, &format!("session:{}", ws.session_id), "you", "linked by you", now_ms())?;
        if flow.live {
            if let Err(e) = crate::live::set_live(app, shared, &flow.project_id, Some(&ws.id)) {
                notes.push(format!("Live didn't move to {}: {e:#}", ws.name));
            }
        }
        deliver(app, shared, &ws, &task.message)?;
        Ok(ws)
    })();
    let now = now_ms();
    match result {
        Ok(ws) => {
            let t = &mut flow.tasks[i];
            t.note = (!notes.is_empty()).then(|| notes.join("; "));
            let _ = shared.db.work_event(report_subject(&t.subject), "workflow", &format!("Workflow \"{}\" started it in {}", flow.name, ws.name), now);
        }
        Err(e) => {
            let note = crate::redact::redact_urls(&format!("{e:#}"));
            end_task(shared, flow, i, "failed", Some(note), now);
        }
    }
}

/// The workspace's Claude moves to a new conversation for a task (the one it was in stays saved; `/resume` finds it):
/// a Claude that runs there closes first, unless it is working or waiting for you.
fn fresh_conversation(shared: &Arc<Shared>, ws: &Workspace) -> Result<Workspace> {
    if let Some(id) = shared.pty.claude_terminal(&ws.id) {
        if matches!(shared.status(&ws.id).as_deref(), Some("working") | Some("needs_you")) {
            bail!("Claude in {} was busy, so no new conversation was started there", ws.name);
        }
        shared.pty.close(&id);
    }
    let session = uuid::Uuid::new_v4().to_string();
    shared.db.set_conversation(&ws.id, &session, None, ws.claude_config_dir.as_deref())?;
    shared.db.workspace(&ws.id)?.ok_or_else(|| anyhow!("The workspace is gone"))
}

/// Types the message into the workspace's running Claude and presses Enter; with none running, starts its Claude with
/// the message (sent once it has started).
fn deliver(app: &Events, shared: &Arc<Shared>, ws: &Workspace, message: &str) -> Result<()> {
    if let Some(id) = shared.pty.claude_terminal(&ws.id) {
        crate::pty::paste(shared, &id, message, true)
    } else {
        let info = crate::commands::spawn_terminal(app, shared, &ws.id, "claude", 120, 32, Some(message.to_string()), true).map_err(|e| anyhow!(e.message))?;
        // the window shows it as the workspace's Claude tab
        app.send(Event::PtySpawned(info.clone()));
        Ok(())
    }
}

/// The workspace a task runs in: the one you picked, or a new one made for it now. A name taken since gets a number;
/// a branch there already is checked out; a branch checked out in another folder (a workspace too) gets a new branch
/// starting where it is.
fn workspace_for(app: &Events, shared: &Arc<Shared>, project_id: &str, target: &Target) -> Result<Workspace> {
    let (name, branch, mode) = match target {
        Target::Workspace { workspace_id } | Target::Conversation { workspace_id } => {
            return shared.db.workspace(workspace_id)?.ok_or_else(|| anyhow!("The workspace it was to run in is gone"));
        }
        Target::New { name, branch, mode } => (name.clone(), branch.clone(), mode.clone()),
    };
    let project = shared.db.project(project_id)?.ok_or_else(|| anyhow!("The project is gone"))?;
    let main = std::path::PathBuf::from(&project.path);
    let all = shared.db.workspaces()?;
    let free = |base: &str| {
        let taken: Vec<&str> = all.iter().filter(|w| w.project_id == project_id).map(|w| w.name.as_str()).collect();
        let mut n = base.to_string();
        let mut k = 2;
        while taken.contains(&n.as_str()) {
            n = format!("{base}-{k}");
            k += 1;
        }
        n
    };
    let mode = if mode == "new" && (crate::git::local_branch_exists(&main, &branch) || crate::git::remote_branch_exists(&main, &branch)) {
        "existing".to_string()
    } else {
        mode
    };
    let req = crate::workspace::CreateRequest { project_id: project_id.into(), name: free(&name), mode, branch: branch.clone(), base: None, session_id: None };
    // the window gets the workspace made for a task as a new tab
    let added = |ws: Workspace| {
        app.send(Event::WorkspaceAdded(ws.clone()));
        ws
    };
    match crate::commands::create_workspace_now(app, shared, &req) {
        Ok(created) => Ok(added(created.workspace)),
        Err(e) if e.code == "branch_taken" => {
            // checked out in another folder (the task's own workspace, say): you asked for a new workspace, so it gets a
            // new branch starting where that one is (git checks a branch out in one folder only); the other folder and
            // its uncommitted changes are left as they are
            let mut k = 2;
            let mut copy = format!("{branch}-{k}");
            while crate::git::local_branch_exists(&main, &copy) {
                k += 1;
                copy = format!("{branch}-{k}");
            }
            let req = crate::workspace::CreateRequest { mode: "copy".into(), branch: copy, base: Some(branch), ..req };
            crate::commands::create_workspace_now(app, shared, &req).map(|c| added(c.workspace)).map_err(|e| anyhow!(e.message))
        }
        Err(e) => Err(anyhow!(e.message)),
    }
}

/// Keeps the Mac from going to sleep on its own while a workflow is due within a day or runs (`caffeinate`, which ends
/// with Enkidu). A closed lid on battery still sleeps.
fn keep_awake(on: bool) {
    static AWAKE: Mutex<Option<Child>> = Mutex::new(None);
    let mut held = AWAKE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(child) = held.as_mut() {
        if child.try_wait().ok().flatten().is_some() {
            *held = None;
        }
    }
    match (on, held.is_some()) {
        (true, false) => {
            *held = std::process::Command::new("/usr/bin/caffeinate").args(["-i", "-s", "-w", &std::process::id().to_string()]).spawn().ok();
        }
        (false, true) => {
            if let Some(mut child) = held.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;

    fn task(started: i64) -> FlowTask {
        FlowTask {
            subject: "ticket:SH-1".into(),
            title: "Fix the export".into(),
            target: Target::Workspace { workspace_id: "w1".into() },
            message: "Fix it".into(),
            edited: false,
            state: "running".into(),
            workspace_id: Some("w1".into()),
            started_at: Some(started),
            ended_at: None,
            note: None,
            busy_at: None,
            nudged_at: None,
            kind: "start".into(),
            instruction: "Start this ticket.".into(),
            minutes: None,
        }
    }

    fn report(state: &str) -> Report {
        Report { id: 1, subject: "SH-1".into(), session_id: None, workspace_id: None, state: state.into(), text: "Done".into(), images: vec![], at: 0 }
    }

    fn flow(tasks: Vec<FlowTask>) -> Flow {
        Flow {
            id: String::new(),
            project_id: "p1".into(),
            name: "Night run".into(),
            instruction: "Fix them".into(),
            start_at: Some(10 * MIN),
            minutes: 60,
            live: true,
            state: "saved".into(),
            current: None,
            live_before: None,
            tasks,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn a_report_ends_the_task_once_claudes_turn_is_over_too() {
        let t = task(0);
        // reported, still at work: the task waits for the turn's end (its Stop hook), so Live and the next task never
        // leave a Claude in the middle of its last steps
        assert_eq!(decide(&t, 60, MIN, Some("working"), Some(&report("review")), true, None), Step::Wait);
        assert_eq!(decide(&t, 60, MIN, Some("done"), Some(&report("review")), true, None), Step::End("review".into(), None));
        assert_eq!(decide(&t, 60, MIN, Some("needs_you"), Some(&report("done")), true, None), Step::End("done".into(), None), "waiting for you after its report");
        assert_eq!(decide(&t, 60, MIN, Some("working"), Some(&report("done")), false, None), Step::End("done".into(), None), "its report counts though it closed after");
        // still going after its report when its time runs out: the report says how it went, a note says the rest
        assert!(matches!(decide(&t, 60, 60 * MIN, Some("working"), Some(&report("blocked")), true, None), Step::End(s, Some(_)) if s == "blocked"));
    }

    #[test]
    fn an_api_error_cuts_the_task_off_and_the_usage_limit_or_login_stops_the_run() {
        let t = task(10 * MIN);
        let error = |kind: &str, at: i64| Failure { kind: kind.into(), text: "You've hit your limit · resets 3am".into(), at };
        let limit = error("rate_limit", 12 * MIN);
        assert_eq!(
            decide(&t, 60, 12 * MIN, Some("needs_you"), None, true, Some(&limit)),
            Step::CutOff { note: "You've hit your limit · resets 3am".into(), stop_run: true },
            "every later task would hit the limit too"
        );
        for kind in ["authentication_failed", "billing_error", "model_not_found"] {
            assert!(matches!(decide(&t, 60, 12 * MIN, Some("needs_you"), None, true, Some(&error(kind, 12 * MIN))), Step::CutOff { stop_run: true, .. }), "{kind}");
        }
        for kind in ["overloaded", "server_error", "invalid_request", "unknown"] {
            assert!(matches!(decide(&t, 60, 12 * MIN, Some("needs_you"), None, true, Some(&error(kind, 12 * MIN))), Step::CutOff { stop_run: false, .. }), "{kind}: the next task may well run");
        }
        // Claude Code said nothing about it: its kind says it
        let quiet = Failure { kind: "overloaded".into(), text: String::new(), at: 12 * MIN };
        assert!(matches!(decide(&t, 60, 12 * MIN, Some("needs_you"), None, true, Some(&quiet)), Step::CutOff { note, .. } if note.contains("overloaded")));
        // an error from before the task started belongs to another turn; one after its report doesn't undo the report
        assert_eq!(decide(&t, 60, 12 * MIN, Some("working"), None, true, Some(&error("rate_limit", 9 * MIN))), Step::Wait);
        assert_eq!(decide(&t, 60, 12 * MIN, Some("needs_you"), Some(&report("done")), true, Some(&limit)), Step::End("done".into(), None));
        assert_eq!(decide(&t, 60, 12 * MIN, Some("working"), Some(&report("done")), true, Some(&limit)), Step::End("done".into(), None));
    }

    #[test]
    fn a_stop_failure_is_kept_per_workspace_and_wakes_the_runner() {
        stop_failure("w-cut", "", "  Claude Code's words  ", 5);
        assert_eq!(failure_in("w-cut"), Some(Failure { kind: "unknown".into(), text: "Claude Code's words".into(), at: 5 }));
        assert_eq!(failure_in("w-other"), None);
        stop_failure("w-cut", "rate_limit", &"x".repeat(1000), 6);
        let f = failure_in("w-cut").unwrap();
        assert_eq!((f.kind.as_str(), f.text.chars().count(), f.at), ("rate_limit", 400, 6), "the last one, its words cut to a note's length");
        // woken: the runner's next nap ends at once
        let started = std::time::Instant::now();
        nap();
        assert!(started.elapsed() < Duration::from_millis(500));
    }

    #[test]
    fn a_claude_that_works_is_left_alone_until_its_time_runs_out() {
        let t = task(0);
        assert_eq!(decide(&t, 60, 59 * MIN, Some("working"), None, true, None), Step::Wait);
        assert!(matches!(decide(&t, 60, 60 * MIN, Some("working"), None, true, None), Step::End(s, Some(_)) if s == "timed_out"));
        assert!(matches!(decide(&t, 60, 5 * MIN, Some("working"), None, false, None), Step::End(s, Some(_)) if s == "failed"));
    }

    #[test]
    fn a_claude_that_stops_without_a_report_is_asked_once_then_the_task_closes() {
        // a new Claude, never seen working yet: it gets three minutes to start
        let t = task(0);
        assert_eq!(decide(&t, 60, 2 * MIN, Some("done"), None, true, None), Step::Wait);
        assert_eq!(decide(&t, 60, 3 * MIN, None, None, true, None), Step::Nudge);
        // it worked, then stopped: asked 45 s after it last worked
        let worked = FlowTask { busy_at: Some(10 * MIN), ..task(0) };
        assert_eq!(decide(&worked, 60, 10 * MIN + 30_000, Some("done"), None, true, None), Step::Wait);
        assert_eq!(decide(&worked, 60, 10 * MIN + 45_000, Some("done"), None, true, None), Step::Nudge);
        // asked: four quiet minutes, then the task closes without a report
        let asked = FlowTask { nudged_at: Some(11 * MIN), ..worked.clone() };
        assert_eq!(decide(&asked, 60, 14 * MIN, Some("done"), None, true, None), Step::Wait);
        assert!(matches!(decide(&asked, 60, 15 * MIN, Some("done"), None, true, None), Step::End(s, Some(_)) if s == "no_report"));
        // it worked again after being asked: the four minutes count from then
        let again = FlowTask { busy_at: Some(14 * MIN), ..asked };
        assert_eq!(decide(&again, 60, 17 * MIN, Some("done"), None, true, None), Step::Wait);
        assert!(matches!(decide(&again, 60, 18 * MIN, Some("done"), None, true, None), Step::End(s, _) if s == "no_report"));
    }

    #[test]
    fn a_question_waits_ten_minutes_for_you_then_the_next_task_starts() {
        let t = FlowTask { busy_at: Some(5 * MIN), ..task(0) };
        assert_eq!(decide(&t, 60, 14 * MIN, Some("needs_you"), None, true, None), Step::Wait);
        assert!(matches!(decide(&t, 60, 15 * MIN, Some("needs_you"), None, true, None), Step::End(s, Some(_)) if s == "asked"));
    }

    #[test]
    fn starts_on_time_but_never_hours_late() {
        let f = Flow { state: "scheduled".into(), ..flow(vec![task(0)]) };
        assert_eq!(due(&f, 9 * MIN), Due::Not);
        assert_eq!(due(&f, 10 * MIN), Due::Start);
        assert_eq!(due(&f, 40 * MIN), Due::Start, "half an hour late (the Mac woke up): still on");
        assert_eq!(due(&f, 41 * MIN), Due::Missed, "Enkidu wasn't running at its time: missed, not started now");
        assert_eq!(due(&Flow { state: "saved".into(), ..f.clone() }, 10 * MIN), Due::Not);
    }

    #[test]
    fn asks_for_the_report_naming_the_card() {
        let f = flow(vec![task(0)]);
        let text = nudge_text(&f, &f.tasks[0]);
        assert!(text.contains("enkidu report done|review|blocked --ticket SH-1"), "{text}");
        assert_eq!(cli_flag("pr:demo-org/shop#2414"), "--pr 2414");
        // a task of your own (item 8.7) is named by its number, and its report is found under its card
        let own = flow(vec![FlowTask { subject: "own:3".into(), ..task(0) }]);
        let text = nudge_text(&own, &own.tasks[0]);
        assert!(text.contains("how task 3 went") && text.contains("enkidu report done|review|blocked --task 3"), "{text}");
        assert_eq!((report_subject("own:3"), report_subject("ticket:SH-1")), ("own:3", "SH-1"));
        assert!(check(&own).is_ok(), "a task of your own can be a workflow's task");
        let stranger = flow(vec![FlowTask { subject: "session:s1".into(), ..task(0) }]);
        assert!(check(&stranger).unwrap_err().to_string().contains("not session:s1"));
    }

    #[test]
    fn knows_the_commands_that_make_or_send_commits() {
        for (cmd, what) in [
            ("git commit -am 'Fix the export'", "git commit"),
            ("git push -u origin feature/SH-1", "git push"),
            ("cd shop && git add -A && git commit -m x", "git commit"),
            ("git -C /w/sh-1 commit -m x", "git commit"),
            ("git -c user.name=x --no-pager commit", "git commit"),
            ("git pull --rebase", "git pull"),
            ("git merge development", "git merge"),
            ("git rebase -i HEAD~2", "git rebase"),
            ("git cherry-pick abc123", "git cherry-pick"),
            ("gh pr create --fill", "gh pr create"),
            ("make test; gh pr merge 12", "gh pr merge"),
        ] {
            assert_eq!(commits(cmd).as_deref(), Some(what), "{cmd}");
        }
        for cmd in ["git status", "git diff --stat", "git log -5", "git stash list", "git add -A", "docker exec orbstack_web python manage.py test", "echo commit", "grep -rn push src"] {
            assert_eq!(commits(cmd), None, "{cmd}");
        }
    }

    #[test]
    fn refuses_commits_only_in_a_workflow_task_that_runs_in_that_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("t.db")).unwrap();
        let mut f = save(&db, flow(vec![task(0)]), 1).unwrap();
        // outside a workflow (scheduled, not running yet): your Claude commits as it likes
        assert_eq!(tool_guard(&db, "w1", "git commit -m x"), None);
        f.state = "running".into();
        f.current = Some(0);
        f.tasks[0] = FlowTask { state: "running".into(), workspace_id: Some("w1".into()), started_at: Some(10), ..f.tasks[0].clone() };
        db.work_put_flow(&f).unwrap();
        let no = tool_guard(&db, "w1", "git add -A && git commit -m 'Fix'").unwrap();
        assert!(no.contains("`git commit` isn't run") && no.contains("Night run"), "{no}");
        assert!(tool_guard(&db, "w1", "git push").is_some());
        assert_eq!(tool_guard(&db, "w1", "git diff"), None, "reading is fine");
        assert_eq!(tool_guard(&db, "w2", "git commit -m x"), None, "another workspace's Claude isn't the task's");
        // the task over: as before
        f.tasks[0].state = "done".into();
        db.work_put_flow(&f).unwrap();
        assert_eq!(tool_guard(&db, "w1", "git commit -m x"), None);
    }

    #[test]
    fn the_stop_hook_asks_a_workflow_task_for_its_report_once_and_nothing_else_ever() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("t.db")).unwrap();
        let mut f = save(&db, flow(vec![task(0)]), 1).unwrap();
        // not running yet: a stop is a stop
        assert_eq!(stop_hook(&db, "w1", false, 5), None);
        f.state = "running".into();
        f.current = Some(0);
        f.tasks[0] = FlowTask { state: "running".into(), workspace_id: Some("w1".into()), started_at: Some(10), ..f.tasks[0].clone() };
        db.work_put_flow(&f).unwrap();
        // another workspace's Claude, or one already going on because of a stop hook: never
        assert_eq!(stop_hook(&db, "w2", false, 20), None);
        assert_eq!(stop_hook(&db, "w1", true, 20), None);
        // its Claude stops without a report: asked, once
        let ask = stop_hook(&db, "w1", false, 20).unwrap();
        assert!(ask.contains("enkidu report done|review|blocked --ticket SH-1"), "{ask}");
        assert_eq!(db.work_flow(&f.id).unwrap().unwrap().tasks[0].nudged_at, Some(20));
        assert_eq!(stop_hook(&db, "w1", false, 30), None, "asked already: the runner closes it if it stays quiet");
        // a report made after the task started: nothing to ask
        db.work_put_flow(&f).unwrap();
        db.work_put_report("SH-1", Some("s1"), Some("w1"), "done", "Fixed", 15).unwrap();
        assert_eq!(stop_hook(&db, "w1", false, 40), None);
    }

    #[test]
    fn keeps_every_run_as_it_ended_so_running_again_loses_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("t.db")).unwrap();
        let mut f = save(&db, flow(vec![task(0), FlowTask { subject: "ticket:SH-2".into(), ..task(0) }]), 1).unwrap();
        // the first night: both done
        f.tasks[0] = FlowTask { state: "done".into(), started_at: Some(100), ended_at: Some(200), ..f.tasks[0].clone() };
        f.tasks[1] = FlowTask { state: "review".into(), started_at: Some(200), ended_at: Some(300), ..f.tasks[1].clone() };
        f.state = "done".into();
        db.work_put_flow_run(&f, 300).unwrap();
        // run again: it starts afresh; the second night is stopped after its first task
        let again = run_now(&db, &f.id, 1_000).unwrap();
        assert!(again.tasks.iter().all(|t| t.state == "waiting"), "a new run starts afresh");
        let mut second = again.clone();
        second.tasks[0] = FlowTask { state: "timed_out".into(), started_at: Some(1_000), ended_at: Some(4_600), ..second.tasks[0].clone() };
        second.state = "stopped".into();
        db.work_put_flow_run(&second, 4_700).unwrap();
        let runs = db.work_flow_runs(10).unwrap();
        assert_eq!(runs.iter().map(|r| (r.state.as_str(), r.started_at, r.ended_at)).collect::<Vec<_>>(), [("stopped", Some(1_000), 4_700), ("done", Some(100), 300)]);
        assert_eq!(runs[1].flow.tasks.iter().map(|t| t.state.as_str()).collect::<Vec<_>>(), ["done", "review"], "the first night, as it ended");
        assert_eq!(runs[1].flow_id, f.id);
    }

    #[test]
    fn a_workflow_that_ended_before_runs_were_kept_becomes_its_first_run_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        {
            let db = Db::open(&path).unwrap();
            let mut f = save(&db, flow(vec![task(0)]), 1).unwrap();
            f.tasks[0] = FlowTask { state: "done".into(), started_at: Some(100), ended_at: Some(900), ..f.tasks[0].clone() };
            f.state = "done".into();
            f.updated_at = 950;
            db.work_put_flow(&f).unwrap();
            // the version before kept no runs: what that database looked like
            db.conn().execute("DELETE FROM work_flow_runs", []).unwrap();
        }
        let db = Db::open(&path).unwrap();
        let runs = db.work_flow_runs(10).unwrap();
        assert_eq!(runs.iter().map(|r| (r.state.as_str(), r.started_at, r.ended_at)).collect::<Vec<_>>(), [("done", Some(100), 950)]);
        drop(db);
        // opened again: still the one run
        assert_eq!(Db::open(&path).unwrap().work_flow_runs(10).unwrap().len(), 1);
    }

    #[test]
    fn each_task_keeps_its_kind_words_minutes_and_where_it_runs() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("t.db")).unwrap();
        let back = FlowTask {
            subject: "ticket:SH-2".into(),
            kind: "came_back".into(),
            instruction: "QA found the export empty on Mondays: fix that only.".into(),
            minutes: Some(30),
            target: Target::Conversation { workspace_id: "w2".into() },
            ..task(0)
        };
        let saved = save(&db, flow(vec![task(0), back.clone()]), 1).unwrap();
        let read = db.work_flow(&saved.id).unwrap().unwrap();
        let t = &read.tasks[1];
        assert_eq!((t.kind.as_str(), t.instruction.as_str(), t.minutes), ("came_back", back.instruction.as_str(), Some(30)));
        assert_eq!(serde_json::to_value(&t.target).unwrap(), serde_json::json!({ "kind": "conversation", "workspaceId": "w2" }));
        // a task without its own minutes has the workflow's; with them, its own end its time
        let mut started = FlowTask { started_at: Some(0), busy_at: Some(1), ..t.clone() };
        started.state = "running".into();
        assert!(matches!(decide(&started, started.minutes.unwrap_or(60), 30 * MIN, Some("working"), None, true, None), Step::End(s, _) if s == "timed_out"));
        assert_eq!(decide(&started, read.tasks[0].minutes.unwrap_or(60), 30 * MIN, Some("working"), None, true, None), Step::Wait);
        // what can't be is refused: no such kind, minutes out of range
        assert!(save(&db, flow(vec![FlowTask { kind: "whatever".into(), ..task(0) }]), 1).unwrap_err().to_string().contains("no kind of task"));
        assert!(save(&db, flow(vec![FlowTask { minutes: Some(1), ..task(0) }]), 1).unwrap_err().to_string().contains("5 minutes"));
        // your own default words per kind are kept; a kind that doesn't exist isn't
        let mine = std::collections::BTreeMap::from([("came_back".to_string(), "Read QA's comment first.".to_string())]);
        db.work_set_flow_kinds(&mine).unwrap();
        assert_eq!(db.work_flow_kinds().unwrap(), mine);
        // what a new workflow starts from (item 8.6): the usual until you set it, and only what makes sense is kept
        assert_eq!(db.work_flow_defaults().unwrap(), FlowDefaults { minutes: 60, time: "02:00".into() });
        let yours = FlowDefaults { minutes: 45, time: "23:30".into() };
        db.work_set_flow_defaults(&yours).unwrap();
        assert_eq!(db.work_flow_defaults().unwrap(), yours);
        assert!(db.work_set_flow_defaults(&FlowDefaults { minutes: 2, time: "23:30".into() }).is_err());
        assert!(db.work_set_flow_defaults(&FlowDefaults { minutes: 45, time: "25:00".into() }).is_err());
        assert!(db.work_set_flow_defaults(&FlowDefaults { minutes: 45, time: "soon".into() }).is_err());
        assert_eq!(db.work_flow_defaults().unwrap(), yours, "a refused one changes nothing");
        assert_eq!((clock("02:00"), clock("7:30"), clock("24:00"), clock("7:5"), clock("")), (Some((2, 0)), Some((7, 30)), None, None, None));
        assert!(db.work_set_flow_kinds(&std::collections::BTreeMap::from([("nap".to_string(), "x".to_string())])).is_err());
    }

    #[test]
    fn keeps_a_workflow_and_its_tasks_and_refuses_one_that_cant_run() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("t.db")).unwrap();
        let new = FlowTask {
            subject: "ticket:SH-2".into(),
            target: Target::New { name: "sh-2-orders".into(), branch: "feature/SH-2-orders".into(), mode: "new".into() },
            ..task(0)
        };
        let saved = save(&db, flow(vec![task(0), new]), 5).unwrap();
        assert_eq!(saved.state, "scheduled");
        assert!(saved.tasks.iter().all(|t| t.state == "waiting" && t.started_at.is_none()), "a saved workflow's tasks start over");
        let back = db.work_flow(&saved.id).unwrap().unwrap();
        assert_eq!(back, saved);
        assert_eq!(serde_json::to_value(&back.tasks[1].target).unwrap(), serde_json::json!({ "kind": "new", "name": "sh-2-orders", "branch": "feature/SH-2-orders", "mode": "new" }));
        assert_eq!(serde_json::to_value(&back.tasks[0].target).unwrap(), serde_json::json!({ "kind": "workspace", "workspaceId": "w1" }));
        // kept without a time: not scheduled
        let kept = save(&db, Flow { start_at: None, ..back.clone() }, 6).unwrap();
        assert_eq!((kept.state.as_str(), kept.created_at), ("saved", 5));
        // run now: scheduled for now
        let now = run_now(&db, &kept.id, 7).unwrap();
        assert_eq!((now.state.as_str(), now.start_at), ("scheduled", Some(7)));
        // one that runs can't be changed
        db.work_put_flow(&Flow { state: "running".into(), ..now.clone() }).unwrap();
        assert!(save(&db, now.clone(), 8).unwrap_err().to_string().contains("stop it"));
        // what can't run is refused
        assert!(save(&db, flow(vec![]), 1).unwrap_err().to_string().contains("at least one task"));
        assert!(save(&db, Flow { minutes: 2, ..flow(vec![task(0)]) }, 1).is_err());
        assert!(save(&db, flow(vec![FlowTask { message: " ".into(), ..task(0) }]), 1).unwrap_err().to_string().contains("empty"));
        db.work_delete_flow(&saved.id).unwrap();
        assert!(db.work_flows().unwrap().is_empty());
    }
}
