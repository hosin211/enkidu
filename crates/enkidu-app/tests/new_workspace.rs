//! Item 1.19: the New workspace dialog, on a real core in a temporary folder and a throwaway project with an origin.
//! ⌘N opens it for the project on screen; a name and Enter make a workspace on a new branch from the base, and it
//! comes on screen; the steps show while it is made; a branch checked out in another folder offers that folder as
//! it is, or a new branch from it; a name that won't do is said, and Create waits.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use enkidu_app::bridge::Bridge;
use enkidu_app::shell::Shell;
use enkidu_core::events::Events;
use enkidu_core::state::Shared;
use enkidu_model::frame::Frame;
use enkidu_testkit::git;
use futures::channel::mpsc::unbounded;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, Entity, TestAppContext, Window, WindowBounds, WindowOptions, px, size,
};

struct Setup {
    home: tempfile::TempDir,
    project: PathBuf,
    workspaces: PathBuf,
    shared: Arc<Shared>,
    frame: Entity<Frame>,
    window: AnyWindowHandle,
}

/// Runs the app until `seen` holds for the window's last frame (the core answers on threads of its own).
fn until(cx: &mut TestAppContext, window: AnyWindowHandle, seen: impl Fn(&Window) -> bool) -> bool {
    for _ in 0..600 {
        cx.run_until_parked();
        let ok = cx
            .update_window(window, |_, w, cx| {
                w.render_frame(cx);
                seen(w)
            })
            .unwrap();
        if ok {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

fn said(window: &Window, id: &'static str) -> Option<String> {
    window.try_find(id).and_then(|e| e.label().map(String::from))
}

/// A core whose workspaces folder is a temporary one, the project "shop" (with an origin) added to it, the stores
/// hearing its news, and the window on them.
fn start(cx: &mut TestAppContext) -> Setup {
    let home = tempfile::tempdir().unwrap();
    let project = enkidu_testkit::repo_with_origin(home.path(), "shop");
    let workspaces = home.path().join("workspaces");
    let shared = Arc::new(Shared::new(home.path().join("data"), Instant::now()).unwrap());
    shared.user.lock().unwrap().workspaces_dir = workspaces.to_string_lossy().into();
    enkidu_core::commands::add_project(&shared, project.to_string_lossy().into()).expect("the project is added");
    let (news, events) = unbounded();
    let sink = Events::new(move |event| {
        let _ = news.unbounded_send(event);
    });
    let frame = cx.update(|cx| {
        enkidu_app::init(cx);
        Bridge::install(cx, shared.clone(), sink);
        enkidu_app::stores::install_for_tests(events, Default::default(), Default::default(), cx)
    });
    let on = frame.clone();
    let (window, _) = cx
        .update(|cx| {
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: Default::default(),
                        size: size(px(1360.), px(860.)),
                    })),
                    ..Default::default()
                },
                cx,
                |window, cx| cx.new(|cx| Shell::kept(on, window, cx)),
            )
        })
        .expect("a test window");
    // the window is in front, as the app's is: GPUI tells an input it lost the keyboard only in an active window
    cx.update_window(window, |_, w, _| w.activate_window()).unwrap();
    cx.executor().allow_parking();
    assert!(until(cx, window, |_| true));
    for _ in 0..300 {
        cx.run_until_parked();
        if frame.read_with(cx, |f, _| !f.projects.is_empty()) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Setup { home, project, workspaces, shared, frame, window }
}

fn press(cx: &mut TestAppContext, window: AnyWindowHandle, key: &str) {
    cx.update_window(window, |_, w, cx| w.press(key, cx)).unwrap();
}

fn typed(cx: &mut TestAppContext, window: AnyWindowHandle, text: &str) {
    cx.update_window(window, |_, w, cx| w.input(text, cx)).unwrap();
}

