//! What you tell Claude about a card (DESIGN.md, "The Work page"): your words about the task, the ticket and PR it is
//! about (their links always go with it), and what waits on you there, each with your own instruction or the usual one.
//! One message, to the Claude that knows the work; it asks for drafts of any reply, never to post one.

use std::collections::BTreeMap;

use enkidu_core::work::model::{Item, Snapshot};
use serde::{Deserialize, Serialize};

use super::claudes::SendTarget;
use super::groups::Group;

/// Your say about one item: left out of the message, and your instruction for it.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct ItemDraft {
    /// False: left out of the message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include: Option<bool>,
    /// How Claude should handle it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// What you wrote about a card, kept until it goes (also across a restart).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Draft {
    /// What to do about the task, in your words.
    #[serde(default)]
    pub note: String,
    /// Per item, by its id.
    #[serde(default)]
    pub items: BTreeMap<String, ItemDraft>,
    /// The Claude you picked for the card: "new", "fresh" (a new workspace), or a session's id (none: the likeliest).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude: Option<String>,
    /// The whole message as you edited it; it is what goes to Claude (none: the one made from the parts above).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

impl Draft {
    /// Whether an item goes with the message: every one does, unless you left it out.
    pub fn includes(&self, item: &str) -> bool {
        self.items.get(item).and_then(|say| say.include) != Some(false)
    }

    /// Your instruction for an item, if you wrote one.
    pub fn note_for(&self, item: &str) -> &str {
        self.items.get(item).and_then(|say| say.note.as_deref()).unwrap_or("")
    }
}

/// Everything that waits on you on a card: its PRs' items, then the ticket's own.
pub fn card_items<'a>(group: &'a Group, work: &'a Snapshot) -> Vec<&'a Item> {
    group.prs.iter().flat_map(|pr| pr.items(work)).chain(group.items(work)).collect()
}

/// The items that go with the message: every one that waits, unless you left it out.
pub fn picked<'a>(group: &'a Group, work: &'a Snapshot, draft: &Draft) -> Vec<&'a Item> {
    card_items(group, work).into_iter().filter(|item| draft.includes(&item.id)).collect()
}

/// Has the draft anything of yours in it (your words, an instruction, an item left out, the message edited)?
pub fn touched(draft: Option<&Draft>) -> bool {
    draft.is_some_and(|draft| {
        !draft.note.trim().is_empty()
            || draft.text.is_some()
            || draft
                .items
                .values()
                .any(|say| say.include == Some(false) || !say.note.as_deref().unwrap_or("").trim().is_empty())
    })
}

fn quote(text: &str) -> String {
    text.trim().split('\n').map(|line| format!("   > {line}")).collect::<Vec<_>>().join("\n")
}

/// What follows the first colon of an item's title ("Back to you: QA Review to In Progress").
fn after_colon(title: &str) -> &str {
    title.split_once(':').map_or(title, |(_, rest)| rest.trim_start())
}

