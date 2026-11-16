//! Item 3.1 on the whole wiring: the core's watcher sees a file change in a workspace's folder, and the workspace's
//! list of changes has it, fresh; a commit empties it; a removed workspace's list is forgotten.

mod common;

use common::App;
use enkidu_testkit::git;
use gpui_kit::TestAppContext;

/// The changed files the store has for a workspace, as (path, kind).
fn list(cx: &mut TestAppContext, workspace: &str) -> Option<Vec<(String, String)>> {
    cx.run_until_parked();
    cx.update(|cx| {
        let lists = enkidu_app::stores::Stores::changes(cx);
        lists.read(cx).of(workspace).map(|files| files.iter().map(|c| (c.path.clone(), c.kind.clone())).collect())
    })
}

fn until_list(cx: &mut TestAppContext, workspace: &str, want: &[(&str, &str)]) -> bool {
    let want: Vec<(String, String)> = want.iter().map(|(p, k)| (p.to_string(), k.to_string())).collect();
    for _ in 0..800 {
        if list(cx, workspace).as_ref() == Some(&want) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    false
}

#[gpui_kit::test]
fn a_workspaces_list_follows_its_folder_and_is_forgotten_with_it(cx: &mut TestAppContext) {
    let app = common::start(cx, |shared, project, _| {
        App::make(shared, project, "stock-fix", "new", "stock-fix", Some("main"))
    });
    let (_, _, folder) = app.on_screen(cx).expect("on screen");
    let folder = std::path::PathBuf::from(folder);
    let ws = app.frame.read_with(cx, |f, _| f.current_workspace().unwrap().id.clone());
    // nothing changed yet: git's first answer is an empty list
    assert!(until_list(cx, &ws, &[]), "{:?}", list(cx, &ws));
    // a file is written, another is edited: the watcher sees them
    std::fs::write(folder.join("notes.txt"), "new\n").unwrap();
    let readme = std::fs::read_dir(&folder)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .find(|name| !name.starts_with('.') && name != "notes.txt")
        .expect("a tracked file in the test repo");
    std::fs::write(folder.join(&readme), "changed\n").unwrap();
    let mut want = vec![("notes.txt", "untracked"), (readme.as_str(), "modified")];
    // the list comes as a tree of its folders lists it: these two by name
    want.sort_by(|a, b| enkidu_core::names::in_tree_order(a.0, b.0));
    assert!(until_list(cx, &ws, &want), "{:?}", list(cx, &ws));
    assert!(app.until_frame(cx, |f| f.changed_files(&ws) == 2), "the tab's count follows");
    // a commit empties it
    git(&folder, &["add", "-A"]);
    git(&folder, &["commit", "-q", "-m", "work"]);
    assert!(until_list(cx, &ws, &[]), "{:?}", list(cx, &ws));
    // the workspace is removed: its list goes with it
    app.click(cx, "ws-menu");
    assert!(app.until(cx, |w| gpui_kit::test::TestWindowExt::try_find(w, "remove-workspace").is_some()));
    app.click(cx, "remove-workspace");
    assert!(app.until(cx, |w| gpui_kit::test::TestWindowExt::try_find(w, "confirm-remove").is_some()));
    app.click(cx, "confirm-remove");
    assert!(app.until_frame(cx, |f| f.workspaces.is_empty()));
    assert_eq!(list(cx, &ws), None);
}

/// How long after `path` is written the store lists it for the workspace, after a quiet moment (so no look at the
/// folder is on its way from something earlier).
fn listed_after(cx: &mut TestAppContext, workspace: &str, folder: &std::path::Path, path: &str) -> std::time::Duration {
    for _ in 0..180 {
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let written = std::time::Instant::now();
    std::fs::write(folder.join(path), "x\n").unwrap();
    loop {
        if list(cx, workspace).is_some_and(|files| files.iter().any(|(listed, _)| listed == path)) {
            return written.elapsed();
        }
        assert!(written.elapsed() < std::time::Duration::from_secs(20), "{path} was never listed");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Item 3.16, found by the load test and by the first look at the built app (an edit took over a second to show): the
/// core's watcher looks at the workspace on screen 0.15 s after an edit and at the others 1.2 s after, and it only
/// knows which one is on screen when the app says so.
#[gpui_kit::test]
fn the_core_is_told_which_workspace_is_on_screen_and_looks_at_it_first(cx: &mut TestAppContext) {
    let app = common::start(cx, |shared, project, _| {
        App::make(shared, project, "one", "new", "one", Some("main"));
        App::make(shared, project, "two", "new", "two", Some("main"));
    });
    let workspace = |cx: &mut TestAppContext, name: &str| {
        app.frame
            .read_with(cx, |f, _| f.workspaces.iter().find(|w| w.name == name).map(|w| (w.id.clone(), w.path.clone())))
            .map(|(id, path)| (id, std::path::PathBuf::from(path)))
            .expect("the workspace")
    };
    let ((one, one_folder), (two, two_folder)) = (workspace(cx, "one"), workspace(cx, "two"));
    let told = |cx: &mut TestAppContext, want: Option<&str>| {
        for _ in 0..400 {
            cx.run_until_parked();
            if app.shared.changes.focused().as_deref() == want {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        false
    };
    // as the window opens: the first workspace is on screen, and the core knows
    app.click(cx, ("tab", 0usize));
    assert!(told(cx, Some(&one)), "the core was told {:?}", app.shared.changes.focused());
    // an edit there is listed within a fraction of a second; one in the other workspace takes over a second
    let on_screen = listed_after(cx, &one, &one_folder, "seen.txt");
    let hidden = listed_after(cx, &two, &two_folder, "unseen.txt");
    println!("WATCH on screen {on_screen:?}, not on screen {hidden:?}");
    assert!(on_screen < std::time::Duration::from_millis(900), "an edit on screen took {on_screen:?} to be listed");
    assert!(hidden > on_screen, "the workspace that isn't on screen was looked at first: {hidden:?} {on_screen:?}");

    // the other tab: the core is told, and now that one is the quick one
    app.click(cx, ("tab", 1usize));
    assert!(told(cx, Some(&two)), "the core was told {:?}", app.shared.changes.focused());
    let on_screen = listed_after(cx, &two, &two_folder, "seen-too.txt");
    assert!(on_screen < std::time::Duration::from_millis(900), "an edit on screen took {on_screen:?} to be listed");
}

/// Item 3.17, found by the load test (13 Nov): git is asked about a workspace each time it comes on screen, and going
/// back and forth started a git for every visit, over thirty at once in a big repository. Now one question at a
/// time for a workspace, and one more when it ends for all the visits meanwhile.
#[gpui_kit::test]
fn going_back_and_forth_asks_git_about_a_workspace_one_question_at_a_time(cx: &mut TestAppContext) {
    let app = common::start(cx, |shared, project, _| {
        App::make(shared, project, "one", "new", "one", Some("main"));
        App::make(shared, project, "two", "new", "two", Some("main"));
    });
    let asked = |cx: &mut TestAppContext| cx.update(enkidu_app::stores::changes_asked);
    // what the start asked is answered: the count stands still
    let settled = |cx: &mut TestAppContext| {
        let (mut last, mut still) = (asked(cx), 0);
        for _ in 0..600 {
            cx.run_until_parked();
            std::thread::sleep(std::time::Duration::from_millis(10));
            cx.run_until_parked();
            let now = asked(cx);
            still = if now == last { still + 1 } else { 0 };
            last = now;
            if still >= 40 {
                break;
            }
        }
        last
    };
    app.click(cx, ("tab", 0usize));
    let before = settled(cx);

    // twenty visits in a row, faster than git answers (no answer is taken in between)
    for _ in 0..10 {
        app.click(cx, ("tab", 1usize));
        app.click(cx, ("tab", 0usize));
    }
    assert_eq!(asked(cx) - before, 2, "one question for each workspace, however many visits");
    // the answers come: one more question each for the visits meanwhile, and then no more
    assert_eq!(settled(cx) - before, 4, "one more for each when its first was answered");

    // the lists are right all the same: an edit in the workspace on screen is listed
    let (id, folder) = app
        .frame
        .read_with(cx, |f, _| f.current_workspace().map(|w| (w.id.clone(), w.path.clone())))
        .expect("a workspace on screen");
    std::fs::write(std::path::Path::new(&folder).join("after.txt"), "x\n").unwrap();
    let mut listed = false;
    for _ in 0..600 {
        if list(cx, &id).is_some_and(|files| files.iter().any(|(path, _)| path == "after.txt")) {
            listed = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(listed, "the edit is listed: {:?}", list(cx, &id));
    // and a visit after all was answered asks again
    let before_visit = settled(cx);
    app.click(cx, ("tab", 1usize));
    assert_eq!(settled(cx) - before_visit, 1);
}
