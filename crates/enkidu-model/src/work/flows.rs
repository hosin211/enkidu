//! Your workflows (DESIGN.md, "Workflows"): the tasks you picked, run one after another from a time you choose, each in
//! its Claude (or a new conversation, or a new workspace) for at most so many minutes. Each task is its own kind of
//! work, picked from its card's state (start it, carry on, fix what came back, answer the review, fix the checks,
//! investigate only, or your own words), with its own words and minutes. Here: each task's message, word for word as it
//! goes, the time a clock time means next, how a workflow and its tasks stand, in words, and which cards are in a
//! workflow that is due or runs.

use std::collections::{BTreeMap, HashMap, HashSet};

use enkidu_core::db::Workspace;
use enkidu_core::work::flows::{Flow, FlowTask, Target, cli_flag};
use enkidu_core::work::model::Snapshot;

use super::claudes::{BranchMode, ClaudeChoice, SendTarget, card_claudes, task_target};
use super::compose::{card_items, item_lines};
use super::groups::Group;
use super::when::{clock_of, local, moment, when_words};

/// A kind of work a task can be: its name in the core, what it is called, and the words its instruction starts from
/// (yours, once you made them yours).
pub struct KindDef {
    pub id: &'static str,
    pub label: &'static str,
    pub words: &'static str,
}

pub const KIND_DEFS: [KindDef; 7] = [
    KindDef {
        id: "start",
        label: "Start it",
        words: "Start this ticket: read it and its comments in Jira, work out what has to change, make the change on \
                this branch, and run the tests that cover it.",
    },
    KindDef {
        id: "continue",
        label: "Carry on",
        words: "Carry on with this ticket where it stands: look at what is already on this branch (the changes in the \
                diff, the ticket and its comments), finish what is left, and run the tests that cover it.",
    },
    KindDef {
        id: "came_back",
        label: "Fix what came back",
        words: "This ticket came back to us. Read its newest comments in Jira (what waits on it is below): find what \
                they found, reproduce it, fix it on this branch, and run the tests that cover it.",
    },
    KindDef {
        id: "review",
        label: "Answer the review",
        words: "Work through the review on this ticket's PR (the remarks are below): fix each one that is right, and \
                for anything that needs an answer, draft the reply in your report; don't post it.",
    },
    KindDef {
        id: "checks",
        label: "Fix the failing checks",
        words: "The PR's checks are failing: find out why from their output, fix it on this branch, and run the \
                failing tests here.",
    },
    KindDef {
        id: "investigate",
        label: "Investigate only",
        words: "Investigate only: find the cause, and the change you would make, but don't change any code. Put what \
                you found in your report.",
    },
    KindDef { id: "custom", label: "Your own words", words: "" },
];

/// What a kind is called ("Fix what came back"); a kind this build doesn't know is called by its own name.
pub fn kind_label(kind: &str) -> &str {
    KIND_DEFS.iter().find(|def| def.id == kind).map_or(kind, |def| def.label)
}

/// The words a kind of task starts from: yours where you made them yours, else Enkidu's.
pub fn kind_text(kind: &str, mine: &BTreeMap<String, String>) -> String {
    match mine.get(kind) {
        Some(yours) => yours.clone(),
        None => KIND_DEFS.iter().find(|def| def.id == kind).map_or("", |def| def.words).to_string(),
    }
}

/// What kind of work a card is now, from what waits on it: it came back (QA, a mention, back to you), its review has
/// remarks, its checks fail; else carry on (it has an open PR or a workspace) or start it.
pub fn kind_for(
    group: &Group,
    work: &Snapshot,
    workspaces: &[Workspace],
    statuses: &HashMap<String, String>,
) -> &'static str {
    // a task of your own is told in your own words
    if group.own.is_some() {
        return "custom";
    }
    let kinds: HashSet<&str> = card_items(group, work).into_iter().map(|item| item.kind.as_str()).collect();
    let any = |of: &[&str]| of.iter().any(|kind| kinds.contains(kind));
    if any(&["back_to_you", "qa_replied", "mentioned"]) {
        return "came_back";
    }
    if any(&["remark", "probably_fixed", "question", "changes_requested"]) {
        return "review";
    }
    if any(&["pipeline_failed"]) {
        return "checks";
    }
    let started = group.prs.iter().any(|view| view.of(work).state == "OPEN")
        || (card_claudes(group, work, workspaces, statuses, None, None).options.iter())
            .any(|option| option.workspace.is_some());
    if started { "continue" } else { "start" }
}

/// Every task's message says it, whatever the instruction: the work stays uncommitted, to be read in the diff in the
/// morning (and Live, which copies uncommitted changes, keeps showing it). Enkidu's hook refuses a commit or a
/// push during a workflow's task anyway.
pub const NO_COMMITS: &str = "Don't commit and don't push (no merge, rebase or PR either): leave every change \
                              uncommitted, so I review it in the diff in the morning. Enkidu refuses those \
                              commands while this task runs.";
pub const DEFAULT_MINUTES: u32 = 60;
pub const DEFAULT_TIME: &str = "02:00";
/// A task's own minutes, and a workflow's, stay within these.
pub const MINUTES: (u32, u32) = (5, 480);

