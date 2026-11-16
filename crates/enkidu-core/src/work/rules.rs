//! What waits on you (WORK-INDEX.md, "What waiting on you means"): the rules that turn one full read of GitHub or
//! Jira into items. Each item's id says what it is and where it came from, so the next read updates it instead of
//! adding a copy; `merge` opens, updates, reopens and closes the stored items.

use anyhow::Result;
use std::collections::HashMap;

use super::github::{Comment, OpenPr};
use super::model::{Item, ItemDraft, Pr};
use super::parse::parse_time;
use crate::db::Db;

/// The first few lines of a comment, for the page.
pub fn excerpt(text: &str, max: usize) -> String {
    let flat: String = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("<!--"))
        .collect::<Vec<_>>()
        .join(" ");
    if flat.chars().count() <= max {
        flat
    } else {
        format!("{}…", flat.chars().take(max).collect::<String>().trim_end())
    }
}

fn time(s: &str) -> i64 {
    parse_time(s).unwrap_or(0)
}

fn is_me(c: &Comment, me: &str) -> bool {
    c.author.as_deref().is_some_and(|a| a.eq_ignore_ascii_case(me))
}

fn who(c: &Comment) -> String {
    c.author.clone().unwrap_or_else(|| "someone".into())
}

/// The GitHub rules for one open PR of yours. `me` is the login GitHub read as; checks named in `ignored` never
/// count as a failed pipeline.
pub fn github_items(o: &OpenPr, me: &str, ignored: &[String]) -> Vec<ItemDraft> {
    let pr = &o.pr;
    let reference = pr.reference();
    let mut out = Vec::new();
    let draft = |id: String, kind: &'static str, title: String, detail: String| ItemDraft {
        id,
        kind,
        pr: Some(reference.clone()),
        ticket: None,
        title,
        detail,
        who: None,
        url: None,
        path: None,
        line: None,
        at: pr.updated_at.clone(),
        bot: false,
    };

    // review threads: a remark waiting, or probably fixed by your push and waiting for a reply
    for t in o.threads.iter().filter(|t| !t.resolved) {
        let Some(last) = t.comments.last() else { continue };
        if is_me(last, me) {
            continue; // you answered: it waits on them
        }
        let replied = t.comments.iter().any(|c| is_me(c, me));
        let fixed = t.outdated && !replied;
        let place = match (&t.path, t.line) {
            (Some(p), Some(l)) => format!(" on {p}:{l}"),
            (Some(p), None) => format!(" on {p}"),
            _ => String::new(),
        };
        let mut d = draft(
            format!("thread:{}", t.id),
            if fixed { "probably_fixed" } else { "remark" },
            if fixed { format!("Probably fixed, needs a reply: {}{place}", who(last)) } else { format!("Remark from {}{place}", who(last)) },
            excerpt(&last.body, 280),
        );
        d.who = Some(who(last));
        d.url = Some(last.url.clone());
        d.path = t.path.clone();
        d.line = t.line;
        d.at = last.at.clone();
        d.bot = t.comments.first().is_some_and(|c| c.bot);
        out.push(d);
    }

    // a question in the conversation since you last spoke or pushed
    let mine_last = o
        .comments
        .iter()
        .filter(|c| is_me(c, me))
        .map(|c| time(&c.at))
        .chain(pr.head_at.as_deref().map(time))
        .max()
        .unwrap_or(0);
    let at_me = format!("@{}", me.to_lowercase());
    for c in o.comments.iter().filter(|c| !is_me(c, me) && time(&c.at) > mine_last) {
        let asks = c.body.contains('?') || c.body.to_lowercase().contains(&at_me);
        if !asks {
            continue;
        }
        let mut d = draft(format!("question:{}", c.id), "question", format!("Question from {}", who(c)), excerpt(&c.body, 280));
        d.who = Some(who(c));
        d.url = Some(c.url.clone());
        d.at = c.at.clone();
        d.bot = c.bot;
        out.push(d);
    }

    // changes requested, and no push since
    if pr.review_decision.as_deref() == Some("CHANGES_REQUESTED") {
        let latest = o.reviews.iter().filter(|r| r.state == "CHANGES_REQUESTED").max_by_key(|r| r.at.as_deref().map(time).unwrap_or(0));
        if let Some(r) = latest {
            let at = r.at.as_deref().map(time).unwrap_or(0);
            let pushed_since = pr.head_at.as_deref().map(time).unwrap_or(0) > at;
            if !pushed_since {
                let name = r.author.clone().unwrap_or_else(|| "a reviewer".into());
                let mut d = draft(format!("changes:{}", r.id), "changes_requested", format!("Changes requested by {name}"), String::new());
                d.who = Some(name);
                d.url = Some(format!("{}#pullrequestreview", pr.url));
                d.at = r.at.clone().unwrap_or_else(|| pr.updated_at.clone());
                d.bot = r.bot;
                out.push(d);
            }
        }
    }

    // a failed check on the latest commit (pending ones and older commits don't count)
    let failing: Vec<_> = pr.failing.iter().filter(|c| !ignored.iter().any(|i| i.eq_ignore_ascii_case(&c.name))).collect();
    if !failing.is_empty() {
        let names: Vec<String> = failing.iter().map(|c| c.name.clone()).collect();
        let states: Vec<String> = failing.iter().map(|c| format!("{}: {}", c.name, c.state.to_lowercase().replace('_', " "))).collect();
        let sha = pr.head_sha.clone().unwrap_or_default();
        let mut d = draft(format!("checks:{reference}:{sha}"), "pipeline_failed", format!("Checks failing: {}", names.join(", ")), states.join("\n"));
        d.url = failing.iter().find_map(|c| c.url.clone()).or_else(|| Some(format!("{}/checks", pr.url)));
        d.at = pr.head_at.clone().unwrap_or_else(|| pr.updated_at.clone());
        out.push(d);
    }

    // can't merge into its base (GitHub's UNKNOWN means "not worked out yet": no change)
    if pr.mergeable.as_deref() == Some("CONFLICTING") {
        let mut d = draft(format!("conflict:{reference}"), "conflict", format!("Conflicts with {}", pr.base), String::new());
        d.url = Some(pr.url.clone());
        out.push(d);
    }
    out
}

