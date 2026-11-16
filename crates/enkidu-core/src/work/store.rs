//! The Work index in SQLite (the tables are in `db.rs`). Everything here is small: tens of PRs and tickets, a few
//! hundred sessions and links.

use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use std::collections::{BTreeMap, HashMap};

use super::model::{is_card, Board, BoardPick, Event, Health, Item, Link, OwnTask, Pr, Session, Ticket, OWN_STATES};
use crate::db::Db;

/// Reads tasks of your own, each with its project's name (empty for a project that is gone), in the order
/// [`own_task_of`] takes the columns.
const OWN_TASKS: &str = "SELECT t.id, t.project_id, t.title, t.notes, t.state, t.created_at, t.updated_at, COALESCE(p.name, '') \
                         FROM work_own_tasks t LEFT JOIN projects p ON p.id = t.project_id";

fn own_task_of(row: &rusqlite::Row) -> rusqlite::Result<OwnTask> {
    Ok(OwnTask {
        id: row.get(0)?,
        project_id: row.get(1)?,
        title: row.get(2)?,
        notes: row.get(3)?,
        state: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
        project: row.get(7)?,
    })
}

/// What one source (a hook, the transcript index, a workspace) knows about a session.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SessionSeen {
    pub id: String,
    pub config_dir: String,
    pub cwd: String,
    pub title: Option<String>,
    pub workspace_id: Option<String>,
    pub first_at: i64,
    pub last_at: i64,
    /// workspace, hook or transcript
    pub source: String,
}

/// Saved progress of the transcript index in one file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TranscriptState {
    pub session_id: String,
    pub size: i64,
    pub mtime: i64,
    pub offset: i64,
    /// Tool calls waiting for their result: tool use id → command
    pub pending: BTreeMap<String, String>,
}

/// A thing's place in a link: tickets (and tasks of your own, which are cards like them) come first, then PRs,
/// sessions, workspaces, so a pair is always stored once.
fn rank(thing: &str) -> u8 {
    match thing.split_once(':').map(|(k, _)| k) {
        Some("ticket") | Some("own") => 0,
        Some("pr") => 1,
        Some("session") => 2,
        Some("workspace") => 3,
        _ => 4,
    }
}

pub fn ordered<'a>(x: &'a str, y: &'a str) -> (&'a str, &'a str) {
    if (rank(x), x) <= (rank(y), y) {
        (x, y)
    } else {
        (y, x)
    }
}

impl Db {
    /// Adds what a source saw of a session: times widen, the latest cwd and title win, and the account folder is the
    /// one seen last (a conversation carried over to another account lives on there).
    pub fn work_session_seen(&self, s: &SessionSeen) -> Result<()> {
        self.conn().execute(
            "INSERT INTO work_sessions (id, config_dir, cwd, title, workspace_id, first_seen, last_seen, ended, source)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8)
             ON CONFLICT(id) DO UPDATE SET
               config_dir = CASE WHEN excluded.last_seen >= work_sessions.last_seen AND excluded.config_dir != ''
                                 THEN excluded.config_dir ELSE work_sessions.config_dir END,
               cwd = CASE WHEN excluded.last_seen >= work_sessions.last_seen AND excluded.cwd != ''
                          THEN excluded.cwd ELSE work_sessions.cwd END,
               title = COALESCE(excluded.title, work_sessions.title),
               workspace_id = COALESCE(excluded.workspace_id, work_sessions.workspace_id),
               first_seen = MIN(work_sessions.first_seen, excluded.first_seen),
               last_seen = MAX(work_sessions.last_seen, excluded.last_seen),
               source = CASE WHEN work_sessions.source = 'transcript' THEN excluded.source ELSE work_sessions.source END",
            params![s.id, s.config_dir, s.cwd, s.title, s.workspace_id, s.first_at, s.last_at, s.source],
        )?;
        Ok(())
    }

    pub fn work_session_ended(&self, id: &str, at: i64) -> Result<()> {
        self.conn().execute(
            "UPDATE work_sessions SET ended = 1, last_seen = MAX(last_seen, ?2) WHERE id = ?1",
            params![id, at],
        )?;
        Ok(())
    }

    /// A branch the session worked on. `how`: branch (known live) or transcript; `detail`: worked, pushed, committed.
    pub fn work_session_branch(&self, session: &str, branch: &str, how: &str, detail: &str, at: i64) -> Result<bool> {
        let n = self.conn().execute(
            "INSERT INTO work_session_branches (session_id, branch, how, detail, at) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(session_id, branch, how) DO UPDATE SET at = MIN(at, excluded.at)
             WHERE excluded.at < work_session_branches.at",
            params![session, branch, how, detail, at],
        )?;
        Ok(n > 0)
    }