/// The next time the clock shows `hhmm` ("02:00"), from `now`: today when it is still ahead, else tomorrow. None
/// for words that are no time of day.
pub fn next_at(hhmm: &str, now: i64) -> Option<i64> {
    let (hours, minutes) = hhmm.trim().split_once(':')?;
    let digits =
        |text: &str, most: usize| !text.is_empty() && text.len() <= most && text.bytes().all(|b| b.is_ascii_digit());
    if !digits(hours, 2) || !(minutes.len() == 2 && digits(minutes, 2)) {
        return None;
    }
    let (hour, minute): (u32, u32) = (hours.parse().ok()?, minutes.parse().ok()?);
    if hour > 23 || minute > 59 {
        return None;
    }
    let today = local(now);
    let at = moment(today.year, today.month, today.day, hour, minute);
    // this very minute is past: the day after (the calendar carries a day past the month's end)
    Some(if at <= now { moment(today.year, today.month, today.day + 1, hour, minute) } else { at })
}

/// How long, in words: "12 min", "1 h 5 min", "2 h".
pub fn length_words(ms: i64) -> String {
    let minutes = ((ms as f64) / 60_000.).round().max(0.) as i64;
    match (minutes / 60, minutes % 60) {
        (0, minutes) => format!("{minutes} min"),
        (hours, 0) => format!("{hours} h"),
        (hours, minutes) => format!("{hours} h {minutes} min"),
    }
}

/// What a task is called: its ticket's key, its PR ("PR #2415"), or "Task 3" for a task of your own.
pub fn task_name(subject: &str) -> String {
    if let Some(id) = enkidu_core::work::model::own_id(subject) {
        return format!("Task {id}");
    }
    match subject.strip_prefix("ticket:") {
        Some(key) => key.to_string(),
        None => format!("PR #{}", subject.rsplit('#').next().unwrap_or_default()),
    }
}

/// What a task of your own starts from in a workflow (item 8.7): it has no ticket to read, so the kinds' words, which
/// speak of Jira, don't fit it. Its kind is "Your own words", with these until you write others.
pub const OWN_TASK_WORDS: &str = "Do this task of mine (it is not in Jira; what it is about is above): work out what \
                                  has to change, make the change on this branch, and run the tests that cover it.";

/// A new workspace for the card. None: nowhere to start one (no project, no branch).
pub fn fresh_target(
    group: &Group,
    work: &Snapshot,
    workspaces: &[Workspace],
    statuses: &HashMap<String, String>,
) -> Option<Target> {
    match task_target(group, work, workspaces, statuses, Some(&ClaudeChoice::Fresh)) {
        SendTarget::New { name, branch, mode, .. } => {
            let mode = if mode == BranchMode::New { "new" } else { "existing" };
            Some(Target::New { name, branch, mode: mode.to_string() })
        }
        _ => None,
    }
}

/// Where a card's task runs unless you say: the Claude the card goes to when it is in a workspace, else a new
/// workspace for it. None: nowhere to start one.
pub fn default_target(
    group: &Group,
    work: &Snapshot,
    workspaces: &[Workspace],
    statuses: &HashMap<String, String>,
    chosen: Option<&ClaudeChoice>,
) -> Option<Target> {
    let claudes = card_claudes(group, work, workspaces, statuses, chosen, None);
    match claudes.picked().and_then(|option| option.workspace.as_ref()) {
        Some(workspace) => Some(Target::Workspace { workspace_id: workspace.id.clone() }),
        None => fresh_target(group, work, workspaces, statuses),
    }
}

/// One place a task can run, as "Runs in" lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct TargetChoice {
    /// `workspace:<id>`, `conversation:<id>` or `new`.
    pub value: String,
    /// "The Claude in sh-21309-export".
    pub label: String,
    pub target: Target,
}

/// A target as a choice's value.
pub fn target_value(target: &Target) -> String {
    match target {
        Target::New { .. } => "new".to_string(),
        Target::Workspace { workspace_id } => format!("workspace:{workspace_id}"),
        Target::Conversation { workspace_id } => format!("conversation:{workspace_id}"),
    }
}

/// Where a task can run: each of the card's workspaces, as its Claude is or in a new conversation, then a new
/// workspace.
pub fn target_choices(
    group: &Group,
    work: &Snapshot,
    workspaces: &[Workspace],
    statuses: &HashMap<String, String>,
) -> Vec<TargetChoice> {
    let mut choices: Vec<TargetChoice> = Vec::new();
    for option in card_claudes(group, work, workspaces, statuses, None, None).options {
        let own = workspaces.iter().find(|workspace| workspace.session_id == option.session.id).cloned();
        let Some(workspace) = own.or(option.workspace) else { continue };
        if choices.iter().any(|choice| choice.value == format!("workspace:{}", workspace.id)) {
            continue;
        }
        choices.push(TargetChoice {
            value: format!("workspace:{}", workspace.id),
            label: format!("The Claude in {}", workspace.name),
            target: Target::Workspace { workspace_id: workspace.id.clone() },
        });
        choices.push(TargetChoice {
            value: format!("conversation:{}", workspace.id),
            label: format!("A new conversation in {}", workspace.name),
            target: Target::Conversation { workspace_id: workspace.id.clone() },
        });
    }
    if let Some(fresh) = fresh_target(group, work, workspaces, statuses) {
        let name = match &fresh {
            Target::New { name, .. } => name.clone(),
            _ => String::new(),
        };
        choices.push(TargetChoice { value: "new".into(), label: format!("A new workspace, {name}"), target: fresh });
    }
    choices
}

