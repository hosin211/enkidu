//! Item 1.16: the projects row on the frame's live store, with a real core on a temporary folder and throwaway git
//! repos. Add project picks a folder and the core adds it, and what went wrong is said; the project on screen opens
//! its menu, which removes a project only once it has no workspaces; a project whose workspaces need you, while
//! another one is on screen, shows how many.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use enkidu_app::bridge::Bridge;
use enkidu_app::shell::Shell;
use enkidu_core::db::{Project, Workspace};
use enkidu_core::events::Event;
use enkidu_core::state::Shared;
use enkidu_model::frame::Frame;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, Entity, TestAppContext, Window, WindowBounds, WindowOptions, px, size,
};

struct Setup {
    home: tempfile::TempDir,
    shared: Arc<Shared>,
    news: UnboundedSender<Event>,
    frame: Entity<Frame>,
    window: AnyWindowHandle,
}

fn project(id: &str, name: &str, path: &Path) -> Project {
    Project {
        id: id.into(),
        name: name.into(),
        path: path.to_string_lossy().into(),
        live_workspace_id: None,
        created_at: 1,
    }
}

fn workspace(id: &str, project_id: &str, name: &str) -> Workspace {
    Workspace {
        id: id.into(),
        project_id: project_id.into(),
        name: name.into(),
        path: format!("/ws/{name}"),
        branch: format!("feature/{name}"),
        base: Some("main".into()),
        session_id: format!("s-{id}"),
        transcript_path: None,
        claude_config_dir: None,
        created_at: 1,
        adopted: false,
    }
}

