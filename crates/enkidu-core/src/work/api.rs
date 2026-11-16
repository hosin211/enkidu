//! The local API the `enkidu` command talks to (routes `/api/<token>/<command>` on the hook server): read the Work index
//! and correct what the automatic links got wrong. `enkidu` run inside a Claude that Enkidu started knows its session
//! and workspace from its environment; nothing here writes to GitHub or Jira.

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use std::collections::HashSet;
use crate::events::Events;

use super::model::{Item, Pr};
use super::{changed, parse, pr_summary, projects, relink, session_context, set_item, snapshot_of};
use crate::db::{now_ms, Db};
use crate::state::Shared;

/// The repo a bare PR number belongs to (the call's project), worked out only when needed.
type RepoHere<'a> = &'a dyn Fn() -> Option<String>;

fn text(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(String::from)
}

/// The things a call is about: its session and workspace (from `enkidu`'s environment).
pub struct Here {
    session: Option<String>,
    workspace: Option<String>,
}

impl Here {
    /// The terminal's workspace knows the conversation its Claude is in now; the session in the terminal's environment
    /// is the one it started with, which `/resume` or `/clear` may have left behind.
    fn of(db: &Db, args: &Value) -> Here {
        let workspace = text(args, "workspace");
        let now = workspace.as_deref().and_then(|w| db.workspace(w).ok().flatten()).map(|w| w.session_id);
        Here { session: now.or_else(|| text(args, "session")), workspace }
    }

    fn things(&self) -> Vec<String> {
        let mut v = Vec::new();
        if let Some(s) = &self.session {
            v.push(format!("session:{s}"));
        }
        if let Some(w) = &self.workspace {
            v.push(format!("workspace:{w}"));
        }
        v
    }
}

/// The GitHub repo of the call's project: the workspace's, else the folder the session ran in.
fn repo_here(shared: &Shared, here: &Here) -> Option<String> {
    let infos = projects(shared);
    if let Some(ws) = here.workspace.as_deref().and_then(|w| shared.db.workspace(w).ok().flatten()) {
        return infos.iter().find(|p| p.id == ws.project_id).and_then(|p| p.repo.clone());
    }
    let session = here.session.as_deref()?;
    let cwd = shared.db.work_sessions().ok()?.into_iter().find(|s| s.id == session)?.cwd;
    let workspaces = shared.db.workspaces().unwrap_or_default();
    infos.iter().find(|p| p.holds(&cwd, &workspaces)).and_then(|p| p.repo.clone())
}

/// `2414`, `#2414`, `owner/repo#2414` or a PR's link, as `pr:owner/repo#2414`; a ticket key as `ticket:SH-1`.
fn target(kind: &str, r: &str, repo: RepoHere) -> Result<String> {
    match kind {
        "pr" => {
            if let Some(found) = parse::pr_refs(r).into_iter().next() {
                return Ok(format!("pr:{found}"));
            }
            if let Some((repo, n)) = r.rsplit_once('#').filter(|(repo, n)| repo.contains('/') && n.parse::<i64>().is_ok()) {
                return Ok(format!("pr:{repo}#{n}"));
            }
            let n = r.trim_start_matches('#');
            if n.parse::<i64>().is_err() {
                bail!("\"{r}\" isn't a PR: give its number, owner/repo#number or its link");
            }
            let repo = repo().ok_or_else(|| anyhow!("Can't tell this session's repo: give owner/repo#{n}"))?;
            Ok(format!("pr:{repo}#{n}"))
        }
        "ticket" => {
            let key = r.trim().to_uppercase();
            if parse::ticket_keys(&key).first() != Some(&key) {
                bail!("\"{r}\" isn't a ticket key (like SH-21533)");
            }
            Ok(format!("ticket:{key}"))
        }
        // a task of your own, by the number its card shows (`3`, `#3`)
        "task" => {
            let n = r.trim().trim_start_matches("own:").trim_start_matches('#');
            match n.parse::<i64>() {
                Ok(id) if id > 0 => Ok(format!("own:{id}")),
                _ => bail!("\"{r}\" isn't one of your own tasks: give its number"),
            }
        }
        other => bail!("Link a pr, a ticket or a task, not \"{other}\""),
    }
}

fn pr_json(p: &Pr) -> Value {
    json!({ "ref": p.reference(), "number": p.number, "title": p.title, "url": p.url, "state": p.state,
            "head": p.head, "base": p.base, "summary": pr_summary(p) })
}

fn item_json(i: &Item) -> Value {
    json!({ "id": i.id, "kind": i.kind, "title": i.title, "who": i.who, "url": i.url, "pr": i.pr, "ticket": i.ticket,
            "path": i.path, "line": i.line, "at": i.at, "bot": i.bot, "detail": i.detail })
}

