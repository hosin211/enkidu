//! The Work board (DESIGN.md, "The Work page"): your tickets as cards in columns, one column per Jira status, in the
//! order a workflow runs (to do, in progress, done). A PR that names no ticket stands where its state puts it. Also:
//! what a card says about a PR in a word or two, the look of a status, a type and a priority, what waits on you said as
//! a sentence, and how the keyboard moves across the board.

use std::sync::LazyLock;

use enkidu_core::work::model::{Item, Pr, Snapshot};
use regex::Regex;

use super::groups::Group;
use super::pr::{Checks, checks_of};

/// Jira's status category: to do, in progress, done.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    New,
    Indeterminate,
    Done,
}

impl Category {
    /// From Jira's word for it; anything else is in progress.
    pub fn of(category: &str) -> Category {
        match category {
            "new" => Category::New,
            "done" => Category::Done,
            _ => Category::Indeterminate,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Category::New => "new",
            Category::Indeterminate => "indeterminate",
            Category::Done => "done",
        }
    }
}

/// How a status is drawn at the head of its column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    /// A hollow ring.
    Todo,
    /// A ring that fills as the work moves right.
    Progress,
    /// Paused.
    Hold,
    /// A check.
    Done,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StatusLook {
    pub glyph: Glyph,
    /// An OKLCH hue, or none for gray.
    pub hue: Option<f32>,
    /// How far along the workflow, 0 to 1 (how much of the progress ring is filled).
    pub fill: f32,
}

/// A column of the board.
#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    /// The status in lower case: the same status in two projects is one column.
    pub id: String,
    /// As Jira names it ("Code Review").
    pub name: String,
    pub category: Category,
    pub look: StatusLook,
    pub cards: Vec<Group>,
}

/// A pattern as the old code wrote it (`\b` is a word's edge in ASCII, as in JavaScript).
fn pattern(words: &str) -> Regex {
    Regex::new(&words.replace(r"\b", r"(?-u:\b)")).expect("a pattern of the board")
}

macro_rules! patterns {
    ($($name:ident = $words:expr;)*) => {
        $(static $name: LazyLock<Regex> = LazyLock::new(|| pattern($words));)*
    };
}

patterns! {
    QA = r"\bqa\b|test|verif|accept|\buat\b";
    REVIEW = r"review|\bpr\b|pull request";
    RELEASE = r"release|deploy|merg|ship";
    HOLD = r"hold|block|wait|pause|park|stuck";
    PROGRESS = r"progress|develop|doing|working|implement|start|active";
    TO_BE = r"\bto be\b";
    BACKLOG = r"backlog|icebox";
    FRESH = r"\bnew\b|open|to ?do|created|reported";
    ANALYSIS = r"triag|analy|refin|groom|estimat|design|spec";
    READY = r"ready|selected|planned|sprint|next";
    PLANNED = r"planned|selected|sprint|commit";
    BUG = r"bug|defect";
    SUBTASK = r"sub-?task";
    STORY = r"story";
    EPIC = r"epic";
    PROBLEM = r"problem|incident";
    TASK = r"task|chore|improvement";
    HIGHEST = r"highest|blocker|critical";
    HIGH = r"high|major";
    LOWEST = r"lowest|trivial";
    LOW = r"low|minor";
}

/// Parked work: On Hold, Blocked, Waiting (but "Waiting for QA" is a QA column).
fn parked(status: &str) -> bool {
    HOLD.is_match(status) && !QA.is_match(status) && !REVIEW.is_match(status)
}

