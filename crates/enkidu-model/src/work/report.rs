//! Claude's reports on a card (`enkidu report`), as the card in front of the board says them: how the task ended first,
//! in Claude's words, with its screenshots; the earlier reports and the updates along the way a click away. And the
//! plain arithmetic of the screenshots: what a thumbnail shows of a picture, how large a picture is kept, which one
//! comes next.

use enkidu_core::db::Workspace;
use enkidu_core::work::model::Snapshot;

use super::groups::Group;
use super::page::report_words;
use super::sources::ago;

/// One report, as the card says it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReportRow {
    pub id: i64,
    /// done, review, blocked or update.
    pub state: String,
    /// "Finished, wants your review".
    pub words: &'static str,
    /// "1 min ago".
    pub when: String,
    /// The workspace whose Claude made it, while it is one of yours: its id and its name.
    pub workspace: Option<(String, String)>,
    /// Claude's own words, as Markdown.
    pub text: String,
    /// The screenshots' names, in the order Claude gave them.
    pub images: Vec<String>,
}

impl ReportRow {
    /// One report as it is said: its state's words, its age, and the workspace whose Claude made it.
    pub fn of(report: &enkidu_core::work::reports::Report, workspaces: &[Workspace], now: i64) -> ReportRow {
        let workspace = (report.workspace_id.as_deref())
            .and_then(|id| workspaces.iter().find(|workspace| workspace.id == id))
            .map(|workspace| (workspace.id.clone(), workspace.name.clone()));
        ReportRow {
            id: report.id,
            state: report.state.clone(),
            words: report_words(&report.state).1,
            when: ago(Some(report.at), now),
            workspace,
            text: report.text.clone(),
            images: report.images.clone(),
        }
    }

    /// All of its head in one line: "Finished, wants your review, 1 min ago by the Claude in sh-21309-export".
    pub fn said(&self) -> String {
        match &self.workspace {
            Some((_, name)) => format!("{}, {} by the Claude in {name}", self.words, self.when),
            None => format!("{}, {}", self.words, self.when),
        }
    }
}

/// Claude's reports on a card: how it ended first (the newest such), then the rest, the newest first, its updates
/// along the way too.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reports {
    /// "Claude's report" when one says how the task ended, else "How it goes".
    pub heading: &'static str,
    pub rows: Vec<ReportRow>,
}

impl Reports {
    /// None for a card Claude never reported on.
    pub fn of(group: &Group, work: &Snapshot, workspaces: &[Workspace], now: i64) -> Option<Reports> {
        let all: Vec<_> = group.reports(work).collect();
        let ended = all.iter().position(|report| report.state != "update");
        let order = ended.into_iter().chain((0..all.len()).filter(|at| Some(*at) != ended));
        let rows: Vec<ReportRow> = order.map(|at| ReportRow::of(all[at], workspaces, now)).collect();
        (!rows.is_empty())
            .then(|| Reports { heading: if ended.is_some() { "Claude's report" } else { "How it goes" }, rows })
    }

    /// How many there are, said beside the heading when there are several.
    pub fn count(&self) -> Option<usize> {
        (self.rows.len() > 1).then_some(self.rows.len())
    }

    /// The words that open the earlier ones: "Earlier reports (1)", "Earlier reports and updates (3)". None when
    /// there is one report.
    pub fn more(&self) -> Option<String> {
        let earlier = self.rows.get(1..).filter(|earlier| !earlier.is_empty())?;
        let words = match earlier.iter().any(|row| row.state == "update") {
            true => "Earlier reports and updates",
            false => "Earlier reports",
        };
        Some(format!("{words} ({})", earlier.len()))
    }
}

/// A screenshot's name without its place in the report: "2-orders.png" is "orders.png".
pub fn shot_name(name: &str) -> &str {
    let digits = name.bytes().take_while(u8::is_ascii_digit).count();
    match name[digits..].strip_prefix('-') {
        Some(rest) if digits > 0 => rest,
        _ => name,
    }
}

/// The screenshot after this one (`by` 1) or before it (-1), around the ends.
pub fn step_shot(count: usize, at: usize, by: i32) -> usize {
    if count == 0 {
        return 0;
    }
    (at as i64 + by as i64).rem_euclid(count as i64) as usize
}

/// How many columns a report's thumbnails stand in: one screenshot alone, several in two columns, three in a window
/// at least 1,000 px wide.
pub fn shot_columns(count: usize, window_width: f32) -> usize {
    match count {
        0 | 1 => 1,
        _ if window_width >= 1000. => 3,
        _ => 2,
    }
}

/// The same on a whole page (a task of a workflow's run, where the screenshots are large): several stand in one
/// column, in two in a window at least 1,100 px wide.
pub fn shot_columns_large(count: usize, window_width: f32) -> usize {
    match count {
        0 | 1 => 1,
        _ if window_width >= 1100. => 2,
        _ => 1,
    }
}

