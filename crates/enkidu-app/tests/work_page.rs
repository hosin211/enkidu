//! The Work page on screen with a given index (items 5.1c and on): the sample index (`enkidu_testkit::WORK_SAMPLE`) at
//! its fixed time. No core is behind these windows; what a press asks of the core is tested on the whole wiring
//! (tests/work_store.rs, tests/work_runs.rs).

use enkidu_app::shell::Shell;
use enkidu_app::work::store::Work;
use enkidu_core::db::{Project, Workspace};
use enkidu_core::work::model::Snapshot;
use enkidu_model::frame::{Frame, View};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, Entity, InputEvent as _, Modifiers, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ScrollDelta, SharedString, TestAppContext, Window, WindowBounds, WindowOptions,
    point, px, size,
};

const NOW: i64 = enkidu_testkit::WORK_SAMPLE_NOW;

fn sample() -> Snapshot {
    serde_json::from_str(enkidu_testkit::WORK_SAMPLE).expect("the sample Work index")
}

fn project(id: &str, name: &str, at: i64) -> Project {
    Project { id: id.into(), name: name.into(), path: format!("/code/{name}"), live_workspace_id: None, created_at: at }
}

fn ws(id: &str, name: &str, branch: &str) -> Workspace {
    Workspace {
        id: id.into(),
        project_id: "p1".into(),
        name: name.into(),
        path: format!("/w/{name}"),
        branch: branch.into(),
        base: Some("development".into()),
        session_id: format!("s-{id}"),
        transcript_path: None,
        claude_config_dir: None,
        created_at: 1,
        adopted: false,
    }
}

struct Page {
    window: AnyWindowHandle,
    shell: Entity<Shell>,
    work: Entity<Work>,
}

/// What these tests start from: shop and enkidu, the workspaces given (their Claudes working), the index given, and the
/// Work page open or not.
fn setup(cx: &mut TestAppContext, index: Snapshot, workspaces: Vec<Workspace>, open: bool) -> Page {
    let statuses = workspaces.iter().map(|w| (w.id.clone(), "working".to_string())).collect();
    let view = View { project_id: Some("p1".into()), work: open, ..Default::default() };
    let frame = Frame::load(vec![project("p1", "shop", 1), project("p2", "enkidu", 2)], workspaces, statuses, &view);
    cx.update(|cx| {
        enkidu_app::init(cx);
        // what is on screen is where it stays, so a test can read it without moving the clock
        cx.set_reduce_motion(true);
        let work = cx.new(|_| Work::given(index, &frame, NOW));
        let frame = cx.new(|_| frame);
        let (on, index) = (frame.clone(), work.clone());
        let (window, shell) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Default::default(),
                    size: size(px(1360.), px(860.)),
                })),
                ..Default::default()
            },
            cx,
            |window, cx| cx.new(|cx| Shell::with_work(on, index, window, cx)),
        )
        .expect("a test window");
        Page { window, shell, work }
    })
}

impl Page {
    fn look<R>(&self, cx: &mut TestAppContext, at: impl FnOnce(&mut Window) -> R) -> R {
        cx.run_until_parked();
        cx.update_window(self.window, |_, w, cx| {
            w.render_frame(cx);
            at(w)
        })
        .unwrap()
    }

    /// What the element says (its label); none when it isn't on screen.
    fn said(&self, cx: &mut TestAppContext, id: &'static str) -> Option<String> {
        self.look(cx, |w| w.try_find(id).and_then(|e| e.label().map(String::from)))
    }

    fn there(&self, cx: &mut TestAppContext, id: &'static str) -> bool {
        self.look(cx, |w| w.try_find(id).is_some())
    }

    fn press(&self, cx: &mut TestAppContext, key: &str) {
        cx.run_until_parked();
        cx.update_window(self.window, |_, w, cx| {
            w.render_frame(cx);
            w.press(key, cx)
        })
        .unwrap();
    }

    fn click(&self, cx: &mut TestAppContext, id: &'static str) {
        cx.run_until_parked();
        cx.update_window(self.window, |_, w, cx| {
            w.render_frame(cx);
            w.click(id, cx)
        })
        .unwrap();
    }

    fn open(&self, cx: &mut TestAppContext) -> bool {
        cx.run_until_parked();
        self.shell.read_with(cx, |shell, cx| shell.work_open(cx))
    }
}

/// Item 5.1c: the tab counts what waits on you while the page is closed.
#[gpui_kit::test]
fn work_is_the_first_tab_of_the_projects_row_and_cmd_i_opens_the_page_and_goes_back(cx: &mut TestAppContext) {
    let workspaces = vec![ws("w1", "stock-fix", "fix/stock-double-count")];
    let page = setup(cx, sample(), workspaces, false);
    // closed: the first tab of the row, before the projects, with the red count of what waits on you
    assert!(!page.open(cx));
    assert_eq!(page.said(cx, "work-tab").as_deref(), Some("Work: 9 waiting on you on 2 PRs"));
    assert_eq!(page.said(cx, "work-tab-count").as_deref(), Some("9"));
    let (tab, first, pressed) = page.look(cx, |w| {
        let tab = w.find("work-tab");
        (tab.bounds(), w.find(("project", 0usize)).bounds(), tab.checked())
    });
    assert!(tab.right() <= first.left(), "Work comes before the first project: {tab:?} {first:?}");
    assert_eq!(pressed, Some(false));
    assert!(page.look(cx, |w| w.try_find(("tab", 0usize)).is_some()), "the workspace's tabs are there");
    assert!(page.there(cx, "status-line") && !page.there(cx, "work-page"));

    // ⌘I: the page takes the place of the tabs and the islands; the projects row and the status line stay
    page.press(cx, "cmd-i");
    assert!(page.open(cx));
    assert!(page.there(cx, "work-page") && page.there(cx, "projects-row"));
    assert!(page.look(cx, |w| w.try_find(("tab", 0usize)).is_none()), "no workspace tabs behind the page");
    assert!(!page.there(cx, "status-line"));
    assert_eq!(
        page.said(cx, "work-status-line").as_deref(),
        Some("GitHub read 2 min ago as sam-lee | Jira read 2 min ago as Sam Lee")
    );
    assert_eq!(page.said(cx, "work-status-github").as_deref(), Some("GitHub read 2 min ago as sam-lee"));
    // the tab is pressed in, and its count rests while you are on the page
    assert_eq!(page.look(cx, |w| w.find("work-tab").checked()), Some(true));
    assert!(!page.there(cx, "work-tab-count"));
    // the page is as wide as the window, between the two rows
    let (row, body, line, window) = page.look(cx, |w| {
        let bounds = |id: &'static str| w.find(id).bounds();
        (bounds("projects-row"), bounds("work-page"), bounds("work-status-line"), w.viewport_size())
    });
    assert_eq!((body.left(), body.right()), (px(0.), window.width));
    assert_eq!((body.top(), body.bottom()), (row.bottom(), line.top()));

    // Esc, with the keyboard on the page, goes back; so does ⌘I, and the tab itself
    page.press(cx, "escape");
    assert!(!page.open(cx), "Esc goes back");
    assert!(page.there(cx, "status-line") && !page.there(cx, "work-page"));
    assert_eq!(page.said(cx, "work-tab-count").as_deref(), Some("9"));
    // the keyboard went back to the workspace: its keys work at once
    page.press(cx, "cmd-i");
    assert!(page.open(cx), "⌘I opens it again");
    page.press(cx, "cmd-i");
    assert!(!page.open(cx), "and goes back");
    page.click(cx, "work-tab");
    assert!(page.open(cx), "a click on the tab opens it");
    page.press(cx, "escape");
    assert!(!page.open(cx), "and Esc goes back from there too");
    page.click(cx, "work-tab");
    page.click(cx, "work-tab");
    assert!(!page.open(cx), "a click on the pressed tab goes back");

    // behind the page no project is the one on screen: each is a plain tab, and a click on one brings it back (the
    // one you were in too); so does a workspace's number
    let first_project = |page: &Page, cx: &mut TestAppContext| {
        page.look(cx, |w| w.find(("project", 0usize)).label().map(String::from)).unwrap_or_default()
    };
    assert_eq!(first_project(&page, cx), "shop: project menu", "on screen, it opens its menu");
    page.press(cx, "cmd-i");
    assert_eq!(first_project(&page, cx), "shop", "behind the page it is a tab like the others");
    cx.update_window(page.window, |_, w, cx| w.click(("project", 0usize), cx)).unwrap();
    assert!(!page.open(cx), "a click on the project you were in goes back to it");
    assert!(page.look(cx, |w| w.try_find(("tab", 0usize)).is_some()));
    page.press(cx, "cmd-i");
    page.press(cx, "cmd-1");
    assert!(!page.open(cx), "⌘1 brings the first workspace on screen");
}

/// Before the first read, offers to check, with nothing to filter.
#[gpui_kit::test]
fn before_the_first_read_offers_to_check_with_nothing_to_filter(cx: &mut TestAppContext) {
    let page = setup(cx, Snapshot::default(), Vec::new(), true);
    assert_eq!(page.said(cx, "work-first-run").as_deref(), Some("Your tickets and PRs, as a board"));
    assert_eq!(page.said(cx, "work-check-now").as_deref(), Some("Check now"));
    assert!(!page.there(cx, "work-needs-you") && !page.there(cx, "work-board") && !page.there(cx, "work-query"));
    // nothing was read: the tab has only its name, and the status line says nothing of the sources
    assert_eq!(page.said(cx, "work-tab").as_deref(), Some("Work"));
    assert!(!page.there(cx, "work-tab-count") && !page.there(cx, "work-tab-broken"));
    assert_eq!(page.said(cx, "work-status-line").as_deref(), Some(""));
    // once something was read, the first run's words are gone
    let read = sample();
    page.work.update(cx, |work, cx| {
        work.snapshot = Some(std::sync::Arc::new(read));
        work.regroup();
        cx.notify();
    });
    assert!(!page.there(cx, "work-first-run"));
}

/// Says first what can't be read, and what to do, and marks the tab.
#[gpui_kit::test]
fn says_first_what_cant_be_read_and_what_to_do_and_marks_the_tab(cx: &mut TestAppContext) {
    let mut index = sample();
    index.health[1].state = "error".into();
    index.health[1].error = Some("Jira: twg isn't signed in".into());
    index.health[1].fix = Some("Run `twg login --force`.".into());
    index.health[1].last_ok = Some(NOW - 42 * 60_000);
    let page = setup(cx, index, Vec::new(), true);
    // the line says what is wrong and what to do (its command as code, without the backticks)
    let line = page.said(cx, "broken-jira").expect("the line of the source that can't be read");
    assert_eq!(line, "Jira can't be read: twg isn't signed in. Run twg login --force.");
    let under = page.look(cx, |w| w.find("broken-jira").description().map(String::from)).unwrap_or_default();
    assert!(under.contains("from its last good read, 42 min ago"), "{under}");
    assert!(!page.there(cx, "broken-github"));
    // it stands above the board's place, under the header row
    let (line, body) = page.look(cx, |w| (w.find("broken-jira").bounds(), w.find("work-page").bounds()));
    assert!(line.top() >= body.top() + px(40.) && line.left() == px(8.), "{line:?}");
    // the tab's red mark comes before any count, and the status line says it in red words
    assert_eq!(page.said(cx, "work-tab").as_deref(), Some("Work: a source can't be read"));
    assert_eq!(page.said(cx, "work-tab-broken").as_deref(), Some("A source can't be read"));
    assert!(!page.there(cx, "work-tab-count"));
    assert_eq!(page.said(cx, "work-status-jira").as_deref(), Some("Jira can't be read"));
    assert_eq!(page.said(cx, "work-status-github").as_deref(), Some("GitHub read 2 min ago as sam-lee"));
    // closed, the mark stays on the tab
    page.press(cx, "escape");
    assert!(!page.open(cx));
    assert_eq!(page.said(cx, "work-tab-broken").as_deref(), Some("A source can't be read"));
}

/// The columns on screen: each one's name and what it says of its count.
fn columns(page: &Page, cx: &mut TestAppContext) -> Vec<(String, String)> {
    page.look(cx, |w| {
        (0..40usize)
            .map_while(|ix| w.try_find(("work-column", ix)))
            .map(|column| {
                let name = column.label().unwrap_or_default().to_string();
                (name, column.description().unwrap_or_default().to_string())
            })
            .collect()
    })
}

/// A card's id on screen.
fn card(key: &str) -> SharedString {
    let id = if key.starts_with("pr:") { key.to_string() } else { format!("ticket:{key}") };
    SharedString::from(format!("work-card:{id}"))
}

/// What a card says, as one line; none when it isn't on the board.
fn card_says(page: &Page, cx: &mut TestAppContext, key: &str) -> Option<String> {
    let id = card(key);
    page.look(cx, |w| w.try_find(id).and_then(|card| card.description().map(String::from)))
}

/// Items 5.2 and 5.3: shows everything read: a column per Jira status (the backlog's too, none folded), a card per
/// ticket, no project chips.
#[gpui_kit::test]
fn shows_everything_read_a_column_per_jira_status_a_card_per_ticket_no_project_chips(cx: &mut TestAppContext) {
    // the workspace whose Claude works on SH-21309
    let page = setup(cx, sample(), vec![ws("w2", "sh-21309-export", "feature/SH-21309-export")], true);
    let named: Vec<(String, String)> = columns(&page, cx);
    let expected = [
        ("On Hold", "1 card"),
        ("New", "1 card"),
        ("Analysed", "1 card"),
        ("Ready for Dev", "3 cards"),
        ("Planned for current sprint", "1 card"),
        ("In Progress", "3 cards"),
        ("Code Review", "3 cards"),
        ("QA Review", "1 card"),
        ("Done", "1 card"),
    ];
    assert_eq!(named, expected.map(|(name, count)| (name.to_string(), count.to_string())));
    // a card says what the ticket is, how its PR stands in a word or two, and how much waits on you
    let export = card_says(&page, cx, "SH-21309").expect("SH-21309's card");
    assert_eq!(
        export,
        "Story SH-21309, High priority | Export orders to the accounting CSV | #2414 Checks failing | \
         Claude in sh-21309-export, working"
    );
    assert_eq!(
        page.said(cx, "card-needs:ticket:SH-21309").as_deref(),
        Some("Needs you: checks failing, 2 remarks waiting, 1 question")
    );
    assert_eq!(page.said(cx, "card-claude:ticket:SH-21309").as_deref(), Some("sh-21309-export"));
    assert_eq!(page.said(cx, "card-pr:ticket:SH-21309:2414").as_deref(), Some("#2414 Checks failing"));
    assert!(!page.there(cx, "card-claude:ticket:SH-21376"), "a session outside Enkidu isn't on the card");
    assert_eq!(page.said(cx, "card-pr:ticket:SH-21030:2312").as_deref(), Some("#2312 Merged"));
    assert!(card_says(&page, cx, "SH-20728").is_some(), "the backlog's tickets are on the board");
    // a PR that names no ticket is a card of its own, in the column its state matches
    let lone = card_says(&page, cx, "pr:demo-org/shop#2415").expect("the PR's card");
    assert_eq!(lone, "Open pull request shop#2415 | Bump Django to 5.2.8 | Waiting for review");
    // the projects are the tabs above: the page has no chips for them; no column folds; nothing hidden, no Hidden
    assert!(!page.there(cx, "work-project") && !page.there(cx, "work-column-fold"));
    assert!(!page.there(cx, "work-show-hidden"));
    assert!(page.said(cx, "work-status-line").unwrap_or_default().contains("GitHub read"));

    // each column is an island at least 244 px wide, side by side with 8 px between, from 8 px inside the window,
    // under the 40 px header row, down to 8 px above the status line
    let (first, second, body, line) = page.look(cx, |w| {
        let bounds = |ix: usize| w.find(("work-column", ix)).bounds();
        (bounds(0), bounds(1), w.find("work-page").bounds(), w.find("work-status-line").bounds())
    });
    assert_eq!((first.left(), first.size.width), (px(8.), px(244.)), "{first:?}");
    assert_eq!(second.left(), first.right() + px(8.));
    assert_eq!((first.top(), first.bottom()), (body.top() + px(40.), line.top() - px(8.)));
    // its cards are stacked in it, 8 px from its edges and from each other, under its 40 px head
    let (supplier, import, tablet, ready) = page.look(cx, |w| {
        let card = |key: &str| w.find(card(key)).bounds();
        (card("SH-20728"), card("SH-20781"), card("SH-20947"), w.find(("work-column", 3usize)).bounds())
    });
    assert_eq!((supplier.left(), supplier.right()), (ready.left() + px(9.), ready.right() - px(9.)), "{supplier:?}");
    assert_eq!(supplier.top(), ready.top() + px(41.));
    assert_eq!((import.top(), tablet.top()), (supplier.bottom() + px(8.), import.bottom() + px(8.)));
    // a title of two lines makes its card taller
    assert!(import.size.height > supplier.size.height, "{import:?} {supplier:?}");

    // nine columns don't fit 1,360 px: the board goes on to the right, and that edge fades
    let more = |page: &Page, cx: &mut TestAppContext| {
        page.look(cx, |w| w.find("work-board").description().unwrap_or_default().to_string())
    };
    assert_eq!(more(&page, cx), "More to the right");
    let last = page.look(cx, |w| w.find(("work-column", 8usize)).bounds());
    assert!(last.left() > px(1360.), "Done is past the window's edge: {last:?}");
    let scroll = |page: &Page, cx: &mut TestAppContext, by: f32| {
        cx.update_window(page.window, |_, w, cx| {
            w.render_frame(cx);
            w.scroll("work-board", ScrollDelta::Pixels(point(px(by), px(0.))), cx)
        })
        .unwrap();
    };
    scroll(&page, cx, -300.);
    assert_eq!(more(&page, cx), "More to the left and to the right");
    assert_eq!(page.look(cx, |w| w.find(("work-column", 0usize)).bounds()).left(), px(8. - 300.));
    scroll(&page, cx, -5000.);
    assert_eq!(more(&page, cx), "More to the left");
    let last = page.look(cx, |w| w.find(("work-column", 8usize)).bounds());
    assert_eq!(last.right(), px(1360. - 8.), "scrolled to its end, Done ends 8 px inside the window");

    // in a window wide enough for all nine they share its width evenly, and no edge fades
    cx.simulate_window_resize(page.window, size(px(2400.), px(900.)));
    let widths: Vec<f32> =
        page.look(cx, |w| (0..9usize).map(|ix| f32::from(w.find(("work-column", ix)).bounds().size.width)).collect());
    // to within the half pixel the layout rounds to
    assert!(widths.iter().all(|width| (width - widths[0]).abs() <= 0.5 && *width > 244.), "{widths:?}");
    assert_eq!(more(&page, cx), "");
    // and in a very wide one a column stops at 380 px
    cx.simulate_window_resize(page.window, size(px(4200.), px(900.)));
    let width = page.look(cx, |w| w.find(("work-column", 0usize)).bounds().size.width);
    assert_eq!(width, px(380.));
}

/// Item 5.2: a column with more cards than fit scrolls on its own, the others stay, and only the cards in view are
/// drawn (a column is a list).
#[gpui_kit::test]
fn a_column_with_many_cards_scrolls_on_its_own_and_draws_only_the_cards_in_view(cx: &mut TestAppContext) {
    let mut index = sample();
    let supplier = index.tickets.iter().find(|t| t.key == "SH-20728").expect("the sample's SH-20728").clone();
    for n in 0..40 {
        let key = format!("SH-{}", 20000 + n);
        index.ticket_projects.insert(key.clone(), "p1".into());
        index.tickets.push(enkidu_core::work::model::Ticket {
            summary: format!("Backlog task {n}"),
            key,
            ..supplier.clone()
        });
    }
    let page = setup(cx, index, Vec::new(), true);
    assert_eq!(columns(&page, cx)[3], ("Ready for Dev".to_string(), "43 cards".to_string()));
    // the column's cards in their order, as the board has them
    let keys: Vec<String> = page.shell.read_with(cx, |shell, cx| {
        let board = shell.work_page().expect("the Work page").read(cx).board();
        board.columns[3].cards.iter().map(|card| card.name().to_string()).collect()
    });
    assert_eq!(keys.len(), 43);
    let drawn = |page: &Page, cx: &mut TestAppContext| -> Vec<usize> {
        let keys = keys.clone();
        page.look(cx, move |w| (0..keys.len()).filter(|n| w.try_find(card(&keys[*n])).is_some()).collect())
    };
    // 43 cards of 62 px and more don't fit 770 px: the first ones are drawn, the last ones aren't until they come
    let first = drawn(&page, cx);
    assert_eq!(first[0], 0);
    assert!(first.len() < 25 && !first.contains(&42), "only the cards in view are drawn: {first:?}");
    let (sixth, other) = page.look(cx, |w| (w.find(card(&keys[5])).bounds(), w.find(card("SH-21309")).bounds()));
    let wheel = |page: &Page, cx: &mut TestAppContext, by: f32| {
        cx.update_window(page.window, |_, w, cx| {
            w.render_frame(cx);
            w.scroll(("work-cards", 3usize), ScrollDelta::Pixels(point(px(0.), px(by))), cx)
        })
        .unwrap();
    };
    wheel(&page, cx, -200.);
    let (sixth_after, other_after) =
        page.look(cx, |w| (w.find(card(&keys[5])).bounds(), w.find(card("SH-21309")).bounds()));
    assert_eq!(sixth_after.top(), sixth.top() - px(200.), "its cards moved up");
    assert_eq!(other_after, other, "the other columns stayed");
    // the column's head stays where it is
    let head = page.look(cx, |w| w.find(("work-column", 3usize)).bounds());
    assert_eq!(head.top(), px(38. + 40.), "{head:?}");
    // scrolled to its end: the last card is drawn, 8 px above the column's foot, and the first ones no longer
    wheel(&page, cx, -20000.);
    let last = drawn(&page, cx);
    assert!(last.contains(&42) && !last.contains(&0), "{last:?}");
    let (end, column) = page.look(cx, |w| (w.find(card(&keys[42])).bounds(), w.find(("work-column", 3usize)).bounds()));
    assert_eq!(end.bottom(), column.bottom() - px(9.), "{end:?} {column:?}");
    // the index is read again with the same cards: the column stays where it was scrolled to
    page.work.update(cx, |work, cx| {
        work.regroup();
        cx.notify();
    });
    assert_eq!(drawn(&page, cx), last);
}

/// Claude's reports on SH-21309 (a review with two screenshots, after an earlier update) and a workflow that runs
/// it now, with SH-21376 waiting its turn.
fn with_report_and_workflow(mut index: Snapshot) -> Snapshot {
    index.reports = serde_json::from_value(serde_json::json!([
        { "id": 3, "subject": "SH-21309", "sessionId": "s-w2", "workspaceId": "w2", "state": "update",
          "text": "Halfway: the writer is done", "images": [], "at": NOW - 5 * 60_000 },
        { "id": 2, "subject": "SH-21309", "sessionId": "s-w2", "workspaceId": "w2", "state": "review",
          "text": "## Export works\n\nAll 14 tests pass.", "images": ["a.png", "b.png"], "at": NOW - 120 * 60_000 },
    ]))
    .expect("two reports");
    let task = |subject: &str, state: &str| {
        serde_json::json!({ "subject": subject, "title": subject, "message": "", "state": state,
            "target": { "kind": "workspace", "workspaceId": "w2" } })
    };
    index.flows = serde_json::from_value(serde_json::json!([{
        "id": "f1", "projectId": "p1", "name": "Night shift", "instruction": "", "startAt": NOW - 60_000,
        "minutes": 60, "live": true, "state": "running", "current": 0,
        "tasks": [task("ticket:SH-21309", "running"), task("ticket:SH-21376", "waiting")],
    }]))
    .expect("a workflow");
    index
}

/// Item 5.3: a card says how Claude's latest report says the task ended, and that its workflow runs it now or next;
/// a card whose waiting items come from a source that can't be read is dimmed, and says why when pointed at.
#[gpui_kit::test]
fn a_card_carries_its_report_its_workflow_time_and_says_when_its_source_cant_be_read(cx: &mut TestAppContext) {
    let mut index = with_report_and_workflow(sample());
    index.health[1].state = "error".into();
    index.health[1].error = Some("Jira: twg isn't signed in".into());
    let page = setup(cx, index, vec![ws("w2", "sh-21309-export", "feature/SH-21309-export")], true);
    // the report: how it ended, with its screenshots counted; the update along the way says nothing about that
    assert_eq!(page.said(cx, "card-report:ticket:SH-21309").as_deref(), Some("Review"));
    let words = page.look(cx, |w| w.find("card-report:ticket:SH-21309").description().map(String::from));
    assert_eq!(words.as_deref(), Some("Finished, wants your review, 2 h ago: Export works"));
    assert!(!page.there(cx, "card-report:ticket:SH-21533"));
    // the workflow: the task that runs says Now, the one waiting its turn Next
    assert_eq!(page.said(cx, "card-flow:ticket:SH-21309").as_deref(), Some("Now"));
    assert_eq!(page.said(cx, "card-flow:ticket:SH-21376").as_deref(), Some("Next"));
    let words = page.look(cx, |w| w.find("card-flow:ticket:SH-21376").description().map(String::from));
    assert_eq!(words.as_deref(), Some("Waiting its turn in the workflow Night shift"));
    let says = card_says(&page, cx, "SH-21309").unwrap_or_default();
    assert!(says.ends_with("Claude in sh-21309-export, working | Report: Review | Workflow: Now"), "{says}");
    // they share the card's last line and its first: the report at the right of the Claude, the time before the badge
    let (claude, report, flow, needs, whole) = page.look(cx, |w| {
        let at = |id: &'static str| w.find(id).bounds();
        (
            at("card-claude:ticket:SH-21309"),
            at("card-report:ticket:SH-21309"),
            at("card-flow:ticket:SH-21309"),
            at("card-needs:ticket:SH-21309"),
            w.find(card("SH-21309")).bounds(),
        )
    });
    assert!(claude.right() <= report.left() && report.right() <= whole.right() - px(12.), "{claude:?} {report:?}");
    assert!(flow.right() <= needs.left() && needs.right() <= whole.right() - px(12.), "{flow:?} {needs:?}");
    // Jira can't be read: the card whose items come from Jira says so when pointed at; one with GitHub's doesn't
    // (QA Review is past the window.s edge at 1,360 px: in a wider window it is in view)
    cx.simulate_window_resize(page.window, size(px(2400.), px(900.)));
    cx.update_window(page.window, |_, w, cx| {
        w.render_frame(cx);
        w.hover(card("SH-21112"), cx)
    })
    .unwrap();
    cx.executor().advance_clock(std::time::Duration::from_millis(900));
    assert_eq!(page.said(cx, "tip").as_deref(), Some("From the last good read: this source can't be read right now"));
}

/// The cards on the board, in its order: down each column, then on to the next.
fn shown(page: &Page, cx: &mut TestAppContext) -> Vec<String> {
    cx.run_until_parked();
    page.shell.read_with(cx, |shell, cx| {
        let board = shell.work_page().expect("the Work page").read(cx).board();
        board.shown().into_iter().map(String::from).collect()
    })
}

/// What the page view itself holds: the card the keyboard is on, the cards picked, the card open.
fn state(page: &Page, cx: &mut TestAppContext) -> (Option<String>, Vec<String>, Option<String>) {
    cx.run_until_parked();
    page.shell.read_with(cx, |shell, cx| {
        let view = shell.work_page().expect("the Work page").read(cx);
        (view.selected().map(String::from), view.picked().to_vec(), view.card().map(String::from))
    })
}

/// A click with keys held (⇧, ⌘), in the middle of the element.
fn click_with(page: &Page, cx: &mut TestAppContext, id: SharedString, modifiers: Modifiers) {
    cx.run_until_parked();
    cx.update_window(page.window, |_, w, cx| {
        w.render_frame(cx);
        let position = w.find(id).bounds().center();
        let left = MouseButton::Left;
        w.dispatch_event(MouseMoveEvent { position, pressed_button: None, modifiers }.to_platform_input(), cx);
        let down = MouseDownEvent { button: left, position, modifiers, click_count: 1, first_mouse: false };
        w.dispatch_event(down.to_platform_input(), cx);
        w.dispatch_event(MouseUpEvent { button: left, position, modifiers, click_count: 1 }.to_platform_input(), cx);
        w.render_frame(cx);
    })
    .unwrap();
}

fn click_id(page: &Page, cx: &mut TestAppContext, id: SharedString) {
    click_with(page, cx, id, Modifiers::default());
}

fn typed(page: &Page, cx: &mut TestAppContext, text: &str) {
    cx.run_until_parked();
    cx.update_window(page.window, |_, w, cx| w.input(text, cx)).unwrap();
}

/// A value an element carries (a count, a lamp's state).
fn value(page: &Page, cx: &mut TestAppContext, id: &'static str) -> Option<String> {
    page.look(cx, |w| w.try_find(id).and_then(|e| e.value().map(String::from)))
}

fn described(page: &Page, cx: &mut TestAppContext, id: &'static str) -> Option<String> {
    page.look(cx, |w| w.try_find(id).and_then(|e| e.description().map(String::from)))
}

fn pressed_in(page: &Page, cx: &mut TestAppContext, id: &'static str) -> Option<bool> {
    page.look(cx, |w| w.try_find(id).and_then(|e| e.checked()))
}

/// Whether an element is the one chosen among its like (a step of a chain).
fn chosen(page: &Page, cx: &mut TestAppContext, id: impl Into<gpui_kit::ElementId>) -> Option<bool> {
    let id = id.into();
    page.look(cx, |w| w.try_find(id).and_then(|e| e.selected()))
}