/// Opens the dialog with ⌘N and waits until origin was asked for its branches.
fn open_dialog(cx: &mut TestAppContext, setup: &Setup) {
    press(cx, setup.window, "cmd-n");
    assert!(
        until(cx, setup.window, |w| said(w, "ws-base-hint").as_deref() == Some("Fresh from origin")),
        "the dialog opened and fetched from origin"
    );
}

fn head(dir: &Path) -> String {
    git(dir, &["rev-parse", "HEAD"])
}

#[gpui_kit::test]
fn a_new_branch_from_the_base_becomes_a_workspace_on_screen(cx: &mut TestAppContext) {
    let setup = start(cx);
    open_dialog(cx, &setup);
    // a dialog's fields are 32 px tall (DESIGN.md), the branch picker with them
    cx.update_window(setup.window, |_, w, cx| {
        w.render_frame(cx);
        for id in ["ws-name", "ws-branch-name", "ws-base"] {
            assert_eq!(w.find(id).bounds().size.height, gpui_kit::px(32.), "{id}");
        }
    })
    .unwrap();
    // the name has the keyboard; the hint says where the folder will be
    typed(cx, setup.window, "stock-fix");
    let folder = setup.workspaces.join("shop").join("stock-fix");
    assert!(
        until(cx, setup.window, |w| said(w, "ws-folder").is_some_and(|f| f.ends_with("/workspaces/shop/stock-fix"))),
        "{:?}",
        cx.update_window(setup.window, |_, w, _| said(w, "ws-folder")).unwrap()
    );
    press(cx, setup.window, "enter");
    assert!(until(cx, setup.window, |w| w.try_find("new-workspace-dialog").is_none()), "the dialog closed");
    let on_screen = setup.frame.read_with(cx, |f, _| f.current_workspace().map(|w| (w.name.clone(), w.branch.clone())));
    assert_eq!(on_screen, Some(("stock-fix".into(), "stock-fix".into())), "the new workspace is on screen");
    // a worktree on its own branch, from origin's main
    assert_eq!(git(&folder, &["rev-parse", "--abbrev-ref", "HEAD"]), "stock-fix");
    assert_eq!(head(&folder), git(&setup.project, &["rev-parse", "origin/main"]));
    assert_eq!(setup.shared.db.workspaces().unwrap().len(), 1, "the core keeps it");
    let _ = &setup.home;
}

#[gpui_kit::test]
fn the_steps_show_while_it_is_made(cx: &mut TestAppContext) {
    let setup = start(cx);
    open_dialog(cx, &setup);
    let project_id = setup.frame.read_with(cx, |f, _| f.projects[0].id.clone());
    // the core's news of the steps, as it sends it while making a workspace (another project's is not this one's)
    let news = |project: &str, step: &str, state: &str, detail: &str| enkidu_core::workspace::Progress {
        project_id: project.into(),
        step: step.into(),
        state: state.into(),
        detail: detail.into(),
    };
    let steps = [
        news(&project_id, "check", "done", ""),
        news(&project_id, "fetch", "skipped", "No origin"),
        news(&project_id, "worktree", "running", ""),
        news("another", "copy", "running", ""),
    ];
    cx.update(|cx| {
        enkidu_app::stores::Stores::progress(cx).update(cx, |log, cx| {
            log.0.extend(steps);
            cx.notify();
        })
    });
    assert!(until(cx, setup.window, |w| said(w, "step-worktree").as_deref() == Some("Create the worktree: running")));
    cx.update_window(setup.window, |_, w, _| {
        assert_eq!(said(w, "step-check").as_deref(), Some("Check name and branch: done"));
        assert_eq!(said(w, "step-fetch").as_deref(), Some("Fetch from origin: skipped"));
        assert_eq!(said(w, "step-copy"), None, "another project's step");
    })
    .unwrap();
}

/// The project's `feature/x` checked out in a folder of its own, as a folder another tool made can have one.
fn checked_out_elsewhere(setup: &Setup) -> PathBuf {
    let elsewhere = setup.home.path().join("elsewhere");
    git(&setup.project, &["worktree", "add", "-q", &elsewhere.to_string_lossy(), "feature/x"]);
    PathBuf::from(git(&elsewhere, &["rev-parse", "--show-toplevel"]))
}

