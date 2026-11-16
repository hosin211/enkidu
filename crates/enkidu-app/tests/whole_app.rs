//! Item 1.23: the whole-app runs of phase 1. The real wiring (the core, its folder watch, the stores, the window) on
//! the sample project (`enkidu_testkit::shop`), driven through the window. Each run starts from its own copy, so the
//! runs need no order. A Claude asking a question is the core's own status call here, as Claude's hooks make it; the
//! fake Claude itself comes with phase 2.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use common::{App, said};
use enkidu_core::state::Shared;
use enkidu_testkit::git;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};

/// What every run starts from: the project with its files to copy and its setup script (which reads `ENKIDU_BRANCH`).
fn start(cx: &mut TestAppContext, prepare: impl FnOnce(&Arc<Shared>, &str, &Path)) -> App {
    common::start_on(cx, enkidu_testkit::shop, |shared, project_id, home| {
        let mut settings = shared.project_settings(project_id).unwrap();
        settings.copy_files = vec![".env".into(), ".claude/settings.local.json".into()];
        settings.setup_script = "echo \"$ENKIDU_BRANCH\" > setup-ran.txt".into();
        shared.save_project_settings(project_id, &settings).unwrap();
        prepare(shared, project_id, home);
    })
}

fn ws_path(app: &App, name: &str) -> PathBuf {
    app.workspaces.join("shop").join(name)
}

fn id_of(cx: &mut TestAppContext, app: &App, name: &str) -> String {
    app.frame.read_with(cx, |f, _| f.workspaces.iter().find(|w| w.name == name).map(|w| w.id.clone())).expect(name)
}

/// The New workspace dialog, opened by the empty screen's button, once origin was fetched.
fn open_dialog(cx: &mut TestAppContext, app: &App) {
    assert!(app.until(cx, |w| w.try_find("first-workspace").is_some() || w.try_find("new-workspace").is_some()));
    let button = cx
        .update_window(app.window, |_, w, _| {
            if w.try_find("first-workspace").is_some() { "first-workspace" } else { "new-workspace" }
        })
        .unwrap();
    app.click(cx, button);
    assert!(
        app.until(cx, |w| said(w, "ws-base-hint").as_deref() == Some("Fresh from origin")),
        "the dialog opened and fetched from origin"
    );
}

/// Existing branch, `branch` searched in the list and picked with Enter.
fn pick_existing(cx: &mut TestAppContext, app: &App, branch: &str) {
    app.click(cx, "ws-mode-existing");
    assert!(app.until(cx, |w| said(w, "ws-existing-hint").is_some()));
    cx.update_window(app.window, |_, w, cx| w.within("ws-existing").click("input", cx)).unwrap();
    assert!(app.until(cx, |_| true));
    app.typed(cx, branch);
    assert!(app.until(cx, |_| true));
    app.press(cx, "enter");
}

fn folder_hint_ends(cx: &mut TestAppContext, app: &App, end: &str) -> bool {
    let end = end.to_string();
    app.until(cx, move |w| said(w, "ws-folder").is_some_and(|f| f.ends_with(&end)))
}

#[gpui_kit::test]
fn creates_a_workspace_on_a_new_branch_from_development_copies_files_and_runs_the_setup_script(
    cx: &mut TestAppContext,
) {
    let app = start(cx, |_, _, _| {});
    open_dialog(cx, &app);
    app.typed(cx, "stock-fix");
    assert!(folder_hint_ends(cx, &app, "/shop/stock-fix"));
    app.click(cx, "create-workspace");
    assert!(app.until_frame(cx, |f| f.current_workspace().is_some_and(|w| w.name == "stock-fix")), "its tab");
    let path = ws_path(&app, "stock-fix");
    assert_eq!(git(&path, &["branch", "--show-current"]), "stock-fix");
    assert_eq!(git(&path, &["rev-parse", "HEAD"]), git(&app.project, &["rev-parse", "origin/development"]));
    assert_eq!(std::fs::read_to_string(path.join(".env")).unwrap(), "SECRET=e2e\n");
    assert!(path.join(".claude/settings.local.json").exists());
    assert_eq!(std::fs::read_to_string(path.join("setup-ran.txt")).unwrap().trim(), "stock-fix");
}

