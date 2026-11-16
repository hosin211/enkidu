//! The Workflows page, as it decides (DESIGN.md, "Workflows: each task its own, and a page of their runs"): three
//! panes, as a mail app has them. First your workflows still to run and every run kept, the newest first; then the one
//! picked, with how it went and all its tasks in a short list; then the task picked, across the rest of the page: what
//! it is, how it stands (the one colored box), what it was given, and what happened, in time order.

use enkidu_core::db::Workspace;
use enkidu_core::work::flows::{Flow, FlowRun, FlowTask, Target};
use enkidu_core::work::model::Snapshot;
use enkidu_core::work::reports::Report;

use super::flows::{TaskTone, flow_words, kind_label, length_words, run_tally, task_look, task_name};
use super::report::ReportRow;
use super::when::{clock_of, day_words, when_words};

/// What the header's "Workflows" says when pointed at: while one runs, and otherwise.
pub const HEADER_RUNNING: &str = "A workflow is running: see how its tasks stand";
pub const HEADER_WORDS: &str = "Your workflows: what is due, and how each task ended";
/// What Stop does, said when pointed at: to a workflow that runs, and to one that is due.
pub const STOP_RUNNING: &str = "Stop after the task that runs (its Claude isn't stopped)";
pub const STOP_DUE: &str = "Keep it, without a time";
pub const DELETE_SAYS: &str = "Delete the workflow (its runs and Claude's reports stay)";
/// What a task says that never got its turn in a run that stopped.
pub const NEVER_RAN: &str = "The run stopped before its turn";
/// What the timeline says when Claude was asked for the report it hadn't made.
pub const ASKED: &str = "Claude stopped without saying how it went: Enkidu asked for its report";

/// The states that need you, in a run: the task shown first when you open it.
pub const NEEDS_YOU: [&str; 7] = ["blocked", "asked", "cut_off", "failed", "review", "no_report", "timed_out"];

/// The header's "Workflows": there once you made a workflow or a run of one is kept. Says whether one runs.
pub fn header_chip(work: &Snapshot) -> Option<bool> {
    (!work.flows.is_empty() || !work.flow_runs.is_empty())
        .then(|| work.flows.iter().any(|flow| flow.state == "running"))
}

/// A task's state in a word, in a run's list of tasks ("Out of time").
pub fn task_short(state: &str) -> &'static str {
    match state {
        "running" => "Running",
        "done" => "Done",
        "review" => "Review",
        "blocked" => "Stuck",
        "no_report" => "No report",
        "timed_out" => "Out of time",
        "asked" => "Asked you",
        "cut_off" => "Cut off",
        "failed" => "Didn't run",
        "stopped" => "Stopped",
        _ => "Waiting",
    }
}

/// The workflows still to run: the one that runs first, then the ones due (the soonest first), then the ones kept
/// without a time (the one changed last first).
pub fn coming(flows: &[Flow]) -> Vec<&Flow> {
    let order = |flow: &Flow| match flow.state.as_str() {
        "running" => Some(0),
        "scheduled" => Some(1),
        "saved" => Some(2),
        _ => None,
    };
    let mut coming: Vec<&Flow> = flows.iter().filter(|flow| order(flow).is_some()).collect();
    coming.sort_by(|a, b| {
        order(a).cmp(&order(b)).then_with(|| match a.state.as_str() {
            "scheduled" => a.start_at.unwrap_or(0).cmp(&b.start_at.unwrap_or(0)),
            _ => b.updated_at.cmp(&a.updated_at),
        })
    });
    coming
}

/// What is shown: the one you picked while it is still there, else the one that runs, else the newest run, else the
/// next one due. `flow:ID` or `run:ID`.
pub fn current(picked: Option<&str>, coming: &[&Flow], runs: &[FlowRun]) -> Option<String> {
    let has = |key: &str| match key.strip_prefix("flow:") {
        Some(id) => coming.iter().any(|flow| flow.id == id),
        None => runs.iter().any(|run| format!("run:{}", run.id) == key),
    };
    if let Some(picked) = picked.filter(|key| has(key)) {
        return Some(picked.to_string());
    }
    match (coming.first(), runs.first()) {
        (Some(flow), _) if flow.state == "running" => Some(format!("flow:{}", flow.id)),
        (_, Some(run)) => Some(format!("run:{}", run.id)),
        (Some(flow), None) => Some(format!("flow:{}", flow.id)),
        (None, None) => None,
    }
}

/// The task a run opens on: the one that runs, else the first that needs you, else the first.
pub fn first_task(flow: &Flow) -> Option<&FlowTask> {
    let running = flow.tasks.iter().find(|task| task.state == "running");
    running.or_else(|| flow.tasks.iter().find(|task| NEEDS_YOU.contains(&task.state.as_str()))).or(flow.tasks.first())
}

/// A task's updates and its report: Claude's reports on its card within the time it ran (a minute after too: a
/// report may land as it ends), in the index's order, the newest first.
pub fn task_reports<'a>(reports: &'a [Report], task: &FlowTask, now: i64) -> Vec<&'a Report> {
    let Some(started) = task.started_at else { return Vec::new() };
    let subject = match task.subject.strip_prefix("ticket:") {
        Some(key) => key,
        None => task.subject.strip_prefix("pr:").unwrap_or(&task.subject),
    };
    let until = task.ended_at.unwrap_or(now) + 60_000;
    (reports.iter())
        .filter(|report| report.subject.eq_ignore_ascii_case(subject) && report.at >= started && report.at <= until)
        .collect()
}

/// How a run ended, as a lamp: anything stuck or failed red, anything for you amber, all done green; gray when it
/// was stopped or didn't run.
pub fn run_tone(run: &FlowRun) -> TaskTone {
    let any = |states: &[&str]| run.flow.tasks.iter().any(|task| states.contains(&task.state.as_str()));
    if run.state == "missed" {
        TaskTone::Quiet
    } else if any(&["blocked", "cut_off", "failed", "asked"]) {
        TaskTone::Red
    } else if any(&["review", "no_report", "timed_out"]) {
        TaskTone::Amber
    } else if run.state == "stopped" {
        TaskTone::Quiet
    } else {
        TaskTone::Green
    }
}

/// How a run went, in a line: "3 done, 1 wants your review", or why there is nothing to count.
pub fn run_line(run: &FlowRun) -> String {
    if run.state == "missed" {
        return format!("Didn't run: Enkidu wasn't open at {}", clock_of(run.flow.start_at.unwrap_or(run.ended_at)));
    }
    let tally = run_tally(&run.flow);
    match (tally.is_empty(), run.state.as_str()) {
        (false, _) => tally,
        (true, "stopped") => "Stopped before a task ended".to_string(),
        (true, _) => "Finished".to_string(),
    }
}

