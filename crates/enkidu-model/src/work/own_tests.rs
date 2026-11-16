//! Items 8.5 and 8.7: a task of your own through the whole model of the Work page. It is a card like a ticket's (in
//! the column its state names, with a face and a card of its own), its message to Claude says what it is and what you
//! wrote about it, a new workspace is named for it, and a workflow can run it.

use std::collections::{BTreeMap, HashMap};

use enkidu_core::work::flows::Target;
use enkidu_core::work::model::{Board, BoardPick, Event, OwnTask, Snapshot};

use super::board::{Category, Statuses, build_board, own_state};
use super::card::{CardModel, Tab};
use super::claudes::{BranchMode, SendTarget, card_claudes, card_subjects, task_target, ticket_workspace};
use super::compose::{Draft, note_asks, note_subject, task_text, with_it};
use super::flows::{FlowWords, OWN_TASK_WORDS, kind_for, task_for, task_name};
use super::groups::tests::{NOW, statuses, workspaces, ws};
use super::groups::{Group, GroupLamp, board_cards, build_groups, matches};
use super::page::{CardFace, CardHead};
use super::sample;

fn own(id: i64, project: &str, title: &str, notes: &str, state: &str) -> OwnTask {
    OwnTask {
        id,
        project_id: project.into(),
        title: title.into(),
        notes: notes.into(),
        state: state.into(),
        created_at: NOW - 3 * 3_600_000,
        updated_at: NOW - 60_000,
        project: if project == "p1" { "shop".into() } else { "enkidu".into() },
    }
}

/// The sample index with three tasks of your own: one to do (with notes), one in progress, one done.
fn work() -> Snapshot {
    let mut work = sample();
    work.own_tasks = vec![
        own(
            1,
            "p1",
            "Try the small Docker setup on the CI runner",
            "The runner has 4 GB.\nKeep the full stack as it is.",
            "todo",
        ),
        own(2, "p1", "Clean up the fixtures of the stock tests", "", "doing"),
        own(3, "p2", "Ask Jonas for the QA data set", "", "done"),
    ];
    work
}

fn groups(work: &Snapshot) -> Vec<Group> {
    build_groups(work, &workspaces(), &statuses(&[("w1", "working"), ("w2", "idle")]), NOW)
}

fn card<'a>(groups: &'a [Group], id: &str) -> &'a Group {
    groups.iter().find(|group| group.id == id).unwrap_or_else(|| panic!("no card {id}"))
}

#[test]
fn a_task_of_your_own_is_a_card_in_the_column_its_state_names() {
    let work = work();
    let all = groups(&work);
    let (todo, doing, done) = (card(&all, "own:1"), card(&all, "own:2"), card(&all, "own:3"));
    assert_eq!((todo.own, todo.key.as_deref(), todo.ticket), (Some(0), None, None));
    assert_eq!(todo.title, "Try the small Docker setup on the CI runner");
    assert_eq!((todo.project_id.as_deref(), done.project_id.as_deref()), (Some("p1"), Some("p2")));
    // nothing waits on it for anyone else: its lamp only says whether it is done
    assert_eq!((todo.lamp, doing.lamp, done.lamp), (GroupLamp::Off, GroupLamp::Off, GroupLamp::Green));
    assert_eq!((todo.waiting, todo.latest), (0, NOW - 60_000));
    // the tickets' cards are as they were without them
    let before = build_groups(&sample(), &workspaces(), &statuses(&[("w1", "working"), ("w2", "idle")]), NOW);
    let tickets: Vec<&Group> = all.iter().filter(|group| group.own.is_none()).collect();
    assert_eq!(tickets.len(), before.len());
    assert!(tickets.iter().zip(&before).all(|(now, was)| *now == was));

    // the board: what is still to do in a column of its own, the rest where Jira's tickets of that state stand
    let named = Statuses::of(&all, &work);
    let board = build_board(all.clone(), &named, &work);
    let column_of = |id: &str| {
        let column = board.iter().find(|column| column.cards.iter().any(|card| card.id == id)).expect("on the board");
        (column.name.clone(), column.category)
    };
    assert_eq!(column_of("own:1"), ("To do".to_string(), Category::New));
    assert_eq!(column_of("own:2"), ("In Progress".to_string(), Category::Indeterminate), "as Jira names it");
    assert_eq!(column_of("own:3"), ("Done".to_string(), Category::Done));
    let in_progress = board.iter().find(|column| column.name == "In Progress").unwrap();
    assert!(in_progress.cards.iter().any(|card| card.key.is_some()), "it shares the column with Jira's tickets");
    // "To do" stands among the columns of what is still to do, before anything in progress
    let names: Vec<&str> = board.iter().map(|column| column.name.as_str()).collect();
    let at = |name: &str| names.iter().position(|column| *column == name).unwrap();
    assert!(at("To do") < at("In Progress") && at("In Progress") < at("Done"), "{names:?}");
    assert_eq!(own_state("todo"), (Category::New, "To do"));
    assert_eq!(own_state("doing"), (Category::Indeterminate, "In progress"));
    assert_eq!(own_state("done"), (Category::Done, "Done"));

    // a Jira board you picked shows only its tickets, and your own tasks: they are yours, not Jira's
    let pick = BoardPick {
        board: Board { id: "7".into(), name: "Sprint".into(), kind: "scrum".into(), project_key: Some("SH".into()) },
        keys: Some(vec!["SH-21309".into()]),
        read_at: Some(NOW),
    };
    let shown: Vec<String> = board_cards(all.clone(), Some(&pick)).into_iter().map(|group| group.id).collect();
    assert_eq!(shown.iter().filter(|id| id.starts_with("own:")).count(), 3, "{shown:?}");
    assert_eq!(shown.iter().filter(|id| id.starts_with("ticket:")).count(), 1, "{shown:?}");

    // the filter finds it by its title, by what you wrote about it, and by its state
    let found = |words: &str| -> Vec<&str> {
        all.iter().filter(|group| group.own.is_some() && matches(group, &work, words)).map(|g| g.id.as_str()).collect()
    };
    assert_eq!(found("docker"), ["own:1"]);
    assert_eq!(found("4 GB"), ["own:1"]);
    assert_eq!(found("in progress"), ["own:2"]);
    assert!(found("warranty").is_empty());
}