    /// A PR or ticket the session named, created or worked on (`target` is `pr:owner/repo#N`, `pr:#N` when the repo
    /// is the session's own, or `ticket:KEY`).
    pub fn work_mention(&self, session: &str, target: &str, how: &str, detail: &str, at: i64) -> Result<bool> {
        let n = self.conn().execute(
            "INSERT OR IGNORE INTO work_mentions (session_id, target, how, detail, at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![session, target, how, detail, at],
        )?;
        Ok(n > 0)
    }

    pub fn work_commit(&self, sha: &str, session: &str, branch: Option<&str>, how: &str, at: i64) -> Result<bool> {
        let n = self.conn().execute(
            "INSERT OR IGNORE INTO work_commits (sha, session_id, branch, how, at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![sha, session, branch, how, at],
        )?;
        Ok(n > 0)
    }

    /// Every session with the branches it worked on (in the order it first did).
    pub fn work_sessions(&self) -> Result<Vec<Session>> {
        let conn = self.conn();
        let mut branches: HashMap<String, Vec<(i64, String)>> = HashMap::new();
        let mut stmt = conn.prepare("SELECT session_id, branch, MIN(at) FROM work_session_branches GROUP BY session_id, branch")?;
        for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))? {
            let (s, b, at) = row?;
            branches.entry(s).or_default().push((at, b));
        }
        let mut stmt = conn.prepare(
            "SELECT id, config_dir, cwd, title, workspace_id, first_seen, last_seen, source FROM work_sessions ORDER BY last_seen DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(Session {
                id: r.get(0)?,
                config_dir: r.get(1)?,
                cwd: r.get(2)?,
                title: r.get(3)?,
                branches: Vec::new(),
                workspace_id: r.get(4)?,
                first_seen: r.get(5)?,
                last_seen: r.get(6)?,
                source: r.get(7)?,
            })
        })?;
        let mut out: Vec<Session> = rows.collect::<rusqlite::Result<_>>()?;
        for s in &mut out {
            if let Some(mut b) = branches.remove(&s.id) {
                b.sort();
                s.branches = b.into_iter().map(|(_, b)| b).collect();
            }
        }
        Ok(out)
    }

    /// (session, branch, how, detail, at) for every branch a session worked on.
    pub fn work_branch_rows(&self) -> Result<Vec<(String, String, String, String, i64)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT session_id, branch, how, detail, at FROM work_session_branches")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// (session, target, how, detail, at)
    pub fn work_mention_rows(&self) -> Result<Vec<(String, String, String, String, i64)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT session_id, target, how, detail, at FROM work_mentions")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// (sha, session, how, at)
    pub fn work_commit_rows(&self) -> Result<Vec<(String, String, String, i64)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT sha, session_id, how, at FROM work_commits")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Links two things. A pair you (or Claude) unlinked stays unlinked: the rules don't bring it back.
    pub fn work_link(&self, x: &str, y: &str, how: &str, detail: &str, at: i64) -> Result<bool> {
        let (a, b) = ordered(x, y);
        let conn = self.conn();
        let explicit = how == "you" || how == "claude";
        if explicit {
            conn.execute("DELETE FROM work_links WHERE a = ?1 AND b = ?2 AND how = 'unlinked'", params![a, b])?;
        } else {
            let blocked: Option<i64> = conn
                .query_row("SELECT 1 FROM work_links WHERE a = ?1 AND b = ?2 AND how = 'unlinked'", params![a, b], |r| r.get(0))
                .optional()?;
            if blocked.is_some() {
                return Ok(false);
            }
        }
        let n = conn.execute(
            "INSERT INTO work_links (a, b, how, detail, at, removed) VALUES (?1, ?2, ?3, ?4, ?5, 0)
             ON CONFLICT(a, b, how, detail) DO UPDATE SET removed = 0 WHERE ?6 AND work_links.removed = 1",
            params![a, b, how, detail, at, explicit],
        )?;
        Ok(n > 0)
    }

    /// Marks every link between two things removed and remembers it, so no rule links them again.
    #[allow(dead_code)] // `enkidu unlink` and the page's unlink (Phase 5)
    pub fn work_unlink(&self, x: &str, y: &str, at: i64) -> Result<()> {
        let (a, b) = ordered(x, y);
        let conn = self.conn();
        conn.execute("UPDATE work_links SET removed = 1 WHERE a = ?1 AND b = ?2", params![a, b])?;
        conn.execute(
            "INSERT INTO work_links (a, b, how, detail, at, removed) VALUES (?1, ?2, 'unlinked', '', ?3, 1)
             ON CONFLICT(a, b, how, detail) DO UPDATE SET at = excluded.at",
            params![a, b, at],
        )?;
        Ok(())
    }

    /// Marks the links of something that is gone (a deleted workspace) removed; they stay in the history.
    pub fn work_remove_links_of(&self, thing: &str) -> Result<()> {
        self.conn().execute("UPDATE work_links SET removed = 1 WHERE a = ?1 OR b = ?1", params![thing])?;
        Ok(())
    }

    /// The links in use (not removed), oldest first.
    pub fn work_links(&self) -> Result<Vec<Link>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT a, b, how, detail, at FROM work_links WHERE removed = 0 ORDER BY at, a, b")?;
        let rows = stmt.query_map([], |r| {
            Ok(Link { a: r.get(0)?, b: r.get(1)?, how: r.get(2)?, detail: r.get(3)?, at: r.get(4)?, removed: false })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn work_transcript(&self, path: &str) -> Result<Option<TranscriptState>> {
        let conn = self.conn();
        Ok(conn
            .query_row(
                "SELECT session_id, size, mtime, offset, pending FROM work_transcripts WHERE path = ?1",
                [path],
                |r| {
                    let pending: String = r.get(4)?;
                    Ok(TranscriptState {
                        session_id: r.get(0)?,
                        size: r.get(1)?,
                        mtime: r.get(2)?,
                        offset: r.get(3)?,
                        pending: serde_json::from_str(&pending).unwrap_or_default(),
                    })
                },
            )
            .optional()?)
    }

    /// Where a session's conversation is saved, newest copy first: (its Claude folder, the file).
    pub fn work_transcript_of(&self, session_id: &str) -> Result<Option<(String, String)>> {
        let conn = self.conn();
        let path: Option<String> = conn
            .query_row(
                "SELECT path FROM work_transcripts WHERE session_id = ?1 ORDER BY mtime DESC LIMIT 1",
                [session_id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(found) = path.and_then(|p| Some((super::hookev::config_dir_of(&p)?, p))) {
            return Ok(Some(found));
        }
        // a session known only from its hooks (its file not scanned yet): where Claude Code keeps it, if it is there
        let seen: Option<(String, String)> = conn
            .query_row("SELECT config_dir, cwd FROM work_sessions WHERE id = ?1", [session_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        Ok(seen.and_then(|(config, cwd)| {
            let encoded: String = cwd.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
            let p = std::path::Path::new(&config).join("projects").join(encoded).join(format!("{session_id}.jsonl"));
            (!config.is_empty() && p.is_file()).then(|| (config, p.to_string_lossy().to_string()))
        }))
    }

    pub fn work_save_transcript(&self, path: &str, st: &TranscriptState) -> Result<()> {
        self.conn().execute(
            "INSERT INTO work_transcripts (path, session_id, size, mtime, offset, pending) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(path) DO UPDATE SET session_id = excluded.session_id, size = excluded.size,
               mtime = excluded.mtime, offset = excluded.offset, pending = excluded.pending",
            params![path, st.session_id, st.size, st.mtime, st.offset, serde_json::to_string(&st.pending)?],
        )?;
        Ok(())
    }

    /// Replaces a project's PRs from one full read: PRs it no longer returns keep their last record (merged long
    /// ago, or closed) but are no longer "seen".
    pub fn work_put_prs(&self, project_id: &str, prs: &[Pr], now: i64) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        for pr in prs {
            tx.execute(
                "INSERT INTO work_prs (ref, project_id, data, state, head, updated_at, seen_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(ref) DO UPDATE SET project_id = excluded.project_id, data = excluded.data, state = excluded.state,
                   head = excluded.head, updated_at = excluded.updated_at, seen_at = excluded.seen_at",
                params![pr.reference(), project_id, serde_json::to_string(pr)?, pr.state, pr.head, pr.updated_at, now],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn work_prs(&self) -> Result<Vec<(Pr, i64)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT data, seen_at FROM work_prs ORDER BY updated_at DESC")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        let mut out = Vec::new();
        for row in rows {
            let (data, seen) = row?;
            out.push((serde_json::from_str(&data)?, seen));
        }
        Ok(out)
    }

    pub fn work_put_tickets(&self, project_id: &str, tickets: &[Ticket], now: i64) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        for t in tickets {
            tx.execute(
                "INSERT INTO work_tickets (key, project_id, data, status, category, updated_at, seen_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(key) DO UPDATE SET project_id = excluded.project_id, data = excluded.data, status = excluded.status,
                   category = excluded.category, updated_at = excluded.updated_at, seen_at = excluded.seen_at",
                params![t.key, project_id, serde_json::to_string(t)?, t.status, t.category, t.updated_at, now],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn work_tickets(&self) -> Result<Vec<(Ticket, i64)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT data, seen_at FROM work_tickets ORDER BY updated_at DESC")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        let mut out = Vec::new();
        for row in rows {
            let (data, seen) = row?;
            out.push((serde_json::from_str(&data)?, seen));
        }
        Ok(out)
    }

    /// One project's tickets as last read.
    pub fn work_project_tickets(&self, project_id: &str) -> Result<Vec<Ticket>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT data FROM work_tickets WHERE project_id = ?1")?;
        let rows = stmt.query_map([project_id], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(serde_json::from_str(&row?)?);
        }
        Ok(out)
    }

    /// What was seen in each project's latest read (`work_tickets` or `work_prs`): (project, latest seen_at).
    pub fn work_latest_reads(&self, table: &str) -> Result<HashMap<String, i64>> {
        anyhow::ensure!(table == "work_tickets" || table == "work_prs", "no such table");
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!("SELECT project_id, MAX(seen_at) FROM {table} GROUP BY project_id"))?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Tickets (or PRs) with the project they were read for and when.
    pub fn work_ticket_rows(&self) -> Result<Vec<(Ticket, String, i64)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT data, project_id, seen_at FROM work_tickets ORDER BY updated_at DESC")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))?;
        let mut out = Vec::new();
        for row in rows {
            let (data, project, seen) = row?;
            out.push((serde_json::from_str(&data)?, project, seen));
        }
        Ok(out)
    }

    pub fn work_event(&self, subject: &str, kind: &str, text: &str, at: i64) -> Result<()> {
        self.conn().execute(
            "INSERT INTO work_events (subject, at, kind, text) VALUES (?1, ?2, ?3, ?4)",
            params![subject, at, kind, text],
        )?;
        Ok(())
    }

    /// Hides cards from the board, or shows them again. A card is a ticket (`ticket:SH-1`), a PR without one
    /// (`pr:owner/repo#2`) or a task of your own (`own:3`).
    pub fn work_set_hidden(&self, cards: &[String], hidden: bool, at: i64) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        for card in cards {
            anyhow::ensure!(is_card(card), "A card is a ticket, a PR or a task of your own, not {card}");
            if hidden {
                tx.execute("INSERT INTO work_hidden (subject, at) VALUES (?1, ?2) ON CONFLICT(subject) DO NOTHING", params![card, at])?;
            } else {
                tx.execute("DELETE FROM work_hidden WHERE subject = ?1", params![card])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// The hidden cards, the latest hidden last.
    pub fn work_hidden(&self) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT subject FROM work_hidden ORDER BY at, subject")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Your own tasks, in the order you made them.
    pub fn work_own_tasks(&self) -> Result<Vec<OwnTask>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!("{OWN_TASKS} ORDER BY t.id"))?;
        let rows = stmt.query_map([], own_task_of)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// One of your own tasks.
    pub fn work_own_task(&self, id: i64) -> Result<Option<OwnTask>> {
        let conn = self.conn();
        Ok(conn.query_row(&format!("{OWN_TASKS} WHERE t.id = ?1"), [id], own_task_of).optional()?)
    }

    /// Saves a task of your own: a new one (`id` none) in a project, or the words and the state of one that is there
    /// (its project stays). The title can't be empty, and the state is `todo`, `doing` or `done`. Returns the task as
    /// it is kept.
    pub fn work_save_own_task(&self, id: Option<i64>, project_id: &str, title: &str, notes: &str, state: &str, now: i64) -> Result<OwnTask> {
        let (title, notes) = (title.trim(), notes.trim());
        anyhow::ensure!(!title.is_empty(), "Give the task a title");
        anyhow::ensure!(OWN_STATES.contains(&state), "A task is to do, in progress or done, not {state}");
        let conn = self.conn();
        let id = match id {
            Some(id) => {
                let changed = conn.execute(
                    "UPDATE work_own_tasks SET title = ?2, notes = ?3, state = ?4, updated_at = ?5 WHERE id = ?1",
                    params![id, title, notes, state, now],
                )?;
                anyhow::ensure!(changed == 1, "That task is no longer there");
                id
            }
            None => {
                let known = conn.query_row("SELECT 1 FROM projects WHERE id = ?1", [project_id], |_| Ok(())).optional()?.is_some();
                anyhow::ensure!(known, "That project is no longer there");
                conn.execute(
                    "INSERT INTO work_own_tasks (project_id, title, notes, state, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
                    params![project_id, title, notes, state, now],
                )?;
                conn.last_insert_rowid()
            }
        };
        Ok(conn.query_row(&format!("{OWN_TASKS} WHERE t.id = ?1"), [id], own_task_of)?)
    }

    /// Deletes a task of your own, with what was kept about its card: that it was hidden, which Claudes were linked
    /// to it, and its timeline.
    pub fn work_delete_own_task(&self, id: i64) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let subject = format!("own:{id}");
        tx.execute("DELETE FROM work_own_tasks WHERE id = ?1", [id])?;
        tx.execute("DELETE FROM work_hidden WHERE subject = ?1", [&subject])?;
        tx.execute("DELETE FROM work_links WHERE a = ?1", [&subject])?;
        tx.execute("DELETE FROM work_events WHERE subject = ?1", [&subject])?;
        tx.commit()?;
        Ok(())
    }

    /// A project was removed: its own tasks go with it.
    pub fn work_delete_own_tasks_of(&self, project_id: &str) -> Result<()> {
        let ids: Vec<i64> = self.work_own_tasks()?.into_iter().filter(|task| task.project_id == project_id).map(|task| task.id).collect();
        for id in ids {
            self.work_delete_own_task(id)?;
        }
        Ok(())
    }

    /// The Jira board you picked for the Work page (none: everything read is on it).
    pub fn work_board(&self) -> Result<Option<Board>> {
        let saved: Option<String> = self.conn().query_row("SELECT value FROM work_prefs WHERE key = 'board'", [], |r| r.get(0)).optional()?;
        // a pick saved by another version that can't be read is no pick
        Ok(saved.and_then(|v| serde_json::from_str(&v).ok()))
    }

    pub fn work_set_board(&self, board: Option<&Board>) -> Result<()> {
        let conn = self.conn();
        match board {
            Some(b) => conn.execute(
                "INSERT INTO work_prefs (key, value) VALUES ('board', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![serde_json::to_string(b)?],
            )?,
            None => conn.execute("DELETE FROM work_prefs WHERE key = 'board'", [])?,
        };
        Ok(())
    }

    /// Jira's boards as last read, for the choice in the page's header.
    pub fn work_boards(&self) -> Result<Vec<Board>> {
        let saved: Option<String> = self.conn().query_row("SELECT value FROM work_prefs WHERE key = 'boards'", [], |r| r.get(0)).optional()?;
        Ok(saved.and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default())
    }

    pub fn work_put_boards(&self, boards: &[Board]) -> Result<()> {
        self.conn().execute(
            "INSERT INTO work_prefs (key, value) VALUES ('boards', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![serde_json::to_string(boards)?],
        )?;
        Ok(())
    }

    /// Which of a project's tickets are on a board, from one full read of it.
    pub fn work_put_board_keys(&self, board_id: &str, project_id: &str, keys: &[String], now: i64) -> Result<()> {
        self.conn().execute(
            "INSERT INTO work_board_keys (board_id, project_id, keys, read_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(board_id, project_id) DO UPDATE SET keys = excluded.keys, read_at = excluded.read_at",
            params![board_id, project_id, serde_json::to_string(keys)?, now],
        )?;
        Ok(())
    }

    /// The board you picked with the tickets on it: every project's as last read (none while no project was read
    /// for it yet), and the oldest of those reads.
    pub fn work_board_pick(&self) -> Result<Option<BoardPick>> {
        let Some(board) = self.work_board()? else { return Ok(None) };
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT keys, read_at FROM work_board_keys WHERE board_id = ?1 ORDER BY project_id")?;
        let rows = stmt.query_map([&board.id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        let mut keys: Option<Vec<String>> = None;
        let mut read_at: Option<i64> = None;
        for row in rows {
            let (list, at) = row?;
            keys.get_or_insert_with(Vec::new).extend(serde_json::from_str::<Vec<String>>(&list)?);
            read_at = Some(read_at.map_or(at, |r| r.min(at)));
        }
        if let Some(k) = keys.as_mut() {
            k.sort();
            k.dedup();
        }
        Ok(Some(BoardPick { board, keys, read_at }))
    }

    /// The latest events, newest first.
    pub fn work_events(&self, limit: usize) -> Result<Vec<Event>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT subject, at, kind, text FROM work_events ORDER BY at DESC, id DESC LIMIT ?1")?;
        let rows = stmt.query_map([limit as i64], |r| Ok(Event { subject: r.get(0)?, at: r.get(1)?, kind: r.get(2)?, text: r.get(3)? }))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn work_put_health(&self, h: &Health) -> Result<()> {
        self.conn().execute(
            "INSERT INTO work_health (project_id, source, state, last_ok, last_try, error, fix, next_try, account)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(project_id, source) DO UPDATE SET state = excluded.state, last_ok = excluded.last_ok,
               last_try = excluded.last_try, error = excluded.error, fix = excluded.fix, next_try = excluded.next_try,
               account = excluded.account",
            params![h.project_id, h.source, h.state, h.last_ok, h.last_try, h.error, h.fix, h.next_try, h.account],
        )?;
        Ok(())
    }

    pub fn work_health(&self) -> Result<Vec<Health>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT project_id, source, state, last_ok, last_try, error, fix, next_try, account FROM work_health ORDER BY project_id, source",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(Health {
                project_id: r.get(0)?,
                source: r.get(1)?,
                state: r.get(2)?,
                last_ok: r.get(3)?,
                last_try: r.get(4)?,
                error: r.get(5)?,
                fix: r.get(6)?,
                next_try: r.get(7)?,
                account: r.get(8)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn work_items(&self) -> Result<Vec<Item>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, kind, source, project_id, pr, ticket, title, detail, who, url, path, line, at, state, opened_at,
                    closed_at, snoozed_until, bot FROM work_items ORDER BY at DESC",
        )?;
        let rows = stmt.query_map([], item_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn work_put_item(&self, i: &Item) -> Result<()> {
        self.conn().execute(
            "INSERT INTO work_items (id, kind, source, project_id, pr, ticket, title, detail, who, url, path, line, at, state,
                                     opened_at, closed_at, snoozed_until, bot)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
             ON CONFLICT(id) DO UPDATE SET kind = excluded.kind, project_id = excluded.project_id, pr = excluded.pr,
               ticket = excluded.ticket, title = excluded.title, detail = excluded.detail, who = excluded.who,
               url = excluded.url, path = excluded.path, line = excluded.line, at = excluded.at, state = excluded.state,
               closed_at = excluded.closed_at, snoozed_until = excluded.snoozed_until, bot = excluded.bot",
            params![
                i.id, i.kind, i.source, i.project_id, i.pr, i.ticket, i.title, i.detail, i.who, i.url, i.path, i.line,
                i.at, i.state, i.opened_at, i.closed_at, i.snoozed_until, i.bot
            ],
        )?;
        Ok(())
    }
}

fn item_from_row(r: &rusqlite::Row) -> rusqlite::Result<Item> {
    Ok(Item {
        id: r.get(0)?,
        kind: r.get(1)?,
        source: r.get(2)?,
        project_id: r.get(3)?,
        pr: r.get(4)?,
        ticket: r.get(5)?,
        title: r.get(6)?,
        detail: r.get(7)?,
        who: r.get(8)?,
        url: r.get(9)?,
        path: r.get(10)?,
        line: r.get(11)?,
        at: r.get(12)?,
        state: r.get(13)?,
        opened_at: r.get(14)?,
        closed_at: r.get(15)?,
        snoozed_until: r.get(16)?,
        bot: r.get(17)?,
    })
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub fn db() -> (tempfile::TempDir, Db) {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("enkidu.db")).unwrap();
        (dir, db)
    }

    /// Item 8.5: a task of your own is kept with its words and its state, changed, and deleted with what was kept
    /// about its card.
    #[test]
    fn a_task_of_your_own_is_saved_changed_and_deleted() {
        let (_d, db) = db();
        let project = |id: &str| crate::db::Project { id: id.into(), name: id.into(), path: format!("/tmp/{id}"), live_workspace_id: None, created_at: 1 };
        db.insert_project(&project("p1")).unwrap();
        db.insert_project(&project("p2")).unwrap();
        assert!(db.work_own_tasks().unwrap().is_empty());
        // a new one: its words without the spaces around them, to do unless said otherwise
        let first = db.work_save_own_task(None, "p1", "  Try the small stack  ", " on the CI runner\n", "todo", 100).unwrap();
        assert_eq!((first.id, first.title.as_str(), first.notes.as_str(), first.state.as_str()), (1, "Try the small stack", "on the CI runner", "todo"));
        assert_eq!((first.project_id.as_str(), first.created_at, first.updated_at, first.subject().as_str()), ("p1", 100, 100, "own:1"));
        assert_eq!(first.project, "p1", "its project's name comes with it, for the card");
        let second = db.work_save_own_task(None, "p2", "Release notes", "", "doing", 110).unwrap();
        assert_eq!((second.id, second.state.as_str()), (2, "doing"));
        // what can't be: no title, a state nobody knows, a project that is gone
        assert_eq!(db.work_save_own_task(None, "p1", "   ", "x", "todo", 120).unwrap_err().to_string(), "Give the task a title");
        assert!(db.work_save_own_task(None, "p1", "x", "", "napping", 120).unwrap_err().to_string().contains("not napping"));
        assert_eq!(db.work_save_own_task(None, "nope", "x", "", "todo", 120).unwrap_err().to_string(), "That project is no longer there");
        assert_eq!(db.work_save_own_task(Some(99), "p1", "x", "", "todo", 120).unwrap_err().to_string(), "That task is no longer there");
        assert_eq!(db.work_own_tasks().unwrap().len(), 2, "a refused save keeps nothing");
        // changed: its words and its state; its project and when it was made stay
        let moved = db.work_save_own_task(Some(1), "p2", "Try the small Docker setup", "", "done", 200).unwrap();
        assert_eq!((moved.title.as_str(), moved.notes.as_str(), moved.state.as_str()), ("Try the small Docker setup", "", "done"));
        assert_eq!((moved.project_id.as_str(), moved.created_at, moved.updated_at), ("p1", 100, 200));
        assert_eq!(db.work_own_task(1).unwrap(), Some(moved.clone()));
        assert_eq!(db.work_own_tasks().unwrap(), [moved, second.clone()], "in the order they were made");
        // its card can be hidden, linked and written about like any card
        db.work_set_hidden(&["own:1".into()], true, 210).unwrap();
        db.work_link("session:s1", "own:1", "you", "linked by you", 210).unwrap();
        let linked: Vec<(String, String)> = db.work_links().unwrap().into_iter().map(|link| (link.a, link.b)).collect();
        assert_eq!(linked, [("own:1".to_string(), "session:s1".to_string())], "the card first, as a ticket's link");
        db.work_event("own:1", "own", "Set to done", 210).unwrap();
        db.work_event("own:2", "own", "Made", 210).unwrap();
        assert!(db.work_set_hidden(&["own:x".into()], true, 220).is_err(), "own:x is no card");
        // deleted: it is gone with what was kept about its card, and the other one stays whole
        db.work_delete_own_task(1).unwrap();
        assert_eq!(db.work_own_tasks().unwrap(), [second]);
        assert_eq!(db.work_own_task(1).unwrap(), None);
        assert!(db.work_hidden().unwrap().is_empty());
        assert!(db.work_links().unwrap().is_empty());
        let events: Vec<String> = db.work_events(10).unwrap().into_iter().map(|event| event.subject).collect();
        assert_eq!(events, ["own:2"]);
        // a number is never given twice
        let third = db.work_save_own_task(None, "p1", "Another", "", "todo", 300).unwrap();
        assert_eq!(third.id, 3);
        // a project's tasks go with it
        db.work_delete_own_tasks_of("p1").unwrap();
        let left: Vec<i64> = db.work_own_tasks().unwrap().into_iter().map(|task| task.id).collect();
        assert_eq!(left, [2]);
    }

    #[test]
    fn hides_cards_and_shows_them_again() {
        let (_d, db) = db();
        db.work_set_hidden(&["ticket:SH-1".into(), "pr:o/r#2".into()], true, 10).unwrap();
        db.work_set_hidden(&["ticket:SH-1".into()], true, 20).unwrap();
        assert_eq!(db.work_hidden().unwrap(), ["pr:o/r#2", "ticket:SH-1"], "hiding twice keeps the first time");
        db.work_set_hidden(&["ticket:SH-1".into()], false, 30).unwrap();
        assert_eq!(db.work_hidden().unwrap(), ["pr:o/r#2"]);
        assert!(db.work_set_hidden(&["session:s1".into()], true, 40).is_err(), "only cards are hidden");
        assert_eq!(db.work_hidden().unwrap(), ["pr:o/r#2"], "a refused batch changes nothing");
    }

    #[test]
    fn keeps_the_board_you_picked_the_boards_last_read_and_the_tickets_on_each() {
        let (_d, db) = db();
        assert_eq!(db.work_board().unwrap(), None);
        assert_eq!(db.work_board_pick().unwrap(), None, "no board picked: everything read is on the page");
        assert!(db.work_boards().unwrap().is_empty());
        let board = |id: &str, name: &str, kind: &str| Board { id: id.into(), name: name.into(), kind: kind.into(), project_key: Some("SH".into()) };
        let dev = board("5", "Shop Dev", "scrum");
        db.work_put_boards(&[dev.clone(), board("9", "Release view", "kanban")]).unwrap();
        assert_eq!(db.work_boards().unwrap().len(), 2);
        db.work_set_board(Some(&dev)).unwrap();
        let pick = db.work_board_pick().unwrap().unwrap();
        assert_eq!((pick.board.clone(), pick.keys.clone(), pick.read_at), (dev.clone(), None, None), "picked, not read yet");
        // each project's tickets on it, as last read; a second read replaces the first
        db.work_put_board_keys("5", "p1", &["SH-2".into(), "SH-1".into()], 100).unwrap();
        db.work_put_board_keys("5", "p2", &["XY-7".into()], 300).unwrap();
        db.work_put_board_keys("5", "p1", &["SH-1".into()], 200).unwrap();
        db.work_put_board_keys("9", "p1", &["SH-9".into()], 400).unwrap();
        let pick = db.work_board_pick().unwrap().unwrap();
        assert_eq!(pick.keys.as_deref(), Some(&["SH-1".to_string(), "XY-7".to_string()][..]));
        assert_eq!(pick.read_at, Some(200), "as old as its oldest read");
        // a board read with nothing of yours on it is read, and empty
        db.work_put_board_keys("5", "p1", &[], 500).unwrap();
        db.work_put_board_keys("5", "p2", &[], 500).unwrap();
        assert_eq!(db.work_board_pick().unwrap().unwrap().keys, Some(Vec::new()));
        // another board: its own tickets, kept from when it was read
        db.work_set_board(Some(&board("9", "Release view", "kanban"))).unwrap();
        assert_eq!(db.work_board_pick().unwrap().unwrap().keys, Some(vec!["SH-9".to_string()]));
        db.work_set_board(None).unwrap();
        assert_eq!(db.work_board_pick().unwrap(), None);
        assert_eq!(db.work_boards().unwrap().len(), 2, "the boards stay listed");
    }

    #[test]
    fn a_pair_is_stored_once_whatever_the_order() {
        let (_d, db) = db();
        assert!(db.work_link("session:s1", "pr:o/r#1", "branch", "worked on its branch", 10).unwrap());
        assert!(!db.work_link("pr:o/r#1", "session:s1", "branch", "worked on its branch", 20).unwrap(), "same link again");
        assert!(db.work_link("pr:o/r#1", "session:s1", "hook:pr-create", "created it", 30).unwrap(), "another how is kept too");
        let links = db.work_links().unwrap();
        assert_eq!(links.len(), 2);
        assert!(links.iter().all(|l| l.a == "pr:o/r#1" && l.b == "session:s1"));
        assert_eq!(ordered("workspace:w", "ticket:SH-1"), ("ticket:SH-1", "workspace:w"));
    }

    #[test]
    fn an_unlinked_pair_stays_unlinked_until_you_link_it_again() {
        let (_d, db) = db();
        db.work_link("pr:o/r#1", "session:s1", "branch", "", 10).unwrap();
        db.work_unlink("session:s1", "pr:o/r#1", 20).unwrap();
        assert!(db.work_links().unwrap().is_empty(), "removed, not deleted");
        assert!(!db.work_link("pr:o/r#1", "session:s1", "branch", "", 30).unwrap());
        assert!(!db.work_link("pr:o/r#1", "session:s1", "transcript", "", 30).unwrap(), "no rule brings it back");
        assert!(db.work_link("pr:o/r#1", "session:s1", "you", "", 40).unwrap());
        assert_eq!(db.work_links().unwrap().len(), 1);
        let kept: i64 = db.conn().query_row("SELECT COUNT(*) FROM work_links", [], |r| r.get(0)).unwrap();
        assert_eq!(kept, 2, "the removed branch link is still in the history");
    }

    #[test]
    fn sessions_merge_what_each_source_saw() {
        let (_d, db) = db();
        let seen = |cfg: &str, cwd: &str, title: Option<&str>, first: i64, last: i64, source: &str| SessionSeen {
            id: "s1".into(),
            config_dir: cfg.into(),
            cwd: cwd.into(),
            title: title.map(String::from),
            workspace_id: None,
            first_at: first,
            last_at: last,
            source: source.into(),
        };
        db.work_session_seen(&seen("/h/.claude", "/code/shop", Some("fix totals"), 100, 200, "transcript")).unwrap();
        db.work_session_seen(&seen("/h/.claude-work", "/code/shop", None, 150, 300, "hook")).unwrap();
        db.work_session_seen(&seen("/h/.claude", "/elsewhere", None, 50, 120, "transcript")).unwrap();
        db.work_session_branch("s1", "feature/a", "transcript", "worked", 110).unwrap();
        db.work_session_branch("s1", "feature/b", "branch", "worked", 250).unwrap();
        db.work_session_branch("s1", "feature/a", "branch", "pushed", 260).unwrap();
        let s = &db.work_sessions().unwrap()[0];
        assert_eq!((s.first_seen, s.last_seen), (50, 300));
        assert_eq!(s.config_dir, "/h/.claude-work", "the account it was seen in last");
        assert_eq!(s.cwd, "/code/shop", "an older sighting doesn't move it");
        assert_eq!(s.title.as_deref(), Some("fix totals"));
        assert_eq!(s.source, "hook", "known live beats found in a transcript");
        assert_eq!(s.branches, ["feature/a", "feature/b"], "in the order it first worked on them");
    }

    #[test]
    fn finds_a_sessions_conversation_scanned_or_known_only_from_its_hooks() {
        let (dir, db) = db();
        let config = dir.path().join(".claude-work");
        let text = |p: &std::path::Path| p.to_string_lossy().to_string();
        // scanned: the newest copy
        let old = config.join("projects/-code-shop/s1.jsonl");
        let new = config.join("projects/-ws-shop-fix/s1.jsonl");
        for (p, mtime) in [(&old, 10), (&new, 20)] {
            let st = TranscriptState { session_id: "s1".into(), mtime, ..Default::default() };
            db.work_save_transcript(&text(p), &st).unwrap();
        }
        assert_eq!(db.work_transcript_of("s1").unwrap(), Some((text(&config), text(&new))));
        // known only from its hooks: found where Claude Code keeps it, once the file is there
        let seen = SessionSeen {
            id: "warp1".into(),
            config_dir: text(&config),
            cwd: "/code/shop".into(),
            title: None,
            workspace_id: None,
            first_at: 1,
            last_at: 2,
            source: "hook".into(),
        };
        db.work_session_seen(&seen).unwrap();
        assert_eq!(db.work_transcript_of("warp1").unwrap(), None);
        let file = config.join("projects/-code-shop/warp1.jsonl");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "{}\n").unwrap();
        assert_eq!(db.work_transcript_of("warp1").unwrap(), Some((text(&config), text(&file))));
        assert_eq!(db.work_transcript_of("nobody").unwrap(), None);
    }
}
