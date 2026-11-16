//! What the frame shos: the projects, their workspaces and the lamps, the project on screen and the workspace you last
//! had in each (item 1.15). Plain data: the app's store applies the core's events to it, and every rule here is tested
//! without a window.

use std::collections::HashMap;

use enkidu_core::db::{Project, Workspace};
use enkidu_core::events::Event;
use serde::{Deserialize, Serialize};

/// What the window remembers of where you were.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct View {
    pub project_id: Option<String>,
    /// Each project's workspace you had on screen.
    pub selected: HashMap<String, String>,
    /// You were on the Work page.
    #[serde(default)]
    pub work: bool,
    /// Each project's workspaces in the order you gave their tabs (item 18.3). One you never placed stands after
    /// them, where the order they were made in puts it.
    #[serde(default)]
    pub order: HashMap<String, Vec<String>>,
    /// The workspaces you put away, each with when you did (ms): their tabs are gone until you bring them back.
    #[serde(default)]
    pub hidden: HashMap<String, i64>,
}

/// What a workspace has changed, as the core counted it: its files and their added and removed lines.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    pub files: usize,
    pub additions: u64,
    pub deletions: u64,
}

impl Changes {
    pub fn of(files: &[enkidu_core::git::Change]) -> Changes {
        Changes {
            files: files.len(),
            additions: files.iter().filter_map(|f| f.additions).sum(),
            deletions: files.iter().filter_map(|f| f.deletions).sum(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frame {
    pub projects: Vec<Project>,
    pub workspaces: Vec<Workspace>,
    /// Claude's state per workspace: `idle`, `working`, `needs_you`, `done`.
    pub statuses: HashMap<String, String>,
    pub current_project: Option<String>,
    pub selected: HashMap<String, String>,
    /// The Work page is on screen, in place of the workspace's tabs and islands (it spans the projects). Putting a
    /// project or a workspace on screen leaves it.
    pub work_open: bool,
    /// What each workspace has changed (the core's changes-updated news); nothing until it says.
    pub changed: HashMap<String, Changes>,
    /// How many remarks each workspace has that weren't sent to Claude yet (the remarks store keeps it up).
    pub remarks: HashMap<String, usize>,
    /// How many times each workspace's lamp changed state while shown: each new count pops its lamp once.
    pub pops: HashMap<String, u32>,
    /// What the database remembers has been read (until then the window shows nothing, not an empty screen).
    pub loaded: bool,
    /// Each project's workspaces in the order you gave their tabs; see [`View::order`].
    pub order: HashMap<String, Vec<String>>,
    /// The workspaces you put away, each with when; see [`View::hidden`].
    pub hidden: HashMap<String, i64>,
    /// Where each project's Live is going while it switches (item 4.5).
    pub live_switching: crate::live::Switching,
}

/// Every workspace: the projects in their order, each with its workspaces in the order they were made.
pub fn workspace_order<'a>(projects: &[Project], workspaces: &'a [Workspace]) -> Vec<&'a Workspace> {
    projects.iter().flat_map(|p| workspaces.iter().filter(move |w| w.project_id == p.id)).collect()
}

/// The workspaces of one project, in the order they were made. No project, none.
pub fn project_tabs<'a>(workspaces: &'a [Workspace], project_id: Option<&str>) -> Vec<&'a Workspace> {
    match project_id {
        Some(id) => workspaces.iter().filter(|w| w.project_id == id).collect(),
        None => Vec::new(),
    }
}

impl Frame {
    /// The frame from what the core knows at the start, with where you were last time: the project and each
    /// project's workspace you had, when they are still there; otherwise the first of each.
    pub fn load(
        projects: Vec<Project>,
        workspaces: Vec<Workspace>,
        statuses: HashMap<String, String>,
        remembered: &View,
    ) -> Frame {
        let current_project = projects
            .iter()
            .find(|p| remembered.project_id.as_deref() == Some(&p.id))
            .or_else(|| projects.first())
            .map(|p| p.id.clone());
        // your order and what you put away, as far as those workspaces are still there
        let there = |id: &String| workspaces.iter().any(|w| &w.id == id);
        let order = (remembered.order.iter())
            .filter(|(project, _)| projects.iter().any(|p| &p.id == *project))
            .map(|(project, ids)| (project.clone(), ids.iter().filter(|id| there(id)).cloned().collect()))
            .collect();
        let hidden = remembered.hidden.iter().filter(|(id, _)| there(id)).map(|(id, at)| (id.clone(), *at)).collect();
        // the Work page is there once there is a project
        let work_open = remembered.work && !projects.is_empty();
        let mut frame =
            Frame { projects, workspaces, statuses, current_project, work_open, order, hidden, ..Frame::default() };
        for project in frame.projects.iter().map(|p| p.id.clone()).collect::<Vec<_>>() {
            // the one you had, while its tab is still there; else the first tab
            let tabs = frame.tabs_of(&project);
            let want = remembered.selected.get(&project);
            let pick = tabs.iter().find(|w| Some(&w.id) == want).or_else(|| tabs.first()).map(|w| w.id.clone());
            if let Some(pick) = pick {
                frame.selected.insert(project, pick);
            }
        }
        frame.loaded = true;
        frame
    }

    /// What to remember of where you are.
    pub fn view(&self) -> View {
        View {
            project_id: self.current_project.clone(),
            selected: self.selected.clone(),
            work: self.work_open,
            order: self.order.clone(),
            hidden: self.hidden.clone(),
        }
    }