#[test]
fn its_face_and_its_card_say_it_is_yours() {
    let mut work = work();
    work.events.insert(
        0,
        Event {
            subject: "own:1".into(),
            at: NOW - 3 * 3_600_000,
            kind: "own".into(),
            text: "You made this task".into(),
        },
    );
    let all = groups(&work);
    let list = workspaces();
    let none = HashMap::new();
    let face = CardFace::of(card(&all, "own:1"), &work, &list, &none, None, &HashMap::new(), NOW);
    assert_eq!(face.head, CardHead::Own { number: 1, project: "shop".into() });
    assert_eq!((face.name(), face.title.as_str()), ("own:1", "Try the small Docker setup on the CI runner"));
    assert!(
        face.prs.is_empty() && face.waiting == 0 && !face.blocked && face.claude.is_none() && face.report.is_none()
    );

    let model = CardModel::of(card(&all, "own:1"), &work, NOW);
    assert_eq!(model.head, CardHead::Own { number: 1, project: "shop".into() });
    let facts = model.own.as_ref().expect("its facts");
    assert_eq!((facts.number, facts.project.as_str(), facts.state.as_str()), (1, "shop", "todo"));
    assert_eq!(
        (facts.notes.as_str(), facts.made.as_str()),
        ("The runner has 4 GB.\nKeep the full stack as it is.", "3 h ago")
    );
    // nothing of Jira's or GitHub's
    assert!(model.jira.is_none() && model.jira_url.is_none() && model.prs.is_empty() && !model.hidden);
    // what you wrote is its story, and its timeline is its own
    assert_eq!(model.tabs, [Tab::Overview, Tab::Activity]);
    assert_eq!(model.timeline, [("3 h ago".to_string(), "You made this task".to_string())]);
    assert!(model.story);
    // one without notes and with nothing told yet has no story: the facts alone
    let bare = CardModel::of(card(&all, "own:2"), &work, NOW);
    assert!(bare.tabs.is_empty() && !bare.story && bare.own.is_some());
    // a ticket's card has no such facts
    let ticket = all.iter().find(|group| group.key.is_some()).unwrap();
    assert!(CardModel::of(ticket, &work, NOW).own.is_none());
    // hidden like any card
    work.hidden.push("own:2".into());
    assert!(CardModel::of(card(&all, "own:2"), &work, NOW).hidden);
}

