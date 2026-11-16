//! The workflow dialog, as it decides (DESIGN.md, "Workflows"): the picked cards as the tasks of a new workflow, or
//! added to one you have. What it holds while you set it up: the workflow as it would be saved, the cards you picked,
//! which workflow they go into, the clock time it starts at and whether it starts at a time at all. Every change of the
//! workflow or of a task makes the tasks' messages again, but the ones you edited whole. It says what is still missing
//! before it can be saved, and what each task's row shows.
//!
//! Since 15 Nov the dialog is a chain with one step shown beside it:
//! how the workflow starts, each task with the time it starts at the latest, and how it ends. So the model
//! also holds which step is shown, and says the chain's two ends and the plan in one line.

use std::collections::HashMap;

use enkidu_core::db::Workspace;
use enkidu_core::work::flows::{Flow, FlowTask, Target};
use enkidu_core::work::model::Snapshot;

use super::claudes::ClaudeChoice;
use super::compose::card_items;
use super::flows::{
    DEFAULT_TIME, MINUTES, TargetChoice, add_tasks, flow_words, kind_label, kind_text, length_words, new_flow, next_at,
    other_project, remake_in, target_choices, target_value, task_name,
};
use super::groups::Group;
use super::pr::summary;
use super::when::{clock_of, when_words};

/// The one line on what is different about a workflow, under its settings.
pub const UNLIKE: &str = "Unlike Tell Claude, a workflow sends each message (Enter) when its task's turn comes, so it \
                          runs while you're away. Nothing is committed or pushed: the changes wait in each \
                          workspace's diff. A task ends when Claude reports how it went, or when its minutes are up. \
                          Enkidu keeps your Mac awake until the workflow ends: leave it open and on power.";
/// What "For every task" is for, and what its empty field shows.
pub const EVERY_HINT: &str = "Added to each task's message: what every task should know tonight.";
pub const EVERY_ASKS: &str = "Use the small Docker stack; leave the migrations alone.";
/// What Live means here, beside its switch.
pub const LIVE: &str = "Live follows the task that runs";
pub const LIVE_SAYS: &str = "(the main folder gets its changes, for the dev environment; it goes back at the end)";
/// A task of your own words asks for them.
pub const OWN_WORDS_ASK: &str = "What Claude does on this task";
/// The start's step, at the right of the chain: its title and the line under it.
pub const START_TITLE: &str = "When it runs, and what every task gets";
pub const START_SAYS: &str = "Set once, for the whole workflow.";
/// What the chain's end says under its time, while Live follows the tasks.
pub const LIVE_BACK: &str = "Live goes back to the workspace it was on.";

/// A step of the workflow's chain, shown at the dialog's right: how the workflow starts and what every task gets,
/// or one task, by its card.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Step {
    #[default]
    Start,
    Task(String),
}

/// What the chain says of its two ends: each a title, and the quieter line under it.
#[derive(Clone, Debug, PartialEq)]
pub struct Ends {
    /// "Starts tomorrow at 02:00", "Kept until you start it"; "Starts at a time" while none is typed that is one.
    pub start: String,
    /// "Up to 60 min a task. Live follows."
    pub start_line: String,
    /// "Ends by 06:45 at the latest"; without a time, "Takes 4 h 45 min at the most".
    pub end: String,
    /// "Live goes back to the workspace it was on."
    pub end_line: Option<&'static str>,
}

/// Where a task runs, in a few words, for its line in the chain; "Runs in" says it in full.
pub fn target_short(target: &Target) -> &'static str {
    match target {
        Target::Workspace { .. } => "its Claude",
        Target::Conversation { .. } => "a new conversation",
        Target::New { .. } => "a new workspace",
    }
}

/// What the dialog decides with: the index and its cards, the workspaces and their lamps, the Claude you picked for
/// each card, and now.
pub struct World<'a> {
    pub work: &'a Snapshot,
    pub groups: &'a [Group],
    pub workspaces: &'a [Workspace],
    pub statuses: &'a HashMap<String, String>,
    pub chosen: &'a HashMap<String, ClaudeChoice>,
    pub now: i64,
}

/// The dialog's state.
#[derive(Clone, Debug, PartialEq)]
pub struct Editor {
    /// The workflow as it would be saved now.
    pub draft: Flow,
    /// The cards you picked on the board (none: a workflow you have is being changed).
    pub picks: Vec<String>,
    /// "Put them in": the workflow the picked cards go into (empty: a new one).
    pub into: String,
    /// "Starts at", as typed ("02:00").
    pub time: String,
    /// It starts at a time (off: it is kept until you start it).
    pub scheduled: bool,
    /// The step of the chain that is shown at the right.
    pub step: Step,
}

/// One task's row.
#[derive(Clone, Debug, PartialEq)]
pub struct TaskRow {
    pub subject: String,
    /// "SH-21309", or "PR #2415".
    pub name: String,
    pub title: String,
    pub kind: String,
    pub kind_label: String,
    /// Its place: the first can't go earlier, the last can't go later.
    pub first: bool,
    pub last: bool,
    /// Where it can run, and the one picked (a choice's value).
    pub choices: Vec<TargetChoice>,
    pub target: String,
    /// "With what waits on it: 2 remarks waiting, 1 question".
    pub waits: Option<String>,
    /// "Use these words for every “Fix what came back”": there once its words differ from its kind's.
    pub make_default: Option<String>,
    /// You edited its whole message.
    pub edited: bool,
    /// What its words' field shows while empty.
    pub asks: &'static str,
    /// The minutes it gets: its own, or the workflow's.
    pub minutes: u32,
    /// When it starts at the latest, while the workflow starts at a time: "02:00" for the first, "by 03:00" after.
    pub starts: Option<String>,
    /// Its line in the chain: "Answer the review, 45 min, its Claude".
    pub summary: String,
    /// It is the step shown at the right.
    pub shown: bool,
}