    /// ⌘I, or the Work tab: the Work page comes on screen, or the workspace is back. False when there is no project
    /// (the page is there once there is one).
    pub fn toggle_work(&mut self) -> bool {
        if self.projects.is_empty() {
            return false;
        }
        self.work_open = !self.work_open;
        true
    }

    /// All of a project's workspaces in your order, the ones you put away too: the ones you placed first, as you
    /// placed them, then the others in the order they were made.
    pub fn ordered(&self, project_id: &str) -> Vec<&Workspace> {
        let mut all = project_tabs(&self.workspaces, Some(project_id));
        if let Some(order) = self.order.get(project_id) {
            // a stable sort: what you never placed keeps its order, after what you placed
            all.sort_by_key(|w| order.iter().position(|id| id == &w.id).unwrap_or(usize::MAX));
        }
        all
    }

    /// Every workspace: the projects in their order, each with its workspaces in yours.
    pub fn all_ordered(&self) -> Vec<&Workspace> {
        self.projects.iter().flat_map(|p| self.ordered(&p.id)).collect()
    }

    /// This workspace is its project's main folder itself (item 19.2): nothing was made for it, and it can't go
    /// Live, since Live copies a workspace's changes into that folder. Both paths are git's own word for a top
    /// level, so they are compared as they are written.
    pub fn is_main_folder(&self, workspace: &Workspace) -> bool {
        self.projects.iter().any(|p| p.id == workspace.project_id && p.path == workspace.path)
    }

    /// You put this workspace away.
    pub fn is_hidden(&self, workspace_id: &str) -> bool {
        self.hidden.contains_key(workspace_id)
    }

    /// A project's tabs: its workspaces that are not put away, in your order.
    pub fn tabs_of(&self, project_id: &str) -> Vec<&Workspace> {
        self.ordered(project_id).into_iter().filter(|w| !self.is_hidden(&w.id)).collect()
    }

    /// The workspace tabs of the project on screen.
    pub fn tabs(&self) -> Vec<&Workspace> {
        self.current_project.as_deref().map(|project| self.tabs_of(project)).unwrap_or_default()
    }

    /// The workspaces of the project on screen that you put away, in your order.
    pub fn hidden_tabs(&self) -> Vec<&Workspace> {
        let Some(project) = self.current_project.as_deref() else { return Vec::new() };
        self.ordered(project).into_iter().filter(|w| self.is_hidden(&w.id)).collect()
    }

    /// A tab is dragged to the place of the tab at `to` (its place among the project's tabs): it stands there, and
    /// the ones between move over by one. The workspaces you put away keep their places among the others. False
    /// when nothing moved.
    pub fn move_tab(&mut self, id: &str, to: usize) -> bool {
        let Some(project) = self.workspaces.iter().find(|w| w.id == id).map(|w| w.project_id.clone()) else {
            return false;
        };
        let all: Vec<String> = self.ordered(&project).iter().map(|w| w.id.clone()).collect();
        let mut tabs: Vec<String> = all.iter().filter(|id| !self.is_hidden(id)).cloned().collect();
        let Some(from) = tabs.iter().position(|tab| tab == id) else { return false };
        let to = to.min(tabs.len() - 1);
        if from == to {
            return false;
        }
        let tab = tabs.remove(from);
        tabs.insert(to, tab);
        // the places of the tabs take the tabs in their new order; a hidden workspace stays in its own place
        let mut tabs = tabs.into_iter();
        let order =
            all.into_iter().map(|id| if self.is_hidden(&id) { id } else { tabs.next().unwrap_or(id) }).collect();
        self.order.insert(project, order);
        true
    }

    /// A workspace is put away: its tab goes, and when it was on screen the tab that takes its place shows (the one
    /// before it, when it was the last; none, when it was the only one). Nothing else of it changes. False when
    /// there is no such workspace or it is put away already.
    pub fn hide_workspace(&mut self, id: &str, now: i64) -> bool {
        let Some(project) = self.workspaces.iter().find(|w| w.id == id).map(|w| w.project_id.clone()) else {
            return false;
        };
        if self.is_hidden(id) {
            return false;
        }
        let tabs: Vec<String> = self.tabs_of(&project).iter().map(|w| w.id.clone()).collect();
        let at = tabs.iter().position(|tab| tab == id).unwrap_or(0);
        self.hidden.insert(id.to_string(), now);
        if self.selected.get(&project).map(String::as_str) == Some(id) {
            let rest: Vec<&String> = tabs.iter().filter(|tab| *tab != id).collect();
            match rest.get(at.min(rest.len().saturating_sub(1))) {
                Some(next) => self.selected.insert(project, (*next).clone()),
                None => self.selected.remove(&project),
            };
        }
        true
    }

    /// A workspace you put away is back: its tab stands where it stood in your order, and it comes on screen.
    /// False when there is no such workspace.
    pub fn show_workspace(&mut self, id: &str) -> bool {
        if !self.workspaces.iter().any(|w| w.id == id) {
            return false;
        }
        let was_hidden = self.hidden.remove(id).is_some();
        self.select_workspace(id) || was_hidden
    }

    /// The workspace on screen: the one you had in the project on screen.
    pub fn current_workspace(&self) -> Option<&Workspace> {
        let project = self.current_project.as_ref()?;
        let id = self.selected.get(project)?;
        self.workspaces.iter().find(|w| &w.id == id)
    }

