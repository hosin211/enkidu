//! Live checks against your real accounts, read-only: what the index holds must match what `gh` and `twg` say
//! directly. Run during the build at the end of every phase (`cargo test --lib live_ -- --ignored --nocapture`, with
//! the project given in `ENKIDU_LIVE_PROJECT`), and later from "Check connections" and `enkidu doctor`.
//! Nothing here writes to GitHub, Jira or the project; nothing prints a token or a remote URL.
//!
//! The comparison deliberately uses other commands than the watchers do (`gh pr list`, `gh pr checks`, `gh pr view`
//! and a minimal GraphQL query, where the watcher uses one big query), so a mistake in the watcher's reading shows.

use anyhow::{anyhow, Result};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeSet;
use std::time::Duration;

use super::github;
use super::links::ProjectInfo;
use super::model::Pr;
use super::run::{Failure, Tool};
use crate::db::Db;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Difference {
    /// What differs: `#2350` or `SH-21309`
    pub thing: String,
    pub field: String,
    pub index: String,
    pub direct: String,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub source: String,
    pub project_id: String,
    /// What was compared, in words
    pub compared: Vec<String>,
    pub differences: Vec<Difference>,
    /// The source couldn't be read directly
    pub error: Option<String>,
}

impl Report {
    pub fn ok(&self) -> bool {
        self.error.is_none() && self.differences.is_empty()
    }

    fn differ(&mut self, thing: &str, field: &str, index: impl ToString, direct: impl ToString) {
        let (index, direct) = (index.to_string(), direct.to_string());
        if index != direct {
            self.differences.push(Difference { thing: thing.into(), field: field.into(), index, direct });
        }
    }
}

/// Runs gh with the account's token and parses its JSON (gh exits 8 while checks are pending, 1 when some failed:
/// what counts is the answer).
fn gh_json(gh: &Tool, token: &str, args: &[&str]) -> Result<Value> {
    let out = gh.run(args, &[("GH_TOKEN", token)], Duration::from_secs(60))?;
    serde_json::from_str(&out.stdout).map_err(|_| {
        anyhow!(github::explain(out.code, &out.stderr, ""))
    })
}

const THREADS: &str = "query($o: String!, $n: String!, $p: Int!) { repository(owner: $o, name: $n) { pullRequest(number: $p) { \
                       reviewThreads(first: 100) { totalCount nodes { isResolved } } } } }";

/// Compares the index's open PRs of a project with what gh says directly, field by field.
pub fn check_github(db: &Db, gh: &Tool, project: &ProjectInfo, account: &str) -> Report {
    let mut r = Report { source: "github".into(), project_id: project.id.clone(), ..Default::default() };
    if let Err(e) = compare_github(db, gh, project, account, &mut r) {
        r.error = Some(Failure::of(&e).message);
    }
    r
}

