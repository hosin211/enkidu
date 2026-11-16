//! Pictures of every named screen state, drawn offscreen by Metal: no window appears on screen (item 1.13). Each
//! picture is `shots/out/<name>.png` (or in
//! `$SHOTS_DIR`), at the window's 2x scale. `SHOTS_ONLY="gallery window"` draws only those.
//!
//!     sh scripts/shots.sh [name ...]      (or: cargo test -p enkidu-app --test shots)
//!
//! A new screen state is one more line in `STATES`.

use std::sync::Arc;

use enkidu_app::shell::Shell;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, HeadlessAppContext, Size, WindowBounds, WindowOptions, px,
};

/// A screen state: its name, its window's size, how its window is opened, and what is done in it before the
/// picture (a click that opens a menu).
struct State {
    name: &'static str,
    size: Size<gpui_kit::Pixels>,
    open: fn(WindowOptions, &mut App) -> AnyWindowHandle,
    then: Option<fn(&mut gpui_kit::Window, &mut App)>,
}

fn demo(options: WindowOptions, cx: &mut App) -> AnyWindowHandle {
    gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Shell::demo(window, cx)))
        .expect("a window drawn offscreen")
        .0
}

/// The demo window with five workspaces, for the pictures of item 18: your own order of the tabs, and workspaces
/// that are hidden. `hide` names the ones to hide, each with how many days ago it was.
fn five_tabs(options: WindowOptions, hide: &[(&str, i64)], cx: &mut App) -> AnyWindowHandle {
    use enkidu_app::demo::Tab;
    use enkidu_ui::lamp::Status;
    let mut tabs = enkidu_app::demo::tabs();
    tabs.push(Tab { name: "lint-cleanup".into(), status: Status::Idle, note: None });
    tabs.push(Tab { name: "docs".into(), status: Status::Done, note: Some("2".into()) });
    let (window, shell) = gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| {
            let terminal = std::sync::Arc::new(enkidu_term::Terminal::kept(
                enkidu_term::GridSize::new(120, 36),
                enkidu_term::Palette::enkidu(),
            ));
            terminal.feed(&enkidu_app::demo::terminal_output());
            let view = cx.new(|cx| enkidu_app::terminal_view::TerminalView::new(terminal, None, cx));
            Shell::new(tabs, 0, vec![view], false, window, cx)
        })
    })
    .expect("a window drawn offscreen");
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |t| t.as_millis() as i64);
    let frame = shell.read(cx).frame().clone();
    frame.update(cx, |frame, cx| {
        for (name, days_ago) in hide {
            let id = frame.workspaces.iter().find(|w| w.name == *name).map(|w| w.id.clone()).expect("the workspace");
            frame.hide_workspace(&id, now - days_ago * 86_400_000);
        }
        cx.notify();
    });
    window
}

fn in_your_order(options: WindowOptions, cx: &mut App) -> AnyWindowHandle {
    five_tabs(options, &[], cx)
}

/// Two of the five are hidden: one for thirteen days, one since yesterday.
fn two_hidden(options: WindowOptions, cx: &mut App) -> AnyWindowHandle {
    five_tabs(options, &[("sh-21309", 13), ("docs", 1)], cx)
}

fn all_hidden(options: WindowOptions, cx: &mut App) -> AnyWindowHandle {
    five_tabs(options, &[("stock-fix", 0), ("sh-21309", 13), ("pr-2175", 2), ("lint-cleanup", 20), ("docs", 1)], cx)
}

/// The demo window with its project's main folder opened as a workspace, and on screen (item 19.4): a tab like the
/// others, with no Go live beside it.
fn main_folder_tab(options: WindowOptions, cx: &mut App) -> AnyWindowHandle {
    let (window, shell) = gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Shell::demo(window, cx)))
        .expect("a window drawn offscreen");
    let frame = shell.read(cx).frame().clone();
    frame.update(cx, |frame, cx| {
        let project = frame
            .projects
            .iter()
            .find(|p| Some(&p.id) == frame.current_project.as_ref())
            .cloned()
            .expect("the project on screen");
        frame.add_workspace(enkidu_core::db::Workspace {
            id: "main".into(),
            project_id: project.id,
            name: enkidu_model::new_workspace::MAIN_NAME.into(),
            path: project.path,
            branch: "development".into(),
            base: None,
            session_id: String::new(),
            transcript_path: None,
            claude_config_dir: None,
            created_at: 2,
            adopted: true,
        });
        cx.notify();
    });
    window
}

/// The first tab is held and dragged over the third: the pointer's button is still down for the picture.
fn drag_first_tab_over_third(window: &mut gpui_kit::Window, cx: &mut App) {
    use gpui_kit::{InputEvent as _, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, point};
    window.render_frame(cx);
    let (start, end) = (window.find(("tab", 0usize)).bounds().center(), window.find(("tab", 2usize)).bounds().center());
    let (modifiers, left) = (Modifiers::default(), MouseButton::Left);
    window.dispatch_event(MouseMoveEvent { position: start, pressed_button: None, modifiers }.to_platform_input(), cx);
    let down = MouseDownEvent { button: left, position: start, modifiers, click_count: 1, first_mouse: false };
    window.dispatch_event(down.to_platform_input(), cx);
    for step in 1..=5 {
        let part = step as f32 / 5.;
        let position = point(start.x + (end.x - start.x) * part, start.y + (end.y - start.y) * part + px(6.) * part);
        window
            .dispatch_event(MouseMoveEvent { position, pressed_button: Some(left), modifiers }.to_platform_input(), cx);
        window.render_frame(cx);
    }
}

/// A right-click on the second tab: the workspace's menu, for that tab's workspace.
fn right_click_second_tab(window: &mut gpui_kit::Window, cx: &mut App) {
    use gpui_kit::{InputEvent as _, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent};
    window.render_frame(cx);
    let at = window.find(("tab", 1usize)).bounds().center();
    let (modifiers, right) = (Modifiers::default(), MouseButton::Right);
    window.dispatch_event(MouseMoveEvent { position: at, pressed_button: None, modifiers }.to_platform_input(), cx);
    let down = MouseDownEvent { button: right, position: at, modifiers, click_count: 1, first_mouse: false };
    window.dispatch_event(down.to_platform_input(), cx);
    window.dispatch_event(
        MouseUpEvent { button: right, position: at, modifiers, click_count: 1 }.to_platform_input(),
        cx,
    );
}

/// The window with nothing changed and three remarks that weren't sent, on two files (items 3.5d and 3.6).
fn only_remarks(options: WindowOptions, cx: &mut App) -> AnyWindowHandle {
    gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| {
            let terminal = std::sync::Arc::new(enkidu_term::Terminal::kept(
                enkidu_term::GridSize::new(120, 36),
                enkidu_term::Palette::enkidu(),
            ));
            terminal.feed(&enkidu_app::demo::terminal_output());
            let view = cx.new(|cx| enkidu_app::terminal_view::TerminalView::new(terminal, None, cx));
            let layout = enkidu_model::ui::DiffLayout::Split;
            let shell = Shell::with_changes(enkidu_app::demo::tabs(), vec![view], Vec::new(), layout, window, cx);
            let remark = |id: &str, path: &str, line: i64, body: &str| enkidu_core::db::Remark {
                id: id.into(),
                workspace_id: "w0".into(),
                path: path.into(),
                side: "new".into(),
                start_line: line,
                end_line: line + 2,
                snippet: String::new(),
                body: body.into(),
                created_at: line,
            };
            let remarks = shell.changes().expect("the island").read(cx).remarks().clone();
            remarks.update(cx, |remarks, cx| {
                remarks.save(remark("r1", "src/orders/models.py", 41, "Should `restocked` have an index?"), cx);
                remarks.save(remark("r2", "src/orders/models.py", 12, "Say what a line is in the docstring."), cx);
                remarks.save(remark("r3", "tests/conftest.py", 7, "This fixture could build the order too."), cx);
            });
            shell
        })
    })
    .expect("a window drawn offscreen")
    .0
}

/// The window on a frame of `projects` with no workspace (the empty screens of item 1.20).
fn empty_with(projects: Vec<enkidu_core::db::Project>, options: WindowOptions, cx: &mut App) -> AnyWindowHandle {
    gpui_kit::open_window(options, cx, |window, cx| {
        let frame =
            cx.new(|_| enkidu_model::frame::Frame::load(projects, vec![], Default::default(), &Default::default()));
        cx.new(|cx| Shell::kept(frame, window, cx))
    })
    .expect("a window drawn offscreen")
    .0
}

fn many_tabs(options: WindowOptions, cx: &mut App) -> AnyWindowHandle {
    use enkidu_app::demo::Tab;
    use enkidu_ui::lamp::Status;
    let mut tabs = enkidu_app::demo::tabs();
    for (name, status) in [("lint-cleanup", Status::Idle), ("docs", Status::Done), ("flaky-test", Status::NeedsYou)] {
        tabs.push(Tab { name: name.into(), status, note: None });
    }
    for n in 1..=3 {
        tabs.push(Tab { name: format!("task-{n}").into(), status: Status::Idle, note: Some("2".into()) });
    }
    gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| {
            let terminal = std::sync::Arc::new(enkidu_term::Terminal::kept(
                enkidu_term::GridSize::new(100, 30),
                enkidu_term::Palette::enkidu(),
            ));
            let view = cx.new(|cx| enkidu_app::terminal_view::TerminalView::new(terminal, None, cx));
            Shell::new(tabs, 0, vec![view], false, window, cx)
        })
    })
    .expect("a window drawn offscreen")
    .0
}

/// A terminal of rows whose bars must line up at column 8: wide characters, emoji and Arabic keep their cells (item
/// 2.7).
fn text_ruler(options: WindowOptions, cx: &mut App) -> AnyWindowHandle {
    gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| {
            let terminal = std::sync::Arc::new(enkidu_term::Terminal::kept(
                enkidu_term::GridSize::new(100, 30),
                enkidu_term::Palette::enkidu(),
            ));
            let rows = [
                "12345678|ruler",
                "abcdefgh|ascii",
                "中文中文|wide",
                "🙂🚀✅👍|emoji",
                "مرحبا بك|arabic",
                "ab مرحبا|mixed",
                "السلام x|ligature",
                "عدد 12x|digits",
                "x\u{301}yz\u{308}abcd|marks",
            ];
            terminal.feed(rows.join("\r\n").as_bytes());
            let view = cx.new(|cx| enkidu_app::terminal_view::TerminalView::new(terminal, None, cx));
            Shell::new(enkidu_app::demo::tabs(), 0, vec![view], false, window, cx)
        })
    })
    .expect("a window drawn offscreen")
    .0
}

/// A Japanese input method composing at a shell prompt: the composition at the cursor, underlined (item 2.8).
fn composing(options: WindowOptions, cx: &mut App) -> AnyWindowHandle {
    use gpui_kit::EntityInputHandler as _;
    gpui_kit::open_window(options, cx, |window, cx| {
        let terminal = std::sync::Arc::new(enkidu_term::Terminal::kept(
            enkidu_term::GridSize::new(100, 30),
            enkidu_term::Palette::enkidu(),
        ));
        terminal.feed("~/shop % echo ".as_bytes());
        let view = cx.new(|cx| enkidu_app::terminal_view::TerminalView::new(terminal, None, cx));
        view.update(cx, |view, cx| view.replace_and_mark_text_in_range(None, "にほんご", None, window, cx));
        cx.new(|cx| Shell::new(enkidu_app::demo::tabs(), 0, vec![view], false, window, cx))
    })
    .expect("a window drawn offscreen")
    .0
}

/// Claude that couldn't start: why, and the two ways on (items 2.11 and 4.3b).
fn claude_error(options: WindowOptions, cx: &mut App) -> AnyWindowHandle {
    use enkidu_app::demo::Tab;
    use enkidu_ui::lamp::Status;
    gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| {
            let tabs = vec![Tab { name: "stock-fix".into(), status: Status::Idle, note: None }];
            // Claude Code isn't on this Mac: the screen leads to Setup (item 21.7)
            let why = enkidu_core::commands::CLAUDE_NOT_INSTALLED;
            Shell::with_claude_error(tabs, why, window, cx)
        })
    })
    .expect("a window drawn offscreen")
    .0
}