/// Item 5.4: filters by text and shows only what needs you.
#[gpui_kit::test]
fn the_header_filters_by_text_shows_only_what_needs_you_and_says_how_each_source_stands(cx: &mut TestAppContext) {
    let page = setup(cx, sample(), Vec::new(), true);
    // the row, where the workspace's tabs were: 40 px, from the board menu to Refresh
    let (row, pick, filter, github, jira, check, refresh) = page.look(cx, |w| {
        let at = |id: &'static str| w.find(id).bounds();
        (
            at("work-header"),
            at("work-board-pick"),
            at("work-filter"),
            at("health-github"),
            at("health-jira"),
            at("work-check"),
            at("work-refresh"),
        )
    });
    assert_eq!((row.top(), row.size.height, row.size.width), (px(38.), px(40.), px(1360.)));
    assert_eq!(pick.left(), px(8.));
    assert_eq!(filter.size.width, px(200.));
    assert!(filter.right() <= github.left() && github.right() <= jira.left() && jira.right() <= check.left());
    assert_eq!((check.right() + px(4.), refresh.right()), (refresh.left(), px(1360. - 8.)));
    // nothing is hidden: no Tasks and Hidden; four cards need you; every source read
    assert_eq!(page.said(cx, "work-board-pick").as_deref(), Some("All tasks"));
    assert!(!page.there(cx, "work-show-tasks") && !page.there(cx, "work-show-hidden"));
    assert_eq!(
        (value(&page, cx, "work-needs-you").as_deref(), pressed_in(&page, cx, "work-needs-you")),
        (Some("4"), Some(false))
    );
    assert_eq!(value(&page, cx, "health-github").as_deref(), Some("ok"));
    assert_eq!(described(&page, cx, "health-github").as_deref(), Some("shop: read 2 min ago as sam-lee"));
    assert_eq!(described(&page, cx, "health-jira").as_deref(), Some("shop: read 2 min ago as Sam Lee"));
    assert!(!page.there(cx, "work-reading"));

    // the filter: keys, titles, branches and names
    page.click(cx, "work-query");
    typed(&page, cx, "stock");
    assert_eq!(shown(&page, cx), ["SH-21580", "SH-21533"]);
    typed(&page, cx, " nothing like this");
    assert_eq!(page.said(cx, "work-no-match").as_deref(), Some("Nothing here matches “stock nothing like this”."));
    assert!(!page.there(cx, "work-board"));
    // Esc empties it, and again leaves it; the page stays
    page.press(cx, "escape");
    assert_eq!(shown(&page, cx).len(), 15);
    assert!(page.open(cx));
    page.press(cx, "escape");
    assert!(page.open(cx), "the second Esc left the field, not the page");
    // / goes back into it, and there the board's keys are letters
    page.press(cx, "/");
    typed(&page, cx, "x");
    assert_eq!(state(&page, cx).1, Vec::<String>::new(), "x was typed, no card was picked");
    let with_x = shown(&page, cx).len();
    assert!(with_x > 0 && with_x < 15, "the board is filtered by the letter: {with_x} cards");
    page.press(cx, "escape");
    page.press(cx, "escape");

    // Needs you: only the cards with something waiting on you, and it is remembered
    page.click(cx, "work-needs-you");
    assert_eq!(shown(&page, cx), ["SH-21309", "SH-21616", "SH-21533", "SH-21112"]);
    assert_eq!(pressed_in(&page, cx, "work-needs-you"), Some(true));
    let kept = page.shell.read_with(cx, |shell, cx| shell.ui().read(cx).work_needs_you);
    assert!(kept, "remembered with the layout");
    page.click(cx, "work-needs-you");
    assert_eq!(shown(&page, cx).len(), 15);

    // while a read runs: both lamps say reading, and the hairline under the row runs
    page.work.update(cx, |work, cx| {
        let mut index = (**work.snapshot.as_ref().unwrap()).clone();
        index.checking = true;
        work.snapshot = Some(std::sync::Arc::new(index));
        cx.notify();
    });
    assert_eq!(value(&page, cx, "health-github").as_deref(), Some("reading"));
    assert_eq!(value(&page, cx, "health-jira").as_deref(), Some("reading"));
    assert_eq!(page.said(cx, "work-reading").as_deref(), Some("Reading GitHub and Jira"));
    let (line, row) = page.look(cx, |w| (w.find("work-reading").bounds(), w.find("work-header").bounds()));
    assert_eq!((line.left(), line.right(), line.size.height), (row.left(), row.right(), px(1.)));
}

/// Item 5.5: "picks cards and hides them; Hidden shows only those, where they are picked and unhidden", "picks a run of
/// cards with ⇧-click, down each column and on to the next" and "doesn't count a hidden card on the Work tab or in
/// Needs you, and the Hidden button shows what waits there".
#[gpui_kit::test]
fn picks_cards_and_hides_them_hidden_shows_only_those_where_they_are_picked_and_unhidden(cx: &mut TestAppContext) {
    let page = setup(cx, sample(), Vec::new(), true);
    // every column in view
    cx.simulate_window_resize(page.window, size(px(2400.), px(900.)));
    let hidden = |page: &Page, cx: &mut TestAppContext| -> Vec<String> {
        cx.run_until_parked();
        page.work.read_with(cx, |work, _| work.snapshot.as_ref().map(|index| index.hidden.clone()).unwrap_or_default())
    };
    let pick = |page: &Page, cx: &mut TestAppContext, key: &str| {
        click_id(page, cx, SharedString::from(format!("card-pick:ticket:{key}")))
    };
    assert!(!page.there(cx, "pick-bar"));
    pick(&page, cx, "SH-21475");
    assert_eq!(page.said(cx, "pick-bar").as_deref(), Some("1 card picked"));
    pick(&page, cx, "SH-21376");
    assert_eq!(page.said(cx, "pick-bar").as_deref(), Some("2 cards picked"));
    let boxed = |page: &Page, cx: &mut TestAppContext, key: &str| {
        let id = SharedString::from(format!("card-pick:ticket:{key}"));
        page.look(cx, |w| w.find(id).checked())
    };
    assert_eq!((boxed(&page, cx, "SH-21475"), boxed(&page, cx, "SH-21580")), (Some(true), Some(false)));
    // while picking, a click picks (it doesn't open the card), and another lets it go
    click_id(&page, cx, card("SH-21522"));
    assert_eq!(state(&page, cx).2, None, "no card opened");
    assert_eq!(boxed(&page, cx, "SH-21522"), Some(true));
    click_id(&page, cx, card("SH-21522"));
    assert_eq!(state(&page, cx).1, ["ticket:SH-21475", "ticket:SH-21376"]);
    // the bar is a pill centred over the board, 20 px above its foot
    let (bar, board) = page.look(cx, |w| (w.find("pick-bar").bounds(), w.find("work-board").bounds()));
    assert_eq!(bar.bottom(), board.bottom() - px(20.));
    // (its words, Workflow…, Hide and Clear: 392 px)
    assert!((bar.center().x - board.center().x).abs() < px(1.) && bar.size.width < px(420.), "{bar:?}");
    // Hide: they leave the board, and the picks are let go
    assert_eq!(page.said(cx, "pick-hide").as_deref(), Some("Hide"));
    assert!(!page.there(cx, "pick-show"));
    page.click(cx, "pick-hide");
    assert_eq!(hidden(&page, cx), ["ticket:SH-21475", "ticket:SH-21376"]);
    assert!(!page.there(cx, "pick-bar"), "the picks are let go");
    assert!(card_says(&page, cx, "SH-21475").is_none());
    assert!(!columns(&page, cx).iter().any(|(name, _)| name == "On Hold"), "On Hold had only that card");
    // Hidden, in the header: only the hidden tasks, in their statuses' columns
    assert_eq!(value(&page, cx, "work-show-hidden").as_deref(), Some("2"));
    assert_eq!(
        described(&page, cx, "work-show-hidden").as_deref(),
        Some("The 2 tasks you hid: pick them there to unhide them")
    );
    assert_eq!(
        (pressed_in(&page, cx, "work-show-tasks"), pressed_in(&page, cx, "work-show-hidden")),
        (Some(true), Some(false))
    );
    page.click(cx, "work-show-hidden");
    assert_eq!(shown(&page, cx), ["SH-21475", "SH-21376"]);
    let names: Vec<String> = columns(&page, cx).into_iter().map(|(name, _)| name).collect();
    assert_eq!(names, ["On Hold", "Code Review"]);
    assert!(!page.there(cx, "work-needs-you"), "Needs you belongs to the board");
    pick(&page, cx, "SH-21475");
    assert!(!page.there(cx, "pick-hide"));
    assert_eq!(page.said(cx, "pick-show").as_deref(), Some("Unhide"));
    page.click(cx, "pick-show");
    assert_eq!(hidden(&page, cx), ["ticket:SH-21376"]);
    assert_eq!(shown(&page, cx), ["SH-21376"], "it left the hidden tasks");
    // Tasks: the board again, with it back on it
    page.click(cx, "work-show-tasks");
    assert!(card_says(&page, cx, "SH-21475").is_some() && card_says(&page, cx, "SH-21376").is_none());

    // a run with ⇧: from the last card picked, down each column and on to the next; it never opens a card
    pick(&page, cx, "SH-20781");
    click_with(&page, cx, card("SH-21522"), Modifiers::shift());
    assert_eq!(state(&page, cx).1, ["ticket:SH-20781", "ticket:SH-20947", "ticket:SH-21522"]);
    assert_eq!(page.said(cx, "pick-bar").as_deref(), Some("3 cards picked"));
    assert_eq!(state(&page, cx).2, None, "a ⇧-click never opens the card");
    // Clear lets go of them; ⌘ with a click picks one without opening it; Esc lets go, and the page stays
    page.click(cx, "pick-clear");
    assert!(!page.there(cx, "pick-bar"));
    click_with(&page, cx, card("SH-21580"), Modifiers::command());
    assert_eq!(state(&page, cx), (None, vec!["ticket:SH-21580".to_string()], None));
    page.press(cx, "escape");
    assert!(state(&page, cx).1.is_empty() && page.open(cx));

    // a hidden card that needs you: it counts neither in Needs you nor on the tab, and Hidden says what waits there
    page.work.update(cx, |work, cx| {
        let mut index = (**work.snapshot.as_ref().unwrap()).clone();
        index.hidden = vec!["ticket:SH-21616".into()];
        work.snapshot = Some(std::sync::Arc::new(index));
        work.regroup();
        cx.notify();
    });
    assert_eq!(value(&page, cx, "work-needs-you").as_deref(), Some("3"), "4 cards need you; one is hidden");
    assert!(page.there(cx, "hidden-waiting"));
    assert_eq!(
        described(&page, cx, "work-show-hidden").as_deref(),
        Some("The task you hid: pick them there to unhide them (1 waiting on you)")
    );
    page.press(cx, "cmd-i");
    assert_eq!(page.said(cx, "work-tab-count").as_deref(), Some("8"), "9 wait on you, 1 of them on the hidden card");
}

/// Item 5.6: "moves across the board with the arrows, opens with Enter, and in front of a card marks done and snoozes",
/// "leaves the hidden tasks with Esc" and "picks with X and lets go with Esc (the page stays)".
#[gpui_kit::test]
fn moves_across_the_board_with_the_arrows_opens_with_enter_picks_with_x_and_esc_lets_go(cx: &mut TestAppContext) {
    let mut index = sample();
    index.hidden = vec!["ticket:SH-21522".into()];
    let page = setup(cx, index, Vec::new(), true);
    let on = |page: &Page, cx: &mut TestAppContext| state(page, cx).0.unwrap_or_default();
    // the status line says the keys that work on the board
    assert_eq!(page.said(cx, "work-keys").as_deref(), Some("←→↑↓ move, ↩ open, X pick, / filter, ⌘I back"));
    page.press(cx, "down");
    assert_eq!(on(&page, cx), "ticket:SH-21475", "the first card of the first column (On Hold)");
    assert_eq!(page.look(cx, |w| w.find(card("SH-21475")).selected()), Some(true));
    for key in ["SH-21580", "SH-21205", "SH-20728", "SH-21309"] {
        page.press(cx, "right");
        assert_eq!(on(&page, cx), format!("ticket:{key}"));
    }
    // In Progress was past the window's edge: the board moved to show the card the keyboard is on
    let (at, window) = page.look(cx, |w| (w.find(card("SH-21309")).bounds(), w.viewport_size()));
    assert!(at.left() >= px(0.) && at.right() <= window.width, "{at:?}");
    assert_eq!(page.look(cx, |w| w.find(card("SH-21309")).selected()), Some(true));
    // down and up within the column, and h j k l do the same as the arrows
    page.press(cx, "down");
    assert_eq!(on(&page, cx), "ticket:SH-21616");
    page.press(cx, "up");
    page.press(cx, "up");
    assert_eq!(on(&page, cx), "ticket:SH-21309", "the top stays");
    page.press(cx, "j");
    assert_eq!(on(&page, cx), "ticket:SH-21616");
    page.press(cx, "k");
    page.press(cx, "h");
    assert_eq!(on(&page, cx), "ticket:SH-20728");
    page.press(cx, "l");
    assert_eq!(on(&page, cx), "ticket:SH-21309");
    // ↩ opens it in front of the board; the status line says the keys that work there; Esc closes it, the page stays
    page.press(cx, "enter");
    assert_eq!(state(&page, cx).2.as_deref(), Some("ticket:SH-21309"));
    assert_eq!(
        page.said(cx, "work-keys").as_deref(),
        Some("JK move, S tell Claude, X leave out, E done, Z snooze, ⌘↩ send, Esc close")
    );
    page.press(cx, "escape");
    assert_eq!(state(&page, cx).2, None);
    assert!(page.open(cx));
    // X picks the card the keyboard is on, and lets it go; Esc lets go of the picks, and the page stays
    page.press(cx, "x");
    assert_eq!(page.said(cx, "pick-bar").as_deref(), Some("1 card picked"));
    page.press(cx, "x");
    assert!(!page.there(cx, "pick-bar"));
    page.press(cx, "x");
    page.press(cx, "escape");
    assert!(!page.there(cx, "pick-bar") && page.open(cx));
    // in the filter ↓ leaves the field for the board, on its first card
    page.press(cx, "/");
    typed(&page, cx, "stock");
    page.press(cx, "down");
    assert_eq!(on(&page, cx), "ticket:SH-21580");
    page.press(cx, "x");
    assert_eq!(state(&page, cx).1, ["ticket:SH-21580"], "x picks again: the keyboard left the field");
    page.press(cx, "escape");
    // the filter still holds its word: back in the field, Esc empties it and Esc leaves it
    assert_eq!(shown(&page, cx), ["SH-21580", "SH-21533"]);
    page.press(cx, "/");
    page.press(cx, "escape");
    page.press(cx, "escape");
    assert_eq!(shown(&page, cx).len(), 14);
    // Hidden, then Esc: the board again, the page stays; one more Esc goes back to the workspace
    page.click(cx, "work-show-hidden");
    assert_eq!(shown(&page, cx), ["SH-21522"]);
    page.press(cx, "escape");
    assert!(shown(&page, cx).contains(&"SH-21580".to_string()) && page.open(cx), "Esc: the board");
    page.press(cx, "escape");
    assert!(!page.open(cx), "with nothing else to let go of, Esc goes back");
}

/// Item 5.6: the keyboard moving down a long column brings each card into view.
#[gpui_kit::test]
fn the_keyboard_moving_down_a_long_column_brings_its_cards_into_view(cx: &mut TestAppContext) {
    let mut index = sample();
    let supplier = index.tickets.iter().find(|t| t.key == "SH-20728").expect("the sample's SH-20728").clone();
    for n in 0..40 {
        let key = format!("SH-{}", 20000 + n);
        index.ticket_projects.insert(key.clone(), "p1".into());
        index.tickets.push(enkidu_core::work::model::Ticket {
            summary: format!("Backlog task {n}"),
            key,
            ..supplier.clone()
        });
    }
    let page = setup(cx, index, Vec::new(), true);
    page.press(cx, "down");
    for _ in 0..3 {
        page.press(cx, "right");
    }
    for _ in 0..30 {
        page.press(cx, "down");
    }
    let selected = state(&page, cx).0.expect("a card has the keyboard");
    let id = SharedString::from(format!("work-card:{selected}"));
    let (at, column) = page.look(cx, |w| (w.find(id).bounds(), w.find(("work-column", 3usize)).bounds()));
    assert!(at.top() >= column.top() + px(40.) && at.bottom() <= column.bottom(), "{at:?} in {column:?}");
    // and back up to the first
    for _ in 0..40 {
        page.press(cx, "up");
    }
    let first = state(&page, cx).0.expect("a card has the keyboard");
    let id = SharedString::from(format!("work-card:{first}"));
    let (at, column) = page.look(cx, |w| (w.find(id).bounds(), w.find(("work-column", 3usize)).bounds()));
    assert_eq!(at.top(), column.top() + px(41.), "the column's first card, at its top");
}

/// What an element says (its label), by any id.
fn says(page: &Page, cx: &mut TestAppContext, id: impl Into<gpui_kit::ElementId>) -> Option<String> {
    let id = id.into();
    page.look(cx, |w| w.try_find(id).and_then(|e| e.label().map(String::from)))
}

/// What an element says more (its description), by any id.
fn tells(page: &Page, cx: &mut TestAppContext, id: impl Into<gpui_kit::ElementId>) -> Option<String> {
    let id = id.into();
    page.look(cx, |w| w.try_find(id).and_then(|e| e.description().map(String::from)))
}

/// Opens a card in front of the board, as a click on it does.
fn open_card(page: &Page, cx: &mut TestAppContext, id: &str) {
    cx.run_until_parked();
    let view = page.shell.read_with(cx, |shell, _| shell.work_page().expect("the Work page").clone());
    cx.update_window(page.window, |_, window, cx| view.update(cx, |view, cx| view.open(id, window, cx))).unwrap();
    cx.run_until_parked();
}

/// A click at a place of the window.
fn click_at(page: &Page, cx: &mut TestAppContext, x: f32, y: f32) {
    cx.run_until_parked();
    cx.update_window(page.window, |_, w, cx| {
        w.render_frame(cx);
        let (position, modifiers, left) = (point(px(x), px(y)), Modifiers::default(), MouseButton::Left);
        w.dispatch_event(MouseMoveEvent { position, pressed_button: None, modifiers }.to_platform_input(), cx);
        let down = MouseDownEvent { button: left, position, modifiers, click_count: 1, first_mouse: false };
        w.dispatch_event(down.to_platform_input(), cx);
        w.dispatch_event(MouseUpEvent { button: left, position, modifiers, click_count: 1 }.to_platform_input(), cx);
        w.render_frame(cx);
    })
    .unwrap();
}

/// Item 5.7: opens a card in front of the board: the task, what needs you, Jira's status, the PR and how it stands (its
/// header, its tabs and its right column; what the Overview tells is in the tests of the items after it).
#[gpui_kit::test]
fn opens_a_card_in_front_of_the_board_its_header_its_tabs_jiras_status_the_pr_and_how_it_stands(
    cx: &mut TestAppContext,
) {
    let page = setup(cx, sample(), Vec::new(), true);
    // In Progress is past the window's edge: the keyboard brings its card into view, then a click opens it
    for key in ["down", "right", "right", "right", "right", "right"] {
        page.press(cx, key);
    }
    click_id(&page, cx, card("SH-21309"));
    assert_eq!(state(&page, cx).2.as_deref(), Some("ticket:SH-21309"));
    assert_eq!(page.said(cx, "task-dialog").as_deref(), Some("Export orders to the accounting CSV"));
    assert_eq!(described(&page, cx, "task-dialog").as_deref(), Some("ticket:SH-21309, wide"));
    // 1,040 px wide with its hairline edge, in the middle of a window of 1,360, a twentieth of its height down, at
    // most nine tenths tall
    let (dialog, window) = page.look(cx, |w| (w.find("task-dialog").bounds(), w.viewport_size()));
    assert_eq!((dialog.size.width, dialog.left()), (px(1038.), px(161.)), "{dialog:?}");
    assert!((dialog.top() - px(43.)).abs() <= px(1.5), "{dialog:?}");
    assert!(dialog.bottom() <= window.height - px(42.), "{dialog:?}");
    // its header: what it is, then the title
    assert_eq!(page.said(cx, "task-key").as_deref(), Some("SH-21309"));
    assert_eq!(page.said(cx, "task-title").as_deref(), Some("Export orders to the accounting CSV"));
    assert!(!page.there(cx, "task-hidden"));
    assert_eq!(page.said(cx, "task-hide").as_deref(), Some("Hide"));
    // its tabs: Overview first and on screen, a tab for its open PR, Activity
    let tabs = ["task-tab:overview", "task-tab:pr:2414", "task-tab:activity"];
    let named: Vec<Option<String>> = tabs.iter().map(|id| page.said(cx, id)).collect();
    assert_eq!(named, [Some("Overview".into()), Some("PR #2414".into()), Some("Activity".into())]);
    let on = |page: &Page, cx: &mut TestAppContext| -> Vec<bool> {
        tabs.iter().map(|id| page.look(cx, |w| w.find(*id).selected()) == Some(true)).collect()
    };
    assert_eq!(on(&page, cx), [true, false, false]);
    // on the right, always in this order: Jira, then the pull request
    assert_eq!(
        described(&page, cx, "task-jira").as_deref(),
        Some("Status In Progress, Type Story, Priority High, Assignee Sam Lee, Updated 12 min ago")
    );
    assert_eq!(page.said(cx, "task-status").as_deref(), Some("In Progress"));
    assert_eq!(says(&page, cx, "task-pr-state:2414").as_deref(), Some("Open"));
    assert_eq!(says(&page, cx, "task-checks:2414").as_deref(), Some("Failing: tests (3.12)"));
    assert_eq!(says(&page, cx, "task-review:2414").as_deref(), Some("Waiting for review"));
    assert_eq!(says(&page, cx, "task-threads:2414").as_deref(), Some("2 open threads"));
    assert_eq!(
        tells(&page, cx, "task-pr:2414").as_deref(),
        Some(
            "#2414 Open | SH-21309 Export orders to the accounting CSV | feature/SH-21309-export into development | \
             Checks: Failing: tests (3.12) | Review: Waiting for review | 2 open threads"
        )
    );
    assert!(!page.there(cx, "task-no-pr"));
    let (facts, jira, pr) = page
        .look(cx, |w| (w.find("task-facts").bounds(), w.find("task-jira").bounds(), w.find("task-pr:2414").bounds()));
    assert_eq!((facts.size.width, facts.right()), (px(316.), dialog.right()), "{facts:?}");
    assert!(jira.bottom() <= pr.top() && jira.left() == pr.left() && jira.size.width == px(283.), "{jira:?} {pr:?}");
    // a link opens in the browser, never in the app
    assert_eq!(cx.opened_url(), None);
    page.click(cx, "detail-open");
    assert_eq!(cx.opened_url().as_deref(), Some("https://demo.atlassian.net/browse/SH-21309"));
    click_id(&page, cx, "task-pr-open:2414".into());
    assert_eq!(cx.opened_url().as_deref(), Some("https://github.com/demo-org/shop/pull/2414"));
    assert_eq!(state(&page, cx).2.as_deref(), Some("ticket:SH-21309"), "the card stays");
    // Activity: what happened to the ticket and its PR, the newest first
    page.click(cx, "task-tab:activity");
    assert_eq!(on(&page, cx), [false, false, true]);
    let rows: Vec<Option<String>> = (0..4usize).map(|n| says(&page, cx, ("timeline-row", n))).collect();
    let line = |words: &str| Some(words.to_string());
    assert_eq!(
        rows,
        [
            line("14 min ago: Question from jonas-qa"),
            line("2 h ago: Checks failed: tests (3.12)"),
            line("2 h ago: Pushed 4973abc"),
            None
        ]
    );
    assert!(!page.there(cx, "timeline-more"));
    // Esc closes it; the page stays, the keyboard back on the board (X picks the card it is on)
    page.press(cx, "escape");
    assert_eq!(state(&page, cx).2, None);
    assert!(!page.there(cx, "task-dialog") && page.open(cx));
    page.press(cx, "x");
    assert_eq!(state(&page, cx).1, ["ticket:SH-21309"]);
    page.press(cx, "x");
    // opened again, from the keyboard: on its Overview; its × closes it
    page.press(cx, "enter");
    assert_eq!(on(&page, cx), [true, false, false]);
    page.click(cx, "task-close");
    assert_eq!(state(&page, cx).2, None);
    assert!(!page.there(cx, "task-dialog"));
    // and so does a click beside it
    page.press(cx, "enter");
    assert!(page.there(cx, "task-dialog"));
    click_at(&page, cx, 60., 500.);
    assert_eq!(state(&page, cx).2, None);
    assert!(!page.there(cx, "task-dialog") && page.open(cx));
}

/// Item 5.7: says a ticket has no PR yet, and gives a PR without a ticket its GitHub link once.
#[gpui_kit::test]
fn says_a_ticket_has_no_pr_yet_and_gives_a_pr_without_a_ticket_its_github_link_once(cx: &mut TestAppContext) {
    let page = setup(cx, sample(), Vec::new(), true);
    open_card(&page, cx, "ticket:SH-20728");
    assert_eq!(page.said(cx, "task-no-pr").as_deref(), Some("No pull request yet."));
    // nothing to tell on the left: the facts alone, in a narrow card, with no tabs
    assert_eq!(described(&page, cx, "task-dialog").as_deref(), Some("ticket:SH-20728, narrow"));
    let (dialog, facts) = page.look(cx, |w| (w.find("task-dialog").bounds(), w.find("task-facts").bounds()));
    assert_eq!((dialog.size.width, dialog.left()), (px(558.), px(401.)), "{dialog:?}");
    assert_eq!((facts.size.width, facts.left()), (dialog.size.width, dialog.left()), "{facts:?}");
    assert!(!page.there(cx, "task-story") && !page.there(cx, "task-tab:overview"));
    assert_eq!(
        described(&page, cx, "task-jira").as_deref(),
        Some("Status Ready for Dev, Type Story, Assignee Sam Lee, Updated 4 days ago")
    );
    // another card opened while one is open takes its place: a PR that names no ticket
    open_card(&page, cx, "pr:demo-org/shop#2415");
    assert_eq!(state(&page, cx).2.as_deref(), Some("pr:demo-org/shop#2415"));
    assert_eq!(page.look(cx, |w| w.find_all("task-dialog").len()), 1);
    assert_eq!(page.said(cx, "task-key").as_deref(), Some("shop#2415"));
    assert!(!page.there(cx, "detail-open"), "no ticket, no link to Jira");
    assert!(!page.there(cx, "task-jira"));
    // its GitHub link once, in its box
    assert_eq!(says(&page, cx, "task-pr-open:2415").as_deref(), Some("GitHub"));
    assert_eq!(tells(&page, cx, "task-pr-open:2415").as_deref(), Some("https://github.com/demo-org/shop/pull/2415"));
    assert_eq!(page.look(cx, |w| w.find_all("task-pr-open:2415").len()), 1);
    // the read knows what is going on in it: it has a tab, so the card is wide again
    assert_eq!(described(&page, cx, "task-dialog").as_deref(), Some("pr:demo-org/shop#2415, wide"));
    assert_eq!(page.look(cx, |w| w.find("task-dialog").bounds().size.width), px(1038.));
    // a merged PR says when, and nothing of checks or review
    open_card(&page, cx, "ticket:SH-21030");
    assert_eq!(says(&page, cx, "task-pr-state:2312").as_deref(), Some("Merged"));
    assert!(says(&page, cx, "task-merged:2312").is_some());
    assert!(says(&page, cx, "task-checks:2312").is_none() && says(&page, cx, "task-review:2312").is_none());
    // a card with two PRs has a box for each, in the card's order
    open_card(&page, cx, "ticket:SH-21533");
    let (first, second) = page.look(cx, |w| (w.find("task-pr:2413").bounds(), w.find("task-pr:2395").bounds()));
    assert!(first.bottom() <= second.top(), "{first:?} {second:?}");
    assert_eq!(says(&page, cx, "task-review:2395").as_deref(), Some("Changes requested"));
}

/// Item 5.7: the card's Hide takes it off the board and closes it; from Hidden, the card says so and Unhide puts it
/// back; a card that left the index closes; a long timeline shows its first eight lines, then all.
#[gpui_kit::test]
fn a_card_hides_and_unhides_its_task_closes_when_it_left_and_folds_a_long_timeline(cx: &mut TestAppContext) {
    let mut index = sample();
    // ten things happened to SH-21522, the newest first
    for n in 0..10 {
        index.events.insert(
            n,
            serde_json::from_value(serde_json::json!({
                "at": NOW - (n as i64 + 1) * 60_000, "kind": "status", "subject": "SH-21522", "text": format!("Step {n}"),
            }))
            .expect("an event"),
        );
    }
    let page = setup(cx, index, Vec::new(), true);
    open_card(&page, cx, "ticket:SH-21522");
    // only Activity has something to tell: the card is wide, with no row of tabs for one tab
    assert_eq!(described(&page, cx, "task-dialog").as_deref(), Some("ticket:SH-21522, wide"));
    assert!(!page.there(cx, "task-tab:activity"));
    assert_eq!(page.said(cx, "task-story").as_deref(), Some("Activity"));
    let rows = |page: &Page, cx: &mut TestAppContext| -> usize {
        (0..12usize).filter(|n| says(page, cx, ("timeline-row", *n)).is_some()).count()
    };
    assert_eq!(rows(&page, cx), 8);
    assert_eq!(says(&page, cx, ("timeline-row", 0usize)).as_deref(), Some("1 min ago: Step 0"));
    assert_eq!(page.said(cx, "timeline-more").as_deref(), Some("Show all 10"));
    page.click(cx, "timeline-more");
    assert_eq!(rows(&page, cx), 10);
    assert!(!page.there(cx, "timeline-more"));
    // Hide: it leaves the board, and the card closes
    page.click(cx, "task-hide");
    assert_eq!(state(&page, cx).2, None);
    assert!(!page.there(cx, "task-dialog"));
    let hidden = |page: &Page, cx: &mut TestAppContext| -> Vec<String> {
        cx.run_until_parked();
        page.work.read_with(cx, |work, _| work.snapshot.as_ref().map(|index| index.hidden.clone()).unwrap_or_default())
    };
    assert_eq!(hidden(&page, cx), ["ticket:SH-21522"]);
    assert!(!shown(&page, cx).contains(&"SH-21522".to_string()));
    // in Hidden its card says so, and offers Unhide: back on the board, the card closed, Hidden left
    page.click(cx, "work-show-hidden");
    open_card(&page, cx, "ticket:SH-21522");
    assert_eq!(page.said(cx, "task-hidden").as_deref(), Some("Hidden"));
    assert_eq!(page.said(cx, "task-hide").as_deref(), Some("Unhide"));
    page.click(cx, "task-hide");
    assert!(hidden(&page, cx).is_empty() && !page.there(cx, "task-dialog"));
    assert!(shown(&page, cx).contains(&"SH-21522".to_string()) && shown(&page, cx).len() == 15);
    // nothing is hidden now: Tasks and Hidden are gone from the header, and the page stays
    assert!(!page.there(cx, "work-show-hidden") && !page.there(cx, "work-show-tasks") && page.open(cx));
    // a card whose ticket is no longer yours closes by itself
    open_card(&page, cx, "ticket:SH-21522");
    assert!(page.there(cx, "task-dialog"));
    page.work.update(cx, |work, cx| {
        let mut index = (**work.snapshot.as_ref().expect("the index")).clone();
        index.tickets.retain(|ticket| ticket.key != "SH-21522");
        work.snapshot = Some(std::sync::Arc::new(index));
        work.regroup();
        cx.notify();
    });
    assert_eq!(state(&page, cx).2, None);
    assert!(!page.there(cx, "task-dialog") && page.open(cx));
}