    /// Claude's state in a workspace; idle until the core says otherwise.
    pub fn status(&self, workspace_id: &str) -> &str {
        self.statuses.get(workspace_id).map_or("idle", String::as_str)
    }

    /// Puts a project on screen, and leaves the Work page. False when there is no such project or it already is.
    pub fn select_project(&mut self, id: &str) -> bool {
        if !self.projects.iter().any(|p| p.id == id) {
            return false;
        }
        let left = std::mem::take(&mut self.work_open);
        if self.current_project.as_deref() == Some(id) {
            return left;
        }
        self.current_project = Some(id.to_string());
        true
    }

    /// Puts a workspace on screen, with its project, and leaves the Work page. False when there is no such workspace
    /// or it already is.
    pub fn select_workspace(&mut self, id: &str) -> bool {
        let Some(ws) = self.workspaces.iter().find(|w| w.id == id) else { return false };
        let project = ws.project_id.clone();
        let left = std::mem::take(&mut self.work_open);
        // going to a workspace you put away brings it back: what is on screen has its tab
        let back = self.hidden.remove(id).is_some();
        if self.current_project.as_deref() == Some(&project)
            && self.selected.get(&project).map(String::as_str) == Some(id)
        {
            return left || back;
        }
        self.selected.insert(project.clone(), id.to_string());
        self.current_project = Some(project);
        true
    }

    /// The next (`step` 1) or previous (-1) tab of the project on screen, wrapping around.
    pub fn cycle_workspace(&mut self, step: i32) -> bool {
        let tabs: Vec<String> = self.tabs().iter().map(|w| w.id.clone()).collect();
        if tabs.is_empty() {
            return false;
        }
        let at = self.current_workspace().and_then(|w| tabs.iter().position(|id| id == &w.id)).unwrap_or(0) as i32;
        let next = (at + step).rem_euclid(tabs.len() as i32) as usize;
        self.select_workspace(&tabs[next])
    }

    /// The tab at `index` (0 for ⌘1) of the project on screen, if there is one.
    pub fn select_tab(&mut self, index: usize) -> bool {
        let Some(id) = self.tabs().get(index).map(|w| w.id.clone()) else { return false };
        self.select_workspace(&id)
    }

    /// A workspace is gone: its tab and lamp go, and when it was on screen, the next tab of its project shows (the
    /// one before, when it was the last).
    pub fn remove_workspace(&mut self, id: &str) {
        let Some(ws) = self.workspaces.iter().find(|w| w.id == id).cloned() else { return };
        let tabs: Vec<String> = self.tabs_of(&ws.project_id).iter().map(|w| w.id.clone()).collect();
        let at = tabs.iter().position(|t| t == id).unwrap_or(0);
        self.workspaces.retain(|w| w.id != id);
        self.statuses.remove(id);
        self.changed.remove(id);
        self.remarks.remove(id);
        self.pops.remove(id);
        self.hidden.remove(id);
        if let Some(order) = self.order.get_mut(&ws.project_id) {
            order.retain(|placed| placed != id);
        }
        if self.selected.get(&ws.project_id).map(String::as_str) == Some(id) {
            let rest: Vec<&String> = tabs.iter().filter(|t| *t != id).collect();
            match rest.get(at.min(rest.len().saturating_sub(1))) {
                Some(next) => {
                    self.selected.insert(ws.project_id.clone(), (*next).clone());
                }
                None => {
                    self.selected.remove(&ws.project_id);
                }
            }
        }
    }

    /// A workspace was made here (or a folder opened as one): its tab joins its project's, and it comes on screen.
    pub fn add_workspace(&mut self, workspace: Workspace) {
        let id = workspace.id.clone();
        if !self.workspaces.iter().any(|w| w.id == id) {
            self.workspaces.push(workspace);
        }
        self.select_workspace(&id);
    }

    /// A project was added: it joins the row at the end and goes on screen.
    pub fn add_project(&mut self, project: Project) {
        let id = project.id.clone();
        if !self.projects.iter().any(|p| p.id == id) {
            self.projects.push(project);
        }
        self.current_project = Some(id);
        self.work_open = false;
    }

    /// A project was removed (it had no workspaces left): the first one left goes on screen.
    pub fn remove_project(&mut self, id: &str) {
        self.projects.retain(|p| p.id != id);
        self.selected.remove(id);
        self.order.remove(id);
        if self.current_project.as_deref() == Some(id) {
            self.current_project = self.projects.first().map(|p| p.id.clone());
        }
        // the Work page goes with the last project
        self.work_open &= !self.projects.is_empty();
    }

    /// How many files a workspace has changed.
    pub fn changed_files(&self, workspace_id: &str) -> usize {
        self.changed.get(workspace_id).map_or(0, |c| c.files)
    }

    /// How many remarks a workspace has that weren't sent yet.
    pub fn remarks(&self, workspace_id: &str) -> usize {
        self.remarks.get(workspace_id).copied().unwrap_or(0)
    }

    /// The remarks store says how many a workspace has now. False when that is what was known.
    pub fn set_remarks(&mut self, workspace_id: &str, count: usize) -> bool {
        if self.remarks(workspace_id) == count {
            return false;
        }
        match count {
            0 => self.remarks.remove(workspace_id),
            _ => self.remarks.insert(workspace_id.to_string(), count),
        };
        true
    }

