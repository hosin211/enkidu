//! What the Work page shows, worked out from the index and your choices: the mark on the Work tab, whether anything was
//! read yet, the board as it is shown (the Jira board you picked, the cards you hid, Needs you, the filter), and what
//! each card says.

use std::collections::{HashMap, HashSet};

use enkidu_core::db::Workspace;
use enkidu_core::work::flows::Flow;
use enkidu_core::work::model::{BoardPick, Pr, Snapshot};

use super::board::{
    Category, Direction, PrPhrase, StatusLook, Statuses, TypeKind, between_in, build_board, pr_phrase, priority_level,
    step_in, type_kind,
};
use super::claudes::{ClaudeChoice, card_claudes};
use super::flows::{FlowMark, flow_mark, flow_of_cards};
use super::groups::{Group, board_cards, matches};
use super::pr::summary;
use super::sources::ago;

/// What the Work tab in the projects row carries beside its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabMark {
    /// A source can't be read: a red mark.
    Broken,
    /// What waits on you on the cards on the board: how many items, on how many PRs. A red count.
    Waiting { items: usize, prs: usize },
    /// Nothing: only its name.
    Quiet,
}

impl TabMark {
    /// What the tab says when pointed at.
    pub fn words(self) -> String {
        match self {
            TabMark::Broken => "Work: a source can't be read".into(),
            TabMark::Waiting { items, prs: 0 } => format!("Work: {items} waiting on you"),
            TabMark::Waiting { items, prs: 1 } => format!("Work: {items} waiting on you on 1 PR"),
            TabMark::Waiting { items, prs } => format!("Work: {items} waiting on you on {prs} PRs"),
            TabMark::Quiet => "Work".into(),
        }
    }
}

/// The Work tab's mark: a source that can't be read comes first; else what people wait on you for, on every card
/// you didn't hide (a card you hid doesn't ask for you here), whatever Jira board is picked.
pub fn tab_mark(work: Option<&Snapshot>, groups: &[Group]) -> TabMark {
    let Some(work) = work else { return TabMark::Quiet };
    if work.health.iter().any(|health| health.state == "error") {
        return TabMark::Broken;
    }
    let hidden: HashSet<&str> = work.hidden.iter().map(String::as_str).collect();
    let mut items: HashSet<&str> = HashSet::new();
    let mut prs: HashSet<&str> = HashSet::new();
    for group in groups.iter().filter(|group| !hidden.contains(group.id.as_str())) {
        for item in group.all_items(work).filter(|item| !item.bot && item.state == "open") {
            if items.insert(&item.id)
                && let Some(pr) = item.pr.as_deref().filter(|pr| !pr.is_empty())
            {
                prs.insert(pr);
            }
        }
    }
    if items.is_empty() { TabMark::Quiet } else { TabMark::Waiting { items: items.len(), prs: prs.len() } }
}

/// Nothing was read yet (no index, or no source has a line): the page says what is connected and offers to check.
pub fn first_run(work: Option<&Snapshot>) -> bool {
    work.is_none_or(|work| work.health.is_empty())
}

/// Your choices on the page.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Choices {
    /// Only the cards with something waiting on you.
    pub needs_you: bool,
    /// The cards you hid, in place of the board.
    pub show_hidden: bool,
    /// The filter's text.
    pub query: String,
}

/// How many PRs a card lists before "and 1 more".
pub const PRS_ON_CARD: usize = 2;

/// A PR's state as GitHub draws it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrState {
    Open,
    Draft,
    Merged,
    Closed,
}

impl PrState {
    pub fn of(pr: &Pr) -> PrState {
        match pr.state.as_str() {
            "MERGED" => PrState::Merged,
            "CLOSED" => PrState::Closed,
            _ if pr.draft => PrState::Draft,
            _ => PrState::Open,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            PrState::Open => "Open",
            PrState::Draft => "Draft",
            PrState::Merged => "Merged",
            PrState::Closed => "Closed",
        }
    }
}

/// The first line of a card: what it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CardHead {
    /// A ticket: its type as Jira names it and as the page draws it, its key, and a priority worth a mark.
    Ticket { kind: TypeKind, type_name: Option<String>, key: String, priority: Option<(i8, String)> },
    /// A PR that names no ticket: its state and `repo#number`.
    Pr { state: PrState, name: String },
    /// A task of your own (item 8.5): its number, and the name of the project it is for.
    Own { number: i64, project: String },
}

/// A PR on a card: its state's glyph, its number, and its state in a word or two.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrLine {
    pub state: PrState,
    pub number: i64,
    pub phrase: PrPhrase,
}