impl TaskRow {
    /// What the picked place to run is called.
    pub fn target_label(&self) -> &str {
        self.choices.iter().find(|choice| choice.value == self.target).map_or("", |choice| choice.label.as_str())
    }

    /// What opens and closes its whole message.
    pub fn message_words(&self, open: bool) -> &'static str {
        match (open, self.edited) {
            (true, _) => "Hide the message",
            (false, true) => "The whole message (edited)",
            (false, false) => "See the whole message",
        }
    }

    /// The row in one line, as a test reads it: "1. SH-21309 Export orders to the accounting CSV".
    pub fn said(&self, n: usize) -> String {
        format!("{n}. {} {}", self.name, self.title)
    }
}

impl Editor {
    /// The dialog as it opens on a workflow: a new one made of the picked cards, or one you have.
    pub fn of(flow: Flow, picks: Vec<String>) -> Editor {
        Editor::of_at(flow, picks, DEFAULT_TIME)
    }

    /// The same, where a workflow that has no start time yet is offered `usual`: the time you set for new workflows
    /// (Settings, Workflows; item 8.6).
    pub fn of_at(flow: Flow, picks: Vec<String>, usual: &str) -> Editor {
        let time = flow.start_at.map_or_else(|| usual.to_string(), clock_of);
        let scheduled = flow.state != "saved" || flow.start_at.is_some();
        Editor { into: flow.id.clone(), time, scheduled, draft: flow, picks, step: Step::Start }
    }