#[test]
fn the_message_to_claude_says_what_it_is_and_what_you_wrote() {
    let work = work();
    let all = groups(&work);
    let (todo, doing) = (card(&all, "own:1"), card(&all, "own:2"));
    let text = task_text(todo, &work, &Draft { note: "Start with the compose file.".into(), ..Draft::default() });
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines[..6],
        [
            "A task of mine (not in Jira): Try the small Docker setup on the CI runner",
            "",
            "The runner has 4 GB.",
            "Keep the full stack as it is.",
            "",
            "Start with the compose file.",
        ],
        "{text}"
    );
    assert!(!text.contains("Ticket:") && !text.contains("PR #"), "{text}");
    // one without notes: its title, then your words
    let bare = task_text(doing, &work, &Draft { note: "Only the factories.".into(), ..Draft::default() });
    assert!(
        bare.starts_with(
            "A task of mine (not in Jira): Clean up the fixtures of the stock tests\n\nOnly the factories."
        ),
        "{bare}"
    );
    assert_eq!(with_it(todo, &work, &Draft::default()), [("your task".to_string(), false)]);
    assert_eq!(note_asks(todo, &work), "What should Claude do about this task?");
    // its timeline and its links go under its card's own name
    assert_eq!(note_subject(todo, &work), "own:1");
    assert_eq!(card_subjects(todo, &work), ["own:1"]);
}

#[test]
fn a_new_workspace_for_it_is_named_for_the_task_and_one_on_its_branch_is_its_claude() {
    let work = work();
    let all = groups(&work);
    let todo = card(&all, "own:1");
    let (name, branch) = ticket_workspace("task-1", &todo.title);
    assert!(name.starts_with("task-1-try-the-small-docker") && branch == format!("feature/{name}"), "{name} {branch}");
    // no Claude has worked on it: a new workspace, on a new branch named for the task
    let none = HashMap::new();
    let target = task_target(todo, &work, &workspaces(), &none, None);
    assert_eq!(
        target,
        SendTarget::New { project_id: "p1".into(), name: name.clone(), branch: branch.clone(), mode: BranchMode::New }
    );
    // a workspace on that branch is the card's Claude, also before any hook linked its conversation
    let mut list = workspaces();
    list.push(ws("w9", &name, &branch));
    let claudes = card_claudes(todo, &work, &list, &none, None, None);
    let picked = claudes.picked().expect("the workspace on its branch");
    assert_eq!(picked.workspace.as_ref().map(|w| w.id.as_str()), Some("w9"));
    assert!(matches!(task_target(todo, &work, &list, &none, None), SendTarget::Workspace { .. }));
    // a task in a project that is not there any more has nowhere to start
    let mut lost = work.clone();
    lost.own_tasks[0].project_id = String::new();
    let all = groups(&lost);
    let orphan = Group { project_id: None, ..card(&all, "own:1").clone() };
    assert_eq!(task_target(&orphan, &lost, &workspaces(), &none, None), SendTarget::None);
}

#[test]
fn a_workflow_runs_it_in_your_own_words_and_claude_reports_on_it_by_its_number() {
    let work = work();
    let all = groups(&work);
    let todo = card(&all, "own:1");
    let none = HashMap::new();
    assert_eq!(kind_for(todo, &work, &workspaces(), &none), "custom");
    assert_eq!(task_name("own:1"), "Task 1");
    let flow =
        FlowWords { name: "Night run".into(), instruction: "Use the small stack.".into(), minutes: 60, live: false };
    let target = Target::New { name: "task-1-x".into(), branch: "feature/task-1-x".into(), mode: "new".into() };
    // your own default words for the kinds speak of Jira's tickets: they are not what a task of your own starts from
    let mine = BTreeMap::from([("custom".to_string(), "Never used for a new task.".to_string())]);
    let task = task_for(todo, &work, target, &flow, &workspaces(), &none, &mine);
    assert_eq!(
        (task.subject.as_str(), task.kind.as_str(), task.instruction.as_str()),
        ("own:1", "custom", OWN_TASK_WORDS)
    );
    assert_eq!(task.title, "Try the small Docker setup on the CI runner");
    let lines: Vec<&str> = task.message.lines().collect();
    assert_eq!(
        lines[..7],
        [
            "Task 1: Try the small Docker setup on the CI runner",
            "It is a task of mine, not in Jira.",
            "",
            "The runner has 4 GB.",
            "Keep the full stack as it is.",
            "",
            OWN_TASK_WORDS,
        ],
        "{}",
        task.message
    );
    assert!(task.message.contains("For every task of this workflow: Use the small stack."), "{}", task.message);
    assert!(task.message.contains("enkidu report done|review|blocked --task 1 "), "{}", task.message);
    assert!(!task.message.contains("Ticket:"), "{}", task.message);
}