/// How far back Jira comments and attachments count as new.
const JIRA_DAYS: i64 = 30;

/// The usual way a ticket goes, for telling a move back ("QA Review" → "In Progress").
const FLOW: [&str; 21] = [
    "backlog", "new", "to do", "open", "selected for development", "ready", "ready for dev", "in progress", "in development",
    "code review", "in review", "review", "qa", "qa review", "in qa", "testing", "ready for release", "done", "resolved", "closed", "live",
];

/// Did a ticket come back to you? With "back to you" statuses set for the project, moving into one of them does;
/// else moving back along the usual way (or to Reopened) does.
pub fn moved_back(from: &str, to: &str, back_to_you: &[String]) -> bool {
    if from.eq_ignore_ascii_case(to) {
        return false;
    }
    if !back_to_you.is_empty() {
        return back_to_you.iter().any(|s| s.trim().eq_ignore_ascii_case(to));
    }
    if to.eq_ignore_ascii_case("reopened") {
        return true;
    }
    let pos = |s: &str| FLOW.iter().position(|f| f.eq_ignore_ascii_case(s.trim()));
    matches!((pos(from), pos(to)), (Some(a), Some(b)) if b < a)
}

/// The Jira rules for one ticket read whole. `me` is your account id; `old_status` is its status at the last read;
/// `back` is an open "back to you" item's move (from, to), kept while the ticket stays there.
pub fn jira_items(
    t: &super::jira::FullTicket,
    me: &str,
    old_status: Option<&str>,
    back: Option<(&str, &str)>,
    back_to_you: &[String],
    now: i64,
) -> Vec<ItemDraft> {
    let ticket = &t.ticket;
    let mut out = Vec::new();
    let since = now - JIRA_DAYS * 86_400_000;
    let mine = |id: Option<&str>| id == Some(me);
    let my_last = t.comments.iter().filter(|c| mine(c.author.as_ref().map(|a| a.id.as_str()))).map(|c| time(&c.created)).max().unwrap_or(0);
    let done = ticket.category == "done";
    let draft = |id: String, kind: &'static str, title: String, detail: String, who: Option<String>, at: String, bot: bool| ItemDraft {
        id,
        kind,
        pr: None,
        ticket: Some(ticket.key.clone()),
        title,
        detail,
        who,
        url: Some(ticket.url.clone()),
        path: None,
        line: None,
        at,
        bot,
    };
    for c in &t.comments {
        let author = c.author.as_ref();
        let at = time(&c.created);
        if mine(author.map(|a| a.id.as_str())) || at <= my_last || at < since {
            continue;
        }
        let name = author.map(|a| a.name.clone()).unwrap_or_else(|| "Someone".into());
        let bot = author.is_some_and(|a| a.bot);
        let mut d = if c.mentions.iter().any(|m| m == me) {
            draft(format!("jira-mention:{}:{}", ticket.key, c.id), "mentioned", format!("{name} mentioned you on {}", ticket.key), excerpt(&c.text, 280), Some(name), c.created.clone(), bot)
        } else if !done {
            draft(format!("jira-comment:{}:{}", ticket.key, c.id), "qa_replied", format!("{name} replied on {}", ticket.key), excerpt(&c.text, 280), Some(name), c.created.clone(), bot)
        } else {
            continue;
        };
        d.url = Some(format!("{}?focusedCommentId={}", ticket.url, c.id));
        out.push(d);
    }
    if !done {
        for a in &t.attachments {
            let at = time(&a.created);
            let by = a.author.as_ref();
            if mine(by.map(|p| p.id.as_str())) || at <= my_last || at < since {
                continue;
            }
            let name = by.map(|p| p.name.clone()).unwrap_or_else(|| "Someone".into());
            out.push(draft(
                format!("jira-attachment:{}:{}", ticket.key, a.id),
                "attachment",
                format!("New attachment on {}: {}", ticket.key, a.filename),
                format!("{name} attached {}", a.filename),
                Some(name),
                a.created.clone(),
                by.is_some_and(|p| p.bot),
            ));
        }
    }
    let assigned = mine(ticket.assignee_id.as_deref());
    let moved = old_status.filter(|from| moved_back(from, &ticket.status, back_to_you)).map(|from| (from.to_string(), ticket.status.clone()));
    let kept = back.filter(|(_, to)| to.eq_ignore_ascii_case(&ticket.status)).map(|(f, t)| (f.to_string(), t.to_string()));
    if let (true, Some((from, to))) = (assigned, moved.or(kept)) {
        out.push(draft(
            format!("jira-back:{}:{from}>{to}", ticket.key),
            "back_to_you",
            format!("{} is back to you: {from} → {to}", ticket.key),
            ticket.summary.clone(),
            None,
            ticket.updated_at.clone(),
            false,
        ));
    }
    out
}

