//! What the Work index knows: tickets (Jira), PRs (GitHub), Claude sessions, the links between them, the items
//! waiting on you, each source's health, and every thing's timeline.

use serde::{Deserialize, Serialize};

/// A check on a PR's latest commit that isn't green.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub name: String,
    /// What GitHub says: FAILURE, CANCELLED, TIMED_OUT, ERROR…
    pub state: String,
    pub url: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Pr {
    pub project_id: String,
    /// owner/name
    pub repo: String,
    pub number: i64,
    pub title: String,
    pub url: String,
    /// OPEN, MERGED or CLOSED
    pub state: String,
    pub draft: bool,
    pub head: String,
    pub base: String,
    /// APPROVED, CHANGES_REQUESTED, REVIEW_REQUIRED, or none
    pub review_decision: Option<String>,
    /// The latest commit's checks together: SUCCESS, FAILURE, ERROR, PENDING, EXPECTED, or none
    pub checks: Option<String>,
    pub failing: Vec<Check>,
    pub running: usize,
    /// MERGEABLE, CONFLICTING or UNKNOWN (GitHub works it out lazily)
    pub mergeable: Option<String>,
    pub head_sha: Option<String>,
    /// When the latest commit was made: close enough to "your last push"
    pub head_at: Option<String>,
    pub unresolved: usize,
    pub updated_at: String,
    pub merged_at: Option<String>,
    /// The PR's commits (up to the last 100), oldest first
    pub commits: Vec<String>,
    /// Ticket keys in the PR's commit messages (the messages themselves aren't kept)
    #[serde(default)]
    pub commit_keys: Vec<String>,
    /// The latest comment in the PR's conversation: who and when (review threads are counted in `unresolved`)
    #[serde(default)]
    pub last_comment_by: Option<String>,
    #[serde(default)]
    pub last_comment_at: Option<String>,
    /// What the card in front of the board shows about it (an open PR's, from the same read)
    #[serde(default)]
    pub details: PrDetails,
}