fn compare_github(db: &Db, gh: &Tool, project: &ProjectInfo, account: &str, r: &mut Report) -> Result<()> {
    let repo = project.repo.clone().ok_or_else(|| anyhow!("This project's origin isn't on GitHub."))?;
    let (owner, name) = repo.split_once('/').ok_or_else(|| anyhow!("not a repo name"))?;
    let token = github::token(gh, account)?;
    let index: Vec<Pr> = db.work_prs()?.into_iter().map(|(p, _)| p).filter(|p| p.project_id == project.id && p.state == "OPEN").collect();

    // the open PRs, as `gh pr list` has them
    let listed = gh_json(gh, &token, &[
        "pr", "list", "--repo", &repo, "--author", "@me", "--state", "open", "--limit", "200",
        "--json", "number,title,isDraft,headRefName,baseRefName,reviewDecision,mergeable,state",
    ])?;
    let listed = listed.as_array().cloned().unwrap_or_default();
    let numbers = |v: &[i64]| v.iter().map(|n| format!("#{n}")).collect::<Vec<_>>().join(" ");
    let direct: BTreeSet<i64> = listed.iter().filter_map(|p| p.get("number").and_then(Value::as_i64)).collect();
    let mine: BTreeSet<i64> = index.iter().map(|p| p.number).collect();
    r.compared.push(format!("{} open PRs as `gh pr list --author @me` lists them", direct.len()));
    r.differ(&repo, "open PRs", numbers(&mine.iter().copied().collect::<Vec<_>>()), numbers(&direct.iter().copied().collect::<Vec<_>>()));
    let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    for p in &listed {
        let n = p.get("number").and_then(Value::as_i64).unwrap_or(0);
        let Some(pr) = index.iter().find(|x| x.number == n) else { continue };
        let thing = format!("#{n}");
        r.differ(&thing, "title", &pr.title, text(p, "title"));
        r.differ(&thing, "draft", pr.draft, p.get("isDraft").and_then(Value::as_bool).unwrap_or(false));
        r.differ(&thing, "head", &pr.head, text(p, "headRefName"));
        r.differ(&thing, "base", &pr.base, text(p, "baseRefName"));
        r.differ(&thing, "review", pr.review_decision.clone().unwrap_or_default(), text(p, "reviewDecision"));
        let (a, b) = (pr.mergeable.clone().unwrap_or_default(), text(p, "mergeable"));
        if a != "UNKNOWN" && b != "UNKNOWN" {
            r.differ(&thing, "mergeable", a, b); // GitHub works this out lazily: UNKNOWN on either side isn't a difference
        }
    }

    // per PR: the latest commit's checks, the open review threads, the latest comment
    for pr in &index {
        let thing = format!("#{}", pr.number);
        let n = pr.number.to_string();
        let checks = match gh_json(gh, &token, &["pr", "checks", &n, "--repo", &repo, "--json", "name,state,bucket"]) {
            Ok(v) => v,
            // a PR without any checks: gh says so and exits 1, even with --json; that is an empty list, not a failure
            Err(e) if e.to_string().contains("no checks reported") => Value::Array(Vec::new()),
            Err(e) => return Err(e),
        };
        let checks = checks.as_array().cloned().unwrap_or_default();
        let bucket = |b: &str| -> BTreeSet<String> { checks.iter().filter(|c| text(c, "bucket") == b).map(|c| text(c, "name")).collect() };
        let mut failing = bucket("fail");
        failing.extend(bucket("cancel"));
        let in_index: BTreeSet<String> = pr.failing.iter().map(|c| c.name.clone()).collect();
        r.differ(&thing, "failing checks", in_index.into_iter().collect::<Vec<_>>().join(", "), failing.into_iter().collect::<Vec<_>>().join(", "));
        r.differ(&thing, "running checks", pr.running, bucket("pending").len());
        // every check the card shows, by name and how it stands
        let mine: Vec<String> = pr.details.checks.iter().map(|c| format!("{}: {}", c.name, c.bucket)).collect();
        let theirs: Vec<String> = checks.iter().map(|c| format!("{}: {}", text(c, "name"), text(c, "bucket"))).collect();
        r.differ(&thing, "checks", sorted(mine), sorted(theirs));

        let threads = gh_json(gh, &token, &["api", "graphql", "-f", &format!("query={THREADS}"), "-f", &format!("o={owner}"), "-f", &format!("n={name}"), "-F", &format!("p={n}")])?;
        let nodes = threads.pointer("/data/repository/pullRequest/reviewThreads/nodes").and_then(Value::as_array).cloned().unwrap_or_default();
        let unresolved = nodes.iter().filter(|t| t.get("isResolved").and_then(Value::as_bool) == Some(false)).count();
        r.differ(&thing, "open review threads", pr.unresolved, unresolved);

        let viewed = gh_json(gh, &token, &[
            "pr", "view", &n, "--repo", &repo, "--json", "comments,additions,deletions,changedFiles,author,labels,reviewRequests,latestReviews",
        ])?;
        let comments = viewed.get("comments").and_then(Value::as_array).cloned().unwrap_or_default();
        let last = comments.iter().max_by_key(|c| text(c, "createdAt"));
        let by = last.and_then(|c| c.pointer("/author/login")).and_then(Value::as_str).unwrap_or("").to_string();
        r.differ(&thing, "latest comment by", pr.last_comment_by.clone().unwrap_or_default(), by);
        r.differ(&thing, "latest comment at", pr.last_comment_at.clone().unwrap_or_default(), last.map(|c| text(c, "createdAt")).unwrap_or_default());

        // what the card in front of the board says about it
        let number = |n: Option<i64>| n.map(|n| n.to_string()).unwrap_or_default();
        let d = &pr.details;
        r.differ(&thing, "additions", number(d.additions), number(viewed.get("additions").and_then(Value::as_i64)));
        r.differ(&thing, "deletions", number(d.deletions), number(viewed.get("deletions").and_then(Value::as_i64)));
        r.differ(&thing, "changed files", number(d.changed_files), number(viewed.get("changedFiles").and_then(Value::as_i64)));
        r.differ(&thing, "author", d.author.clone().unwrap_or_default(), viewed.pointer("/author/login").and_then(Value::as_str).unwrap_or(""));
        let list = |k: &str| viewed.get(k).and_then(Value::as_array).cloned().unwrap_or_default();
        let labels: Vec<String> = list("labels").iter().map(|l| text(l, "name")).collect();
        r.differ(&thing, "labels", sorted(d.labels.clone()), sorted(labels));
        // a person by their login, a team by its name
        let asked: Vec<String> = list("reviewRequests").iter().map(|x| if x.get("login").is_some() { text(x, "login") } else { text(x, "name") }).collect();
        r.differ(&thing, "asked to review", sorted(d.requested.clone()), sorted(asked));
        let reviews: Vec<String> =
            list("latestReviews").iter().map(|x| format!("{}: {}", x.pointer("/author/login").and_then(Value::as_str).unwrap_or(""), text(x, "state"))).collect();
        r.differ(&thing, "latest reviews", sorted(d.reviews.iter().map(|x| format!("{}: {}", x.who, x.state)).collect()), sorted(reviews));
        r.compared.push(format!("{thing}: checks, review threads, latest comment, size, author, labels, reviewers, reviews"));
    }
    Ok(())
}

