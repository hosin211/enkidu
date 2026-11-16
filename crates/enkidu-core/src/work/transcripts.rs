//! The transcript index: which Claude sessions worked on which branches, PRs and tickets, read from the sessions
//! Claude saves (`<config>/projects/<folder>/<session>.jsonl`). It covers the sessions started outside Enkidu
//! (Warp) and the ones from before its hooks existed.
//!
//! Incremental: the files only grow, so each is read from where the last scan stopped. Most lines are passed over
//! with a few substring checks; only the few that can matter (a git step, a prompt, a title) are parsed. Nothing of
//! what was said is kept: only branch names, PR and ticket references, commit ids, the folder and the title.

use anyhow::Result;
use memchr::memmem;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use super::parse::{self, GitOps};
use super::store::{SessionSeen, TranscriptState};
use crate::db::Db;

/// What one read of a file found.
#[derive(Debug, Default, PartialEq)]
pub struct Found {
    pub session_id: String,
    pub config_dir: String,
    pub cwd: Option<String>,
    pub title: Option<String>,
    pub first_at: Option<i64>,
    pub last_at: Option<i64>,
    /// (branch, worked/pushed/committed, when)
    pub branches: Vec<(String, String, i64)>,
    /// (`pr:…` or `ticket:…`, created/edited/named…, when)
    pub mentions: Vec<(String, String, i64)>,
    /// (sha, branch, when)
    pub commits: Vec<(String, Option<String>, i64)>,
}

impl Found {
    fn branch(&mut self, b: &str, detail: &str, at: i64) {
        if b.is_empty() || b == "HEAD" {
            return;
        }
        if !self.branches.iter().any(|(x, d, _)| x == b && d == detail) {
            self.branches.push((b.to_string(), detail.to_string(), at));
        }
    }

    fn mention(&mut self, target: String, detail: &str, at: i64) {
        if !self.mentions.iter().any(|(t, d, _)| *t == target && d == detail) {
            self.mentions.push((target, detail.to_string(), at));
        }
    }