/// What is going on in a PR, as the card in front of the board shows it: its description, size, every check, each
/// reviewer's latest review, who is asked to review, every review thread, the conversation and the latest commits.
/// Texts are cut short; the rest is a click away on GitHub.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct PrDetails {
    pub author: Option<String>,
    pub created_at: Option<String>,
    /// The PR's own description (Markdown)
    pub body: String,
    pub additions: Option<i64>,
    pub deletions: Option<i64>,
    pub changed_files: Option<i64>,
    pub labels: Vec<String>,
    /// Every check on the latest commit, by gh's buckets: pass, fail, pending, skipping, cancel
    pub checks: Vec<CheckLine>,
    /// Each reviewer's latest review
    pub reviews: Vec<ReviewLine>,
    /// Asked to review (people, or a team by its name)
    pub requested: Vec<String>,
    /// Every review thread, the open ones first
    pub threads: Vec<ThreadLine>,
    /// The conversation's latest comments, oldest first
    pub comments: Vec<CommentLine>,
    pub commit_count: usize,
    /// The latest commits, newest first
    pub recent_commits: Vec<CommitLine>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CheckLine {
    pub name: String,
    pub bucket: String,
    pub url: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewLine {
    pub who: String,
    /// APPROVED, CHANGES_REQUESTED, COMMENTED or DISMISSED
    pub state: String,
    pub at: Option<String>,
    pub bot: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ThreadLine {
    pub path: Option<String>,
    pub line: Option<i64>,
    pub resolved: bool,
    pub outdated: bool,
    /// Who started it, and what they said (cut short)
    pub who: Option<String>,
    pub bot: bool,
    pub text: String,
    pub replies: usize,
    /// The latest reply: who and when
    pub last_who: Option<String>,
    pub last_at: Option<String>,
    pub url: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CommentLine {
    pub who: Option<String>,
    pub bot: bool,
    pub text: String,
    pub at: String,
    pub url: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CommitLine {
    /// The first 7 characters
    pub oid: String,
    pub headline: String,
}

impl Pr {
    pub fn reference(&self) -> String {
        pr_ref(&self.repo, self.number)
    }
}

pub fn pr_ref(repo: &str, number: i64) -> String {
    format!("{repo}#{number}")
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Ticket {
    pub key: String,
    pub project_key: String,
    pub summary: String,
    pub url: String,
    pub status: String,
    /// new, indeterminate or done (Jira's status category)
    pub category: String,
    pub assignee: Option<String>,
    pub assignee_id: Option<String>,
    pub updated_at: String,
    pub comments: usize,
    pub attachments: usize,
    /// The latest comment: who and when (known for tickets read whole)
    #[serde(default)]
    pub last_comment_by: Option<String>,
    #[serde(default)]
    pub last_comment_at: Option<String>,
    /// What the ticket asks for: its description as Jira's document format (ADF), read again only when the ticket
    /// changes (see `described_at`)
    #[serde(default)]
    pub description: Option<serde_json::Value>,
    /// The `updated_at` the description, type and priority were read at
    #[serde(default)]
    pub described_at: Option<String>,
    /// Task, Bug, Story, Sub-task, Epic…
    #[serde(default)]
    pub issue_type: Option<String>,
    /// Highest, High, Medium, Low, Lowest
    #[serde(default)]
    pub priority: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    /// Claude's session id
    pub id: String,
    /// The Claude setup folder it lives in (`~/.claude`, `~/.claude-work`…), so it can be resumed with the right
    /// account
    pub config_dir: String,
    pub cwd: String,
    pub title: Option<String>,
    /// Every branch it worked on, in the order it first did
    pub branches: Vec<String>,
    pub workspace_id: Option<String>,
    pub first_seen: i64,
    pub last_seen: i64,
    /// workspace, hook or transcript: how Enkidu learned about it
    pub source: String,
}

/// A link between two things, kept forever: removing it only marks it.
/// Things are written `ticket:SH-1`, `pr:owner/repo#12`, `session:<id>`, `workspace:<id>`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Link {
    pub a: String,
    pub b: String,
    /// branch, transcript, hook:pr-create, hook:pr, hook:commit, hook:mention, ticket-key, jira-dev, claude, you
    pub how: String,
    /// In words, for the page: "pushed its branch", "created it", "the key is in its branch"
    pub detail: String,
    pub at: i64,
    pub removed: bool,
}

/// Something waiting on you. Its id says what it is and where it came from, so reading again updates it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub id: String,
    /// remark, probably_fixed, question, changes_requested, pipeline_failed, conflict,
    /// qa_replied, mentioned, back_to_you, attachment
    pub kind: String,
    /// github or jira
    pub source: String,
    pub project_id: String,
    pub pr: Option<String>,
    pub ticket: Option<String>,
    pub title: String,
    pub detail: String,
    pub who: Option<String>,
    pub url: Option<String>,
    pub path: Option<String>,
    pub line: Option<i64>,
    /// When it happened at the source
    pub at: String,
    /// open, closed (its reason went away), done (you said so) or snoozed
    pub state: String,
    pub opened_at: i64,
    pub closed_at: Option<i64>,
    pub snoozed_until: Option<i64>,
    /// written by a bot: shown apart
    pub bot: bool,
}

/// What the rules find on one read, before the store merges it with what it already had.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemDraft {
    pub id: String,
    pub kind: &'static str,
    pub pr: Option<String>,
    pub ticket: Option<String>,
    pub title: String,
    pub detail: String,
    pub who: Option<String>,
    pub url: Option<String>,
    pub path: Option<String>,
    pub line: Option<i64>,
    pub at: String,
    pub bot: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    /// What it is about: a ticket key, a PR reference, a session id
    pub subject: String,
    pub at: i64,
    pub kind: String,
    pub text: String,
}

/// Whether a source (GitHub or Jira) could be read for a project.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Health {
    pub project_id: String,
    /// github or jira
    pub source: String,
    /// ok, error, off (not used for this project) or pending (not read yet)
    pub state: String,
    pub last_ok: Option<i64>,
    pub last_try: Option<i64>,
    pub error: Option<String>,
    /// What to do about the error, in words: "Run `twg login`."
    pub fix: Option<String>,
    pub next_try: Option<i64>,
    /// The GitHub login or Jira user it reads as
    pub account: Option<String>,
}

/// A board in Jira (Jira Software): the Work page shows one board's tickets when you pick it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Board {
    pub id: String,
    pub name: String,
    /// scrum or kanban
    pub kind: String,
    /// The Jira project it lives in (none for a board in someone's profile)
    #[serde(default)]
    pub project_key: Option<String>,
}