/// The island's band with Claude exited and two shells beside it: the bar that offers to resume it (item 2.11).
fn exited_claude(options: WindowOptions, cx: &mut App) -> AnyWindowHandle {
    use enkidu_app::demo::Tab;
    use enkidu_ui::lamp::Status;
    gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| {
            let mut made = Vec::new();
            for printed in [enkidu_app::demo::terminal_output(), b"~/shop % ".to_vec(), b"~/shop % ".to_vec()] {
                let terminal = std::sync::Arc::new(enkidu_term::Terminal::kept(
                    enkidu_term::GridSize::new(100, 30),
                    enkidu_term::Palette::enkidu(),
                ));
                terminal.feed(&printed);
                made.push(cx.new(|cx| enkidu_app::terminal_view::TerminalView::new(terminal, None, cx)));
            }
            let tabs = vec![Tab { name: "stock-fix".into(), status: Status::Idle, note: None }];
            Shell::with_exited_claude(tabs, made, 1, window, cx)
        })
    })
    .expect("a window drawn offscreen")
    .0
}

/// The demo workspace with changes to review, side by side or in one column (item 3.2d).
/// Types into Search once its field has the keyboard (it takes it a step after the popup opens).
fn type_in_search(window: &mut gpui_kit::Window, cx: &mut App, words: &'static str) {
    window.defer(cx, move |window, cx| {
        window.render_frame(cx);
        window.defer(cx, move |window, cx| {
            window.render_frame(cx);
            window.input(words, cx);
        });
    });
}

fn reviewing(layout: enkidu_model::ui::DiffLayout, options: WindowOptions, cx: &mut App) -> AnyWindowHandle {
    gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| {
            let terminal = std::sync::Arc::new(enkidu_term::Terminal::kept(
                enkidu_term::GridSize::new(120, 36),
                enkidu_term::Palette::enkidu(),
            ));
            terminal.feed(&enkidu_app::demo::terminal_output());
            let view = cx.new(|cx| enkidu_app::terminal_view::TerminalView::new(terminal, None, cx));
            Shell::with_changes(enkidu_app::demo::tabs(), vec![view], enkidu_app::demo::changes(), layout, window, cx)
        })
    })
    .expect("a window drawn offscreen")
    .0
}

/// The same with more changed files in folders of every depth, for the pictures of the Changed files tree.
fn tree_reviewing(options: WindowOptions, cx: &mut App) -> AnyWindowHandle {
    gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| {
            let terminal = std::sync::Arc::new(enkidu_term::Terminal::kept(
                enkidu_term::GridSize::new(120, 36),
                enkidu_term::Palette::enkidu(),
            ));
            terminal.feed(&enkidu_app::demo::terminal_output());
            let view = cx.new(|cx| enkidu_app::terminal_view::TerminalView::new(terminal, None, cx));
            let layout = enkidu_model::ui::DiffLayout::Split;
            Shell::with_changes(
                enkidu_app::demo::tabs(),
                vec![view],
                enkidu_app::demo::tree_changes(),
                layout,
                window,
                cx,
            )
        })
    })
    .expect("a window drawn offscreen")
    .0
}

/// The row of Changed files that says `path`: a file's (`change-row`) or a folder's (`change-folder`).
fn tree_row(window: &gpui_kit::Window, id: &'static str, path: &str) -> Option<usize> {
    (0..60usize).find(|ix| window.try_find((id, *ix)).is_some_and(|row| row.label() == Some(path)))
}

thread_local! {
    /// The terminal of the window `at_work` opened, for the step that fills it once it has its width.
    static AT_WORK: std::cell::RefCell<Option<Arc<enkidu_term::Terminal>>> = const { std::cell::RefCell::new(None) };
}

/// The whole window at work, for the first picture of the README (item 30.3): the Work tab with its count, six
/// workspaces with their lamps and pull requests, the changes above in one column with the tree of the changed files
/// beside them, and Claude under them.
fn at_work(options: WindowOptions, cx: &mut App) -> AnyWindowHandle {
    gpui_kit::open_window(options, cx, |window, cx| {
        let (mut frame, mut index) = (work_frame(false), work_sample());
        // two more workspaces of the shop, each with the pull request the sample index has for its branch
        for (id, name, branch, status, pr) in [
            ("w6", "sh-21376-bundle-units", "feature/SH-21376-bundle-units", "idle", 2375),
            ("w7", "django-5.2.8", "chore/django-5.2.8", "done", 2415),
        ] {
            let mut workspace = frame.workspaces[0].clone();
            workspace.id = id.into();
            workspace.name = name.into();
            workspace.path = format!("/Users/demo/enkidu/workspaces/shop/{name}");
            workspace.branch = branch.into();
            workspace.session_id = format!("s-{id}");
            frame.workspaces.push(workspace);
            frame.statuses.insert(id.into(), status.into());
            let mut link =
                index.links.iter().find(|link| link.b == "workspace:w2").cloned().expect("the link of a workspace");
            (link.a, link.b) = (format!("pr:demo-org/shop#{pr}"), format!("workspace:{id}"));
            index.links.push(link);
        }
        // the one without a pull request has changes too: its tab says how many files
        frame.changed.insert("w4".into(), enkidu_model::frame::Changes { files: 3, additions: 27, deletions: 6 });
        let work = cx.new(|_| enkidu_app::work::store::Work::given(index, &frame, enkidu_testkit::WORK_SAMPLE_NOW));
        let frame = cx.new(|_| frame);
        let terminal =
            Arc::new(enkidu_term::Terminal::kept(enkidu_term::GridSize::new(120, 36), enkidu_term::Palette::enkidu()));
        AT_WORK.with(|kept| *kept.borrow_mut() = Some(terminal.clone()));
        cx.new(|cx| {
            let view = cx.new(|cx| enkidu_app::terminal_view::TerminalView::new(terminal, None, cx));
            let ui = enkidu_model::ui::Ui {
                diff_layout: enkidu_model::ui::DiffLayout::Unified,
                terminal_height: 384,
                ..Default::default()
            };
            Shell::with_work_and_changes(frame, work, vec![view], enkidu_app::demo::tree_changes(), ui, window, cx)
        })
    })
    .expect("a window drawn offscreen")
    .0
}

/// The frame the sample Work index matches: shop (its pr-remarks-2175 Live) and enkidu, five workspaces with their
/// lamps.
fn work_frame(open: bool) -> enkidu_model::frame::Frame {
    use enkidu_core::db::{Project, Workspace};
    let project = |id: &str, name: &str, live: Option<&str>| Project {
        id: id.into(),
        name: name.into(),
        path: format!("/Users/demo/code/{name}"),
        live_workspace_id: live.map(String::from),
        created_at: 1,
    };
    let ws = |id: &str, project: &str, of: &str, name: &str, branch: &str, base: Option<&str>| Workspace {
        id: id.into(),
        project_id: project.into(),
        name: name.into(),
        path: format!("/Users/demo/enkidu/workspaces/{of}/{name}"),
        branch: branch.into(),
        base: base.map(String::from),
        session_id: format!("s-{id}"),
        transcript_path: None,
        claude_config_dir: None,
        created_at: 1,
        adopted: false,
    };
    let statuses = [("w1", "working"), ("w2", "needs_you"), ("w3", "done"), ("w4", "idle"), ("w5", "working")]
        .into_iter()
        .map(|(id, status)| (id.to_string(), status.to_string()))
        .collect();
    enkidu_model::frame::Frame::load(
        vec![project("p1", "shop", Some("w3")), project("p2", "enkidu", None)],
        vec![
            ws("w1", "p1", "shop", "stock-fix", "fix/stock-double-count", Some("development")),
            ws("w2", "p1", "shop", "sh-21309-export", "feature/SH-21309-export", Some("development")),
            ws("w3", "p1", "shop", "pr-remarks-2175", "feature/SH-21112-bundles", None),
            ws("w4", "p1", "shop", "spike-search", "spike/search-index", Some("development")),
            ws("w5", "p2", "enkidu", "review-polish", "feature/review-polish", Some("main")),
        ],
        statuses,
        &enkidu_model::frame::View { project_id: Some("p1".into()), work: open, ..Default::default() },
    )
}

/// The sample Work index, at its fixed time.
fn work_sample() -> enkidu_core::work::model::Snapshot {
    serde_json::from_str(enkidu_testkit::WORK_SAMPLE).expect("the sample Work index")
}

/// The sample with four tasks of your own (item 8.5): two to do (one with notes, one for the other project), one in
/// progress, one done.
fn work_with_own() -> enkidu_core::work::model::Snapshot {
    use serde_json::json;
    let now = enkidu_testkit::WORK_SAMPLE_NOW;
    let hour = 3_600_000;
    let mut index = work_sample();
    let notes = "The runner has 4 GB, so the full stack doesn't fit.\n\n- Use `docker-compose.small.yml`\n- Keep the full stack as it is\n- Say in the README how to pick one";
    index.own_tasks = serde_json::from_value(json!([
        { "id": 1, "projectId": "p1", "title": "Try the small Docker setup on the CI runner", "notes": notes,
          "state": "todo", "createdAt": now - 3 * hour, "updatedAt": now - 3 * hour, "project": "shop" },
        { "id": 2, "projectId": "p2", "title": "Write the release notes for the preview", "notes": "",
          "state": "todo", "createdAt": now - 26 * hour, "updatedAt": now - 26 * hour, "project": "enkidu" },
        { "id": 3, "projectId": "p1", "title": "Clean up the fixtures of the stock tests", "notes": "",
          "state": "doing", "createdAt": now - 50 * hour, "updatedAt": now - 2 * hour, "project": "shop" },
        { "id": 4, "projectId": "p1", "title": "Ask Jonas for the QA data set", "notes": "",
          "state": "done", "createdAt": now - 70 * hour, "updatedAt": now - 20 * hour, "project": "shop" },
    ]))
    .expect("your own tasks");
    let events: Vec<enkidu_core::work::model::Event> = serde_json::from_value(json!([
        { "subject": "own:1", "at": now - 3 * hour, "kind": "own", "text": "You made this task" },
    ]))
    .expect("an event");
    index.events.splice(0..0, events);
    index
}

/// The sample with a workflow you have, due tonight: picked cards can go into it.
fn work_with_a_flow() -> enkidu_core::work::model::Snapshot {
    let mut index = work_sample();
    index.flows = serde_json::from_value(serde_json::json!([{
        "id": "f2", "projectId": "p1", "name": "Tonight", "instruction": "",
        "startAt": enkidu_testkit::WORK_SAMPLE_NOW + 9 * 3_600_000, "minutes": 60, "live": true, "state": "scheduled",
        "tasks": [{
            "subject": "ticket:SH-21580", "title": "Outlet groups in the stock report", "message": "SH-21580",
            "kind": "start", "instruction": "Start it.", "state": "waiting",
            "target": { "kind": "new", "name": "sh-21580-outlet", "branch": "feature/SH-21580-outlet", "mode": "new" },
        }],
    }]))
    .expect("a workflow");
    index
}