/// A list as one sorted line, so the same things in another order are the same.
fn sorted(mut v: Vec<String>) -> String {
    v.sort();
    v.join(", ")
}

/// Runs twg with JSON output; a `{ok: false}` answer is a failure in words.
fn twg_json(twg: &Tool, args: &[&str]) -> Result<Value> {
    let mut all = args.to_vec();
    all.extend(["--output", "json", "--output-summary", "none"]);
    let out = twg.run(&all, &[], Duration::from_secs(90))?;
    let v: Value = serde_json::from_str(&out.stdout).map_err(|_| anyhow!(super::jira::explain(out.code, &Value::Null, &out.stderr)))?;
    if v.get("ok") == Some(&Value::Bool(false)) {
        return Err(anyhow!(super::jira::explain(out.code, &v, &out.stderr)));
    }
    Ok(v)
}

/// Compares the index's tickets of a project with what twg says directly: the same search, the tickets the PRs
/// name, and for every ticket read whole its status, assignee (Jira's bulk API) and latest comment (its comments).
pub fn check_jira(db: &Db, twg: &Tool, project: &ProjectInfo, ws: &crate::settings::WorkSettings, now: i64) -> Report {
    let mut r = Report { source: "jira".into(), project_id: project.id.clone(), ..Default::default() };
    if let Err(e) = compare_jira(db, twg, project, ws, now, &mut r) {
        r.error = Some(Failure::of(&e).message);
    }
    r
}

fn compare_jira(db: &Db, twg: &Tool, project: &ProjectInfo, ws: &crate::settings::WorkSettings, now: i64, r: &mut Report) -> Result<()> {
    let text = |v: &Value, p: &str| v.pointer(p).and_then(Value::as_str).unwrap_or("").to_string();
    // the search, asked directly, every page
    let jql = super::watch::assigned_jql(&ws.jira_keys);
    let mut direct: BTreeSet<String> = BTreeSet::new();
    let mut after: Option<String> = None;
    loop {
        let mut args = vec!["jira", "workitem", "query", "--jql", jql.as_str(), "--limit", "100"];
        if let Some(a) = &after {
            args.extend(["--after", a.as_str()]);
        }
        let v = twg_json(twg, &args)?;
        for i in v.pointer("/data/issues").and_then(Value::as_array).into_iter().flatten() {
            direct.insert(text(i, "/key"));
        }
        match v.pointer("/pageInfo/nextCursor").and_then(Value::as_str) {
            Some(c) if v.pointer("/pageInfo/hasNextPage") == Some(&Value::Bool(true)) => after = Some(c.to_string()),
            _ => break,
        }
    }
    r.compared.push(format!("{} open tickets of yours, as the same search through twg finds them", direct.len()));
    // the tickets your PRs name, asked directly (those Jira knows)
    let linked = super::watch::linked_keys(db, &project.id, &ws.jira_keys, now)?;
    if !linked.is_empty() {
        let mut args = vec!["jira", "workitem", "bulk-get"];
        args.extend(linked.iter().map(String::as_str));
        args.extend(["--fields", "status"]);
        let v = twg_json(twg, &args)?;
        for i in v.pointer("/data/items").and_then(Value::as_array).into_iter().flatten() {
            if i.get("ok") == Some(&Value::Bool(true)) {
                direct.insert(text(i, "/data/key"));
            }
        }
        r.compared.push(format!("{} tickets your PRs name", linked.len()));
    }
    let latest = db.work_latest_reads("work_tickets")?;
    let index: Vec<super::model::Ticket> = db
        .work_ticket_rows()?
        .into_iter()
        .filter(|(_, p, seen)| p == &project.id && latest.get(p) == Some(seen))
        .map(|(t, _, _)| t)
        .collect();
    let keys: BTreeSet<String> = index.iter().map(|t| t.key.clone()).collect();
    let joined = |s: &BTreeSet<String>| s.iter().cloned().collect::<Vec<_>>().join(" ");
    r.differ(&project.id, "tickets", joined(&keys), joined(&direct));

    // every ticket read whole: status, category, assignee through Jira's bulk API; the latest comment from its comments
    let whole: Vec<&super::model::Ticket> = index.iter().filter(|t| t.category != "new").collect();
    for chunk in whole.chunks(25) {
        let mut args = vec!["jira", "workitem", "bulk-get"];
        args.extend(chunk.iter().map(|t| t.key.as_str()));
        args.extend(["--fields", "status,assignee"]);
        let v = twg_json(twg, &args)?;
        for i in v.pointer("/data/items").and_then(Value::as_array).into_iter().flatten() {
            let key = text(i, "/data/key");
            let Some(t) = chunk.iter().find(|t| t.key == key) else { continue };
            r.differ(&key, "status", &t.status, text(i, "/data/status/name"));
            r.differ(&key, "category", &t.category, text(i, "/data/status/statusCategory/key"));
            r.differ(&key, "assignee", t.assignee_id.clone().unwrap_or_default(), text(i, "/data/assignee/accountId"));
        }
    }
    for t in &whole {
        let mut all: Vec<Value> = Vec::new();
        let mut after: Option<String> = None;
        loop {
            let mut args = vec!["jira", "workitem", "comment", "query", "--issue-id", t.key.as_str()];
            if let Some(a) = &after {
                args.extend(["--after", a.as_str()]);
            }
            let v = twg_json(twg, &args)?;
            all.extend(v.get("data").and_then(Value::as_array).cloned().unwrap_or_default());
            match v.pointer("/pageInfo/nextCursor").and_then(Value::as_str) {
                Some(c) if v.pointer("/pageInfo/hasNextPage") == Some(&Value::Bool(true)) => after = Some(c.to_string()),
                _ => break,
            }
        }
        let last = all.iter().max_by_key(|c| super::parse::parse_time(&text(c, "/created")).unwrap_or(0));
        r.differ(&t.key, "comments", t.comments, all.len());
        r.differ(&t.key, "latest comment by", t.last_comment_by.clone().unwrap_or_default(), last.map(|c| text(c, "/author/displayName")).unwrap_or_default());
        r.differ(&t.key, "latest comment at", t.last_comment_at.clone().unwrap_or_default(), last.map(|c| text(c, "/created")).unwrap_or_default());
    }
    r.compared.push(format!("{} tickets read whole: status, assignee, comments, latest comment", whole.len()));

    // every ticket on the board: its status, type, priority and what it asks for (its description, as text)
    for chunk in index.chunks(25) {
        let mut args = vec!["jira", "workitem", "bulk-get"];
        args.extend(chunk.iter().map(|t| t.key.as_str()));
        args.extend(["--fields", "status,issuetype,priority,description"]);
        let v = twg_json(twg, &args)?;
        for i in v.pointer("/data/items").and_then(Value::as_array).into_iter().flatten() {
            let key = text(i, "/data/key");
            let Some(t) = chunk.iter().find(|t| t.key == key) else { continue };
            r.differ(&key, "status", &t.status, text(i, "/data/status/name"));
            r.differ(&key, "type", t.issue_type.clone().unwrap_or_default(), text(i, "/data/issuetype/name"));
            r.differ(&key, "priority", t.priority.clone().unwrap_or_default(), text(i, "/data/priority/name"));
            let mine = t.description.as_ref().map(|d| super::jira::adf_text(d).0).unwrap_or_default();
            let theirs = super::jira::kept_description(i.pointer("/data/description")).map(|d| super::jira::adf_text(&d).0).unwrap_or_default();
            if mine != theirs {
                r.differ(&key, "description", about(&mine), about(&theirs));
            }
        }
    }
    r.compared.push(format!("{} tickets on the board: status, type, priority, description", index.len()));
    Ok(())
}