/// How Claude's latest report says the task ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReportMark {
    /// done, review or blocked.
    pub state: String,
    /// "Done", "Review", "Stuck".
    pub label: &'static str,
    /// How many screenshots it has.
    pub images: usize,
    /// When pointed at: "Finished, nothing needs you, 2 h ago: All 14 tests pass".
    pub words: String,
}

/// What a report's state says, on a card and in full.
pub fn report_words(state: &str) -> (&'static str, &'static str) {
    match state {
        "done" => ("Done", "Finished, nothing needs you"),
        "review" => ("Review", "Finished, wants your review"),
        "update" => ("Update", "Update"),
        _ => ("Stuck", "Stuck, needs you"),
    }
}

/// What a card says, at a glance: what the ticket is, how its PRs stand in a word or two, whether Claude is on it, and
/// how much waits on you.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CardFace {
    /// The group's id: `ticket:SH-1` or `pr:owner/repo#N`.
    pub id: String,
    pub head: CardHead,
    pub title: String,
    /// What people wait on you for: how many items, whether something blocks (a red badge, else blue), and the
    /// words when pointed at ("Needs you: checks failing, 2 remarks waiting").
    pub waiting: usize,
    pub blocked: bool,
    pub needs: String,
    /// Its PRs, the first two; a PR without a ticket has one line with its state alone.
    pub prs: Vec<PrLine>,
    /// How many more PRs it has ("and 1 more").
    pub more: usize,
    /// The Claude the card goes to, when it is in an Enkidu workspace: the workspace's name and its lamp.
    pub claude: Option<(String, String)>,
    pub report: Option<ReportMark>,
    /// The workflow it is in, when that one is due or runs: its start time, "Next" or "Now".
    pub flow: Option<FlowMark>,
    /// Its waiting items come from a source that can't be read now: dimmed.
    pub stale: bool,
}

impl CardFace {
    /// The key when it has one, else its id (how the tests name a card).
    pub fn name(&self) -> &str {
        match &self.head {
            CardHead::Ticket { key, .. } => key,
            CardHead::Pr { .. } | CardHead::Own { .. } => &self.id,
        }
    }

    /// The face of a card. `chosen` is the Claude you picked for it, if you did; `flows` the cards in a workflow that
    /// is due or runs (`flow_of_cards`).
    pub fn of(
        group: &Group,
        work: &Snapshot,
        workspaces: &[Workspace],
        statuses: &HashMap<String, String>,
        chosen: Option<&ClaudeChoice>,
        flows: &HashMap<&str, &Flow>,
        now: i64,
    ) -> CardFace {
        let ticket = group.ticket(work);
        let lone = match &group.key {
            None => group.prs.first().map(|view| view.of(work)),
            Some(_) => None,
        };
        let head = match (group.own(work), lone, &group.key) {
            (Some(task), _, _) => CardHead::Own { number: task.id, project: task.project.clone() },
            (None, Some(pr), _) => {
                let repo = pr.repo.rsplit('/').next().unwrap_or(&pr.repo);
                CardHead::Pr { state: PrState::of(pr), name: format!("{repo}#{}", pr.number) }
            }
            (None, None, key) => {
                let issue_type = ticket.and_then(|ticket| ticket.issue_type.clone());
                let priority = ticket.and_then(|ticket| ticket.priority.clone());
                CardHead::Ticket {
                    kind: type_kind(issue_type.as_deref()),
                    type_name: issue_type,
                    key: key.clone().unwrap_or_default(),
                    priority: priority_level(priority.as_deref()).zip(priority),
                }
            }
        };
        let needs = match summary(group.all_items(work)) {
            Some(said) => format!("Needs you: {}", said.text),
            None => format!("Needs you: {} waiting on you", group.waiting),
        };
        let line = |pr: &Pr| PrLine { state: PrState::of(pr), number: pr.number, phrase: pr_phrase(pr) };
        let prs: Vec<PrLine> = group.prs.iter().take(PRS_ON_CARD).map(|view| line(view.of(work))).collect();
        // the Claude the card goes to (yours, else the likeliest), when it is in an Enkidu workspace
        let claudes = card_claudes(group, work, workspaces, statuses, chosen, None);
        let claude = claudes.picked().and_then(|option| {
            let workspace = option.workspace.as_ref()?;
            Some((workspace.name.clone(), option.status.clone().unwrap_or_else(|| "idle".into())))
        });
        // how the task ended (an update along the way says nothing about that)
        let report = group.reports(work).find(|report| report.state != "update").map(|report| {
            let (label, words) = report_words(&report.state);
            let first = report.text.trim().lines().next().unwrap_or("").trim_start_matches('#').trim_start();
            ReportMark {
                state: report.state.clone(),
                label,
                images: report.images.len(),
                words: format!("{words}, {}: {first}", ago(Some(report.at), now)),
            }
        });
        CardFace {
            id: group.id.clone(),
            head,
            title: group.title.clone(),
            waiting: group.waiting,
            blocked: group.lamp == super::groups::GroupLamp::Red,
            needs,
            more: group.prs.len().saturating_sub(PRS_ON_CARD),
            prs,
            claude,
            report,
            flow: flow_mark(flows, &group.id, now),
            stale: group.stale,
        }
    }
}