/// What of a workflow goes into every task's message.
#[derive(Clone, Debug, PartialEq)]
pub struct FlowWords {
    pub name: String,
    pub instruction: String,
    pub minutes: u32,
    pub live: bool,
}

impl FlowWords {
    pub fn of(flow: &Flow) -> FlowWords {
        FlowWords {
            name: flow.name.clone(),
            instruction: flow.instruction.clone(),
            minutes: flow.minutes,
            live: flow.live,
        }
    }
}

/// The message a task's Claude gets: what the card is (its links), the task's own words, what waits on the card (a QA
/// reply, the review's remarks, failing checks), what every task of the workflow is told, then the rules (no
/// commits), how long it has, Live, and how to report. `group` is the task's card when it is still on the board.
pub fn flow_message(
    group: Option<(&Group, &Snapshot)>,
    subject: &str,
    title: &str,
    instruction: &str,
    minutes: Option<u32>,
    flow: &FlowWords,
) -> String {
    let mut lines: Vec<String> = Vec::new();
    let name = task_name(subject);
    lines.push(format!("{name}: {}", group.map_or(title, |(group, _)| group.title.as_str())));
    let mut waiting = 0;
    if let Some((group, work)) = group {
        if let Some(ticket) = group.ticket(work).filter(|ticket| !ticket.url.is_empty()) {
            lines.push(format!("Ticket: {}", ticket.url));
        }
        // a task of your own: that it is yours, and what you wrote about it
        if let Some(task) = group.own(work) {
            lines.push("It is a task of mine, not in Jira.".to_string());
            if !task.notes.is_empty() {
                lines.extend([String::new(), task.notes.clone()]);
            }
        }
        for pr in group.prs.iter().map(|view| view.of(work)).filter(|pr| pr.state == "OPEN") {
            lines.push(format!("PR #{} ({} into {}): {}", pr.number, pr.head, pr.base, pr.url));
        }
    }
    let own = instruction.trim();
    if !own.is_empty() {
        lines.extend([String::new(), own.to_string()]);
    }
    if let Some((group, work)) = group {
        let items = card_items(group, work);
        waiting = items.len();
        if waiting > 0 {
            lines.extend([String::new(), "What waits on it:".to_string()]);
            lines.extend(item_lines(group, work, &items));
        }
    }
    let every = flow.instruction.trim();
    if !every.is_empty() {
        let said = match !own.is_empty() || waiting > 0 {
            true => format!("For every task of this workflow: {every}"),
            false => every.to_string(),
        };
        lines.extend([String::new(), said]);
    }
    lines.extend([String::new(), NO_COMMITS.to_string()]);
    let minutes = minutes.unwrap_or(flow.minutes);
    lines.extend([
        String::new(),
        format!(
            "This is a task of Enkidu's workflow \"{}\", which runs while I'm away: you have about {minutes} \
             minutes, then the next task starts.",
            flow.name.trim()
        ),
    ]);
    lines.push(
        match flow.live {
            true => {
                "This workspace is Live while you work: its changes are in the project's main folder, where the \
                     dev environment runs, so check your work there."
            }
            false => {
                "To check your work in the dev environment, run `enkidu live on` first (it copies this workspace's \
                      changes into the project's main folder, where the dev environment runs)."
            }
        }
        .to_string(),
    );
    lines.push(format!(
        "While you work, `enkidu update \"…\"` (with `--image FILE` for a screenshot) tells the workflow how it goes; `enkidu \
         workflow` shows the workflow and your time left. When you're done, or can't go on, report how it went: `enkidu \
         report done|review|blocked {} \"what you changed, how you checked it, what is left\"`, with `--image FILE` \
         for screenshots of what you checked (repeat it for more). done: nothing needs me; review: it wants my \
         review; blocked: you couldn't finish.",
        cli_flag(subject)
    ));
    lines.join("\n")
}

/// A task for a card: its kind from the card's state, its instruction that kind's words, its message made from both.
pub fn task_for(
    group: &Group,
    work: &Snapshot,
    target: Target,
    flow: &FlowWords,
    workspaces: &[Workspace],
    statuses: &HashMap<String, String>,
    mine: &BTreeMap<String, String>,
) -> FlowTask {
    let kind = kind_for(group, work, workspaces, statuses);
    let instruction = if group.own.is_some() { OWN_TASK_WORDS.to_string() } else { kind_text(kind, mine) };
    FlowTask {
        subject: group.id.clone(),
        title: group.title.clone(),
        target,
        message: flow_message(Some((group, work)), &group.id, &group.title, &instruction, None, flow),
        edited: false,
        kind: kind.to_string(),
        instruction,
        minutes: None,
        state: "waiting".into(),
        workspace_id: None,
        started_at: None,
        ended_at: None,
        note: None,
        busy_at: None,
        nudged_at: None,
    }
}

