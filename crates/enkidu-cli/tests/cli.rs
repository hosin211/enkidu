//! The `enkidu` program itself: run as Claude (or you) would, against a stand-in for the app's local API.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::process::{Command, Output, Stdio};
use std::thread;

const CLI: &str = env!("CARGO_BIN_EXE_enkidu");

/// What the stand-in app saw: the request line and the JSON body.
struct Seen {
    path: String,
    body: Value,
}

/// Answers one call with `status` and `reply`, as the app would; returns its address and what it saw.
fn app_answering(status: &'static str, reply: String, chunked: bool) -> (String, thread::JoinHandle<Seen>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
        let mut length = 0;
        loop {
            let mut h = String::new();
            reader.read_line(&mut h).unwrap();
            if h == "\r\n" || h.is_empty() {
                break;
            }
            if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
                length = v.trim().parse().unwrap();
            }
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        let mut out = stream;
        let response = if chunked {
            let half = reply.len() / 2;
            format!(
                "HTTP/1.1 {status}\r\nTransfer-Encoding: chunked\r\nContent-Type: application/json\r\n\r\n{:x}\r\n{}\r\n{:x}\r\n{}\r\n0\r\n\r\n",
                half,
                &reply[..half],
                reply.len() - half,
                &reply[half..]
            )
        } else {
            format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/json\r\n\r\n{reply}",
                reply.len()
            )
        };
        out.write_all(response.as_bytes()).unwrap();
        Seen { path, body: serde_json::from_slice(&body).unwrap_or(Value::Null) }
    });
    (addr, handle)
}

/// Runs enkidu as a terminal Enkidu started would (its API, token, session and workspace in the environment).
fn cli(api: &str, args: &[&str], claude: bool) -> Output {
    let data = tempfile::tempdir().unwrap();
    let mut cmd = Command::new(CLI);
    cmd.args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", data.path())
        .env("ENKIDU_DATA_DIR", data.path())
        .env("ENKIDU_API", api)
        .env("ENKIDU_TOKEN", "t0ken")
        .env("ENKIDU_SESSION", "s1")
        .env("ENKIDU_WORKSPACE", "w1");
    if claude {
        cmd.env("CLAUDECODE", "1");
    }
    cmd.output().unwrap()
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).to_string()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).to_string()
}

#[test]
fn status_asks_the_app_about_this_session_and_prints_what_waits() {
    let reply = json!({
        "text": "This session's work in Enkidu: PR #2414 (feature/SH-21533-totals into development, open).",
        "items": [{ "id": "thread:t1", "title": "mira-k left a review remark", "path": "orders/export.py", "line": 48, "bot": false }],
    });
    let (api, seen) = app_answering("200 OK", reply.to_string(), false);
    let o = cli(&api, &["status"], true);
    assert!(o.status.success(), "{}", err(&o));
    let seen = seen.join().unwrap();
    assert_eq!(seen.path, "/api/t0ken/status");
    assert_eq!(seen.body, json!({ "session": "s1", "workspace": "w1" }));
    assert!(out(&o).contains("PR #2414"));
    assert!(out(&o).contains("Waiting on you:\n  thread:t1  mira-k left a review remark (orders/export.py:48)"));
}

#[test]
fn json_answers_are_printed_whole_even_when_sent_in_chunks() {
    let reply = json!([{ "id": "thread:t1", "title": "a remark", "kind": "remark" }]).to_string();
    let (api, seen) = app_answering("200 OK", reply.clone(), true);
    let o = cli(&api, &["items", "--pr", "2414", "--json"], false);
    assert!(o.status.success(), "{}", err(&o));
    assert_eq!(serde_json::from_str::<Value>(&out(&o)).unwrap(), serde_json::from_str::<Value>(&reply).unwrap());
    assert_eq!(seen.join().unwrap().body["pr"], "2414");
}

#[test]
fn a_link_says_whether_claude_or_you_made_it() {
    let (api, seen) =
        app_answering("200 OK", json!({ "linked": "pr:Org/shop#2414", "to": "session:s1" }).to_string(), false);
    let o = cli(&api, &["link", "pr", "2414"], true);
    assert!(o.status.success(), "{}", err(&o));
    assert_eq!(out(&o).trim(), "Linked pr:Org/shop#2414");
    let body = seen.join().unwrap().body;
    assert_eq!(
        (body["kind"].as_str(), body["ref"].as_str(), body["how"].as_str()),
        (Some("pr"), Some("2414"), Some("claude"))
    );

    let (api, seen) =
        app_answering("200 OK", json!({ "unlinked": "ticket:SH-1", "to": "session:s1" }).to_string(), false);
    let o = cli(&api, &["unlink", "ticket", "SH-1"], false);
    assert_eq!(out(&o).trim(), "Unlinked ticket:SH-1");
    assert_eq!(seen.join().unwrap().body["how"], "you");
}