/// A workflow still to run, in the first pane.
#[derive(Clone, Debug, PartialEq)]
pub struct ComingItem {
    pub id: String,
    /// `flow:ID`.
    pub key: String,
    /// running, scheduled or saved.
    pub state: String,
    pub name: String,
    /// How it stands: "Starts tomorrow at 02:00, 2 tasks".
    pub words: String,
    /// It is the one shown.
    pub current: bool,
}

/// A run that ended, in the first pane.
#[derive(Clone, Debug, PartialEq)]
pub struct RunItem {
    pub id: i64,
    /// `run:ID`.
    pub key: String,
    pub flow_id: String,
    /// done, stopped or missed.
    pub state: String,
    pub tone: TaskTone,
    pub name: String,
    /// "Yesterday, 02:00".
    pub day: String,
    /// "1 wants your review, 1 stuck".
    pub line: String,
    pub current: bool,
}

/// A segment per task, in its state's color: a run at a glance.
#[derive(Clone, Debug, PartialEq)]
pub struct Bar {
    /// Each task's color; none for one that hasn't run (a faint segment).
    pub segments: Vec<Option<TaskTone>>,
    /// "1 of 3 tasks over".
    pub label: String,
}

impl Bar {
    fn of(flow: &Flow) -> Bar {
        let color = |state: &str| match state {
            "done" => Some(TaskTone::Green),
            "review" | "no_report" | "timed_out" => Some(TaskTone::Amber),
            "blocked" | "cut_off" | "failed" | "asked" => Some(TaskTone::Red),
            "running" => Some(TaskTone::Accent),
            _ => None,
        };
        let over = flow.tasks.iter().filter(|task| !matches!(task.state.as_str(), "waiting" | "running")).count();
        Bar {
            segments: flow.tasks.iter().map(|task| color(&task.state)).collect(),
            label: format!("{over} of {} tasks over", flow.tasks.len()),
        }
    }
}

/// What can be done with the workflow or run shown: each names the workflow it acts on.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Buttons {
    /// "Run it again": the workflow of the run shown, while it doesn't run.
    pub run_again: Option<String>,
    /// "Run now": the workflow shown, while it doesn't run.
    pub run_now: Option<String>,
    /// "Change": that workflow, in its dialog.
    pub change: Option<String>,
    /// "Stop", with what it does to this one.
    pub stop: Option<(String, &'static str)>,
    /// Delete.
    pub delete: Option<String>,
}

/// A task in a run's list.
#[derive(Clone, Debug, PartialEq)]
pub struct TaskLine {
    pub subject: String,
    /// "SH-21309".
    pub name: String,
    pub state: String,
    /// "Out of time".
    pub short: &'static str,
    pub tone: TaskTone,
    /// How long it ran (or runs), "20 min"; empty before it started.
    pub took: String,
    pub title: String,
    /// "Fix what came back · 1 note".
    pub kind: String,
}

impl TaskLine {
    /// The line as a test reads it: "2 SH-21309 Running 20 min".
    pub fn said(&self, n: usize) -> String {
        format!("{n} {} {} {}", self.name, self.short, self.took).trim_end().to_string()
    }
}

/// The second pane: the workflow or run shown.
#[derive(Clone, Debug, PartialEq)]
pub struct Detail {
    /// `flow:ID` or `run:ID`.
    pub key: String,
    pub name: String,
    /// "Yesterday, 02:00 · took 1 h 30 min", "Started 02:00", "Starts tomorrow at 02:00", "Kept without a time".
    pub when: String,
    /// "Finished: 1 done, 1 stuck", "Running 2 of 3: SH-21309".
    pub words: String,
    /// A run's lamp.
    pub lamp: Option<TaskTone>,
    pub bar: Option<Bar>,
    /// "For every task: Use the small stack."
    pub every: Option<String>,
    pub buttons: Buttons,
    /// "3 tasks".
    pub count: String,
    pub lines: Vec<TaskLine>,
    /// The workflow as it is shown: a workflow still to run with every task waiting (none has run in this run); a
    /// run that stopped with the tasks that never got their turn saying so.
    pub shown: Flow,
    /// The task it opens on.
    pub first: Option<String>,
}

/// What the page's first two panes say.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FlowsModel {
    pub coming: Vec<ComingItem>,
    pub runs: Vec<RunItem>,
    /// What is shown: `flow:ID` or `run:ID`.
    pub current: Option<String>,
    pub detail: Option<Detail>,
}

/// When a workflow that runs started: its first task's start.
fn first_start(flow: &Flow) -> Option<i64> {
    flow.tasks.iter().filter_map(|task| task.started_at).min()
}

/// The two ends of the chain of a run that is over (`run:ID`), as the page draws them since 15 Nov (design 2):
/// what happened. "02:00, started", and "05:30, ended" ("stopped" for one that was), with where Live went
/// back to while that workspace is still yours. A workflow that runs or waits for its time has none (its head says
/// when it started or starts), and neither has a run that never started.
pub fn chain_ends(key: &str, work: &Snapshot, workspaces: &[Workspace]) -> Option<(String, String)> {
    let run = work.flow_runs.iter().find(|run| format!("run:{}", run.id) == key)?;
    let started = run.started_at?;
    let how = if run.state == "stopped" { "stopped" } else { "ended" };
    let back = (run.flow.live_before.as_ref().filter(|_| run.flow.live))
        .and_then(|id| workspaces.iter().find(|workspace| &workspace.id == id))
        .map(|workspace| format!(". Live went back to {}.", workspace.name))
        .unwrap_or_default();
    Some((format!("{}, started", clock_of(started)), format!("{}, {how}{back}", clock_of(run.ended_at))))
}

impl FlowsModel {
    /// The page for the index as it is, with what you picked in the first pane (`flow:ID` or `run:ID`).
    pub fn of(work: &Snapshot, picked: Option<&str>, now: i64) -> FlowsModel {
        let still = coming(&work.flows);
        let current = current(picked, &still, &work.flow_runs);
        let is = |key: &str| current.as_deref() == Some(key);
        let coming_items = (still.iter())
            .map(|flow| {
                let key = format!("flow:{}", flow.id);
                ComingItem {
                    id: flow.id.clone(),
                    current: is(&key),
                    key,
                    state: flow.state.clone(),
                    name: flow.name.clone(),
                    words: flow_words(flow, now),
                }
            })
            .collect();
        let runs = (work.flow_runs.iter())
            .map(|run| {
                let key = format!("run:{}", run.id);
                RunItem {
                    id: run.id,
                    current: is(&key),
                    key,
                    flow_id: run.flow_id.clone(),
                    state: run.state.clone(),
                    tone: run_tone(run),
                    name: run.flow.name.clone(),
                    day: day_words(run.started_at.unwrap_or(run.ended_at), now),
                    line: run_line(run),
                }
            })
            .collect();
        let detail = current.as_deref().and_then(|key| match key.strip_prefix("flow:") {
            Some(id) => still.iter().find(|flow| flow.id == id).map(|flow| Detail::of_flow(flow, work, now)),
            None => (work.flow_runs.iter())
                .find(|run| format!("run:{}", run.id) == key)
                .map(|run| Detail::of_run(run, work, now)),
        });
        FlowsModel { coming: coming_items, runs, current, detail }
    }
}