/// The tasks' messages made again from the workflow and each task (their words, their minutes, the workflow's name or
/// Live changed); the ones whose whole message you edited stay as you wrote them.
pub fn remake(mut flow: Flow, work: &Snapshot, groups: &[Group]) -> Flow {
    remake_in(&mut flow, work, groups);
    flow
}

/// The same, on the workflow where it is (the dialog's, after every change).
pub fn remake_in(flow: &mut Flow, work: &Snapshot, groups: &[Group]) {
    let words = FlowWords::of(flow);
    for task in flow.tasks.iter_mut().filter(|task| !task.edited) {
        let group = groups.iter().find(|group| group.id == task.subject).map(|group| (group, work));
        task.message = flow_message(group, &task.subject, &task.title, &task.instruction, task.minutes, &words);
    }
}

/// The cards you picked added to a workflow, after its own tasks (a card it has already is left as it is; one with
/// nowhere to run is left out).
pub fn add_tasks(
    mut flow: Flow,
    cards: &[&Group],
    work: &Snapshot,
    workspaces: &[Workspace],
    statuses: &HashMap<String, String>,
    chosen: &HashMap<String, ClaudeChoice>,
    mine: &BTreeMap<String, String>,
) -> Flow {
    let words = FlowWords::of(&flow);
    for card in cards {
        if flow.tasks.iter().any(|task| task.subject == card.id) {
            continue;
        }
        if let Some(target) = default_target(card, work, workspaces, statuses, chosen.get(&card.id)) {
            flow.tasks.push(task_for(card, work, target, &words, workspaces, statuses, mine));
        }
    }
    flow
}

/// A new workflow for the cards you picked, starting tonight at two: each its kind, its words, to its Claude.
pub fn new_flow(
    cards: &[&Group],
    work: &Snapshot,
    workspaces: &[Workspace],
    statuses: &HashMap<String, String>,
    chosen: &HashMap<String, ClaudeChoice>,
    now: i64,
    mine: &BTreeMap<String, String>,
) -> Flow {
    let flow = Flow {
        id: String::new(),
        project_id: cards.iter().find_map(|card| card.project_id.clone()).unwrap_or_default(),
        name: "Night run".into(),
        instruction: String::new(),
        // at the time and with the minutes you set for new workflows (Settings, Workflows), 02:00 and 60 until you do
        start_at: next_at(&work.flow_defaults.time, now).or_else(|| next_at(DEFAULT_TIME, now)),
        minutes: work.flow_defaults.minutes,
        live: true,
        state: "saved".into(),
        current: None,
        live_before: None,
        tasks: Vec::new(),
        created_at: 0,
        updated_at: 0,
    };
    add_tasks(flow, cards, work, workspaces, statuses, chosen, mine)
}

/// The cards that can't be in this workflow: another project's (Live is a project's).
pub fn other_project<'a>(flow: &Flow, cards: &[&'a Group]) -> Vec<&'a Group> {
    (cards.iter().copied())
        .filter(|card| {
            card.project_id.as_deref().is_some_and(|id| !flow.project_id.is_empty() && id != flow.project_id)
        })
        .collect()
}

/// How a state reads: quiet, in the accent, or in a lamp's color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskTone {
    Quiet,
    Accent,
    Green,
    Amber,
    Red,
}

/// How a task stands, in words and tone ("Done, wants your review", amber).
pub fn task_look(state: &str) -> (&'static str, TaskTone) {
    match state {
        "running" => ("Running", TaskTone::Accent),
        "done" => ("Done, nothing needs you", TaskTone::Green),
        "review" => ("Done, wants your review", TaskTone::Amber),
        "blocked" => ("Stuck", TaskTone::Red),
        "no_report" => ("Stopped without a report", TaskTone::Amber),
        "timed_out" => ("Its time ran out", TaskTone::Amber),
        "asked" => ("Asked you something", TaskTone::Red),
        "cut_off" => ("Claude was cut off", TaskTone::Red),
        "failed" => ("Didn't run", TaskTone::Red),
        "stopped" => ("Stopped", TaskTone::Quiet),
        _ => ("Waiting", TaskTone::Quiet),
    }
}

/// How a run's tasks ended, in a few words: "2 done, 1 wants your review, 1 without a report".
pub fn run_tally(flow: &Flow) -> String {
    let count = |state: &str| flow.tasks.iter().filter(|task| task.state == state).count();
    let ended = flow.tasks.iter().filter(|task| !matches!(task.state.as_str(), "waiting" | "running")).count();
    let named = count("done") + count("review") + count("blocked") + count("cut_off") + count("stopped");
    let rest = ended.saturating_sub(named);
    let mut said: Vec<String> = Vec::new();
    if count("done") > 0 {
        said.push(format!("{} done", count("done")));
    }
    if count("review") > 0 {
        said.push(format!("{} want{} your review", count("review"), if count("review") == 1 { "s" } else { "" }));
    }
    if count("blocked") > 0 {
        said.push(format!("{} stuck", count("blocked")));
    }
    if count("cut_off") > 0 {
        said.push(format!("{} cut off", count("cut_off")));
    }
    if rest > 0 {
        said.push(format!("{rest} without a report"));
    }
    if count("stopped") > 0 {
        said.push(format!("{} stopped", count("stopped")));
    }
    said.join(", ")
}