/// A column of the board as it is drawn: a status, and the faces of its cards.
#[derive(Clone, Debug, PartialEq)]
pub struct ColumnView {
    /// The status in lower case.
    pub id: String,
    /// As Jira names it.
    pub name: String,
    pub category: Category,
    pub look: StatusLook,
    pub cards: Vec<CardFace>,
}

/// What the page says in the board's place when no card is shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Quiet {
    /// The filter matches nothing: "Nothing here matches “stock”."
    NoMatch(String),
    /// Nothing to show, said in a line: "You hid every task."
    AllQuiet(String),
}

impl Quiet {
    pub fn words(&self) -> &str {
        match self {
            Quiet::NoMatch(words) | Quiet::AllQuiet(words) => words,
        }
    }
}

/// The board as the page shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BoardView {
    pub columns: Vec<ColumnView>,
    /// The ids of every card of the Jira board you picked, the ones you hid too: what a card you open or picked
    /// must be one of.
    pub cards: Vec<String>,
    /// How many cards you hid, and how many items wait on you there (a red dot on Hidden).
    pub away: usize,
    pub away_waiting: usize,
    /// How many cards on the board have something waiting on you.
    pub needs_count: usize,
    /// Only those are shown (your choice, while there are some and you aren't looking at the hidden ones).
    pub needs_you: bool,
    /// The cards you hid are shown, in place of the board (your choice, while there are some).
    pub show_hidden: bool,
    /// With no card to show: what the page says there, if anything.
    pub quiet: Option<Quiet>,
}

impl BoardView {
    /// The cards shown, in the board's order: down each column, then on to the next.
    pub fn shown(&self) -> Vec<&str> {
        self.columns.iter().flat_map(|column| column.cards.iter().map(CardFace::name)).collect()
    }

    /// Their ids, in the same order.
    pub fn shown_ids(&self) -> impl Iterator<Item = &str> {
        self.columns.iter().flat_map(|column| column.cards.iter().map(|card| card.id.as_str()))
    }

    /// The cards' ids, column by column.
    fn ids(&self) -> Vec<Vec<&str>> {
        self.columns.iter().map(|column| column.cards.iter().map(|card| card.id.as_str()).collect()).collect()
    }

    /// The card a key moves to from `current` (see `board::step`).
    pub fn step(&self, current: Option<&str>, direction: Direction) -> Option<String> {
        step_in(&self.ids(), current, direction)
    }

    /// What a ⇧-click on `to` picks, from the card picked last (see `board::between`).
    pub fn between(&self, from: Option<&str>, to: &str) -> Vec<String> {
        between_in(&self.ids(), from, to)
    }

    /// Where a card is on the board: its column and its place in it.
    pub fn place_of(&self, id: &str) -> Option<(usize, usize)> {
        self.columns
            .iter()
            .enumerate()
            .find_map(|(col, column)| column.cards.iter().position(|card| card.id == id).map(|row| (col, row)))
    }

    pub fn has_cards(&self) -> bool {
        self.columns.iter().any(|column| !column.cards.is_empty())
    }
}