/// One item in a line or two: who, where, and what they said.
fn describe(item: &Item, group: &Group, work: &Snapshot) -> Vec<String> {
    let pr = (item.pr.as_deref())
        .and_then(|reference| group.prs.iter().map(|view| view.of(work)).find(|pr| pr.reference() == reference));
    let on = match (pr, &group.key) {
        (Some(pr), _) => format!("PR #{}", pr.number),
        (None, Some(key)) => format!("{key} in Jira"),
        (None, None) => "the PR".to_string(),
    };
    let place = match (item.path.as_deref().filter(|path| !path.is_empty()), item.line.filter(|line| *line != 0)) {
        (Some(path), Some(line)) => format!(" on {path}:{line}"),
        (Some(path), None) => format!(" on {path}"),
        _ => String::new(),
    };
    let who = item.who.as_deref().unwrap_or("Someone");
    let ticket = group.key.as_deref();
    let said = (!item.detail.trim().is_empty()).then(|| quote(&item.detail));
    let with = |head: String| -> Vec<String> { std::iter::once(head).chain(said.clone()).collect() };
    match item.kind.as_str() {
        "remark" => with(format!("{who} left a remark{place} ({on}):")),
        "probably_fixed" => {
            with(format!("{who}'s remark{place} ({on}) looks fixed by a later push, but has no reply yet:"))
        }
        "question" => with(format!("{who} asked on {on}:")),
        "changes_requested" => vec![format!("{who} requested changes on {on}.")],
        "pipeline_failed" => {
            let detail = item.detail.trim();
            let checks = if detail.is_empty() { "see the checks" } else { detail };
            vec![format!("Checks are failing on the latest commit of {on}: {checks}.")]
        }
        "conflict" => vec![format!("{on} conflicts with {}.", pr.map_or("its base", |pr| pr.base.as_str()))],
        "qa_replied" => with(format!("{who} replied on {} in Jira:", ticket.unwrap_or("the ticket"))),
        "mentioned" => with(format!("{who} mentioned you on {} in Jira:", ticket.unwrap_or("the ticket"))),
        "back_to_you" => {
            vec![format!("{} came back to you in Jira: {}.", ticket.unwrap_or("The ticket"), after_colon(&item.title))]
        }
        _ => vec![format!(
            "{who} added an attachment to {} in Jira: {}.",
            ticket.unwrap_or("the ticket"),
            after_colon(&item.title)
        )],
    }
}

/// What Claude does about an item when you didn't say.
pub fn usual(kind: &str) -> &'static str {
    match kind {
        "remark" => "Fix it if it's right; if not, draft a reply that says why.",
        "probably_fixed" => "Check that it is fixed, and draft the reply.",
        "question" => "Draft an answer.",
        "changes_requested" => "Go through their review and its comments.",
        "pipeline_failed" => "Find out why and fix it.",
        "conflict" => "Bring the base into the branch and resolve the conflicts.",
        "qa_replied" => "Fix what they found, or draft an answer.",
        "mentioned" => "Draft an answer.",
        "back_to_you" => "Find out why it came back.",
        _ => "Look at it and say what it changes.",
    }
}

/// How every message ends: fixes in the code, replies as drafts, nothing posted.
pub const NEVER_POST: &str = "Fix what is right in the code. For anything that needs an answer, draft the reply here \
                              for me; don't post anything to GitHub or Jira.";

/// The message: what the card is (its links), your words, then each item you kept with your instruction for it.
pub fn task_text(group: &Group, work: &Snapshot, draft: &Draft) -> String {
    let mut lines: Vec<String> = Vec::new();
    let first = group.prs.first().map(|view| view.of(work));
    lines.push(match (&group.key, first) {
        (Some(key), _) => format!("{key}: {}", group.title),
        (None, Some(pr)) => format!("PR #{}: {}", pr.number, group.title),
        // a task of your own: there is nothing to look up, so the message says it is yours
        (None, None) if group.own.is_some() => format!("A task of mine (not in Jira): {}", group.title),
        (None, None) => group.title.clone(),
    });
    // and what you wrote about it
    if let Some(notes) = group.own(work).map(|task| task.notes.as_str()).filter(|notes| !notes.is_empty()) {
        lines.push(String::new());
        lines.push(notes.to_string());
    }
    if let Some(url) = group.ticket(work).map(|ticket| ticket.url.as_str()).filter(|url| !url.is_empty()) {
        lines.push(format!("Ticket: {url}"));
    }
    // the PRs that are still open (a merged one only when it is all there is)
    let open: Vec<_> = group.prs.iter().map(|view| view.of(work)).filter(|pr| pr.state == "OPEN").collect();
    let all: Vec<_> = group.prs.iter().map(|view| view.of(work)).collect();
    for pr in if open.is_empty() { &all } else { &open } {
        lines.push(format!("PR #{} ({} into {}): {}", pr.number, pr.head, pr.base, pr.url));
    }
    let note = draft.note.trim();
    if !note.is_empty() {
        lines.push(String::new());
        lines.push(note.to_string());
    }
    let items = picked(group, work, draft);
    if !items.is_empty() {
        lines.push(String::new());
        lines.push(if items.len() == 1 { "Address this:" } else { "Address these:" }.to_string());
        for (n, item) in items.iter().enumerate() {
            let mut said = describe(item, group, work).into_iter();
            lines.push(format!("{}. {}", n + 1, said.next().unwrap_or_default()));
            lines.extend(said);
            let mine = draft.note_for(&item.id).trim();
            lines.push(if mine.is_empty() {
                format!("   {}", usual(&item.kind))
            } else {
                format!("   What to do: {mine}")
            });
            if let Some(url) = item.url.as_deref().filter(|url| !url.is_empty()) {
                lines.push(format!("   Link: {url}"));
            }
        }
    }
    lines.push(String::new());
    lines.push(NEVER_POST.to_string());
    lines.join("\n")
}

