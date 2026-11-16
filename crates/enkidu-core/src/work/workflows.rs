//! Workflows (WORK-INDEX.md, "Workflows"): trigger → action recipes, each off until you turn it on in a project's
//! settings. Detection stays in code and Claude does the work. Neither recipe ever runs on stale or half-read data: each
//! acts only right after a full, fresh read of what it uses, and says why when it doesn't.
//!
//! - Failing checks: new failing checks on the latest commit of a PR whose session is in an Enkidu workspace go to
//!   that session, like Send to Claude. Decided here right after GitHub's read; the app's window does the sending (it
//!   owns the terminals) and reports back.
//! - Morning summary: once a day at the set time (or the first time Enkidu runs after it), GitHub and Jira are read
//!   fresh; if both read, a read-only Claude in the project's main folder writes one line per ticket with `enkidu note`.

use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use std::collections::HashSet;

use super::model::Health;
use crate::db::Db;

pub const FAILED_CHECKS: &str = "failed_checks";
pub const MORNING: &str = "morning";

/// One time a workflow acted, or decided not to.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub id: i64,
    pub project_id: String,
    pub workflow: String,
    /// The item it sends (failing checks), or the day it ran (morning summary)
    pub subject: String,
    /// pending, taken (the window is sending it), running, done, skipped or failed
    pub state: String,
    pub at: i64,
    /// What happened, in words
    pub text: String,
}

fn run_from_row(r: &rusqlite::Row) -> rusqlite::Result<Run> {
    Ok(Run { id: r.get(0)?, project_id: r.get(1)?, workflow: r.get(2)?, subject: r.get(3)?, state: r.get(4)?, at: r.get(5)?, text: r.get(6)? })
}

const RUN_COLS: &str = "id, project_id, workflow, subject, state, at, text";

impl Db {
    /// Records that a workflow acts on something, once: the new run's id, or None when it already did.
    pub fn work_workflow_add(&self, project_id: &str, workflow: &str, subject: &str, state: &str, at: i64) -> Result<Option<i64>> {
        let conn = self.conn();
        let n = conn.execute(
            "INSERT OR IGNORE INTO work_workflow_runs (project_id, workflow, subject, state, at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![project_id, workflow, subject, state, at],
        )?;
        Ok((n > 0).then(|| conn.last_insert_rowid()))
    }