/// The sample with a night's workflows, as the Workflows page shows them. "running": a workflow at its second task
/// of three, with Claude's update and a report. "runs": last night's run (one done, one for your review with two
/// screenshots, one stuck) and the night before's (stopped), with a workflow due tonight. "cut": a run the usage
/// limit stopped.
fn work_with_flows(which: &str) -> enkidu_core::work::model::Snapshot {
    use serde_json::json;
    let now = enkidu_testkit::WORK_SAMPLE_NOW;
    let min = 60_000i64;
    let mut index = work_sample();
    let task = |key: &str, title: &str, kind: &str, state: &str, ran: Option<(i64, Option<i64>)>| {
        let mut task = json!({
            "subject": format!("ticket:{key}"), "title": title, "kind": kind, "state": state, "edited": false,
            "instruction": "Fix what QA found.",
            "message": format!("{key}: {title}\nTicket: https://demo.atlassian.net/browse/{key}\n\nThis ticket came back to us. Read its newest comments in Jira (what waits on it is below): find what they found, reproduce it, fix it on this branch, and run the tests that cover it.\n\nWhat waits on it:\n1. jonas-qa asked on PR #2414:\n   > Does the export include orders that were cancelled after delivery?\n   Draft an answer.\n\nDon't commit and don't push (no merge, rebase or PR either): leave every change uncommitted, so I review it in the diff in the morning. Enkidu refuses those commands while this task runs.\n\nThis is a task of Enkidu's workflow \"Night run\", which runs while I'm away: you have about 60 minutes, then the next task starts.\nThis workspace is Live while you work: its changes are in the project's main folder, where the dev environment runs, so check your work there."),
            "target": { "kind": "workspace", "workspaceId": "w2" },
        });
        if let Some((started, ended)) = ran {
            task["startedAt"] = json!(started);
            task["endedAt"] = json!(ended);
            task["workspaceId"] = json!("w2");
        }
        task
    };
    let flow = |id: &str, name: &str, state: &str, start_at: i64, tasks: Vec<serde_json::Value>| {
        json!({ "id": id, "projectId": "p1", "name": name, "instruction": "Use the small stack.", "startAt": start_at,
            "minutes": 60, "live": true, "state": state, "tasks": tasks, "updatedAt": start_at })
    };
    let export = "Export orders to the accounting CSV";
    let (flows, runs, reports) = match which {
        "running" => {
            let started = now - 20 * min;
            let mut night = flow(
                "f1",
                "Night run",
                "running",
                now - 60 * min,
                vec![
                    task(
                        "SH-21533",
                        "Stock count doubles after a transfer",
                        "came_back",
                        "done",
                        Some((now - 50 * min, Some(started))),
                    ),
                    task("SH-21309", export, "came_back", "running", Some((started, None))),
                    task("SH-21376", "Bundle units in the cost report", "continue", "waiting", None),
                ],
            );
            night["current"] = json!(1);
            night["tasks"][1]["minutes"] = json!(45);
            let reports = json!([
                { "id": 3, "subject": "SH-21309", "sessionId": "s-w2", "workspaceId": "w2", "state": "update",
                  "text": "Found it: the separator. The writer ignores the outlet's setting.", "images": ["1-half.png"],
                  "at": started + 5 * min },
                { "id": 2, "subject": "SH-21533", "sessionId": "s-w2", "workspaceId": "w2", "state": "done",
                  "text": "Fixed the double count.\n\nThe tests pass.", "images": [], "at": started - min },
            ]);
            (json!([night]), json!([]), reports)
        }
        "runs" => {
            let at = now - 26 * 60 * min;
            let before = at - 24 * 60 * min;
            let mut stuck = task(
                "SH-21522",
                "Planned orders keep the old delivery day",
                "start",
                "blocked",
                Some((at + 50 * min, Some(at + 90 * min))),
            );
            stuck["nudgedAt"] = json!(at + 80 * min);
            stuck["note"] = json!("It needs the supplier's calendar, which isn't in the test data.");
            let night_tasks = vec![
                task(
                    "SH-21376",
                    "Bundle units in the cost report",
                    "continue",
                    "done",
                    Some((at, Some(at + 30 * min))),
                ),
                task("SH-21309", export, "review", "review", Some((at + 30 * min, Some(at + 50 * min)))),
                stuck,
            ];
            let night = flow("f1", "Night run", "done", at, night_tasks);
            let tonight = flow(
                "f2",
                "Tonight",
                "scheduled",
                now + 9 * 60 * min,
                vec![task("SH-21580", "Outlet groups in the stock report", "start", "waiting", None)],
            );
            let mut earlier = flow(
                "f1",
                "Night run",
                "stopped",
                before,
                vec![
                    task("SH-21309", export, "review", "done", Some((before, Some(before + 30 * min)))),
                    task(
                        "SH-21376",
                        "Bundle units in the cost report",
                        "continue",
                        "stopped",
                        Some((before + 30 * min, Some(before + 40 * min))),
                    ),
                ],
            );
            earlier["tasks"][1]["note"] = json!("You stopped the workflow");
            let runs = json!([
                { "id": 7, "flowId": "f1", "startedAt": at, "endedAt": at + 90 * min, "state": "done", "flow": night },
                { "id": 5, "flowId": "f1", "startedAt": before, "endedAt": before + 40 * min, "state": "stopped", "flow": earlier },
            ]);
            let reports = json!([
                { "id": 4, "subject": "SH-21309", "sessionId": "s-w2", "workspaceId": "w2", "state": "review",
                  "text": "Added the export; the 7 s is worth your look.\n\n- The writer takes the outlet's separator now.\n- A month of 40,000 orders exports in 7 s.",
                  "images": ["1-list.png", "2-excel.png"], "at": at + 49 * min },
                { "id": 3, "subject": "SH-21309", "sessionId": "s-w2", "workspaceId": "w2", "state": "done",
                  "text": "The first half of the export.", "images": [], "at": before + 29 * min },
            ]);
            (json!([night, tonight]), runs, reports)
        }
        "states" => {
            // one task in every state a task can end in, and a workflow that runs (Running, Waiting)
            let at = now - 10 * 60 * min;
            let ended = [
                ("SH-21309", "done"),
                ("SH-21376", "review"),
                ("SH-21522", "blocked"),
                ("SH-21580", "no_report"),
                ("SH-21616", "timed_out"),
                ("SH-21533", "asked"),
                ("SH-21112", "cut_off"),
                ("SH-21475", "failed"),
                ("SH-20728", "stopped"),
            ];
            let tasks: Vec<serde_json::Value> = (ended.iter().enumerate())
                .map(|(n, (key, state))| {
                    let from = at + n as i64 * 10 * min;
                    task(
                        key,
                        &format!("A task that ended as {state}"),
                        "start",
                        state,
                        Some((from, Some(from + 9 * min))),
                    )
                })
                .collect();
            let night = flow("f1", "Night run", "done", at, tasks);
            let live = flow(
                "f2",
                "Before lunch",
                "running",
                now - 5 * min,
                vec![
                    task("SH-21309", export, "review", "running", Some((now - 5 * min, None))),
                    task("SH-21376", "Bundle units in the cost report", "continue", "waiting", None),
                ],
            );
            let runs = json!([{ "id": 1, "flowId": "f1", "startedAt": at, "endedAt": at + 90 * min, "state": "done", "flow": night }]);
            (json!([live]), runs, json!([]))
        }
        _ => {
            let at = now - 6 * 60 * min;
            let mut cut = task("SH-21309", export, "start", "cut_off", Some((at + 20 * min, Some(at + 31 * min))));
            cut["note"] = json!("You've hit your limit · resets 3am (Europe/Paris)");
            let night = flow(
                "f1",
                "Night run",
                "stopped",
                at,
                vec![
                    task(
                        "SH-21376",
                        "Bundle units in the cost report",
                        "start",
                        "done",
                        Some((at, Some(at + 20 * min))),
                    ),
                    cut,
                    task("SH-21522", "Planned orders keep the old delivery day", "start", "waiting", None),
                ],
            );
            let runs = json!([{ "id": 9, "flowId": "f1", "startedAt": at, "endedAt": at + 31 * min, "state": "stopped", "flow": night }]);
            (json!([night]), runs, json!([]))
        }
    };
    index.flows = serde_json::from_value(flows).expect("workflows");
    index.flow_runs = serde_json::from_value(runs).expect("runs");
    index.reports = serde_json::from_value(reports).expect("reports");
    index
}

/// The window on the old preview's frame with a given Work index, the Work page open or not (phase 5).
fn work_window(
    index: enkidu_core::work::model::Snapshot,
    open: bool,
    options: WindowOptions,
    cx: &mut App,
) -> AnyWindowHandle {
    let (window, _) = gpui_kit::open_window(options, cx, |window, cx| {
        let frame = work_frame(open);
        let work = cx.new(|_| enkidu_app::work::store::Work::given(index, &frame, enkidu_testkit::WORK_SAMPLE_NOW));
        let frame = cx.new(|_| frame);
        cx.new(|cx| Shell::with_work(frame, work, window, cx))
    })
    .expect("a window drawn offscreen");
    window
}

/// The sample with three of Claude's reports on SH-21309: it ended wanting a review, with three screenshots (the
/// last one a whole page); before that an update along the way and a night it got stuck.
fn work_with_reports() -> enkidu_core::work::model::Snapshot {
    let now = enkidu_testkit::WORK_SAMPLE_NOW;
    let mut index = work_sample();
    let ended = "Fixed the CSV header and the decimal separator.\n\n\
        - The writer now takes the outlet's separator (`export.py`, line 48).\n\
        - Checked the export with 3 orders: the totals match the invoices.\n\n\
        **Left for you:** the date format question on the PR.";
    index.reports = serde_json::from_value(serde_json::json!([
        { "id": 7, "subject": "SH-21309", "sessionId": "s-w2", "workspaceId": "w2", "state": "review", "text": ended,
          "images": ["1-export.png", "2-orders.png", "3-whole-page.png"], "at": now - 12 * 60_000 },
        { "id": 6, "subject": "SH-21309", "sessionId": "s-w2", "workspaceId": "w2", "state": "update",
          "text": "The writer is done. Running the export against three orders now.", "images": [],
          "at": now - 50 * 60_000 },
        { "id": 3, "subject": "SH-21309", "sessionId": "s-w2", "workspaceId": "w2", "state": "blocked",
          "text": "The export needs a decision on the date format: accounting wants `DD/MM/YYYY`, the API sends ISO.",
          "images": ["1-dates.png"], "at": now - 26 * 60 * 60_000 },
    ]))
    .expect("three reports");
    index
}

/// What Check connections answers in the pictures: one source that matches, one that can't be read, one that
/// differs in two fields; or nothing yet, for the picture of a check that still runs.
fn check_answers(still_running: bool, cx: &mut App) {
    use enkidu_core::work::{Checked, live::Difference};
    cx.set_global(enkidu_app::work::check::CheckSource(std::rc::Rc::new(move |cx: &mut App| {
        if still_running {
            return cx.background_spawn(std::future::pending());
        }
        let checked = |project: &str, source: &str, ok: bool, compared: &[&str], error: Option<&str>| Checked {
            project: project.into(),
            source: source.into(),
            ok,
            compared: compared.iter().map(|line| line.to_string()).collect(),
            differences: Vec::new(),
            error: error.map(String::from),
        };
        let differs = |thing: &str, field: &str, index: &str, direct: &str| Difference {
            thing: thing.into(),
            field: field.into(),
            index: index.into(),
            direct: direct.into(),
        };
        gpui_kit::Task::ready(Ok(vec![
            checked(
                "shop",
                "github",
                true,
                &["4 open PRs as `gh pr list --author @me` lists them", "their checks, reviews and open threads"],
                None,
            ),
            checked("shop", "jira", false, &[], Some("twg isn't signed in. Run `twg login --force`.")),
            Checked {
                differences: vec![
                    differs("#12", "title", "Fix the stock count", "Fix the stock count twice"),
                    differs("#14", "review", "", "CHANGES_REQUESTED"),
                ],
                ..checked("enkidu", "github", false, &["2 open PRs as `gh pr list --author @me` lists them"], None)
            },
        ]))
    })));
}

/// The screenshots of the pictures' reports: made-up pages, the one named a whole page far taller than wide.
fn made_up_screenshots(cx: &mut App) {
    cx.set_global(enkidu_app::work::shots::ShotSource(Arc::new(|report, name| {
        let seed = report as u32 + name.bytes().next().map(u32::from).unwrap_or(0);
        Ok(match name.contains("whole-page") {
            true => enkidu_testkit::screenshot_png(1280, 3600, seed),
            false => enkidu_testkit::screenshot_png(1440, 900, seed),
        })
    })));
}

/// The sample with Claude's report on SH-21309 (a review with two screenshots), a workflow that runs it now with
/// SH-21376 next, and Jira signed out.
fn work_with_marks() -> enkidu_core::work::model::Snapshot {
    let now = enkidu_testkit::WORK_SAMPLE_NOW;
    let mut index = work_jira_broken();
    index.reports = serde_json::from_value(serde_json::json!([
        { "id": 2, "subject": "SH-21309", "sessionId": "s-w2", "workspaceId": "w2", "state": "review",
          "text": "## Export works\n\nAll 14 tests pass.", "images": ["a.png", "b.png"], "at": now - 120 * 60_000 },
    ]))
    .expect("a report");
    let task = |subject: &str, state: &str| {
        serde_json::json!({ "subject": subject, "title": subject, "message": "", "state": state,
            "target": { "kind": "workspace", "workspaceId": "w2" } })
    };
    index.flows = serde_json::from_value(serde_json::json!([{
        "id": "f1", "projectId": "p1", "name": "Night shift", "instruction": "", "startAt": now - 60_000,
        "minutes": 60, "live": true, "state": "running", "current": 0,
        "tasks": [task("ticket:SH-21309", "running"), task("ticket:SH-21376", "waiting")],
    }]))
    .expect("a workflow");
    index
}

/// The sample with its first Jira board picked (all of its tickets on it), two cards hidden (SH-21616 needs you),
/// and a read running.
fn work_header_sample() -> enkidu_core::work::model::Snapshot {
    let mut index = work_sample();
    index.hidden = vec!["ticket:SH-21616".into(), "ticket:SH-21475".into()];
    index.board = Some(enkidu_core::work::model::BoardPick {
        board: index.boards[0].clone(),
        keys: Some(index.tickets.iter().map(|ticket| ticket.key.clone()).collect()),
        read_at: Some(enkidu_testkit::WORK_SAMPLE_NOW),
    });
    index.checking = true;
    index
}

/// The sample with SH-21309's description as a document with every kind of block.
fn work_rich_ticket() -> enkidu_core::work::model::Snapshot {
    let mut index = work_sample();
    let doc: serde_json::Value = serde_json::from_str(enkidu_testkit::WORK_RICH_DOC).expect("the description");
    for ticket in index.tickets.iter_mut().filter(|ticket| ticket.key == "SH-21309") {
        ticket.description = Some(doc.clone());
    }
    index
}

