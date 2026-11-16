//! Claude Code reports its state through hooks. Each claude the app starts gets `--settings`
//! with hooks that call this small local HTTP server, which turns them into status dots.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use crate::events::Events;
use tiny_http::{Response, Server};

use crate::state::Shared;

#[derive(Clone, Debug)]
pub struct HookServer {
    pub port: u16,
    pub token: String,
}

pub fn start(app: Events, shared: Arc<Shared>) -> Result<HookServer> {
    let server = Server::http("127.0.0.1:0").map_err(|e| anyhow!("Can't start the hook server: {e}"))?;
    let port = server
        .server_addr()
        .to_ip()
        .map(|a| a.port())
        .ok_or_else(|| anyhow!("The hook server has no port"))?;
    let token = uuid::Uuid::new_v4().simple().to_string();
    let expected = token.clone();
    // where `enkidu` finds the app when it runs outside a terminal Enkidu started (readable by you only)
    write_api_file(&shared.data_dir, port, &token);
    std::thread::Builder::new().name("hook-server".into()).spawn(move || {
        for mut request in server.incoming_requests() {
            let mut body = String::new();
            let _ = request.as_reader().take(1_000_000).read_to_string(&mut body);
            // `enkidu`'s calls run on their own thread: a slow one (enkidu doctor reads GitHub and Jira) never holds up a hook
            if let Some(rest) = request.url().strip_prefix("/api/") {
                let (given, command) = rest.split_once('/').unwrap_or((rest, ""));
                if given != expected {
                    let _ = request.respond(Response::from_string("").with_status_code(403));
                    continue;
                }
                let (app, shared, command) = (app.clone(), shared.clone(), command.to_string());
                std::thread::spawn(move || {
                    let (code, text) = crate::work::api::handle(&app, &shared, &command, &body);
                    let json = tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).expect("a valid header");
                    let _ = request.respond(Response::from_string(text).with_status_code(code).with_header(json));
                });
                continue;
            }
            let (code, text) = handle(&app, &shared, &expected, request.url(), &body);
            let _ = request.respond(Response::from_string(text).with_status_code(code));
        }
    })?;
    Ok(HookServer { port, token })
}

/// `api.json` in the app's data folder: the port and token `enkidu` needs. Only you can read it.
fn write_api_file(data_dir: &Path, port: u16, token: &str) {
    use std::os::unix::fs::OpenOptionsExt;
    let path = data_dir.join("api.json");
    let _ = std::fs::remove_file(&path);
    let file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path);
    if let Ok(mut f) = file {
        use std::io::Write;
        let _ = f.write_all(json!({ "url": format!("http://127.0.0.1:{port}"), "token": token, "pid": std::process::id() }).to_string().as_bytes());
    }
}

/// When each workspace's Claude last ended its turn (its Stop or StopFailure hook), until its next prompt.
static TURN_ENDED: Mutex<BTreeMap<String, Instant>> = Mutex::new(BTreeMap::new());
/// How long after a turn ended a background hook of that turn may still come in (a shell and curl take ~20 ms; the
/// model's last words before the Stop take far longer, so this only covers a Mac too busy to start them).
const LATE: Duration = Duration::from_millis(1500);

fn note_turn_ended(workspace_id: &str) {
    TURN_ENDED.lock().unwrap_or_else(|e| e.into_inner()).insert(workspace_id.into(), Instant::now());
}

fn note_prompt(workspace_id: &str) {
    TURN_ENDED.lock().unwrap_or_else(|e| e.into_inner()).remove(workspace_id);
}

/// The workspace's Claude ended its turn a moment ago (and got no prompt since).
fn turn_just_ended(workspace_id: &str) -> bool {
    TURN_ENDED.lock().unwrap_or_else(|e| e.into_inner()).get(workspace_id).is_some_and(|t| t.elapsed() < LATE)
}

