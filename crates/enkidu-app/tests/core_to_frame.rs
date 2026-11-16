//! Item 1.15, end to end: the stores start the real core in a data folder of their own, the frame shows what its
//! database holds, and a hook call over HTTP (as Claude's hooks make it) turns the workspace's lamp in the frame.
//!
//! One test in this file on purpose: it sets the process's environment (a home folder of its own, so nothing reads
//! the real one).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use enkidu_core::db::{Db, Project, Workspace};
use gpui_kit::TestAppContext;

fn post(url: &str, path: &str, body: &str) -> String {
    let addr = url.trim_start_matches("http://");
    let mut stream = TcpStream::connect(addr).unwrap();
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).unwrap();
    let mut answer = String::new();
    let _ = stream.read_to_string(&mut answer);
    answer
}

#[gpui_kit::test]
fn a_hook_call_turns_the_lamp_in_the_frame(cx: &mut TestAppContext) {
    let home = tempfile::tempdir().unwrap();
    let data = home.path().join("data");
    let never = home.path().join("never-run");
    std::fs::write(&never, "#!/bin/sh\nexit 1\n").unwrap();
    std::fs::set_permissions(&never, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    // SAFETY: the only test in this binary; nothing else reads the environment meanwhile
    unsafe {
        std::env::set_var("HOME", home.path());
        std::env::set_var("SHELL", "/bin/sh");
        std::env::set_var("ENKIDU_DATA_DIR", &data);
        std::env::set_var("ENKIDU_CLAUDE_DIRS", home.path().join("no-claude-folders"));
        std::env::set_var("ENKIDU_GH", &never);
        std::env::set_var("ENKIDU_TWG", &never);
    }
    // the database already holds a project with one workspace
    {
        std::fs::create_dir_all(&data).unwrap();
        let db = Db::open(&data.join("enkidu.db")).unwrap();
        db.insert_project(&Project {
            id: "p1".into(),
            name: "shop".into(),
            path: home.path().join("shop").to_string_lossy().into(),
            live_workspace_id: None,
            created_at: 1,
        })
        .unwrap();
        db.insert_workspace(&Workspace {
            id: "w1".into(),
            project_id: "p1".into(),
            name: "stock-fix".into(),
            path: home.path().join("ws/stock-fix").to_string_lossy().into(),
            branch: "fix/stock".into(),
            base: Some("development".into()),
            session_id: "s1".into(),
            transcript_path: None,
            claude_config_dir: None,
            created_at: 1,
            adopted: false,
        })
        .unwrap();
    }

    let frame = cx.update(enkidu_app::stores::start).expect("the core starts");
    cx.executor().allow_parking();
    let until = |cx: &mut TestAppContext, done: &dyn Fn(&enkidu_model::frame::Frame) -> bool| {
        for _ in 0..400 {
            cx.run_until_parked();
            if frame.read_with(cx, |f, _| done(f)) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    };
    assert!(until(cx, &|f| !f.workspaces.is_empty()), "the frame shows what the database holds");
    frame.read_with(cx, |f, _| {
        assert_eq!(f.current_workspace().map(|w| w.name.as_str()), Some("stock-fix"));
        assert_eq!(f.status("w1"), "idle");
    });

    // Claude's prompt hook, as its settings make it call the app
    let api: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(data.join("api.json")).unwrap()).unwrap();
    let (url, token) = (api["url"].as_str().unwrap(), api["token"].as_str().unwrap());
    let answer =
        post(url, &format!("/prompt/{token}/w1"), r#"{"session_id":"s1","hook_event_name":"UserPromptSubmit"}"#);
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
    assert!(until(cx, &|f| f.status("w1") == "working"), "the lamp turned");

    // a wrong token changes nothing
    let refused = post(url, "/stop/not-the-token/w1", "{}");
    assert!(refused.starts_with("HTTP/1.1 403"), "{refused}");
    cx.run_until_parked();
    assert_eq!(frame.read_with(cx, |f, _| f.status("w1").to_string()), "working");
}
