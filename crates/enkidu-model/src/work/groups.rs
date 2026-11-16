//! The Work page's view of the index: one group per ticket, or per PR that names no ticket, with its PRs, what waits on
//! you there and the Claudes behind it; what each group's lamp says; the filter. The board (`super::board`) lays the
//! groups out in columns.
//!
//! A group names its ticket, PRs, items, sessions and reports by their place in the snapshot it was built from, so
//! building the groups copies nothing big (a ticket's description, a PR's threads): read them with the snapshot.

use std::collections::{HashMap, HashSet};

use enkidu_core::db::Workspace;
use enkidu_core::work::model::{BoardPick, Item, OwnTask, Pr, Session, Snapshot, Ticket};
use enkidu_core::work::parse::parse_time;
use enkidu_core::work::reports::Report;

use super::pr::{Checks, checks_of};

/// What a card's lamp says: red, something blocks (failing checks, changes requested, a conflict, back to you);
/// blue, a person waits on you; amber, checks are running; green, done; off, nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GroupLamp {
    Red,
    Blue,
    Amber,
    Off,
    Green,
}

impl GroupLamp {
    pub fn name(self) -> &'static str {
        match self {
            GroupLamp::Red => "red",
            GroupLamp::Blue => "blue",
            GroupLamp::Amber => "amber",
            GroupLamp::Off => "off",
            GroupLamp::Green => "green",
        }
    }
}

/// A Claude session behind a card.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionView {
    /// Its place in the snapshot's sessions.
    pub session: usize,
    /// The Enkidu workspace it runs in, if any.
    pub workspace: Option<Workspace>,
    /// That workspace's lamp: `idle`, `working`, `needs_you`, `done`.
    pub status: Option<String>,
    /// How it is linked, in words ("created it"): its strongest link.
    pub how: String,
}

impl SessionView {
    pub fn of<'a>(&self, work: &'a Snapshot) -> &'a Session {
        &work.sessions[self.session]
    }
}

/// A PR on a card.
#[derive(Clone, Debug, PartialEq)]
pub struct PrView {
    /// Its place in the snapshot's PRs.
    pub pr: usize,
    /// Its items that wait on you (people first, then bots), by their place in the snapshot's items.
    pub items: Vec<usize>,
    /// The sessions behind this PR itself, the most recent first.
    pub sessions: Vec<SessionView>,
}

impl PrView {
    pub fn of<'a>(&self, work: &'a Snapshot) -> &'a Pr {
        &work.prs[self.pr]
    }

    pub fn items<'a>(&'a self, work: &'a Snapshot) -> impl Iterator<Item = &'a Item> + 'a {
        self.items.iter().map(|ix| &work.items[*ix])
    }
}

/// How much a link says that a session is the one doing a card's work: being named in a prompt says little (the
/// sh-21000 Claude named nine tickets), making the PR or committing to it says a lot, and you saying so says most.
pub fn link_strength(how: &str) -> u8 {
    match how {
        "linked by you" => 5,
        "created it" | "pushed its branch" | "made a commit in it" | "committed to its branch" => 4,
        "linked by Claude" | "worked on its branch" => 3,
        "worked on it" => 2,
        _ => 1,
    }
}

/// A card of the page: a ticket with its PRs, a PR that names no ticket, or a task of your own.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    /// `ticket:SH-1`, `pr:owner/repo#N` or `own:3`.
    pub id: String,
    /// For a task of your own (item 8.5): its place in the snapshot's own tasks.
    pub own: Option<usize>,
    /// The ticket's place in the snapshot's tickets, when it was read.
    pub ticket: Option<usize>,
    /// The ticket's key, also when the ticket itself wasn't read (another Jira project).
    pub key: Option<String>,
    pub title: String,
    pub url: Option<String>,
    pub prs: Vec<PrView>,
    pub sessions: Vec<SessionView>,
    /// The ticket's own items (from Jira), by their place in the snapshot's items.
    pub items: Vec<usize>,
    pub lamp: GroupLamp,
    /// Open items from people.
    pub waiting: usize,
    /// The latest activity, in milliseconds.
    pub latest: i64,
    /// Who the newest waiting item is from.
    pub who: Option<String>,
    /// It has items from a source that can't be read now.
    pub stale: bool,
    /// The project it belongs to (its PRs', else where its ticket was read).
    pub project_id: Option<String>,
    /// Claude's reports on it (`enkidu report`), the newest first, by their place in the snapshot's reports.
    pub reports: Vec<usize>,
}