/// Handles `/hook/<token>/<workspace>/<status>`, `/prompt/<token>/<workspace>`, `/session/<token>/<workspace>`,
/// `/stop/…`, `/failure/…` and `/tool/…`. A prompt marks the workspace as working and answers with the Live message,
/// which Claude Code adds to the prompt's context; a starting session is told about its own PRs and tickets the same
/// way. A turn's end (Stop, or StopFailure when an API error cut it short) wakes the workflow runner, so a workflow
/// moves on in the same second. Every hook also feeds the Work index (which session worked on which branch, PR and
/// ticket).
fn handle(app: &Events, shared: &Arc<Shared>, token: &str, url: &str, body: &str) -> (u16, String) {
    let parts: Vec<&str> = url.trim_start_matches('/').split('/').collect();
    let (kind, given, workspace_id, mut status) = match parts.as_slice() {
        ["hook", t, ws, status] => ("hook", *t, *ws, *status),
        ["prompt", t, ws] => ("prompt", *t, *ws, "working"),
        ["session", t, ws] => ("session", *t, *ws, ""),
        ["stop", t, ws] => ("stop", *t, *ws, "done"),
        // an API error ended the turn (the usage limit, a login that ran out): Claude waits for you
        ["failure", t, ws] => ("failure", *t, *ws, "needs_you"),
        ["tool", t, ws] => ("tool", *t, *ws, ""),
        _ => return (404, String::new()),
    };
    if given != token {
        return (403, String::new());
    }
    // a shell command Claude is about to run: refused only when it makes or sends commits during a workflow's task
    // here (the work stays uncommitted, for the diff in the morning); nothing else is touched for it
    if kind == "tool" {
        let command = serde_json::from_str::<Value>(body).ok().and_then(|v| v.pointer("/tool_input/command").and_then(Value::as_str).map(String::from));
        return match command.and_then(|c| crate::work::flows::tool_guard(&shared.db, workspace_id, &c)) {
            Some(reason) => (
                200,
                json!({ "hookSpecificOutput": { "hookEventName": "PreToolUse", "permissionDecision": "deny", "permissionDecisionReason": reason } }).to_string(),
            ),
            None => (200, String::new()),
        };
    }
    let Ok(Some(mut workspace)) = shared.db.workspace(workspace_id) else {
        return (404, String::new());
    };
    let payload = serde_json::from_str::<Value>(body).ok();
    let field = |k: &str| payload.as_ref().and_then(|v| v.get(k)).and_then(|p| p.as_str()).filter(|s| !s.is_empty()).map(String::from);
    // Claude stopped in the middle of a workflow's task without reporting how it went: it is asked to, once, and goes
    // on (Claude Code's own `stop_hook_active` says it already went on because of a stop hook: never twice)
    let ask = if kind == "stop" {
        let active = payload.as_ref().and_then(|v| v.get("stop_hook_active")).and_then(Value::as_bool).unwrap_or(false);
        crate::work::flows::stop_hook(&shared.db, workspace_id, active, crate::db::now_ms())
    } else {
        None
    };
    if ask.is_some() {
        status = "working";
    }
    // `/resume` (or `/clear`) in the workspace's Claude: that conversation is the workspace's from now on, so a restart
    // brings it back and `enkidu` there reads it
    if kind == "session" {
        if let Some(session) = field("session_id").filter(|s| *s != workspace.session_id) {
            let _ = shared.db.set_session_id(workspace_id, &session);
            workspace.session_id = session;
        }
    }
    // only the conversation the workspace is in records where it is saved: the last hooks of a Claude that was just
    // closed for another conversation (New Claude on a card) arrive late and must not take the workspace back
    // (a SessionStart above already made a resumed or cleared conversation the workspace's)
    let current = field("session_id").is_none_or(|s| s == workspace.session_id);
    if let Some(transcript) = field("transcript_path").filter(|_| current) {
        if workspace.transcript_path.as_deref() != Some(transcript.as_str()) {
            let _ = shared.db.set_transcript_path(workspace_id, &transcript);
        }
    }
    // the lamp hooks run in the background (Claude doesn't wait for them): one from a tool call of a turn that just
    // ended may come in after its Stop, and never turns a finished Claude's lamp back to working
    if kind == "prompt" {
        note_prompt(workspace_id);
    } else if kind == "hook" && status == "working" && turn_just_ended(workspace_id) {
        status = "";
    }
    // the same for the lamp: a closing Claude's "idle" never lands on the one that took its place
    if !status.is_empty() && current {
        shared.set_status(app, workspace_id, status);
    }
    // the turn is over (not when the Stop hook asked for a report: then Claude goes on): a workflow's task can end and
    // the next one start now, and an API error is the task's end, with Claude Code's words for it
    if current && (kind == "failure" || (kind == "stop" && ask.is_none())) {
        note_turn_ended(workspace_id);
        if kind == "failure" {
            let error = field("error").unwrap_or_default();
            let said = field("last_assistant_message").unwrap_or_default();
            crate::work::flows::stop_failure(workspace_id, &error, &said, crate::db::now_ms());
        } else {
            crate::work::flows::wake();
        }
    }
    let context = crate::work::on_hook(app, shared, &workspace, body);
    if let Some(reason) = ask {
        crate::work::changed(app);
        return (200, json!({ "decision": "block", "reason": reason }).to_string());
    }
    if kind == "session" {
        // a Claude started with a message gets it typed into its prompt now that it is up
        crate::pty::type_draft_when_ready(shared.clone(), workspace_id);
        return (200, context);
    }
    if kind == "prompt" {
        let live = shared
            .db
            .project(&workspace.project_id)
            .ok()
            .flatten()
            .is_some_and(|p| p.live_workspace_id.as_deref() == Some(workspace_id));
        if live {
            let message = shared.project_settings(&workspace.project_id).map(|s| s.live.message).unwrap_or_default();
            return (200, message);
        }
    }
    (200, String::new())
}

