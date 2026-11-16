//! The watchers: GitHub (and, from Phase 3, Jira) read for every project on its interval, on Refresh, and right after
//! Claude pushes or finishes a turn. A read is all or nothing: a failed one keeps the last good data and items as they
//! were, and the source's health line says what is wrong, what to do, and when the next try is.

use anyhow::Result;
use std::collections::HashMap;

use super::github;
use super::links::ProjectInfo;
use super::model::{Board, Health, Pr};
use super::rules;
use super::run::{Failure, Tool};
use crate::db::Db;
use crate::settings::WorkSettings;

/// Never read one project more often than this, however often Claude pushes or stops.
pub const MIN_GAP_MS: i64 = 30_000;

/// The gap, or the one end-to-end tests set (`ENKIDU_WORK_MIN_GAP_MS`) to break and fix sources quickly.
pub fn min_gap() -> i64 {
    std::env::var("ENKIDU_WORK_MIN_GAP_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(MIN_GAP_MS)
}
/// How far back merged PRs are kept in view (QA tests after the merge).
pub const MERGED_DAYS: i64 = 30;
/// Waits after failed reads, one after another (then the project's interval).
const BACKOFF_MS: [i64; 5] = [60_000, 120_000, 300_000, 600_000, 900_000];

/// The programs the watchers run. `ENKIDU_GH` and `ENKIDU_TWG` point them at fakes in tests.
#[derive(Clone, Debug)]
pub struct Sources {
    pub gh: Tool,
    pub twg: Tool,
}

impl Sources {
    pub fn from_env(path_env: &str) -> Self {
        let pick = |var: &str, default: &str| std::env::var(var).ok().filter(|v| !v.trim().is_empty()).unwrap_or_else(|| default.to_string());
        Sources { gh: Tool::new(&pick("ENKIDU_GH", "gh"), path_env), twg: Tool::new(&pick("ENKIDU_TWG", "twg"), path_env) }
    }
}

/// `YYYY-MM-DD` of `days` before `now` (UTC), for GitHub's `merged:>=` search.
pub fn date_days_before(now: i64, days: i64) -> String {
    let z = (now - days * 86_400_000).div_euclid(86_400_000) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}")
}

pub fn health_of(db: &Db, project_id: &str, source: &str) -> Option<Health> {
    db.work_health().ok()?.into_iter().find(|h| h.project_id == project_id && h.source == source)
}

/// The next try after `failures` failed reads in a row.
pub fn backoff(failures: u32, every_minutes: u32) -> i64 {
    let cap = every_minutes.max(1) as i64 * 60_000;
    BACKOFF_MS[(failures.max(1) as usize - 1).min(BACKOFF_MS.len() - 1)].min(cap)
}

/// Saves one full GitHub read of a project: PRs (and those that vanished), the timeline, the items.
pub fn apply_github(db: &Db, project_id: &str, ignored: &[String], read: &github::Read, now: i64) -> Result<rules::Merged> {
    let old: HashMap<String, Pr> = db.work_prs()?.into_iter().map(|(p, _)| p).filter(|p| p.project_id == project_id).map(|p| (p.reference(), p)).collect();
    let mut all: Vec<Pr> = read.open.iter().map(|o| o.pr.clone()).collect();
    all.extend(read.merged.iter().cloned());
    // an open PR that is neither open nor merged now was closed (or moved away, or deleted)
    for (reference, p) in &old {
        if p.state == "OPEN" && !all.iter().any(|n| &n.reference() == reference) {
            all.push(Pr { state: "CLOSED".into(), failing: Vec::new(), running: 0, ..p.clone() });
        }
    }
    for p in &all {
        for (kind, text) in rules::pr_events(old.get(&p.reference()), p) {
            db.work_event(&p.reference(), kind, &text, now)?;
        }
    }
    db.work_put_prs(project_id, &all, now)?;
    let drafts = read.open.iter().flat_map(|o| rules::github_items(o, &read.me, ignored)).collect();
    rules::merge(db, project_id, "github", drafts, now, &|_| false)
}

/// One GitHub pass for a project; returns its health (also saved).
pub fn github_pass(db: &Db, gh: &Tool, project: &ProjectInfo, ws: &WorkSettings, failures: &mut u32, now: i64) -> Health {
    let prev = health_of(db, &project.id, "github");
    let mut h = Health {
        project_id: project.id.clone(),
        source: "github".into(),
        state: "ok".into(),
        last_ok: prev.as_ref().and_then(|p| p.last_ok),
        last_try: Some(now),
        error: None,
        fix: None,
        next_try: Some(now + ws.every_minutes.max(1) as i64 * 60_000),
        account: prev.as_ref().and_then(|p| p.account.clone()),
    };
    let off = |mut h: Health, why: &str| {
        h.state = "off".into();
        h.error = Some(why.into());
        h
    };
    let h = if !ws.github {
        off(h, "GitHub isn't read for this project (Settings).")
    } else if let Some(repo) = project.repo.clone() {
        let result = (|| -> Result<(String, rules::Merged)> {
            // the account set in Settings; else the one that read last time (no need to ask gh again); else the
            // first that can read the repo
            let (account, token) = if !ws.github_account.trim().is_empty() {
                let a = ws.github_account.trim().to_string();
                let t = github::token(gh, &a)?;
                (a, t)
            } else if let Some(t) = h.account.as_deref().and_then(|a| github::token(gh, a).ok()) {
                (h.account.clone().unwrap_or_default(), t)
            } else {
                github::pick_account(gh, &repo, None)?
            };
            let since = date_days_before(now, MERGED_DAYS);
            let read = github::read(gh, &token, &account, &repo, &project.id, &since)?;
            let merged = apply_github(db, &project.id, &ws.ignored_checks, &read, now)?;
            Ok((account, merged))
        })();
        match result {
            Ok((account, _)) => {
                *failures = 0;
                h.last_ok = Some(now);
                h.account = Some(account);
                h
            }
            Err(e) => {
                *failures += 1;
                let f = Failure::of(&e);
                h.state = "error".into();
                h.error = Some(crate::redact::redact_urls(&f.message));
                h.fix = Some(f.fix);
                h.next_try = Some(now + backoff(*failures, ws.every_minutes));
                if f.message.contains("can't find the repository") || f.message.contains("refused the login") {
                    h.account = None; // pick again next time: another account may read it
                }
                h
            }
        }
    } else {
        off(h, "This project's origin isn't on GitHub.")
    };
    let _ = db.work_put_health(&h);
    h
}

/// The tickets your PRs name (open ones, and those merged in the last 30 days: QA tests after the merge), in the
/// project's Jira projects.
pub fn linked_keys(db: &Db, project_id: &str, jira_keys: &[String], now: i64) -> Result<Vec<String>> {
    let since = date_days_before(now, MERGED_DAYS);
    let prs: HashMap<String, Pr> = db.work_prs()?.into_iter().map(|(p, _)| p).filter(|p| p.project_id == project_id).map(|p| (format!("pr:{}", p.reference()), p)).collect();
    let mut keys: Vec<String> = db
        .work_links()?
        .into_iter()
        .filter(|l| l.how == "ticket-key")
        .filter(|l| prs.get(&l.b).is_some_and(|p| p.state == "OPEN" || p.merged_at.as_deref().is_some_and(|m| m[..10.min(m.len())] >= since[..])))
        .filter_map(|l| l.a.strip_prefix("ticket:").map(String::from))
        .filter(|k| jira_keys.iter().any(|j| super::parse::key_project(k).eq_ignore_ascii_case(j)))
        .collect();
    keys.sort();
    keys.dedup();
    Ok(keys)
}

