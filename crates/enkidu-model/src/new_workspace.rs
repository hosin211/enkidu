//! The New workspace dialog's rules (item 1.19): what a name may be, the new branch that follows the name until you
//! change it, which branch a request names, the branches to pick from, and the steps shown while the workspace is made.

use enkidu_core::workspace::{CreateRequest, Progress};

/// What the name field says when the name won't do.
pub const NAME_RULE: &str = "Letters, numbers, dots, dashes and underscores";

/// A workspace's name: a letter or digit, then up to 63 letters, digits, dots, dashes or underscores (`NAME_RE`).
pub fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else { return false };
    first.is_ascii_alphanumeric()
        && name.len() <= 64
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// A workspace name suggested from a branch: "feature/SH-21309-x" gives "SH-21309-x".
pub fn name_from_branch(branch: &str) -> String {
    let last = branch.rsplit('/').next().unwrap_or(branch);
    let mut out = String::new();
    let mut in_run = false;
    for c in last.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
            out.push(c);
            in_run = false;
        } else if !in_run {
            out.push('-');
            in_run = true;
        }
    }
    let start = out.find(|c: char| c.is_ascii_alphanumeric()).unwrap_or(out.len());
    out[start..].chars().take(64).collect()
}

/// Where Claude works in the project (item 19.2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Place {
    /// A workspace of its own: a checkout on its own branch.
    #[default]
    Workspace,
    /// The project's main folder itself, as it is: nothing is made.
    Main,
}

/// What the dialog asks first, and its two answers.
pub const WHERE: &str = "Where Claude works";
pub const WHERE_NEW: &str = "A new workspace";
pub const WHERE_MAIN: &str = "The main folder";
/// The name the main folder gets as a workspace (its tab).
pub const MAIN_NAME: &str = "main-folder";
/// What working in the main folder means, under its path.
pub const MAIN_SAYS: &str = "Claude works in the project's own folder, on the branch it is on. Nothing is made or \
                             copied there, and removing the workspace later only forgets it.";

impl Place {
    /// The line under the dialog's title.
    pub fn says(self, project: &str) -> String {
        match self {
            Place::Workspace => format!("A worktree of {project} on its own branch, with Claude in it."),
            Place::Main => format!("Claude in {project}'s main folder, as it is."),
        }
    }

    /// What the dialog's button says, and while the core is at it.
    pub fn button(self, busy: bool) -> &'static str {
        match (self, busy) {
            (Place::Workspace, false) => "Create workspace",
            (Place::Workspace, true) => "Creating…",
            (Place::Main, false) => "Open the main folder",
            (Place::Main, true) => "Opening…",
        }
    }

    /// What Enter does, said in the dialog's foot.
    pub fn enter(self) -> &'static str {
        match self {
            Place::Workspace => "↩ to create",
            Place::Main => "↩ to open",
        }
    }
}

/// Where the new branch comes from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// A new branch, from a base branch.
    #[default]
    New,
    /// A branch that exists, as it is.
    Existing,
}

/// What has been typed and picked so far.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Draft {
    /// Where Claude works: a new workspace (what follows is for it), or the main folder.
    pub place: Place,
    pub name: String,
    /// The name field was left once: from then on a bad name is said.
    pub name_touched: bool,
    pub mode: Mode,
    /// The new branch's name as typed; until it is typed, it is the workspace's name.
    pub branch: String,
    pub branch_touched: bool,
    /// The base of a new branch.
    pub base: String,
    /// The existing branch picked.
    pub existing: String,
}

impl Draft {
    /// The new branch's name: what was typed for it, or else the workspace's name.
    pub fn new_branch(&self) -> &str {
        if self.branch_touched { &self.branch } else { &self.name }
    }