/// How a workflow stands, in a few words.
pub fn flow_words(flow: &Flow, now: i64) -> String {
    let count = flow.tasks.len();
    let tasks = format!("{count} {}", if count == 1 { "task" } else { "tasks" });
    let tally = run_tally(flow);
    match flow.state.as_str() {
        "scheduled" => match flow.start_at {
            Some(at) => format!("Starts {}, {tasks}", when_words(at, now)),
            None => "Scheduled".into(),
        },
        "running" => {
            let at = flow.current.unwrap_or(0) as usize;
            match flow.tasks.get(at) {
                Some(task) => format!("Running {} of {count}: {}", at + 1, task_name(&task.subject)),
                None => "Running".into(),
            }
        }
        "done" => {
            let when = if flow.updated_at != 0 { when_words(flow.updated_at, now) } else { String::new() };
            let tally = if tally.is_empty() { String::new() } else { format!(": {tally}") };
            format!("Finished {when}{tally}").trim().to_string()
        }
        "stopped" if tally.is_empty() => "Stopped".into(),
        "stopped" => format!("Stopped ({tally})"),
        "missed" => {
            let when = flow.start_at.map_or_else(|| "at its time".to_string(), |at| when_words(at, now));
            format!("Missed: Enkidu wasn't running {when}")
        }
        _ => format!("Not scheduled, {tasks}"),
    }
}

/// The cards in a workflow that runs or is due, by card: the first workflow that has each as a task still to run.
pub fn flow_of_cards(flows: &[Flow]) -> HashMap<&str, &Flow> {
    let mut of: HashMap<&str, &Flow> = HashMap::new();
    for flow in flows.iter().filter(|flow| matches!(flow.state.as_str(), "running" | "scheduled")) {
        for task in flow.tasks.iter().filter(|task| matches!(task.state.as_str(), "waiting" | "running")) {
            of.entry(task.subject.as_str()).or_insert(flow);
        }
    }
    of
}

/// A card in a workflow that is due or runs: when it starts ("02:00"), that it is next, or that it runs now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlowMark {
    /// Its task runs now.
    pub running: bool,
    /// "Now", "Next", or the start as the clock shows it.
    pub text: String,
    /// When pointed at: "In the workflow Night shift, which starts tomorrow at 02:00".
    pub words: String,
}