fn curl() -> &'static str {
    if Path::new("/usr/bin/curl").exists() {
        "/usr/bin/curl"
    } else {
        "curl"
    }
}

/// The settings JSON passed to claude with `--settings`.
pub fn claude_settings(server: &HookServer, workspace_id: &str) -> Value {
    let base = format!("http://127.0.0.1:{}", server.port);
    let report = |status: &str| {
        json!([{ "type": "command", "timeout": 5, "command": format!(
            "{} -s -m 2 --data-binary @- {base}/hook/{}/{workspace_id}/{status} >/dev/null 2>&1 || true",
            curl(), server.token
        ) }])
    };
    // a hook that only moves the lamp runs in the background: Claude doesn't wait for it (a shell and curl, ~20 ms on
    // every tool call). Not SessionEnd: Claude may be gone before a background one runs.
    let behind = |status: &str| {
        let mut hooks = report(status);
        hooks[0]["async"] = json!(true);
        hooks
    };
    // these keep stdout: what they answer becomes context for Claude (or its decision)
    let answered = |route: &str| {
        json!([{ "type": "command", "timeout": 5, "command": format!(
            "{} -s -m 2 --data-binary @- {base}/{route}/{}/{workspace_id} 2>/dev/null || true",
            curl(), server.token
        ) }])
    };
    json!({
        // `enkidu` reads and corrects the Work index (see CLI_PROMPT); Claude may run it without asking
        "permissions": { "allow": ["Bash(enkidu:*)", "Bash(enkidu)"] },
        "hooks": {
            "SessionStart": [{ "hooks": answered("session") }],
            "UserPromptSubmit": [{ "hooks": answered("prompt") }],
            "PreToolUse": [
                { "matcher": "AskUserQuestion|ExitPlanMode", "hooks": behind("needs_you") },
                // its answer is empty, except a refusal of a commit or push during a workflow's task (flows::tool_guard)
                { "matcher": "Bash", "hooks": answered("tool") }
            ],
            "PermissionRequest": [{ "hooks": behind("needs_you") }],
            "PostToolUse": [{ "hooks": behind("working") }],
            "Notification": [{ "matcher": "elicitation_dialog|elicitation_url_dialog", "hooks": behind("needs_you") }],
            // its answer is empty, except in a workflow's task that has no report yet: then Claude is asked for one
            "Stop": [{ "hooks": answered("stop") }],
            // instead of Stop when an API error ended the turn (the usage limit, a login that ran out): Claude Code
            // ignores what it answers; the lamp says Claude needs you, and a workflow's task ends on it
            "StopFailure": [{ "hooks": [{ "type": "command", "timeout": 5, "command": format!(
                "{} -s -m 2 --data-binary @- {base}/failure/{}/{workspace_id} >/dev/null 2>&1 || true",
                curl(), server.token
            ) }] }],
            "SessionEnd": [{ "hooks": report("idle") }]
        }
    })
}