/// Is this key in one of the project's Jira projects?
fn old_project_key(jira_keys: &[String], key: &str) -> bool {
    jira_keys.iter().any(|j| super::parse::key_project(key).eq_ignore_ascii_case(j))
}

/// The search for your open tickets in the project's Jira projects. The tickets your PRs name are read by key
/// instead: Jira fails a whole search that names a key it doesn't know.
pub fn assigned_jql(jira_keys: &[String]) -> String {
    format!("assignee = currentUser() AND statusCategory != Done AND project in ({}) ORDER BY updated DESC", jira_keys.join(", "))
}

/// Saves one full Jira read of a project: tickets (and those gone or moved), the timeline, the items.
pub fn apply_jira(
    db: &Db,
    project_id: &str,
    ws: &WorkSettings,
    me: &str,
    assigned: &[super::model::Ticket],
    linked: &[String],
    got: &super::jira::Got,
    last_ok: Option<i64>,
    now: i64,
) -> Result<rules::Merged> {
    use super::model::Ticket;
    let old: HashMap<String, Ticket> = db.work_project_tickets(project_id)?.into_iter().map(|t| (t.key.clone(), t)).collect();
    // your work: what your PRs name, what the search found, and what is still assigned to you; a ticket read again
    // only to learn why it left the search drops off (with a line in its timeline)
    let mut all: Vec<Ticket> = Vec::new();
    for full in &got.tickets {
        let t = &full.ticket;
        let yours = linked.contains(&t.key) || assigned.iter().any(|a| a.key == t.key) || t.assignee_id.as_deref() == Some(me);
        if yours {
            all.push(t.clone());
        } else {
            let who = t.assignee.clone().map_or("no one".to_string(), |a| a);
            db.work_event(&t.key, "ticket-left", &format!("No longer yours: assigned to {who}"), now)?;
        }
    }
    for t in assigned {
        // a ticket Jira says doesn't exist is gone, whatever the search found a moment earlier
        if !all.iter().any(|x| x.key == t.key) && !got.gone.contains(&t.key) {
            all.push(t.clone());
        }
    }
    for key in &got.gone {
        if old.contains_key(key) {
            db.work_event(key, "ticket-gone", "No longer in Jira (deleted, or you can't see it)", now)?;
        }
    }
    for (from, to) in &got.moved {
        db.work_event(from, "ticket-moved", &format!("Moved to {to}"), now)?;
        // the PRs that name the old key belong to the new one
        for l in db.work_links()?.into_iter().filter(|l| l.a == format!("ticket:{from}") && l.b.starts_with("pr:")) {
            db.work_link(&format!("ticket:{to}"), &l.b, "ticket-key", &format!("the key moved from {from}"), now)?;
        }
    }
    for t in &all {
        if let Some(o) = old.get(&t.key) {
            if o.status != t.status {
                db.work_event(&t.key, "status", &format!("{} → {}", o.status, t.status), now)?;
            }
        }
    }
    // what was said and attached since the last read (nothing on the first read: that is history, not news)
    if let Some(since) = last_ok {
        for full in &got.tickets {
            for c in full.comments.iter().filter(|c| super::parse::parse_time(&c.created).unwrap_or(0) > since) {
                if c.author.as_ref().is_some_and(|a| a.id != me) {
                    let name = c.author.as_ref().map(|a| a.name.as_str()).unwrap_or("Someone");
                    db.work_event(&full.ticket.key, "comment", &format!("Comment from {name}"), now)?;
                }
            }
        }
    }
    db.work_put_tickets(project_id, &all, now)?;
    let open_back: HashMap<String, (String, String)> = db
        .work_items()?
        .into_iter()
        .filter(|i| i.kind == "back_to_you" && i.project_id == project_id && matches!(i.state.as_str(), "open" | "snoozed" | "done"))
        .filter_map(|i| {
            let (from, to) = i.id.rsplit_once(':')?.1.split_once('>')?;
            Some((i.ticket.clone()?, (from.to_string(), to.to_string())))
        })
        .collect();
    let drafts = got
        .tickets
        .iter()
        .flat_map(|t| {
            let back = open_back.get(&t.ticket.key).map(|(f, to)| (f.as_str(), to.as_str()));
            rules::jira_items(t, me, old.get(&t.ticket.key).map(|o| o.status.as_str()), back, &ws.back_to_you, now)
        })
        .collect();
    rules::merge(db, project_id, "jira", drafts, now, &|_| false)
}

/// Each ticket's description, type and priority: kept from the last read while the ticket hasn't changed (the same
/// `updated`), read again only for new and changed tickets, so a read stays small.
pub fn describe_tickets(db: &Db, twg: &Tool, assigned: &mut [super::model::Ticket], got: &mut super::jira::Got) -> Result<()> {
    let stored: HashMap<String, super::model::Ticket> = db.work_tickets()?.into_iter().map(|(t, _)| (t.key.clone(), t)).collect();
    let mut need: Vec<String> = Vec::new();
    for t in assigned.iter_mut().chain(got.tickets.iter_mut().map(|f| &mut f.ticket)) {
        match stored.get(&t.key) {
            Some(old) if old.described_at.as_deref() == Some(t.updated_at.as_str()) => {
                t.description = old.description.clone();
                t.described_at = old.described_at.clone();
                t.issue_type = t.issue_type.take().or_else(|| old.issue_type.clone());
                t.priority = t.priority.take().or_else(|| old.priority.clone());
            }
            _ => {
                if !need.contains(&t.key) {
                    need.push(t.key.clone());
                }
            }
        }
    }
    if need.is_empty() {
        return Ok(());
    }
    let read = super::jira::describe(twg, "", &need)?;
    for t in assigned.iter_mut().chain(got.tickets.iter_mut().map(|f| &mut f.ticket)) {
        if let Some(d) = read.get(&t.key) {
            t.description = d.description.clone();
            t.issue_type = d.issue_type.clone().or_else(|| t.issue_type.take());
            t.priority = d.priority.clone().or_else(|| t.priority.take());
            t.described_at = Some(t.updated_at.clone());
        }
    }
    Ok(())
}

/// Which of these tickets are on a Jira board, as its own page in Jira shows it (a scrum board: the sprint that is
/// running). None when Jira says the board doesn't exist any more.
pub fn board_keys(twg: &Tool, board: &Board, keys: &[String]) -> Result<Option<Vec<String>>> {
    let Some(scope) = super::jira::board_scope(twg, "", &board.id)? else { return Ok(None) };
    Ok(Some(super::jira::on_board(twg, "", &scope, keys)?))
}