    /// What the name field says, once it was left with a name that won't do.
    pub fn name_error(&self) -> Option<&'static str> {
        (self.name_touched && !self.name.is_empty() && !valid_name(&self.name)).then_some(NAME_RULE)
    }

    /// Whether Create workspace can be pressed. The main folder needs nothing typed: it can always be opened.
    pub fn valid(&self) -> bool {
        self.place == Place::Main
            || valid_name(&self.name)
                && match self.mode {
                    Mode::New => {
                        let branch = self.new_branch().trim();
                        !branch.is_empty() && !branch.contains(char::is_whitespace) && !self.base.is_empty()
                    }
                    Mode::Existing => !self.existing.is_empty(),
                }
    }

    /// An existing branch is picked: with no name yet, the workspace is named after it.
    pub fn pick_existing(&mut self, branch: &str) {
        self.existing = branch.to_string();
        if self.name.is_empty() {
            self.name = name_from_branch(branch);
        }
    }

    /// What the core is asked to make.
    pub fn request(&self, project_id: &str) -> CreateRequest {
        let (mode, branch, base) = match self.mode {
            Mode::New => ("new", self.new_branch().trim().to_string(), Some(self.base.clone())),
            Mode::Existing => ("existing", self.existing.clone(), None),
        };
        CreateRequest {
            project_id: project_id.into(),
            name: self.name.trim().into(),
            mode: mode.into(),
            branch,
            base,
            session_id: None,
        }
    }
}

/// Where a branch to pick from is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchItem {
    pub name: String,
    /// "local", or "origin" for one only on origin.
    pub hint: &'static str,
}

/// The branches to pick from: every local one, and those only on origin, by name.
pub fn branch_items(local: &[String], remote: &[String]) -> Vec<BranchItem> {
    let mut items: Vec<BranchItem> = local.iter().map(|b| BranchItem { name: b.clone(), hint: "local" }).collect();
    for b in remote {
        if !local.contains(b) {
            items.push(BranchItem { name: b.clone(), hint: "origin" });
        }
    }
    items.sort_by(|a, b| a.name.cmp(&b.name));
    items
}

/// The words for a step of making a workspace.
pub fn step_label(step: &str) -> &str {
    match step {
        "check" => "Check name and branch",
        "fetch" => "Fetch from origin",
        "worktree" => "Create the worktree",
        "copy" => "Copy your files",
        "setup" => "Run the setup script",
        other => other,
    }
}

/// The steps shown while a workspace is made: each step once, where it first came, with its latest state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Steps(pub Vec<Progress>);

impl Steps {
    /// The core's news of a step, for the project being made in. "done" ends the list and isn't shown.
    pub fn apply(&mut self, progress: &Progress, project_id: &str) -> bool {
        if progress.project_id != project_id || progress.step == "done" {
            return false;
        }
        match self.0.iter_mut().find(|s| s.step == progress.step) {
            Some(step) if step == progress => false,
            Some(step) => {
                *step = progress.clone();
                true
            }
            None => {
                self.0.push(progress.clone());
                true
            }
        }
    }
}

