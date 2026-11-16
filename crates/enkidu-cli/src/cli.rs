//! `enkidu`: Enkidu's Work index from the command line. In a terminal Enkidu started (Claude's or a shell) it
//! knows the app's address, the workspace and the session from its environment; elsewhere it finds the running app
//! through `api.json` in the app's data folder. `enkidu event` is for Claude's hooks outside Enkidu: it records the
//! event in the app's event log (no words of yours: only the ticket keys and PR links in a prompt), even while the app
//! is closed, and prints nothing.

use enkidu_core::work::{hookev, parse};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::Duration;

const HELP: &str = "enkidu: Enkidu's Work index (your tickets, PRs and the Claude sessions behind them)

  enkidu status                 this session's PRs, tickets and what waits on you there
  enkidu items [--pr N] [--ticket KEY]
                                what waits on you
  enkidu list prs|tickets|sessions [--needs-me]
  enkidu link pr N|ticket KEY|task N
                                this session works on it (enkidu unlink … undoes a wrong link);
                                a task is one of your own, by the number on its card
  enkidu item done|snooze ID    mark something done, or snooze it for a day
  enkidu note \"text\" [--pr N] [--ticket KEY] [--task N]
                                a line in the ticket's (or PR's, or your own task's) timeline
  enkidu report done|review|blocked \"what you did\" [--image FILE]... [--ticket KEY] [--pr N] [--task N]
                                how the task ended, shown on it in the Work page: done
                                (nothing needs the user), review (it wants their review),
                                blocked (you couldn't finish); --image adds a screenshot
                                (png, jpg, gif, webp; repeat it for more). \"-\" reads the
                                text from stdin
  enkidu update \"how it goes\" [--image FILE]... [--ticket KEY] [--pr N] [--task N]
                                a word on the task while you work (a screenshot with
                                --image): it shows in the workflow and on the task, and
                                ends nothing (enkidu report does)
  enkidu workflow               the workflow you work for: this task, its time left, and
                                how the others stand
  enkidu live [on|off]          on: this workspace goes Live (its changes into the main
                                folder, where the dev environment runs; another one's go
                                back first); off: the main folder goes back; alone: which
                                one is Live
  enkidu refresh                read GitHub and Jira again now
  enkidu doctor                 check that the index matches GitHub and Jira (reads both)

  --json                        answers as JSON
  --session ID, --workspace ID  outside a terminal Enkidu started
Nothing here writes to GitHub or Jira.";

/// Enkidu's data folder (`ENKIDU_DATA_DIR`, else its own folder).
fn data_dir() -> PathBuf {
    enkidu_core::places::data_dir()
}

/// Where the running app answers: (http://127.0.0.1:PORT, token).
fn api() -> Result<(String, String), String> {
    if let (Ok(url), Ok(token)) = (std::env::var("ENKIDU_API"), std::env::var("ENKIDU_TOKEN")) {
        if !url.is_empty() && !token.is_empty() {
            return Ok((url, token));
        }
    }
    let file = data_dir().join("api.json");
    let read = || -> Option<(String, String)> {
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&file).ok()?).ok()?;
        Some((v.get("url")?.as_str()?.to_string(), v.get("token")?.as_str()?.to_string()))
    };
    read().ok_or_else(|| NOT_RUNNING.to_string())
}

const NOT_RUNNING: &str = "Enkidu isn't running. Open it, then try again.";