/// A screenshot alone is at most this wide.
pub const LONE_SHOT: f32 = 460.;
/// On a whole page, one alone is at most this wide.
pub const LONE_SHOT_LARGE: f32 = 880.;
/// How wide a large thumbnail is kept at most: one alone (880 px on a screen with two pixels a point), and one of
/// several (640 px each).
pub const THUMB_WIDTH_LARGE: u32 = 1760;
pub const THUMB_WIDTH_LARGE_OF_MANY: u32 = 1280;
/// A thumbnail is 16 wide to 10 high.
pub const THUMB_SHAPE: (u32, u32) = (16, 10);
/// A thumbnail is kept this wide at most, twice the widest it is drawn (for a screen with two pixels a point): one
/// alone is drawn up to 460 px wide, one of several up to 318.
pub const THUMB_WIDTH: u32 = 920;
pub const THUMB_WIDTH_OF_MANY: u32 = 640;

/// The part of a picture a thumbnail shows, and how large it is kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Thumb {
    /// The part: left, top, width, height, in the picture's own pixels.
    pub part: (u32, u32, u32, u32),
    /// How large that part is kept.
    pub size: (u32, u32),
}

/// What a thumbnail shows of a picture `width` by `height`: it fills the thumbnail's 16 to 10 from the top, as wide
/// as the picture (a picture wider than that shows its middle, top to bottom), and is never kept wider than `most`
/// ([`THUMB_WIDTH`] for a screenshot alone, [`THUMB_WIDTH_OF_MANY`] for one of several). None for a picture without
/// pixels.
pub fn thumb_of(width: u32, height: u32, most: u32) -> Option<Thumb> {
    if width == 0 || height == 0 {
        return None;
    }
    let (wide, high) = (THUMB_SHAPE.0 as u64, THUMB_SHAPE.1 as u64);
    let (w, h) = (width as u64, height as u64);
    let part = if w * high > h * wide {
        // wider than the thumbnail: all of its height, the middle of its width
        let part_w = (h * wide / high).max(1);
        (((w - part_w) / 2) as u32, 0, part_w as u32, height)
    } else {
        // as wide as it is, from the top
        (0, 0, width, ((w * high / wide).max(1) as u32).min(height))
    };
    let kept_w = part.2.min(most.max(1));
    let kept_h = ((part.3 as u64 * kept_w as u64) as f64 / part.2 as f64).round().max(1.) as u32;
    Some(Thumb { part, size: (kept_w, kept_h) })
}