/// A description in a difference line: how long it is and how it starts.
fn about(text: &str) -> String {
    if text.is_empty() {
        return "none".into();
    }
    let start: String = text.chars().take(48).collect();
    let more = if text.chars().count() > 48 { "…" } else { "" };
    format!("{} characters, “{}{more}”", text.chars().count(), start.replace('\n', " "))
}

#[cfg(test)]
mod tests {
    use crate::db::{now_ms, Db, Project};
    use crate::shellenv::{expand_tilde, home_dir};
    use crate::work::links::{relink, ProjectInfo};
    use crate::work::{github, run::Tool, transcripts};
    use std::path::PathBuf;

    fn env(name: &str) -> String {
        std::env::var(name).unwrap_or_else(|_| panic!("set {name} for the live check"))
    }

    /// The fake gh, as the app runs it, with extra environment.
    fn fake_gh(extra: &[(&str, &str)]) -> Tool {
        let mut gh = Tool::new(&format!("node {}", crate::work::watch::tests::fake("fake-gh.mjs").display()), &std::env::var("PATH").unwrap_or_default());
        gh.env = [("FAKE_GH_MODE", "ok")].iter().chain(extra).map(|(k, v)| (k.to_string(), v.to_string())).collect();
        gh
    }

    #[test]
    fn the_check_matches_the_recorded_prs_and_a_pr_without_any_checks_is_no_failure() {
        let (_d, db) = crate::work::store::tests::db();
        let mut project = crate::work::links::tests::shop();
        project.repo = Some("example-org/shop".into());
        let settings = crate::settings::WorkSettings::default();
        for extra in [vec![], vec![("FAKE_GH_NO_CHECKS", "2265")]] {
            let gh = fake_gh(&extra);
            let mut failures = 0;
            let h = crate::work::watch::github_pass(&db, &gh, &project, &settings, &mut failures, now_ms());
            assert_eq!(h.state, "ok", "{:?}", h.error);
            // gh says "no checks reported on the … branch" and exits 1 for such a PR: that is an empty list
            let r = super::check_github(&db, &gh, &project, "me-dev");
            assert_eq!((r.error.clone(), r.differences.len()), (None, 0), "{extra:?}: {:?}", r.differences);
            assert!(r.compared.iter().any(|c| c.starts_with("#2265")));
        }
    }