/// The board you picked, and which of your tickets are on it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BoardPick {
    #[serde(flatten)]
    pub board: Board,
    /// The tickets on it, as its own page in Jira shows it; none until it was read
    pub keys: Option<Vec<String>>,
    pub read_at: Option<i64>,
}

/// A task of your own on the Work board (item 8.5): something to do in one project. On the board it is
/// the card `own:<id>`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OwnTask {
    pub id: i64,
    pub project_id: String,
    pub title: String,
    /// What it is about, in your words; may be empty
    #[serde(default)]
    pub notes: String,
    /// `todo`, `doing` or `done`
    pub state: String,
    pub created_at: i64,
    pub updated_at: i64,
    /// Its project's name, for the card (empty when the project is gone)
    #[serde(default)]
    pub project: String,
}

/// The states a task of your own can be in, as the database keeps them.
pub const OWN_STATES: [&str; 3] = ["todo", "doing", "done"];

impl OwnTask {
    /// Its card on the board, and what links, reports and workflows name it by: `own:3`.
    pub fn subject(&self) -> String {
        format!("own:{}", self.id)
    }
}

/// The id in a subject that names a task of your own (`own:3`).
pub fn own_id(subject: &str) -> Option<i64> {
    subject.strip_prefix("own:")?.parse().ok()
}

/// Whether a subject names a card of the board: a ticket (`ticket:SH-1`), a PR without one (`pr:owner/repo#2`), or a
/// task of your own (`own:3`).
pub fn is_card(subject: &str) -> bool {
    subject.starts_with("ticket:") || subject.starts_with("pr:") || own_id(subject).is_some()
}

/// Everything the Work page shows. (It can be read back too: the sample the tests and pictures use is kept as JSON.)
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Snapshot {
    pub health: Vec<Health>,
    pub prs: Vec<Pr>,
    pub tickets: Vec<Ticket>,
    /// Each ticket's project (by key): the board shows one project's tickets, also those without a PR
    pub ticket_projects: std::collections::BTreeMap<String, String>,
    /// The cards you hid from the board (`ticket:SH-1`, `pr:owner/repo#2`)
    pub hidden: Vec<String>,
    /// Jira's boards, as last read (the choice in the page's header)
    pub boards: Vec<Board>,
    /// The Jira board you picked: only its tickets are on the page (none: everything read)
    pub board: Option<BoardPick>,
    pub sessions: Vec<Session>,
    pub links: Vec<Link>,
    pub items: Vec<Item>,
    pub events: Vec<Event>,
    /// Claude's reports on its tasks (`enkidu report`) of the last 30 days, the newest first
    pub reports: Vec<super::reports::Report>,
    /// Your scheduled workflows, the newest first
    pub flows: Vec<super::flows::Flow>,
    /// Your own default words per kind of workflow task, where you changed them
    pub flow_kinds: std::collections::BTreeMap<String, String>,
    /// What a new workflow starts from, where you set it (the minutes a task gets, the time it starts at)
    pub flow_defaults: super::flows::FlowDefaults,
    /// The runs of your workflows, kept as they ended, the newest first
    pub flow_runs: Vec<super::flows::FlowRun>,
    /// Your own tasks, in the order you made them
    pub own_tasks: Vec<OwnTask>,
    pub checking: bool,
}