#[gpui_kit::test]
fn creates_a_workspace_directly_on_an_existing_branch_from_origin(cx: &mut TestAppContext) {
    let app = start(cx, |_, _, _| {});
    open_dialog(cx, &app);
    pick_existing(cx, &app, "feature/remote-only");
    assert!(folder_hint_ends(cx, &app, "/shop/remote-only"), "named after the branch");
    app.click(cx, "create-workspace");
    assert!(app.until_frame(cx, |f| f.current_workspace().is_some_and(|w| w.name == "remote-only")));
    let path = ws_path(&app, "remote-only");
    assert_eq!(git(&path, &["branch", "--show-current"]), "feature/remote-only");
    assert_eq!(git(&path, &["rev-parse", "--abbrev-ref", "@{upstream}"]), "origin/feature/remote-only");
}

#[gpui_kit::test]
fn shows_the_projects_workspaces_as_tabs_says_who_needs_you_in_words_and_cmd_1_to_9_follow_the_tabs(
    cx: &mut TestAppContext,
) {
    let app = start(cx, |shared, project, _| {
        App::make(shared, project, "stock-fix", "new", "stock-fix", Some("development"));
        App::make(shared, project, "remote-only", "existing", "feature/remote-only", None);
    });
    app.click(cx, ("tab", 0usize));
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        // the top row: the project by its name alone (its workspaces are the tabs under it), and no search field
        let shop = said(w, ("project", 0usize)).unwrap_or_default();
        assert!(shop.contains("shop") && !shop.chars().any(|c| c.is_ascii_digit()), "{shop}");
        assert!(w.try_find("open-search").is_none());
    })
    .unwrap();
    // remote-only asks a question while you look at stock-fix: its tab says so, in red words
    let remote = id_of(cx, &app, "remote-only");
    app.shared.set_status(&app.sink, &remote, "needs_you");
    assert!(
        app.until(cx, |w| said(w, ("tab-words", 1usize)).as_deref() == Some("Needs you")),
        "the tab shows the question"
    );
    // ⌘2 is the second tab, and the status line says the same in a sentence
    app.press(cx, "cmd-2");
    assert!(app.until(cx, |w| said(w, "status-claude").as_deref() == Some("Claude needs you")));
    cx.update_window(app.window, |_, w, _| {
        assert_eq!(said(w, "status-branch").as_deref(), Some("feature/remote-only"))
    })
    .unwrap();
    // answered, the words go
    app.shared.set_status(&app.sink, &remote, "working");
    assert!(app.until(cx, |w| said(w, ("tab-words", 1usize)).as_deref() != Some("Needs you")));
    // ⌘1 goes back to the first tab
    app.press(cx, "cmd-1");
    assert!(app.until_frame(cx, |f| f.current_workspace().is_some_and(|w| w.name == "stock-fix")));
}

#[gpui_kit::test]
fn removes_a_workspace_after_asking_about_its_uncommitted_work_and_keeps_the_branch(cx: &mut TestAppContext) {
    let app = start(cx, |shared, project, _| {
        App::make(shared, project, "remote-only", "existing", "feature/remote-only", None)
    });
    std::fs::write(ws_path(&app, "remote-only").join("src/new.py"), "print('unsaved')\n").unwrap();
    app.click(cx, ("tab", 0usize));
    app.click(cx, "ws-menu");
    assert!(app.until(cx, |w| w.try_find("remove-workspace").is_some()));
    app.click(cx, "remove-workspace");
    assert!(app.until(cx, |w| w.try_find("confirm-remove").is_some()));
    app.click(cx, "confirm-remove");
    assert!(app.until(cx, |w| w.try_find("dirty-warning").is_some()), "its uncommitted work is said");
    app.click(cx, "confirm-remove");
    assert!(app.until_frame(cx, |f| f.workspaces.is_empty()), "its tab is gone");
    assert!(!ws_path(&app, "remote-only").exists());
    assert!(git(&app.project, &["branch", "--list", "feature/remote-only"]).contains("feature/remote-only"));
}

/// A branch that is checked out in a folder another tool made: git refuses to check it out again.
fn other_folder(app: &App) -> PathBuf {
    let elsewhere = app.home.path().join("elsewhere").join("harbor");
    git(
        &app.project,
        &["worktree", "add", "-q", "-b", "feature/elsewhere", &elsewhere.to_string_lossy(), "origin/development"],
    );
    PathBuf::from(git(&elsewhere, &["rev-parse", "--show-toplevel"]))
}