    /// What a workspace has changed, once the core has said.
    pub fn changes(&self, workspace_id: &str) -> Option<Changes> {
        self.changed.get(workspace_id).copied()
    }

    /// How many times a workspace's lamp changed state while shown (its lamp pops at each new count).
    pub fn pops(&self, workspace_id: &str) -> u32 {
        self.pops.get(workspace_id).copied().unwrap_or(0)
    }

    /// Whether a workspace is its project's Live one.
    pub fn is_live(&self, workspace: &Workspace) -> bool {
        self.projects
            .iter()
            .any(|p| p.id == workspace.project_id && p.live_workspace_id.as_deref() == Some(workspace.id.as_str()))
    }

    /// What a workspace's Live button shows: off, Live, or which way its project's Live is switching.
    pub fn live_state(&self, workspace: &Workspace) -> crate::live::LiveState {
        let live =
            self.projects.iter().find(|p| p.id == workspace.project_id).and_then(|p| p.live_workspace_id.as_deref());
        crate::live::LiveState::of(&workspace.id, live, self.live_switching.of(&workspace.project_id))
    }

    /// The name of the project's other workspace that is Live now: its changes go back first when this one goes Live.
    pub fn live_elsewhere(&self, workspace: &Workspace) -> Option<&str> {
        let live = self.projects.iter().find(|p| p.id == workspace.project_id)?.live_workspace_id.as_deref()?;
        (live != workspace.id).then(|| self.workspaces.iter().find(|w| w.id == live).map(|w| w.name.as_str())).flatten()
    }

    /// How many of a project's workspaces wait for you (the red count on its tab when it isn't on screen). One you
    /// put away is quiet: it is not counted.
    pub fn waiting(&self, project_id: &str) -> usize {
        let waits = |w: &&Workspace| w.project_id == project_id && self.status(&w.id) == "needs_you";
        self.workspaces.iter().filter(waits).filter(|w| !self.is_hidden(&w.id)).count()
    }