/// Where a status stands in a workflow, from its words: On Hold first, the furthest left (parked work, out of the way),
/// then Backlog, New, Analysed, Ready for Dev, Planned, In Progress, Code Review, QA, Release; then Done. A status with
/// other words sits after In Progress.
pub fn status_rank(name: &str, category: Category) -> f32 {
    let s = name.to_lowercase();
    if category == Category::Done {
        return 200.;
    }
    // On Hold (Blocked, Waiting…) stands left of everything, whichever of Jira's categories it is in
    if parked(&s) {
        return -10.;
    }
    // a step still to take stands just before the step itself: "To be analysed", then "Analysed"
    let ahead = if TO_BE.is_match(&s) { 0.25 } else { 0. };
    if category == Category::New {
        let steps: [&LazyLock<Regex>; 4] = [&BACKLOG, &FRESH, &ANALYSIS, &READY];
        let at = steps.iter().position(|step| step.is_match(&s)).map_or(1.5, |at| at as f32);
        return at - ahead;
    }
    // the most telling words first: "QA Review" is QA, "Ready for Release" a release
    let rules: [(&LazyLock<Regex>, f32); 6] =
        [(&QA, 4.), (&RELEASE, 5.), (&REVIEW, 3.), (&HOLD, -1.), (&PLANNED, 0.), (&PROGRESS, 1.)];
    let step = rules.iter().find(|(rule, _)| rule.is_match(&s)).map_or(1.5, |(_, step)| *step);
    100. + step - ahead
}

/// A status's look: to do is a hollow gray ring; in progress a ring that fills further right (blue, review violet,
/// QA amber, release teal); on hold a gray pause; done a green check. The hues are the status pill's.
pub fn status_look(name: &str, category: Category) -> StatusLook {
    let s = name.to_lowercase();
    match category {
        Category::Done => return StatusLook { glyph: Glyph::Done, hue: Some(155.), fill: 1. },
        Category::New => return StatusLook { glyph: Glyph::Todo, hue: None, fill: 0. },
        Category::Indeterminate => {}
    }
    if parked(&s) {
        return StatusLook { glyph: Glyph::Hold, hue: None, fill: 0. };
    }
    // 0 (planned) to 5 (release)
    let step = status_rank(name, category) - 100.;
    let hue = if QA.is_match(&s) {
        75.
    } else if RELEASE.is_match(&s) {
        185.
    } else if REVIEW.is_match(&s) {
        300.
    } else {
        258.
    };
    StatusLook { glyph: Glyph::Progress, hue: Some(hue), fill: (0.25 + step * 0.13).min(0.9) }
}

/// A state of a task of your own (`todo`, `doing`, `done`) as the board sees it: the category Jira would give it, and
/// its words.
pub fn own_state(state: &str) -> (Category, &'static str) {
    match state {
        "doing" => (Category::Indeterminate, "In progress"),
        "done" => (Category::Done, "Done"),
        _ => (Category::New, "To do"),
    }
}

/// Every status your tickets have, in workflow order: what the columns are named after, so a PR without a ticket
/// stands in the same column whatever part of the board is shown.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Statuses(Vec<(String, Category)>);

impl Statuses {
    /// The statuses of every ticket read (pass every group, not only the ones shown).
    pub fn of(groups: &[Group], work: &Snapshot) -> Statuses {
        let mut known: Vec<(String, Category)> = Vec::new();
        for ticket in groups.iter().filter_map(|group| group.ticket(work)) {
            if !known.iter().any(|(name, _)| same(name, &ticket.status)) {
                known.push((ticket.status.clone(), Category::of(&ticket.category)));
            }
        }
        known.sort_by(|a, b| status_rank(&a.0, a.1).total_cmp(&status_rank(&b.0, b.1)));
        Statuses(known)
    }

    /// The first status of a category whose words match, else `fallback`.
    fn pick(&self, category: Category, words: Option<&Regex>, fallback: &str) -> (String, Category) {
        (self.0.iter())
            .find(|(name, of)| *of == category && words.is_none_or(|words| words.is_match(&name.to_lowercase())))
            .cloned()
            .unwrap_or_else(|| (fallback.to_string(), category))
    }
}

