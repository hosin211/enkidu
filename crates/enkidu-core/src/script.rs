//! Runs the user's shell snippets (setup script, Live on/off) with their login PATH.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::redact::redact_urls;
use crate::shellenv::{login_path, user_shell};

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptResult {
    pub ok: bool,
    pub code: Option<i32>,
    pub timed_out: bool,
    /// The end of the output, with URLs removed.
    pub output: String,
}

const OUTPUT_TAIL: usize = 6_000;

pub fn run(script: &str, cwd: &Path, env: &[(String, String)], timeout: Duration, log: Option<&Path>) -> ScriptResult {
    let wrapped = format!("{{\n{script}\n}} 2>&1");
    let spawned = Command::new(user_shell())
        .arg("-lc")
        .arg(&wrapped)
        .current_dir(cwd)
        .env("PATH", login_path())
        .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(e) => {
            return ScriptResult { ok: false, code: None, timed_out: false, output: format!("Can't start the shell: {e}") }
        }
    };

    // Read in the background: a script can leave a background process holding stdout open.
    let collected = Arc::new(Mutex::new(Vec::<u8>::new()));
    if let Some(mut stdout) = child.stdout.take() {
        let sink = collected.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            while let Ok(n) = stdout.read(&mut buf) {
                if n == 0 {
                    break;
                }
                sink.lock().unwrap_or_else(|e| e.into_inner()).extend_from_slice(&buf[..n]);
            }
        });
    }

    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() >= deadline => {
                timed_out = true;
                let _ = child.kill();
                break child.wait().ok();
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(40)),
            Err(_) => break None,
        }
    };
    std::thread::sleep(Duration::from_millis(60));
    let raw = collected.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let text = redact_urls(&String::from_utf8_lossy(&raw));
    if let Some(log) = log {
        if let Some(dir) = log.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(log, &text);
    }
    let code = status.and_then(|s| s.code());
    ScriptResult { ok: !timed_out && code == Some(0), code, timed_out, output: tail(&text, OUTPUT_TAIL) }
}

pub fn tail(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut start = text.len() - max;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    format!("…{}", &text[start..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_output_exit_code_and_env() {
        let dir = tempfile::tempdir().unwrap();
        let env = vec![("ENKIDU_TEST_VALUE".to_string(), "hello".to_string())];
        let r = run("echo \"$ENKIDU_TEST_VALUE\"; echo oops >&2; exit 3", dir.path(), &env, Duration::from_secs(10), None);
        assert!(!r.ok);
        assert_eq!(r.code, Some(3));
        assert!(r.output.contains("hello"));
        assert!(r.output.contains("oops"), "stderr is captured too");
    }

    #[test]
    fn times_out_and_writes_the_log() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("logs/run.log");
        let r = run("echo start; sleep 5", dir.path(), &[], Duration::from_millis(400), Some(&log));
        assert!(r.timed_out);
        assert!(!r.ok);
        assert!(std::fs::read_to_string(log).unwrap().contains("start"));
    }

    #[test]
    fn hides_urls_in_output() {
        let dir = tempfile::tempdir().unwrap();
        let r = run("echo https://token@example.com/x", dir.path(), &[], Duration::from_secs(10), None);
        assert!(r.ok);
        assert!(r.output.contains("<url>"));
        assert!(!r.output.contains("token@"));
    }
}
