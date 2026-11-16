//! items 1.4, 1.6 and 1.8 in one run. The real core is started with `enkidu_core::start` in a home folder of
//! its own, and the real `enkidu` runs beside it. Both work in the app's own data folder under that home: the
//! command finds the core there and answers from it, a hook's event goes into the app's own log, and neither `gh`
//! nor `twg` is run for it.
//!
//! One test in this file on purpose: it sets the process's environment.

use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex};

const CLI: &str = env!("CARGO_BIN_EXE_enkidu");

fn said(out: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

#[test]
fn the_core_starts_in_the_apps_own_folder_and_the_command_finds_it_there() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().canonicalize().unwrap();

    // a gh and a twg that leave a mark if they are ever run
    let marks = home.join("marks");
    let never = home.join("never-run");
    std::fs::write(&never, format!("#!/bin/sh\necho \"$0 $*\" >> '{}'\nexit 1\n", marks.display())).unwrap();
    std::fs::set_permissions(&never, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();

    // HOME and the shell are the test's own, so nothing reads the real home folder
    std::env::set_var("HOME", &home);
    std::env::set_var("SHELL", "/bin/sh");
    std::env::set_var("ENKIDU_GH", &never);
    std::env::set_var("ENKIDU_TWG", &never);
    std::env::set_var("ENKIDU_CLAUDE_DIRS", home.join("no-claude-folders"));
    for name in ["ENKIDU_DATA_DIR", "ENKIDU_API", "ENKIDU_TOKEN"] {
        std::env::remove_var(name);
    }

    // the core starts in its own folder: the default one under this home
    let seen = Arc::new(Mutex::new(Vec::new()));
    let core = {
        let seen = seen.clone();
        enkidu_core::start(enkidu_core::Events::new(move |e| seen.lock().unwrap().push(format!("{e:?}"))))
            .expect("it starts")
    };
    let data = home.join("Library/Application Support/io.github.hosin211.enkidu");
    assert_eq!(core.shared.data_dir, data);
    let api: Value = serde_json::from_str(&std::fs::read_to_string(data.join("api.json")).unwrap()).unwrap();
    let url = api["url"].as_str().unwrap();
    assert!(url.starts_with("http://127.0.0.1:"), "the core listens on this Mac only: {url}");

    // enkidu, run with nothing but that home, finds the core by its folder and answers from it
    let cli = |args: &[&str]| Command::new(CLI).args(args).output().unwrap();
    let out = cli(&["list", "prs", "--json"]);
    assert!(out.status.success(), "{}", said(&out));
    let answer: Value = serde_json::from_slice(&out.stdout).expect("the app's JSON answer");
    assert!(!answer.is_null(), "{answer}");

    // enkidu event writes to the app's own event log
    let payload = json!({ "session_id": "s1", "cwd": home, "hook_event_name": "SessionStart" });
    let mut child = Command::new(CLI).arg("event").stdin(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(payload.to_string().as_bytes()).unwrap();
    assert!(child.wait().unwrap().success());
    let log = std::fs::read_to_string(data.join("events.jsonl")).unwrap();
    assert!(log.contains("\"SessionStart\"") && log.contains("\"s1\""), "{log}");

    // and none of it ran gh or twg
    assert!(!marks.exists(), "gh or twg was run: {}", std::fs::read_to_string(&marks).unwrap_or_default());

    core.shutdown();
}