/// The sample with Jira signed out since 42 minutes.
fn work_jira_broken() -> enkidu_core::work::model::Snapshot {
    let mut index = work_sample();
    index.health[1].state = "error".into();
    index.health[1].error = Some("Jira: twg isn't signed in".into());
    index.health[1].fix = Some("Run `twg login --force`.".into());
    index.health[1].last_ok = Some(enkidu_testkit::WORK_SAMPLE_NOW - 42 * 60_000);
    index.health[1].next_try = Some(enkidu_testkit::WORK_SAMPLE_NOW + 3 * 60_000);
    index
}

const STATES: &[State] = &[
    // the frame with the demo workspace: a working Claude, a red and a green tab (phase 0)
    State { name: "window", size: Size { width: px(1360.), height: px(860.) }, open: demo, then: None },
    // the project on screen with its menu open (item 1.16)
    State {
        name: "project-menu",
        size: Size { width: px(1360.), height: px(860.) },
        open: demo,
        then: Some(|window, cx| window.click(("project", 0usize), cx)),
    },
    // the New workspace dialog, opened by ⌘N (item 1.19; with no core behind it, so no branches)
    State {
        name: "new-workspace",
        size: Size { width: px(1360.), height: px(860.) },
        open: demo,
        then: Some(|window, cx| window.dispatch_action(Box::new(enkidu_app::NewWorkspace), cx)),
    },
    // the same dialog on "The main folder" (item 19.3): the folder and what working there means, with no name and
    // no branch to give, and the button that opens it
    State {
        name: "new-workspace-main",
        size: Size { width: px(1360.), height: px(860.) },
        open: demo,
        then: Some(|window, cx| {
            window.dispatch_action(Box::new(enkidu_app::NewWorkspace), cx);
            window.defer(cx, |window, cx| {
                window.render_frame(cx);
                window.click("ws-where-main", cx);
            });
        }),
    },
    // the main folder opened as a workspace, on screen (item 19.4): a tab like the others, and no Go live beside it
    State {
        name: "tabs-main-folder",
        size: Size { width: px(1360.), height: px(560.) },
        open: main_folder_tab,
        then: None,
    },
    // Remove workspace, asked from the workspace's menu (items 1.22b and 7.1a)
    State {
        name: "remove-workspace",
        size: Size { width: px(1360.), height: px(860.) },
        open: demo,
        then: Some(|window, cx| {
            window.click("ws-menu", cx);
            window.defer(cx, |window, cx| {
                window.render_frame(cx);
                window.click("remove-workspace", cx);
            });
        }),
    },
    // the workspace on screen's menu, at the right of the tabs row (item 1.22b)
    State {
        name: "workspace-menu",
        size: Size { width: px(1360.), height: px(860.) },
        open: demo,
        then: Some(|window, cx| window.click("ws-menu", cx)),
    },
    // the palette, opened by ⌘K (item 1.21)
    State {
        name: "palette",
        size: Size { width: px(1360.), height: px(860.) },
        open: demo,
        then: Some(|window, cx| window.dispatch_action(Box::new(enkidu_app::TogglePalette), cx)),
    },
    // the folder's path copied from the status line: the toast says so (item 1.18)
    State {
        name: "path-copied",
        size: Size { width: px(1360.), height: px(860.) },
        open: demo,
        then: Some(|window, cx| window.click("status-path", cx)),
    },
    // toasts (item 4.6): what went wrong, and news with a long detail folded to three lines
    State {
        name: "toasts",
        size: Size { width: px(1360.), height: px(860.) },
        open: demo,
        then: Some(|window, cx| {
            enkidu_app::toast::say(window, "shop is already a project here.", cx);
            enkidu_app::toast::note(window, "Path copied", "~/enkidu/workspaces/shop/stock-fix", cx);
            let detail = "Copied 9 files with its changes into ~/code/shop; new changes follow while it is \
                          Live.\n\n$ on (stock-fix)\nrestarting web\nrestarting worker\ndone";
            enkidu_app::toast::done(window, "stock-fix is Live", detail, cx);
        }),
    },
    // a tooltip with the key of what its button does (item 4.6)
    State {
        name: "tooltip",
        size: Size { width: px(1360.), height: px(860.) },
        open: demo,
        then: Some(|window, cx| {
            window.hover("terminal", cx);
            window.hover("settings", cx);
        }),
    },
    // Live on (item 4.5): the Live workspace on screen, its button green with the bars, its tab and the status line
    State {
        name: "live",
        size: Size { width: px(1360.), height: px(860.) },
        open: demo,
        then: Some(|window, cx| window.click(("tab", 2usize), cx)),
    },
    // the tabs with little room: compact, shrunk to 112 px, the strip cut and faded on the right (item 1.17)
    State { name: "tabs-narrow", size: Size { width: px(900.), height: px(560.) }, open: many_tabs, then: None },
    // a tab held while it is dragged to another's place (item 18.4): it follows the pointer, its place in the row
    // dims, and a blue line stands where it would land
    State {
        name: "tabs-dragging",
        size: Size { width: px(1360.), height: px(560.) },
        open: in_your_order,
        then: Some(drag_first_tab_over_third),
    },
    // a right-click on a tab: the workspace's menu for that tab, with Hide workspace above Remove workspace… (18.5)
    State {
        name: "tab-menu",
        size: Size { width: px(1360.), height: px(560.) },
        open: in_your_order,
        then: Some(right_click_second_tab),
    },
    // two workspaces hidden: their button stands after the tabs, an eye with a line through it and their number
    State { name: "tabs-hidden", size: Size { width: px(1360.), height: px(560.) }, open: two_hidden, then: None },
    // its menu: each with its lamp, its name and since when
    State {
        name: "tabs-hidden-menu",
        size: Size { width: px(1360.), height: px(560.) },
        open: two_hidden,
        then: Some(|window, cx| window.click("hidden-workspaces", cx)),
    },
    // every workspace of the project hidden: the row keeps the button, and the screen says so
    State { name: "empty-hidden", size: Size { width: px(1360.), height: px(560.) }, open: all_hidden, then: None },
    // the window behind another app: the edge of the tab on screen is gray, and
    // no part has the keyboard's edge (DESIGN.md, "States")
    State { name: "window-behind", size: Size { width: px(1360.), height: px(860.) }, open: demo, then: None },
    // a tab that has the keyboard (← and → move between the tabs): its blue ring stands inside the tab, apart from
    // the blue edge of the tab on screen (DESIGN.md, "States")
    State {
        name: "tab-keyboard",
        size: Size { width: px(900.), height: px(560.) },
        open: many_tabs,
        then: Some(|window, cx| {
            window.click(("tab", 0usize), cx);
            window.press("right", cx);
        }),
    },
    // and the tab on screen with the keyboard: its edge, and the ring inside it
    State {
        name: "tab-keyboard-on-screen",
        size: Size { width: px(900.), height: px(560.) },
        open: many_tabs,
        then: Some(|window, cx| {
            window.click(("tab", 0usize), cx);
            window.press("right", cx);
            window.press("left", cx);
        }),
    },
    // no project yet: how to start (item 1.20)
    State {
        name: "no-projects",
        size: Size { width: px(1360.), height: px(860.) },
        open: |options, cx| empty_with(vec![], options, cx),
        then: None,
    },
    // a project with no workspace yet: the first one (item 1.20)
    State {
        name: "no-workspaces",
        size: Size { width: px(1360.), height: px(860.) },
        open: |options, cx| {
            let shop = enkidu_core::db::Project {
                id: "p1".into(),
                name: "shop".into(),
                path: "~/code/shop".into(),
                live_workspace_id: None,
                created_at: 1,
            };
            empty_with(vec![shop], options, cx)
        },
        then: None,
    },
    State { name: "terminal-text", size: Size { width: px(1360.), height: px(860.) }, open: text_ruler, then: None },
    State { name: "terminal-ime", size: Size { width: px(1360.), height: px(860.) }, open: composing, then: None },
    State {
        name: "terminal-exited",
        size: Size { width: px(1360.), height: px(860.) },
        open: exited_claude,
        then: None,
    },
    State { name: "claude-error", size: Size { width: px(1360.), height: px(860.) }, open: claude_error, then: None },
    // ⌘ held over an address in the terminal: underlined (item 2.5)
    State {
        name: "terminal-link",
        size: Size { width: px(1360.), height: px(860.) },
        open: demo,
        then: Some(|window, cx| {
            use gpui_kit::{InputEvent as _, Modifiers, MouseMoveEvent};
            let (cell_width, line_height) = enkidu_app::terminal_view::cell_size(window);
            let corner = window.find("terminal").bounds().origin;
            // row 19 of the demo session: "● Opened https://github.com/acme/shop/pull/12 for review."
            let position = gpui_kit::point(corner.x + cell_width * 20.5, corner.y + line_height * 19.5);
            let moved = MouseMoveEvent { position, pressed_button: None, modifiers: Modifiers::command() };
            window.dispatch_event(moved.to_platform_input(), cx);
        }),
    },
    // the changes island above the terminal: side by side, as the window opens (item 3.2d)
    State {
        name: "changes",
        size: Size { width: px(1360.), height: px(860.) },
        open: |options, cx| reviewing(enkidu_model::ui::DiffLayout::Split, options, cx),
        then: None,
    },
    // the same in one column
    State {
        name: "changes-unified",
        size: Size { width: px(1360.), height: px(860.) },
        open: |options, cx| reviewing(enkidu_model::ui::DiffLayout::Unified, options, cx),
        then: None,
    },
    // scrolled into a file: its header stays at the top while its lines pass under it (item 3.2c)
    State {
        name: "changes-scrolled",
        size: Size { width: px(1360.), height: px(860.) },
        open: |options, cx| reviewing(enkidu_model::ui::DiffLayout::Split, options, cx),
        then: Some(|window, cx| {
            window.scroll("diff-view", gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-430.))), cx)
        }),
    },
    // remarks in the diff: a card under its line, one on a removed line, and the composer open (item 3.5c)
    State {
        name: "changes-remarks",
        size: Size { width: px(1360.), height: px(1100.) },
        open: |options, cx| {
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| {
                    let terminal = std::sync::Arc::new(enkidu_term::Terminal::kept(
                        enkidu_term::GridSize::new(120, 36),
                        enkidu_term::Palette::enkidu(),
                    ));
                    let view = cx.new(|cx| enkidu_app::terminal_view::TerminalView::new(terminal, None, cx));
                    let layout = enkidu_model::ui::DiffLayout::Split;
                    let shell = Shell::with_changes(
                        enkidu_app::demo::tabs(),
                        vec![view],
                        enkidu_app::demo::changes(),
                        layout,
                        window,
                        cx,
                    );
                    let remark = |id: &str, side: &str, line: i64, code: &str, body: &str| enkidu_core::db::Remark {
                        id: id.into(),
                        workspace_id: "w0".into(),
                        path: "src/orders/stock.py".into(),
                        side: side.into(),
                        start_line: line,
                        end_line: line,
                        snippet: code.into(),
                        body: body.into(),
                        created_at: line,
                    };
                    let remarks = shell.changes().expect("the island").read(cx).remarks().clone();
                    remarks.update(cx, |remarks, cx| {
                        let code = "from shop.orders.signals import stock_changed";
                        remarks.save(
                            remark(
                                "r1",
                                "new",
                                6,
                                code,
                                "Is the signal sent anywhere else? If not, call the function directly.",
                            ),
                            cx,
                        );
                        let gone = "        item.save(update_fields=[\"stock\"])";
                        remarks.save(
                            remark(
                                "r2",
                                "old",
                                27,
                                gone,
                                "This saved every line, restocked or not.\nKeep that in the new code?",
                            ),
                            cx,
                        );
                    });
                    shell
                })
            })
            .expect("a window drawn offscreen")
            .0
        },
        then: Some(|window, cx| {
            use gpui_kit::{InputEvent as _, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent};
            // down to the change, then a press on line 29's number and a few words
            window.scroll("diff-view", gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-560.))), cx);
            let number = (0..60usize)
                .flat_map(|ix| window.find_all(("line", ix)))
                .find(|number| number.label() == Some("Line 29, after"))
                .expect("line 29");
            let (position, modifiers) = (number.bounds().center(), Modifiers::default());
            let down =
                MouseDownEvent { button: MouseButton::Left, position, modifiers, click_count: 1, first_mouse: false };
            window.dispatch_event(down.to_platform_input(), cx);
            let up = MouseUpEvent { button: MouseButton::Left, position, modifiers, click_count: 1 };
            window.dispatch_event(up.to_platform_input(), cx);
            // the composer opens once the press has been heard, which is after this step: the words are typed then
            window.defer(cx, |window, cx| {
                window.render_frame(cx);
                window.render_frame(cx);
                window.input("Say in the docstring that this is what stops the double count", cx);
            });
        }),
    },
    // the changes beside the terminal, in one column (item 3.8)
    State {
        name: "layout-side",
        size: Size { width: px(1360.), height: px(860.) },
        open: |options, cx| reviewing(enkidu_model::ui::DiffLayout::Unified, options, cx),
        then: Some(|window, cx| window.dispatch_action(Box::new(enkidu_app::ToggleSideLayout), cx)),
    },
    // the changes alone, with the whole workspace (item 3.8)
    State {
        name: "layout-review",
        size: Size { width: px(1360.), height: px(860.) },
        open: |options, cx| reviewing(enkidu_model::ui::DiffLayout::Split, options, cx),
        then: Some(|window, cx| window.dispatch_action(Box::new(enkidu_app::ToggleFullReview), cx)),
    },
    // the Files panel: folders open, a changed file and its folder marked (item 3.9d)
    State {
        name: "files-panel",
        size: Size { width: px(1360.), height: px(860.) },
        open: |options, cx| reviewing(enkidu_model::ui::DiffLayout::Split, options, cx),
        then: Some(|window, cx| {
            window.dispatch_action(Box::new(enkidu_app::ToggleFiles), cx);
            // the list of files arrives a step after the panel is first drawn, and a folder's rows a step after it
            // is opened: each step is done once the one before it has settled
            fn open(window: &mut gpui_kit::Window, cx: &mut App, left: usize) {
                window.render_frame(cx);
                let closed = (0..30usize).find(|ix| {
                    window.try_find(("folder-row", *ix)).is_some_and(|row| {
                        row.description() == Some("closed")
                            && row.label().is_some_and(|path| path == "src" || path == "src/orders")
                    })
                });
                if let Some(ix) = closed {
                    window.click(("folder-row", ix), cx);
                }
                if left > 0 {
                    window.defer(cx, move |window, cx| open(window, cx, left - 1));
                }
            }
            window.defer(cx, |window, cx| open(window, cx, 4));
        }),
    },
    // the same with something typed in its filter
    State {
        name: "files-filter",
        size: Size { width: px(1360.), height: px(860.) },
        open: |options, cx| reviewing(enkidu_model::ui::DiffLayout::Split, options, cx),
        then: Some(|window, cx| {
            window.dispatch_action(Box::new(enkidu_app::ToggleFiles), cx);
            window.defer(cx, |window, cx| {
                window.render_frame(cx);
                window.render_frame(cx);
                window.click("files-filter", cx);
                window.input("stock", cx);
            });
        }),
    },
    // Changed files as a tree of folders (item 29.5): every folder open, each with its count, and the file whose
    // diff is on screen marked
    State {
        name: "changed-tree",
        size: Size { width: px(1360.), height: px(860.) },
        open: tree_reviewing,
        then: Some(|window, cx| {
            window.defer(cx, |window, cx| {
                window.render_frame(cx);
                if let Some(ix) = tree_row(window, "change-row", "src/orders/stock.py") {
                    window.click(("change-row", ix), cx);
                }
            });
        }),
    },
    // after "Collapse all", with one folder opened again: the folders in it are closed (item 29.10)
    State {
        name: "changed-tree-collapsed",
        size: Size { width: px(1360.), height: px(860.) },
        open: tree_reviewing,
        then: Some(|window, cx| {
            window.defer(cx, |window, cx| {
                window.render_frame(cx);
                window.click("changed-collapse", cx);
                window.defer(cx, |window, cx| {
                    window.render_frame(cx);
                    if let Some(ix) = tree_row(window, "change-folder", "src") {
                        window.click(("change-folder", ix), cx);
                    }
                });
            });
        }),
    },
    // the filter's menu: the kinds of file among the changes, each with how many have it (item 29.10)
    State {
        name: "changed-tree-kinds",
        size: Size { width: px(1360.), height: px(860.) },
        open: tree_reviewing,
        then: Some(|window, cx| {
            window.defer(cx, |window, cx| {
                window.render_frame(cx);
                window.click("changed-kind", cx);
            });
        }),
    },
    // one kind picked: its files with the folders that hold them, and the button in signal blue
    State {
        name: "changed-tree-kind",
        size: Size { width: px(1360.), height: px(860.) },
        open: tree_reviewing,
        then: Some(|window, cx| {
            window.defer(cx, |window, cx| {
                window.render_frame(cx);
                window.click("changed-kind", cx);
                window.defer(cx, |window, cx| {
                    window.render_frame(cx);
                    window.click("changed-kind-option:vue", cx);
                });
            });
        }),
    },
    // the whole window at work (item 30.3), README's first picture: the fix's diff on screen and its file marked in
    // the tree, Claude's session as wide as its terminal came out
    State {
        name: "main",
        size: Size { width: px(1600.), height: px(1000.) },
        open: at_work,
        then: Some(|window, cx| {
            if let Some(terminal) = AT_WORK.with(|kept| kept.borrow_mut().take()) {
                terminal.feed(&enkidu_app::demo::working_session(terminal.size().cols as usize));
            }
            window.defer(cx, |window, cx| {
                window.render_frame(cx);
                if let Some(ix) = tree_row(window, "change-row", "src/orders/stock.py") {
                    window.click(("change-row", ix), cx);
                }
                window.defer(cx, |window, cx| {
                    window.render_frame(cx);
                    let down = gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-200.)));
                    window.scroll("diff-view", down, cx);
                    // typing goes to Claude: the terminal has the keyboard, and the pointer rests there
                    window.defer(cx, |window, cx| {
                        window.render_frame(cx);
                        window.click("terminal", cx);
                    });
                });
            });
        }),
    },
    // Search (item 4.1): its Files tab before anything is typed, with something typed, its Symbols and its Text
    State {
        name: "search-files",
        size: Size { width: px(1360.), height: px(860.) },
        open: |options, cx| reviewing(enkidu_model::ui::DiffLayout::Split, options, cx),
        then: Some(|window, cx| window.dispatch_action(Box::new(enkidu_app::SearchFiles), cx)),
    },
    State {
        name: "search-files-typed",
        size: Size { width: px(1360.), height: px(860.) },
        open: |options, cx| reviewing(enkidu_model::ui::DiffLayout::Split, options, cx),
        then: Some(|window, cx| {
            window.dispatch_action(Box::new(enkidu_app::SearchFiles), cx);
            type_in_search(window, cx, "stock");
        }),
    },
    State {
        name: "search-symbols",
        size: Size { width: px(1360.), height: px(860.) },
        open: |options, cx| reviewing(enkidu_model::ui::DiffLayout::Split, options, cx),
        then: Some(|window, cx| {
            window.dispatch_action(Box::new(enkidu_app::SearchSymbols), cx);
            type_in_search(window, cx, "slv");
        }),
    },
    State {
        name: "search-text",
        size: Size { width: px(1360.), height: px(860.) },
        open: |options, cx| reviewing(enkidu_model::ui::DiffLayout::Split, options, cx),
        then: Some(|window, cx| {
            window.dispatch_action(Box::new(enkidu_app::SearchText), cx);
            type_in_search(window, cx, "stock");
        }),
    },
    // the same with a line of a changed file picked (item 10.6): the file under the rows, with its marks
    State {
        name: "search-text-changed",
        size: Size { width: px(1360.), height: px(860.) },
        open: |options, cx| reviewing(enkidu_model::ui::DiffLayout::Split, options, cx),
        then: Some(|window, cx| {
            window.dispatch_action(Box::new(enkidu_app::SearchText), cx);
            window.defer(cx, |window, cx| {
                window.render_frame(cx);
                window.defer(cx, |window, cx| {
                    window.render_frame(cx);
                    window.input("stock", cx);
                    // the rows are there once what was typed was heard
                    window.defer(cx, |window, cx| {
                        window.render_frame(cx);
                        for _ in 0..4 {
                            window.press("down", cx);
                        }
                    });
                });
            });
        }),
    },
    // Settings (item 4.3): the General page, the Claude page and a project's page
    State {
        name: "settings-general",
        size: Size { width: px(1360.), height: px(860.) },
        open: demo,
        then: Some(|window, cx| window.dispatch_action(Box::new(enkidu_app::OpenSettings), cx)),
    },
    State {
        name: "settings-claude",
        size: Size { width: px(1360.), height: px(860.) },
        open: demo,
        then: Some(|window, cx| {
            window.dispatch_action(Box::new(enkidu_app::OpenSettings), cx);
            window.defer(cx, |window, cx| {
                window.render_frame(cx);
                window.click(("settings-nav", 1usize), cx);
            });
        }),
    },
    // the Claude page's second part (item 4.4): the Claude folders and what the hooks add
    State {
        name: "settings-hooks",
        size: Size { width: px(1360.), height: px(860.) },
        open: demo,
        then: Some(|window, cx| {
            window.dispatch_action(Box::new(enkidu_app::OpenSettings), cx);
            window.defer(cx, |window, cx| {
                window.render_frame(cx);
                window.click(("settings-nav", 1usize), cx);
                window.defer(cx, |window, cx| {
                    window.render_frame(cx);
                    let down = gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-2000.)));
                    window.scroll("settings-body", down, cx);
                    window.click("claude-hooks-show", cx);
                    window.render_frame(cx);
                    window.scroll("settings-body", down, cx);
                });
            });
        }),
    },
    State {
        name: "settings-project",
        size: Size { width: px(1360.), height: px(860.) },
        open: demo,
        then: Some(|window, cx| {
            window.dispatch_action(Box::new(enkidu_app::OpenSettings), cx);
            window.defer(cx, |window, cx| {
                window.render_frame(cx);
                window.click(("settings-nav", 3usize), cx);
            });
        }),
    },
    // Setup (items 21.5 and 21.6): everything there and signed in
    State {
        name: "setup-ready",
        size: Size { width: px(1360.), height: px(860.) },
        open: demo,
        then: Some(|window, cx| {
            cx.set_global(enkidu_app::demo::SetupSample::Ready);
            window.dispatch_action(Box::new(enkidu_app::OpenSettings), cx);
            window.defer(cx, |window, cx| {
                window.render_frame(cx);
                window.click("settings-nav-setup", cx);
            });
        }),
    },
    // the first launch on a Mac that is new to it: no project behind it, the welcome above the page, Claude Code
    // not installed, gh signed out, twg missing
    State {
        name: "setup-first-launch",
        size: Size { width: px(1360.), height: px(860.) },
        open: |options, cx| empty_with(vec![], options, cx),
        then: Some(|window, cx| {
            cx.set_global(enkidu_app::demo::SetupSample::New);
            let frame = cx.new(|_| enkidu_model::frame::Frame { loaded: true, ..Default::default() });
            enkidu_app::settings::open_welcome(frame, window, cx);
        }),
    },
    // Jira ready with a project that reads none of it yet, in a build without its Python server
    State {
        name: "setup-partly",
        size: Size { width: px(1360.), height: px(860.) },
        open: demo,
        then: Some(|window, cx| {
            cx.set_global(enkidu_app::demo::SetupSample::Partly);
            window.dispatch_action(Box::new(enkidu_app::OpenSettings), cx);
            window.defer(cx, |window, cx| {
                window.render_frame(cx);
                window.click("settings-nav-setup", cx);
                window.defer(cx, |window, cx| {
                    window.render_frame(cx);
                    let down = gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-2000.)));
                    window.scroll("settings-body", down, cx);
                });
            });
        }),
    },
    // the Workflows page (item 8.6): what a new workflow starts with, and the words of each kind of task
    State {
        name: "settings-workflows",
        size: Size { width: px(1360.), height: px(1100.) },
        open: demo,
        then: Some(|window, cx| {
            window.dispatch_action(Box::new(enkidu_app::OpenSettings), cx);
            window.defer(cx, |window, cx| {
                window.render_frame(cx);
                window.click(("settings-nav", 2usize), cx);
            });
        }),
    },
    // a project's page further down: what is read from GitHub and Jira (items 5.16a and 5.16)
    State {
        name: "settings-project-work",
        size: Size { width: px(1360.), height: px(860.) },
        open: demo,
        then: Some(|window, cx| {
            window.dispatch_action(Box::new(enkidu_app::OpenSettings), cx);
            window.defer(cx, |window, cx| {
                window.render_frame(cx);
                window.click(("settings-nav", 3usize), cx);
                window.defer(cx, |window, cx| {
                    window.render_frame(cx);
                    let down = gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-1010.)));
                    window.scroll("settings-body", down, cx);
                });
            });
        }),
    },
    // and its end: the workflows, their switches, the morning summary's time, the latest runs
    State {
        name: "settings-project-workflows",
        size: Size { width: px(1360.), height: px(860.) },
        open: demo,
        then: Some(|window, cx| {
            window.dispatch_action(Box::new(enkidu_app::OpenSettings), cx);
            window.defer(cx, |window, cx| {
                window.render_frame(cx);
                window.click(("settings-nav", 3usize), cx);
                window.defer(cx, |window, cx| {
                    window.render_frame(cx);
                    let down = gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-4000.)));
                    window.scroll("settings-body", down, cx);
                });
            });
        }),
    },
    // a name that leads to its definition, as it looks while ⌘ is held over it (item 10.10): `save` in line 30
    State {
        name: "definition-link",
        size: Size { width: px(1360.), height: px(1100.) },
        open: |options, cx| {
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| {
                    let terminal = std::sync::Arc::new(enkidu_term::Terminal::kept(
                        enkidu_term::GridSize::new(120, 36),
                        enkidu_term::Palette::enkidu(),
                    ));
                    let view = cx.new(|cx| enkidu_app::terminal_view::TerminalView::new(terminal, None, cx));
                    let layout = enkidu_model::ui::DiffLayout::Split;
                    let shell = Shell::with_changes(
                        enkidu_app::demo::tabs(),
                        vec![view],
                        enkidu_app::demo::changes(),
                        layout,
                        window,
                        cx,
                    );
                    let island = shell.changes().expect("the island").clone();
                    island.update(cx, |island, cx| island.open_file("src/orders/stock.py", Some(29), cx));
                    // once the file is drawn: the pointer rests on `save` in line 30 with ⌘ held, and the name is said
                    // to be a link (in the app the core says so)
                    let handle = window.window_handle();
                    cx.spawn(async move |_, cx| {
                        for _ in 0..150 {
                            cx.background_executor().timer(std::time::Duration::from_millis(20)).await;
                            let linked = handle.update(cx, |_, window, cx| {
                                window.render_frame(cx);
                                let Some(state) = island.read(cx).code_on_screen(cx) else { return false };
                                let points = (150..1000).step_by(8).flat_map(|y| {
                                    (60..900).step_by(4).map(move |x| gpui_kit::point(px(x as f32), px(y as f32)))
                                });
                                let on_save = |name: &enkidu_diff::DiffName| {
                                    name.position.line() == 30 && name.range.len() == 4 && name.range.start > 16
                                };
                                let found = { points }
                                    .find_map(|at| state.read(cx).name_at(at).filter(on_save).map(|name| (at, name)));
                                let Some((at, name)) = found else { return false };
                                state.update(cx, |state, cx| {
                                    state.point_with(true, at, cx);
                                    state.set_link(Some(name), cx);
                                });
                                true
                            });
                            if linked.unwrap_or(true) {
                                break;
                            }
                        }
                    })
                    .detach();
                    shell
                })
            })
            .expect("a window drawn offscreen")
            .0
        },
        then: None,
    },
    // the list of a name's definitions, when a ⌘-click finds several (item 10.11)
    State {
        name: "definitions",
        size: Size { width: px(1360.), height: px(860.) },
        open: |options, cx| {
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| {
                    let terminal = std::sync::Arc::new(enkidu_term::Terminal::kept(
                        enkidu_term::GridSize::new(120, 36),
                        enkidu_term::Palette::enkidu(),
                    ));
                    terminal.feed(&enkidu_app::demo::terminal_output());
                    let view = cx.new(|cx| enkidu_app::terminal_view::TerminalView::new(terminal, None, cx));
                    let layout = enkidu_model::ui::DiffLayout::Split;
                    let shell = Shell::with_changes(
                        enkidu_app::demo::tabs(),
                        vec![view],
                        enkidu_app::demo::changes(),
                        layout,
                        window,
                        cx,
                    );
                    let island = shell.changes().expect("the island").clone();
                    let place = |path: &str, line: u32, class: &str| enkidu_core::goto::Place {
                        path: path.into(),
                        line,
                        name: "save".into(),
                        kind: Some(enkidu_core::symbols::Kind::Method),
                        container: Some(class.into()),
                    };
                    let found = enkidu_core::goto::Found {
                        word: "save".into(),
                        start: 5,
                        end: 9,
                        later: false,
                        places: vec![
                            place("src/stock/models.py", 41, "StockItem"),
                            place("src/orders/models.py", 88, "OrderLine"),
                            place("src/orders/stock.py", 12, "StockLevel"),
                            place("backend/services/warehouse/apps/stock_levels/api/v2/views.py", 207, "LevelView"),
                        ],
                    };
                    let choosing = enkidu_app::definitions::Choosing {
                        workspace: "w0".into(),
                        from: ("src/orders/stock.py".into(), 31),
                        found,
                    };
                    window.defer(cx, move |window, cx| {
                        enkidu_app::definitions::open(choosing, island.downgrade(), window, cx)
                    });
                    shell
                })
            })
            .expect("a window drawn offscreen")
            .0
        },
        then: None,
    },
    // a file opened where the diffs are, at a line, with a remark on its lines (items 3.7 and 3.5e)
    State {
        name: "opened-file",
        size: Size { width: px(1360.), height: px(1100.) },
        open: |options, cx| {
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| {
                    let terminal = std::sync::Arc::new(enkidu_term::Terminal::kept(
                        enkidu_term::GridSize::new(120, 36),
                        enkidu_term::Palette::enkidu(),
                    ));
                    let view = cx.new(|cx| enkidu_app::terminal_view::TerminalView::new(terminal, None, cx));
                    let layout = enkidu_model::ui::DiffLayout::Split;
                    let shell = Shell::with_changes(
                        enkidu_app::demo::tabs(),
                        vec![view],
                        enkidu_app::demo::changes(),
                        layout,
                        window,
                        cx,
                    );
                    let island = shell.changes().expect("the island").clone();
                    let remarks = island.read(cx).remarks().clone();
                    remarks.update(cx, |remarks, cx| {
                        remarks.save(
                            enkidu_core::db::Remark {
                                id: "r1".into(),
                                workspace_id: "w0".into(),
                                path: "src/orders/stock.py".into(),
                                side: "new".into(),
                                start_line: 23,
                                end_line: 24,
                                snippet: "def restock(order: Order) -> None:\n    \"\"\"Puts a returned order's lines back into stock, once.\"\"\"".into(),
                                body: "Say here that an order can come back in parts.".into(),
                                created_at: 1,
                            },
                            cx,
                        );
                    });
                    island.update(cx, |island, cx| island.open_file("src/orders/stock.py", Some(29), cx));
                    shell
                })
            })
            .expect("a window drawn offscreen")
            .0
        },
        then: None,
    },
    // nothing changed, two remarks not sent yet: their files are listed for them (item 3.5d)
    State {
        name: "changes-only-remarks",
        size: Size { width: px(1360.), height: px(860.) },
        open: only_remarks,
        then: None,
    },
    // the same with "Add note" pressed and a note typed: the bar at the foot (item 3.6)
    State {
        name: "changes-note",
        size: Size { width: px(1360.), height: px(860.) },
        open: only_remarks,
        then: Some(|window, cx| {
            window.click("add-note", cx);
            // the note's field is made as the island is next drawn: the words are typed then
            window.defer(cx, |window, cx| {
                window.render_frame(cx);
                window.render_frame(cx);
                window.input("Then run the tests, and say which ones you added.", cx);
            });
        }),
    },
    // new files in a few languages, in one column: their syntax colors (item 3.3)
    State {
        name: "changes-languages",
        size: Size { width: px(1360.), height: px(2360.) },
        open: |options, cx| {
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| {
                    let terminal = std::sync::Arc::new(enkidu_term::Terminal::kept(
                        enkidu_term::GridSize::new(120, 36),
                        enkidu_term::Palette::enkidu(),
                    ));
                    let view = cx.new(|cx| enkidu_app::terminal_view::TerminalView::new(terminal, None, cx));
                    let layout = enkidu_model::ui::DiffLayout::Unified;
                    Shell::with_changes(
                        enkidu_app::demo::tabs(),
                        vec![view],
                        enkidu_app::demo::languages(),
                        layout,
                        window,
                        cx,
                    )
                })
            })
            .expect("a window drawn offscreen")
            .0
        },
        then: None,
    },
    // the Work page (phase 5): before anything was read, the tab's count while the workspace is on screen, and a
    // source that can't be read
    State {
        name: "work-first-run",
        size: Size { width: px(1360.), height: px(860.) },
        open: |options, cx| work_window(Default::default(), true, options, cx),
        then: None,
    },
    State {
        name: "work-tab",
        size: Size { width: px(1360.), height: px(860.) },
        open: |options, cx| work_window(work_sample(), false, options, cx),
        then: None,
    },
    // the board: the sample's fifteen cards in their nine columns
    State {
        name: "work-board",
        size: Size { width: px(1360.), height: px(860.) },
        open: |options, cx| work_window(work_sample(), true, options, cx),
        then: None,
    },
    State {
        name: "work-board-wide",
        size: Size { width: px(2400.), height: px(900.) },
        open: |options, cx| work_window(work_sample(), true, options, cx),
        then: None,
    },
    // cards with Claude's report and a workflow's time, and Jira signed out (its cards dimmed)
    State {
        name: "work-board-marks",
        size: Size { width: px(2400.), height: px(900.) },
        open: |options, cx| work_window(work_with_marks(), true, options, cx),
        then: None,
    },
    // tasks of your own on the board (item 8.5): "To do" has two, one stands with Jira's tickets in progress, one is
    // done; New task is in the header
    State {
        name: "work-own-board",
        size: Size { width: px(2700.), height: px(860.) },
        open: |options, cx| work_window(work_with_own(), true, options, cx),
        then: None,
    },
    // New task: the dialog, with the project to pick
    State {
        name: "work-own-new",
        size: Size { width: px(1360.), height: px(860.) },
        open: |options, cx| work_window(work_with_own(), true, options, cx),
        then: Some(|window, cx| {
            window.click("work-new-task", cx);
            window.defer(cx, |window, cx| {
                window.render_frame(cx);
                window.input("Write down how the small stack is started", cx);
            });
        }),
    },
    // a task of your own in front of the board: what you wrote about it, its own box with its state, Edit
    State {
        name: "work-own-card",
        size: Size { width: px(1360.), height: px(860.) },
        open: |options, cx| work_window(work_with_own(), true, options, cx),
        then: Some(|window, cx| window.click(gpui_kit::SharedString::from("work-card:own:1"), cx)),
    },
    // the header with everything in it: a Jira board picked, two cards hidden (one of them needs you), Needs you on,
    // and a read running
    State {
        name: "work-header",
        size: Size { width: px(1360.), height: px(860.) },
        open: |options, cx| work_window(work_header_sample(), true, options, cx),
        then: None,
    },
    // cards picked with X: the round boxes on every card, and the bar
    State {
        name: "work-picked",
        size: Size { width: px(1360.), height: px(860.) },
        open: |options, cx| work_window(work_sample(), true, options, cx),
        then: Some(|window, cx| {
            for key in ["down", "x", "right", "x", "right", "right", "x", "down"] {
                window.press(key, cx);
            }
        }),
    },
    // the workflow dialog for three picked cards (item 17.4): the chain at the left, and the step it opens on at the
    // right, the start: when it runs and what every task gets
    State {
        name: "work-flow-editor",
        size: Size { width: px(1500.), height: px(940.) },
        open: |options, cx| work_window(work_sample(), true, options, cx),
        then: Some(|window, cx| {
            for key in ["down", "x", "right", "x", "right", "right", "x", "down"] {
                window.press(key, cx);
            }
            window.defer(cx, |window, cx| {
                window.render_frame(cx);
                window.click("pick-workflow", cx);
            });
        }),
    },
    // the same with a task picked in the chain: its own setup at the right
    State {
        name: "work-flow-editor-task",
        size: Size { width: px(1500.), height: px(940.) },
        open: |options, cx| work_window(work_sample(), true, options, cx),
        then: Some(|window, cx| {
            for key in ["down", "x", "right", "x", "right", "right", "x", "down"] {
                window.press(key, cx);
            }
            window.defer(cx, |window, cx| {
                window.render_frame(cx);
                window.click("pick-workflow", cx);
                window.defer(cx, |window, cx| {
                    window.render_frame(cx);
                    window.click(("flow-task", 1usize), cx);
                });
            });
        }),
    },
    // the same for a card with remarks waiting, with a workflow to put it into: the whole message open, and the
    // name emptied, so the foot says what is missing and its buttons are off
    State {
        name: "work-flow-editor-message",
        size: Size { width: px(1500.), height: px(940.) },
        open: |options, cx| work_window(work_with_a_flow(), true, options, cx),
        then: Some(|window, cx| {
            for key in ["down", "right", "right", "right", "right", "right", "x"] {
                window.press(key, cx);
            }
            window.defer(cx, |window, cx| {
                window.render_frame(cx);
                window.click("pick-workflow", cx);
                window.defer(cx, |window, cx| {
                    window.render_frame(cx);
                    window.click(("flow-task", 0usize), cx);
                    window.render_frame(cx);
                    window.click(("flow-task-message-toggle", 0usize), cx);
                    window.click("flow-name", cx);
                    window.press("cmd-a", cx);
                    window.press("backspace", cx);
                });
            });
        }),
    },
    // a task's kinds of work, as its menu lists them
    State {
        name: "work-flow-editor-kinds",
        size: Size { width: px(1360.), height: px(860.) },
        open: |options, cx| work_window(work_sample(), true, options, cx),
        then: Some(|window, cx| {
            for key in ["down", "right", "right", "right", "right", "right", "x"] {
                window.press(key, cx);
            }
            window.defer(cx, |window, cx| {
                window.render_frame(cx);
                window.click("pick-workflow", cx);
                window.defer(cx, |window, cx| {
                    window.render_frame(cx);
                    window.click(("flow-task", 0usize), cx);
                    window.render_frame(cx);
                    window.click(("flow-task-kind", 0usize), cx);
                });
            });
        }),
    },
    // the Workflows page in the board's place: a workflow at its second task, the task that runs across the page
    State {
        name: "work-flows-running",
        size: Size { width: px(1728.), height: px(1000.) },
        open: |options, cx| {
            made_up_screenshots(cx);
            work_window(work_with_flows("running"), true, options, cx)
        },
        then: Some(|window, cx| window.click("work-flows", cx)),
    },
    // every run kept: last night's opens on the task that wants your review, its screenshots large
    State {
        name: "work-flows-runs",
        size: Size { width: px(1728.), height: px(1400.) },
        open: |options, cx| {
            made_up_screenshots(cx);
            work_window(work_with_flows("runs"), true, options, cx)
        },
        then: Some(|window, cx| window.click("work-flows", cx)),
    },
    // a run the usage limit stopped: it opens on the task that was cut off
    State {
        name: "work-flows-cut",
        size: Size { width: px(1360.), height: px(860.) },
        open: |options, cx| work_window(work_with_flows("cut"), true, options, cx),
        then: Some(|window, cx| window.click("work-flows", cx)),
    },
    // every state a task can end in, in one run: each its icon, its word and its color
    State {
        name: "work-flows-states",
        size: Size { width: px(1360.), height: px(1000.) },
        open: |options, cx| work_window(work_with_flows("states"), true, options, cx),
        then: Some(|window, cx| {
            window.click("work-flows", cx);
            window.defer(cx, |window, cx| {
                window.render_frame(cx);
                // the run is in the menu of the page's name
                window.click("flows-switch", cx);
                window.defer(cx, |window, cx| {
                    window.render_frame(cx);
                    window.click(("flow-run-item", 0usize), cx);
                });
            });
        }),
    },
    // the menu of the page's name (item 17.5): the workflows still to run, then every run kept, the one shown checked
    State {
        name: "work-flows-menu",
        size: Size { width: px(1728.), height: px(1000.) },
        open: |options, cx| {
            made_up_screenshots(cx);
            work_window(work_with_flows("runs"), true, options, cx)
        },
        then: Some(|window, cx| {
            window.click("work-flows", cx);
            window.defer(cx, |window, cx| {
                window.render_frame(cx);
                window.click("flows-switch", cx);
            });
        }),
    },
    State {
        name: "work-broken",
        size: Size { width: px(1360.), height: px(860.) },
        open: |options, cx| work_window(work_jira_broken(), true, options, cx),
        then: None,
    },
    // the card in front of the board: SH-21309, opened from the keyboard
    State {
        name: "work-card",
        size: Size { width: px(1360.), height: px(860.) },
        open: |options, cx| work_window(work_sample(), true, options, cx),
        then: Some(|window, cx| {
            for key in ["down", "right", "right", "right", "right", "right", "enter"] {
                window.press(key, cx);
            }
        }),
    },
    // what needs you in a tall window: every item, the second with the keyboard, the last left out of the message
    State {
        name: "work-card-needs",
        size: Size { width: px(1360.), height: px(1500.) },
        open: |options, cx| work_window(work_sample(), true, options, cx),
        then: Some(|window, cx| {
            for key in ["down", "right", "right", "right", "right", "right", "enter"] {
                window.press(key, cx);
            }
            window.defer(cx, |window, cx| {
                window.render_frame(cx);
                for key in ["j", "j", "j", "x", "k", "k"] {
                    window.press(key, cx);
                }
            });
        }),
    },
    // Tell Claude with your words written and the whole message shown, a new Claude picked
    State {
        name: "work-card-message",
        size: Size { width: px(1360.), height: px(1500.) },
        open: |options, cx| work_window(work_sample(), true, options, cx),
        then: Some(|window, cx| {
            for key in ["down", "right", "right", "right", "right", "right", "enter"] {
                window.press(key, cx);
            }
            window.defer(cx, |window, cx| {
                window.render_frame(cx);
                window.press("s", cx);
                window.input("Finish the export first; QA tests it tomorrow.", cx);
                window.press("escape", cx);
                window.click("task-preview-toggle", cx);
                window.click("claude-option:new", cx);
            });
        }),
    },
    // a description with every kind of block Jira writes, shown whole
    State {
        name: "work-card-doc",
        size: Size { width: px(1360.), height: px(1400.) },
        open: |options, cx| work_window(work_rich_ticket(), true, options, cx),
        then: Some(|window, cx| {
            for key in ["down", "right", "right", "right", "right", "right", "enter"] {
                window.press(key, cx);
            }
            window.defer(cx, |window, cx| {
                window.render_frame(cx);
                window.render_frame(cx);
                window.click("task-description-more", cx);
                window.defer(cx, |window, cx| {
                    window.render_frame(cx);
                    window.click("jira-doc:fold:1", cx);
                });
            });
        }),
    },
    // a PR's tab: what is going on there
    State {
        name: "work-card-pr",
        size: Size { width: px(1360.), height: px(1500.) },
        open: |options, cx| work_window(work_sample(), true, options, cx),
        then: Some(|window, cx| {
            for key in ["down", "right", "right", "right", "right", "right", "enter"] {
                window.press(key, cx);
            }
            window.defer(cx, |window, cx| {
                window.render_frame(cx);
                window.click("task-tab:pr:2414", cx);
            });
        }),
    },
    // its Activity tab
    State {
        name: "work-card-activity",
        size: Size { width: px(1360.), height: px(860.) },
        open: |options, cx| work_window(work_sample(), true, options, cx),
        then: Some(|window, cx| {
            for key in ["down", "right", "right", "right", "right", "right", "enter"] {
                window.press(key, cx);
            }
            window.defer(cx, |window, cx| {
                window.render_frame(cx);
                window.click("task-tab:activity", cx);
            });
        }),
    },
    // Claude's report first in the card: how it ended, its words, its screenshots as thumbnails
    State {
        name: "work-card-report",
        size: Size { width: px(1360.), height: px(1100.) },
        open: |options, cx| work_window(work_with_reports(), true, options, cx),
        then: Some(|window, cx| {
            for key in ["down", "right", "right", "right", "right", "right", "enter"] {
                window.press(key, cx);
            }
        }),
    },
    // the earlier reports opened under it
    State {
        name: "work-card-report-earlier",
        size: Size { width: px(1360.), height: px(1400.) },
        open: |options, cx| work_window(work_with_reports(), true, options, cx),
        then: Some(|window, cx| {
            for key in ["down", "right", "right", "right", "right", "right", "enter"] {
                window.press(key, cx);
            }
            window.defer(cx, |window, cx| {
                window.render_frame(cx);
                window.click("task-reports-more", cx);
            });
        }),
    },
    // a screenshot as large as the window, the second of three
    State {
        name: "work-card-shot",
        size: Size { width: px(1360.), height: px(860.) },
        open: |options, cx| work_window(work_with_reports(), true, options, cx),
        then: Some(|window, cx| {
            for key in ["down", "right", "right", "right", "right", "right", "enter"] {
                window.press(key, cx);
            }
            window.defer(cx, |window, cx| {
                window.render_frame(cx);
                window.click("report-image:7:2-orders.png", cx);
            });
        }),
    },
    // a whole page shown large: as tall as the window lets it be
    State {
        name: "work-card-shot-tall",
        size: Size { width: px(1360.), height: px(860.) },
        open: |options, cx| work_window(work_with_reports(), true, options, cx),
        then: Some(|window, cx| {
            for key in ["down", "right", "right", "right", "right", "right", "enter"] {
                window.press(key, cx);
            }
            window.defer(cx, |window, cx| {
                window.render_frame(cx);
                window.click("report-image:7:3-whole-page.png", cx);
            });
        }),
    },
    // a card that just moved to another column: its glow at three moments
    State {
        name: "work-card-moved",
        size: Size { width: px(900.), height: px(130.) },
        open: |options, cx| {
            gpui_kit::open_window(options, cx, |_, cx| cx.new(|_| MovedGlow))
                .expect("a window drawn offscreen")
                .0
                .into()
        },
        then: None,
    },
    // Check connections: a source that matches, one that can't be read, one that differs
    State {
        name: "work-check",
        size: Size { width: px(1360.), height: px(860.) },
        open: |options, cx| {
            check_answers(false, cx);
            work_window(work_sample(), true, options, cx)
        },
        then: Some(|window, cx| window.click("work-check", cx)),
    },
    // the same while the check still runs
    State {
        name: "work-check-running",
        size: Size { width: px(1360.), height: px(860.) },
        open: |options, cx| {
            check_answers(true, cx);
            work_window(work_sample(), true, options, cx)
        },
        then: Some(|window, cx| window.click("work-check", cx)),
    },
    // a ticket with nothing to tell: the facts alone, in a narrow card
    State {
        name: "work-card-narrow",
        size: Size { width: px(1360.), height: px(860.) },
        open: |options, cx| work_window(work_sample(), true, options, cx),
        then: Some(|window, cx| {
            for key in ["down", "right", "right", "right", "enter"] {
                window.press(key, cx);
            }
        }),
    },
    // every token and piece (item 1.12)
    State {
        name: "gallery",
        size: Size { width: px(1360.), height: px(1760.) },
        open: |options, cx| {
            gpui_kit::open_window(options, cx, |_, cx| cx.new(|_| enkidu_ui::gallery::Gallery::default()))
                .expect("a window drawn offscreen")
                .0
                .into()
        },
        then: None,
    },
];