/// What goes to Claude: the message as you edited it, else the one made from the card and your words.
pub fn message_of(group: &Group, work: &Snapshot, draft: &Draft) -> String {
    draft.text.clone().unwrap_or_else(|| task_text(group, work, draft))
}

/// What waits on a card, as lines of a message: each item (who, where, what they said) with what Claude does about
/// it. A workflow's task carries them, so a ticket that came back brings what QA said with it.
pub fn item_lines(group: &Group, work: &Snapshot, items: &[&Item]) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for (n, item) in items.iter().enumerate() {
        let mut said = describe(item, group, work).into_iter();
        lines.push(format!("{}. {}", n + 1, said.next().unwrap_or_default()));
        lines.extend(said);
        lines.push(format!("   {}", usual(&item.kind)));
        if let Some(url) = item.url.as_deref().filter(|url| !url.is_empty()) {
            lines.push(format!("   Link: {url}"));
        }
    }
    lines
}

/// A piece of the line that says where the message goes: plain words, a workspace's name (brighter), or a branch
/// (in the code face).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Said {
    Words(String),
    Name(String),
    Branch(String),
}

/// Where a card's message goes, as its kind: what the button does follows from it.
pub fn target_kind(target: &SendTarget) -> &'static str {
    match target {
        SendTarget::Workspace { .. } => "workspace",
        SendTarget::Switch { .. } => "switch",
        SendTarget::Outside { .. } => "outside",
        SendTarget::New { .. } => "new",
        SendTarget::None => "none",
    }
}

/// Where the message goes, in a sentence (the line beside "Tell Claude").
pub fn target_said(target: &SendTarget) -> Vec<Said> {
    let words = |words: &str| Said::Words(words.to_string());
    match target {
        SendTarget::Workspace { workspace, .. } => vec![
            words("goes into the prompt of the Claude in "),
            Said::Name(workspace.name.clone()),
            words("; you press Enter there"),
        ],
        SendTarget::Switch { workspace, session: Some(_) } => vec![
            words("resumes its conversation in "),
            Said::Name(workspace.name.clone()),
            words(", with this in its prompt; you press Enter there"),
        ],
        SendTarget::Switch { workspace, session: None } => vec![
            words("a new conversation in "),
            Said::Name(workspace.name.clone()),
            words(" (the one there now stays saved), with this in its prompt"),
        ],
        SendTarget::Outside { session, branch, .. } => vec![
            Said::Words(format!(
                "goes into its session from {}, brought into a new workspace on ",
                crate::text::short_path(&session.cwd)
            )),
            Said::Branch(branch.clone()),
            words("; you press Enter there"),
        ],
        SendTarget::New { name, branch, mode, .. } => vec![
            words("a new workspace, "),
            Said::Name(name.clone()),
            words(", on "),
            Said::Branch(branch.clone()),
            Said::Words(format!(
                "{}, with a new Claude and this in its prompt",
                if *mode == crate::work::claudes::BranchMode::New { " (a new branch)" } else { "" }
            )),
        ],
        SendTarget::None => vec![words("nowhere to start Claude on this: the message goes on your clipboard")],
    }
}

