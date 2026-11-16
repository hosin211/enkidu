//! Which Claude a card goes to, and where a message for it goes. Enkidu types a message into a Claude's prompt and
//! never presses Enter: you read it and send it yourself.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use enkidu_core::db::Workspace;
use enkidu_core::work::model::{Item, Pr, Session, Snapshot};
use regex::Regex;
use unicode_normalization::UnicodeNormalization as _;

use super::groups::{Group, SessionView, link_strength};

/// A workspace on a new branch, or on one that exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BranchMode {
    New,
    Existing,
}

/// Where a message for Claude goes.
#[derive(Clone, Debug, PartialEq)]
pub enum SendTarget {
    /// An Enkidu workspace whose Claude is in that conversation: it gets it typed in (or starts, resuming it).
    Workspace { workspace: Workspace, session: Option<Session> },
    /// A workspace whose Claude moves to another conversation first: `session` (one it or a Claude outside
    /// Enkidu had), or a new one (none). Its Claude, if it runs and isn't busy, closes; the conversation it was
    /// in stays saved.
    Switch { workspace: Workspace, session: Option<Session> },
    /// A session that ran outside Enkidu (Warp, the main folder): it comes into a new workspace on the card's
    /// branch.
    Outside { session: Session, project_id: String, name: String, branch: String, mode: BranchMode },
    /// A new Claude in a new workspace for the card: on its open PR's branch (existing), else on a new branch named
    /// for the ticket, from the project's base (new); a branch named so that is already there is checked out.
    New { project_id: String, name: String, branch: String, mode: BranchMode },
    /// Nowhere to start it (no project, no branch): the message goes on the clipboard.
    None,
}

/// Which Claude a card's message goes to.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ClaudeChoice {
    /// A new one: a new conversation in the card's workspace, else a new workspace.
    New,
    /// Always a new workspace.
    Fresh,
    /// A session, by its id: one linked to the card, or any workspace's that you picked.
    Session(String),
}

impl ClaudeChoice {
    /// As the card's draft keeps it: `new`, `fresh`, or the session's id.
    pub fn read(kept: &str) -> ClaudeChoice {
        match kept {
            "new" => ClaudeChoice::New,
            "fresh" => ClaudeChoice::Fresh,
            id => ClaudeChoice::Session(id.to_string()),
        }
    }

    pub fn write(&self) -> &str {
        match self {
            ClaudeChoice::New => "new",
            ClaudeChoice::Fresh => "fresh",
            ClaudeChoice::Session(id) => id,
        }
    }

    fn session(&self) -> Option<&str> {
        match self {
            ClaudeChoice::Session(id) => Some(id),
            _ => None,
        }
    }
}

/// The reason a session is a card's Claude when nothing links them: you picked its workspace's Claude for the card.
pub const PICKED: &str = "picked by you";
/// The reason that is the card's own branch, not a link: there is nothing to unlink.
pub const ON_BRANCH: &str = "its workspace is on this branch";

/// Is this Claude on the card because of a link, which you can undo when it is wrong?
pub fn unlinkable(how: &str) -> bool {
    how != PICKED && how != ON_BRANCH
}

/// A Claude a card can go to, and why it is one.
#[derive(Clone, Debug, PartialEq)]
pub struct ClaudeOption {
    pub session: Session,
    /// The workspace whose Claude is in this conversation now, else the one it ran in.
    pub workspace: Option<Workspace>,
    pub status: Option<String>,
    /// How it is linked, in words.
    pub how: String,
    /// Its workspace is on the card's branch.
    pub on_branch: bool,
    /// Its workspace is on a branch named for another ticket: it is busy with that one.
    pub elsewhere: bool,
}

/// The Claudes a card can go to, the likeliest first, and the one picked.
#[derive(Clone, Debug, PartialEq)]
pub struct CardClaudes {
    pub options: Vec<ClaudeOption>,
    pub pick: ClaudeChoice,
}

impl CardClaudes {
    /// The Claude picked, when it is one of the sessions (not a new one).
    pub fn picked(&self) -> Option<&ClaudeOption> {
        let id = self.pick.session()?;
        self.options.iter().find(|option| option.session.id == id)
    }
}

static KEY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[A-Za-z][A-Za-z0-9]+-\d+").expect("a ticket key"));

/// Ticket keys a branch is named for ("feature/SH-21000-x" is SH-21000's).
fn keys_in(branch: &str) -> Vec<String> {
    KEY.find_iter(branch).map(|key| key.as_str().to_uppercase()).collect()
}

/// What a card's own branch is named for: its ticket's key, or `task-3` for a task of your own (which has no key).
fn branch_key(group: &Group, work: &Snapshot) -> Option<String> {
    group.key.clone().or_else(|| group.own(work).map(|task| format!("task-{}", task.id)))
}

/// The title a ticket's branch is named after: the ticket's own, else the card's.
fn named_after<'a>(group: &'a Group, work: &'a Snapshot) -> &'a str {
    group.ticket(work).map_or(group.title.as_str(), |ticket| ticket.summary.as_str())
}

/// The branches a card's work is on, the likeliest first: the given PR's, its open PRs', its other PRs', and the
/// one named for its ticket.
fn card_branches(group: &Group, work: &Snapshot, pr: Option<&Pr>) -> Vec<String> {
    let open = group.prs.iter().map(|view| view.of(work)).filter(|pr| pr.state == "OPEN");
    let all = group.prs.iter().map(|view| view.of(work));
    let mut branches: Vec<String> = Vec::new();
    let mut add = |branch: String| {
        if !branches.contains(&branch) {
            branches.push(branch);
        }
    };
    for pr in pr.into_iter().chain(open).chain(all) {
        add(pr.head.clone());
    }
    if let Some(key) = branch_key(group, work) {
        add(ticket_workspace(&key, named_after(group, work)).1);
    }
    branches
}

/// The branch a new Claude for the card works on: the PR's, else its open PR's (both exist), else the one named for
/// its ticket (new).
fn card_branch(group: &Group, work: &Snapshot, pr: Option<&Pr>) -> Option<(String, BranchMode)> {
    let open = (pr.filter(|pr| pr.state == "OPEN"))
        .or_else(|| group.prs.iter().map(|view| view.of(work)).find(|pr| pr.state == "OPEN"));
    if let Some(open) = open {
        return Some((open.head.clone(), BranchMode::Existing));
    }
    let key = branch_key(group, work)?;
    Some((ticket_workspace(&key, named_after(group, work)).1, BranchMode::New))
}