/// One Jira pass for a project; returns its health (also saved). `me` caches who twg reads as.
pub fn jira_pass(db: &Db, twg: &Tool, project: &ProjectInfo, ws: &WorkSettings, me: &mut Option<(String, String)>, failures: &mut u32, now: i64) -> Health {
    let prev = health_of(db, &project.id, "jira");
    let mut h = Health {
        project_id: project.id.clone(),
        source: "jira".into(),
        state: "ok".into(),
        last_ok: prev.as_ref().and_then(|p| p.last_ok),
        last_try: Some(now),
        error: None,
        fix: None,
        next_try: Some(now + ws.every_minutes.max(1) as i64 * 60_000),
        account: prev.as_ref().and_then(|p| p.account.clone()),
    };
    if ws.jira_keys.is_empty() {
        h.state = "off".into();
        h.error = Some("Jira isn't read for this project: no Jira project keys in Settings.".into());
        let _ = db.work_put_health(&h);
        return h;
    }
    let result = (|| -> Result<String> {
        let (id, name) = match me.clone() {
            Some(m) => m,
            None => {
                let m = super::jira::whoami(twg, "")?;
                *me = Some(m.clone());
                m
            }
        };
        let assigned = super::jira::query(twg, "", &assigned_jql(&ws.jira_keys))?;
        let linked = linked_keys(db, &project.id, &ws.jira_keys, now)?;
        // read whole: the ones your PRs name, the ones you are working on (not the backlog), and the ones you were
        // working on at the last read, so a ticket that left the search says why (done, someone else's, deleted)
        let current = crate::work::snapshot_of(db)?.tickets;
        let mut keys = linked.clone();
        let more = assigned.iter().filter(|t| t.category != "new").map(|t| t.key.clone());
        let before = current.iter().filter(|t| t.category == "indeterminate" && old_project_key(&ws.jira_keys, &t.key)).map(|t| t.key.clone());
        for k in more.chain(before) {
            if !keys.contains(&k) {
                keys.push(k);
            }
        }
        let mut got = super::jira::get(twg, "", &keys)?;
        let mut assigned = assigned;
        describe_tickets(db, twg, &mut assigned, &mut got)?;
        // the board you picked: which of the tickets just read are on it. Part of the read: when it can't be read,
        // nothing of this read is kept (the page would show a board that is no longer true)
        let board = db.work_board()?;
        let on_board = match &board {
            Some(b) => {
                let mut read: Vec<String> = got.tickets.iter().map(|t| t.ticket.key.clone()).collect();
                for t in assigned.iter().filter(|t| !got.gone.contains(&t.key)) {
                    if !read.contains(&t.key) {
                        read.push(t.key.clone());
                    }
                }
                board_keys(twg, b, &read)?
            }
            None => None,
        };
        apply_jira(db, &project.id, ws, &id, &assigned, &linked, &got, h.last_ok, now)?;
        match (&board, on_board) {
            (Some(b), Some(on)) => db.work_put_board_keys(&b.id, &project.id, &on, now)?,
            // the board is gone from Jira (deleted, or you can't see it any more): back to everything read, unless
            // you picked another one meanwhile
            (Some(b), None) => {
                if db.work_board()?.as_ref() == Some(b) {
                    db.work_set_board(None)?;
                }
            }
            (None, _) => {}
        }
        Ok(name)
    })();
    match result {
        Ok(name) => {
            *failures = 0;
            h.last_ok = Some(now);
            h.account = Some(name);
        }
        Err(e) => {
            *failures += 1;
            let f = Failure::of(&e);
            if f.message.contains("isn't signed in") {
                *me = None; // a new login may be someone else
            }
            h.state = "error".into();
            h.error = Some(crate::redact::redact_urls(&f.message));
            h.fix = Some(f.fix);
            h.next_try = Some(now + backoff(*failures, ws.every_minutes));
        }
    }
    let _ = db.work_put_health(&h);
    h
}