/// A card's glow after it moved to another column, at its start, part of the way and settled (a motion can't be
/// stopped for a picture, so its three moments are drawn side by side).
struct MovedGlow;

impl gpui_kit::Render for MovedGlow {
    fn render(&mut self, _: &mut gpui_kit::Window, _: &mut gpui_kit::Context<Self>) -> impl gpui_kit::IntoElement {
        use enkidu_ui::tokens as theme;
        use gpui_kit::prelude::*;
        let moment = |n: usize, t: f32, says: &'static str| {
            let card = enkidu_ui::work::card_surface(("moved", n), false, false)
                .w(px(260.))
                .px(px(12.))
                .pt(px(8.))
                .pb(px(10.))
                .child(gpui_kit::div().text_size(px(12.)).text_color(theme::INK_3).child("SH-21309"))
                .child(
                    gpui_kit::div()
                        .text_size(px(13.))
                        .text_color(theme::INK)
                        .child("Export orders to the accounting CSV"),
                );
            gpui_kit::div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(enkidu_ui::work::card_moved(card, t))
                .child(gpui_kit::div().text_size(px(12.)).text_color(theme::INK_3).child(says))
        };
        gpui_kit::div()
            .size_full()
            .bg(theme::FRAME)
            .p(px(24.))
            .flex()
            .gap(px(24.))
            .child(moment(0, 0., "just moved"))
            .child(moment(1, 0.4, "0.6 s later"))
            .child(moment(2, 1., "settled"))
    }
}