/// One call to the app: POST /api/<token>/<command> with a JSON body.
fn call(command: &str, body: &Value) -> Result<Value, String> {
    let (url, token) = api()?;
    let addr = url.trim_start_matches("http://").trim_end_matches('/').to_string();
    let mut stream = TcpStream::connect(&addr).map_err(|_| NOT_RUNNING.to_string())?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(300)));
    let payload = body.to_string();
    let request = format!(
        "POST /api/{token}/{command} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
        payload.len()
    );
    stream.write_all(request.as_bytes()).map_err(|_| NOT_RUNNING.to_string())?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).map_err(|e| format!("Enkidu didn't answer: {e}"))?;
    let raw = String::from_utf8_lossy(&raw);
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
    let code: u16 = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    // tiny_http answers with chunks when it doesn't know the length
    let body = if head.to_ascii_lowercase().contains("transfer-encoding: chunked") { unchunk(body) } else { body.to_string() };
    match code {
        200 => serde_json::from_str(&body).map_err(|e| format!("Enkidu's answer isn't JSON: {e}")),
        403 => Err("Enkidu refused the call: it was restarted since this terminal opened. Open a new terminal.".into()),
        _ => Err(serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|v| v.get("error").and_then(Value::as_str).map(String::from))
            .unwrap_or_else(|| format!("Enkidu answered {code}"))),
    }
}

fn unchunk(body: &str) -> String {
    let mut out = String::new();
    let mut rest = body;
    while let Some((size, after)) = rest.split_once("\r\n") {
        let n = usize::from_str_radix(size.trim(), 16).unwrap_or(0);
        if n == 0 || after.len() < n {
            break;
        }
        out.push_str(&after[..n]);
        rest = after[n..].trim_start_matches("\r\n");
    }
    out
}

/// `enkidu event`: a hook's payload from stdin, as one line in the event log. Prints nothing (a hook's output would become
/// Claude's context) and never fails the hook.
fn event() {
    let mut body = String::new();
    let _ = std::io::stdin().take(2_000_000).read_to_string(&mut body);
    let Some(p) = hookev::read_payload(&body) else { return };
    let branch = p.cwd.as_deref().filter(|_| matches!(p.event.as_str(), "SessionStart" | "UserPromptSubmit" | "PostToolUse")).and_then(|cwd| {
        let out = std::process::Command::new("git").args(["-C", cwd, "symbolic-ref", "--quiet", "--short", "HEAD"]).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string()).filter(|b| !b.is_empty())
    });
    // what you typed stays out: only the ticket keys and PR links in it
    let typed = p.prompt.as_deref().map(|t| parse::typed_text(&Value::String(t.to_string()))).unwrap_or_default();
    let mut mentions = parse::ticket_keys(&typed);
    mentions.extend(parse::pr_refs(&typed).into_iter().map(|r| {
        let (repo, n) = r.split_once('#').unwrap_or((&r, ""));
        format!("https://github.com/{repo}/pull/{n}")
    }));
    let line = json!({
        "v": 1,
        "at": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0),
        "event": p.event, "session_id": p.session_id, "transcript_path": p.transcript_path, "cwd": p.cwd, "branch": branch,
        "mentions": mentions, "prs": p.ops.prs, "pushed": p.ops.pushed, "commits": p.ops.commits,
        // the tool call used GitHub or Jira: the end of its turn reads them again
        "touched": p.touched,
    });
    // a hook never fails: what can't be written is left out
    let dir = data_dir();
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("events.jsonl")) {
        let _ = f.write_all(format!("{line}\n").as_bytes());
    }
}

/// A file named on the command line, as a full path (the app copies it from there).
fn absolute(file: &str) -> String {
    let p = PathBuf::from(file);
    let full = if p.is_absolute() { p } else { std::env::current_dir().unwrap_or_default().join(p) };
    full.to_string_lossy().to_string()
}

fn take(args: &mut Vec<String>, flag: &str) -> Option<String> {
    let i = args.iter().position(|a| a == flag)?;
    args.remove(i);
    (i < args.len()).then(|| args.remove(i))
}

fn flag(args: &mut Vec<String>, flag: &str) -> bool {
    match args.iter().position(|a| a == flag) {
        Some(i) => {
            args.remove(i);
            true
        }
        None => false,
    }
}