impl Group {
    pub fn ticket<'a>(&self, work: &'a Snapshot) -> Option<&'a Ticket> {
        self.ticket.map(|ix| &work.tickets[ix])
    }

    /// The task of your own this card is, if it is one.
    pub fn own<'a>(&self, work: &'a Snapshot) -> Option<&'a OwnTask> {
        self.own.map(|ix| &work.own_tasks[ix])
    }

    /// The ticket's own items.
    pub fn items<'a>(&'a self, work: &'a Snapshot) -> impl Iterator<Item = &'a Item> + 'a {
        self.items.iter().map(|ix| &work.items[*ix])
    }

    /// Everything that waits on the card: the ticket's own items, then each PR's.
    pub fn all_items<'a>(&'a self, work: &'a Snapshot) -> impl Iterator<Item = &'a Item> + 'a {
        self.items(work).chain(self.prs.iter().flat_map(move |pr| pr.items(work)))
    }

    pub fn reports<'a>(&'a self, work: &'a Snapshot) -> impl Iterator<Item = &'a Report> + 'a {
        self.reports.iter().map(|ix| &work.reports[*ix])
    }

    /// The key when it has one, else its id (how the tests name a card).
    pub fn name(&self) -> &str {
        self.key.as_deref().unwrap_or(&self.id)
    }
}

/// A time as the sources write it, in milliseconds; 0 for none.
pub fn time(text: &str) -> i64 {
    parse_time(text).unwrap_or(0)
}

/// An item waits now: it is open, or it was snoozed and its time has come.
fn open(item: &Item, now: i64) -> bool {
    item.state == "open" || (item.state == "snoozed" && item.snoozed_until.unwrap_or(0) <= now)
}

/// What makes a card red.
const RED: [&str; 4] = ["pipeline_failed", "changes_requested", "conflict", "back_to_you"];

/// A map that remembers the order its keys came in (the old code leaned on JavaScript's `Map` and `Set` for that:
/// cards that tie keep the order of the index).
struct Ordered<'a, V> {
    order: Vec<&'a str>,
    of: HashMap<&'a str, V>,
}

impl<'a, V> Ordered<'a, V> {
    fn new() -> Self {
        Ordered { order: Vec::new(), of: HashMap::new() }
    }

    fn entry(&mut self, key: &'a str, make: impl FnOnce() -> V) -> &mut V {
        let order = &mut self.order;
        self.of.entry(key).or_insert_with(|| {
            order.push(key);
            make()
        })
    }

    fn get(&self, key: &str) -> Option<&V> {
        self.of.get(key)
    }
}

