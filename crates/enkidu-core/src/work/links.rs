//! The link rules: from what the hooks, the transcript index, the workspaces and the watchers saw, which tickets,
//! PRs, sessions and workspaces belong together. Run after anything changes; links are only ever added here (a link
//! someone removed stays removed, see `Db::work_link`).

use anyhow::Result;
use std::collections::{HashMap, HashSet};

use super::model::Pr;
use super::parse;
use crate::db::{Db, Workspace};

/// What the rules need to know about a project.
#[derive(Clone, Debug, Default)]
pub struct ProjectInfo {
    pub id: String,
    /// The main folder
    pub path: String,
    /// Where its workspaces live (`~/enkidu/workspaces/<project>`), so a session in a removed workspace
    /// still counts
    pub workspaces_dir: String,
    /// `owner/name` on GitHub, when known
    pub repo: Option<String>,
    /// The branch new work starts from (`development`)
    pub base: String,
    /// Jira project keys whose tickets belong here (`SH`)
    pub jira_keys: Vec<String>,
}

impl ProjectInfo {
    /// Is this folder the project's, or one of its workspaces?
    pub fn holds(&self, cwd: &str, workspaces: &[Workspace]) -> bool {
        let under = |root: &str| !root.is_empty() && (cwd == root || cwd.starts_with(&format!("{}/", root.trim_end_matches('/'))));
        under(&self.path)
            || under(&self.workspaces_dir)
            || workspaces.iter().any(|w| w.project_id == self.id && under(&w.path))
    }
}

/// Words for a link's detail.
fn branch_words(detail: &str) -> &'static str {
    match detail {
        "pushed" => "pushed its branch",
        "committed" => "committed to its branch",
        _ => "worked on its branch",
    }
}

fn mention_words(detail: &str) -> &'static str {
    match detail {
        "created" => "created it",
        "named" => "named in a prompt",
        _ => "worked on it",
    }
}