fn print_items(items: &[Value]) {
    for i in items {
        let s = |k: &str| i.get(k).and_then(Value::as_str).unwrap_or("");
        // where it is, unless the title already says ("Remark from mira-k on orders/export.py:48")
        let place = match (i.get("path").and_then(Value::as_str), i.get("line").and_then(Value::as_i64)) {
            (Some(p), _) if s("title").contains(p) => String::new(),
            (Some(p), Some(l)) => format!(" ({p}:{l})"),
            (Some(p), None) => format!(" ({p})"),
            _ => String::new(),
        };
        println!("  {}  {}{}{}", s("id"), s("title"), place, if i.get("bot").and_then(Value::as_bool) == Some(true) { " [bot]" } else { "" });
    }
}

fn run(mut args: Vec<String>) -> Result<(), String> {
    let json_out = flag(&mut args, "--json");
    let session = take(&mut args, "--session").or_else(|| std::env::var("ENKIDU_SESSION").ok());
    let workspace = take(&mut args, "--workspace").or_else(|| std::env::var("ENKIDU_WORKSPACE").ok());
    let mut body = json!({ "session": session, "workspace": workspace });
    let command = if args.is_empty() { "help".to_string() } else { args.remove(0) };
    let show = |v: &Value| {
        if json_out {
            println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
            true
        } else {
            false
        }
    };
    match command.as_str() {
        "help" | "--help" | "-h" => println!("{HELP}"),
        "status" => {
            let v = call("status", &body)?;
            if !show(&v) {
                let text = v.get("text").and_then(Value::as_str).unwrap_or("");
                if text.is_empty() {
                    println!("Enkidu knows no PR or ticket of this session yet. Link one: enkidu link pr 2414");
                } else {
                    println!("{text}");
                }
                let items = v.get("items").and_then(Value::as_array).cloned().unwrap_or_default();
                if !items.is_empty() {
                    println!("Waiting on you:");
                    print_items(&items);
                }
            }
        }
        "items" => {
            body["pr"] = json!(take(&mut args, "--pr"));
            body["ticket"] = json!(take(&mut args, "--ticket"));
            let v = call("items", &body)?;
            if !show(&v) {
                let items = v.as_array().cloned().unwrap_or_default();
                if items.is_empty() {
                    println!("Nothing waits on you there.");
                }
                print_items(&items);
            }
        }
        "list" => {
            body["needs_me"] = json!(flag(&mut args, "--needs-me"));
            body["what"] = json!(args.first().cloned().unwrap_or_default());
            let v = call("list", &body)?;
            if !show(&v) {
                for x in v.as_array().cloned().unwrap_or_default() {
                    let s = |k: &str| match x.get(k) {
                        Some(Value::String(t)) => t.clone(),
                        Some(Value::Null) | None => String::new(),
                        Some(v) => v.to_string(),
                    };
                    match body["what"].as_str() {
                        Some("prs") => println!("  #{}  {}  {} → {}  {}", s("number"), s("title"), s("head"), s("base"), s("summary")),
                        Some("tickets") => {
                            let prs: Vec<String> =
                                x.get("prs").and_then(Value::as_array).into_iter().flatten().filter_map(|p| p.get("number")).map(|n| format!("#{n}")).collect();
                            let waits = x.get("waiting").and_then(Value::as_i64).unwrap_or(0);
                            let mut line = format!("  {}  {}  [{}]", s("key"), s("summary"), s("status"));
                            if !prs.is_empty() {
                                line.push_str(&format!("  {}", prs.join(" ")));
                            }
                            if waits > 0 {
                                line.push_str(&format!("  {waits} waiting"));
                            }
                            println!("{line}");
                        }
                        _ => println!("  {}  {}  {}", s("id"), s("title"), s("cwd")),
                    }
                }
            }
        }
        "link" | "unlink" => {
            if args.len() < 2 {
                return Err(format!("enkidu {command} pr 2414, or enkidu {command} ticket SH-21533"));
            }
            body["kind"] = json!(args[0]);
            body["ref"] = json!(args[1]);
            // Claude Code marks the commands it runs with CLAUDECODE=1: a link Claude made, else one you made
            body["how"] = json!(if std::env::var_os("CLAUDECODE").is_some() { "claude" } else { "you" });
            let v = call(&command, &body)?;
            if !show(&v) {
                let what = v.get("linked").or_else(|| v.get("unlinked")).and_then(Value::as_str).unwrap_or("");
                println!("{} {what}", if command == "link" { "Linked" } else { "Unlinked" });
            }
        }
        "item" => {
            if args.len() < 2 {
                return Err("enkidu item done ID, or enkidu item snooze ID".into());
            }
            body["action"] = json!(match args[0].as_str() {
                "done" => "done",
                "snooze" => "snoozed",
                "open" => "open",
                other => return Err(format!("enkidu item done or snooze, not \"{other}\"")),
            });
            body["id"] = json!(args[1]);
            let v = call("item", &body)?;
            if !show(&v) {
                println!("{} is {}", args[1], v.get("state").and_then(Value::as_str).unwrap_or(""));
            }
        }
        "note" => {
            body["pr"] = json!(take(&mut args, "--pr"));
            body["ticket"] = json!(take(&mut args, "--ticket"));
            body["task"] = json!(take(&mut args, "--task"));
            body["text"] = json!(args.join(" "));
            let v = call("note", &body)?;
            if !show(&v) {
                println!("Noted on {}", v.get("noted").and_then(Value::as_str).unwrap_or(""));
            }
        }
        "report" => {
            body["ticket"] = json!(take(&mut args, "--ticket"));
            body["pr"] = json!(take(&mut args, "--pr"));
            body["task"] = json!(take(&mut args, "--task"));
            let mut images = Vec::new();
            while let Some(file) = take(&mut args, "--image") {
                images.push(absolute(&file));
            }
            body["images"] = json!(images);
            if args.is_empty() {
                return Err("enkidu report done|review|blocked \"what you did, how you checked it, what is left\"".into());
            }
            body["state"] = json!(args.remove(0));
            let text = if args.len() == 1 && args[0] == "-" {
                let mut s = String::new();
                std::io::stdin().take(200_000).read_to_string(&mut s).map_err(|e| format!("Can't read the text: {e}"))?;
                s
            } else {
                args.join(" ")
            };
            body["text"] = json!(text);
            let v = call("report", &body)?;
            if !show(&v) {
                let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                let n = v.get("images").and_then(Value::as_i64).unwrap_or(0);
                let pictures = match n {
                    0 => String::new(),
                    1 => ", with 1 screenshot".to_string(),
                    n => format!(", with {n} screenshots"),
                };
                println!("Reported on {}: {}{pictures}. It shows on the task in Enkidu's Work page.", s("reported"), s("words"));
            }
        }
        "update" => {
            body["ticket"] = json!(take(&mut args, "--ticket"));
            body["pr"] = json!(take(&mut args, "--pr"));
            body["task"] = json!(take(&mut args, "--task"));
            let mut images = Vec::new();
            while let Some(file) = take(&mut args, "--image") {
                images.push(absolute(&file));
            }
            body["images"] = json!(images);
            if args.is_empty() {
                return Err("enkidu update \"how it goes\" (with --image FILE for a screenshot)".into());
            }
            let text = if args.len() == 1 && args[0] == "-" {
                let mut s = String::new();
                std::io::stdin().take(200_000).read_to_string(&mut s).map_err(|e| format!("Can't read the text: {e}"))?;
                s
            } else {
                args.join(" ")
            };
            body["text"] = json!(text);
            let v = call("update", &body)?;
            if !show(&v) {
                let n = v.get("images").and_then(Value::as_i64).unwrap_or(0);
                let pictures = match n {
                    0 => String::new(),
                    1 => ", with 1 screenshot".to_string(),
                    n => format!(", with {n} screenshots"),
                };
                println!("Noted on {}{pictures}. It shows in the workflow and on the task.", v.get("reported").and_then(Value::as_str).unwrap_or(""));
            }
        }
        "workflow" => {
            let v = call("workflow", &body)?;
            if !show(&v) {
                let Some(f) = v.get("workflow").filter(|f| !f.is_null()) else {
                    println!("This workspace's Claude works for no workflow.");
                    return Ok(());
                };
                let s = |x: &Value, k: &str| x.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                let key = |subject: &str| subject.strip_prefix("ticket:").map(String::from).unwrap_or_else(|| format!("PR #{}", subject.rsplit('#').next().unwrap_or("")));
                let mut head = format!("Workflow \"{}\" ({})", s(f, "name"), s(f, "state"));
                if let Some(t) = f.get("task").filter(|t| !t.is_null()) {
                    head.push_str(&format!(": task {} of {}, {} ({})", t["n"], f["of"], key(&s(t, "subject")), s(t, "kind")));
                    if let Some(left) = t.get("minutesLeft").and_then(Value::as_i64) {
                        head.push_str(&format!(", {left} min left of {}", t["minutes"]));
                    }
                }
                println!("{head}.");
                for t in f.get("tasks").and_then(Value::as_array).into_iter().flatten() {
                    let yours = if t.get("yours").and_then(Value::as_bool) == Some(true) { "  (this one)" } else { "" };
                    println!("  {}. {} {}: {}{yours}", t["n"], key(&s(t, "subject")), s(t, "title"), s(t, "state"));
                }
            }
        }
        "live" => {
            body["action"] = json!(args.first().cloned());
            let v = call("live", &body)?;
            if !show(&v) {
                let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                match args.first().map(String::as_str) {
                    Some("on") | Some("off") => {
                        let note = s("note");
                        println!("{}", if note.is_empty() { format!("{} is {}.", s("workspace"), if args[0] == "on" { "Live" } else { "not Live" }) } else { note });
                        let output = s("output");
                        if !output.trim().is_empty() {
                            println!("{}", output.trim_end());
                        }
                    }
                    _ => match v.get("liveWorkspace").and_then(Value::as_str) {
                        Some(name) if v.get("live").and_then(Value::as_bool) == Some(true) => println!("{name} (this workspace) is Live."),
                        Some(name) => println!("{name} is Live, not this workspace."),
                        None => println!("No workspace is Live."),
                    },
                }
            }
        }
        "refresh" => {
            let v = call("refresh", &body)?;
            if !show(&v) {
                println!("Reading GitHub and Jira again.");
            }
        }
        "doctor" => {
            let v = call("doctor", &body)?;
            if !show(&v) {
                let checks = v.as_array().cloned().unwrap_or_default();
                if checks.is_empty() {
                    println!("No project reads GitHub or Jira yet.");
                }
                let mut bad = false;
                for c in checks {
                    let s = |k: &str| c.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                    let ok = c.get("ok").and_then(Value::as_bool) == Some(true);
                    bad |= !ok;
                    let source = if s("source") == "github" { "GitHub" } else { "Jira" };
                    println!("{} {} for {}", if ok { "✓" } else { "✗" }, source, s("project"));
                    for line in c.get("compared").and_then(Value::as_array).into_iter().flatten() {
                        println!("    compared {}", line.as_str().unwrap_or(""));
                    }
                    if let Some(e) = c.get("error").and_then(Value::as_str) {
                        println!("    can't read: {e}");
                    }
                    for d in c.get("differences").and_then(Value::as_array).into_iter().flatten() {
                        let s = |k: &str| d.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                        println!("    {} {}: the index has {:?}, {} says {:?}", s("thing"), s("field"), s("index"), source, s("direct"));
                    }
                }
                if bad {
                    return Err("The index doesn't match GitHub or Jira (see above).".into());
                }
            }
        }
        other => return Err(format!("enkidu doesn't know \"{other}\". enkidu --help lists what it does.")),
    }
    Ok(())
}

pub fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("event") {
        event();
        return;
    }
    if let Err(e) = run(args) {
        eprintln!("enkidu: {e}");
        std::process::exit(1);
    }
}
