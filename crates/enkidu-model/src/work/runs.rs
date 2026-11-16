//! A project's workflows as Settings says them: the latest times one acted or decided not to, and what the window says
//! when the morning summary ends.

use enkidu_core::work::workflows::Run;

use super::sources::ago;

/// What the Workflows part says under its title.
pub const WORKFLOWS_SAY: &str = "Each is off until you turn it on. Neither acts on stale or half-read data: each runs \
                                 only right after a full read of GitHub (and Jira), and says why when it doesn't.";
/// The switch for failing checks, and the line under it.
pub const FAILED_CHECKS: &str = "When checks fail, send them to their Claude";
pub const FAILED_CHECKS_SAYS: &str = "New failing checks on the latest commit of a PR whose session is in an Enkidu \
                                      workspace are typed into that Claude's prompt, for you to send. Once per commit.";
/// The switch for the morning summary, and the line under it (code between backticks).
pub const MORNING: &str = "Morning summary";
pub const MORNING_SAYS: &str = "Every morning at this time (or when Enkidu opens after it), GitHub and Jira are \
                                read fresh. If both read, Claude runs read-only in the main folder and writes one line \
                                per ticket on its timeline. It can only read files and run `enkidu` and gh and twg read \
                                commands; your own and the project's Claude flags aren't used.";
/// What a run says while it runs.
pub const RUN_RUNS: &str = "Reading GitHub and Jira, then Claude writes…";

/// How a run's lamp is lit: amber while it waits or runs, green done, gray skipped, red failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunLamp {
    Amber,
    Green,
    Gray,
    Red,
}

/// One time a workflow acted, or decided not to, as its line says it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunRow {
    pub id: i64,
    /// failed_checks or morning.
    pub workflow: String,
    /// pending, taken, running, done, skipped or failed.
    pub state: String,
    pub lamp: RunLamp,
    /// "Morning summary" or "Failing checks".
    pub name: &'static str,
    /// What happened, or why not (code between backticks); while it runs, that it reads and writes.
    pub words: String,
    /// Its words shimmer while it runs.
    pub running: bool,
    /// "2 min ago".
    pub age: String,
}

impl RunRow {
    /// The whole line: "Morning summary: Claude wrote 9 lines on the tickets' timelines., 2 min ago".
    pub fn said(&self) -> String {
        format!("{}: {}, {}", self.name, self.words, self.age)
    }
}

/// The latest runs, as the core gives them (the newest first).
pub fn run_rows(runs: &[Run], now: i64) -> Vec<RunRow> {
    runs.iter()
        .map(|run| {
            let running = run.state == "running";
            RunRow {
                id: run.id,
                workflow: run.workflow.clone(),
                state: run.state.clone(),
                lamp: match run.state.as_str() {
                    "done" => RunLamp::Green,
                    "skipped" => RunLamp::Gray,
                    "failed" => RunLamp::Red,
                    _ => RunLamp::Amber,
                },
                name: if run.workflow == "morning" { "Morning summary" } else { "Failing checks" },
                words: match (running, run.text.is_empty()) {
                    (true, _) => RUN_RUNS.to_string(),
                    (false, true) => run.state.clone(),
                    (false, false) => run.text.clone(),
                },
                running,
                age: ago(Some(run.at), now),
            }
        })
        .collect()
}

/// Whether a morning summary runs now: "Run now" waits for it.
pub fn morning_runs(runs: &[Run]) -> bool {
    runs.iter().any(|run| run.workflow == "morning" && run.state == "running")
}

/// How a run that ended is said in a toast.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RanKind {
    Done,
    Note,
    Failed,
}

/// What the window says when a run ends: only the morning summary says anything (Settings keeps the latest runs).
pub fn ran_toast(run: &Run) -> Option<(RanKind, &'static str, String)> {
    if run.workflow != "morning" {
        return None;
    }
    Some(match run.state.as_str() {
        "done" => (RanKind::Done, "Morning summary written", run.text.clone()),
        "skipped" => (RanKind::Note, "Morning summary", run.text.clone()),
        _ => (RanKind::Failed, "Morning summary", run.text.clone()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_794_132_000_000;

    fn run(id: i64, workflow: &str, state: &str, text: &str, ago: i64) -> Run {
        Run {
            id,
            project_id: "p1".into(),
            workflow: workflow.into(),
            subject: "2026-11-08".into(),
            state: state.into(),
            at: NOW - ago,
            text: text.into(),
        }
    }

    /// WorkflowsSection: each run's lamp, name, words and age.
    #[test]
    fn the_latest_runs_say_what_each_did_or_why_it_didnt() {
        let runs = [
            run(4, "morning", "running", "", 20_000),
            run(
                3,
                "morning",
                "skipped",
                "Didn't run: Jira can't be read: twg isn't signed in. Run `twg login --force`.",
                120_000,
            ),
            run(2, "failed_checks", "done", "Sent to the Claude in stock-fix", 3 * 3_600_000),
            run(1, "failed_checks", "failed", "", 26 * 3_600_000),
            run(0, "failed_checks", "pending", "", 27 * 3_600_000),
        ];
        let rows = run_rows(&runs, NOW);
        let lamps: Vec<RunLamp> = rows.iter().map(|row| row.lamp).collect();
        assert_eq!(lamps, [RunLamp::Amber, RunLamp::Gray, RunLamp::Green, RunLamp::Red, RunLamp::Amber]);
        assert_eq!(rows[0].said(), "Morning summary: Reading GitHub and Jira, then Claude writes…, just now");
        assert!(rows[0].running && !rows[1].running);
        assert_eq!(
            rows[1].said(),
            "Morning summary: Didn't run: Jira can't be read: twg isn't signed in. Run `twg login --force`., 2 min ago"
        );
        assert_eq!(rows[2].said(), "Failing checks: Sent to the Claude in stock-fix, 3 h ago");
        // a run that says nothing says how it stands
        assert_eq!(rows[3].said(), "Failing checks: failed, yesterday");
        assert_eq!(rows[4].words, "pending");
        // Run now waits while a morning summary runs
        assert!(morning_runs(&runs));
        assert!(!morning_runs(&runs[1..]));
    }

    /// The morning summary ended: how, in a toast.
    #[test]
    fn a_morning_summary_that_ends_says_how_and_other_workflows_say_nothing() {
        let written =
            run(5, "morning", "done", "Claude wrote 9 lines on the tickets' timelines. Summarized 9 tickets.", 0);
        assert_eq!(
            ran_toast(&written),
            Some((
                RanKind::Done,
                "Morning summary written",
                "Claude wrote 9 lines on the tickets' timelines. Summarized 9 tickets.".to_string()
            ))
        );
        let skipped = run(6, "morning", "skipped", "Didn't run: Jira can't be read", 0);
        assert_eq!(ran_toast(&skipped).map(|(kind, title, _)| (kind, title)), Some((RanKind::Note, "Morning summary")));
        let failed = run(7, "morning", "failed", "Claude ended with an error", 0);
        assert_eq!(
            ran_toast(&failed).map(|(kind, title, _)| (kind, title)),
            Some((RanKind::Failed, "Morning summary"))
        );
        assert_eq!(ran_toast(&run(8, "failed_checks", "done", "Sent", 0)), None);
    }
}