/// The workspace already on the card's branch, if there is one.
fn home_workspace<'a>(
    group: &Group,
    work: &Snapshot,
    pr: Option<&Pr>,
    workspaces: &'a [Workspace],
) -> Option<&'a Workspace> {
    let (branch, _) = card_branch(group, work, pr)?;
    let project = group.project_id.as_ref()?;
    workspaces.iter().find(|w| &w.project_id == project && w.branch == branch)
}

/// The conversation a workspace is in, as a session (for a workspace no hook has linked yet).
fn session_of(workspace: &Workspace) -> Session {
    Session {
        id: workspace.session_id.clone(),
        config_dir: String::new(),
        cwd: workspace.path.clone(),
        title: None,
        branches: vec![workspace.branch.clone()],
        workspace_id: Some(workspace.id.clone()),
        first_seen: workspace.created_at,
        last_seen: workspace.created_at,
        source: "workspace".into(),
    }
}

/// The Claudes a card can go to: the sessions linked to it, and the one in the workspace on its branch (also before
/// any hook linked it), the likeliest first. `pick` is yours when you picked one, else the likeliest when its link
/// says enough (its workspace is on the card's branch; or you or Claude linked it; or it made, pushed or committed to
/// the PR and isn't busy with another ticket), else a new Claude: named in a prompt is not enough. `pr` narrows it to
/// one of the card's PRs (its place in the snapshot's PRs).
pub fn card_claudes(
    group: &Group,
    work: &Snapshot,
    workspaces: &[Workspace],
    statuses: &HashMap<String, String>,
    chosen: Option<&ClaudeChoice>,
    pr: Option<usize>,
) -> CardClaudes {
    let of_pr = pr.and_then(|ix| group.prs.iter().find(|view| view.pr == ix));
    let pr = pr.map(|ix| &work.prs[ix]);
    let branches = card_branches(group, work, pr);
    let status_of = |workspace: &Workspace| statuses.get(&workspace.id).cloned();
    // every Claude it could be, with the session itself: the PR's, the card's, then the two that no link names
    struct View {
        session: Session,
        workspace: Option<Workspace>,
        status: Option<String>,
        how: String,
    }
    let linked = |view: &SessionView| View {
        session: view.of(work).clone(),
        workspace: view.workspace.clone(),
        status: view.status.clone(),
        how: view.how.clone(),
    };
    let mut views: Vec<View> =
        of_pr.into_iter().flat_map(|view| &view.sessions).chain(&group.sessions).map(linked).collect();
    let unlinked = |workspace: &Workspace, how: &str| View {
        session: session_of(workspace),
        workspace: Some(workspace.clone()),
        status: Some(status_of(workspace).unwrap_or_else(|| "idle".into())),
        how: how.into(),
    };
    // the conversation the card's workspace is in, linked or not yet
    let home = home_workspace(group, work, pr, workspaces);
    if let Some(home) = home.filter(|home| !views.iter().any(|view| view.session.id == home.session_id)) {
        views.push(unlinked(home, ON_BRANCH));
    }
    // a workspace's Claude you picked for the card, though nothing links them (the links were wrong, or missing)
    let other = (chosen.and_then(ClaudeChoice::session))
        .filter(|id| !views.iter().any(|view| view.session.id == *id))
        .and_then(|id| workspaces.iter().find(|w| w.session_id == id));
    if let Some(other) = other {
        views.push(unlinked(other, PICKED));
    }
    let mine = group.key.as_ref().map(|key| key.to_uppercase());
    let mut seen: HashSet<String> = HashSet::new();
    let mut options: Vec<ClaudeOption> = Vec::new();
    for view in views {
        if !seen.insert(view.session.id.clone()) {
            continue;
        }
        // the workspace whose Claude is in this conversation now, else the one it ran in
        let workspace = workspaces.iter().find(|w| w.session_id == view.session.id).cloned().or(view.workspace);
        let on_branch = workspace.as_ref().is_some_and(|w| branches.contains(&w.branch));
        let elsewhere = !on_branch
            && workspace.as_ref().is_some_and(|w| keys_in(&w.branch).iter().any(|key| Some(key) != mine.as_ref()));
        let status =
            (workspace.as_ref()).map(|w| status_of(w).or(view.status.clone()).unwrap_or_else(|| "idle".into()));
        options.push(ClaudeOption { session: view.session, workspace, status, how: view.how, on_branch, elsewhere });
    }
    let rank = |option: &ClaudeOption| -> i32 {
        let branch = match (option.on_branch, &option.workspace) {
            (true, Some(workspace)) if branches.first() == Some(&workspace.branch) => 20,
            (true, _) => 10,
            (false, _) => 0,
        };
        branch + i32::from(link_strength(&option.how)) - if option.elsewhere { 10 } else { 0 }
    };
    options.sort_by(|a, b| rank(b).cmp(&rank(a)).then_with(|| b.session.last_seen.cmp(&a.session.last_seen)));
    // a session from outside Enkidu comes in on an open PR's branch; with none, a new Claude unless you pick it
    let open = pr.is_some_and(|pr| pr.state == "OPEN") || group.prs.iter().any(|view| view.of(work).state == "OPEN");
    let sure = options.first().is_some_and(|best| {
        // linked on purpose, by you or by Claude (`enkidu link`), counts even for a Claude whose workspace is on
        // another ticket's branch; anything it only did along the way doesn't
        let explicit = best.how == "linked by you" || best.how == "linked by Claude";
        let strong = explicit || (link_strength(&best.how) >= 3 && !best.elsewhere);
        best.on_branch || (strong && (best.workspace.is_some() || open))
    });
    let valid = chosen.filter(|chosen| match chosen {
        ClaudeChoice::New | ClaudeChoice::Fresh => true,
        ClaudeChoice::Session(id) => options.iter().any(|option| &option.session.id == id),
    });
    let pick = match (valid, options.first()) {
        (Some(chosen), _) => chosen.clone(),
        (None, Some(best)) if sure => ClaudeChoice::Session(best.session.id.clone()),
        _ => ClaudeChoice::New,
    };
    CardClaudes { options, pick }
}

/// One Claude of the card's Claude box: a session linked to the card, the conversation of the workspace on its
/// branch, or a workspace's Claude you picked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaudeRow {
    /// The session's id: what your pick keeps.
    pub session: String,
    /// Its workspace, else the session's own title, else the start of its id.
    pub name: String,
    /// Its workspace's lamp.
    pub status: Option<String>,
    /// Its strongest link in words, when it last ran, and where it ran when that is in no workspace: "Created it,
    /// 2 h ago, in ~/code/shop".
    pub line: String,
    pub picked: bool,
    /// The workspace to go to with "Open".
    pub workspace: Option<String>,
    /// The link can be undone (a workspace on the card's branch, or your own pick, can't).
    pub unlinkable: bool,
}