/// Pictures side by side, for [`measure_report`]: nothing else is in the window.
struct Pictures(Vec<Arc<gpui_kit::RenderImage>>);

impl gpui_kit::Render for Pictures {
    fn render(&mut self, _: &mut gpui_kit::Window, _: &mut gpui_kit::Context<Self>) -> impl gpui_kit::IntoElement {
        use gpui_kit::prelude::*;
        gpui_kit::div()
            .size_full()
            .flex()
            .flex_wrap()
            .gap(px(8.))
            .children(self.0.iter().map(|picture| gpui_kit::img(picture.clone()).flex_none().w(px(320.)).h(px(200.))))
    }
}

/// `SHOTS_MEASURE=report`: what a report's screenshots cost the drawing thread where the window's own renderer draws
/// them (Metal, offscreen): a picture's pixels go to the GPU in the first frame that shows it, which a test window
/// never does. First that alone, for twelve thumbnails at once and for one whole-screen picture (2,880 by 1,800);
/// then the card with twelve such screenshots and one shown large, at rest. Run it in a release build; it prints the
/// lines that start with MEASURE.
fn measure_report(cx: &mut HeadlessAppContext) {
    const SHOTS: usize = 12;
    let picture = Arc::new(enkidu_testkit::busy_screenshot_png(2880, 1800, 1));
    {
        use enkidu_app::work::shots::{large_of, small_of};
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Default::default(),
                size: Size { width: px(1360.), height: px(860.) },
            })),
            focus: false,
            show: false,
            ..Default::default()
        };
        let (window, view) = cx.update(|cx| {
            gpui_kit::open_window(options, cx, |_, cx| cx.new(|_| Pictures(Vec::new()))).expect("a window")
        });
        let drawn = |pixels: image::RgbaImage| Arc::new(gpui_kit::RenderImage::new(vec![image::Frame::new(pixels)]));
        let small =
            || drawn(small_of(&picture, enkidu_model::work::report::THUMB_WIDTH_OF_MANY).expect("a thumbnail").0);
        let whole = || drawn(large_of(&picture, (3456, 2234)).expect("the whole picture"));
        // the pictures change and the frame is drawn in one go: its first frame, then the least of twenty more
        let mut shown = |pictures: Vec<Arc<gpui_kit::RenderImage>>| -> (f64, f64) {
            cx.update_window(window.into(), |_, window, cx| {
                view.update(cx, |view, cx| {
                    view.0 = pictures;
                    cx.notify();
                });
                let frame = |window: &mut gpui_kit::Window, cx: &mut App| {
                    let started = std::time::Instant::now();
                    window.render_frame(cx);
                    started.elapsed().as_secs_f64() * 1000.
                };
                let first = frame(window, cx);
                (first, (0..20).map(|_| frame(window, cx)).fold(f64::MAX, f64::min))
            })
            .expect("a frame")
        };
        shown(Vec::new());
        let (thumbs, thumbs_after) = shown((0..SHOTS).map(|_| small()).collect());
        let (one, one_after) = shown(vec![whole()]);
        let (again, _) = shown(vec![whole()]);
        println!(
            "MEASURE report: the first frame that shows {SHOTS} new thumbnails (640 by 400 each) {thumbs:.1} ms, the \
             frames after it {thumbs_after:.1} ms; the first frame that shows one whole screenshot (2,880 by 1,800) \
             {one:.1} ms and {again:.1} ms for the next one, the frames after it {one_after:.1} ms"
        );
    }
    let given = picture.clone();
    cx.update(|cx| cx.set_global(enkidu_app::work::shots::ShotSource(Arc::new(move |_, _| Ok(given.to_vec())))));
    let names: Vec<String> = (1..=SHOTS).map(|n| format!("{n}-screen.png")).collect();
    let mut index = work_sample();
    index.reports = vec![enkidu_core::work::reports::Report {
        id: 7,
        subject: "SH-21309".into(),
        session_id: Some("s-w2".into()),
        workspace_id: Some("w2".into()),
        state: "review".into(),
        text: "Fixed the CSV header.\n\nChecked the export with 3 orders.".into(),
        images: names.clone(),
        at: enkidu_testkit::WORK_SAMPLE_NOW - 60_000,
    }];
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: Default::default(),
            size: Size { width: px(1360.), height: px(860.) },
        })),
        focus: false,
        show: false,
        ..Default::default()
    };
    let handle = cx.update(|cx| {
        cx.set_reduce_motion(true);
        work_window(index, true, options, cx)
    });
    let frame = |cx: &mut HeadlessAppContext| -> f64 {
        cx.update_window(handle, |_, window, cx| {
            let started = std::time::Instant::now();
            window.render_frame(cx);
            started.elapsed().as_secs_f64() * 1000.
        })
        .expect("a frame")
    };
    // a frame drawn many times over: the least it takes is its own work
    let at_rest = |cx: &mut HeadlessAppContext| -> f64 { (0..20).map(|_| frame(cx)).fold(f64::MAX, f64::min) };
    for _ in 0..2 {
        frame(cx);
    }
    cx.update_window(handle, |_, window, cx| {
        for key in ["down", "right", "right", "right", "right", "right", "enter"] {
            window.press(key, cx);
        }
    })
    .expect("the card opens");
    frame(cx);
    let empty = at_rest(cx);
    // the twelve are read and shrunk, and shown
    cx.run_until_parked();
    frame(cx);
    let with_thumbs = at_rest(cx);
    println!(
        "MEASURE report: the card at rest {empty:.1} ms a frame before its screenshots are read, {with_thumbs:.1} ms \
         with its {SHOTS} thumbnails"
    );
    cx.update_window(handle, |_, window, cx| {
        window.click(gpui_kit::SharedString::from(format!("report-image:7:{}", names[0])), cx)
    })
    .expect("a thumbnail");
    frame(cx);
    let stand_in = at_rest(cx);
    cx.run_until_parked();
    frame(cx);
    let large = at_rest(cx);
    println!(
        "MEASURE report: one shown as large as the window {stand_in:.1} ms a frame with its thumbnail meanwhile, \
         {large:.1} ms with the whole picture"
    );
    let image = cx.capture_screenshot(handle).expect("Metal draws offscreen");
    let (w, h) = image.dimensions();
    println!("MEASURE report: drawn {w}x{h}");
}