    /// The core's news, applied. True when the frame changed.
    pub fn apply(&mut self, event: &Event) -> bool {
        match event {
            Event::StatusChanged { workspace_id, status } => {
                let before = self.statuses.insert(workspace_id.clone(), status.clone());
                let changed = before.as_deref().unwrap_or("idle") != status.as_str();
                if changed {
                    *self.pops.entry(workspace_id.clone()).or_default() += 1;
                }
                changed || before.is_none()
            }
            Event::ChangesUpdated { workspace_id, files } => {
                let changes = Changes::of(files);
                self.changed.insert(workspace_id.clone(), changes) != Some(changes)
            }
            // a workspace's folder moved to another branch (Claude or you checked one out in it)
            Event::WorkspaceUpdated(ws) => match self.workspaces.iter_mut().find(|w| w.id == ws.id) {
                Some(old) if old != ws => {
                    *old = ws.clone();
                    true
                }
                _ => false,
            },
            // a workflow made a workspace for its task: a new tab, not put on screen
            Event::WorkspaceAdded(ws) => {
                if self.workspaces.iter().any(|w| w.id == ws.id) {
                    return false;
                }
                self.workspaces.push(ws.clone());
                if !self.selected.contains_key(&ws.project_id) {
                    self.selected.insert(ws.project_id.clone(), ws.id.clone());
                }
                true
            }
            Event::LiveChanged(outcome) => match self.projects.iter_mut().find(|p| p.id == outcome.project_id) {
                Some(p) if p.live_workspace_id != outcome.live_workspace_id => {
                    p.live_workspace_id = outcome.live_workspace_id.clone();
                    true
                }
                _ => false,
            },
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use enkidu_core::live::LiveOutcome;

    fn project(id: &str, name: &str) -> Project {
        Project {
            id: id.into(),
            name: name.into(),
            path: format!("/code/{name}"),
            live_workspace_id: None,
            created_at: 1,
        }
    }

    fn ws(id: &str, name: &str) -> Workspace {
        Workspace {
            id: id.into(),
            project_id: "p1".into(),
            name: name.into(),
            path: format!("/ws/{name}"),
            branch: format!("feature/{name}"),
            base: Some("development".into()),
            session_id: format!("s{id}"),
            transcript_path: None,
            claude_config_dir: None,
            created_at: 1,
            adopted: false,
        }
    }

    fn of(project_id: &str, w: Workspace) -> Workspace {
        Workspace { project_id: project_id.into(), ..w }
    }

    fn loaded(workspaces: Vec<Workspace>) -> Frame {
        let statuses = HashMap::from([("w1".to_string(), "working".to_string())]);
        Frame::load(vec![project("p1", "shop")], workspaces, statuses, &View::default())
    }

    fn name(frame: &Frame) -> Option<&str> {
        frame.current_workspace().map(|w| w.name.as_str())
    }

    #[test]
    fn loads_state_and_picks_the_first_workspace() {
        let frame = loaded(vec![ws("w1", "one"), ws("w2", "two")]);
        assert_eq!(frame.current_project.as_deref(), Some("p1"));
        assert_eq!(name(&frame), Some("one"));
        assert_eq!(frame.status("w1"), "working");
        assert_eq!(frame.status("w2"), "idle");
    }

    #[test]
    fn remembers_the_selected_workspace_across_loads() {
        let mut frame = loaded(vec![ws("w1", "one"), ws("w2", "two")]);
        assert!(frame.select_workspace("w2"));
        let view = frame.view();
        let again = Frame::load(frame.projects.clone(), frame.workspaces.clone(), HashMap::new(), &view);
        assert_eq!(name(&again), Some("two"));
        // what was remembered and is gone falls back to the first
        let gone = View {
            project_id: Some("p9".into()),
            selected: HashMap::from([("p1".into(), "w9".into())]),
            ..View::default()
        };
        let fresh = Frame::load(frame.projects.clone(), frame.workspaces.clone(), HashMap::new(), &gone);
        assert_eq!((fresh.current_project.as_deref(), name(&fresh)), (Some("p1"), Some("one")));
    }

    fn tabs(frame: &Frame) -> Vec<&str> {
        frame.tabs().iter().map(|w| w.name.as_str()).collect()
    }

    /// Item 18.3: a tab is
    /// dragged to another's place; the keys, the palette's order and a restart follow.
    #[test]
    fn a_tab_moves_to_anothers_place_and_your_order_is_kept() {
        let four = vec![ws("w1", "one"), ws("w2", "two"), ws("w3", "three"), ws("w4", "four")];
        let mut frame = loaded(four.clone());
        assert_eq!(tabs(&frame), ["one", "two", "three", "four"], "as they were made");
        // to the right: the ones between move left by one
        assert!(frame.move_tab("w1", 2));
        assert_eq!(tabs(&frame), ["two", "three", "one", "four"]);
        // to the left
        assert!(frame.move_tab("w4", 0));
        assert_eq!(tabs(&frame), ["four", "two", "three", "one"]);
        // onto itself, or a tab that isn't there: nothing moves
        assert!(!frame.move_tab("w2", 1) && !frame.move_tab("w9", 0));
        // past the end is the end
        assert!(frame.move_tab("w4", 9));
        assert_eq!(tabs(&frame), ["two", "three", "one", "four"]);
        // the workspace on screen is the one it was; ⌘1 and ⌃⇥ follow the order
        assert_eq!(name(&frame), Some("one"));
        frame.select_tab(0);
        assert_eq!(name(&frame), Some("two"));
        frame.cycle_workspace(1);
        assert_eq!(name(&frame), Some("three"));
        // a workspace made after you ordered them joins at the end
        frame.add_workspace(ws("w5", "five"));
        assert_eq!(tabs(&frame), ["two", "three", "one", "four", "five"]);
        // the order is the project's own
        frame.projects.push(project("p2", "second"));
        frame.workspaces.push(of("p2", ws("w6", "six")));
        frame.workspaces.push(of("p2", ws("w7", "seven")));
        assert!(frame.move_tab("w7", 0));
        frame.select_project("p2");
        assert_eq!(tabs(&frame), ["seven", "six"]);
        assert_eq!(
            frame.all_ordered().iter().map(|w| w.id.as_str()).collect::<Vec<_>>(),
            ["w2", "w3", "w1", "w4", "w5", "w7", "w6"]
        );
        // and it is there after a restart, as far as its workspaces still are
        let view = frame.view();
        let gone: Vec<Workspace> = frame.workspaces.iter().filter(|w| w.id != "w3").cloned().collect();
        let again = Frame::load(frame.projects.clone(), gone, HashMap::new(), &view);
        assert_eq!(
            again.tabs_of("p1").iter().map(|w| w.name.as_str()).collect::<Vec<_>>(),
            ["two", "one", "four", "five"]
        );
        assert_eq!(tabs(&again), ["seven", "six"]);
        // a workspace that is removed leaves the order
        let mut less = frame.clone();
        less.remove_workspace("w1");
        assert_eq!(less.order["p1"], ["w2", "w3", "w4"]);
        less.remove_project("p2");
        assert!(!less.order.contains_key("p2"));
    }

    /// Item 18.3: a workspace is put away and brought back.
    #[test]
    fn a_workspace_is_put_away_and_comes_back_to_its_place() {
        let mut frame = loaded(vec![ws("w1", "one"), ws("w2", "two"), ws("w3", "three")]);
        frame.apply(&Event::StatusChanged { workspace_id: "w2".into(), status: "needs_you".into() });
        frame.select_project("p1");
        assert_eq!(frame.waiting("p1"), 1);
        // the one on screen is put away: its tab goes, and the one that takes its place shows
        frame.select_workspace("w2");
        assert!(frame.hide_workspace("w2", 700));
        assert_eq!((tabs(&frame), name(&frame)), (vec!["one", "three"], Some("three")));
        assert_eq!(frame.hidden_tabs().iter().map(|w| w.name.as_str()).collect::<Vec<_>>(), ["two"]);
        assert!(frame.is_hidden("w2") && !frame.hide_workspace("w2", 800), "it is put away already");
        // it is quiet: not counted, and the keys pass it by
        assert_eq!(frame.waiting("p1"), 0);
        frame.cycle_workspace(1);
        assert_eq!(name(&frame), Some("one"));
        assert!(!frame.select_tab(2), "two tabs: there is no third");
        // nothing of it is gone: it is still a workspace, with its lamp
        assert_eq!((frame.workspaces.len(), frame.status("w2")), (3, "needs_you"));
        // the last tab put away: the one before it shows; one that isn't on screen: what is on screen stays
        frame.select_workspace("w3");
        assert!(frame.hide_workspace("w3", 900));
        assert_eq!((tabs(&frame), name(&frame)), (vec!["one"], Some("one")));
        // it is kept: after a restart they are still put away, and the workspace you had is a tab
        let again = Frame::load(frame.projects.clone(), frame.workspaces.clone(), HashMap::new(), &frame.view());
        assert_eq!((tabs(&again), name(&again)), (vec!["one"], Some("one")));
        assert_eq!(again.hidden, HashMap::from([("w2".to_string(), 700), ("w3".to_string(), 900)]));
        // brought back: its tab stands where it stood, and it is on screen
        assert!(frame.show_workspace("w2"));
        assert_eq!((tabs(&frame), name(&frame)), (vec!["one", "two"], Some("two")));
        assert_eq!(frame.waiting("p1"), 1, "back among the tabs, it is counted again");
        // going to one that is put away any other way brings it back too (the palette, a card's Open)
        assert!(frame.select_workspace("w3"));
        assert_eq!((tabs(&frame), name(&frame)), (vec!["one", "two", "three"], Some("three")));
        assert!(frame.hidden.is_empty());
        // every workspace put away: no tab, nothing on screen, and they are all there to bring back
        for id in ["w1", "w2", "w3"] {
            assert!(frame.hide_workspace(id, 1000));
        }
        assert_eq!((tabs(&frame).len(), name(&frame), frame.hidden_tabs().len()), (0, None, 3));
        let again = Frame::load(frame.projects.clone(), frame.workspaces.clone(), HashMap::new(), &frame.view());
        assert_eq!((tabs(&again).len(), name(&again)), (0, None));
        assert!(frame.show_workspace("w2"));
        assert_eq!((tabs(&frame), name(&frame)), (vec!["two"], Some("two")));
        // one that is removed is no longer put away; one that was never there can't be
        frame.remove_workspace("w1");
        assert!(!frame.hidden.contains_key("w1") && !frame.hide_workspace("w9", 1) && !frame.show_workspace("w9"));
    }

    /// A tab that moves leaves the workspaces you put away where they stood among the others, so one that comes
    /// back stands where it was.
    #[test]
    fn a_hidden_workspace_keeps_its_place_while_the_tabs_move() {
        let mut frame = loaded(vec![ws("w1", "one"), ws("w2", "two"), ws("w3", "three"), ws("w4", "four")]);
        assert!(frame.hide_workspace("w2", 1));
        assert_eq!(tabs(&frame), ["one", "three", "four"]);
        // the last tab goes first: it takes the first tab's place, the hidden one stays second of all
        assert!(frame.move_tab("w4", 0));
        assert_eq!(tabs(&frame), ["four", "one", "three"]);
        assert_eq!(
            frame.ordered("p1").iter().map(|w| w.name.as_str()).collect::<Vec<_>>(),
            ["four", "two", "one", "three"]
        );
        assert!(frame.show_workspace("w2"));
        assert_eq!(tabs(&frame), ["four", "two", "one", "three"]);
    }

    /// DESIGN.md, principle 8: "the last workspace of every project" survives a restart.
    #[test]
    fn each_project_comes_back_on_the_workspace_you_left_it_on() {
        let projects = vec![project("p1", "shop"), project("p2", "second")];
        let workspaces =
            vec![ws("w1", "one"), ws("w2", "two"), of("p2", ws("w3", "three")), of("p2", ws("w4", "four"))];
        let mut frame = Frame::load(projects.clone(), workspaces.clone(), HashMap::new(), &View::default());
        assert_eq!(name(&frame), Some("one"));
        frame.select_workspace("w2");
        frame.select_project("p2");
        assert_eq!(name(&frame), Some("three"), "a project you never opened shows its first workspace");
        frame.select_workspace("w4");
        frame.select_project("p1");
        assert_eq!(name(&frame), Some("two"), "back where you left it");
        frame.select_project("p2");
        assert_eq!(name(&frame), Some("four"));
        // quit and opened again: the project that was on screen, and each project on its own workspace
        let mut again = Frame::load(projects, workspaces, HashMap::new(), &frame.view());
        assert_eq!((again.current_project.as_deref(), name(&again)), (Some("p2"), Some("four")));
        again.select_project("p1");
        assert_eq!(name(&again), Some("two"));
    }

    /// Item 5.1c: the Work page is where you are until you put a project or a workspace on screen, and it is
    /// remembered.
    #[test]
    fn the_work_page_is_where_you_are_until_a_project_or_a_workspace_comes_on_screen() {
        let workspaces = vec![ws("w1", "one"), ws("w2", "two"), of("p2", ws("w3", "elsewhere"))];
        let projects = vec![project("p1", "shop"), project("p2", "cafe")];
        let mut frame = Frame::load(projects, workspaces, HashMap::new(), &View::default());
        assert!(!frame.work_open && !frame.view().work);
        assert!(frame.toggle_work() && frame.work_open);
        // it is remembered with where you were, and read back
        let view = frame.view();
        assert!(view.work);
        let text = serde_json::to_string(&view).unwrap();
        let read: View = serde_json::from_str(&text).unwrap();
        let again = Frame::load(frame.projects.clone(), frame.workspaces.clone(), HashMap::new(), &read);
        assert!(again.work_open);
        // a file from before the Work page
        let old: View = serde_json::from_str(r#"{"project_id":"p1","selected":{"p1":"w2"}}"#).unwrap();
        assert!(!old.work);
        // the workspace that is already on screen: asking for it leaves the page, and that counts as a change
        assert!(frame.select_workspace("w1"), "it was on screen behind the page");
        assert!(!frame.work_open);
        assert!(!frame.select_workspace("w1"), "now nothing changes");
        // so does asking for the project on screen, another project, a tab by its number, the next tab
        frame.toggle_work();
        assert!(frame.select_project("p1") && !frame.work_open);
        frame.toggle_work();
        assert!(frame.select_project("p2") && !frame.work_open);
        frame.select_project("p1");
        frame.toggle_work();
        assert!(frame.select_tab(0) && !frame.work_open);
        frame.toggle_work();
        assert!(frame.cycle_workspace(1) && !frame.work_open);
        assert_eq!(name(&frame), Some("two"));
        // ⌘I again goes back to it, and back
        assert!(frame.toggle_work() && frame.work_open);
        assert!(frame.toggle_work() && !frame.work_open);
        // with no project there is no Work page to go to, and it goes with the last project
        let mut empty = Frame::load(Vec::new(), Vec::new(), HashMap::new(), &View { work: true, ..View::default() });
        assert!(!empty.work_open && !empty.toggle_work());
        frame.toggle_work();
        frame.remove_project("p2");
        assert!(frame.work_open, "a project is left");
        frame.remove_project("p1");
        assert!(!frame.work_open);
    }

    #[test]
    fn removing_the_shown_workspace_shows_the_next_one() {
        let mut frame = loaded(vec![ws("w1", "one"), ws("w2", "two"), ws("w3", "three")]);
        frame.remove_workspace("w1");
        assert_eq!(frame.workspaces.iter().map(|w| w.id.as_str()).collect::<Vec<_>>(), ["w2", "w3"]);
        assert_eq!(name(&frame), Some("two"));
        assert!(!frame.statuses.contains_key("w1"), "its lamp goes too");
        // the last tab shown: the one before it shows
        frame.select_workspace("w3");
        frame.remove_workspace("w3");
        assert_eq!(name(&frame), Some("two"));
        frame.remove_workspace("w2");
        assert_eq!(name(&frame), None, "no tabs left");
    }

    #[test]
    fn goes_to_the_next_and_previous_tab_of_the_project_on_screen_wrapping_around() {
        // a workspace of another project is not one of the tabs, so cycling never lands on it
        let mut frame =
            loaded(vec![ws("w1", "one"), ws("w2", "two"), of("p2", ws("w4", "elsewhere")), ws("w3", "three")]);
        assert_eq!(name(&frame), Some("one"));
        frame.cycle_workspace(1);
        assert_eq!(name(&frame), Some("two"));
        frame.cycle_workspace(1);
        frame.cycle_workspace(1);
        assert_eq!(name(&frame), Some("one"));
        frame.cycle_workspace(-1);
        assert_eq!(name(&frame), Some("three"));
        assert!(frame.select_tab(1));
        assert_eq!(name(&frame), Some("two"));
        assert!(!frame.select_tab(8), "no ninth tab");
    }

    #[test]
    fn follows_status_live_and_workspace_events_from_the_core() {
        let mut frame = loaded(vec![ws("w1", "one")]);
        assert!(frame.apply(&Event::StatusChanged { workspace_id: "w1".into(), status: "needs_you".into() }));
        assert!(
            !frame.apply(&Event::StatusChanged { workspace_id: "w1".into(), status: "needs_you".into() }),
            "no change"
        );
        assert_eq!(frame.status("w1"), "needs_you");
        let live = LiveOutcome {
            project_id: "p1".into(),
            live_workspace_id: Some("w1".into()),
            note: String::new(),
            output: String::new(),
        };
        assert!(frame.apply(&Event::LiveChanged(live)));
        assert_eq!(frame.projects[0].live_workspace_id.as_deref(), Some("w1"));
        let moved = Workspace { branch: "feature/other".into(), ..ws("w1", "one") };
        assert!(frame.apply(&Event::WorkspaceUpdated(moved)));
        assert_eq!(frame.workspaces[0].branch, "feature/other");
        // a workflow's new workspace is a tab, not put on screen; the same one twice is one tab
        assert!(frame.apply(&Event::WorkspaceAdded(ws("w5", "task"))));
        assert!(!frame.apply(&Event::WorkspaceAdded(ws("w5", "task"))));
        assert_eq!(frame.tabs().len(), 2);
        assert_eq!(name(&frame), Some("one"));
        assert!(!frame.apply(&Event::WorkChanged), "not the frame's news");
    }

    fn change(path: &str) -> enkidu_core::git::Change {
        enkidu_core::git::Change {
            path: path.into(),
            old_path: None,
            kind: "modified".into(),
            staged: false,
            additions: Some(1),
            deletions: Some(0),
            sig: "s".into(),
        }
    }

    #[test]
    fn creates_a_workspace_and_selects_it() {
        // the workspace made (here, by the New workspace dialog) joins its project's tabs and comes on screen; made
        // again, it isn't added twice
        let mut frame = loaded(vec![ws("w1", "one")]);
        frame.add_workspace(ws("w2", "two"));
        assert_eq!(frame.tabs().iter().map(|w| w.name.as_str()).collect::<Vec<_>>(), ["one", "two"]);
        assert_eq!(frame.current_workspace().map(|w| w.id.as_str()), Some("w2"));
        frame.add_workspace(ws("w2", "two"));
        assert_eq!(frame.workspaces.len(), 2);
    }

    #[test]
    fn counts_each_workspaces_changed_files_and_pops_a_lamp_only_on_a_new_state() {
        let mut frame = loaded(vec![ws("w1", "one"), ws("w2", "two")]);
        assert_eq!(frame.changed_files("w1"), 0, "none until the core says");
        let files = vec![change("a.py"), change("b.py")];
        assert!(frame.apply(&Event::ChangesUpdated { workspace_id: "w1".into(), files: files.clone() }));
        assert!(!frame.apply(&Event::ChangesUpdated { workspace_id: "w1".into(), files }), "the same count");
        assert_eq!((frame.changed_files("w1"), frame.changed_files("w2")), (2, 0));
        assert_eq!(
            frame.changes("w1"),
            Some(Changes { files: 2, additions: 2, deletions: 0 }),
            "lines added and removed"
        );
        assert_eq!(frame.changes("w2"), None, "not said yet");

        // w1 was working at the start, w2 idle (no word from the core yet)
        assert_eq!((frame.pops("w1"), frame.pops("w2")), (0, 0), "what the core knew at the start never pops");
        frame.apply(&Event::StatusChanged { workspace_id: "w2".into(), status: "idle".into() });
        assert_eq!(frame.pops("w2"), 0, "idle was already shown");
        for status in ["working", "working", "needs_you"] {
            frame.apply(&Event::StatusChanged { workspace_id: "w2".into(), status: status.into() });
        }
        assert_eq!(frame.pops("w2"), 2, "working once, then needs you");

        // remarks not sent yet are counted too, as the remarks store says them
        assert!(frame.set_remarks("w1", 2) && !frame.set_remarks("w1", 2) && !frame.set_remarks("w2", 0));
        assert_eq!((frame.remarks("w1"), frame.remarks("w2")), (2, 0));

        frame.remove_workspace("w1");
        frame.remove_workspace("w2");
        assert_eq!((frame.changed_files("w1"), frame.pops("w2")), (0, 0), "a removed workspace leaves nothing");
        assert_eq!(frame.remarks("w1"), 0);
    }

    #[test]
    fn knows_each_workspaces_live_button_and_who_is_live_elsewhere() {
        use crate::live::LiveState;
        let mut frame = loaded(vec![ws("w1", "one"), ws("w2", "two")]);
        let (one, two) = (frame.workspaces[0].clone(), frame.workspaces[1].clone());
        assert_eq!((frame.live_state(&one), frame.live_elsewhere(&one)), (LiveState::Off, None));
        frame.projects[0].live_workspace_id = Some(two.id.clone());
        assert_eq!(frame.live_state(&two), LiveState::Live);
        assert_eq!(frame.live_elsewhere(&two), None, "it is the Live one itself");
        assert_eq!((frame.live_state(&one), frame.live_elsewhere(&one)), (LiveState::Off, Some(two.name.as_str())));
        // a press on the first one's button: the one that is Live goes back first
        assert!(frame.live_switching.start(&one.project_id, Some(&one.id)));
        assert_eq!((frame.live_state(&one), frame.live_state(&two)), (LiveState::Starting, LiveState::Stopping));
        frame.live_switching.end(&one.project_id);
        assert_eq!(frame.live_state(&two), LiveState::Live);
    }

    #[test]
    fn knows_the_live_workspace() {
        let mut frame = loaded(vec![ws("w1", "one"), ws("w2", "two")]);
        frame.projects[0].live_workspace_id = Some("w2".into());
        let (one, two) = (frame.workspaces[0].clone(), frame.workspaces[1].clone());
        assert!(!frame.is_live(&one));
        assert!(frame.is_live(&two));
    }

    #[test]
    fn projects_come_and_go_and_say_how_many_of_their_workspaces_wait_for_you() {
        let mut frame = loaded(vec![ws("w1", "one"), of("p2", ws("w2", "two")), of("p2", ws("w3", "three"))]);
        frame.add_project(project("p2", "second"));
        assert_eq!(frame.current_project.as_deref(), Some("p2"), "an added project goes on screen");
        frame.add_project(project("p2", "second"));
        assert_eq!(frame.projects.len(), 2, "once");
        frame.statuses.insert("w2".into(), "needs_you".into());
        frame.statuses.insert("w3".into(), "needs_you".into());
        assert_eq!((frame.waiting("p1"), frame.waiting("p2")), (0, 2));
        frame.add_project(project("p3", "third"));
        frame.remove_project("p3");
        assert_eq!(frame.current_project.as_deref(), Some("p1"), "the first one left goes on screen");
        assert_eq!(frame.projects.len(), 2);
    }

    #[test]
    fn orders_workspaces_by_project_and_gives_the_tabs_of_the_project_on_screen() {
        let projects = vec![project("p1", "shop"), project("p2", "second")];
        let all = vec![of("p2", ws("a", "a")), ws("b", "b"), ws("c", "c")];
        let ids = |list: Vec<&Workspace>| list.iter().map(|w| w.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(workspace_order(&projects, &all)), ["b", "c", "a"]);
        assert_eq!(ids(project_tabs(&all, Some("p1"))), ["b", "c"]);
        assert_eq!(ids(project_tabs(&all, Some("p2"))), ["a"]);
        assert!(project_tabs(&all, None).is_empty());
    }
}