/// "New Claude", or "New workspace": picked or not, and what it would start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewRow {
    pub picked: bool,
    pub line: String,
}

/// The card's Claude box: which Claude the message goes to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaudeBox {
    /// Each Claude linked to the card, the likeliest first.
    pub rows: Vec<ClaudeRow>,
    /// A new conversation in the card's workspace, else a new workspace.
    pub new_claude: NewRow,
    /// A new workspace though the card has one: only offered when "New Claude" would be a new conversation there.
    pub new_workspace: Option<NewRow>,
    /// Every other workspace's Claude of the card's project, for links that are wrong or missing: (session,
    /// workspace, branch).
    pub others: Vec<(String, String, String)>,
}

fn new_line(target: &SendTarget) -> String {
    match target {
        SendTarget::Switch { workspace, .. } => format!("A new conversation in {}", workspace.name),
        SendTarget::New { name, .. } => format!("A new workspace, {name}"),
        _ => "Nowhere to start one: no branch for it".to_string(),
    }
}

/// The Claude box of a card. None when no Claude is linked to it and its project has no workspace to pick: then
/// there is nothing to choose, and the card's button starts a workspace.
pub fn claude_box(
    group: &Group,
    work: &Snapshot,
    workspaces: &[Workspace],
    statuses: &HashMap<String, String>,
    chosen: Option<&ClaudeChoice>,
    now: i64,
) -> Option<ClaudeBox> {
    let claudes = card_claudes(group, work, workspaces, statuses, chosen, None);
    let others: Vec<(String, String, String)> = match &group.project_id {
        Some(project) => (workspaces.iter())
            .filter(|w| &w.project_id == project)
            .filter(|w| !claudes.options.iter().any(|option| option.session.id == w.session_id))
            .map(|w| (w.session_id.clone(), w.name.clone(), w.branch.clone()))
            .collect(),
        None => Vec::new(),
    };
    if claudes.options.is_empty() && others.is_empty() {
        return None;
    }
    let rows = (claudes.options.iter())
        .map(|option| {
            let session = &option.session;
            let name = (option.workspace.as_ref().map(|w| w.name.clone()))
                .or_else(|| session.title.clone())
                .unwrap_or_else(|| session.id.chars().take(8).collect());
            let mut how = option.how.chars();
            let mut line: String = how.next().map(|first| first.to_uppercase().collect()).unwrap_or_default();
            line.push_str(how.as_str());
            // a workspace's conversation no hook has reported yet has no time of its own
            if session.source != "workspace" {
                line.push_str(&format!(", {}", super::sources::ago(Some(session.last_seen), now)));
            }
            if option.workspace.is_none() {
                line.push_str(&format!(", in {}", crate::text::short_path(&session.cwd)));
            }
            ClaudeRow {
                session: session.id.clone(),
                name,
                status: option.status.clone(),
                line,
                picked: claudes.pick == ClaudeChoice::Session(session.id.clone()),
                workspace: option.workspace.as_ref().map(|w| w.id.clone()),
                unlinkable: unlinkable(&option.how),
            }
        })
        .collect();
    let fresh = task_target(group, work, workspaces, statuses, Some(&ClaudeChoice::New));
    let new_workspace = matches!(fresh, SendTarget::Switch { .. }).then(|| NewRow {
        picked: claudes.pick == ClaudeChoice::Fresh,
        line: new_line(&task_target(group, work, workspaces, statuses, Some(&ClaudeChoice::Fresh))),
    });
    let new_claude = NewRow { picked: claudes.pick == ClaudeChoice::New, line: new_line(&fresh) };
    Some(ClaudeBox { rows, new_claude, new_workspace, others })
}

/// The card's ticket and PRs, as the index names them, for "linked by you": its open PRs, a merged one only when it
/// is all there is.
pub fn card_subjects(group: &Group, work: &Snapshot) -> Vec<String> {
    let open: Vec<&Pr> = group.prs.iter().map(|view| view.of(work)).filter(|pr| pr.state == "OPEN").collect();
    let all: Vec<&Pr> = group.prs.iter().map(|view| view.of(work)).collect();
    (group.key.iter().map(|key| format!("ticket:{key}")))
        // a task of your own is linked under its card's name
        .chain(group.own.map(|_| group.id.clone()))
        .chain(if open.is_empty() { all } else { open }.into_iter().map(|pr| format!("pr:{}", pr.reference())))
        .collect()
}

/// Where an item goes: as its card's message, to the Claude picked for the card (yours, else the likeliest for the
/// item's PR).
pub fn send_target(
    item: &Item,
    group: &Group,
    work: &Snapshot,
    workspaces: &[Workspace],
    statuses: &HashMap<String, String>,
    chosen: Option<&ClaudeChoice>,
) -> SendTarget {
    let pr = (item.pr.as_deref())
        .and_then(|reference| group.prs.iter().find(|view| view.of(work).reference() == reference))
        .map(|view| view.pr);
    target_for(group, work, pr, workspaces, statuses, chosen)
}

/// Where a message about a whole card goes: to the Claude picked for it.
pub fn task_target(
    group: &Group,
    work: &Snapshot,
    workspaces: &[Workspace],
    statuses: &HashMap<String, String>,
    chosen: Option<&ClaudeChoice>,
) -> SendTarget {
    target_for(group, work, None, workspaces, statuses, chosen)
}

/// A ticket's workspace and branch, from its key and the first words of its title, as your branches are named:
/// "sh-21000-price-lists-keep-their-order" on "feature/SH-21000-price-lists-keep-their-order".
pub fn ticket_workspace(key: &str, title: &str) -> (String, String) {
    // accents fold to their letters (NFKD, without its combining marks)
    let plain: String = title.nfkd().filter(|c| !('\u{300}'..='\u{36f}').contains(c)).collect();
    let plain = plain.to_lowercase();
    let words: Vec<&str> = plain
        .split(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit()))
        .filter(|word| !word.is_empty())
        .collect();
    let mut slug = String::new();
    for word in &words {
        let next = if slug.is_empty() { word.to_string() } else { format!("{slug}-{word}") };
        if next.len() > 32 {
            break;
        }
        slug = next;
    }
    if slug.is_empty()
        && let Some(first) = words.first()
    {
        slug = first.chars().take(32).collect();
    }
    let tail = if slug.is_empty() { String::new() } else { format!("-{slug}") };
    (format!("{}{tail}", key.to_lowercase()), format!("feature/{key}{tail}"))
}