/// The board from every card read (`read`: the store's groups), the index and your choices. `chosen` is the Claude
/// you picked for a card, by the card's id.
pub fn board_view(
    work: &Snapshot,
    read: &[Group],
    workspaces: &[Workspace],
    statuses: &HashMap<String, String>,
    choices: &Choices,
    chosen: &HashMap<String, ClaudeChoice>,
    now: i64,
) -> BoardView {
    // the Jira board you picked: only its tickets (none picked: everything read)
    let groups = board_cards(read.to_vec(), work.board.as_ref());
    // two views: the board, everything read but the tasks you hid; and the tasks you hid, where they are unhidden
    let hidden: HashSet<&str> = work.hidden.iter().map(String::as_str).collect();
    let names = Statuses::of(&groups, work);
    let cards: Vec<String> = groups.iter().map(|group| group.id.clone()).collect();
    let (away, on_board): (Vec<Group>, Vec<Group>) =
        groups.into_iter().partition(|group| hidden.contains(group.id.as_str()));
    let show_hidden = choices.show_hidden && !away.is_empty();
    let away_waiting = away.iter().map(|group| group.waiting).sum();
    // what waits on you counts only on the board (a hidden task puts a dot on Hidden)
    let needs_count = on_board.iter().filter(|group| group.waiting > 0).count();
    let needs_you = choices.needs_you && needs_count > 0 && !show_hidden;
    let away_count = away.len();
    let shown: Vec<Group> = (if show_hidden { away } else { on_board })
        .into_iter()
        .filter(|group| (!needs_you || group.waiting > 0) && matches(group, work, &choices.query))
        .collect();
    let flows = flow_of_cards(&work.flows);
    let columns: Vec<ColumnView> = build_board(shown, &names, work)
        .into_iter()
        .map(|column| ColumnView {
            cards: (column.cards.iter())
                .map(|group| CardFace::of(group, work, workspaces, statuses, chosen.get(&group.id), &flows, now))
                .collect(),
            id: column.id,
            name: column.name,
            category: column.category,
            look: column.look,
        })
        .collect();
    let quiet = if columns.iter().any(|column| !column.cards.is_empty()) {
        None
    } else if !choices.query.trim().is_empty() {
        Some(Quiet::NoMatch(format!("Nothing here matches “{}”.", choices.query)))
    } else {
        let read_ok = work.health.iter().filter(|health| health.state == "ok").count() > 0;
        let words = match &work.board {
            // the board's tickets are being read: a moment, nothing to say
            Some(board) if board.keys.is_none() => None,
            Some(board) if away_count == 0 => {
                let sprint = if board.board.kind == "scrum" { " in its running sprint" } else { "" };
                Some(format!("None of your tickets is on {}{sprint}.", board.board.name))
            }
            _ if away_count > 0 => Some("You hid every task.".to_string()),
            _ if read_ok => {
                let latest = work.health.iter().filter_map(|health| health.last_ok).max();
                Some(format!("Read {}: nothing is open.", ago(latest, now)))
            }
            _ => None,
        };
        words.map(Quiet::AllQuiet)
    };
    BoardView { columns, cards, away: away_count, away_waiting, needs_count, needs_you, show_hidden, quiet }
}

/// A key (or a few that do one thing) and what it does, for the status line.
pub type KeyHint = (&'static [&'static str], &'static str);

/// The keys that work right now, as the status line shows them: on the board, or in front of a card.
pub fn status_keys(card_open: bool, flows_open: bool) -> &'static [KeyHint] {
    const BOARD: &[KeyHint] =
        &[(&["←", "→", "↑", "↓"], "move"), (&["↩"], "open"), (&["X"], "pick"), (&["/"], "filter"), (&["⌘I"], "back")];
    const CARD: &[KeyHint] = &[
        (&["J", "K"], "move"),
        (&["S"], "tell Claude"),
        (&["X"], "leave out"),
        (&["E"], "done"),
        (&["Z"], "snooze"),
        (&["⌘↩"], "send"),
        (&["Esc"], "close"),
    ];
    // the Workflows page has one way out; a card opened from it has the card's keys
    const FLOWS: &[KeyHint] = &[(&["Esc"], "back to the board")];
    match (card_open, flows_open) {
        (true, _) => CARD,
        (false, true) => FLOWS,
        (false, false) => BOARD,
    }
}

/// The same as one line of words: "←→↑↓ move, ↩ open, X pick, / filter, ⌘I back".
pub fn status_keys_words(card_open: bool, flows_open: bool) -> String {
    let hints = status_keys(card_open, flows_open);
    let said: Vec<String> = hints.iter().map(|(keys, does)| format!("{} {does}", keys.concat())).collect();
    said.join(", ")
}

/// What the header's Hidden says when pointed at: "The 2 tasks you hid: pick them there to unhide them (1 waiting on
/// you)".
pub fn hidden_words(away: usize, away_waiting: usize) -> String {
    let tasks = if away == 1 { "task".to_string() } else { format!("{away} tasks") };
    let waiting = if away_waiting > 0 { format!(" ({away_waiting} waiting on you)") } else { String::new() };
    format!("The {tasks} you hid: pick them there to unhide them{waiting}")
}

/// What the header's Tasks says when pointed at.
pub const TASKS_WORDS: &str = "Everything read from GitHub and Jira, but the tasks you hid";

