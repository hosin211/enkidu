//! Item 5.17: the live check of the Work page, read only, with your own `gh` and `twg` (never the fakes): the whole
//! app in a window that isn't on screen, on a core of its own in a temporary folder. Its one project is a throwaway
//! repository whose origin only names your real repository on GitHub; no real folder is opened, nothing is fetched
//! from or pushed to it, and nothing is ever written to GitHub or Jira (the core has no call that could).
//!
//! It runs only when named, and stops with a plain word when either tool is signed out:
//!
//!   ENKIDU_LIVE_REPO=owner/repo ENKIDU_LIVE_JIRA=SH cargo test -p enkidu-app --test work_live -- --ignored --nocapture
//!
//! It prints counts, never a title, a key or a name.

mod common;

use std::time::{Duration, Instant};

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, px, size};

/// Waits up to `seconds` for `seen` on screen.
fn until(cx: &mut TestAppContext, app: &common::App, seconds: u64, seen: impl Fn(&gpui_kit::Window) -> bool) -> bool {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(seconds) {
        cx.run_until_parked();
        let ok = cx
            .update_window(app.window, |_, w, cx| {
                w.render_frame(cx);
                seen(w)
            })
            .unwrap();
        if ok {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

#[gpui_kit::test]
#[ignore = "live: reads GitHub and Jira with your own gh and twg"]
fn live_the_board_matches_what_gh_and_twg_say(cx: &mut TestAppContext) {
    let repo = std::env::var("ENKIDU_LIVE_REPO").expect("ENKIDU_LIVE_REPO=owner/repo, the repository your PRs are in");
    let named = |part: &str| !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
    assert!(repo.split('/').count() == 2 && repo.split('/').all(named), "ENKIDU_LIVE_REPO is owner/repo");
    let keys: Vec<String> = (std::env::var("ENKIDU_LIVE_JIRA").unwrap_or_default().split(','))
        .map(|key| key.trim().to_uppercase())
        .filter(|key| !key.is_empty())
        .collect();
    let jira = !keys.is_empty();

    let origin = format!("https://github.com/{repo}.git");
    let app = common::start_on(
        cx,
        move |home| {
            // a folder of its own: its origin is only read as a name, never fetched from
            let dir = enkidu_testkit::repo(home, "live");
            enkidu_testkit::git(&dir, &["remote", "add", "origin", &origin]);
            dir
        },
        move |shared, project, _| {
            let mut settings = shared.project_settings(project).unwrap();
            settings.work.jira_keys = keys;
            shared.save_project_settings(project, &settings).unwrap();
        },
    );
    cx.simulate_window_resize(app.window, size(px(2600.), px(1400.)));
    // the core's own watcher, on the real gh and twg of your login shell (this test gives it no fakes); only the
    // watcher: nothing here reads a Claude folder
    let started = Instant::now();
    app.start_watcher();
    app.press(cx, "cmd-i");

    // both sources read, as the app reads them
    let wanted = if jira { 2 } else { 1 };
    let mut index = None;
    while started.elapsed() < Duration::from_secs(240) {
        if let Some(work) = app.work(cx)
            && !work.checking
            && work.health.len() == wanted
            && work.health.iter().all(|h| h.last_try.is_some())
        {
            index = Some(work);
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let index = index.expect("GitHub and Jira were not read within four minutes");
    for health in &index.health {
        if health.state != "ok" {
            app.shared.work.stop();
            // signed out, or something else it says in its own words: this is where the item stops
            panic!("LIVE STOP: {} can't be read: {}", health.source, health.error.clone().unwrap_or_default());
        }
    }
    let read_in = started.elapsed().as_secs_f64();
    let open = index.prs.iter().filter(|pr| pr.state == "OPEN").count();
    let waiting = index.items.iter().filter(|item| item.state == "open").count();
    println!(
        "LIVE: read in {read_in:.1} s: {} PRs ({open} open), {} tickets, {} things waiting on you",
        index.prs.len(),
        index.tickets.len(),
        waiting
    );

    // the board shows what was read: one card for each ticket or lone PR, none left out
    assert!(until(cx, &app, 30, |w| w.try_find("work-board").is_some()), "the board never showed");
    let (groups, hidden) = cx.update(|cx| {
        let work = enkidu_app::stores::Stores::work(cx).read(cx);
        let hidden = work.snapshot.as_ref().map(|index| index.hidden.clone()).unwrap_or_default();
        (work.groups.len(), work.groups.iter().filter(|group| hidden.contains(&group.id)).count())
    });
    let (shown, columns) = app.shell.read_with(cx, |shell, cx| {
        let board = shell.work_page().expect("the Work page").read(cx).board();
        (board.shown().len(), board.columns.len())
    });
    println!("LIVE: the board has {shown} cards in {columns} columns ({groups} cards in all, {hidden} of them hidden)");
    assert_eq!(shown, groups - hidden, "every card that isn't hidden is on the board");
    // every open PR that was read is on a card
    let on_cards: usize = cx.update(|cx| {
        let work = enkidu_app::stores::Stores::work(cx).read(cx);
        work.groups.iter().map(|group| group.prs.len()).sum()
    });
    assert_eq!(on_cards, index.prs.len(), "every PR that was read is on a card");

    // Check connections: read again now, then asked of gh and twg directly, field by field
    let checking = Instant::now();
    app.click(cx, "work-check");
    assert!(until(cx, &app, 20, |w| w.try_find("check-dialog").is_some()), "the dialog never opened");
    let done = until(cx, &app, 300, |w| {
        w.try_find("check-dialog").and_then(|dialog| dialog.description().map(String::from)).as_deref()
            == Some("checked")
            && w.try_find(("check-result", 0usize)).is_some()
    });
    assert!(done, "the check never ended");
    let cards: Vec<(String, String)> = cx
        .update_window(app.window, |_, w, _| {
            (0..6usize)
                .filter_map(|at| w.try_find(("check-result", at)))
                .map(|card| {
                    (
                        card.description().map(String::from).unwrap_or_default(),
                        card.label().map(String::from).unwrap_or_default(),
                    )
                })
                .collect()
        })
        .unwrap();
    app.shared.work.stop();
    println!("LIVE: checked in {:.1} s", checking.elapsed().as_secs_f64());
    for (stands, says) in &cards {
        // the first line names the source and how it stands; the lines after it say what was compared
        let mut lines = says.lines();
        let head = lines.next().unwrap_or_default().replace(&repo, "<repo>");
        println!(
            "LIVE: {stands}: {} ({} lines compared or differing)",
            head.replace("live", "<project>"),
            lines.count()
        );
    }
    let differing: Vec<&(String, String)> = cards.iter().filter(|(stands, _)| !stands.ends_with(" true")).collect();
    if !differing.is_empty() {
        // what differs is for the one who runs this, not for a file
        for (stands, says) in &differing {
            eprintln!("LIVE DIFFERS: {stands}\n{says}");
        }
        panic!("LIVE: {} of {} sources don't match", differing.len(), cards.len());
    }
    assert_eq!(cards.len(), wanted, "one card for each source of the project");
}