/// The sample with one ticket's description replaced by the document with every kind of block.
fn with_rich_description(mut index: Snapshot, key: &str) -> Snapshot {
    let doc: serde_json::Value = serde_json::from_str(enkidu_testkit::WORK_RICH_DOC).expect("the description");
    for ticket in index.tickets.iter_mut().filter(|ticket| ticket.key == key) {
        ticket.description = Some(doc.clone());
    }
    index
}

/// A press and a release at a place of the window with a frame between them, as a hand does it (a text that hears
/// clicks on its links learns of the press when it is drawn).
fn click_slowly_at(page: &Page, cx: &mut TestAppContext, x: f32, y: f32) {
    cx.run_until_parked();
    cx.update_window(page.window, |_, w, cx| {
        w.render_frame(cx);
        let (position, modifiers, left) = (point(px(x), px(y)), Modifiers::default(), MouseButton::Left);
        w.dispatch_event(MouseMoveEvent { position, pressed_button: None, modifiers }.to_platform_input(), cx);
        w.render_frame(cx);
        let down = MouseDownEvent { button: left, position, modifiers, click_count: 1, first_mouse: false };
        w.dispatch_event(down.to_platform_input(), cx);
        w.render_frame(cx);
        w.dispatch_event(MouseUpEvent { button: left, position, modifiers, click_count: 1 }.to_platform_input(), cx);
        w.render_frame(cx);
    })
    .unwrap();
}

/// Item 5.8: Jira's description in the card, folded when it is long, its links opened in the browser.
#[gpui_kit::test]
fn a_cards_overview_draws_jiras_description_folds_a_long_one_and_opens_its_links_in_the_browser(
    cx: &mut TestAppContext,
) {
    // the sample's own descriptions
    let page = setup(cx, sample(), Vec::new(), true);
    open_card(&page, cx, "ticket:SH-21309");
    assert_eq!(page.said(cx, "task-description").as_deref(), Some("The task"));
    let said = page.said(cx, "jira-doc").unwrap_or_default();
    assert!(said.starts_with("Accounting imports the orders of a month as a CSV."), "{said}");
    assert!(said.contains("\nWhat to build\n"), "{said}");
    // a short one is shown whole, with nothing to press
    open_card(&page, cx, "ticket:SH-21112");
    assert_eq!(
        page.said(cx, "jira-doc").as_deref(),
        Some("Show the cost of one article next to the bundle's total cost, using the article count on the bundle.")
    );
    assert_eq!(described(&page, cx, "task-description").as_deref(), Some("All of it"));
    assert!(!page.there(cx, "task-description-more"));
    // a ticket without a description has no such part
    open_card(&page, cx, "ticket:SH-21376");
    assert!(!page.there(cx, "task-description") && page.there(cx, "task-dialog"));

    // a long one, on a card with things that need you: its first 168 px, then a button for the rest
    let page =
        setup(cx, with_rich_description(with_rich_description(sample(), "SH-21309"), "SH-21475"), Vec::new(), true);
    open_card(&page, cx, "ticket:SH-21309");
    let height =
        |page: &Page, cx: &mut TestAppContext| page.look(cx, |w| w.find("task-description-text").bounds().size.height);
    assert_eq!(described(&page, cx, "task-description").as_deref(), Some("Its start"));
    assert_eq!(height(&page, cx), px(168.));
    assert_eq!(page.said(cx, "task-description-more").as_deref(), Some("Show the whole description"));
    // all of it is said, whatever is in view
    let said = page.said(cx, "jira-doc").unwrap_or_default();
    for line in [
        "the spec says what accounting needs. Ask @Mira about the columns; the import is IN REVIEW.",
        "What to build",
        "An Export to CSV action on the order list.",
        "a comma in France",
        "Filter the list",
        "Columns as asked",
        "def export(orders):\n    return csv(orders)",
        "Accounting imports these with a comma in France.",
        "Don't change the order of the columns: the import depends on it.",
        "a number, with the outlet's separator",
        "An image or file, in Jira",
        "More about the format",
        "UTF-8, one header row.",
        "Words can be slanted, struck or underlined; see https://example.com/import.",
    ] {
        assert!(said.contains(line), "{line:?} is missing from {said}");
    }
    // a link opens in the browser, never in the app: the first words of the first line are one
    let line = page.look(cx, |w| w.find("jira-doc:links:0").bounds());
    assert_eq!(tells(&page, cx, "jira-doc:links:0").as_deref(), Some("https://example.com/spec"));
    assert_eq!(cx.opened_url(), None);
    click_slowly_at(&page, cx, f32::from(line.left()) + 4., f32::from(line.top()) + 8.);
    assert_eq!(cx.opened_url().as_deref(), Some("https://example.com/spec"));
    assert!(page.there(cx, "task-dialog"), "the card stays");
    // the whole description: the card is never taller than nine tenths of the window, its header stays, and the
    // left side scrolls by itself
    page.click(cx, "task-description-more");
    assert_eq!(page.said(cx, "task-description-more").as_deref(), Some("Show less"));
    assert_eq!(described(&page, cx, "task-description").as_deref(), Some("All of it"));
    let whole = height(&page, cx);
    assert!(whole > px(488.), "{whole:?}");
    let (dialog, story, title, window) = page.look(cx, |w| {
        (
            w.find("task-dialog").bounds(),
            w.find("task-story").bounds(),
            w.find("task-title").bounds(),
            w.viewport_size(),
        )
    });
    assert!(dialog.bottom() <= window.height - px(42.) && dialog.size.height <= window.height * 0.9, "{dialog:?}");
    assert!(story.bottom() <= dialog.bottom() + px(0.5) && story.size.height < whole, "{story:?}");
    cx.update_window(page.window, |_, w, cx| {
        w.render_frame(cx);
        w.scroll("task-story", ScrollDelta::Pixels(point(px(0.), px(-300.))), cx);
    })
    .unwrap();
    let (moved, stays) =
        page.look(cx, |w| (w.find("task-description").bounds().top(), w.find("task-title").bounds().top()));
    assert!(moved < story.top() - px(100.), "{moved:?} {story:?}");
    assert_eq!(stays, title.top(), "the header doesn't scroll");
    // something folded behind its title opens where it is
    assert_eq!(page.look(cx, |w| w.find("jira-doc:fold:1").expanded()), Some(false));
    click_id(&page, cx, "jira-doc:fold:1".into());
    assert_eq!(page.look(cx, |w| w.find("jira-doc:fold:1").expanded()), Some(true));
    assert!(height(&page, cx) > whole);
    // Show less, at the description's end (what needs you is under it), folds it again
    cx.update_window(page.window, |_, w, cx| {
        w.render_frame(cx);
        let (button, story) = (w.find("task-description-more").bounds(), w.find("task-story").bounds());
        let by = story.center().y - button.center().y;
        w.scroll("task-story", ScrollDelta::Pixels(point(px(0.), by)), cx);
    })
    .unwrap();
    page.click(cx, "task-description-more");
    assert_eq!(height(&page, cx), px(168.));
    // with nothing that needs you under it, a long description shows more of its start: 440 px
    open_card(&page, cx, "ticket:SH-21475");
    assert_eq!(height(&page, cx), px(440.));
    assert_eq!(page.said(cx, "task-description-more").as_deref(), Some("Show the whole description"));
    // a card opened again starts folded
    page.click(cx, "task-description-more");
    open_card(&page, cx, "ticket:SH-21309");
    open_card(&page, cx, "ticket:SH-21475");
    assert_eq!(height(&page, cx), px(440.));
}

/// Items 5.9 and 5.11: shows what is going on in each open PR in its own tab.
#[gpui_kit::test]
fn shows_what_is_going_on_in_each_open_pr_in_its_own_tab(cx: &mut TestAppContext) {
    let page = setup(cx, sample(), Vec::new(), true);
    // a tall window, so the whole tab is in view
    cx.simulate_window_resize(page.window, size(px(1360.), px(2400.)));
    open_card(&page, cx, "ticket:SH-21309");
    page.click(cx, "task-tab:pr:2414");
    assert_eq!(page.said(cx, "task-story").as_deref(), Some("PR #2414"));
    // who opened it and when, how big it is, its commits and labels
    assert_eq!(page.said(cx, "pr-details").as_deref(), Some("PR #2414"));
    assert_eq!(
        described(&page, cx, "pr-details").as_deref(),
        Some("sam-lee opened it 2 days ago, +412 −38 in 9 files, 5 commits, accounting")
    );
    assert_eq!(page.said(cx, "pr-size").as_deref(), Some("+412 −38 in 9 files"));
    // what the PR says: GitHub's Markdown as text, the template's comment dropped
    assert_eq!(page.said(cx, "pr-body").as_deref(), Some("What the PR says"));
    let said = page.said(cx, "pr-markdown").unwrap_or_default();
    for line in [
        "Accounting imports the orders of a month as a CSV; this adds the export to the order list.",
        "What changes",
        "An Export to CSV action on the order list, for the filtered orders",
        "One row per order line, numbers in the outlet's format (1.234,50 in France)",
        "Filter the order list to last month",
        "Columns as accounting asked",
        "10,000 lines in under 5 seconds",
        "Closes SH-21309",
    ] {
        assert!(said.contains(line), "{line:?} is missing from {said}");
    }
    assert!(!said.contains("reminders") && !said.contains("<!--"), "{said}");
    assert_eq!(described(&page, cx, "pr-body").as_deref(), Some("All of it"));
    assert!(!page.there(cx, "pr-body-more"), "a short description has nothing to open");
    // the failing check first; the ones that passed behind a click
    let checks = |page: &Page, cx: &mut TestAppContext| -> Vec<String> {
        (0..8usize).filter_map(|n| says(page, cx, ("pr-check", n))).collect()
    };
    assert_eq!(described(&page, cx, "pr-checks-list").as_deref(), Some("1 failed, 4 passed, 1 skipped"));
    assert_eq!(checks(&page, cx), ["tests (3.12): Failed"]);
    assert_eq!(page.said(cx, "pr-checks-more").as_deref(), Some("Show the 5 that passed or were skipped"));
    page.click(cx, "pr-checks-more");
    assert_eq!(
        checks(&page, cx),
        [
            "tests (3.12): Failed",
            "build: Passed",
            "lint: Passed",
            "SonarCloud: Passed",
            "tests (3.11): Passed",
            "deploy preview: Skipped"
        ]
    );
    assert!(!page.there(cx, "pr-checks-more"));
    // a check's own page opens in the browser; one without a page has no sign for it
    click_id(&page, cx, "pr-check-open:0".into());
    assert_eq!(cx.opened_url().as_deref(), Some("https://ci.example.com/run/88123"));
    assert!(says(&page, cx, "pr-check-open:5").is_none());
    // each reviewer's latest review, and who is asked and hasn't yet
    assert_eq!(says(&page, cx, ("pr-review", 0usize)).as_deref(), Some("mira-k Commented 40 min ago"));
    assert_eq!(says(&page, cx, ("pr-review", 1usize)).as_deref(), Some("coderabbitai Commented 2 days ago"));
    assert_eq!(says(&page, cx, ("pr-requested", 0usize)).as_deref(), Some("tom-b asked to review, hasn't yet"));
    // the open threads, the resolved one behind a click
    let threads = |page: &Page, cx: &mut TestAppContext| -> Vec<String> {
        (0..6usize).filter_map(|n| tells(page, cx, ("pr-thread", n))).collect()
    };
    assert_eq!(described(&page, cx, "pr-threads").as_deref(), Some("2 open, 1 resolved"));
    assert_eq!(
        threads(&page, cx),
        ["Open, orders/export.py:48", "Open, orders/export.py:112, 1 reply, the latest from sam-lee 2 h ago"]
    );
    assert_eq!(
        says(&page, cx, ("pr-thread", 0usize)).as_deref(),
        Some(
            "mira-k: The CSV writer should use the outlet's decimal separator, accounting imports these with a comma \
             in France."
        )
    );
    assert_eq!(page.said(cx, "pr-threads-more").as_deref(), Some("Show the 1 resolved"));
    page.click(cx, "pr-threads-more");
    assert_eq!(threads(&page, cx).len(), 3);
    assert_eq!(threads(&page, cx)[2], "Resolved, orders/views.py:20, 2 replies, the latest from sam-lee yesterday");
    click_id(&page, cx, "pr-thread-open:0".into());
    assert_eq!(cx.opened_url().as_deref(), Some("https://github.com/demo-org/shop/pull/2414#discussion_r48"));
    // the conversation, each comment a click from GitHub
    assert_eq!(described(&page, cx, "pr-conversation").as_deref(), Some("2"));
    assert_eq!(
        says(&page, cx, ("pr-comment", 1usize)).as_deref(),
        Some("jonas-qa: Does the export include orders that were cancelled after delivery?")
    );
    assert!(says(&page, cx, ("pr-comment", 2usize)).is_none());
    click_id(&page, cx, "pr-comment-open:1".into());
    assert_eq!(cx.opened_url().as_deref(), Some("https://github.com/demo-org/shop/pull/2414#issuecomment-2"));
    // the latest commits
    let commits = described(&page, cx, "pr-commits").unwrap_or_default();
    assert!(commits.starts_with("Skip rows with a zero total\nUse the outlet's locale for numbers"), "{commits}");
    // GitHub is a click away
    page.click(cx, "pr-open");
    assert_eq!(cx.opened_url().as_deref(), Some("https://github.com/demo-org/shop/pull/2414"));
    assert!(page.there(cx, "task-dialog"), "the card stays");
    // the parts stand in the old order, top to bottom
    let tops: Vec<f32> =
        ["pr-details", "pr-body", "pr-checks-list", "pr-reviews", "pr-threads", "pr-conversation", "pr-commits"]
            .iter()
            .map(|id| f32::from(page.look(cx, |w| w.find(*id).bounds().top())))
            .collect();
    assert!(tops.windows(2).all(|pair| pair[0] < pair[1]), "{tops:?}");
    // seen again, the tab is as it is first seen: the passed checks and the resolved thread folded away
    page.click(cx, "task-tab:overview");
    assert!(!page.there(cx, "pr-details"));
    page.click(cx, "task-tab:pr:2414");
    assert_eq!(checks(&page, cx).len(), 1);
    assert_eq!(threads(&page, cx).len(), 2);
    // a card with two open PRs has a tab for each
    open_card(&page, cx, "ticket:SH-21533");
    let tabs: Vec<Option<String>> = ["task-tab:overview", "task-tab:pr:2413", "task-tab:pr:2395", "task-tab:activity"]
        .iter()
        .map(|id| page.said(cx, id))
        .collect();
    assert_eq!(
        tabs,
        [Some("Overview".into()), Some("PR #2413".into()), Some("PR #2395".into()), Some("Activity".into())]
    );
    page.click(cx, "task-tab:pr:2395");
    assert_eq!(page.said(cx, "pr-details").as_deref(), Some("PR #2395"));
    page.click(cx, "task-tab:pr:2413");
    assert_eq!(page.said(cx, "pr-details").as_deref(), Some("PR #2413"));
}

/// Item 5.9: a long PR description shows its start, with a button for the rest; nothing in it is ever a picture or
/// a tag.
#[gpui_kit::test]
fn a_long_pr_description_shows_its_start_and_images_are_named_never_fetched(cx: &mut TestAppContext) {
    let mut index = sample();
    let body = [
        "## Screens".to_string(),
        "![the export button](https://example.com/shot.png) <img src=x onerror=alert(1)>".to_string(),
        "<!-- a reminder of the template -->".to_string(),
        (0..40)
            .map(|n| format!("- Step {n} of the long list of steps this description goes through"))
            .collect::<Vec<_>>()
            .join("\n"),
        "See [the plan](https://example.com/plan).".to_string(),
    ]
    .join("\n\n");
    assert!(body.chars().count() > 900);
    for pr in index.prs.iter_mut().filter(|pr| pr.number == 2414) {
        pr.details.body = body.clone();
    }
    let page = setup(cx, index, Vec::new(), true);
    cx.simulate_window_resize(page.window, size(px(1360.), px(2400.)));
    open_card(&page, cx, "ticket:SH-21309");
    page.click(cx, "task-tab:pr:2414");
    let height = |page: &Page, cx: &mut TestAppContext| page.look(cx, |w| w.find("pr-body-text").bounds().size.height);
    assert_eq!(described(&page, cx, "pr-body").as_deref(), Some("Its start"));
    assert_eq!(height(&page, cx), px(220.));
    assert_eq!(page.said(cx, "pr-body-more").as_deref(), Some("Show the whole description"));
    // the image is named, the tag and the template's comment are gone
    let said = page.said(cx, "pr-markdown").unwrap_or_default();
    assert!(said.starts_with("Screens\nthe export button, on GitHub\n"), "{said}");
    assert!(!said.contains("onerror") && !said.contains("reminder"), "{said}");
    assert_eq!(tells(&page, cx, "pr-markdown:links:0").as_deref(), Some("https://example.com/plan"));
    page.click(cx, "pr-body-more");
    assert_eq!(page.said(cx, "pr-body-more").as_deref(), Some("Show less"));
    assert_eq!(described(&page, cx, "pr-body").as_deref(), Some("All of it"));
    assert!(height(&page, cx) > px(600.));
    page.click(cx, "pr-body-more");
    assert_eq!(height(&page, cx), px(220.));
}

/// What is kept of a card's draft, as text.
fn draft(page: &Page, cx: &mut TestAppContext, card: &str) -> String {
    cx.run_until_parked();
    page.shell.read_with(cx, |shell, cx| {
        let drafts = shell.work_page().expect("the Work page").read(cx).drafts().read(cx);
        serde_json::to_string(&drafts.of(card)).expect("a draft as text")
    })
}

/// How an item of the index stands (open, done, snoozed) and until when it is snoozed.
fn item_state(page: &Page, cx: &mut TestAppContext, id: &str) -> (String, Option<i64>) {
    cx.run_until_parked();
    page.work.read_with(cx, |work, _| {
        let index = work.snapshot.as_ref().expect("the index");
        let item = index.items.iter().find(|item| item.id == id).expect("the item");
        (item.state.clone(), item.snoozed_until)
    })
}