/// What Needs you says when pointed at: what a press does.
pub fn needs_you_words(on: bool) -> &'static str {
    if on { "Show every card" } else { "Show only the cards with something waiting on you" }
}

/// The board menu's name ("All tasks", or the board's) and what it says when pointed at.
pub fn board_words(board: Option<&BoardPick>) -> (String, String) {
    match board {
        Some(pick) if pick.board.kind == "scrum" => {
            (pick.board.name.clone(), format!("{}: your tickets in its running sprint", pick.board.name))
        }
        Some(pick) => (pick.board.name.clone(), format!("{}: your tickets on it", pick.board.name)),
        None => ("All tasks".to_string(), "Pick a Jira board to show only its tickets".to_string()),
    }
}

/// The cards that stand in another column than they did (DESIGN.md, "Motion with a job": "a card that moved to
/// another column glows once, so you see it move"). A card that is new on the board didn't move.
pub fn moved_cards(before: &BoardView, now: &BoardView) -> Vec<String> {
    let was: std::collections::HashMap<&str, &str> = (before.columns.iter())
        .flat_map(|column| column.cards.iter().map(move |card| (card.id.as_str(), column.id.as_str())))
        .collect();
    (now.columns.iter())
        .flat_map(|column| column.cards.iter().map(move |card| (card.id.as_str(), column.id.as_str())))
        .filter(|(card, column)| was.get(card).is_some_and(|before| before != column))
        .map(|(card, _)| card.to_string())
        .collect()
}

/// What stands at the right of a board in the menu: "sprint" for a scrum board, else its kind ("kanban").
pub fn board_kind(kind: &str) -> &str {
    if kind == "scrum" { "sprint" } else { kind }
}

