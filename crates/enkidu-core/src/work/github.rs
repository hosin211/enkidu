//! GitHub, through `gh` and your own login: your open PRs (with their review threads, comments, reviews and the
//! latest commit's checks) and the ones merged lately. Read-only, one query per page, and all or nothing: a page
//! that fails fails the whole read.

use anyhow::{anyhow, bail, Result};
use serde_json::Value;
use std::path::Path;
use std::time::Duration;

use super::json::{array, boolean, field, int, opt_int, opt_string, parse, string};
use super::model::{Check, CheckLine, CommentLine, CommitLine, Pr, PrDetails, ReviewLine, ThreadLine};
use super::run::{Failure, Tool};

const SRC: &str = "GitHub";
const LIMIT: Duration = Duration::from_secs(40);

pub const OPEN_QUERY: &str = include_str!("queries/open_prs.graphql");
pub const MERGED_QUERY: &str = include_str!("queries/merged_prs.graphql");
pub const THREADS_QUERY: &str = include_str!("queries/threads_page.graphql");
pub const THREAD_COMMENTS_QUERY: &str = include_str!("queries/thread_comments_page.graphql");
pub const CHECKS_QUERY: &str = include_str!("queries/checks_page.graphql");

/// A comment on a PR (in a review thread or in its conversation).
#[derive(Clone, Debug, PartialEq)]
pub struct Comment {
    pub id: String,
    /// The login, or none when the account was deleted
    pub author: Option<String>,
    pub bot: bool,
    pub body: String,
    pub at: String,
    pub url: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Thread {
    pub id: String,
    pub resolved: bool,
    /// The code it was about has changed since
    pub outdated: bool,
    pub path: Option<String>,
    pub line: Option<i64>,
    pub comments: Vec<Comment>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Review {
    pub id: String,
    pub state: String,
    pub author: Option<String>,
    pub bot: bool,
    pub at: Option<String>,
}

/// An open PR with everything the rules look at.
#[derive(Clone, Debug, PartialEq)]
pub struct OpenPr {
    pub pr: Pr,
    pub threads: Vec<Thread>,
    pub comments: Vec<Comment>,
    pub reviews: Vec<Review>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Read {
    /// Your login, as GitHub sees the account Enkidu read with
    pub me: String,
    pub open: Vec<OpenPr>,
    pub merged: Vec<Pr>,
}

/// The repo's `owner/name` from the project's `origin`, if it is on GitHub. The URL itself is never logged or kept:
/// a remote can carry a token.
pub fn repo_slug(project: &Path) -> Option<String> {
    let out = crate::git::git(project, &["config", "--get", "remote.origin.url"]).ok()?;
    slug_from_url(out.stdout.trim())
}

pub fn slug_from_url(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("git@github.com:")
        .or_else(|| url.strip_prefix("ssh://git@github.com/"))
        .or_else(|| {
            // https://[user[:token]@]github.com/owner/name(.git)
            let after_scheme = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"))?;
            let host_and_path = after_scheme.rsplit_once('@').map(|(_, h)| h).unwrap_or(after_scheme);
            host_and_path.strip_prefix("github.com/")
        })?;
    let rest = rest.trim_end_matches('/').trim_end_matches(".git");
    let mut parts = rest.split('/');
    let (owner, name) = (parts.next()?, parts.next()?);
    (!owner.is_empty() && !name.is_empty() && parts.next().is_none()).then(|| format!("{owner}/{name}"))
}

/// The logins `gh` is signed in with on github.com.
pub fn accounts(gh: &Tool) -> Result<Vec<String>> {
    let out = gh.run(&["auth", "status", "--hostname", "github.com"], &[], Duration::from_secs(15))?;
    let text = format!("{}\n{}", out.stdout, out.stderr);
    let found: Vec<String> = text
        .lines()
        .filter_map(|l| l.split("Logged in to github.com account ").nth(1))
        .filter_map(|rest| rest.split_whitespace().next())
        .map(String::from)
        .collect();
    if found.is_empty() {
        bail!(Failure::new("GitHub: gh isn't signed in", "Run `gh auth login`."));
    }
    Ok(found)
}

/// The token of one of `gh`'s accounts, for this process only (never written or printed).
pub fn token(gh: &Tool, account: &str) -> Result<String> {
    let out = gh.run(&["auth", "token", "--hostname", "github.com", "--user", account], &[], Duration::from_secs(15))?;
    let t = out.stdout.trim().to_string();
    if out.code != 0 || t.is_empty() {
        bail!(Failure::new(&format!("GitHub: no token for {account}"), &format!("Run `gh auth login` for {account}.")));
    }
    Ok(t)
}

/// Can this token read the repo? No (GitHub says it doesn't exist for this account) is an answer; a refused login,
/// no connection or a used-up rate limit is a failure.
pub fn can_read(gh: &Tool, token: &str, account: &str, repo: &str) -> Result<bool> {
    let out = gh
        .run(&["api", &format!("repos/{repo}"), "--jq", ".full_name"], &[("GH_TOKEN", token)], Duration::from_secs(20))
        .map_err(|e| anyhow!(Failure::new(&format!("GitHub: {e}"), "Check the internet connection; Enkidu tries again.")))?;
    if out.code == 0 {
        return Ok(out.stdout.trim().eq_ignore_ascii_case(repo));
    }
    let e = out.stderr.to_lowercase();
    if e.contains("http 404") || e.contains("not found") {
        return Ok(false);
    }
    Err(anyhow!(explain(out.code, &out.stderr, account)))
}

/// The account to read a repo with: the one tried last if it still works, else the first that can read it.
pub fn pick_account(gh: &Tool, repo: &str, last: Option<&str>) -> Result<(String, String)> {
    let all = accounts(gh)?;
    let mut order: Vec<&String> = all.iter().filter(|a| Some(a.as_str()) == last).collect();
    order.extend(all.iter().filter(|a| Some(a.as_str()) != last));
    let mut first_error = None;
    for account in order {
        match token(gh, account).and_then(|t| can_read(gh, &t, account, repo).map(|ok| (ok, t))) {
            Ok((true, t)) => return Ok((account.clone(), t)),
            Ok((false, _)) => {}
            Err(e) => {
                first_error.get_or_insert(e);
            }
        }
    }
    if let Some(e) = first_error {
        return Err(e);
    }
    bail!(Failure::new(
        &format!("GitHub: none of your accounts ({}) can read {repo}", all.join(", ")),
        "Sign in with an account that has access: `gh auth login`.",
    ))
}

/// What went wrong, in words, from `gh`'s exit and message.
pub fn explain(code: i32, stderr: &str, account: &str) -> Failure {
    let e = stderr.to_lowercase();
    if e.contains("bad credentials") || e.contains("http 401") {
        Failure::new(&format!("GitHub refused the login of {account}"), &format!("Run `gh auth refresh` (or `gh auth login`) for {account}."))
    } else if e.contains("gh auth login") || e.contains("not logged") {
        Failure::new("GitHub: gh isn't signed in", "Run `gh auth login`.")
    } else if e.contains("rate limit") {
        Failure::new("GitHub's rate limit is used up for now", "It resets within the hour; Enkidu tries again.")
    } else if e.contains("http 404") || e.contains("could not resolve to") {
        Failure::new(&format!("GitHub can't find the repository for {account}"), "Check the project's remote and your access.")
    } else if let Some(rest) = stderr.trim_start().strip_prefix("GraphQL:") {
        let msg = crate::redact::redact_urls(rest.lines().next().unwrap_or("").trim());
        Failure::new(&format!("GitHub answered with an error: {msg}"), "Enkidu tries again; GitHub sometimes fails for a moment.")
    } else if ["error connecting", "check your internet connection", "could not resolve host", "dial tcp", "connection refused", "network is unreachable", "i/o timeout", "no such host", "tls handshake"]
        .iter()
        .any(|n| e.contains(n))
    {
        Failure::new("No connection to GitHub", "Check the internet connection; Enkidu tries again.")
    } else {
        let first = crate::redact::redact_urls(stderr.lines().find(|l| !l.trim().is_empty()).unwrap_or("no message").trim());
        Failure::new(&format!("gh failed ({code}): {first}"), "Try again; if it keeps failing, run the same `gh` command in a terminal.")
    }
}

/// One GraphQL request. GraphQL errors inside an answer count as a failure too.
fn graphql(gh: &Tool, token: &str, account: &str, query: &str, vars: &[(&str, &str)]) -> Result<Value> {
    let q = format!("query={query}");
    let mut args = vec!["api", "graphql", "-f", q.as_str()];
    let pairs: Vec<String> = vars.iter().map(|(k, v)| format!("{k}={v}")).collect();
    for p in &pairs {
        args.push("-f");
        args.push(p);
    }
    let out = gh.run(&args, &[("GH_TOKEN", token)], LIMIT).map_err(|e| anyhow!(Failure::new(&format!("GitHub: {e}"), "Check the internet connection; Enkidu tries again.")))?;
    if out.code != 0 {
        return Err(anyhow!(explain(out.code, &out.stderr, account)));
    }
    let v = parse(&out.stdout, SRC)?;
    if let Some(errors) = v.get("errors").and_then(|e| e.as_array()).filter(|e| !e.is_empty()) {
        let msg = errors[0].get("message").and_then(|m| m.as_str()).unwrap_or("an error");
        bail!(Failure::new(&format!("GitHub answered with an error: {msg}"), "Try again; Enkidu tries again too."));
    }
    Ok(v)
}

fn comment(v: &Value) -> Result<Comment> {
    let author = field(v, "author", SRC)?;
    Ok(Comment {
        id: string(v, "id", SRC)?,
        author: if author.is_null() { None } else { Some(string(author, "login", SRC)?) },
        bot: !author.is_null() && string(author, "__typename", SRC)? == "Bot",
        body: string(v, "body", SRC)?,
        at: string(v, "createdAt", SRC)?,
        url: string(v, "url", SRC)?,
    })
}

/// A page cursor when the list goes on.
fn next_page(conn: &Value) -> Result<Option<String>> {
    if boolean(conn, "pageInfo.hasNextPage", SRC)? {
        Ok(Some(string(conn, "pageInfo.endCursor", SRC)?))
    } else {
        Ok(None)
    }
}

/// A thread, and the cursor of its further comments if it has more than one page.
fn thread(v: &Value) -> Result<(Thread, Option<String>)> {
    let conn = field(v, "comments", SRC)?;
    let comments = array(conn, "nodes", SRC)?.iter().map(comment).collect::<Result<_>>()?;
    Ok((
        Thread {
            id: string(v, "id", SRC)?,
            resolved: boolean(v, "isResolved", SRC)?,
            outdated: boolean(v, "isOutdated", SRC)?,
            path: opt_string(v, "path", SRC)?,
            line: opt_int(v, "line", SRC)?.or(opt_int(v, "originalLine", SRC)?),
            comments,
        },
        next_page(conn)?,
    ))
}

const FAILING: [&str; 5] = ["FAILURE", "TIMED_OUT", "CANCELLED", "STARTUP_FAILURE", "ACTION_REQUIRED"];

/// The checks that aren't green, and how many are still running.
fn contexts(nodes: &[Value]) -> Result<(Vec<Check>, usize)> {
    let mut failing = Vec::new();
    let mut running = 0;
    for n in nodes {
        match string(n, "__typename", SRC)?.as_str() {
            "CheckRun" => {
                let conclusion = opt_string(n, "conclusion", SRC)?;
                let status = string(n, "status", SRC)?;
                if status != "COMPLETED" {
                    running += 1;
                } else if let Some(c) = conclusion.filter(|c| FAILING.contains(&c.as_str())) {
                    failing.push(Check { name: string(n, "name", SRC)?, state: c, url: opt_string(n, "detailsUrl", SRC)? });
                }
            }
            "StatusContext" => {
                let state = string(n, "state", SRC)?;
                if state == "PENDING" || state == "EXPECTED" {
                    running += 1;
                } else if state == "FAILURE" || state == "ERROR" {
                    failing.push(Check { name: string(n, "context", SRC)?, state, url: opt_string(n, "targetUrl", SRC)? });
                }
            }
            other => bail!("GitHub sent a check of an unknown kind: {other}"),
        }
    }
    Ok((failing, running))
}

/// Every check, in gh's buckets (`gh pr checks` says the same: pass, fail, pending, skipping, cancel).
fn check_lines(nodes: &[Value]) -> Result<Vec<CheckLine>> {
    nodes
        .iter()
        .map(|n| {
            Ok(match string(n, "__typename", SRC)?.as_str() {
                "CheckRun" => {
                    let bucket = if string(n, "status", SRC)? != "COMPLETED" {
                        "pending"
                    } else {
                        match opt_string(n, "conclusion", SRC)?.as_deref() {
                            Some("SUCCESS") => "pass",
                            Some("SKIPPED") | Some("NEUTRAL") => "skipping",
                            Some("CANCELLED") => "cancel",
                            _ => "fail",
                        }
                    };
                    CheckLine { name: string(n, "name", SRC)?, bucket: bucket.into(), url: opt_string(n, "detailsUrl", SRC)? }
                }
                _ => {
                    let bucket = match string(n, "state", SRC)?.as_str() {
                        "SUCCESS" => "pass",
                        "PENDING" | "EXPECTED" => "pending",
                        _ => "fail",
                    };
                    CheckLine { name: string(n, "context", SRC)?, bucket: bucket.into(), url: opt_string(n, "targetUrl", SRC)? }
                }
            })
        })
        .collect()
}

/// A text cut to `max` characters: a few words more never matter, a 50 KB description would.
fn cut(text: &str, max: usize) -> String {
    let t = text.trim();
    if t.chars().count() <= max {
        return t.to_string();
    }
    let mut out: String = t.chars().take(max).collect();
    out.push('…');
    out
}

const BODY_MAX: usize = 12_000;
const TEXT_MAX: usize = 1_500;
const COMMENTS_KEPT: usize = 20;
const COMMITS_KEPT: usize = 10;

/// The rest of a PR's details, once all its pages are in: each reviewer's latest review, every thread (the open ones
/// first, then the latest), the conversation.
pub fn fill_details(pr: &mut OpenPr) {
    let mut latest: Vec<&Review> = Vec::new();
    // the author's own replies in threads count as reviews on GitHub, but aren't reviews of the PR: GitHub's
    // "latest reviews" leaves them out too (found by the live check)
    let author = pr.pr.details.author.clone();
    for r in pr.reviews.iter().filter(|r| r.state != "PENDING" && r.author.is_some() && r.author != author) {
        match latest.iter().position(|x| x.author == r.author) {
            Some(i) if latest[i].at <= r.at => latest[i] = r,
            Some(_) => {}
            None => latest.push(r),
        }
    }
    latest.sort_by(|a, b| b.at.cmp(&a.at));
    pr.pr.details.reviews =
        latest.into_iter().map(|r| ReviewLine { who: r.author.clone().unwrap_or_default(), state: r.state.clone(), at: r.at.clone(), bot: r.bot }).collect();
    let mut threads: Vec<ThreadLine> = pr
        .threads
        .iter()
        .map(|t| {
            let first = t.comments.first();
            let last = t.comments.last().filter(|_| t.comments.len() > 1);
            ThreadLine {
                path: t.path.clone(),
                line: t.line,
                resolved: t.resolved,
                outdated: t.outdated,
                who: first.and_then(|c| c.author.clone()),
                bot: first.is_some_and(|c| c.bot),
                text: first.map(|c| cut(&c.body, TEXT_MAX)).unwrap_or_default(),
                replies: t.comments.len().saturating_sub(1),
                last_who: last.and_then(|c| c.author.clone()),
                last_at: last.map(|c| c.at.clone()),
                url: first.map(|c| c.url.clone()),
            }
        })
        .collect();
    let latest_of = |t: &ThreadLine| t.last_at.clone().unwrap_or_default();
    threads.sort_by(|a, b| a.resolved.cmp(&b.resolved).then_with(|| latest_of(b).cmp(&latest_of(a))));
    pr.pr.details.threads = threads;
    let mut comments: Vec<&Comment> = pr.comments.iter().collect();
    comments.sort_by(|a, b| a.at.cmp(&b.at));
    let skip = comments.len().saturating_sub(COMMENTS_KEPT);
    pr.pr.details.comments = comments
        .into_iter()
        .skip(skip)
        .map(|c| CommentLine { who: c.author.clone(), bot: c.bot, text: cut(&c.body, TEXT_MAX), at: c.at.clone(), url: c.url.clone() })
        .collect();
}

/// Where to continue a PR that has more than one page of something.
#[derive(Default, Debug)]
struct More {
    threads: Option<String>,
    thread_comments: Vec<(usize, String)>,
    checks: Option<(String, String)>,
}

fn open_pr(v: &Value, project_id: &str, repo: &str) -> Result<(OpenPr, String, More)> {
    let mut more = More::default();
    let head = array(v, "headCommit.nodes", SRC)?.first().map(|n| field(n, "commit", SRC)).transpose()?;
    let (mut checks, mut failing, mut running, mut head_sha, mut head_at) = (None, Vec::new(), 0, None, None);
    let mut check_list = Vec::new();
    if let Some(c) = head {
        head_sha = Some(string(c, "oid", SRC)?);
        head_at = Some(string(c, "committedDate", SRC)?);
        let rollup = field(c, "statusCheckRollup", SRC)?;
        if !rollup.is_null() {
            checks = Some(string(rollup, "state", SRC)?);
            let conn = field(rollup, "contexts", SRC)?;
            (failing, running) = contexts(array(conn, "nodes", SRC)?)?;
            check_list = check_lines(array(conn, "nodes", SRC)?)?;
            if let Some(after) = next_page(conn)? {
                more.checks = Some((string(c, "id", SRC)?, after));
            }
        }
    }
    let mut threads = Vec::new();
    let threads_conn = field(v, "reviewThreads", SRC)?;
    for (i, t) in array(threads_conn, "nodes", SRC)?.iter().enumerate() {
        let (t, after) = thread(t)?;
        if let Some(after) = after {
            more.thread_comments.push((i, after));
        }
        threads.push(t);
    }
    more.threads = next_page(threads_conn)?;
    let pr = Pr {
        project_id: project_id.into(),
        repo: repo.into(),
        number: int(v, "number", SRC)?,
        title: string(v, "title", SRC)?,
        url: string(v, "url", SRC)?,
        state: string(v, "state", SRC)?,
        draft: boolean(v, "isDraft", SRC)?,
        head: string(v, "headRefName", SRC)?,
        base: string(v, "baseRefName", SRC)?,
        review_decision: opt_string(v, "reviewDecision", SRC)?,
        checks,
        failing,
        running,
        mergeable: opt_string(v, "mergeable", SRC)?,
        head_sha,
        head_at,
        unresolved: 0,
        updated_at: string(v, "updatedAt", SRC)?,
        merged_at: opt_string(v, "mergedAt", SRC)?,
        commits: array(v, "allCommits.nodes", SRC)?.iter().map(|n| string(n, "commit.oid", SRC)).collect::<Result<_>>()?,
        commit_keys: {
            let mut keys: Vec<String> = Vec::new();
            for n in array(v, "allCommits.nodes", SRC)? {
                for k in super::parse::ticket_keys(&string(n, "commit.messageHeadline", SRC)?) {
                    if !keys.contains(&k) {
                        keys.push(k);
                    }
                }
            }
            keys
        },
        last_comment_by: None,
        last_comment_at: None,
        details: Default::default(),
    };
    let comments: Vec<Comment> = array(v, "comments.nodes", SRC)?.iter().map(comment).collect::<Result<_>>()?;
    let mut pr = pr;
    let author = field(v, "author", SRC)?;
    let requested = array(v, "reviewRequests.nodes", SRC)?
        .iter()
        .filter_map(|r| r.get("requestedReviewer").filter(|x| !x.is_null()))
        .map(|x| x.get("login").or_else(|| x.get("name")).and_then(Value::as_str).map(str::to_string).ok_or_else(|| anyhow!("GitHub sent a reviewer without a name")))
        .collect::<Result<_>>()?;
    pr.details = PrDetails {
        author: if author.is_null() { None } else { Some(string(author, "login", SRC)?) },
        created_at: Some(string(v, "createdAt", SRC)?),
        body: cut(&string(v, "body", SRC)?, BODY_MAX),
        additions: Some(int(v, "additions", SRC)?),
        deletions: Some(int(v, "deletions", SRC)?),
        changed_files: Some(int(v, "changedFiles", SRC)?),
        labels: array(v, "labels.nodes", SRC)?.iter().map(|l| string(l, "name", SRC)).collect::<Result<_>>()?,
        requested,
        checks: check_list,
        commit_count: int(v, "allCommits.totalCount", SRC)? as usize,
        recent_commits: array(v, "allCommits.nodes", SRC)?
            .iter()
            .rev()
            .take(COMMITS_KEPT)
            .map(|n| Ok(CommitLine { oid: string(n, "commit.oid", SRC)?.chars().take(7).collect(), headline: string(n, "commit.messageHeadline", SRC)? }))
            .collect::<Result<_>>()?,
        ..Default::default()
    };
    if let Some(last) = comments.iter().max_by(|a, b| a.at.cmp(&b.at)) {
        pr.last_comment_by = last.author.clone();
        pr.last_comment_at = Some(last.at.clone());
    }
    let reviews = array(v, "reviews.nodes", SRC)?
        .iter()
        .map(|r| {
            let author = field(r, "author", SRC)?;
            Ok(Review {
                id: string(r, "id", SRC)?,
                state: string(r, "state", SRC)?,
                author: if author.is_null() { None } else { Some(string(author, "login", SRC)?) },
                bot: !author.is_null() && string(author, "__typename", SRC)? == "Bot",
                at: opt_string(r, "submittedAt", SRC)?,
            })
        })
        .collect::<Result<_>>()?;
    Ok((OpenPr { pr, threads, comments, reviews }, string(v, "id", SRC)?, more))
}

/// One page of the open-PR search: your login, the PRs, and the cursor of the next page (the tests read the
/// recorded fixtures through it; `read` does the same page by page).
#[cfg(test)]
pub fn parse_open_page(v: &Value, project_id: &str, repo: &str) -> Result<(String, Vec<(OpenPr, String)>, Option<String>)> {
    let me = string(v, "data.viewer.login", SRC)?;
    let search = field(v, "data.search", SRC)?;
    let mut prs = Vec::new();
    for n in array(search, "nodes", SRC)? {
        let (mut pr, id, _) = open_pr(n, project_id, repo)?;
        fill_details(&mut pr);
        prs.push((pr, id));
    }
    Ok((me, prs, next_page(search)?))
}

pub fn parse_merged_page(v: &Value, project_id: &str, repo: &str) -> Result<(Vec<Pr>, Option<String>)> {
    let search = field(v, "data.search", SRC)?;
    let prs = array(search, "nodes", SRC)?
        .iter()
        .map(|n| {
            Ok(Pr {
                project_id: project_id.into(),
                repo: repo.into(),
                number: int(n, "number", SRC)?,
                title: string(n, "title", SRC)?,
                url: string(n, "url", SRC)?,
                state: string(n, "state", SRC)?,
                draft: false,
                head: string(n, "headRefName", SRC)?,
                base: string(n, "baseRefName", SRC)?,
                review_decision: None,
                checks: None,
                failing: Vec::new(),
                running: 0,
                mergeable: None,
                head_sha: None,
                head_at: None,
                unresolved: 0,
                updated_at: string(n, "updatedAt", SRC)?,
                merged_at: opt_string(n, "mergedAt", SRC)?,
                commits: Vec::new(),
                commit_keys: Vec::new(),
                last_comment_by: None,
                last_comment_at: None,
                details: Default::default(),
            })
        })
        .collect::<Result<_>>()?;
    Ok((prs, next_page(search)?))
}

/// Reads everything for one repo: the open PRs (every page of them, their threads, thread comments and checks)
/// and the ones merged in the last `days`. Any failed page fails the read.
pub fn read(gh: &Tool, token: &str, account: &str, repo: &str, project_id: &str, merged_since: &str) -> Result<Read> {
    let q = format!("is:pr is:open author:@me repo:{repo}");
    let mut me;
    let mut open = Vec::new();
    let mut after: Option<String> = None;
    loop {
        let mut vars = vec![("q", q.as_str())];
        if let Some(a) = &after {
            vars.push(("after", a.as_str()));
        }
        let page = graphql(gh, token, account, OPEN_QUERY, &vars)?;
        let search = field(&page, "data.search", SRC)?;
        me = string(&page, "data.viewer.login", SRC)?;
        for n in array(search, "nodes", SRC)? {
            let (mut pr, id, more) = open_pr(n, project_id, repo)?;
            complete(gh, token, account, &mut pr, &id, more)?;
            pr.pr.unresolved = pr.threads.iter().filter(|t| !t.resolved).count();
            fill_details(&mut pr);
            open.push(pr);
        }
        after = next_page(search)?;
        if after.is_none() {
            break;
        }
    }
    let mq = format!("is:pr is:merged author:@me repo:{repo} merged:>={merged_since}");
    let mut merged = Vec::new();
    let mut after: Option<String> = None;
    loop {
        let mut vars = vec![("q", mq.as_str())];
        if let Some(a) = &after {
            vars.push(("after", a.as_str()));
        }
        let page = graphql(gh, token, account, MERGED_QUERY, &vars)?;
        let (mut prs, next) = parse_merged_page(&page, project_id, repo)?;
        merged.append(&mut prs);
        after = next;
        if after.is_none() {
            break;
        }
    }
    Ok(Read { me, open, merged })
}

/// Fetches the rest of a PR whose threads, thread comments or checks run over one page.
fn complete(gh: &Tool, token: &str, account: &str, pr: &mut OpenPr, id: &str, more: More) -> Result<()> {
    for (i, mut after) in more.thread_comments {
        loop {
            let page = graphql(gh, token, account, THREAD_COMMENTS_QUERY, &[("id", &pr.threads[i].id.clone()), ("after", &after)])?;
            let conn = field(&page, "data.node.comments", SRC)?;
            for c in array(conn, "nodes", SRC)? {
                pr.threads[i].comments.push(comment(c)?);
            }
            match next_page(conn)? {
                Some(a) => after = a,
                None => break,
            }
        }
    }
    if let Some(mut after) = more.threads {
        loop {
            let page = graphql(gh, token, account, THREADS_QUERY, &[("id", id), ("after", &after)])?;
            let conn = field(&page, "data.node.reviewThreads", SRC)?;
            for t in array(conn, "nodes", SRC)? {
                let (t, more_comments) = thread(t)?;
                let i = pr.threads.len();
                pr.threads.push(t);
                if let Some(mut a) = more_comments {
                    loop {
                        let page = graphql(gh, token, account, THREAD_COMMENTS_QUERY, &[("id", &pr.threads[i].id.clone()), ("after", &a)])?;
                        let conn = field(&page, "data.node.comments", SRC)?;
                        for c in array(conn, "nodes", SRC)? {
                            pr.threads[i].comments.push(comment(c)?);
                        }
                        match next_page(conn)? {
                            Some(next) => a = next,
                            None => break,
                        }
                    }
                }
            }
            match next_page(conn)? {
                Some(a) => after = a,
                None => break,
            }
        }
    }
    if let Some((commit, mut after)) = more.checks {
        loop {
            let page = graphql(gh, token, account, CHECKS_QUERY, &[("id", &commit), ("after", &after)])?;
            let conn = field(&page, "data.node.statusCheckRollup.contexts", SRC)?;
            let (mut failing, running) = contexts(array(conn, "nodes", SRC)?)?;
            pr.pr.details.checks.extend(check_lines(array(conn, "nodes", SRC)?)?);
            pr.pr.failing.append(&mut failing);
            pr.pr.running += running;
            match next_page(conn)? {
                Some(a) => after = a,
                None => break,
            }
        }
    }
    Ok(())
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub fn fixture(name: &str) -> Value {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/work").join(name);
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap()
    }

    #[test]
    fn reads_the_slug_and_never_keeps_credentials() {
        assert_eq!(slug_from_url("git@github.com:Org/repo.git").as_deref(), Some("Org/repo"));
        assert_eq!(slug_from_url("https://github.com/Org/repo").as_deref(), Some("Org/repo"));
        assert_eq!(slug_from_url("https://x-access-token:ghp_secret@github.com/Org/repo.git").as_deref(), Some("Org/repo"));
        assert_eq!(slug_from_url("ssh://git@github.com/Org/repo.git").as_deref(), Some("Org/repo"));
        assert_eq!(slug_from_url("https://gitlab.com/Org/repo.git"), None);
        assert_eq!(slug_from_url("https://github.com/Org"), None);
    }

    #[test]
    fn keeps_what_the_card_shows_about_each_pr() {
        let (_, prs, _) = parse_open_page(&fixture("gh-open-prs.json"), "p1", "example-org/shop").unwrap();
        let details = |n: i64| prs.iter().find(|(p, _)| p.pr.number == n).unwrap().0.pr.details.clone();
        let d = details(2350);
        assert_eq!(d.author.as_deref(), Some("me-dev"));
        assert_eq!((d.additions, d.deletions, d.changed_files), (Some(1069), Some(33), Some(28)));
        assert!(d.body.starts_with("Text "), "the description, as recorded (anonymized)");
        assert_eq!(d.requested, ["reviewer-1"]);
        assert_eq!((d.commit_count, d.recent_commits.len()), (2, 2));
        assert!(d.recent_commits.iter().all(|c| c.oid.len() == 7 && !c.headline.is_empty()));
        // each reviewer's latest review, people and bots alike, the latest first; not the author's own
        let reviews: Vec<String> = d.reviews.iter().map(|r| format!("{}: {}", r.who, r.state)).collect();
        assert_eq!(reviews.len(), 3, "{reviews:?}");
        assert!(!reviews.iter().any(|r| r.starts_with("me-dev")), "your own replies aren't reviews of your PR: {reviews:?}");
        assert!(reviews.contains(&"person-1: CHANGES_REQUESTED".to_string()), "{reviews:?}");
        assert!(d.reviews.windows(2).all(|w| w[0].at >= w[1].at));
        // every thread, the open ones first, each with who started it and what they said
        assert_eq!(d.threads.len(), 5);
        assert!(d.threads[..4].iter().all(|t| !t.resolved) && d.threads[4].resolved);
        assert!(d.threads.iter().all(|t| t.who.is_some() && !t.text.is_empty() && t.url.is_some()));
        assert_eq!(d.comments.len(), 2);
        // 2159: twelve threads, all resolved; thirteen commits, the latest ten kept; four asked to review
        let d = details(2159);
        assert_eq!((d.threads.len(), d.threads.iter().filter(|t| !t.resolved).count()), (12, 0));
        assert_eq!((d.commit_count, d.recent_commits.len()), (13, 10));
        assert_eq!(d.requested.len(), 4);
        assert_eq!(d.labels, Vec::<String>::new());
        // 2265: every check on the latest commit, in gh's buckets
        let d = details(2265);
        assert_eq!(d.checks.len(), 8);
        assert!(d.checks.iter().all(|c| ["pass", "fail", "pending", "skipping", "cancel"].contains(&c.bucket.as_str())), "{:?}", d.checks);
        assert_eq!(details(2329).labels, ["bug"]);
    }

    #[test]
    fn a_long_description_is_cut() {
        let long = "x".repeat(BODY_MAX + 500);
        let kept = cut(&long, BODY_MAX);
        assert_eq!(kept.chars().count(), BODY_MAX + 1);
        assert!(kept.ends_with('…'));
        assert_eq!(cut("  short  ", BODY_MAX), "short");
    }

    #[test]
    fn parses_the_real_open_prs() {
        let (me, prs, next) = parse_open_page(&fixture("gh-open-prs.json"), "p1", "example-org/shop").unwrap();
        assert_eq!(me, "me-dev");
        assert_eq!(next, None);
        assert_eq!(prs.len(), 6);
        let failing = prs.iter().find(|(p, _)| p.pr.checks.as_deref() == Some("FAILURE")).unwrap();
        // the cancelled SonarCloud run is what turns it red, as on GitHub
        assert_eq!(failing.0.pr.failing.len(), 1);
        assert_eq!(failing.0.pr.failing[0].state, "CANCELLED");
        let first = &prs[0].0;
        assert_eq!(first.pr.head, "feature/SH-21349-change-719");
        assert_eq!(first.pr.review_decision.as_deref(), Some("CHANGES_REQUESTED"));
        assert!(first.threads.iter().any(|t| t.comments.iter().any(|c| c.bot)), "the bot's thread is marked");
        assert!(prs.iter().all(|(p, id)| !id.is_empty() && p.pr.head_sha.is_some() && !p.pr.commits.is_empty()));
    }

    #[test]
    fn parses_the_real_merged_prs() {
        let (prs, next) = parse_merged_page(&fixture("gh-merged-prs.json"), "p1", "example-org/shop").unwrap();
        assert_eq!(next, None);
        assert_eq!(prs.len(), 8);
        assert!(prs.iter().all(|p| p.state == "MERGED" && p.merged_at.is_some()));
        // one ticket, five PRs (four to development, one fix to the release branch)
        assert_eq!(prs.iter().filter(|p| p.head.contains("SH-20698")).count(), 5);
        assert_eq!(prs.iter().filter(|p| p.base.starts_with("release/")).count(), 1);
    }

    #[test]
    fn a_missing_field_fails_the_whole_page() {
        let mut v = fixture("gh-open-prs.json");
        v["data"]["search"]["nodes"][2].as_object_mut().unwrap().remove("reviewDecision");
        let err = parse_open_page(&v, "p1", "r").unwrap_err().to_string();
        assert!(err.contains("reviewDecision"), "{err}");
        let mut v = fixture("gh-open-prs.json");
        v["data"]["search"]["nodes"][0]["reviewThreads"]["nodes"][0]["comments"]["nodes"][0]["author"] = Value::String("x".into());
        assert!(parse_open_page(&v, "p1", "r").is_err());
    }

    #[test]
    fn says_what_to_do_about_each_failure() {
        let bad = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/work/gh-bad-credentials.stderr")).unwrap();
        assert!(explain(1, &bad, "me").fix.contains("gh auth refresh"));
        assert!(explain(4, "To get started with GitHub CLI, please run:  gh auth login", "me").fix.contains("gh auth login"));
        let logged_out = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/work/gh-logged-out.stderr")).unwrap();
        assert_eq!(explain(4, &logged_out, "me").message, "GitHub: gh isn't signed in", "the real logged-out answer");
        assert!(explain(1, "error connecting to api.github.com\ncheck your internet connection or https://githubstatus.com", "me").message.contains("No connection"));
        let internal = "GraphQL: Something went wrong while executing your query. This may be the result of a timeout, or it could be a GitHub bug. (search)";
        assert!(explain(1, internal, "me").message.starts_with("GitHub answered with an error"), "not taken for a network problem");
        assert!(!explain(2, "boom at https://x-access-token:t@github.com/o/r", "me").message.contains("github.com"), "URLs are never shown");
        assert!(explain(1, "API rate limit exceeded for user", "me").message.contains("rate limit"));
        assert!(explain(1, "error connecting to api.github.com: dial tcp: lookup api.github.com: no such host", "me").message.contains("No connection"));
        assert!(explain(1, "GraphQL: Could not resolve to a Repository with the name 'x'", "me").message.contains("can't find"));
        assert!(explain(2, "something new\nmore", "me").message.contains("something new"));
    }
}