/// The same sentence as plain words.
pub fn target_words(target: &SendTarget) -> String {
    (target_said(target).into_iter())
        .map(|said| match said {
            Said::Words(words) | Said::Name(words) | Said::Branch(words) => words,
        })
        .collect()
}

/// The card's one accent button: what it says, what it says while it works, and whether its sign is a plus
/// (something is made) or Claude's spark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SendButton {
    pub label: &'static str,
    pub busy: &'static str,
    pub plus: bool,
}

pub fn send_button(target: &SendTarget) -> SendButton {
    match target {
        SendTarget::New { .. } => SendButton { label: "Start workspace", busy: "Starting…", plus: true },
        SendTarget::Switch { session, .. } => {
            SendButton { label: "Start Claude", busy: "Starting…", plus: session.is_none() }
        }
        SendTarget::None => SendButton { label: "Copy the message", busy: "Copy the message", plus: false },
        SendTarget::Workspace { .. } | SendTarget::Outside { .. } => {
            SendButton { label: "Open in Claude", busy: "Opening…", plus: false }
        }
    }
}

/// What goes with your words, as chips: the ticket, its open PRs, and how many of the things that need you. The
/// last is brighter when you left some out.
pub fn with_it(group: &Group, work: &Snapshot, draft: &Draft) -> Vec<(String, bool)> {
    let mut chips: Vec<(String, bool)> = Vec::new();
    if let (Some(key), Some(_)) = (&group.key, group.ticket(work)) {
        chips.push((key.clone(), false));
    }
    if group.own.is_some() {
        chips.push(("your task".to_string(), false));
    }
    let open: Vec<_> = group.prs.iter().map(|view| view.of(work)).filter(|pr| pr.state == "OPEN").collect();
    let all: Vec<_> = group.prs.iter().map(|view| view.of(work)).collect();
    chips.extend(if open.is_empty() { all } else { open }.into_iter().map(|pr| (format!("PR #{}", pr.number), false)));
    let (every, kept) = (card_items(group, work).len(), picked(group, work, draft).len());
    if every > 0 {
        chips.push(match (kept == every, every) {
            (true, 1) => ("what needs you".to_string(), false),
            (true, _) => (format!("the {every} things that need you"), false),
            (false, _) => (format!("{kept} of the {every} things that need you"), true),
        });
    }
    chips
}

/// What the field for your words asks: "What should Claude do about SH-21309?".
pub fn note_asks(group: &Group, work: &Snapshot) -> String {
    let about = match (&group.key, group.prs.first()) {
        (Some(key), _) => key.clone(),
        (None, Some(view)) => format!("PR #{}", view.of(work).number),
        (None, None) if group.own.is_some() => "this task".to_string(),
        (None, None) => "this".to_string(),
    };
    format!("What should Claude do about {about}?")
}

/// What a card's timeline says once its message went: where it went, and the first line of your words.
pub fn sent_note(target: &SendTarget, workspace: &str, draft: &Draft) -> String {
    let place = match target {
        SendTarget::New { .. } | SendTarget::Outside { .. } => {
            format!("Started {workspace} for it; the message is in its Claude's prompt")
        }
        SendTarget::Switch { session, .. } => format!(
            "{} in {workspace} for it; the message is in its prompt",
            if session.is_some() { "Resumed a conversation" } else { "Started a new conversation" }
        ),
        _ => format!("Put in Claude's prompt in {workspace}"),
    };
    match draft.note.trim().split('\n').next().filter(|said| !said.is_empty()) {
        Some(said) => format!("{place}: {}", said.chars().take(120).collect::<String>()),
        None => place,
    }
}