    #[test]
    fn the_check_matches_the_recorded_tickets_on_the_board_and_names_a_wrong_description() {
        let (_d, db) = crate::work::store::tests::db();
        let project = crate::work::links::tests::shop();
        let ws = crate::settings::WorkSettings { jira_keys: vec!["SH".into(), "OPS".into()], ..Default::default() };
        let mut twg = Tool::new(&format!("node {}", crate::work::watch::tests::fake("fake-twg.mjs").display()), &std::env::var("PATH").unwrap_or_default());
        twg.env = vec![("FAKE_TWG_MODE".into(), "ok".into())];
        let h = crate::work::watch::jira_pass(&db, &twg, &project, &ws, &mut None, &mut 0, now_ms());
        assert_eq!(h.state, "ok", "{:?}", h.error);
        let r = super::check_jira(&db, &twg, &project, &ws, now_ms());
        assert_eq!((r.error.clone(), r.differences.len()), (None, 0), "{:?}", r.differences);
        assert!(r.compared.iter().any(|c| c.ends_with("tickets on the board: status, type, priority, description")), "{:?}", r.compared);

        // the index keeps a description Jira no longer has: the check names the ticket and both texts' starts
        let mut tickets = db.work_project_tickets("p1").unwrap();
        let t = tickets.iter_mut().find(|t| t.key == "SH-21349").unwrap();
        t.description = Some(serde_json::json!({"type": "doc", "version": 1, "content": [{"type": "paragraph", "content": [{"type": "text", "text": "An old text"}]}]}));
        t.priority = Some("Highest".into());
        db.work_put_tickets("p1", &tickets, now_ms()).unwrap();
        let r = super::check_jira(&db, &twg, &project, &ws, now_ms());
        let found: Vec<String> = r.differences.iter().map(|d| format!("{} {}: {} | {}", d.thing, d.field, d.index, d.direct)).collect();
        assert!(found.iter().any(|d| d.starts_with("SH-21349 description: 11 characters, “An old text” | ")), "{found:?}");
        assert!(found.iter().any(|d| d.starts_with("SH-21349 priority: Highest | Medium")), "{found:?}");
    }

    /// Phase 1: the transcript index, run on your real saved sessions, links the session that made a PR to it.
    /// `ENKIDU_LIVE_PROJECT` = the project's main folder, `ENKIDU_LIVE_PR` = a PR number of yours (open or merged in the
    /// last 30 days), `ENKIDU_LIVE_SESSION` = the start of the session id that created it.
    #[test]
    #[ignore = "live: reads your saved sessions and GitHub"]
    fn live_phase1_transcripts_link_your_real_sessions() {
        let project_path = expand_tilde(&env("ENKIDU_LIVE_PROJECT"));
        let number: i64 = env("ENKIDU_LIVE_PR").parse().unwrap();
        let session_start = env("ENKIDU_LIVE_SESSION");
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("live.db")).unwrap();
        let path = project_path.to_string_lossy().to_string();
        db.insert_project(&Project { id: "p1".into(), name: "live".into(), path: path.clone(), live_workspace_id: None, created_at: 0 }).unwrap();
        let repo = github::repo_slug(&project_path).expect("the project's origin is on GitHub");
        let gh = Tool::new("gh", &std::env::var("PATH").unwrap_or_default());
        let (account, token) = github::pick_account(&gh, &repo, None).unwrap();
        let since = chrono_days_ago(30);
        let read = github::read(&gh, &token, &account, &repo, "p1", &since).unwrap();
        let mut prs: Vec<_> = read.open.iter().map(|o| o.pr.clone()).collect();
        prs.extend(read.merged.iter().cloned());
        db.work_put_prs("p1", &prs, now_ms()).unwrap();
        println!("GitHub as {account}: {} open, {} merged in 30 days", read.open.len(), read.merged.len());