/// Existing branch, `feature/x` picked from the list (searched, then Enter), and Create workspace.
fn ask_for_feature_x(cx: &mut TestAppContext, setup: &Setup) {
    open_dialog(cx, setup);
    cx.update_window(setup.window, |_, w, cx| w.click("ws-mode-existing", cx)).unwrap();
    assert!(
        until(cx, setup.window, |w| said(w, "ws-existing-hint").as_deref()
            == Some("Local branches and branches on origin")),
        "Existing branch is chosen"
    );
    cx.update_window(setup.window, |_, w, cx| w.within("ws-existing").click("input", cx)).unwrap();
    assert!(until(cx, setup.window, |_| true));
    typed(cx, setup.window, "feature/x");
    assert!(until(cx, setup.window, |_| true));
    press(cx, setup.window, "enter");
    // named after the branch
    assert!(
        until(cx, setup.window, |w| said(w, "ws-folder").is_some_and(|f| f.ends_with("/workspaces/shop/x"))),
        "{:?}",
        cx.update_window(setup.window, |_, w, _| said(w, "ws-folder")).unwrap()
    );
    cx.update_window(setup.window, |_, w, cx| w.click("create-workspace", cx)).unwrap();
}

#[gpui_kit::test]
fn a_branch_checked_out_elsewhere_can_be_opened_there_as_it_is(cx: &mut TestAppContext) {
    let setup = start(cx);
    let elsewhere = checked_out_elsewhere(&setup);
    ask_for_feature_x(cx, &setup);
    assert!(
        until(cx, setup.window, |w| said(w, "branch-taken")
            .is_some_and(|s| s.starts_with("feature/x is checked out in"))),
        "the dialog says where the branch is"
    );
    cx.update_window(setup.window, |_, w, cx| w.click("taken-open", cx)).unwrap();
    assert!(until(cx, setup.window, |w| w.try_find("new-workspace-dialog").is_none()), "the dialog closed");
    let on_screen = setup.frame.read_with(cx, |f, _| f.current_workspace().map(|w| (w.name.clone(), w.path.clone())));
    let (name, path) = on_screen.expect("a workspace on screen");
    assert_eq!(name, "x");
    assert_eq!(PathBuf::from(path).canonicalize().unwrap(), elsewhere.canonicalize().unwrap(), "that folder, as it is");
}

#[gpui_kit::test]
fn a_branch_checked_out_elsewhere_can_give_a_new_branch_from_it(cx: &mut TestAppContext) {
    let setup = start(cx);
    checked_out_elsewhere(&setup);
    ask_for_feature_x(cx, &setup);
    assert!(until(cx, setup.window, |w| w.try_find("taken-new-branch").is_some()));
    // the new branch offered is the first free one: feature/x-2
    cx.update_window(setup.window, |_, w, cx| w.click("taken-new-branch", cx)).unwrap();
    assert!(until(cx, setup.window, |w| w.try_find("new-workspace-dialog").is_none()), "the dialog closed");
    let on_screen = setup
        .frame
        .read_with(cx, |f, _| f.current_workspace().map(|w| (w.name.clone(), w.branch.clone(), w.path.clone())));
    let (name, branch, path) = on_screen.expect("a workspace on screen");
    assert_eq!((name.as_str(), branch.as_str()), ("x", "feature/x-2"));
    assert_eq!(
        git(Path::new(&path), &["rev-parse", "HEAD"]),
        git(&setup.project, &["rev-parse", "feature/x"]),
        "from where it is"
    );
}

/// Every file of a folder outside its `.git`, with what is in it.
fn files_of(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut found = Vec::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(at) = todo.pop() {
        for entry in std::fs::read_dir(&at).unwrap().flatten() {
            let path = entry.path();
            if path.file_name().is_some_and(|name| name == ".git") {
                continue;
            }
            if path.is_dir() {
                todo.push(path);
            } else {
                found.push((path.strip_prefix(dir).unwrap().to_path_buf(), std::fs::read(&path).unwrap()));
            }
        }
    }
    found.sort();
    found
}