impl Detail {
    /// A workflow still to run.
    fn of_flow(flow: &Flow, work: &Snapshot, now: i64) -> Detail {
        let running = flow.state == "running";
        // one that doesn't run shows what each task will do: none has run yet in this run
        let shown = match running {
            true => flow.clone(),
            false => Flow {
                tasks: (flow.tasks.iter().cloned())
                    .map(|task| FlowTask {
                        state: "waiting".into(),
                        started_at: None,
                        ended_at: None,
                        note: None,
                        nudged_at: None,
                        ..task
                    })
                    .collect(),
                ..flow.clone()
            },
        };
        let when = match (running, flow.start_at) {
            (true, at) => format!("Started {}", clock_of(first_start(flow).or(at).unwrap_or(now))),
            (false, Some(at)) => format!("Starts {}", when_words(at, now)),
            (false, None) => "Kept without a time".to_string(),
        };
        let id = flow.id.clone();
        let buttons = Buttons {
            run_again: None,
            run_now: (!running).then(|| id.clone()),
            change: (!running).then(|| id.clone()),
            stop: match flow.state.as_str() {
                "running" => Some((id.clone(), STOP_RUNNING)),
                "scheduled" => Some((id.clone(), STOP_DUE)),
                _ => None,
            },
            delete: Some(id),
        };
        Detail::with(
            format!("flow:{}", flow.id),
            when,
            flow_words(flow, now),
            None,
            running.then(|| Bar::of(flow)),
            buttons,
            shown,
            work,
            now,
        )
    }

    /// A run that ended.
    fn of_run(run: &FlowRun, work: &Snapshot, now: i64) -> Detail {
        // in a run that stopped (you, or the usage limit), a task that never got its turn says so
        let shown = match run.state.as_str() {
            "stopped" => Flow {
                tasks: (run.flow.tasks.iter().cloned())
                    .map(|task| match task.state.as_str() {
                        "waiting" => FlowTask {
                            state: "stopped".into(),
                            note: task.note.clone().or_else(|| Some(NEVER_RAN.to_string())),
                            ..task
                        },
                        _ => task,
                    })
                    .collect(),
                ..run.flow.clone()
            },
            _ => run.flow.clone(),
        };
        let day = day_words(run.started_at.unwrap_or(run.ended_at), now);
        let when = match run.started_at {
            Some(started) => format!("{day} · took {}", length_words(run.ended_at - started)),
            None => day,
        };
        let tally = run_tally(&run.flow);
        let words = match run.state.as_str() {
            "missed" => run_line(run),
            state => {
                let how = if state == "stopped" { "Stopped" } else { "Finished" };
                if tally.is_empty() { how.to_string() } else { format!("{how}: {tally}") }
            }
        };
        // its workflow, while you still have it and it doesn't run: it can run again, or be changed
        let its = work.flows.iter().find(|flow| flow.id == run.flow_id).filter(|flow| flow.state != "running");
        let buttons = Buttons {
            run_again: its.map(|flow| flow.id.clone()),
            change: its.map(|flow| flow.id.clone()),
            ..Buttons::default()
        };
        let bar = (run.state != "missed").then(|| Bar::of(&run.flow));
        Detail::with(format!("run:{}", run.id), when, words, Some(run_tone(run)), bar, buttons, shown, work, now)
    }

    #[allow(clippy::too_many_arguments)]
    fn with(
        key: String,
        when: String,
        words: String,
        lamp: Option<TaskTone>,
        bar: Option<Bar>,
        buttons: Buttons,
        shown: Flow,
        work: &Snapshot,
        now: i64,
    ) -> Detail {
        let count = shown.tasks.len();
        let lines = (shown.tasks.iter())
            .map(|task| {
                let notes = task_reports(&work.reports, task, now).len();
                let kind = match notes {
                    0 => kind_label(&task.kind).to_string(),
                    1 => format!("{} · 1 note", kind_label(&task.kind)),
                    notes => format!("{} · {notes} notes", kind_label(&task.kind)),
                };
                TaskLine {
                    subject: task.subject.clone(),
                    name: task_name(&task.subject),
                    state: task.state.clone(),
                    short: task_short(&task.state),
                    tone: task_look(&task.state).1,
                    took: task.started_at.map(|at| length_words(task.ended_at.unwrap_or(now) - at)).unwrap_or_default(),
                    title: task.title.clone(),
                    kind,
                }
            })
            .collect();
        let every = Some(shown.instruction.trim())
            .filter(|every| !every.is_empty())
            .map(|every| format!("For every task: {every}"));
        Detail {
            key,
            name: shown.name.clone(),
            when,
            words,
            lamp,
            bar,
            every,
            buttons,
            count: if count == 1 { "1 task".to_string() } else { format!("{count} tasks") },
            lines,
            first: first_task(&shown).map(|task| task.subject.clone()),
            shown,
        }
    }

    /// The task shown in the third pane: the one you picked in this workflow or run, else the one it opens on.
    pub fn task(&self, picked: Option<&str>) -> Option<&FlowTask> {
        let picked = picked.and_then(|subject| self.shown.tasks.iter().find(|task| task.subject == subject));
        picked.or_else(|| first_task(&self.shown))
    }

    /// The task after the one shown (`by` 1) or before it (-1), stopping at the ends: what ↓ and ↑ pick.
    pub fn step(&self, picked: Option<&str>, by: isize) -> Option<&str> {
        let now = self.task(picked)?;
        let at = self.shown.tasks.iter().position(|task| task.subject == now.subject)? as isize;
        let next = (at + by).clamp(0, self.shown.tasks.len() as isize - 1) as usize;
        self.shown.tasks.get(next).map(|task| task.subject.as_str())
    }
}

/// A step of what happened to a task.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Its message went to its Claude.
    Sent(String),
    /// Enkidu asked for the report Claude hadn't made.
    Asked(&'static str),
    /// How it ended, when no report came.
    Ended(String),
    /// An update along the way, or its report (the report's place in the index's reports comes with it).
    Report(ReportRow),
}

impl Step {
    /// The step's kind: sent, asked, ended, or the report's state (update, done, review, blocked).
    pub fn kind(&self) -> &str {
        match self {
            Step::Sent(_) => "sent",
            Step::Asked(_) => "asked",
            Step::Ended(_) => "ended",
            Step::Report(report) => &report.state,
        }
    }