/// The board menu is there once Jira is read for a project.
pub fn reads_jira(work: &Snapshot) -> bool {
    work.health.iter().any(|health| health.source == "jira" && health.state != "off")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::work::groups::build_groups;
    use crate::work::groups::tests::NOW;
    use crate::work::sample;
    use crate::work::sources::Health;
    use std::collections::HashMap;

    fn groups(work: &Snapshot) -> Vec<Group> {
        build_groups(work, &[], &HashMap::new(), NOW)
    }

    /// "the tab counts what waits on you while the page is closed", and the tab's part of "doesn't count a hidden card
    /// on the Work tab or in Needs you".
    #[test]
    fn the_tab_counts_what_waits_on_you_on_the_cards_you_didnt_hide() {
        let mut work = sample();
        let mark = tab_mark(Some(&work), &groups(&work));
        // nine items from people (the sample has them on two PRs, #2414 and #2395, and on three tickets)
        assert_eq!(mark, TabMark::Waiting { items: 9, prs: 2 });
        assert_eq!(mark.words(), "Work: 9 waiting on you on 2 PRs");
        // a card you hid doesn't ask for you on the tab: 9 wait on you, 1 of them on the hidden card
        work.hidden = vec!["ticket:SH-21616".into()];
        assert_eq!(tab_mark(Some(&work), &groups(&work)), TabMark::Waiting { items: 8, prs: 2 });
        // nothing read, or nothing waiting: only its name
        assert_eq!(tab_mark(None, &[]), TabMark::Quiet);
        assert_eq!(TabMark::Quiet.words(), "Work");
        work.items.clear();
        assert_eq!(tab_mark(Some(&work), &groups(&work)), TabMark::Quiet);
        assert_eq!(TabMark::Waiting { items: 1, prs: 0 }.words(), "Work: 1 waiting on you");
        assert_eq!(TabMark::Waiting { items: 2, prs: 1 }.words(), "Work: 2 waiting on you on 1 PR");
    }

    /// Says first what can't be read, and what to do, and marks the tab.
    #[test]
    fn a_source_that_cant_be_read_marks_the_tab_before_any_count() {
        let mut work = sample();
        work.health[1].state = "error".into();
        work.health[1].error = Some("Jira: twg isn't signed in".into());
        let mark = tab_mark(Some(&work), &groups(&work));
        assert_eq!((mark, mark.words().as_str()), (TabMark::Broken, "Work: a source can't be read"));
    }

    /// Before the first read, offers to check, with nothing to filter.
    #[test]
    fn before_the_first_read_nothing_was_read_yet() {
        assert!(first_run(None));
        assert!(first_run(Some(&Snapshot::default())));
        assert!(!first_run(Some(&sample())));
    }

    use crate::work::board::Tone;
    use crate::work::groups::tests::{statuses, ws};

    /// The board of the sample with one workspace, whose Claude works on SH-21309.
    fn board(work: &Snapshot, choices: &Choices) -> BoardView {
        let workspaces = [ws("w2", "sh-21309-export", "feature/SH-21309-export")];
        let lamps = statuses(&[("w2", "working")]);
        let read = build_groups(work, &workspaces, &lamps, NOW);
        board_view(work, &read, &workspaces, &lamps, choices, &HashMap::new(), NOW)
    }

    fn card<'a>(board: &'a BoardView, key: &str) -> &'a CardFace {
        (board.columns.iter().flat_map(|column| &column.cards))
            .find(|card| card.name() == key)
            .unwrap_or_else(|| panic!("no card {key} on the board"))
    }

    fn typed(query: &str) -> Choices {
        Choices { query: query.into(), ..Choices::default() }
    }

    /// Shows everything read: a column per Jira status (the backlog's too, none folded), a card per ticket, no project
    /// chips.
    #[test]
    fn shows_everything_read_a_column_per_jira_status_and_a_card_per_ticket() {
        let work = sample();
        let board = board(&work, &Choices::default());
        let columns: Vec<&str> = board.columns.iter().map(|column| column.id.as_str()).collect();
        assert_eq!(
            columns,
            [
                "on hold",
                "new",
                "analysed",
                "ready for dev",
                "planned for current sprint",
                "in progress",
                "code review",
                "qa review",
                "done"
            ]
        );
        // a card says what the ticket is, how its PR stands in a word or two, and how much waits on you
        let export = card(&board, "SH-21309");
        assert_eq!(export.title, "Export orders to the accounting CSV");
        let prs: Vec<String> = export.prs.iter().map(|pr| format!("#{} {}", pr.number, pr.phrase.text)).collect();
        assert_eq!(prs, ["#2414 Checks failing"]);
        assert_eq!((export.prs[0].phrase.tone, export.prs[0].state), (Tone::Red, PrState::Open));
        assert_eq!((export.waiting, export.blocked), (4, true));
        assert_eq!(export.needs, "Needs you: checks failing, 2 remarks waiting, 1 question");
        assert_eq!(export.claude, Some(("sh-21309-export".to_string(), "working".to_string())));
        assert_eq!(card(&board, "SH-21376").claude, None, "a session outside Enkidu isn't on the card");
        assert_eq!(card(&board, "SH-21030").prs[0].phrase.text, "Merged");
        assert_eq!(card(&board, "SH-20728").id, "ticket:SH-20728", "the backlog's tickets are on the board");
        // its first line: the type as Jira names it, the key, a priority only when it isn't Medium
        let CardHead::Ticket { kind, type_name, key, priority } = &export.head else { panic!("{:?}", export.head) };
        assert_eq!((*kind, type_name.as_deref(), key.as_str()), (TypeKind::Story, Some("Story"), "SH-21309"));
        assert_eq!(priority.as_ref().map(|(level, name)| (*level, name.as_str())), Some((1, "High")));
        // a PR that names no ticket: its state and repo#number, then its state in a word or two
        let lone = card(&board, "pr:demo-org/shop#2415");
        assert_eq!(lone.head, CardHead::Pr { state: PrState::Open, name: "shop#2415".into() });
        assert_eq!((lone.prs.len(), lone.more), (1, 0));
        // nothing hidden, four cards need you, nothing to say in the board's place
        assert_eq!((board.away, board.away_waiting, board.needs_count), (0, 0, 4));
        assert!(!board.needs_you && !board.show_hidden && board.quiet.is_none() && board.has_cards());
        assert_eq!(board.cards.len(), 15, "every card can be opened");
        // a ticket with two PRs lists both, the open one first; a third would be "and 1 more"
        let stock = card(&board, "SH-21533");
        let prs: Vec<String> = stock.prs.iter().map(|pr| format!("#{} {}", pr.number, pr.phrase.text)).collect();
        assert_eq!(
            (prs, stock.more),
            (vec!["#2413 Checks running".to_string(), "#2395 Changes requested".to_string()], 0)
        );
        assert_eq!((stock.waiting, stock.blocked), (2, true));
        // something waits, nothing blocks: a blue badge
        let bundles = card(&board, "SH-21112");
        assert_eq!((bundles.waiting, bundles.blocked), (2, false));
        assert_eq!(bundles.needs, "Needs you: 1 QA reply, 1 new attachment");
    }

    /// Filters by text and shows only what needs you.
    #[test]
    fn filters_by_text_and_shows_only_what_needs_you() {
        let work = sample();
        assert_eq!(board(&work, &typed("stock")).shown(), ["SH-21580", "SH-21533"]);
        let nothing = board(&work, &typed("nothing like this"));
        assert_eq!(nothing.quiet, Some(Quiet::NoMatch("Nothing here matches “nothing like this”.".into())));
        assert!(!nothing.has_cards() && nothing.columns.is_empty());
        let needs = board(&work, &Choices { needs_you: true, ..Choices::default() });
        assert_eq!(needs.shown(), ["SH-21309", "SH-21616", "SH-21533", "SH-21112"]);
        assert!(needs.needs_you);
        // both at once: only what needs you among what matches
        let both = board(&work, &Choices { needs_you: true, query: "stock".into(), show_hidden: false });
        assert_eq!(both.shown(), ["SH-21533"]);
    }

    /// Picks cards and hides them: Hidden shows only those; a hidden card isn't counted in Needs you, and the Hidden
    /// button shows what waits there; and the words of an empty board.
    #[test]
    fn hidden_cards_leave_the_board_and_stand_in_their_columns_under_hidden() {
        let mut work = sample();
        work.hidden = vec!["ticket:SH-21475".into(), "ticket:SH-21376".into()];
        let on = board(&work, &Choices::default());
        assert!(!on.shown().contains(&"SH-21475") && !on.shown().contains(&"SH-21376"));
        assert!(!on.columns.iter().any(|column| column.id == "on hold"), "On Hold had only that card");
        assert_eq!((on.away, on.away_waiting), (2, 0));
        let away = board(&work, &Choices { show_hidden: true, ..Choices::default() });
        assert_eq!(away.shown(), ["SH-21475", "SH-21376"]);
        assert_eq!(
            away.columns.iter().map(|column| column.id.as_str()).collect::<Vec<_>>(),
            ["on hold", "code review"]
        );
        assert!(away.show_hidden && !away.needs_you, "Needs you belongs to the board");
        // a hidden card that needs you: it doesn't count on the board, and Hidden says what waits there
        work.hidden = vec!["ticket:SH-21616".into()];
        let on = board(&work, &Choices { needs_you: true, ..Choices::default() });
        assert_eq!((on.needs_count, on.away, on.away_waiting), (3, 1, 1), "4 cards need you; one is hidden");
        assert_eq!(on.shown(), ["SH-21309", "SH-21533", "SH-21112"]);
        // nothing hidden: asking for the hidden ones shows the board
        work.hidden.clear();
        let none = board(&work, &Choices { show_hidden: true, ..Choices::default() });
        assert!(!none.show_hidden && none.shown().len() == 15);
        // everything hidden: the board says so
        work.hidden = none.cards.clone();
        let all = board(&work, &Choices::default());
        assert_eq!(all.quiet, Some(Quiet::AllQuiet("You hid every task.".into())));
        assert_eq!(all.away, 15);
    }

    /// What the page says in the board's place when there is no card.
    #[test]
    fn an_empty_board_says_why_in_one_line_or_nothing() {
        let mut work = sample();
        work.prs.clear();
        work.tickets.clear();
        work.links.clear();
        let empty = board(&work, &Choices::default());
        assert_eq!(empty.quiet, Some(Quiet::AllQuiet("Read 2 min ago: nothing is open.".into())));
        // a Jira board with none of your tickets says so; one that is still being read says nothing
        let full = sample();
        let mut picked = sample();
        let pick = |keys: Option<Vec<String>>| enkidu_core::work::model::BoardPick {
            board: full.boards[0].clone(),
            keys,
            read_at: Some(NOW),
        };
        picked.board = Some(pick(Some(Vec::new())));
        let name = &full.boards[0].name;
        let sprint = if full.boards[0].kind == "scrum" { " in its running sprint" } else { "" };
        let said = board(&picked, &Choices::default()).quiet;
        assert_eq!(said, Some(Quiet::AllQuiet(format!("None of your tickets is on {name}{sprint}."))));
        picked.board = Some(pick(None));
        assert_eq!(board(&picked, &Choices::default()).quiet, None);
        // a board with some of your tickets shows those, and a PR without a ticket is on no Jira board
        picked.board = Some(pick(Some(vec!["SH-21309".into(), "SH-21030".into()])));
        let some = board(&picked, &Choices::default());
        assert_eq!((some.shown(), some.cards.len()), (vec!["SH-21309", "SH-21030"], 2));
        // nothing read at all: nothing to say
        let pending = |h: Health| Health { state: "pending".into(), last_ok: None, ..h };
        let never = Snapshot { health: full.health.iter().cloned().map(pending).collect(), ..Default::default() };
        assert_eq!(board(&never, &Choices::default()).quiet, None);
    }

    /// A card with Claude's report says how the task ended; an update along the way doesn't.
    #[test]
    fn a_card_says_how_claudes_latest_report_says_the_task_ended() {
        use enkidu_core::work::reports::Report;
        let mut work = sample();
        let report = |id: i64, state: &str, text: &str, images: usize, minutes: i64| Report {
            id,
            subject: "SH-21309".into(),
            session_id: Some("s-w2".into()),
            workspace_id: Some("w2".into()),
            state: state.into(),
            text: text.into(),
            images: (0..images).map(|n| format!("shot-{n}.png")).collect(),
            at: NOW - minutes * 60_000,
        };
        // the newest first, as the core gives them
        work.reports = vec![
            report(3, "update", "Halfway: the writer is done", 0, 5),
            report(2, "review", "## Export works\n\nAll 14 tests pass.", 2, 120),
            report(1, "blocked", "Stuck on the locale", 0, 300),
        ];
        let board = board(&work, &Choices::default());
        let mark = card(&board, "SH-21309").report.clone().expect("the card's report");
        assert_eq!((mark.state.as_str(), mark.label, mark.images), ("review", "Review", 2));
        assert_eq!(mark.words, "Finished, wants your review, 2 h ago: Export works");
        assert_eq!(card(&board, "SH-21533").report, None);
        assert_eq!(report_words("done"), ("Done", "Finished, nothing needs you"));
        assert_eq!(report_words("blocked"), ("Stuck", "Stuck, needs you"));
    }

    /// A card in a workflow that is due says when it starts; the one whose task runs says "Now".
    #[test]
    fn a_card_in_a_workflow_that_is_due_or_runs_says_so() {
        use crate::work::flows::tests::{flow, task};
        let mut work = sample();
        work.flows =
            vec![flow("running", vec![task("ticket:SH-21309", "running"), task("ticket:SH-21376", "waiting")])];
        let board = board(&work, &Choices::default());
        let mark = |key: &str| card(&board, key).flow.as_ref().map(|flow| (flow.running, flow.text.clone()));
        assert_eq!(mark("SH-21309"), Some((true, "Now".to_string())));
        assert_eq!(mark("SH-21376"), Some((false, "Next".to_string())));
        assert_eq!(mark("SH-21533"), None);
    }

    /// A card that moved to another column since the last read glows once.
    #[test]
    fn a_card_that_stands_in_another_column_than_before_moved() {
        let before = sample();
        let was = board(&before, &Choices::default());
        assert!(moved_cards(&was, &was).is_empty(), "the same board: nothing moved");
        // SH-21309 goes on to Code Review
        let mut after = before.clone();
        after.tickets.iter_mut().find(|ticket| ticket.key == "SH-21309").unwrap().status = "Code Review".into();
        let now = board(&after, &Choices::default());
        assert_eq!(moved_cards(&was, &now), ["ticket:SH-21309"]);
        // filtered, the cards that stay are where they were; one that comes back didn't move either
        let filtered = board(&before, &Choices { query: "stock".into(), ..Choices::default() });
        assert!(moved_cards(&was, &filtered).is_empty() && moved_cards(&filtered, &was).is_empty());
        // before anything was read, every card is new
        assert!(moved_cards(&BoardView::default(), &was).is_empty());
    }

    /// The header's words when pointed at.
    #[test]
    fn the_headers_chips_and_the_board_menu_say_what_they_do() {
        assert_eq!(hidden_words(1, 0), "The task you hid: pick them there to unhide them");
        assert_eq!(hidden_words(2, 1), "The 2 tasks you hid: pick them there to unhide them (1 waiting on you)");
        assert_eq!(needs_you_words(false), "Show only the cards with something waiting on you");
        assert_eq!(needs_you_words(true), "Show every card");
        let work = sample();
        assert!(reads_jira(&work));
        assert!(!reads_jira(&Snapshot::default()));
        let (name, tip) = board_words(None);
        assert_eq!((name.as_str(), tip.as_str()), ("All tasks", "Pick a Jira board to show only its tickets"));
        let pick = |kind: &str| enkidu_core::work::model::BoardPick {
            board: enkidu_core::work::model::Board { kind: kind.into(), ..work.boards[0].clone() },
            keys: None,
            read_at: None,
        };
        let board = &work.boards[0].name;
        assert_eq!(
            board_words(Some(&pick("scrum"))),
            (board.clone(), format!("{board}: your tickets in its running sprint"))
        );
        assert_eq!(board_words(Some(&pick("kanban"))).1, format!("{board}: your tickets on it"));
        assert_eq!((board_kind("scrum"), board_kind("kanban")), ("sprint", "kanban"));
    }
}
