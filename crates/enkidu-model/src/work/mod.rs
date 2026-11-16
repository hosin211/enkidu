//! The Work page's plain logic (phase 5):
//!
//! - [`sources`]: how GitHub and Jira stand (the lamps, the ages, a source that can't be read);
//! - [`pr`]: a PR's checks and lamp, a workspace's PRs, what waits on you said in a few words;
//! - [`groups`]: one group per ticket (or per PR that names no ticket), with its PRs, items and Claudes;
//! - [`board`]: the groups as cards in columns, a column per Jira status, and what a card says.
//! - [`page`]: what the page shows from the index and your choices (the Work tab's mark, the first run).
//! - [`card`]: the card in front of the board: its header, its tabs, its timeline, Jira's and each PR's facts.
//! - [`check`]: Check connections: what each source's comparison with `gh` and `twg` says.
//! - [`claudes`]: which Claude a card goes to, where a message for it goes, a ticket's workspace and branch.
//! - [`compose`]: what you tell Claude about a card, as one message, and the draft of it.
//! - [`prtab`]: what is going on in a PR, as its tab in the card says it.
//! - [`queued`]: what the failing-checks workflow queued: into which Claude's prompt it goes, or why not.
//! - [`report`]: Claude's reports on a card, how it ended first, and the arithmetic of their screenshots.
//! - [`rich`]: Jira's document and GitHub's Markdown as the same blocks of text, never run and never fetched.
//! - [`runs`]: a project's workflows as Settings says them: the latest runs, and a summary that ended.
//! - [`markdown`]: GitHub's Markdown read into those blocks.
//! - [`flows`] and [`when`]: your workflows (each task's message, how one stands, the cards in one that is due or
//!   runs), and a moment in the Mac's own time.
//! - [`flow_editor`]: the workflow dialog as it decides: what it holds, what is missing, what each task's row shows.
//! - [`flows_page`]: the Workflows page as it decides: what is still to run and every run kept, the one shown with
//!   its tasks, and one task across the page (how it stands, what it was given, what happened).

pub mod board;
pub mod card;
pub mod check;
pub mod claudes;
pub mod compose;
pub mod flow_editor;
pub mod flows;
pub mod flows_page;
pub mod groups;
pub mod markdown;
#[cfg(test)]
mod own_tests;
pub mod page;
pub mod pr;
pub mod prtab;
pub mod queued;
pub mod report;
pub mod rich;
pub mod runs;
pub mod sources;
pub mod when;

pub use sources::*;

/// The sample index, read as the core's snapshot (the tests of every module here stand on it).
#[cfg(test)]
pub(crate) fn sample() -> Snapshot {
    serde_json::from_str(enkidu_testkit::WORK_SAMPLE).expect("the sample Work index reads as the core's Snapshot")
}