/// What git knows of a folder: where it is, its branches, its worktrees, and what is changed in it.
fn git_state(dir: &Path) -> [String; 4] {
    [
        head(dir),
        git(dir, &["branch", "--list", "--all"]),
        git(dir, &["worktree", "list", "--porcelain"]),
        git(dir, &["status", "--porcelain"]),
    ]
}

/// The dialog on "The main folder": opened with ⌘N, the second answer clicked.
fn ask_for_the_main_folder(cx: &mut TestAppContext, setup: &Setup) {
    open_dialog(cx, setup);
    cx.update_window(setup.window, |_, w, cx| w.click("ws-where-main", cx)).unwrap();
    assert!(
        until(cx, setup.window, |w| w.try_find("ws-where-main").is_some_and(|main| main.selected() == Some(true))),
        "The main folder is chosen"
    );
}

/// Item 19.3: work in a project's main folder, without making a workspace. The dialog asks where Claude works. "The
/// main folder" shows the folder and what working there means,
/// and Enter opens it: the project's own folder is the workspace, as it is (no worktree, no branch, no file made or
/// changed in it, an edit of your own left alone). Asked for again, the one that is open comes on screen.
#[gpui_kit::test]
fn the_main_folder_is_opened_as_a_workspace_and_nothing_is_made(cx: &mut TestAppContext) {
    let setup = start(cx);
    // your own work in the main folder: an edit that isn't committed, and a file git doesn't know
    std::fs::write(setup.project.join("README.md"), "# shop\n\nmine, not committed\n").unwrap();
    std::fs::write(setup.project.join("notes.txt"), "mine\n").unwrap();
    let (files, state) = (files_of(&setup.project), git_state(&setup.project));
    open_dialog(cx, &setup);
    // the question comes first, and a new workspace is the answer until another is given
    cx.update_window(setup.window, |_, w, cx| {
        w.render_frame(cx);
        assert_eq!(said(w, "ws-where").as_deref(), Some("Where Claude works"));
        assert_eq!(said(w, "ws-where-new").as_deref(), Some("A new workspace"));
        assert_eq!(said(w, "ws-where-main").as_deref(), Some("The main folder"));
        assert_eq!(w.find("ws-where-new").selected(), Some(true));
        assert_eq!(w.find("ws-where-main").selected(), Some(false));
        assert_eq!(
            w.find("new-workspace-dialog").description(),
            Some("A worktree of shop on its own branch, with Claude in it.")
        );
        assert_eq!(said(w, "create-workspace").as_deref(), Some("Create workspace"));
        // the question is above the name
        assert!(w.find("ws-where").bounds().bottom() <= w.find("ws-name").bounds().top());
    })
    .unwrap();
    // a name typed for a new workspace stays there while the other answer is looked at
    typed(cx, setup.window, "stock-fix");
    assert!(until(cx, setup.window, |w| said(w, "ws-folder").is_some_and(|f| f.ends_with("/shop/stock-fix"))));
    cx.update_window(setup.window, |_, w, cx| w.click("ws-where-main", cx)).unwrap();
    assert!(until(cx, setup.window, |w| w.try_find("ws-main-says").is_some()), "the main folder's side shows");
    cx.update_window(setup.window, |_, w, _| {
        assert_eq!(w.find("ws-where-main").selected(), Some(true));
        // nothing of a new workspace is asked: no name, no branch
        for gone in ["ws-name", "ws-folder", "ws-branch-box", "ws-branch-name", "ws-base", "ws-mode-new"] {
            assert!(w.try_find(gone).is_none(), "{gone} is for a new workspace");
        }
        assert!(said(w, "ws-main-folder").is_some_and(|folder| folder.ends_with("/shop")), "the folder is named");
        assert_eq!(
            said(w, "ws-main-says").as_deref(),
            Some(
                "Claude works in the project's own folder, on the branch it is on. Nothing is made or copied \
                 there, and removing the workspace later only forgets it."
            )
        );
        assert_eq!(w.find("new-workspace-dialog").description(), Some("Claude in shop's main folder, as it is."));
        assert_eq!(said(w, "create-workspace").as_deref(), Some("Open the main folder"));
    })
    .unwrap();
    // back to a new workspace: the name is as it was typed, and it has the keyboard again
    cx.update_window(setup.window, |_, w, cx| w.click("ws-where-new", cx)).unwrap();
    assert!(until(cx, setup.window, |w| said(w, "ws-folder").is_some_and(|f| f.ends_with("/shop/stock-fix"))));
    typed(cx, setup.window, "-2");
    assert!(
        until(cx, setup.window, |w| said(w, "ws-folder").is_some_and(|f| f.ends_with("/shop/stock-fix-2"))),
        "typing goes to the name: {:?}",
        cx.update_window(setup.window, |_, w, _| said(w, "ws-folder")).unwrap()
    );
    cx.update_window(setup.window, |_, w, cx| w.click("ws-where-main", cx)).unwrap();
    assert!(until(cx, setup.window, |w| w.try_find("ws-main-says").is_some()));
    // Enter is the dialog's OK here too, though no field has the keyboard
    press(cx, setup.window, "enter");
    assert!(until(cx, setup.window, |w| w.try_find("new-workspace-dialog").is_none()), "the dialog closed");
    let on_screen = setup.frame.read_with(cx, |f, _| {
        f.current_workspace()
            .map(|w| (w.name.clone(), w.branch.clone(), w.path.clone(), w.adopted, f.is_main_folder(w)))
    });
    let (name, branch, path, adopted, is_main) = on_screen.expect("a workspace on screen");
    assert_eq!(
        (name.as_str(), branch.as_str()),
        ("main-folder", "main"),
        "named for what it is, on the folder's branch"
    );
    assert_eq!(
        PathBuf::from(&path).canonicalize().unwrap(),
        setup.project.canonicalize().unwrap(),
        "the project's own folder"
    );
    assert!(adopted && is_main, "a folder Enkidu didn't make, known as the main folder");
    // nothing was made or changed: not in the folder, not in git, and no folder of workspaces either
    assert_eq!(files_of(&setup.project), files, "every file as it was, your edit and your new file too");
    assert_eq!(git_state(&setup.project), state, "the same commit, branches, worktrees and changes");
    assert!(!setup.workspaces.exists(), "no workspace folder was made");
    assert_eq!(setup.shared.db.workspaces().unwrap().len(), 1, "the core keeps it");
    // asked for again, the one that is open is the one on screen: no second workspace. Also when it was put away
    // meanwhile: it comes back to the tabs
    setup.frame.update(cx, |f, cx| {
        let id = f.workspaces[0].id.clone();
        assert!(f.hide_workspace(&id, 1));
        cx.notify();
    });
    assert_eq!(setup.frame.read_with(cx, |f, _| (f.tabs().len(), f.hidden_tabs().len())), (0, 1), "it is hidden");
    ask_for_the_main_folder(cx, &setup);
    cx.update_window(setup.window, |_, w, cx| w.click("create-workspace", cx)).unwrap();
    assert!(until(cx, setup.window, |w| w.try_find("new-workspace-dialog").is_none()), "the dialog closed again");
    let all = setup.shared.db.workspaces().unwrap();
    assert_eq!(all.iter().map(|w| w.name.as_str()).collect::<Vec<_>>(), ["main-folder"]);
    assert_eq!(setup.frame.read_with(cx, |f, _| f.workspaces.len()), 1, "and the window has it once");
    let back = setup
        .frame
        .read_with(cx, |f, _| (f.tabs().len(), f.hidden_tabs().len(), f.current_workspace().map(|w| w.name.clone())));
    assert_eq!(back, (1, 0, Some("main-folder".into())), "back among the tabs, and on screen");
    assert_eq!(git_state(&setup.project), state);
}

