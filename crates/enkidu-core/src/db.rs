//! The app's own state: projects and workspaces, in SQLite.

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    pub path: String,
    pub live_workspace_id: Option<String>,
    pub created_at: i64,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub path: String,
    pub branch: String,
    pub base: Option<String>,
    pub session_id: String,
    pub transcript_path: Option<String>,
    /// The Claude folder (account) its session lives in, when it isn't the project's: a session brought in from
    /// outside Enkidu keeps its own account
    #[serde(default)]
    pub claude_config_dir: Option<String>,
    pub created_at: i64,
    /// A folder Enkidu didn't make, opened as a workspace (its branch was checked out there): removing the
    /// workspace only forgets it, the folder stays
    #[serde(default)]
    pub adopted: bool,
}

/// A remark on some lines of a workspace's diff, kept until it is sent to Claude.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Remark {
    pub id: String,
    pub workspace_id: String,
    pub path: String,
    /// "new" for lines as they are now, "old" for removed lines
    pub side: String,
    pub start_line: i64,
    pub end_line: i64,
    pub snippet: String,
    pub body: String,
    pub created_at: i64,
}

pub struct Db {
    conn: Mutex<Connection>,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS projects (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    path TEXT NOT NULL UNIQUE,
    live_workspace_id TEXT,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS workspaces (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    path TEXT NOT NULL UNIQUE,
    branch TEXT NOT NULL,
    base TEXT,
    session_id TEXT NOT NULL,
    transcript_path TEXT,
    created_at INTEGER NOT NULL,
    UNIQUE(project_id, name)
);
CREATE TABLE IF NOT EXISTS remarks (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    side TEXT NOT NULL,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    snippet TEXT NOT NULL,
    body TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS remarks_by_workspace ON remarks(workspace_id);
";

/// The Work index: tickets, PRs and the Claude sessions behind them, linked many to many (see WORK-INDEX.md).
/// Tickets and PRs keep their whole record as JSON next to the columns that are searched.
const WORK_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS work_tickets (
    key TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    data TEXT NOT NULL,
    status TEXT NOT NULL,
    category TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    seen_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS work_prs (
    ref TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    data TEXT NOT NULL,
    state TEXT NOT NULL,
    head TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    seen_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS work_prs_by_head ON work_prs(head);
CREATE TABLE IF NOT EXISTS work_sessions (
    id TEXT PRIMARY KEY,
    config_dir TEXT NOT NULL,
    cwd TEXT NOT NULL,
    title TEXT,
    workspace_id TEXT,
    first_seen INTEGER NOT NULL,
    last_seen INTEGER NOT NULL,
    ended INTEGER NOT NULL DEFAULT 0,
    source TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS work_session_branches (
    session_id TEXT NOT NULL,
    branch TEXT NOT NULL,
    how TEXT NOT NULL,
    detail TEXT NOT NULL,
    at INTEGER NOT NULL,
    PRIMARY KEY (session_id, branch, how)
);
CREATE INDEX IF NOT EXISTS work_branches_by_name ON work_session_branches(branch);
CREATE TABLE IF NOT EXISTS work_mentions (
    session_id TEXT NOT NULL,
    target TEXT NOT NULL,
    how TEXT NOT NULL,
    detail TEXT NOT NULL,
    at INTEGER NOT NULL,
    PRIMARY KEY (session_id, target, how)
);
CREATE TABLE IF NOT EXISTS work_commits (
    sha TEXT NOT NULL,
    session_id TEXT NOT NULL,
    branch TEXT,
    how TEXT NOT NULL,
    at INTEGER NOT NULL,
    PRIMARY KEY (sha, session_id)
);
CREATE TABLE IF NOT EXISTS work_links (
    a TEXT NOT NULL,
    b TEXT NOT NULL,
    how TEXT NOT NULL,
    detail TEXT NOT NULL,
    at INTEGER NOT NULL,
    removed INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (a, b, how, detail)
);
CREATE INDEX IF NOT EXISTS work_links_by_b ON work_links(b);
CREATE TABLE IF NOT EXISTS work_items (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    source TEXT NOT NULL,
    project_id TEXT NOT NULL,
    pr TEXT,
    ticket TEXT,
    title TEXT NOT NULL,
    detail TEXT NOT NULL,
    who TEXT,
    url TEXT,
    path TEXT,
    line INTEGER,
    at TEXT NOT NULL,
    state TEXT NOT NULL,
    opened_at INTEGER NOT NULL,
    closed_at INTEGER,
    snoozed_until INTEGER,
    bot INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS work_items_by_state ON work_items(state);
CREATE TABLE IF NOT EXISTS work_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    subject TEXT NOT NULL,
    at INTEGER NOT NULL,
    kind TEXT NOT NULL,
    text TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS work_events_by_subject ON work_events(subject, at);
CREATE TABLE IF NOT EXISTS work_health (
    project_id TEXT NOT NULL,
    source TEXT NOT NULL,
    state TEXT NOT NULL,
    last_ok INTEGER,
    last_try INTEGER,
    error TEXT,
    fix TEXT,
    next_try INTEGER,
    account TEXT,
    PRIMARY KEY (project_id, source)
);
CREATE TABLE IF NOT EXISTS work_transcripts (
    path TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    size INTEGER NOT NULL,
    mtime INTEGER NOT NULL,
    offset INTEGER NOT NULL,
    pending TEXT NOT NULL DEFAULT '{}'
);
-- what the workflows did (or decided not to): one row per item sent (failing checks) or per day (morning summary)
CREATE TABLE IF NOT EXISTS work_workflow_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id TEXT NOT NULL,
    workflow TEXT NOT NULL,
    subject TEXT NOT NULL,
    state TEXT NOT NULL,
    at INTEGER NOT NULL,
    text TEXT NOT NULL DEFAULT '',
    UNIQUE (project_id, workflow, subject)
);
-- the cards you hid from the Work board (`ticket:SH-1`, or `pr:owner/repo#2` for a PR without a ticket), until you
-- show them again
CREATE TABLE IF NOT EXISTS work_hidden (
    subject TEXT PRIMARY KEY,
    at INTEGER NOT NULL
);
-- what the Work page keeps between runs: the Jira board you picked (`board`) and Jira's boards as last read (`boards`)
CREATE TABLE IF NOT EXISTS work_prefs (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
-- which of a project's tickets are on a Jira board (a JSON list of keys), as of its last read
CREATE TABLE IF NOT EXISTS work_board_keys (
    board_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    keys TEXT NOT NULL,
    read_at INTEGER NOT NULL,
    PRIMARY KEY (board_id, project_id)
);
-- Claude's reports on its tasks (`enkidu report`): done, review or blocked, its words, and its screenshots (file names in
-- reports/<id>/ in the data folder)
CREATE TABLE IF NOT EXISTS work_reports (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    subject TEXT NOT NULL,
    session_id TEXT,
    workspace_id TEXT,
    state TEXT NOT NULL,
    text TEXT NOT NULL,
    images TEXT NOT NULL DEFAULT '[]',
    at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS work_reports_subject ON work_reports (subject, at);
-- your scheduled workflows: tasks Claude works on one after another from a time you pick, each for at most `minutes`
CREATE TABLE IF NOT EXISTS work_flows (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    name TEXT NOT NULL,
    instruction TEXT NOT NULL,
    start_at INTEGER,
    minutes INTEGER NOT NULL,
    live INTEGER NOT NULL DEFAULT 1,
    state TEXT NOT NULL,
    current INTEGER,
    live_before TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
-- a workflow's tasks in order: the card, where it runs (`target`, JSON), the message, and how it went
CREATE TABLE IF NOT EXISTS work_flow_tasks (
    flow_id TEXT NOT NULL,
    position INTEGER NOT NULL,
    subject TEXT NOT NULL,
    title TEXT NOT NULL,
    target TEXT NOT NULL,
    message TEXT NOT NULL,
    edited INTEGER NOT NULL DEFAULT 0,
    state TEXT NOT NULL,
    workspace_id TEXT,
    started_at INTEGER,
    ended_at INTEGER,
    note TEXT,
    busy_at INTEGER,
    nudged_at INTEGER,
    kind TEXT NOT NULL DEFAULT 'custom',
    instruction TEXT NOT NULL DEFAULT '',
    minutes INTEGER,
    PRIMARY KEY (flow_id, position)
);
-- each run of a workflow, kept as it ended (the workflow then, as JSON, its tasks with how each went)
CREATE TABLE IF NOT EXISTS work_flow_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    flow_id TEXT NOT NULL,
    started_at INTEGER,
    ended_at INTEGER NOT NULL,
    state TEXT NOT NULL,
    flow TEXT NOT NULL
);
-- your own tasks on the Work board (`own:3`): what you want to do in a project that is not in Jira, each `todo`, `doing`
-- or `done` (item 8.5)
CREATE TABLE IF NOT EXISTS work_own_tasks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id TEXT NOT NULL,
    title TEXT NOT NULL,
    notes TEXT NOT NULL DEFAULT '',
    state TEXT NOT NULL DEFAULT 'todo',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
";

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn project_from_row(row: &Row) -> rusqlite::Result<Project> {
    Ok(Project {
        id: row.get(0)?,
        name: row.get(1)?,
        path: row.get(2)?,
        live_workspace_id: row.get(3)?,
        created_at: row.get(4)?,
    })
}

fn workspace_from_row(row: &Row) -> rusqlite::Result<Workspace> {
    Ok(Workspace {
        id: row.get(0)?,
        project_id: row.get(1)?,
        name: row.get(2)?,
        path: row.get(3)?,
        branch: row.get(4)?,
        base: row.get(5)?,
        session_id: row.get(6)?,
        transcript_path: row.get(7)?,
        created_at: row.get(8)?,
        claude_config_dir: row.get(9)?,
        adopted: row.get(10)?,
    })
}

const PROJECT_COLS: &str = "id, name, path, live_workspace_id, created_at";
const WORKSPACE_COLS: &str =
    "id, project_id, name, path, branch, base, session_id, transcript_path, created_at, claude_config_dir, adopted";

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(SCHEMA)?;
        conn.execute_batch(WORK_SCHEMA)?;
        // columns added after the first release: added to an older database once
        let has = |table: &str, column: &str| -> rusqlite::Result<bool> {
            let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
            let names: Vec<String> = stmt.query_map([], |r| r.get::<_, String>(1))?.collect::<rusqlite::Result<_>>()?;
            Ok(names.iter().any(|n| n == column))
        };
        if !has("workspaces", "claude_config_dir")? {
            conn.execute_batch("ALTER TABLE workspaces ADD COLUMN claude_config_dir TEXT")?;
        }
        if !has("workspaces", "adopted")? {
            conn.execute_batch("ALTER TABLE workspaces ADD COLUMN adopted INTEGER NOT NULL DEFAULT 0")?;
        }
        // a workflow's tasks got their kind of work, own words and own minutes after the table was first made
        for (column, kind) in [("kind", "TEXT NOT NULL DEFAULT 'custom'"), ("instruction", "TEXT NOT NULL DEFAULT ''"), ("minutes", "INTEGER")] {
            if !has("work_flow_tasks", column)? {
                conn.execute_batch(&format!("ALTER TABLE work_flow_tasks ADD COLUMN {column} {kind}"))?;
            }
        }
        let db = Self { conn: Mutex::new(conn) };
        // workflows that ended before their runs were kept: kept as their runs, once (never in the way of opening)
        let _ = db.work_keep_ended_flows();
        Ok(db)
    }

    pub(crate) fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn projects(&self) -> Result<Vec<Project>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!("SELECT {PROJECT_COLS} FROM projects ORDER BY created_at"))?;
        let rows = stmt.query_map([], project_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn project(&self, id: &str) -> Result<Option<Project>> {
        let conn = self.conn();
        Ok(conn
            .query_row(&format!("SELECT {PROJECT_COLS} FROM projects WHERE id = ?1"), [id], project_from_row)
            .optional()?)
    }

    pub fn project_by_path(&self, path: &str) -> Result<Option<Project>> {
        let conn = self.conn();
        Ok(conn
            .query_row(&format!("SELECT {PROJECT_COLS} FROM projects WHERE path = ?1"), [path], project_from_row)
            .optional()?)
    }

    pub fn insert_project(&self, project: &Project) -> Result<()> {
        self.conn().execute(
            "INSERT INTO projects (id, name, path, live_workspace_id, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![project.id, project.name, project.path, project.live_workspace_id, project.created_at],
        )?;
        Ok(())
    }

    pub fn delete_project(&self, id: &str) -> Result<()> {
        self.conn().execute("DELETE FROM projects WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn set_live(&self, project_id: &str, workspace_id: Option<&str>) -> Result<()> {
        self.conn().execute(
            "UPDATE projects SET live_workspace_id = ?2 WHERE id = ?1",
            params![project_id, workspace_id],
        )?;
        Ok(())
    }

    pub fn workspaces(&self) -> Result<Vec<Workspace>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!("SELECT {WORKSPACE_COLS} FROM workspaces ORDER BY created_at"))?;
        let rows = stmt.query_map([], workspace_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn workspace(&self, id: &str) -> Result<Option<Workspace>> {
        let conn = self.conn();
        Ok(conn
            .query_row(&format!("SELECT {WORKSPACE_COLS} FROM workspaces WHERE id = ?1"), [id], workspace_from_row)
            .optional()?)
    }

    pub fn insert_workspace(&self, ws: &Workspace) -> Result<()> {
        self.conn().execute(
            "INSERT INTO workspaces (id, project_id, name, path, branch, base, session_id, transcript_path, created_at, claude_config_dir, adopted)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                ws.id,
                ws.project_id,
                ws.name,
                ws.path,
                ws.branch,
                ws.base,
                ws.session_id,
                ws.transcript_path,
                ws.created_at,
                ws.claude_config_dir,
                ws.adopted
            ],
        )?;
        Ok(())
    }

    /// The conversation the workspace's Claude starts in from now on, where it is saved and in which Claude folder.
    pub fn set_conversation(&self, id: &str, session_id: &str, transcript_path: Option<&str>, claude_config_dir: Option<&str>) -> Result<()> {
        self.conn().execute(
            "UPDATE workspaces SET session_id = ?2, transcript_path = ?3, claude_config_dir = ?4 WHERE id = ?1",
            params![id, session_id, transcript_path, claude_config_dir],
        )?;
        Ok(())
    }

    /// The branch the workspace's folder is on now (Claude or you checked out another one in it).
    pub fn set_branch(&self, id: &str, branch: &str) -> Result<()> {
        self.conn().execute("UPDATE workspaces SET branch = ?2 WHERE id = ?1", params![id, branch])?;
        Ok(())
    }

    pub fn delete_workspace(&self, id: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute("UPDATE projects SET live_workspace_id = NULL WHERE live_workspace_id = ?1", [id])?;
        conn.execute("DELETE FROM workspaces WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn set_transcript_path(&self, id: &str, path: &str) -> Result<()> {
        self.conn().execute("UPDATE workspaces SET transcript_path = ?2 WHERE id = ?1", params![id, path])?;
        Ok(())
    }

    /// The conversation the workspace's Claude is in now (you resumed another one in it, or cleared it).
    pub fn set_session_id(&self, id: &str, session_id: &str) -> Result<()> {
        self.conn().execute("UPDATE workspaces SET session_id = ?2 WHERE id = ?1", params![id, session_id])?;
        Ok(())
    }

    pub fn remarks(&self, workspace_id: &str) -> Result<Vec<Remark>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, workspace_id, path, side, start_line, end_line, snippet, body, created_at
             FROM remarks WHERE workspace_id = ?1 ORDER BY created_at",
        )?;
        let rows = stmt.query_map([workspace_id], |row| {
            Ok(Remark {
                id: row.get(0)?,
                workspace_id: row.get(1)?,
                path: row.get(2)?,
                side: row.get(3)?,
                start_line: row.get(4)?,
                end_line: row.get(5)?,
                snippet: row.get(6)?,
                body: row.get(7)?,
                created_at: row.get(8)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Adds a remark, or replaces the one with the same id (an edit).
    pub fn save_remark(&self, r: &Remark) -> Result<()> {
        self.conn().execute(
            "INSERT INTO remarks (id, workspace_id, path, side, start_line, end_line, snippet, body, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET body = excluded.body, start_line = excluded.start_line,
               end_line = excluded.end_line, snippet = excluded.snippet",
            params![r.id, r.workspace_id, r.path, r.side, r.start_line, r.end_line, r.snippet, r.body, r.created_at],
        )?;
        Ok(())
    }

    pub fn delete_remark(&self, id: &str) -> Result<()> {
        self.conn().execute("DELETE FROM remarks WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn clear_remarks(&self, workspace_id: &str) -> Result<()> {
        self.conn().execute("DELETE FROM remarks WHERE workspace_id = ?1", [workspace_id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_ws(id: &str, project: &str, name: &str) -> Workspace {
        Workspace {
            id: id.into(),
            project_id: project.into(),
            name: name.into(),
            path: format!("/tmp/{project}/{name}"),
            branch: format!("feature/{name}"),
            base: Some("development".into()),
            session_id: format!("session-{id}"),
            transcript_path: None,
            claude_config_dir: None,
            adopted: false,
            created_at: now_ms(),
        }
    }

    #[test]
    fn a_workflow_saved_by_the_first_version_gets_its_tasks_kind_words_and_minutes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("enkidu.db");
        {
            // the tasks' table as it was first made
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE work_flow_tasks (flow_id TEXT NOT NULL, position INTEGER NOT NULL, subject TEXT NOT NULL, title TEXT NOT NULL,
                   target TEXT NOT NULL, message TEXT NOT NULL, edited INTEGER NOT NULL DEFAULT 0, state TEXT NOT NULL, workspace_id TEXT,
                   started_at INTEGER, ended_at INTEGER, note TEXT, busy_at INTEGER, nudged_at INTEGER, PRIMARY KEY (flow_id, position));
                 INSERT INTO work_flow_tasks (flow_id, position, subject, title, target, message, state)
                   VALUES ('f1', 0, 'ticket:SH-1', 'Fix it', '{\"kind\":\"workspace\",\"workspaceId\":\"w1\"}', 'SH-1: Fix it', 'waiting');",
            )
            .unwrap();
        }
        let db = Db::open(&path).unwrap();
        db.conn()
            .execute_batch(
                "INSERT INTO work_flows (id, project_id, name, instruction, start_at, minutes, live, state, created_at, updated_at)
                 VALUES ('f1', 'p1', 'Night', 'Fix them', NULL, 60, 1, 'saved', 1, 1)",
            )
            .unwrap();
        let t = &db.work_flows().unwrap()[0].tasks[0];
        assert_eq!((t.kind.as_str(), t.instruction.as_str(), t.minutes, t.message.as_str()), ("custom", "", None, "SH-1: Fix it"));
    }

    #[test]
    fn an_older_database_gets_the_new_columns_and_keeps_its_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("enkidu.db");
        {
            // a database from before workspaces knew their Claude folder
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(SCHEMA).unwrap();
            conn.execute_batch("ALTER TABLE workspaces DROP COLUMN claude_config_dir").ok();
            let cols: Vec<String> = conn.prepare("PRAGMA table_info(workspaces)").unwrap().query_map([], |r| r.get(1)).unwrap().map(|c| c.unwrap()).collect();
            assert!(!cols.contains(&"claude_config_dir".to_string()), "the old shape");
            conn.execute("INSERT INTO projects (id, name, path, live_workspace_id, created_at) VALUES ('p1', 'shop', '/code/shop', NULL, 1)", []).unwrap();
            conn.execute(
                "INSERT INTO workspaces (id, project_id, name, path, branch, base, session_id, transcript_path, created_at)
                 VALUES ('w1', 'p1', 'one', '/ws/one', 'feature/one', 'development', 's1', NULL, 1)",
                [],
            )
            .unwrap();
        }
        let db = Db::open(&path).unwrap();
        let ws = db.workspace("w1").unwrap().unwrap();
        assert_eq!((ws.name.as_str(), ws.claude_config_dir.as_deref(), ws.adopted), ("one", None, false));
        let mut moved = ws.clone();
        moved.id = "w2".into();
        moved.name = "two".into();
        moved.path = "/ws/two".into();
        moved.claude_config_dir = Some("/h/.claude-work".into());
        moved.adopted = true;
        db.insert_workspace(&moved).unwrap();
        let two = db.workspace("w2").unwrap().unwrap();
        assert_eq!((two.claude_config_dir.as_deref(), two.adopted), (Some("/h/.claude-work"), true));
        // the folder moved to another branch: the workspace follows
        db.set_branch("w1", "feature/other").unwrap();
        assert_eq!(db.workspace("w1").unwrap().unwrap().branch, "feature/other");
        drop(db);
        // opening it again changes nothing
        assert_eq!(Db::open(&path).unwrap().workspaces().unwrap().len(), 2);
    }

    #[test]
    fn stores_projects_workspaces_and_live() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("enkidu.db")).unwrap();
        let project = Project {
            id: "p1".into(),
            name: "shop".into(),
            path: "/code/shop".into(),
            live_workspace_id: None,
            created_at: now_ms(),
        };
        db.insert_project(&project).unwrap();
        assert!(db.insert_project(&project).is_err(), "paths are unique");
        db.insert_workspace(&sample_ws("w1", "p1", "one")).unwrap();
        db.insert_workspace(&sample_ws("w2", "p1", "two")).unwrap();
        assert!(db.insert_workspace(&sample_ws("w3", "p1", "one")).is_err(), "names are unique per project");

        db.set_live("p1", Some("w2")).unwrap();
        assert_eq!(db.project("p1").unwrap().unwrap().live_workspace_id.as_deref(), Some("w2"));

        db.set_transcript_path("w1", "/x/t.jsonl").unwrap();
        assert_eq!(db.workspace("w1").unwrap().unwrap().transcript_path.as_deref(), Some("/x/t.jsonl"));

        db.delete_workspace("w2").unwrap();
        assert_eq!(db.project("p1").unwrap().unwrap().live_workspace_id, None, "deleting the live one clears live");
        assert_eq!(db.workspaces().unwrap().len(), 1);

        db.delete_project("p1").unwrap();
        assert!(db.workspaces().unwrap().is_empty(), "workspaces go with their project");
    }

    #[test]
    fn keeps_remarks_until_cleared_and_drops_them_with_the_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("enkidu.db")).unwrap();
        let project = Project { id: "p1".into(), name: "shop".into(), path: "/code/shop".into(), live_workspace_id: None, created_at: 1 };
        db.insert_project(&project).unwrap();
        db.insert_workspace(&sample_ws("w1", "p1", "one")).unwrap();
        db.insert_workspace(&sample_ws("w2", "p1", "two")).unwrap();
        let remark = |id: &str, ws: &str, at: i64| Remark {
            id: id.into(),
            workspace_id: ws.into(),
            path: "src/a.py".into(),
            side: "new".into(),
            start_line: 3,
            end_line: 4,
            snippet: "x = 1\ny = 2".into(),
            body: "Name these".into(),
            created_at: at,
        };
        db.save_remark(&remark("r2", "w1", 20)).unwrap();
        db.save_remark(&remark("r1", "w1", 10)).unwrap();
        db.save_remark(&remark("r3", "w2", 30)).unwrap();
        let mut edited = remark("r1", "w1", 10);
        edited.body = "Name these better".into();
        db.save_remark(&edited).unwrap();

        let w1 = db.remarks("w1").unwrap();
        assert_eq!(w1.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["r1", "r2"], "oldest first");
        assert_eq!(w1[0].body, "Name these better", "saving the same id edits it");

        db.delete_remark("r2").unwrap();
        assert_eq!(db.remarks("w1").unwrap().len(), 1);
        db.clear_remarks("w1").unwrap();
        assert!(db.remarks("w1").unwrap().is_empty());

        db.delete_workspace("w2").unwrap();
        assert!(db.remarks("w2").unwrap().is_empty(), "remarks go with their workspace");
    }
}
