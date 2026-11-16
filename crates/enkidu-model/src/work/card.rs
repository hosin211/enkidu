//! The card in front of the board (DESIGN.md, "The card in front of the board"): what its header says, which tabs it
//! has, its timeline, and on the right how it stands: Jira's status first, then its pull requests, each always in the
//! same place.

use enkidu_core::work::model::{Pr, Snapshot};

use super::board::{Category, Tone, TypeKind, pr_phrase, priority_level, status_look, type_kind};
use super::compose::card_items;
use super::groups::Group;
use super::page::{CardHead, PrState};
use super::pr::{Checks, checks_of};
use super::sources::ago;

/// How many lines of the timeline show before "Show all".
pub const TIMELINE_SHOWN: usize = 8;

/// A tab of the card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tab {
    /// The story: Claude's report, the task, what needs you.
    Overview,
    /// What is going on in one PR: its place in the snapshot's PRs, and its number.
    Pr { pr: usize, number: i64 },
    /// The timeline.
    Activity,
}

impl Tab {
    pub fn label(&self) -> String {
        match self {
            Tab::Overview => "Overview".into(),
            Tab::Pr { number, .. } => format!("PR #{number}"),
            Tab::Activity => "Activity".into(),
        }
    }
}

/// Jira's side of the card, at the right.
#[derive(Clone, Debug, PartialEq)]
pub enum JiraBox {
    /// The ticket as it was read.
    Read(JiraFacts),
    /// A key a PR names whose Jira project isn't read for this project.
    NotRead,
}

/// What `JiraBox::NotRead` says.
pub const JIRA_NOT_READ: &str = "Its Jira project isn't read for this project, so its status isn't known here.";

#[derive(Clone, Debug, PartialEq)]
pub struct JiraFacts {
    /// The status as Jira names it, and the hue of its pill (none: gray; the hue of its column's ring).
    pub status: String,
    pub hue: Option<f32>,
    /// The type as Jira names it, and as the page draws it.
    pub issue_type: Option<(String, TypeKind)>,
    /// The priority as Jira names it, with its mark when it isn't Medium.
    pub priority: Option<(String, Option<i8>)>,
    pub assignee: String,
    /// "2 h ago".
    pub updated: String,
}

/// One PR of the card, as its box at the right says it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrFacts {
    /// Its place in the snapshot's PRs.
    pub pr: usize,
    pub number: i64,
    pub state: PrState,
    /// The color of its state's word: green when open, else as its state says.
    pub tone: Tone,
    pub title: String,
    pub head: String,
    pub base: String,
    pub url: String,
    /// The latest commit's checks, for an open PR that has some: "Failing: tests (3.12)", "Running (2)", "Passed".
    pub checks: Option<(Checks, String)>,
    /// Where its review stands, for an open PR, and the color of that (green approved, red changes requested).
    pub review: Option<(String, Tone)>,
    /// "Conflicts with development".
    pub conflict: Option<String>,
    /// "2 open threads".
    pub threads: Option<String>,
    /// When it was merged: "3 days ago".
    pub merged: Option<String>,
}

