//! What Claude's hooks tell the Work index, live: the session and its account folder (SessionStart), the branch and
//! the tickets or PRs you name (UserPromptSubmit), the PR it creates, the commits it makes and the branches it pushes
//! (PostToolUse), when to read GitHub and Jira again (a push at once, and the end of a turn that used them: a push, gh,
//! a Jira tool or twg; a turn that only read and edited code reads nothing), and when the session ends (SessionEnd).

use anyhow::Result;
use regex::Regex;
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::{LazyLock, Mutex};

use super::parse::{self, GitOps};
use super::store::SessionSeen;
use super::transcripts::string_after;
use crate::db::{Db, Workspace};

#[derive(Debug, Default, PartialEq)]
pub struct Payload {
    pub event: String,
    pub session_id: String,
    pub transcript_path: Option<String>,
    pub cwd: Option<String>,
    pub prompt: Option<String>,
    pub ops: GitOps,
    /// The tool call used GitHub or Jira (a push, gh, a Jira or GitHub tool, twg): the turn's end reads them again
    pub touched: bool,
}

/// A command that runs gh, twg, jira or acli, or pushes, wherever it stands in a command line (`cd x && gh pr view`),
/// but not one that only names them (`grep -r jira src`).
static USES_A_SOURCE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|[;&|(`]\s*)(?:(?:gh|twg|jira|acli)\s|git\b[^;&|\n]*\bpush\b)").expect("a valid pattern"));

/// Whether a tool call used GitHub or Jira, from its name and (for Bash) its command, without parsing it whole.
fn touches_a_source(raw: &[u8]) -> bool {
    let tool = string_after(raw, b"\"tool_name\":\"").unwrap_or_default();
    if tool.starts_with("mcp__") {
        let tool = tool.to_ascii_lowercase();
        return ["atlassian", "jira", "github"].iter().any(|n| tool.contains(n));
    }
    // the first "command" is the tool's input (Claude Code sends tool_input before tool_response)
    tool == "Bash" && string_after(raw, b"\"command\":\"").is_some_and(|c| USES_A_SOURCE.is_match(c.trim_start()))
}

/// Reads a hook's JSON. A tool call that can't matter (most of them: reads, edits, searches) is not parsed whole.
pub fn read_payload(body: &str) -> Option<Payload> {
    let raw = body.as_bytes();
    let event = string_after(raw, b"\"hook_event_name\":\"")?;
    let session_id = string_after(raw, b"\"session_id\":\"")?;
    let touched = event == "PostToolUse" && touches_a_source(raw);
    if event == "PostToolUse"
        && !["gitOperation", "gh pr create", "git commit", "git push"].iter().any(|n| body.contains(n))
    {
        return Some(Payload { event, session_id, touched, ..Default::default() });
    }
    let v: Value = serde_json::from_str(body).ok()?;
    let text = |k: &str| v.get(k).and_then(Value::as_str).map(String::from);
    let mut p = Payload { event, session_id, transcript_path: text("transcript_path"), cwd: text("cwd"), prompt: text("prompt"), touched, ..Default::default() };
    if p.event == "PostToolUse" && v.get("tool_name").and_then(Value::as_str) == Some("Bash") {
        let response = v.get("tool_response").cloned().unwrap_or(Value::Null);
        let command = v.pointer("/tool_input/command").and_then(Value::as_str).unwrap_or("");
        p.ops = match response.get("gitOperation") {
            Some(op) => parse::confirmed(parse::from_git_operation(op), command),
            None => {
                let out = |k: &str| response.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                parse::from_command_output(command, &out("stdout"), &out("stderr"))
            }
        };
    }
    Some(p)
}

/// The Claude folder (`~/.claude-work`) a session's transcript lives in: `<folder>/projects/<project>/<id>.jsonl`.
pub fn config_dir_of(transcript: &str) -> Option<String> {
    let p = Path::new(transcript);
    let projects = p.parent()?.parent()?;
    (projects.file_name()? == "projects").then(|| projects.parent().map(|c| c.to_string_lossy().to_string()))?
}

#[derive(Debug, Default, PartialEq)]
pub struct Outcome {
    /// Read GitHub and Jira again now (a push, or the end of a turn that used them)
    pub refresh: bool,
}

/// The sessions whose turn used GitHub or Jira since their last Stop.
static TOUCHED: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

fn mark_touched(session: &str) {
    TOUCHED.lock().unwrap_or_else(|e| e.into_inner()).insert(session.into());
}

/// The turn is over: whether it used GitHub or Jira (and the next turn starts with a clean slate).
fn turn_used_a_source(session: &str) -> bool {
    TOUCHED.lock().unwrap_or_else(|e| e.into_inner()).remove(session)
}

/// Saves what a hook said. `branch_of` reads a folder's current branch (git), so tests can fake it.
pub fn apply(db: &Db, ws: Option<&Workspace>, p: &Payload, now: i64, branch_of: &dyn Fn(&str) -> Option<String>) -> Result<Outcome> {
    let mut out = Outcome::default();
    if p.session_id.is_empty() {
        return Ok(out);
    }
    let id = p.session_id.as_str();
    db.work_session_seen(&SessionSeen {
        id: id.into(),
        config_dir: p.transcript_path.as_deref().and_then(config_dir_of).unwrap_or_default(),
        cwd: p.cwd.clone().or_else(|| ws.map(|w| w.path.clone())).unwrap_or_default(),
        title: ws.map(|w| w.name.clone()),
        workspace_id: ws.map(|w| w.id.clone()),
        first_at: now,
        last_at: now,
        source: "hook".into(),
    })?;
    match p.event.as_str() {
        "SessionStart" | "UserPromptSubmit" => {
            if let Some(b) = p.cwd.as_deref().and_then(branch_of) {
                db.work_session_branch(id, &b, "branch", "worked", now)?;
            }
            if let Some(prompt) = &p.prompt {
                let typed = parse::typed_text(&Value::String(prompt.clone()));
                for key in parse::ticket_keys(&typed) {
                    db.work_mention(id, &format!("ticket:{key}"), "hook:mention", "named", now)?;
                }
                for r in parse::pr_refs(&typed) {
                    db.work_mention(id, &format!("pr:{r}"), "hook:mention", "named", now)?;
                }
            }
        }
        "PostToolUse" => {
            for (target, action) in &p.ops.prs {
                let how = if action == "created" { "hook:pr-create" } else { "hook:pr" };
                db.work_mention(id, &format!("pr:{target}"), how, action, now)?;
            }
            for b in &p.ops.pushed {
                db.work_session_branch(id, b, "branch", "pushed", now)?;
                out.refresh = true;
            }
            for (sha, branch) in &p.ops.commits {
                db.work_commit(sha, id, branch.as_deref(), "hook", now)?;
                if let Some(b) = branch {
                    db.work_session_branch(id, b, "branch", "committed", now)?;
                }
            }
            // read again when the turn ends too: what GitHub and Jira did about it (a ticket a PR moves) comes later
            if p.touched || !p.ops.pushed.is_empty() || !p.ops.prs.is_empty() {
                mark_touched(id);
            }
        }
        // a turn that only read and edited code changed nothing there: the read every few minutes is enough
        "Stop" | "StopFailure" => out.refresh = turn_used_a_source(id),
        "SessionEnd" => db.work_session_ended(id, now)?,
        _ => {}
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::now_ms;
    use crate::work::links::{relink, tests as l};
    use crate::work::store::tests::db;
    use serde_json::json;

    /// Hook input as Claude Code sends it (the common fields, then the event's own).
    fn hook(event: &str, extra: Value) -> String {
        let mut v = json!({
            "session_id": "s1",
            "transcript_path": "/h/.claude-work/projects/-ws-shop-totals/s1.jsonl",
            "cwd": "/ws/shop/totals",
            "permission_mode": "default",
            "hook_event_name": event
        });
        for (k, x) in extra.as_object().unwrap() {
            v[k] = x.clone();
        }
        v.to_string()
    }

    fn bash(command: &str, stdout: &str, op: Option<Value>) -> String {
        let mut response = json!({"stdout": stdout, "stderr": "", "interrupted": false, "isImage": false});
        if let Some(op) = op {
            response["gitOperation"] = op;
        }
        hook("PostToolUse", json!({"tool_name": "Bash", "tool_input": {"command": command, "description": "x"}, "tool_response": response, "tool_use_id": "t1"}))
    }

    fn on_branch(_: &str) -> Option<String> {
        Some("feature/SH-21533-totals".into())
    }

    #[test]
    fn reads_the_account_folder_from_the_transcript_path() {
        assert_eq!(config_dir_of("/h/.claude-work/projects/-ws/s1.jsonl").as_deref(), Some("/h/.claude-work"));
        assert_eq!(config_dir_of("/somewhere/else/s1.jsonl"), None);
    }

    #[test]
    fn an_ordinary_tool_call_is_not_parsed_whole() {
        let p = read_payload(&hook("PostToolUse", json!({"tool_name": "Read", "tool_response": {"file": "x".repeat(5000)}}))).unwrap();
        assert_eq!((p.event.as_str(), p.session_id.as_str(), p.cwd.as_deref()), ("PostToolUse", "s1", None));
        assert!(read_payload("not json").is_none());
    }

    #[test]
    fn gh_pr_create_in_a_session_links_the_pr() {
        let (_d, db) = db();
        db.work_put_prs("p1", &[l::pr(2414, "hotfix/round-totals", "Round the totals")], now_ms()).unwrap();
        // with Claude Code's gitOperation record
        let p = read_payload(&bash("gh pr create --base master", "https://github.com/Org/shop/pull/2414\n",
            Some(json!({"pr": {"number": 2414, "url": "https://github.com/Org/shop/pull/2414", "action": "created"}})))).unwrap();
        assert_eq!(p.ops.prs, [("Org/shop#2414".to_string(), "created".to_string())]);
        apply(&db, None, &p, 10, &on_branch).unwrap();
        // and without it, from what gh printed
        let p = read_payload(&bash("gh pr create --fill", "https://github.com/Org/shop/pull/2414\n", None)).unwrap();
        assert_eq!(p.ops.prs.len(), 1);
        // a search that quotes the words next to a PR link is not a PR made
        let p = read_payload(&bash("rg 'gh pr create' notes", "https://github.com/Org/shop/pull/1\n",
            Some(json!({"pr": {"number": 1, "url": "https://github.com/Org/shop/pull/1", "action": "created"}})))).unwrap();
        assert!(p.ops.is_empty());
        relink(&db, &[l::shop()], now_ms(), &|_| None).unwrap();
        let links = db.work_links().unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!((links[0].a.as_str(), links[0].b.as_str(), links[0].how.as_str(), links[0].detail.as_str()),
            ("pr:Org/shop#2414", "session:s1", "hook:pr-create", "created it"));
        let s = &db.work_sessions().unwrap()[0];
        assert_eq!((s.config_dir.as_str(), s.source.as_str()), ("/h/.claude-work", "hook"));
    }

    #[test]
    fn prompts_record_the_branch_and_what_you_named_and_a_push_asks_for_a_read() {
        let (_d, db) = db();
        db.work_put_prs("p1", &[l::pr(2395, "feature/SH-21533-totals", "SH-21533 Totals"), l::pr(5000, "feature/SH-3-b", "B")], now_ms()).unwrap();
        let p = read_payload(&hook("UserPromptSubmit", json!({"prompt": "QA on SH-21533 says the total is off, also look at https://github.com/Org/shop/pull/5000"}))).unwrap();
        let out = apply(&db, None, &p, 10, &on_branch).unwrap();
        assert!(!out.refresh);
        let pushed = read_payload(&bash("git push", "", Some(json!({"push": {"branch": "feature/SH-21533-totals"}})))).unwrap();
        assert!(apply(&db, None, &pushed, 20, &on_branch).unwrap().refresh, "a push reads GitHub again at once");
        assert!(apply(&db, None, &read_payload(&hook("Stop", json!({}))).unwrap(), 30, &on_branch).unwrap().refresh, "and again when the turn that pushed ends");
        relink(&db, &[l::shop()], now_ms(), &|_| None).unwrap();
        let mut got: Vec<String> = db.work_links().unwrap().iter().map(|l| format!("{} {} {}", l.a, l.b, l.how)).collect();
        got.sort();
        assert_eq!(got, [
            "pr:Org/shop#2395 session:s1 branch",
            "pr:Org/shop#5000 session:s1 hook:mention",
            "ticket:SH-21533 pr:Org/shop#2395 ticket-key",
            "ticket:SH-21533 session:s1 hook:mention",
            "ticket:SH-3 pr:Org/shop#5000 ticket-key",
        ]);
        apply(&db, None, &read_payload(&hook("SessionEnd", json!({"reason": "exit"}))).unwrap(), 40, &on_branch).unwrap();
        let ended: i64 = db.conn().query_row("SELECT ended FROM work_sessions WHERE id = 's1'", [], |r| r.get(0)).unwrap();
        assert_eq!(ended, 1);
    }

    #[test]
    fn a_turn_reads_github_and_jira_again_only_when_it_used_them() {
        let (_d, db) = db();
        // each case its own session: the hooks of other tests run at the same time
        let turn = |session: &str, calls: Vec<Value>| {
            for call in calls {
                let mut v: Value = serde_json::from_str(&hook("PostToolUse", call)).unwrap();
                v["session_id"] = json!(session);
                apply(&db, None, &read_payload(&v.to_string()).unwrap(), 10, &on_branch).unwrap();
            }
            apply(&db, None, &read_payload(&hook("Stop", json!({"session_id": session}))).unwrap(), 20, &on_branch).unwrap().refresh
        };
        let bash = |command: &str| json!({"tool_name": "Bash", "tool_input": {"command": command}, "tool_response": {"stdout": "", "stderr": ""}});
        let read = json!({"tool_name": "Read", "tool_input": {"file_path": "/ws/a.py"}, "tool_response": {"file": "jira gh git push"}});
        assert!(!turn("s-code", vec![read.clone(), bash("pytest -q"), bash("grep -rn jira src"), bash("git status")]), "only code: nothing to read");
        assert!(turn("s-gh", vec![read.clone(), bash("cd /ws/shop && gh pr view 12 --json state")]));
        assert!(turn("s-twg", vec![bash("twg jira workitem get SH-21533")]));
        assert!(turn("s-push", vec![bash("git -C /ws/shop push origin HEAD")]));
        assert!(turn("s-mcp", vec![json!({"tool_name": "mcp__claude_ai_Atlassian__transitionJiraIssue", "tool_input": {"issueIdOrKey": "SH-1"}, "tool_response": {}})]));
        assert!(!turn("s-mcp", vec![]), "the next turn starts with a clean slate");
        assert!(!turn("s-browser", vec![json!({"tool_name": "mcp__claude-in-chrome__navigate", "tool_input": {"url": "https://x.atlassian.net"}, "tool_response": {}})]));
        // a turn an API error cut short reads again too, when it used them
        let mut v: Value = serde_json::from_str(&hook("PostToolUse", bash("gh pr create --fill"))).unwrap();
        v["session_id"] = json!("s-cut");
        apply(&db, None, &read_payload(&v.to_string()).unwrap(), 10, &on_branch).unwrap();
        let cut = read_payload(&hook("StopFailure", json!({"session_id": "s-cut", "error": "rate_limit"}))).unwrap();
        assert!(apply(&db, None, &cut, 20, &on_branch).unwrap().refresh);
    }

    #[test]
    fn a_commit_links_the_pr_that_has_it() {
        let (_d, db) = db();
        let mut pr = l::pr(7, "feature/x", "X");
        pr.commits = vec!["3f2c1ab9d0e1f2a3b4c5d6e7f8091a2b3c4d5e6f".into()];
        db.work_put_prs("p1", &[pr], now_ms()).unwrap();
        let p = read_payload(&bash("git commit -m x", "[detached 3f2c1ab] x\n", None)).unwrap();
        assert_eq!(p.ops.commits, [("3f2c1ab".to_string(), None)], "no branch from a detached HEAD");
        apply(&db, None, &p, 10, &|_| None).unwrap();
        relink(&db, &[l::shop()], now_ms(), &|_| None).unwrap();
        let links = db.work_links().unwrap();
        assert_eq!((links[0].how.as_str(), links[0].detail.as_str()), ("hook:commit", "made a commit in it"));
    }
}