/// What changed when a read's items were merged with the stored ones.
#[derive(Debug, Default, PartialEq)]
pub struct Merged {
    pub opened: Vec<String>,
    pub reopened: Vec<String>,
    pub closed: Vec<String>,
}

/// Merges one full read's items for a project and source (`github` or `jira`) into the stored ones: new ones open,
/// ones whose reason went away close, a closed one whose reason came back opens again, a done one opens again only
/// when something newer happened, a snoozed one wakes when its time is up. Every change goes into the history.
/// `keep` names items the read didn't cover (a PR it couldn't see this time): they stay as they are.
pub fn merge(db: &Db, project_id: &str, source: &str, drafts: Vec<ItemDraft>, now: i64, keep: &dyn Fn(&Item) -> bool) -> Result<Merged> {
    let mut merged = Merged::default();
    let existing: HashMap<String, Item> = db
        .work_items()?
        .into_iter()
        .filter(|i| i.project_id == project_id && i.source == source)
        .map(|i| (i.id.clone(), i))
        .collect();
    let subject = |i: &ItemDraft| i.ticket.clone().or_else(|| i.pr.clone()).unwrap_or_default();
    let mut seen = std::collections::HashSet::new();
    for d in drafts {
        seen.insert(d.id.clone());
        let old = existing.get(&d.id);
        let newer = old.is_some_and(|o| time(&d.at) > time(&o.at));
        let (state, opened_at, closed_at, snoozed_until) = match old {
            None => {
                merged.opened.push(d.id.clone());
                db.work_event(&subject(&d), "item-opened", &d.title, now)?;
                ("open".to_string(), now, None, None)
            }
            Some(o) => match o.state.as_str() {
                "closed" => {
                    merged.reopened.push(d.id.clone());
                    db.work_event(&subject(&d), "item-reopened", &d.title, now)?;
                    ("open".into(), now, None, None)
                }
                "done" if newer => {
                    merged.reopened.push(d.id.clone());
                    db.work_event(&subject(&d), "item-reopened", &d.title, now)?;
                    ("open".into(), now, None, None)
                }
                "snoozed" if o.snoozed_until.is_some_and(|u| u <= now) || newer => ("open".into(), o.opened_at, None, None),
                s => (s.to_string(), o.opened_at, o.closed_at, o.snoozed_until),
            },
        };
        db.work_put_item(&Item {
            id: d.id.clone(),
            kind: d.kind.to_string(),
            source: source.into(),
            project_id: project_id.into(),
            pr: d.pr.clone(),
            ticket: d.ticket.clone(),
            title: d.title.clone(),
            detail: d.detail.clone(),
            who: d.who.clone(),
            url: d.url.clone(),
            path: d.path.clone(),
            line: d.line,
            at: d.at.clone(),
            state,
            opened_at,
            closed_at,
            snoozed_until,
            bot: d.bot,
        })?;
    }
    for (id, old) in existing {
        if seen.contains(&id) || keep(&old) || !matches!(old.state.as_str(), "open" | "snoozed") {
            continue;
        }
        merged.closed.push(id);
        db.work_event(old.ticket.as_deref().or(old.pr.as_deref()).unwrap_or(""), "item-closed", &old.title, now)?;
        db.work_put_item(&Item { state: "closed".into(), closed_at: Some(now), snoozed_until: None, ..old })?;
    }
    Ok(merged)
}