    fn ops(&mut self, ops: GitOps, at: i64) {
        for (target, action) in ops.prs {
            self.mention(format!("pr:{target}"), &action, at);
        }
        for b in ops.pushed {
            self.branch(&b, "pushed", at);
        }
        for (sha, branch) in ops.commits {
            if let Some(b) = &branch {
                self.branch(b, "committed", at);
            }
            if !self.commits.iter().any(|(s, _, _)| *s == sha) {
                self.commits.push((sha, branch, at));
            }
        }
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct Stats {
    pub files: usize,
    pub read: usize,
    pub bytes: u64,
    pub unreadable: usize,
}

fn has(line: &[u8], needle: &[u8]) -> bool {
    memmem::find(line, needle).is_some()
}

/// The text value right after `"key":"` in a raw line (the first one), unescaped.
pub(crate) fn string_after(line: &[u8], key: &[u8]) -> Option<String> {
    let start = memmem::find(line, key)? + key.len();
    let mut i = start;
    let mut escaped = false;
    while i < line.len() {
        match line[i] {
            b'\\' => {
                escaped = true;
                i += 2;
                continue;
            }
            b'"' => break,
            _ => i += 1,
        }
    }
    if i >= line.len() {
        return None; // no closing quote: a broken line
    }
    let raw = line.get(start..i)?;
    if escaped {
        let quoted = format!("\"{}\"", std::str::from_utf8(raw).ok()?);
        serde_json::from_str(&quoted).ok()
    } else {
        std::str::from_utf8(raw).ok().map(String::from)
    }
}

/// Reads one line of a saved session.
fn scan_line(line: &[u8], pending: &mut BTreeMap<String, String>, f: &mut Found) {
    let at = string_after(line, b"\"timestamp\":\"").and_then(|t| parse::parse_time(&t));
    if let Some(at) = at {
        f.first_at = Some(f.first_at.map_or(at, |x| x.min(at)));
        f.last_at = Some(f.last_at.map_or(at, |x| x.max(at)));
    }
    let at = at.or(f.last_at).unwrap_or(0);
    if let Some(b) = string_after(line, b"\"gitBranch\":\"") {
        f.branch(&b, "worked", at);
    }
    if let Some(cwd) = string_after(line, b"\"cwd\":\"").filter(|c| !c.is_empty()) {
        f.cwd = Some(cwd);
    }
    if has(line, b"\"type\":\"custom-title\"") {
        if let Some(t) = parse(line).and_then(|v| v.get("customTitle").and_then(Value::as_str).map(String::from)) {
            f.title = Some(t).filter(|t| !t.trim().is_empty());
        }
    }
    let result_of = |line: &[u8]| string_after(line, b"\"tool_use_id\":\"");
    if has(line, b"\"gitOperation\"") {
        // Claude Code's record counts only when the command it answers really ran gh or git (see `confirmed`)
        if let Some(command) = result_of(line).and_then(|id| pending.remove(&id)) {
            if let Some(op) = parse(line).and_then(|v| v.pointer("/toolUseResult/gitOperation").cloned()) {
                f.ops(parse::confirmed(parse::from_git_operation(&op), &command), at);
            }
        }
    } else if !pending.is_empty() && has(line, b"\"tool_use_id\"") {
        if let Some(command) = result_of(line).and_then(|id| pending.remove(&id)) {
            if let Some(v) = parse(line) {
                let out = |k: &str| v.pointer(&format!("/toolUseResult/{k}")).and_then(Value::as_str).unwrap_or("").to_string();
                let (mut stdout, stderr) = (out("stdout"), out("stderr"));
                if stdout.is_empty() {
                    // older sessions: the result only as the message's text
                    stdout = v.pointer("/message/content/0/content").and_then(Value::as_str).unwrap_or("").to_string();
                }
                f.ops(parse::from_command_output(&command, &stdout, &stderr), at);
            }
        }
    }
    if has(line, b"\"type\":\"assistant\"") && has(line, b"\"name\":\"Bash\"") && (has(line, b"gh pr") || has(line, b"git ")) {
        if pending.len() > 500 {
            pending.clear(); // calls whose results never came (interrupted): don't keep them forever
        }
        if let Some(v) = parse(line) {
            for block in v.pointer("/message/content").and_then(Value::as_array).into_iter().flatten() {
                let bash = block.get("type").and_then(Value::as_str) == Some("tool_use") && block.get("name").and_then(Value::as_str) == Some("Bash");
                let command = block.pointer("/input/command").and_then(Value::as_str).unwrap_or("");
                if let (true, Some(id)) = (bash && parse::interesting_command(command), block.get("id").and_then(Value::as_str)) {
                    pending.insert(id.to_string(), command.to_string());
                }
            }
        }
    }
    if has(line, b"\"type\":\"user\"")
        && !has(line, b"\"tool_use_id\"")
        && !has(line, b"\"isMeta\":true")
        && !has(line, b"\"isCompactSummary\":true")
        && !has(line, b"\"isSidechain\":true")
    {
        if let Some(v) = parse(line).filter(|v| v.get("type").and_then(Value::as_str) == Some("user")) {
            let text = v.pointer("/message/content").map(parse::typed_text).unwrap_or_default();
            for key in parse::ticket_keys(&text) {
                f.mention(format!("ticket:{key}"), "named", at);
            }
            for r in parse::pr_refs(&text) {
                f.mention(format!("pr:{r}"), "named", at);
            }
        }
    }
}

fn parse(line: &[u8]) -> Option<Value> {
    serde_json::from_slice(line).ok()
}

/// Reads a file from where the last scan stopped, up to its last complete line.
pub fn read_from(path: &Path, st: &mut TranscriptState, config_dir: &str, session_id: &str) -> Result<Found> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(st.offset.max(0) as u64))?;
    let mut reader = BufReader::with_capacity(1 << 20, file);
    let mut line = Vec::with_capacity(1 << 16);
    let mut found = Found { session_id: session_id.into(), config_dir: config_dir.into(), ..Default::default() };
    loop {
        line.clear();
        let n = reader.read_until(b'\n', &mut line)?;
        if n == 0 || line.last() != Some(&b'\n') {
            break; // the end, or a line still being written: read it next time
        }
        st.offset += n as i64;
        scan_line(&line, &mut st.pending, &mut found);
    }
    Ok(found)
}