    /// Its dot's color: a report's as its state, the asking amber, the rest quiet.
    pub fn tone(&self) -> TaskTone {
        match self {
            Step::Asked(_) => TaskTone::Amber,
            Step::Report(report) => match report.state.as_str() {
                "update" => TaskTone::Accent,
                "done" => TaskTone::Green,
                "review" => TaskTone::Amber,
                _ => TaskTone::Red,
            },
            Step::Sent(_) | Step::Ended(_) => TaskTone::Quiet,
        }
    }
}

/// The third pane: one task of the workflow or run shown.
#[derive(Clone, Debug, PartialEq)]
pub struct TaskDetail {
    pub subject: String,
    /// "SH-21309".
    pub name: String,
    /// "Task 2 of 5".
    pub place: String,
    pub state: String,
    pub tone: TaskTone,
    pub title: String,
    /// "Fix what came back · sh-21309-export · 02:00–02:52 · 52 min of 60 min · Live".
    pub facts: String,
    /// The one colored box: how it stands ("Done, nothing needs you", "Running for 20 min of 45 min").
    pub outcome: String,
    /// What matters most under it: where and when it runs, or the note on what went wrong.
    pub outcome_line: Option<String>,
    /// Claude's own first words of its report, without Markdown's marks.
    pub said: Option<String>,
    /// "The message we sent", "The message it will get", "The message it would have got".
    pub message_heading: &'static str,
    /// When it was sent, "02:00".
    pub message_at: Option<String>,
    pub message: String,
    /// The workspace it ran in (or runs in), while it is one of yours: its id and name.
    pub workspace: Option<(String, String)>,
    /// What happened, in time order: each step's clock time and what it was.
    pub steps: Vec<(String, Step)>,
}

impl TaskDetail {
    /// `task` is one of `shown`'s (see [`Detail::task`]).
    pub fn of(shown: &Flow, task: &FlowTask, work: &Snapshot, workspaces: &[Workspace], now: i64) -> TaskDetail {
        let n = shown.tasks.iter().position(|other| other.subject == task.subject).map_or(1, |at| at + 1);
        let count = shown.tasks.len();
        let (label, tone) = task_look(&task.state);
        let ran_in = task.workspace_id.as_deref().or(match &task.target {
            Target::New { .. } => None,
            Target::Workspace { workspace_id } | Target::Conversation { workspace_id } => Some(workspace_id.as_str()),
        });
        let workspace = ran_in.and_then(|id| workspaces.iter().find(|workspace| workspace.id == id));
        let its = workspace.map_or("its workspace", |workspace| workspace.name.as_str());
        let mine = task_reports(&work.reports, task, now);
        let ended = mine.iter().find(|report| report.state != "update");
        let minutes = task.minutes.unwrap_or(shown.minutes);
        let used = task.started_at.map_or(0, |at| task.ended_at.unwrap_or(now) - at);
        let started = task.started_at;
        let (outcome, outcome_line) = match (task.state.as_str(), started) {
            ("waiting", _) => {
                let place = match (&task.target, started) {
                    (Target::New { .. }, None) => "a new workspace".to_string(),
                    (Target::Conversation { .. }, None) => format!("a new conversation in {its}"),
                    _ => format!("the Claude in {its}"),
                };
                let after = (n >= 2)
                    .then(|| shown.tasks.get(n - 2))
                    .flatten()
                    .map(|before| format!(", after {}", task_name(&before.subject)))
                    .unwrap_or_default();
                (
                    "Waiting its turn".to_string(),
                    Some(format!("Task {n} of {count}{after}: {place}, up to {minutes} minutes.")),
                )
            }
            ("running", Some(since)) => (
                format!("Running for {} of {minutes} min", length_words(used)),
                Some(format!("In {its} since {}{}.", clock_of(since), if shown.live { ", Live" } else { "" })),
            ),
            _ => (label.to_string(), task.note.clone()),
        };
        // Claude's own first words: the first line of how it ended, without a heading's marks or bold's
        let said = ended.and_then(|report| {
            let first = report.text.trim().lines().next().unwrap_or_default();
            let plain = first.trim_start_matches('#').trim_start().replace("**", "");
            (!plain.is_empty()).then_some(plain)
        });
        let mut facts: Vec<String> = vec![kind_label(&task.kind).to_string()];
        match (workspace, &task.target) {
            (Some(workspace), _) => facts.push(workspace.name.clone()),
            (None, Target::New { .. }) => facts.push("a new workspace".to_string()),
            (None, _) => {}
        }
        match started {
            Some(at) => {
                let until = task.ended_at.map(|ended| format!("–{}", clock_of(ended))).unwrap_or_default();
                facts.push(format!("{}{until}", clock_of(at)));
                facts.push(format!("{} of {minutes} min", length_words(used)));
                if shown.live {
                    facts.push("Live".to_string());
                }
            }
            None => facts.push(format!("up to {minutes} min")),
        }
        // what happened, in time order
        let mut steps: Vec<(i64, Step)> = Vec::new();
        if let Some(at) = started {
            let live = if shown.live { "; Live followed it" } else { "" };
            steps.push((at, Step::Sent(format!("Sent the message to the Claude in {its}{live}"))));
        }
        for report in &mine {
            steps.push((report.at, Step::Report(ReportRow::of(report, workspaces, now))));
        }
        if let Some(at) = task.nudged_at {
            steps.push((at, Step::Asked(ASKED)));
        }
        if let (Some(at), None) = (task.ended_at, ended) {
            let note = task.note.as_ref().map(|note| format!(": {note}")).unwrap_or_default();
            steps.push((at, Step::Ended(format!("{label}{note}"))));
        }
        steps.sort_by_key(|(at, _)| *at);
        TaskDetail {
            subject: task.subject.clone(),
            name: task_name(&task.subject),
            place: format!("Task {n} of {count}"),
            state: task.state.clone(),
            tone,
            title: task.title.clone(),
            facts: facts.join(" · "),
            outcome,
            outcome_line,
            said,
            message_heading: match (started.is_some(), task.state.as_str()) {
                (true, _) => "The message we sent",
                (false, "stopped") => "The message it would have got",
                (false, _) => "The message it will get",
            },
            message_at: started.map(clock_of),
            message: task.message.clone(),
            workspace: workspace.map(|workspace| (workspace.id.clone(), workspace.name.clone())),
            steps: steps.into_iter().map(|(at, step)| (clock_of(at), step)).collect(),
        }
    }

