//! The Work page on the whole wiring (phase 5, item 5.19): the real core with its watcher on the fake `gh` and the fake
//! `twg` (never the real ones, never the network), the stores and the window.

mod common;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, SharedString, TestAppContext};

/// The app on the sample project, whose origin says it is the repository the recorded answers are about, with its Jira
/// projects set.
fn start(cx: &mut TestAppContext) -> common::App {
    common::start_on(cx, enkidu_testkit::shop_on_github, |shared, project, _| common::reads_jira(shared, project))
}

/// Item 5.1c on the real core: ⌘I opens the page; before anything was read it offers to check, and Check now makes
/// the core read; where you were is remembered, so the page is there again after the app was quit and opened.
#[gpui_kit::test]
fn the_first_run_checks_now_and_the_page_is_there_again_after_a_restart(cx: &mut TestAppContext) {
    let app = start(cx);
    let fakes = app.fakes();
    assert!(app.until(cx, |w| w.try_find("work-tab").is_some()), "the Work tab is there once there is a project");
    app.press(cx, "cmd-i");
    assert!(app.until(cx, |w| w.try_find("work-first-run").is_some()), "nothing was read yet");
    assert!(app.shared.work.wanted().is_empty());
    // Check now: the core is asked to read this project's GitHub and Jira
    app.click(cx, "work-check-now");
    let mut asked = false;
    for _ in 0..400 {
        cx.run_until_parked();
        asked = app.shared.work.wanted().contains_key(&app.project_id);
        if asked {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(asked, "Check now asked the core for a read");
    // its watcher reads (through the fakes): the first run's words go, and each source's lamp in the header says how
    // it stands
    app.start_watcher();
    assert!(app.until(cx, |w| w.try_find("work-first-run").is_none()), "something was read");
    let lamp = |w: &gpui_kit::Window, id: &'static str| {
        w.try_find(id).map(|lamp| (lamp.value().map(String::from), lamp.description().map(String::from)))
    };
    let read = |as_whom: &str| Some((Some("ok".to_string()), Some(format!("shop: read just now as {as_whom}"))));
    assert!(
        app.until(cx, |w| lamp(w, "health-github") == read("me-dev") && lamp(w, "health-jira") == read("Me Dev")),
        "{:?}",
        cx.update_window(app.window, |_, w, _| (lamp(w, "health-github"), lamp(w, "health-jira"))).unwrap()
    );
    // the page is where you are: kept a moment after you got there
    cx.executor().advance_clock(std::time::Duration::from_millis(400));
    let view = app.home.path().join("data").join("view.json");
    let mut kept = String::new();
    for _ in 0..400 {
        cx.run_until_parked();
        kept = std::fs::read_to_string(&view).unwrap_or_default();
        if kept.contains(r#""work":true"#) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(kept.contains(r#""work":true"#), "view.json: {kept}");
    // the reads asked only to read
    let log = fakes.gh_log();
    assert!(!log.is_empty() && !log.contains("fake-token") && !log.contains("mutation"), "only reads, no token");
    common::never_wrote(&fakes);
    drop(fakes);

    // quit and open again: the page is on screen at once, with what the core's database has (no first run)
    let app = common::restart_as(cx, app, false);
    assert!(app.until(cx, |w| w.try_find("work-page").is_some()), "the Work page is where you were");
    assert!(app.until(cx, |w| lamp(w, "health-github").is_some_and(|(state, _)| state.as_deref() == Some("ok"))));
    assert!(app.until(cx, |w| w.try_find("work-first-run").is_none()));
    // and ⌘I goes back to the workspace
    app.press(cx, "cmd-i");
    assert!(app.until(cx, |w| w.try_find("work-page").is_none() && w.try_find("tabs-row").is_some()));
}

/// The app with the Work page open on what the fakes give: both sources read, the board on screen, in a window
/// wide enough for every column.
fn on_the_board(cx: &mut TestAppContext) -> (common::App, common::Fakes) {
    let app = start(cx);
    let fakes = app.start_work();
    cx.simulate_window_resize(app.window, gpui_kit::size(gpui_kit::px(2600.), gpui_kit::px(900.)));
    app.press(cx, "cmd-i");
    app.until_work(cx, |work| !work.checking && work.health.len() == 2 && !work.tickets.is_empty());
    assert!(app.until(cx, |w| w.try_find(card("SH-21349")).is_some()), "the board is on screen");
    (app, fakes)
}

/// A ticket's card on screen.
fn card(key: &str) -> SharedString {
    SharedString::from(format!("work-card:ticket:{key}"))
}

fn pick_box(key: &str) -> SharedString {
    SharedString::from(format!("card-pick:ticket:{key}"))
}

/// Picks a card as a person does: the pointer goes onto the card, its round box shows, and a click on that picks it.
fn pick(cx: &mut TestAppContext, app: &common::App, key: &str) {
    cx.run_until_parked();
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        w.hover(card(key), cx);
        w.render_frame(cx);
        w.click(pick_box(key), cx)
    })
    .unwrap();
}

/// The cards on the board, in its order.
fn shown(cx: &mut TestAppContext, app: &common::App) -> Vec<String> {
    cx.run_until_parked();
    app.shell.read_with(cx, |shell, cx| {
        let board = shell.work_page().expect("the Work page").read(cx).board();
        board.shown().into_iter().map(String::from).collect()
    })
}

/// Waits until the board shows what `seen` wants.
fn until_shown(cx: &mut TestAppContext, app: &common::App, seen: impl Fn(&[String]) -> bool) -> Vec<String> {
    for _ in 0..1500 {
        let now = shown(cx, app);
        if seen(&now) {
            return now;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the board never showed what was expected; it shows {:?}", shown(cx, app));
}

/// The index as the core's own database has it (what a restart reads).
fn kept(app: &common::App) -> enkidu_core::work::model::Snapshot {
    enkidu_core::work::snapshot(&app.shared).expect("the core's index")
}

/// Hides picked cards, keeps them in the app's database, and unhides them from Hidden.
#[gpui_kit::test]
fn hides_picked_cards_keeps_them_in_the_apps_database_and_unhides_them_from_hidden(cx: &mut TestAppContext) {
    let (app, fakes) = on_the_board(cx);
    // everything read is on the board: no project chips (the projects are the tabs above), nothing hidden
    assert!(app.until(cx, |w| w.try_find("work-project").is_none() && w.try_find("work-show-hidden").is_none()));
    for key in ["SH-20844", "SH-20890"] {
        pick(cx, &app, key);
    }
    assert!(app.until(cx, |w| common::said(w, "pick-bar").as_deref() == Some("2 cards picked")));
    app.click(cx, "pick-hide");
    assert!(app.until(cx, |w| w.try_find(card("SH-20844")).is_none()), "the card left the board");
    // the app's database has them, and each card's timeline says when
    let mut hidden = Vec::new();
    for _ in 0..600 {
        hidden = kept(&app).hidden;
        if hidden.len() == 2 {
            break;
        }
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    hidden.sort();
    assert_eq!(hidden, ["ticket:SH-20844", "ticket:SH-20890"]);
    let said = kept(&app).events.iter().any(|e| e.subject == "SH-20844" && e.text == "Hidden from the board");
    assert!(said, "the card's timeline says it was hidden");
    // Hidden, in the header, shows only them
    assert!(app.until(cx, |w| w.try_find("work-show-hidden").and_then(|e| e.value().map(String::from)).as_deref()
        == Some("2")));
    app.click(cx, "work-show-hidden");
    let two = until_shown(cx, &app, |now| now.len() == 2);
    assert!(two.contains(&"SH-20844".to_string()) && two.contains(&"SH-20890".to_string()), "{two:?}");

    // they live in the app's database: quit and open again, they are still hidden (where you were, the Work page, is
    // kept a moment after you got there)
    cx.executor().advance_clock(std::time::Duration::from_millis(400));
    let view = app.home.path().join("data").join("view.json");
    for _ in 0..600 {
        cx.run_until_parked();
        if std::fs::read_to_string(&view).unwrap_or_default().contains(r#""work":true"#) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    common::never_wrote(&fakes);
    drop(fakes);
    let app = common::restart_as(cx, app, false);
    cx.simulate_window_resize(app.window, gpui_kit::size(gpui_kit::px(2600.), gpui_kit::px(900.)));
    assert!(app.until(cx, |w| w.try_find("work-show-hidden").and_then(|e| e.value().map(String::from)).as_deref()
        == Some("2")));
    assert!(app.until(cx, |w| w.try_find(card("SH-20844")).is_none() && w.try_find(card("SH-21349")).is_some()));
    app.click(cx, "work-show-hidden");
    until_shown(cx, &app, |now| now.len() == 2);
    // one comes back
    pick(cx, &app, "SH-20890");
    assert!(app.until(cx, |w| common::said(w, "pick-show").as_deref() == Some("Unhide")));
    app.click(cx, "pick-show");
    until_shown(cx, &app, |now| now == ["SH-20844"]);
    for _ in 0..600 {
        if kept(&app).hidden == ["ticket:SH-20844"] {
            break;
        }
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(kept(&app).hidden, ["ticket:SH-20844"], "SH-20890 came back");
    // the last one too: with nothing hidden, the board is back, both on it, and Hidden is gone from the header
    pick(cx, &app, "SH-20844");
    app.click(cx, "pick-show");
    assert!(app.until(cx, |w| w.try_find("work-show-hidden").is_none()));
    let all =
        until_shown(cx, &app, |now| now.contains(&"SH-20844".to_string()) && now.contains(&"SH-20890".to_string()));
    assert!(all.len() > 2);
    let column = |cx: &mut TestAppContext, key: &str| -> Option<String> {
        cx.run_until_parked();
        app.shell.read_with(cx, |shell, cx| {
            let board = shell.work_page().expect("the Work page").read(cx).board();
            board.columns.iter().find(|column| column.cards.iter().any(|card| card.name() == key)).map(|c| c.id.clone())
        })
    };
    assert_eq!(column(cx, "SH-20890").as_deref(), Some("code review"));
    assert_eq!(column(cx, "SH-20844").as_deref(), Some("code review"));
    assert!(kept(&app).hidden.is_empty());
}

/// Shows one Jira board's tickets once you pick it (your tickets in its running sprint), and all of them again.
#[gpui_kit::test]
fn shows_one_jira_boards_tickets_once_you_pick_it_and_all_of_them_again(cx: &mut TestAppContext) {
    let (app, fakes) = on_the_board(cx);
    // the tickets in the fake's running sprint
    let sprint: Vec<String> = serde_json::from_str(
        &std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../enkidu-core/tests/fixtures/work/twg-sprint.json"),
        )
        .expect("the recorded sprint"),
    )
    .expect("a list of keys");
    let all = shown(cx, &app);
    let tickets =
        |cards: &[String]| -> Vec<String> { cards.iter().filter(|key| !key.starts_with("pr:")).cloned().collect() };
    assert!(tickets(&all).iter().any(|key| !sprint.contains(key)), "the fake has tickets outside the sprint");
    assert!(app.until(cx, |w| common::said(w, "work-board-pick").as_deref() == Some("All tasks")));
    // Jira's boards were read ahead of the menu: its first opening has them
    app.until_work(cx, |work| !work.boards.is_empty());
    app.click(cx, "work-board-pick");
    assert!(app.until(cx, |w| common::said(w, "work-board-option:5").as_deref() == Some("Shop Dev")));
    app.click(cx, "work-board-option:5");
    // the header names the board at once; its cards come when its read lands: only your tickets in its sprint
    assert!(app.until(cx, |w| common::said(w, "work-board-pick").as_deref() == Some("Shop Dev")));
    let on_board = until_shown(cx, &app, |now| !now.is_empty() && now.iter().all(|key| sprint.contains(key)));
    assert!(on_board.len() < all.len());
    // the app keeps the pick
    let mut picked = None;
    for _ in 0..600 {
        picked = kept(&app).board.map(|pick| pick.board.id);
        if picked.is_some() {
            break;
        }
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(picked.as_deref(), Some("5"));
    // All tasks: all of them again
    app.click(cx, "work-board-pick");
    assert!(app.until(cx, |w| w.try_find("work-board-option:").is_some()));
    app.click(cx, "work-board-option:");
    until_shown(cx, &app, |now| now.len() == all.len());
    assert!(app.until(cx, |w| common::said(w, "work-board-pick").as_deref() == Some("All tasks")));
    for _ in 0..600 {
        if kept(&app).board.is_none() {
            break;
        }
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(kept(&app).board.is_none());
    // only reads went to Jira: who you are, the boards and what is on one, searches and single tickets
    let reads = [
        "whoami",
        "jira board query",
        "jira board scope query",
        "jira workitem query",
        "jira workitem get",
        "jira workitem bulk-get",
        "jira workitem comment query",
    ];
    let log = fakes.twg_log();
    assert!(log.contains("| jira board query"), "the boards were read");
    for line in log.lines() {
        let call = line.split(" | ").nth(1).unwrap_or("");
        assert!(reads.iter().any(|read| call.starts_with(read)), "only reads: {call}");
    }
    common::never_wrote(&fakes);
}

/// The thing that needs you the keyboard is on, in the card in front of the board.
fn item_on(cx: &mut TestAppContext, app: &common::App) -> Option<String> {
    cx.run_until_parked();
    app.shell.read_with(cx, |shell, cx| {
        let card = shell.work_page()?.read(cx).card_view()?.read(cx);
        card.current_item().map(|item| item.id.clone())
    })
}

/// Opens a card from the keyboard, marks done and snoozes what needs you there, and Esc closes it, then goes back.
#[gpui_kit::test]
fn opens_a_card_from_the_keyboard_marks_done_and_snoozes_what_needs_you_there_and_esc_closes_it_then_goes_back(
    cx: &mut TestAppContext,
) {
    let (app, fakes) = on_the_board(cx);
    let page = app.shell.read_with(cx, |shell, _| shell.work_page().expect("the Work page").clone());
    // the arrows move across the board
    app.press(cx, "down");
    cx.run_until_parked();
    assert!(page.read_with(cx, |page, _| page.selected().is_some()), "the arrows reached a card");
    // the card where the most waits on you, selected as the arrows select, opened with Enter
    let id = page.read_with(cx, |page, _| {
        let cards = page.board().columns.iter().flat_map(|column| column.cards.iter());
        cards.max_by_key(|card| card.waiting).map(|card| (card.id.clone(), card.waiting)).expect("a card")
    });
    assert!(id.1 >= 2, "the fakes give a card with two things or more waiting: {id:?}");
    let id = id.0;
    page.update(cx, |page, cx| page.select(Some(id.clone()), cx));
    app.press(cx, "enter");
    let opened = SharedString::from(format!("{id}, wide"));
    assert!(
        app.until(cx, |w| w.try_find("task-dialog").and_then(|e| e.description().map(String::from)).as_deref()
            == Some(&*opened)),
        "Enter opened the card"
    );
    // e: the item the keyboard is on is done, and leaves the card
    let first = item_on(cx, &app).expect("the keyboard is on what needs you");
    let item = |id: &str| SharedString::from(format!("detail-item:{id}"));
    assert!(app.until(cx, |w| w.try_find(item(&first)).is_some_and(|e| e.selected() == Some(true))));
    app.press(cx, "e");
    assert!(app.until(cx, |w| w.try_find(item(&first)).is_none()), "the item was marked done");
    // the item that took its place is picked: snooze it
    let second = item_on(cx, &app).expect("the next item");
    assert_ne!(second, first);
    app.press(cx, "z");
    assert!(app.until(cx, |w| w.try_find(item(&second)).is_none()), "the item was snoozed");
    // the core's own index has both, and when the snooze ends: about a day from now
    let mut states = (String::new(), String::new(), None);
    for _ in 0..600 {
        let index = kept(&app);
        let state = |id: &str| {
            index.items.iter().find(|item| item.id == id).map(|item| (item.state.clone(), item.snoozed_until))
        };
        if let (Some(done), Some(snoozed)) = (state(&first), state(&second)) {
            states = (done.0, snoozed.0, snoozed.1);
        }
        if states.0 == "done" && states.1 == "snoozed" {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!((states.0.as_str(), states.1.as_str()), ("done", "snoozed"));
    let day = 86_400_000;
    let until = states.2.expect("when the snooze ends");
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
    assert!((until - now - day).abs() < 120_000, "a day from now: {} ms off", until - now - day);
    // the window follows the core's index: they stay away after its next read
    cx.run_until_parked();
    assert!(app.until(cx, |w| w.try_find(item(&first)).is_none() && w.try_find(item(&second)).is_none()));
    // Esc closed the card, not the page; once more goes back
    app.press(cx, "escape");
    assert!(app.until(cx, |w| w.try_find("task-dialog").is_none() && w.try_find("work-page").is_some()));
    app.press(cx, "escape");
    assert!(app.until(cx, |w| w.try_find("work-page").is_none()));
    // nothing was written to GitHub or Jira for it: both logs hold only reads
    let (gh, twg) = (fakes.gh_log(), fakes.twg_log());
    assert!(!gh.contains("mutation") && !gh.contains("fake-token"), "{gh}");
    assert!(!twg.contains(" comment add") && !twg.contains(" transition"), "{twg}");
    common::never_wrote(&fakes);
}

/// Shows what is going on in a PR in its own tab, from the same read.
#[gpui_kit::test]
fn shows_what_is_going_on_in_a_pr_in_its_own_tab_from_the_same_read(cx: &mut TestAppContext) {
    let (app, fakes) = on_the_board(cx);
    let reads = fakes.gh_log().lines().count();
    app.click(cx, card("SH-21349"));
    assert!(app.until(cx, |w| w.try_find("task-tab:pr:2350").is_some()), "the card has a tab for its PR");
    app.click(cx, "task-tab:pr:2350");
    assert!(app.until(cx, |w| common::said(w, "pr-details").as_deref() == Some("PR #2350")));
    // as recorded from GitHub (anonymized): its size, the reviewer who asked for changes, who is asked, its threads
    let (size, reviews, asked, threads, body) = cx
        .update_window(app.window, |_, w, cx| {
            w.render_frame(cx);
            let rows =
                |id: &'static str| -> Vec<String> { (0..12usize).filter_map(|n| common::said(w, (id, n))).collect() };
            (
                common::said(w, "pr-size"),
                rows("pr-review"),
                rows("pr-requested"),
                w.try_find("pr-threads").and_then(|e| e.description().map(String::from)),
                common::said(w, "pr-markdown").unwrap_or_default(),
            )
        })
        .unwrap();
    assert_eq!(size.as_deref(), Some("+1,069 −33 in 28 files"));
    assert!(reviews.iter().any(|review| review.contains("Asked for changes")), "{reviews:?}");
    assert!(asked.iter().any(|who| who.starts_with("reviewer-1 ")), "{asked:?}");
    assert_eq!(threads.as_deref(), Some("4 open, 1 resolved"));
    // the recording's fake words: "Text 12 (40 chars)"
    let fake = body.split("Text ").skip(1).any(|rest| {
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        let after = &rest[digits..];
        let counted =
            after.strip_prefix(" (").map_or(0, |count| count.chars().take_while(char::is_ascii_digit).count());
        digits > 0 && counted > 0 && after[2 + counted..].starts_with(" chars)")
    });
    assert!(fake, "{body}");
    // from the same read: opening the card and its tab asked GitHub nothing
    cx.run_until_parked();
    assert_eq!(fakes.gh_log().lines().count(), reads, "the tab read nothing new");
    common::never_wrote(&fakes);
}

/// Item 5.10 (DESIGN.md, "Tell Claude": "What you write is kept until it goes, also across a restart"): an
/// instruction written for an item and an item left out are in `drafts.json` a moment later, and in the card again
/// after the app was quit and opened.
#[gpui_kit::test]
fn what_you_wrote_about_a_card_is_there_again_after_a_restart(cx: &mut TestAppContext) {
    let (app, fakes) = on_the_board(cx);
    app.click(cx, card("SH-21349"));
    assert!(app.until(cx, |w| w.try_find("task-needs").is_some()), "the card says what needs you");
    let first = item_on(cx, &app).expect("the keyboard is on what needs you");
    // an instruction for the first item, and the second left out
    app.press(cx, "n");
    app.typed(cx, "Answer in Dutch.");
    app.press(cx, "escape");
    app.press(cx, "j");
    let second = item_on(cx, &app).expect("the next item");
    assert_ne!(second, first);
    app.press(cx, "x");
    // kept a moment after the last change
    cx.executor().advance_clock(std::time::Duration::from_millis(400));
    let file = app.home.path().join("data").join("drafts.json");
    let mut kept = String::new();
    for _ in 0..400 {
        cx.run_until_parked();
        kept = std::fs::read_to_string(&file).unwrap_or_default();
        if kept.contains("Answer in Dutch.") && kept.contains(r#""include":false"#) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let drafts: serde_json::Value = serde_json::from_str(&kept).unwrap_or_else(|_| panic!("drafts.json: {kept}"));
    assert_eq!(drafts["ticket:SH-21349"]["items"][&first]["note"], "Answer in Dutch.", "{kept}");
    assert_eq!(drafts["ticket:SH-21349"]["items"][&second]["include"], false, "{kept}");
    common::never_wrote(&fakes);
    drop(fakes);

    // quit and open again: the card has your words and the item is still left out
    let app = common::restart_as(cx, app, false);
    cx.simulate_window_resize(app.window, gpui_kit::size(gpui_kit::px(2600.), gpui_kit::px(1600.)));
    assert!(app.until(cx, |w| w.try_find(card("SH-21349")).is_some()), "the board is where you were");
    app.click(cx, card("SH-21349"));
    let left_out = SharedString::from(format!("item-include:{second}"));
    assert!(app.until(cx, |w| w.try_find(left_out.clone()).is_some_and(|e| e.checked() == Some(false))));
    let words = app.shell.read_with(cx, |shell, cx| {
        let drafts = shell.work_page().expect("the Work page").read(cx).drafts().read(cx);
        drafts.of("ticket:SH-21349").note_for(&first).to_string()
    });
    assert_eq!(words, "Answer in Dutch.");
    assert!(app.until(cx, |w| w.try_find(SharedString::from(format!("item-note:{first}"))).is_some()));
}

/// Item 5.12 on the real core: a Claude you linked to a card is on the card, picked; unlinked there, it leaves the
/// card and the core keeps the link undone (so no later read makes it again).
#[gpui_kit::test]
fn a_claude_linked_to_a_card_is_unlinked_from_it_and_the_core_keeps_that(cx: &mut TestAppContext) {
    // a workspace of the project whose branch is no card's
    let app = common::start_on(cx, enkidu_testkit::shop_on_github, |shared, project, _| {
        common::reads_jira(shared, project);
        common::App::make(shared, project, "scratch", "new", "scratch", Some("main"));
    });
    let fakes = app.start_work();
    cx.simulate_window_resize(app.window, gpui_kit::size(gpui_kit::px(2600.), gpui_kit::px(1600.)));
    app.press(cx, "cmd-i");
    app.until_work(cx, |work| !work.checking && work.health.len() == 2 && !work.tickets.is_empty());
    assert!(app.until(cx, |w| w.try_find(card("SH-21349")).is_some()), "the board is on screen");
    let session = app.frame.read_with(cx, |frame, _| frame.workspaces[0].session_id.clone());
    // you link its Claude to SH-21349 (as `enkidu link` or a send does)
    let subjects = vec!["ticket:SH-21349".to_string()];
    enkidu_core::commands::work_link_session(app.sink.clone(), &app.shared, subjects, session.clone()).expect("linked");
    app.click(cx, card("SH-21349"));
    let (row, undo) = (
        SharedString::from(format!("claude-option:{session}")),
        SharedString::from(format!("claude-unlink:{session}")),
    );
    let linked = row.clone();
    assert!(
        app.until(cx, move |w| {
            w.try_find(linked.clone()).is_some_and(|e| {
                e.checked() == Some(true) && e.description().is_some_and(|said| said.starts_with("Linked by you"))
            })
        }),
        "the Claude you linked is on the card, and picked"
    );
    assert!(app.until(cx, |w| common::said(w, "detail-sessions").is_some()));
    // the core's own index names the link (it lists the links that hold)
    let holds = |app: &common::App| {
        let between = |link: &enkidu_core::work::model::Link| {
            link.a == "ticket:SH-21349" && link.b == format!("session:{session}")
        };
        kept(app).links.iter().any(|link| between(link) && !link.removed)
    };
    assert!(holds(&app), "the link you made is in the core's index");
    // not this task's Claude: unlinked, it leaves the card at once
    app.click(cx, undo);
    let gone = row.clone();
    assert!(app.until(cx, move |w| w.try_find(gone.clone()).is_none()), "it left the card");
    // the core keeps the link undone: its index no longer names it, and its next reads leave it so
    let mut undone = false;
    for _ in 0..600 {
        undone = !holds(&app);
        if undone {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(undone, "the core keeps the link undone");
    enkidu_core::commands::work_refresh(&app.shared);
    std::thread::sleep(std::time::Duration::from_millis(300));
    cx.run_until_parked();
    assert!(!holds(&app), "another read didn't make the link again");
    let still_gone = row.clone();
    assert!(app.until(cx, move |w| w.try_find(still_gone.clone()).is_none() && w.try_find("claude-other").is_some()));
    common::never_wrote(&fakes);
    drop(fakes);
}

/// How each card of the Check connections dialog stands: "github true, jira false".
fn checked(cx: &mut TestAppContext, app: &common::App) -> String {
    cx.run_until_parked();
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        let stands: Vec<String> = (0..4usize)
            .filter_map(|at| w.try_find(("check-result", at)).and_then(|e| e.description().map(String::from)))
            .collect();
        stands.join(", ")
    })
    .unwrap()
}

/// Waits until the dialog's cards stand as `want` says, and gives what each says.
fn until_checked(cx: &mut TestAppContext, app: &common::App, want: &str) -> Vec<String> {
    for _ in 0..6000 {
        if checked(cx, app) == want {
            return cx
                .update_window(app.window, |_, w, _| {
                    (0..4usize)
                        .filter_map(|at| w.try_find(("check-result", at)).and_then(|e| e.label().map(String::from)))
                        .collect()
                })
                .unwrap();
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the check never showed {want}; it shows {:?}", checked(cx, app));
}

/// Checks the connections from the Work page, each source broken in turn and recovered (item 5.15 on the whole wiring:
/// the core reads through the fake gh and the fake twg and compares with them directly).
#[gpui_kit::test]
fn checks_the_connections_from_the_work_page_each_source_broken_in_turn_and_recovered(cx: &mut TestAppContext) {
    let (app, fakes) = on_the_board(cx);
    app.click(cx, "work-check");
    assert!(app.until(cx, |w| w.try_find("check-dialog").is_some()), "the dialog opens");
    let said = until_checked(cx, &app, "github true, jira true");
    assert!(said[0].starts_with("GitHub for shop, matches\n"), "{said:?}");
    assert!(said[0].contains("open PRs as `gh pr list --author @me` lists them"), "{said:?}");
    assert!(said[1].starts_with("Jira for shop, matches\n"), "{said:?}");

    fakes.twg("bad-site");
    app.click(cx, "check-again");
    let said = until_checked(cx, &app, "github true, jira false");
    assert!(said[1].starts_with("Jira for shop, can't be read\n"), "{said:?}");
    assert!(said[1].contains("can't find the Jira site"), "{said:?}");

    fakes.twg("ok");
    fakes.gh("rate-limit");
    app.click(cx, "check-again");
    let said = until_checked(cx, &app, "github false, jira true");
    assert!(said[0].contains("rate limit"), "{said:?}");

    fakes.gh("ok");
    app.click(cx, "check-again");
    until_checked(cx, &app, "github true, jira true");
    app.press(cx, "escape");
    assert!(app.until(cx, |w| w.try_find("check-dialog").is_none()), "Esc closes it");
    common::never_wrote(&fakes);
}

/// A source's lamp in the header: "ok", "error" or "reading".
fn lamp(window: &gpui_kit::Window, source: &'static str) -> Option<String> {
    window.try_find(source).and_then(|lamp| lamp.value().map(String::from))
}

/// Presses Refresh and waits until both lamps stand as wanted.
fn refresh_until(cx: &mut TestAppContext, app: &common::App, github: &str, jira: &str) {
    app.click(cx, "work-refresh");
    let stand = |w: &gpui_kit::Window| (lamp(w, "health-github"), lamp(w, "health-jira"));
    let wanted = (Some(github.to_string()), Some(jira.to_string()));
    for _ in 0..3000 {
        cx.run_until_parked();
        let now = cx
            .update_window(app.window, |_, w, cx| {
                w.render_frame(cx);
                stand(w)
            })
            .unwrap();
        if now == wanted {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the lamps never stood as {wanted:?}: {:?}", cx.update_window(app.window, |_, w, _| stand(w)).unwrap());
}

/// Keeps a broken source's data, says what to do, and recovers, for each source (on screen; the same on the store and
/// the model is in tests/work_store.rs).
#[gpui_kit::test]
fn keeps_a_broken_sources_data_says_what_to_do_and_recovers_for_each_source(cx: &mut TestAppContext) {
    let (app, fakes) = on_the_board(cx);
    let on_screen = |cx: &mut TestAppContext, id: &'static str| -> Option<String> {
        cx.run_until_parked();
        cx.update_window(app.window, |_, w, cx| {
            w.render_frame(cx);
            common::said(w, id)
        })
        .unwrap()
    };
    let there = |cx: &mut TestAppContext, id: SharedString| -> bool {
        cx.run_until_parked();
        cx.update_window(app.window, |_, w, cx| {
            w.render_frame(cx);
            w.try_find(id).is_some()
        })
        .unwrap()
    };
    // GitHub signed out: the page says what is wrong and what to do, the tab is marked, and its cards stay
    fakes.gh("logged-out");
    refresh_until(cx, &app, "error", "ok");
    let says = on_screen(cx, "broken-github").unwrap_or_default();
    assert!(says.contains("gh isn't signed in") && says.contains("gh auth login"), "{says}");
    assert!(there(cx, "work-tab-broken".into()));
    assert!(there(cx, card("SH-21349")), "what was read stays on the board");
    fakes.gh("ok");
    refresh_until(cx, &app, "ok", "ok");
    assert!(!there(cx, "broken-github".into()) && !there(cx, "work-tab-broken".into()));

    // Jira signed out
    fakes.twg("logged-out");
    refresh_until(cx, &app, "ok", "error");
    let says = on_screen(cx, "broken-jira").unwrap_or_default();
    assert!(says.contains("twg isn't signed in") && says.contains("twg login --force"), "{says}");
    assert!(there(cx, card("SH-21349")));
    fakes.twg("ok");
    refresh_until(cx, &app, "ok", "ok");
    assert!(!there(cx, "broken-jira".into()));

    // both answer garbage at once, then both come back
    fakes.gh("garbage");
    fakes.twg("garbage");
    refresh_until(cx, &app, "error", "error");
    let (github, jira) =
        (on_screen(cx, "broken-github").unwrap_or_default(), on_screen(cx, "broken-jira").unwrap_or_default());
    assert!(github.contains("isn't valid JSON") && jira.contains("isn't valid JSON"), "{github} | {jira}");
    assert!(there(cx, card("SH-21349")));
    fakes.gh("ok");
    fakes.twg("ok");
    refresh_until(cx, &app, "ok", "ok");
    assert!(!there(cx, "work-broken".into()));
    // nothing was ever written to GitHub or Jira
    common::never_wrote(&fakes);
}
