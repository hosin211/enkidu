//! Running `gh` and `twg`: a time limit, both outputs read while the program runs (so a big answer can't block it),
//! and nothing of the environment (where a token may travel) ever logged.

use anyhow::{anyhow, Result};
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// A program to run, with arguments and extra environment. Tests point `program` at a fake.
#[derive(Clone, Debug)]
pub struct Tool {
    pub program: String,
    /// Arguments that come before every call's own (a fake run through `node` needs its script here).
    pub prefix: Vec<String>,
    pub path_env: String,
    /// Extra environment for every run (a fake's mode in tests; empty in the app).
    pub env: Vec<(String, String)>,
    /// A shorter time limit than the callers ask for (tests of a hanging tool).
    pub max_wait: Option<Duration>,
}

/// A failed read with what to do about it, in words for the health line: "Jira: twg isn't signed in" and
/// "Run `twg login --force`."
#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
    pub message: String,
    pub fix: String,
}

impl Failure {
    pub fn new(message: &str, fix: &str) -> Self {
        Self { message: message.into(), fix: fix.into() }
    }

    /// The failure inside an error, or a plain one made from its message.
    pub fn of(e: &anyhow::Error) -> Failure {
        match e.downcast_ref::<Failure>() {
            Some(f) => f.clone(),
            None => Failure::new(&crate::redact::redact_urls(&format!("{e:#}")), "Enkidu tries again."),
        }
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Failure {}

#[derive(Debug)]
pub struct Output {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Tool {
    pub fn new(program: &str, path_env: &str) -> Self {
        // `node /path/fake.mjs` style: the first word runs, the rest come first
        let mut words = program.split_whitespace().map(String::from);
        let first = words.next().unwrap_or_default();
        Self { program: first, prefix: words.collect(), path_env: path_env.to_string(), env: Vec::new(), max_wait: None }
    }

    /// Runs and waits at most `limit`. The environment gets `env` on top of the app's own.
    pub fn run(&self, args: &[&str], env: &[(&str, &str)], limit: Duration) -> Result<Output> {
        let limit = self.max_wait.map_or(limit, |m| m.min(limit));
        let mut cmd = Command::new(&self.program);
        cmd.args(&self.prefix).args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        if !self.path_env.is_empty() {
            cmd.env("PATH", &self.path_env);
        }
        // never ask a person for anything: no prompts, no pager, no colors
        cmd.env("GH_PROMPT_DISABLED", "1").env("GH_NO_UPDATE_NOTIFIER", "1").env("NO_COLOR", "1").env("PAGER", "cat");
        for (k, v) in self.env.iter().map(|(k, v)| (k.as_str(), v.as_str())).chain(env.iter().copied()) {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().map_err(|e| anyhow!("can't start {}: {e}", self.program))?;
        let mut out = child.stdout.take().expect("piped");
        let mut err = child.stderr.take().expect("piped");
        let out_reader = std::thread::spawn(move || {
            let mut s = String::new();
            let _ = out.read_to_string(&mut s);
            s
        });
        let err_reader = std::thread::spawn(move || {
            let mut s = String::new();
            let _ = err.read_to_string(&mut s);
            s
        });
        let started = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if started.elapsed() > limit {
                let _ = child.kill();
                let _ = child.wait();
                return Err(anyhow!("{} took longer than {} s", self.name(), limit.as_secs()));
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        Ok(Output {
            code: status.code().unwrap_or(-1),
            stdout: out_reader.join().unwrap_or_default(),
            stderr: err_reader.join().unwrap_or_default(),
        })
    }

    /// The tool's name for messages: `gh`, `twg`, or the fake's script name.
    pub fn name(&self) -> String {
        let last = self.prefix.last().unwrap_or(&self.program);
        std::path::Path::new(last).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| last.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_reads_both_outputs_and_reports_the_code() {
        let sh = Tool::new("/bin/sh", "/usr/bin:/bin");
        let out = sh.run(&["-c", "echo out; echo err >&2; exit 3"], &[("X", "1")], Duration::from_secs(5)).unwrap();
        assert_eq!(out.code, 3);
        assert_eq!(out.stdout.trim(), "out");
        assert_eq!(out.stderr.trim(), "err");
    }

    #[test]
    fn a_big_answer_does_not_block_and_a_slow_one_is_stopped() {
        let sh = Tool::new("/bin/sh", "/usr/bin:/bin");
        let out = sh.run(&["-c", "head -c 3000000 /dev/zero | tr '\\0' a"], &[], Duration::from_secs(10)).unwrap();
        assert_eq!(out.stdout.len(), 3_000_000);
        let slow = sh.run(&["-c", "sleep 5"], &[], Duration::from_millis(300));
        assert!(slow.unwrap_err().to_string().contains("took longer"));
    }

    #[test]
    fn a_fake_run_through_node_keeps_its_script_first() {
        let t = Tool::new("node /x/fake-gh.mjs", "");
        assert_eq!(t.program, "node");
        assert_eq!(t.prefix, vec!["/x/fake-gh.mjs"]);
        assert_eq!(t.name(), "fake-gh.mjs");
    }
}