/// What Enkidu knows about the session's work: its PRs, their tickets, and what waits on you there.
fn status(db: &Db, here: &Here) -> Result<Value> {
    let snap = snapshot_of(db)?;
    let mine: HashSet<String> = here.things().into_iter().collect();
    let pr_refs: HashSet<String> = snap.links.iter().filter(|l| mine.contains(&l.b) && l.a.starts_with("pr:")).map(|l| l.a[3..].to_string()).collect();
    let mut keys: HashSet<String> = snap.links.iter().filter(|l| mine.contains(&l.b) && l.a.starts_with("ticket:")).map(|l| l.a[7..].to_string()).collect();
    keys.extend(snap.links.iter().filter(|l| l.a.starts_with("ticket:") && l.b.starts_with("pr:") && pr_refs.contains(&l.b[3..])).map(|l| l.a[7..].to_string()));
    let prs: Vec<Value> = snap.prs.iter().filter(|p| pr_refs.contains(&p.reference())).map(pr_json).collect();
    let tickets: Vec<Value> = snap
        .tickets
        .iter()
        .filter(|t| keys.contains(&t.key))
        .map(|t| {
            // what the ticket asks for, as text, so Claude knows the task
            let description = t.description.as_ref().map(|d| super::jira::adf_text(d).0);
            json!({ "key": t.key, "summary": t.summary, "status": t.status, "type": t.issue_type, "description": description, "url": t.url })
        })
        .collect();
    let items: Vec<Value> = snap
        .items
        .iter()
        .filter(|i| i.state == "open")
        .filter(|i| i.pr.as_ref().is_some_and(|p| pr_refs.contains(p)) || i.ticket.as_ref().is_some_and(|k| keys.contains(k)))
        .map(item_json)
        .collect();
    let text = here.session.as_deref().map(|s| session_context(db, s).unwrap_or_default()).unwrap_or_default();
    Ok(json!({ "session": here.session, "workspace": here.workspace, "prs": prs, "tickets": tickets, "items": items, "text": text,
               "health": snap.health }))
}

fn link(db: &Db, here: &Here, args: &Value, remove: bool, repo: RepoHere) -> Result<Value> {
    let kind = text(args, "kind").unwrap_or_default();
    let r = text(args, "ref").ok_or_else(|| anyhow!("Say what to link: enkidu link pr 2414, or enkidu link ticket SH-21533"))?;
    let thing = here
        .things()
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("Run enkidu in a terminal Enkidu started (it knows the session there), or pass --session ID"))?;
    let target = target(&kind, &r, repo)?;
    let now = now_ms();
    if remove {
        db.work_unlink(&target, &thing, now)?;
    } else {
        let how = if text(args, "how").as_deref() == Some("you") { "you" } else { "claude" };
        let words = if how == "you" { "linked by you" } else { "linked by Claude" };
        db.work_link(&target, &thing, how, words, now)?;
    }
    Ok(json!({ if remove { "unlinked" } else { "linked" }: target, "to": thing }))
}

/// The PRs a ticket's key joins it to (its branch, title or commits), as references.
fn prs_of_ticket(snap: &super::model::Snapshot, key: &str) -> Vec<String> {
    let me = format!("ticket:{key}");
    snap.links.iter().filter(|l| l.a == me).filter_map(|l| l.b.strip_prefix("pr:")).map(String::from).collect()
}

fn list(db: &Db, args: &Value) -> Result<Value> {
    let snap = snapshot_of(db)?;
    let needs_me = args.get("needs_me").and_then(Value::as_bool).unwrap_or(false);
    let waiting: Vec<&Item> = snap.items.iter().filter(|i| i.state == "open" && !i.bot).collect();
    match text(args, "what").as_deref() {
        Some("prs") => Ok(json!(snap
            .prs
            .iter()
            .filter(|p| p.state == "OPEN")
            .filter(|p| !needs_me || waiting.iter().any(|i| i.pr.as_deref() == Some(&p.reference())))
            .map(pr_json)
            .collect::<Vec<_>>())),
        // each ticket with its PRs and how much waits on you there (on it, or on its PRs)
        Some("tickets") => Ok(json!(snap
            .tickets
            .iter()
            .filter_map(|t| {
                let prs = prs_of_ticket(&snap, &t.key);
                let waits = waiting.iter().filter(|i| i.ticket.as_deref() == Some(&t.key) || i.pr.as_ref().is_some_and(|p| prs.contains(p))).count();
                if needs_me && waits == 0 {
                    return None;
                }
                let prs: Vec<Value> = snap.prs.iter().filter(|p| prs.contains(&p.reference())).map(pr_json).collect();
                Some(json!({ "key": t.key, "summary": t.summary, "status": t.status, "url": t.url, "prs": prs, "waiting": waits }))
            })
            .collect::<Vec<_>>())),
        Some("sessions") => Ok(json!(snap
            .sessions
            .iter()
            .map(|s| json!({ "id": s.id, "title": s.title, "cwd": s.cwd, "branches": s.branches, "workspace": s.workspace_id,
                              "lastSeen": s.last_seen }))
            .collect::<Vec<_>>())),
        _ => bail!("List prs, tickets or sessions"),
    }
}

fn items(db: &Db, args: &Value, repo: RepoHere) -> Result<Value> {
    let snap = snapshot_of(db)?;
    let pr = match text(args, "pr") {
        Some(r) => Some(target("pr", &r, repo)?[3..].to_string()),
        None => None,
    };
    let ticket = text(args, "ticket").map(|t| t.to_uppercase());
    // a ticket's items include those on its PRs
    let ticket_prs = ticket.as_deref().map(|k| prs_of_ticket(&snap, k)).unwrap_or_default();
    Ok(json!(snap
        .items
        .iter()
        .filter(|i| i.state == "open")
        .filter(|i| pr.is_none() || i.pr == pr)
        .filter(|i| ticket.is_none() || i.ticket == ticket || i.pr.as_ref().is_some_and(|p| ticket_prs.contains(p)))
        .map(item_json)
        .collect::<Vec<_>>()))
}