/// Item 19.3: what a try at a new workspace left in the dialog (its steps, what went wrong) is not shown on the main
/// folder's side, where it would read as the main folder's own.
#[gpui_kit::test]
fn a_failed_try_at_a_new_workspace_is_not_shown_beside_the_main_folder(cx: &mut TestAppContext) {
    let setup = start(cx);
    open_dialog(cx, &setup);
    let project_id = setup.frame.read_with(cx, |f, _| f.projects[0].id.clone());
    // the steps of a try, as the core sends them
    cx.update(|cx| {
        enkidu_app::stores::Stores::progress(cx).update(cx, |log, cx| {
            log.0.push(enkidu_core::workspace::Progress {
                project_id,
                step: "check".into(),
                state: "failed".into(),
                detail: "The branch is taken".into(),
            });
            cx.notify();
        })
    });
    assert!(until(cx, setup.window, |w| w.try_find("step-check").is_some()), "the step shows");
    cx.update_window(setup.window, |_, w, cx| w.click("ws-where-main", cx)).unwrap();
    assert!(until(cx, setup.window, |w| w.try_find("ws-main-says").is_some()));
    cx.update_window(setup.window, |_, w, _| assert!(w.try_find("step-check").is_none(), "not the main folder's step"))
        .unwrap();
    assert!(setup.shared.db.workspaces().unwrap().is_empty());
}

