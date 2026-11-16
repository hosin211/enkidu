//! Claude's reports on its tasks (`enkidu report`): how a task ended (done and nothing needs you, done and wants your
//! review, or stuck), in Claude's own words, with screenshots of what it checked. They show on the task in the Work
//! page, so a run that went on while you slept reads like a handover in the morning. The images are copied into the
//! app's data folder (`reports/<id>/`) when the report is made: what Claude leaves in its folder can go, the report
//! stays.

use anyhow::{anyhow, bail, Result};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::db::Db;

/// How a task can end: done (nothing needs you), review (done, wants your review), blocked (couldn't finish).
pub const STATES: [&str; 3] = ["done", "review", "blocked"];
/// How it goes while Claude works (`enkidu update`): a word, maybe a screenshot; it ends nothing.
pub const UPDATE: &str = "update";
pub const MAX_IMAGES: usize = 12;
pub const MAX_IMAGE_BYTES: u64 = 25 * 1024 * 1024;
/// Characters of a report's text kept (a report is a handover, not a log)
pub const MAX_TEXT: usize = 6000;
const IMAGE_TYPES: [&str; 5] = ["png", "jpg", "jpeg", "gif", "webp"];

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub id: i64,
    /// What it is about: a ticket key (`SH-1`) or a PR reference (`owner/repo#2`)
    pub subject: String,
    pub session_id: Option<String>,
    pub workspace_id: Option<String>,
    /// done, review or blocked
    pub state: String,
    pub text: String,
    /// The screenshots' file names, in `reports/<id>/`, in the order given
    pub images: Vec<String>,
    pub at: i64,
}

/// What a state says, on the task's timeline and in `enkidu`'s answer.
pub fn words(state: &str) -> &'static str {
    match state {
        "done" => "Finished, nothing needs you",
        "review" => "Finished, wants your review",
        "update" => "Update",
        _ => "Stuck, needs you",
    }
}

/// Where a report's images are kept.
pub fn dir(data_dir: &Path, id: i64) -> PathBuf {
    data_dir.join("reports").join(id.to_string())
}

/// A picture file's type, from its name: png, jpg, jpeg, gif or webp.
fn image_type(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    IMAGE_TYPES.contains(&ext.as_str()).then_some(ext)
}

/// The name a copied image keeps: its place in the report, then its own name made safe ("2-orders-after.png").
fn kept_name(n: usize, path: &Path) -> String {
    let stem: String = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("image")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .take(48)
        .collect();
    let stem = stem.trim_matches('-');
    let ext = image_type(path).unwrap_or_else(|| "png".into());
    format!("{n}-{}.{ext}", if stem.is_empty() { "image" } else { stem })
}

/// Checks the images before anything is saved: each a picture file (by its name), there, and not too big.
pub fn check_images(paths: &[PathBuf]) -> Result<()> {
    if paths.len() > MAX_IMAGES {
        bail!("A report takes {MAX_IMAGES} images at most; this one has {}", paths.len());
    }
    for p in paths {
        if image_type(p).is_none() {
            bail!("{} isn't a picture Enkidu shows (png, jpg, gif or webp)", p.display());
        }
        let meta = std::fs::metadata(p).map_err(|_| anyhow!("There is no file at {}", p.display()))?;
        if !meta.is_file() {
            bail!("{} isn't a file", p.display());
        }
        if meta.len() > MAX_IMAGE_BYTES {
            bail!("{} is over {} MB", p.display(), MAX_IMAGE_BYTES / 1024 / 1024);
        }
    }
    Ok(())
}

/// Saves a report, copying its images in. `text` is cut to `MAX_TEXT` characters.
#[allow(clippy::too_many_arguments)]
pub fn save(
    db: &Db,
    data_dir: &Path,
    subject: &str,
    session: Option<&str>,
    workspace: Option<&str>,
    state: &str,
    text: &str,
    images: &[PathBuf],
    at: i64,
) -> Result<Report> {
    if !STATES.contains(&state) && state != UPDATE {
        bail!("A report says done, review or blocked, not \"{state}\"");
    }
    let text: String = text.trim().chars().take(MAX_TEXT).collect();
    if text.is_empty() {
        bail!("Say what you did: enkidu report {state} \"what changed, how you checked it, what is left\"");
    }
    check_images(images)?;
    let id = db.work_put_report(subject, session, workspace, state, &text, at)?;
    let folder = dir(data_dir, id);
    let mut kept = Vec::new();
    let copied = (|| -> Result<()> {
        if !images.is_empty() {
            std::fs::create_dir_all(&folder)?;
        }
        for (n, p) in images.iter().enumerate() {
            let name = kept_name(n + 1, p);
            std::fs::copy(p, folder.join(&name)).map_err(|e| anyhow!("Can't copy {}: {e}", p.display()))?;
            kept.push(name);
        }
        Ok(())
    })();
    if let Err(e) = copied {
        // all or nothing: a report whose images didn't come along is not made
        let _ = db.work_delete_report(id);
        let _ = std::fs::remove_dir_all(&folder);
        return Err(e);
    }
    db.work_set_report_images(id, &kept)?;
    Ok(Report {
        id,
        subject: subject.to_string(),
        session_id: session.map(String::from),
        workspace_id: workspace.map(String::from),
        state: state.to_string(),
        text,
        images: kept,
        at,
    })
}