/// Item 5.10: "moves across the board with the arrows, opens with Enter, and in front of a card marks done and
/// snoozes", and the item parts of "opens a card in front of the board" and "tells Claude what to do".
#[gpui_kit::test]
fn in_front_of_a_card_the_keys_move_through_what_needs_you_mark_it_done_snooze_it_and_leave_it_out(
    cx: &mut TestAppContext,
) {
    let page = setup(cx, sample(), Vec::new(), true);
    cx.simulate_window_resize(page.window, size(px(1360.), px(2000.)));
    for key in ["down", "right", "right", "right", "right", "right", "enter"] {
        page.press(cx, key);
    }
    assert_eq!(state(&page, cx).2.as_deref(), Some("ticket:SH-21309"));
    // what needs you, in the card's order, each as a sentence a person would say with the words quoted
    let ids = ["question:c9", "thread:t1", "checks:demo-org/shop#2414:4973abc0def1", "thread:t2"];
    let item = |id: &str| SharedString::from(format!("detail-item:{id}"));
    let shown = |page: &Page, cx: &mut TestAppContext| -> Vec<&'static str> {
        ids.into_iter().filter(|id| says(page, cx, item(id)).is_some()).collect()
    };
    let picked = |page: &Page, cx: &mut TestAppContext| -> Vec<&'static str> {
        ids.into_iter()
            .filter(|id| page.look(cx, |w| w.try_find(item(id)).and_then(|e| e.selected())) == Some(true))
            .collect()
    };
    assert_eq!(shown(&page, cx), ids);
    assert_eq!(
        (page.said(cx, "task-needs").as_deref(), described(&page, cx, "task-needs").as_deref()),
        (Some("Needs you"), Some("4"))
    );
    assert_eq!(
        says(&page, cx, item("thread:t1")).as_deref(),
        Some(
            "mira-k left a remark on export.py, line 48: The CSV writer should use the outlet's decimal separator, \
             accounting imports these with a comma in France."
        )
    );
    assert_eq!(tells(&page, cx, item("question:c9")).as_deref(), Some("14 min ago"));
    let tops: Vec<f32> = ids.iter().map(|id| f32::from(page.look(cx, |w| w.find(item(id)).bounds().top()))).collect();
    assert!(tops.windows(2).all(|pair| pair[0] < pair[1]), "{tops:?}");
    // the description stands above them
    let task = page.look(cx, |w| w.find("task-description").bounds());
    assert!(f32::from(task.bottom()) <= tops[0], "{task:?} {tops:?}");
    // the keyboard is on the first one as the card opens
    assert_eq!(picked(&page, cx), ["question:c9"]);
    // e marks it done: it leaves the card at once, and the one that took its place is picked
    page.press(cx, "e");
    assert_eq!(item_state(&page, cx, "question:c9").0, "done");
    assert_eq!(shown(&page, cx), ids[1..]);
    assert_eq!(picked(&page, cx), ["thread:t1"]);
    assert_eq!(described(&page, cx, "task-needs").as_deref(), Some("3"));
    // z snoozes that one for a day
    page.press(cx, "z");
    assert_eq!(item_state(&page, cx, "thread:t1"), ("snoozed".to_string(), Some(NOW + 86_400_000)));
    assert_eq!(shown(&page, cx), ids[2..]);
    assert_eq!(picked(&page, cx), [ids[2]]);
    // j and k move through what is left and stay at the ends; so do the arrows
    page.press(cx, "j");
    assert_eq!(picked(&page, cx), ["thread:t2"]);
    page.press(cx, "j");
    assert_eq!(picked(&page, cx), ["thread:t2"]);
    page.press(cx, "k");
    page.press(cx, "up");
    assert_eq!(picked(&page, cx), [ids[2]]);
    page.press(cx, "down");
    assert_eq!(picked(&page, cx), ["thread:t2"]);
    // x leaves it out of the message to Claude: its box is empty, it has no instruction field, the draft says so
    let (include, note) = ("item-include:thread:t2", "item-note:thread:t2");
    assert_eq!(page.said(cx, include).as_deref(), Some("In the message to Claude"));
    assert_eq!(pressed_in(&page, cx, include), Some(true));
    assert!(page.there(cx, note));
    page.press(cx, "x");
    assert_eq!(page.said(cx, include).as_deref(), Some("Left out of the message to Claude"));
    assert_eq!(pressed_in(&page, cx, include), Some(false));
    assert!(!page.there(cx, note), "no instruction for one left out");
    assert_eq!(draft(&page, cx, "ticket:SH-21309"), r#"{"note":"","items":{"thread:t2":{"include":false}}}"#);
    // x again puts it back, as if it was never touched
    page.press(cx, "x");
    assert_eq!(pressed_in(&page, cx, include), Some(true));
    assert_eq!(draft(&page, cx, "ticket:SH-21309"), r#"{"note":"","items":{}}"#);
    // n writes its instruction: in the field x and e are letters, and what you write is kept as you write
    page.press(cx, "n");
    typed(&page, cx, "exclude zero rows");
    assert_eq!(
        draft(&page, cx, "ticket:SH-21309"),
        r#"{"note":"","items":{"thread:t2":{"note":"exclude zero rows"}}}"#
    );
    assert_eq!(shown(&page, cx), ids[2..], "e in the field marked nothing done");
    // Esc leaves the field, the card stays; the keys are the card's again
    page.press(cx, "escape");
    assert_eq!(state(&page, cx).2.as_deref(), Some("ticket:SH-21309"));
    page.press(cx, "k");
    assert_eq!(picked(&page, cx), [ids[2]]);
    // ↩ opens where it was said, in the browser (the failing check's own page)
    assert_eq!(cx.opened_url(), None);
    page.press(cx, "enter");
    assert_eq!(cx.opened_url().as_deref(), Some("https://ci.example.com/run/88123"));
    assert!(page.there(cx, "task-dialog"));
    // the same by hand: the box, a click on an item, its Open, Snooze and Done
    page.click(cx, "item-include:checks:demo-org/shop#2414:4973abc0def1");
    assert_eq!(pressed_in(&page, cx, "item-include:checks:demo-org/shop#2414:4973abc0def1"), Some(false));
    page.click(cx, "item-include:checks:demo-org/shop#2414:4973abc0def1");
    // (a click on the item itself, above its instruction field: the field's presses are its own)
    let at = page.look(cx, |w| w.find(item("thread:t2")).bounds());
    click_at(&page, cx, f32::from(at.center().x), f32::from(at.top()) + 20.);
    assert_eq!(picked(&page, cx), ["thread:t2"]);
    // after a click the keys are still the card's
    page.press(cx, "k");
    assert_eq!(picked(&page, cx), [ids[2]]);
    page.press(cx, "j");
    assert_eq!(picked(&page, cx), ["thread:t2"]);
    assert!(!page.there(cx, "detail-item-open:thread:t2"), "nowhere to open: no such button");
    page.click(cx, "detail-item-open:checks:demo-org/shop#2414:4973abc0def1");
    assert_eq!(picked(&page, cx), [ids[2]], "its buttons are its own");
    page.click(cx, "detail-snooze:thread:t2");
    assert_eq!(item_state(&page, cx, "thread:t2").0, "snoozed");
    assert_eq!(shown(&page, cx), [ids[2]]);
    page.click(cx, "detail-done:checks:demo-org/shop#2414:4973abc0def1");
    assert_eq!(item_state(&page, cx, ids[2]).0, "done");
    // nothing needs you there any more: no such part, and the card's badge is gone from the board
    assert!(!page.there(cx, "task-needs") && page.there(cx, "task-description"));
    // Esc closes the card, then goes back
    page.press(cx, "escape");
    assert_eq!(state(&page, cx).2, None);
    assert!(page.open(cx) && !page.there(cx, "card-needs:ticket:SH-21309"));
    page.press(cx, "escape");
    assert!(!page.open(cx));
}

/// Item 5.10: an item the keyboard moves to is brought into view; what you wrote about an item is there when the
/// card is opened again.
#[gpui_kit::test]
fn an_item_the_keyboard_moves_to_comes_into_view_and_its_instruction_is_kept(cx: &mut TestAppContext) {
    // a window as small as the app allows: the card shows little of its left side
    let page = setup(cx, sample(), Vec::new(), true);
    cx.simulate_window_resize(page.window, size(px(900.), px(560.)));
    open_card(&page, cx, "ticket:SH-21309");
    let inside = |page: &Page, cx: &mut TestAppContext, id: &str| -> bool {
        let id = SharedString::from(format!("detail-item:{id}"));
        let (item, story) = page.look(cx, |w| (w.find(id).bounds(), w.find("task-story").bounds()));
        item.top() >= story.top() && item.bottom() <= story.bottom()
    };
    // as it opens the card shows what the task is: nothing moved to what needs you
    assert!(!inside(&page, cx, "thread:t2"));
    let top = page.look(cx, |w| w.find("task-description").bounds().top());
    for (key, on) in [("j", "thread:t1"), ("j", "checks:demo-org/shop#2414:4973abc0def1"), ("j", "thread:t2")] {
        page.press(cx, key);
        assert!(inside(&page, cx, on), "{on} is in view after {key}");
    }
    assert!(page.look(cx, |w| w.find("task-description").bounds().top()) < top, "the left side scrolled");
    for on in ["checks:demo-org/shop#2414:4973abc0def1", "thread:t1", "question:c9"] {
        page.press(cx, "k");
        assert!(inside(&page, cx, on), "{on} is in view after k");
    }
    // an instruction, then another card and back: the words are in the field again
    page.press(cx, "n");
    typed(&page, cx, "Say yes: they are left out.");
    page.press(cx, "escape");
    open_card(&page, cx, "ticket:SH-21533");
    assert!(!page.there(cx, "item-note:question:c9"));
    open_card(&page, cx, "ticket:SH-21309");
    assert_eq!(
        draft(&page, cx, "ticket:SH-21309"),
        r#"{"note":"","items":{"question:c9":{"note":"Say yes: they are left out."}}}"#
    );
    page.press(cx, "n");
    typed(&page, cx, " Thanks.");
    assert!(
        draft(&page, cx, "ticket:SH-21309").contains("Say yes: they are left out. Thanks."),
        "it went on after the kept words"
    );
}

/// The sample with the stock-fix Claude having only named SH-21309 in a prompt (the old Claude box tests).
fn named_in_a_prompt() -> Snapshot {
    let mut index = sample();
    let link = serde_json::json!({ "a": "ticket:SH-21309", "b": "session:s-w1", "how": "transcript",
        "detail": "named in a prompt", "at": NOW, "removed": false });
    index.links.push(serde_json::from_value(link).expect("a link"));
    index
}

/// The rows of the card's Claude box that are on screen, each with whether it is picked.
fn claude_rows(page: &Page, cx: &mut TestAppContext) -> Vec<String> {
    ["s-w2", "s-w1", "s-w3", "new", "fresh"]
        .into_iter()
        .filter_map(|id| {
            let row = SharedString::from(format!("claude-option:{id}"));
            let picked = page.look(cx, |w| w.try_find(row).map(|e| e.checked() == Some(true)))?;
            Some(format!("{id} {picked}"))
        })
        .collect()
}

/// Item 5.12: lets you pick the Claude a card goes to: one linked to it, or a new one.
#[gpui_kit::test]
fn lets_you_pick_the_claude_a_card_goes_to_one_linked_to_it_or_a_new_one(cx: &mut TestAppContext) {
    let workspaces =
        vec![ws("w1", "stock-fix", "fix/stock-double-count"), ws("w2", "sh-21309-export", "feature/SH-21309-export")];
    let page = setup(cx, named_in_a_prompt(), workspaces, true);
    cx.simulate_window_resize(page.window, size(px(2400.), px(1600.)));
    // the board's card shows the Claude in the workspace on its branch
    assert_eq!(page.said(cx, "card-claude:ticket:SH-21309").as_deref(), Some("sh-21309-export"));
    open_card(&page, cx, "ticket:SH-21309");
    // each Claude linked to the card, the likeliest first and picked; then a new one, and a new workspace
    assert_eq!(claude_rows(&page, cx), ["s-w2 true", "s-w1 false", "new false", "fresh false"]);
    assert_eq!(page.said(cx, "claude-option:s-w2").as_deref(), Some("sh-21309-export"));
    let named = described(&page, cx, "claude-option:s-w1").unwrap_or_default();
    assert!(named.starts_with("Named in a prompt") && named.ends_with(", working"), "{named}");
    assert_eq!(described(&page, cx, "claude-option:new").as_deref(), Some("A new conversation in sh-21309-export"));
    assert_eq!(described(&page, cx, "claude-option:fresh").as_deref(), Some("A new workspace, sh-21309-export-2"));
    assert!(!page.there(cx, "claude-other"), "both workspaces are on the card: no other to offer");
    // the box is the last of the right column: Jira, the pull request, Claude
    let (jira, pr, claude, rows) = page.look(cx, |w| {
        let at = |id: &'static str| w.find(id).bounds();
        let rows: Vec<f32> = ["claude-option:s-w2", "claude-option:s-w1", "claude-option:new", "claude-option:fresh"]
            .iter()
            .map(|id| f32::from(w.find(*id).bounds().top()))
            .collect();
        (at("task-jira"), at("task-pr:2414"), at("detail-sessions"), rows)
    });
    assert!(jira.bottom() <= pr.top() && pr.bottom() <= claude.top(), "{jira:?} {pr:?} {claude:?}");
    assert_eq!(claude.size.width, jira.size.width);
    assert!(rows.windows(2).all(|pair| pair[0] < pair[1]), "{rows:?}");
    assert_eq!(
        described(&page, cx, "detail-sessions").as_deref(),
        Some("The message goes to the Claude in sh-21309-export")
    );
    // a new one: your pick is kept with the card's draft
    page.click(cx, "claude-option:new");
    assert_eq!(claude_rows(&page, cx), ["s-w2 false", "s-w1 false", "new true", "fresh false"]);
    assert_eq!(draft(&page, cx, "ticket:SH-21309"), r#"{"note":"","items":{},"claude":"new"}"#);
    assert_eq!(described(&page, cx, "detail-sessions").as_deref(), Some("The message goes to a new Claude"));
    // the one only named in a prompt: picked, and the board shows it on the card
    page.click(cx, "claude-option:s-w1");
    assert_eq!(claude_rows(&page, cx), ["s-w2 false", "s-w1 true", "new false", "fresh false"]);
    page.press(cx, "escape");
    assert_eq!(page.said(cx, "card-claude:ticket:SH-21309").as_deref(), Some("stock-fix"));
    // opened again, the card is still with the Claude you picked
    open_card(&page, cx, "ticket:SH-21309");
    assert_eq!(claude_rows(&page, cx), ["s-w2 false", "s-w1 true", "new false", "fresh false"]);
    // a card nobody worked on, in a project with workspaces: no Claude of its own, New Claude picked, any
    // workspace's Claude on offer
    open_card(&page, cx, "ticket:SH-21522");
    assert_eq!(claude_rows(&page, cx), ["new true"]);
    assert!(described(&page, cx, "claude-option:new").unwrap_or_default().starts_with("A new workspace, sh-21522-"));
    assert!(page.there(cx, "claude-other"));
}

/// Item 5.12: sends a card to a new workspace though it has one, to any workspace's Claude, and unlinks a wrong one;
/// and Open, which goes to the Claude's workspace.
#[gpui_kit::test]
fn picks_a_new_workspace_or_any_workspaces_claude_unlinks_a_wrong_one_and_opens_a_claudes_workspace(
    cx: &mut TestAppContext,
) {
    let workspaces = vec![
        ws("w1", "stock-fix", "fix/stock-double-count"),
        ws("w2", "sh-21309-export", "feature/SH-21309-export"),
        ws("w3", "scratch", "scratch"),
    ];
    let page = setup(cx, named_in_a_prompt(), workspaces, true);
    cx.simulate_window_resize(page.window, size(px(2400.), px(1600.)));
    open_card(&page, cx, "ticket:SH-21309");
    assert_eq!(claude_rows(&page, cx), ["s-w2 true", "s-w1 false", "new false", "fresh false"]);
    // a new workspace, though the card has one
    page.click(cx, "claude-option:fresh");
    assert_eq!(claude_rows(&page, cx), ["s-w2 false", "s-w1 false", "new false", "fresh true"]);
    assert_eq!(described(&page, cx, "detail-sessions").as_deref(), Some("The message goes to a new workspace"));
    // any other workspace's Claude: scratch is linked to nothing, but you can send it there
    page.click(cx, "claude-other");
    assert_eq!(page.said(cx, "claude-other-option:s-w3").as_deref(), Some("scratch"));
    assert_eq!(described(&page, cx, "claude-other-option:s-w3").as_deref(), Some("scratch"));
    assert!(!page.there(cx, "claude-other-option:s-w1") && !page.there(cx, "claude-other-option:s-w2"));
    page.click(cx, "claude-other-option:s-w3");
    assert!(claude_rows(&page, cx).contains(&"s-w3 true".to_string()), "{:?}", claude_rows(&page, cx));
    let scratch = described(&page, cx, "claude-option:s-w3").unwrap_or_default();
    assert!(scratch.starts_with("Picked by you"), "{scratch}");
    assert!(!page.there(cx, "claude-other"), "every workspace's Claude is on the card now");
    // both others are linked to the card: a wrong link can be undone; scratch is only your pick
    let unlinks: Vec<&str> = ["s-w2", "s-w1", "s-w3"]
        .into_iter()
        .filter(|id| page.look(cx, |w| w.try_find(SharedString::from(format!("claude-unlink:{id}"))).is_some()))
        .collect();
    assert_eq!(unlinks, ["s-w2", "s-w1"]);
    // stock-fix only named the ticket in a prompt: unlinked, it leaves the card at once
    page.click(cx, "claude-unlink:s-w1");
    assert!(!page.there(cx, "claude-option:s-w1"));
    let removed = page.work.read_with(cx, |work, _| {
        let index = work.snapshot.as_ref().expect("the index");
        let link = index.links.iter().find(|link| link.a == "ticket:SH-21309" && link.b == "session:s-w1");
        link.map(|link| link.removed)
    });
    assert_eq!(removed, Some(true));
    assert!(page.there(cx, "claude-other"), "it is one of the other workspaces again");
    // a Claude you picked and then unlinked: the card is back with the likeliest one
    page.click(cx, "claude-option:s-w2");
    assert_eq!(draft(&page, cx, "ticket:SH-21309"), r#"{"note":"","items":{},"claude":"s-w2"}"#);
    page.click(cx, "claude-unlink:s-w2");
    assert_eq!(draft(&page, cx, "ticket:SH-21309"), r#"{"note":"","items":{}}"#);
    // its workspace is on the card's branch, so it is still there, and picked: only what linked it is gone
    let rows = claude_rows(&page, cx);
    assert!(rows.contains(&"s-w2 true".to_string()), "{rows:?}");
    assert!(!page.there(cx, "claude-unlink:s-w2"), "nothing to undo: its workspace is on the branch");
    assert_eq!(described(&page, cx, "claude-option:s-w2").as_deref(), Some("Its workspace is on this branch, working"));
    // Open: the card closes and its workspace comes on screen, in place of the Work page
    assert_eq!(page.said(cx, "detail-go-workspace:s-w2").as_deref(), Some("Open sh-21309-export"));
    page.click(cx, "detail-go-workspace:s-w2");
    assert_eq!(state(&page, cx).2, None);
    assert!(!page.there(cx, "task-dialog") && !page.open(cx));
    let on_screen =
        page.shell.read_with(cx, |shell, cx| shell.frame().read(cx).current_workspace().map(|ws| ws.id.clone()));
    assert_eq!(on_screen.as_deref(), Some("w2"));
}

/// What the open card would send, and what its whole-message field holds while it is shown.
fn message(page: &Page, cx: &mut TestAppContext) -> (String, Option<String>) {
    cx.run_until_parked();
    page.shell.read_with(cx, |shell, cx| {
        let card = shell.work_page().expect("the Work page").read(cx).card_view().expect("an open card").read(cx);
        (card.message(cx), card.whole_text(cx))
    })
}

/// Item 5.13: tells Claude what to do: your words, an instruction for an item, one left out; it goes into the prompt of
/// the Claude in the workspace.
#[gpui_kit::test]
fn tells_claude_what_to_do_your_words_an_instruction_for_an_item_one_left_out(cx: &mut TestAppContext) {
    let page = setup(cx, sample(), vec![ws("w2", "sh-21309-export", "feature/SH-21309-export")], true);
    cx.simulate_window_resize(page.window, size(px(1360.), px(2000.)));
    open_card(&page, cx, "ticket:SH-21309");
    // where it goes, and what the button does
    assert_eq!(
        page.said(cx, "task-target").as_deref(),
        Some("goes into the prompt of the Claude in sh-21309-export; you press Enter there")
    );
    assert_eq!(described(&page, cx, "task-target").as_deref(), Some("workspace"));
    assert_eq!(page.said(cx, "task-send").as_deref(), Some("Open in Claude"));
    // the foot of the card: always in view, under both sides
    let (dialog, composer, story, facts) = page.look(cx, |w| {
        let at = |id: &'static str| w.find(id).bounds();
        (at("task-dialog"), at("task-composer"), at("task-story"), at("task-facts"))
    });
    assert_eq!(
        (composer.left(), composer.right(), composer.bottom()),
        (dialog.left(), dialog.right(), dialog.bottom())
    );
    assert!(story.bottom() <= composer.top() && facts.bottom() <= composer.top(), "{story:?} {facts:?} {composer:?}");
    // what goes with your words
    assert_eq!(page.said(cx, "task-with").as_deref(), Some("SH-21309, PR #2414, the 4 things that need you"));
    // s writes to Claude: your words are kept as you write (a restart keeps them too)
    page.press(cx, "s");
    typed(&page, cx, "Finish the export first.");
    assert_eq!(draft(&page, cx, "ticket:SH-21309"), r#"{"note":"Finish the export first.","items":{}}"#);
    page.press(cx, "escape");
    assert!(page.there(cx, "task-dialog"), "Esc left the field, not the card");
    // an instruction for the remark, and the second remark left out
    page.press(cx, "j");
    page.press(cx, "n");
    typed(&page, cx, "Use the outlet's locale.");
    page.press(cx, "escape");
    page.click(cx, "item-include:thread:t2");
    assert!(!page.there(cx, "item-note:thread:t2"), "no instruction for one left out");
    assert_eq!(says(&page, cx, ("task-chip", 2usize)).as_deref(), Some("3 of the 4 things that need you"));
    // See the message: the whole of it, word for word as it goes
    assert_eq!(page.said(cx, "task-preview-toggle").as_deref(), Some("See the message"));
    assert!(!page.there(cx, "task-preview"));
    page.click(cx, "task-preview-toggle");
    assert_eq!(page.said(cx, "task-preview-toggle").as_deref(), Some("Hide the message"));
    assert_eq!(described(&page, cx, "task-preview").as_deref(), Some("as made for you"));
    let (goes, shown) = message(&page, cx);
    assert_eq!(shown.as_deref(), Some(goes.as_str()));
    for line in [
        "SH-21309: Export orders to the accounting CSV",
        "Ticket: https://demo.atlassian.net/browse/SH-21309",
        "\n\nFinish the export first.\n\nAddress these:\n",
        "   What to do: Use the outlet's locale.",
        "don't post anything to GitHub or Jira.",
    ] {
        assert!(goes.contains(line), "{line:?} is missing from {goes}");
    }
    assert!(!goes.contains("Rows with a zero total"), "the one left out isn't in it: {goes}");
    // it follows your words while you write
    page.press(cx, "s");
    typed(&page, cx, " QA tests it tomorrow.");
    let (goes, shown) = message(&page, cx);
    assert!(goes.contains("Finish the export first. QA tests it tomorrow."), "{goes}");
    assert_eq!(shown.as_deref(), Some(goes.as_str()), "the field shows the message as it is now");
    page.press(cx, "escape");
    page.click(cx, "task-preview-toggle");
    assert!(!page.there(cx, "task-preview"));
    // in a window that runs no terminals nothing can be typed anywhere: it says so, and your words stay
    page.click(cx, "task-send");
    cx.run_until_parked();
    assert_eq!(page.said(cx, "toast").as_deref(), Some("Couldn't put it in Claude's prompt"));
    assert!(draft(&page, cx, "ticket:SH-21309").contains("Finish the export first. QA tests it tomorrow."));
    assert!(page.there(cx, "task-dialog"));
}

/// Item 5.13: shows the whole message and lets you edit it: your text is what goes, and Undo my edits brings back the
/// one made for you (up to the sending).
#[gpui_kit::test]
fn shows_the_whole_message_and_lets_you_edit_it_and_undo_my_edits_brings_back_the_one_made_for_you(
    cx: &mut TestAppContext,
) {
    let page = setup(cx, sample(), vec![ws("w2", "sh-21309-export", "feature/SH-21309-export")], true);
    cx.simulate_window_resize(page.window, size(px(1360.), px(2000.)));
    open_card(&page, cx, "ticket:SH-21309");
    page.press(cx, "c");
    typed(&page, cx, "Finish the export first.");
    page.press(cx, "escape");
    page.click(cx, "task-preview-toggle");
    let (made, shown) = message(&page, cx);
    assert!(
        made.contains("SH-21309: Export orders to the accounting CSV") && made.contains("Finish the export first.")
    );
    assert_eq!(shown.as_deref(), Some(made.as_str()));
    // edited: your text is the message, and the field for your words steps aside
    page.click(cx, "task-preview");
    page.press(cx, "cmd-a");
    typed(&page, cx, "SH-21309: only fix the CSV header, nothing else.");
    assert_eq!(described(&page, cx, "task-preview").as_deref(), Some("edited"));
    assert!(!page.there(cx, "task-note") && !page.there(cx, "task-preview-toggle"));
    assert_eq!(page.said(cx, "task-message-reset").as_deref(), Some("Undo my edits"));
    assert_eq!(message(&page, cx).0, "SH-21309: only fix the CSV header, nothing else.");
    let kept = draft(&page, cx, "ticket:SH-21309");
    assert!(kept.contains(r#""text":"SH-21309: only fix the CSV header, nothing else.""#), "{kept}");
    // s goes to the message itself now, after what is there
    page.press(cx, "escape");
    page.press(cx, "s");
    typed(&page, cx, " Please.");
    assert_eq!(message(&page, cx).0, "SH-21309: only fix the CSV header, nothing else. Please.");
    page.press(cx, "escape");
    // undone: the message made from the card and your words is back, and so is the field for your words
    page.click(cx, "task-message-reset");
    assert!(page.there(cx, "task-note") && !page.there(cx, "task-message-reset"));
    assert!(!draft(&page, cx, "ticket:SH-21309").contains("\"text\""));
    let (again, shown) = message(&page, cx);
    assert_eq!(again, made);
    assert_eq!(shown.as_deref(), Some(made.as_str()), "the field holds the one made for you again");
    // the card opened again keeps an edit: the whole message is shown at once
    page.click(cx, "task-preview");
    page.press(cx, "cmd-a");
    typed(&page, cx, "Only the header.");
    page.press(cx, "escape");
    page.press(cx, "escape");
    assert_eq!(state(&page, cx).2, None);
    open_card(&page, cx, "ticket:SH-21309");
    assert_eq!(message(&page, cx), ("Only the header.".to_string(), Some("Only the header.".to_string())));
    assert!(!page.there(cx, "task-note"));
}

/// Item 5.13: the line at the card's foot says where the message goes as you pick, the button follows, and a press on
/// the line brings the keyboard to the box.
#[gpui_kit::test]
fn the_line_at_the_cards_foot_says_where_the_message_goes_as_you_pick_a_claude(cx: &mut TestAppContext) {
    let workspaces = vec![
        ws("w1", "stock-fix", "fix/stock-double-count"),
        ws("w2", "sh-21309-export", "feature/SH-21309-export"),
        ws("w3", "scratch", "scratch"),
    ];
    let mut index = named_in_a_prompt();
    index.ticket_projects.remove("SH-20728");
    let page = setup(cx, index, workspaces, true);
    cx.simulate_window_resize(page.window, size(px(2400.), px(1600.)));
    open_card(&page, cx, "ticket:SH-21309");
    let goes = |page: &Page, cx: &mut TestAppContext| -> (String, String, String) {
        (
            page.said(cx, "task-target").unwrap_or_default(),
            described(page, cx, "task-target").unwrap_or_default(),
            page.said(cx, "task-send").unwrap_or_default(),
        )
    };
    let said = |words: &str, kind: &str, button: &str| (words.to_string(), kind.to_string(), button.to_string());
    assert_eq!(
        goes(&page, cx),
        said(
            "goes into the prompt of the Claude in sh-21309-export; you press Enter there",
            "workspace",
            "Open in Claude"
        )
    );
    // a new one: a new conversation in the card's workspace
    page.click(cx, "claude-option:new");
    assert_eq!(
        goes(&page, cx),
        said(
            "a new conversation in sh-21309-export (the one there now stays saved), with this in its prompt",
            "switch",
            "Start Claude"
        )
    );
    // the one only named in a prompt: picked, the message goes there
    page.click(cx, "claude-option:s-w1");
    assert_eq!(
        goes(&page, cx),
        said("goes into the prompt of the Claude in stock-fix; you press Enter there", "workspace", "Open in Claude")
    );
    // the line at the bottom shows the box, the keyboard on the one picked
    assert_eq!(page.look(cx, |w| w.find("claude-option:s-w1").focused()), Some(false));
    page.click(cx, "task-target");
    assert_eq!(page.look(cx, |w| w.find("claude-option:s-w1").focused()), Some(true));
    // a new workspace, though the card has one: its own name, on the card's branch
    page.click(cx, "claude-option:fresh");
    let (words, kind, button) = goes(&page, cx);
    assert!(words.starts_with("a new workspace, sh-21309-export-2, on feature/SH-21309-export, with"), "{words}");
    assert_eq!((kind.as_str(), button.as_str()), ("new", "Start workspace"));
    // any other workspace's Claude
    page.click(cx, "claude-other");
    page.click(cx, "claude-other-option:s-w3");
    assert_eq!(goes(&page, cx).0, "goes into the prompt of the Claude in scratch; you press Enter there");
    // the menu is gone and the keyboard is the card's again: j moves through what needs you
    page.press(cx, "j");
    assert_eq!(page.look(cx, |w| w.find("detail-item:thread:t1").selected()), Some(true));
    // a ticket whose project isn't known has nowhere to start Claude: the message goes on the clipboard
    open_card(&page, cx, "ticket:SH-20728");
    assert_eq!(
        goes(&page, cx),
        said("nowhere to start Claude on this: the message goes on your clipboard", "none", "Copy the message")
    );
    assert!(!page.there(cx, "detail-sessions"), "no Claude box: nothing to pick");
    page.press(cx, "s");
    typed(&page, cx, "Start on the import.");
    page.press(cx, "escape");
    page.click(cx, "task-send");
    cx.run_until_parked();
    let copied = cx.read_from_clipboard().and_then(|item| item.text()).unwrap_or_default();
    assert!(
        copied.starts_with("SH-20728: Supplier price import\n") && copied.contains("\n\nStart on the import.\n"),
        "{copied}"
    );
    assert_eq!(page.said(cx, "toast").as_deref(), Some("Nowhere to start Claude on this"));
    assert!(page.there(cx, "task-dialog"), "the card stays: nothing went anywhere but the clipboard");
}

/// The old top bar test's index: two open PRs, each on a workspace's branch; #2414's checks fail, and that waits on
/// you.
fn two_prs() -> Snapshot {
    let pr = |number: i64, checks: &str, failing: serde_json::Value| {
        serde_json::json!({
            "projectId": "p1", "repo": "Org/shop", "number": number, "title": format!("PR {number}"), "url": "",
            "state": "OPEN", "draft": false, "head": format!("feature/{number}"), "base": "development",
            "reviewDecision": null, "checks": checks, "failing": failing, "running": 0, "mergeable": null,
            "headSha": null, "headAt": null, "unresolved": 0, "updatedAt": "2026-11-05T10:00:00Z", "mergedAt": null,
            "commits": [],
        })
    };
    let link = |a: &str, b: &str| serde_json::json!({ "a": a, "b": b, "how": "branch", "detail": "", "at": 1, "removed": false });
    let item = |id: &str, kind: &str, pr: &str| {
        serde_json::json!({
            "id": id, "kind": kind, "source": "github", "projectId": "p1", "pr": pr, "ticket": null,
            "title": "Checks failing: tests", "detail": "", "who": "mira-k", "url": null, "path": null, "line": null,
            "at": "2026-11-05T10:00:00Z", "state": "open", "openedAt": 1, "closedAt": null, "snoozedUntil": null,
            "bot": false,
        })
    };
    serde_json::from_value(serde_json::json!({
        "health": [], "tickets": [], "sessions": [], "events": [], "checking": false,
        "prs": [
            pr(2395, "SUCCESS", serde_json::json!([])),
            pr(2414, "FAILURE", serde_json::json!([{ "name": "tests", "state": "FAILURE", "url": null }])),
            pr(2437, "SUCCESS", serde_json::json!([])),
        ],
        "links": [
            link("pr:Org/shop#2395", "workspace:w1"),
            link("pr:Org/shop#2414", "workspace:w2"),
            link("pr:Org/shop#2437", "workspace:w2"),
        ],
        "items": [
            item("checks:Org/shop#2414:x", "pipeline_failed", "Org/shop#2414"),
            item("thread:r1", "remark", "Org/shop#2437"),
        ],
    }))
    .expect("an index")
}

/// Item 5.11b: "shows a workspace's PR on its tab, with its checks, and blue when something waits on you"; and the
/// status line's "PR #2414: checks failing".
#[gpui_kit::test]
fn shows_a_workspaces_pr_on_its_tab_with_its_checks_and_in_the_status_line_what_waits_on_it(cx: &mut TestAppContext) {
    let workspaces = vec![ws("w1", "stock-fix", "feature/2395"), ws("w2", "export", "feature/2414")];
    let page = setup(cx, two_prs(), workspaces, false);
    let chip = |page: &Page, cx: &mut TestAppContext, ix: usize| -> (Option<String>, Option<String>, Option<String>) {
        (says(page, cx, ("tab-pr", ix)), value_of(page, cx, ("tab-pr", ix)), tells(page, cx, ("tab-pr", ix)))
    };
    let said = |title: &str, lamp: &str, waits: &str| {
        (Some(title.to_string()), Some(lamp.to_string()), Some(waits.to_string()))
    };
    // stock-fix's PR: its checks passed and nothing waits; export's: failing, and that waits on you
    assert_eq!(chip(&page, cx, 0), said("PR #2395: PR 2395", "green", "nothing waits"));
    assert_eq!(chip(&page, cx, 1), said("PR #2414: checks failing", "red", "waits on you"));
    // it stands at the end of its tab, after the name: a dot and the number
    let (tab, pr) = page.look(cx, |w| (w.find(("tab", 0usize)).bounds(), w.find(("tab-pr", 0usize)).bounds()));
    assert!(pr.left() > tab.left() + px(60.) && pr.right() <= tab.right() - px(9.), "{tab:?} {pr:?}");
    assert!(pr.size.width > px(30.) && pr.size.height <= px(16.), "the dot and #2395: {pr:?}");
    // pointed at, it says what waits
    cx.update_window(page.window, |_, w, cx| {
        w.render_frame(cx);
        w.hover(("tab-pr", 1usize), cx)
    })
    .unwrap();
    cx.executor().advance_clock(std::time::Duration::from_millis(900));
    assert_eq!(page.said(cx, "tip").as_deref(), Some("PR #2414: checks failing"));
    // the status line says it for the workspace on screen: nothing for stock-fix, whose PR waits on nothing
    assert!(!page.there(cx, "status-work"));
    cx.update_window(page.window, |_, w, cx| {
        w.render_frame(cx);
        w.click(("tab", 1usize), cx)
    })
    .unwrap();
    assert_eq!(page.said(cx, "status-work").as_deref(), Some("PR #2414: checks failing, and 1 more PR"));
    assert_eq!(described(&page, cx, "status-work").as_deref(), Some("blocks"));
    let (branch, claude, work) = page.look(cx, |w| {
        let at = |id: &'static str| w.find(id).bounds();
        (at("status-branch"), at("status-claude"), at("status-work"))
    });
    assert!(branch.right() < claude.left() && claude.right() < work.left(), "{branch:?} {claude:?} {work:?}");
    // with little room the tabs are compact: the dot alone
    let many: Vec<Workspace> = (0..9)
        .map(|n| ws(&format!("w{}", n + 1), &format!("task-{n}"), &format!("feature/{}", 2395 + n * 11)))
        .collect();
    let page = setup(cx, two_prs(), many, false);
    cx.simulate_window_resize(page.window, size(px(1000.), px(700.)));
    let pr = page.look(cx, |w| w.find(("tab-pr", 0usize)).bounds());
    assert_eq!(pr.size.width, px(6.), "{pr:?}");
    assert_eq!(chip(&page, cx, 0).0.as_deref(), Some("PR #2395: PR 2395"), "it still says what it is");
    // a workspace without a PR has no such mark
    assert!(page.look(cx, |w| w.try_find(("tab-pr", 3usize)).is_none()));
}

/// The value an element carries, by any id.
fn value_of(page: &Page, cx: &mut TestAppContext, id: impl Into<gpui_kit::ElementId>) -> Option<String> {
    let id = id.into();
    page.look(cx, |w| w.try_find(id).and_then(|e| e.value().map(String::from)))
}

/// One of Claude's reports on SH-21309, by the Claude in the workspace w2, `ago` milliseconds before now.
fn report(id: i64, state: &str, text: &str, images: &[&str], ago: i64) -> enkidu_core::work::reports::Report {
    enkidu_core::work::reports::Report {
        id,
        subject: "SH-21309".into(),
        session_id: Some("s-w2".into()),
        workspace_id: Some("w2".into()),
        state: state.into(),
        text: text.into(),
        images: images.iter().map(|name| name.to_string()).collect(),
        at: NOW - ago,
    }
}

/// What was asked of the screenshots' reader, in order.
type Asked = std::sync::Arc<std::sync::Mutex<Vec<(i64, String)>>>;

/// The window's screenshots come from the test: a made-up page 640 by 400, or nothing for a name with "broken" in it.
fn give_screenshots(cx: &mut TestAppContext) -> Asked {
    let asked = Asked::default();
    let heard = asked.clone();
    cx.update(|cx| {
        cx.set_global(enkidu_app::work::shots::ShotSource(std::sync::Arc::new(move |report, name| {
            heard.lock().unwrap().push((report, name.to_string()));
            match name.contains("broken") {
                true => Err("The picture is gone.".to_string()),
                false => Ok(enkidu_testkit::screenshot_png(640, 400, report as u32)),
            }
        })));
    });
    asked
}

/// Item 5.14: shows how Claude says a task ended: on the card, then its report and screenshots in the task.
#[gpui_kit::test]
fn shows_how_claude_says_a_task_ended_on_the_card_then_its_report_and_screenshots_in_the_task(cx: &mut TestAppContext) {
    let asked = give_screenshots(cx);
    let mut work = sample();
    work.reports = vec![
        report(
            7,
            "review",
            "Fixed the CSV header.\nChecked the export with 3 orders.",
            &["1-export.png", "2-orders.png"],
            60_000,
        ),
        report(3, "blocked", "The export needs a decision on the date format.", &[], 86_400_000),
    ];
    let page = setup(cx, work, vec![ws("w2", "sh-21309-export", "feature/SH-21309-export")], true);
    // on the card: how the latest one ended
    assert_eq!(page.said(cx, "card-report:ticket:SH-21309").as_deref(), Some("Review"));
    open_card(&page, cx, "ticket:SH-21309");
    // in the task it comes first: the newest report, how it ended, when and whose Claude, its words
    assert_eq!(page.said(cx, "task-reports").as_deref(), Some("Claude's report"));
    assert_eq!(
        page.said(cx, "task-report:7").as_deref(),
        Some("Finished, wants your review, 1 min ago by the Claude in sh-21309-export")
    );
    assert_eq!(value_of(&page, cx, "task-report:7").as_deref(), Some("review"));
    let words = page.said(cx, "report-text:7").unwrap_or_default();
    assert!(words.contains("Checked the export with 3 orders."), "{words}");
    assert!(!page.there(cx, "task-report:3"), "the earlier report waits behind its button");
    let (reports, task, needs) = page.look(cx, |w| {
        let at = |id: &'static str| w.find(id).bounds();
        (at("task-reports"), at("task-description"), at("task-needs"))
    });
    assert!(reports.bottom() <= task.top() && task.bottom() <= needs.top(), "{reports:?} {task:?} {needs:?}");
    // its screenshots, in the order Claude gave them: each read once, small, off the drawing thread
    assert_eq!(page.said(cx, "report-images:7").as_deref(), Some("1-export.png, 2-orders.png"));
    assert_eq!(*asked.lock().unwrap(), [(7, "1-export.png".to_string()), (7, "2-orders.png".to_string())]);
    assert_eq!(tells(&page, cx, "report-image:7:1-export.png").as_deref(), Some("shown"));
    assert_eq!(tells(&page, cx, "report-image:7:2-orders.png").as_deref(), Some("shown"));
    let (first, second, inside) = page.look(cx, |w| {
        let at = |id: &'static str| w.find(id).bounds();
        (at("report-image:7:1-export.png"), at("report-image:7:2-orders.png"), at("task-report:7"))
    });
    // the same size but for half a pixel (a third of the row's width, put on whole pixels), side by side
    let near = |a: gpui_kit::Pixels, b: gpui_kit::Pixels| (f32::from(a) - f32::from(b)).abs() <= 0.5;
    assert!(near(first.size.width, second.size.width) && first.size.height == second.size.height);
    assert_eq!(first.top(), second.top(), "side by side");
    assert!(near(second.left(), first.right() + px(8.)), "{first:?} {second:?}");
    let shape = f32::from(first.size.width) / f32::from(first.size.height);
    assert!((shape - 1.6).abs() < 0.02, "16 to 10: {first:?}");
    // three to a row in a window this wide, inside the report's box
    let room = f32::from(inside.size.width) - 30.;
    assert!((f32::from(first.size.width) - (room - 16.) / 3.).abs() < 1., "{first:?} in {inside:?}");

    // a screenshot opens as large as the window; → goes on around the end; Esc closes it, not the task
    page.click(cx, "report-image:7:2-orders.png");
    assert_eq!(page.said(cx, "report-lightbox").as_deref(), Some("2-orders.png"));
    assert_eq!(tells(&page, cx, "report-lightbox").as_deref(), Some("2 of 2"));
    let (layer, picture) =
        page.look(cx, |w| (w.find("report-lightbox").bounds(), w.find("report-lightbox-image").bounds()));
    assert_eq!(layer.size, size(px(1360.), px(860.)), "over the whole window");
    assert_eq!(picture.size, size(px(640.), px(400.)), "never larger than the picture is");
    assert_eq!(tells(&page, cx, "report-lightbox-image").as_deref(), Some("whole"));
    // the card's own keys rest while it shows: j moves nothing behind it
    let on = |page: &Page, cx: &mut TestAppContext, id: &'static str| page.look(cx, |w| w.find(id).selected());
    assert_eq!(on(&page, cx, "detail-item:question:c9"), Some(true));
    page.press(cx, "j");
    assert_eq!(on(&page, cx, "detail-item:question:c9"), Some(true));
    page.press(cx, "right");
    assert_eq!(page.said(cx, "report-lightbox").as_deref(), Some("1-export.png"));
    assert_eq!(tells(&page, cx, "report-lightbox").as_deref(), Some("1 of 2"));
    page.press(cx, "left");
    assert_eq!(page.said(cx, "report-lightbox").as_deref(), Some("2-orders.png"));
    page.press(cx, "escape");
    assert!(!page.there(cx, "report-lightbox"));
    assert!(page.there(cx, "task-dialog"), "the task stays");
    // it is over the card: a press where the card's Activity tab stands is a press beside the picture, and the
    // card hears nothing of it
    let tab = page.look(cx, |w| w.find("task-tab:activity").bounds().center());
    page.click(cx, "report-image:7:1-export.png");
    click_at(&page, cx, f32::from(tab.x), f32::from(tab.y));
    assert!(!page.there(cx, "report-lightbox"));
    assert_eq!(page.look(cx, |w| w.find("task-tab:overview").selected()), Some(true), "the card's tab stays");
    // a click on the picture keeps it, a click beside it closes it, and so does its ×
    page.click(cx, "report-image:7:1-export.png");
    page.click(cx, "report-lightbox-image");
    assert!(page.there(cx, "report-lightbox"));
    assert!(page.look(cx, |w| w.find("task-tab:overview").selected()) == Some(true));
    click_at(&page, cx, 60., 700.);
    assert!(!page.there(cx, "report-lightbox"));
    assert!(page.there(cx, "task-dialog"), "a click beside the picture is not a click beside the task");
    page.click(cx, "report-image:7:1-export.png");
    page.click(cx, "report-lightbox-next");
    assert_eq!(page.said(cx, "report-lightbox").as_deref(), Some("2-orders.png"));
    page.click(cx, "report-lightbox-close");
    assert!(!page.there(cx, "report-lightbox"));
    // the keyboard is the card's again: j moves through what needs you (and brings it into view)
    page.press(cx, "j");
    assert_eq!(on(&page, cx, "detail-item:thread:t1"), Some(true));
    cx.update_window(page.window, |_, w, cx| {
        w.render_frame(cx);
        w.scroll("task-story", ScrollDelta::Pixels(point(px(0.), px(4000.))), cx)
    })
    .unwrap();

    // the earlier report is a click away
    assert_eq!(page.said(cx, "task-reports-more").as_deref(), Some("Earlier reports (1)"));
    page.click(cx, "task-reports-more");
    assert!(!page.there(cx, "task-reports-more"));
    assert_eq!(value_of(&page, cx, "task-report:3").as_deref(), Some("blocked"));
    assert_eq!(
        page.said(cx, "task-report:3").as_deref(),
        Some("Stuck, needs you, yesterday by the Claude in sh-21309-export")
    );
    let (newest, earlier) = page.look(cx, |w| (w.find("task-report:7").bounds(), w.find("task-report:3").bounds()));
    assert!(newest.bottom() < earlier.top(), "{newest:?} {earlier:?}");
    // the report names its workspace's Claude: a press there goes to it, and the task closes
    page.click(cx, "report-workspace:7");
    assert!(!page.there(cx, "task-dialog"));
    assert!(!page.open(cx), "the workspace is on screen in place of the Work page");
}

/// Item 5.14: a screenshot that can't be read says so in its place, small and large; the others show. An update
/// along the way alone is "How it goes". Another card in the same dialog asks for its own screenshots only.
#[gpui_kit::test]
fn a_screenshot_that_cant_be_read_says_so_and_an_update_alone_says_how_it_goes(cx: &mut TestAppContext) {
    let asked = give_screenshots(cx);
    let mut work = sample();
    work.reports = vec![
        report(9, "update", "Running the migrations now.", &["1-broken.png", "2-migrations.png"], 30_000),
        enkidu_core::work::reports::Report {
            subject: "SH-21533".into(),
            ..report(5, "done", "All 14 tests pass.", &["1-tests.png"], 120_000)
        },
    ];
    let page = setup(cx, work, vec![ws("w2", "sh-21309-export", "feature/SH-21309-export")], true);
    // an update says nothing about how the task ended: no mark on the card
    assert!(!page.there(cx, "card-report:ticket:SH-21309"));
    assert_eq!(page.said(cx, "card-report:ticket:SH-21533").as_deref(), Some("Done"));
    open_card(&page, cx, "ticket:SH-21309");
    assert_eq!(page.said(cx, "task-reports").as_deref(), Some("How it goes"));
    assert_eq!(page.said(cx, "task-report:9").as_deref(), Some("Update, just now by the Claude in sh-21309-export"));
    assert!(!page.there(cx, "task-reports-more"));
    assert_eq!(tells(&page, cx, "report-image:9:1-broken.png").as_deref(), Some("can't be shown"));
    assert_eq!(tells(&page, cx, "report-image:9:2-migrations.png").as_deref(), Some("shown"));
    // large, it says so too, and → still goes to the one that shows
    page.click(cx, "report-image:9:1-broken.png");
    assert_eq!(page.said(cx, "report-lightbox-failed").as_deref(), Some("Can't show 1-broken.png"));
    page.press(cx, "right");
    assert_eq!(tells(&page, cx, "report-lightbox-image").as_deref(), Some("whole"));
    page.press(cx, "escape");
    // another card takes the dialog: its screenshot is read, the first card's are not read again
    let before = asked.lock().unwrap().len();
    open_card(&page, cx, "ticket:SH-21533");
    assert_eq!(
        page.said(cx, "task-report:5").as_deref(),
        Some("Finished, nothing needs you, 2 min ago by the Claude in sh-21309-export")
    );
    assert_eq!(tells(&page, cx, "report-image:5:1-tests.png").as_deref(), Some("shown"));
    assert_eq!(asked.lock().unwrap()[before..], [(5, "1-tests.png".to_string())]);
    // one screenshot alone: at most 460 px wide
    let alone = page.look(cx, |w| w.find("report-image:5:1-tests.png").bounds());
    assert_eq!(alone.size.width, px(460.), "{alone:?}");
    // one alone has no arrows, and → stays on it
    page.click(cx, "report-image:5:1-tests.png");
    assert!(!page.there(cx, "report-lightbox-next") && !page.there(cx, "report-lightbox-prev"));
    assert_eq!(tells(&page, cx, "report-lightbox").as_deref(), Some("1 of 1"));
    page.press(cx, "right");
    assert_eq!(page.said(cx, "report-lightbox").as_deref(), Some("1-tests.png"));
}

/// Item 5.14: a picture that was drawn is given back when it goes. The window keeps every picture it drew until it
/// is told to let go: the whole one goes when you move on to the next and when it closes, a card's thumbnails when
/// another card takes the dialog and when the card closes.
#[gpui_kit::test]
fn the_pictures_of_a_report_leave_the_window_when_they_go(cx: &mut TestAppContext) {
    use enkidu_app::work::shots::Small;
    type Picture = std::sync::Arc<gpui_kit::RenderImage>;
    give_screenshots(cx);
    let mut work = sample();
    work.reports = vec![
        report(7, "review", "Checked the export.", &["1-export.png", "2-orders.png"], 60_000),
        enkidu_core::work::reports::Report {
            subject: "SH-21533".into(),
            ..report(5, "done", "All 14 tests pass.", &["1-tests.png"], 120_000)
        },
    ];
    let page = setup(cx, work, vec![ws("w2", "sh-21309-export", "feature/SH-21309-export")], true);
    open_card(&page, cx, "ticket:SH-21309");
    // (weak, so the card can go when its dialog does)
    let shots = page.shell.read_with(cx, |shell, cx| {
        let card = shell.work_page().expect("the Work page").read(cx).card_view().expect("the card").clone();
        card.read(cx).shots().downgrade()
    });
    let small = |cx: &mut TestAppContext, report: i64, name: &str| -> Option<Picture> {
        cx.run_until_parked();
        let shots = shots.upgrade()?;
        shots.read_with(cx, |shots, _| match shots.small(report, name) {
            Some(Small::Ready { picture, .. }) => Some(picture.clone()),
            _ => None,
        })
    };
    let large = |cx: &mut TestAppContext| -> Option<Picture> {
        cx.run_until_parked();
        shots.upgrade()?.read_with(cx, |shots, _| shots.large_picture().cloned())
    };
    let held =
        |cx: &mut TestAppContext, picture: &Picture| -> bool { page.look(cx, |w| w.has_image_atlas_entry(picture)) };
    let export = small(cx, 7, "1-export.png").expect("the first thumbnail is read");
    let orders = small(cx, 7, "2-orders.png").expect("the second thumbnail is read");
    assert!(held(cx, &export) && held(cx, &orders), "drawn, so the window keeps them");
    // large: the whole picture is the window's too; → lets it go for the next one, Esc lets that one go
    page.click(cx, "report-image:7:1-export.png");
    let whole = large(cx).expect("the whole picture is read");
    assert!(held(cx, &whole));
    page.press(cx, "right");
    let next = large(cx).expect("the next whole picture is read");
    assert!(next.id != whole.id && held(cx, &next));
    assert!(!held(cx, &whole), "the one you left is given back");
    page.press(cx, "escape");
    assert!(large(cx).is_none() && !held(cx, &next), "closed, the whole picture is given back");
    assert!(held(cx, &export) && held(cx, &orders), "the thumbnails stay with the card");
    // another card takes the dialog: the first card's thumbnails go, its own come
    open_card(&page, cx, "ticket:SH-21533");
    let tests = small(cx, 5, "1-tests.png").expect("the other card's thumbnail is read");
    assert!(held(cx, &tests));
    assert!(!held(cx, &export) && !held(cx, &orders), "the first card's thumbnails are given back");
    assert!(small(cx, 7, "1-export.png").is_none(), "and forgotten");
    // the card closes: its pictures go with it
    page.press(cx, "escape");
    cx.executor().advance_clock(std::time::Duration::from_secs(1));
    assert!(!page.there(cx, "task-dialog"));
    assert!(shots.upgrade().is_none(), "the card is gone, and its screenshots with it");
    assert!(!held(cx, &tests), "the last thumbnail is given back");
}

/// Item 5.15: checks the connections: each source compared, a broken one says why, differences listed.
#[gpui_kit::test]
fn checks_the_connections_each_source_compared_a_broken_one_says_why_differences_listed(cx: &mut TestAppContext) {
    use enkidu_core::work::{Checked, live::Difference};
    let asked = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = asked.clone();
    cx.update(|cx| {
        cx.set_global(enkidu_app::work::check::CheckSource(std::rc::Rc::new(move |cx: &mut gpui_kit::App| {
            count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let checked = |project: &str, source: &str, ok: bool, compared: &[&str], error: Option<&str>| Checked {
                project: project.into(),
                source: source.into(),
                ok,
                compared: compared.iter().map(|line| line.to_string()).collect(),
                differences: Vec::new(),
                error: error.map(String::from),
            };
            let differs =
                Difference { thing: "#12".into(), field: "title".into(), index: "Old".into(), direct: "New".into() };
            let results = vec![
                checked("shop", "github", true, &["4 open PRs as `gh pr list --author @me` lists them"], None),
                checked("shop", "jira", false, &[], Some("twg isn't signed in. Run: twg login --force")),
                Checked { differences: vec![differs], ..checked("cafe", "github", false, &["1 open PRs"], None) },
            ];
            // the answer comes a moment later, as the core's does
            cx.background_spawn(async move { Ok(results) })
        })));
    });
    let page = setup(cx, sample(), Vec::new(), true);
    let asked = move || asked.load(std::sync::atomic::Ordering::SeqCst);
    // what is on screen right now, before anything that was asked comes back
    let now = |page: &Page, cx: &mut TestAppContext, id: &'static str| -> Option<String> {
        cx.update_window(page.window, |_, w, cx| {
            w.render_frame(cx);
            w.try_find(id).and_then(|e| e.label().map(String::from))
        })
        .unwrap()
    };
    // opening it starts a check: it says so while it runs, and its button waits
    page.click(cx, "work-check");
    assert_eq!(now(&page, cx, "check-dialog").as_deref(), Some("Check connections"));
    assert_eq!(
        now(&page, cx, "check-running").as_deref(),
        Some("Reading GitHub and Jira, then asking gh and twg directly…")
    );
    assert_eq!(now(&page, cx, "check-again").as_deref(), Some("Checking…"));
    cx.update_window(page.window, |_, w, cx| w.click("check-again", cx)).unwrap();
    // each source of each project is a card: how it stands, why a broken one can't be read, each difference
    assert!(!page.there(cx, "check-running"));
    assert_eq!(asked(), 1, "a press while it runs asks nothing more");
    let stands: Vec<Option<String>> = (0..3usize).map(|at| tells(&page, cx, ("check-result", at))).collect();
    assert_eq!(stands, [Some("github true".into()), Some("jira false".into()), Some("github false".into())]);
    assert!(!page.look(cx, |w| w.try_find(("check-result", 3usize)).is_some()));
    let said: Vec<String> = (0..3usize).map(|at| says(&page, cx, ("check-result", at)).unwrap_or_default()).collect();
    assert_eq!(said[0], "GitHub for shop, matches\n4 open PRs as `gh pr list --author @me` lists them");
    assert_eq!(said[1], "Jira for shop, can't be read\ntwg isn't signed in. Run: twg login --force");
    assert_eq!(said[2], "GitHub for cafe, 1 differ\n1 open PRs\n#12 title: the index has Old, GitHub says New");
    assert_eq!(page.said(cx, "check-again").as_deref(), Some("Check again"));
    // the cards stand under each other, inside a dialog 600 px wide, the button at its foot
    let (dialog, first, second, third, again) = page.look(cx, |w| {
        let at = |at: usize| w.find(("check-result", at)).bounds();
        (w.find("check-dialog").bounds(), at(0), at(1), at(2), w.find("check-again").bounds())
    });
    assert_eq!(dialog.size.width, px(598.), "600 px less the dialog's edge: {dialog:?}");
    assert!(first.bottom() < second.top() && second.bottom() < third.top() && third.bottom() < again.top());
    assert_eq!(first.size.width, px(558.), "{first:?}");
    // checking again asks again
    page.click(cx, "check-again");
    assert!(page.look(cx, |w| w.try_find(("check-result", 2usize)).is_some()), "the cards are there again");
    assert_eq!(asked(), 2);
    // ↩ checks again too, and never closes it; Esc closes it and the page is as it was
    page.press(cx, "enter");
    assert!(page.there(cx, "check-dialog"));
    assert_eq!(asked(), 3);
    page.press(cx, "escape");
    assert!(!page.there(cx, "check-dialog"));
    assert!(page.open(cx), "the Work page stays on screen");
}

/// Item 5.15: with no project that reads GitHub or Jira the dialog says so, and how to turn Jira on.
#[gpui_kit::test]
fn checking_the_connections_of_nothing_says_how_to_turn_them_on(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.set_global(enkidu_app::work::check::CheckSource(std::rc::Rc::new(|_: &mut gpui_kit::App| {
            gpui_kit::Task::ready(Ok(Vec::new()))
        })))
    });
    let page = setup(cx, sample(), Vec::new(), true);
    page.click(cx, "work-check");
    assert_eq!(
        page.said(cx, "check-nothing").as_deref(),
        Some("No project reads GitHub or Jira yet. Jira is turned on per project in its settings.")
    );
    assert!(page.look(cx, |w| w.try_find(("check-result", 0usize)).is_none()));
    page.click(cx, "check-close");
    assert!(!page.there(cx, "check-dialog"));
}

/// Shows one Jira board's tickets once you pick it, and all of them again (the menu on screen with two boards; the same
/// through the core and the fake twg is in tests/work_runs.rs).
#[gpui_kit::test]
fn the_board_menu_lists_jiras_boards_and_a_picked_one_shows_only_its_tickets(cx: &mut TestAppContext) {
    use enkidu_core::work::model::{Board, BoardPick};
    let board = |id: &str, name: &str, kind: &str| Board {
        id: id.into(),
        name: name.into(),
        kind: kind.into(),
        project_key: Some("SH".into()),
    };
    let mut work = sample();
    work.boards = vec![board("5", "Shop Dev", "scrum"), board("9", "Deployment view", "kanban")];
    let page = setup(cx, work, Vec::new(), true);
    let all = shown(&page, cx).len();
    assert!(all > 2);
    assert_eq!(page.said(cx, "work-board-pick").as_deref(), Some("All tasks"));
    // the menu: All tasks, then each board with "sprint" or its kind
    page.click(cx, "work-board-pick");
    let option =
        |page: &Page, cx: &mut TestAppContext, id: &'static str| -> (Option<String>, Option<String>, Option<bool>) {
            (says(page, cx, id), tells(page, cx, id), page.look(cx, |w| w.try_find(id).and_then(|e| e.selected())))
        };
    assert_eq!(option(&page, cx, "work-board-option:"), (Some("All tasks".into()), Some(String::new()), Some(true)));
    assert_eq!(option(&page, cx, "work-board-option:5"), (Some("Shop Dev".into()), Some("sprint".into()), Some(false)));
    assert_eq!(
        option(&page, cx, "work-board-option:9"),
        (Some("Deployment view".into()), Some("kanban".into()), Some(false))
    );
    // picked: the header names the board at once
    page.click(cx, "work-board-option:5");
    assert_eq!(page.said(cx, "work-board-pick").as_deref(), Some("Shop Dev"));
    let picked = page.work.read_with(cx, |work, _| work.snapshot.as_ref().and_then(|index| index.board.clone()));
    assert_eq!(picked.as_ref().map(|pick| (pick.board.id.as_str(), pick.keys.clone())), Some(("5", None)));
    // its cards come when its read lands (the core's answer, given here by hand): only your tickets on it
    page.work.update(cx, |work, cx| {
        let mut index = (**work.snapshot.as_ref().expect("the index")).clone();
        index.board = Some(BoardPick {
            board: board("5", "Shop Dev", "scrum"),
            keys: Some(vec!["SH-21309".into(), "SH-21376".into()]),
            read_at: Some(NOW),
        });
        work.snapshot = Some(std::sync::Arc::new(index));
        work.regroup();
        cx.notify();
    });
    let mut on_board = shown(&page, cx);
    on_board.sort();
    assert_eq!(on_board, ["SH-21309", "SH-21376"]);
    // the menu marks it; All tasks shows all of them again
    page.click(cx, "work-board-pick");
    assert_eq!(page.look(cx, |w| w.find("work-board-option:5").selected()), Some(true));
    page.click(cx, "work-board-option:");
    assert_eq!(shown(&page, cx).len(), all);
    assert_eq!(page.said(cx, "work-board-pick").as_deref(), Some("All tasks"));
}

/// DESIGN.md, "Motion with a job": a card that moved to another column glows once, for 1.6 s, and only it; and in front
/// of a card a PR's dot pops when its checks flip, the row keeping its place.
#[gpui_kit::test]
fn a_card_that_moved_to_another_column_glows_once_and_a_prs_dot_pops_when_its_checks_flip(cx: &mut TestAppContext) {
    let page = setup(cx, sample(), vec![ws("w2", "sh-21309-export", "feature/SH-21309-export")], true);
    let moved = |page: &Page, cx: &mut TestAppContext| -> Vec<String> {
        cx.run_until_parked();
        page.shell.read_with(cx, |shell, cx| shell.work_page().expect("the Work page").read(cx).moved())
    };
    let change = |page: &Page, cx: &mut TestAppContext, with: &dyn Fn(&mut Snapshot)| {
        page.work.update(cx, |work, cx| {
            let mut index = (**work.snapshot.as_ref().expect("the index")).clone();
            with(&mut index);
            work.snapshot = Some(std::sync::Arc::new(index));
            work.regroup();
            cx.notify();
        });
    };
    assert!(moved(&page, cx).is_empty(), "a board that opens shows no card as moved");
    // what moves nothing: the filter, Needs you, a card hidden
    page.press(cx, "/");
    typed(&page, cx, "stock");
    page.press(cx, "escape");
    page.press(cx, "escape");
    page.click(cx, "work-needs-you");
    page.click(cx, "work-needs-you");
    assert!(moved(&page, cx).is_empty());
    // SH-21309 goes on to Code Review: it glows, the others don't
    change(&page, cx, &|index| {
        index.tickets.iter_mut().find(|ticket| ticket.key == "SH-21309").unwrap().status = "Code Review".into();
    });
    assert_eq!(moved(&page, cx), ["ticket:SH-21309"]);
    let column = page.shell.read_with(cx, |shell, cx| {
        let board = shell.work_page().unwrap().read(cx).board();
        let column = board.columns.iter().find(|column| column.cards.iter().any(|card| card.id == "ticket:SH-21309"));
        column.map(|column| column.name.clone())
    });
    assert_eq!(column.as_deref(), Some("Code Review"));
    // its time over, it is a card like the others
    cx.executor().advance_clock(std::time::Duration::from_millis(1800));
    assert!(moved(&page, cx).is_empty());
    // a second move while the first still glows: both glow, and the first one's end doesn't end the second's early
    change(&page, cx, &|index| {
        index.tickets.iter_mut().find(|ticket| ticket.key == "SH-21616").unwrap().status = "Code Review".into();
    });
    cx.executor().advance_clock(std::time::Duration::from_millis(900));
    change(&page, cx, &|index| {
        index.tickets.iter_mut().find(|ticket| ticket.key == "SH-21533").unwrap().status = "In Progress".into();
    });
    assert_eq!(moved(&page, cx), ["ticket:SH-21533"], "the newest move is the one that glows");
    cx.executor().advance_clock(std::time::Duration::from_millis(900));
    assert_eq!(moved(&page, cx), ["ticket:SH-21533"], "the earlier glow's end leaves it");
    cx.executor().advance_clock(std::time::Duration::from_millis(900));
    assert!(moved(&page, cx).is_empty());

    // in front of the card: the checks' dot. With motion, it pops when the checks flip, and only then
    cx.update(|cx| cx.set_reduce_motion(false));
    open_card(&page, cx, "ticket:SH-21309");
    let dot = |page: &Page, cx: &mut TestAppContext| -> (Option<String>, gpui_kit::Bounds<gpui_kit::Pixels>) {
        cx.update_window(page.window, |_, w, cx| {
            w.render_frame(cx);
            let dot = w.find("task-checks-lamp:2414");
            (dot.label().map(String::from), dot.bounds())
        })
        .unwrap()
    };
    cx.run_until_parked();
    cx.executor().advance_clock(std::time::Duration::from_millis(600));
    let (stands, still) = dot(&page, cx);
    assert_eq!(stands.as_deref(), Some("failing"));
    assert_eq!(still.size, size(px(7.), px(7.)), "a card that opens shows its dot still");
    let row = page.look(cx, |w| w.find("task-checks:2414").bounds());
    // the checks pass now
    change(&page, cx, &|index| {
        let pr = index.prs.iter_mut().find(|pr| pr.number == 2414).unwrap();
        pr.checks = Some("SUCCESS".into());
        pr.failing.clear();
    });
    cx.run_until_parked();
    let (stands, popping) = dot(&page, cx);
    assert_eq!(stands.as_deref(), Some("passed"));
    assert!(popping.size.width < px(4.), "it starts small: {popping:?}");
    assert_eq!(popping.center(), still.center(), "around its own middle");
    assert_eq!(page.look(cx, |w| w.find("task-checks:2414").bounds()), row, "the row keeps its place");
    cx.executor().advance_clock(std::time::Duration::from_millis(400));
    let (_, settled) = dot(&page, cx);
    assert_eq!(settled.size, size(px(7.), px(7.)));
}

/// A click on an element by any id (a row's own pieces are numbered), in its middle.
fn press_on(page: &Page, cx: &mut TestAppContext, id: impl Into<gpui_kit::ElementId>) {
    let id = id.into();
    cx.run_until_parked();
    cx.update_window(page.window, |_, w, cx| {
        w.render_frame(cx);
        let (position, modifiers, left) = (w.find(id).bounds().center(), Modifiers::default(), MouseButton::Left);
        w.dispatch_event(MouseMoveEvent { position, pressed_button: None, modifiers }.to_platform_input(), cx);
        let down = MouseDownEvent { button: left, position, modifiers, click_count: 1, first_mouse: false };
        w.dispatch_event(down.to_platform_input(), cx);
        w.dispatch_event(MouseUpEvent { button: left, position, modifiers, click_count: 1 }.to_platform_input(), cx);
        w.render_frame(cx);
    })
    .unwrap();
}

/// Picks cards on the board by their round boxes.
fn pick_cards(page: &Page, cx: &mut TestAppContext, keys: &[&str]) {
    for key in keys {
        click_id(page, cx, SharedString::from(format!("card-pick:ticket:{key}")));
    }
}

/// The workflow dialog's tasks, in their order: each one's card and its kind of work.
fn flow_rows(page: &Page, cx: &mut TestAppContext) -> Vec<(String, String)> {
    page.look(cx, |w| {
        (0..12usize)
            .filter_map(|n| w.try_find(("flow-task", n)))
            .map(|row| {
                (
                    row.value().map(String::from).unwrap_or_default(),
                    row.description().map(String::from).unwrap_or_default(),
                )
            })
            .collect()
    })
}

/// Shows a step of the workflow dialog's chain at its right: a task by its place, or the start (none). The dialog
/// draws the setup of the step that is picked only (design 2, item 17.4).
fn show_step(page: &Page, cx: &mut TestAppContext, at: Option<usize>) {
    match at {
        Some(at) => press_on(page, cx, ("flow-task", at)),
        None => press_on(page, cx, "flow-start"),
    }
}

/// What a field shows, found by what it is called.
fn field(page: &Page, cx: &mut TestAppContext, label: &str) -> Option<String> {
    use gpui_kit::test::TestQueryExt as _;
    page.look(cx, |w| w.try_find_by_label(label).and_then(|field| field.value().map(String::from)))
}

/// The workflows the window's index holds, the newest first.
fn flows(page: &Page, cx: &mut TestAppContext) -> Vec<enkidu_core::work::flows::Flow> {
    cx.run_until_parked();
    page.work.read_with(cx, |work, _| work.snapshot.as_ref().map(|index| index.flows.clone()).unwrap_or_default())
}

/// Whether the page holds a workflow dialog now.
fn editing(page: &Page, cx: &mut TestAppContext) -> bool {
    cx.run_until_parked();
    page.shell.read_with(cx, |shell, cx| shell.work_page().expect("the Work page").read(cx).flow_editor().is_some())
}

/// "tomorrow at 02:00", as the Mac's own clock has it for the sample's now.
fn starts(clock: &str) -> (i64, String) {
    let at = enkidu_model::work::flows::next_at(clock, NOW).expect("a time of day");
    (at, enkidu_model::work::when::when_words(at, NOW))
}

/// Items 6.1 and 6.8: makes a workflow of the picked cards: each to its Claude or a new workspace, its message to read,
/// at the time you set.
#[gpui_kit::test]
fn makes_a_workflow_of_the_picked_cards_each_to_its_claude_or_a_new_workspace_at_the_time_you_set(
    cx: &mut TestAppContext,
) {
    let page = setup(cx, sample(), vec![ws("w2", "sh-21309-export", "feature/SH-21309-export")], true);
    // every column, and the whole dialog, in view
    cx.simulate_window_resize(page.window, size(px(2400.), px(2000.)));
    pick_cards(&page, cx, &["SH-21309", "SH-21376"]);
    assert_eq!(page.said(cx, "pick-workflow").as_deref(), Some("Workflow…"));
    page.click(cx, "pick-workflow");
    assert_eq!(page.said(cx, "flow-editor").as_deref(), Some("A workflow for these tasks"));
    assert_eq!(
        page.said(cx, "flow-says").as_deref(),
        Some("In shop: one task after another, each with what you tell it here.")
    );
    // you have no workflow yet: nothing to put them into
    assert!(!page.there(cx, "flow-into"));
    assert_eq!(value(&page, cx, "flow-name").as_deref(), Some("Night run"));
    assert_eq!(value(&page, cx, "flow-tasks").as_deref(), Some("2"));
    assert_eq!(page.said(cx, "flow-save"), Some(format!("Start {}", starts("02:00").1)));
    assert_eq!(page.said(cx, "flow-starts"), Some(starts("02:00").1));
    assert!(!page.there(cx, "flow-problem"));
    // a chain at the left, and at the right the step it opens on: the start, with what every task gets. The chain's
    // two ends say when it starts and when it is over at the latest, and the foot says the plan in one line
    assert_eq!(page.said(cx, "flow-step").as_deref(), Some("When it runs, and what every task gets"));
    assert_eq!(page.said(cx, "flow-start"), Some(format!("Starts {}", starts("02:00").1)));
    assert_eq!(tells(&page, cx, "flow-start").as_deref(), Some("Up to 60 min a task. Live follows."));
    assert_eq!(chosen(&page, cx, "flow-start"), Some(true));
    assert_eq!(page.said(cx, "flow-end").as_deref(), Some("Ends by 04:00 at the latest"));
    assert_eq!(tells(&page, cx, "flow-end").as_deref(), Some("Live goes back to the workspace it was on."));
    assert_eq!(page.said(cx, "flow-plan").as_deref(), Some("2 tasks, one after another. At most 2 h, done by 04:00."));
    let (chain, step) = page.look(cx, |w| (w.find("flow-chain").bounds(), w.find("flow-step").bounds()));
    assert!(chain.right() <= step.left() && chain.bottom() == step.bottom(), "{chain:?} beside {step:?}");

    let rows = flow_rows(&page, cx);
    let at = |key: &str| rows.iter().position(|(subject, _)| subject == &format!("ticket:{key}")).expect("its row");
    let (export, units) = (at("SH-21309"), at("SH-21376"));
    assert_eq!(rows.len(), 2);
    assert_eq!(
        says(&page, cx, ("flow-task", export)),
        Some(format!("{}. SH-21309 Export orders to the accounting CSV", export + 1))
    );
    // each task's line in the chain: its kind of work (from its card: SH-21309 has review remarks, SH-21376 an open
    // PR), its minutes and where it runs; and beside it when it starts at the latest
    assert_eq!((rows[export].1.as_str(), rows[units].1.as_str()), ("review", "continue"));
    assert_eq!(says(&page, cx, ("flow-task-line", export)).as_deref(), Some("Answer the review, 60 min, its Claude"));
    assert_eq!(says(&page, cx, ("flow-task-line", units)).as_deref(), Some("Carry on, 60 min, a new workspace"));
    assert_eq!(says(&page, cx, ("flow-task-starts", 0usize)).as_deref(), Some("02:00"));
    assert_eq!(says(&page, cx, ("flow-task-starts", 1usize)).as_deref(), Some("by 03:00"));
    // a task's own setup is at the right once it is picked in the chain: nothing of it before
    assert!(!page.look(cx, |w| w.try_find(("flow-task-kind", export)).is_some()));
    show_step(&page, cx, Some(units));
    assert_eq!(chosen(&page, cx, "flow-start"), Some(false));
    assert_eq!(chosen(&page, cx, ("flow-task", units)), Some(true));
    assert_eq!(page.said(cx, "flow-step-place"), Some(format!("SH-21376, task {} of 2", units + 1)));
    assert_eq!(value_of(&page, cx, ("flow-task-target", units)).as_deref(), Some("new"));
    assert_eq!(says(&page, cx, ("flow-task-kind", units)).as_deref(), Some("Carry on"));
    assert_eq!(says(&page, cx, ("flow-task-waits", units)), None, "nothing waits on SH-21376");
    // each to its Claude, or a new workspace
    show_step(&page, cx, Some(export));
    assert!(!page.look(cx, |w| w.try_find(("flow-task-kind", units)).is_some()), "one step at a time");
    assert_eq!(value_of(&page, cx, ("flow-task-target", export)).as_deref(), Some("workspace:w2"));
    assert_eq!(says(&page, cx, ("flow-task-target", export)).as_deref(), Some("The Claude in sh-21309-export"));
    assert_eq!(tells(&page, cx, ("flow-task-target", export)).as_deref(), Some("Runs in"));
    assert_eq!(says(&page, cx, ("flow-task-kind", export)).as_deref(), Some("Answer the review"));
    let waits = says(&page, cx, ("flow-task-waits", export)).unwrap_or_default();
    assert!(waits.starts_with("With what waits on it: "), "{waits}");
    assert!(field(&page, cx, "What to do on SH-21309").unwrap_or_default().starts_with("Work through the review"));
    // its minutes are the workflow's until it has its own
    assert_eq!(value_of(&page, cx, ("flow-task-minutes", export)).as_deref(), Some(""));

    // its own words, its own minutes, and a new conversation in its workspace instead of the one there
    press_on(&page, cx, ("flow-task-words", export));
    page.press(cx, "cmd-a");
    typed(&page, cx, "Answer mira-k's remark only; leave the question.");
    assert_eq!(
        says(&page, cx, ("flow-task-default", export)).as_deref(),
        Some("Use these words for every “Answer the review”")
    );
    press_on(&page, cx, ("flow-task-minutes", export));
    typed(&page, cx, "3x0");
    assert_eq!(value_of(&page, cx, ("flow-task-minutes", export)).as_deref(), Some("30"), "only digits go in");
    // its line in the chain and the plan follow
    assert_eq!(says(&page, cx, ("flow-task-line", export)).as_deref(), Some("Answer the review, 30 min, its Claude"));
    assert_eq!(
        page.said(cx, "flow-plan").as_deref(),
        Some("2 tasks, one after another. At most 1 h 30 min, done by 03:30.")
    );
    press_on(&page, cx, ("flow-task-target", export));
    let choices = page.look(cx, |w| {
        ["workspace:w2", "conversation:w2", "new"].map(|value| {
            let option = w.find(SharedString::from(format!("flow-option:{value}")));
            (option.label().map(String::from).unwrap_or_default(), option.selected())
        })
    });
    assert_eq!(choices[0], ("The Claude in sh-21309-export".to_string(), Some(true)));
    assert_eq!(choices[1], ("A new conversation in sh-21309-export".to_string(), Some(false)));
    assert!(choices[2].0.starts_with("A new workspace, sh-21309-"), "{choices:?}");
    page.click(cx, "flow-option:conversation:w2");
    assert_eq!(value_of(&page, cx, ("flow-task-target", export)).as_deref(), Some("conversation:w2"));
    assert_eq!(says(&page, cx, ("flow-task-target", export)).as_deref(), Some("A new conversation in sh-21309-export"));
    assert_eq!(
        says(&page, cx, ("flow-task-line", export)).as_deref(),
        Some("Answer the review, 30 min, a new conversation")
    );
    // another kind: its words come with it
    show_step(&page, cx, Some(units));
    press_on(&page, cx, ("flow-task-kind", units));
    page.click(cx, "flow-option:investigate");
    assert_eq!(says(&page, cx, ("flow-task-kind", units)).as_deref(), Some("Investigate only"));
    let words = field(&page, cx, "What to do on SH-21376").unwrap_or_default();
    assert!(words.starts_with("Investigate only"), "{words}");
    assert_eq!(flow_rows(&page, cx)[units].1, "investigate");
    // what every task is told (at the start), and the whole message: yours to read and change
    show_step(&page, cx, None);
    page.click(cx, "flow-instruction");
    typed(&page, cx, "Use the small stack.");
    show_step(&page, cx, Some(units));
    assert_eq!(says(&page, cx, ("flow-task-message-toggle", units)).as_deref(), Some("See the whole message"));
    press_on(&page, cx, ("flow-task-message-toggle", units));
    assert_eq!(says(&page, cx, ("flow-task-message-toggle", units)).as_deref(), Some("Hide the message"));
    let message = field(&page, cx, "The message for SH-21376").unwrap_or_default();
    for said in ["Investigate only", "For every task of this workflow: Use the small stack.", "--ticket SH-21376"] {
        assert!(message.contains(said), "{said:?} is missing from:\n{message}");
    }
    assert!(!page.look(cx, |w| w.try_find(("flow-task-undo", units)).is_some()), "nothing of yours to undo yet");
    press_on(&page, cx, ("flow-task-message", units));
    typed(&page, cx, " Start with the bundle page. ");
    assert_eq!(says(&page, cx, ("flow-task-undo", units)).as_deref(), Some("Undo my edits"));
    // edited, it no longer follows the workflow's words
    show_step(&page, cx, None);
    page.click(cx, "flow-time");
    page.press(cx, "cmd-a");
    typed(&page, cx, "03:30");
    page.click(cx, "flow-minutes");
    page.press(cx, "cmd-a");
    typed(&page, cx, "45");
    assert_eq!(page.said(cx, "flow-save"), Some(format!("Start {}", starts("03:30").1)));
    // the chain's ends and the plan follow the time and the minutes
    assert_eq!(page.said(cx, "flow-start"), Some(format!("Starts {}", starts("03:30").1)));
    assert_eq!(tells(&page, cx, "flow-start").as_deref(), Some("Up to 45 min a task. Live follows."));
    assert_eq!(page.said(cx, "flow-end").as_deref(), Some("Ends by 04:45 at the latest"));
    assert_eq!(
        page.said(cx, "flow-plan").as_deref(),
        Some("2 tasks, one after another. At most 1 h 15 min, done by 04:45.")
    );
    show_step(&page, cx, Some(units));
    press_on(&page, cx, ("flow-task-message-toggle", units));
    assert_eq!(
        says(&page, cx, ("flow-task-message-toggle", units)).as_deref(),
        Some("The whole message (edited)"),
        "closed, it says that it is yours"
    );
    assert!(flows(&page, cx).is_empty(), "nothing is saved before you say so");

    page.click(cx, "flow-save");
    let saved = flows(&page, cx);
    assert_eq!(saved.len(), 1);
    let saved = &saved[0];
    assert_eq!((saved.project_id.as_str(), saved.minutes, saved.live), ("p1", 45, true));
    assert_eq!((saved.name.as_str(), saved.state.as_str()), ("Night run", "scheduled"));
    assert_eq!(saved.instruction, "Use the small stack.");
    let (at_half_past_three, when) = starts("03:30");
    assert_eq!(saved.start_at, Some(at_half_past_three));
    assert_eq!(enkidu_model::work::when::local(at_half_past_three).hour, 3);
    assert!(at_half_past_three > NOW && at_half_past_three - NOW <= 86_400_000);
    let task = |key: &str| saved.tasks.iter().find(|task| task.subject == format!("ticket:{key}")).expect("its task");
    let first = task("SH-21309");
    assert_eq!((first.kind.as_str(), first.minutes, first.edited), ("review", Some(30), false));
    assert_eq!(first.instruction, "Answer mira-k's remark only; leave the question.");
    assert_eq!(first.target, enkidu_core::work::flows::Target::Conversation { workspace_id: "w2".into() });
    for said in ["Answer mira-k's remark only; leave the question.", "about 30 minutes", "mira-k left a remark"] {
        assert!(first.message.contains(said), "{said:?} is missing from:\n{}", first.message);
    }
    let second = task("SH-21376");
    assert_eq!((second.kind.as_str(), second.edited), ("investigate", true));
    assert!(
        matches!(&second.target, enkidu_core::work::flows::Target::New { branch, .. } if branch == "feature/SH-21376-bundle-units"),
        "{:?}",
        second.target
    );
    assert!(second.message.contains("Start with the bundle page."), "{}", second.message);
    assert!(second.message.contains("about 60 minutes"), "an edited message keeps what it said: {}", second.message);
    // saved: the dialog closes, the picks go, and a toast says when it starts
    assert!(!page.there(cx, "flow-editor") && !editing(&page, cx));
    assert!(state(&page, cx).1.is_empty(), "the picks are let go");
    assert!(!page.there(cx, "pick-bar"));
    assert_eq!(page.said(cx, "toast"), Some(format!("Night run starts {when}")));
    // on the board each of its cards says when its workflow starts
    assert_eq!(
        says(&page, cx, SharedString::from("card-flow:ticket:SH-21309")).as_deref(),
        Some("03:30"),
        "the card says its start time"
    );
}

/// Items 6.1 and 6.8: puts picked cards into a workflow you have, and keeps your words for a kind as its default.
#[gpui_kit::test]
fn puts_picked_cards_into_a_workflow_you_have_and_keeps_your_words_for_a_kind_as_its_default(cx: &mut TestAppContext) {
    let mut index = sample();
    index.flows = serde_json::from_value(serde_json::json!([{
        "id": "f2", "projectId": "p1", "name": "Tonight", "instruction": "", "startAt": NOW + 3_600_000,
        "minutes": 60, "live": true, "state": "scheduled",
        "tasks": [{
            "subject": "ticket:SH-21580", "title": "Outlet groups", "message": "SH-21580", "edited": false,
            "kind": "start", "instruction": "Start it.", "state": "waiting",
            "target": { "kind": "new", "name": "sh-21580-outlet", "branch": "feature/SH-21580-outlet", "mode": "new" },
        }],
    }]))
    .expect("a workflow");
    let page = setup(cx, index, Vec::new(), true);
    cx.simulate_window_resize(page.window, size(px(2400.), px(2000.)));
    pick_cards(&page, cx, &["SH-21616"]);
    page.click(cx, "pick-workflow");
    // a new workflow unless you say: the picked card alone
    assert_eq!(page.said(cx, "flow-into").as_deref(), Some("A new workflow"));
    assert_eq!(tells(&page, cx, "flow-into").as_deref(), Some("Put them in"));
    assert_eq!(flow_rows(&page, cx), [("ticket:SH-21616".to_string(), "came_back".to_string())]);
    page.click(cx, "flow-into");
    let tonight = page.said(cx, "flow-option:f2").unwrap_or_default();
    assert!(tonight.starts_with("Tonight: Starts ") && tonight.ends_with(", 1 task"), "{tonight}");
    page.click(cx, "flow-option:f2");
    let rows: Vec<String> = flow_rows(&page, cx).iter().map(|(subject, kind)| format!("{subject} {kind}")).collect();
    assert_eq!(rows, ["ticket:SH-21580 start", "ticket:SH-21616 came_back"]);
    assert_eq!(value(&page, cx, "flow-name").as_deref(), Some("Tonight"), "its name is in the field");
    assert_eq!(value(&page, cx, "flow-into").as_deref(), Some("f2"));
    assert_eq!(chosen(&page, cx, "flow-start"), Some(true), "other tasks now: the chain is shown from its start");
    // the task it had keeps its words and the workspace it was to make
    show_step(&page, cx, Some(0));
    assert_eq!(field(&page, cx, "What to do on SH-21580").as_deref(), Some("Start it."));
    assert_eq!(says(&page, cx, ("flow-task-target", 0usize)).as_deref(), Some("A new workspace, sh-21580-outlet"));
    // your words for "Fix what came back", kept for the next time a ticket comes back
    show_step(&page, cx, Some(1));
    assert!(!page.look(cx, |w| w.try_find(("flow-task-default", 1usize)).is_some()));
    press_on(&page, cx, ("flow-task-words", 1usize));
    page.press(cx, "cmd-a");
    typed(&page, cx, "Read QA's newest comment first, then fix only that.");
    assert_eq!(
        says(&page, cx, ("flow-task-default", 1usize)).as_deref(),
        Some("Use these words for every “Fix what came back”")
    );
    press_on(&page, cx, ("flow-task-default", 1usize));
    let kinds = page.work.read_with(cx, |work, _| work.snapshot.as_ref().map(|index| index.flow_kinds.clone()));
    let kept: Vec<(String, String)> = kinds.unwrap_or_default().into_iter().collect();
    assert_eq!(kept, [("came_back".to_string(), "Read QA's newest comment first, then fix only that.".to_string())]);
    assert_eq!(page.said(cx, "toast").as_deref(), Some("Kept as the default words"));
    assert!(
        !page.look(cx, |w| w.try_find(("flow-task-default", 1usize)).is_some()),
        "they are the kind's words now: nothing more to keep"
    );
    page.click(cx, "flow-save");
    let saved = flows(&page, cx);
    assert_eq!(saved.len(), 1, "the same workflow, one task more");
    assert_eq!(saved[0].id, "f2");
    let subjects: Vec<&str> = saved[0].tasks.iter().map(|task| task.subject.as_str()).collect();
    assert_eq!(subjects, ["ticket:SH-21580", "ticket:SH-21616"]);
    assert_eq!(saved[0].tasks[1].instruction, "Read QA's newest comment first, then fix only that.");
    assert_eq!(saved[0].start_at, Some(NOW + 3_600_000), "at the time it had");
    assert!(!page.there(cx, "flow-editor"));
}

/// Item 6.1: the dialog says what is still missing and saves nothing meanwhile; the tasks' order, taking one out;
/// kept without a time; Cancel and Esc save nothing; Run now starts it now.
#[gpui_kit::test]
fn the_workflow_dialog_says_what_is_missing_orders_its_tasks_and_cancel_saves_nothing(cx: &mut TestAppContext) {
    let page = setup(cx, sample(), Vec::new(), true);
    cx.simulate_window_resize(page.window, size(px(2400.), px(2000.)));
    pick_cards(&page, cx, &["SH-21580", "SH-21376"]);
    page.click(cx, "pick-workflow");
    // the keyboard is in the name as it opens: typing goes there
    page.press(cx, "cmd-a");
    page.press(cx, "backspace");
    assert_eq!(value(&page, cx, "flow-name").as_deref(), Some(""));
    assert_eq!(page.said(cx, "flow-problem").as_deref(), Some("Give it a name."));
    // the buttons do nothing while something is missing
    page.click(cx, "flow-save");
    page.click(cx, "flow-run-now");
    assert!(flows(&page, cx).is_empty() && page.there(cx, "flow-editor"));
    page.click(cx, "flow-name");
    typed(&page, cx, "Before lunch");
    assert!(!page.there(cx, "flow-problem"));
    // a time that is none
    page.click(cx, "flow-time");
    page.press(cx, "cmd-a");
    typed(&page, cx, "25:00");
    assert_eq!(page.said(cx, "flow-problem").as_deref(), Some("Say when it starts, like 02:00."));
    assert_eq!(page.said(cx, "flow-save").as_deref(), Some("Keep it"));
    // kept without a time, it needs none
    assert_eq!(pressed_in(&page, cx, "flow-scheduled"), Some(true));
    page.click(cx, "flow-scheduled");
    assert_eq!(pressed_in(&page, cx, "flow-scheduled"), Some(false));
    assert!(!page.there(cx, "flow-problem"));
    assert_eq!(page.said(cx, "flow-starts").as_deref(), Some("kept until you start it"));
    page.click(cx, "flow-scheduled");
    page.click(cx, "flow-time");
    page.press(cx, "cmd-a");
    typed(&page, cx, "02:00");
    assert!(!page.there(cx, "flow-problem"));
    // minutes a task can't get
    page.click(cx, "flow-minutes");
    page.press(cx, "cmd-a");
    typed(&page, cx, "4");
    assert_eq!(page.said(cx, "flow-problem").as_deref(), Some("A task gets between 5 and 480 minutes."));
    typed(&page, cx, "5");
    assert!(!page.there(cx, "flow-problem"));
    // Live is on unless you turn it off, and every task's message follows it
    assert_eq!(pressed_in(&page, cx, "flow-live"), Some(true));
    page.click(cx, "flow-live");
    assert_eq!(pressed_in(&page, cx, "flow-live"), Some(false));
    assert_eq!(tells(&page, cx, "flow-start").as_deref(), Some("Up to 45 min a task."), "the chain says so too");
    assert_eq!(tells(&page, cx, "flow-end").as_deref(), Some(""));
    show_step(&page, cx, Some(0));
    press_on(&page, cx, ("flow-task-message-toggle", 0usize));
    let first = flow_rows(&page, cx)[0].0.trim_start_matches("ticket:").to_string();
    let message = field(&page, cx, &format!("The message for {first}")).unwrap_or_default();
    assert!(message.contains("run `enkidu live on` first") && message.contains("about 45 minutes"), "{message}");
    assert!(message.contains("workflow \"Before lunch\""), "{message}");

    // the order: the first can't go earlier, the last can't go later
    let order = |page: &Page, cx: &mut TestAppContext| -> Vec<String> {
        flow_rows(page, cx).into_iter().map(|(subject, _)| subject).collect()
    };
    let before = order(&page, cx);
    press_on(&page, cx, ("flow-task-earlier", 0usize));
    show_step(&page, cx, Some(1));
    press_on(&page, cx, ("flow-task-later", 1usize));
    assert_eq!(order(&page, cx), before, "the ends stay where they are");
    show_step(&page, cx, Some(0));
    press_on(&page, cx, ("flow-task-later", 0usize));
    assert_eq!(order(&page, cx), [before[1].clone(), before[0].clone()]);
    // the step shown went with its task, and so did the message you were looking at
    assert_eq!(chosen(&page, cx, ("flow-task", 1usize)), Some(true));
    assert_eq!(says(&page, cx, ("flow-task-message-toggle", 1usize)).as_deref(), Some("Hide the message"));
    show_step(&page, cx, Some(0));
    assert_eq!(says(&page, cx, ("flow-task-message-toggle", 0usize)).as_deref(), Some("See the whole message"));
    show_step(&page, cx, Some(1));
    press_on(&page, cx, ("flow-task-earlier", 1usize));
    assert_eq!(order(&page, cx), before);
    // a task taken out: the one that takes its place is the step shown. The last one too: then there is nothing
    // to run, and the start is shown
    press_on(&page, cx, ("flow-task-remove", 0usize));
    assert_eq!(order(&page, cx), [before[1].clone()]);
    assert_eq!(chosen(&page, cx, ("flow-task", 0usize)), Some(true));
    assert_eq!(page.said(cx, "flow-plan").as_deref(), Some("1 task. At most 45 min, done by 02:45."));
    press_on(&page, cx, ("flow-task-remove", 0usize));
    assert_eq!(chosen(&page, cx, "flow-start"), Some(true));
    assert!(!page.there(cx, "flow-end") && !page.there(cx, "flow-plan"), "nothing to end, and nothing to plan");
    assert_eq!(
        page.said(cx, "flow-problem").as_deref(),
        Some("Pick the tasks on the board first (x, or ⌘-click a card).")
    );

    // Cancel: nothing is saved, and the cards are still picked
    page.click(cx, "flow-cancel");
    assert!(!page.there(cx, "flow-editor") && !editing(&page, cx));
    assert!(flows(&page, cx).is_empty());
    assert_eq!(state(&page, cx).1.len(), 2, "still picked");
    // Esc closes it too, from a field
    page.click(cx, "pick-workflow");
    assert_eq!(value(&page, cx, "flow-name").as_deref(), Some("Night run"), "a new dialog starts over");
    page.press(cx, "escape");
    assert!(!page.there(cx, "flow-editor") && !editing(&page, cx));
    assert!(flows(&page, cx).is_empty());
    // its ×
    page.click(cx, "pick-workflow");
    page.click(cx, "flow-close");
    assert!(!page.there(cx, "flow-editor") && !editing(&page, cx));

    // Run now: saved to start now, whatever its time says
    page.click(cx, "pick-workflow");
    page.click(cx, "flow-run-now");
    let saved = flows(&page, cx);
    assert_eq!(saved.len(), 1);
    assert_eq!((saved[0].start_at, saved[0].state.as_str()), (Some(NOW), "scheduled"));
    assert_eq!(saved[0].tasks.len(), 2);
    assert!(!page.there(cx, "flow-editor"));
    assert_eq!(page.said(cx, "toast").as_deref(), Some("Night run starts now"));
    assert!(state(&page, cx).1.is_empty(), "the picks are let go");
}

const MIN: i64 = 60_000;

/// A task of a workflow: to the Claude in w2, titled by its key, with what happened to it (when it started, and when it
/// ended).
fn flow_task(key: &str, state: &str, kind: &str, ran: Option<(i64, Option<i64>)>) -> serde_json::Value {
    let mut task = serde_json::json!({
        "subject": format!("ticket:{key}"), "title": key, "message": format!("{key}: the message it got"),
        "edited": false, "kind": kind, "instruction": "Fix what QA found.", "state": state,
        "target": { "kind": "workspace", "workspaceId": "w2" },
    });
    if let Some((started, ended)) = ran {
        task["startedAt"] = serde_json::json!(started);
        task["endedAt"] = serde_json::json!(ended);
        task["workspaceId"] = serde_json::json!("w2");
    }
    task
}

fn a_flow(
    id: &str,
    name: &str,
    state: &str,
    start_at: Option<i64>,
    tasks: Vec<serde_json::Value>,
) -> serde_json::Value {
    serde_json::json!({
        "id": id, "projectId": "p1", "name": name, "instruction": "", "startAt": start_at, "minutes": 60,
        "live": true, "state": state, "tasks": tasks,
    })
}

fn a_report(id: i64, key: &str, state: &str, text: &str, images: &[&str], at: i64) -> serde_json::Value {
    serde_json::json!({
        "id": id, "subject": key, "sessionId": "s-w2", "workspaceId": "w2", "state": state, "text": text,
        "images": images, "at": at,
    })
}

/// The tasks of the workflow or run shown, in their order: each one's state, and whether it is the one picked.
fn flow_lines(page: &Page, cx: &mut TestAppContext) -> Vec<String> {
    page.look(cx, |w| {
        (0..16usize)
            .filter_map(|n| w.try_find(("flow-line", n)))
            .map(|line| format!("{} {}", line.value().unwrap_or_default(), line.selected() == Some(true)))
            .collect()
    })
}

/// What happened to the task shown, in order: each step's kind.
fn run_steps(page: &Page, cx: &mut TestAppContext) -> Vec<String> {
    page.look(cx, |w| {
        (0..16usize)
            .filter_map(|n| w.try_find(("run-task-step", n)))
            .map(|step| step.value().map(String::from).unwrap_or_default())
            .collect()
    })
}

/// Opens the menu of the Workflows page's name: your workflows still to run, and every run kept (design 2, item
/// 17.5: no pane of their own).
fn open_runs(page: &Page, cx: &mut TestAppContext) {
    page.click(cx, "flows-switch");
    assert!(page.look(cx, |w| w.try_find(("flow-item", 0usize)).or(w.try_find(("flow-run-item", 0usize))).is_some()));
}

/// Closes that menu with Esc, picking nothing: the page behind it stays.
fn close_runs(page: &Page, cx: &mut TestAppContext) {
    page.press(cx, "escape");
    assert!(!page.look(cx, |w| w.try_find(("flow-item", 0usize)).or(w.try_find(("flow-run-item", 0usize))).is_some()));
    assert!(page.there(cx, "flows-page"), "Esc closed the menu, not the page");
}

/// What that menu lists: the workflows still to run (each one's id and state), and the runs kept (each one's id,
/// lamp and state). It opens the menu, reads it and closes it.
fn flows_nav(page: &Page, cx: &mut TestAppContext) -> (Vec<String>, Vec<String>) {
    open_runs(page, cx);
    let listed = page.look(cx, |w| {
        let listed = |id: &'static str| -> Vec<String> {
            (0..16usize)
                .filter_map(|n| w.try_find((id, n)))
                .map(|item| format!("{} {}", item.description().unwrap_or_default(), item.value().unwrap_or_default()))
                .collect()
        };
        (listed("flow-item"), listed("flow-run-item"))
    });
    close_runs(page, cx);
    listed
}

/// Whether the Workflows page stands in the board's place, and what you picked in its name's menu.
fn flows_state(page: &Page, cx: &mut TestAppContext) -> (bool, Option<String>) {
    cx.run_until_parked();
    page.shell.read_with(cx, |shell, cx| {
        let view = shell.work_page().expect("the Work page").read(cx);
        (view.flows_open(), view.flows().read(cx).selected().map(String::from))
    })
}

fn clock(at: i64) -> String {
    enkidu_model::work::when::clock_of(at)
}

/// Items 6.3 to 6.5 and 6.8: shows a workflow in three panes: its tasks at a glance, and the one picked across the
/// page, in the order you read it. Since item 17.5 the page is one list and the task: the workflows are the menu of the
/// name, no pane of their own.
#[gpui_kit::test]
fn shows_a_workflow_as_one_list_and_the_task_its_tasks_at_a_glance_and_the_one_picked_across_the_page(
    cx: &mut TestAppContext,
) {
    let started = NOW - 20 * MIN;
    let mut index = sample();
    let mut running = flow_task("SH-21309", "running", "came_back", Some((started, None)));
    running["minutes"] = serde_json::json!(45);
    let mut night = a_flow(
        "f1",
        "Night run",
        "running",
        Some(NOW - 60 * MIN),
        vec![
            flow_task("SH-21533", "done", "came_back", Some((NOW - 50 * MIN, Some(started)))),
            running,
            flow_task("SH-21376", "waiting", "came_back", None),
        ],
    );
    night["current"] = serde_json::json!(1);
    index.flows = serde_json::from_value(serde_json::json!([night])).expect("a workflow");
    index.reports = serde_json::from_value(serde_json::json!([
        a_report(3, "SH-21309", "update", "Found it: the separator.", &["1-half.png"], started + 5 * MIN),
        a_report(2, "SH-21533", "done", "Fixed the double count.\n\nThe tests pass.", &[], started - MIN),
    ]))
    .expect("reports");
    let page = setup(cx, index, vec![ws("w2", "sh-21309-export", "feature/SH-21309-export")], true);
    cx.simulate_window_resize(page.window, size(px(1728.), px(1100.)));
    let asked = give_screenshots(cx);
    assert_eq!(page.said(cx, "work-flows").as_deref(), Some("Workflows"));
    assert_eq!(pressed_in(&page, cx, "work-flows"), Some(false));
    page.click(cx, "work-flows");
    // a whole page in place of the board
    assert!(page.there(cx, "flows-page") && !page.there(cx, "work-board"));
    assert_eq!(pressed_in(&page, cx, "work-flows"), Some(true));
    assert_eq!(flows_state(&page, cx), (true, None));
    // one list and the task: 348 px, and the rest. No list of workflows beside it ("this double left banner")
    assert!(!page.there(cx, "flows-nav"));
    let (detail, task, body) = page.look(cx, |w| {
        let at = |id: &'static str| w.find(id).bounds();
        (at("flow-detail"), at("flow-task-pane"), at("flows-page"))
    });
    assert_eq!(detail.size.width, px(348.));
    assert_eq!((detail.left(), task.left()), (px(8.), px(364.)));
    assert_eq!(task.right(), body.right());
    assert_eq!((detail.top(), detail.bottom()), (body.top(), body.bottom() - px(8.)));
    // your workflows are the menu of its name: closed until asked for; the one that runs, checked as the one shown
    assert_eq!(page.said(cx, "flows-switch").as_deref(), Some("Night run"));
    assert!(!page.look(cx, |w| w.try_find(("flow-item", 0usize)).is_some()), "the menu is closed");
    assert_eq!(flows_nav(&page, cx), (vec!["f1 running".to_string()], Vec::new()));
    open_runs(&page, cx);
    assert_eq!(says(&page, cx, ("flow-item", 0usize)).as_deref(), Some("Night run: Running 2 of 3: SH-21309"));
    assert_eq!(chosen(&page, cx, ("flow-item", 0usize)), Some(true), "the one shown");
    close_runs(&page, cx);
    // it still runs: its chain has no ends yet (its head says when it started)
    assert!(!page.there(cx, "flow-started") && !page.there(cx, "flow-ended"));
    // the left: how it goes, and all its tasks at a glance, each with its state
    assert_eq!(value(&page, cx, "flow-detail").as_deref(), Some("flow:f1"));
    assert_eq!(page.said(cx, "flow-detail").as_deref(), Some("Night run"));
    assert_eq!(page.said(cx, "flow-when"), Some(format!("Started {}", clock(NOW - 50 * MIN))));
    assert_eq!(page.said(cx, "flow-words").as_deref(), Some("Running 2 of 3: SH-21309"));
    assert_eq!(page.said(cx, "flow-bar").as_deref(), Some("1 of 3 tasks over"));
    assert_eq!(page.said(cx, "flow-task-count").as_deref(), Some("3 tasks"));
    assert_eq!(flow_lines(&page, cx), ["done false", "running true", "waiting false"], "the one that runs is picked");
    assert_eq!(says(&page, cx, ("flow-line", 1usize)).as_deref(), Some("2 SH-21309 Running 20 min"));
    assert_eq!(tells(&page, cx, ("flow-line", 0usize)).as_deref(), Some("Fix what came back · 1 note"));
    // it runs: Stop and delete, nothing that starts or changes it
    assert_eq!(page.said(cx, "flow-stop").as_deref(), Some("Stop"));
    assert!(page.there(cx, "flow-delete") && !page.there(cx, "flow-run") && !page.there(cx, "flow-edit"));
    // the right: the task, how it stands (the colored box), what it was given, what happened
    assert_eq!(value(&page, cx, "run-task").as_deref(), Some("ticket:SH-21309"));
    assert_eq!(page.said(cx, "run-task-place").as_deref(), Some("Task 2 of 3"));
    assert_eq!(
        page.said(cx, "run-task-facts"),
        Some(format!("Fix what came back · sh-21309-export · {} · 20 min of 45 min · Live", clock(started)))
    );
    assert_eq!(
        page.said(cx, "run-task-outcome"),
        Some(format!("Running for 20 min of 45 minIn sh-21309-export since {}, Live.", clock(started)))
    );
    assert_eq!(value(&page, cx, "run-task-outcome").as_deref(), Some("running"));
    assert_eq!(page.said(cx, "run-task-message").as_deref(), Some("The message we sent"));
    assert_eq!(described(&page, cx, "run-task-message"), Some(clock(started)));
    assert_eq!(value(&page, cx, "run-task-message").as_deref(), Some("SH-21309: the message it got"));
    let ticket = page.said(cx, "run-ticket-doc").unwrap_or_default();
    assert!(ticket.contains("Accounting imports the orders of a month as a CSV."), "{ticket}");
    assert_eq!(page.said(cx, "run-task-jira").as_deref(), Some("Open in Jira"));
    assert_eq!(run_steps(&page, cx), ["sent", "update"]);
    assert_eq!(
        says(&page, cx, ("run-task-step", 0usize)).as_deref(),
        Some("Sent the message to the Claude in sh-21309-export; Live followed it")
    );
    assert_eq!(tells(&page, cx, ("run-task-step", 0usize)), Some(clock(started)));
    assert_eq!(page.said(cx, "report-text:3").as_deref(), Some("Found it: the separator."));
    // its screenshot, read once, large: one alone as wide as its report, 880 px at most
    assert_eq!(described(&page, cx, "report-image:3:1-half.png").as_deref(), Some("shown"));
    assert_eq!(asked.lock().unwrap().clone(), [(3, "1-half.png".to_string())]);
    let shot = page.look(cx, |w| w.find("report-image:3:1-half.png").bounds());
    assert!(shot.size.width > px(600.) && shot.size.width <= px(880.), "{shot:?}");
    assert_eq!(page.said(cx, "run-task-workspace").as_deref(), Some("Open sh-21309-export"));
    // what it was given, side by side in a window this wide: the message we sent, and the Jira ticket
    let (message, ticket) =
        page.look(cx, |w| (w.find("run-task-message").bounds(), w.find("run-task-ticket").bounds()));
    assert_eq!(message.top(), ticket.top());
    assert!(message.right() < ticket.left(), "{message:?} {ticket:?}");
    assert!((message.size.width - ticket.size.width).abs() < px(1.), "each takes half");
    // a short message shows whole; the long description shows its start, with words under it for the rest
    assert!(!page.there(cx, "run-task-message-more"));
    assert_eq!(page.said(cx, "run-task-ticket-more").as_deref(), Some("Show the whole description"));
    let folded = page.look(cx, |w| w.find("run-task-ticket").bounds().size.height);
    page.click(cx, "run-task-ticket-more");
    assert_eq!(page.said(cx, "run-task-ticket-more").as_deref(), Some("Show less"));
    let whole = page.look(cx, |w| w.find("run-task-ticket").bounds().size.height);
    assert!(whole > folded + px(100.), "{folded:?} then {whole:?}");

    // another task, picked: how it ended, Claude's own first words up top; then its report
    press_on(&page, cx, ("flow-line", 0usize));
    assert_eq!(value(&page, cx, "run-task").as_deref(), Some("ticket:SH-21533"));
    assert_eq!(page.said(cx, "run-task-outcome").as_deref(), Some("Done, nothing needs you“Fixed the double count.”"));
    assert_eq!(run_steps(&page, cx), ["sent", "done"]);
    assert!(!page.there(cx, "report-image:3:1-half.png"), "the other task's screenshot went with it");
    // ↓ goes through the tasks: the one waiting says where and after which it runs, and the message it will get
    page.press(cx, "down");
    assert_eq!(value(&page, cx, "run-task").as_deref(), Some("ticket:SH-21309"));
    page.press(cx, "down");
    assert_eq!(value(&page, cx, "run-task").as_deref(), Some("ticket:SH-21376"));
    assert_eq!(
        page.said(cx, "run-task-outcome").as_deref(),
        Some("Waiting its turnTask 3 of 3, after SH-21309: the Claude in sh-21309-export, up to 60 minutes.")
    );
    assert_eq!(page.said(cx, "run-task-message").as_deref(), Some("The message it will get"));
    assert!(!page.there(cx, "run-task-happened"), "nothing happened yet");
    assert_eq!(flow_lines(&page, cx), ["done false", "running false", "waiting true"]);
    // a card opens on top of the page; Esc closes the card, the next Esc goes back to the board
    press_on(&page, cx, ("flow-line", 0usize));
    page.click(cx, "run-task-card");
    assert_eq!(state(&page, cx).2.as_deref(), Some("ticket:SH-21533"));
    assert!(page.there(cx, "task-dialog"));
    assert!(flows_state(&page, cx).0, "the Workflows page is still there behind it");
    page.press(cx, "escape");
    assert_eq!(state(&page, cx).2, None, "Esc closes the card");
    assert!(page.there(cx, "flows-page") && !page.there(cx, "task-dialog"));
    page.press(cx, "escape");
    assert!(!page.there(cx, "flows-page") && page.there(cx, "work-board"), "the next Esc goes back to the board");
    assert!(!flows_state(&page, cx).0);
    assert!(page.open(cx), "the Work page stays");
    // the keyboard is back on the board: its keys work at once
    page.press(cx, "down");
    assert!(state(&page, cx).0.is_some(), "a card has the keyboard");
}

/// Items 6.3, 6.4 and 6.8: keeps every run: listed under the workflows still to run, the newest first, each opening on
/// the task that needs you.
#[gpui_kit::test]
fn keeps_every_run_listed_under_the_workflows_still_to_run_each_opening_on_the_task_that_needs_you(
    cx: &mut TestAppContext,
) {
    let at = NOW - 26 * 60 * MIN;
    let before = at - 24 * 60 * MIN;
    let start = |key: &str, state: &str, ran: Option<(i64, Option<i64>)>| {
        let mut task = flow_task(key, state, "start", ran);
        task["instruction"] = serde_json::json!("");
        task["message"] = serde_json::json!(key);
        task
    };
    let mut stuck = start("SH-21522", "blocked", Some((at + 50 * MIN, Some(at + 90 * MIN))));
    stuck["nudgedAt"] = serde_json::json!(at + 80 * MIN);
    let night = a_flow(
        "f1",
        "Night run",
        "done",
        Some(at),
        vec![
            start("SH-21376", "done", Some((at, Some(at + 30 * MIN)))),
            start("SH-21309", "review", Some((at + 30 * MIN, Some(at + 50 * MIN)))),
            stuck,
        ],
    );
    let tonight = a_flow("f2", "Tonight", "scheduled", Some(NOW + 60 * MIN), vec![start("SH-21376", "waiting", None)]);
    let earlier = a_flow(
        "f1",
        "Night run",
        "stopped",
        Some(at),
        vec![
            start("SH-21309", "done", Some((before, Some(before + 30 * MIN)))),
            start("SH-21376", "stopped", Some((before + 30 * MIN, Some(before + 40 * MIN)))),
        ],
    );
    let mut index = sample();
    index.flows = serde_json::from_value(serde_json::json!([night, tonight])).expect("workflows");
    index.flow_runs = serde_json::from_value(serde_json::json!([
        { "id": 7, "flowId": "f1", "startedAt": at, "endedAt": at + 90 * MIN, "state": "done", "flow": night },
        { "id": 5, "flowId": "f1", "startedAt": before, "endedAt": before + 40 * MIN, "state": "stopped", "flow": earlier },
    ]))
    .expect("runs");
    index.reports = serde_json::from_value(serde_json::json!([
        a_report(
            4,
            "SH-21309",
            "review",
            "Added the export; the 7 s is worth your look.",
            &["1-list.png", "2-excel.png"],
            at + 49 * MIN
        ),
        a_report(3, "SH-21309", "done", "The first half of the export.", &[], before + 29 * MIN),
    ]))
    .expect("reports");
    let page = setup(cx, index, Vec::new(), true);
    cx.simulate_window_resize(page.window, size(px(1728.), px(1100.)));
    give_screenshots(cx);
    page.click(cx, "work-flows");
    // still to run first, then every run that ended: the done workflow is there as its runs
    let (coming, runs) = flows_nav(&page, cx);
    assert_eq!(coming, ["f2 scheduled"]);
    assert_eq!(runs, ["7 red done", "5 gray stopped"]);
    let day = enkidu_model::work::when::day_words(at, NOW);
    assert!(day.starts_with("Yesterday, "), "{day}");
    open_runs(&page, cx);
    assert_eq!(
        says(&page, cx, ("flow-run-item", 0usize)),
        Some(format!("Night run, {day}: 1 done, 1 wants your review, 1 stuck"))
    );
    assert_eq!(chosen(&page, cx, ("flow-run-item", 0usize)), Some(true), "the run shown has the check");
    assert_eq!(chosen(&page, cx, ("flow-item", 0usize)), Some(false));
    close_runs(&page, cx);
    // nothing runs: the newest run is shown, its 3 tasks, and it opens on the first that needs you
    assert_eq!(value(&page, cx, "flow-detail").as_deref(), Some("run:7"));
    assert_eq!(page.said(cx, "flow-when"), Some(format!("{day} · took 1 h 30 min")));
    assert_eq!(page.said(cx, "flow-words").as_deref(), Some("Finished: 1 done, 1 wants your review, 1 stuck"));
    assert_eq!(value(&page, cx, "flow-words").as_deref(), Some("red"), "its lamp");
    assert_eq!(page.said(cx, "flow-bar").as_deref(), Some("3 of 3 tasks over"));
    assert_eq!(page.said(cx, "flow-task-count").as_deref(), Some("3 tasks"));
    assert_eq!(flow_lines(&page, cx), ["done false", "review true", "blocked false"]);
    // its chain has its two ends: when it started, and when it ended; its tasks stand between them
    assert_eq!(page.said(cx, "flow-started"), Some(format!("{}, started", clock(at))));
    assert_eq!(page.said(cx, "flow-ended"), Some(format!("{}, ended", clock(at + 90 * MIN))));
    let chain = page.look(cx, |w| {
        ["flow-started", "flow-ended"]
            .map(|id| w.find(id).bounds())
            .into_iter()
            .chain([0usize, 2].map(|n| w.find(("flow-line", n)).bounds()))
            .collect::<Vec<_>>()
    });
    assert!(chain[0].bottom() <= chain[2].top() && chain[3].bottom() <= chain[1].top(), "{chain:?}");
    assert_eq!(
        page.said(cx, "run-task-outcome").as_deref(),
        Some("Done, wants your review“Added the export; the 7 s is worth your look.”")
    );
    let shots =
        page.look(cx, |w| ["report-image:4:1-list.png", "report-image:4:2-excel.png"].map(|id| w.find(id).bounds()));
    // two screenshots side by side in a window this wide, each large
    assert_eq!(shots[0].top(), shots[1].top());
    assert!(shots[0].size.width > px(380.) && shots[0].right() < shots[1].left(), "{shots:?}");
    // its workspace is no longer one of yours: nothing to open
    assert!(!page.there(cx, "run-task-workspace"));
    // a click shows a screenshot as large as the window, over the page; → goes on, Esc closes it, not the page
    page.click(cx, "report-image:4:1-list.png");
    assert_eq!(page.said(cx, "report-lightbox").as_deref(), Some("1-list.png"));
    assert_eq!(described(&page, cx, "report-lightbox").as_deref(), Some("1 of 2"));
    let (large, window) = page.look(cx, |w| (w.find("report-lightbox").bounds(), w.viewport_size()));
    assert_eq!((large.size.width, large.size.height), (window.width, window.height));
    page.press(cx, "right");
    assert_eq!(page.said(cx, "report-lightbox").as_deref(), Some("2-excel.png"));
    // the page's own keys rest under it: ↓ picks no other task
    page.press(cx, "down");
    assert_eq!(flow_lines(&page, cx), ["done false", "review true", "blocked false"]);
    page.press(cx, "escape");
    assert!(!page.there(cx, "report-lightbox") && page.there(cx, "flows-page"), "Esc closed the screenshot only");
    // and the keyboard is back on the page
    page.press(cx, "down");
    assert_eq!(flow_lines(&page, cx), ["done false", "review false", "blocked true"]);
    page.press(cx, "up");
    // the stuck one: Enkidu asked for its report, and how it ended
    press_on(&page, cx, ("flow-line", 2usize));
    assert_eq!(run_steps(&page, cx), ["sent", "asked", "ended"]);
    assert_eq!(value(&page, cx, "run-task-outcome").as_deref(), Some("blocked"));
    assert_eq!(
        says(&page, cx, ("run-task-step", 1usize)).as_deref(),
        Some("Claude stopped without saying how it went: Enkidu asked for its report")
    );
    assert_eq!(says(&page, cx, ("run-task-step", 2usize)).as_deref(), Some("Stuck"));
    // the run before, a click away in the name's menu, with its own report (not the newer one)
    open_runs(&page, cx);
    press_on(&page, cx, ("flow-run-item", 1usize));
    assert!(!page.look(cx, |w| w.try_find(("flow-run-item", 1usize)).is_some()), "picked: the menu closed");
    assert_eq!(value(&page, cx, "flow-detail").as_deref(), Some("run:5"));
    assert_eq!(page.said(cx, "flow-ended"), Some(format!("{}, stopped", clock(before + 40 * MIN))));
    assert_eq!(flows_state(&page, cx).1.as_deref(), Some("run:5"));
    assert_eq!(page.said(cx, "flow-words").as_deref(), Some("Stopped: 1 done, 1 stopped"));
    assert_eq!(value(&page, cx, "run-task").as_deref(), Some("ticket:SH-21309"));
    assert_eq!(page.said(cx, "report-text:3").as_deref(), Some("The first half of the export."));
    assert!(!page.there(cx, "report-text:4"), "the newer report is the other run's");
    // run it again: the same workflow starts, and the page follows it
    assert_eq!(page.said(cx, "flow-run-again").as_deref(), Some("Run it again"));
    page.click(cx, "flow-run-again");
    assert_eq!(flows_state(&page, cx).1.as_deref(), Some("flow:f1"));
    let again = flows(&page, cx).into_iter().find(|flow| flow.id == "f1").expect("the workflow");
    assert_eq!((again.start_at, again.state.as_str()), (Some(NOW), "scheduled"));
    assert!(
        again.tasks.iter().all(|task| task.state == "waiting" && task.started_at.is_none()),
        "its tasks start over"
    );
    assert_eq!(value(&page, cx, "flow-detail").as_deref(), Some("flow:f1"), "the page shows it");
    assert!(!page.there(cx, "flow-started") && !page.there(cx, "flow-ended"), "it waits: nothing happened yet");
    assert_eq!(flows_nav(&page, cx).0, ["f1 scheduled", "f2 scheduled"], "the one due first comes first");
    // every task waits again, and says what it will get
    assert_eq!(flow_lines(&page, cx), ["waiting true", "waiting false", "waiting false"]);
    assert_eq!(page.said(cx, "run-task-message").as_deref(), Some("The message it will get"));
}

/// Items 6.6 and 6.8: shows a run the usage limit stopped: it opens on the task that was cut off, in Claude Code's
/// words; the rest never ran.
#[gpui_kit::test]
fn shows_a_run_the_usage_limit_stopped_it_opens_on_the_task_that_was_cut_off_the_rest_never_ran(
    cx: &mut TestAppContext,
) {
    let at = NOW - 6 * 60 * MIN;
    let start = |key: &str, state: &str, ran: Option<(i64, Option<i64>)>| {
        let mut task = flow_task(key, state, "start", ran);
        task["message"] = serde_json::json!(format!("{key}: start this ticket."));
        task
    };
    let mut cut = start("SH-21309", "cut_off", Some((at + 20 * MIN, Some(at + 31 * MIN))));
    cut["note"] = serde_json::json!("You've hit your limit · resets 3am (Europe/Paris)");
    let night = a_flow(
        "f1",
        "Night run",
        "stopped",
        Some(at),
        vec![start("SH-21376", "done", Some((at, Some(at + 20 * MIN)))), cut, start("SH-21522", "waiting", None)],
    );
    let mut index = sample();
    index.flows = serde_json::from_value(serde_json::json!([night])).expect("a workflow");
    index.flow_runs = serde_json::from_value(serde_json::json!([
        { "id": 9, "flowId": "f1", "startedAt": at, "endedAt": at + 31 * MIN, "state": "stopped", "flow": night },
    ]))
    .expect("a run");
    let page = setup(cx, index, Vec::new(), true);
    page.click(cx, "work-flows");
    assert_eq!(flows_nav(&page, cx), (Vec::new(), vec!["9 red stopped".to_string()]));
    open_runs(&page, cx);
    let item = says(&page, cx, ("flow-run-item", 0usize)).unwrap_or_default();
    close_runs(&page, cx);
    assert!(item.ends_with(": 1 done, 1 cut off"), "{item}");
    assert_eq!(page.said(cx, "flow-words").as_deref(), Some("Stopped: 1 done, 1 cut off"));
    // it opens on the task that needs you: cut off, with what Claude Code said
    assert_eq!(flow_lines(&page, cx), ["done false", "cut_off true", "stopped false"]);
    assert_eq!(says(&page, cx, ("flow-line", 1usize)).as_deref(), Some("2 SH-21309 Cut off 11 min"));
    assert_eq!(value(&page, cx, "run-task-outcome").as_deref(), Some("cut_off"));
    assert_eq!(
        page.said(cx, "run-task-outcome").as_deref(),
        Some("Claude was cut offYou've hit your limit · resets 3am (Europe/Paris)")
    );
    // the task after it never got its turn: it says so, not "waiting its turn"
    press_on(&page, cx, ("flow-line", 2usize));
    assert_eq!(page.said(cx, "run-task-outcome").as_deref(), Some("StoppedThe run stopped before its turn"));
    assert_eq!(page.said(cx, "run-task-message").as_deref(), Some("The message it would have got"));
    assert_eq!(value(&page, cx, "run-task-message").as_deref(), Some("SH-21522: start this ticket."));
    assert!(!page.there(cx, "run-task-happened"));
}

/// Item 6.2: "Workflows" in the header: there once you made a workflow, with its running line while one runs;
/// pressed in while its page stands in the board's place; a filter of the board brings the board back.
#[gpui_kit::test]
fn workflows_in_the_header_opens_its_page_and_a_board_filter_brings_the_board_back(cx: &mut TestAppContext) {
    // you never made one: nothing about workflows in the header
    let page = setup(cx, sample(), Vec::new(), true);
    assert!(!page.there(cx, "work-flows"));
    // one kept, not running
    let mut index = sample();
    let kept = a_flow(
        "f1",
        "Tonight",
        "scheduled",
        Some(NOW + 60 * MIN),
        vec![flow_task("SH-21376", "waiting", "start", None)],
    );
    index.flows = serde_json::from_value(serde_json::json!([kept])).expect("a workflow");
    let page = setup(cx, index.clone(), Vec::new(), true);
    cx.simulate_window_resize(page.window, size(px(2400.), px(900.)));
    assert_eq!(page.said(cx, "work-flows").as_deref(), Some("Workflows"));
    assert_eq!(described(&page, cx, "work-flows").as_deref(), Some(""));
    assert!(!page.there(cx, "work-flows-running"));
    // it stands after Needs you, in the row
    let (needs, chip, filter) = page.look(cx, |w| {
        let at = |id: &'static str| w.find(id).bounds();
        (at("work-needs-you"), at("work-flows"), at("work-filter"))
    });
    assert!(needs.right() <= chip.left() && chip.right() < filter.left(), "{needs:?} {chip:?} {filter:?}");
    assert_eq!(chip.size.height, px(26.));
    // pointed at, it says what it is for
    cx.update_window(page.window, |_, w, cx| w.hover("work-flows", cx)).unwrap();
    cx.executor().advance_clock(std::time::Duration::from_millis(900));
    assert_eq!(page.said(cx, "tip").as_deref(), Some("Your workflows: what is due, and how each task ended"));
    // the status line's keys are the board's
    let keys = page.said(cx, "work-keys").unwrap_or_default();
    assert!(keys.starts_with("←→↑↓ move"), "{keys}");

    // picked cards wait while its page is open: the bar comes back with the board
    pick_cards(&page, cx, &["SH-21580"]);
    assert!(page.there(cx, "pick-bar"));
    page.click(cx, "work-flows");
    assert!(page.there(cx, "flows-page") && !page.there(cx, "work-board") && !page.there(cx, "pick-bar"));
    assert_eq!(pressed_in(&page, cx, "work-flows"), Some(true));
    assert_eq!(page.said(cx, "work-keys").as_deref(), Some("Esc back to the board"));
    // the board's keys rest while it is away: nothing is picked, opened or moved behind the page
    let before = state(&page, cx);
    for key in ["right", "x", "enter", "l", "h", "/"] {
        page.press(cx, key);
    }
    assert_eq!(state(&page, cx), before);
    assert!(page.there(cx, "flows-page") && !page.there(cx, "task-dialog"));
    // the chip again: the board is back, with what was picked
    page.click(cx, "work-flows");
    assert!(!page.there(cx, "flows-page") && page.there(cx, "work-board") && page.there(cx, "pick-bar"));
    assert_eq!(pressed_in(&page, cx, "work-flows"), Some(false));
    // Needs you is a filter of the board: it brings the board back
    page.click(cx, "work-flows");
    assert!(page.there(cx, "flows-page"));
    page.click(cx, "work-needs-you");
    assert!(!page.there(cx, "flows-page") && page.there(cx, "work-board"));
    assert_eq!(pressed_in(&page, cx, "work-needs-you"), Some(true));
    page.click(cx, "work-needs-you");
    // so does a word typed into the filter, and the keyboard stays in the field
    page.click(cx, "work-flows");
    page.click(cx, "work-query");
    typed(&page, cx, "e");
    assert!(!page.there(cx, "flows-page") && page.there(cx, "work-board"));
    typed(&page, cx, "xport");
    assert_eq!(shown(&page, cx), ["SH-21309"], "the board, filtered by what was typed");

    // while one runs: the chip says so, with the line of light along its lower edge
    index.flows[0].state = "running".into();
    index.flows[0].tasks[0].state = "running".into();
    index.flows[0].tasks[0].started_at = Some(NOW - 5 * MIN);
    index.flows[0].current = Some(0);
    let page = setup(cx, index, Vec::new(), true);
    assert_eq!(described(&page, cx, "work-flows").as_deref(), Some("a workflow is running"));
    assert_eq!(page.said(cx, "work-flows-running").as_deref(), Some("A workflow is running"));
    let (chip, line) = page.look(cx, |w| (w.find("work-flows").bounds(), w.find("work-flows-running").bounds()));
    assert_eq!((line.left(), line.right(), line.size.height), (chip.left(), chip.right(), px(1.)));
    assert_eq!(line.bottom(), chip.bottom());
    cx.update_window(page.window, |_, w, cx| w.hover("work-flows", cx)).unwrap();
    cx.executor().advance_clock(std::time::Duration::from_millis(900));
    assert_eq!(page.said(cx, "tip").as_deref(), Some("A workflow is running: see how its tasks stand"));
    // a card of it says "Now" on the board
    assert_eq!(says(&page, cx, SharedString::from("card-flow:ticket:SH-21376")).as_deref(), Some("Now"));
}

/// Item 17.5: every run is kept, so the menu of the page's name grows by one each night. A long one is no taller
/// than its 480 px: it scrolls under the wheel, and a run far down is reached and picked.
#[gpui_kit::test]
fn a_long_menu_of_runs_scrolls_to_the_older_ones(cx: &mut TestAppContext) {
    let day = 24 * 60 * MIN;
    let first = NOW - 32 * day;
    let runs: Vec<serde_json::Value> = (0..30i64)
        .map(|n| {
            // the newest first, as the core keeps them
            let at = first + (29 - n) * day;
            let task = flow_task("SH-21376", "done", "start", Some((at, Some(at + 30 * MIN))));
            let night = a_flow("f1", "Night run", "done", Some(at), vec![task]);
            serde_json::json!({
                "id": 100 - n, "flowId": "f1", "startedAt": at, "endedAt": at + 30 * MIN, "state": "done", "flow": night,
            })
        })
        .collect();
    let mut index = sample();
    index.flow_runs = serde_json::from_value(serde_json::json!(runs)).expect("runs");
    let page = setup(cx, index, Vec::new(), true);
    cx.simulate_window_resize(page.window, size(px(1728.), px(1100.)));
    page.click(cx, "work-flows");
    assert_eq!(value(&page, cx, "flow-detail").as_deref(), Some("run:100"), "the newest run is shown");
    open_runs(&page, cx);
    let (newest, oldest) =
        page.look(cx, |w| (w.find(("flow-run-item", 0usize)).bounds(), w.find(("flow-run-item", 29usize)).bounds()));
    assert!(oldest.top() - newest.top() > px(1000.), "thirty runs are longer than the menu: {newest:?} {oldest:?}");
    // the wheel, with the pointer in the menu: the list moves under it, the page behind it doesn't
    let before = page.look(cx, |w| w.find(("flow-line", 0usize)).bounds());
    cx.update_window(page.window, |_, w, cx| {
        let position = newest.center();
        let modifiers = Modifiers::default();
        w.dispatch_event(MouseMoveEvent { position, pressed_button: None, modifiers }.to_platform_input(), cx);
        let wheel = gpui_kit::ScrollEvent {
            position,
            delta: ScrollDelta::Pixels(point(px(0.), px(-4000.))),
            modifiers,
            ..Default::default()
        };
        w.dispatch_event(wheel.to_platform_input(), cx);
        w.render_frame(cx);
    })
    .unwrap();
    let (moved, reached) =
        page.look(cx, |w| (w.find(("flow-run-item", 0usize)).bounds(), w.find(("flow-run-item", 29usize)).bounds()));
    assert!(moved.top() < newest.top() - px(500.), "the menu scrolled: {newest:?} then {moved:?}");
    assert!(reached.bottom() <= newest.top() + px(480.), "the oldest run is in the menu's 480 px: {reached:?}");
    assert_eq!(page.look(cx, |w| w.find(("flow-line", 0usize)).bounds()), before, "the page stood still");
    // and it is picked like any other
    press_on(&page, cx, ("flow-run-item", 29usize));
    assert_eq!(value(&page, cx, "flow-detail").as_deref(), Some("run:71"));
    assert_eq!(flows_state(&page, cx).1.as_deref(), Some("run:71"));
    // the menu opens from its top again, the run shown checked where it stands
    open_runs(&page, cx);
    assert_eq!(page.look(cx, |w| w.find(("flow-run-item", 0usize)).bounds()), newest);
    assert_eq!(chosen(&page, cx, ("flow-run-item", 29usize)), Some(true));
    assert_eq!(chosen(&page, cx, ("flow-run-item", 0usize)), Some(false));
    close_runs(&page, cx);
}

/// Item 17.4: the dialog keeps one height: 820 px where the window gives it that, 78% of a smaller window. At 820
/// px its chain holds a workflow of eight tasks whole (the start, the tasks, the end and New task) with nothing to
/// scroll; in a small window the chain scrolls.
#[gpui_kit::test]
fn the_dialogs_chain_holds_eight_tasks_whole_and_scrolls_in_a_small_window(cx: &mut TestAppContext) {
    let keys = ["SH-21309", "SH-21376", "SH-21522", "SH-21580", "SH-21616", "SH-21533", "SH-21112", "SH-21475"];
    let tasks: Vec<serde_json::Value> = keys.iter().map(|key| flow_task(key, "waiting", "start", None)).collect();
    let tonight = a_flow("f1", "Tonight", "scheduled", Some(NOW + 60 * MIN), tasks);
    let mut index = sample();
    index.flows = serde_json::from_value(serde_json::json!([tonight])).expect("a workflow");
    let page = setup(cx, index, Vec::new(), true);
    cx.simulate_window_resize(page.window, size(px(1728.), px(1100.)));
    page.click(cx, "work-flows");
    page.click(cx, "flow-edit");
    assert_eq!(value(&page, cx, "flow-tasks").as_deref(), Some("8"));
    let measure = |page: &Page, cx: &mut TestAppContext| {
        page.look(cx, |w| {
            let at = |id: &'static str| w.find(id).bounds();
            (at("flow-editor"), at("flow-chain"), at("flow-start"), at("flow-new-task"))
        })
    };
    let (dialog, chain, start, new_task) = measure(&page, cx);
    // (its surface is 1040 px wide; what it holds stands inside the surface's 1 px edge)
    assert_eq!((dialog.size.width, dialog.size.height), (px(1038.), px(820.)));
    assert!(
        start.top() >= chain.top() && new_task.bottom() <= chain.bottom(),
        "{start:?} to {new_task:?} in {chain:?}"
    );
    // the same size with another step shown: it stands still from step to step
    show_step(&page, cx, Some(3));
    assert_eq!(measure(&page, cx).0, dialog);
    // a small window: 78% of its height, and eight tasks are more than its chain holds
    cx.simulate_window_resize(page.window, size(px(1728.), px(700.)));
    let (dialog, chain, _, new_task) = measure(&page, cx);
    assert!((dialog.size.height - px(546.)).abs() < px(0.5), "{dialog:?}");
    assert!(new_task.bottom() > chain.bottom(), "the chain scrolls now: {new_task:?} under {chain:?}");
}

/// Item 6.6: every state shows right, in the list and in the task's box: its word, its color, and what it says.
#[gpui_kit::test]
fn every_state_of_a_task_shows_its_word_in_the_list_and_what_it_means_in_its_box(cx: &mut TestAppContext) {
    let at = NOW - 10 * 60 * MIN;
    let states = [
        ("done", "Done", "Done, nothing needs you"),
        ("review", "Review", "Done, wants your review"),
        ("blocked", "Stuck", "Stuck"),
        ("no_report", "No report", "Stopped without a report"),
        ("timed_out", "Out of time", "Its time ran out"),
        ("asked", "Asked you", "Asked you something"),
        ("cut_off", "Cut off", "Claude was cut off"),
        ("failed", "Didn't run", "Didn't run"),
        ("stopped", "Stopped", "Stopped"),
    ];
    let keys =
        ["SH-21309", "SH-21376", "SH-21522", "SH-21580", "SH-21616", "SH-21533", "SH-21112", "SH-21475", "SH-20728"];
    let tasks: Vec<serde_json::Value> = (states.iter().zip(keys).enumerate())
        .map(|(n, ((state, _, _), key))| {
            let from = at + n as i64 * 10 * MIN;
            flow_task(key, state, "start", Some((from, Some(from + 9 * MIN))))
        })
        .collect();
    let night = a_flow("f1", "Night run", "done", Some(at), tasks);
    let mut index = sample();
    index.flow_runs = serde_json::from_value(serde_json::json!([
        { "id": 1, "flowId": "f1", "startedAt": at, "endedAt": at + 90 * MIN, "state": "done", "flow": night },
    ]))
    .expect("a run");
    // and a workflow that runs: its first task runs, its second waits
    let live = a_flow(
        "f2",
        "Before lunch",
        "running",
        Some(NOW - 5 * MIN),
        vec![
            flow_task("SH-21309", "running", "review", Some((NOW - 5 * MIN, None))),
            flow_task("SH-21376", "waiting", "continue", None),
        ],
    );
    index.flows = serde_json::from_value(serde_json::json!([live])).expect("a workflow");
    let page = setup(cx, index, Vec::new(), true);
    cx.simulate_window_resize(page.window, size(px(1728.), px(1400.)));
    page.click(cx, "work-flows");
    // the one that runs is shown first: Running, and Waiting
    assert_eq!(value(&page, cx, "flow-detail").as_deref(), Some("flow:f2"));
    assert_eq!(says(&page, cx, ("flow-line", 0usize)).as_deref(), Some("1 SH-21309 Running 5 min"));
    assert_eq!(says(&page, cx, ("flow-line", 1usize)).as_deref(), Some("2 SH-21376 Waiting"));
    assert_eq!(
        page.said(cx, "run-task-outcome"),
        Some(format!("Running for 5 min of 60 minIn its workspace since {}, Live.", clock(NOW - 5 * MIN)))
    );
    press_on(&page, cx, ("flow-line", 1usize));
    assert_eq!(
        page.said(cx, "run-task-outcome").as_deref(),
        Some("Waiting its turnTask 2 of 2, after SH-21309: the Claude in its workspace, up to 60 minutes.")
    );
    // the run: each state its word in the list, and its meaning in the task's box
    open_runs(&page, cx);
    press_on(&page, cx, ("flow-run-item", 0usize));
    assert_eq!(page.said(cx, "flow-bar").as_deref(), Some("9 of 9 tasks over"));
    assert_eq!(
        page.said(cx, "flow-words").as_deref(),
        Some("Finished: 1 done, 1 wants your review, 1 stuck, 1 cut off, 4 without a report, 1 stopped")
    );
    // it opens on the first task that needs you, in the tasks' own order: the one for your review
    assert_eq!(value(&page, cx, "run-task-outcome").as_deref(), Some("review"));
    for (n, ((state, short, label), key)) in states.iter().zip(keys).enumerate() {
        assert_eq!(says(&page, cx, ("flow-line", n)), Some(format!("{} {key} {short} 9 min", n + 1)));
        press_on(&page, cx, ("flow-line", n));
        assert_eq!(value(&page, cx, "run-task-outcome").as_deref(), Some(*state));
        assert_eq!(page.said(cx, "run-task-outcome").as_deref(), Some(*label), "{state}");
        // nothing was reported: what happened ends with how it ended
        assert_eq!(run_steps(&page, cx), ["sent", "ended"], "{state}");
        assert_eq!(says(&page, cx, ("run-task-step", 1usize)).as_deref(), Some(*label), "{state}");
    }
}

/// Items 6.4 and 6.7: the page's keys (↑ ↓, j k through the tasks, Esc back to the board), and what can be done
/// with a workflow there: Change, Stop, Run now, delete.
#[gpui_kit::test]
fn the_workflows_page_goes_through_its_tasks_by_key_and_changes_stops_starts_and_deletes_a_workflow(
    cx: &mut TestAppContext,
) {
    let keys = ["SH-21309", "SH-21376", "SH-21522", "SH-21580", "SH-21616", "SH-21533", "SH-21112", "SH-21475"];
    let tasks: Vec<serde_json::Value> = keys.iter().map(|key| flow_task(key, "waiting", "start", None)).collect();
    let tonight = a_flow("f1", "Tonight", "scheduled", Some(NOW + 60 * MIN), tasks);
    let kept = a_flow("f2", "Some day", "saved", None, vec![flow_task("SH-20728", "waiting", "start", None)]);
    let mut index = sample();
    index.flows = serde_json::from_value(serde_json::json!([kept, tonight])).expect("workflows");
    let page = setup(cx, index, Vec::new(), true);
    // a short window: the list of tasks is longer than its pane
    cx.simulate_window_resize(page.window, size(px(1360.), px(520.)));
    page.click(cx, "work-flows");
    // the one due comes before the one kept without a time, and is shown
    assert_eq!(flows_nav(&page, cx).0, ["f1 scheduled", "f2 saved"]);
    assert_eq!(value(&page, cx, "flow-detail").as_deref(), Some("flow:f1"));
    let when = enkidu_model::work::when::when_words(NOW + 60 * MIN, NOW);
    assert_eq!(page.said(cx, "flow-when"), Some(format!("Starts {when}")));
    assert_eq!(page.said(cx, "flow-words"), Some(format!("Starts {when}, 8 tasks")));
    assert!(!page.there(cx, "flow-bar"), "nothing ran: no bar");
    let shown_task = |page: &Page, cx: &mut TestAppContext| value(page, cx, "run-task").unwrap_or_default();
    assert_eq!(shown_task(&page, cx), "ticket:SH-21309", "its first task");
    // ↓ and j go down, ↑ and k up; they stop at the ends
    page.press(cx, "up");
    assert_eq!(shown_task(&page, cx), "ticket:SH-21309");
    page.press(cx, "down");
    page.press(cx, "j");
    assert_eq!(shown_task(&page, cx), "ticket:SH-21522");
    page.press(cx, "k");
    assert_eq!(shown_task(&page, cx), "ticket:SH-21376");
    for _ in 0..12 {
        page.press(cx, "down");
    }
    assert_eq!(shown_task(&page, cx), "ticket:SH-21475", "the last one");
    // the line of the task the keyboard went to is in view
    let (pane, line) = page.look(cx, |w| (w.find("flow-detail").bounds(), w.find(("flow-line", 7usize)).bounds()));
    assert!(line.top() >= pane.top() && line.bottom() <= pane.bottom(), "{line:?} in {pane:?}");
    page.press(cx, "up");
    assert_eq!(shown_task(&page, cx), "ticket:SH-21112");

    // its header scrolled away with the list: back to its top
    let up = ScrollDelta::Pixels(point(px(0.), px(4000.)));
    cx.update_window(page.window, |_, w, cx| w.scroll("flow-detail-list", up, cx)).unwrap();
    // Change: its dialog, on the workflow as you have it; saved, the page shows it
    assert_eq!(page.said(cx, "flow-edit").as_deref(), Some("Change"));
    page.click(cx, "flow-edit");
    assert_eq!(page.said(cx, "flow-editor").as_deref(), Some("Change Tonight"));
    assert!(!page.there(cx, "flow-into"), "nothing is picked: nothing to put anywhere");
    assert_eq!(value(&page, cx, "flow-tasks").as_deref(), Some("8"));
    // the page's keys rest under the dialog
    page.press(cx, "escape");
    assert!(!page.there(cx, "flow-editor") && page.there(cx, "flows-page"), "Esc closed the dialog, not the page");
    page.click(cx, "flow-edit");
    page.click(cx, "flow-name");
    page.press(cx, "cmd-a");
    typed(&page, cx, "Tonight, late");
    page.click(cx, "flow-save");
    assert!(!page.there(cx, "flow-editor"));
    assert_eq!(page.said(cx, "flow-detail").as_deref(), Some("Tonight, late"));
    assert_eq!(flows_state(&page, cx), (true, Some("flow:f1".to_string())));
    // Stop, for one that is due: it is kept, without a time
    cx.update_window(page.window, |_, w, cx| w.hover("flow-stop", cx)).unwrap();
    cx.executor().advance_clock(std::time::Duration::from_millis(900));
    assert_eq!(page.said(cx, "tip").as_deref(), Some("Keep it, without a time"));
    page.click(cx, "flow-stop");
    let kept = flows(&page, cx).into_iter().find(|flow| flow.id == "f1").expect("the workflow");
    assert_eq!((kept.state.as_str(), kept.start_at), ("saved", None));
    assert_eq!(page.said(cx, "flow-when").as_deref(), Some("Kept without a time"));
    assert!(!page.there(cx, "flow-stop"), "nothing to stop now");
    // Run now: due at once, and the page stays on it
    page.click(cx, "flow-run");
    let due = flows(&page, cx).into_iter().find(|flow| flow.id == "f1").expect("the workflow");
    assert_eq!((due.state.as_str(), due.start_at), ("scheduled", Some(NOW)));
    assert_eq!(value(&page, cx, "flow-detail").as_deref(), Some("flow:f1"));
    // delete: it is gone, and the page shows what is left
    page.click(cx, "flow-delete");
    assert_eq!(flows(&page, cx).iter().map(|flow| flow.id.as_str()).collect::<Vec<_>>(), ["f2"]);
    assert_eq!(flows_nav(&page, cx).0, ["f2 saved"]);
    assert_eq!(value(&page, cx, "flow-detail").as_deref(), Some("flow:f2"));
    // Esc: back to the board; what you picked in the name's menu is kept for the next time
    page.press(cx, "escape");
    assert!(!page.there(cx, "flows-page") && page.there(cx, "work-board"));
    page.click(cx, "work-flows");
    assert_eq!(value(&page, cx, "flow-detail").as_deref(), Some("flow:f2"));
}

/// The mouse goes down at one place of the window, moves to another and comes up there, with frames between, as a
/// hand drags.
fn drag_from_to(
    page: &Page,
    cx: &mut TestAppContext,
    from: gpui_kit::Point<gpui_kit::Pixels>,
    to: gpui_kit::Point<gpui_kit::Pixels>,
) {
    cx.run_until_parked();
    cx.update_window(page.window, |_, w, cx| {
        w.render_frame(cx);
        let (modifiers, left) = (Modifiers::default(), MouseButton::Left);
        w.dispatch_event(MouseMoveEvent { position: from, pressed_button: None, modifiers }.to_platform_input(), cx);
        w.render_frame(cx);
        let down = MouseDownEvent { button: left, position: from, modifiers, click_count: 1, first_mouse: false };
        w.dispatch_event(down.to_platform_input(), cx);
        w.render_frame(cx);
        let middle = point((from.x + to.x) / 2., (from.y + to.y) / 2.);
        for position in [middle, to] {
            let moved = MouseMoveEvent { position, pressed_button: Some(left), modifiers };
            w.dispatch_event(moved.to_platform_input(), cx);
            w.render_frame(cx);
        }
        w.dispatch_event(
            MouseUpEvent { button: left, position: to, modifiers, click_count: 1 }.to_platform_input(),
            cx,
        );
        w.render_frame(cx);
    })
    .unwrap();
}

/// What is selected in the window, as ⌘C would copy it.
fn selected(page: &Page, cx: &mut TestAppContext) -> String {
    cx.run_until_parked();
    cx.update_window(page.window, |_, w, cx| {
        w.render_frame(cx);
        gpui_kit::base::TextSelection::selected_text(w, cx)
    })
    .unwrap()
}

/// Item 7.5: a drag over a description selects its words, from one paragraph on into the next, ⌘C copies them, and
/// the card keeps the keyboard.
#[gpui_kit::test]
fn a_descriptions_words_can_be_selected_with_the_mouse_and_copied(cx: &mut TestAppContext) {
    let page = setup(cx, sample(), Vec::new(), true);
    // one paragraph: from before its first letter to after its last
    open_card(&page, cx, "ticket:SH-21112");
    let whole = "Show the cost of one article next to the bundle's total cost, using the article count on the bundle.";
    let text = page.look(cx, |w| w.find("jira-doc").bounds());
    drag_from_to(&page, cx, point(text.left(), text.top() + px(8.)), point(text.right(), text.bottom() - px(4.)));
    assert_eq!(selected(&page, cx).trim(), whole);
    // ⌘C puts it on the clipboard
    page.press(cx, "cmd-c");
    assert_eq!(cx.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some(whole));
    // the card still has the keyboard: Escape closes it
    page.press(cx, "escape");
    assert!(!page.there(cx, "task-dialog"), "the card lost the keyboard to the selection");

    // several paragraphs are one text: a drag from the first into the heading under it takes both, in order
    open_card(&page, cx, "ticket:SH-21309");
    let text = page.look(cx, |w| w.find("jira-doc").bounds());
    drag_from_to(&page, cx, point(text.left(), text.top() + px(8.)), point(text.right(), text.top() + px(60.)));
    let taken = selected(&page, cx);
    let first = taken.find("Accounting imports the orders of a month as a CSV.");
    let second = taken.find("What to build");
    assert!(first.is_some() && second.is_some() && first < second, "what was selected: {taken:?}");
    // a press elsewhere lets it go
    click_slowly_at(&page, cx, f32::from(text.left()) + 4., f32::from(text.bottom()) - 2.);
    assert_eq!(selected(&page, cx).trim(), "", "a click left the selection there");

    // a line with a link, a name and a status on their fills: what is copied is what it says, without the thin
    // spaces that give those fills their room
    let page = setup(cx, with_rich_description(sample(), "SH-21309"), Vec::new(), true);
    open_card(&page, cx, "ticket:SH-21309");
    let line = page.look(cx, |w| w.find("jira-doc:links:0").bounds());
    let said = page.said(cx, "jira-doc:links:0").unwrap_or_default();
    assert!(said.ends_with("Ask @Mira about the columns; the import is IN REVIEW."), "{said}");
    drag_from_to(&page, cx, point(line.left(), line.top() + px(8.)), point(line.right(), line.bottom() - px(4.)));
    assert!(selected(&page, cx).contains('\u{2009}'), "the line as it is drawn has that room");
    page.press(cx, "cmd-c");
    assert_eq!(cx.read_from_clipboard().and_then(|item| item.text()), Some(said));
    // the link still opens on a click: a press without a drag selects nothing
    click_slowly_at(&page, cx, f32::from(line.left()) + 12., f32::from(line.top()) + 10.);
    assert_eq!(cx.opened_url().as_deref(), Some("https://example.com/spec"));
}

/// Item 7.5b: a recording among a report's screenshots (a GIF) plays when it is shown large, at its own size. Small
/// it is its first picture that shows something.
#[gpui_kit::test]
fn a_recording_in_a_report_plays_when_it_is_shown_large(cx: &mut TestAppContext) {
    // four pictures of 320 by 200, a tenth of a second apart
    let mut recording = Vec::new();
    {
        let mut encoder = image::codecs::gif::GifEncoder::new(&mut recording);
        for n in 0..4u8 {
            let picture = image::RgbaImage::from_pixel(320, 200, image::Rgba([40 + n * 50, 0, 0, 255]));
            let delay = image::Delay::from_numer_denom_ms(100, 1);
            encoder.encode_frame(image::Frame::from_parts(picture, 0, 0, delay)).unwrap();
        }
    }
    cx.update(|cx| {
        cx.set_global(enkidu_app::work::shots::ShotSource(std::sync::Arc::new(move |report, name| {
            match name.ends_with(".gif") {
                true => Ok(recording.clone()),
                false => Ok(enkidu_testkit::screenshot_png(640, 400, report as u32)),
            }
        })));
    });
    let mut work = sample();
    work.reports = vec![report(7, "review", "Recorded the export.", &["1-export.png", "2-flow.gif"], 60_000)];
    let page = setup(cx, work, vec![ws("w2", "sh-21309-export", "feature/SH-21309-export")], true);
    open_card(&page, cx, "ticket:SH-21309");
    assert_eq!(tells(&page, cx, "report-image:7:2-flow.gif").as_deref(), Some("shown"), "small: its first picture");
    assert_eq!(page.said(cx, "report-image-kind:7:2-flow.gif").as_deref(), Some("GIF"), "and its kind in its corner");
    // large, with "Reduce motion" (as these tests run): it stands still on the picture it is known by
    page.click(cx, "report-image:7:2-flow.gif");
    assert_eq!(page.said(cx, "report-lightbox").as_deref(), Some("2-flow.gif"));
    assert_eq!(tells(&page, cx, "report-lightbox-image").as_deref(), Some("whole"));
    page.press(cx, "escape");
    // large, where things may move: it plays, and goes on playing
    cx.update(|cx| cx.set_reduce_motion(false));
    page.click(cx, "report-image:7:2-flow.gif");
    assert_eq!(tells(&page, cx, "report-lightbox-image").as_deref(), Some("whole, playing"));
    for _ in 0..6 {
        cx.executor().advance_clock(std::time::Duration::from_millis(100));
        cx.run_until_parked();
    }
    assert_eq!(
        tells(&page, cx, "report-lightbox-image").as_deref(),
        Some("whole, playing"),
        "around its four pictures"
    );
    let picture = page.look(cx, |w| w.find("report-lightbox-image").bounds());
    assert_eq!(picture.size, size(px(320.), px(200.)), "never larger than the recording is");
    // a still one beside it is one picture, as before
    page.press(cx, "left");
    assert_eq!(page.said(cx, "report-lightbox").as_deref(), Some("1-export.png"));
    assert_eq!(tells(&page, cx, "report-lightbox-image").as_deref(), Some("whole"));
}

/// Phase 26: every thumbnail of a report says what kind of file it is, and a recording among them plays by itself:
/// only while a frame that shows it is drawn, and not behind a screenshot shown large.
#[gpui_kit::test]
fn a_recording_plays_in_its_thumbnail_and_every_thumbnail_says_its_kind(cx: &mut TestAppContext) {
    // four pictures of 320 by 200, a tenth of a second apart, each of two colors (it shows something)
    let mut recording = Vec::new();
    {
        let mut encoder = image::codecs::gif::GifEncoder::new(&mut recording);
        for n in 0..4u8 {
            let picture = image::RgbaImage::from_fn(320, 200, |x, _| match x < 160 {
                true => image::Rgba([40 + n * 50, 0, 0, 255]),
                false => image::Rgba([0, 0, 40 + n * 50, 255]),
            });
            let delay = image::Delay::from_numer_denom_ms(100, 1);
            encoder.encode_frame(image::Frame::from_parts(picture, 0, 0, delay)).unwrap();
        }
    }
    cx.update(|cx| {
        cx.set_global(enkidu_app::work::shots::ShotSource(std::sync::Arc::new(move |report, name| {
            match name.ends_with(".gif") {
                true => Ok(recording.clone()),
                false => Ok(enkidu_testkit::screenshot_png(640, 400, report as u32)),
            }
        })));
    });
    let mut work = sample();
    work.reports = vec![report(7, "review", "Recorded the export.", &["1-export.png", "2-flow.gif"], 60_000)];
    let page = setup(cx, work, vec![ws("w2", "sh-21309-export", "feature/SH-21309-export")], true);
    // things may move here (these tests otherwise run as with "Reduce motion")
    cx.update(|cx| cx.set_reduce_motion(false));
    open_card(&page, cx, "ticket:SH-21309");
    assert_eq!(page.said(cx, "report-image-kind:7:1-export.png").as_deref(), Some("PNG"));
    assert_eq!(page.said(cx, "report-image-kind:7:2-flow.gif").as_deref(), Some("GIF"));
    assert_eq!(tells(&page, cx, "report-image:7:1-export.png").as_deref(), Some("shown"));
    assert_eq!(tells(&page, cx, "report-image:7:2-flow.gif").as_deref(), Some("shown, playing"));
    // which picture a thumbnail shows now
    let shots = page.shell.read_with(cx, |shell, cx| {
        let work = shell.work_page().expect("the Work page").read(cx);
        work.card_view().expect("the card is open").read(cx).shots().clone()
    });
    let shows = |cx: &mut TestAppContext, name: &str| {
        shots.read_with(cx, |shots, _| match shots.small(7, name) {
            Some(enkidu_app::work::shots::Small::Ready { picture, .. }) => std::sync::Arc::as_ptr(picture) as usize,
            _ => 0,
        })
    };
    // a tenth of a second passes, and the app draws its frame after it (a test window is drawn after every update
    // that changed it; that a thumbnail no paint saw waits is tested with the rule itself, in the app's shots.rs)
    let step = |cx: &mut TestAppContext| {
        cx.executor().advance_clock(std::time::Duration::from_millis(100));
        cx.run_until_parked();
        page.look(cx, |_| ());
    };
    // how many times the recording's thumbnail went on to another picture in so many steps
    let goes_on = |cx: &mut TestAppContext, steps: usize| {
        let mut last = shows(cx, "2-flow.gif");
        let mut changed = 0;
        for _ in 0..steps {
            step(cx);
            let now = shows(cx, "2-flow.gif");
            changed += usize::from(now != last);
            last = now;
        }
        changed
    };
    let still = shows(cx, "1-export.png");
    let changed = goes_on(cx, 8);
    assert!(changed >= 5, "in sight, the recording went on {changed} times in 8 steps");
    assert_eq!(shows(cx, "1-export.png"), still, "a still picture stays the picture it is");
    // behind a screenshot shown large it waits too
    page.click(cx, "report-image:7:1-export.png");
    assert_eq!(page.said(cx, "report-lightbox").as_deref(), Some("1-export.png"));
    goes_on(cx, 3);
    assert_eq!(goes_on(cx, 6), 0, "it went on behind the screenshot shown large");
    // and goes on when that one closes
    page.press(cx, "escape");
    assert!(goes_on(cx, 8) >= 5);
    // the card closes: nothing paints its thumbnail any more, so the recording waits (and ends with the card's
    // pictures when they go)
    page.press(cx, "escape");
    goes_on(cx, 3);
    assert_eq!(goes_on(cx, 6), 0, "it went on though its card is closed");
}

/// Smooth first: the board, and a card full of text that can be selected, are not drawn again while nothing
/// happens. (A test window is drawn after an update only when something asked for a frame; no cursor blinks here.)
#[gpui_kit::test]
fn the_board_and_a_card_at_rest_are_not_drawn_again(cx: &mut TestAppContext) {
    let page = setup(cx, with_rich_description(sample(), "SH-21309"), Vec::new(), true);
    let idle = |cx: &mut TestAppContext| {
        for _ in 0..3 {
            cx.executor().advance_clock(std::time::Duration::from_millis(2_000));
            cx.run_until_parked();
            cx.update_window(page.window, |_, w, cx| w.render_frame(cx)).unwrap();
        }
        let before = page.shell.read_with(cx, |shell, _| shell.drawn());
        for _ in 0..40 {
            cx.executor().advance_clock(std::time::Duration::from_millis(100));
            cx.run_until_parked();
            cx.update_window(page.window, |_, _, _| {}).unwrap();
        }
        page.shell.read_with(cx, |shell, _| shell.drawn()) - before
    };
    let board = idle(cx);
    open_card(&page, cx, "ticket:SH-21309");
    let card = idle(cx);
    assert_eq!((board, card), (0, 0), "frames in four seconds of nothing: the board, then the card");
}

/// The column a card stands in, by the card's id; none when it isn't on the board.
fn column_of(page: &Page, cx: &mut TestAppContext, id: &str) -> Option<String> {
    cx.run_until_parked();
    page.shell.read_with(cx, |shell, cx| {
        let board = shell.work_page().expect("the Work page").read(cx).board();
        (board.columns.iter()).find(|column| column.cards.iter().any(|card| card.id == id)).map(|c| c.name.clone())
    })
}

/// Your own tasks as the page's index has them: number, project, title, notes, state.
fn own_tasks(page: &Page, cx: &mut TestAppContext) -> Vec<(i64, String, String, String, String)> {
    cx.run_until_parked();
    page.work.read_with(cx, |work, _| {
        let index = work.snapshot.as_ref().expect("the index");
        (index.own_tasks.iter())
            .map(|t| (t.id, t.project_id.clone(), t.title.clone(), t.notes.clone(), t.state.clone()))
            .collect()
    })
}

fn click_any(page: &Page, cx: &mut TestAppContext, id: impl Into<gpui_kit::ElementId>) {
    let id = id.into();
    cx.run_until_parked();
    cx.update_window(page.window, |_, w, cx| {
        w.render_frame(cx);
        w.click(id, cx)
    })
    .unwrap();
}

/// Item 8.5: New task in the header makes a task of your own for one of
/// your projects; its card stands in "To do"; in front of the board it has its words, its state (which moves it to
/// another column) and Edit, which changes its words or deletes it.
#[gpui_kit::test]
fn a_task_of_your_own_is_added_from_the_header_moved_by_its_state_changed_and_deleted(cx: &mut TestAppContext) {
    let page = setup(cx, sample(), vec![], true);
    assert_eq!(page.said(cx, "work-new-task").as_deref(), Some("New task"));
    assert!(columns(&page, cx).iter().all(|(name, _)| name != "To do"), "no such column until there is such a task");
    let cards_before = shown(&page, cx).len();

    // the dialog: a title, what it is about, the project (shop is the one on screen), To do
    page.click(cx, "work-new-task");
    assert_eq!(page.said(cx, "own-task-dialog").as_deref(), Some("A task of your own"));
    page.look(cx, |w| {
        assert_eq!(w.find(("own-task-project", 0usize)).label(), Some("shop"));
        assert_eq!(w.find(("own-task-project", 0usize)).selected(), Some(true), "the project on screen");
        assert_eq!(w.find(("own-task-project", 1usize)).selected(), Some(false));
        assert_eq!(w.find("own-task-state-todo").selected(), Some(true));
        assert!(w.try_find("own-task-delete").is_none(), "nothing to delete yet");
    });
    // nothing is kept without a title
    page.click(cx, "own-task-save");
    assert_eq!(page.said(cx, "own-task-error").as_deref(), Some("Give the task a title."));
    assert!(own_tasks(&page, cx).is_empty());
    // the title has the keyboard: typed, then what it is about, the other project, and Add task
    page.click(cx, "own-task-title");
    typed(&page, cx, "  Write the release notes ");
    page.click(cx, "own-task-notes");
    typed(&page, cx, "For the preview: what is new since phase 7.");
    click_any(&page, cx, ("own-task-project", 1usize));
    page.look(cx, |w| assert_eq!(w.find(("own-task-project", 1usize)).selected(), Some(true)));
    page.click(cx, "own-task-save");
    assert!(!page.there(cx, "own-task-dialog"), "the dialog stayed: {:?}", page.said(cx, "own-task-error"));
    assert_eq!(
        own_tasks(&page, cx),
        [(
            1,
            "p2".to_string(),
            "Write the release notes".to_string(),
            "For the preview: what is new since phase 7.".to_string(),
            "todo".to_string()
        )]
    );
    assert_eq!(page.said(cx, "toast").as_deref(), Some("Added to the board"));

    // on the board: one card more, in a column of its own, saying whose it is
    assert_eq!(shown(&page, cx).len(), cards_before + 1);
    assert_eq!(column_of(&page, cx, "own:1").as_deref(), Some("To do"));
    let said = page.look(cx, |w| w.find("work-card:own:1").description().map(String::from));
    assert_eq!(said.as_deref(), Some("Your task 1 for enkidu | Write the release notes"));
    // the filter finds it by what you wrote about it
    page.click(cx, "work-query");
    typed(&page, cx, "phase 7");
    assert_eq!(shown(&page, cx), ["own:1"]);
    page.press(cx, "cmd-a");
    page.press(cx, "backspace");
    assert_eq!(shown(&page, cx).len(), cards_before + 1);

    // in front of the board: its words, no Jira and no pull request, and its own box
    open_card(&page, cx, "own:1");
    assert_eq!(page.said(cx, "task-key").as_deref(), Some("Your task 1"));
    assert_eq!(page.said(cx, "task-title").as_deref(), Some("Write the release notes"));
    assert!(page.there(cx, "task-description"), "what you wrote about it is the card's story");
    assert!(!page.there(cx, "task-no-pr") && !page.there(cx, "detail-open") && !page.there(cx, "task-jira"));
    let facts = described(&page, cx, "task-own").unwrap_or_default();
    assert!(facts.starts_with("State To do, project enkidu, made "), "{facts}");
    page.look(cx, |w| assert_eq!(w.find("task-own-state-todo").selected(), Some(true)));

    // In progress: the card stands with Jira's tickets in progress, in the column as Jira names it
    page.click(cx, "task-own-state-doing");
    assert_eq!(own_tasks(&page, cx)[0].4, "doing");
    assert_eq!(column_of(&page, cx, "own:1").as_deref(), Some("In Progress"));
    page.look(cx, |w| assert_eq!(w.find("task-own-state-doing").selected(), Some(true)));
    assert!(columns(&page, cx).iter().all(|(name, _)| name != "To do"), "an empty column isn't kept");
    // Done
    page.click(cx, "task-own-state-done");
    assert_eq!((own_tasks(&page, cx)[0].4.as_str(), column_of(&page, cx, "own:1").as_deref()), ("done", Some("Done")));
    assert_eq!(state(&page, cx).2.as_deref(), Some("own:1"), "the card stays open while it moves");

    // Edit: its dialog in front of the card, with its words; other words, and Save
    page.click(cx, "task-edit");
    assert_eq!(page.said(cx, "own-task-dialog").as_deref(), Some("Your task"));
    page.look(cx, |w| {
        assert!(w.try_find(("own-task-project", 0usize)).is_none(), "its project stays");
        assert_eq!(w.find("own-task-state-done").selected(), Some(true));
    });
    page.click(cx, "own-task-title");
    page.press(cx, "cmd-a");
    typed(&page, cx, "Write the release notes for the preview");
    page.click(cx, "own-task-state-doing");
    page.click(cx, "own-task-save");
    assert!(!page.there(cx, "own-task-dialog"));
    let now = own_tasks(&page, cx);
    assert_eq!((now[0].2.as_str(), now[0].4.as_str()), ("Write the release notes for the preview", "doing"));
    // the card under it is still open, and says the new words
    assert_eq!(page.said(cx, "task-title").as_deref(), Some("Write the release notes for the preview"));
    assert_eq!(column_of(&page, cx, "own:1").as_deref(), Some("In Progress"));

    // Delete asks once more; Keep it keeps it; Delete it takes it off the board and closes its card too
    page.click(cx, "task-edit");
    page.click(cx, "own-task-delete");
    assert!(page.there(cx, "own-task-keep") && page.there(cx, "own-task-delete-it"));
    page.click(cx, "own-task-keep");
    assert!(page.there(cx, "own-task-delete") && own_tasks(&page, cx).len() == 1);
    page.click(cx, "own-task-delete");
    page.click(cx, "own-task-delete-it");
    assert!(own_tasks(&page, cx).is_empty());
    let open = (page.there(cx, "own-task-dialog"), page.there(cx, "task-title"));
    assert_eq!(open, (false, false), "a dialog stayed open (the task's, the card's)");
    assert_eq!((column_of(&page, cx, "own:1"), shown(&page, cx).len()), (None, cards_before));
    assert_eq!(state(&page, cx).2, None);
}