/// A workspace's name from a branch: its last part, as a folder name ("feature/SH-1-x" is "sh-1-x").
pub fn branch_workspace_name(branch: &str) -> String {
    let last = branch.rsplit('/').next().unwrap_or(branch).to_lowercase();
    let mut name = String::new();
    let mut dashed = false;
    for c in last.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-') {
            name.push(c);
            dashed = false;
        } else if !dashed {
            // a run of anything else is one dash
            name.push('-');
            dashed = true;
        }
    }
    let name: String =
        name.trim_start_matches(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit())).chars().take(56).collect();
    if name.is_empty() { "work".into() } else { name }
}

/// A workspace name not taken in the project: `base`, else `base-2`, `base-3`…
pub fn free_name(base: &str, project_id: &str, workspaces: &[Workspace]) -> String {
    let taken: HashSet<&str> =
        workspaces.iter().filter(|w| w.project_id == project_id).map(|w| w.name.as_str()).collect();
    let mut name = base.to_string();
    let mut n = 2;
    while taken.contains(name.as_str()) {
        name = format!("{base}-{n}");
        n += 1;
    }
    name
}

/// Where the picked Claude is. A session that is a workspace's conversation now: that workspace. A session that
/// isn't any workspace's conversation now: its own workspace (or the card's) moves to it, else it comes into a new
/// workspace on the card's branch. A new Claude: a new conversation in the card's workspace, else a new workspace.
fn target_for(
    group: &Group,
    work: &Snapshot,
    pr: Option<usize>,
    workspaces: &[Workspace],
    statuses: &HashMap<String, String>,
    chosen: Option<&ClaudeChoice>,
) -> SendTarget {
    let claudes = card_claudes(group, work, workspaces, statuses, chosen, pr);
    let pr = pr.map(|ix| &work.prs[ix]);
    let home = home_workspace(group, work, pr, workspaces);
    let at = card_branch(group, work, pr);
    // a new workspace for the card: its project, a free name, the branch and whether it exists
    let fresh = || -> Option<(String, String, String, BranchMode)> {
        let project = group.project_id.clone()?;
        let (branch, mode) = at.clone()?;
        let named = match (branch_key(group, work), mode) {
            (Some(key), BranchMode::New) => ticket_workspace(&key, named_after(group, work)).0,
            _ => branch_workspace_name(&branch),
        };
        let name = free_name(&named, &project, workspaces);
        Some((project, name, branch, mode))
    };
    let new = || match fresh() {
        Some((project_id, name, branch, mode)) => SendTarget::New { project_id, name, branch, mode },
        None => SendTarget::None,
    };
    // a new workspace, whatever the card has already: on the card's branch (another folder having it, you pick then
    // between that folder and a new branch from it)
    if claudes.pick == ClaudeChoice::Fresh {
        return new();
    }
    if let Some(option) = claudes.picked() {
        let session = option.session.clone();
        if let Some(now) = workspaces.iter().find(|w| w.session_id == session.id) {
            return SendTarget::Workspace { workspace: now.clone(), session: Some(session) };
        }
        let own = (session.workspace_id.as_ref()).and_then(|id| workspaces.iter().find(|w| &w.id == id)).or(home);
        return match (own, fresh()) {
            (Some(own), _) => SendTarget::Switch { workspace: own.clone(), session: Some(session) },
            (None, Some((project_id, name, branch, mode))) => {
                SendTarget::Outside { session, project_id, name, branch, mode }
            }
            (None, None) => SendTarget::None,
        };
    }
    match home {
        Some(home) => SendTarget::Switch { workspace: home.clone(), session: None },
        None => new(),
    }
}

fn quote(text: &str) -> String {
    text.split('\n').map(|line| format!("> {line}")).collect::<Vec<_>>().join("\n")
}

/// What follows the first colon of an item's title ("Back to you: QA Review to In Progress").
fn after_colon(title: &str) -> &str {
    title.split_once(':').map_or(title, |(_, rest)| rest.trim_start())
}