    /// The sends waiting for the window, marked taken so they go once even if it asks twice.
    pub fn work_workflow_take(&self, at: i64) -> Result<Vec<Run>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!("SELECT {RUN_COLS} FROM work_workflow_runs WHERE state = 'pending' ORDER BY id"))?;
        let runs: Vec<Run> = stmt.query_map([], run_from_row)?.collect::<rusqlite::Result<_>>()?;
        for r in &runs {
            conn.execute("UPDATE work_workflow_runs SET state = 'taken', at = ?2 WHERE id = ?1", params![r.id, at])?;
        }
        Ok(runs)
    }

    pub fn work_workflow_finish(&self, id: i64, state: &str, text: &str, at: i64) -> Result<()> {
        anyhow::ensure!(matches!(state, "done" | "skipped" | "failed"), "A run ends done, skipped or failed");
        self.conn()
            .execute("UPDATE work_workflow_runs SET state = ?2, text = ?3, at = ?4 WHERE id = ?1", params![id, state, text, at])?;
        Ok(())
    }

    /// Whether a workflow already acted on (or for) this subject.
    pub fn work_workflow_ran(&self, project_id: &str, workflow: &str, subject: &str) -> Result<bool> {
        let conn = self.conn();
        let found: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM work_workflow_runs WHERE project_id = ?1 AND workflow = ?2 AND subject = ?3",
                params![project_id, workflow, subject],
                |r| r.get(0),
            )
            .optional()?;
        Ok(found.is_some())
    }

    pub fn work_workflow_run(&self, id: i64) -> Result<Option<Run>> {
        let conn = self.conn();
        Ok(conn.query_row(&format!("SELECT {RUN_COLS} FROM work_workflow_runs WHERE id = ?1"), [id], run_from_row).optional()?)
    }

    /// A project's latest runs, newest first.
    pub fn work_workflow_runs(&self, project_id: &str, limit: usize) -> Result<Vec<Run>> {
        let conn = self.conn();
        let mut stmt =
            conn.prepare(&format!("SELECT {RUN_COLS} FROM work_workflow_runs WHERE project_id = ?1 ORDER BY at DESC, id DESC LIMIT ?2"))?;
        let rows = stmt.query_map(params![project_id, limit as i64], run_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

/// A read that is full and fresh: it succeeded, and it is the read that just ran (not an older good one).
pub fn fresh(h: &Health, read_at: i64) -> bool {
    h.state == "ok" && h.last_ok.is_some_and(|t| t >= read_at)
}

/// Right after a GitHub read of a project: the failing checks it found new (or back) on a PR whose session is in a
/// Enkidu workspace, queued once each for the window to send. Nothing when the recipe is off or the read wasn't full
/// and fresh. Returns how many it queued.
pub fn after_github(db: &Db, project_id: &str, on: bool, health: &Health, read_at: i64) -> Result<usize> {
    if !on || !fresh(health, read_at) {
        return Ok(0);
    }
    let workspaces: HashSet<String> = db.workspaces()?.into_iter().map(|w| w.id).collect();
    let in_workspace: HashSet<String> = db
        .work_sessions()?
        .into_iter()
        .filter(|s| s.workspace_id.as_ref().is_some_and(|w| workspaces.contains(w)))
        .map(|s| format!("session:{}", s.id))
        .collect();
    let links = db.work_links()?;
    let mut queued = 0;
    for item in db.work_items()? {
        if item.project_id != project_id || item.kind != "pipeline_failed" || item.state != "open" || item.opened_at < read_at {
            continue;
        }
        let Some(pr) = item.pr.as_deref() else { continue };
        let thing = format!("pr:{pr}");
        let has_session = links.iter().any(|l| {
            l.a == thing && (in_workspace.contains(&l.b) || l.b.strip_prefix("workspace:").is_some_and(|w| workspaces.contains(w)))
        });
        if has_session && db.work_workflow_add(project_id, FAILED_CHECKS, &item.id, "pending", read_at)?.is_some() {
            queued += 1;
        }
    }
    Ok(queued)
}

/// Minutes after midnight of a time of day ("09:00", 24 h), if it is one.
pub fn minutes_of(at: &str) -> Option<u32> {
    let (h, m) = at.trim().split_once(':')?;
    if h.is_empty() || h.len() > 2 || m.len() != 2 {
        return None;
    }
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

/// Whether the morning summary is due: on, at or after its time today, and not run yet today.
pub fn morning_due(on: bool, at: &str, now_minutes: u32, ran_today: bool) -> bool {
    on && !ran_today && minutes_of(at).is_some_and(|m| now_minutes >= m)
}

/// The morning summary runs only on full, fresh reads of every source the project uses; otherwise it says which one
/// can't be read and why.
pub fn morning_ready(github: Option<&Health>, jira: Option<&Health>, read_at: i64) -> std::result::Result<(), String> {
    if github.is_none() && jira.is_none() {
        return Err("this project reads neither GitHub nor Jira".into());
    }
    for (name, h) in [("GitHub", github), ("Jira", jira)] {
        if let Some(h) = h.filter(|h| !fresh(h, read_at)) {
            // the health's own words ("Jira: twg isn't signed in"), without saying "Jira" twice, and what to do
            let error = h.error.clone().unwrap_or_else(|| "it wasn't read".into());
            let mut why = format!("{name} can't be read: {}", error.strip_prefix(&format!("{name}: ")).unwrap_or(&error));
            if let Some(fix) = h.fix.as_deref().filter(|f| !f.is_empty()) {
                if !why.ends_with('.') {
                    why.push('.');
                }
                why.push(' ');
                why.push_str(fix);
            }
            return Err(why);
        }
    }
    Ok(())
}

/// Today's date (2026-11-08) and the minutes since midnight, in local time.
pub fn local_now() -> (String, u32) {
    // SAFETY: time() with a null pointer only returns the time; localtime_r writes into the tm we own
    let tm = unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        tm
    };
    (format!("{:04}-{:02}-{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday), (tm.tm_hour * 60 + tm.tm_min).max(0) as u32)
}

/// What the morning summary's Claude is asked (on stdin).
pub const MORNING_PROMPT: &str = "This is Enkidu's morning summary of the user's tickets in this project. You run \
read-only: don't change any file, and don't post or change anything on GitHub or Jira.

1. Run `enkidu list tickets --json`: each ticket with its Jira status, its PRs (state, checks, reviews) and how many \
things wait on the user.
2. Where you need more: `enkidu items --ticket KEY` (what waits, with the words), `twg jira workitem get KEY --output json` \
(its description, status and comments), `gh pr view NUMBER` (a PR's description).
3. For each ticket, write exactly one line on its timeline with `enkidu note --ticket KEY \"...\"`: where it stands and \
what comes next, under 160 characters, facts only.

End with one line saying how many tickets you summarized.";

/// How long the morning summary's Claude may take before it is stopped.
pub const MORNING_LIMIT: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// Runs a program in `cwd` with exactly `env`, `input` on its stdin, for at most `limit`: both outputs are read while it
/// runs (so a long answer can't block it), and it is stopped when the time is up.
pub fn run_headless(
    program: &std::path::Path,
    args: &[String],
    cwd: &std::path::Path,
    env: &[(String, String)],
    input: &str,
    limit: std::time::Duration,
) -> Result<super::run::Output> {
    use std::io::{Read, Write};
    use std::process::{Command, Stdio};
    let mut child = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .env_clear()
        .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| anyhow::anyhow!("Can't start {}: {e}", program.display()))?;
    let reader = |mut pipe: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = pipe.read_to_string(&mut s);
            s
        })
    };
    let out = reader(Box::new(child.stdout.take().expect("piped")));
    let err = reader(Box::new(child.stderr.take().expect("piped")));
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input.as_bytes()); // closed when dropped: the prompt is complete
    }
    let started = std::time::Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("Claude took longer than {} minutes and was stopped", limit.as_secs() / 60);
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    };
    Ok(super::run::Output { code: status.code().unwrap_or(-1), stdout: out.join().unwrap_or_default(), stderr: err.join().unwrap_or_default() })
}