impl PrFacts {
    pub fn of(pr: &Pr, at: usize, now: i64) -> PrFacts {
        let state = PrState::of(pr);
        let open = pr.state == "OPEN";
        let checks = checks_of(pr);
        let said = match checks {
            Checks::Failing if pr.failing.is_empty() => "Failing".to_string(),
            Checks::Failing => {
                format!(
                    "Failing: {}",
                    pr.failing.iter().map(|check| check.name.as_str()).collect::<Vec<_>>().join(", ")
                )
            }
            Checks::Running if pr.running > 0 => format!("Running ({})", pr.running),
            Checks::Running => "Running".to_string(),
            Checks::Passed => "Passed".to_string(),
            Checks::None => String::new(),
        };
        let review = open.then(|| match (pr.draft, pr.review_decision.as_deref()) {
            (true, _) => ("Draft, not asked for review yet".to_string(), Tone::Muted),
            (false, Some("APPROVED")) => ("Approved".to_string(), Tone::Green),
            (false, Some("CHANGES_REQUESTED")) => ("Changes requested".to_string(), Tone::Red),
            (false, Some("REVIEW_REQUIRED")) => ("Waiting for review".to_string(), Tone::Muted),
            (false, _) => ("No review needed".to_string(), Tone::Muted),
        });
        let threads = match pr.unresolved {
            _ if !open => None,
            0 => None,
            1 => Some("1 open thread".to_string()),
            n => Some(format!("{n} open threads")),
        };
        PrFacts {
            pr: at,
            number: pr.number,
            state,
            tone: if state == PrState::Open { Tone::Green } else { pr_phrase(pr).tone },
            title: pr.title.clone(),
            head: pr.head.clone(),
            base: pr.base.clone(),
            url: pr.url.clone(),
            checks: (open && checks != Checks::None).then_some((checks, said)),
            review,
            conflict: (open && pr.mergeable.as_deref() == Some("CONFLICTING"))
                .then(|| format!("Conflicts with {}", pr.base)),
            threads,
            merged: (pr.state == "MERGED")
                .then(|| pr.merged_at.as_deref().and_then(enkidu_core::work::parse::parse_time))
                .flatten()
                .map(|at| ago(Some(at), now)),
        }
    }
}

/// What kind of thing waits on you: it picks the item's sign.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Remark,
    ProbablyFixed,
    Question,
    ChangesRequested,
    PipelineFailed,
    Conflict,
    QaReplied,
    Mentioned,
    BackToYou,
    Attachment,
}

impl ItemKind {
    pub fn of(kind: &str) -> ItemKind {
        match kind {
            "remark" => ItemKind::Remark,
            "probably_fixed" => ItemKind::ProbablyFixed,
            "question" => ItemKind::Question,
            "changes_requested" => ItemKind::ChangesRequested,
            "pipeline_failed" => ItemKind::PipelineFailed,
            "conflict" => ItemKind::Conflict,
            "qa_replied" => ItemKind::QaReplied,
            "mentioned" => ItemKind::Mentioned,
            "back_to_you" => ItemKind::BackToYou,
            _ => ItemKind::Attachment,
        }
    }

    /// It blocks the work: failing checks, changes asked for, a conflict, a ticket that came back.
    pub fn severe(self) -> bool {
        matches!(self, ItemKind::PipelineFailed | ItemKind::ChangesRequested | ItemKind::Conflict | ItemKind::BackToYou)
    }
}

/// One thing that waits on you, as the card says it: a sentence a person would say, the words quoted under it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemRow {
    pub id: String,
    pub kind: ItemKind,
    /// "mira-k left a remark on export.py, line 48".
    pub sentence: String,
    /// Written by a bot.
    pub bot: bool,
    /// "40 min ago".
    pub when: String,
    /// What was said, quoted.
    pub detail: Option<String>,
    /// Where it was said.
    pub url: Option<String>,
}

/// Everything that waits on you on a card, in the card's order: its PRs' items, then the ticket's own.
pub fn item_rows(group: &Group, work: &Snapshot, now: i64) -> Vec<ItemRow> {
    (card_items(group, work).into_iter())
        .map(|item| {
            // a conflict names the branch the PR goes into
            let base = (item.pr.as_deref())
                .and_then(|pr| group.prs.iter().map(|view| view.of(work)).find(|found| found.reference() == pr))
                .map(|pr| pr.base.as_str());
            ItemRow {
                id: item.id.clone(),
                kind: ItemKind::of(&item.kind),
                sentence: super::board::item_sentence(item, base),
                bot: item.bot,
                when: ago(enkidu_core::work::parse::parse_time(&item.at), now),
                detail: Some(item.detail.clone()).filter(|detail| !detail.is_empty()),
                url: item.url.clone().filter(|url| !url.is_empty()),
            }
        })
        .collect()
}