/// Item 19.3: a main folder that is on no branch (a commit checked out by itself) is not opened. The dialog says so
/// in the core's words and stays, nothing is kept, and once the folder is on a branch the same button opens it.
#[gpui_kit::test]
fn a_main_folder_on_no_branch_is_said_and_nothing_is_opened(cx: &mut TestAppContext) {
    let setup = start(cx);
    git(&setup.project, &["checkout", "-q", "--detach"]);
    ask_for_the_main_folder(cx, &setup);
    cx.update_window(setup.window, |_, w, cx| w.click("create-workspace", cx)).unwrap();
    assert!(until(cx, setup.window, |w| said(w, "create-error").is_some()), "what went wrong is said");
    cx.update_window(setup.window, |_, w, _| {
        let error = said(w, "create-error").unwrap_or_default();
        assert!(error.ends_with("/shop isn't on a branch"), "{error}");
        assert!(w.try_find("new-workspace-dialog").is_some(), "the dialog stays");
        assert_eq!(said(w, "create-workspace").as_deref(), Some("Open the main folder"), "to try again");
    })
    .unwrap();
    assert!(setup.shared.db.workspaces().unwrap().is_empty(), "nothing was opened");
    git(&setup.project, &["checkout", "-q", "main"]);
    cx.update_window(setup.window, |_, w, cx| w.click("create-workspace", cx)).unwrap();
    assert!(until(cx, setup.window, |w| w.try_find("new-workspace-dialog").is_none()), "the dialog closed");
    let on_screen = setup.frame.read_with(cx, |f, _| f.current_workspace().map(|w| (w.name.clone(), w.branch.clone())));
    assert_eq!(on_screen, Some(("main-folder".into(), "main".into())));
}