/// What a note without `--pr` or `--ticket` is about: the ticket linked to the session, else the ticket of the PR it
/// works on now (its workspace's branch, or its only PR), else that PR.
fn subject_here(db: &Db, here: &Here) -> Result<String> {
    let snap = snapshot_of(db)?;
    let mine: HashSet<String> = here.things().into_iter().collect();
    let linked: Vec<&str> = snap.links.iter().filter(|l| mine.contains(&l.b)).map(|l| l.a.as_str()).collect();
    if let Some(t) = linked.iter().find_map(|a| a.strip_prefix("ticket:")) {
        return Ok(t.to_string());
    }
    // a task of your own that this session was linked to is named whole (`own:3`): its number alone says nothing
    if let Some(own) = linked.iter().find(|a| a.starts_with("own:")) {
        return Ok(own.to_string());
    }
    let prs: Vec<&str> = linked.iter().filter_map(|a| a.strip_prefix("pr:")).collect();
    let branch = here.workspace.as_deref().and_then(|w| db.workspace(w).ok().flatten()).map(|w| w.branch);
    let current = branch.and_then(|b| snap.prs.iter().find(|p| p.head == b && prs.contains(&p.reference().as_str())).map(|p| p.reference()));
    let pr = match (current, prs.as_slice()) {
        (Some(pr), _) => pr,
        (None, [only]) => only.to_string(),
        (None, []) => bail!("This session has no PR or ticket yet: enkidu note --pr 2414 \"…\" or --ticket SH-1"),
        (None, _) => bail!("This session works on {} PRs: say which, enkidu note --pr 2414 \"…\" (or --ticket SH-1)", prs.len()),
    };
    let ticket = snap.links.iter().find(|l| l.b == format!("pr:{pr}")).and_then(|l| l.a.strip_prefix("ticket:"));
    Ok(ticket.map(String::from).unwrap_or(pr))
}

/// `--task 3`: the card of that task of your own (`own:3`), if there is one.
fn own_task(db: &Db, r: &str) -> Result<String> {
    let card = target("task", r, &|| None)?;
    let id = super::model::own_id(&card).unwrap_or_default();
    if db.work_own_task(id)?.is_none() {
        bail!("There is no task {id} of your own");
    }
    Ok(card)
}

fn note(db: &Db, here: &Here, args: &Value, repo: RepoHere) -> Result<Value> {
    let words = text(args, "text").ok_or_else(|| anyhow!("Say what to note: enkidu note \"…\""))?;
    let subject = if let Some(t) = text(args, "ticket") {
        t.to_uppercase()
    } else if let Some(t) = text(args, "task") {
        own_task(db, &t)?
    } else if let Some(p) = text(args, "pr") {
        target("pr", &p, repo)?[3..].to_string()
    } else {
        subject_here(db, here)?
    };
    let words: String = words.chars().take(500).collect();
    db.work_event(&subject, "note", &words, now_ms())?;
    Ok(json!({ "noted": subject }))
}

fn item(db: &Db, args: &Value) -> Result<Value> {
    match (text(args, "action").as_deref(), text(args, "id")) {
        (Some(a @ ("done" | "snoozed" | "open")), Some(id)) => {
            set_item(db, &id, a, None, now_ms())?;
            Ok(json!({ "item": id, "state": a }))
        }
        _ => bail!("enkidu item done ID, or enkidu item snooze ID"),
    }
}

/// A call that reads or corrects the index: its answer, and whether the index changed.
pub fn answer(db: &Db, command: &str, args: &Value, repo: RepoHere) -> Result<(Value, bool)> {
    let here = Here::of(db, args);
    Ok(match command {
        "status" => (status(db, &here)?, false),
        "link" => (link(db, &here, args, false, repo)?, true),
        "unlink" => (link(db, &here, args, true, repo)?, true),
        "list" => (list(db, args)?, false),
        "items" => (items(db, args, repo)?, false),
        "item" => (item(db, args)?, true),
        "note" => (note(db, &here, args, repo)?, true),
        _ => bail!("enkidu doesn't know \"{command}\""),
    })
}

/// `enkidu report done|review|blocked "…" [--image FILE]…`: how the task ended, in Claude's words, with screenshots. It is
/// about the ticket (or PR) named, else the one this session works on; it shows on that task in the Work page and in
/// its timeline. The images are given as absolute paths (`enkidu` makes them so) and copied in.
pub fn report(db: &Db, data_dir: &std::path::Path, here: &Here, args: &Value, repo: RepoHere) -> Result<Value> {
    let state = text(args, "state").unwrap_or_default();
    if !super::reports::STATES.contains(&state.as_str()) && state != super::reports::UPDATE {
        bail!("enkidu report done, review or blocked: done when nothing needs the user, review when it wants their review, blocked when you couldn't finish");
    }
    let words = text(args, "text").unwrap_or_default();
    let subject = if let Some(t) = text(args, "ticket") {
        t.to_uppercase()
    } else if let Some(t) = text(args, "task") {
        own_task(db, &t)?
    } else if let Some(p) = text(args, "pr") {
        target("pr", &p, repo)?[3..].to_string()
    } else {
        subject_here(db, here)?
    };
    let images: Vec<std::path::PathBuf> =
        args.get("images").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(std::path::PathBuf::from).collect();
    if let Some(relative) = images.iter().find(|p| !p.is_absolute()) {
        bail!("{} isn't a full path", relative.display());
    }
    let now = now_ms();
    let r = super::reports::save(db, data_dir, &subject, here.session.as_deref(), here.workspace.as_deref(), &state, &words, &images, now)?;
    let first = r.text.lines().next().unwrap_or("").chars().take(160).collect::<String>();
    let pictures = match r.images.len() {
        0 => String::new(),
        1 => " (1 screenshot)".into(),
        n => format!(" ({n} screenshots)"),
    };
    db.work_event(&subject, "report", &format!("{}: {first}{pictures}", super::reports::words(&state)), now)?;
    Ok(json!({ "reported": subject, "state": state, "words": super::reports::words(&state), "id": r.id, "images": r.images.len() }))
}