/// The item the keyboard is on, of `count`: the one you moved to (`selected`, its place among the items), or the
/// one that took its place (`last`, where the keyboard was), the first as the card opens.
pub fn current_item(count: usize, selected: Option<usize>, last: usize) -> Option<usize> {
    match selected {
        Some(at) if at < count => Some(at),
        _ if count > 0 => Some(last.min(count - 1)),
        _ => None,
    }
}

/// One item on (`by` 1) or back (`by` -1) from the one the keyboard is on, staying at the ends.
pub fn step_item(count: usize, at: Option<usize>, by: i32) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let from = at.map_or(-1, |at| at as i64);
    Some((from + i64::from(by)).clamp(0, count as i64 - 1) as usize)
}

/// The card in front of the board, apart from what you tell Claude.
#[derive(Clone, Debug, PartialEq)]
pub struct CardModel {
    /// The group's id.
    pub id: String,
    pub head: CardHead,
    pub title: String,
    /// It is one of the cards you hid: the header says so, and offers Unhide.
    pub hidden: bool,
    /// The ticket's page in Jira, for a ticket that was read (a PR without a ticket has its link in its box).
    pub jira_url: Option<String>,
    pub tabs: Vec<Tab>,
    /// There is something to tell on the left (a tab at least): the wide card. Else the facts alone, narrow.
    pub story: bool,
    /// Jira's box, for a card with a key.
    pub jira: Option<JiraBox>,
    /// Its PRs' boxes, in the card's order; none: "No pull request yet."
    pub prs: Vec<PrFacts>,
    /// Its timeline, the newest first: when, in words, and what happened.
    pub timeline: Vec<(String, String)>,
    /// For a task of your own: its facts, in the box where a ticket has Jira's.
    pub own: Option<OwnFacts>,
}

/// The facts of a task of your own (item 8.5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnFacts {
    /// The number on its card, which `enkidu report --task` names it by.
    pub number: i64,
    /// The name of the project it is for.
    pub project: String,
    /// `todo`, `doing` or `done`.
    pub state: String,
    /// What it is about, in your words; may be empty.
    pub notes: String,
    /// When you made it: "3 h ago".
    pub made: String,
}

impl CardModel {
    pub fn of(group: &Group, work: &Snapshot, now: i64) -> CardModel {
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
        // its timeline: what happened to the ticket and to its PRs
        let mut subjects: Vec<String> = group.prs.iter().map(|view| view.of(work).reference()).collect();
        subjects.extend(group.key.clone());
        // a task of your own is written about under its card's own name
        subjects.extend(group.own.map(|_| group.id.clone()));
        let timeline: Vec<(String, String)> = (work.events.iter())
            .filter(|event| subjects.contains(&event.subject))
            .map(|event| (ago(Some(event.at), now), event.text.clone()))
            .collect();
        // a tab per PR the read knows the details of
        let detailed = group.prs.iter().filter(|view| view.of(work).details.created_at.is_some());
        let overview = ticket.is_some_and(|ticket| ticket.description.is_some())
            || group.own(work).is_some_and(|task| !task.notes.is_empty())
            || !card_items(group, work).is_empty()
            || !group.reports.is_empty();
        let mut tabs: Vec<Tab> = Vec::new();
        if overview {
            tabs.push(Tab::Overview);
        }
        tabs.extend(detailed.map(|view| Tab::Pr { pr: view.pr, number: view.of(work).number }));
        if !timeline.is_empty() {
            tabs.push(Tab::Activity);
        }
        let jira = group.key.as_ref().map(|_| match ticket {
            Some(ticket) => JiraBox::Read(JiraFacts {
                status: ticket.status.clone(),
                hue: status_look(&ticket.status, Category::of(&ticket.category)).hue,
                issue_type: ticket.issue_type.clone().map(|name| {
                    let kind = type_kind(Some(&name));
                    (name, kind)
                }),
                priority: ticket.priority.clone().map(|name| {
                    let level = priority_level(Some(&name));
                    (name, level)
                }),
                assignee: ticket.assignee.clone().unwrap_or_else(|| "No one".into()),
                updated: ago(enkidu_core::work::parse::parse_time(&ticket.updated_at), now),
            }),
            None => JiraBox::NotRead,
        });
        CardModel {
            id: group.id.clone(),
            head,
            title: group.title.clone(),
            hidden: work.hidden.contains(&group.id),
            jira_url: ticket.map(|ticket| ticket.url.clone()).filter(|url| !url.is_empty()),
            story: !tabs.is_empty(),
            tabs,
            jira,
            prs: group.prs.iter().map(|view| PrFacts::of(view.of(work), view.pr, now)).collect(),
            timeline,
            own: group.own(work).map(|task| OwnFacts {
                number: task.id,
                project: task.project.clone(),
                state: task.state.clone(),
                notes: task.notes.clone(),
                made: ago(Some(task.created_at), now),
            }),
        }
    }