    /// The cards you picked, as the page has them now.
    pub fn picked<'a>(&self, groups: &'a [Group]) -> Vec<&'a Group> {
        self.picks.iter().filter_map(|id| groups.iter().find(|group| &group.id == id)).collect()
    }

    /// The workflows the picked cards can go into: yours that don't run, of their project.
    pub fn others<'a>(&self, flows: &'a [Flow]) -> Vec<&'a Flow> {
        if self.picks.is_empty() {
            return Vec::new();
        }
        flows.iter().filter(|flow| flow.state != "running" && flow.project_id == self.draft.project_id).collect()
    }

    /// What "Put them in" lists: "A new workflow", then each of the others with how it stands.
    pub fn into_choices(&self, world: &World) -> Vec<(String, String)> {
        let mut choices = vec![(String::new(), "A new workflow".to_string())];
        for flow in self.others(&world.work.flows) {
            choices.push((flow.id.clone(), format!("{}: {}", flow.name, flow_words(flow, world.now))));
        }
        choices
    }

    /// "Put them in": the picked cards become the tasks of a new workflow, or go after the tasks of one you have.
    pub fn put_into(&mut self, id: &str, world: &World) {
        let picked = self.picked(world.groups);
        let mine = &world.work.flow_kinds;
        let base = world.work.flows.iter().find(|flow| !id.is_empty() && flow.id == id);
        let next = match base {
            Some(base) => {
                add_tasks(base.clone(), &picked, world.work, world.workspaces, world.statuses, world.chosen, mine)
            }
            None => new_flow(&picked, world.work, world.workspaces, world.statuses, world.chosen, world.now, mine),
        };
        self.into = id.to_string();
        self.time = next.start_at.map_or_else(|| world.work.flow_defaults.time.clone(), clock_of);
        self.scheduled = next.start_at.is_some() || next.state != "saved";
        self.draft = next;
        // other tasks now: the chain is shown from its start
        self.step = Step::Start;
    }

    /// A card joins the workflow's tasks, after the ones it has (item 8.7: a task of your own made from the dialog).
    /// False when it is one of them already, isn't on the board, or has nowhere to run.
    pub fn add_card(&mut self, id: &str, world: &World) -> bool {
        let Some(card) = world.groups.iter().find(|group| group.id == id) else { return false };
        let before = self.draft.tasks.len();
        let mine = &world.work.flow_kinds;
        self.draft =
            add_tasks(self.draft.clone(), &[card], world.work, world.workspaces, world.statuses, world.chosen, mine);
        let joined = self.draft.tasks.len() > before;
        if joined {
            // the task that just joined is the one to set up
            self.step = Step::Task(id.to_string());
        }
        joined
    }

    /// Something of the workflow or of a task changed: every task's message is made again, but the ones you edited.
    pub fn change(&mut self, world: &World, change: impl FnOnce(&mut Flow)) {
        change(&mut self.draft);
        remake_in(&mut self.draft, world.work, world.groups);
    }

    /// One task changed.
    pub fn change_task(&mut self, at: usize, world: &World, change: impl FnOnce(&mut FlowTask)) {
        self.change(world, |flow| {
            if let Some(task) = flow.tasks.get_mut(at) {
                change(task);
            }
        });
    }

    /// Another kind of work for a task: its words come with it (your own words stay your own).
    pub fn set_kind(&mut self, at: usize, kind: &str, world: &World) {
        let words = (kind != "custom").then(|| kind_text(kind, &world.work.flow_kinds));
        self.change_task(at, world, |task| {
            task.kind = kind.to_string();
            if let Some(words) = words {
                task.instruction = words;
            }
        });
    }

    /// Your edits of a task's whole message are undone: it is the one made from the workflow and the task again.
    pub fn undo_edits(&mut self, at: usize, world: &World) {
        self.change_task(at, world, |task| task.edited = false);
    }

    /// A task goes one place earlier (`step` -1) or later (1).
    pub fn move_task(&mut self, at: usize, step: isize) {
        let to = at as isize + step;
        if at < self.draft.tasks.len() && to >= 0 && (to as usize) < self.draft.tasks.len() {
            let task = self.draft.tasks.remove(at);
            self.draft.tasks.insert(to as usize, task);
        }
    }

    /// A task is taken out of the workflow. Was it the step shown, the task that takes its place is shown, else the
    /// one before it, else the start.
    pub fn remove(&mut self, at: usize) {
        if at < self.draft.tasks.len() {
            let gone = self.draft.tasks.remove(at);
            if self.step == Step::Task(gone.subject) {
                let next = self.draft.tasks.get(at).or_else(|| self.draft.tasks.last());
                self.step = next.map_or(Step::Start, |task| Step::Task(task.subject.clone()));
            }
        }
    }

    /// The task shown at the right, by its place; none while the start is shown (or its task is no longer there).
    pub fn shown_task(&self) -> Option<usize> {
        match &self.step {
            Step::Start => None,
            Step::Task(subject) => self.draft.tasks.iter().position(|task| &task.subject == subject),
        }
    }

    /// When it starts, while it starts at a time and the time typed is one.
    fn timed_start(&self, now: i64) -> Option<i64> {
        self.start_at(now).filter(|_| self.scheduled)
    }

    /// The minutes its tasks can take together: each its own, or the workflow's.
    fn total_minutes(&self) -> u32 {
        self.draft.tasks.iter().map(|task| task.minutes.unwrap_or(self.draft.minutes)).sum()
    }

    /// The chain's two ends: how it starts with what every task gets, and when it is over at the latest.
    pub fn ends(&self, now: i64) -> Ends {
        let start = match (self.scheduled, self.start_at(now)) {
            (false, _) => "Kept until you start it".to_string(),
            (true, Some(at)) => format!("Starts {}", when_words(at, now)),
            (true, None) => "Starts at a time".to_string(),
        };
        let follows = if self.draft.live { " Live follows." } else { "" };
        let total = i64::from(self.total_minutes()) * 60_000;
        let end = match self.timed_start(now) {
            Some(at) => format!("Ends by {} at the latest", clock_of(at + total)),
            None => format!("Takes {} at the most", length_words(total)),
        };
        Ends {
            start,
            start_line: format!("Up to {} min a task.{follows}", self.draft.minutes),
            end,
            end_line: self.draft.live.then_some(LIVE_BACK),
        }
    }

    /// The plan in one line, at the dialog's foot: "5 tasks, one after another. At most 4 h 45 min, done by
    /// 06:45." Kept without a time it has no "done by". None without a task or without minutes.
    pub fn plan_words(&self, now: i64) -> Option<String> {
        let (count, total) = (self.draft.tasks.len(), self.total_minutes());
        if count == 0 || total == 0 {
            return None;
        }
        let tasks = if count == 1 { "1 task".to_string() } else { format!("{count} tasks, one after another") };
        let total = i64::from(total) * 60_000;
        Some(match self.timed_start(now) {
            Some(at) => format!("{tasks}. At most {}, done by {}.", length_words(total), clock_of(at + total)),
            None => format!("{tasks}. At most {}.", length_words(total)),
        })
    }

    /// When it would start: the next time the clock shows the time typed (none: that is no time of day).
    pub fn start_at(&self, now: i64) -> Option<i64> {
        next_at(&self.time, now)
    }

    /// What is still missing before it can be saved, in one line; none when nothing is.
    pub fn problem(&self, world: &World) -> Option<String> {
        let draft = &self.draft;
        if draft.tasks.is_empty() {
            return Some("Pick the tasks on the board first (x, or ⌘-click a card).".into());
        }
        if draft.project_id.is_empty() {
            return Some("These tasks belong to no project Enkidu knows.".into());
        }
        let strangers = other_project(draft, &self.picked(world.groups));
        if !strangers.is_empty() {
            let names: Vec<&str> =
                strangers.iter().map(|group| group.key.as_deref().unwrap_or(group.title.as_str())).collect();
            let are = if strangers.len() == 1 { "is" } else { "are" };
            return Some(format!(
                "{} {are} in another project: a workflow is one project's (its Live too).",
                names.join(", ")
            ));
        }
        if draft.name.trim().is_empty() {
            return Some("Give it a name.".into());
        }
        let every = draft.instruction.trim().is_empty();
        if let Some(silent) =
            draft.tasks.iter().find(|task| task.instruction.trim().is_empty() && every && !task.edited)
        {
            return Some(format!("Say what Claude does on {}.", task_name(&silent.subject)));
        }
        if self.scheduled && self.start_at(world.now).is_none() {
            return Some("Say when it starts, like 02:00.".into());
        }
        let within = |minutes: u32| (MINUTES.0..=MINUTES.1).contains(&minutes);
        if !within(draft.minutes) || draft.tasks.iter().any(|task| task.minutes.is_some_and(|own| !within(own))) {
            return Some(format!("A task gets between {} and {} minutes.", MINUTES.0, MINUTES.1));
        }
        None
    }

    /// The dialog's title: "Change Night run" for a workflow you have, else "A workflow for these tasks".
    pub fn title(&self) -> String {
        if !self.draft.id.is_empty() && self.into == self.draft.id && self.picks.is_empty() {
            format!("Change {}", self.draft.name)
        } else {
            "A workflow for these tasks".to_string()
        }
    }

    /// The line under the title, once the project is known.
    pub fn says(project: &str) -> String {
        format!("In {project}: one task after another, each with what you tell it here.")
    }

    /// What the white button says: "Start tomorrow at 02:00", or "Keep it".
    pub fn save_words(&self, now: i64) -> String {
        match self.start_at(now).filter(|_| self.scheduled) {
            Some(at) => format!("Start {}", when_words(at, now)),
            None => "Keep it".to_string(),
        }
    }

    /// Beside the time: "tomorrow at 02:00", or that it is kept until you start it.
    pub fn starts_words(&self, now: i64) -> String {
        match (self.scheduled, self.start_at(now)) {
            (false, _) => "kept until you start it".to_string(),
            (true, Some(at)) => when_words(at, now),
            (true, None) => String::new(),
        }
    }

    /// The workflow as the white button saves it: at its time, or kept without one.
    pub fn to_save(&self, now: i64) -> Flow {
        Flow { start_at: self.start_at(now).filter(|_| self.scheduled), ..self.draft.clone() }
    }

    /// The workflow as "Run now" saves it: starting now.
    pub fn to_run_now(&self, now: i64) -> Flow {
        Flow { start_at: Some(now), ..self.draft.clone() }
    }

    /// The message a task would have if you had not edited it (what "Undo my edits" brings back).
    pub fn remade(&self, at: usize, world: &World) -> String {
        let mut draft = self.draft.clone();
        if let Some(task) = draft.tasks.get_mut(at) {
            task.edited = false;
        }
        remake_in(&mut draft, world.work, world.groups);
        draft.tasks.get(at).map(|task| task.message.clone()).unwrap_or_default()
    }

    /// Every task as its row shows it.
    pub fn rows(&self, world: &World) -> Vec<TaskRow> {
        let count = self.draft.tasks.len();
        let mine = &world.work.flow_kinds;
        let shown = self.shown_task();
        // the latest each task starts: the first at the start, each next after the minutes of those before it
        let mut latest = self.timed_start(world.now);
        (self.draft.tasks.iter().enumerate())
            .map(|(at, task)| {
                let minutes = task.minutes.unwrap_or(self.draft.minutes);
                let starts = latest.map(|time| match at {
                    0 => clock_of(time),
                    _ => format!("by {}", clock_of(time)),
                });
                latest = latest.map(|time| time + i64::from(minutes) * 60_000);
                let group = world.groups.iter().find(|group| group.id == task.subject);
                let mut choices = group
                    .map(|group| target_choices(group, world.work, world.workspaces, world.statuses))
                    .unwrap_or_default();
                let target = target_value(&task.target);
                // where it runs now is always one of the choices, as it is: also when its card no longer offers it,
                // and a new workspace under the name the task has for it (its card may offer another by now)
                let label = match &task.target {
                    Target::New { name, .. } => format!("A new workspace, {name}"),
                    _ => "Its workspace".to_string(),
                };
                match choices.iter_mut().find(|choice| choice.value == target) {
                    Some(own) if matches!(task.target, Target::New { .. }) => {
                        (own.label, own.target) = (label, task.target.clone());
                    }
                    Some(_) => {}
                    None => {
                        choices.insert(0, TargetChoice { value: target.clone(), label, target: task.target.clone() })
                    }
                }
                let waits = group
                    .and_then(|group| summary(card_items(group, world.work)))
                    .map(|waits| format!("With what waits on it: {}", waits.text));
                let (words, usual) = (task.instruction.trim(), kind_text(&task.kind, mine));
                let changed = task.kind != "custom" && !words.is_empty() && words != usual.trim();
                TaskRow {
                    subject: task.subject.clone(),
                    name: task_name(&task.subject),
                    title: task.title.clone(),
                    kind: task.kind.clone(),
                    kind_label: kind_label(&task.kind).to_string(),
                    first: at == 0,
                    last: at + 1 == count,
                    choices,
                    target,
                    waits,
                    make_default: changed.then(|| format!("Use these words for every “{}”", kind_label(&task.kind))),
                    edited: task.edited,
                    asks: if task.kind == "custom" { OWN_WORDS_ASK } else { "" },
                    minutes,
                    starts,
                    summary: format!("{}, {minutes} min, {}", kind_label(&task.kind), target_short(&task.target)),
                    shown: shown == Some(at),
                }
            })
            .collect()
    }
}