/// One of a report's images, as bytes with its type (`image/png`). Only a name the report lists is read.
pub fn image(db: &Db, data_dir: &Path, id: i64, name: &str) -> Result<(Vec<u8>, &'static str)> {
    let report = db.work_report(id)?.ok_or_else(|| anyhow!("That report is gone"))?;
    if !report.images.iter().any(|i| i == name) {
        bail!("The report has no image {name}");
    }
    let bytes = std::fs::read(dir(data_dir, id).join(name))?;
    let kind = match image_type(Path::new(name)).as_deref() {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        _ => "image/png",
    };
    Ok((bytes, kind))
}

fn report_from_row(r: &rusqlite::Row) -> rusqlite::Result<Report> {
    let images: String = r.get(6)?;
    Ok(Report {
        id: r.get(0)?,
        subject: r.get(1)?,
        session_id: r.get(2)?,
        workspace_id: r.get(3)?,
        state: r.get(4)?,
        text: r.get(5)?,
        images: serde_json::from_str(&images).unwrap_or_default(),
        at: r.get(7)?,
    })
}

const COLS: &str = "id, subject, session_id, workspace_id, state, text, images, at";

impl Db {
    pub fn work_put_report(&self, subject: &str, session: Option<&str>, workspace: Option<&str>, state: &str, text: &str, at: i64) -> Result<i64> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO work_reports (subject, session_id, workspace_id, state, text, images, at) VALUES (?1, ?2, ?3, ?4, ?5, '[]', ?6)",
            params![subject, session, workspace, state, text, at],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn work_set_report_images(&self, id: i64, images: &[String]) -> Result<()> {
        self.conn().execute("UPDATE work_reports SET images = ?2 WHERE id = ?1", params![id, serde_json::to_string(images)?])?;
        Ok(())
    }

    pub fn work_delete_report(&self, id: i64) -> Result<()> {
        self.conn().execute("DELETE FROM work_reports WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn work_report(&self, id: i64) -> Result<Option<Report>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!("SELECT {COLS} FROM work_reports WHERE id = ?1"))?;
        let mut rows = stmt.query_map(params![id], report_from_row)?;
        Ok(rows.next().transpose()?)
    }

    /// The reports made since `since`, the newest first.
    pub fn work_reports(&self, since: i64) -> Result<Vec<Report>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!("SELECT {COLS} FROM work_reports WHERE at >= ?1 ORDER BY at DESC, id DESC"))?;
        let rows = stmt.query_map(params![since], report_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> (tempfile::TempDir, Db) {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("t.db")).unwrap();
        (dir, db)
    }

    #[test]
    fn keeps_a_report_with_copies_of_its_images() {
        let (dir, db) = db();
        let shots = dir.path().join("shots");
        std::fs::create_dir_all(&shots).unwrap();
        std::fs::write(shots.join("orders after.PNG"), b"\x89PNG one").unwrap();
        std::fs::write(shots.join("export.jpg"), b"jpeg two").unwrap();
        let data = dir.path().join("data");
        let r = save(&db, &data, "SH-1", Some("s1"), Some("w1"), "review", "  Fixed the CSV header.\nChecked the export.  ", &[shots.join("orders after.PNG"), shots.join("export.jpg")], 5).unwrap();
        assert_eq!(r.images, ["1-orders-after.png", "2-export.jpg"]);
        assert_eq!(r.text, "Fixed the CSV header.\nChecked the export.");
        // Claude's own files can go; the report keeps its copies
        std::fs::remove_dir_all(&shots).unwrap();
        assert_eq!(image(&db, &data, r.id, "1-orders-after.png").unwrap(), (b"\x89PNG one".to_vec(), "image/png"));
        assert_eq!(image(&db, &data, r.id, "2-export.jpg").unwrap().1, "image/jpeg");
        assert!(image(&db, &data, r.id, "../../enkidu.db").is_err(), "only the images the report lists");
        assert_eq!(db.work_reports(0).unwrap(), vec![r]);
        assert_eq!(words("review"), "Finished, wants your review");
    }

    #[test]
    fn refuses_what_isnt_a_report_and_leaves_nothing_behind() {
        let (dir, db) = db();
        let data = dir.path().join("data");
        let txt = dir.path().join("notes.txt");
        std::fs::write(&txt, "x").unwrap();
        assert!(save(&db, &data, "SH-1", None, None, "finished", "Done.", &[], 1).unwrap_err().to_string().contains("done, review or blocked"));
        assert!(save(&db, &data, "SH-1", None, None, "done", "   ", &[], 1).unwrap_err().to_string().contains("Say what you did"));
        assert!(save(&db, &data, "SH-1", None, None, "done", "Done.", &[txt], 1).unwrap_err().to_string().contains("isn't a picture"));
        assert!(save(&db, &data, "SH-1", None, None, "done", "Done.", &[dir.path().join("gone.png")], 1).unwrap_err().to_string().contains("no file"));
        let many: Vec<PathBuf> = (0..13).map(|n| dir.path().join(format!("{n}.png"))).collect();
        assert!(save(&db, &data, "SH-1", None, None, "done", "Done.", &many, 1).unwrap_err().to_string().contains("12 images at most"));
        assert!(db.work_reports(0).unwrap().is_empty());
        // a long report is cut, not refused
        let r = save(&db, &data, "SH-1", None, None, "done", &"a".repeat(MAX_TEXT + 50), &[], 2).unwrap();
        assert_eq!(r.text.chars().count(), MAX_TEXT);
    }
}