/// How large a picture `width` by `height` is kept to be shown whole inside `room`: never larger than it is, never
/// larger than the room (the window's own pixels), its shape kept.
pub fn fit_within(width: u32, height: u32, room: (u32, u32)) -> (u32, u32) {
    if width == 0 || height == 0 || room.0 == 0 || room.1 == 0 {
        return (width, height);
    }
    let scale = (room.0 as f64 / width as f64).min(room.1 as f64 / height as f64).min(1.);
    (((width as f64 * scale).round() as u32).max(1), ((height as f64 * scale).round() as u32).max(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::work::groups::build_groups;
    use crate::work::groups::tests::{NOW, ws};
    use crate::work::sample;
    use enkidu_core::work::reports::Report;
    use std::collections::HashMap;

    fn report(id: i64, state: &str, text: &str, images: &[&str], ago: i64) -> Report {
        Report {
            id,
            subject: "SH-21309".into(),
            session_id: Some("s-w2".into()),
            workspace_id: Some("w2".into()),
            state: state.into(),
            text: text.into(),
            images: images.iter().map(|name| name.to_string()).collect(),
            at: NOW - ago,
        }
    }

    fn reports_of(work: &Snapshot, workspaces: &[Workspace]) -> Option<Reports> {
        let groups = build_groups(work, workspaces, &HashMap::new(), NOW);
        let group = groups.iter().find(|group| group.id == "ticket:SH-21309").expect("the export card");
        Reports::of(group, work, workspaces, NOW)
    }

    /// Shows how Claude says a task ended: on the card, then its report and screenshots in the task.
    #[test]
    fn says_how_claude_says_a_task_ended_then_the_earlier_reports() {
        let mut work = sample();
        assert_eq!(reports_of(&work, &[]), None, "a card Claude never reported on has no such section");
        work.reports = vec![
            report(
                7,
                "review",
                "Fixed the CSV header.\nChecked the export with 3 orders.",
                &["1-export.png", "2-orders.png"],
                60_000,
            ),
            report(3, "blocked", "The export needs a decision on the date format.", &[], 86_400_000),
        ];
        let workspaces = [ws("w2", "sh-21309-export", "feature/SH-21309-export")];
        let reports = reports_of(&work, &workspaces).expect("its reports");
        assert_eq!((reports.heading, reports.count()), ("Claude's report", Some(2)));
        let first = &reports.rows[0];
        assert_eq!((first.id, first.state.as_str()), (7, "review"));
        assert_eq!(first.said(), "Finished, wants your review, 1 min ago by the Claude in sh-21309-export");
        assert!(first.text.contains("Checked the export with 3 orders."));
        assert_eq!(first.images, ["1-export.png", "2-orders.png"]);
        // the earlier report is a click away
        assert_eq!(reports.more().as_deref(), Some("Earlier reports (1)"));
        let states: Vec<&str> = reports.rows.iter().map(|row| row.state.as_str()).collect();
        assert_eq!(states, ["review", "blocked"]);
        assert_eq!(reports.rows[1].said(), "Stuck, needs you, yesterday by the Claude in sh-21309-export");
        // a workspace that is gone isn't named
        let alone = reports_of(&work, &[]).expect("its reports");
        assert_eq!(alone.rows[0].said(), "Finished, wants your review, 1 min ago");
        assert_eq!(alone.rows[0].workspace, None);
    }

    /// How it ended comes first, also when an update is newer; with only updates the section is "How it goes".
    #[test]
    fn how_it_ended_comes_before_a_newer_update_and_updates_alone_say_how_it_goes() {
        let mut work = sample();
        work.reports = vec![
            report(9, "update", "Running the migrations now.", &[], 30_000),
            report(8, "done", "All 14 tests pass.", &["1-tests.png"], 120_000),
            report(6, "update", "Reading the importer.", &[], 600_000),
        ];
        let reports = reports_of(&work, &[]).expect("its reports");
        assert_eq!(reports.heading, "Claude's report");
        let order: Vec<i64> = reports.rows.iter().map(|row| row.id).collect();
        assert_eq!(order, [8, 9, 6]);
        assert_eq!(reports.rows[0].words, "Finished, nothing needs you");
        assert_eq!(reports.more().as_deref(), Some("Earlier reports and updates (2)"));

        work.reports = vec![report(9, "update", "Running the migrations now.", &[], 30_000)];
        let going = reports_of(&work, &[]).expect("its update");
        assert_eq!((going.heading, going.count(), going.more()), ("How it goes", None, None));
        assert_eq!(going.rows[0].said(), "Update, just now");
    }

    /// The large screenshot's name leaves its number out, and the arrows go around the ends.
    #[test]
    fn names_a_screenshot_and_steps_through_them_around_the_ends() {
        assert_eq!(shot_name("2-orders.png"), "orders.png");
        assert_eq!(shot_name("12-after-fix.jpg"), "after-fix.jpg");
        assert_eq!(shot_name("orders.png"), "orders.png");
        assert_eq!(shot_name("-orders.png"), "-orders.png");
        assert_eq!(shot_name("7"), "7");
        // → from the last is the first, ← from the first the last
        assert_eq!((step_shot(2, 1, 1), step_shot(2, 0, -1), step_shot(3, 1, 1)), (0, 1, 2));
        assert_eq!((step_shot(1, 0, 1), step_shot(0, 0, 1)), (0, 0));
        // one alone, two columns, three in a wide window
        assert_eq!((shot_columns(1, 1400.), shot_columns(2, 999.), shot_columns(2, 1000.)), (1, 2, 3));
    }

    #[test]
    fn a_thumbnail_shows_a_picture_from_its_top_and_is_kept_small() {
        // a retina screenshot, 16 to 10 itself: all of it, kept 920 wide when it is alone, 640 as one of several
        let alone = THUMB_WIDTH;
        assert_eq!(thumb_of(2880, 1800, alone), Some(Thumb { part: (0, 0, 2880, 1800), size: (920, 575) }));
        assert_eq!(
            thumb_of(2880, 1800, THUMB_WIDTH_OF_MANY),
            Some(Thumb { part: (0, 0, 2880, 1800), size: (640, 400) })
        );
        // a whole page, far taller: its top, as wide as it is
        assert_eq!(thumb_of(1280, 6000, alone), Some(Thumb { part: (0, 0, 1280, 800), size: (920, 575) }));
        // a wide strip: its middle, top to bottom
        assert_eq!(thumb_of(3000, 500, alone), Some(Thumb { part: (1100, 0, 800, 500), size: (800, 500) }));
        // a small picture is never made larger
        assert_eq!(thumb_of(320, 200, alone), Some(Thumb { part: (0, 0, 320, 200), size: (320, 200) }));
        assert_eq!(thumb_of(0, 200, alone), None);
        // large: whole, inside the window's own pixels, never larger than it is
        assert_eq!(fit_within(2880, 1800, (2400, 1600)), (2400, 1500));
        assert_eq!(fit_within(1280, 6000, (2400, 1600)), (341, 1600));
        assert_eq!(fit_within(640, 400, (2400, 1600)), (640, 400));
        assert_eq!(fit_within(640, 400, (0, 0)), (640, 400));
    }
}