/// What a saved workflow's toast says: "Night run starts tomorrow at 02:00" with "2 tasks, one after another, up to
/// 60 minutes each.", "Night run starts now", or "Kept Night run".
pub fn saved_toast(saved: &Flow, now: i64) -> (String, Option<String>) {
    let Some(at) = saved.start_at else { return (format!("Kept {}", saved.name), None) };
    let title = match at - now < 60_000 {
        true => format!("{} starts now", saved.name),
        false => format!("{} starts {}", saved.name, when_words(at, now)),
    };
    let count = saved.tasks.len();
    let tasks = if count == 1 { "task" } else { "tasks" };
    (title, Some(format!("{count} {tasks}, one after another, up to {} minutes each.", saved.minutes)))
}

/// What is said when your words for a kind were kept as its default.
pub const KIND_KEPT: (&str, &str) = ("Kept as the default words", "New tasks of that kind start from them.");

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::work::groups::build_groups;
    use crate::work::groups::tests::{NOW, statuses, ws};
    use crate::work::when::tests::{at, now};

    struct Sample {
        work: Snapshot,
        groups: Vec<Group>,
        workspaces: Vec<Workspace>,
        statuses: HashMap<String, String>,
        chosen: HashMap<String, ClaudeChoice>,
    }

    impl Sample {
        /// What these tests start from: the sample, with the workspace w2 on SH-21309's branch.
        fn new(change: impl FnOnce(&mut Snapshot)) -> Sample {
            let mut work = crate::work::sample();
            change(&mut work);
            let workspaces = vec![ws("w2", "sh-21309-export", "feature/SH-21309-export")];
            let statuses = statuses(&[]);
            let groups = build_groups(&work, &workspaces, &statuses, NOW);
            Sample { work, groups, workspaces, statuses, chosen: HashMap::new() }
        }

        fn world(&self) -> World<'_> {
            World {
                work: &self.work,
                groups: &self.groups,
                workspaces: &self.workspaces,
                statuses: &self.statuses,
                chosen: &self.chosen,
                now: now(),
            }
        }

        /// The dialog as "Workflow…" opens it for these cards.
        fn editor(&self, keys: &[&str]) -> Editor {
            let picks: Vec<String> = keys.iter().map(|key| format!("ticket:{key}")).collect();
            let cards: Vec<&Group> =
                picks.iter().filter_map(|id| self.groups.iter().find(|group| &group.id == id)).collect();
            let world = self.world();
            let flow = new_flow(
                &cards,
                world.work,
                world.workspaces,
                world.statuses,
                world.chosen,
                world.now,
                &world.work.flow_kinds,
            );
            Editor::of(flow, picks)
        }
    }

    fn row<'a>(rows: &'a [TaskRow], key: &str) -> &'a TaskRow {
        rows.iter().find(|row| row.name == key).unwrap_or_else(|| panic!("no row for {key}"))
    }

    /// Makes a workflow of the picked cards: each to its Claude or a new workspace, its message to read, at the time
    /// you set.
    #[test]
    fn a_workflow_of_the_picked_cards_each_task_set_up_on_its_own_at_the_time_you_set() {
        let sample = Sample::new(|_| {});
        let world = sample.world();
        let mut editor = sample.editor(&["SH-21309", "SH-21376"]);
        assert_eq!(editor.title(), "A workflow for these tasks");
        assert_eq!(Editor::says("shop"), "In shop: one task after another, each with what you tell it here.");
        assert_eq!((editor.time.as_str(), editor.scheduled, editor.into.as_str()), ("02:00", true, ""));
        assert_eq!(editor.save_words(world.now), "Start tomorrow at 02:00");
        assert_eq!(editor.starts_words(world.now), "tomorrow at 02:00");
        // the chain's two ends, and the plan in one line
        assert_eq!(
            editor.ends(world.now),
            Ends {
                start: "Starts tomorrow at 02:00".into(),
                start_line: "Up to 60 min a task. Live follows.".into(),
                end: "Ends by 04:00 at the latest".into(),
                end_line: Some(LIVE_BACK),
            }
        );
        assert_eq!(
            editor.plan_words(world.now).as_deref(),
            Some("2 tasks, one after another. At most 2 h, done by 04:00.")
        );
        // it opens on its start; each task says when it starts at the latest
        assert_eq!((editor.step.clone(), editor.shown_task()), (Step::Start, None));
        let chain: Vec<(Option<String>, bool)> =
            editor.rows(&world).into_iter().map(|row| (row.starts, row.shown)).collect();
        assert_eq!(chain, [(Some("02:00".to_string()), false), (Some("by 03:00".to_string()), false)]);
        assert_eq!(editor.problem(&world), None);
        // nothing to put them into: you have no workflow yet
        assert_eq!(editor.into_choices(&world), [(String::new(), "A new workflow".to_string())]);

        let rows = editor.rows(&world);
        assert_eq!(
            rows.iter().map(|row| row.subject.as_str()).collect::<Vec<_>>(),
            ["ticket:SH-21309", "ticket:SH-21376"]
        );
        assert_eq!(rows[0].said(1), "1. SH-21309 Export orders to the accounting CSV");
        // each to its Claude, or a new workspace
        assert_eq!(
            (row(&rows, "SH-21309").target.as_str(), row(&rows, "SH-21376").target.as_str()),
            ("workspace:w2", "new")
        );
        assert_eq!(row(&rows, "SH-21309").target_label(), "The Claude in sh-21309-export");
        assert_eq!(row(&rows, "SH-21376").target_label(), "A new workspace, sh-21376-bundle-units");
        // each its own kind of work, from its card: SH-21309 has review remarks, SH-21376 an open PR
        assert_eq!(
            (row(&rows, "SH-21309").kind.as_str(), row(&rows, "SH-21376").kind.as_str()),
            ("review", "continue")
        );
        assert_eq!(row(&rows, "SH-21309").kind_label, "Answer the review");
        let waits = row(&rows, "SH-21309").waits.clone().unwrap_or_default();
        assert!(waits.starts_with("With what waits on it: ") && waits.contains("remark"), "{waits}");
        assert_eq!(row(&rows, "SH-21376").waits, None, "nothing waits on SH-21376");
        assert_eq!((rows[0].first, rows[0].last, rows[1].first, rows[1].last), (true, false, false, true));
        assert_eq!(rows[0].make_default, None, "its words are its kind's");
        assert_eq!(rows[0].message_words(false), "See the whole message");
        assert_eq!(rows[0].message_words(true), "Hide the message");

        // its own words, its own minutes, and a new conversation in its workspace instead of the one there
        editor.change_task(0, &world, |task| {
            task.instruction = "Answer mira-k's remark only; leave the question.".into()
        });
        editor.change_task(0, &world, |task| task.minutes = Some(30));
        let conversation =
            row(&rows, "SH-21309").choices.iter().find(|choice| choice.value == "conversation:w2").unwrap();
        assert_eq!(conversation.label, "A new conversation in sh-21309-export");
        let target = conversation.target.clone();
        editor.change_task(0, &world, |task| task.target = target);
        // another kind: its words come with it
        editor.set_kind(1, "investigate", &world);
        assert!(editor.draft.tasks[1].instruction.contains("Investigate only"));
        // what every task is told, and the whole message: yours to read and change
        editor.change(&world, |flow| flow.instruction = "Use the small stack.".into());
        let message = editor.draft.tasks[1].message.clone();
        for said in ["Investigate only", "For every task of this workflow: Use the small stack.", "--ticket SH-21376"] {
            assert!(message.contains(said), "{said:?} is missing from:\n{message}");
        }
        editor.change_task(1, &world, |task| {
            task.message = format!("{message}\nStart with the bundle page.");
            task.edited = true;
        });
        assert_eq!(editor.rows(&world)[1].message_words(false), "The whole message (edited)");
        assert_eq!(editor.remade(1, &world), message, "what Undo my edits brings back");
        editor.time = "03:30".into();
        editor.change(&world, |flow| flow.minutes = 45);
        assert_eq!(editor.save_words(world.now), "Start tomorrow at 03:30");
        assert_eq!(
            editor.plan_words(world.now).as_deref(),
            Some("2 tasks, one after another. At most 1 h 15 min, done by 04:45.")
        );
        // a task's line in the chain: its kind of work, the minutes it gets, where it runs, in a few words
        let lines: Vec<String> = editor.rows(&world).into_iter().map(|row| row.summary).collect();
        assert!(lines.contains(&"Answer the review, 30 min, a new conversation".to_string()), "{lines:?}");
        assert!(lines.contains(&"Investigate only, 45 min, a new workspace".to_string()), "{lines:?}");

        let saved = editor.to_save(world.now);
        assert_eq!((saved.project_id.as_str(), saved.minutes, saved.live), ("p1", 45, true));
        assert_eq!(saved.instruction, "Use the small stack.");
        assert_eq!(saved.start_at, Some(at(12, 3, 30)));
        let first = &saved.tasks[0];
        assert_eq!((first.kind.as_str(), first.minutes), ("review", Some(30)));
        assert_eq!(first.instruction, "Answer mira-k's remark only; leave the question.");
        assert_eq!(first.target, Target::Conversation { workspace_id: "w2".into() });
        for said in ["Answer mira-k's remark only; leave the question.", "about 30 minutes", "mira-k left a remark"] {
            assert!(first.message.contains(said), "{said:?} is missing from:\n{}", first.message);
        }
        let second = &saved.tasks[1];
        assert_eq!((second.kind.as_str(), second.edited), ("investigate", true));
        assert!(matches!(&second.target, Target::New { branch, .. } if branch == "feature/SH-21376-bundle-units"));
        assert!(second.message.ends_with("Start with the bundle page."), "an edited message stays as you wrote it");
        // Run now starts it now, whatever its time
        assert_eq!(editor.to_run_now(world.now).start_at, Some(world.now));
        // and what the toast says of each
        assert_eq!(
            saved_toast(&saved, world.now),
            (
                "Night run starts tomorrow at 03:30".into(),
                Some("2 tasks, one after another, up to 45 minutes each.".into())
            )
        );
        assert_eq!(saved_toast(&editor.to_run_now(world.now), world.now).0, "Night run starts now");
        editor.scheduled = false;
        assert_eq!(
            (editor.save_words(world.now).as_str(), editor.starts_words(world.now).as_str()),
            ("Keep it", "kept until you start it")
        );
        assert_eq!(editor.to_save(world.now).start_at, None);
        assert_eq!(saved_toast(&editor.to_save(world.now), world.now), ("Kept Night run".into(), None));
    }

    /// Puts picked cards into a workflow you have, and keeps your words for a kind as its default.
    #[test]
    fn picked_cards_go_into_a_workflow_you_have_and_your_words_for_a_kind_can_be_its_default() {
        let tonight = |now: i64| Flow {
            id: "f2".into(),
            project_id: "p1".into(),
            name: "Tonight".into(),
            instruction: String::new(),
            start_at: Some(now + 3_600_000),
            minutes: 60,
            live: true,
            state: "scheduled".into(),
            current: None,
            live_before: None,
            tasks: vec![FlowTask {
                subject: "ticket:SH-21580".into(),
                title: "Outlet groups".into(),
                target: Target::New {
                    name: "sh-21580-outlet".into(),
                    branch: "feature/SH-21580-outlet".into(),
                    mode: "new".into(),
                },
                message: "SH-21580".into(),
                edited: false,
                kind: "start".into(),
                instruction: "Start it.".into(),
                minutes: None,
                state: "waiting".into(),
                workspace_id: None,
                started_at: None,
                ended_at: None,
                note: None,
                busy_at: None,
                nudged_at: None,
            }],
            created_at: 0,
            updated_at: 0,
        };
        let sample = Sample::new(|work| work.flows = vec![tonight(now())]);
        let world = sample.world();
        let mut editor = sample.editor(&["SH-21616"]);
        assert_eq!(
            editor.into_choices(&world),
            [
                (String::new(), "A new workflow".to_string()),
                ("f2".to_string(), "Tonight: Starts today at 16:00, 1 task".to_string())
            ]
        );
        editor.put_into("f2", &world);
        let rows = editor.rows(&world);
        let said: Vec<String> = rows.iter().map(|row| format!("{} {}", row.subject, row.kind)).collect();
        assert_eq!(said, ["ticket:SH-21580 start", "ticket:SH-21616 came_back"]);
        assert_eq!(
            (editor.draft.name.as_str(), editor.draft.id.as_str(), editor.into.as_str()),
            ("Tonight", "f2", "f2")
        );
        assert_eq!((editor.time.as_str(), editor.scheduled), ("16:00", true));
        // the picked cards are still picked: it is still "a workflow for these tasks"
        assert_eq!(editor.title(), "A workflow for these tasks");
        // the task it had keeps where it runs, though its card no longer offers that workspace
        assert_eq!(rows[0].target_label(), "A new workspace, sh-21580-outlet");
        // your words for "Fix what came back" can be kept for the next time a ticket comes back
        assert_eq!(rows[1].make_default, None);
        editor.change_task(1, &world, |task| {
            task.instruction = "Read QA's newest comment first, then fix only that.".into()
        });
        assert_eq!(
            editor.rows(&world)[1].make_default.as_deref(),
            Some("Use these words for every “Fix what came back”")
        );
        let saved = editor.to_save(world.now);
        assert_eq!(saved.id, "f2", "the same workflow, one task more");
        assert_eq!(
            saved.tasks.iter().map(|task| task.subject.as_str()).collect::<Vec<_>>(),
            ["ticket:SH-21580", "ticket:SH-21616"]
        );
        // back to a new workflow: only the picked card
        editor.put_into("", &world);
        assert_eq!(
            (editor.draft.id.as_str(), editor.draft.name.as_str(), editor.draft.tasks.len()),
            ("", "Night run", 1)
        );
        // with your words kept, a new task of that kind starts from them
        let mut mine = BTreeMap::new();
        mine.insert("came_back".to_string(), "Read QA's newest comment first, then fix only that.".to_string());
        let kept = Sample::new(|work| work.flow_kinds = mine);
        let editor = kept.editor(&["SH-21616"]);
        assert_eq!(editor.draft.tasks[0].instruction, "Read QA's newest comment first, then fix only that.");
        assert_eq!(editor.rows(&kept.world())[0].make_default, None);
    }

    /// A workflow you have, opened to change it; the order of its tasks; and each line on what is missing.
    #[test]
    fn says_what_is_missing_and_moves_and_removes_tasks() {
        let sample = Sample::new(|_| {});
        let world = sample.world();
        let mut editor = sample.editor(&["SH-21309", "SH-21376", "SH-21580"]);
        let order =
            |editor: &Editor| editor.draft.tasks.iter().map(|task| task_name(&task.subject)).collect::<Vec<_>>();
        editor.move_task(0, 1);
        assert_eq!(order(&editor), ["SH-21376", "SH-21309", "SH-21580"]);
        editor.move_task(2, -1);
        assert_eq!(order(&editor), ["SH-21376", "SH-21580", "SH-21309"]);
        editor.move_task(0, -1);
        editor.move_task(2, 1);
        assert_eq!(order(&editor), ["SH-21376", "SH-21580", "SH-21309"], "the ends stay where they are");
        // the step shown is a task by its card: it stays the one shown where it moves
        editor.step = Step::Task("ticket:SH-21580".into());
        assert_eq!(editor.shown_task(), Some(1));
        assert_eq!(editor.rows(&world).iter().map(|row| row.shown).collect::<Vec<_>>(), [false, true, false]);
        // taken out, the task that takes its place is shown
        editor.remove(1);
        assert_eq!(order(&editor), ["SH-21376", "SH-21309"]);
        assert_eq!((editor.step.clone(), editor.shown_task()), (Step::Task("ticket:SH-21309".into()), Some(1)));
        // the last one taken out: the one before it; another one taken out: the step stays; none left: the start
        let mut fewer = editor.clone();
        fewer.remove(1);
        assert_eq!(fewer.step, Step::Task("ticket:SH-21376".into()));
        let mut other = editor.clone();
        other.remove(0);
        assert_eq!((other.step.clone(), other.shown_task()), (Step::Task("ticket:SH-21309".into()), Some(0)));
        fewer.remove(0);
        assert_eq!((fewer.step.clone(), fewer.shown_task()), (Step::Start, None));
        editor.step = Step::Start;

        // what is missing, the first thing first
        let problem = |change: &dyn Fn(&mut Editor)| {
            let mut tried = editor.clone();
            change(&mut tried);
            tried.problem(&world)
        };
        assert_eq!(problem(&|_| {}), None);
        assert_eq!(
            problem(&|e| e.draft.tasks.clear()).as_deref(),
            Some("Pick the tasks on the board first (x, or ⌘-click a card).")
        );
        assert_eq!(
            problem(&|e| e.draft.project_id.clear()).as_deref(),
            Some("These tasks belong to no project Enkidu knows.")
        );
        assert_eq!(problem(&|e| e.draft.name = "  ".into()).as_deref(), Some("Give it a name."));
        assert_eq!(
            problem(&|e| e.draft.tasks[1].instruction.clear()).as_deref(),
            Some("Say what Claude does on SH-21309.")
        );
        // what every task is told is enough for a task without words of its own, and so is a message you wrote
        assert_eq!(
            problem(&|e| {
                e.draft.tasks[1].instruction.clear();
                e.draft.instruction = "Use the small stack.".into();
            }),
            None
        );
        assert_eq!(
            problem(&|e| {
                e.draft.tasks[1].instruction.clear();
                e.draft.tasks[1].edited = true;
            }),
            None
        );
        assert_eq!(problem(&|e| e.time = "soon".into()).as_deref(), Some("Say when it starts, like 02:00."));
        assert_eq!(
            problem(&|e| {
                e.time = "soon".into();
                e.scheduled = false;
            }),
            None,
            "kept without a time, it needs none"
        );
        let minutes = Some("A task gets between 5 and 480 minutes.");
        assert_eq!(problem(&|e| e.draft.minutes = 4).as_deref(), minutes);
        assert_eq!(problem(&|e| e.draft.minutes = 481).as_deref(), minutes);
        assert_eq!(problem(&|e| e.draft.tasks[0].minutes = Some(0)).as_deref(), minutes);
        assert_eq!(problem(&|e| e.draft.tasks[0].minutes = Some(480)), None);
        // a card of another project can't be in it
        let mut elsewhere = Sample::new(|_| {});
        for group in elsewhere.groups.iter_mut().filter(|group| group.key.as_deref() == Some("SH-21376")) {
            group.project_id = Some("p2".into());
        }
        assert_eq!(
            editor.problem(&elsewhere.world()).as_deref(),
            Some("SH-21376 is in another project: a workflow is one project's (its Live too).")
        );

        // a workflow you have, opened by Change: no picked cards, its own time, and its name in the title
        let mut kept = editor.to_save(world.now);
        (kept.id, kept.state) = ("f1".into(), "scheduled".into());
        let change = Editor::of(kept.clone(), Vec::new());
        assert_eq!(change.title(), "Change Night run");
        assert_eq!((change.time.as_str(), change.scheduled, change.into.as_str()), ("02:00", true, "f1"));
        assert!(change.others(&[kept.clone()]).is_empty(), "nothing is picked: nothing to put anywhere");
        // one kept without a time opens without one
        let waiting = Editor::of(Flow { start_at: None, state: "saved".into(), ..kept.clone() }, Vec::new());
        assert_eq!((waiting.time.as_str(), waiting.scheduled), ("02:00", false));
        assert_eq!(waiting.save_words(world.now), "Keep it");
        // one that ran last night opens on its own time, for the next night
        let ran = Editor::of(Flow { start_at: Some(at(11, 3, 15)), state: "done".into(), ..kept }, Vec::new());
        assert_eq!((ran.time.as_str(), ran.scheduled), ("03:15", true));
        assert_eq!(ran.save_words(world.now), "Start tomorrow at 03:15");
        // the plan of several tasks, and of one; kept without a time it has no end to name
        let mut long = editor.clone();
        long.draft.minutes = 75;
        assert_eq!(
            long.plan_words(world.now).as_deref(),
            Some("2 tasks, one after another. At most 2 h 30 min, done by 04:30.")
        );
        long.draft.tasks.truncate(1);
        assert_eq!(long.plan_words(world.now).as_deref(), Some("1 task. At most 1 h 15 min, done by 03:15."));
        (long.scheduled, long.draft.live) = (false, false);
        assert_eq!(long.plan_words(world.now).as_deref(), Some("1 task. At most 1 h 15 min."));
        assert_eq!(
            long.ends(world.now),
            Ends {
                start: "Kept until you start it".into(),
                start_line: "Up to 75 min a task.".into(),
                end: "Takes 1 h 15 min at the most".into(),
                end_line: None,
            }
        );
        assert_eq!(long.rows(&world)[0].starts, None, "no time: nothing to say when a task starts");
        long.draft.tasks.clear();
        assert_eq!(long.plan_words(world.now), None);
        // a time typed that is none
        let mut soon = editor.clone();
        soon.time = "soon".into();
        assert_eq!(soon.ends(world.now).start, "Starts at a time");
    }
}
