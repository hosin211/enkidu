//! Item 1.22b: the workspace's menu (⋯ at the right of the tabs row) and removing a workspace, on a real core. A
//! workspace made here loses its folder and keeps its branch, after its uncommitted work is said; a folder made
//! elsewhere is only forgotten; the menu copies the path and the branch name.

mod common;

use common::{App, said};
use enkidu_testkit::git;
use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{AppContext as _, TestAppContext};

fn open_menu_and_remove(cx: &mut TestAppContext, app: &App) {
    app.click(cx, "ws-menu");
    assert!(app.until(cx, |w| w.try_find("remove-workspace").is_some()), "the menu opened");
    app.click(cx, "remove-workspace");
    assert!(app.until(cx, |w| w.try_find("remove-dialog").is_some()), "the dialog asks first");
}

#[gpui_kit::test]
fn removes_a_workspace_after_saying_its_uncommitted_work_and_keeps_the_branch(cx: &mut TestAppContext) {
    let app = common::start(cx, |shared, project, _| {
        App::make(shared, project, "stock-fix", "new", "stock-fix", Some("main"))
    });
    let folder = app.workspaces.join("shop").join("stock-fix");
    std::fs::write(folder.join("unsaved.txt"), "work in progress\n").unwrap();
    open_menu_and_remove(cx, &app);
    cx.update_window(app.window, |_, w, _| {
        let words = said(w, "remove-deletes").unwrap_or_default();
        assert!(words.contains("This deletes the folder") && words.contains("The branch stock-fix stays."), "{words}");
    })
    .unwrap();
    app.click(cx, "confirm-remove");
    assert!(
        app.until(cx, |w| said(w, "dirty-warning").as_deref()
            == Some("It has uncommitted changes. Removing it deletes them.")),
        "the uncommitted work is said, nothing removed yet"
    );
    assert!(folder.exists());
    assert!(app.until(cx, |w| said(w, "confirm-remove").as_deref() == Some("Remove anyway")));
    app.click(cx, "confirm-remove");
    assert!(app.until_frame(cx, |f| f.workspaces.is_empty()), "the tab is gone");
    assert!(app.until(cx, |w| w.try_find("remove-dialog").is_none() && said(w, "toast-detail").is_some()));
    cx.update_window(app.window, |_, w, _| {
        assert_eq!(said(w, "toast-detail").as_deref(), Some("The branch stock-fix is still there."));
    })
    .unwrap();
    assert!(!folder.exists(), "the folder is deleted");
    assert_eq!(
        git(&app.project, &["branch", "--list", "stock-fix"]).trim_start_matches("* ").trim(),
        "stock-fix",
        "the branch stays"
    );
}

#[gpui_kit::test]
fn a_folder_made_elsewhere_is_only_forgotten(cx: &mut TestAppContext) {
    let app = common::start(cx, |shared, project_id, home| {
        let elsewhere = home.join("elsewhere");
        let project = shared.db.project(project_id).unwrap().unwrap();
        git(
            std::path::Path::new(&project.path),
            &["worktree", "add", "-q", "-b", "feature/elsewhere", &elsewhere.to_string_lossy(), "main"],
        );
        enkidu_core::commands::adopt_workspace(
            enkidu_core::events::Events::none(),
            shared,
            project_id.into(),
            elsewhere.to_string_lossy().into(),
            None,
            Some("from-elsewhere".into()),
        )
        .expect("adopted");
    });
    let elsewhere = app.home.path().join("elsewhere");
    std::fs::write(elsewhere.join("mine.txt"), "not Enkidu's to delete\n").unwrap();
    open_menu_and_remove(cx, &app);
    cx.update_window(app.window, |_, w, _| {
        assert!(said(w, "remove-forgets").unwrap_or_default().contains("stay as they are"));
    })
    .unwrap();
    app.click(cx, "confirm-remove");
    assert!(app.until_frame(cx, |f| f.workspaces.is_empty()), "forgotten");
    assert_eq!(std::fs::read_to_string(elsewhere.join("mine.txt")).unwrap(), "not Enkidu's to delete\n");
    assert_eq!(git(&elsewhere, &["branch", "--show-current"]), "feature/elsewhere");
    assert!(git(&app.project, &["worktree", "list"]).contains("elsewhere"));
}

#[gpui_kit::test]
fn the_menu_copies_the_path_and_the_branch_name(cx: &mut TestAppContext) {
    let app = common::start(cx, |shared, project, _| {
        App::make(shared, project, "stock-fix", "new", "fix/stock", Some("main"))
    });
    let (_, branch, path) = app.on_screen(cx).expect("on screen");
    app.click(cx, "ws-menu");
    assert!(app.until(cx, |w| w.try_find("ws-copy-path").is_some()));
    app.click(cx, "ws-copy-path");
    assert_eq!(cx.read_from_clipboard().and_then(|c| c.text()), Some(path));
    assert!(app.until(cx, |w| !w.find_all_by_label("Path copied").is_empty()), "a toast says so");
    app.click(cx, "ws-menu");
    assert!(app.until(cx, |w| w.try_find("ws-copy-branch").is_some()));
    app.click(cx, "ws-copy-branch");
    assert_eq!(cx.read_from_clipboard().and_then(|c| c.text()), Some(branch));
    assert!(app.until(cx, |w| !w.find_all_by_label("Branch name copied").is_empty()), "a toast says so");
}