/// The groups of the page, worst first, then the latest activity.
pub fn build_groups(
    work: &Snapshot,
    workspaces: &[Workspace],
    statuses: &HashMap<String, String>,
    now: i64,
) -> Vec<Group> {
    let broken: HashSet<(&str, &str)> = (work.health.iter())
        .filter(|h| h.state == "error")
        .map(|h| (h.project_id.as_str(), h.source.as_str()))
        .collect();
    let references: Vec<String> = work.prs.iter().map(Pr::reference).collect();
    let prs: HashMap<&str, usize> = references.iter().enumerate().map(|(ix, r)| (r.as_str(), ix)).collect();
    // a key read twice is the one read last, at the place of the first
    let mut tickets: Ordered<usize> = Ordered::new();
    for (ix, ticket) in work.tickets.iter().enumerate() {
        *tickets.entry(&ticket.key, || ix) = ix;
    }
    let sessions: HashMap<&str, usize> = work.sessions.iter().enumerate().map(|(ix, s)| (s.id.as_str(), ix)).collect();
    // what waits now, by the PR it is on, or else by its ticket
    let mut on_pr: HashMap<&str, Vec<usize>> = HashMap::new();
    let mut on_ticket: HashMap<&str, Vec<usize>> = HashMap::new();
    for (ix, item) in work.items.iter().enumerate().filter(|(_, item)| open(item, now)) {
        match (item.pr.as_deref().filter(|pr| !pr.is_empty()), item.ticket.as_deref()) {
            (Some(pr), _) => on_pr.entry(pr).or_default().push(ix),
            (None, Some(key)) => on_ticket.entry(key).or_default().push(ix),
            (None, None) => {}
        }
    }
    // people first, then bots; the newest first among each
    let people_first = |items: &mut Vec<usize>| {
        items.sort_by(|a, b| {
            let (a, b) = (&work.items[*a], &work.items[*b]);
            a.bot.cmp(&b.bot).then_with(|| time(&b.at).cmp(&time(&a.at)))
        });
    };
    // ticket → its PRs, PR or ticket → its sessions (each with its strongest link), PR → its ticket
    let mut prs_of: Ordered<Vec<usize>> = Ordered::new();
    let mut sessions_of: HashMap<&str, Ordered<&str>> = HashMap::new();
    let mut ticket_of_pr: HashMap<usize, &str> = HashMap::new();
    for link in work.links.iter().filter(|link| !link.removed) {
        if let (Some(key), Some(reference)) = (link.a.strip_prefix("ticket:"), link.b.strip_prefix("pr:")) {
            let Some(pr) = prs.get(reference).copied() else { continue };
            let its = prs_of.entry(key, Vec::new);
            if !its.contains(&pr) {
                its.push(pr);
            }
            ticket_of_pr.entry(pr).or_insert(key);
        } else if let Some(session) = link.b.strip_prefix("session:")
            && (link.a.starts_with("pr:") || link.a.starts_with("ticket:") || link.a.starts_with("own:"))
        {
            let detail = link.detail.as_str();
            let how = sessions_of.entry(link.a.as_str()).or_insert_with(Ordered::new).entry(session, || detail);
            if link_strength(detail) > link_strength(how) {
                *how = detail;
            }
        }
    }
    let by_id: HashMap<&str, &Workspace> = workspaces.iter().map(|w| (w.id.as_str(), w)).collect();
    let session_views = |things: &[String]| -> Vec<SessionView> {
        let mut seen: Ordered<&str> = Ordered::new();
        for thing in things {
            let Some(theirs) = sessions_of.get(thing.as_str()) else { continue };
            for session in &theirs.order {
                let how = theirs.of[session];
                let had = seen.entry(session, || how);
                if link_strength(how) > link_strength(had) {
                    *had = how;
                }
            }
        }
        let mut views: Vec<SessionView> = (seen.order.iter())
            .filter_map(|id| {
                let ix = *sessions.get(id)?;
                let session = &work.sessions[ix];
                let workspace = (session.workspace_id.as_deref())
                    .and_then(|id| by_id.get(id).copied())
                    .or_else(|| workspaces.iter().find(|w| w.session_id == session.id));
                let status = workspace.map(|w| statuses.get(&w.id).cloned().unwrap_or_else(|| "idle".into()));
                Some(SessionView { session: ix, workspace: workspace.cloned(), status, how: seen.of[id].to_string() })
            })
            .collect();
        views.sort_by(|a, b| work.sessions[b.session].last_seen.cmp(&work.sessions[a.session].last_seen));
        views
    };
    let pr_view = |pr: usize| -> PrView {
        let mut items = on_pr.get(references[pr].as_str()).cloned().unwrap_or_default();
        people_first(&mut items);
        PrView { pr, items, sessions: session_views(&[format!("pr:{}", references[pr])]) }
    };
    let state_order = |pr: &Pr| match pr.state.as_str() {
        "OPEN" => 0,
        "MERGED" => 1,
        _ => 2,
    };
    // the rest of a group, from what it holds
    let finish = |id: String,
                  ticket: Option<usize>,
                  key: Option<&str>,
                  title: String,
                  url: Option<String>,
                  prs: Vec<PrView>,
                  sessions: Vec<SessionView>,
                  items: Vec<usize>|
     -> Group {
        // Claude's reports on the ticket or its PRs
        let mut subjects: HashSet<String> = prs.iter().map(|pr| references[pr.pr].to_uppercase()).collect();
        subjects.extend(key.map(str::to_uppercase));
        let reports: Vec<usize> = (work.reports.iter().enumerate())
            .filter(|(_, report)| subjects.contains(&report.subject.to_uppercase()))
            .map(|(ix, _)| ix)
            .collect();
        let all: Vec<&Item> =
            items.iter().chain(prs.iter().flat_map(|pr| pr.items.iter())).map(|ix| &work.items[*ix]).collect();
        let people: Vec<&Item> = all.iter().copied().filter(|item| !item.bot).collect();
        let open_with = |checks: Checks| {
            prs.iter().map(|pr| &work.prs[pr.pr]).any(|pr| pr.state == "OPEN" && checks_of(pr) == checks)
        };
        let (failing, running) = (open_with(Checks::Failing), open_with(Checks::Running));
        let done = match ticket {
            Some(ix) => work.tickets[ix].category == "done",
            None => prs.iter().all(|pr| work.prs[pr.pr].state != "OPEN"),
        };
        let lamp = if failing || people.iter().any(|item| RED.contains(&item.kind.as_str())) {
            GroupLamp::Red
        } else if !people.is_empty() {
            GroupLamp::Blue
        } else if running {
            GroupLamp::Amber
        } else if done {
            GroupLamp::Green
        } else {
            GroupLamp::Off
        };
        let latest = [ticket.map_or(0, |ix| time(&work.tickets[ix].updated_at))]
            .into_iter()
            .chain(prs.iter().map(|pr| time(&work.prs[pr.pr].updated_at)))
            .chain(all.iter().map(|item| time(&item.at)))
            .chain(sessions.iter().map(|s| work.sessions[s.session].last_seen))
            .chain(reports.first().map(|ix| work.reports[*ix].at))
            .fold(0, i64::max);
        // the newest of the people's items (the first of the newest, as a stable sort gave it)
        let newest = people.iter().fold(None::<&Item>, |best, item| match best {
            Some(best) if time(&best.at) >= time(&item.at) => Some(best),
            _ => Some(item),
        });
        let stale = all.iter().any(|item| broken.contains(&(item.project_id.as_str(), item.source.as_str())));
        let project_id = (prs.first().map(|pr| work.prs[pr.pr].project_id.clone()))
            .or_else(|| key.and_then(|key| work.ticket_projects.get(key).cloned()))
            .or_else(|| all.first().map(|item| item.project_id.clone()));
        Group {
            id,
            own: None,
            ticket,
            key: key.map(String::from),
            title,
            url,
            waiting: people.len(),
            who: newest.and_then(|item| item.who.clone()),
            lamp,
            latest,
            stale,
            project_id,
            reports,
            prs,
            sessions,
            items,
        }
    };
    let mut groups: Vec<Group> = Vec::new();
    // a group per ticket: the tickets read, and the keys PRs name that weren't read
    let keys = tickets.order.iter().chain(prs_of.order.iter().filter(|key| tickets.get(key).is_none()));
    for key in keys.copied() {
        let ticket = tickets.get(key).copied();
        let mut its: Vec<PrView> = prs_of.get(key).into_iter().flatten().map(|pr| pr_view(*pr)).collect();
        its.sort_by(|a, b| {
            let (a, b) = (&work.prs[a.pr], &work.prs[b.pr]);
            state_order(a).cmp(&state_order(b)).then_with(|| time(&b.updated_at).cmp(&time(&a.updated_at)))
        });
        if ticket.is_none() && its.is_empty() {
            continue;
        }
        let mut items = on_ticket.get(key).cloned().unwrap_or_default();
        people_first(&mut items);
        let read = ticket.map(|ix| &work.tickets[ix]);
        let title = (read.map(|t| t.summary.clone()))
            .or_else(|| its.first().map(|pr| work.prs[pr.pr].title.clone()))
            .unwrap_or_else(|| key.to_string());
        let mut things = vec![format!("ticket:{key}")];
        things.extend(its.iter().map(|pr| format!("pr:{}", references[pr.pr])));
        let sessions = session_views(&things);
        groups.push(finish(
            format!("ticket:{key}"),
            ticket,
            Some(key),
            title,
            read.map(|t| t.url.clone()),
            its,
            sessions,
            items,
        ));
    }
    // a PR that names no ticket is a group of its own
    for (ix, pr) in work.prs.iter().enumerate().filter(|(ix, _)| !ticket_of_pr.contains_key(ix)) {
        let id = format!("pr:{}", references[ix]);
        let sessions = session_views(std::slice::from_ref(&id));
        groups.push(finish(
            id,
            None,
            None,
            pr.title.clone(),
            Some(pr.url.clone()),
            vec![pr_view(ix)],
            sessions,
            Vec::new(),
        ));
    }
    // a task of your own is a card too: its Claudes are the ones linked to it, its reports the ones made on it
    for (ix, task) in work.own_tasks.iter().enumerate() {
        let id = task.subject();
        let sessions = session_views(std::slice::from_ref(&id));
        let mut group = finish(id.clone(), None, None, task.title.clone(), None, Vec::new(), sessions, Vec::new());
        group.own = Some(ix);
        group.project_id = Some(task.project_id.clone());
        group.reports =
            (work.reports.iter().enumerate()).filter(|(_, report)| report.subject == id).map(|(ix, _)| ix).collect();
        let reported = group.reports.first().map_or(0, |ix| work.reports[*ix].at);
        group.latest = group.latest.max(task.updated_at).max(reported);
        // nothing waits on it for anyone else, so its lamp only says whether it is done
        group.lamp = if task.state == "done" { GroupLamp::Green } else { GroupLamp::Off };
        groups.push(group);
    }
    groups.sort_by(|a, b| a.lamp.cmp(&b.lamp).then_with(|| b.latest.cmp(&a.latest)));
    groups
}