/// The only commands it may run.
pub const MORNING_ALLOWED: &str = "Bash(enkidu:*),Bash(twg jira workitem get:*),Bash(twg jira workitem comment query:*),\
Bash(gh pr view:*),Bash(gh pr checks:*),Bash(gh pr diff:*),Read,Grep,Glob";

/// How the morning summary's Claude starts, whatever your own or the project's Claude flags say (they may skip
/// permissions): print mode with the prompt on stdin; restricted, so your settings files (and their allow rules) are
/// ignored, file tools stay in the folder and bypassing permissions is refused; no MCP servers; only Bash and the read
/// tools exist, and Bash runs only the commands allowed above (in print mode nothing else can be approved). Nothing of
/// the run is saved as a session.
pub fn morning_args() -> Vec<String> {
    [
        "-p",
        "--restricted",
        "--strict-mcp-config",
        "--tools",
        "Bash,Read,Grep,Glob",
        "--allowedTools",
        MORNING_ALLOWED,
        "--output-format",
        "text",
        "--no-session-persistence",
    ]
    .map(String::from)
    .to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{now_ms, Project, Workspace};
    use crate::work::links::tests as l;
    use crate::work::model::Item;
    use crate::work::store::tests::db;

    fn health(state: &str, last_ok: Option<i64>, error: Option<&str>) -> Health {
        Health {
            project_id: "p1".into(),
            source: "github".into(),
            state: state.into(),
            last_ok,
            last_try: last_ok,
            error: error.map(String::from),
            fix: None,
            next_try: None,
            account: None,
        }
    }

    fn failing(id: &str, pr: &str, opened_at: i64) -> Item {
        Item {
            id: id.into(),
            kind: "pipeline_failed".into(),
            source: "github".into(),
            project_id: "p1".into(),
            pr: Some(pr.into()),
            ticket: None,
            title: "Checks failing: tests".into(),
            detail: "tests: failure".into(),
            who: None,
            url: None,
            path: None,
            line: None,
            at: "2026-11-08T08:00:00Z".into(),
            state: "open".into(),
            opened_at,
            closed_at: None,
            snoozed_until: None,
            bot: false,
        }
    }

    fn workspace(db: &Db) {
        db.insert_project(&Project { id: "p1".into(), name: "shop".into(), path: "/code/shop".into(), live_workspace_id: None, created_at: 1 })
            .unwrap();
        db.insert_workspace(&Workspace {
            id: "w1".into(),
            project_id: "p1".into(),
            name: "totals".into(),
            path: "/ws/shop/totals".into(),
            branch: "feature/SH-21533-totals".into(),
            base: Some("development".into()),
            session_id: "s-w1".into(),
            transcript_path: None,
            claude_config_dir: None,
            adopted: false,
            created_at: 1,
        })
        .unwrap();
    }

    #[test]
    fn new_failing_checks_on_a_pr_whose_session_is_in_a_workspace_are_queued_once() {
        let (_d, db) = db();
        workspace(&db);
        let read_at = now_ms();
        db.work_put_prs("p1", &[l::pr(2395, "feature/SH-21533-totals", "SH-21533 Totals"), l::pr(2437, "fix/other", "Other")], read_at).unwrap();
        db.work_link("pr:Org/shop#2395", "workspace:w1", "branch", "its workspace", read_at).unwrap();
        db.work_put_item(&failing("checks:Org/shop#2395:abc", "Org/shop#2395", read_at)).unwrap(); // new in this read
        db.work_put_item(&failing("checks:Org/shop#2395:old", "Org/shop#2395", read_at - 60_000)).unwrap(); // from before
        db.work_put_item(&failing("checks:Org/shop#2437:def", "Org/shop#2437", read_at)).unwrap(); // no session in Enkidu
        let ok = health("ok", Some(read_at), None);

        // off: nothing; on: only the new failure on the PR with a workspace
        assert_eq!(after_github(&db, "p1", false, &ok, read_at).unwrap(), 0);
        assert_eq!(after_github(&db, "p1", true, &ok, read_at).unwrap(), 1);
        let runs = db.work_workflow_take(read_at + 1).unwrap();
        assert_eq!(runs.iter().map(|r| r.subject.as_str()).collect::<Vec<_>>(), ["checks:Org/shop#2395:abc"]);
        assert_eq!(runs[0].workflow, FAILED_CHECKS);
        // never twice: not queued again, and taken runs aren't handed out again
        assert_eq!(after_github(&db, "p1", true, &ok, read_at).unwrap(), 0);
        assert!(db.work_workflow_take(read_at + 2).unwrap().is_empty());
        db.work_workflow_finish(runs[0].id, "done", "Sent to Claude in totals", read_at + 3).unwrap();
        let last = &db.work_workflow_runs("p1", 5).unwrap()[0];
        assert_eq!((last.state.as_str(), last.text.as_str()), ("done", "Sent to Claude in totals"));
        assert!(db.work_workflow_finish(runs[0].id, "pending", "", 0).is_err(), "a run ends done, skipped or failed");
    }

    #[test]
    fn a_session_linked_to_the_pr_counts_when_it_is_in_a_workspace() {
        let (_d, db) = db();
        workspace(&db);
        let read_at = now_ms();
        db.work_put_prs("p1", &[l::pr(2395, "hotfix/x", "X")], read_at).unwrap();
        db.work_put_item(&failing("checks:Org/shop#2395:abc", "Org/shop#2395", read_at)).unwrap();
        // made by a session outside Enkidu: not sent (the window would have nowhere to paste it)
        l::session(&db, "warp1", "/code/shop", "hook");
        db.work_link("pr:Org/shop#2395", "session:warp1", "hook:pr-create", "created it", read_at).unwrap();
        assert_eq!(after_github(&db, "p1", true, &health("ok", Some(read_at), None), read_at).unwrap(), 0);
        // the workspace's own session made it: sent
        db.work_session_seen(&crate::work::store::SessionSeen {
            id: "s-w1".into(),
            config_dir: "/h/.claude".into(),
            cwd: "/ws/shop/totals".into(),
            title: None,
            workspace_id: Some("w1".into()),
            first_at: 1,
            last_at: 2,
            source: "workspace".into(),
        })
        .unwrap();
        db.work_link("pr:Org/shop#2395", "session:s-w1", "hook:pr-create", "created it", read_at).unwrap();
        assert_eq!(after_github(&db, "p1", true, &health("ok", Some(read_at), None), read_at).unwrap(), 1);
    }

    #[test]
    fn never_on_stale_or_half_read_data() {
        let (_d, db) = db();
        workspace(&db);
        let read_at = now_ms();
        db.work_put_prs("p1", &[l::pr(2395, "feature/SH-21533-totals", "SH-21533 Totals")], read_at).unwrap();
        db.work_link("pr:Org/shop#2395", "workspace:w1", "branch", "its workspace", read_at).unwrap();
        db.work_put_item(&failing("checks:Org/shop#2395:abc", "Org/shop#2395", read_at)).unwrap();
        // the read failed (a page missing, logged out…): the last good read is older than this one
        assert_eq!(after_github(&db, "p1", true, &health("error", Some(read_at - 300_000), Some("gh isn't signed in")), read_at).unwrap(), 0);
        // an older good read isn't this read
        assert_eq!(after_github(&db, "p1", true, &health("ok", Some(read_at - 300_000), None), read_at).unwrap(), 0);
        assert_eq!(after_github(&db, "p1", true, &health("off", None, None), read_at).unwrap(), 0);

        // the morning summary: every source the project uses read fully just now, or it says which can't be
        let t = 1_000_000;
        let gh = health("ok", Some(t), None);
        let mut jira = health("error", Some(t - 600_000), Some("Jira: twg isn't signed in"));
        jira.source = "jira".into();
        jira.fix = Some("Run `twg login --force`.".into());
        assert_eq!(morning_ready(Some(&gh), Some(&jira), t), Err("Jira can't be read: twg isn't signed in. Run `twg login --force`.".into()));
        assert_eq!(morning_ready(Some(&health("ok", Some(t - 1), None)), None, t), Err("GitHub can't be read: it wasn't read".into()));
        assert_eq!(morning_ready(Some(&gh), None, t), Ok(()), "Jira off for the project: GitHub alone");
        jira = health("ok", Some(t + 5), None);
        assert_eq!(morning_ready(Some(&gh), Some(&jira), t), Ok(()));
        assert!(morning_ready(None, None, t).unwrap_err().contains("neither"));
    }

    #[test]
    fn the_morning_summary_runs_once_a_day_at_or_after_its_time() {
        assert_eq!(minutes_of("09:00"), Some(540));
        assert_eq!(minutes_of("9:05"), Some(545));
        assert_eq!(minutes_of("23:59"), Some(1439));
        for bad in ["24:00", "09:60", "9", "09:5", "", "nine", "-1:00", "09:00:00"] {
            assert_eq!(minutes_of(bad), None, "{bad}");
        }
        assert!(!morning_due(true, "09:00", 8 * 60 + 59, false), "not before its time");
        assert!(morning_due(true, "09:00", 9 * 60, false));
        assert!(morning_due(true, "09:00", 14 * 60, false), "Enkidu opened later that day: it runs then");
        assert!(!morning_due(true, "09:00", 14 * 60, true), "once a day");
        assert!(!morning_due(false, "09:00", 14 * 60, false), "off by default");
        assert!(!morning_due(true, "late", 14 * 60, false));
        let (day, minutes) = local_now();
        assert!(day.len() == 10 && day.starts_with("20") && minutes < 1440);
    }

    #[test]
    fn runs_headless_with_its_prompt_on_stdin_its_own_folder_and_environment_and_a_time_limit() {
        let dir = tempfile::tempdir().unwrap();
        let sh = std::path::Path::new("/bin/sh");
        let env = vec![("PATH".to_string(), "/usr/bin:/bin".to_string()), ("ONLY".to_string(), "this".to_string())];
        let args = ["-c".to_string(), "cat; echo; echo \"$ONLY ${HOME:-no home}\"; pwd -P; echo oops >&2; exit 3".to_string()];
        let out = run_headless(sh, &args, dir.path(), &env, "the prompt", std::time::Duration::from_secs(10)).unwrap();
        let lines: Vec<&str> = out.stdout.lines().collect();
        assert_eq!(&lines[..2], ["the prompt", "this no home"], "exactly the environment it was given");
        assert_eq!(std::fs::canonicalize(lines[2]).unwrap(), std::fs::canonicalize(dir.path()).unwrap());
        assert_eq!((out.code, out.stderr.trim()), (3, "oops"));
        // a long answer doesn't block it
        let big = ["-c".to_string(), "head -c 300000 /dev/zero | tr '\\0' x".to_string()];
        assert_eq!(run_headless(sh, &big, dir.path(), &env, "", std::time::Duration::from_secs(10)).unwrap().stdout.len(), 300_000);
        // stopped when the time is up
        let slow = ["-c".to_string(), "sleep 5".to_string()];
        let started = std::time::Instant::now();
        let e = run_headless(sh, &slow, dir.path(), &env, "", std::time::Duration::from_millis(300)).unwrap_err();
        assert!(e.to_string().contains("took longer") && started.elapsed() < std::time::Duration::from_secs(4));
    }

    #[test]
    fn the_morning_claude_is_read_only_whatever_your_own_flags_say() {
        let args = morning_args();
        let at = |flag: &str| args.iter().position(|a| a == flag).map(|i| args[i + 1].as_str());
        assert_eq!(args[0], "-p", "print mode: the prompt comes on stdin, and nothing can be approved by hand");
        assert!(args.contains(&"--restricted".to_string()) && args.contains(&"--strict-mcp-config".to_string()));
        assert_eq!(at("--tools"), Some("Bash,Read,Grep,Glob"), "no Edit, no Write");
        let allowed = at("--allowedTools").unwrap();
        assert!(allowed.split(',').any(|t| t == "Bash(enkidu:*)"));
        for t in allowed.split(',') {
            let ok = t == "Read" || t == "Grep" || t == "Glob" || t == "Bash(enkidu:*)" || t.starts_with("Bash(twg jira workitem ") || t.starts_with("Bash(gh pr ");
            assert!(ok, "only read commands: {t}");
            assert!(!t.contains("gh api") && !t.contains("create") && !t.contains("edit") && !t.contains("comment add"), "{t}");
        }
        assert!(!args.iter().any(|a| a.contains("skip-permissions") || a.contains("bypass")), "never skips permissions");
        assert!(MORNING_PROMPT.contains("enkidu note --ticket KEY") && MORNING_PROMPT.contains("read-only"));
    }
}