    /// The tab to show when `wanted` is asked for: that one if the card has it, else its first.
    pub fn tab(&self, wanted: Option<&Tab>) -> Option<&Tab> {
        wanted.and_then(|wanted| self.tabs.iter().find(|tab| *tab == wanted)).or(self.tabs.first())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::work::groups::build_groups;
    use crate::work::groups::tests::NOW;
    use crate::work::sample;
    use std::collections::HashMap;

    fn card(work: &Snapshot, id: &str) -> CardModel {
        let groups = build_groups(work, &[], &HashMap::new(), NOW);
        let group = groups.iter().find(|g| g.id == id).unwrap_or_else(|| panic!("no card {id}"));
        CardModel::of(group, work, NOW)
    }

    /// Opens a card in front of the board: the task, what needs you, Jira's status, the PR and how it stands.
    #[test]
    fn a_card_says_the_task_jiras_status_the_pr_and_how_it_stands() {
        let work = sample();
        let export = card(&work, "ticket:SH-21309");
        assert_eq!(export.title, "Export orders to the accounting CSV");
        assert_eq!(export.jira_url.as_deref(), Some("https://demo.atlassian.net/browse/SH-21309"));
        assert!(export.story && !export.hidden);
        let labels: Vec<String> = export.tabs.iter().map(Tab::label).collect();
        assert_eq!(labels, ["Overview", "PR #2414", "Activity"]);
        let Some(JiraBox::Read(jira)) = &export.jira else { panic!("{:?}", export.jira) };
        assert_eq!((jira.status.as_str(), jira.hue), ("In Progress", Some(258.)));
        assert_eq!(
            jira.issue_type.as_ref().map(|(name, kind)| (name.as_str(), *kind)),
            Some(("Story", TypeKind::Story))
        );
        assert_eq!(jira.priority.as_ref().map(|(name, level)| (name.as_str(), *level)), Some(("High", Some(1))));
        assert_eq!(jira.assignee, "Sam Lee");
        let pr = &export.prs[0];
        assert_eq!((pr.number, pr.state, pr.tone), (2414, PrState::Open, Tone::Green));
        assert_eq!((pr.head.as_str(), pr.base.as_str()), ("feature/SH-21309-export", "development"));
        assert_eq!(pr.checks, Some((Checks::Failing, "Failing: tests (3.12)".to_string())));
        assert_eq!(pr.review, Some(("Waiting for review".to_string(), Tone::Muted)));
        assert_eq!((pr.conflict.clone(), pr.merged.clone()), (None, None));
        // the tab asked for when the card has it, else its first
        let activity = Tab::Activity;
        assert_eq!(export.tab(Some(&activity)), Some(&Tab::Activity));
        assert_eq!(export.tab(None), Some(&Tab::Overview));
        assert_eq!(export.tab(Some(&Tab::Pr { pr: 99, number: 1 })), Some(&Tab::Overview));
        // one ticket, two PRs: both boxes, each in its own words
        let stock = card(&work, "ticket:SH-21533");
        let said: Vec<String> = (stock.prs.iter())
            .map(|pr| {
                let checks = pr.checks.as_ref().map_or("", |(_, words)| words.as_str());
                let review = pr.review.as_ref().map_or("", |(words, _)| words.as_str());
                format!("#{} {checks} | {review}", pr.number)
            })
            .collect();
        assert_eq!(said[1], "#2395 Passed | Changes requested");
        assert!(said[0].starts_with("#2413 Running"), "{said:?}");
        assert_eq!(stock.prs[1].review.as_ref().map(|(_, tone)| *tone), Some(Tone::Red));
        // a merged PR says when, in its state's color, and nothing of checks or review
        let done = card(&work, "ticket:SH-21030");
        let merged = &done.prs[0];
        assert_eq!((merged.state, merged.tone), (PrState::Merged, Tone::Violet));
        assert!(merged.merged.is_some() && merged.checks.is_none() && merged.review.is_none(), "{merged:?}");
    }

    /// Says a ticket has no PR yet, and gives a PR without a ticket its GitHub link once.
    #[test]
    fn a_ticket_without_a_pr_and_a_pr_without_a_ticket() {
        let work = sample();
        // a backlog ticket without a description that nothing happened to: nothing to tell on the left, the facts
        // alone
        let backlog = card(&work, "ticket:SH-20728");
        assert!(backlog.prs.is_empty() && !backlog.story && backlog.tabs.is_empty());
        assert!(matches!(backlog.jira, Some(JiraBox::Read(_))));
        // a PR that names no ticket: no Jira box and no link to Jira (its GitHub link is in its box, once)
        let lone = card(&work, "pr:demo-org/shop#2415");
        assert_eq!((lone.jira.clone(), lone.jira_url.clone()), (None, None));
        assert_eq!(lone.head, CardHead::Pr { state: PrState::Open, name: "shop#2415".into() });
        assert_eq!(lone.prs.len(), 1);
        assert_eq!(lone.prs[0].url, "https://github.com/demo-org/shop/pull/2415");
        // a key a PR names whose ticket wasn't read
        let mut unread = sample();
        unread.tickets.retain(|ticket| ticket.key != "SH-21309");
        let card = card(&unread, "ticket:SH-21309");
        assert_eq!((card.jira, card.jira_url), (Some(JiraBox::NotRead), None));
        // one of the cards you hid says so
        let mut hidden = sample();
        hidden.hidden = vec!["ticket:SH-21522".into()];
        assert!(self::card(&hidden, "ticket:SH-21522").hidden);
    }

    /// The timeline: what happened to the ticket and its PRs, the newest first, in words of time.
    #[test]
    fn the_timeline_is_the_tickets_and_its_prs_events() {
        let work = sample();
        let export = card(&work, "ticket:SH-21309");
        assert!(!export.timeline.is_empty());
        let subjects = ["SH-21309", "demo-org/shop#2414"];
        let count = work.events.iter().filter(|event| subjects.contains(&event.subject.as_str())).count();
        assert_eq!(export.timeline.len(), count);
        assert!(
            export.timeline.iter().all(|(when, text)| !when.is_empty() && !text.is_empty()),
            "{:?}",
            export.timeline
        );
    }

    /// What needs you, in the card's order, and which item the keyboard is on.
    #[test]
    fn what_needs_you_is_said_as_a_person_would_and_the_keyboard_is_on_the_first() {
        let work = sample();
        let groups = build_groups(&work, &[], &HashMap::new(), NOW);
        let export = groups.iter().find(|g| g.id == "ticket:SH-21309").expect("the sample's card");
        let rows = item_rows(export, &work, NOW);
        let ids: Vec<&str> = rows.iter().map(|row| row.id.as_str()).collect();
        assert_eq!(ids, ["question:c9", "thread:t1", "checks:demo-org/shop#2414:4973abc0def1", "thread:t2"]);
        assert_eq!(rows[1].sentence, "mira-k left a remark on export.py, line 48");
        assert_eq!((rows[0].sentence.as_str(), rows[0].when.as_str()), ("jonas-qa asked a question", "14 min ago"));
        assert_eq!(
            rows[0].detail.as_deref(),
            Some("Does the export include orders that were cancelled after delivery?")
        );
        assert_eq!((rows[2].kind, rows[2].kind.severe()), (ItemKind::PipelineFailed, true));
        assert_eq!(rows[2].url.as_deref(), Some("https://ci.example.com/run/88123"));
        assert!(!rows[0].kind.severe() && !rows[0].bot);
        // a ticket's own items come after its PRs', in Jira's words
        let bundles = groups.iter().find(|g| g.id == "ticket:SH-21112").expect("the sample's card");
        let said: Vec<String> = item_rows(bundles, &work, NOW).into_iter().map(|row| row.sentence).collect();
        assert_eq!(said, ["Jonas Weber added a file in Jira", "Jonas Weber replied in Jira"]);
        // the keyboard: on the first as the card opens; where you moved it; on the one that took its place
        assert_eq!(current_item(4, None, 0), Some(0));
        assert_eq!(current_item(4, Some(2), 0), Some(2));
        assert_eq!(current_item(3, None, 2), Some(2), "the item under it went: the next took its place");
        assert_eq!(current_item(2, None, 2), Some(1), "the last one went: the one before it");
        assert_eq!(current_item(0, None, 0), None);
        // j and k (the arrows) move through them and stay at the ends
        assert_eq!(step_item(4, Some(0), 1), Some(1));
        assert_eq!(step_item(4, Some(3), 1), Some(3));
        assert_eq!(step_item(4, Some(0), -1), Some(0));
        assert_eq!(step_item(4, None, 1), Some(0));
        assert_eq!(step_item(0, None, 1), None);
    }

    /// A PR's box in every state it can be in.
    #[test]
    fn a_prs_box_says_its_checks_review_conflict_threads_and_when_it_was_merged() {
        let base = crate::work::pr::tests::pr(7);
        let some = |text: &str| Some(text.to_string());
        let facts = |pr: Pr| PrFacts::of(&pr, 0, NOW);
        let plain = facts(base.clone());
        assert_eq!(plain.checks, Some((Checks::Passed, "Passed".to_string())));
        assert_eq!(plain.review, Some(("No review needed".to_string(), Tone::Muted)));
        let draft = facts(Pr { draft: true, running: 2, checks: some("PENDING"), ..base.clone() });
        assert_eq!((draft.state, draft.tone), (PrState::Draft, Tone::Muted));
        assert_eq!(draft.checks, Some((Checks::Running, "Running (2)".to_string())));
        assert_eq!(draft.review.map(|(words, _)| words).as_deref(), Some("Draft, not asked for review yet"));
        let approved = facts(Pr { review_decision: some("APPROVED"), unresolved: 2, ..base.clone() });
        assert_eq!(approved.review, Some(("Approved".to_string(), Tone::Green)));
        assert_eq!(approved.threads.as_deref(), Some("2 open threads"));
        let conflict = facts(Pr { mergeable: some("CONFLICTING"), unresolved: 1, checks: None, ..base.clone() });
        assert_eq!(conflict.conflict.as_deref(), Some("Conflicts with development"));
        assert_eq!((conflict.threads.as_deref(), conflict.checks), (Some("1 open thread"), None));
        let failing = facts(Pr { checks: some("FAILURE"), ..base.clone() });
        assert_eq!(failing.checks, Some((Checks::Failing, "Failing".to_string())));
        let merged_at = "2026-11-05T10:00:00Z";
        let merged = facts(Pr { state: "MERGED".into(), merged_at: some(merged_at), unresolved: 3, ..base.clone() });
        assert_eq!((merged.merged.as_deref(), merged.threads), (Some("3 days ago"), None));
        let closed = facts(Pr { state: "CLOSED".into(), ..base });
        assert_eq!((closed.state, closed.tone, closed.review), (PrState::Closed, Tone::Muted, None));
    }
}