/// The cards of the Jira board you picked: the tickets on it, as its own page in Jira shows it (a scrum board: the
/// sprint that is running). No board picked: every card. A PR that names no ticket is on no Jira board. Until the
/// board was read, none. A task of your own is on every board: it is yours, not Jira's.
pub fn board_cards(groups: Vec<Group>, board: Option<&BoardPick>) -> Vec<Group> {
    let Some(board) = board else { return groups };
    let keys: HashSet<&str> = board.keys.iter().flatten().map(String::as_str).collect();
    groups
        .into_iter()
        .filter(|group| group.own.is_some() || group.key.as_deref().is_some_and(|key| keys.contains(key)))
        .collect()
}

/// Does the filter's text match a group (keys, titles, branches, names)? Every word of it must be somewhere.
pub fn matches(group: &Group, work: &Snapshot, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return true;
    }
    let mut hay: Vec<String> = Vec::new();
    hay.extend(group.key.clone());
    hay.push(group.title.clone());
    if let Some(task) = group.own(work) {
        hay.push(task.notes.clone());
        hay.push(crate::work::board::own_state(&task.state).1.to_string());
    }
    if let Some(ticket) = group.ticket(work) {
        hay.push(ticket.status.clone());
        hay.extend(ticket.assignee.clone());
    }
    for view in &group.prs {
        let pr = view.of(work);
        hay.extend([format!("#{}", pr.number), pr.title.clone(), pr.head.clone(), pr.base.clone()]);
    }
    for item in group.prs.iter().flat_map(|pr| pr.items(work)).chain(group.items(work)) {
        hay.push(format!("{} {}", item.title, item.who.as_deref().unwrap_or("")));
    }
    for view in &group.sessions {
        let named = view.of(work).title.clone().or_else(|| view.workspace.as_ref().map(|w| w.name.clone()));
        hay.extend(named);
    }
    let hay = hay.join("\n").to_lowercase();
    query.split_whitespace().all(|word| hay.contains(word))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::work::sample;

    pub(crate) const NOW: i64 = enkidu_testkit::WORK_SAMPLE_NOW;
    const HOME: &str = enkidu_testkit::WORK_SAMPLE_HOME;

    /// The workspace of these tests: in project p1, with the session `s-<id>`.
    pub(crate) fn ws(id: &str, name: &str, branch: &str) -> Workspace {
        Workspace {
            id: id.into(),
            project_id: "p1".into(),
            name: name.into(),
            path: format!("{HOME}/enkidu/workspaces/shop/{name}"),
            branch: branch.into(),
            base: Some("development".into()),
            session_id: format!("s-{id}"),
            transcript_path: None,
            claude_config_dir: None,
            created_at: 1,
            adopted: false,
        }
    }

    pub(crate) fn workspaces() -> Vec<Workspace> {
        vec![ws("w1", "stock-fix", "fix/stock-double-count"), ws("w2", "sh-21309-export", "feature/SH-21309-export")]
    }

    pub(crate) fn statuses(of: &[(&str, &str)]) -> HashMap<String, String> {
        of.iter().map(|(id, status)| (id.to_string(), status.to_string())).collect()
    }

    fn groups(work: &Snapshot) -> Vec<Group> {
        build_groups(work, &workspaces(), &statuses(&[("w1", "working"), ("w2", "idle")]), NOW)
    }

    fn by_key<'a>(groups: &'a [Group], key: &str) -> &'a Group {
        groups.iter().find(|g| g.key.as_deref() == Some(key)).unwrap_or_else(|| panic!("no card for {key}"))
    }

    fn kinds<'a>(items: impl Iterator<Item = &'a Item>) -> Vec<String> {
        items.map(|item| format!("{}{}", item.kind, if item.bot { " (bot)" } else { "" })).collect()
    }

    /// Makes one group per ticket, with its PRs, items and sessions, worst first.
    #[test]
    fn makes_one_group_per_ticket_with_its_prs_items_and_sessions_worst_first() {
        let work = sample();
        let groups = groups(&work);
        let order: Vec<String> = groups.iter().map(|g| format!("{} {}", g.name(), g.lamp.name())).collect();
        assert_eq!(
            order,
            [
                "SH-21309 red",                 // checks failing
                "SH-21533 red",                 // changes requested
                "SH-21616 red",                 // back to you
                "SH-21112 blue",                // QA replied
                "pr:demo-org/enkidu#212 amber", // checks running (another project, no ticket)
                // nothing waits: the latest activity first
                "pr:demo-org/shop#2415 off", // a PR without a ticket
                "SH-21522 off",
                "SH-21376 off",
                "SH-21580 off",
                "SH-21475 off",
                "SH-20728 off",
                "SH-20781 off",
                "SH-21205 off",
                "SH-20947 off",
                "SH-21030 green", // done
            ]
        );
        let other = groups.iter().find(|g| g.id == "pr:demo-org/enkidu#212").unwrap();
        assert_eq!(other.project_id.as_deref(), Some("p2"), "a PR's project");
        assert_eq!(
            by_key(&groups, "SH-20728").project_id.as_deref(),
            Some("p1"),
            "a ticket without a PR: where it was read"
        );
        let export = by_key(&groups, "SH-21309");
        assert_eq!(export.prs.iter().map(|pr| pr.of(&work).number).collect::<Vec<_>>(), [2414]);
        assert_eq!(kinds(export.prs[0].items(&work)), ["question", "remark", "pipeline_failed", "probably_fixed"]);
        assert_eq!(export.waiting, 4);
        let claudes: Vec<String> = (export.sessions.iter())
            .map(|s| format!("{} {}", s.workspace.as_ref().map_or("", |w| &w.name), s.status.as_deref().unwrap_or("")))
            .collect();
        assert_eq!(claudes, ["sh-21309-export idle"]);
        // one ticket, two PRs: development and a hotfix to master, both from the same session
        let stock = by_key(&groups, "SH-21533");
        let prs: Vec<String> =
            (stock.prs.iter().map(|pr| pr.of(&work))).map(|pr| format!("{}→{}", pr.number, pr.base)).collect();
        assert_eq!(prs, ["2413→master", "2395→development"]);
        assert_eq!(stock.sessions.len(), 1);
        let review = stock.prs.iter().find(|pr| pr.of(&work).number == 2395).unwrap();
        assert_eq!(kinds(review.items(&work)), ["changes_requested", "remark", "remark (bot)"]);
        assert_eq!(stock.waiting, 2, "the bot's remark isn't counted");
        // a Warp session found in a transcript
        assert_eq!(by_key(&groups, "SH-21376").sessions[0].workspace, None);
        assert_eq!(kinds(by_key(&groups, "SH-21112").items(&work)), ["attachment", "qa_replied"]);
    }

    /// Filters by keys, titles, branches and names.
    #[test]
    fn filters_by_keys_titles_branches_and_names() {
        let work = sample();
        let groups = groups(&work);
        let found = |query: &str| -> Vec<&str> {
            groups.iter().filter(|g| matches(g, &work, query)).map(|g| g.key.as_deref().unwrap_or("")).collect()
        };
        assert_eq!(found("stock"), ["SH-21533", "SH-21580"]);
        assert_eq!(found("#2375"), ["SH-21376"]);
        let mut jonas = found("jonas");
        jonas.sort();
        assert_eq!(jonas, ["SH-21112", "SH-21309", "SH-21616"]);
        assert_eq!(found("").len(), groups.len());
        // every word must be somewhere, in any case
        assert_eq!(found("  EXPORT   csv "), ["SH-21309"]);
        assert!(found("export nowhere-to-be-found").is_empty());
    }

    /// Marks a broken source's items stale.
    #[test]
    fn marks_a_broken_sources_items_stale() {
        let mut work = sample();
        for health in work.health.iter_mut().filter(|h| h.source == "jira") {
            health.state = "error".into();
            health.error = Some("Jira: twg isn't signed in".into());
        }
        let groups = groups(&work);
        assert!(by_key(&groups, "SH-21112").stale);
        assert!(!by_key(&groups, "SH-21376").stale);
    }

    /// A PR that names no ticket is a group of its own, and a snoozed item sleeps until its time.
    #[test]
    fn a_pr_that_names_no_ticket_is_a_group_of_its_own_and_a_snoozed_item_sleeps_until_its_time() {
        let mut work = sample();
        let mut lonely = work.prs[0].clone();
        lonely.number = 2177;
        lonely.title = "Pictures in the test data".into();
        lonely.head = "feature/setup-pictures".into();
        lonely.url = "u".into();
        work.prs.push(lonely);
        let thread = work.items.iter_mut().find(|item| item.id == "thread:t1").expect("the sample's thread:t1");
        thread.state = "snoozed".into();
        thread.snoozed_until = Some(NOW + 60_000);
        let all = workspaces();
        let none = HashMap::new();
        let groups = build_groups(&work, &all, &none, NOW);
        assert!(groups.iter().any(|g| g.id == "pr:demo-org/shop#2177" && g.key.is_none()));
        assert_eq!(by_key(&groups, "SH-21309").waiting, 3);
        let later = build_groups(&work, &all, &none, NOW + 120_000);
        assert_eq!(by_key(&later, "SH-21309").waiting, 4);
    }

    /// Ranks the links: named in a prompt says least, you saying so most.
    #[test]
    fn ranks_the_links_named_in_a_prompt_says_least_you_saying_so_most() {
        let ranked = ["named it in a prompt", "worked on it", "worked on its branch", "created it", "linked by you"];
        let strengths: Vec<u8> = ranked.iter().map(|how| link_strength(how)).collect();
        assert_eq!(strengths, [1, 2, 3, 4, 5]);
    }

    /// A Jira board you picked keeps only its tickets; a PR that names no ticket is on no board; a board that wasn't
    /// read yet shows none.
    #[test]
    fn a_picked_jira_board_keeps_only_its_tickets() {
        let work = sample();
        let all = groups(&work);
        assert_eq!(board_cards(all.clone(), None).len(), all.len());
        let board = work.boards[0].clone();
        let pick = |keys: Option<Vec<&str>>| BoardPick {
            board: board.clone(),
            keys: keys.map(|keys| keys.into_iter().map(String::from).collect()),
            read_at: Some(NOW),
        };
        let on = board_cards(all.clone(), Some(&pick(Some(vec!["SH-21309", "SH-21030", "SH-1"]))));
        assert_eq!(on.iter().map(Group::name).collect::<Vec<_>>(), ["SH-21309", "SH-21030"]);
        assert!(board_cards(all, Some(&pick(None))).is_empty(), "not read yet: none");
    }

    /// Building the groups of an index much bigger than a usual one (600 tickets, 600 PRs, 3,000 items, 2,400 links)
    /// stays
    /// far under a frame: the page builds them whenever the index or a lamp changes.
    #[test]
    fn a_big_index_is_grouped_in_a_few_milliseconds() {
        let base = sample();
        let mut work = Snapshot::default();
        for n in 0..40 {
            let key = |old: &str| old.replace("SH-", &format!("X{n}-"));
            let reference = |old: &str| old.replace("demo-org/", &format!("org{n}/"));
            work.tickets.extend(base.tickets.iter().cloned().map(|t| Ticket { key: key(&t.key), ..t }));
            work.prs.extend(base.prs.iter().cloned().map(|p| Pr { repo: reference(&p.repo), ..p }));
            work.sessions.extend(base.sessions.iter().cloned().map(|s| Session { id: format!("{}-{n}", s.id), ..s }));
            for round in 0..8 {
                work.items.extend(base.items.iter().cloned().map(|i| Item {
                    id: format!("{}-{n}-{round}", i.id),
                    pr: i.pr.as_deref().map(reference),
                    ticket: i.ticket.as_deref().map(key),
                    ..i
                }));
            }
            for _ in 0..4 {
                work.links.extend(base.links.iter().cloned().map(|mut l| {
                    l.a = reference(&key(&l.a));
                    l.b = if l.b.starts_with("session:") { format!("{}-{n}", l.b) } else { reference(&key(&l.b)) };
                    l
                }));
            }
        }
        assert!(work.tickets.len() >= 500 && work.prs.len() >= 350 && work.items.len() >= 3000, "a big index");
        let all = workspaces();
        let none = HashMap::new();
        // the best of five, so a busy Mac doesn't fail it
        let took = (0..5)
            .map(|_| {
                let started = std::time::Instant::now();
                let groups = build_groups(&work, &all, &none, NOW);
                assert!(groups.len() >= 560);
                started.elapsed()
            })
            .min()
            .unwrap();
        println!(
            "WORK groups of {} tickets, {} PRs, {} items: {took:?}",
            work.tickets.len(),
            work.prs.len(),
            work.items.len()
        );
        let budget = if cfg!(debug_assertions) { 120 } else { 16 };
        assert!(took.as_millis() < budget, "it took {took:?}");
    }
}
