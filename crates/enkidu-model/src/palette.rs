//! The palette's first form (⌘K; item 1.21): every workspace of every project, then the actions that work so far,
//! narrowed as you type, the best matches first (nucleo's scoring, as the core ranks symbols).

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use crate::frame::Frame;
use crate::ui::{DiffLayout, Layout, Mode, Panel, Ui};

/// What a row does when picked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pick {
    Workspace(String),
    NewWorkspace,
    /// The remarks of this workspace (the one on screen) go into Claude's prompt.
    TypeRemarks(String),
    NextWorkspace,
    PreviousWorkspace,
    AddProject,
    /// The changes beside the terminal, or above it.
    Layout(Layout),
    /// The terminal alone, or the changes back (⌥⌘B).
    TerminalOnly,
    /// The changes alone, or the terminal back (⇧⌘E).
    FullReview,
    /// The diffs in one column, or before and after side by side.
    DiffLayout,
    /// A tab of the files' column: shown, or the column folded away if it was the one shown (⌥⌘1, ⌥⌘2).
    Panel(Panel),
    /// The rail and the files' column go, or come back (⌘S).
    Lists,
    /// Search, on one of its tabs (⇧⌘O, ⌘O, ⇧⌘F).
    Search(crate::search::Tab),
    /// The Settings dialog (⌘,).
    Settings,
    /// Live of `project` goes to `workspace` (none: it stops).
    Live {
        project: String,
        workspace: Option<String>,
    },
}

/// One row of the palette.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub pick: Pick,
    /// What a workspace or action is called.
    pub name: String,
    /// A workspace's project and branch: "shop / fix/stock".
    pub detail: Option<String>,
    /// A workspace's lamp: Claude's state in the core's word.
    pub status: Option<String>,
    pub live: bool,
    /// The workspace on screen, and the layout on screen: its row has the blue bar of what is on screen.
    pub current: bool,
    /// Its key: ⌘1 to ⌘9 for the tabs of the project on screen, ⌘N, ⌃⇥…
    pub key: Option<&'static str>,
    /// What typing is matched against.
    text: String,
}

/// A heading and its rows.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub heading: &'static str,
    pub rows: Vec<Row>,
}

const TAB_KEYS: [&str; 9] = ["⌘1", "⌘2", "⌘3", "⌘4", "⌘5", "⌘6", "⌘7", "⌘8", "⌘9"];

fn action(pick: Pick, name: &str, key: Option<&'static str>) -> Row {
    Row { pick, name: name.into(), detail: None, status: None, live: false, current: false, key, text: name.into() }
}