/// `enkidu workflow`: the workflow this workspace's Claude works for (the one running a task here, else the latest that
/// ran one here): its name, this task (which, its kind, its minutes and how many are left) and how the others stand.
pub fn workflow(db: &Db, args: &Value) -> Result<Value> {
    let here = Here::of(db, args);
    let ws = here.workspace.clone().ok_or_else(|| anyhow!("enkidu workflow works in a terminal of an Enkidu workspace (it knows the workspace there), or pass --workspace ID"))?;
    let flows = db.work_flows()?;
    let here_runs = |t: &super::flows::FlowTask| t.workspace_id.as_deref() == Some(ws.as_str());
    let found = flows
        .iter()
        .find(|f| f.state == "running" && f.current.and_then(|c| f.tasks.get(c as usize)).is_some_and(|t| here_runs(t) && t.state == "running"))
        .or_else(|| flows.iter().filter(|f| f.tasks.iter().any(here_runs)).max_by_key(|f| f.updated_at));
    let Some(f) = found else { return Ok(json!({ "workflow": null })) };
    let now = now_ms();
    let mine = f.tasks.iter().position(|t| here_runs(t) && t.state == "running").or_else(|| f.tasks.iter().rposition(here_runs));
    let task = mine.map(|i| {
        let t = &f.tasks[i];
        let minutes = t.minutes.unwrap_or(f.minutes);
        // minutes left, counted up: 19 min 59 s is 20
        let left = t.started_at.filter(|_| t.state == "running").map(|s| ((s + minutes as i64 * 60_000 - now + 59_999) / 60_000).max(0));
        json!({ "n": i + 1, "subject": t.subject, "title": t.title, "kind": t.kind, "state": t.state, "minutes": minutes, "minutesLeft": left })
    });
    let tasks: Vec<Value> = f
        .tasks
        .iter()
        .enumerate()
        .map(|(n, t)| json!({ "n": n + 1, "subject": t.subject, "title": t.title, "kind": t.kind, "state": t.state, "yours": Some(n) == mine }))
        .collect();
    Ok(json!({ "workflow": { "name": f.name, "state": f.state, "live": f.live, "of": f.tasks.len(), "task": task, "tasks": tasks } }))
}

/// `enkidu live on|off`: this workspace goes Live (its changes into the project's main folder, where the dev environment
/// runs; another workspace's Live goes back first), or the main folder goes back. `enkidu live` says which one is Live.
fn live(app: &Events, shared: &Shared, args: &Value) -> Result<Value> {
    let here = Here::of(&shared.db, args);
    let id = here.workspace.clone().ok_or_else(|| anyhow!("enkidu live works in a terminal of an Enkidu workspace (it knows the workspace there), or pass --workspace ID"))?;
    let ws = shared.db.workspace(&id)?.ok_or_else(|| anyhow!("That workspace is gone"))?;
    let project = shared.db.project(&ws.project_id)?.ok_or_else(|| anyhow!("Its project is gone"))?;
    let live_name = |id: Option<&str>| id.and_then(|i| shared.db.workspace(i).ok().flatten()).map(|w| w.name);
    match text(args, "action").as_deref() {
        Some("on") => {
            let out = crate::live::set_live(app, shared, &ws.project_id, Some(&ws.id))?;
            Ok(json!({ "live": true, "workspace": ws.name, "note": out.note, "output": out.output }))
        }
        Some("off") => {
            if project.live_workspace_id.as_deref() != Some(ws.id.as_str()) {
                let now = live_name(project.live_workspace_id.as_deref());
                return Ok(json!({ "live": false, "workspace": ws.name, "note": match now {
                    Some(other) => format!("{} isn't Live ({other} is); nothing to put back.", ws.name),
                    None => format!("{} isn't Live; nothing to put back.", ws.name),
                }}));
            }
            let out = crate::live::set_live(app, shared, &ws.project_id, None)?;
            Ok(json!({ "live": false, "workspace": ws.name, "note": out.note, "output": out.output }))
        }
        None | Some("status") => {
            let now = live_name(project.live_workspace_id.as_deref());
            Ok(json!({ "live": project.live_workspace_id.as_deref() == Some(ws.id.as_str()), "workspace": ws.name, "liveWorkspace": now }))
        }
        Some(other) => bail!("enkidu live on, enkidu live off, or enkidu live (which one is Live), not \"{other}\""),
    }
}

