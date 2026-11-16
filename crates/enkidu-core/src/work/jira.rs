//! Jira, through `twg` with your own login: the tickets of your work (the ones your PRs name, and the open ones
//! assigned to you), with their comments and attachments, and Jira's boards (which of those tickets are on the one
//! you picked). Read-only: only twg's read commands (`whoami`, `jira workitem query`, `jira workitem get`, `jira
//! board query`, `jira board scope query`) are ever run. A read is all or nothing, like GitHub's: a missing field or
//! a failed item fails it. A ticket Jira says doesn't exist (deleted, or no longer visible to you) is an answer, not
//! a failure.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;

use super::json::{array, boolean, field, opt_string, parse, string};
use super::model::{Board, Ticket};
use super::run::{Failure, Tool};

const SRC: &str = "Jira";
const LIMIT: Duration = Duration::from_secs(90);
/// Tickets per `twg jira workitem get`
pub const BATCH: usize = 25;
/// Tickets per page of a query
const PAGE: &str = "50";

#[derive(Clone, Debug, PartialEq)]
pub struct Person {
    pub id: String,
    pub name: String,
    /// An app (automation, an integration), not a person
    pub bot: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Comment {
    pub id: String,
    pub author: Option<Person>,
    /// The comment as plain text (@mentions as "@Name")
    pub text: String,
    /// Account ids of the people it @mentions
    pub mentions: Vec<String>,
    pub created: String,
    pub updated: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Attachment {
    pub id: String,
    pub filename: String,
    pub mime: String,
    pub author: Option<Person>,
    pub created: String,
    pub size: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FullTicket {
    pub ticket: Ticket,
    pub comments: Vec<Comment>,
    pub attachments: Vec<Attachment>,
}

/// What a batch read returned: the tickets, the keys Jira says don't exist (or you can't see), and keys that moved
/// (`SH-1` is now `XY-5`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Got {
    pub tickets: Vec<FullTicket>,
    pub gone: Vec<String>,
    pub moved: Vec<(String, String)>,
}

/// What went wrong, in words, from twg's answer (it answers with `{ok: false, error: {…}}` on failures).
pub fn explain(code: i32, answer: &Value, stderr: &str) -> Failure {
    let err = answer.get("error").cloned().unwrap_or(Value::Null);
    let text = |k: &str| err.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let (kind, message, status) = (text("code"), text("message"), err.get("statusCode").and_then(Value::as_i64).unwrap_or(0));
    let lower = format!("{message} {stderr}").to_lowercase();
    if kind == "AUTH_REQUIRED" || status == 401 || code == 77 || lower.contains("not signed in") || lower.contains("no valid credentials") {
        let command = err.pointer("/remediation/command").and_then(Value::as_str).unwrap_or("twg login");
        Failure::new("Jira: twg isn't signed in", &format!("Run `{command}`."))
    } else if lower.contains("could not resolve site") {
        Failure::new("Jira: twg can't find the Jira site", "Set the site for twg (`twg config`, or TWG_SITE), then check again.")
    } else if status == 429 || lower.contains("rate limit") || lower.contains("too many requests") {
        Failure::new("Jira's rate limit is used up for now", "Enkidu tries again in a few minutes.")
    } else if status == 403 {
        Failure::new("Jira refused the read", "Check that your Atlassian account can see these tickets.")
    } else if ["enotfound", "econnrefused", "etimedout", "econnreset", "fetch failed", "network", "getaddrinfo"].iter().any(|n| lower.contains(n)) {
        Failure::new("No connection to Jira", "Check the internet connection; Enkidu tries again.")
    } else {
        let first = crate::redact::redact_urls(message.lines().next().unwrap_or("").trim());
        let shown = if first.is_empty() { stderr.lines().find(|l| !l.trim().is_empty()).unwrap_or("no message").trim().to_string() } else { first };
        Failure::new(&format!("twg failed ({code}): {shown}"), "Try again; if it keeps failing, run the same `twg` command in a terminal.")
    }
}

/// Runs twg with JSON output and no summary lines.
fn call(twg: &Tool, site: &str, args: &[&str]) -> Result<(i32, Value)> {
    let (code, v, stderr) = run_json(twg, site, args)?;
    if v.get("ok") == Some(&Value::Bool(false)) {
        return Err(anyhow!(explain(code, &v, &stderr)));
    }
    Ok((code, v))
}

/// Like `call`, for `jira workitem get` with one key: twg answers a ticket that doesn't exist (or that you can't see)
/// with a failure of the whole command (404) instead of a failed item, and that is an answer: None.
fn call_or_gone(twg: &Tool, site: &str, args: &[&str]) -> Result<Option<(i32, Value)>> {
    let (code, v, stderr) = run_json(twg, site, args)?;
    if v.get("ok") == Some(&Value::Bool(false)) {
        if v.pointer("/error/statusCode").and_then(Value::as_i64) == Some(404) {
            return Ok(None);
        }
        return Err(anyhow!(explain(code, &v, &stderr)));
    }
    Ok(Some((code, v)))
}

/// Runs twg and parses its JSON answer, whatever it says.
fn run_json(twg: &Tool, site: &str, args: &[&str]) -> Result<(i32, Value, String)> {
    let mut all: Vec<&str> = args.to_vec();
    if !site.is_empty() {
        all.extend(["--site", site]);
    }
    all.extend(["--output", "json", "--output-summary", "none"]);
    let out = twg.run(&all, &[], LIMIT).map_err(|e| {
        if e.to_string().contains("can't start") {
            anyhow!(Failure::new("Jira: can't start twg", "Install twg (Atlassian's command line) and sign in with `twg login`."))
        } else {
            anyhow!(Failure::new(&format!("Jira: {e}"), "Check the internet connection; Enkidu tries again."))
        }
    })?;
    let v = match parse(&out.stdout, "twg") {
        Ok(v) => v,
        Err(e) if out.code != 0 => return Err(anyhow!(explain(out.code, &Value::Null, &out.stderr)).context(e.to_string())),
        Err(e) => return Err(anyhow!(Failure::new(&format!("Jira: {e}"), "Try again; if it keeps failing, run `twg doctor`."))),
    };
    Ok((out.code, v, out.stderr))
}

/// Who twg reads as: (account id, name).
pub fn whoami(twg: &Tool, site: &str) -> Result<(String, String)> {
    let (_, v) = call(twg, site, &["whoami"])?;
    Ok((string(&v, "data.accountId", SRC)?, string(&v, "data.name", SRC)?))
}

fn person(v: &Value) -> Result<Option<Person>> {
    if v.is_null() {
        return Ok(None);
    }
    Ok(Some(Person {
        id: string(v, "accountId", SRC)?,
        name: string(v, "displayName", SRC)?,
        bot: v.get("accountType").and_then(Value::as_str) == Some("app"),
    }))
}

/// A field's name (`{"name": "Bug", …}`), when the answer has it: the search sends the type as `issueType`, a read
/// of the ticket as `issuetype`.
fn name_of(v: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| v.get(*k)).and_then(|x| x.get("name")).and_then(Value::as_str).map(str::to_string)
}

fn ticket_from(v: &Value, comments: usize, attachments: usize) -> Result<Ticket> {
    let key = string(v, "key", SRC)?;
    let assignee = person(field(v, "assignee", SRC)?)?;
    Ok(Ticket {
        issue_type: name_of(v, &["issueType", "issuetype"]),
        priority: name_of(v, &["priority"]),
        description: None,
        described_at: None,
        project_key: super::parse::key_project(&key).to_string(),
        summary: string(v, "summary", SRC)?,
        url: string(v, "url", SRC)?,
        status: string(v, "status.name", SRC)?,
        category: string(v, "status.statusCategory.key", SRC)?,
        assignee: assignee.as_ref().map(|p| p.name.clone()),
        assignee_id: assignee.map(|p| p.id),
        updated_at: string(v, "updated", SRC)?,
        comments,
        attachments,
        last_comment_by: None,
        last_comment_at: None,
        key,
    })
}

/// One page of a query: its tickets and the cursor of the next page.
pub fn parse_query_page(v: &Value) -> Result<(Vec<Ticket>, Option<String>)> {
    let tickets = array(v, "data.issues", SRC)?.iter().map(|i| ticket_from(i, 0, 0)).collect::<Result<_>>()?;
    let next = match v.get("pageInfo") {
        Some(p) if p.get("hasNextPage").and_then(Value::as_bool) == Some(true) => Some(string(p, "nextCursor", SRC)?),
        _ => None,
    };
    Ok((tickets, next))
}

/// Every ticket a JQL search finds, all pages.
pub fn query(twg: &Tool, site: &str, jql: &str) -> Result<Vec<Ticket>> {
    let mut out = Vec::new();
    let mut after: Option<String> = None;
    loop {
        let mut args = vec!["jira", "workitem", "query", "--jql", jql, "--limit", PAGE];
        if let Some(a) = &after {
            args.extend(["--after", a.as_str()]);
        }
        let (_, v) = call(twg, site, &args)?;
        let (mut page, next) = parse_query_page(&v)?;
        out.append(&mut page);
        match next {
            Some(n) if Some(&n) != after.as_ref() => after = Some(n),
            _ => break,
        }
    }
    Ok(out)
}

/// Boards per page of a board search
const BOARD_PAGE: usize = 50;
/// Keys per search when asking which tickets are on a board (a search's text has a limit)
const BOARD_KEYS: usize = 60;

/// A number Jira sends as a number or as text ("5"), as text.
fn id_text(v: &Value, path: &str) -> Result<String> {
    let f = field(v, path, SRC)?;
    match f {
        Value::String(s) if !s.trim().is_empty() => Ok(s.trim().to_string()),
        Value::Number(n) => Ok(n.to_string()),
        _ => Err(anyhow!("{SRC}'s `{path}` isn't an id")),
    }
}

/// One page of a board search: its boards, and whether it was the last page.
pub fn parse_boards_page(v: &Value) -> Result<(Vec<Board>, bool)> {
    let boards = array(v, "data.boards", SRC)?
        .iter()
        .map(|b| {
            Ok(Board {
                id: id_text(b, "id")?,
                name: string(b, "name", SRC)?,
                kind: string(b, "type", SRC)?,
                // a board in someone's profile has no project
                project_key: b.pointer("/location/projectKey").and_then(Value::as_str).map(String::from),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((boards, boolean(v, "data.isLast", SRC)?))
}

/// The boards of a Jira project, all pages.
pub fn boards(twg: &Tool, site: &str, project_key: &str) -> Result<Vec<Board>> {
    let mut out: Vec<Board> = Vec::new();
    let size = BOARD_PAGE.to_string();
    // never more than 40 pages: an answer that never says "last" ends the read
    for _ in 0..40 {
        // a page may hold fewer boards than asked for: the next one starts after what came
        let start = out.len().to_string();
        let (_, v) = call(twg, site, &["jira", "board", "query", "--project", project_key, "--start-at", &start, "--max-results", &size])?;
        let (found, last) = parse_boards_page(&v)?;
        let none = found.is_empty();
        out.extend(found);
        if last || none {
            return Ok(out);
        }
    }
    Err(anyhow!(Failure::new("Jira's list of boards never ended", "Enkidu tries again.")))
}

/// What is on a board: its kind, the saved search behind it, the statuses its columns show, and (a kanban board) the
/// extra search it adds.
#[derive(Clone, Debug, PartialEq)]
pub struct BoardScope {
    /// scrum or kanban
    pub kind: String,
    /// The id of the board's saved search (its filter)
    pub filter: String,
    /// The ids of the statuses in its columns, in order
    pub statuses: Vec<String>,
    pub sub_query: Option<String>,
}

/// One `twg jira board scope query` answer. On a kanban board the first column is its backlog (always there, named
/// "Backlog", empty unless the board has a backlog page): tickets in it are on the backlog page, not on the board.
pub fn parse_board_scope(v: &Value) -> Result<BoardScope> {
    let kind = string(v, "data.type", SRC)?;
    let filter = id_text(v, "data.filter.id")?;
    let mut statuses: Vec<String> = Vec::new();
    for (n, column) in array(v, "data.columnConfig.columns", SRC)?.iter().enumerate() {
        let name = string(column, "name", SRC)?;
        let ids = array(column, "statuses", SRC)?.iter().map(|s| id_text(s, "id")).collect::<Result<Vec<_>>>()?;
        if kind == "kanban" && n == 0 && name.eq_ignore_ascii_case("backlog") {
            continue;
        }
        for id in ids {
            if !statuses.contains(&id) {
                statuses.push(id);
            }
        }
    }
    let sub_query = v.pointer("/data/subQuery/query").and_then(Value::as_str).map(str::trim).filter(|q| !q.is_empty()).map(String::from);
    Ok(BoardScope { kind, filter, statuses, sub_query })
}

/// Reads what is on a board. None when Jira says the board doesn't exist (deleted, or you can't see it any more).
pub fn board_scope(twg: &Tool, site: &str, board_id: &str) -> Result<Option<BoardScope>> {
    let (code, v, stderr) = run_json(twg, site, &["jira", "board", "scope", "query", "--id", board_id])?;
    if v.get("ok") == Some(&Value::Bool(false)) {
        if v.pointer("/error/statusCode").and_then(Value::as_i64) == Some(404) {
            return Ok(None);
        }
        return Err(anyhow!(explain(code, &v, &stderr)));
    }
    Ok(Some(parse_board_scope(&v)?))
}

/// The search for what a board's own page in Jira shows: the board's saved search, in the statuses of its columns;
/// on a scrum board only what is in a sprint that is running; on a kanban board with the board's extra search. None
/// when no status is in any column: nothing is on such a board.
pub fn board_jql(scope: &BoardScope) -> Result<Option<String>> {
    let number = |id: &str| !id.is_empty() && id.chars().all(|c| c.is_ascii_digit());
    anyhow::ensure!(number(&scope.filter), "Jira's board has a saved search Enkidu can't use ({})", scope.filter);
    if let Some(bad) = scope.statuses.iter().find(|id| !number(id)) {
        return Err(anyhow!("Jira's board has a status Enkidu can't use ({bad})"));
    }
    if scope.statuses.is_empty() {
        return Ok(None);
    }
    let mut jql = format!("filter = {}", scope.filter);
    if scope.kind == "scrum" {
        jql.push_str(" AND sprint in openSprints()");
    }
    jql.push_str(&format!(" AND status in ({})", scope.statuses.join(", ")));
    if scope.kind != "scrum" {
        if let Some(q) = &scope.sub_query {
            jql.push_str(&format!(" AND ({q})"));
        }
    }
    Ok(Some(jql))
}

/// Which of these tickets are on the board, as its own page in Jira shows it. The keys must exist in Jira (a search
/// that names a key Jira doesn't know fails whole), so they come from a read made just before.
pub fn on_board(twg: &Tool, site: &str, scope: &BoardScope, keys: &[String]) -> Result<Vec<String>> {
    let Some(board) = board_jql(scope)? else { return Ok(Vec::new()) };
    let mut on: Vec<String> = Vec::new();
    for chunk in keys.chunks(BOARD_KEYS) {
        let jql = format!("{board} AND key in ({})", chunk.join(", "));
        for t in query(twg, site, &jql)? {
            // only what was asked about: an answer that names another ticket is not this board's word on ours
            if chunk.contains(&t.key) && !on.contains(&t.key) {
                on.push(t.key);
            }
        }
    }
    on.sort();
    Ok(on)
}

/// Jira's rich text (ADF) as plain text, and the account ids it @mentions.
pub fn adf_text(doc: &Value) -> (String, Vec<String>) {
    fn walk(n: &Value, out: &mut String, mentions: &mut Vec<String>) {
        match n.get("type").and_then(Value::as_str).unwrap_or("") {
            "text" => out.push_str(n.get("text").and_then(Value::as_str).unwrap_or("")),
            "mention" => {
                let attrs = n.get("attrs").cloned().unwrap_or(Value::Null);
                if let Some(id) = attrs.get("id").and_then(Value::as_str) {
                    if !mentions.iter().any(|m| m == id) {
                        mentions.push(id.to_string());
                    }
                }
                out.push_str(attrs.get("text").and_then(Value::as_str).unwrap_or("@someone"));
            }
            "hardBreak" => out.push('\n'),
            "inlineCard" | "blockCard" => out.push_str(n.pointer("/attrs/url").and_then(Value::as_str).unwrap_or("")),
            "emoji" => out.push_str(n.pointer("/attrs/text").or_else(|| n.pointer("/attrs/shortName")).and_then(Value::as_str).unwrap_or("")),
            "media" | "mediaInline" => out.push_str("[attachment]"),
            "rule" => out.push_str("\n---\n"),
            kind => {
                for c in n.get("content").and_then(Value::as_array).into_iter().flatten() {
                    walk(c, out, mentions);
                }
                if matches!(kind, "paragraph" | "heading" | "listItem" | "tableRow" | "codeBlock" | "blockquote" | "mediaSingle") && !out.ends_with('\n') {
                    out.push('\n');
                }
                if kind == "tableCell" || kind == "tableHeader" {
                    out.push(' ');
                }
            }
        }
    }
    let (mut text, mut mentions) = (String::new(), Vec::new());
    if let Some(s) = doc.as_str() {
        return (s.to_string(), mentions); // an older plain-text body
    }
    walk(doc, &mut text, &mut mentions);
    (text.trim().to_string(), mentions)
}

fn comment(c: &Value) -> Result<Comment> {
    let (text, mentions) = adf_text(field(c, "body", SRC)?);
    Ok(Comment {
        id: string(c, "id", SRC)?,
        author: person(field(c, "author", SRC)?)?,
        text,
        mentions,
        created: string(c, "created", SRC)?,
        updated: string(c, "updated", SRC)?,
    })
}

fn attachment(a: &Value) -> Result<Attachment> {
    Ok(Attachment {
        id: string(a, "id", SRC)?,
        filename: string(a, "filename", SRC)?,
        mime: opt_string(a, "mimeType", SRC)?.unwrap_or_default(),
        author: person(a.get("author").unwrap_or(&Value::Null))?,
        created: string(a, "created", SRC)?,
        size: a.get("size").and_then(Value::as_i64).unwrap_or(0),
    })
}

/// One `twg jira workitem get` answer.
pub fn parse_get(v: &Value) -> Result<Got> {
    let mut got = Got::default();
    // one key asked: twg answers with the ticket itself, in a list, not with items
    if let Some(Value::Array(tickets)) = v.get("data") {
        let input = v.pointer("/request/issueIdOrKey").and_then(Value::as_str).unwrap_or("").to_string();
        for d in tickets {
            add_full(&mut got, &input, d)?;
        }
        return Ok(got);
    }
    for (input, d) in items(v)? {
        match d {
            Some(d) => add_full(&mut got, &input, d)?,
            None => got.gone.push(input),
        }
    }
    Ok(got)
}

/// The items of a batch answer: each key asked for, with its ticket, or None when Jira says it doesn't exist (or you
/// can't see it). Any other failed item, or an answer cut short, fails the read.
fn items(v: &Value) -> Result<Vec<(String, Option<&Value>)>> {
    let summary = field(v, "data.summary", SRC)?;
    let truncated = summary.get("truncated").and_then(Value::as_i64).unwrap_or(0) + summary.get("omitted").and_then(Value::as_i64).unwrap_or(0);
    if truncated > 0 {
        return Err(anyhow!(Failure::new("Jira sent only part of the tickets", "Enkidu tries again.")));
    }
    let mut out = Vec::new();
    for item in array(v, "data.items", SRC)? {
        let input = string(item, "input", SRC)?;
        if item.get("ok").and_then(Value::as_bool) != Some(true) {
            let status = item.pointer("/error/status").and_then(Value::as_i64).unwrap_or(0);
            if status == 404 {
                out.push((input, None));
                continue;
            }
            let msg = item.pointer("/error/message").and_then(Value::as_str).unwrap_or("an error");
            return Err(anyhow!(Failure::new(&format!("Jira couldn't read {input}: {msg}"), "Enkidu tries again.")));
        }
        out.push((input, Some(field(item, "data", SRC)?)));
    }
    Ok(out)
}

/// One ticket read whole, asked for as `input`.
fn add_full(got: &mut Got, input: &str, d: &Value) -> Result<()> {
    let comments: Vec<Comment> = match d.get("comments") {
        Some(Value::Array(cs)) => cs.iter().map(comment).collect::<Result<_>>()?,
        Some(Value::Null) | None => return Err(anyhow!("Jira's answer for {input} has no comments")),
        Some(_) => return Err(anyhow!("Jira's comments for {input} aren't a list")),
    };
    let attachments: Vec<Attachment> = match d.get("attachment") {
        Some(Value::Array(a)) => a.iter().map(attachment).collect::<Result<_>>()?,
        _ => Vec::new(), // Jira sends null for none
    };
    let mut ticket = ticket_from(d, comments.len(), attachments.len())?;
    if let Some(last) = comments.iter().max_by(|a, b| super::parse::parse_time(&a.created).cmp(&super::parse::parse_time(&b.created))) {
        ticket.last_comment_by = last.author.as_ref().map(|p| p.name.clone());
        ticket.last_comment_at = Some(last.created.clone());
    }
    if !input.is_empty() && !ticket.key.eq_ignore_ascii_case(input) {
        got.moved.push((input.to_string(), ticket.key.clone()));
    }
    got.tickets.push(FullTicket { ticket, comments, attachments });
    Ok(())
}

/// Reads tickets whole (fields, comments, attachments), in batches.
pub fn get(twg: &Tool, site: &str, keys: &[String]) -> Result<Got> {
    let mut all = Got::default();
    for chunk in keys.chunks(BATCH) {
        let mut args = vec!["jira", "workitem", "get"];
        args.extend(chunk.iter().map(String::as_str));
        args.extend(["--comments", "--fields", "summary,status,assignee,updated,attachment"]);
        let (code, v) = if chunk.len() == 1 {
            match call_or_gone(twg, site, &args)? {
                Some(answer) => answer,
                None => {
                    all.gone.push(chunk[0].clone());
                    continue;
                }
            }
        } else {
            call(twg, site, &args)?
        };
        let got = parse_get(&v)?;
        // exit 3 means some items failed; they must all be "gone" answers to count as a read
        if code != 0 && code != 3 {
            return Err(anyhow!(explain(code, &v, "")));
        }
        all.tickets.extend(got.tickets);
        all.gone.extend(got.gone);
        all.moved.extend(got.moved);
    }
    Ok(all)
}

/// What a ticket asks for, and what kind of ticket it is.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Described {
    /// Jira's document (ADF), or None when the ticket has no description
    pub description: Option<Value>,
    pub issue_type: Option<String>,
    pub priority: Option<String>,
}

/// The longest description kept, as JSON; a longer one keeps its first blocks and says the rest is in Jira.
const DESCRIPTION_LIMIT: usize = 64 * 1024;

/// Reads tickets' descriptions, types and priorities, in batches, by the keys asked for. Read only for new and
/// changed tickets (a description is a few KB, the search's answer grows ninefold with them). A ticket Jira says
/// doesn't exist is left out.
pub fn describe(twg: &Tool, site: &str, keys: &[String]) -> Result<HashMap<String, Described>> {
    let mut out = HashMap::new();
    for chunk in keys.chunks(BATCH) {
        let mut args = vec!["jira", "workitem", "get"];
        args.extend(chunk.iter().map(String::as_str));
        args.extend(["--fields", "description,issuetype,priority"]);
        let answer = if chunk.len() == 1 { call_or_gone(twg, site, &args)? } else { Some(call(twg, site, &args)?) };
        let Some((code, v)) = answer else { continue };
        // exit 3 means some items failed; parse_describe fails the read unless they are all "gone"
        let described = parse_describe(&v)?;
        if code != 0 && code != 3 {
            return Err(anyhow!(explain(code, &v, "")));
        }
        out.extend(described);
    }
    Ok(out)
}

/// One answer of `describe`: each ticket's description, type and priority, by the key asked for.
pub fn parse_describe(v: &Value) -> Result<HashMap<String, Described>> {
    let one = |d: &Value| Described {
        description: kept_description(d.get("description")),
        issue_type: name_of(d, &["issuetype", "issueType"]),
        priority: name_of(d, &["priority"]),
    };
    let mut out = HashMap::new();
    // one key asked: the ticket itself, in a list
    if let Some(Value::Array(tickets)) = v.get("data") {
        let input = v.pointer("/request/issueIdOrKey").and_then(Value::as_str).unwrap_or("");
        for d in tickets {
            let key = if input.is_empty() { string(d, "key", SRC)? } else { input.to_string() };
            out.insert(key, one(d));
        }
        return Ok(out);
    }
    for (input, d) in items(v)? {
        if let Some(d) = d {
            out.insert(input, one(d));
        }
    }
    Ok(out)
}

/// A description as kept: none for null or an empty document; a very long one keeps its first blocks.
pub(crate) fn kept_description(d: Option<&Value>) -> Option<Value> {
    let d = d?;
    match d {
        Value::String(s) if s.trim().is_empty() => None,
        Value::String(_) => Some(d.clone()), // an older plain-text description
        Value::Object(_) => {
            let content = d.get("content").and_then(Value::as_array)?;
            if content.is_empty() {
                return None;
            }
            if d.to_string().len() <= DESCRIPTION_LIMIT {
                return Some(d.clone());
            }
            let mut kept = Vec::new();
            let mut size = 0;
            for block in content {
                let n = block.to_string().len();
                if size + n > DESCRIPTION_LIMIT {
                    break;
                }
                size += n;
                kept.push(block.clone());
            }
            kept.push(json!({"type": "paragraph", "content": [{"type": "text", "text": "The rest of the description is in Jira.", "marks": [{"type": "em"}]}]}));
            let mut out = d.clone();
            out["content"] = Value::Array(kept);
            Some(out)
        }
        _ => None,
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::work::github::tests::fixture;
    use serde_json::json;

    #[test]
    fn reads_the_real_query_pages() {
        let (first, next) = parse_query_page(&fixture("twg-query-page1.json")).unwrap();
        assert_eq!((first.len(), next.as_deref()), (20, Some("cursor-1")));
        let (_, next) = parse_query_page(&fixture("twg-query-page2.json")).unwrap();
        assert_eq!(next.as_deref(), Some("cursor-2"));
        let (last, next) = parse_query_page(&fixture("twg-query-page3.json")).unwrap();
        assert_eq!((last.len(), next), (5, None));
        let (all, _) = parse_query_page(&fixture("twg-tickets.json")).unwrap();
        assert_eq!(all.len(), 45);
        let t = &all[0];
        assert_eq!((t.key.as_str(), t.project_key.as_str(), t.category.as_str()), ("OPS-6294", "OPS", "indeterminate"));
        assert_eq!(t.assignee_id.as_deref(), Some("acc-me"));
    }

    #[test]
    fn reads_tickets_whole_with_comments_mentions_and_attachments() {
        let got = parse_get(&fixture("twg-get.json")).unwrap();
        assert_eq!(got.tickets.len(), 25);
        assert!(got.gone.is_empty() && got.moved.is_empty());
        let busy = got.tickets.iter().find(|t| t.ticket.key == "SH-20698").unwrap();
        assert_eq!(busy.comments.len(), 18);
        assert_eq!(busy.ticket.attachments, busy.attachments.len());
        assert!(busy.attachments.len() >= 1);
        let mentions: usize = got.tickets.iter().flat_map(|t| &t.comments).map(|c| c.mentions.len()).sum();
        assert!(mentions > 0, "the @mentions in the comments are found");
        let with_mention = got.tickets.iter().flat_map(|t| &t.comments).find(|c| !c.mentions.is_empty()).unwrap();
        assert!(with_mention.text.contains("@Person"), "{}", with_mention.text);
        assert!(got.tickets.iter().flat_map(|t| &t.comments).any(|c| c.author.as_ref().is_some_and(|a| a.bot)), "apps are told apart");
    }

    #[test]
    fn a_deleted_ticket_is_an_answer_and_any_other_failed_item_fails_the_read() {
        let got = parse_get(&fixture("twg-get-partial.json")).unwrap();
        assert_eq!(got.tickets.len(), 1);
        assert_eq!(got.gone, ["SH-99999999"]);
        let mut v = fixture("twg-get-partial.json");
        v["data"]["items"][1]["error"]["status"] = json!(500);
        assert!(parse_get(&v).unwrap_err().to_string().contains("couldn't read SH-99999999"));
        let mut v = fixture("twg-get.json");
        v["data"]["summary"]["truncated"] = json!(2);
        assert!(parse_get(&v).is_err(), "part of an answer is a failed read");
        let mut v = fixture("twg-get.json");
        v["data"]["items"][0]["data"]["status"].as_object_mut().unwrap().remove("statusCategory");
        assert!(parse_get(&v).unwrap_err().to_string().contains("status.statusCategory.key"));
        let mut v = fixture("twg-get.json");
        v["data"]["items"][0]["data"]["key"] = json!("XY-5");
        assert_eq!(parse_get(&v).unwrap().moved, [("OPS-6294".to_string(), "XY-5".to_string())]);
    }

    #[test]
    fn one_key_is_answered_with_the_ticket_itself_and_still_read() {
        // twg answers `get` with one key as {data: [ticket]}, not {data: {items, summary}}
        let got = parse_get(&fixture("twg-get-single.json")).unwrap();
        assert_eq!(got.tickets.len(), 1);
        assert_eq!(got.tickets[0].ticket.key, "SH-21349");
        assert!(got.gone.is_empty() && got.moved.is_empty());
        let mut v = fixture("twg-get-single.json");
        v["data"][0]["key"] = json!("XY-5");
        assert_eq!(parse_get(&v).unwrap().moved, [("SH-21349".to_string(), "XY-5".to_string())]);
        let mut v = fixture("twg-get-single.json");
        v["data"][0].as_object_mut().unwrap().remove("comments");
        assert!(parse_get(&v).is_err(), "a missing field still fails the read");
    }

    #[test]
    fn the_search_says_each_tickets_type_and_priority() {
        let (all, _) = parse_query_page(&fixture("twg-tickets.json")).unwrap();
        assert!(all.iter().all(|t| t.issue_type.is_some() && t.priority.is_some()), "the search sends both");
        assert!(all.iter().any(|t| t.issue_type.as_deref() == Some("Bug")));
        assert!(all.iter().all(|t| t.description.is_none()), "descriptions are read apart, for changed tickets");
        let got = parse_get(&fixture("twg-get.json")).unwrap();
        assert!(got.tickets.iter().all(|t| t.ticket.issue_type.is_none()), "a read whole doesn't ask for them");
    }

    #[test]
    fn reads_descriptions_types_and_priorities_from_the_real_answers() {
        let d = parse_describe(&fixture("twg-describe.json")).unwrap();
        assert_eq!(d.len(), 47, "every ticket asked for but the one Jira says doesn't exist");
        assert!(!d.contains_key("SH-99999999"));
        assert_eq!(d.values().filter(|x| x.description.is_none()).count(), 6, "six tickets have no description");
        let types: std::collections::BTreeSet<&str> = d.values().filter_map(|x| x.issue_type.as_deref()).collect();
        assert!(types.contains("Bug") && types.contains("Task") && types.contains("Sub-task"), "{types:?}");
        assert!(d.values().all(|x| x.priority.is_some()));
        let doc = d.values().find_map(|x| x.description.clone()).unwrap();
        assert_eq!(doc["type"], "doc", "kept as Jira's document, so the page can show its headings, lists and code");
        // one key: the ticket itself, by the key asked for
        let one = parse_describe(&fixture("twg-describe-single.json")).unwrap();
        assert_eq!(one.keys().collect::<Vec<_>>(), ["SH-21349"]);
        assert!(one["SH-21349"].description.is_some());
        // a failed item (not a missing ticket) fails the read
        let mut v = fixture("twg-describe.json");
        v["data"]["items"][0] = json!({"input": "OPS-6294", "ok": false, "error": {"message": "Internal server error", "status": 500}});
        assert!(parse_describe(&v).unwrap_err().to_string().contains("couldn't read OPS-6294"));
    }

    #[test]
    fn reads_jiras_boards_from_the_real_answers() {
        let (boards, last) = parse_boards_page(&fixture("twg-boards.json")).unwrap();
        assert!(last);
        assert_eq!(boards.len(), 8);
        let dev = boards.iter().find(|b| b.name == "Shop Dev").unwrap();
        assert_eq!((dev.id.as_str(), dev.kind.as_str(), dev.project_key.as_deref()), ("5", "scrum", Some("SH")));
        // a board in someone's profile has no project
        let (other, _) = parse_boards_page(&fixture("twg-boards-ops.json")).unwrap();
        assert_eq!((other[0].id.as_str(), other[0].kind.as_str(), other[0].project_key.as_deref()), ("14", "kanban", None));
        // a missing field fails the read
        let mut v = fixture("twg-boards.json");
        v["data"]["boards"][0].as_object_mut().unwrap().remove("type");
        assert!(parse_boards_page(&v).unwrap_err().to_string().contains("`type`"));
        let mut v = fixture("twg-boards.json");
        v["data"].as_object_mut().unwrap().remove("isLast");
        assert!(parse_boards_page(&v).unwrap_err().to_string().contains("data.isLast"));
    }

    #[test]
    fn a_boards_own_page_in_jira_as_a_search() {
        // a scrum board: its saved search, only the sprint that is running, the statuses of its columns
        let scrum = parse_board_scope(&fixture("twg-board-scope-5.json")).unwrap();
        assert_eq!((scrum.kind.as_str(), scrum.filter.as_str(), scrum.sub_query.as_deref()), ("scrum", "10437", None));
        assert_eq!(
            board_jql(&scrum).unwrap().unwrap(),
            "filter = 10437 AND sprint in openSprints() AND status in (10116, 10112, 10113, 10000, 10111, 10154, 10151, 3, 10002, 10162, 10168, 10001, 10152, 10130, 10115)"
        );
        // a kanban board: no sprint, its extra search, and its first column is its backlog page, not the board
        let kanban = parse_board_scope(&fixture("twg-board-scope-20.json")).unwrap();
        assert_eq!((kanban.kind.as_str(), kanban.filter.as_str()), ("kanban", "10461"));
        assert!(!kanban.statuses.contains(&"10000".to_string()), "what is in its backlog isn't on the board");
        assert_eq!(
            board_jql(&kanban).unwrap().unwrap(),
            "filter = 10461 AND status in (10111, 10154, 10151, 10112, 10113, 10116, 3, 10002, 10130, 10001, 10162, 10115) AND (fixVersion in unreleasedVersions() OR fixVersion is EMPTY)"
        );
        // a scrum board has no backlog column: a first column named so is a column like the others
        let mut v = fixture("twg-board-scope-5.json");
        v["data"]["columnConfig"]["columns"][0]["name"] = json!("Backlog");
        assert!(parse_board_scope(&v).unwrap().statuses.contains(&"10116".to_string()));
        // no status in any column: nothing is on such a board, and nothing is asked
        let empty = BoardScope { kind: "kanban".into(), filter: "7".into(), statuses: Vec::new(), sub_query: None };
        assert_eq!(board_jql(&empty).unwrap(), None);
        // ids are numbers: anything else never reaches a search
        let odd = BoardScope { kind: "scrum".into(), filter: "7) OR (1 = 1".into(), statuses: vec!["3".into()], sub_query: None };
        assert!(board_jql(&odd).is_err());
        let odd = BoardScope { kind: "scrum".into(), filter: "7".into(), statuses: vec!["3) OR status in (4".into()], sub_query: None };
        assert!(board_jql(&odd).is_err());
        // a missing field fails the read
        let mut v = fixture("twg-board-scope-5.json");
        v["data"].as_object_mut().unwrap().remove("filter");
        assert!(parse_board_scope(&v).unwrap_err().to_string().contains("data.filter.id"));
        let mut v = fixture("twg-board-scope-5.json");
        v["data"]["columnConfig"]["columns"][1].as_object_mut().unwrap().remove("statuses");
        assert!(parse_board_scope(&v).is_err());
    }

    #[test]
    fn an_empty_description_is_none_and_a_huge_one_keeps_its_start() {
        assert_eq!(kept_description(Some(&json!(null))), None);
        assert_eq!(kept_description(Some(&json!({"type": "doc", "version": 1, "content": []}))), None);
        assert_eq!(kept_description(Some(&json!("  "))), None);
        assert_eq!(kept_description(Some(&json!("plain text"))), Some(json!("plain text")));
        let block = json!({"type": "paragraph", "content": [{"type": "text", "text": "x".repeat(10_000)}]});
        let huge = json!({"type": "doc", "version": 1, "content": vec![block; 20]});
        let kept = kept_description(Some(&huge)).unwrap();
        assert!(kept.to_string().len() < DESCRIPTION_LIMIT + 500);
        let content = kept["content"].as_array().unwrap();
        assert_eq!(content.last().unwrap()["content"][0]["text"], "The rest of the description is in Jira.");
        assert_eq!(content.len(), 7, "six blocks of 10 KB fit, then the note");
    }

    #[test]
    fn says_what_to_do_about_each_failure() {
        let out = explain(77, &fixture("twg-logged-out.json"), "");
        assert_eq!(out.message, "Jira: twg isn't signed in");
        assert_eq!(out.fix, "Run `twg login --force`.");
        assert!(explain(1, &fixture("twg-bad-site.json"), "").message.contains("can't find the Jira site"));
        let net = json!({"ok": false, "error": {"message": "fetch failed: getaddrinfo ENOTFOUND api.atlassian.com"}});
        assert_eq!(explain(1, &net, "").message, "No connection to Jira");
        let limit = json!({"ok": false, "error": {"statusCode": 429, "message": "Too Many Requests"}});
        assert!(explain(1, &limit, "").message.contains("rate limit"));
        let other = json!({"ok": false, "error": {"message": "Something new at https://x.atlassian.net/rest"}});
        let msg = explain(2, &other, "").message;
        assert!(msg.contains("Something new") && !msg.contains("atlassian.net"), "URLs are never shown: {msg}");
    }

    #[test]
    fn turns_rich_text_into_plain_text() {
        let doc = json!({"type": "doc", "version": 1, "content": [
            {"type": "paragraph", "content": [
                {"type": "mention", "attrs": {"id": "acc-me", "text": "@Me Dev"}},
                {"type": "text", "text": " the total is off"},
                {"type": "hardBreak"},
                {"type": "text", "text": "see", "marks": [{"type": "code"}]},
                {"type": "inlineCard", "attrs": {"url": "https://example.com/x"}}]},
            {"type": "mediaSingle", "content": [{"type": "media", "attrs": {"id": "m1"}}]},
            {"type": "bulletList", "content": [{"type": "listItem", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "one"}]}]}]}
        ]});
        let (text, mentions) = adf_text(&doc);
        assert_eq!(text, "@Me Dev the total is off\nseehttps://example.com/x\n[attachment]\none");
        assert_eq!(mentions, ["acc-me"]);
        assert_eq!(adf_text(&json!("plain")).0, "plain");
    }
}