/// What a card says of the workflow it is in, if it is in one that is due or runs.
pub fn flow_mark(flows: &HashMap<&str, &Flow>, card: &str, now: i64) -> Option<FlowMark> {
    let flow = flows.get(card)?;
    let running = flow.tasks.iter().any(|task| task.subject == card && task.state == "running");
    let name = &flow.name;
    let (text, words) = if running {
        ("Now".to_string(), format!("Running now in the workflow {name}"))
    } else if flow.state == "running" {
        ("Next".to_string(), format!("Waiting its turn in the workflow {name}"))
    } else {
        let starts = flow.start_at.map_or_else(|| "later".to_string(), |at| when_words(at, now));
        let clock = flow.start_at.map(clock_of).unwrap_or_default();
        (clock, format!("In the workflow {name}, which starts {starts}"))
    };
    Some(FlowMark { running, text, words })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::work::when::tests::{at, now};
    use enkidu_core::work::flows::{FlowTask, Target};

    pub(crate) fn task(subject: &str, state: &str) -> FlowTask {
        FlowTask {
            subject: subject.into(),
            title: subject.into(),
            target: Target::Workspace { workspace_id: "w2".into() },
            message: String::new(),
            edited: false,
            kind: "custom".into(),
            instruction: String::new(),
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

    pub(crate) fn flow(state: &str, tasks: Vec<FlowTask>) -> Flow {
        Flow {
            id: "f1".into(),
            project_id: "p1".into(),
            name: "Night shift".into(),
            instruction: String::new(),
            start_at: Some(at(12, 2, 0)),
            minutes: 60,
            live: true,
            state: state.into(),
            current: None,
            live_before: None,
            tasks,
            created_at: now(),
            updated_at: now(),
        }
    }

    /// Says how a workflow stands, and which cards are in one that is due or runs.
    #[test]
    fn knows_which_cards_are_in_a_workflow_that_is_due_or_runs() {
        let two = || vec![task("ticket:SH-21309", "waiting"), task("ticket:SH-21376", "waiting")];
        let scheduled = [flow("scheduled", two())];
        let mut cards: Vec<&str> = flow_of_cards(&scheduled).into_keys().collect();
        cards.sort();
        assert_eq!(cards, ["ticket:SH-21309", "ticket:SH-21376"]);
        // on the board: the task that runs and the one waiting its turn; not the one that ended, nor a finished one
        let running = [flow("running", vec![task("ticket:SH-21309", "review"), task("ticket:SH-21376", "running")])];
        assert_eq!(flow_of_cards(&running).into_keys().collect::<Vec<_>>(), ["ticket:SH-21376"]);
        let done = [flow("done", vec![task("ticket:SH-21309", "review"), task("ticket:SH-21376", "timed_out")])];
        assert!(flow_of_cards(&done).is_empty());
        assert!(flow_of_cards(&[flow("saved", two())]).is_empty(), "a workflow that is only kept isn't due");
    }

    /// The workspace of these tests, whose Claude is done, and the sample's cards with it.
    fn w2() -> Workspace {
        crate::work::groups::tests::ws("w2", "sh-21309-export", "feature/SH-21309-export")
    }

    fn lamps() -> HashMap<String, String> {
        crate::work::groups::tests::statuses(&[("w2", "done")])
    }

    fn cards() -> (Snapshot, Vec<Group>) {
        let work = crate::work::sample();
        let groups = crate::work::groups::build_groups(&work, &[w2()], &lamps(), crate::work::groups::tests::NOW);
        (work, groups)
    }

    fn card<'a>(groups: &'a [Group], key: &str) -> &'a Group {
        groups.iter().find(|group| group.key.as_deref() == Some(key)).unwrap_or_else(|| panic!("no card {key}"))
    }

    fn none() -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    /// Reads a clock time as the next time the clock shows it (its time words are in when.rs).
    #[test]
    fn reads_a_clock_time_as_the_next_time_the_clock_shows_it() {
        assert_eq!(next_at("02:00", now()), Some(at(12, 2, 0)), "already past today: tomorrow");
        assert_eq!(next_at("16:30", now()), Some(at(11, 16, 30)), "still ahead today");
        assert_eq!(next_at("15:00", now()), Some(at(12, 15, 0)), "this very minute is past");
        assert_eq!(next_at(" 2:05 ", now()), Some(at(12, 2, 5)));
        assert_eq!(next_at("25:00", now()), None);
        assert_eq!(next_at("soon", now()), None);
        assert_eq!(next_at("2:5", now()), None, "minutes are two digits");
        // the day after the month's last
        assert_eq!(next_at("02:00", moment(2026, 11, 30, 15, 0)), Some(moment(2026, 12, 1, 2, 0)));
        assert_eq!(
            [length_words(12 * 60_000), length_words(65 * 60_000), length_words(120 * 60_000)],
            ["12 min", "1 h 5 min", "2 h"]
        );
    }

    /// Knows what kind of work a card is from what waits on it.
    #[test]
    fn knows_what_kind_of_work_a_card_is_from_what_waits_on_it() {
        let (work, groups) = cards();
        let kind = |key: &str| kind_for(card(&groups, key), &work, &[w2()], &HashMap::new());
        assert_eq!(kind("SH-21616"), "came_back", "it came back to you in Jira");
        assert_eq!(kind("SH-21112"), "came_back", "QA replied");
        assert_eq!(kind("SH-21309"), "review", "remarks on its PR");
        assert_eq!(kind("SH-21376"), "continue", "an open PR, nothing waiting: carry on");
        assert_eq!(kind("SH-21580"), "start", "nothing started: start it");
        // your own words for a kind win over Enkidu's
        assert!(kind_text("came_back", &none()).contains("came back to us"));
        let mine: BTreeMap<String, String> =
            [("came_back".to_string(), "Read QA's comment first.".to_string())].into_iter().collect();
        assert_eq!(kind_text("came_back", &mine), "Read QA's comment first.");
        assert_eq!(kind_text("custom", &none()), "");
        assert_eq!((kind_label("came_back"), kind_label("custom")), ("Fix what came back", "Your own words"));
        // every kind the core knows has its words here
        let known: Vec<&str> = KIND_DEFS.iter().map(|def| def.id).collect();
        assert_eq!(known, enkidu_core::work::flows::KINDS);
    }

    /// Makes each task's message: the card, its own words, what waits on it, what every task is told, the rules.
    #[test]
    fn makes_each_tasks_message_the_card_its_own_words_what_waits_on_it_what_every_task_is_told_the_rules() {
        let (work, groups) = cards();
        let flow =
            FlowWords { name: "Night run".into(), instruction: "Use the small stack.".into(), minutes: 45, live: true };
        let export = card(&groups, "SH-21309");
        let text =
            flow_message(Some((export, &work)), "ticket:SH-21309", "x", "Answer mira-k's remarks only.", None, &flow);
        assert_eq!(text.lines().next(), Some("SH-21309: Export orders to the accounting CSV"));
        for said in [
            "Ticket: https://",
            "PR #2414 (feature/SH-21309-export into development)",
            "Answer mira-k's remarks only.",
            // what waits on the card goes with it, each with what to do: the same task coming back brings QA's words
            "What waits on it:",
            "Fix it if it's right; if not, draft a reply that says why.",
            "For every task of this workflow: Use the small stack.",
            "Don't commit and don't push (no merge, rebase or PR either): leave every change uncommitted",
            "workflow \"Night run\"",
            "about 45 minutes",
            "This workspace is Live while you work",
            "enkidu report done|review|blocked --ticket SH-21309 \"what you changed",
            "`enkidu update \"…\"`",
            "`enkidu workflow`",
        ] {
            assert!(text.contains(said), "{said:?} is missing from:\n{text}");
        }
        let numbered = text.lines().any(|line| {
            let (number, rest) = line.split_once(". ").unwrap_or(("", ""));
            number.len() == 1
                && number.bytes().all(|b| b.is_ascii_digit())
                && rest.starts_with("mira-k left a remark on orders/export.py:48")
        });
        assert!(numbered, "{text}");
        // a task's own minutes win; without Live, Claude is told how to go Live itself
        let not_live = FlowWords { live: false, ..flow.clone() };
        let own = flow_message(
            Some((card(&groups, "SH-21580"), &work)),
            "ticket:SH-21580",
            "x",
            "Start it.",
            Some(20),
            &not_live,
        );
        assert!(own.contains("about 20 minutes") && own.contains("run `enkidu live on` first"), "{own}");
        assert!(!own.contains("What waits on it:"), "nothing waits on it");
        // a card that left the board keeps its title, and what every task is told stands alone when it is all there is
        let gone = flow_message(None, "pr:demo-org/shop#2415", "Bump the linter", "", None, &flow);
        assert!(gone.starts_with("PR #2415: Bump the linter\n\nUse the small stack.\n\nDon't commit"), "{gone}");
        assert!(gone.contains("enkidu report done|review|blocked --pr 2415 "));
        assert_eq!(task_name("pr:demo-org/shop#2415"), "PR #2415");
    }

    /// Makes a workflow of the picked cards: each its own kind and words, to its Claude or a new workspace, tonight at
    /// two.
    #[test]
    fn makes_a_workflow_of_the_picked_cards_each_its_own_kind_and_words_to_its_claude_or_a_new_workspace() {
        let (work, groups) = cards();
        let picked = [card(&groups, "SH-21309"), card(&groups, "SH-21376"), card(&groups, "SH-21580")];
        let mine: BTreeMap<String, String> =
            [("start".to_string(), "Begin with a plan in your first update.".to_string())].into_iter().collect();
        let flow = new_flow(&picked, &work, &[w2()], &lamps(), &HashMap::new(), now(), &mine);
        assert_eq!(flow.project_id, "p1");
        assert_eq!(flow.start_at, Some(at(12, 2, 0)));
        assert_eq!(
            (flow.minutes, flow.live, flow.instruction.as_str(), flow.name.as_str()),
            (60, true, "", "Night run")
        );
        let tasks: Vec<String> = flow.tasks.iter().map(|task| format!("{} {}", task.subject, task.kind)).collect();
        assert_eq!(tasks, ["ticket:SH-21309 review", "ticket:SH-21376 continue", "ticket:SH-21580 start"]);
        assert_eq!(flow.tasks[0].instruction, kind_text("review", &none()));
        assert_eq!(flow.tasks[2].instruction, "Begin with a plan in your first update.", "your words for a kind");
        assert!(flow.tasks[0].message.contains(&kind_text("review", &none())));
        assert_eq!(flow.tasks[0].target, Target::Workspace { workspace_id: "w2".into() });
        // SH-21376's only Claude ran outside Enkidu: a new workspace, on its open PR's branch
        assert_eq!(
            flow.tasks[1].target,
            Target::New {
                name: "sh-21376-bundle-units".into(),
                branch: "feature/SH-21376-bundle-units".into(),
                mode: "existing".into()
            }
        );
        // where a task can run: its Claude as it is, a new conversation there, or a new workspace
        let choices = target_choices(card(&groups, "SH-21309"), &work, &[w2()], &HashMap::new());
        let values: Vec<&str> = choices.iter().map(|choice| choice.value.as_str()).collect();
        assert_eq!(values, ["workspace:w2", "conversation:w2", "new"]);
        assert_eq!(choices[0].label, "The Claude in sh-21309-export");
        assert_eq!(choices[1].label, "A new conversation in sh-21309-export");
        assert!(choices[2].label.starts_with("A new workspace, sh-21309-"), "{}", choices[2].label);
        assert_eq!(target_value(&flow.tasks[1].target), "new");
    }

    /// Adds picked cards to a workflow you have, leaving the ones it has as they are.
    #[test]
    fn adds_picked_cards_to_a_workflow_you_have_leaving_the_ones_it_has_as_they_are() {
        let (work, groups) = cards();
        let chosen = HashMap::new();
        let mut mine = new_flow(&[card(&groups, "SH-21309")], &work, &[w2()], &HashMap::new(), &chosen, now(), &none());
        mine.id = "f1".into();
        mine.tasks[0].instruction = "Only the header.".into();
        let both = [card(&groups, "SH-21309"), card(&groups, "SH-21580")];
        let more = add_tasks(mine, &both, &work, &[w2()], &HashMap::new(), &chosen, &none());
        let subjects: Vec<&str> = more.tasks.iter().map(|task| task.subject.as_str()).collect();
        assert_eq!(subjects, ["ticket:SH-21309", "ticket:SH-21580"]);
        assert_eq!(more.tasks[0].instruction, "Only the header.", "kept as you made it");
        // a card of another project can't be in it (Live is a project's)
        let elsewhere = Group { project_id: Some("p2".into()), ..card(&groups, "SH-21580").clone() };
        let others: Vec<Option<&str>> =
            other_project(&more, &[&elsewhere]).iter().map(|group| group.key.as_deref()).collect();
        assert_eq!(others, [Some("SH-21580")]);
        assert!(other_project(&more, &both).is_empty());
    }

    /// Makes the messages again when the workflow or a task changes, keeping the whole messages you edited.
    #[test]
    fn makes_the_messages_again_when_the_workflow_or_a_task_changes_keeping_the_whole_messages_you_edited() {
        let (work, groups) = cards();
        let picked = [card(&groups, "SH-21309"), card(&groups, "SH-21376")];
        let mut edited = new_flow(&picked, &work, &[w2()], &HashMap::new(), &HashMap::new(), now(), &none());
        edited.tasks[0].message = "Only the header, please.".into();
        edited.tasks[0].edited = true;
        edited.tasks[1].instruction = "Finish the units.".into();
        edited.instruction = "Write the tests first.".into();
        let next = remake(edited, &work, &groups);
        assert_eq!(next.tasks[0].message, "Only the header, please.");
        assert!(next.tasks[1].message.contains("Finish the units."));
        assert!(next.tasks[1].message.contains("For every task of this workflow: Write the tests first."));
    }

    /// Says how a workflow stands, and which cards are in one that is due or runs (the cards are in the test above this
    /// file's first).
    #[test]
    fn says_how_a_workflow_stands() {
        let (work, groups) = cards();
        let picked = [card(&groups, "SH-21309"), card(&groups, "SH-21376")];
        let base = new_flow(&picked, &work, &[w2()], &HashMap::new(), &HashMap::new(), now(), &none());
        let with = |state: &str, states: &[&str]| -> Flow {
            let mut flow = base.clone();
            flow.state = state.into();
            for (task, state) in flow.tasks.iter_mut().zip(states) {
                task.state = state.to_string();
            }
            flow
        };
        assert_eq!(flow_words(&with("scheduled", &[]), now()), "Starts tomorrow at 02:00, 2 tasks");
        let running = Flow { current: Some(1), ..with("running", &["review", "running"]) };
        assert_eq!(flow_words(&running, now()), "Running 2 of 2: SH-21376");
        let done = Flow { updated_at: at(12, 3, 40), ..with("done", &["review", "timed_out"]) };
        assert_eq!(flow_words(&done, at(12, 9, 0)), "Finished today at 03:40: 1 wants your review, 1 without a report");
        // the usage limit cut the second task off and stopped the run: the third never ran
        let mut limit = with("stopped", &["done", "cut_off"]);
        let mut third = limit.tasks[1].clone();
        (third.subject, third.state) = ("ticket:SH-1".into(), "waiting".into());
        limit.tasks.push(third);
        assert_eq!(flow_words(&limit, at(12, 9, 0)), "Stopped (1 done, 1 cut off)");
        let missed = Flow { start_at: Some(at(12, 2, 0)), ..with("missed", &[]) };
        assert_eq!(flow_words(&missed, at(12, 9, 0)), "Missed: Enkidu wasn't running today at 02:00");
        assert_eq!(flow_words(&with("saved", &[]), now()), "Not scheduled, 2 tasks");
        assert_eq!(run_tally(&with("done", &["review", "review"])), "2 want your review");
        // how each state reads
        assert_eq!(task_look("review"), ("Done, wants your review", TaskTone::Amber));
        assert_eq!(task_look("cut_off"), ("Claude was cut off", TaskTone::Red));
        assert_eq!(task_look("waiting"), ("Waiting", TaskTone::Quiet));
    }

    /// What a card in a workflow says.
    #[test]
    fn a_card_says_when_its_workflow_starts_that_it_is_next_or_that_it_runs_now() {
        let scheduled = [flow("scheduled", vec![task("ticket:SH-21309", "waiting")])];
        let mark = flow_mark(&flow_of_cards(&scheduled), "ticket:SH-21309", now()).unwrap();
        assert_eq!((mark.running, mark.text.as_str()), (false, "02:00"));
        assert_eq!(mark.words, "In the workflow Night shift, which starts tomorrow at 02:00");
        let tasks = vec![task("ticket:SH-21309", "running"), task("ticket:SH-21376", "waiting")];
        let running = [flow("running", tasks)];
        let of = flow_of_cards(&running);
        let mark = flow_mark(&of, "ticket:SH-21309", now()).unwrap();
        assert_eq!(
            (mark.running, mark.text.as_str(), mark.words.as_str()),
            (true, "Now", "Running now in the workflow Night shift")
        );
        let next = flow_mark(&of, "ticket:SH-21376", now()).unwrap();
        assert_eq!((next.running, next.text.as_str()), (false, "Next"));
        assert_eq!(next.words, "Waiting its turn in the workflow Night shift");
        assert_eq!(flow_mark(&of, "ticket:SH-1", now()), None, "a card in no workflow says nothing");
    }
}