/// Runs one call.
pub fn handle(app: &Events, shared: &Shared, command: &str, body: &str) -> (u16, String) {
    let args: Value = serde_json::from_str(body).unwrap_or_else(|_| json!({}));
    let result = match command {
        "refresh" => {
            for p in shared.db.projects().unwrap_or_default() {
                shared.work.want(&p.id);
            }
            Ok(json!({ "refreshing": true }))
        }
        "doctor" => Ok(json!(super::check_connections(app, shared))),
        "report" => {
            let here = Here::of(&shared.db, &args);
            report(&shared.db, &shared.data_dir, &here, &args, &|| repo_here(shared, &here)).inspect(|_| {
                changed(app);
                // a workflow task whose Claude's turn is already over ends now (one still at work ends at its Stop)
                super::flows::wake();
            })
        }
        "update" => {
            let here = Here::of(&shared.db, &args);
            let mut with = args.clone();
            with["state"] = json!(super::reports::UPDATE);
            report(&shared.db, &shared.data_dir, &here, &with, &|| repo_here(shared, &here)).inspect(|_| changed(app))
        }
        "workflow" => workflow(&shared.db, &args),
        "live" => live(app, shared, &args),
        _ => {
            let here = Here::of(&shared.db, &args);
            answer(&shared.db, command, &args, &|| repo_here(shared, &here)).map(|(v, was_changed)| {
                if matches!(command, "link" | "unlink") {
                    let _ = relink(shared);
                }
                if was_changed {
                    changed(app);
                }
                v
            })
        }
    };
    match result {
        Ok(v) => (200, v.to_string()),
        Err(e) => (400, json!({ "error": crate::redact::redact_urls(&format!("{e:#}")) }).to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::work::links::tests as l;
    use crate::work::model::Ticket;
    use crate::work::store::tests::db;

    const REPO: RepoHere = &|| Some("Org/shop".to_string());

    fn ticket(key: &str) -> Ticket {
        Ticket {
            key: key.into(),
            project_key: key.split('-').next().unwrap_or_default().into(),
            summary: format!("{key} summary"),
            url: format!("https://example.atlassian.net/browse/{key}"),
            status: "In Progress".into(),
            category: "indeterminate".into(),
            assignee: Some("Me Dev".into()),
            assignee_id: Some("me".into()),
            updated_at: "2026-11-08T09:00:00.000+0100".into(),
            comments: 0,
            attachments: 0,
            last_comment_by: None,
            last_comment_at: None,
            description: None,
            described_at: None,
            issue_type: None,
            priority: None,
        }
    }

    fn open_item(id: &str, pr: &str, title: &str) -> Item {
        Item {
            id: id.into(),
            kind: "remark".into(),
            source: "github".into(),
            project_id: "p1".into(),
            pr: Some(pr.into()),
            ticket: None,
            title: title.into(),
            detail: "Use the outlet's separator".into(),
            who: Some("mira-k".into()),
            url: None,
            path: Some("orders/export.py".into()),
            line: Some(48),
            at: "2026-11-08T08:00:00Z".into(),
            state: "open".into(),
            opened_at: now_ms(),
            closed_at: None,
            snoozed_until: None,
            bot: false,
        }
    }

    #[test]
    fn names_a_pr_or_ticket_however_it_is_written() {
        assert_eq!(target("pr", "2414", REPO).unwrap(), "pr:Org/shop#2414");
        assert_eq!(target("pr", "#2414", REPO).unwrap(), "pr:Org/shop#2414");
        assert_eq!(target("pr", "Other/repo#12", REPO).unwrap(), "pr:Other/repo#12");
        assert_eq!(target("pr", "https://github.com/Other/repo/pull/12", REPO).unwrap(), "pr:Other/repo#12");
        assert_eq!(target("ticket", "sh-21533", REPO).unwrap(), "ticket:SH-21533");
        assert!(target("pr", "the export one", REPO).unwrap_err().to_string().contains("isn't a PR"));
        assert!(target("pr", "2414", &|| None).unwrap_err().to_string().contains("give owner/repo#2414"));
        assert!(target("ticket", "SH", REPO).unwrap_err().to_string().contains("isn't a ticket key"));
        assert!(target("branch", "main", REPO).is_err());
    }

    #[test]
    fn status_shows_the_sessions_prs_tickets_and_what_waits() {
        let (_dir, db) = db();
        let now = now_ms();
        db.work_put_prs("p1", &[l::pr(2414, "feature/SH-21533-totals", "SH-21533 Totals"), l::pr(2437, "fix/other", "Other")], now).unwrap();
        let mut t = ticket("SH-21533");
        t.issue_type = Some("Bug".into());
        t.description = Some(json!({"type": "doc", "version": 1, "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Totals are off after a return."}]}]}));
        db.work_put_tickets("p1", &[t], now).unwrap();
        db.work_put_item(&open_item("thread:t1", "Org/shop#2414", "mira-k left a remark")).unwrap();
        db.work_put_item(&open_item("thread:t9", "Org/shop#2437", "someone else's PR")).unwrap();
        crate::work::links::relink(&db, &[l::shop()], now, &|_| None).unwrap(); // the ticket in the branch: SH-21533 ↔ #2414
        let args = json!({ "session": "s1", "workspace": "w1" });
        // nothing linked yet: an empty answer, not an error
        let (v, changed) = answer(&db, "status", &args, REPO).unwrap();
        assert!(!changed);
        assert_eq!((v["prs"].as_array().unwrap().len(), v["items"].as_array().unwrap().len()), (0, 0));

        let (v, changed) = answer(&db, "link", &json!({ "session": "s1", "kind": "pr", "ref": "2414", "how": "claude" }), REPO).unwrap();
        assert!(changed);
        assert_eq!(v, json!({ "linked": "pr:Org/shop#2414", "to": "session:s1" }));
        let (v, _) = answer(&db, "status", &args, REPO).unwrap();
        assert_eq!(v["prs"][0]["number"], 2414);
        assert_eq!(v["tickets"][0]["key"], "SH-21533", "the PR's ticket comes with it");
        assert_eq!(v["tickets"][0]["description"], "Totals are off after a return.", "what it asks for, as text");
        assert_eq!(v["tickets"][0]["type"], "Bug");
        let waiting: Vec<&str> = v["items"].as_array().unwrap().iter().map(|i| i["id"].as_str().unwrap()).collect();
        assert_eq!(waiting, ["thread:t1"], "only what waits on this session's work");
        assert!(v["text"].as_str().unwrap().contains("#2414"));
        let link = db.work_links().unwrap().into_iter().find(|x| x.b == "session:s1").unwrap();
        assert_eq!((link.how.as_str(), link.detail.as_str()), ("claude", "linked by Claude"));
    }

    #[test]
    fn an_unlinked_pr_stays_unlinked_until_linked_by_hand() {
        let (_dir, db) = db();
        let now = now_ms();
        db.work_put_prs("p1", &[l::pr(2414, "feature/x", "X")], now).unwrap();
        db.work_link("pr:Org/shop#2414", "session:s1", "branch", "feature/x", now).unwrap();
        let (v, _) = answer(&db, "unlink", &json!({ "session": "s1", "kind": "pr", "ref": "#2414" }), REPO).unwrap();
        assert_eq!(v["unlinked"], "pr:Org/shop#2414");
        assert!(db.work_links().unwrap().is_empty());
        // the branch rule can't bring it back; you (or Claude) can
        assert!(!db.work_link("pr:Org/shop#2414", "session:s1", "branch", "feature/x", now).unwrap());
        assert!(db.work_links().unwrap().is_empty());
        answer(&db, "link", &json!({ "session": "s1", "kind": "pr", "ref": "2414", "how": "you" }), REPO).unwrap();
        let back: Vec<String> = db.work_links().unwrap().iter().map(|x| format!("{} {}", x.how, x.detail)).collect();
        assert_eq!(back, ["you linked by you"]);
        // without a session or workspace there is nothing to link to
        let e = answer(&db, "link", &json!({ "kind": "pr", "ref": "2414" }), REPO).unwrap_err();
        assert!(e.to_string().contains("--session"));
    }

    #[test]
    fn a_ticket_comes_with_its_prs_and_what_waits_on_them() {
        let (_dir, db) = db();
        let now = now_ms();
        db.work_put_prs("p1", &[l::pr(2414, "feature/SH-21533-totals", "SH-21533 Totals"), l::pr(2437, "fix/other", "Other")], now).unwrap();
        db.work_put_tickets("p1", &[ticket("SH-21533"), ticket("SH-21627")], now).unwrap();
        db.work_put_item(&open_item("thread:t1", "Org/shop#2414", "a remark")).unwrap();
        db.work_put_item(&open_item("thread:t2", "Org/shop#2437", "another")).unwrap();
        crate::work::links::relink(&db, &[l::shop()], now, &|_| None).unwrap(); // SH-21533 is in #2414's branch
        let tickets = answer(&db, "list", &json!({ "what": "tickets" }), REPO).unwrap().0;
        let t = tickets.as_array().unwrap().iter().find(|t| t["key"] == "SH-21533").unwrap();
        assert_eq!((t["prs"][0]["number"].as_i64(), t["waiting"].as_i64()), (Some(2414), Some(1)));
        assert!(t["prs"][0]["summary"].as_str().is_some());
        let keys = |v: Value| v.as_array().unwrap().iter().map(|t| t["key"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        assert_eq!(keys(answer(&db, "list", &json!({ "what": "tickets", "needs_me": true }), REPO).unwrap().0), ["SH-21533"]);
        let items = answer(&db, "items", &json!({ "ticket": "sh-21533" }), REPO).unwrap().0;
        assert_eq!(items.as_array().unwrap().iter().map(|i| i["id"].as_str().unwrap()).collect::<Vec<_>>(), ["thread:t1"], "its PR's items");
    }

    #[test]
    fn lists_items_marks_them_and_notes_on_the_sessions_ticket() {
        let (_dir, db) = db();
        let now = now_ms();
        db.work_put_prs("p1", &[l::pr(2414, "feature/SH-21533-totals", "SH-21533 Totals"), l::pr(2437, "fix/other", "Other")], now).unwrap();
        db.work_put_tickets("p1", &[ticket("SH-21533")], now).unwrap();
        db.work_put_item(&open_item("thread:t1", "Org/shop#2414", "a remark")).unwrap();
        db.work_put_item(&open_item("thread:t2", "Org/shop#2437", "another")).unwrap();
        let ids = |v: &Value| v.as_array().unwrap().iter().map(|i| i["id"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        assert_eq!(ids(&answer(&db, "items", &json!({ "pr": "2414" }), REPO).unwrap().0), ["thread:t1"]);
        assert_eq!(answer(&db, "items", &json!({}), REPO).unwrap().0.as_array().unwrap().len(), 2);
        let prs = answer(&db, "list", &json!({ "what": "prs", "needs_me": true }), REPO).unwrap().0;
        assert_eq!(prs.as_array().unwrap().len(), 2);

        answer(&db, "item", &json!({ "action": "done", "id": "thread:t2" }), REPO).unwrap();
        assert_eq!(ids(&answer(&db, "items", &json!({}), REPO).unwrap().0), ["thread:t1"]);
        let prs = answer(&db, "list", &json!({ "what": "prs", "needs_me": true }), REPO).unwrap().0;
        assert_eq!(prs.as_array().unwrap().iter().map(|p| p["number"].as_i64().unwrap()).collect::<Vec<_>>(), [2414]);
        answer(&db, "item", &json!({ "action": "snoozed", "id": "thread:t1" }), REPO).unwrap();
        let snoozed = db.work_items().unwrap().into_iter().find(|i| i.id == "thread:t1").unwrap();
        assert!(snoozed.snoozed_until.unwrap() > now + 86_000_000, "snoozed for a day");
        assert!(answer(&db, "item", &json!({ "action": "done", "id": "nope" }), REPO).unwrap_err().to_string().contains("gone"));

        // a note goes to the session's ticket, else the ticket of the PR it works on, else that PR; with nothing it
        // says how to name one
        let note = |args: Value| answer(&db, "note", &args, REPO).map(|(v, _)| v["noted"].as_str().unwrap_or_default().to_string());
        assert!(note(json!({ "session": "s1", "text": "QA asked again" })).unwrap_err().to_string().contains("--pr"));
        // only a PR linked: its ticket (the key in its branch)
        crate::work::links::relink(&db, &[l::shop()], now, &|_| None).unwrap();
        db.work_link("pr:Org/shop#2414", "session:s1", "claude", "linked by Claude", now).unwrap();
        assert_eq!(note(json!({ "session": "s1", "text": "QA asked again" })).unwrap(), "SH-21533");
        // two PRs: the one of the workspace's branch, else it asks which
        db.work_link("pr:Org/shop#2437", "session:s1", "claude", "linked by Claude", now).unwrap();
        assert!(note(json!({ "session": "s1", "text": "x" })).unwrap_err().to_string().contains("works on 2 PRs"));
        db.insert_project(&crate::db::Project { id: "p1".into(), name: "shop".into(), path: "/code/shop".into(), live_workspace_id: None, created_at: 1 }).unwrap();
        db.insert_workspace(&crate::db::Workspace {
            id: "w1".into(),
            project_id: "p1".into(),
            name: "other".into(),
            path: "/ws/shop/other".into(),
            branch: "fix/other".into(),
            base: Some("development".into()),
            session_id: "s1".into(),
            transcript_path: None,
            claude_config_dir: None,
            adopted: false,
            created_at: 1,
        })
        .unwrap();
        assert_eq!(note(json!({ "session": "s1", "workspace": "w1", "text": "Rebased" })).unwrap(), "Org/shop#2437", "no ticket: the PR");
        // a ticket linked to the session itself wins
        db.work_link("ticket:SH-21533", "session:s1", "claude", "linked by Claude", now).unwrap();
        assert_eq!(note(json!({ "session": "s1", "workspace": "w1", "text": "QA asked again" })).unwrap(), "SH-21533");
        assert_eq!(note(json!({ "pr": "2437", "text": "Rebased" })).unwrap(), "Org/shop#2437");
        let notes: Vec<String> = db.work_events(10).unwrap().into_iter().filter(|e| e.kind == "note").map(|e| format!("{} {}", e.subject, e.text)).collect();
        assert!(notes.contains(&"SH-21533 QA asked again".to_string()));
        assert!(notes.contains(&"Org/shop#2437 Rebased".to_string()));
        assert!(answer(&db, "teleport", &json!({}), REPO).unwrap_err().to_string().contains("doesn't know"));
        // `/resume` in the workspace's Claude: enkidu there reads the conversation it is in now, not the terminal's first
        db.set_session_id("w1", "s2").unwrap();
        let here = Here::of(&db, &json!({ "session": "s1", "workspace": "w1" }));
        assert_eq!(here.things(), ["session:s2", "workspace:w1"]);
        // outside a workspace, the session given is the one
        assert_eq!(Here::of(&db, &json!({ "session": "s9" })).things(), ["session:s9"]);
    }

    #[test]
    fn a_report_goes_on_the_sessions_ticket_with_its_screenshots_and_its_timeline_says_so() {
        let (dir, db) = db();
        let now = now_ms();
        db.work_put_tickets("p1", &[ticket("SH-21533")], now).unwrap();
        db.work_link("ticket:SH-21533", "session:s1", "you", "linked by you", now).unwrap();
        let shot = dir.path().join("after.png");
        std::fs::write(&shot, b"png").unwrap();
        let here = Here { session: Some("s1".into()), workspace: None };
        let data = dir.path().join("data");
        let v = report(&db, &data, &here, &json!({ "state": "done", "text": "Fixed the totals.\nRan the tests.", "images": [shot.to_string_lossy()] }), REPO).unwrap();
        assert_eq!((v["reported"].as_str(), v["images"].as_i64()), (Some("SH-21533"), Some(1)));
        let r = &db.work_reports(0).unwrap()[0];
        assert_eq!((r.subject.as_str(), r.state.as_str(), r.session_id.as_deref()), ("SH-21533", "done", Some("s1")));
        let timeline: Vec<String> = db.work_events(10).unwrap().into_iter().filter(|e| e.kind == "report").map(|e| e.text).collect();
        assert_eq!(timeline, ["Finished, nothing needs you: Fixed the totals. (1 screenshot)"]);
        // a ticket named wins; a state that isn't one, or a relative path, is refused
        let v = report(&db, &data, &here, &json!({ "state": "review", "text": "Look at the CSV", "ticket": "sh-7" }), REPO).unwrap();
        assert_eq!(v["reported"].as_str(), Some("SH-7"));
        assert!(report(&db, &data, &here, &json!({ "state": "finished", "text": "x" }), REPO).unwrap_err().to_string().contains("done, review or blocked"));
        assert!(report(&db, &data, &here, &json!({ "state": "done", "text": "x", "images": ["shot.png"] }), REPO).unwrap_err().to_string().contains("full path"));
    }

    /// Items 8.5 and 8.7: Claude reports on a task of your own by its number, or by being linked to it; its card's
    /// timeline gets the line.
    #[test]
    fn a_report_and_a_note_go_on_a_task_of_your_own_by_its_number_or_its_link() {
        let (dir, db) = db();
        let now = now_ms();
        db.insert_project(&crate::db::Project { id: "p1".into(), name: "shop".into(), path: "/tmp/shop".into(), live_workspace_id: None, created_at: 1 }).unwrap();
        let task = db.work_save_own_task(None, "p1", "Try the small stack", "", "todo", now).unwrap();
        assert_eq!(task.subject(), "own:1");
        let here = Here { session: Some("s1".into()), workspace: None };
        let data = dir.path().join("data");
        // named: `--task 1` (the number on its card), also written `#1`
        let v = report(&db, &data, &here, &json!({ "state": "done", "text": "It runs on the runner.", "task": "1" }), REPO).unwrap();
        assert_eq!(v["reported"].as_str(), Some("own:1"));
        assert_eq!(db.work_reports(0).unwrap()[0].subject, "own:1");
        assert_eq!(note(&db, &here, &json!({ "text": "Asked Sam", "task": "#1" }), REPO).unwrap()["noted"].as_str(), Some("own:1"));
        // one that isn't there, and what is no number
        assert_eq!(report(&db, &data, &here, &json!({ "state": "done", "text": "x", "task": "7" }), REPO).unwrap_err().to_string(), "There is no task 7 of your own");
        assert!(report(&db, &data, &here, &json!({ "state": "done", "text": "x", "task": "the stack one" }), REPO).unwrap_err().to_string().contains("give its number"));
        // linked (`enkidu link task 1`, or the card sent it there): no need to name it
        assert_eq!(target("task", "1", REPO).unwrap(), "own:1");
        let (linked, _) = answer(&db, "link", &json!({ "session": "s1", "kind": "task", "ref": "1", "how": "you" }), REPO).unwrap();
        assert_eq!(linked["linked"].as_str(), Some("own:1"), "{linked}");
        let v = report(&db, &data, &here, &json!({ "state": "review", "text": "Look at the runner's log." }), REPO).unwrap();
        assert_eq!(v["reported"].as_str(), Some("own:1"));
        let timeline: Vec<(String, String)> = db.work_events(10).unwrap().into_iter().map(|e| (e.subject, e.kind)).collect();
        assert_eq!(timeline.iter().filter(|(subject, _)| subject == "own:1").count(), timeline.len(), "{timeline:?}");
        assert_eq!(timeline.iter().filter(|(_, kind)| kind == "report").count(), 2);
    }

    #[test]
    fn tells_claude_about_its_workflow_and_keeps_its_updates_without_ending_the_task() {
        use crate::work::flows::{self, Flow, FlowTask, Target};
        let (dir, db) = db();
        let now = now_ms();
        let t = |subject: &str, state: &str, ws: Option<&str>, started: Option<i64>| FlowTask {
            subject: subject.into(),
            title: format!("{subject} title"),
            target: Target::Workspace { workspace_id: "w1".into() },
            message: "Fix it".into(),
            edited: false,
            kind: "came_back".into(),
            instruction: "Fix what QA found.".into(),
            minutes: Some(30),
            state: state.into(),
            workspace_id: ws.map(String::from),
            started_at: started,
            ended_at: None,
            note: None,
            busy_at: None,
            nudged_at: None,
        };
        let flow = Flow {
            id: "f1".into(),
            project_id: "p1".into(),
            name: "Night run".into(),
            instruction: String::new(),
            start_at: Some(now),
            minutes: 60,
            live: true,
            state: "running".into(),
            current: Some(1),
            live_before: None,
            tasks: vec![t("ticket:SH-1", "done", Some("w0"), Some(now - 3_600_000)), t("ticket:SH-2", "running", Some("w1"), Some(now - 10 * 60_000))],
            created_at: now,
            updated_at: now,
        };
        db.work_put_flow(&flow).unwrap();
        let v = workflow(&db, &json!({ "workspace": "w1" })).unwrap();
        assert_eq!(v["workflow"]["name"], "Night run");
        assert_eq!((v["workflow"]["task"]["n"].as_i64(), v["workflow"]["task"]["minutes"].as_i64()), (Some(2), Some(30)));
        assert_eq!(v["workflow"]["task"]["minutesLeft"].as_i64(), Some(20), "30 minutes, 10 used");
        assert_eq!(v["workflow"]["tasks"].as_array().unwrap().iter().map(|x| x["yours"].as_bool().unwrap()).collect::<Vec<_>>(), [false, true]);
        assert_eq!(workflow(&db, &json!({ "workspace": "w9" })).unwrap()["workflow"], Value::Null, "no workflow here");
        // an update on the way: kept with its screenshot, on the timeline, and the task still owes its report
        db.work_link("ticket:SH-2", "session:s1", "you", "linked by you", now).unwrap();
        let shot = dir.path().join("half.png");
        std::fs::write(&shot, b"png").unwrap();
        let here = Here { session: Some("s1".into()), workspace: Some("w1".into()) };
        let v = report(&db, &dir.path().join("data"), &here, &json!({ "state": "update", "text": "Found it: the Monday filter.", "images": [shot.to_string_lossy()] }), REPO).unwrap();
        assert_eq!((v["reported"].as_str(), v["state"].as_str(), v["images"].as_i64()), (Some("SH-2"), Some("update"), Some(1)));
        assert!(db.work_events(10).unwrap().iter().any(|e| e.subject == "SH-2" && e.text == "Update: Found it: the Monday filter. (1 screenshot)"));
        assert!(flows::stop_hook(&db, "w1", false, now).is_some(), "an update ends nothing: the report is still asked for");
    }
}
