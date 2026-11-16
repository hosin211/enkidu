//! Claude started outside Enkidu (from an alias in another terminal, say) tells it what it works on through the
//! hooks added from
//! Settings: each runs `enkidu event`, which appends one line to `events.jsonl` in the app's data folder (nothing of what
//! you typed, only the ticket keys and PR links in it), so nothing is lost while the app is closed. The app reads what
//! was added since last time, at start and every minute, and treats it like its own hooks.

use anyhow::Result;
use serde_json::Value;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;

use super::hookev::{self, Payload};
use super::parse::GitOps;
use super::store::TranscriptState;
use crate::db::Db;

#[derive(Debug, Default, PartialEq)]
pub struct Ingested {
    pub events: usize,
    /// A push or a finished turn: read GitHub and Jira again
    pub refresh: bool,
}

fn strings(v: &Value, k: &str) -> Vec<String> {
    v.get(k).and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(String::from).collect()
}

fn pairs(v: &Value, k: &str) -> Vec<(String, Option<String>)> {
    v.get(k)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|p| Some((p.get(0)?.as_str()?.to_string(), p.get(1).and_then(Value::as_str).map(String::from))))
        .collect()
}

/// One recorded event, applied like a hook from a Claude Enkidu started (without a workspace).
pub fn apply_line(db: &Db, line: &str) -> Result<Option<bool>> {
    let Ok(v) = serde_json::from_str::<Value>(line) else { return Ok(None) };
    if v.get("v").and_then(Value::as_i64) != Some(1) {
        return Ok(None);
    }
    let text = |k: &str| v.get(k).and_then(Value::as_str).map(String::from);
    let mentions = strings(&v, "mentions");
    let p = Payload {
        event: text("event").unwrap_or_default(),
        session_id: text("session_id").unwrap_or_default(),
        transcript_path: text("transcript_path"),
        cwd: text("cwd"),
        // the keys and links found in the prompt, which is all that was kept of it
        prompt: (!mentions.is_empty()).then(|| mentions.join(" ")),
        ops: GitOps {
            prs: pairs(&v, "prs").into_iter().map(|(a, b)| (a, b.unwrap_or_else(|| "worked".into()))).collect(),
            pushed: strings(&v, "pushed"),
            commits: pairs(&v, "commits"),
        },
        touched: v.get("touched").and_then(Value::as_bool).unwrap_or(false),
    };
    let at = v.get("at").and_then(Value::as_i64).unwrap_or_else(crate::db::now_ms);
    let branch = text("branch");
    let out = hookev::apply(db, None, &p, at, &|_| branch.clone())?;
    Ok(Some(out.refresh))
}

/// Reads what was added to the event log since the last time (lines still being written wait for the next read).
pub fn ingest(db: &Db, path: &Path) -> Result<Ingested> {
    let mut done = Ingested::default();
    let Ok(meta) = std::fs::metadata(path) else { return Ok(done) };
    let key = path.to_string_lossy().to_string();
    let mut st = db.work_transcript(&key)?.unwrap_or_default();
    let size = meta.len() as i64;
    if size == st.size {
        return Ok(done);
    }
    if size < st.offset {
        st = TranscriptState::default(); // the log was cleared: read it from the start
    }
    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::Start(st.offset.max(0) as u64))?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line)?;
        if n == 0 || !line.ends_with('\n') {
            break;
        }
        st.offset += n as i64;
        if let Some(refresh) = apply_line(db, line.trim())? {
            done.events += 1;
            done.refresh |= refresh;
        }
    }
    st.size = size;
    st.session_id = "events".into();
    db.work_save_transcript(&key, &st)?;
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::now_ms;
    use crate::work::links::{relink, tests as l};
    use crate::work::store::tests::db;
    use std::io::Write;

    #[test]
    fn events_from_claude_outside_enkidu_link_like_its_own_hooks() {
        let (dir, db) = db();
        db.work_put_prs("p1", &[l::pr(2395, "feature/SH-21533-totals", "SH-21533 Totals"), l::pr(2414, "hotfix/x", "X")], now_ms()).unwrap();
        let log = dir.path().join("events.jsonl");
        let mut f = std::fs::File::create(&log).unwrap();
        let line = |v: serde_json::Value| format!("{v}\n");
        f.write_all(line(serde_json::json!({"v": 1, "at": 10, "event": "SessionStart", "session_id": "warp1",
            "transcript_path": "/h/.claude-work/projects/-code-shop/warp1.jsonl", "cwd": "/code/shop", "branch": "feature/SH-21533-totals",
            "mentions": [], "prs": [], "pushed": [], "commits": []})).as_bytes()).unwrap();
        f.write_all(line(serde_json::json!({"v": 1, "at": 20, "event": "PostToolUse", "session_id": "warp1", "cwd": "/code/shop",
            "mentions": [], "prs": [["Org/shop#2414", "created"]], "pushed": ["hotfix/x"], "commits": []})).as_bytes()).unwrap();
        f.write_all(b"not json\n{\"v\": 9}\n{\"v\":1,\"event\":\"Stop\",\"session_id\":\"warp1\"").unwrap(); // the last line is still being written
        drop(f);
        let got = ingest(&db, &log).unwrap();
        assert_eq!(got, Ingested { events: 2, refresh: true }, "a push asks for a read; broken and unknown lines are passed over");
        relink(&db, &[l::shop()], now_ms(), &|_| None).unwrap();
        let mut links: Vec<String> = db.work_links().unwrap().iter().filter(|x| x.b == "session:warp1").map(|x| format!("{} {}", x.a, x.how)).collect();
        links.sort();
        assert_eq!(links, ["pr:Org/shop#2395 branch", "pr:Org/shop#2414 branch", "pr:Org/shop#2414 hook:pr-create"]);
        let s = db.work_sessions().unwrap().into_iter().find(|s| s.id == "warp1").unwrap();
        assert_eq!((s.config_dir.as_str(), s.source.as_str()), ("/h/.claude-work", "hook"));
        // nothing new: nothing read; the rest of the last line arrives: read
        assert_eq!(ingest(&db, &log).unwrap().events, 0);
        std::fs::OpenOptions::new().append(true).open(&log).unwrap().write_all(b",\"at\":30}\n").unwrap();
        assert_eq!(ingest(&db, &log).unwrap(), Ingested { events: 1, refresh: true });
    }
}