/// Runs the app until `done` holds for the frame (the core answers on threads of its own).
fn until(cx: &mut TestAppContext, frame: &Entity<Frame>, done: impl Fn(&Frame) -> bool) -> bool {
    for _ in 0..400 {
        cx.run_until_parked();
        if frame.read_with(cx, |f, _| done(f)) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

/// Runs the app until `seen` holds for the window's last frame.
fn until_seen(cx: &mut TestAppContext, window: AnyWindowHandle, seen: impl Fn(&Window) -> bool) -> bool {
    seen_within(cx, window, 400, seen)
}

/// The same, giving up after `tries` (10 ms each).
fn seen_within(cx: &mut TestAppContext, window: AnyWindowHandle, tries: usize, seen: impl Fn(&Window) -> bool) -> bool {
    for _ in 0..tries {
        cx.run_until_parked();
        let ok = cx
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                seen(window)
            })
            .unwrap();
        if ok {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

/// A core on a temporary folder whose database holds `projects` and `workspaces` (`statuses` set on the core), the
/// stores on it, and the window on the stores, once the first read has come back.
fn start(
    cx: &mut TestAppContext,
    projects: impl Fn(&Path) -> Vec<Project>,
    workspaces: &[Workspace],
    statuses: &[(&str, &str)],
) -> Setup {
    let home = tempfile::tempdir().unwrap();
    let shared = Arc::new(Shared::new(home.path().join("data"), Instant::now()).unwrap());
    let projects = projects(home.path());
    for p in &projects {
        shared.db.insert_project(p).unwrap();
    }
    for w in workspaces {
        shared.db.insert_workspace(w).unwrap();
    }
    for (id, status) in statuses {
        shared.set_status(&enkidu_core::events::Events::none(), id, status);
    }
    let (news, events) = unbounded();
    let frame = cx.update(|cx| {
        enkidu_app::init(cx);
        Bridge::install(cx, shared.clone(), enkidu_core::events::Events::none());
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
    cx.executor().allow_parking();
    let want = projects.len();
    assert!(until(cx, &frame, |f| f.projects.len() == want), "the first read came back");
    Setup { home, shared, news, frame, window }
}

fn click(cx: &mut TestAppContext, window: AnyWindowHandle, id: impl Into<gpui_kit::ElementId>) {
    let id = id.into();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx);
    })
    .unwrap();
}

fn pick(cx: &mut TestAppContext, setup: &Setup, folder: Option<PathBuf>) {
    click(cx, setup.window, "add-project");
    assert!(cx.did_prompt_for_paths(), "Add project opens the folder picker");
    cx.simulate_path_prompt_response(move |options| {
        assert!(options.directories && !options.files && !options.multiple, "one folder");
        folder.map(|f| vec![f])
    });
}

#[gpui_kit::test]
fn add_project_picks_a_folder_and_the_project_comes_on_screen(cx: &mut TestAppContext) {
    let setup = start(cx, |home| vec![project("p1", "first", &home.join("first"))], &[], &[]);
    let shop = enkidu_testkit::repo(setup.home.path(), "shop");

    pick(cx, &setup, Some(shop.clone()));
    assert!(until(cx, &setup.frame, |f| f.projects.len() == 2), "the project was added");
    let added = setup.frame.read_with(cx, |f, _| {
        let added = f.projects[1].clone();
        assert_eq!(added.name, "shop", "named after its folder");
        assert_eq!(added.path, shop.to_string_lossy());
        assert_eq!(f.current_project.as_deref(), Some(added.id.as_str()), "it comes on screen");
        added
    });
    assert!(setup.shared.db.projects().unwrap().iter().any(|p| p.id == added.id), "the core keeps it");
    assert!(
        until_seen(cx, setup.window, |window| {
            window.try_find_by_label("shop: project menu").is_some()
                && window.find(("project", 0usize)).label() == Some("first")
        }),
        "its tab opens its menu, and the first project is a tab to switch to"
    );

    // a folder already added, and one that isn't a git repository: each is said, and nothing is added
    let plain = setup.home.path().join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    for (folder, words) in
        [(shop.clone(), "That project is already here"), (plain, "That folder isn't a git repository")]
    {
        pick(cx, &setup, Some(folder));
        assert!(until_seen(cx, setup.window, |window| window.try_find_by_label(words).is_some()), "said: {words}");
    }
    // closing the picker without a folder does nothing
    pick(cx, &setup, None);
    cx.run_until_parked();
    assert_eq!(setup.frame.read_with(cx, |f, _| f.projects.len()), 2);
    assert_eq!(setup.shared.db.projects().unwrap().len(), 2);
}

#[gpui_kit::test]
fn the_project_menu_removes_a_project_only_once_it_has_no_workspaces(cx: &mut TestAppContext) {
    let setup = start(
        cx,
        |home| vec![project("p1", "shop", &home.join("shop")), project("p2", "spare", &home.join("spare"))],
        &[workspace("w1", "p1", "stock-fix")],
        &[],
    );
    let shop_path = setup.home.path().join("shop").to_string_lossy().to_string();

    // shop has a workspace: its menu says where it is, and won't remove it
    click(cx, setup.window, ("project", 0usize));
    assert!(
        until_seen(cx, setup.window, |window| window.try_find_by_label("Remove its workspaces first").is_some()),
        "the menu opened"
    );
    cx.update_window(setup.window, |_, window, cx| {
        assert!(window.try_find_by_label(&shop_path).is_some(), "the menu names the project's folder");
        assert!(window.try_find_by_label("Show in Finder").is_some());
        window.click("remove-project", cx);
    })
    .unwrap();
    cx.run_until_parked();
    std::thread::sleep(Duration::from_millis(50));
    cx.run_until_parked();
    assert_eq!(setup.shared.db.projects().unwrap().len(), 2, "nothing removed");
    assert!(
        !seen_within(cx, setup.window, 40, |w| w.try_find_by_label("Remove the project's workspaces first").is_some()),
        "the item can't be chosen, so the core was never asked"
    );
    assert_eq!(setup.frame.read_with(cx, |f, _| f.projects.len()), 2);
    cx.update_window(setup.window, |_, window, cx| window.press("escape", cx)).unwrap();

    // spare has none: on screen, its menu removes it, and the first project left comes on screen
    click(cx, setup.window, ("project", 1usize));
    assert_eq!(setup.frame.read_with(cx, |f, _| f.current_project.clone()), Some("p2".into()));
    click(cx, setup.window, ("project", 1usize));
    assert!(
        until_seen(cx, setup.window, |window| window.try_find_by_label("Remove project").is_some()),
        "spare's menu offers Remove project"
    );
    cx.update_window(setup.window, |_, window, cx| window.click("remove-project", cx)).unwrap();
    assert!(until(cx, &setup.frame, |f| f.projects.len() == 1), "spare is gone from the row");
    assert_eq!(setup.frame.read_with(cx, |f, _| f.current_project.clone()), Some("p1".into()));
    let left: Vec<String> = setup.shared.db.projects().unwrap().into_iter().map(|p| p.id).collect();
    assert_eq!(left, ["p1"], "and from the core");
}

#[gpui_kit::test]
fn a_project_whose_workspaces_need_you_says_how_many_while_another_is_on_screen(cx: &mut TestAppContext) {
    let setup = start(
        cx,
        |home| vec![project("p1", "shop", &home.join("shop")), project("p2", "tools", &home.join("tools"))],
        &[workspace("w1", "p1", "stock-fix"), workspace("w2", "p2", "lint"), workspace("w3", "p2", "docs")],
        &[("w2", "needs_you")],
    );
    let count =
        |window: &Window, ix: usize| window.try_find(("needs-you", ix)).and_then(|c| c.label().map(String::from));
    assert!(
        until_seen(cx, setup.window, |w| count(w, 1).as_deref() == Some("1 workspace needs you")),
        "tools shows 1, from what the core knew at the start"
    );

    // the core's news: another of tools' workspaces needs you, then shop's own
    setup.news.unbounded_send(Event::StatusChanged { workspace_id: "w3".into(), status: "needs_you".into() }).unwrap();
    assert!(until_seen(cx, setup.window, |w| count(w, 1).as_deref() == Some("2 workspaces need you")));
    setup.news.unbounded_send(Event::StatusChanged { workspace_id: "w1".into(), status: "needs_you".into() }).unwrap();
    cx.run_until_parked();
    cx.update_window(setup.window, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(count(window, 0), None, "the project on screen shows no count: its tabs are right below");
    })
    .unwrap();

    // tools comes on screen: now shop shows its one
    click(cx, setup.window, ("project", 1usize));
    assert!(until_seen(cx, setup.window, |w| count(w, 0).as_deref() == Some("1 workspace needs you")
        && count(w, 1).is_none()));
    // and when it is answered, the count goes
    setup.news.unbounded_send(Event::StatusChanged { workspace_id: "w1".into(), status: "working".into() }).unwrap();
    assert!(until_seen(cx, setup.window, |w| count(w, 0).is_none()));
}