/// Item 8.7: a
/// task of your own, made while the workflow dialog is open, joins the workflow's tasks after the ones it has, in
/// your own words, to a new workspace named for it.
#[test]
fn a_task_of_your_own_made_from_the_workflow_dialog_joins_its_tasks() {
    use super::flow_editor::{Editor, World};
    use super::flows::new_flow;
    let mut work = sample();
    let list = workspaces();
    let none = HashMap::new();
    let chosen = HashMap::new();
    let before = build_groups(&work, &list, &none, NOW);
    let ticket = before.iter().find(|group| group.key.as_deref() == Some("SH-21309")).expect("the sample's ticket");
    let flow = new_flow(&[ticket], &work, &list, &none, &chosen, NOW, &work.flow_kinds);
    let mut editor = Editor::of(flow, vec![ticket.id.clone()]);
    assert_eq!(editor.draft.tasks.len(), 1);
    let project = editor.draft.project_id.clone();
    // the task is made (the dialog asks its project: the workflow's), and the board has its card
    work.own_tasks.push(own(4, &project, "Try the small Docker setup", "The runner has 4 GB.", "todo"));
    let groups = build_groups(&work, &list, &none, NOW);
    let world = World { work: &work, groups: &groups, workspaces: &list, statuses: &none, chosen: &chosen, now: NOW };
    assert_eq!(editor.shown_task(), None, "the dialog shows its start");
    assert!(editor.add_card("own:4", &world));
    // the task that just joined is the step shown, to set it up
    assert_eq!(editor.shown_task(), Some(1));
    assert!(!editor.add_card("own:4", &world), "it is one of the tasks already");
    let tasks: Vec<(&str, &str)> = editor.draft.tasks.iter().map(|t| (t.subject.as_str(), t.kind.as_str())).collect();
    assert_eq!(tasks.len(), 2);
    assert_eq!(tasks[1], ("own:4", "custom"), "after the ticket, in your own words");
    let added = &editor.draft.tasks[1];
    assert_eq!(added.instruction, OWN_TASK_WORDS);
    assert!(
        added.message.starts_with("Task 4: Try the small Docker setup\nIt is a task of mine, not in Jira."),
        "{}",
        added.message
    );
    assert!(
        matches!(&added.target, Target::New { name, .. } if name.starts_with("task-4-try-the-small")),
        "{:?}",
        added.target
    );
    // its row says what it is, and the dialog has nothing to complain about
    let rows = editor.rows(&world);
    assert_eq!((rows[1].name.as_str(), rows[1].title.as_str()), ("Task 4", "Try the small Docker setup"));
    assert_eq!(editor.problem(&world), None);
    // it is there once: adding it again, or a card that isn't on the board, adds nothing
    assert!(!editor.add_card("own:4", &world) && !editor.add_card("own:99", &world));
    assert_eq!(editor.draft.tasks.len(), 2);
    // the workflow as it would be saved passes the core's own check
    assert!(enkidu_core::work::flows::check(&editor.to_save(NOW)).is_ok());
}

/// Item 8.6: a new workflow starts with what Settings, Workflows says (the minutes a task gets, the time it starts
/// at), and each task of a kind from your own words for that kind.
#[test]
fn a_new_workflow_starts_with_what_settings_say() {
    use super::flows::{kind_text, new_flow, next_at};
    use enkidu_core::work::flows::FlowDefaults;
    let mut work = sample();
    let list = workspaces();
    let (none, chosen) = (HashMap::new(), HashMap::new());
    let all = build_groups(&work, &list, &none, NOW);
    let ticket = all.iter().find(|group| group.key.as_deref() == Some("SH-21309")).expect("the sample's ticket");
    // as Enkidu has it: 60 minutes, tonight at two
    let usual = new_flow(&[ticket], &work, &list, &none, &chosen, NOW, &work.flow_kinds);
    assert_eq!((usual.minutes, usual.start_at), (60, next_at("02:00", NOW)));
    // as you set it
    work.flow_defaults = FlowDefaults { minutes: 45, time: "23:30".into() };
    let kind = usual.tasks[0].kind.clone();
    work.flow_kinds.insert(kind.clone(), "Read the ticket, then ask me before you change anything.".into());
    let yours = new_flow(&[ticket], &work, &list, &none, &chosen, NOW, &work.flow_kinds);
    assert_eq!((yours.minutes, yours.start_at), (45, next_at("23:30", NOW)));
    assert_eq!(yours.tasks[0].instruction, kind_text(&kind, &work.flow_kinds));
    assert_eq!(yours.tasks[0].instruction, "Read the ticket, then ask me before you change anything.");
    assert!(yours.tasks[0].message.contains("Read the ticket, then ask me before you change anything."));
    // a time nobody can read (a database changed by hand) falls back to two at night
    work.flow_defaults.time = "soon".into();
    let odd = new_flow(&[ticket], &work, &list, &none, &chosen, NOW, &work.flow_kinds);
    assert_eq!(odd.start_at, next_at("02:00", NOW));
}