/// Runs every rule; returns how many links are new. `repo_of` tells the GitHub repo of a folder outside the
/// project's known ones (another clone or worktree of it, like a folder another tool made), from its `origin`.
pub fn relink(db: &Db, projects: &[ProjectInfo], now: i64, repo_of: &dyn Fn(&str) -> Option<String>) -> Result<usize> {
    let prs: Vec<Pr> = db.work_prs()?.into_iter().map(|(p, _)| p).collect();
    let workspaces = db.workspaces()?;
    let sessions: HashMap<String, String> = db.work_sessions()?.into_iter().map(|s| (s.id, s.cwd)).collect();
    let by_id: HashMap<&str, &ProjectInfo> = projects.iter().map(|p| (p.id.as_str(), p)).collect();
    let mut by_head: HashMap<&str, Vec<&Pr>> = HashMap::new();
    for pr in &prs {
        by_head.entry(pr.head.as_str()).or_default().push(pr);
    }
    let known_repos: HashSet<String> = projects.iter().filter_map(|p| p.repo.clone()).chain(prs.iter().map(|p| p.repo.clone())).collect();
    let mut known_keys: HashSet<String> = projects.iter().flat_map(|p| p.jira_keys.iter().cloned()).collect();
    known_keys.extend(db.work_tickets()?.into_iter().map(|(t, _)| parse::key_project(&t.key).to_string()));
    let belongs = |cwd: &str, p: &ProjectInfo| {
        p.holds(cwd, &workspaces) || p.repo.as_ref().is_some_and(|r| repo_of(cwd).is_some_and(|x| x.eq_ignore_ascii_case(r)))
    };
    let project_of_cwd = |cwd: &str| {
        projects.iter().find(|p| p.holds(cwd, &workspaces)).or_else(|| projects.iter().find(|p| belongs(cwd, p)))
    };
    let mut added = 0;
    let mut link = |a: &str, b: &str, how: &str, detail: &str, at: i64| -> Result<()> {
        if db.work_link(a, b, how, detail, at)? {
            added += 1;
        }
        Ok(())
    };

    // a workspace on a PR's head branch
    for ws in &workspaces {
        for pr in by_head.get(ws.branch.as_str()).into_iter().flatten() {
            let base = by_id.get(pr.project_id.as_str()).map(|p| p.base.as_str()).unwrap_or("");
            if pr.project_id == ws.project_id && !parse::long_lived(&ws.branch, base) {
                link(&format!("pr:{}", pr.reference()), &format!("workspace:{}", ws.id), "branch", "its workspace", ws.created_at)?;
            }
        }
    }

    // a session that worked on, pushed or committed to a PR's head branch, in that project's folders
    for (session, branch, how, detail, at) in db.work_branch_rows()? {
        let Some(cwd) = sessions.get(&session) else { continue };
        for pr in by_head.get(branch.as_str()).into_iter().flatten() {
            let Some(project) = by_id.get(pr.project_id.as_str()) else { continue };
            if parse::long_lived(&branch, &project.base) || !belongs(cwd, project) {
                continue;
            }
            let how = if how == "transcript" { "transcript" } else { "branch" };
            link(&format!("pr:{}", pr.reference()), &format!("session:{session}"), how, branch_words(&detail), at)?;
        }
    }

    // a session that made a commit the PR has
    for (sha, session, how, at) in db.work_commit_rows()? {
        for pr in prs.iter().filter(|p| p.commits.iter().any(|c| c.starts_with(&sha))) {
            let how = if how == "transcript" { "transcript" } else { "hook:commit" };
            link(&format!("pr:{}", pr.reference()), &format!("session:{session}"), how, "made a commit in it", at)?;
        }
    }

    // a PR or ticket the session created, worked on or named
    for (session, target, how, detail, at) in db.work_mention_rows()? {
        let resolved = if let Some(number) = target.strip_prefix("pr:#") {
            // `gh pr ready 12`: the repo is the session's own
            let repo = sessions.get(&session).and_then(|cwd| project_of_cwd(cwd)).and_then(|p| p.repo.clone());
            match repo {
                Some(r) => format!("pr:{r}#{number}"),
                None => continue,
            }
        } else if let Some(r) = target.strip_prefix("pr:") {
            let repo = r.split('#').next().unwrap_or("");
            if !known_repos.iter().any(|k| k.eq_ignore_ascii_case(repo)) {
                continue;
            }
            // made or changed with gh (checked against the command that ran it), or pasted: from any folder
            target.clone()
        } else if let Some(key) = target.strip_prefix("ticket:") {
            if !known_keys.contains(parse::key_project(key)) {
                continue;
            }
            target.clone()
        } else {
            continue;
        };
        link(&resolved, &format!("session:{session}"), &how, mention_words(&detail), at)?;
    }

    // a ticket whose key is in the PR's branch, title or commit messages: the same rule Jira's GitHub integration
    // links by (twg can't read Jira's stored links back as PRs, so they are worked out from GitHub's side)
    for pr in &prs {
        let mut keys = parse::ticket_keys(&pr.head);
        for k in parse::ticket_keys(&pr.title).into_iter().chain(pr.commit_keys.iter().cloned()) {
            if !keys.contains(&k) {
                keys.push(k);
            }
        }
        for key in keys {
            let words = if pr.head.contains(&key) {
                "the key is in its branch"
            } else if pr.title.contains(&key) {
                "the key is in its title"
            } else {
                "the key is in a commit"
            };
            link(&format!("ticket:{key}"), &format!("pr:{}", pr.reference()), "ticket-key", words, now)?;
        }
    }
    Ok(added)
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::db::{now_ms, Project};
    use crate::work::store::tests::db;
    use crate::work::store::SessionSeen;

    pub fn pr(number: i64, head: &str, title: &str) -> Pr {
        Pr {
            project_id: "p1".into(),
            repo: "Org/shop".into(),
            number,
            title: title.into(),
            url: format!("https://github.com/Org/shop/pull/{number}"),
            state: "OPEN".into(),
            draft: false,
            head: head.into(),
            base: "development".into(),
            review_decision: None,
            checks: None,
            failing: Vec::new(),
            running: 0,
            mergeable: None,
            head_sha: None,
            head_at: None,
            unresolved: 0,
            updated_at: "2026-11-05T10:00:00Z".into(),
            merged_at: None,
            commits: Vec::new(),
            commit_keys: Vec::new(),
            last_comment_by: None,
            last_comment_at: None,
            details: Default::default(),
        }
    }

    pub fn shop() -> ProjectInfo {
        ProjectInfo {
            id: "p1".into(),
            path: "/code/shop".into(),
            workspaces_dir: "/ws/shop".into(),
            repo: Some("Org/shop".into()),
            base: "development".into(),
            jira_keys: vec!["SH".into()],
        }
    }

    pub fn session(db: &Db, id: &str, cwd: &str, source: &str) {
        db.work_session_seen(&SessionSeen {
            id: id.into(),
            config_dir: "/h/.claude".into(),
            cwd: cwd.into(),
            source: source.into(),
            first_at: 1,
            last_at: 2,
            ..Default::default()
        })
        .unwrap();
    }

    fn add_project_and_workspace(db: &Db, branch: &str) {
        db.insert_project(&Project { id: "p1".into(), name: "shop".into(), path: "/code/shop".into(), live_workspace_id: None, created_at: 1 })
            .unwrap();
        db.insert_workspace(&Workspace {
            id: "w1".into(),
            project_id: "p1".into(),
            name: "totals".into(),
            path: "/ws/shop/totals".into(),
            branch: branch.into(),
            base: Some("development".into()),
            session_id: "s-ws".into(),
            transcript_path: None,
            claude_config_dir: None,
            adopted: false,
            created_at: 5,
        })
        .unwrap();
    }

    fn pairs(db: &Db) -> Vec<String> {
        let mut v: Vec<String> = db.work_links().unwrap().into_iter().map(|l| format!("{} {} {}", l.a, l.b, l.how)).collect();
        v.sort();
        v
    }

    #[test]
    fn a_workspace_and_a_warp_session_on_a_prs_branch_are_both_its_sessions() {
        let (_d, db) = db();
        add_project_and_workspace(&db, "feature/SH-21533-totals");
        db.work_put_prs("p1", &[pr(2395, "feature/SH-21533-totals", "SH-21533 Fix totals")], now_ms()).unwrap();
        // the workspace's own Claude, known live
        session(&db, "s-ws", "/ws/shop/totals", "workspace");
        db.work_session_branch("s-ws", "feature/SH-21533-totals", "branch", "worked", 10).unwrap();
        // a Claude started in Warp in the main folder, found in its saved transcript
        session(&db, "s-warp", "/code/shop", "transcript");
        db.work_session_branch("s-warp", "feature/SH-21533-totals", "transcript", "worked", 20).unwrap();
        // same branch name, another repo: not this PR's
        session(&db, "s-other", "/code/other", "transcript");
        db.work_session_branch("s-other", "feature/SH-21533-totals", "transcript", "worked", 30).unwrap();
        relink(&db, &[shop()], now_ms(), &|_| None).unwrap();
        assert_eq!(
            pairs(&db),
            [
                "pr:Org/shop#2395 session:s-warp transcript",
                "pr:Org/shop#2395 session:s-ws branch",
                "pr:Org/shop#2395 workspace:w1 branch",
                "ticket:SH-21533 pr:Org/shop#2395 ticket-key",
            ]
        );
        assert_eq!(relink(&db, &[shop()], now_ms(), &|_| None).unwrap(), 0, "running the rules again adds nothing");
        // what the Work page gets: both sessions, as the PR's; the other repo's session isn't sent
        let snap = crate::work::snapshot_of(&db).unwrap();
        let mut ids: Vec<&str> = snap.sessions.iter().map(|s| s.id.as_str()).collect();
        ids.sort();
        assert_eq!(ids, ["s-warp", "s-ws"]);
        let of_pr: Vec<&str> = snap.links.iter().filter(|l| l.a == "pr:Org/shop#2395" && l.b.starts_with("session:")).map(|l| l.b.as_str()).collect();
        assert_eq!(of_pr.len(), 2);
    }

    #[test]
    fn one_session_on_two_branches_links_two_prs_and_one_pr_has_two_sessions() {
        let (_d, db) = db();
        db.work_put_prs("p1", &[pr(1, "feature/SH-1-a", "A"), pr(2, "feature/SH-2-b", "B")], now_ms()).unwrap();
        session(&db, "s1", "/code/shop", "hook");
        db.work_session_branch("s1", "feature/SH-1-a", "branch", "worked", 10).unwrap();
        db.work_session_branch("s1", "feature/SH-2-b", "branch", "worked", 20).unwrap();
        session(&db, "s2", "/ws/shop/b", "transcript");
        db.work_session_branch("s2", "feature/SH-2-b", "transcript", "pushed", 30).unwrap();
        relink(&db, &[shop()], now_ms(), &|_| None).unwrap();
        let links = pairs(&db);
        assert!(links.contains(&"pr:Org/shop#1 session:s1 branch".to_string()));
        assert!(links.contains(&"pr:Org/shop#2 session:s1 branch".to_string()));
        assert!(links.contains(&"pr:Org/shop#2 session:s2 transcript".to_string()), "a session in a removed workspace's folder still counts");
        let detail = db.work_links().unwrap().into_iter().find(|l| l.b == "session:s2").unwrap().detail;
        assert_eq!(detail, "pushed its branch");
    }

    #[test]
    fn another_clone_or_worktree_of_the_repo_counts_as_the_project() {
        let (_d, db) = db();
        db.work_put_prs("p1", &[pr(2329, "feature/SH-20844-cost", "SH-20844 Cost")], now_ms()).unwrap();
        session(&db, "s-old", "/old-tool/workspaces/shop/kabul", "transcript");
        db.work_mention("s-old", "pr:Org/shop#2329", "transcript", "created", 10).unwrap();
        session(&db, "s-gone", "/old-tool/workspaces/shop/gone", "transcript");
        db.work_session_branch("s-gone", "feature/SH-20844-cost", "transcript", "worked", 11).unwrap();
        let repo_of = |folder: &str| folder.ends_with("/kabul").then(|| "org/SHOP".to_string());
        relink(&db, &[shop()], now_ms(), &repo_of).unwrap();
        let links = pairs(&db);
        assert!(links.contains(&"pr:Org/shop#2329 session:s-old transcript".to_string()), "{links:?}");
        assert!(!links.iter().any(|l| l.contains("s-gone")), "a folder that is gone can't be told apart: no link");
    }

    #[test]
    fn one_ticket_with_two_prs_and_long_lived_branches_dont_link() {
        let (_d, db) = db();
        db.work_put_prs(
            "p1",
            &[pr(10, "feature/SH-7-x", "SH-7 To development"), pr(11, "hotfix/fix-x", "SH-7 Hotfix to master"), pr(12, "development", "Release")],
            now_ms(),
        )
        .unwrap();
        session(&db, "s1", "/code/shop", "transcript");
        db.work_session_branch("s1", "development", "transcript", "worked", 10).unwrap();
        relink(&db, &[shop()], now_ms(), &|_| None).unwrap();
        assert_eq!(pairs(&db), ["ticket:SH-7 pr:Org/shop#10 ticket-key", "ticket:SH-7 pr:Org/shop#11 ticket-key"]);
    }

    #[test]
    fn a_key_in_a_commit_message_links_like_jira_does() {
        let (_d, db) = db();
        let mut quiet = pr(13, "feature/setup-pictures", "Pictures in test data");
        quiet.commit_keys = vec!["SH-9".into()];
        db.work_put_prs("p1", &[quiet], now_ms()).unwrap();
        relink(&db, &[shop()], now_ms(), &|_| None).unwrap();
        let links = db.work_links().unwrap();
        assert_eq!((links[0].a.as_str(), links[0].detail.as_str()), ("ticket:SH-9", "the key is in a commit"));
    }

    #[test]
    fn created_named_and_committed_prs_link_their_sessions() {
        let (_d, db) = db();
        let mut with_commit = pr(20, "feature/c", "C");
        with_commit.commits = vec!["3f2c1ab9d0e1f2a3b4c5d6e7f8091a2b3c4d5e6f".into()];
        db.work_put_prs("p1", &[pr(2395, "hotfix/round-totals", "Round the totals"), with_commit, pr(30, "feature/d", "D")], now_ms())
            .unwrap();
        session(&db, "s1", "/code/shop", "hook");
        db.work_mention("s1", "pr:Org/shop#2395", "hook:pr-create", "created", 10).unwrap();
        db.work_mention("s1", "pr:#30", "hook:pr-create", "ready", 11).unwrap();
        db.work_mention("s1", "pr:Stranger/repo#5", "hook:mention", "named", 12).unwrap();
        db.work_mention("s1", "ticket:SH-21533", "hook:mention", "named", 13).unwrap();
        db.work_mention("s1", "ticket:UX-12", "hook:mention", "named", 14).unwrap();
        db.work_commit("3f2c1ab", "s1", Some("feature/c"), "hook", 15).unwrap();
        // `gh pr create --repo Org/shop` from a folder that is gone: the command itself was checked, so it counts
        session(&db, "s-far", "/old-tool/workspaces/gone", "transcript");
        db.work_mention("s-far", "pr:Org/shop#2395", "transcript", "created", 16).unwrap();
        relink(&db, &[shop()], now_ms(), &|_| None).unwrap();
        assert_eq!(
            pairs(&db),
            [
                "pr:Org/shop#20 session:s1 hook:commit",
                "pr:Org/shop#2395 session:s-far transcript",
                "pr:Org/shop#2395 session:s1 hook:pr-create",
                "pr:Org/shop#30 session:s1 hook:pr-create",
                "ticket:SH-21533 session:s1 hook:mention",
            ],
            "a repo or a Jira project that isn't yours doesn't link"
        );
    }
}