#[test]
fn the_apps_refusals_and_errors_are_said_plainly() {
    let (api, _seen) = app_answering("403 Forbidden", String::new(), false);
    let o = cli(&api, &["status"], true);
    assert!(!o.status.success());
    assert!(err(&o).contains("restarted since this terminal opened"), "{}", err(&o));

    let (api, _seen) = app_answering(
        "400 Bad Request",
        json!({ "error": "\"x\" isn't a ticket key (like SH-21533)" }).to_string(),
        false,
    );
    let o = cli(&api, &["link", "ticket", "x"], true);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(err(&o).trim(), "enkidu: \"x\" isn't a ticket key (like SH-21533)");

    let o = cli("http://127.0.0.1:9", &["link"], true);
    assert!(err(&o).contains("enkidu link pr 2414"), "a link needs what to link: {}", err(&o));
    let o = cli("http://127.0.0.1:9", &["teleport"], true);
    assert!(err(&o).contains("enkidu --help"));
}

#[test]
fn doctor_prints_each_source_and_fails_when_the_index_differs() {
    let reply = json!([
        { "project": "shop", "source": "github", "ok": true, "compared": ["6 open PRs as `gh pr list --author @me` lists them"], "differences": [], "error": null },
        { "project": "shop", "source": "jira", "ok": false, "compared": [], "differences": [{ "thing": "SH-1", "field": "status", "index": "To Do", "direct": "In Progress" }], "error": null },
    ]);
    let (api, _seen) = app_answering("200 OK", reply.to_string(), false);
    let o = cli(&api, &["doctor"], false);
    assert_eq!(o.status.code(), Some(1));
    let text = out(&o);
    assert!(text.contains("✓ GitHub for shop\n    compared 6 open PRs"));
    assert!(text.contains("✗ Jira for shop\n    SH-1 status: the index has \"To Do\", Jira says \"In Progress\""));
    assert!(err(&o).contains("doesn't match"));
}

#[test]
fn without_the_app_it_says_to_open_it() {
    let data = tempfile::tempdir().unwrap();
    // no API in the environment and no api.json: the app isn't running
    let o = Command::new(CLI)
        .arg("status")
        .env_clear()
        .env("HOME", data.path())
        .env("ENKIDU_DATA_DIR", data.path())
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(err(&o).trim(), "enkidu: Enkidu isn't running. Open it, then try again.");
    // api.json left by an app that quit: same answer, not a hang
    std::fs::write(data.path().join("api.json"), json!({ "url": "http://127.0.0.1:9", "token": "old" }).to_string())
        .unwrap();
    let o = Command::new(CLI)
        .arg("status")
        .env_clear()
        .env("HOME", data.path())
        .env("ENKIDU_DATA_DIR", data.path())
        .output()
        .unwrap();
    assert!(err(&o).contains("isn't running"));
    // help needs no app
    let o = Command::new(CLI).arg("--help").env_clear().output().unwrap();
    assert!(o.status.success());
    assert!(out(&o).contains("Nothing here writes to GitHub or Jira."));
}

#[test]
fn event_records_the_work_but_not_what_you_typed() {
    let data = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        assert!(Command::new("git").args(args).current_dir(repo.path()).output().unwrap().status.success())
    };
    git(&["init", "-q", "-b", "feature/SH-21533-totals"]);
    let payload = json!({
        "session_id": "warp1", "transcript_path": "/h/.claude-work/projects/-code-shop/warp1.jsonl",
        "cwd": repo.path(), "hook_event_name": "UserPromptSubmit",
        "prompt": "my private words about SH-21533 and https://github.com/Org/shop/pull/2414",
    });
    let mut child = Command::new(CLI)
        .arg("event")
        .env("ENKIDU_DATA_DIR", data.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(payload.to_string().as_bytes()).unwrap();
    let o = child.wait_with_output().unwrap();
    assert!(o.status.success());
    assert_eq!((out(&o), err(&o)), (String::new(), String::new()), "a hook's output would become Claude's context");
    let log = std::fs::read_to_string(data.path().join("events.jsonl")).unwrap();
    assert!(!log.contains("private words"));
    let v: Value = serde_json::from_str(log.trim()).unwrap();
    assert_eq!(v["event"], "UserPromptSubmit");
    assert_eq!(v["branch"], "feature/SH-21533-totals");
    assert_eq!(v["mentions"], json!(["SH-21533", "https://github.com/Org/shop/pull/2414"]));
    // a broken payload records nothing and still never fails the hook
    let mut child =
        Command::new(CLI).arg("event").env("ENKIDU_DATA_DIR", data.path()).stdin(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(b"not json").unwrap();
    assert!(child.wait().unwrap().success());
    assert_eq!(std::fs::read_to_string(data.path().join("events.jsonl")).unwrap().lines().count(), 1);
}
