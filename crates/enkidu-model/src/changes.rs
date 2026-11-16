//! Each workspace's changed files (item 3.1). Instant first, fresh second (DESIGN.md, Principle 9): the lists as last
//! seen show from the first frame, what the core's watcher has already worked out goes on top, and git's fresh answers
//! replace them a moment later. A list that says the same as the one shown is kept as it is, so nothing drawn from it
//! is drawn again: git is asked each time a workspace comes on screen, and mostly nothing changed.
//!
//! Every list is kept in the order a tree of its folders lists it (`names::in_tree_order`; item 29.2): the core hands
//! its lists over in that order already, and a list kept from before that is put into it here.

use std::collections::HashMap;
use std::sync::Arc;

use enkidu_core::git::Change;
use enkidu_core::names::in_tree_order;

/// A list in the order a tree of its folders lists it. One that is in it already costs a look at each file.
fn in_order(mut files: Vec<Change>) -> Vec<Change> {
    files.sort_by(|a, b| in_tree_order(&a.path, &b.path));
    files
}

/// Every workspace's changed files.
#[derive(Default)]
pub struct ChangeLists {
    lists: HashMap<String, Arc<Vec<Change>>>,
}

impl ChangeLists {
    /// At launch: every workspace's changes as last seen (`remembered`; a workspace that is gone is left out), with
    /// what the core's watcher already knows (`known`) on top.
    pub fn load(
        remembered: HashMap<String, Vec<Change>>,
        known: HashMap<String, Vec<Change>>,
        workspaces: &[String],
    ) -> ChangeLists {
        let mut lists: HashMap<String, Arc<Vec<Change>>> = remembered
            .into_iter()
            .filter(|(id, _)| workspaces.contains(id))
            .map(|(id, files)| (id, Arc::new(in_order(files))))
            .collect();
        lists.extend(known.into_iter().map(|(id, files)| (id, Arc::new(in_order(files)))));
        ChangeLists { lists }
    }

    /// A workspace's changed files, once anything is known of them.
    pub fn of(&self, workspace: &str) -> Option<&Arc<Vec<Change>>> {
        self.lists.get(workspace)
    }

    /// Git's answer for a workspace. False when it says the same as the list shown, which is kept.
    pub fn set(&mut self, workspace: &str, files: Vec<Change>) -> bool {
        let files = in_order(files);
        if self.lists.get(workspace).is_some_and(|shown| **shown == files) {
            return false;
        }
        self.lists.insert(workspace.to_string(), Arc::new(files));
        true
    }

    /// A workspace was removed. False when nothing was known of it.
    pub fn forget(&mut self, workspace: &str) -> bool {
        self.lists.remove(workspace).is_some()
    }

    /// Every list, to keep for the next launch.
    pub fn all(&self) -> HashMap<String, Arc<Vec<Change>>> {
        self.lists.clone()
    }
}

/// One question to git at a time for each workspace (item 3.17). Git is asked each time a workspace comes on screen,
/// and in a big repository an answer takes a moment: going back and forth between two workspaces must not start a
/// git for every visit (the load test saw over thirty at once). A visit while a question is out is answered by one
/// more question when that one ends, however many visits there were.
#[derive(Default)]
pub struct OneAtATime {
    /// The workspaces whose question is out, and whether another was wished for meanwhile.
    out: HashMap<String, bool>,
}

impl OneAtATime {
    /// A wish to ask about `workspace`: whether to ask now. While its question is out the wish is kept.
    pub fn wish(&mut self, workspace: &str) -> bool {
        match self.out.get_mut(workspace) {
            Some(again) => {
                *again = true;
                false
            }
            None => {
                self.out.insert(workspace.to_string(), false);
                true
            }
        }
    }