    /// The box in one line, as a test reads it: its title, the line under it, and Claude's words in quotes.
    pub fn outcome_said(&self) -> String {
        let mut said = self.outcome.clone();
        if let Some(line) = &self.outcome_line {
            said.push_str(line);
        }
        if let Some(words) = &self.said {
            said.push_str(&format!("“{words}”"));
        }
        said
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::work::groups::tests::ws;
    use crate::work::when::tests::now;
    use enkidu_core::work::flows::Target;

    fn task(key: &str, state: &str) -> FlowTask {
        FlowTask {
            subject: format!("ticket:{key}"),
            title: key.into(),
            target: Target::Workspace { workspace_id: "w2".into() },
            message: format!("{key}: the message it got"),
            edited: false,
            kind: "came_back".into(),
            instruction: "Fix what QA found.".into(),
            minutes: None,
            state: state.into(),
            workspace_id: None,
            started_at: None,
            ended_at: None,
            note: None,
            busy_at: None,
            nudged_at: None,
        }
    }

    fn ran(mut task: FlowTask, started: i64, ended: Option<i64>) -> FlowTask {
        (task.started_at, task.ended_at, task.workspace_id) = (Some(started), ended, Some("w2".into()));
        task
    }

    fn flow(id: &str, name: &str, state: &str, start_at: Option<i64>, tasks: Vec<FlowTask>) -> Flow {
        Flow {
            id: id.into(),
            project_id: "p1".into(),
            name: name.into(),
            instruction: String::new(),
            start_at,
            minutes: 60,
            live: true,
            state: state.into(),
            current: None,
            live_before: None,
            tasks,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn report(id: i64, key: &str, state: &str, text: &str, images: &[&str], at: i64) -> Report {
        Report {
            id,
            subject: key.into(),
            session_id: Some("s-w2".into()),
            workspace_id: Some("w2".into()),
            state: state.into(),
            text: text.into(),
            images: images.iter().map(|name| name.to_string()).collect(),
            at,
        }
    }

    fn workspaces() -> Vec<Workspace> {
        vec![ws("w2", "sh-21309-export", "feature/SH-21309-export")]
    }

    const MIN: i64 = 60_000;

    /// Shows a workflow: its tasks at a glance, and the one picked, in the order you read it (since item 17.5 the page
    /// draws it as one list and the task).
    #[test]
    fn a_workflow_that_runs_its_tasks_at_a_glance_and_the_one_picked_across_the_page() {
        let now = now();
        let started = now - 20 * MIN;
        let mut work = crate::work::sample();
        let mut running = ran(task("SH-21309", "running"), started, None);
        running.minutes = Some(45);
        work.flows = vec![Flow {
            current: Some(1),
            ..flow(
                "f1",
                "Night run",
                "running",
                Some(now - 60 * MIN),
                vec![
                    ran(task("SH-21533", "done"), now - 50 * MIN, Some(started)),
                    running,
                    task("SH-21376", "waiting"),
                ],
            )
        }];
        work.reports = vec![
            report(3, "SH-21309", "update", "Found it: the separator.", &["1-half.png"], started + 5 * MIN),
            report(2, "SH-21533", "done", "Fixed the double count.\n\nThe tests pass.", &[], started - MIN),
        ];
        assert_eq!(header_chip(&work), Some(true), "one runs");
        let page = FlowsModel::of(&work, None, now);
        assert_eq!(page.current.as_deref(), Some("flow:f1"), "the one that runs is shown");
        // it still runs: its chain has no ends yet (its head says when it started)
        assert_eq!(chain_ends("flow:f1", &work, &workspaces()), None);
        assert_eq!(page.coming.len(), 1);
        assert_eq!((page.coming[0].words.as_str(), page.coming[0].current), ("Running 2 of 3: SH-21309", true));
        assert!(page.runs.is_empty());
        // the middle: how it goes, and all its tasks at a glance, each with its state
        let detail = page.detail.as_ref().expect("the workflow shown");
        assert_eq!(detail.words, "Running 2 of 3: SH-21309");
        assert_eq!(detail.when, format!("Started {}", clock_of(now - 50 * MIN)), "its first task's start");
        assert_eq!(detail.lamp, None);
        let bar = detail.bar.as_ref().expect("a bar while it runs");
        assert_eq!(bar.label, "1 of 3 tasks over");
        assert_eq!(bar.segments, [Some(TaskTone::Green), Some(TaskTone::Accent), None]);
        assert_eq!(detail.count, "3 tasks");
        assert_eq!(detail.first.as_deref(), Some("ticket:SH-21309"), "it opens on the one that runs");
        let said: Vec<String> = detail.lines.iter().enumerate().map(|(n, line)| line.said(n + 1)).collect();
        assert_eq!(said, ["1 SH-21533 Done 30 min", "2 SH-21309 Running 20 min", "3 SH-21376 Waiting"]);
        assert_eq!(detail.lines[0].kind, "Fix what came back · 1 note");
        assert_eq!(detail.lines[2].kind, "Fix what came back");
        // it runs: it can be stopped or deleted, not started or changed
        assert_eq!(detail.buttons.stop, Some(("f1".to_string(), STOP_RUNNING)));
        assert_eq!(detail.buttons.delete.as_deref(), Some("f1"));
        assert_eq!((detail.buttons.run_now.as_ref(), detail.buttons.change.as_ref()), (None, None));

        // the right: the task, how it stands (the colored box), what it was given, what happened
        let shown = &detail.shown;
        let of = |subject: Option<&str>| {
            TaskDetail::of(shown, detail.task(subject).expect("a task"), &work, &workspaces(), now)
        };
        let page = of(None);
        assert_eq!((page.subject.as_str(), page.place.as_str()), ("ticket:SH-21309", "Task 2 of 3"));
        assert_eq!(
            page.facts,
            format!("Fix what came back · sh-21309-export · {} · 20 min of 45 min · Live", clock_of(started))
        );
        assert_eq!(
            page.outcome_said(),
            format!("Running for 20 min of 45 minIn sh-21309-export since {}, Live.", clock_of(started))
        );
        assert_eq!((page.message_heading, page.message_at.clone()), ("The message we sent", Some(clock_of(started))));
        assert_eq!(page.message, "SH-21309: the message it got");
        assert_eq!(page.workspace, Some(("w2".to_string(), "sh-21309-export".to_string())));
        let kinds: Vec<&str> = page.steps.iter().map(|(_, step)| step.kind()).collect();
        assert_eq!(kinds, ["sent", "update"]);
        assert_eq!(
            page.steps[0],
            (
                clock_of(started),
                Step::Sent("Sent the message to the Claude in sh-21309-export; Live followed it".into())
            )
        );
        let Step::Report(update) = &page.steps[1].1 else { panic!("an update") };
        assert_eq!((update.text.as_str(), update.images.len()), ("Found it: the separator.", 1));
        assert_eq!(page.steps[1].1.tone(), TaskTone::Accent);
        // another task, picked: how it ended, Claude's own first words up top; then its report
        let page = of(Some("ticket:SH-21533"));
        assert_eq!(page.outcome_said(), "Done, nothing needs you“Fixed the double count.”");
        assert_eq!(page.steps.iter().map(|(_, step)| step.kind()).collect::<Vec<_>>(), ["sent", "done"]);
        // ↓ goes through the tasks, and stops at the last
        assert_eq!(detail.step(Some("ticket:SH-21533"), 1), Some("ticket:SH-21309"));
        assert_eq!(detail.step(Some("ticket:SH-21309"), 1), Some("ticket:SH-21376"));
        assert_eq!(detail.step(Some("ticket:SH-21376"), 1), Some("ticket:SH-21376"));
        assert_eq!(detail.step(Some("ticket:SH-21533"), -1), Some("ticket:SH-21533"));
        assert_eq!(detail.step(None, -1), Some("ticket:SH-21533"), "from the one it opened on");
        // the one waiting says where and after which it runs, and the message it will get
        let page = of(Some("ticket:SH-21376"));
        assert_eq!(
            page.outcome_said(),
            "Waiting its turnTask 3 of 3, after SH-21309: the Claude in sh-21309-export, up to 60 minutes."
        );
        assert_eq!((page.message_heading, page.message_at.clone()), ("The message it will get", None));
        assert!(page.steps.is_empty(), "nothing happened yet");
        assert_eq!(page.facts, "Fix what came back · sh-21309-export · up to 60 min");
    }

    /// Keeps every run: listed under the workflows still to run, the newest first, each opening on the task that needs
    /// you.
    #[test]
    fn every_run_is_kept_the_newest_first_each_opening_on_the_task_that_needs_you() {
        let now = now();
        let at = now - 26 * 60 * MIN;
        let before = at - 24 * 60 * MIN;
        let start = |key: &str, state: &str| FlowTask { kind: "start".into(), ..task(key, state) };
        let mut stuck = ran(start("SH-21522", "blocked"), at + 50 * MIN, Some(at + 90 * MIN));
        stuck.nudged_at = Some(at + 80 * MIN);
        let night = flow(
            "f1",
            "Night run",
            "done",
            Some(at),
            vec![
                ran(start("SH-21376", "done"), at, Some(at + 30 * MIN)),
                ran(start("SH-21309", "review"), at + 30 * MIN, Some(at + 50 * MIN)),
                stuck,
            ],
        );
        let tonight = flow("f2", "Tonight", "scheduled", Some(now + 60 * MIN), vec![start("SH-21376", "waiting")]);
        let earlier = Flow {
            state: "stopped".into(),
            tasks: vec![
                ran(start("SH-21309", "done"), before, Some(before + 30 * MIN)),
                ran(start("SH-21376", "stopped"), before + 30 * MIN, Some(before + 40 * MIN)),
            ],
            ..night.clone()
        };
        let mut work = crate::work::sample();
        work.flows = vec![night.clone(), tonight];
        work.flow_runs = vec![
            FlowRun {
                id: 7,
                flow_id: "f1".into(),
                started_at: Some(at),
                ended_at: at + 90 * MIN,
                state: "done".into(),
                flow: night,
            },
            FlowRun {
                id: 5,
                flow_id: "f1".into(),
                started_at: Some(before),
                ended_at: before + 40 * MIN,
                state: "stopped".into(),
                flow: earlier,
            },
        ];
        work.reports = vec![
            report(
                4,
                "SH-21309",
                "review",
                "Added the export; the 7 s is worth your look.",
                &["1-list.png", "2-excel.png"],
                at + 49 * MIN,
            ),
            report(3, "SH-21309", "done", "The first half of the export.", &[], before + 29 * MIN),
        ];
        assert_eq!(header_chip(&work), Some(false), "none runs");
        let page = FlowsModel::of(&work, None, now);
        // still to run first, then every run that ended: the done workflow is there as its runs
        assert_eq!(page.coming.iter().map(|item| item.id.as_str()).collect::<Vec<_>>(), ["f2"]);
        let runs: Vec<String> = page.runs.iter().map(|run| format!("{} {}", run.id, run.state)).collect();
        assert_eq!(runs, ["7 done", "5 stopped"]);
        assert_eq!(page.runs[0].day, format!("Yesterday, {}", clock_of(at)));
        assert_eq!(page.runs[0].line, "1 done, 1 wants your review, 1 stuck");
        assert_eq!((page.runs[0].tone, page.runs[1].tone), (TaskTone::Red, TaskTone::Quiet));
        // nothing runs: the newest run is shown, its 3 tasks, and it opens on the first that needs you
        assert_eq!(page.current.as_deref(), Some("run:7"));
        // the ends of its chain: when it started and when it ended; a run that was stopped says so; one that waits
        // for its time has no ends yet
        let ends = |key: &str, work: &Snapshot| chain_ends(key, work, &workspaces());
        assert_eq!(
            ends("run:7", &work),
            Some((format!("{}, started", clock_of(at)), format!("{}, ended", clock_of(at + 90 * MIN))))
        );
        let ended = |key: &str, work: &Snapshot| ends(key, work).map(|(_, ended)| ended);
        assert_eq!(ended("run:5", &work), Some(format!("{}, stopped", clock_of(before + 40 * MIN))));
        assert_eq!(ends("flow:f2", &work), None);
        assert_eq!(ends("run:99", &work), None);
        // Live went back to the workspace it was on, while that one is still yours
        let mut lived = work.clone();
        lived.flow_runs[0].flow.live_before = Some("w2".into());
        assert_eq!(
            ended("run:7", &lived),
            Some(format!("{}, ended. Live went back to sh-21309-export.", clock_of(at + 90 * MIN)))
        );
        lived.flow_runs[0].flow.live_before = Some("gone".into());
        assert_eq!(ended("run:7", &lived), Some(format!("{}, ended", clock_of(at + 90 * MIN))));
        // a run that never started (Enkidu wasn't open) has nothing that happened
        lived.flow_runs[0].started_at = None;
        assert_eq!(ends("run:7", &lived), None);
        let detail = page.detail.as_ref().expect("the run shown");
        assert_eq!(detail.when, format!("Yesterday, {} · took 1 h 30 min", clock_of(at)));
        assert_eq!(detail.words, "Finished: 1 done, 1 wants your review, 1 stuck");
        assert_eq!((detail.lamp, detail.count.as_str()), (Some(TaskTone::Red), "3 tasks"));
        assert_eq!(detail.first.as_deref(), Some("ticket:SH-21309"));
        let states: Vec<&str> = detail.lines.iter().map(|line| line.state.as_str()).collect();
        assert_eq!(states, ["done", "review", "blocked"]);
        let of = |detail: &Detail, subject: Option<&str>| {
            TaskDetail::of(&detail.shown, detail.task(subject).expect("a task"), &work, &workspaces(), now)
        };
        let task = of(detail, None);
        assert_eq!(task.outcome_said(), "Done, wants your review“Added the export; the 7 s is worth your look.”");
        let Some((_, Step::Report(ended))) = task.steps.last() else { panic!("its report") };
        assert_eq!(ended.images.len(), 2);
        // the stuck one: Enkidu asked for its report, and how it ended
        let task = of(detail, Some("ticket:SH-21522"));
        assert_eq!(task.steps.iter().map(|(_, step)| step.kind()).collect::<Vec<_>>(), ["sent", "asked", "ended"]);
        assert_eq!((task.state.as_str(), task.tone), ("blocked", TaskTone::Red));
        assert_eq!(task.steps[1].1, Step::Asked(ASKED));
        assert_eq!(task.steps[2].1, Step::Ended("Stuck".into()));
        // its workflow doesn't run: it can run again, or be changed
        assert_eq!(detail.buttons.run_again.as_deref(), Some("f1"));
        assert_eq!(detail.buttons.change.as_deref(), Some("f1"));
        assert_eq!((detail.buttons.stop.as_ref(), detail.buttons.delete.as_ref()), (None, None));
        // the run before, a click away, with its own report (not the newer one)
        let page = FlowsModel::of(&work, Some("run:5"), now);
        assert_eq!(page.current.as_deref(), Some("run:5"));
        assert_eq!((page.runs[0].current, page.runs[1].current), (false, true));
        let detail = page.detail.as_ref().expect("the run shown");
        assert_eq!(detail.words, "Stopped: 1 done, 1 stopped");
        let task = of(detail, None);
        assert_eq!(task.subject, "ticket:SH-21309", "nothing needs you there: its first task");
        let texts: Vec<&str> = (task.steps.iter())
            .filter_map(|(_, step)| match step {
                Step::Report(report) => Some(report.text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, ["The first half of the export."]);
        // the workflow due tonight, picked: every task waiting, what it will get, and what can be done with it
        let page = FlowsModel::of(&work, Some("flow:f2"), now);
        let detail = page.detail.as_ref().expect("the workflow shown");
        assert_eq!((detail.when.as_str(), detail.bar.as_ref()), ("Starts today at 16:00", None));
        assert_eq!(detail.words, "Starts today at 16:00, 1 task");
        assert_eq!(detail.buttons.run_now.as_deref(), Some("f2"));
        assert_eq!(detail.buttons.stop, Some(("f2".to_string(), STOP_DUE)));
        assert_eq!(detail.count, "1 task");
        // a pick that is gone falls back to what is shown by itself
        assert_eq!(FlowsModel::of(&work, Some("run:99"), now).current.as_deref(), Some("run:7"));
        assert_eq!(FlowsModel::of(&work, Some("flow:f1"), now).current.as_deref(), Some("run:7"), "it ended: its runs");
    }

    /// Shows a run the usage limit stopped: it opens on the task that was cut off, in Claude Code's words; the rest
    /// never ran.
    #[test]
    fn a_run_the_usage_limit_stopped_opens_on_the_task_that_was_cut_off_and_the_rest_never_ran() {
        let now = now();
        let at = now - 6 * 60 * MIN;
        let start = |key: &str, state: &str| FlowTask {
            kind: "start".into(),
            message: format!("{key}: start this ticket."),
            ..task(key, state)
        };
        let mut cut = ran(start("SH-21309", "cut_off"), at + 20 * MIN, Some(at + 31 * MIN));
        cut.note = Some("You've hit your limit · resets 3am (Europe/Paris)".into());
        let night = flow(
            "f1",
            "Night run",
            "stopped",
            Some(at),
            vec![ran(start("SH-21376", "done"), at, Some(at + 20 * MIN)), cut, start("SH-21522", "waiting")],
        );
        let mut work = crate::work::sample();
        work.flows = vec![night.clone()];
        work.flow_runs = vec![FlowRun {
            id: 9,
            flow_id: "f1".into(),
            started_at: Some(at),
            ended_at: at + 31 * MIN,
            state: "stopped".into(),
            flow: night,
        }];
        let page = FlowsModel::of(&work, None, now);
        assert!(page.coming.is_empty(), "a workflow that stopped is there as its run");
        assert_eq!((page.runs[0].state.as_str(), page.runs[0].line.as_str()), ("stopped", "1 done, 1 cut off"));
        assert_eq!(page.runs[0].tone, TaskTone::Red);
        let detail = page.detail.as_ref().expect("the run shown");
        assert_eq!(detail.words, "Stopped: 1 done, 1 cut off");
        // it opens on the task that needs you: cut off, with what Claude Code said
        let states: Vec<(&str, &str)> = detail.lines.iter().map(|line| (line.state.as_str(), line.short)).collect();
        assert_eq!(states, [("done", "Done"), ("cut_off", "Cut off"), ("stopped", "Stopped")]);
        assert_eq!(detail.first.as_deref(), Some("ticket:SH-21309"));
        // the bar is the run's own: the task that never ran is a faint segment
        assert_eq!(
            detail.bar.as_ref().map(|bar| bar.segments.clone()),
            Some(vec![Some(TaskTone::Green), Some(TaskTone::Red), None])
        );
        let of = |subject: Option<&str>| {
            TaskDetail::of(&detail.shown, detail.task(subject).expect("a task"), &work, &workspaces(), now)
        };
        let task = of(None);
        assert_eq!(task.state, "cut_off");
        assert_eq!(task.outcome_said(), "Claude was cut offYou've hit your limit · resets 3am (Europe/Paris)");
        assert_eq!(
            task.steps.last().map(|(_, step)| step.clone()),
            Some(Step::Ended("Claude was cut off: You've hit your limit · resets 3am (Europe/Paris)".into()))
        );
        // the task after it never got its turn: it says so, not "waiting its turn"
        let task = of(Some("ticket:SH-21522"));
        assert_eq!(task.outcome_said(), "StoppedThe run stopped before its turn");
        assert_eq!(task.message_heading, "The message it would have got");
        assert!(task.steps.is_empty());
    }

    /// The first pane's order, a run that was missed, and the words of each state.
    #[test]
    fn orders_what_is_still_to_run_and_says_a_night_that_was_missed() {
        let now = now();
        let kept = |id: &str, updated: i64| Flow {
            updated_at: updated,
            ..flow(id, id, "saved", None, vec![task("SH-1", "waiting")])
        };
        let due = |id: &str, at: i64| flow(id, id, "scheduled", Some(at), vec![task("SH-1", "waiting")]);
        let flows = vec![
            kept("old", now - 5 * MIN),
            due("later", now + 120 * MIN),
            flow("done", "done", "done", Some(now - 600 * MIN), vec![task("SH-1", "done")]),
            kept("new", now - MIN),
            due("sooner", now + 30 * MIN),
            flow("runs", "runs", "running", Some(now - MIN), vec![task("SH-1", "running")]),
        ];
        let order: Vec<&str> = coming(&flows).iter().map(|flow| flow.id.as_str()).collect();
        assert_eq!(order, ["runs", "sooner", "later", "new", "old"]);
        // nothing runs and no run is kept: the next one due is shown
        let quiet: Vec<Flow> = flows.iter().filter(|flow| flow.state != "running").cloned().collect();
        assert_eq!(current(None, &coming(&quiet), &[]).as_deref(), Some("flow:sooner"));
        assert_eq!(current(None, &[], &[]), None);
        // a night Enkidu wasn't open
        let missed = FlowRun {
            id: 3,
            flow_id: "later".into(),
            started_at: None,
            ended_at: now - 300 * MIN,
            state: "missed".into(),
            flow: flow("later", "Night run", "missed", Some(now - 780 * MIN), vec![task("SH-1", "waiting")]),
        };
        assert_eq!(run_tone(&missed), TaskTone::Quiet);
        assert_eq!(run_line(&missed), "Didn't run: Enkidu wasn't open at 02:00");
        let mut work = Snapshot { flows: quiet, flow_runs: vec![missed], ..Default::default() };
        let page = FlowsModel::of(&work, None, now);
        let detail = page.detail.as_ref().expect("the missed run");
        assert_eq!(detail.words, "Didn't run: Enkidu wasn't open at 02:00");
        assert_eq!(detail.when, format!("Today, {}", clock_of(now - 300 * MIN)));
        assert_eq!(detail.bar, None, "nothing ran: no bar");
        assert_eq!(detail.buttons.run_again.as_deref(), Some("later"));
        // a run that ended with nothing to count, and one stopped before a task ended
        let ended = |state: &str, tasks: Vec<FlowTask>| FlowRun {
            id: 4,
            flow_id: "gone".into(),
            started_at: Some(now - 60 * MIN),
            ended_at: now - 50 * MIN,
            state: state.into(),
            flow: flow("gone", "Gone", state, Some(now - 60 * MIN), tasks),
        };
        assert_eq!(run_line(&ended("stopped", vec![task("SH-1", "waiting")])), "Stopped before a task ended");
        assert_eq!(run_line(&ended("done", vec![])), "Finished");
        assert_eq!(run_tone(&ended("done", vec![task("SH-1", "done")])), TaskTone::Green);
        assert_eq!(run_tone(&ended("done", vec![task("SH-1", "done"), task("SH-2", "timed_out")])), TaskTone::Amber);
        // a run whose workflow you deleted can't run again
        work.flow_runs = vec![ended("done", vec![task("SH-1", "done")])];
        let page = FlowsModel::of(&work, Some("run:4"), now);
        assert_eq!(page.detail.as_ref().map(|detail| detail.buttons.clone()), Some(Buttons::default()));
        // every state's word, and "what every task was told" on the run
        let words: Vec<&str> = [
            "waiting",
            "running",
            "done",
            "review",
            "blocked",
            "no_report",
            "timed_out",
            "asked",
            "cut_off",
            "failed",
            "stopped",
        ]
        .iter()
        .map(|state| task_short(state))
        .collect();
        assert_eq!(
            words,
            [
                "Waiting",
                "Running",
                "Done",
                "Review",
                "Stuck",
                "No report",
                "Out of time",
                "Asked you",
                "Cut off",
                "Didn't run",
                "Stopped"
            ]
        );
        let mut told = ended("done", vec![task("SH-1", "done")]);
        told.flow.instruction = " Use the small stack. ".into();
        work.flow_runs = vec![told];
        let page = FlowsModel::of(&work, None, now);
        assert_eq!(
            page.detail.and_then(|detail| detail.every).as_deref(),
            Some("For every task: Use the small stack.")
        );
        // nothing to show the header's Workflows for
        assert_eq!(header_chip(&Snapshot::default()), None);
    }

    /// A task that is to run in a new workspace, or in a new conversation, says so while it waits; one that didn't
    /// end with a report says how it ended, with its note.
    #[test]
    fn a_waiting_task_says_where_it_will_run_and_one_without_a_report_how_it_ended() {
        let now = now();
        let mut fresh = task("SH-21580", "waiting");
        fresh.target =
            Target::New { name: "sh-21580-outlet".into(), branch: "feature/SH-21580".into(), mode: "new".into() };
        let mut again = task("SH-21309", "waiting");
        again.target = Target::Conversation { workspace_id: "w2".into() };
        let mut out = ran(task("SH-21376", "timed_out"), now - 90 * MIN, Some(now - 30 * MIN));
        out.note = Some("Its 60 minutes were up".into());
        out.workspace_id = Some("gone".into());
        let shown =
            Flow { live: false, ..flow("f1", "Night run", "scheduled", Some(now + MIN), vec![fresh, again, out]) };
        let work = Snapshot::default();
        let of = |n: usize| TaskDetail::of(&shown, &shown.tasks[n], &work, &workspaces(), now);
        let first = of(0);
        assert_eq!(first.outcome_said(), "Waiting its turnTask 1 of 3: a new workspace, up to 60 minutes.");
        assert_eq!(first.facts, "Fix what came back · a new workspace · up to 60 min");
        assert_eq!(first.workspace, None);
        assert_eq!(
            of(1).outcome_said(),
            "Waiting its turnTask 2 of 3, after SH-21580: a new conversation in sh-21309-export, up to 60 minutes."
        );
        // its workspace is no longer one of yours: it is "its workspace", and nothing to open
        let third = of(2);
        assert_eq!(third.outcome_said(), "Its time ran outIts 60 minutes were up");
        assert_eq!(third.workspace, None);
        assert_eq!(
            third.facts,
            format!("Fix what came back · {}–{} · 1 h of 60 min", clock_of(now - 90 * MIN), clock_of(now - 30 * MIN)),
            "not Live: it doesn't say so"
        );
        assert_eq!(
            third.steps.iter().map(|(_, step)| step.clone()).collect::<Vec<_>>(),
            [
                Step::Sent("Sent the message to the Claude in its workspace".into()),
                Step::Ended("Its time ran out: Its 60 minutes were up".into())
            ]
        );
        // Claude's first words lose a heading's marks and bold's
        let work = Snapshot {
            reports: vec![report(
                1,
                "SH-21376",
                "review",
                "\n## **Export** works\n\nAll tests pass.",
                &[],
                now - 31 * MIN,
            )],
            ..Default::default()
        };
        let third = TaskDetail::of(&shown, &shown.tasks[2], &work, &workspaces(), now);
        assert_eq!(third.said.as_deref(), Some("Export works"));
        assert_eq!(third.steps.iter().map(|(_, step)| step.kind()).collect::<Vec<_>>(), ["sent", "review"]);
    }
}