pub fn write_claude_settings(data_dir: &Path, server: &HookServer, workspace_id: &str) -> Result<PathBuf> {
    let dir = data_dir.join("hooks");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{workspace_id}.json"));
    std::fs::write(&path, serde_json::to_string_pretty(&claude_settings(server, workspace_id))?)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_have_every_status_hook_and_no_output_for_decisions() {
        let server = HookServer { port: 4321, token: "tok".into() };
        let v = claude_settings(&server, "ws1");
        let hooks = &v["hooks"];
        for event in ["SessionStart", "UserPromptSubmit", "PreToolUse", "PermissionRequest", "PostToolUse", "Notification", "Stop", "StopFailure", "SessionEnd"]
        {
            assert!(hooks[event].is_array(), "{event} missing");
        }
        // the hooks that only move the lamp run in the background (Claude doesn't wait); the ones whose answer counts,
        // a turn's end and the session's end don't
        let background = |event: &str, i: usize| hooks[event][i]["hooks"][0]["async"] == json!(true);
        assert!(background("PostToolUse", 0) && background("PermissionRequest", 0) && background("Notification", 0) && background("PreToolUse", 0));
        for (event, i) in [("PreToolUse", 1), ("SessionStart", 0), ("UserPromptSubmit", 0), ("Stop", 0), ("StopFailure", 0), ("SessionEnd", 0)] {
            assert!(!background(event, i), "{event} must be waited for");
        }
        let failure = hooks["StopFailure"][0]["hooks"][0]["command"].as_str().unwrap();
        assert!(failure.contains("http://127.0.0.1:4321/failure/tok/ws1") && failure.ends_with("|| true"));
        // Stop keeps its answer: empty, except in a workflow's task with no report yet (flows::stop_hook)
        let stop = hooks["Stop"][0]["hooks"][0]["command"].as_str().unwrap();
        assert!(stop.contains("http://127.0.0.1:4321/stop/tok/ws1") && !stop.contains(" >/dev/null") && stop.ends_with("|| true"));
        let start = hooks["PreToolUse"][0]["hooks"][0]["command"].as_str().unwrap();
        assert!(start.contains(">/dev/null"), "the other hooks that could make decisions print nothing");
        // Bash keeps its answer too: empty, except a refusal of a commit or push during a workflow's task
        assert_eq!(hooks["PreToolUse"][1]["matcher"], "Bash");
        let bash = hooks["PreToolUse"][1]["hooks"][0]["command"].as_str().unwrap();
        assert!(bash.contains("http://127.0.0.1:4321/tool/tok/ws1") && !bash.contains(" >/dev/null"));
        let perm = hooks["PermissionRequest"][0]["hooks"][0]["command"].as_str().unwrap();
        assert!(perm.ends_with("|| true") && perm.contains(">/dev/null"));
        let prompt = hooks["UserPromptSubmit"][0]["hooks"][0]["command"].as_str().unwrap();
        // stdout is kept (only stderr is dropped), because its output becomes context
        assert!(prompt.contains("/prompt/tok/ws1") && !prompt.contains(" >/dev/null"));
        assert_eq!(hooks["Notification"][0]["matcher"], "elicitation_dialog|elicitation_url_dialog");
        let start = hooks["SessionStart"][0]["hooks"][0]["command"].as_str().unwrap();
        assert!(start.contains("/session/tok/ws1") && !start.contains(" >/dev/null"), "a starting session hears about its PRs");
    }

    #[test]
    fn a_late_background_hook_of_a_turn_that_ended_is_told_apart_until_the_next_prompt() {
        assert!(!turn_just_ended("ws-late"));
        note_turn_ended("ws-late");
        assert!(turn_just_ended("ws-late"), "a moment after the Stop: a 'working' then is the ended turn's");
        assert!(!turn_just_ended("ws-other"), "only that workspace");
        note_prompt("ws-late");
        assert!(!turn_just_ended("ws-late"), "a new prompt starts a new turn: its tools count again");
    }
}