/// The first of "x-2", "x-3"… that isn't a branch yet (the new branch offered from one checked out elsewhere).
pub fn next_branch(branch: &str, taken: &[String]) -> String {
    (2..).map(|n| format!("{branch}-{n}")).find(|b| !taken.contains(b)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    // The dialog's rules, without the screen.

    #[test]
    fn mirrors_the_name_into_the_branch_until_the_branch_is_edited() {
        let mut draft = Draft { name: "stock-fix".into(), ..Default::default() };
        assert_eq!(draft.new_branch(), "stock-fix");
        draft.name = "stock-fix-2".into();
        assert_eq!(draft.new_branch(), "stock-fix-2");
        draft.branch = "fix/SH-21309".into();
        draft.branch_touched = true;
        draft.name = "other".into();
        assert_eq!(draft.new_branch(), "fix/SH-21309", "edited, it stays");
    }

    #[test]
    fn refuses_bad_names_and_sends_a_new_branch_request() {
        for bad in ["", "-dash-first", "has space", "a/b", "ü", &"x".repeat(65)] {
            assert!(!valid_name(bad), "{bad:?}");
        }
        for good in ["a", "SH-21254_stock.fix", &"x".repeat(64)] {
            assert!(valid_name(good), "{good:?}");
        }
        let mut draft = Draft { name: "bad name".into(), base: "main".into(), ..Default::default() };
        assert_eq!(draft.name_error(), None, "said only once the field was left");
        draft.name_touched = true;
        assert_eq!(draft.name_error(), Some(NAME_RULE));
        assert!(!draft.valid());
        draft.name = "stock-fix".into();
        assert!(draft.valid());
        let request = draft.request("p1");
        assert_eq!(
            (request.mode.as_str(), request.branch.as_str(), request.base.as_deref(), request.name.as_str()),
            ("new", "stock-fix", Some("main"), "stock-fix")
        );
        draft.base.clear();
        assert!(!draft.valid(), "a new branch needs a base");
        draft.base = "main".into();
        draft.branch = "has space".into();
        draft.branch_touched = true;
        assert!(!draft.valid(), "a branch name has no spaces");
    }

    /// Item 19.2: the
    /// dialog's first choice. A new workspace needs its name and its branch; the main folder needs nothing.
    #[test]
    fn the_main_folder_needs_nothing_typed_and_says_what_it_is() {
        let mut draft = Draft::default();
        assert_eq!(draft.place, Place::Workspace, "a new workspace, until you say otherwise");
        assert!(!draft.valid(), "a new workspace needs a name and a branch");
        draft.place = Place::Main;
        assert!(draft.valid(), "the main folder is there as it is");
        // a name that won't do for a workspace is nothing to the main folder
        draft.name = "bad name".into();
        assert!(draft.valid());
        draft.place = Place::Workspace;
        assert!(!draft.valid());
        // what each choice says
        assert_eq!(Place::Workspace.says("shop"), "A worktree of shop on its own branch, with Claude in it.");
        assert_eq!(Place::Main.says("shop"), "Claude in shop's main folder, as it is.");
        assert_eq!((Place::Workspace.button(false), Place::Workspace.button(true)), ("Create workspace", "Creating…"));
        assert_eq!((Place::Main.button(false), Place::Main.button(true)), ("Open the main folder", "Opening…"));
        assert_eq!((Place::Workspace.enter(), Place::Main.enter()), ("↩ to create", "↩ to open"));
        assert!(valid_name(MAIN_NAME), "the main folder's name is one a workspace can have");
    }

    #[test]
    fn needs_a_branch_in_existing_mode() {
        let mut draft = Draft { mode: Mode::Existing, ..Default::default() };
        assert!(!draft.valid());
        draft.pick_existing("feature/SH-21349-api-gift-card-balances");
        assert_eq!(draft.name, "SH-21349-api-gift-card-balances", "named after the branch");
        assert!(draft.valid());
        let request = draft.request("p1");
        assert_eq!(
            (request.mode.as_str(), request.branch.as_str(), request.base),
            ("existing", "feature/SH-21349-api-gift-card-balances", None)
        );
        draft.pick_existing("fix/other");
        assert_eq!(draft.name, "SH-21349-api-gift-card-balances", "a name you have stays");
    }

    #[test]
    fn names_from_branches_and_the_next_free_branch() {
        assert_eq!(name_from_branch("feature/SH-21309-x"), "SH-21309-x");
        assert_eq!(name_from_branch("weird name!!here"), "weird-name-here");
        assert_eq!(name_from_branch("--x"), "x");
        assert_eq!(next_branch("fix", &["fix-2".into(), "fix-3".into()]), "fix-4");
        assert_eq!(next_branch("fix", &[]), "fix-2");
    }

    #[test]
    fn lists_local_branches_and_those_only_on_origin_by_name() {
        let items = branch_items(&["main".into(), "fix".into()], &["main".into(), "dev".into()]);
        let seen: Vec<(&str, &str)> = items.iter().map(|i| (i.name.as_str(), i.hint)).collect();
        assert_eq!(seen, [("dev", "origin"), ("fix", "local"), ("main", "local")]);
    }

    #[test]
    fn shows_each_step_once_with_its_latest_state() {
        let step = |step: &str, state: &str| Progress {
            project_id: "p1".into(),
            step: step.into(),
            state: state.into(),
            detail: String::new(),
        };
        let mut steps = Steps::default();
        assert!(steps.apply(&step("check", "running"), "p1"));
        assert!(steps.apply(&step("check", "done"), "p1"));
        assert!(steps.apply(&step("fetch", "skipped"), "p1"));
        assert!(!steps.apply(&step("fetch", "skipped"), "p1"), "the same news");
        assert!(!steps.apply(&step("worktree", "running"), "p2"), "another project's");
        assert!(!steps.apply(&step("done", "done"), "p1"), "done ends the list");
        let seen: Vec<(&str, &str)> = steps.0.iter().map(|s| (s.step.as_str(), s.state.as_str())).collect();
        assert_eq!(seen, [("check", "done"), ("fetch", "skipped")]);
        assert_eq!(step_label("worktree"), "Create the worktree");
    }
}