fn main() {
    let out = std::env::var("SHOTS_DIR")
        .unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../../shots/out").to_string());
    std::fs::create_dir_all(&out).expect("the pictures' folder");
    let only: Vec<String> =
        std::env::var("SHOTS_ONLY").unwrap_or_default().split_whitespace().map(String::from).collect();
    if let Some(unknown) = only.iter().find(|name| !STATES.iter().any(|s| s.name == name.as_str())) {
        let known: Vec<&str> = STATES.iter().map(|s| s.name).collect();
        eprintln!("shots: no screen state named {unknown:?}; there are {}", known.join(", "));
        std::process::exit(2);
    }

    let mut cx = HeadlessAppContext::with_platform(
        gpui_kit::platform::current_platform(true).text_system(),
        Arc::new(enkidu_ui::assets::Assets),
        gpui_kit::platform::current_headless_renderer,
    );
    cx.update(enkidu_app::init);
    if std::env::var("SHOTS_MEASURE").as_deref() == Ok("report") {
        measure_report(&mut cx);
        return;
    }
    cx.update(made_up_screenshots);

    for state in STATES.iter().filter(|s| only.is_empty() || only.iter().any(|o| o == s.name)) {
        let started = std::time::Instant::now();
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds { origin: Default::default(), size: state.size })),
            focus: false,
            show: false,
            ..Default::default()
        };
        let handle = cx.update(|cx| (state.open)(options, cx));
        // the pictures show the app as the front one: its marks of where you are in blue, the keyboard's edge on.
        // Behind another app they rest (DESIGN.md, "States"): the states named "…-behind" show that
        if !state.name.ends_with("-behind") {
            cx.update_window(handle, |_, window, _| window.activate_window()).expect("the window comes in front");
            cx.run_until_parked();
        }
        // two frames: what lays itself out from the first (the tab pill's places) shows in the second
        for _ in 0..2 {
            cx.update_window(handle, |_, window, cx| window.render_frame(cx)).expect("a frame");
        }
        if let Some(then) = state.then {
            cx.update_window(handle, |_, window, cx| then(window, cx)).expect("the step before the picture");
        }
        // then the opening motion (a count popping in, the mark's wedges pressed in) runs its course, and the picture
        // shows where it settles
        cx.advance_clock(std::time::Duration::from_secs(2));
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| window.render_frame(cx)).expect("a frame");
        // what was read meanwhile (a report's screenshots) comes in with a motion of its own: once more
        cx.advance_clock(std::time::Duration::from_secs(1));
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| window.render_frame(cx)).expect("a frame");
        // `SHOTS_MEASURE=frames`: what one frame of this state costs where the real text system shapes it and Metal
        // draws it (a test window does neither). Run it in a release build; it prints lines that start with MEASURE.
        if std::env::var("SHOTS_MEASURE").as_deref() == Ok("frames") {
            let mut took: Vec<f64> = (0..40)
                .map(|_| {
                    let began = std::time::Instant::now();
                    cx.update_window(handle, |_, window, cx| {
                        window.refresh();
                        window.render_frame(cx)
                    })
                    .expect("a frame");
                    began.elapsed().as_secs_f64() * 1000.
                })
                .collect();
            took.sort_by(|a, b| a.total_cmp(b));
            println!(
                "MEASURE {}: a frame {:.2} ms in the middle, {:.2} ms at most (the state was made in {} ms)",
                state.name,
                took[took.len() / 2],
                took[took.len() - 1],
                started.elapsed().as_millis()
            );
        } else {
            let image = cx.capture_screenshot(handle).expect("Metal draws offscreen");
            let (w, h) = image.dimensions();
            let path = format!("{out}/{}.png", state.name);
            image.save(&path).expect("the PNG is written");
            println!("shots: {path} {w}x{h} in {} ms", started.elapsed().as_millis());
            // The edge of the tab on screen, read from the picture itself, half a pixel inside the pill's top:
            // signal blue in the front window, gray behind another app (DESIGN.md, "States").
            if matches!(state.name, "window" | "window-behind") {
                let pill = cx
                    .update_window(handle, |_, window, _| window.find("workspace-tabs-pill").bounds())
                    .expect("the pill of the tab on screen");
                let scale = w as f32 / f32::from(state.size.width);
                let at = |v: gpui_kit::Pixels, add: f32| ((f32::from(v) + add) * scale) as u32;
                let [red, green, blue, _] = image.get_pixel(at(pill.center().x, 0.), at(pill.top(), 0.5)).0;
                let bluer = blue as i32 - red as i32;
                match state.name {
                    "window" => assert!(bluer > 80, "the tab on screen's edge isn't blue: {red} {green} {blue}"),
                    _ => assert!(bluer.abs() < 40 && red > 70, "behind, it isn't gray: {red} {green} {blue}"),
                }
            }
            // The squint test of DESIGN.md, "States": the two top rows of five workspaces, blurred. The tab on
            // screen must still be the shape that stands out, and that is read from the blurred picture: inside
            // the pill against the bare row above it, 1.2 to 1 or more (before 14 Nov 2026 the pill stood at 1.05).
            if state.name == "tabs-narrow" {
                let scale = w as f32 / f32::from(state.size.width);
                let rows = (100. * scale) as u32;
                let top = image::imageops::crop_imm(&image, 0, 0, w, rows.min(h)).to_image();
                let squint = image::imageops::blur(&top, 4. * scale);
                let path = format!("{out}/tabs-squint.png");
                squint.save(&path).expect("the PNG is written");
                let pill = cx
                    .update_window(handle, |_, window, _| window.find("workspace-tabs-pill").bounds())
                    .expect("the pill of the tab on screen");
                // how light the blurred picture is at a place of the window (WCAG's relative luminance)
                let light = |x: f32, y: f32| {
                    let [red, green, blue, _] = squint.get_pixel((x * scale) as u32, (y * scale) as u32).0;
                    let lin = |c: u8| {
                        let c = c as f32 / 255.;
                        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
                    };
                    0.2126 * lin(red) + 0.7152 * lin(green) + 0.0722 * lin(blue)
                };
                // inside the pill, left of its lamp; and the bare row, the same way in, above the pill
                let (x, inside, above) =
                    (f32::from(pill.left()) + 6., f32::from(pill.center().y), f32::from(pill.top()));
                let (tab, row) = (light(x, inside), light(x, above - 12.));
                let apart = (tab.max(row) + 0.05) / (tab.min(row) + 0.05);
                assert!(apart >= 1.2, "blurred, the tab on screen is lost in its row: {apart:.2} to 1");
                println!("shots: {path} {w}x{rows}, the top rows blurred: the tab on screen at {apart:.2} to 1");
            }
        }
        // The window goes. One that stayed was drawn again whenever a later state moved the clock (a cursor blinks
        // in every terminal), so with forty windows open a picture took a minute where it takes two seconds alone.
        // (a picture of a drag leaves it held: let go of it, or the next state's first click would drop it there)
        cx.update_window(handle, |_, window, cx| {
            cx.stop_active_drag(window);
            window.remove_window()
        })
        .expect("the window closes");
        cx.run_until_parked();
    }
}