/// Picks a branch that is taken: Existing branch, feature/elsewhere, named `name`, Create workspace.
fn pick_taken_branch(cx: &mut TestAppContext, app: &App, name: Option<&str>) {
    open_dialog(cx, app);
    pick_existing(cx, app, "feature/elsewhere");
    assert!(folder_hint_ends(cx, app, "/shop/elsewhere"), "named after the branch");
    if let Some(name) = name {
        app.click(cx, "ws-name");
        app.press(cx, "cmd-a");
        app.typed(cx, name);
        assert!(folder_hint_ends(cx, app, &format!("/shop/{name}")));
    }
    app.click(cx, "create-workspace");
    assert!(app.until(cx, |w| w.try_find("branch-taken").is_some()), "the dialog offers the two ways");
}

#[gpui_kit::test]
fn offers_the_folder_a_branch_is_checked_out_in_opened_as_a_workspace_and_removing_it_only_forgets_it(
    cx: &mut TestAppContext,
) {
    let app = start(cx, |_, _, _| {});
    let elsewhere = other_folder(&app);
    pick_taken_branch(cx, &app, Some("from-elsewhere"));
    cx.update_window(app.window, |_, w, _| {
        assert!(said(w, "branch-taken").unwrap_or_default().starts_with("feature/elsewhere is checked out in"));
        // a choice, not an error
        assert!(w.try_find("create-error").is_none());
    })
    .unwrap();
    app.click(cx, "taken-open");
    assert!(app.until_frame(cx, |f| f.current_workspace().is_some_and(|w| w.name == "from-elsewhere")));
    let ws = app.frame.read_with(cx, |f, _| f.current_workspace().cloned()).unwrap();
    assert_eq!(PathBuf::from(&ws.path).canonicalize().unwrap(), elsewhere.canonicalize().unwrap());
    assert_eq!((ws.branch.as_str(), ws.adopted), ("feature/elsewhere", true));
    // removing it forgets it; the folder and its branch stay as they were
    std::fs::write(elsewhere.join("mine.txt"), "not Enkidu's to delete\n").unwrap();
    app.click(cx, "ws-menu");
    assert!(app.until(cx, |w| w.try_find("remove-workspace").is_some()));
    app.click(cx, "remove-workspace");
    assert!(app.until(cx, |w| said(w, "remove-forgets").is_some_and(|s| s.contains("stay as they are"))));
    app.click(cx, "confirm-remove");
    assert!(app.until_frame(cx, |f| f.workspaces.is_empty()));
    assert_eq!(std::fs::read_to_string(elsewhere.join("mine.txt")).unwrap(), "not Enkidu's to delete\n");
    assert_eq!(git(&elsewhere, &["branch", "--show-current"]), "feature/elsewhere");
    assert!(git(&app.project, &["worktree", "list"]).contains("harbor"));
}

#[gpui_kit::test]
fn or_starts_a_new_branch_from_it_in_a_workspace_of_its_own(cx: &mut TestAppContext) {
    let app = start(cx, |_, _, _| {});
    let elsewhere = other_folder(&app);
    pick_taken_branch(cx, &app, None);
    assert!(app.until(cx, |w| w.try_find("taken-new-branch").is_some()));
    app.click(cx, "taken-new-branch");
    assert!(app.until_frame(cx, |f| f.current_workspace().is_some_and(|w| w.name == "elsewhere")));
    let path = ws_path(&app, "elsewhere");
    assert_eq!(git(&path, &["branch", "--show-current"]), "feature/elsewhere-2", "the first free name");
    assert_eq!(git(&path, &["rev-parse", "HEAD"]), git(&elsewhere, &["rev-parse", "HEAD"]));
    assert!(!app.frame.read_with(cx, |f, _| f.current_workspace().unwrap().adopted));
    // the other folder still has its branch
    assert_eq!(git(&elsewhere, &["branch", "--show-current"]), "feature/elsewhere");
}

#[gpui_kit::test]
fn follows_the_branch_its_folder_moves_to(cx: &mut TestAppContext) {
    let app = start(cx, |shared, project, _| {
        App::make(shared, project, "elsewhere", "new", "feature/elsewhere", Some("development"))
    });
    git(&ws_path(&app, "elsewhere"), &["checkout", "-q", "-b", "feature/elsewhere-moved"]);
    assert!(
        app.until_frame(cx, |f| f.workspaces.iter().any(|w| w.branch == "feature/elsewhere-moved")),
        "the workspace kept its old branch"
    );
    assert!(app.until(cx, |w| said(w, "status-branch").as_deref() == Some("feature/elsewhere-moved")), "and says it");
}