/// Item 19.4: Live copies a workspace's changes into the main folder, so the main folder itself has no Go live
/// beside it, while a workspace of its own in the same project has. And Remove only forgets the main folder.
#[gpui_kit::test]
fn the_main_folder_has_no_go_live_and_is_only_forgotten(cx: &mut TestAppContext) {
    let setup = start(cx);
    // a workspace of its own first: Go live is beside it
    open_dialog(cx, &setup);
    typed(cx, setup.window, "stock-fix");
    assert!(until(cx, setup.window, |w| said(w, "ws-folder").is_some_and(|f| f.ends_with("/shop/stock-fix"))));
    press(cx, setup.window, "enter");
    assert!(until(cx, setup.window, |w| w.try_find("new-workspace-dialog").is_none()));
    assert!(until(cx, setup.window, |w| w.try_find("live-button").is_some()), "a workspace of its own can go Live");
    // the main folder beside it
    ask_for_the_main_folder(cx, &setup);
    press(cx, setup.window, "enter");
    assert!(until(cx, setup.window, |w| w.try_find("new-workspace-dialog").is_none()));
    let names = |cx: &mut TestAppContext| {
        setup.frame.read_with(cx, |f, _| f.tabs().iter().map(|w| w.name.clone()).collect::<Vec<_>>())
    };
    assert_eq!(names(cx), ["stock-fix", "main-folder"], "a tab like any other, after the one that was there");
    assert!(
        until(cx, setup.window, |w| w.try_find("ws-menu").is_some() && w.try_find("live-button").is_none()),
        "no Go live beside the main folder"
    );
    // the other tab still has it
    let first = setup.frame.read_with(cx, |f, _| f.tabs()[0].id.clone());
    setup.frame.update(cx, |f, cx| {
        f.select_workspace(&first);
        cx.notify();
    });
    assert!(until(cx, setup.window, |w| w.try_find("live-button").is_some()), "stock-fix has Go live");
    // and the core refuses it too, in words that say why
    let (project_id, main_id) = setup.frame.read_with(cx, |f, _| (f.projects[0].id.clone(), f.tabs()[1].id.clone()));
    let sink = Events::new(|_| {});
    let request = enkidu_core::commands::LiveRequest { project_id, workspace_id: Some(main_id.clone()) };
    let refused = enkidu_core::commands::set_live(sink.clone(), &setup.shared, request).unwrap_err();
    assert_eq!(refused.message, "main-folder is the main folder itself, so there is nothing to copy into it");
    // removed, it is only forgotten: the folder, its branch and its files stay
    let state = git_state(&setup.project);
    let files = files_of(&setup.project);
    enkidu_core::commands::remove_workspace(sink, &setup.shared, main_id, false).expect("the main folder is forgotten");
    assert!(setup.project.join("README.md").exists(), "the folder stays");
    assert_eq!((git_state(&setup.project), files_of(&setup.project)), (state, files));
    assert_eq!(setup.shared.db.workspaces().unwrap().len(), 1, "only stock-fix is left");
    // Esc closes the dialog from the main folder's side too, and opens nothing
    ask_for_the_main_folder(cx, &setup);
    press(cx, setup.window, "escape");
    assert!(until(cx, setup.window, |w| w.try_find("new-workspace-dialog").is_none()), "Esc closed it");
    std::thread::sleep(Duration::from_millis(200));
    cx.run_until_parked();
    assert_eq!(setup.shared.db.workspaces().unwrap().len(), 1, "nothing was opened");
}

#[gpui_kit::test]
fn a_name_that_wont_do_is_said_and_nothing_is_made(cx: &mut TestAppContext) {
    let setup = start(cx);
    open_dialog(cx, &setup);
    typed(cx, setup.window, "bad name");
    // leaving the field (a click into the branch's)
    cx.update_window(setup.window, |_, w, cx| w.click("ws-branch-name", cx)).unwrap();
    assert!(
        until(cx, setup.window, |w| said(w, "ws-name-error").as_deref()
            == Some("Letters, numbers, dots, dashes and underscores")),
        "said once the field is left"
    );
    press(cx, setup.window, "enter");
    cx.update_window(setup.window, |_, w, cx| w.click("create-workspace", cx)).unwrap();
    assert!(until(cx, setup.window, |w| w.try_find("new-workspace-dialog").is_some()));
    std::thread::sleep(Duration::from_millis(200));
    cx.run_until_parked();
    assert!(setup.shared.db.workspaces().unwrap().is_empty(), "nothing was made");
    // Esc closes it
    press(cx, setup.window, "escape");
    assert!(until(cx, setup.window, |w| w.try_find("new-workspace-dialog").is_none()), "Esc closed it");
}
