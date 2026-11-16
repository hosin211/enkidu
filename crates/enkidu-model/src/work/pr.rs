//! The Work index, read for the screen: which PRs belong to a workspace, how a PR's checks stand, and what waits on
//! you, said in a few words.

use std::collections::HashSet;

use enkidu_core::work::model::{Item, Pr, Snapshot};

/// Items that wait on you now (open, not snoozed).
pub fn waiting<'a>(items: impl IntoIterator<Item = &'a Item>) -> Vec<&'a Item> {
    items.into_iter().filter(|item| item.state == "open").collect()
}

/// The PRs of a workspace: those on its branch, and those its own Claude made or worked on. Open ones first (the one
/// on its own branch before the ones its Claude made on other branches), then the most recently updated.
pub fn prs_of_workspace<'a>(work: Option<&'a Snapshot>, workspace_id: &str, session_id: Option<&str>) -> Vec<&'a Pr> {
    let Some(work) = work else { return Vec::new() };
    let (workspace, session) = (format!("workspace:{workspace_id}"), session_id.map(|id| format!("session:{id}")));
    let mut own: HashSet<&str> = HashSet::new();
    let mut by_session: HashSet<&str> = HashSet::new();
    for link in &work.links {
        let Some(reference) = link.a.strip_prefix("pr:") else { continue };
        if link.b == workspace {
            own.insert(reference);
        } else if session.as_deref() == Some(link.b.as_str()) {
            by_session.insert(reference);
        }
    }
    let order = |pr: &Pr, own: bool| {
        let state = match pr.state.as_str() {
            "OPEN" => 0,
            "MERGED" => 2,
            _ => 4,
        };
        state + if own { 0 } else { 1 }
    };
    let mut prs: Vec<(&Pr, i32)> = work
        .prs
        .iter()
        .filter_map(|pr| {
            let reference = pr.reference();
            let own = own.contains(reference.as_str());
            (own || by_session.contains(reference.as_str())).then(|| (pr, order(pr, own)))
        })
        .collect();
    prs.sort_by(|(x, x_order), (y, y_order)| x_order.cmp(y_order).then_with(|| y.updated_at.cmp(&x.updated_at)));
    prs.into_iter().map(|(pr, _)| pr).collect()
}

/// The PR a workspace's tab shows: the one that waits on you most, else its own open one.
pub fn tab_pr<'a>(work: Option<&'a Snapshot>, workspace_id: &str, session_id: Option<&str>) -> Option<&'a Pr> {
    workspace_line(work, workspace_id, session_id)
        .map(|line| line.pr)
        .or_else(|| prs_of_workspace(work, workspace_id, session_id).first().copied())
}

/// How the latest commit's checks stand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Checks {
    Failing,
    /// Pending ones are running too.
    Running,
    Passed,
    /// Nothing reported.
    None,
}

pub fn checks_of(pr: &Pr) -> Checks {
    let checks = pr.checks.as_deref();
    if !pr.failing.is_empty() {
        return Checks::Failing;
    }
    if pr.running > 0 || matches!(checks, Some("PENDING" | "EXPECTED")) {
        return Checks::Running;
    }
    match checks {
        Some("SUCCESS") => Checks::Passed,
        Some("FAILURE" | "ERROR") => Checks::Failing,
        _ => Checks::None,
    }
}

/// The lamp color of a PR: merged and closed ones by their state, open ones by their checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrLamp {
    Red,
    Amber,
    Green,
    Violet,
    Gray,
}

pub fn pr_lamp(pr: &Pr) -> PrLamp {
    match pr.state.as_str() {
        "MERGED" => PrLamp::Violet,
        "CLOSED" => PrLamp::Gray,
        _ => match checks_of(pr) {
            Checks::Failing => PrLamp::Red,
            Checks::Running => PrLamp::Amber,
            Checks::Passed => PrLamp::Green,
            Checks::None => PrLamp::Gray,
        },
    }
}

/// Worst first: what blocks the PR, then what someone waits for you to answer.
fn rank(kind: &str) -> u8 {
    match kind {
        "pipeline_failed" => 0,
        "conflict" => 1,
        "back_to_you" => 2,
        "changes_requested" => 3,
        "qa_replied" => 4,
        "mentioned" => 5,
        "remark" => 6,
        "probably_fixed" => 7,
        "question" => 8,
        _ => 9,
    }
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// The waiting items of a PR (or a ticket), as a few words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    /// "checks failing, 2 remarks waiting"
    pub text: String,
    /// Something blocks: failing checks, a conflict, back to you, changes requested.
    pub severe: bool,
}