/// The toast once it went: where it is, and that Enter is yours.
pub fn sent_toast(target: &SendTarget, workspace: &str) -> (String, &'static str) {
    let title = match target {
        SendTarget::New { .. } => format!("Started {workspace}"),
        _ => format!("In Claude's prompt in {workspace}"),
    };
    (title, "Press Enter there to send it.")
}

/// What the card's timeline is told about: the ticket's key, else its first PR as GitHub names it; a task of your
/// own under its card's name (`own:3`).
pub fn note_subject(group: &Group, work: &Snapshot) -> String {
    match (&group.key, group.prs.first()) {
        (Some(key), _) => key.clone(),
        (None, Some(view)) => view.of(work).reference(),
        (None, None) if group.own.is_some() => group.id.clone(),
        (None, None) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::work::claudes::{BranchMode, SendTarget, task_target};
    use crate::work::groups::build_groups;
    use crate::work::groups::tests::{NOW, statuses};
    use crate::work::sample;
    use enkidu_core::db::Workspace;
    use std::collections::HashMap;

    /// The workspace of these tests: on a branch that is no card's.
    fn ws(id: &str, name: &str) -> Workspace {
        Workspace {
            id: id.into(),
            project_id: "p1".into(),
            name: name.into(),
            path: format!("/w/{name}"),
            branch: "b".into(),
            base: Some("development".into()),
            session_id: format!("s-{id}"),
            transcript_path: None,
            claude_config_dir: None,
            created_at: 1,
            adopted: false,
        }
    }

    fn groups() -> (Snapshot, Vec<Group>) {
        let work = sample();
        let groups = build_groups(&work, &[ws("w2", "sh-21309-export")], &statuses(&[("w2", "working")]), NOW);
        (work, groups)
    }

    fn card<'a>(groups: &'a [Group], key: &str) -> &'a Group {
        groups.iter().find(|g| g.key.as_deref() == Some(key)).unwrap_or_else(|| panic!("no card for {key}"))
    }

    fn say(note: Option<&str>, include: Option<bool>) -> ItemDraft {
        ItemDraft { include, note: note.map(String::from) }
    }

    /// Always says what the card is: the ticket and its open PRs, with their links.
    #[test]
    fn always_says_what_the_card_is_the_ticket_and_its_open_prs_with_their_links() {
        let (work, groups) = groups();
        let text = task_text(card(&groups, "SH-21309"), &work, &Draft::default());
        let lines: Vec<&str> = text.split('\n').collect();
        assert_eq!(lines[0], "SH-21309: Export orders to the accounting CSV");
        assert_eq!(lines[1], "Ticket: https://demo.atlassian.net/browse/SH-21309");
        assert_eq!(
            lines[2],
            "PR #2414 (feature/SH-21309-export into development): https://github.com/demo-org/shop/pull/2414"
        );
        assert!(text.trim_end().ends_with("don't post anything to GitHub or Jira."));
        // a card with two open PRs names both; a merged one only when it is all there is
        let stock = task_text(card(&groups, "SH-21533"), &work, &Draft::default());
        assert!(stock.contains("PR #2395 (fix/stock-double-count into development)"), "{stock}");
        assert!(stock.contains("PR #2413 (hotfix/SH-21533-stock-double-count into master)"), "{stock}");
        let done = task_text(card(&groups, "SH-21030"), &work, &Draft::default());
        assert!(done.contains("PR #2312 (feature/SH-21030-warranty-labels into development)"), "{done}");
    }

    /// Puts your words first, then each item you kept with your instruction, or the usual one.
    #[test]
    fn puts_your_words_first_then_each_item_you_kept_with_your_instruction_or_the_usual_one() {
        let (work, groups) = groups();
        let draft = Draft {
            note: "Finish the export first; QA tests it tomorrow.".into(),
            items: [
                ("thread:t1".to_string(), say(Some("Read the separator from the outlet's locale."), None)),
                ("thread:t2".to_string(), say(Some("this one is ignored"), Some(false))),
            ]
            .into(),
            ..Draft::default()
        };
        let export = card(&groups, "SH-21309");
        let text = task_text(export, &work, &draft);
        assert!(text.contains("\n\nFinish the export first; QA tests it tomorrow.\n\nAddress these:\n"), "{text}");
        // the order of the card: the question, the remark, the failing checks (thread:t2 is left out)
        let items = [
            "1. jonas-qa asked on PR #2414:",
            "   > Does the export include orders that were cancelled after delivery?",
            "   Draft an answer.",
            "2. mira-k left a remark on orders/export.py:48 (PR #2414):",
            "   > The CSV writer should use the outlet's decimal separator, accounting imports these with a comma in France.",
            "   What to do: Read the separator from the outlet's locale.",
            "3. Checks are failing on the latest commit of PR #2414: tests (3.12): failure.",
            "   Find out why and fix it.",
            "   Link: https://ci.example.com/run/88123",
        ]
        .join("\n");
        assert!(text.contains(&items), "{text}");
        assert!(!text.contains("this one is ignored") && !text.contains("Rows with a zero total"), "{text}");
        let kept: Vec<&str> = picked(export, &work, &draft).iter().map(|item| item.id.as_str()).collect();
        assert_eq!(kept, ["question:c9", "thread:t1", "checks:demo-org/shop#2414:2414abc0def1"]);
        // the message as you edited it is what goes
        let edited = Draft { text: Some("Just this.".into()), ..draft.clone() };
        assert_eq!(message_of(export, &work, &edited), "Just this.");
        assert_eq!(message_of(export, &work, &draft), text);
    }

    /// Says the Jira side in Jira's words, and nothing extra when nothing waits.
    #[test]
    fn says_the_jira_side_in_jiras_words_and_nothing_extra_when_nothing_waits() {
        let (work, groups) = groups();
        let qa = task_text(card(&groups, "SH-21112"), &work, &Draft::default());
        assert!(qa.contains("Jonas Weber replied on SH-21112 in Jira:"), "{qa}");
        assert!(qa.contains("Fix what they found, or draft an answer."), "{qa}");
        let quiet =
            task_text(card(&groups, "SH-21522"), &work, &Draft { note: "Start on it.".into(), ..Draft::default() });
        assert!(!quiet.contains("Address") && quiet.contains("\n\nStart on it.\n"), "{quiet}");
        assert!(card_items(card(&groups, "SH-21522"), &work).is_empty());
        // a workflow's task carries what waits, each with the usual thing to do
        let bundles = card(&groups, "SH-21112");
        let lines = item_lines(bundles, &work, &card_items(bundles, &work));
        assert!(
            lines[0].starts_with("1. ")
                && lines.iter().any(|line| line == "   Fix what they found, or draft an answer.")
        );
    }

    /// Knows a draft you wrote in from an empty one.
    #[test]
    fn knows_a_draft_you_wrote_in_from_an_empty_one() {
        assert!(!touched(None));
        assert!(!touched(Some(&Draft::default())));
        let blank =
            Draft { note: "  ".into(), items: [("a".to_string(), say(Some(""), None))].into(), ..Draft::default() };
        assert!(!touched(Some(&blank)));
        assert!(touched(Some(&Draft { note: "Go".into(), ..Draft::default() })));
        let left_out = Draft { items: [("a".to_string(), say(None, Some(false)))].into(), ..Draft::default() };
        assert!(touched(Some(&left_out)));
        assert!(touched(Some(&Draft { text: Some(String::new()), ..Draft::default() })), "an edited message counts");
        // a draft is kept as text, and read back as it was
        let kept = serde_json::to_string(&left_out).unwrap();
        assert_eq!(kept, r#"{"note":"","items":{"a":{"include":false}}}"#);
        assert_eq!(serde_json::from_str::<Draft>(&kept).unwrap(), left_out);
    }

    /// Goes to the Claude in the card's workspace, else a session from outside on its open PR, else a new workspace for
    /// it.
    #[test]
    fn goes_to_the_claude_in_the_cards_workspace_else_a_session_from_outside_else_a_new_workspace() {
        let (work, groups) = groups();
        let list = [ws("w2", "sh-21309-export")];
        let none = HashMap::new();
        let target = task_target(card(&groups, "SH-21309"), &work, &list, &none, None);
        let SendTarget::Workspace { workspace, .. } = target else { panic!("{target:?}") };
        assert_eq!(workspace.name, "sh-21309-export");
        // SH-21376's PR was made by a session in Warp: it is brought into a workspace on the PR's branch
        let outside = task_target(card(&groups, "SH-21376"), &work, &list, &none, None);
        let SendTarget::Outside { branch, mode, .. } = outside else { panic!("{outside:?}") };
        assert_eq!((branch.as_str(), mode), ("feature/SH-21376-bundle-units", BranchMode::Existing));
        // no Claude has worked on it: a workspace named for the ticket, on a new branch
        let fresh = task_target(card(&groups, "SH-21522"), &work, &list, &none, None);
        let SendTarget::New { name, branch, mode, .. } = fresh else { panic!("{fresh:?}") };
        let named =
            ("sh-21522-planned-orders-keep-the-old", "feature/SH-21522-planned-orders-keep-the-old", BranchMode::New);
        assert_eq!((name.as_str(), branch.as_str(), mode), named);
        // a PR without a ticket that no Claude made: a workspace on its own branch
        let pr = groups.iter().find(|g| g.id == "pr:demo-org/shop#2415").unwrap();
        let lone = task_target(pr, &work, &list, &none, None);
        let SendTarget::New { branch, mode, .. } = lone else { panic!("{lone:?}") };
        assert_eq!((branch, mode), (pr.prs[0].of(&work).head.clone(), BranchMode::Existing));
    }

    /// The words of Tell Claude: where the message goes, what the button says, what goes with your words.
    #[test]
    fn says_where_the_message_goes_what_the_button_does_and_what_goes_with_your_words() {
        use crate::work::claudes::ClaudeChoice;
        use crate::work::groups::tests::ws as on;
        let work = sample();
        let list = [
            on("w1", "stock-fix", "fix/stock-double-count"),
            on("w2", "sh-21309-export", "feature/SH-21309-export"),
            on("w3", "scratch", "scratch"),
        ];
        let none = HashMap::new();
        let groups = build_groups(&work, &list, &none, NOW);
        let export = card(&groups, "SH-21309");
        let to = |group: &Group, pick: Option<&str>| {
            let pick = pick.map(ClaudeChoice::read);
            task_target(group, &work, &list, &none, pick.as_ref())
        };
        let said = |group: &Group, pick: Option<&str>| {
            let target = to(group, pick);
            (target_words(&target), target_kind(&target), send_button(&target).label)
        };
        // the Claude in the card's workspace: typed into its prompt, never sent
        assert_eq!(
            said(export, None),
            (
                "goes into the prompt of the Claude in sh-21309-export; you press Enter there".to_string(),
                "workspace",
                "Open in Claude"
            )
        );
        // a new one: a new conversation in the card's workspace
        assert_eq!(
            said(export, Some("new")),
            (
                "a new conversation in sh-21309-export (the one there now stays saved), with this in its prompt"
                    .to_string(),
                "switch",
                "Start Claude"
            )
        );
        assert!(send_button(&to(export, Some("new"))).plus);
        // any workspace's Claude you picked
        assert_eq!(
            said(export, Some("s-w3")).0,
            "goes into the prompt of the Claude in scratch; you press Enter there"
        );
        // a new workspace though the card has one: its own name, on the card's branch
        let fresh = said(export, Some("fresh"));
        assert!(
            fresh.0.starts_with("a new workspace, sh-21309-export-2, on feature/SH-21309-export, with"),
            "{}",
            fresh.0
        );
        assert_eq!((fresh.1, fresh.2), ("new", "Start workspace"));
        // a ticket nobody worked on: a workspace named for it, on a new branch
        assert_eq!(
            said(card(&groups, "SH-21522"), None).0,
            "a new workspace, sh-21522-planned-orders-keep-the-old, on feature/SH-21522-planned-orders-keep-the-old (a \
             new branch), with a new Claude and this in its prompt"
        );
        // a session from outside Enkidu on the card's open PR
        let outside = said(card(&groups, "SH-21376"), None);
        assert!(outside.0.starts_with("goes into its session from ~/"), "{}", outside.0);
        assert!(
            outside
                .0
                .ends_with(", brought into a new workspace on feature/SH-21376-bundle-units; you press Enter there"),
            "{}",
            outside.0
        );
        assert_eq!((outside.1, outside.2), ("outside", "Open in Claude"));
        // the names in the sentence are told apart from its words
        let parts = target_said(&to(export, None));
        assert_eq!(parts[1], Said::Name("sh-21309-export".into()));
        assert!(
            matches!(&target_said(&to(card(&groups, "SH-21376"), None))[1], Said::Branch(branch) if branch == "feature/SH-21376-bundle-units")
        );
        // nowhere to start it: the clipboard
        let nowhere = SendTarget::None;
        assert_eq!(target_words(&nowhere), "nowhere to start Claude on this: the message goes on your clipboard");
        assert_eq!((target_kind(&nowhere), send_button(&nowhere).label), ("none", "Copy the message"));
        // what goes with your words
        let chips = |group: &Group, draft: &Draft| -> Vec<String> {
            with_it(group, &work, draft)
                .into_iter()
                .map(|(chip, bright)| format!("{chip}{}", if bright { " !" } else { "" }))
                .collect()
        };
        assert_eq!(chips(export, &Draft::default()), ["SH-21309", "PR #2414", "the 4 things that need you"]);
        let left_out = Draft { items: [("thread:t2".to_string(), say(None, Some(false)))].into(), ..Draft::default() };
        assert_eq!(chips(export, &left_out), ["SH-21309", "PR #2414", "3 of the 4 things that need you !"]);
        assert_eq!(
            chips(card(&groups, "SH-21616"), &Draft::default()).last().map(String::as_str),
            Some("what needs you")
        );
        assert_eq!(chips(card(&groups, "SH-21522"), &Draft::default()), ["SH-21522"]);
        // what the field asks, and what the card's timeline and the toast say once it went
        assert_eq!(note_asks(export, &work), "What should Claude do about SH-21309?");
        let lone = groups.iter().find(|g| g.id == "pr:demo-org/shop#2415").unwrap();
        assert_eq!(note_asks(lone, &work), "What should Claude do about PR #2415?");
        assert_eq!(
            (note_subject(export, &work), note_subject(lone, &work)),
            ("SH-21309".to_string(), "demo-org/shop#2415".to_string())
        );
        let words = Draft { note: "Finish the export first.\nThen the header.".into(), ..Draft::default() };
        assert_eq!(
            sent_note(&to(export, None), "sh-21309-export", &words),
            "Put in Claude's prompt in sh-21309-export: Finish the export first."
        );
        assert_eq!(
            sent_note(&to(export, Some("new")), "sh-21309-export", &Draft::default()),
            "Started a new conversation in sh-21309-export for it; the message is in its prompt"
        );
        assert_eq!(
            sent_note(&to(export, Some("fresh")), "sh-21309-export-2", &Draft::default()),
            "Started sh-21309-export-2 for it; the message is in its Claude's prompt"
        );
        assert_eq!(
            sent_toast(&to(export, None), "sh-21309-export"),
            ("In Claude's prompt in sh-21309-export".to_string(), "Press Enter there to send it.")
        );
        assert_eq!(sent_toast(&to(export, Some("fresh")), "sh-21309-export-2").0, "Started sh-21309-export-2");
    }
}