/// Saves what a read found.
pub fn apply(db: &Db, f: &Found, mtime: i64) -> Result<()> {
    db.work_session_seen(&SessionSeen {
        id: f.session_id.clone(),
        config_dir: f.config_dir.clone(),
        cwd: f.cwd.clone().unwrap_or_default(),
        title: f.title.clone(),
        workspace_id: None,
        first_at: f.first_at.unwrap_or(mtime),
        last_at: f.last_at.unwrap_or(mtime),
        source: "transcript".into(),
    })?;
    for (b, detail, at) in &f.branches {
        db.work_session_branch(&f.session_id, b, "transcript", detail, *at)?;
    }
    for (target, detail, at) in &f.mentions {
        db.work_mention(&f.session_id, target, "transcript", detail, *at)?;
    }
    for (sha, branch, at) in &f.commits {
        db.work_commit(sha, &f.session_id, branch.as_deref(), "transcript", *at)?;
    }
    Ok(())
}

fn mtime_ms(meta: &std::fs::Metadata) -> i64 {
    meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Scans every saved session changed since `since` (ms) in the given Claude folders (`~/.claude`, …).
pub fn scan(db: &Db, config_dirs: &[PathBuf], since: i64) -> Result<Stats> {
    let mut stats = Stats::default();
    for config in config_dirs {
        let Ok(folders) = std::fs::read_dir(config.join("projects")) else { continue };
        let config_dir = config.to_string_lossy().to_string();
        for folder in folders.flatten() {
            if !folder.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let Ok(files) = std::fs::read_dir(folder.path()) else { continue };
            for entry in files.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                    continue;
                }
                let Ok(meta) = entry.metadata() else { continue };
                let mtime = mtime_ms(&meta);
                if !meta.is_file() || mtime < since {
                    continue;
                }
                stats.files += 1;
                let key = path.to_string_lossy().to_string();
                let size = meta.len() as i64;
                let mut st = db.work_transcript(&key)?.unwrap_or_default();
                if st.size == size && st.mtime == mtime {
                    continue;
                }
                if size < st.offset {
                    st = TranscriptState::default(); // rewritten: read it again
                }
                let session_id = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                let before = st.offset;
                // a file that can't be read (gone since the listing, no permission) is skipped, not the whole scan
                let found = match read_from(&path, &mut st, &config_dir, &session_id) {
                    Ok(f) => f,
                    Err(_) => {
                        stats.unreadable += 1;
                        continue;
                    }
                };
                apply(db, &found, mtime)?;
                st.session_id = session_id;
                st.size = size;
                st.mtime = mtime;
                db.work_save_transcript(&key, &st)?;
                stats.read += 1;
                stats.bytes += (st.offset - before).max(0) as u64;
            }
        }
    }
    Ok(stats)
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::work::store::tests::db;
    use serde_json::json;
    use std::io::Write;

    /// Lines shaped like Claude Code's (same keys, same order of the parts that matter).
    pub fn line(kind: &str, branch: &str, cwd: &str, at: &str, extra: Value) -> String {
        let mut v = json!({
            "parentUuid": null, "isSidechain": false, "userType": "external", "cwd": cwd, "sessionId": "s1",
            "version": "2.1.318", "gitBranch": branch, "type": kind, "uuid": "u", "timestamp": at
        });
        for (k, x) in extra.as_object().unwrap() {
            v[k] = x.clone();
        }
        format!("{}\n", serde_json::to_string(&v).unwrap())
    }

    pub fn prompt(branch: &str, cwd: &str, at: &str, text: &str) -> String {
        line("user", branch, cwd, at, json!({"message": {"role": "user", "content": text}}))
    }

    pub fn tool_use(branch: &str, cwd: &str, at: &str, id: &str, command: &str) -> String {
        line("assistant", branch, cwd, at, json!({"message": {"role": "assistant", "content": [
            {"type": "tool_use", "id": id, "name": "Bash", "input": {"command": command, "description": "x"}}]}}))
    }

    pub fn tool_result(branch: &str, cwd: &str, at: &str, id: &str, stdout: &str, op: Option<Value>) -> String {
        let mut result = json!({"stdout": stdout, "stderr": "", "interrupted": false, "isImage": false, "noOutputExpected": false});
        if let Some(op) = op {
            result["gitOperation"] = op;
        }
        line("user", branch, cwd, at, json!({
            "message": {"role": "user", "content": [{"tool_use_id": id, "type": "tool_result", "content": stdout, "is_error": false}]},
            "toolUseResult": result
        }))
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
        f.write_all(text.as_bytes()).unwrap();
    }

    #[test]
    fn finds_branches_prs_tickets_and_commits_and_nothing_said() {
        let cwd = "/code/shop";
        let mut text = String::new();
        text += &line("custom-title", "", "", "", json!({"customTitle": "round the totals"}));
        text += &prompt("development", cwd, "2026-11-05T10:00:00Z", "QA said SH-21533 fails, see https://github.com/Org/shop/pull/2326");
        text += &line("user", "development", cwd, "2026-11-05T10:00:01Z", json!({"isMeta": true, "message": {"content": "SH-1 meta"}}));
        text += &line("user", "development", cwd, "2026-11-05T10:00:02Z", json!({"isCompactSummary": true, "message": {"content": "SH-2 summary"}}));
        text += &tool_use("development", cwd, "2026-11-05T10:01:00Z", "t1", "git push origin HEAD:hotfix/round-totals");
        text += &tool_result("development", cwd, "2026-11-05T10:01:05Z", "t1", "", Some(json!({"push": {"branch": "hotfix/round-totals"}})));
        text += &tool_use("development", cwd, "2026-11-05T10:02:00Z", "t2", "gh pr create --base development");
        text += &tool_result("development", cwd, "2026-11-05T10:02:05Z", "t2", "https://github.com/Org/shop/pull/2395\n",
            Some(json!({"pr": {"number": 2395, "url": "https://github.com/Org/shop/pull/2395", "action": "created"}})));
        // an older Claude Code: no gitOperation, so the command and its output tell
        text += &tool_use("feature/SH-21533-b", cwd, "2026-11-05T11:00:00Z", "t3", "git commit -m 'Totals'");
        text += &tool_result("feature/SH-21533-b", cwd, "2026-11-05T11:00:02Z", "t3", "[feature/SH-21533-b 3f2c1ab] Totals\n", None);
        // a search through notes that quotes the words next to a PR link: Claude Code guesses "created", it wasn't
        text += &tool_use("feature/SH-21533-b", cwd, "2026-11-05T11:05:00Z", "t4", "command grep -F 'gh pr create' notes.jsonl");
        text += &tool_result("feature/SH-21533-b", cwd, "2026-11-05T11:05:01Z", "t4", "https://github.com/Org/shop/pull/1\n",
            Some(json!({"pr": {"number": 1, "url": "https://github.com/Org/shop/pull/1", "action": "created"}})));
        // a git step that Claude Code recorded for a call the index never saw (not a Bash git call)
        text += &tool_result("feature/SH-21533-b", cwd, "2026-11-05T11:06:01Z", "t9", "", Some(json!({"push": {"branch": "feature/z"}})));
        let f = scan_text(&text);
        assert_eq!(f.title.as_deref(), Some("round the totals"));
        assert_eq!(f.cwd.as_deref(), Some(cwd));
        let branches: Vec<_> = f.branches.iter().map(|(b, d, _)| format!("{b}:{d}")).collect();
        assert_eq!(branches, ["development:worked", "hotfix/round-totals:pushed", "feature/SH-21533-b:worked", "feature/SH-21533-b:committed"]);
        let mentions: Vec<_> = f.mentions.iter().map(|(t, d, _)| format!("{t}:{d}")).collect();
        assert_eq!(mentions, ["ticket:SH-21533:named", "pr:Org/shop#2326:named", "pr:Org/shop#2395:created"], "meta and summaries don't count");
        assert_eq!(f.commits, [("3f2c1ab".to_string(), Some("feature/SH-21533-b".to_string()), parse::parse_time("2026-11-05T11:00:02Z").unwrap())]);
        assert_eq!(f.first_at, parse::parse_time("2026-11-05T10:00:00Z"));
    }

    fn scan_text(text: &str) -> Found {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.jsonl");
        write(&p, text);
        let mut st = TranscriptState::default();
        read_from(&p, &mut st, "/h/.claude", "s1").unwrap()
    }

    #[test]
    fn reads_only_what_was_added_and_waits_for_a_line_being_written() {
        let (_d, db) = db();
        let home = tempfile::tempdir().unwrap();
        let config = home.path().join(".claude-work");
        let file = config.join("projects/-code-shop/abc.jsonl");
        write(&file, &tool_use("feature/a", "/code/shop", "2026-11-05T10:00:00Z", "t1", "gh pr create"));
        let half = tool_result("feature/a", "/code/shop", "2026-11-05T10:00:03Z", "t1", "https://github.com/Org/shop/pull/7\n", None);
        let (first, second) = half.split_at(40);
        write(&file, first);
        let stats = scan(&db, std::slice::from_ref(&config), 0).unwrap();
        assert_eq!((stats.files, stats.read), (1, 1));
        assert!(db.work_mention_rows().unwrap().is_empty(), "the half line waits");
        assert_eq!(scan(&db, std::slice::from_ref(&config), 0).unwrap().read, 0, "nothing new: not read again");
        write(&file, second);
        scan(&db, std::slice::from_ref(&config), 0).unwrap();
        let mentions = db.work_mention_rows().unwrap();
        assert_eq!(mentions.len(), 1, "the tool call from the first read is matched with its result from the second");
        assert_eq!((mentions[0].0.as_str(), mentions[0].1.as_str(), mentions[0].3.as_str()), ("abc", "pr:Org/shop#7", "created"));
        let s = &db.work_sessions().unwrap()[0];
        assert_eq!((s.id.as_str(), s.config_dir.as_str(), s.source.as_str()), ("abc", config.to_str().unwrap(), "transcript"));
        assert_eq!(s.branches, ["feature/a"]);
    }

    #[test]
    fn a_rewritten_file_is_read_again_and_old_files_are_skipped() {
        let (_d, db) = db();
        let home = tempfile::tempdir().unwrap();
        let config = home.path().join(".claude");
        let file = config.join("projects/-code-shop/abc.jsonl");
        write(&file, &prompt("feature/a", "/code/shop", "2026-11-05T10:00:00Z", "SH-1 and SH-2"));
        write(&file, &prompt("feature/a", "/code/shop", "2026-11-05T10:00:01Z", "more"));
        scan(&db, std::slice::from_ref(&config), 0).unwrap();
        std::fs::write(&file, prompt("feature/b", "/code/shop", "2026-11-05T12:00:00Z", "SH-3")).unwrap();
        scan(&db, std::slice::from_ref(&config), 0).unwrap();
        let targets: Vec<String> = db.work_mention_rows().unwrap().into_iter().map(|m| m.1).collect();
        assert!(targets.contains(&"ticket:SH-3".to_string()));
        assert_eq!(scan(&db, &[config], i64::MAX).unwrap().files, 0, "files older than the window are left alone");
    }

    #[test]
    fn broken_lines_and_unreadable_files_are_passed_over() {
        let (_d, db) = db();
        let home = tempfile::tempdir().unwrap();
        let config = home.path().join(".claude");
        let good = config.join("projects/-code-shop/good.jsonl");
        write(&good, "{\"type\":\"user\",\"message\":{\"content\":\"SH-1 cut off mid\n");
        write(&good, "not json at all \"gitBranch\":\"\n");
        write(&good, &prompt("feature/a", "/code/shop", "2026-11-05T10:00:00Z", "SH-21533 please"));
        let locked = config.join("projects/-code-shop/locked.jsonl");
        write(&locked, &prompt("feature/b", "/code/shop", "2026-11-05T10:00:00Z", "SH-7"));
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        let stats = scan(&db, &[config.clone(), home.path().join("missing")], 0).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!((stats.read, stats.unreadable), (1, 1));
        let targets: Vec<String> = db.work_mention_rows().unwrap().into_iter().map(|m| m.1).collect();
        assert_eq!(targets, ["ticket:SH-21533"], "the broken lines gave nothing, the good one was read");
        assert_eq!(scan(&db, &[config], 0).unwrap().read, 1, "the file that couldn't be read is tried again");
    }

    #[test]
    fn escaped_values_are_read_whole() {
        let l = br#"{"cwd":"/code/a \"b\"","gitBranch":"x"}"#;
        assert_eq!(string_after(l, b"\"cwd\":\"").as_deref(), Some("/code/a \"b\""));
        assert_eq!(string_after(l, b"\"gitBranch\":\"").as_deref(), Some("x"));
        assert_eq!(string_after(l, b"\"nope\":\""), None);
    }
}