/// What waits, worst first. Bots, closed and snoozed items stay out; nothing waiting says nothing.
pub fn summary<'a>(items: impl IntoIterator<Item = &'a Item>) -> Option<Summary> {
    let mine: Vec<&Item> = waiting(items).into_iter().filter(|item| !item.bot).collect();
    if mine.is_empty() {
        return None;
    }
    let count = |kinds: &[&str]| mine.iter().filter(|item| kinds.contains(&item.kind.as_str())).count();
    let mut parts: Vec<(u8, String)> = Vec::new();
    let mut say = |kind: &str, words: String| parts.push((rank(kind), words));
    if count(&["pipeline_failed"]) > 0 {
        say("pipeline_failed", "checks failing".into());
    }
    if count(&["conflict"]) > 0 {
        say("conflict", "conflicts with its base".into());
    }
    if count(&["back_to_you"]) > 0 {
        say("back_to_you", "back to you in Jira".into());
    }
    if count(&["changes_requested"]) > 0 {
        say("changes_requested", "changes requested".into());
    }
    if count(&["qa_replied"]) > 0 {
        say("qa_replied", plural(count(&["qa_replied"]), "QA reply", "QA replies"));
    }
    if count(&["mentioned"]) > 0 {
        say("mentioned", "mentioned in Jira".into());
    }
    let remarks = count(&["remark", "probably_fixed"]);
    if remarks > 0 {
        say("remark", format!("{} waiting", plural(remarks, "remark", "remarks")));
    }
    if count(&["question"]) > 0 {
        say("question", plural(count(&["question"]), "question", "questions"));
    }
    if count(&["attachment"]) > 0 {
        say("attachment", plural(count(&["attachment"]), "new attachment", "new attachments"));
    }
    parts.sort_by_key(|(rank, _)| *rank);
    let severe = parts.first().is_some_and(|(first, _)| *first <= rank("changes_requested"));
    let text = parts.into_iter().map(|(_, words)| words).collect::<Vec<_>>().join(", ");
    Some(Summary { text, severe })
}

/// What the status line says for a workspace: its most pressing PR and what waits there.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkspaceLine<'a> {
    pub pr: &'a Pr,
    /// "PR #2395: checks failing"
    pub text: String,
    pub severe: bool,
    /// How many more of its PRs have something waiting.
    pub more: usize,
}

pub fn workspace_line<'a>(
    work: Option<&'a Snapshot>,
    workspace_id: &str,
    session_id: Option<&str>,
) -> Option<WorkspaceLine<'a>> {
    let work = work?;
    let mut lines: Vec<(&Pr, Summary)> = prs_of_workspace(Some(work), workspace_id, session_id)
        .into_iter()
        .filter_map(|pr| {
            let reference = pr.reference();
            let said = summary(work.items.iter().filter(|item| item.pr.as_deref() == Some(reference.as_str())))?;
            Some((pr, said))
        })
        .collect();
    // a stable sort: among equally pressing ones the workspace's own PR (the first) wins
    lines.sort_by_key(|(_, said)| !said.severe);
    let more = lines.len().saturating_sub(1);
    let (pr, said) = lines.into_iter().next()?;
    Some(WorkspaceLine { pr, text: format!("PR #{}: {}", pr.number, said.text), severe: said.severe, more })
}

/// A workspace's PR as its tab shows it, quietly: its checks as a dot and its number, blue when something on it
/// waits on you.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabPr {
    pub number: i64,
    pub lamp: PrLamp,
    /// "PR #2395: checks failing, 2 remarks waiting", or with nothing waiting the PR's own title.
    pub title: String,
    /// Something on it waits on you.
    pub waiting: bool,
}