/// What Send to Claude says: the item, where it is, and what to do. Replies are drafted, never posted.
pub fn send_text(item: &Item, group: &Group, work: &Snapshot) -> String {
    let pr = (item.pr.as_deref())
        .and_then(|reference| group.prs.iter().map(|view| view.of(work)).find(|pr| pr.reference() == reference));
    let on = match (pr, &group.key) {
        (Some(pr), _) => format!("PR #{} ({} into {})", pr.number, pr.head, pr.base),
        (None, Some(key)) => format!("{key} in Jira"),
        (None, None) => "your work".into(),
    };
    let place = item.path.as_deref().filter(|path| !path.is_empty()).map(|path| match item.line.filter(|l| *l != 0) {
        Some(line) => format!("{path}:{line}"),
        None => path.to_string(),
    });
    let who = item.who.as_deref().unwrap_or("Someone");
    let ticket = group.key.as_deref();
    let base = pr.map(|pr| pr.base.as_str());
    let mut lines: Vec<String> = Vec::new();
    match item.kind.as_str() {
        "remark" => {
            let at = place.map(|place| format!(", at {place}")).unwrap_or_default();
            lines.push(format!("{who} left a review remark on {on}{at}:"));
            lines.push(quote(&item.detail));
        }
        "probably_fixed" => {
            let at = place.map(|place| format!(" at {place}")).unwrap_or_default();
            lines.push(format!("{who}'s remark on {on}{at} looks fixed by a later push, but it has no reply yet:"));
            lines.push(quote(&item.detail));
            lines.push("Check that it is fixed, and draft the reply for me.".into());
        }
        "question" => {
            lines.push(format!("{who} asked on {on}:"));
            lines.push(quote(&item.detail));
        }
        "changes_requested" => {
            lines.push(format!("{who} requested changes on {on}. Go through their review and its comments."))
        }
        "pipeline_failed" => {
            lines.push(format!("Checks are failing on the latest commit of {on}:"));
            lines.push(item.detail.clone());
            lines.push("Find out why and fix it.".into());
        }
        "conflict" => lines.push(format!(
            "{on} conflicts with {}. Bring {} into the branch and resolve the conflicts.",
            base.unwrap_or("its base"),
            base.unwrap_or("the base")
        )),
        "qa_replied" => {
            lines.push(format!("{who} replied on {} in Jira ({}):", ticket.unwrap_or("the ticket"), group.title));
            lines.push(quote(&item.detail));
        }
        "mentioned" => {
            lines.push(format!("{who} mentioned you on {} in Jira ({}):", ticket.unwrap_or("the ticket"), group.title));
            lines.push(quote(&item.detail));
        }
        "back_to_you" => lines.push(format!(
            "{} came back to you in Jira: {}. {}.",
            ticket.unwrap_or("The ticket"),
            after_colon(&item.title),
            group.title
        )),
        _ => lines.push(format!(
            "{who} added an attachment to {} in Jira: {}.",
            ticket.unwrap_or("the ticket"),
            after_colon(&item.title)
        )),
    }
    if let Some(url) = item.url.as_deref().filter(|url| !url.is_empty()) {
        lines.push(format!("Link: {url}"));
    }
    lines.push(
        "Fix what is right in the code. For anything that needs an answer, draft the reply here for me; don't post \
         anything to GitHub or Jira."
            .into(),
    );
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::work::groups::tests::{NOW, statuses, workspaces, ws};
    use crate::work::groups::{GroupLamp, PrView, build_groups};
    use crate::work::sample;

    const HOME: &str = enkidu_testkit::WORK_SAMPLE_HOME;

    fn none() -> HashMap<String, String> {
        HashMap::new()
    }

    fn by_key<'a>(groups: &'a [Group], key: &str) -> &'a Group {
        groups.iter().find(|g| g.key.as_deref() == Some(key)).unwrap_or_else(|| panic!("no card for {key}"))
    }

    fn item_of<'a>(work: &'a Snapshot, group: &'a Group, id: &str) -> &'a Item {
        group.all_items(work).find(|item| item.id == id).unwrap_or_else(|| panic!("no item {id}"))
    }

    /// The sample's cards, as the old "Send to Claude" tests built them.
    fn sample_groups() -> (Snapshot, Vec<Group>) {
        let work = sample();
        let groups = build_groups(&work, &workspaces(), &statuses(&[("w1", "working"), ("w2", "idle")]), NOW);
        (work, groups)
    }

    /// Send to Claude: goes to the workspace whose Claude knows the work.
    #[test]
    fn goes_to_the_workspace_whose_claude_knows_the_work() {
        let (work, groups) = sample_groups();
        let all = workspaces();
        let export = by_key(&groups, "SH-21309");
        let target = send_target(item_of(&work, export, "thread:t1"), export, &work, &all, &none(), None);
        let SendTarget::Workspace { workspace, .. } = target else { panic!("{target:?}") };
        assert_eq!(workspace.name, "sh-21309-export");
        // the hotfix PR's remark goes to the session that made it, in the stock-fix workspace
        let stock = by_key(&groups, "SH-21533");
        let target = send_target(item_of(&work, stock, "thread:t5"), stock, &work, &all, &none(), None);
        let SendTarget::Workspace { workspace, .. } = target else { panic!("{target:?}") };
        assert_eq!(workspace.name, "stock-fix");
    }

    /// Brings a session from outside Enkidu into a workspace on its PR's branch, else starts a workspace for the card.
    #[test]
    fn brings_a_session_from_outside_into_a_workspace_on_its_prs_branch_else_starts_one_for_the_card() {
        let (work, groups) = sample_groups();
        let all = workspaces();
        // SH-21376's PR #2375 was made in Warp, in the main folder
        let units = by_key(&groups, "SH-21376");
        let item = Item {
            id: "question:x".into(),
            kind: "question".into(),
            pr: Some("demo-org/shop#2375".into()),
            detail: "Units?".into(),
            ..work.items[0].clone()
        };
        let target = send_target(&item, units, &work, &all, &none(), None);
        let SendTarget::Outside { session, branch, name, .. } = target else { panic!("{target:?}") };
        assert_eq!(
            format!("{} {branch} {name}", session.title.unwrap_or_default()),
            "units on bundles feature/SH-21376-bundle-units sh-21376-bundle-units"
        );
        // the backlog ticket has no session at all: a new workspace for it, on a new branch named for it
        let lonely = by_key(&groups, "SH-20728");
        let asked = Item { pr: None, ticket: Some("SH-20728".into()), ..item.clone() };
        assert_eq!(
            send_target(&asked, lonely, &work, &all, &none(), None),
            SendTarget::New {
                project_id: "p1".into(),
                name: "sh-20728-supplier-price-import".into(),
                branch: "feature/SH-20728-supplier-price-import".into(),
                mode: BranchMode::New,
            }
        );
        // a workspace already on that branch is the one, in the conversation it is in
        let mut with_made = all.clone();
        with_made.push(ws("w9", "sh-20728-supplier-price-import", "feature/SH-20728-supplier-price-import"));
        let there = send_target(&asked, lonely, &work, &with_made, &none(), None);
        let SendTarget::Workspace { workspace, session } = there else { panic!("{there:?}") };
        assert_eq!((workspace.id.as_str(), session.map(|s| s.id).as_deref()), ("w9", Some("s-w9")));
        // a name another branch has gets a number
        let mut with_clash = all.clone();
        with_clash.push(ws("w4", "sh-20728-supplier-price-import", "feature/other"));
        let clash = send_target(&asked, lonely, &work, &with_clash, &none(), None);
        let SendTarget::New { name, .. } = clash else { panic!("{clash:?}") };
        assert_eq!(name, "sh-20728-supplier-price-import-2");
        // nowhere to start it: no project
        let nowhere = Group { project_id: None, ..lonely.clone() };
        assert_eq!(send_target(&asked, &nowhere, &work, &all, &none(), None), SendTarget::None);
    }

    /// Names a ticket's workspace and branch after its key and the first words of its title.
    #[test]
    fn names_a_tickets_workspace_and_branch_after_its_key_and_the_first_words_of_its_title() {
        let named = ticket_workspace("SH-21000", "Price lists: keep their order when a supplier sends a new one");
        assert_eq!(
            named,
            (
                "sh-21000-price-lists-keep-their-order".to_string(),
                "feature/SH-21000-price-lists-keep-their-order".to_string()
            )
        );
        assert_eq!(ticket_workspace("SH-1", "Élan Noël: prices").0, "sh-1-elan-noel-prices");
        let long = ticket_workspace("SH-2", "round_invoice_totals_per_currency_and_region breaks");
        assert_eq!(long.0, "sh-2-round-invoice-totals-per");
        assert_eq!(ticket_workspace("SH-3", "!!!"), ("sh-3".to_string(), "feature/SH-3".to_string()));
        // one word longer than a name can be is cut, and a ligature or a full-width letter is its letters
        assert_eq!(ticket_workspace("SH-4", &"x".repeat(40)).0, format!("sh-4-{}", "x".repeat(32)));
        assert_eq!(ticket_workspace("SH-5", "ﬁx the Ｃache").0, "sh-5-fix-the-cache");
        // a workspace's name from a branch, and a name that is free
        assert_eq!(branch_workspace_name("feature/SH-1-x"), "sh-1-x");
        assert_eq!(branch_workspace_name("hotfix/Fix it, now!"), "fix-it-now-");
        assert_eq!(branch_workspace_name("///"), "work");
        let all = workspaces();
        assert_eq!(free_name("stock-fix", "p1", &all), "stock-fix-2");
        assert_eq!(free_name("stock-fix", "p2", &all), "stock-fix", "another project's name is free here");
    }

    /// Says what waits, where, and that replies are drafted, never posted.
    #[test]
    fn says_what_waits_where_and_that_replies_are_drafted_never_posted() {
        let (work, groups) = sample_groups();
        let export = by_key(&groups, "SH-21309");
        let text = send_text(item_of(&work, export, "thread:t1"), export, &work);
        let remark = "mira-k left a review remark on PR #2414 (feature/SH-21309-export into development), at orders/export.py:48:";
        assert!(text.contains(remark), "{text}");
        assert!(text.contains("> The CSV writer should use the outlet's decimal separator"), "{text}");
        assert!(text.contains("don't post anything to GitHub or Jira"), "{text}");
        let checks = send_text(item_of(&work, export, "checks:demo-org/shop#2414:4973abc0def1"), export, &work);
        assert!(checks.contains("Checks are failing on the latest commit of PR #2414"), "{checks}");
        assert!(checks.contains("Link: https://ci.example.com/run/88123"), "{checks}");
        let bundles = by_key(&groups, "SH-21112");
        let reply = send_text(item_of(&work, bundles, "jira-comment:SH-21112:9001"), bundles, &work);
        assert!(
            reply.contains("Jonas Weber replied on SH-21112 in Jira (Bundles show the cost per article):"),
            "{reply}"
        );
        let returned = by_key(&groups, "SH-21616");
        let back = send_text(item_of(&work, returned, "jira-back:SH-21616"), returned, &work);
        assert!(back.contains("SH-21616 came back to you in Jira"), "{back}");
    }

    // Which Claude a card goes to. The case below: the sh-21000 Claude (a resumed conversation that made PR #2350
    // earlier, and named nine other tickets in prompts) must not be offered for SH-21349, whose PR #2350 a Claude in
    // its own workspace is working on.

    fn at(minutes: i64) -> i64 {
        NOW - minutes * 60_000
    }

    fn sh12524() -> Workspace {
        ws("wA", "sh-21000-price-lists-keep-their-order", "feature/SH-21000-price-lists-keep-their-order")
    }

    fn sh12783() -> Workspace {
        ws("wB", "SH-21349-api-gift-card-balances", "feature/SH-21349-api-gift-card-balances")
    }

    fn a_session(id: &str, workspace: Option<&str>, last_seen: i64) -> Session {
        Session {
            id: id.into(),
            config_dir: format!("{HOME}/.claude-work"),
            cwd: format!("{HOME}/enkidu/workspaces/shop.example/{id}"),
            title: None,
            branches: Vec::new(),
            workspace_id: workspace.map(String::from),
            first_seen: at(600),
            last_seen,
            source: "hook".into(),
        }
    }

    fn pr(number: i64, head: &str, state: &str) -> Pr {
        Pr {
            project_id: "p1".into(),
            repo: "demo-org/shop".into(),
            head: head.into(),
            state: state.into(),
            ..crate::work::pr::tests::pr(number)
        }
    }

    fn pr4937() -> Pr {
        pr(2350, "feature/SH-21349-api-gift-card-balances", "OPEN")
    }

    /// The card of these tests: a ticket key (or none), its PRs, and the sessions linked to it with how; in an index
    /// that holds just those.
    fn card(key: Option<&str>, prs: Vec<Pr>, sessions: Vec<(Session, &str)>) -> (Snapshot, Group) {
        let list = [sh12524(), sh12783()];
        let work = Snapshot {
            prs,
            sessions: sessions.iter().map(|(session, _)| session.clone()).collect(),
            ..Default::default()
        };
        let views = (sessions.iter().enumerate())
            .map(|(ix, (session, how))| SessionView {
                session: ix,
                workspace: list.iter().find(|w| Some(&w.id) == session.workspace_id.as_ref()).cloned(),
                status: None,
                how: how.to_string(),
            })
            .collect();
        let group = Group {
            own: None,
            id: match key {
                Some(key) => format!("ticket:{key}"),
                None => format!("pr:x#{}", work.prs.first().map_or(0, |pr| pr.number)),
            },
            ticket: None,
            key: key.map(String::from),
            title: if key == Some("SH-21349") { "API gift card balances".into() } else { "Something else".into() },
            url: None,
            prs: (0..work.prs.len()).map(|pr| PrView { pr, items: Vec::new(), sessions: Vec::new() }).collect(),
            sessions: views,
            items: Vec::new(),
            lamp: GroupLamp::Off,
            waiting: 0,
            latest: 0,
            who: None,
            stale: false,
            project_id: Some("p1".into()),
            reports: Vec::new(),
        };
        (work, group)
    }

    fn s_a() -> Session {
        a_session("s-wA", Some("wA"), at(1))
    }

    fn s_b() -> Session {
        a_session("s-wB", Some("wB"), at(30))
    }

    fn picked(claudes: &CardClaudes) -> &str {
        claudes.pick.write()
    }

    /// "puts the Claude in the workspace on the card's branch first, and a Claude busy with another ticket last".
    #[test]
    fn puts_the_claude_on_the_cards_branch_first_and_a_claude_busy_with_another_ticket_last() {
        let list = [sh12524(), sh12783()];
        let linked = vec![(s_a(), "created it"), (s_b(), "worked on its branch")];
        let (work, g) = card(Some("SH-21349"), vec![pr4937()], linked);
        let claudes = card_claudes(&g, &work, &list, &statuses(&[("wA", "idle"), ("wB", "done")]), None, None);
        let said: Vec<String> = (claudes.options.iter())
            .map(|o| {
                let place = if o.on_branch {
                    "on its branch"
                } else if o.elsewhere {
                    "elsewhere"
                } else {
                    "-"
                };
                let name = o.workspace.as_ref().map_or("", |w| w.name.as_str());
                format!("{name} {place} {}", o.status.as_deref().unwrap_or(""))
            })
            .collect();
        assert_eq!(
            said,
            [
                "SH-21349-api-gift-card-balances on its branch done",
                "sh-21000-price-lists-keep-their-order elsewhere idle"
            ]
        );
        assert_eq!(picked(&claudes), "s-wB");
        let target = task_target(&g, &work, &list, &none(), None);
        let SendTarget::Workspace { workspace, .. } = target else { panic!("{target:?}") };
        assert_eq!(workspace.name, "SH-21349-api-gift-card-balances");
        // before the workspace followed its folder to the PR's branch: still not the busy one
        let stale = [sh12524(), Workspace { branch: "SH-21349-api-gift-card-balances".into(), ..sh12783() }];
        assert_eq!(picked(&card_claudes(&g, &work, &stale, &none(), None, None)), "s-wB");
    }

    /// "starts a new Claude when the only Claudes linked were named in a prompt, and keeps them there to pick".
    #[test]
    fn starts_a_new_claude_when_the_only_claudes_linked_were_named_in_a_prompt() {
        let list = [sh12524(), sh12783()];
        let (work, g) = card(Some("SH-20259"), Vec::new(), vec![(s_a(), "named in a prompt")]);
        let claudes = card_claudes(&g, &work, &list, &none(), None, None);
        assert_eq!(claudes.options.iter().map(|o| o.session.id.as_str()).collect::<Vec<_>>(), ["s-wA"]);
        assert_eq!(picked(&claudes), "new");
        let target = task_target(&g, &work, &list, &none(), None);
        let SendTarget::New { name, branch, mode, .. } = target else { panic!("{target:?}") };
        let new = ("sh-20259-something-else", "feature/SH-20259-something-else", BranchMode::New);
        assert_eq!((name.as_str(), branch.as_str(), mode), new);
        // you pick it: it goes there, and the pick is what the card shows from then on
        let yours = task_target(&g, &work, &list, &none(), Some(&ClaudeChoice::read("s-wA")));
        let SendTarget::Workspace { workspace, .. } = yours else { panic!("{yours:?}") };
        assert_eq!(workspace.id, "wA");
        // a pick of a Claude no longer linked falls back to the likeliest
        let gone = card_claudes(&g, &work, &list, &none(), Some(&ClaudeChoice::read("gone")), None);
        assert_eq!(picked(&gone), "new");
    }

    /// "starts a new conversation in the card's workspace, or a new workspace when it has none".
    #[test]
    fn starts_a_new_conversation_in_the_cards_workspace_or_a_new_workspace_when_it_has_none() {
        let list = [sh12524(), sh12783()];
        let (work, g) = card(Some("SH-21349"), vec![pr4937()], vec![(s_b(), "worked on its branch")]);
        let fresh = task_target(&g, &work, &list, &none(), Some(&ClaudeChoice::New));
        let SendTarget::Switch { workspace, session } = fresh else { panic!("{fresh:?}") };
        assert_eq!((workspace.id.as_str(), session), ("wB", None));
        let none_there = task_target(&g, &work, &[sh12524()], &none(), Some(&ClaudeChoice::New));
        let SendTarget::New { branch, mode, .. } = none_there else { panic!("{none_there:?}") };
        assert_eq!((branch.as_str(), mode), ("feature/SH-21349-api-gift-card-balances", BranchMode::Existing));
        // always a new workspace when you ask for one, though the card has one: a name that is free
        let another = task_target(&g, &work, &list, &none(), Some(&ClaudeChoice::Fresh));
        let SendTarget::New { name, branch, mode, .. } = another else { panic!("{another:?}") };
        assert_eq!(name, "sh-21349-api-gift-card-balances");
        assert_eq!((branch.as_str(), mode), ("feature/SH-21349-api-gift-card-balances", BranchMode::Existing));
    }

    /// "lists the workspace on the card's branch before any hook linked its conversation".
    #[test]
    fn lists_the_workspace_on_the_cards_branch_before_any_hook_linked_its_conversation() {
        let list = [sh12524(), sh12783()];
        let (work, g) = card(Some("SH-21349"), vec![pr4937()], Vec::new());
        let claudes = card_claudes(&g, &work, &list, &none(), None, None);
        let said: Vec<String> = claudes.options.iter().map(|o| format!("{} {}", o.session.id, o.how)).collect();
        assert_eq!(said, ["s-wB its workspace is on this branch"]);
        assert_eq!(picked(&claudes), "s-wB");
        assert!(!unlinkable(ON_BRANCH) && !unlinkable(PICKED) && unlinkable("created it"));
    }

    /// "moves a workspace to an older conversation of its own, and brings one from outside into the card's workspace
    /// or a new one".
    #[test]
    fn moves_a_workspace_to_an_older_conversation_of_its_own_and_brings_one_from_outside_in() {
        let list = [sh12524(), sh12783()];
        // the workspace's Claude moved on to another conversation since
        let older = a_session("old-1", Some("wB"), at(300));
        let (work, g) = card(Some("SH-21349"), vec![pr4937()], vec![(older, "made a commit in it")]);
        let back = task_target(&g, &work, &list, &none(), Some(&ClaudeChoice::read("old-1")));
        let SendTarget::Switch { workspace, session } = back else { panic!("{back:?}") };
        assert_eq!((workspace.id.as_str(), session.map(|s| s.id).as_deref()), ("wB", Some("old-1")));
        // a conversation from the main folder (Warp): into the workspace on the card's branch, else a new one there
        let warp = Session { cwd: format!("{HOME}/code/shop.example"), ..a_session("warp-1", None, at(60)) };
        let (work, g2) = card(Some("SH-21349"), vec![pr4937()], vec![(warp.clone(), "created it")]);
        let into = task_target(&g2, &work, &list, &none(), Some(&ClaudeChoice::read("warp-1")));
        let SendTarget::Switch { workspace, session } = into else { panic!("{into:?}") };
        assert_eq!((workspace.id.as_str(), session.map(|s| s.id).as_deref()), ("wB", Some("warp-1")));
        let brought = task_target(&g2, &work, &[sh12524()], &none(), None);
        let SendTarget::Outside { session, branch, mode, .. } = brought else { panic!("{brought:?}") };
        let came = ("warp-1", "feature/SH-21349-api-gift-card-balances", BranchMode::Existing);
        assert_eq!((session.id.as_str(), branch.as_str(), mode), came);
        // with no open PR to bring it in on, a new Claude unless you pick it
        let merged = pr(2350, "feature/SH-21349-api-gift-card-balances", "MERGED");
        let (work, g3) = card(Some("SH-21349"), vec![merged], vec![(warp, "created it")]);
        assert_eq!(picked(&card_claudes(&g3, &work, &[sh12524()], &none(), None, None)), "new");
    }

    /// "goes where you or Claude linked it, even to a Claude busy with another ticket, and not where it only made
    /// the PR".
    #[test]
    fn goes_where_you_or_claude_linked_it_even_to_a_claude_busy_with_another_ticket() {
        let pick = |how: &str| {
            let (work, g) = card(Some("SH-21349"), vec![pr4937()], vec![(s_a(), how)]);
            picked(&card_claudes(&g, &work, &[sh12524()], &none(), None, None)).to_string()
        };
        assert_eq!(pick("linked by you"), "s-wA");
        assert_eq!(pick("linked by Claude"), "s-wA");
        assert_eq!(pick("created it"), "new");
    }

    /// A Claude you picked that nothing links to the card is on it as "picked by you" (the old Claude box's
    /// "Another workspace's Claude").
    #[test]
    fn a_workspaces_claude_you_picked_for_the_card_is_on_it_though_nothing_links_them() {
        let list = [sh12524(), sh12783()];
        let (work, g) = card(Some("SH-20259"), Vec::new(), Vec::new());
        let yours = ClaudeChoice::read("s-wA");
        let claudes = card_claudes(&g, &work, &list, &statuses(&[("wA", "working")]), Some(&yours), None);
        let option = claudes.picked().expect("the Claude you picked");
        assert_eq!((option.how.as_str(), option.status.as_deref()), (PICKED, Some("working")));
        assert_eq!(claudes.pick, yours);
        assert_eq!((ClaudeChoice::read("new"), ClaudeChoice::read("fresh")), (ClaudeChoice::New, ClaudeChoice::Fresh));
        assert_eq!((ClaudeChoice::New.write(), ClaudeChoice::Fresh.write(), yours.write()), ("new", "fresh", "s-wA"));
    }

    /// The sample with the stock-fix Claude having only named SH-21309 in a prompt (the old Claude box tests).
    fn named_in_a_prompt() -> Snapshot {
        let mut work = sample();
        let link = serde_json::json!({ "a": "ticket:SH-21309", "b": "session:s-w1", "how": "transcript",
            "detail": "named in a prompt", "at": NOW, "removed": false });
        work.links.push(serde_json::from_value(link).expect("a link"));
        work
    }

    fn box_of(work: &Snapshot, list: &[Workspace], chosen: Option<&str>) -> ClaudeBox {
        let groups = build_groups(work, list, &none(), NOW);
        let chosen = chosen.map(ClaudeChoice::read);
        claude_box(by_key(&groups, "SH-21309"), work, list, &none(), chosen.as_ref(), NOW).expect("its Claude box")
    }

    fn rows(the: &ClaudeBox) -> Vec<String> {
        let mut rows: Vec<String> = the.rows.iter().map(|row| format!("{} {}", row.session, row.picked)).collect();
        rows.push(format!("new {}", the.new_claude.picked));
        rows.extend(the.new_workspace.iter().map(|row| format!("fresh {}", row.picked)));
        rows
    }

    /// Lets you pick the Claude a card goes to: one linked to it, or a new one.
    #[test]
    fn the_claude_box_lists_each_claude_linked_to_the_card_then_a_new_one() {
        let work = named_in_a_prompt();
        let list =
            [ws("w1", "stock-fix", "fix/stock-double-count"), ws("w2", "sh-21309-export", "feature/SH-21309-export")];
        let the = box_of(&work, &list, None);
        // the workspace on the card's branch is picked; named in a prompt is never enough
        assert_eq!(rows(&the), ["s-w2 true", "s-w1 false", "new false", "fresh false"]);
        let named = &the.rows[1];
        assert!(named.line.starts_with("Named in a prompt"), "{}", named.line);
        assert_eq!((named.name.as_str(), named.workspace.as_deref()), ("stock-fix", Some("w1")));
        assert_eq!(the.rows[0].name, "sh-21309-export");
        assert_eq!(the.new_claude.line, "A new conversation in sh-21309-export");
        assert_eq!(the.new_workspace.as_ref().map(|row| row.line.as_str()), Some("A new workspace, sh-21309-export-2"));
        assert!(the.others.is_empty(), "both workspaces are on the card");
        // your pick is the one picked, whichever it is
        assert_eq!(rows(&box_of(&work, &list, Some("new"))), ["s-w2 false", "s-w1 false", "new true", "fresh false"]);
        assert_eq!(rows(&box_of(&work, &list, Some("s-w1"))), ["s-w2 false", "s-w1 true", "new false", "fresh false"]);
        assert_eq!(rows(&box_of(&work, &list, Some("fresh"))), ["s-w2 false", "s-w1 false", "new false", "fresh true"]);
        // the card's ticket and open PR, for "linked by you"
        let groups = build_groups(&work, &list, &none(), NOW);
        assert_eq!(card_subjects(by_key(&groups, "SH-21309"), &work), ["ticket:SH-21309", "pr:demo-org/shop#2414"]);
        assert_eq!(card_subjects(by_key(&groups, "SH-21030"), &work), ["ticket:SH-21030", "pr:demo-org/shop#2312"]);
    }

    /// Sends a card to a new workspace though it has one, to any workspace's Claude, and unlinks a wrong one.
    #[test]
    fn the_claude_box_offers_any_other_workspaces_claude_and_says_which_links_can_be_undone() {
        let work = named_in_a_prompt();
        let list = [
            ws("w1", "stock-fix", "fix/stock-double-count"),
            ws("w2", "sh-21309-export", "feature/SH-21309-export"),
            ws("w3", "scratch", "scratch"),
        ];
        // scratch is linked to nothing, but you can send it there
        let the = box_of(&work, &list, None);
        assert_eq!(the.others, [("s-w3".to_string(), "scratch".to_string(), "scratch".to_string())]);
        let picked = box_of(&work, &list, Some("s-w3"));
        let scratch = picked.rows.iter().find(|row| row.session == "s-w3").expect("scratch is on the card now");
        assert!(scratch.picked && scratch.line.starts_with("Picked by you"), "{scratch:?}");
        assert!(picked.others.is_empty());
        // both others are linked to the card: a wrong link can be undone; scratch is only your pick
        let undo: Vec<&str> = picked.rows.iter().filter(|row| row.unlinkable).map(|row| row.session.as_str()).collect();
        assert_eq!(undo, ["s-w2", "s-w1"]);
        // a session that ran outside Enkidu says where; a card nobody worked on in a project without
        // workspaces has no box at all
        let groups = build_groups(&work, &[], &none(), NOW);
        let units = claude_box(by_key(&groups, "SH-21376"), &work, &[], &none(), None, NOW).expect("its box");
        assert!(units.rows[0].line.contains(", in ~/"), "{}", units.rows[0].line);
        assert_eq!((units.rows[0].workspace.clone(), units.new_workspace.clone()), (None, None));
        assert!(claude_box(by_key(&groups, "SH-21522"), &work, &[], &none(), None, NOW).is_none());
    }
}