/// The palette's rows for the frame as it is: the workspaces in their order, the actions that apply, and with a
/// workspace on screen the layouts (`ui` is how the window is laid out; none in a window that keeps no layout).
pub fn groups(frame: &Frame, ui: Option<&Ui>) -> Vec<Group> {
    let current = frame.current_project.as_deref();
    // the tabs of the project on screen, in your order: ⌘1 to ⌘9 are theirs
    let on_screen = frame.tabs();
    let workspaces: Vec<Row> = frame
        .all_ordered()
        .into_iter()
        .map(|w| {
            let project = frame.projects.iter().find(|p| p.id == w.project_id).map(|p| p.name.as_str()).unwrap_or("");
            let tab = on_screen.iter().position(|t| t.id == w.id).filter(|ix| *ix < 9);
            // one you put away is listed too, and says so: picking it brings it back
            let hidden = if frame.is_hidden(&w.id) { ", hidden" } else { "" };
            Row {
                pick: Pick::Workspace(w.id.clone()),
                name: w.name.clone(),
                detail: Some(format!("{project} / {}{hidden}", w.branch)),
                status: Some(frame.status(&w.id).to_string()),
                live: frame.is_live(w),
                current: frame.current_workspace().is_some_and(|on_screen| on_screen.id == w.id),
                key: tab.map(|ix| TAB_KEYS[ix]),
                text: format!("{} {project} {}{hidden}", w.name, w.branch),
            }
        })
        .collect();
    let mut actions = Vec::new();
    if current.is_some() {
        actions.push(action(Pick::NewWorkspace, "New workspace", Some("⌘N")));
    }
    // the remarks that wait on the workspace on screen: "Type 3 remarks into Claude"
    if let Some(ws) = frame.current_workspace().filter(|ws| frame.remarks(&ws.id) > 0) {
        let words = crate::remarks::send_words(frame.remarks(&ws.id));
        actions.push(action(Pick::TypeRemarks(ws.id.clone()), &words, None));
    }
    // Live for the workspace on screen: "Go live with stock-fix", or "Stop Live for stock-fix" while it is. The
    // main folder itself has none: Live copies a workspace's changes into it
    if let Some(ws) = frame.current_workspace().filter(|ws| !frame.is_main_folder(ws)) {
        let live = frame.is_live(ws);
        let pick = Pick::Live { project: ws.project_id.clone(), workspace: (!live).then(|| ws.id.clone()) };
        actions.push(action(pick, &crate::live::command(&ws.name, live), None));
    }
    if frame.workspaces.len() > 1 {
        actions.push(action(Pick::NextWorkspace, "Next workspace", Some("⇧⌘]")));
        actions.push(action(Pick::PreviousWorkspace, "Previous workspace", Some("⇧⌘[")));
    }
    // Search, for the workspace on screen, in a window that has it (`ui` says so: the app's own window)
    if ui.is_some() && frame.current_workspace().is_some() {
        use crate::search::Tab;
        actions.push(action(Pick::Search(Tab::Files), "Go to file…", Some("⇧⌘O")));
        actions.push(action(Pick::Search(Tab::Symbols), "Go to symbol (class, function, method)…", Some("⌘O")));
        actions.push(action(Pick::Search(Tab::Text), "Find in files…", Some("⇧⌘F")));
    }
    actions.push(action(Pick::AddProject, "Add project", None));
    actions.push(action(Pick::Settings, "Settings", Some("⌘,")));
    // where the changes go, for the workspace on screen
    let mut layouts = Vec::new();
    if let Some(ui) = ui.filter(|_| frame.current_workspace().is_some()) {
        let chosen = ui.chosen();
        let current = |row: Row, current: bool| Row { current, ..row };
        layouts.push(current(
            action(Pick::Layout(Layout::Side), "Changes beside the terminal", None),
            chosen == Mode::Side,
        ));
        layouts.push(current(
            action(Pick::Layout(Layout::Stack), "Changes above the terminal", None),
            chosen == Mode::Stack,
        ));
        let alone = if ui.review_hidden { "Show the changes again" } else { "Terminal only" };
        layouts.push(action(Pick::TerminalOnly, alone, Some("⌥⌘B")));
        let full = if ui.review_maximized { "Back to the terminal" } else { "Changes in full size" };
        layouts.push(action(Pick::FullReview, full, Some("⇧⌘E")));
        let changed = match ui.right_panel {
            Some(Panel::Changes) => "Hide the changed files",
            _ => "Show the changed files",
        };
        layouts.push(action(Pick::Panel(Panel::Changes), changed, Some("⌥⌘1")));
        let files = match ui.right_panel {
            Some(Panel::Files) => "Hide the project's files",
            _ => "Show the project's files",
        };
        layouts.push(action(Pick::Panel(Panel::Files), files, Some("⌥⌘2")));
        let lists = if ui.lists_hidden { "Show the lists" } else { "Hide the lists" };
        layouts.push(action(Pick::Lists, lists, Some("⌘S")));
        let diffs = match ui.diff_layout {
            DiffLayout::Split => "Diffs in one column",
            DiffLayout::Unified => "Diffs before and after, side by side",
        };
        layouts.push(action(Pick::DiffLayout, diffs, None));
    }
    [("Workspaces", workspaces), ("Actions", actions), ("Layout", layouts)]
        .into_iter()
        .filter(|(_, rows)| !rows.is_empty())
        .map(|(heading, rows)| Group { heading, rows })
        .collect()
}