/// Timeline events from how a PR changed between two reads.
pub fn pr_events(old: Option<&Pr>, new: &Pr) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    let Some(old) = old else {
        out.push(("pr-seen", format!("PR #{} {} → {}", new.number, new.head, new.base)));
        return out;
    };
    if old.head_sha != new.head_sha {
        if let Some(sha) = &new.head_sha {
            out.push(("pushed", format!("Pushed {}", &sha[..sha.len().min(7)])));
        }
    }
    if old.state != new.state {
        out.push(("pr-state", match new.state.as_str() {
            "MERGED" => format!("Merged into {}", new.base),
            "CLOSED" => "Closed".to_string(),
            s => format!("Now {}", s.to_lowercase()),
        }));
    }
    if old.review_decision != new.review_decision {
        if let Some(r) = &new.review_decision {
            out.push(("review", match r.as_str() {
                "APPROVED" => "Approved".to_string(),
                "CHANGES_REQUESTED" => "Changes requested".to_string(),
                _ => "Waiting for review".to_string(),
            }));
        }
    }
    let failed = |p: &Pr| !p.failing.is_empty();
    if old.checks != new.checks || failed(old) != failed(new) {
        match new.checks.as_deref() {
            Some("SUCCESS") => out.push(("checks", "Checks passed".to_string())),
            Some("FAILURE" | "ERROR") if failed(new) => {
                let names: Vec<&str> = new.failing.iter().map(|c| c.name.as_str()).collect();
                out.push(("checks", format!("Checks failed: {}", names.join(", "))));
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::work::github::tests::fixture;
    use crate::work::github::{parse_open_page, Review, Thread};
    use crate::work::links::tests::pr;
    use crate::work::model::Check;
    use crate::work::store::tests::db;

    fn comment(id: &str, author: &str, body: &str, at: &str) -> Comment {
        Comment { id: id.into(), author: Some(author.into()), bot: author.ends_with("[bot]") || author == "coderabbitai", body: body.into(), at: at.into(), url: format!("https://github.com/Org/shop/pull/1#{id}") }
    }

    fn thread(id: &str, resolved: bool, outdated: bool, comments: Vec<Comment>) -> Thread {
        Thread { id: id.into(), resolved, outdated, path: Some("app/totals.py".into()), line: Some(12), comments }
    }

    fn open(threads: Vec<Thread>, comments: Vec<Comment>, reviews: Vec<Review>) -> OpenPr {
        let mut p = pr(1, "feature/SH-1-a", "A");
        p.head_at = Some("2026-11-05T10:00:00Z".into());
        p.head_sha = Some("abc1234def".into());
        OpenPr { pr: p, threads, comments, reviews }
    }

    fn kinds(items: &[ItemDraft]) -> Vec<String> {
        items.iter().map(|i| format!("{}:{}", i.kind, i.id)).collect()
    }

    #[test]
    fn remarks_wait_until_you_answer_or_they_are_resolved() {
        let o = open(
            vec![
                thread("t1", false, false, vec![comment("c1", "reviewer", "Rename this", "2026-11-05T11:00:00Z")]),
                thread("t2", false, false, vec![comment("c2", "reviewer", "Why?", "2026-11-05T11:00:00Z"), comment("c3", "me-dev", "Because", "2026-11-05T12:00:00Z")]),
                thread("t3", true, false, vec![comment("c4", "reviewer", "Typo", "2026-11-05T11:00:00Z")]),
                thread("t4", false, false, vec![comment("c5", "reviewer", "A", "2026-11-05T11:00:00Z"), comment("c6", "me-dev", "B", "2026-11-05T12:00:00Z"), comment("c7", "reviewer", "Still wrong", "2026-11-05T13:00:00Z")]),
            ],
            vec![],
            vec![],
        );
        let items = github_items(&o, "Me-Dev", &[]);
        assert_eq!(kinds(&items), ["remark:thread:t1", "remark:thread:t4"]);
        assert_eq!(items[0].title, "Remark from reviewer on app/totals.py:12");
        assert_eq!(items[1].detail, "Still wrong", "the latest word in the thread");
    }

    #[test]
    fn an_outdated_thread_you_never_answered_is_probably_fixed() {
        let o = open(
            vec![
                thread("t1", false, true, vec![comment("c1", "reviewer", "Off by one", "2026-11-05T09:00:00Z")]),
                thread("t2", false, true, vec![comment("c2", "reviewer", "X", "2026-11-05T09:00:00Z"), comment("c3", "me-dev", "Y", "2026-11-05T09:30:00Z"), comment("c4", "reviewer", "Z", "2026-11-05T09:40:00Z")]),
            ],
            vec![],
            vec![],
        );
        let items = github_items(&o, "me-dev", &[]);
        assert_eq!(kinds(&items), ["probably_fixed:thread:t1", "remark:thread:t2"], "once you replied, it is a conversation again");
        assert!(items[0].title.starts_with("Probably fixed, needs a reply"));
    }

    #[test]
    fn questions_after_your_last_word_or_push() {
        let o = open(
            vec![],
            vec![
                comment("q0", "reviewer", "Old question?", "2026-11-05T09:00:00Z"),
                comment("q1", "reviewer", "Can you add a test?", "2026-11-05T11:00:00Z"),
                comment("q2", "reviewer", "Looks fine", "2026-11-05T11:05:00Z"),
                comment("q3", "qa", "@me-dev ping", "2026-11-05T11:10:00Z"),
                comment("q4", "me-dev", "Will do", "2026-11-05T11:20:00Z"),
                comment("q5", "coderabbitai", "Want me to open an issue?", "2026-11-05T11:30:00Z"),
            ],
            vec![],
        );
        let items = github_items(&o, "me-dev", &[]);
        // q0 was before the push at 10:00, q1 and q3 before your comment at 11:20
        assert_eq!(kinds(&items), ["question:question:q5"]);
        assert!(items[0].bot, "a bot's question is shown apart");
        let mut quiet = o.clone();
        quiet.comments.retain(|c| c.id != "q4");
        assert_eq!(kinds(&github_items(&quiet, "me-dev", &[])), ["question:question:q1", "question:question:q3", "question:question:q5"]);
    }

    #[test]
    fn changes_requested_wait_until_you_push() {
        let review = |at: &str| Review { id: "r1".into(), state: "CHANGES_REQUESTED".into(), author: Some("reviewer".into()), bot: false, at: Some(at.into()) };
        let mut o = open(vec![], vec![], vec![review("2026-11-05T11:00:00Z")]);
        o.pr.review_decision = Some("CHANGES_REQUESTED".into());
        assert_eq!(kinds(&github_items(&o, "me-dev", &[])), ["changes_requested:changes:r1"]);
        o.pr.head_at = Some("2026-11-05T12:00:00Z".into());
        assert!(github_items(&o, "me-dev", &[]).is_empty(), "you pushed since");
        o.pr.head_at = Some("2026-11-05T10:00:00Z".into());
        o.pr.review_decision = Some("APPROVED".into());
        assert!(github_items(&o, "me-dev", &[]).is_empty(), "the review changed");
    }

    #[test]
    fn failed_checks_on_the_latest_commit_count_unless_ignored() {
        let mut o = open(vec![], vec![], vec![]);
        o.pr.checks = Some("FAILURE".into());
        o.pr.failing = vec![
            Check { name: "tests".into(), state: "FAILURE".into(), url: Some("https://ci.example.com/1".into()) },
            Check { name: "SonarCloud Code Analysis".into(), state: "CANCELLED".into(), url: None },
        ];
        let items = github_items(&o, "me-dev", &[]);
        assert_eq!(kinds(&items), ["pipeline_failed:checks:Org/shop#1:abc1234def"]);
        assert_eq!(items[0].title, "Checks failing: tests, SonarCloud Code Analysis");
        assert_eq!(items[0].detail, "tests: failure\nSonarCloud Code Analysis: cancelled");
        assert_eq!(items[0].url.as_deref(), Some("https://ci.example.com/1"));
        let only_sonar = github_items(&o, "me-dev", &["tests".into(), "sonarcloud code analysis".into()]);
        assert!(only_sonar.is_empty(), "ignored checks never count");
        o.pr.failing.clear();
        o.pr.running = 2;
        assert!(github_items(&o, "me-dev", &[]).is_empty(), "running checks don't count");
    }

    #[test]
    fn conflicts_count_and_unknown_is_not_a_conflict() {
        let mut o = open(vec![], vec![], vec![]);
        o.pr.mergeable = Some("CONFLICTING".into());
        assert_eq!(kinds(&github_items(&o, "me-dev", &[])), ["conflict:conflict:Org/shop#1"]);
        o.pr.mergeable = Some("UNKNOWN".into());
        assert!(github_items(&o, "me-dev", &[]).is_empty());
    }

    #[test]
    fn the_real_prs_give_the_items_github_shows() {
        let (me, prs, _) = parse_open_page(&fixture("gh-open-prs.json"), "p1", "example-org/shop").unwrap();
        let items: Vec<ItemDraft> = prs.iter().flat_map(|(p, _)| {
            let mut p = p.clone();
            p.pr.unresolved = p.threads.iter().filter(|t| !t.resolved).count();
            github_items(&p, &me, &[])
        }).collect();
        let count = |k: &str| items.iter().filter(|i| i.kind == k).count();
        // the read in the fixture: one red pipeline (the cancelled SonarCloud run) and three change requests
        assert_eq!(count("pipeline_failed"), 1);
        assert!(count("changes_requested") <= 3);
        assert!(count("remark") + count("probably_fixed") >= 9, "the nine open threads on the red PR wait on you");
        assert!(items.iter().all(|i| i.pr.is_some() && !i.id.is_empty()));
        let ids: std::collections::HashSet<&str> = items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids.len(), items.len(), "every item once");
    }

    fn put(db: &Db, drafts: Vec<ItemDraft>, now: i64) -> Merged {
        merge(db, "p1", "github", drafts, now, &|_| false).unwrap()
    }

    fn state(db: &Db, id: &str) -> String {
        db.work_items().unwrap().into_iter().find(|i| i.id == id).unwrap().state
    }

    #[test]
    fn items_open_update_and_close_themselves() {
        let (_d, db) = db();
        let o = open(vec![thread("t1", false, false, vec![comment("c1", "reviewer", "Rename", "2026-11-05T11:00:00Z")])], vec![], vec![]);
        let first = put(&db, github_items(&o, "me-dev", &[]), 100);
        assert_eq!(first.opened, ["thread:t1"]);
        let again = put(&db, github_items(&o, "me-dev", &[]), 200);
        assert_eq!(again, Merged::default(), "the same read again changes nothing");
        assert_eq!(db.work_items().unwrap().len(), 1, "updated, not copied");
        let gone = put(&db, vec![], 300);
        assert_eq!(gone.closed, ["thread:t1"]);
        assert_eq!(state(&db, "thread:t1"), "closed");
        let back = put(&db, github_items(&o, "me-dev", &[]), 400);
        assert_eq!(back.reopened, ["thread:t1"], "its reason came back");
        let history: Vec<String> = db.work_events(10).unwrap().into_iter().map(|e| e.kind).collect();
        assert_eq!(history, ["item-reopened", "item-closed", "item-opened"]);
    }

    #[test]
    fn done_stays_done_until_something_newer_and_snoozed_wakes_up() {
        let (_d, db) = db();
        let mut o = open(vec![thread("t1", false, false, vec![comment("c1", "reviewer", "Rename", "2026-11-05T11:00:00Z")])], vec![], vec![]);
        put(&db, github_items(&o, "me-dev", &[]), 100);
        let mut item = db.work_items().unwrap().pop().unwrap();
        item.state = "done".into();
        db.work_put_item(&item).unwrap();
        put(&db, github_items(&o, "me-dev", &[]), 200);
        assert_eq!(state(&db, "thread:t1"), "done");
        assert!(put(&db, vec![], 250).closed.is_empty(), "done items aren't closed again");
        o.threads[0].comments.push(comment("c2", "reviewer", "Any news?", "2026-11-05T15:00:00Z"));
        put(&db, github_items(&o, "me-dev", &[]), 300);
        assert_eq!(state(&db, "thread:t1"), "open", "a newer remark opens it again");
        let mut item = db.work_items().unwrap().pop().unwrap();
        item.state = "snoozed".into();
        item.snoozed_until = Some(1_000);
        db.work_put_item(&item).unwrap();
        put(&db, github_items(&o, "me-dev", &[]), 500);
        assert_eq!(state(&db, "thread:t1"), "snoozed");
        put(&db, github_items(&o, "me-dev", &[]), 1_000);
        assert_eq!(state(&db, "thread:t1"), "open", "snooze over");
    }

    #[test]
    fn items_the_read_did_not_cover_are_kept() {
        let (_d, db) = db();
        let o = open(vec![thread("t1", false, false, vec![comment("c1", "reviewer", "Rename", "2026-11-05T11:00:00Z")])], vec![], vec![]);
        put(&db, github_items(&o, "me-dev", &[]), 100);
        let kept = merge(&db, "p1", "github", vec![], 200, &|i| i.pr.as_deref() == Some("Org/shop#1")).unwrap();
        assert!(kept.closed.is_empty());
        assert_eq!(state(&db, "thread:t1"), "open");
    }

    use crate::work::jira::{Attachment, Comment as JComment, FullTicket, Person};
    use crate::work::model::Ticket;

    const NOW: i64 = 1_793_887_402_000; // 2026-11-05 14:03 UTC

    fn person(id: &str, bot: bool) -> Person {
        Person { id: id.into(), name: if id == "acc-me" { "Me Dev".into() } else { format!("Name of {id}") }, bot }
    }

    fn jc(id: &str, author: &str, at: &str, text: &str, mentions: &[&str]) -> JComment {
        JComment {
            id: id.into(),
            author: Some(person(author, author.starts_with("app")),),
            text: text.into(),
            mentions: mentions.iter().map(|m| m.to_string()).collect(),
            created: at.into(),
            updated: at.into(),
        }
    }

    fn full(status: &str, category: &str, comments: Vec<JComment>, attachments: Vec<Attachment>) -> FullTicket {
        FullTicket {
            ticket: Ticket {
                key: "SH-21112".into(),
                project_key: "SH".into(),
                summary: "Bundles show the cost per article".into(),
                url: "https://example.atlassian.net/browse/SH-21112".into(),
                status: status.into(),
                category: category.into(),
                assignee: Some("Me Dev".into()),
                assignee_id: Some("acc-me".into()),
                updated_at: "2026-11-05T12:00:00.000+0100".into(),
                comments: comments.len(),
                attachments: attachments.len(),
                last_comment_by: None,
                last_comment_at: None,
                description: None,
                described_at: None,
                issue_type: None,
                priority: None,
            },
            comments,
            attachments,
        }
    }

    fn jkinds(items: &[ItemDraft]) -> Vec<String> {
        items.iter().map(|i| format!("{} {}", i.kind, i.id)).collect()
    }

    #[test]
    fn jira_replies_and_mentions_after_your_last_comment() {
        let t = full("QA Review", "indeterminate", vec![
            jc("1", "acc-qa", "2026-11-01T09:00:00.000+0100", "Old note", &[]),
            jc("2", "acc-me", "2026-11-02T09:00:00.000+0100", "Fixed in the PR", &[]),
            jc("3", "acc-qa", "2026-11-03T09:00:00.000+0100", "Still NaN with a sub-bundle", &[]),
            jc("4", "acc-po", "2026-11-03T10:00:00.000+0100", "@Me Dev can you check?", &["acc-me"]),
            jc("5", "app-automation", "2026-11-03T11:00:00.000+0100", "Moved to QA by automation", &[]),
            jc("6", "acc-qa", "2026-09-01T09:00:00.000+0100", "Ancient", &[]),
        ], vec![]);
        let items = jira_items(&t, "acc-me", Some("QA Review"), None, &[], NOW);
        assert_eq!(jkinds(&items), [
            "qa_replied jira-comment:SH-21112:3",
            "mentioned jira-mention:SH-21112:4",
            "qa_replied jira-comment:SH-21112:5",
        ]);
        assert_eq!(items[0].title, "Name of acc-qa replied on SH-21112");
        assert!(items[2].bot, "an app's comment is shown apart");
        assert_eq!(items[1].url.as_deref(), Some("https://example.atlassian.net/browse/SH-21112?focusedCommentId=4"));
        // you answered: they all close
        let mut answered = t.clone();
        answered.comments.push(jc("7", "acc-me", "2026-11-04T09:00:00.000+0100", "Done", &[]));
        assert!(jira_items(&answered, "acc-me", Some("QA Review"), None, &[], NOW).is_empty());
    }

    #[test]
    fn a_done_ticket_only_counts_mentions() {
        let t = full("Done", "done", vec![
            jc("3", "acc-qa", "2026-11-03T09:00:00.000+0100", "Thanks", &[]),
            jc("4", "acc-po", "2026-11-03T10:00:00.000+0100", "@Me Dev this broke on live", &["acc-me"]),
        ], vec![Attachment { id: "a1".into(), filename: "x.png".into(), mime: "image/png".into(), author: Some(person("acc-qa", false)), created: "2026-11-03T09:00:00.000+0100".into(), size: 1 }]);
        assert_eq!(jkinds(&jira_items(&t, "acc-me", None, None, &[], NOW)), ["mentioned jira-mention:SH-21112:4"]);
    }

    #[test]
    fn new_attachments_from_others_count_until_you_answer() {
        let shot = |id: &str, by: &str, at: &str| Attachment { id: id.into(), filename: format!("{id}.png"), mime: "image/png".into(), author: Some(person(by, false)), created: at.into(), size: 10 };
        let t = full("QA Review", "indeterminate", vec![jc("1", "acc-me", "2026-11-02T09:00:00.000+0100", "Ready", &[])], vec![
            shot("old", "acc-qa", "2026-11-01T09:00:00.000+0100"),
            shot("mine", "acc-me", "2026-11-03T09:00:00.000+0100"),
            shot("nan", "acc-qa", "2026-11-03T09:00:00.000+0100"),
        ]);
        let items = jira_items(&t, "acc-me", None, None, &[], NOW);
        assert_eq!(jkinds(&items), ["attachment jira-attachment:SH-21112:nan"]);
        assert_eq!(items[0].title, "New attachment on SH-21112: nan.png");
    }

    #[test]
    fn back_to_you_when_it_moves_back_and_while_it_stays() {
        assert!(moved_back("QA Review", "In Progress", &[]));
        assert!(moved_back("Code Review", "Reopened", &[]));
        assert!(!moved_back("In Progress", "Code Review", &[]), "forward is not back");
        assert!(!moved_back("QA Review", "Canceled", &[]), "an unknown status is no move back");
        assert!(moved_back("QA Review", "QA Failed", &["qa failed".into()]), "the project's own statuses");
        assert!(!moved_back("QA Review", "In Progress", &["QA Failed".into()]), "set statuses replace the usual way");

        let now_in_progress = full("In Progress", "indeterminate", vec![], vec![]);
        let items = jira_items(&now_in_progress, "acc-me", Some("QA Review"), None, &[], NOW);
        assert_eq!(jkinds(&items), ["back_to_you jira-back:SH-21112:QA Review>In Progress"]);
        assert_eq!(items[0].title, "SH-21112 is back to you: QA Review → In Progress");
        // the next read: no move this time, but it is still there
        assert_eq!(jira_items(&now_in_progress, "acc-me", Some("In Progress"), Some(("QA Review", "In Progress")), &[], NOW).len(), 1);
        // it moves on: gone
        let forward = full("Code Review", "indeterminate", vec![], vec![]);
        assert!(jira_items(&forward, "acc-me", Some("In Progress"), Some(("QA Review", "In Progress")), &[], NOW).is_empty());
        // not yours: not back to you
        let mut theirs = now_in_progress.clone();
        theirs.ticket.assignee_id = Some("acc-qa".into());
        assert!(jira_items(&theirs, "acc-me", Some("QA Review"), None, &[], NOW).is_empty());
    }

    #[test]
    fn the_real_tickets_give_items() {
        let got = crate::work::jira::parse_get(&fixture("twg-get.json")).unwrap();
        let items: Vec<ItemDraft> = got.tickets.iter().flat_map(|t| jira_items(t, "acc-me", None, None, &[], NOW)).collect();
        let ids: std::collections::HashSet<&str> = items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids.len(), items.len(), "every item once");
        assert!(items.iter().all(|i| i.ticket.is_some() && i.pr.is_none()));
    }

    #[test]
    fn a_prs_timeline_follows_its_changes() {
        let mut a = pr(1, "feature/a", "A");
        a.head_sha = Some("1111111aaaa".into());
        a.checks = Some("PENDING".into());
        assert_eq!(pr_events(None, &a)[0].0, "pr-seen");
        let mut b = a.clone();
        b.head_sha = Some("2222222bbbb".into());
        b.checks = Some("FAILURE".into());
        b.failing = vec![Check { name: "tests".into(), state: "FAILURE".into(), url: None }];
        b.review_decision = Some("CHANGES_REQUESTED".into());
        let ev: Vec<String> = pr_events(Some(&a), &b).into_iter().map(|(_, t)| t).collect();
        assert_eq!(ev, ["Pushed 2222222", "Changes requested", "Checks failed: tests"]);
        let mut c = b.clone();
        c.state = "MERGED".into();
        assert_eq!(pr_events(Some(&b), &c)[0].1, "Merged into development");
    }
}