/// The PR a workspace's tab shows, if it has one.
pub fn tab_chip(work: Option<&Snapshot>, workspace_id: &str, session_id: Option<&str>) -> Option<TabPr> {
    let pr = tab_pr(work, workspace_id, session_id)?;
    let reference = pr.reference();
    let waits = summary(work?.items.iter().filter(|item| item.pr.as_deref() == Some(reference.as_str())));
    Some(TabPr {
        number: pr.number,
        lamp: pr_lamp(pr),
        title: format!("PR #{}: {}", pr.number, waits.as_ref().map_or(pr.title.as_str(), |waits| waits.text.as_str())),
        waiting: waits.is_some(),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use enkidu_core::work::model::{Check, Link};

    /// Shows a workspace's PR on its tab, with its checks, and blue when something waits on you (what the tab says;
    /// where it is drawn is enkidu-app's tests/tabs_row.rs).
    #[test]
    fn a_workspaces_tab_shows_its_pr_with_its_checks_and_that_something_waits_on_you() {
        let failed = Check { name: "tests".into(), state: "FAILURE".into(), url: None };
        let failing =
            Pr { head: "feature/export".into(), checks: Some("FAILURE".into()), failing: vec![failed], ..pr(2414) };
        let work = Snapshot {
            prs: vec![Pr { head: "feature/stock-fix".into(), ..pr(2395) }, failing],
            links: vec![
                link("pr:Org/shop#2395", "workspace:w1", "branch"),
                link("pr:Org/shop#2414", "workspace:w2", "branch"),
            ],
            items: vec![Item {
                title: "Checks failing: tests".into(),
                who: None,
                ..item("pipeline_failed", "Org/shop#2414")
            }],
            ..Default::default()
        };
        let stock = tab_chip(Some(&work), "w1", Some("s-w1")).expect("stock-fix has a PR");
        assert_eq!((stock.number, stock.title.as_str()), (2395, "PR #2395: PR 2395"));
        assert_eq!((stock.lamp, stock.waiting), (PrLamp::Green, false));
        let export = tab_chip(Some(&work), "w2", Some("s-w2")).expect("export has a PR");
        assert_eq!((export.lamp, export.waiting), (PrLamp::Red, true));
        assert_eq!(export.title, "PR #2414: checks failing");
        // a workspace without a PR, and a window that read nothing yet, show none
        assert_eq!(tab_chip(Some(&work), "w3", None), None);
        assert_eq!(tab_chip(None, "w1", None), None);
    }

    /// The PR of these tests: `Org/shop#number`, open, checks passed.
    pub(crate) fn pr(number: i64) -> Pr {
        Pr {
            project_id: "p1".into(),
            repo: "Org/shop".into(),
            number,
            title: format!("PR {number}"),
            url: format!("https://github.com/Org/shop/pull/{number}"),
            state: "OPEN".into(),
            draft: false,
            head: format!("feature/{number}"),
            base: "development".into(),
            review_decision: None,
            checks: Some("SUCCESS".into()),
            failing: Vec::new(),
            running: 0,
            mergeable: Some("MERGEABLE".into()),
            head_sha: Some("abc".into()),
            head_at: Some("2026-11-05T10:00:00Z".into()),
            unresolved: 0,
            updated_at: "2026-11-05T10:00:00Z".into(),
            merged_at: None,
            commits: Vec::new(),
            commit_keys: Vec::new(),
            last_comment_by: None,
            last_comment_at: None,
            details: Default::default(),
        }
    }

    /// The item of these tests: one of `kind` on a PR, from a reviewer, open.
    pub(crate) fn item(kind: &str, pr: &str) -> Item {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        Item {
            id: format!("{kind}:{}", NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)),
            kind: kind.into(),
            source: "github".into(),
            project_id: "p1".into(),
            pr: Some(pr.into()),
            ticket: None,
            title: kind.into(),
            detail: String::new(),
            who: Some("reviewer".into()),
            url: None,
            path: None,
            line: None,
            at: "2026-11-05T11:00:00Z".into(),
            state: "open".into(),
            opened_at: 1,
            closed_at: None,
            snoozed_until: None,
            bot: false,
        }
    }

    pub(crate) fn link(a: &str, b: &str, how: &str) -> Link {
        Link { a: a.into(), b: b.into(), how: how.into(), detail: String::new(), at: 1, removed: false }
    }

    fn numbers(prs: Vec<&Pr>) -> Vec<i64> {
        prs.into_iter().map(|pr| pr.number).collect()
    }

    /// Finds a workspace's PRs by its branch and by what its Claude made, open ones first.
    #[test]
    fn finds_a_workspaces_prs_by_its_branch_and_by_what_its_claude_made_open_ones_first() {
        let merged = Pr { state: "MERGED".into(), updated_at: "2026-11-06T00:00:00Z".into(), ..pr(1) };
        let work = Snapshot {
            prs: vec![merged, pr(2), pr(3), pr(4)],
            links: vec![
                link("pr:Org/shop#1", "workspace:w1", "branch"),
                link("pr:Org/shop#2", "workspace:w1", "branch"),
                link("pr:Org/shop#3", "session:s1", "hook:pr-create"),
                link("pr:Org/shop#4", "workspace:w2", "branch"),
            ],
            ..Default::default()
        };
        assert_eq!(numbers(prs_of_workspace(Some(&work), "w1", None)), [2, 1]);
        assert_eq!(numbers(prs_of_workspace(Some(&work), "w1", Some("s1"))), [2, 3, 1], "its own branch's PR first");
        assert!(prs_of_workspace(None, "w1", None).is_empty());
    }

    /// Tells a PR's checks and lamp.
    #[test]
    fn tells_a_prs_checks_and_lamp() {
        assert_eq!(checks_of(&pr(1)), Checks::Passed);
        let failed = Check { name: "tests".into(), state: "FAILURE".into(), url: None };
        assert_eq!(checks_of(&Pr { failing: vec![failed], checks: Some("FAILURE".into()), ..pr(1) }), Checks::Failing);
        assert_eq!(checks_of(&Pr { checks: Some("PENDING".into()), ..pr(1) }), Checks::Running);
        assert_eq!(checks_of(&Pr { checks: None, ..pr(1) }), Checks::None);
        assert_eq!(pr_lamp(&Pr { state: "MERGED".into(), checks: Some("FAILURE".into()), ..pr(1) }), PrLamp::Violet);
        assert_eq!(pr_lamp(&Pr { running: 2, checks: Some("PENDING".into()), ..pr(1) }), PrLamp::Amber);
        // the rest of both
        assert_eq!(checks_of(&Pr { checks: Some("ERROR".into()), ..pr(1) }), Checks::Failing);
        assert_eq!(checks_of(&Pr { checks: Some("EXPECTED".into()), ..pr(1) }), Checks::Running);
        assert_eq!(pr_lamp(&Pr { state: "CLOSED".into(), ..pr(1) }), PrLamp::Gray);
        assert_eq!((pr_lamp(&pr(1)), pr_lamp(&Pr { checks: None, ..pr(1) })), (PrLamp::Green, PrLamp::Gray));
        assert_eq!(pr_lamp(&Pr { checks: Some("FAILURE".into()), ..pr(1) }), PrLamp::Red);
    }

    /// Says what waits, worst first, and leaves bots and closed items out.
    #[test]
    fn says_what_waits_worst_first_and_leaves_bots_and_closed_items_out() {
        let r = "Org/shop#2";
        assert_eq!(summary(&[]), None);
        let items = [
            item("remark", r),
            item("probably_fixed", r),
            item("question", r),
            item("pipeline_failed", r),
            Item { bot: true, ..item("question", r) },
            Item { state: "closed".into(), ..item("remark", r) },
            Item { state: "snoozed".into(), ..item("remark", r) },
        ];
        let said = summary(&items).unwrap();
        assert_eq!((said.text.as_str(), said.severe), ("checks failing, 2 remarks waiting, 1 question", true));
        let one = [item("remark", r)];
        assert_eq!(summary(&one), Some(Summary { text: "1 remark waiting".into(), severe: false }));
        // every kind has its words, the worst first
        let kinds = [
            "attachment",
            "question",
            "question",
            "mentioned",
            "qa_replied",
            "qa_replied",
            "changes_requested",
            "back_to_you",
            "conflict",
            "attachment",
        ];
        let all: Vec<Item> = kinds.iter().map(|kind| item(kind, r)).collect();
        assert_eq!(
            summary(&all).unwrap().text,
            "conflicts with its base, back to you in Jira, changes requested, 2 QA replies, mentioned in Jira, \
             2 questions, 2 new attachments"
        );
        let reply = [item("qa_replied", r), item("attachment", r)];
        assert_eq!(summary(&reply), Some(Summary { text: "1 QA reply, 1 new attachment".into(), severe: false }));
    }

    /// Gives the status line the most pressing PR of the workspace.
    #[test]
    fn gives_the_status_line_the_most_pressing_pr_of_the_workspace() {
        let work = Snapshot {
            prs: vec![pr(2), pr(3)],
            links: vec![
                link("pr:Org/shop#2", "workspace:w1", "branch"),
                link("pr:Org/shop#3", "workspace:w1", "branch"),
            ],
            items: vec![item("remark", "Org/shop#2"), item("changes_requested", "Org/shop#3")],
            ..Default::default()
        };
        let line = workspace_line(Some(&work), "w1", None).unwrap();
        assert_eq!(
            line,
            WorkspaceLine { pr: &work.prs[1], text: "PR #3: changes requested".into(), severe: true, more: 1 }
        );
        assert_eq!(workspace_line(Some(&work), "w9", None), None);
    }

    /// TabPr: shows the PR that waits on you, else the workspace's own.
    #[test]
    fn a_tab_shows_the_pr_that_waits_on_you_else_the_workspaces_own() {
        let prs = vec![
            Pr { updated_at: "2026-11-05T09:00:00Z".into(), ..pr(2395) },
            Pr { updated_at: "2026-11-05T12:00:00Z".into(), checks: Some("PENDING".into()), running: 2, ..pr(2413) },
        ];
        let links =
            vec![link("pr:Org/shop#2395", "workspace:w1", "branch"), link("pr:Org/shop#2413", "session:s1", "branch")];
        let base = Snapshot { prs, links, ..Default::default() };
        let own = tab_pr(Some(&base), "w1", Some("s1")).map(|pr| pr.number);
        assert_eq!(own, Some(2395), "its own branch's PR, though the other is newer");
        let waiting = Snapshot { items: vec![item("question", "Org/shop#2413")], ..base.clone() };
        let asked = tab_pr(Some(&waiting), "w1", Some("s1")).map(|pr| pr.number);
        assert_eq!(asked, Some(2413), "the one where someone waits on you");
        assert_eq!(tab_pr(Some(&Snapshot::default()), "w1", None), None);
    }
}