/// The groups narrowed to what matches `query`, each group's best matches first; all of them for an empty query.
pub fn narrow(groups: &[Group], query: &str) -> Vec<Group> {
    let query = query.trim();
    if query.is_empty() {
        return groups.to_vec();
    }
    let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
    let mut matcher = Matcher::new(Config::DEFAULT);
    let mut buf = Vec::new();
    groups
        .iter()
        .filter_map(|group| {
            let mut scored: Vec<(u32, &Row)> = group
                .rows
                .iter()
                .filter_map(|row| pattern.score(Utf32Str::new(&row.text, &mut buf), &mut matcher).map(|s| (s, row)))
                .collect();
            // the best first; equal scores keep their order
            scored.sort_by_key(|row| std::cmp::Reverse(row.0));
            let rows: Vec<Row> = scored.into_iter().map(|(_, row)| row.clone()).collect();
            (!rows.is_empty()).then_some(Group { heading: group.heading, rows })
        })
        .collect()
}

/// The rows of the groups, in the order they show (for moving through them with ↑ and ↓).
pub fn flat(groups: &[Group]) -> Vec<&Row> {
    groups.iter().flat_map(|g| g.rows.iter()).collect()
}

/// The row after (`step` 1) or before (-1) `at` among `count`, around the ends.
pub fn step(at: usize, step: i32, count: usize) -> usize {
    if count == 0 {
        return 0;
    }
    (at as i32 + step).rem_euclid(count as i32) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::View;
    use enkidu_core::db::{Project, Workspace};

    fn project(id: &str, name: &str) -> Project {
        Project {
            id: id.into(),
            name: name.into(),
            path: format!("/code/{name}"),
            live_workspace_id: None,
            created_at: 1,
        }
    }

    fn ws(id: &str, project_id: &str, name: &str, branch: &str) -> Workspace {
        Workspace {
            id: id.into(),
            project_id: project_id.into(),
            name: name.into(),
            path: format!("/ws/{name}"),
            branch: branch.into(),
            base: None,
            session_id: String::new(),
            transcript_path: None,
            claude_config_dir: None,
            created_at: 1,
            adopted: false,
        }
    }

    fn frame() -> Frame {
        let mut frame = Frame::load(
            vec![project("p1", "shop"), project("p2", "tools")],
            vec![
                ws("w1", "p1", "stock-fix", "fix/stock-double-count"),
                ws("w2", "p2", "lint", "chore/lint"),
                ws("w3", "p1", "sh-21309", "feature/SH-21309-returns"),
            ],
            Default::default(),
            &View::default(),
        );
        frame.statuses.insert("w3".into(), "needs_you".into());
        frame.projects[0].live_workspace_id = Some("w1".into());
        frame
    }

    #[test]
    fn lists_every_workspace_by_project_then_the_actions() {
        let groups = groups(&frame(), None);
        let names: Vec<(&str, Vec<&str>)> =
            groups.iter().map(|g| (g.heading, g.rows.iter().map(|r| r.name.as_str()).collect())).collect();
        assert_eq!(
            names,
            [
                ("Workspaces", vec!["stock-fix", "sh-21309", "lint"]),
                (
                    "Actions",
                    vec![
                        "New workspace",
                        "Stop Live for stock-fix",
                        "Next workspace",
                        "Previous workspace",
                        "Add project",
                        "Settings"
                    ]
                ),
            ]
        );
        let first = &groups[0].rows[0];
        assert_eq!(first.detail.as_deref(), Some("shop / fix/stock-double-count"));
        assert!(first.live);
        assert_eq!(first.key, Some("⌘1"), "the project on screen's tabs carry their keys");
        assert_eq!(groups[0].rows[1].status.as_deref(), Some("needs_you"));
        assert_eq!(groups[0].rows[2].key, None, "another project's workspace has no key");
        // remarks that wait on the workspace on screen can be typed into Claude from here (item 3.6)
        let mut waiting = frame();
        waiting.set_remarks("w1", 3);
        waiting.set_remarks("w2", 1);
        let actions = &super::groups(&waiting, None)[1].rows;
        assert_eq!(actions[1].name, "Type 3 remarks into Claude");
        assert_eq!(actions[1].pick, Pick::TypeRemarks("w1".into()));
        assert_eq!(actions.len(), 7, "only for the workspace on screen");
    }

    #[test]
    fn narrows_to_what_matches_the_best_first() {
        let groups = groups(&frame(), None);
        let names = |q: &str| -> Vec<String> { flat(&narrow(&groups, q)).iter().map(|r| r.name.clone()).collect() };
        assert_eq!(names("lint"), ["lint"]);
        assert_eq!(names("returns"), ["sh-21309"], "a branch matches");
        assert_eq!(names("tools"), ["lint"], "a project matches");
        assert!(names("zzzz").is_empty(), "nothing matches");
        assert_eq!(names("  ").len(), 9, "an empty query shows all");
        let new = names("new");
        assert_eq!(new.first().map(String::as_str), Some("New workspace"));
    }

    /// DESIGN.md, "States": the palette says where you are. The row of the workspace on screen is marked, and no
    /// other workspace's; with no workspace on screen, none.
    #[test]
    fn marks_the_workspace_on_screen() {
        let marked = |frame: &Frame| -> Vec<String> {
            let groups = groups(frame, None);
            flat(&groups)
                .into_iter()
                .filter(|row| row.current && matches!(row.pick, Pick::Workspace(_)))
                .map(|row| row.name.clone())
                .collect()
        };
        // stock-fix (w1) is on screen
        assert_eq!(marked(&frame()), ["stock-fix"]);
        let empty = Frame::load(vec![], vec![], Default::default(), &View::default());
        assert!(marked(&empty).is_empty());
    }

    /// Item 18.5: the workspaces stand in your order, with ⌘1 to ⌘9 on the tabs as they stand now; one you put away
    /// is listed too and says so, without a key (it has no tab): picking it brings it back.
    #[test]
    fn follows_your_order_and_names_a_workspace_you_put_away() {
        let mut frame = frame();
        frame.workspaces.push(ws("w4", "p1", "export", "feature/export"));
        assert!(frame.move_tab("w4", 0) && frame.hide_workspace("w3", 1));
        let groups = groups(&frame, None);
        let rows: Vec<(&str, Option<&str>, Option<&'static str>)> = flat(&groups)
            .into_iter()
            .filter(|row| matches!(row.pick, Pick::Workspace(_)))
            .map(|row| (row.name.as_str(), row.detail.as_deref(), row.key))
            .collect();
        assert_eq!(
            rows,
            [
                ("export", Some("shop / feature/export"), Some("⌘1")),
                ("stock-fix", Some("shop / fix/stock-double-count"), Some("⌘2")),
                ("sh-21309", Some("shop / feature/SH-21309-returns, hidden"), None),
                ("lint", Some("tools / chore/lint"), None),
            ]
        );
        // typed, "hidden" finds the ones you put away
        let hidden = flat(&groups).into_iter().find(|row| row.name == "sh-21309").expect("its row");
        assert!(hidden.text.ends_with("hidden"), "{}", hidden.text);
    }

    /// Item 19.4: the main folder opened as a workspace has no Live to offer (Live copies a workspace's changes
    /// into that folder), and the frame knows it by its path, which is the project's.
    #[test]
    fn the_main_folder_has_no_live_to_offer() {
        let mut frame = frame();
        let live = |frame: &Frame| {
            let groups = groups(frame, None);
            flat(&groups).into_iter().any(|row| matches!(row.pick, Pick::Live { .. }))
        };
        assert!(live(&frame), "a workspace of its own can go Live");
        let main =
            Workspace { path: "/code/shop".into(), adopted: true, ..ws("w9", "p1", "main-folder", "development") };
        assert!(!frame.is_main_folder(&frame.workspaces[0]) && frame.is_main_folder(&main));
        // the same folder under another project's name is not that project's main folder
        assert!(!frame.is_main_folder(&Workspace { project_id: "p2".into(), ..main.clone() }));
        frame.add_workspace(main);
        assert_eq!(frame.current_workspace().map(|w| w.name.as_str()), Some("main-folder"));
        assert!(!live(&frame), "the main folder is what Live copies into");
    }

    /// Item 4.2c: Live for the workspace on screen.
    #[test]
    fn offers_live_for_the_workspace_on_screen() {
        let live_row = |frame: &Frame| {
            let groups = groups(frame, None);
            flat(&groups)
                .into_iter()
                .find(|row| matches!(row.pick, Pick::Live { .. }))
                .map(|row| (row.name.clone(), row.pick.clone()))
        };
        // stock-fix (w1) is on screen and Live: the command stops it
        let mut frame = frame();
        assert_eq!(
            live_row(&frame),
            Some(("Stop Live for stock-fix".to_string(), Pick::Live { project: "p1".into(), workspace: None }))
        );
        // not Live: the command takes it Live
        frame.projects[0].live_workspace_id = None;
        assert_eq!(
            live_row(&frame),
            Some((
                "Go live with stock-fix".to_string(),
                Pick::Live { project: "p1".into(), workspace: Some("w1".into()) }
            ))
        );
        // another workspace of the project is Live: this one can still go Live (the other goes back first)
        frame.projects[0].live_workspace_id = Some("w2".into());
        assert_eq!(live_row(&frame).map(|row| row.0), Some("Go live with stock-fix".to_string()));
        // no workspace on screen: no such command
        let empty = Frame::load(vec![], vec![], Default::default(), &View::default());
        assert_eq!(live_row(&empty), None);
    }

    /// Item 4.2a: Search's three tabs are commands too, between the workspace commands and "Add project".
    #[test]
    fn offers_search_for_the_workspace_on_screen() {
        let actions = |frame: &Frame, ui: Option<&Ui>| -> Vec<(String, Option<&'static str>)> {
            let groups = groups(frame, ui);
            let actions = groups.iter().find(|group| group.heading == "Actions").expect("the actions");
            actions.rows.iter().map(|row| (row.name.clone(), row.key)).collect()
        };
        let ui = Ui::default();
        let rows = actions(&frame(), Some(&ui));
        let search: Vec<_> = rows.iter().filter(|(name, _)| name.contains('…')).cloned().collect();
        assert_eq!(
            search,
            [
                ("Go to file…".to_string(), Some("⇧⌘O")),
                ("Go to symbol (class, function, method)…".to_string(), Some("⌘O")),
                ("Find in files…".to_string(), Some("⇧⌘F")),
            ]
        );
        // they stand before "Add project"
        let at = |name: &str| rows.iter().position(|(row, _)| row == name).unwrap();
        assert!(at("Find in files…") < at("Add project") && at("Go to file…") > at("New workspace"));
        // picked, each opens its tab
        let picks: Vec<Pick> = groups(&frame(), Some(&ui))
            .into_iter()
            .flat_map(|group| group.rows)
            .filter_map(|row| matches!(row.pick, Pick::Search(_)).then_some(row.pick))
            .collect();
        use crate::search::Tab;
        assert_eq!(picks, [Pick::Search(Tab::Files), Pick::Search(Tab::Symbols), Pick::Search(Tab::Text)]);
        // typed, they are found
        let found = narrow(&groups(&frame(), Some(&ui)), "find in");
        assert_eq!(found[0].rows[0].name, "Find in files…");
        // a window that has no Search (a picture without the stores) doesn't offer it
        assert!(actions(&frame(), None).iter().all(|(name, _)| !name.contains('…')));
        // nor is there anything to search without a workspace on screen
        let mut empty = frame();
        empty.workspaces.clear();
        assert!(actions(&empty, Some(&ui)).iter().all(|(name, _)| !name.contains('…')));
    }

    /// Item 3.8: the layouts in the palette, for the workspace on screen.
    #[test]
    fn lists_the_layouts_and_says_which_is_the_current_one() {
        let layouts = |ui: &Ui| -> Vec<(String, bool, Option<&'static str>)> {
            let groups = groups(&frame(), Some(ui));
            assert_eq!(groups.iter().map(|g| g.heading).collect::<Vec<_>>(), ["Workspaces", "Actions", "Layout"]);
            groups[2].rows.iter().map(|row| (row.name.clone(), row.current, row.key)).collect()
        };
        let mut ui = Ui::default();
        assert_eq!(
            layouts(&ui),
            [
                ("Changes beside the terminal".to_string(), false, None),
                ("Changes above the terminal".to_string(), true, None),
                ("Terminal only".to_string(), false, Some("⌥⌘B")),
                ("Changes in full size".to_string(), false, Some("⇧⌘E")),
                ("Hide the changed files".to_string(), false, Some("⌥⌘1")),
                ("Show the project's files".to_string(), false, Some("⌥⌘2")),
                ("Hide the lists".to_string(), false, Some("⌘S")),
                ("Diffs in one column".to_string(), false, None),
            ]
        );
        // what each row says follows what is on screen
        ui.show(Layout::Side);
        ui.diff_layout = DiffLayout::Unified;
        let rows = layouts(&ui);
        assert!(rows[0].1 && !rows[1].1, "beside is the current one: {rows:?}");
        assert_eq!(rows[7].0, "Diffs before and after, side by side");
        assert_eq!(rows[6].0, "Hide the lists");
        ui.toggle_panel(Panel::Files);
        let rows = layouts(&ui);
        assert_eq!((rows[4].0.as_str(), rows[5].0.as_str()), ("Show the changed files", "Hide the project's files"));
        ui.terminal_only();
        let rows = layouts(&ui);
        assert_eq!(rows[2].0, "Show the changes again");
        assert!(!rows[0].1 && !rows[1].1, "neither is on screen: {rows:?}");
        ui.toggle_full_review();
        assert_eq!(layouts(&ui)[3].0, "Back to the terminal");
        // a row's pick is what it does
        let picks: Vec<Pick> = groups(&frame(), Some(&ui))[2].rows.iter().map(|row| row.pick.clone()).collect();
        assert_eq!(
            picks,
            [
                Pick::Layout(Layout::Side),
                Pick::Layout(Layout::Stack),
                Pick::TerminalOnly,
                Pick::FullReview,
                Pick::Panel(Panel::Changes),
                Pick::Panel(Panel::Files),
                Pick::Lists,
                Pick::DiffLayout
            ]
        );
        // with the lists hidden the row brings them back
        ui.toggle_lists();
        assert_eq!(layouts(&ui)[6].0, "Show the lists");
    }

    #[test]
    fn moves_around_the_ends() {
        assert_eq!(step(0, -1, 3), 2);
        assert_eq!(step(2, 1, 3), 0);
        assert_eq!(step(1, 1, 3), 2);
        assert_eq!(step(0, 1, 0), 0);
    }

    /// Item 4.2b: "Settings" is always the last command, with its key.
    #[test]
    fn with_no_project_only_add_project_and_settings() {
        let empty = Frame::load(vec![], vec![], Default::default(), &View::default());
        let groups = groups(&empty, Some(&Ui::default()));
        assert_eq!(groups.len(), 1);
        assert_eq!(
            groups[0].rows.iter().map(|r| r.pick.clone()).collect::<Vec<_>>(),
            [Pick::AddProject, Pick::Settings]
        );
        let settings = groups[0].rows.last().unwrap();
        assert_eq!((settings.name.as_str(), settings.key), ("Settings", Some("⌘,")));
    }
}