    /// The question about `workspace` ended (answered or not): whether to ask once more, for the wishes kept.
    pub fn ended(&mut self, workspace: &str) -> bool {
        self.out.remove(workspace).unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_is_asked_once_at_a_time_for_a_workspace_and_once_more_for_the_visits_meanwhile() {
        let mut asking = OneAtATime::default();
        assert!(asking.wish("one"), "the first visit asks");
        assert!(!asking.wish("one"), "a visit while it is asked doesn't ask again");
        assert!(!asking.wish("one"));
        // another workspace has its own question
        assert!(asking.wish("two"));
        // the answer came: one more question for all the visits meanwhile
        assert!(asking.ended("one"));
        assert!(asking.wish("one"), "that question is asked");
        assert!(!asking.ended("one"), "nobody came meanwhile: nothing more to ask");
        assert!(!asking.ended("two"));
        // and then a visit asks again
        assert!(asking.wish("one"));
        // a question that was never out has nothing to ask again
        assert!(!asking.ended("three"));
    }

    fn change(path: &str, sig: &str) -> Change {
        Change {
            path: path.into(),
            old_path: None,
            kind: "modified".into(),
            staged: false,
            additions: Some(2),
            deletions: Some(1),
            sig: sig.into(),
        }
    }

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|id| id.to_string()).collect()
    }

    /// Opens with every workspace's changes as last seen, the core's own lists winning.
    #[test]
    fn opens_with_every_workspaces_changes_as_last_seen_the_cores_own_lists_winning() {
        let remembered = HashMap::from([
            ("w1".to_string(), vec![change("a.py", "1")]),
            ("w2".to_string(), vec![change("b.py", "1")]),
            ("w9-removed".to_string(), vec![change("c.py", "1")]),
        ]);
        // the watcher has already looked at w2 again
        let known = HashMap::from([("w2".to_string(), vec![change("b.py", "2"), change("d.py", "1")])]);
        let lists = ChangeLists::load(remembered, known, &ids(&["w1", "w2", "w3"]));
        assert_eq!(lists.of("w1").unwrap().as_slice(), [change("a.py", "1")]);
        assert_eq!(lists.of("w2").unwrap().len(), 2, "the core's answer wins");
        assert!(lists.of("w3").is_none(), "nothing known yet");
        assert!(lists.of("w9-removed").is_none(), "a workspace that is gone is left out");
    }

    /// Keeps the changes it shows when git answers the same again, so no diff renders twice.
    #[test]
    fn keeps_the_changes_it_shows_when_git_answers_the_same_again_so_no_diff_renders_twice() {
        let mut lists = ChangeLists::default();
        assert!(lists.set("w1", vec![change("a.py", "1")]));
        let shown = lists.of("w1").unwrap().clone();
        // git says the same again (a workspace came on screen): the list shown stays the very same one
        assert!(!lists.set("w1", vec![change("a.py", "1")]));
        assert!(Arc::ptr_eq(&shown, lists.of("w1").unwrap()));
        // the file changed again: its signature differs
        assert!(lists.set("w1", vec![change("a.py", "2")]));
        assert!(!Arc::ptr_eq(&shown, lists.of("w1").unwrap()));
        // the same file staged: that is news too
        let staged = enkidu_core::git::Change { staged: true, ..change("a.py", "2") };
        assert!(lists.set("w1", vec![staged.clone()]));
        assert!(lists.of("w1").unwrap()[0].staged);
        assert!(!lists.set("w1", vec![staged]), "and the same again is kept");
        // a commit emptied it
        assert!(lists.set("w1", Vec::new()));
        assert!(lists.of("w1").unwrap().is_empty());
        assert!(!lists.set("w1", Vec::new()));
    }

    /// Item 29.2: whatever order a list comes in, it is kept as a tree of its folders lists it.
    #[test]
    fn every_list_is_kept_in_the_order_a_tree_of_its_folders_lists_it() {
        let paths = |list: &[Change]| list.iter().map(|c| c.path.clone()).collect::<Vec<_>>();
        // a list kept from before, in the order of its paths' bytes
        let remembered = HashMap::from([("w1".to_string(), vec![change("notes.md", "1"), change("src/app.py", "1")])]);
        let mut lists = ChangeLists::load(remembered, HashMap::new(), &ids(&["w1"]));
        assert_eq!(paths(lists.of("w1").unwrap()), ["src/app.py", "notes.md"], "a folder stands before a file");
        // git's answer in another order says the same: the list shown is kept
        let shown = lists.of("w1").unwrap().clone();
        assert!(!lists.set("w1", vec![change("notes.md", "1"), change("src/app.py", "1")]));
        assert!(Arc::ptr_eq(&shown, lists.of("w1").unwrap()));
        // a new file takes its place in its folder
        assert!(
            lists.set(
                "w1",
                vec![change("zebra.txt", "1"), change("src/orders/stock.py", "1"), change("src/app.py", "1")]
            )
        );
        assert_eq!(paths(lists.of("w1").unwrap()), ["src/orders/stock.py", "src/app.py", "zebra.txt"]);
    }

    #[test]
    fn a_removed_workspace_is_forgotten() {
        let mut lists = ChangeLists::default();
        lists.set("w1", vec![change("a.py", "1")]);
        assert!(lists.forget("w1"));
        assert!(!lists.forget("w1"));
        assert!(lists.all().is_empty());
    }
}
