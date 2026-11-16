//! What the failing-checks workflow queued, as the window sends it. The core decides what to queue, right after a full
//! read of GitHub; the window owns the terminals, so it types each one into the prompt of the Claude that worked on it,
//! never sends it, and tells the core how it went. It never acts on stale data, never starts a Claude, and never takes
//! you anywhere.

use std::collections::HashMap;

use enkidu_core::db::Workspace;
use enkidu_core::work::model::Snapshot;

use super::claudes::{ClaudeChoice, SendTarget, send_target, send_text};
use super::groups::Group;

/// A queued item that no longer waits on you (it is gone, done or snoozed).
pub const NO_LONGER_WAITS: &str = "it no longer waits on you.";
/// GitHub's last read of the project failed: nothing is decided on old data.
pub const GITHUB_UNREAD: &str = "GitHub can't be read right now.";
/// No Claude that worked on it is in a workspace of yours now.
pub const NO_WORKSPACE: &str = "its session isn't in an Enkidu workspace now.";
/// Its workspace is there, but its Claude took nothing.
pub const NOT_REACHED: &str = "Claude couldn't be reached.";

/// What to do with one queued item.
#[derive(Clone, Debug, PartialEq)]
pub enum Queued {
    /// Type `text` into the prompt of the Claude in `workspace`.
    Type {
        /// (boxed: a workspace is far larger than the other two ways out)
        workspace: Box<Workspace>,
        text: String,
        /// The ticket or PR its timeline's note goes on.
        subject: String,
        /// The item's own title, for the note and the toast.
        title: String,
    },
    /// Not sent, and why (the run is kept as skipped with these words).
    Skip(&'static str),
    /// Its workspace has no session that worked on it: the run failed.
    Fail(&'static str),
}

/// Decides one queued item (`item_id` in the project `project_id`) on the index as it is now. `chosen` is the Claude
/// you picked for each card: a pick of a new Claude is passed over (a workflow never starts one), a pick of a
/// session is kept.
pub fn queued_send(
    item_id: &str,
    project_id: &str,
    work: &Snapshot,
    groups: &[Group],
    workspaces: &[Workspace],
    statuses: &HashMap<String, String>,
    chosen: &HashMap<String, ClaudeChoice>,
) -> Queued {
    let found =
        groups.iter().find_map(|group| group.all_items(work).find(|item| item.id == item_id).map(|item| (group, item)));
    // never on stale data: the item still waits, and GitHub's last read was a good one
    let Some((group, item)) = found.filter(|(_, item)| item.state == "open") else {
        return Queued::Skip(NO_LONGER_WAITS);
    };
    let github = work.health.iter().find(|health| health.project_id == project_id && health.source == "github");
    if github.is_none_or(|health| health.state != "ok") {
        return Queued::Skip(GITHUB_UNREAD);
    }
    let pick = chosen.get(&group.id).filter(|pick| matches!(pick, ClaudeChoice::Session(_)));
    match send_target(item, group, work, workspaces, statuses, pick) {
        // only into a Claude that worked on it, in a workspace of yours
        SendTarget::Workspace { workspace, session: Some(_) } => Queued::Type {
            workspace: Box::new(workspace),
            text: send_text(item, group, work),
            subject: (item.ticket.clone())
                .or_else(|| item.pr.clone())
                .or_else(|| group.key.clone())
                .unwrap_or_default(),
            title: item.title.clone(),
        },
        SendTarget::Workspace { session: None, .. } => Queued::Fail(NOT_REACHED),
        _ => Queued::Skip(NO_WORKSPACE),
    }
}

/// What the run says once it is in Claude's prompt.
pub fn queued_done(workspace: &str) -> String {
    format!("In Claude's prompt in {workspace}, for you to send.")
}

/// The line on the item's timeline.
pub fn queued_note(workspace: &str, title: &str) -> String {
    format!("Put in Claude's prompt in {workspace} by the failing-checks workflow: {title}")
}

/// The toast: its title and the line under it.
pub fn queued_toast(workspace: &str, title: &str) -> (String, String) {
    (
        format!("In Claude's prompt in {workspace}"),
        format!("{title}: the failing-checks workflow typed it in. Press Enter there to send it."),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::work::groups::build_groups;
    use crate::work::groups::tests::{NOW, statuses, workspaces};
    use crate::work::sample;

    /// The sample's failing checks on PR #2414, whose branch the workspace sh-21309-export is on.
    const FAILING: &str = "checks:demo-org/shop#2414:4973abc0def1";

    fn decide(work: &Snapshot, list: &[Workspace], chosen: &HashMap<String, ClaudeChoice>) -> Queued {
        let lamps = statuses(&[("w1", "working"), ("w2", "idle")]);
        let groups = build_groups(work, list, &lamps, NOW);
        queued_send(FAILING, "p1", work, &groups, list, &lamps, chosen)
    }

    /// The failing-checks workflow: types what it queued into the prompt of the Claude in the PR's workspace, once,
    /// never presses Enter, and says the workflow did (what is typed and where, and its words; the typing itself is
    /// enkidu-app's tests/work_whole.rs).
    #[test]
    fn what_it_queued_goes_into_the_prompt_of_the_claude_in_the_prs_workspace() {
        let work = sample();
        let Queued::Type { workspace, text, subject, title } = decide(&work, &workspaces(), &HashMap::new()) else {
            panic!("it is typed in: {:?}", decide(&work, &workspaces(), &HashMap::new()));
        };
        assert_eq!(workspace.name, "sh-21309-export");
        assert!(text.contains("Checks are failing on the latest commit of PR #2414"), "{text}");
        assert_eq!(subject, "demo-org/shop#2414");
        assert_eq!(queued_done(&workspace.name), "In Claude's prompt in sh-21309-export, for you to send.");
        assert!(queued_note(&workspace.name, &title).contains("by the failing-checks workflow"));
        assert!(queued_toast(&workspace.name, &title).1.contains("the failing-checks workflow typed it in"));
    }

    /// "types into the PR's Claude even while the card has a new Claude picked: a workflow never starts one".
    #[test]
    fn it_goes_to_the_prs_claude_even_while_the_card_has_a_new_claude_picked() {
        let work = sample();
        for pick in [ClaudeChoice::New, ClaudeChoice::Fresh] {
            let chosen: HashMap<String, ClaudeChoice> = [("ticket:SH-21309".to_string(), pick)].into_iter().collect();
            let Queued::Type { workspace, .. } = decide(&work, &workspaces(), &chosen) else {
                panic!("it is typed in: {:?}", decide(&work, &workspaces(), &chosen));
            };
            assert_eq!(workspace.name, "sh-21309-export");
        }
    }

    /// "never sends on stale data or to a session outside Enkidu, and says why".
    #[test]
    fn never_on_stale_data_or_to_a_session_outside_and_it_says_why() {
        let none = HashMap::new();
        // gone
        let mut gone = sample();
        gone.items.retain(|item| item.id != FAILING);
        assert_eq!(decide(&gone, &workspaces(), &none), Queued::Skip("it no longer waits on you."));
        // done
        let mut done = sample();
        done.items.iter_mut().find(|item| item.id == FAILING).unwrap().state = "done".into();
        assert_eq!(decide(&done, &workspaces(), &none), Queued::Skip("it no longer waits on you."));
        // GitHub broken
        let mut broken = sample();
        broken.health.iter_mut().find(|health| health.source == "github").unwrap().state = "error".into();
        assert_eq!(decide(&broken, &workspaces(), &none), Queued::Skip("GitHub can't be read right now."));
        // its workspace was removed (a workspace on the PR's branch would be the PR's, linked or not)
        let mut alone = sample();
        alone.links.retain(|link| {
            !(link.a == "pr:demo-org/shop#2414" && (link.b == "session:s-w2" || link.b == "workspace:w2"))
        });
        let without: Vec<Workspace> = workspaces().into_iter().filter(|workspace| workspace.id != "w2").collect();
        assert_eq!(decide(&alone, &without, &none), Queued::Skip("its session isn't in an Enkidu workspace now."));
    }
}