/// Is a project due for a read? On its interval (or backoff), or when wanted and not read in the last 30 s.
pub fn due(health: Option<&Health>, wanted: bool, now: i64) -> bool {
    match health {
        None => true,
        Some(h) => {
            let recent = h.last_try.is_some_and(|t| now - t < min_gap());
            (wanted && !recent) || h.next_try.is_none_or(|t| t <= now)
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::work::links::tests::shop;
    use crate::work::store::tests::db;
    use std::path::{Path, PathBuf};

    pub fn fake(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fakes").join(name)
    }

    fn run_with(mode: &str) -> (tempfile::TempDir, Db, Health, u32) {
        let (dir, db) = db();
        let log = dir.path().join("gh.log");
        let gh = Tool::new(&format!("node {}", fake("fake-gh.mjs").display()), &std::env::var("PATH").unwrap_or_default());
        let mode_file = dir.path().join("gh.mode");
        std::fs::write(&mode_file, mode).unwrap();
        let tool = with_env(gh, &[("FAKE_GH_MODE_FILE", mode_file.to_str().unwrap()), ("FAKE_GH_LOG", log.to_str().unwrap())]);
        let mut failures = 0;
        let mut p = shop();
        p.repo = Some("example-org/shop".into());
        let h = github_pass(&db, &tool, &p, &WorkSettings::default(), &mut failures, 1_793_887_402_000);
        (dir, db, h, failures)
    }

    /// The fake reads its mode and log from the environment the watcher passes on.
    fn with_env(mut t: Tool, env: &[(&str, &str)]) -> Tool {
        t.env = env.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        t
    }

    #[test]
    fn dates_for_the_merged_search() {
        assert_eq!(date_days_before(1_793_887_402_123, 0), "2026-11-05");
        assert_eq!(date_days_before(1_793_887_402_123, 30), "2026-10-06");
        assert_eq!(date_days_before(0, 0), "1970-01-01");
    }

    #[test]
    fn backs_off_after_failures_and_reads_when_due() {
        assert_eq!(backoff(1, 5), 60_000);
        assert_eq!(backoff(3, 5), 300_000);
        assert_eq!(backoff(9, 5), 300_000, "never longer than the project's interval");
        assert_eq!(backoff(9, 60), 900_000);
        let h = Health { project_id: "p".into(), source: "github".into(), state: "ok".into(), last_ok: Some(0), last_try: Some(1_000), error: None, fix: None, next_try: Some(301_000), account: None };
        assert!(due(None, false, 0), "never read: read now");
        assert!(!due(Some(&h), false, 200_000));
        assert!(due(Some(&h), true, 200_000), "a push asks for a read");
        assert!(!due(Some(&h), true, 20_000), "but not twice in 30 s");
        assert!(due(Some(&h), false, 301_000));
    }

    #[test]
    fn a_full_read_through_the_fake_gh_stores_prs_and_items() {
        let (dir, db, h, failures) = run_with("ok");
        assert_eq!((h.state.as_str(), h.account.as_deref(), failures), ("ok", Some("me-dev"), 0), "{h:?}");
        let prs = db.work_prs().unwrap();
        assert_eq!(prs.iter().filter(|(p, _)| p.state == "OPEN").count(), 6);
        assert_eq!(prs.iter().filter(|(p, _)| p.state == "MERGED").count(), 8);
        let items = db.work_items().unwrap();
        assert!(items.iter().any(|i| i.kind == "pipeline_failed"));
        assert!(items.iter().all(|i| i.state == "open" && i.source == "github"));
        let log = std::fs::read_to_string(dir.path().join("gh.log")).unwrap();
        assert!(!log.contains("fake-token"), "the token travels in the environment, never on the command line");
        for write in [" pr comment", " pr review", " pr merge", " pr edit", " pr close", "-X POST", "--method POST", "mutation"] {
            assert!(!log.contains(write), "nothing is ever written to GitHub ({write})");
        }
    }

    #[test]
    fn every_failure_keeps_the_last_good_read_and_says_what_to_do() {
        for (mode, says, fix) in [
            ("logged-out", "gh isn't signed in", "gh auth login"),
            ("bad-credentials", "refused the login", "gh auth refresh"),
            ("rate-limit", "rate limit", "resets"),
            ("network", "No connection to GitHub", "internet connection"),
            ("garbage", "isn't valid JSON", ""),
            ("graphql-error", "answered with an error", ""),
            ("missing-field", "reviewDecision", ""),
            ("no-access", "can read example-org/shop", "gh auth login"),
        ] {
            let (_dir, db, h, failures) = run_with(mode);
            assert_eq!(h.state, "error", "{mode}");
            assert!(h.error.as_deref().unwrap_or("").contains(says), "{mode}: {h:?}");
            assert!(h.fix.as_deref().unwrap_or("").contains(fix), "{mode}: {h:?}");
            assert_eq!(failures, 1);
            assert!(h.next_try.unwrap() > h.last_try.unwrap());
            assert!(db.work_prs().unwrap().is_empty(), "{mode}: half a read never lands");
        }
    }

    #[test]
    fn a_failed_read_after_a_good_one_changes_nothing_and_a_good_one_recovers() {
        let (dir, db, good, _) = run_with("ok");
        let prs_before = db.work_prs().unwrap();
        let items_before = db.work_items().unwrap();
        let mode_file = dir.path().join("gh.mode");
        let gh = with_env(
            Tool::new(&format!("node {}", fake("fake-gh.mjs").display()), &std::env::var("PATH").unwrap_or_default()),
            &[("FAKE_GH_MODE_FILE", mode_file.to_str().unwrap())],
        );
        let mut p = shop();
        p.repo = Some("example-org/shop".into());
        let mut failures = 0;
        for (i, mode) in ["network", "garbage", "missing-field"].iter().enumerate() {
            std::fs::write(&mode_file, mode).unwrap();
            let h = github_pass(&db, &gh, &p, &WorkSettings::default(), &mut failures, 1_793_887_600_000 + i as i64);
            assert_eq!(h.state, "error");
            assert_eq!(h.last_ok, good.last_ok, "the last good read's time stays, so the page can say how old it is");
            assert_eq!(h.account, good.account);
            assert_eq!(db.work_prs().unwrap(), prs_before, "{mode}: the PRs stay as they were");
            assert_eq!(db.work_items().unwrap(), items_before, "{mode}: no item closes or changes");
        }
        assert_eq!(failures, 3);
        assert!(backoff(failures, 5) > backoff(1, 5), "it waits longer each time");
        std::fs::write(&mode_file, "ok").unwrap();
        let h = github_pass(&db, &gh, &p, &WorkSettings::default(), &mut failures, 1_793_887_700_000);
        assert_eq!((h.state.as_str(), failures, h.error), ("ok", 0, None));
        assert_eq!(h.last_ok, Some(1_793_887_700_000));
    }

    #[test]
    fn a_hanging_gh_is_stopped_and_counts_as_a_failed_read() {
        let (dir, db) = db();
        let mode_file = dir.path().join("gh.mode");
        std::fs::write(&mode_file, "slow").unwrap();
        let mut gh = with_env(
            Tool::new(&format!("node {}", fake("fake-gh.mjs").display()), &std::env::var("PATH").unwrap_or_default()),
            &[("FAKE_GH_MODE_FILE", mode_file.to_str().unwrap())],
        );
        gh.max_wait = Some(std::time::Duration::from_secs(2));
        let mut p = shop();
        p.repo = Some("example-org/shop".into());
        let started = std::time::Instant::now();
        let h = github_pass(&db, &gh, &p, &WorkSettings::default(), &mut 0, 1_793_887_402_000);
        assert!(started.elapsed() < std::time::Duration::from_secs(10), "it doesn't wait for gh forever");
        assert_eq!(h.state, "error");
        assert!(h.error.as_deref().unwrap_or("").contains("took longer"), "{h:?}");
        assert!(db.work_prs().unwrap().is_empty());
    }

    #[test]
    fn more_than_one_page_of_prs_threads_comments_and_checks_is_read_whole() {
        let (dir, db, h, _) = run_with("pages");
        assert_eq!(h.state, "ok", "{h:?}");
        let log = std::fs::read_to_string(dir.path().join("gh.log")).unwrap();
        assert!(log.contains("after=page-2"), "the second page of PRs was asked for");
        assert!(log.contains("threads_page"), "the PR's further review threads were asked for");
        assert!(log.contains("thread_comments_page"), "the thread's further comments were asked for");
        assert!(log.contains("checks_page"), "the further checks were asked for");
        let prs = db.work_prs().unwrap();
        assert_eq!(prs.iter().filter(|(p, _)| p.state == "OPEN").count(), 6);
        let red = prs.iter().map(|(p, _)| p).find(|p| p.number == 2177).unwrap();
        assert!(red.unresolved > 9, "the threads from the second page count: {}", red.unresolved);
        // the card shows them all too
        assert_eq!(red.details.threads.iter().filter(|t| !t.resolved).count(), red.unresolved);
        assert!(red.details.checks.iter().any(|c| c.name.ends_with("(second page)")), "{:?}", red.details.checks);
    }

    #[test]
    fn a_pr_that_vanished_is_closed_and_its_items_close() {
        let (dir, db, _, _) = run_with("ok");
        let before = db.work_items().unwrap().iter().filter(|i| i.state == "open").count();
        assert!(before > 0);
        let mode_file = dir.path().join("gh.mode");
        std::fs::write(&mode_file, "empty").unwrap();
        let gh = with_env(
            Tool::new(&format!("node {}", fake("fake-gh.mjs").display()), &std::env::var("PATH").unwrap_or_default()),
            &[("FAKE_GH_MODE_FILE", mode_file.to_str().unwrap())],
        );
        let mut p = shop();
        p.repo = Some("example-org/shop".into());
        let h = github_pass(&db, &gh, &p, &WorkSettings::default(), &mut 0, 1_793_887_500_000);
        assert_eq!(h.state, "ok");
        let prs = db.work_prs().unwrap();
        assert!(prs.iter().all(|(p, _)| p.state != "OPEN"), "every open PR is gone from GitHub's answer");
        assert!(db.work_items().unwrap().iter().all(|i| i.state == "closed"), "and their items closed themselves");
        let ev: Vec<String> = db.work_events(500).unwrap().into_iter().map(|e| e.text).collect();
        assert!(ev.iter().any(|t| t == "Closed"));
    }

    // ---- Jira, through the fake twg ----

    const NOW: i64 = 1_793_887_402_000; // 2026-11-05: the recorded comments fall in and out of the 30 days around it

    fn jira_settings() -> WorkSettings {
        WorkSettings { jira_keys: vec!["SH".into(), "OPS".into()], ..Default::default() }
    }

    /// The fake twg with its mode in a file (so a test can change it between reads) and its calls logged.
    fn fake_twg(dir: &Path, extra: &[(&str, &str)]) -> Tool {
        let mode = dir.join("twg.mode");
        if !mode.exists() {
            std::fs::write(&mode, "ok").unwrap();
        }
        let mut env = vec![("FAKE_TWG_MODE_FILE", mode.to_str().unwrap().to_string()), ("FAKE_TWG_LOG", dir.join("twg.log").to_str().unwrap().to_string())];
        env.extend(extra.iter().map(|(k, v)| (*k, v.to_string())));
        let mut t = Tool::new(&format!("node {}", fake("fake-twg.mjs").display()), &std::env::var("PATH").unwrap_or_default());
        t.env = env.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        t
    }

    fn jira_once(db: &Db, twg: &Tool, me: &mut Option<(String, String)>, failures: &mut u32, now: i64) -> Health {
        jira_pass(db, twg, &shop(), &jira_settings(), me, failures, now)
    }

    fn current_tickets(db: &Db) -> Vec<String> {
        crate::work::snapshot_of(db).unwrap().tickets.into_iter().map(|t| t.key).collect()
    }

    #[test]
    fn a_full_jira_read_through_the_fake_twg() {
        let (dir, db) = db();
        let twg = fake_twg(dir.path(), &[]);
        let (mut me, mut failures) = (None, 0);
        let h = jira_once(&db, &twg, &mut me, &mut failures, NOW);
        assert_eq!((h.state.as_str(), h.account.as_deref()), ("ok", Some("Me Dev")), "{h:?}");
        assert_eq!(me.as_ref().map(|m| m.0.as_str()), Some("acc-me"));
        let tickets = current_tickets(&db);
        assert_eq!(tickets.len(), 45, "every open ticket of yours, from all three pages");
        let whole = db.work_project_tickets("p1").unwrap();
        let read_whole = whole.iter().filter(|t| t.category == "indeterminate" && t.last_comment_by.is_some()).count();
        assert!(read_whole > 0, "the ones in progress were read with their comments");
        let items = db.work_items().unwrap();
        assert!(items.iter().all(|i| i.source == "jira" && i.ticket.is_some() && i.state == "open"));
        let log = std::fs::read_to_string(dir.path().join("twg.log")).unwrap();
        assert!(log.contains("--after cursor-1") && log.contains("--after cursor-2"), "every page of the search");
        for line in log.lines() {
            let cmd = line.split(" | ").nth(1).unwrap_or("");
            assert!(cmd.starts_with("whoami") || cmd.starts_with("jira workitem query") || cmd.starts_with("jira workitem get"), "only reads: {cmd}");
        }
        // who you are is asked once, not on every read
        jira_once(&db, &twg, &mut me, &mut failures, NOW + 600_000);
        assert_eq!(std::fs::read_to_string(dir.path().join("twg.log")).unwrap().matches("| whoami").count(), 1);
    }

    #[test]
    fn every_jira_failure_keeps_the_last_good_read_and_says_what_to_do() {
        for (mode, says, fix) in [
            ("logged-out", "twg isn't signed in", "twg login --force"),
            ("bad-site", "can't find the Jira site", "TWG_SITE"),
            ("network", "No connection to Jira", "internet connection"),
            ("rate-limit", "rate limit", "tries again"),
            ("garbage", "isn't valid JSON", ""),
            ("missing-field", "status.statusCategory.key", ""),
            ("item-error", "couldn't read", "tries again"),
            ("truncated", "only part of the tickets", "tries again"),
        ] {
            let (dir, db) = db();
            let twg = fake_twg(dir.path(), &[]);
            let (mut me, mut failures) = (None, 0);
            jira_once(&db, &twg, &mut me, &mut failures, NOW);
            let before = (db.work_project_tickets("p1").unwrap(), db.work_items().unwrap(), current_tickets(&db));
            std::fs::write(dir.path().join("twg.mode"), mode).unwrap();
            let h = jira_once(&db, &twg, &mut me, &mut failures, NOW + 600_000);
            assert_eq!(h.state, "error", "{mode}");
            assert!(h.error.as_deref().unwrap_or("").contains(says), "{mode}: {h:?}");
            assert!(h.fix.as_deref().unwrap_or("").contains(fix), "{mode}: {h:?}");
            assert_eq!(h.last_ok, Some(NOW), "{mode}: the last good read's time stays");
            assert_eq!((db.work_project_tickets("p1").unwrap(), db.work_items().unwrap(), current_tickets(&db)), before, "{mode}: nothing changes");
            if mode == "logged-out" {
                assert!(me.is_none(), "after signing in again it may be someone else");
            }
            std::fs::write(dir.path().join("twg.mode"), "ok").unwrap();
            let h = jira_once(&db, &twg, &mut me, &mut failures, NOW + 1_200_000);
            assert_eq!((h.state.as_str(), failures), ("ok", 0), "{mode}: it recovers");
        }
    }

    #[test]
    fn a_hanging_twg_is_stopped() {
        let (dir, db) = db();
        std::fs::write(dir.path().join("twg.mode"), "slow").unwrap();
        let mut twg = fake_twg(dir.path(), &[]);
        twg.max_wait = Some(std::time::Duration::from_secs(2));
        let started = std::time::Instant::now();
        let h = jira_once(&db, &twg, &mut None, &mut 0, NOW);
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        assert!(h.error.as_deref().unwrap_or("").contains("took longer"), "{h:?}");
    }

    fn board(id: &str, name: &str, kind: &str) -> Board {
        Board { id: id.into(), name: name.into(), kind: kind.into(), project_key: Some("SH".into()) }
    }

    fn picked(db: &Db) -> Option<crate::work::model::BoardPick> {
        crate::work::snapshot_of(db).unwrap().board
    }

    #[test]
    fn jiras_boards_are_read_page_by_page_and_a_project_that_isnt_there_says_so() {
        let (dir, _db) = db();
        let twg = fake_twg(dir.path(), &[]);
        let all = crate::work::jira::boards(&twg, "", "SH").unwrap();
        assert_eq!(all.len(), 8);
        assert!(all.iter().any(|b| b.name == "Shop Dev" && b.kind == "scrum" && b.id == "5"));
        std::fs::write(dir.path().join("twg.mode"), "board-pages").unwrap();
        assert_eq!(crate::work::jira::boards(&twg, "", "SH").unwrap(), all, "three pages give the same boards");
        let log = std::fs::read_to_string(dir.path().join("twg.log")).unwrap();
        assert!(log.contains("--start-at 3") && log.contains("--start-at 6"), "each page starts after the boards that came: {log}");
        std::fs::write(dir.path().join("twg.mode"), "ok").unwrap();
        let e = crate::work::jira::boards(&twg, "", "NOPE").unwrap_err();
        assert!(Failure::of(&e).message.contains("No project could be found"), "{e:#}");
        std::fs::write(dir.path().join("twg.mode"), "missing-field").unwrap();
        assert!(crate::work::jira::boards(&twg, "", "SH").unwrap_err().to_string().contains("`type`"));
    }

    #[test]
    fn the_board_you_picked_says_which_of_your_tickets_are_on_it() {
        let (dir, db) = db();
        let twg = fake_twg(dir.path(), &[]);
        let (mut me, mut failures) = (None, 0);
        // no board picked: Jira's boards aren't asked about at all
        jira_once(&db, &twg, &mut me, &mut failures, NOW);
        assert!(!std::fs::read_to_string(dir.path().join("twg.log")).unwrap().contains("jira board"));
        assert_eq!(picked(&db), None);
        // a scrum board: your tickets in the sprint that is running, in the statuses of its columns
        db.work_set_board(Some(&board("5", "Shop Dev", "scrum"))).unwrap();
        assert_eq!(picked(&db).unwrap().keys, None, "picked, not read yet");
        let h = jira_once(&db, &twg, &mut me, &mut failures, NOW + 600_000);
        assert_eq!(h.state, "ok", "{h:?}");
        let pick = picked(&db).unwrap();
        assert_eq!(pick.board.name, "Shop Dev");
        assert_eq!(pick.keys.unwrap(), ["SH-20383", "SH-20787", "SH-20844", "SH-20890", "SH-21000", "SH-21074", "SH-21349", "SH-21376"]);
        assert_eq!(pick.read_at, Some(NOW + 600_000));
        assert_eq!(current_tickets(&db).len(), 45, "every ticket of yours is still read: the board only says which are on it");
        // a kanban board: no sprint; here, what is in its In Review column
        db.work_set_board(Some(&board("9", "Release view", "kanban"))).unwrap();
        assert_eq!(picked(&db).unwrap().keys, None);
        jira_once(&db, &twg, &mut me, &mut failures, NOW + 1_200_000);
        let on = picked(&db).unwrap().keys.unwrap();
        assert_eq!(on.len(), 12, "{on:?}");
        assert!(on.contains(&"SH-21309".to_string()) && !on.contains(&"SH-21000".to_string()));
        // the first board's tickets are kept from its read: going back to it shows them at once
        db.work_set_board(Some(&board("5", "Shop Dev", "scrum"))).unwrap();
        assert_eq!(picked(&db).unwrap().keys.unwrap().len(), 8);
        // only reads, and the searches say what the board's page shows
        let log = std::fs::read_to_string(dir.path().join("twg.log")).unwrap();
        for line in log.lines() {
            let cmd = line.split(" | ").nth(1).unwrap_or("");
            let read = ["whoami", "jira workitem query", "jira workitem get", "jira board scope query"].iter().any(|c| cmd.starts_with(c));
            assert!(read, "only reads: {cmd}");
        }
        assert!(log.contains("--jql filter = 10437 AND sprint in openSprints() AND status in (10116, "), "{log}");
        assert!(log.contains("--jql filter = 10414 AND status in (10002, 10001, 10114, 10115) AND (fixVersion in unreleasedVersions() OR fixVersion is EMPTY) AND key in ("), "{log}");
    }

    #[test]
    fn a_ticket_your_pr_names_is_on_the_board_too_when_the_sprint_has_it() {
        let (dir, db) = db();
        let twg = fake_twg(dir.path(), &[]);
        // SH-21254 is done and not in your search: it is read because a PR of yours names it
        let mut pr = crate::work::links::tests::pr(2311, "feature/SH-21254-change", "SH-21254 A change");
        pr.merged_at = None;
        db.work_put_prs("p1", &[pr], NOW).unwrap();
        crate::work::links::relink(&db, &[shop()], NOW, &|_| None).unwrap();
        db.work_set_board(Some(&board("5", "Shop Dev", "scrum"))).unwrap();
        let h = jira_once(&db, &twg, &mut None, &mut 0, NOW);
        assert_eq!(h.state, "ok", "{h:?}");
        assert!(current_tickets(&db).contains(&"SH-21254".to_string()));
        assert!(picked(&db).unwrap().keys.unwrap().contains(&"SH-21254".to_string()), "in the sprint, in its Done column");
    }

    #[test]
    fn the_board_follows_the_sprint() {
        let (dir, db) = db();
        let sprint = dir.path().join("sprint.txt");
        std::fs::write(&sprint, "SH-21349,SH-21000").unwrap();
        let twg = fake_twg(dir.path(), &[("FAKE_TWG_SPRINT_FILE", sprint.to_str().unwrap())]);
        db.work_set_board(Some(&board("5", "Shop Dev", "scrum"))).unwrap();
        let (mut me, mut failures) = (None, 0);
        jira_once(&db, &twg, &mut me, &mut failures, NOW);
        assert_eq!(picked(&db).unwrap().keys.unwrap(), ["SH-21000", "SH-21349"]);
        // the sprint ended and the next one holds other tickets
        std::fs::write(&sprint, "SH-21000,SH-21376").unwrap();
        jira_once(&db, &twg, &mut me, &mut failures, NOW + 600_000);
        assert_eq!(picked(&db).unwrap().keys.unwrap(), ["SH-21000", "SH-21376"]);
        // no sprint is running: nothing is on the board, and that is an answer
        std::fs::write(&sprint, "NONE-0").unwrap();
        let h = jira_once(&db, &twg, &mut me, &mut failures, NOW + 1_200_000);
        assert_eq!(h.state, "ok");
        assert_eq!(picked(&db).unwrap().keys, Some(Vec::new()));
    }

    #[test]
    fn a_board_that_cant_be_read_fails_the_read_and_keeps_the_last_one() {
        let (dir, db) = db();
        let twg = fake_twg(dir.path(), &[]);
        db.work_set_board(Some(&board("5", "Shop Dev", "scrum"))).unwrap();
        let (mut me, mut failures) = (None, 0);
        jira_once(&db, &twg, &mut me, &mut failures, NOW);
        let before = (db.work_project_tickets("p1").unwrap(), db.work_items().unwrap(), current_tickets(&db), picked(&db));
        std::fs::write(dir.path().join("twg.mode"), "board-error").unwrap();
        let h = jira_once(&db, &twg, &mut me, &mut failures, NOW + 600_000);
        assert_eq!(h.state, "error");
        assert!(h.error.as_deref().unwrap_or("").contains("Board scope query failed"), "{h:?}");
        assert_eq!(h.last_ok, Some(NOW));
        assert_eq!((db.work_project_tickets("p1").unwrap(), db.work_items().unwrap(), current_tickets(&db), picked(&db)), before, "nothing changes, the board neither");
        std::fs::write(dir.path().join("twg.mode"), "ok").unwrap();
        let h = jira_once(&db, &twg, &mut me, &mut failures, NOW + 1_200_000);
        assert_eq!((h.state.as_str(), failures), ("ok", 0));
        assert_eq!(picked(&db).unwrap().read_at, Some(NOW + 1_200_000));
    }

    #[test]
    fn a_board_that_is_gone_from_jira_is_no_longer_picked() {
        let (dir, db) = db();
        let twg = fake_twg(dir.path(), &[]);
        db.work_set_board(Some(&board("5", "Shop Dev", "scrum"))).unwrap();
        std::fs::write(dir.path().join("twg.mode"), "board-gone").unwrap();
        let h = jira_once(&db, &twg, &mut None, &mut 0, NOW);
        assert_eq!(h.state, "ok", "the tickets were read: {h:?}");
        assert_eq!(db.work_board().unwrap(), None, "back to everything read");
        assert_eq!(current_tickets(&db).len(), 45);
        // a board Jira doesn't know (a pick from another Jira site) goes the same way
        std::fs::write(dir.path().join("twg.mode"), "ok").unwrap();
        db.work_set_board(Some(&board("777", "Somewhere else", "kanban"))).unwrap();
        jira_once(&db, &twg, &mut None, &mut 0, NOW + 600_000);
        assert_eq!(db.work_board().unwrap(), None);
    }

    #[test]
    fn asking_about_a_board_by_hand_reads_only_the_board() {
        let (dir, db) = db();
        let twg = fake_twg(dir.path(), &[]);
        jira_once(&db, &twg, &mut None, &mut 0, NOW);
        let keys = current_tickets(&db);
        std::fs::write(dir.path().join("twg.log"), "").unwrap();
        let on = board_keys(&twg, &board("5", "Shop Dev", "scrum"), &keys).unwrap().unwrap();
        assert_eq!(on.len(), 8);
        let log = std::fs::read_to_string(dir.path().join("twg.log")).unwrap();
        assert_eq!(log.lines().count(), 2, "its columns, then one search: {log}");
        // nothing read yet: nothing to ask about
        std::fs::write(dir.path().join("twg.log"), "").unwrap();
        assert_eq!(board_keys(&twg, &board("5", "Shop Dev", "scrum"), &[]).unwrap(), Some(Vec::new()));
        assert_eq!(std::fs::read_to_string(dir.path().join("twg.log")).unwrap().lines().count(), 1, "only its columns");
        // a board that is gone, and a ticket deleted since the last read (Jira fails a search that names it)
        assert_eq!(board_keys(&twg, &board("777", "Gone", "scrum"), &keys).unwrap(), None);
        let mut with_gone = keys.clone();
        with_gone.push("SH-99999999".into());
        assert!(board_keys(&twg, &board("5", "Shop Dev", "scrum"), &with_gone).is_err());
    }

    #[test]
    fn jira_is_off_for_a_project_without_jira_keys() {
        let (dir, db) = db();
        let h = jira_pass(&db, &fake_twg(dir.path(), &[]), &shop(), &WorkSettings::default(), &mut None, &mut 0, NOW);
        assert_eq!(h.state, "off");
        assert!(!dir.path().join("twg.log").exists(), "twg isn't even asked");
    }

    /// The `twg jira workitem get … --fields description,issuetype,priority` calls in the fake's log: the keys of each.
    fn describe_calls(dir: &Path) -> Vec<Vec<String>> {
        let log = std::fs::read_to_string(dir.join("twg.log")).unwrap_or_default();
        log.lines()
            .filter(|l| l.contains("--fields description,issuetype,priority"))
            .map(|l| l.split(" | ").nth(1).unwrap_or("").split(' ').filter(|w| w.contains('-') && w.chars().next().is_some_and(|c| c.is_ascii_uppercase())).map(str::to_string).collect())
            .collect()
    }

    #[test]
    fn descriptions_are_read_once_and_again_only_for_a_changed_ticket() {
        let (dir, db) = db();
        let (mut me, mut failures) = (None, 0);
        let h = jira_once(&db, &fake_twg(dir.path(), &[]), &mut me, &mut failures, NOW);
        assert_eq!(h.state, "ok", "{h:?}");
        let first = describe_calls(dir.path());
        let asked: usize = first.iter().map(Vec::len).sum();
        let tickets = db.work_project_tickets("p1").unwrap();
        assert_eq!(asked, tickets.len(), "every ticket's description is read on the first read, once");
        assert!(first.iter().all(|keys| keys.len() <= crate::work::jira::BATCH), "in batches of 25 at most");
        let t = tickets.iter().find(|t| t.key == "SH-21349").unwrap();
        assert_eq!(t.description.as_ref().map(|d| d["type"].clone()), Some(serde_json::json!("doc")));
        assert_eq!(t.described_at.as_deref(), Some(t.updated_at.as_str()));
        assert!(t.issue_type.is_some() && t.priority.is_some(), "{t:?}");
        assert!(tickets.iter().all(|t| t.described_at.as_deref() == Some(t.updated_at.as_str())), "a ticket without a description is described too");

        // nothing changed: no description is read again
        jira_once(&db, &fake_twg(dir.path(), &[]), &mut me, &mut failures, NOW + 600_000);
        assert_eq!(describe_calls(dir.path()).len(), first.len());
        assert_eq!(db.work_project_tickets("p1").unwrap(), tickets, "the descriptions are kept as they were");

        // one ticket changed: only its description is read, with one key (twg answers that one differently)
        let changed = fake_twg(dir.path(), &[("FAKE_TWG_UPDATED", "SH-21349=2026-11-09T10:00:00.000+0100")]);
        let h = jira_once(&db, &changed, &mut me, &mut failures, NOW + 1_200_000);
        assert_eq!(h.state, "ok", "{h:?}");
        let calls = describe_calls(dir.path());
        assert_eq!(calls.last().unwrap(), &["SH-21349".to_string()]);
        assert_eq!(calls.len(), first.len() + 1);
        let t = db.work_project_tickets("p1").unwrap().into_iter().find(|t| t.key == "SH-21349").unwrap();
        assert_eq!(t.described_at.as_deref(), Some("2026-11-09T10:00:00.000+0100"));
        assert!(t.description.is_some());
    }

    #[test]
    fn a_failed_description_fails_the_read_and_keeps_the_last_one() {
        // on the first read (many tickets, one failed item)
        let (dir, first) = db();
        std::fs::write(dir.path().join("twg.mode"), "describe-error").unwrap();
        let h = jira_once(&first, &fake_twg(dir.path(), &[]), &mut None, &mut 0, NOW);
        assert_eq!(h.state, "error");
        assert!(h.error.as_deref().unwrap_or("").contains("couldn't read"), "{h:?}");
        assert!(first.work_project_tickets("p1").unwrap().is_empty(), "nothing half read is kept");

        // on a later read, for the one ticket that changed (one key: the whole command fails)
        let (dir, db) = db();
        let (mut me, mut failures) = (None, 0);
        jira_once(&db, &fake_twg(dir.path(), &[]), &mut me, &mut failures, NOW);
        let before = db.work_project_tickets("p1").unwrap();
        std::fs::write(dir.path().join("twg.mode"), "describe-error").unwrap();
        let changed = fake_twg(dir.path(), &[("FAKE_TWG_UPDATED", "SH-21349=2026-11-09T10:00:00.000+0100")]);
        let h = jira_once(&db, &changed, &mut me, &mut failures, NOW + 600_000);
        assert_eq!(h.state, "error");
        assert!(h.error.as_deref().unwrap_or("").contains("Internal server error"), "{h:?}");
        assert_eq!(db.work_project_tickets("p1").unwrap(), before, "the last good read stays");
        std::fs::write(dir.path().join("twg.mode"), "ok").unwrap();
        let h = jira_once(&db, &changed, &mut me, &mut failures, NOW + 1_200_000);
        assert_eq!(h.state, "ok", "it recovers");
    }

    #[test]
    fn one_ticket_read_whole_and_one_missing_ticket_are_answers_too() {
        let (dir, _db) = db();
        let twg = fake_twg(dir.path(), &[]);
        let got = crate::work::jira::get(&twg, "", &["SH-21349".to_string()]).unwrap();
        assert_eq!(got.tickets.iter().map(|t| t.ticket.key.as_str()).collect::<Vec<_>>(), ["SH-21349"]);
        let got = crate::work::jira::get(&twg, "", &["SH-99999999".to_string()]).unwrap();
        assert!(got.tickets.is_empty());
        assert_eq!(got.gone, ["SH-99999999"], "twg fails the whole command for one missing ticket; that is an answer");
        let d = crate::work::jira::describe(&twg, "", &["SH-99999999".to_string()]).unwrap();
        assert!(d.is_empty());
        std::fs::write(dir.path().join("twg.mode"), "item-error").unwrap();
        let err = crate::work::jira::get(&twg, "", &["SH-21349".to_string()]).unwrap_err();
        assert!(crate::work::run::Failure::of(&err).message.contains("Internal server error"), "{err:#}");
    }

    #[test]
    fn a_deleted_ticket_leaves_the_page_and_its_items_close() {
        let (dir, db) = db();
        let (mut me, mut failures) = (None, 0);
        jira_once(&db, &fake_twg(dir.path(), &[]), &mut me, &mut failures, NOW);
        let key = "SH-20598"; // in progress, with comments
        assert!(current_tickets(&db).contains(&key.to_string()));
        let had_items = db.work_items().unwrap().iter().any(|i| i.ticket.as_deref() == Some(key));
        // Jira now says it doesn't exist (and the search no longer finds it either)
        let gone = fake_twg(dir.path(), &[("FAKE_TWG_GONE", key), ("FAKE_TWG_STATUS", &format!("{key}=Done/done"))]);
        let h = jira_once(&db, &gone, &mut me, &mut failures, NOW + 600_000);
        assert_eq!(h.state, "ok", "a deleted ticket is an answer, not a failure: {h:?}");
        assert!(!current_tickets(&db).contains(&key.to_string()));
        assert!(db.work_events(500).unwrap().iter().any(|e| e.subject == key && e.kind == "ticket-gone"));
        if had_items {
            assert!(db.work_items().unwrap().iter().filter(|i| i.ticket.as_deref() == Some(key)).all(|i| i.state == "closed"));
        }
    }

    #[test]
    fn a_ticket_given_to_someone_else_leaves_the_page_and_says_so() {
        let (dir, db) = db();
        let (mut me, mut failures) = (None, 0);
        jira_once(&db, &fake_twg(dir.path(), &[]), &mut me, &mut failures, NOW);
        let key = "SH-20598";
        assert!(current_tickets(&db).contains(&key.to_string()));
        let theirs = fake_twg(dir.path(), &[("FAKE_TWG_ASSIGNEE", &format!("{key}=acc-qa/Jonas QA"))]);
        let h = jira_once(&db, &theirs, &mut me, &mut failures, NOW + 600_000);
        assert_eq!(h.state, "ok", "{h:?}");
        assert!(!current_tickets(&db).contains(&key.to_string()));
        assert!(db.work_events(500).unwrap().iter().any(|e| e.subject == key && e.text == "No longer yours: assigned to Jonas QA"));
        // a PR of yours still names it: then it stays, whoever it is assigned to (QA tests it, the PR is yours)
        let mut p = crate::work::links::tests::pr(2446, &format!("feature/{key}-x"), "x");
        p.state = "OPEN".into();
        db.work_put_prs("p1", &[p], NOW).unwrap();
        crate::work::links::relink(&db, &[shop()], NOW, &|_| None).unwrap();
        jira_once(&db, &theirs, &mut me, &mut failures, NOW + 1_200_000);
        assert!(current_tickets(&db).contains(&key.to_string()));
    }

    #[test]
    fn a_moved_ticket_keeps_its_prs() {
        let (dir, db) = db();
        // a PR that names the first ticket read whole
        let mut p = crate::work::links::tests::pr(2350, "feature/OPS-6294-x", "OPS-6294 x");
        p.state = "OPEN".into();
        db.work_put_prs("p1", &[p], NOW).unwrap();
        crate::work::links::relink(&db, &[shop()], NOW, &|_| None).unwrap();
        std::fs::write(dir.path().join("twg.mode"), "moved").unwrap();
        let h = jira_once(&db, &fake_twg(dir.path(), &[]), &mut None, &mut 0, NOW);
        assert_eq!(h.state, "ok", "{h:?}");
        assert!(db.work_events(500).unwrap().iter().any(|e| e.subject == "OPS-6294" && e.text == "Moved to XY-6294"));
        assert!(db.work_links().unwrap().iter().any(|l| l.a == "ticket:XY-6294" && l.b == "pr:Org/shop#2350"), "the PR follows its ticket");
    }

    #[test]
    fn back_to_you_opens_while_the_ticket_stays_back_and_closes_when_it_moves_on() {
        let (dir, db) = db();
        let (mut me, mut failures) = (None, 0);
        let key = "SH-21349";
        let at = |status: &str| fake_twg(dir.path(), &[("FAKE_TWG_STATUS", &format!("{key}={status}/indeterminate"))]);
        let back = || db.work_items().unwrap().into_iter().find(|i| i.kind == "back_to_you");
        jira_once(&db, &at("QA Review"), &mut me, &mut failures, NOW);
        assert!(back().is_none());
        jira_once(&db, &at("In Progress"), &mut me, &mut failures, NOW + 600_000);
        let item = back().expect("QA sent it back");
        assert_eq!((item.state.as_str(), item.title.as_str()), ("open", "SH-21349 is back to you: QA Review → In Progress"));
        assert!(db.work_events(500).unwrap().iter().any(|e| e.subject == key && e.text == "QA Review → In Progress"));
        jira_once(&db, &at("In Progress"), &mut me, &mut failures, NOW + 1_200_000);
        assert_eq!(back().unwrap().state, "open", "still back with you");
        jira_once(&db, &at("Code Review"), &mut me, &mut failures, NOW + 1_800_000);
        assert_eq!(back().unwrap().state, "closed", "it moved on");
    }
}