        // every Claude folder of this Mac that has saved sessions: `~/.claude`, and any other `~/.claude*`
        let configs: Vec<PathBuf> = std::fs::read_dir(home_dir()).into_iter().flatten().flatten().map(|e| e.path())
            .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(".claude")) && p.join("projects").is_dir()).collect();
        let started = std::time::Instant::now();
        let stats = transcripts::scan(&db, &configs, now_ms() - 60 * 86_400_000).unwrap();
        println!(
            "transcripts: {} files in 60 days, {} read, {} MB in {:.1} s",
            stats.files,
            stats.read,
            stats.bytes / 1_000_000,
            started.elapsed().as_secs_f64()
        );
        let again = std::time::Instant::now();
        let second = transcripts::scan(&db, &configs, now_ms() - 60 * 86_400_000).unwrap();
        println!("second scan: {} read, {} KB in {:.2} s", second.read, second.bytes / 1000, again.elapsed().as_secs_f64());

        let info = ProjectInfo {
            id: "p1".into(),
            path,
            workspaces_dir: crate::shellenv::expand_tilde(crate::places::WORKSPACES_DIR).join(crate::workspace::project_slug(&project_path.file_name().unwrap().to_string_lossy())).to_string_lossy().to_string(),
            repo: Some(repo.clone()),
            base: String::new(),
            jira_keys: Vec::new(),
        };
        let hub = crate::work::Hub::default();
        relink(&db, &[info], now_ms(), &|folder| hub.folder_repo(folder)).unwrap();
        let links = db.work_links().unwrap();
        for pr in &prs {
            let r = format!("pr:{}", pr.reference());
            let mine: Vec<String> = links
                .iter()
                .filter(|l| l.a == r && l.b.starts_with("session:"))
                .map(|l| format!("{} ({}: {})", l.b["session:".len()..]. chars().take(8).collect::<String>(), l.how, l.detail))
                .collect();
            println!("#{} {} [{}]: {}", pr.number, pr.head, pr.state, if mine.is_empty() { "no session".to_string() } else { mine.join(", ") });
        }
        let wanted = format!("pr:{repo}#{number}");
        let found = links.iter().any(|l| l.a == wanted && l.b.starts_with(&format!("session:{session_start}")));
        assert!(found, "the session {session_start}… is not linked to #{number}");

        // Jira: the tickets the PRs link to by key are real, and twg reads them with your login
        let twg = Tool::new("twg", &std::env::var("PATH").unwrap_or_default());
        let (_, me) = crate::work::jira::whoami(&twg, "").expect("twg is signed in");
        let mut keys: Vec<String> = links.iter().filter(|l| l.how == "ticket-key").map(|l| l.a.trim_start_matches("ticket:").to_string()).collect();
        keys.sort();
        keys.dedup();
        let got = crate::work::jira::get(&twg, "", &keys).unwrap();
        println!("Jira as {me}: {} tickets linked by key, {} read", keys.len(), got.tickets.len());
        for t in &got.tickets {
            println!("  {} [{}] {} comments, {} attachments", t.ticket.key, t.ticket.status, t.comments.len(), t.attachments.len());
        }
        assert!(got.gone.is_empty(), "keys in branch names that Jira doesn't know: {:?}", got.gone);
        assert_eq!(got.tickets.len(), keys.len());
    }

    /// Phase 2: the GitHub watcher's read of your real PRs matches what gh says directly, field by field.
    #[test]
    #[ignore = "live: reads GitHub"]
    fn live_phase2_index_matches_gh() {
        let project_path = expand_tilde(&env("ENKIDU_LIVE_PROJECT"));
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("live.db")).unwrap();
        let path = project_path.to_string_lossy().to_string();
        db.insert_project(&Project { id: "p1".into(), name: "live".into(), path: path.clone(), live_workspace_id: None, created_at: 0 }).unwrap();
        let repo = github::repo_slug(&project_path).expect("the project's origin is on GitHub");
        let gh = Tool::new("gh", &std::env::var("PATH").unwrap_or_default());
        let info = ProjectInfo { id: "p1".into(), path, repo: Some(repo), ..Default::default() };
        // the app's own watcher reads it
        let health = crate::work::watch::github_pass(&db, &gh, &info, &crate::settings::WorkSettings::default(), &mut 0, now_ms());
        assert_eq!(health.state, "ok", "{health:?}");
        let account = health.account.clone().unwrap();
        let items = db.work_items().unwrap();
        let mut kinds: Vec<String> = items.iter().map(|i| format!("{}{}", i.kind, if i.bot { " (bot)" } else { "" })).collect();
        kinds.sort();
        kinds.dedup_by(|a, b| a == b);
        println!("watcher as {account}: {} PRs, {} items waiting: {}", db.work_prs().unwrap().len(), items.len(), kinds.join(", "));
        for (p, _) in db.work_prs().unwrap().iter().filter(|(p, _)| p.state == "OPEN") {
            let mine: Vec<&str> = items.iter().filter(|i| i.pr.as_deref() == Some(&p.reference())).map(|i| i.kind.as_str()).collect();
            println!("  #{} {}: {} | {}", p.number, p.head, crate::work::pr_summary(p), mine.join(" "));
        }
        // and gh, asked directly, agrees
        let report = super::check_github(&db, &gh, &info, &account);
        for c in &report.compared {
            println!("  compared {c}");
        }
        for d in &report.differences {
            println!("  DIFFERENT {} {}: index {:?}, gh {:?}", d.thing, d.field, d.index, d.direct);
        }
        assert!(report.ok(), "{:?} {:?}", report.error, report.differences);
    }

    /// Phase 3: the Jira watcher's read matches twg asked directly (after the GitHub read, so the tickets your PRs name
    /// are in it). `ENKIDU_LIVE_JIRA` = the project's Jira keys (`SH`).
    #[test]
    #[ignore = "live: reads GitHub and Jira"]
    fn live_phase3_index_matches_twg() {
        let project_path = expand_tilde(&env("ENKIDU_LIVE_PROJECT"));
        let jira_keys: Vec<String> = env("ENKIDU_LIVE_JIRA").split(',').map(|k| k.trim().to_string()).collect();
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("live.db")).unwrap();
        let path = project_path.to_string_lossy().to_string();
        db.insert_project(&Project { id: "p1".into(), name: "live".into(), path: path.clone(), live_workspace_id: None, created_at: 0 }).unwrap();
        let repo = github::repo_slug(&project_path).expect("the project's origin is on GitHub");
        let path_env = std::env::var("PATH").unwrap_or_default();
        let (gh, twg) = (Tool::new("gh", &path_env), Tool::new("twg", &path_env));
        let info = ProjectInfo { id: "p1".into(), path, repo: Some(repo), jira_keys: jira_keys.clone(), ..Default::default() };
        let ws = crate::settings::WorkSettings { jira_keys, ..Default::default() };
        let now = now_ms();
        let gh_health = crate::work::watch::github_pass(&db, &gh, &info, &ws, &mut 0, now);
        assert_eq!(gh_health.state, "ok", "{gh_health:?}");
        relink(&db, std::slice::from_ref(&info), now, &|_| None).unwrap();
        let started = std::time::Instant::now();
        let mut me = None;
        let health = crate::work::watch::jira_pass(&db, &twg, &info, &ws, &mut me, &mut 0, now_ms());
        assert_eq!(health.state, "ok", "{health:?}");
        let tickets = crate::work::snapshot_of(&db).unwrap().tickets;
        let items: Vec<_> = db.work_items().unwrap().into_iter().filter(|i| i.source == "jira").collect();
        println!(
            "Jira as {}: {} tickets ({} in progress) in {:.1} s; {} items: {}",
            health.account.unwrap_or_default(),
            tickets.len(),
            tickets.iter().filter(|t| t.category == "indeterminate").count(),
            started.elapsed().as_secs_f64(),
            items.len(),
            items.iter().map(|i| format!("{} {}", i.kind, i.ticket.clone().unwrap_or_default())).collect::<Vec<_>>().join(", ")
        );
        let report = super::check_jira(&db, &twg, &info, &ws, now_ms());
        for c in &report.compared {
            println!("  compared {c}");
        }
        for d in &report.differences {
            println!("  DIFFERENT {} {}: index {:?}, twg {:?}", d.thing, d.field, d.index, d.direct);
        }
        assert!(report.ok(), "{:?} {:?}", report.error, report.differences);
    }

    /// Phase 4: both sources at once, as the app reads them, and what the Work page is sent holds exactly that.
    #[test]
    #[ignore = "live: reads GitHub and Jira"]
    fn live_phase4_both_sources_at_once() {
        let project_path = expand_tilde(&env("ENKIDU_LIVE_PROJECT"));
        let jira_keys: Vec<String> = env("ENKIDU_LIVE_JIRA").split(',').map(|k| k.trim().to_string()).collect();
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("live.db")).unwrap();
        let path = project_path.to_string_lossy().to_string();
        db.insert_project(&Project { id: "p1".into(), name: "live".into(), path: path.clone(), live_workspace_id: None, created_at: 0 }).unwrap();
        let repo = github::repo_slug(&project_path).expect("the project's origin is on GitHub");
        let path_env = std::env::var("PATH").unwrap_or_default();
        let sources = crate::work::watch::Sources { gh: Tool::new("gh", &path_env), twg: Tool::new("twg", &path_env) };
        let info = ProjectInfo { id: "p1".into(), path, repo: Some(repo), jira_keys: jira_keys.clone(), ..Default::default() };
        let ws = crate::settings::WorkSettings { jira_keys, ..Default::default() };
        // the watcher's order: GitHub, the links (so Jira hears of the tickets PRs name), Jira, the links again
        let gh = crate::work::watch::github_pass(&db, &sources.gh, &info, &ws, &mut 0, now_ms());
        relink(&db, std::slice::from_ref(&info), now_ms(), &|_| None).unwrap();
        let jira = crate::work::watch::jira_pass(&db, &sources.twg, &info, &ws, &mut None, &mut 0, now_ms());
        relink(&db, std::slice::from_ref(&info), now_ms(), &|_| None).unwrap();
        assert_eq!((gh.state.as_str(), jira.state.as_str()), ("ok", "ok"), "{gh:?} {jira:?}");
        let gh_report = super::check_github(&db, &sources.gh, &info, gh.account.as_deref().unwrap());
        let jira_report = super::check_jira(&db, &sources.twg, &info, &ws, now_ms());
        for r in [&gh_report, &jira_report] {
            for d in &r.differences {
                println!("  DIFFERENT {} {} {}: index {:?}, direct {:?}", r.source, d.thing, d.field, d.index, d.direct);
            }
            assert!(r.ok(), "{}: {:?} {:?}", r.source, r.error, r.differences);
        }
        // what the page gets: every open PR and every ticket of the read, the health of both, the items
        let snap = crate::work::snapshot_of(&db).unwrap();
        let open: Vec<i64> = snap.prs.iter().filter(|p| p.state == "OPEN").map(|p| p.number).collect();
        let open_direct = db.work_prs().unwrap().into_iter().filter(|(p, _)| p.state == "OPEN").count();
        assert_eq!(open.len(), open_direct);
        assert_eq!(snap.tickets.len(), db.work_project_tickets("p1").unwrap().len());
        assert!(snap.health.iter().all(|h| h.state == "ok") && snap.health.len() == 2);
        println!(
            "both sources at once: GitHub as {} ({} open PRs), Jira as {} ({} tickets), {} items waiting, {} links; {} and {}",
            gh.account.unwrap_or_default(),
            open.len(),
            jira.account.unwrap_or_default(),
            snap.tickets.len(),
            snap.items.iter().filter(|i| i.state == "open").count(),
            snap.links.len(),
            gh_report.compared.len(),
            jira_report.compared.len()
        );
    }

    /// `YYYY-MM-DD` of `days` ago (UTC), for GitHub's `merged:>=` search.
    pub fn chrono_days_ago(days: i64) -> String {
        let ms = now_ms() - days * 86_400_000;
        let z = ms.div_euclid(86_400_000) + 719468;
        let era = z.div_euclid(146097);
        let doe = z - era * 146097;
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        format!("{:04}-{:02}-{:02}", if m <= 2 { y + 1 } else { y }, m, d)
    }

    #[test]
    fn dates_for_github_searches() {
        let today = chrono_days_ago(0);
        assert_eq!(today.len(), 10);
        assert!(today.starts_with("20"));
        assert_eq!(&today[4..5], "-");
    }

    /// The Jira board you pick (a scrum board): the tickets the Work page keeps for it are the ones Jira puts in the
    /// board's running sprint, asked another way (the sprint by its id, where the page's read asks for open sprints in
    /// the board's columns). `ENKIDU_LIVE_BOARD` = the board's id (5, "Shop Dev").
    #[test]
    #[ignore = "live: reads Jira"]
    fn live_phase7_picked_board_matches_its_sprint() {
        let project_path = expand_tilde(&env("ENKIDU_LIVE_PROJECT"));
        let jira_keys: Vec<String> = env("ENKIDU_LIVE_JIRA").split(',').map(|k| k.trim().to_string()).collect();
        let board_id = std::env::var("ENKIDU_LIVE_BOARD").unwrap_or_else(|_| "5".into());
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("live.db")).unwrap();
        let path = project_path.to_string_lossy().to_string();
        db.insert_project(&Project { id: "p1".into(), name: "live".into(), path: path.clone(), live_workspace_id: None, created_at: 0 }).unwrap();
        let path_env = std::env::var("PATH").unwrap_or_default();
        let twg = Tool::new("twg", &path_env);
        let info = ProjectInfo { id: "p1".into(), path, repo: None, jira_keys: jira_keys.clone(), ..Default::default() };
        let ws = crate::settings::WorkSettings { jira_keys: jira_keys.clone(), ..Default::default() };
        let health = crate::work::watch::jira_pass(&db, &twg, &info, &ws, &mut None, &mut 0, now_ms());
        assert_eq!(health.state, "ok", "{health:?}");
        let keys: Vec<String> = crate::work::snapshot_of(&db).unwrap().tickets.into_iter().map(|t| t.key).collect();
        let boards = crate::work::jira::boards(&twg, "", &jira_keys[0]).unwrap();
        println!("{} boards: {}", boards.len(), boards.iter().map(|b| format!("{} {} ({})", b.id, b.name, b.kind)).collect::<Vec<_>>().join(", "));
        let board = boards.into_iter().find(|b| b.id == board_id).expect("the board is one of the project's");
        assert_eq!(board.kind, "scrum", "this check compares a scrum board's running sprint");
        let on: std::collections::BTreeSet<String> = crate::work::watch::board_keys(&twg, &board, &keys).unwrap().expect("the board exists").into_iter().collect();

        // directly: the board's running sprint by its id, then which of the same tickets are in it
        let out = twg
            .run(&["jira", "board", "sprints", "query", "--id", &board_id, "--state", "active", "--output", "json", "--output-summary", "none"], &[], std::time::Duration::from_secs(60))
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
        let sprints: Vec<String> = v["data"]["sprints"].as_array().unwrap().iter().map(|s| s["id"].to_string()).collect();
        let mut direct = std::collections::BTreeSet::new();
        if !sprints.is_empty() {
            for chunk in keys.chunks(60) {
                let jql = format!("sprint in ({}) AND key in ({})", sprints.join(", "), chunk.join(", "));
                for t in crate::work::jira::query(&twg, "", &jql).unwrap() {
                    if chunk.contains(&t.key) {
                        direct.insert(t.key);
                    }
                }
            }
        }
        println!("{}: {} of your {} tickets on it ({}); sprint {:?} has {}", board.name, on.len(), keys.len(), on.iter().cloned().collect::<Vec<_>>().join(" "), sprints, direct.len());
        assert_eq!(on, direct, "the board's tickets differ from its running sprint's");
    }
}