/// Two names of one status (they differ only in case).
fn same(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// The board: a column per status of the cards on it, in workflow order. A PR without a ticket goes to the column
/// its state matches (a draft to the one in progress, an open PR to the review, a merged one to done), named as Jira
/// names it when one of your tickets has such a status. The cards keep the order they come in (worst first, then
/// the latest).
pub fn build_board(groups: Vec<Group>, statuses: &Statuses, work: &Snapshot) -> Vec<Column> {
    let place_of = |group: &Group| -> (String, Category) {
        if let Some(ticket) = group.ticket(work) {
            return (ticket.status.clone(), Category::of(&ticket.category));
        }
        // a task of your own stands with the tickets where Jira has such a status (in progress, done), in the column
        // named as Jira names it; what is still to do has a column of its own, "To do"
        if let Some(task) = group.own(work) {
            return match own_state(&task.state) {
                (Category::Indeterminate, _) => statuses.pick(Category::Indeterminate, Some(&PROGRESS), "In Progress"),
                (Category::Done, _) => statuses.pick(Category::Done, None, "Done"),
                (category, words) => (words.to_string(), category),
            };
        }
        let open: Vec<&Pr> = group.prs.iter().map(|pr| pr.of(work)).filter(|pr| pr.state == "OPEN").collect();
        if open.is_empty() {
            statuses.pick(Category::Done, None, "Done")
        } else if open.iter().all(|pr| pr.draft) {
            statuses.pick(Category::Indeterminate, Some(&PROGRESS), "In Progress")
        } else {
            statuses.pick(Category::Indeterminate, Some(&REVIEW), "In Review")
        }
    };
    let mut columns: Vec<Column> = Vec::new();
    for group in groups {
        let (name, category) = place_of(&group);
        let id = name.to_lowercase();
        let at = columns.iter().position(|column| column.id == id).unwrap_or_else(|| {
            columns.push(Column { id, look: status_look(&name, category), name, category, cards: Vec::new() });
            columns.len() - 1
        });
        columns[at].cards.push(group);
    }
    columns.sort_by(|a, b| {
        let by_rank = status_rank(&a.name, a.category).total_cmp(&status_rank(&b.name, b.category));
        by_rank.then_with(|| a.id.cmp(&b.id)).then_with(|| a.name.cmp(&b.name))
    });
    columns
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Red,
    Amber,
    Green,
    Violet,
    Muted,
}

/// A PR's state in a word or two.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrPhrase {
    pub text: &'static str,
    pub tone: Tone,
}

/// A PR's state in a word or two, the most pressing first: what blocks it, then where its review stands.
pub fn pr_phrase(pr: &Pr) -> PrPhrase {
    let say = |text, tone| PrPhrase { text, tone };
    match pr.state.as_str() {
        "MERGED" => return say("Merged", Tone::Violet),
        "CLOSED" => return say("Closed", Tone::Muted),
        _ => {}
    }
    let checks = checks_of(pr);
    let review = pr.review_decision.as_deref();
    if pr.mergeable.as_deref() == Some("CONFLICTING") {
        say("Conflicts", Tone::Red)
    } else if checks == Checks::Failing {
        say("Checks failing", Tone::Red)
    } else if review == Some("CHANGES_REQUESTED") {
        say("Changes requested", Tone::Red)
    } else if pr.draft {
        say("Draft", Tone::Muted)
    } else if checks == Checks::Running {
        say("Checks running", Tone::Amber)
    } else if review == Some("APPROVED") {
        say("Approved", Tone::Green)
    } else if review == Some("REVIEW_REQUIRED") {
        say("Waiting for review", Tone::Muted)
    } else if checks == Checks::Passed {
        say("Checks passed", Tone::Green)
    } else {
        say("Open", Tone::Muted)
    }
}

/// A Jira issue type as one of the kinds the page draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeKind {
    Bug,
    Task,
    Story,
    Subtask,
    Epic,
    Problem,
    Other,
}

impl TypeKind {
    pub fn name(self) -> &'static str {
        match self {
            TypeKind::Bug => "bug",
            TypeKind::Task => "task",
            TypeKind::Story => "story",
            TypeKind::Subtask => "subtask",
            TypeKind::Epic => "epic",
            TypeKind::Problem => "problem",
            TypeKind::Other => "other",
        }
    }
}

/// A Jira issue type (Bug, Task, Story, Sub-task, Epic, Problem) as a kind.
pub fn type_kind(issue_type: Option<&str>) -> TypeKind {
    let s = issue_type.unwrap_or("").to_lowercase();
    if BUG.is_match(&s) {
        TypeKind::Bug
    } else if SUBTASK.is_match(&s) {
        TypeKind::Subtask
    } else if STORY.is_match(&s) {
        TypeKind::Story
    } else if EPIC.is_match(&s) {
        TypeKind::Epic
    } else if PROBLEM.is_match(&s) {
        TypeKind::Problem
    } else if TASK.is_match(&s) {
        TypeKind::Task
    } else {
        TypeKind::Other
    }
}

/// A priority worth showing: 2 Highest, 1 High, -1 Low, -2 Lowest; Medium (the usual) shows nothing.
pub fn priority_level(priority: Option<&str>) -> Option<i8> {
    let s = priority.unwrap_or("").to_lowercase();
    if HIGHEST.is_match(&s) {
        Some(2)
    } else if HIGH.is_match(&s) {
        Some(1)
    } else if LOWEST.is_match(&s) {
        Some(-2)
    } else if LOW.is_match(&s) {
        Some(-1)
    } else {
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

/// The card a key moves to: up and down within its column, left and right to the nearest card of the next column.
/// With nothing selected (or a card no longer on the board), the first card.
pub fn step(columns: &[Column], current: Option<&str>, direction: Direction) -> Option<String> {
    step_in(&ids(columns), current, direction)
}

/// The cards' ids, column by column.
fn ids(columns: &[Column]) -> Vec<Vec<&str>> {
    columns.iter().map(|column| column.cards.iter().map(|card| card.id.as_str()).collect()).collect()
}

/// [`step`] on the cards' ids, column by column (the board as it is drawn has faces, not groups).
pub fn step_in(columns: &[Vec<&str>], current: Option<&str>, direction: Direction) -> Option<String> {
    let shown: Vec<&Vec<&str>> = columns.iter().filter(|cards| !cards.is_empty()).collect();
    let first = shown.first()?;
    let place = shown.iter().enumerate().find_map(|(col, cards)| {
        let row = cards.iter().position(|card| Some(*card) == current)?;
        Some((col, row))
    });
    let Some((col, row)) = place else { return Some(first[0].to_string()) };
    let cards = shown[col];
    let to = match direction {
        Direction::Up => cards[row.saturating_sub(1)],
        Direction::Down => cards[(row + 1).min(cards.len() - 1)],
        Direction::Left | Direction::Right => {
            let next =
                if direction == Direction::Right { (col + 1).min(shown.len() - 1) } else { col.saturating_sub(1) };
            let next = shown[next];
            next[row.min(next.len() - 1)]
        }
    };
    Some(to.to_string())
}

/// What a ⇧-click picks: every card from the last one picked to the one clicked, both in, in the board's order
/// (down each column, then on to the next). The card clicked comes last, so the next ⇧-click starts there. With no
/// card picked before (or one no longer on the board), just the card clicked.
pub fn between(columns: &[Column], from: Option<&str>, to: &str) -> Vec<String> {
    between_in(&ids(columns), from, to)
}

/// [`between`] on the cards' ids, column by column.
pub fn between_in(columns: &[Vec<&str>], from: Option<&str>, to: &str) -> Vec<String> {
    let order: Vec<&str> = columns.iter().flatten().copied().collect();
    let place = |id: &str| order.iter().position(|card| *card == id);
    let (Some(a), Some(b)) = (from.and_then(place), place(to)) else { return vec![to.to_string()] };
    let mut run: Vec<String> = order[a.min(b)..=a.max(b)].iter().map(|id| id.to_string()).collect();
    if a > b {
        run.reverse();
    }
    run
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// What waits on you, said as a sentence a person would say: "mira-k left a remark on export.py, line 48".
pub fn item_sentence(item: &Item, base: Option<&str>) -> String {
    let who = item.who.as_deref().unwrap_or("Someone");
    let at = match (item.path.as_deref(), item.line.filter(|line| *line != 0)) {
        (Some(path), Some(line)) if !path.is_empty() => format!(" on {}, line {line}", file_name(path)),
        (Some(path), None) if !path.is_empty() => format!(" on {}", file_name(path)),
        _ => String::new(),
    };
    match item.kind.as_str() {
        "remark" => format!("{who} left a remark{at}"),
        "probably_fixed" => format!("{who}'s remark{at} looks fixed, but has no reply yet"),
        "question" => format!("{who} asked a question"),
        "changes_requested" => format!("{who} asked for changes"),
        "pipeline_failed" => "Checks failed on the latest commit".into(),
        "conflict" => format!("The PR conflicts with {}", base.unwrap_or("its base")),
        "qa_replied" => format!("{who} replied in Jira"),
        "mentioned" => format!("{who} mentioned you in Jira"),
        "back_to_you" => "The ticket came back to you in Jira".into(),
        _ => format!("{who} added a file in Jira"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::work::groups::build_groups;
    use crate::work::groups::tests::NOW;
    use crate::work::sample;
    use enkidu_core::work::model::Check;
    use std::collections::HashMap;

    /// The board of these tests: the sample's groups with no workspace at all.
    fn board() -> (Snapshot, Vec<Group>, Vec<Column>) {
        let work = sample();
        let groups = build_groups(&work, &[], &HashMap::new(), NOW);
        let board = build_board(groups.clone(), &Statuses::of(&groups, &work), &work);
        (work, groups, board)
    }

    fn column<'a>(board: &'a [Column], id: &str) -> &'a Column {
        board.iter().find(|column| column.id == id).unwrap_or_else(|| panic!("no column {id}"))
    }

    fn keys(column: &Column) -> Vec<&str> {
        column.cards.iter().map(Group::name).collect()
    }

    /// Puts the statuses of a real Jira workflow in its order.
    #[test]
    fn puts_the_statuses_of_a_real_jira_workflow_in_its_order() {
        use Category::{Done, Indeterminate, New};
        // the statuses the recorded tickets of SH and OPS have, shuffled
        let mut statuses = vec![
            ("Code Review", Indeterminate),
            ("Done", Done),
            ("Ready for Dev", New),
            ("On Hold", Indeterminate),
            ("New", New),
            ("In Progress", Indeterminate),
            ("Planned for current sprint", Indeterminate),
            ("Analysed", New),
            ("To be analysed", New),
            ("QA Review", Indeterminate),
            ("Ready for Release", Indeterminate),
            ("Backlog", New),
        ];
        statuses.sort_by(|a, b| status_rank(a.0, a.1).total_cmp(&status_rank(b.0, b.1)));
        assert_eq!(
            statuses.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
            [
                "On Hold",
                "Backlog",
                "New",
                "To be analysed",
                "Analysed",
                "Ready for Dev",
                "Planned for current sprint",
                "In Progress",
                "Code Review",
                "QA Review",
                "Ready for Release",
                "Done",
            ]
        );
        // a status with other words sits right after In Progress
        assert!(status_rank("Translation", Indeterminate) > status_rank("In Progress", Indeterminate));
        assert!(status_rank("Translation", Indeterminate) < status_rank("Code Review", Indeterminate));
        // parked work is the furthest left, before the backlog too, whichever Jira category it is in
        assert!(status_rank("On Hold", Indeterminate) < status_rank("Backlog", New));
        assert!(status_rank("On Hold", New) < status_rank("Backlog", New));
        assert!(status_rank("Blocked", Indeterminate) < status_rank("New", New));
        let waiting_for_qa = status_rank("Waiting for QA", Indeterminate);
        assert!(waiting_for_qa > status_rank("Code Review", Indeterminate), "QA words win: a QA column");
    }

    /// Draws a status as a ring that fills as the work moves right, in the status pill's hues.
    #[test]
    fn draws_a_status_as_a_ring_that_fills_as_the_work_moves_right() {
        use Category::{Done, Indeterminate, New};
        assert_eq!(status_look("New", New), StatusLook { glyph: Glyph::Todo, hue: None, fill: 0. });
        assert_eq!(status_look("On Hold", Indeterminate).glyph, Glyph::Hold);
        assert_eq!(status_look("Done", Done), StatusLook { glyph: Glyph::Done, hue: Some(155.), fill: 1. });
        let progress = status_look("In Progress", Indeterminate);
        let review = status_look("Code Review", Indeterminate);
        let qa = status_look("QA Review", Indeterminate);
        assert_eq!([progress.hue, review.hue, qa.hue], [Some(258.), Some(300.), Some(75.)]);
        assert!(progress.fill < review.fill && review.fill < qa.fill);
        // a release is teal and the fullest, never a whole ring
        let release = status_look("Ready for Release", Indeterminate);
        assert_eq!((release.glyph, release.hue), (Glyph::Progress, Some(185.)));
        assert!(qa.fill < release.fill && release.fill <= 0.9);
    }

    /// Makes a column per status of the cards on it, the backlog's too: the board is everything read.
    #[test]
    fn makes_a_column_per_status_of_the_cards_on_it_the_backlogs_too() {
        let (_, _, board) = board();
        let columns: Vec<String> = board.iter().map(|c| format!("{} {}", c.name, c.cards.len())).collect();
        assert_eq!(
            columns,
            [
                "On Hold 1",
                "New 1",
                "Analysed 1",
                "Ready for Dev 3",
                "Planned for current sprint 1",
                "In Progress 3",
                "Code Review 3",
                "QA Review 1",
                "Done 1",
            ]
        );
        // the cards keep their order: what needs you first, then the latest
        assert_eq!(keys(column(&board, "in progress")), ["SH-21309", "SH-21616", "pr:demo-org/enkidu#212"]);
        assert_eq!(keys(column(&board, "code review")), ["SH-21533", "pr:demo-org/shop#2415", "SH-21376"]);
    }

    /// Puts a PR without a ticket in the column its state matches, named as Jira names it.
    #[test]
    fn puts_a_pr_without_a_ticket_in_the_column_its_state_matches_named_as_jira_names_it() {
        let (mut work, groups, board) = board();
        // an open PR goes to the review column (Code Review here), a draft to the one in progress
        assert!(keys(column(&board, "code review")).contains(&"pr:demo-org/shop#2415"));
        assert!(keys(column(&board, "in progress")).contains(&"pr:demo-org/enkidu#212"));
        // with no Jira at all (a GitHub-only project), the columns are named for the PRs' states
        let mut lone: Vec<Group> = groups.iter().filter(|g| g.ticket.is_none()).cloned().collect();
        let mut merged = lone[0].clone();
        let pr = Pr { number: 1, state: "MERGED".into(), ..merged.prs[0].of(&work).clone() };
        work.prs.push(pr);
        merged.id = "pr:x#1".into();
        merged.prs[0].pr = work.prs.len() - 1;
        lone.push(merged);
        let own = build_board(lone.clone(), &Statuses::of(&lone, &work), &work);
        let named: Vec<String> = own.iter().map(|c| format!("{} {}", c.name, c.category.name())).collect();
        assert_eq!(named, ["In Progress indeterminate", "In Review indeterminate", "Done done"]);
    }

    /// Moves the keyboard down a column and across to the nearest card of the next one.
    #[test]
    fn moves_the_keyboard_down_a_column_and_across_to_the_nearest_card_of_the_next_one() {
        use Direction::{Down, Left, Right, Up};
        let (_, _, board) = board();
        let to = |from: Option<&str>, direction| step(&board, from, direction);
        let card = |id: &str| Some(id.to_string());
        assert_eq!(to(None, Down), card("ticket:SH-21475"), "nothing selected: the first card (On Hold's)");
        assert_eq!(to(Some("ticket:SH-21309"), Down), card("ticket:SH-21616"));
        assert_eq!(to(Some("ticket:SH-21309"), Up), card("ticket:SH-21309"), "the top stays");
        assert_eq!(to(Some("ticket:SH-21616"), Right), card("pr:demo-org/shop#2415"), "the card at the same height");
        assert_eq!(to(Some("ticket:SH-21309"), Left), card("ticket:SH-21522"), "or the last of a shorter column");
        assert_eq!(to(Some("ticket:SH-21580"), Left), card("ticket:SH-21475"), "New's card: On Hold, the first column");
        assert_eq!(to(Some("ticket:SH-21475"), Left), card("ticket:SH-21475"), "the first column stays");
        assert_eq!(to(Some("ticket:SH-21030"), Right), card("ticket:SH-21030"), "the last column stays");
        assert_eq!(to(Some("ticket:SH-1"), Down), card("ticket:SH-21475"), "a card no longer on the board: the first");
        assert_eq!(step(&[], None, Down), None, "an empty board has no card to go to");
    }

    /// Picks a run with ⇧: down each column, then on to the next, the card clicked last.
    #[test]
    fn picks_a_run_with_shift_down_each_column_then_on_to_the_next_the_card_clicked_last() {
        let (_, _, board) = board();
        let run = between(&board, Some("ticket:SH-20781"), "ticket:SH-21522");
        assert_eq!(run, ["ticket:SH-20781", "ticket:SH-20947", "ticket:SH-21522"]);
        // upwards: the same run, ending where you clicked, so the next run starts there
        let back = between(&board, Some("ticket:SH-21522"), "ticket:SH-20781");
        assert_eq!(back, ["ticket:SH-21522", "ticket:SH-20947", "ticket:SH-20781"]);
        assert_eq!(between(&board, None, "ticket:SH-21475"), ["ticket:SH-21475"], "nothing picked before: the card");
        let gone = between(&board, Some("ticket:SH-1"), "ticket:SH-21475");
        assert_eq!(gone, ["ticket:SH-21475"], "the last pick is no longer on the board: the card");
    }

    /// The tasks you hid: stand in their statuses' columns when you look at them, and leave the board's.
    #[test]
    fn the_tasks_you_hid_stand_in_their_statuses_columns_and_leave_the_boards() {
        let (work, groups, _) = board();
        let hidden = ["ticket:SH-21475", "ticket:SH-21616", "pr:demo-org/shop#2415"];
        let (away, rest): (Vec<Group>, Vec<Group>) =
            groups.iter().cloned().partition(|g| hidden.contains(&g.id.as_str()));
        // the columns keep the names your tickets give them: the PR without a ticket stands in Code Review here too
        let away = build_board(away, &Statuses::of(&groups, &work), &work);
        let said: Vec<String> = away.iter().map(|c| format!("{}: {}", c.name, keys(c).join(", "))).collect();
        assert_eq!(said, ["On Hold: SH-21475", "In Progress: SH-21616", "Code Review: pr:demo-org/shop#2415"]);
        let rest = build_board(rest.clone(), &Statuses::of(&rest, &work), &work);
        assert!(!rest.iter().any(|c| c.id == "on hold"), "On Hold had only SH-21475");
        assert!(!rest.iter().flat_map(|c| &c.cards).any(|g| hidden.contains(&g.id.as_str())));
    }

    /// What a card says: says a PR's state in a word or two, the most pressing first.
    #[test]
    fn says_a_prs_state_in_a_word_or_two_the_most_pressing_first() {
        let (work, groups, _) = board();
        let base = groups.iter().find(|g| g.key.as_deref() == Some("SH-21376")).unwrap().prs[0].of(&work).clone();
        let failed = || vec![Check { name: "t".into(), state: "FAILURE".into(), url: None }];
        let some = |text: &str| Some(text.to_string());
        let said = |pr: Pr| pr_phrase(&pr);
        assert_eq!(
            said(Pr { state: "MERGED".into(), ..base.clone() }),
            PrPhrase { text: "Merged", tone: Tone::Violet }
        );
        assert_eq!(said(Pr { state: "CLOSED".into(), ..base.clone() }), PrPhrase { text: "Closed", tone: Tone::Muted });
        let conflict =
            Pr { mergeable: some("CONFLICTING"), checks: some("FAILURE"), failing: failed(), ..base.clone() };
        assert_eq!(said(conflict).text, "Conflicts");
        let failing = Pr {
            checks: some("FAILURE"),
            failing: failed(),
            review_decision: some("CHANGES_REQUESTED"),
            ..base.clone()
        };
        assert_eq!(said(failing).text, "Checks failing");
        let changes = Pr { review_decision: some("CHANGES_REQUESTED"), ..base.clone() };
        assert_eq!(said(changes), PrPhrase { text: "Changes requested", tone: Tone::Red });
        let draft = Pr { draft: true, running: 2, ..base.clone() };
        assert_eq!(said(draft).text, "Draft", "a draft says so before its running checks");
        let running = Pr { running: 2, review_decision: some("APPROVED"), ..base.clone() };
        assert_eq!(said(running), PrPhrase { text: "Checks running", tone: Tone::Amber });
        let approved = Pr { review_decision: some("APPROVED"), ..base.clone() };
        assert_eq!(said(approved), PrPhrase { text: "Approved", tone: Tone::Green });
        assert_eq!(said(Pr { review_decision: some("REVIEW_REQUIRED"), ..base.clone() }).text, "Waiting for review");
        let free = Pr { review_decision: None, ..base.clone() };
        assert_eq!(said(free).text, "Checks passed", "no review needed, checks green");
        assert_eq!(said(Pr { review_decision: None, checks: None, ..base }).text, "Open");
    }

    /// Knows Jira's issue types and shows only the priorities that stand out.
    #[test]
    fn knows_jiras_issue_types_and_shows_only_the_priorities_that_stand_out() {
        // the types the recorded tickets have
        let types = ["Task", "Bug", "Sub-task", "Epic", "Story", "[System] Problem", "Spike"];
        let kinds: Vec<&str> = types.iter().map(|t| type_kind(Some(t)).name()).collect();
        assert_eq!(kinds, ["task", "bug", "subtask", "epic", "story", "problem", "other"]);
        assert_eq!(type_kind(None), TypeKind::Other);
        let priorities = [Some("Highest"), Some("High"), Some("Medium"), Some("Low"), Some("Lowest"), None];
        let levels: Vec<Option<i8>> = priorities.iter().map(|p| priority_level(*p)).collect();
        assert_eq!(levels, [Some(2), Some(1), None, Some(-1), Some(-2), None]);
    }

    /// Says what waits on you as a sentence.
    #[test]
    fn says_what_waits_on_you_as_a_sentence() {
        let (work, groups, _) = board();
        let base = groups[0].prs[0].items(&work).next().expect("the first card's first item").clone();
        let item =
            |kind: &str, who: Option<&str>| Item { kind: kind.into(), who: who.map(String::from), ..base.clone() };
        let remark = Item { path: Some("orders/export.py".into()), line: Some(48), ..item("remark", Some("mira-k")) };
        assert_eq!(item_sentence(&remark, None), "mira-k left a remark on export.py, line 48");
        assert_eq!(item_sentence(&item("changes_requested", Some("mira-k")), None), "mira-k asked for changes");
        assert_eq!(item_sentence(&item("conflict", None), Some("development")), "The PR conflicts with development");
        assert_eq!(item_sentence(&item("qa_replied", Some("Jonas Weber")), None), "Jonas Weber replied in Jira");
        assert_eq!(item_sentence(&item("question", None), None), "Someone asked a question");
        // the rest of the kinds
        let fixed = Item { path: Some("a/b.py".into()), line: None, ..item("probably_fixed", Some("mira-k")) };
        assert_eq!(item_sentence(&fixed, None), "mira-k's remark on b.py looks fixed, but has no reply yet");
        assert_eq!(item_sentence(&item("pipeline_failed", None), None), "Checks failed on the latest commit");
        assert_eq!(item_sentence(&item("conflict", None), None), "The PR conflicts with its base");
        assert_eq!(item_sentence(&item("mentioned", Some("Jonas Weber")), None), "Jonas Weber mentioned you in Jira");
        assert_eq!(item_sentence(&item("back_to_you", None), None), "The ticket came back to you in Jira");
        assert_eq!(item_sentence(&item("attachment", Some("Jonas Weber")), None), "Jonas Weber added a file in Jira");
    }
}
