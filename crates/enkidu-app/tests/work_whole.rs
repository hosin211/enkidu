//! More runs of the Work page on the whole wiring (item 5.19): the real core with its watcher on the fake `gh` and the
//! fake `twg` (never the real ones, never the network), the window as the app opens it, and the fake Claude in the
//! workspace's terminal, which runs `enkidu` as Claude's own Bash tool would. Each run has its own app.

mod common;

use std::time::Duration;

use common::{App, said};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, SharedString, TestAppContext, px, size};

/// The branch of the fakes' PR #2350, which names SH-21349.
const BRANCH: &str = "feature/SH-21349-change-719";

/// The app on the project the recorded answers are about, with the workspace `sh-21349` on the branch of PR #2350
/// (on screen, its Claude up and waiting at its prompt), and the index read through the fakes.
fn start(cx: &mut TestAppContext) -> (App, common::Fakes) {
    let app = common::start_live(cx, enkidu_testkit::shop_on_github, |shared, project, _| {
        common::reads_jira(shared, project);
        App::make(shared, project, "sh-21349", "new", BRANCH, Some("main"));
    });
    let fakes = app.start_work();
    cx.simulate_window_resize(app.window, size(px(2600.), px(1600.)));
    app.until_claude_started(cx, 1);
    assert!(app.until_terminal(cx, "\n>"), "Claude's prompt");
    app.until_work(cx, |work| !work.checking && work.health.len() == 2 && !work.tickets.is_empty());
    (app, fakes)
}

fn card(key: &str) -> SharedString {
    SharedString::from(format!("work-card:ticket:{key}"))
}

/// Everything a card says (its description; its label is the ticket's key alone).
fn card_says(window: &gpui_kit::Window, key: &str) -> Option<String> {
    window.try_find(card(key)).and_then(|card| card.description().map(String::from))
}

/// The workspace on screen.
fn workspace(cx: &mut TestAppContext, app: &App) -> enkidu_core::db::Workspace {
    app.frame.read_with(cx, |frame, _| frame.current_workspace().cloned()).expect("a workspace on screen")
}

/// Waits (up to a minute) until the terminal on screen holds `words`.
fn until_terminal(cx: &mut TestAppContext, app: &App, words: &str) -> bool {
    for _ in 0..6000 {
        if app.terminal_text(cx).contains(words) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

/// The index as the core's own database has it.
fn kept(app: &App) -> enkidu_core::work::model::Snapshot {
    enkidu_core::work::snapshot(&app.shared).expect("the core's index")
}

/// Reads GitHub and Jira through the fakes, and puts each ticket with its PRs and the sessions behind them.
#[gpui_kit::test]
fn reads_github_and_jira_through_the_fakes_and_puts_each_ticket_with_its_prs_and_the_sessions_behind_them(
    cx: &mut TestAppContext,
) {
    let (app, fakes) = start(cx);
    app.press(cx, "cmd-i");
    assert!(app.until(cx, |w| w.try_find("work-refresh").is_some()), "the Work page");
    app.click(cx, "work-refresh");
    let lamp =
        |w: &gpui_kit::Window, source: &'static str| w.try_find(source).and_then(|e| e.value().map(String::from));
    assert!(
        app.until(cx, |w| {
            lamp(w, "health-github").as_deref() == Some("ok") && lamp(w, "health-jira").as_deref() == Some("ok")
        }),
        "both lamps read"
    );
    // the ticket's card, in its Jira status's column, with its PR and the workspace's Claude on it
    assert!(
        app.until(cx, |w| card_says(w, "SH-21349").is_some_and(|says| says.contains("Claude in sh-21349"))),
        "the workspace's Claude isn't on its ticket's card: {:?}",
        cx.update_window(app.window, |_, w, _| card_says(w, "SH-21349")).unwrap()
    );
    let says = cx.update_window(app.window, |_, w, _| card_says(w, "SH-21349")).unwrap().unwrap_or_default();
    assert!(says.contains("#2350"), "{says}");
    let (column, failing, needs) = app.shell.read_with(cx, |shell, cx| {
        let board = shell.work_page().expect("the Work page").read(cx).board();
        let column = (board.columns.iter())
            .find(|column| column.cards.iter().any(|card| card.id == "ticket:SH-21349"))
            .map(|column| column.name.clone());
        let phrases = board.columns.iter().flat_map(|column| column.cards.iter()).flat_map(|card| card.prs.iter());
        (column, phrases.filter(|pr| pr.phrase.text.contains("Checks failing")).count(), board.needs_count)
    });
    assert_eq!(column.as_deref(), Some("Code Review"));
    // what waits on you is counted, and the red PR of the recorded answers says so on its card
    assert!(needs > 0);
    assert_eq!(failing, 1, "one PR's checks fail");
    // the card opens in front of the board: what the task is (its description, read through twg), Jira's status,
    // the PR and how it stands
    app.click(cx, card("SH-21349"));
    assert!(app.until(cx, |w| w.try_find("task-dialog").is_some()));
    let (task, status, pr) = cx
        .update_window(app.window, |_, w, cx| {
            w.render_frame(cx);
            (
                said(w, "jira-doc").unwrap_or_default(),
                said(w, "task-status").unwrap_or_default(),
                said(w, "task-pr:2350"),
            )
        })
        .unwrap();
    // the recorded descriptions are stand-in words that say their own length: "Text 2 (85 chars)"
    assert!(task.starts_with("Text ") && task.contains(" chars)"), "{task}");
    assert!(status.contains("Code Review"), "{status}");
    assert!(pr.is_some(), "the PR's box");
    app.press(cx, "escape");
    assert!(app.until(cx, |w| w.try_find("task-dialog").is_none()));
    let line = cx.update_window(app.window, |_, w, _| said(w, "work-status-line")).unwrap().unwrap_or_default();
    assert!(line.contains("GitHub read") && line.contains("as me-dev"), "{line}");
    assert!(line.contains("Jira read") && line.contains("as Me Dev"), "{line}");
    // back to the workspace: its tab carries its PR, the status line what waits there
    app.press(cx, "cmd-i");
    assert!(app.until(cx, |w| w.try_find(("tab-pr", 0usize)).is_some()), "the tab's PR");
    let chip = cx.update_window(app.window, |_, w, _| said(w, ("tab-pr", 0usize))).unwrap().unwrap_or_default();
    assert!(chip.contains("PR #2350"), "{chip}");
    common::never_wrote(&fakes);
}

/// Tells a starting Claude about its PR and ticket.
#[gpui_kit::test]
fn tells_a_starting_claude_about_its_pr_and_ticket(cx: &mut TestAppContext) {
    let (app, fakes) = start(cx);
    // a turn, so there is a conversation to come back to; then Claude ends
    app.type_in_terminal(cx, "add a test\r");
    assert!(until_terminal(cx, &app, "Done."));
    app.type_in_terminal(cx, "exit\r");
    assert!(app.until(cx, |w| w.try_find("restart-terminal").is_some()));
    // Claude starts again in the workspace (it resumes) and hears what Enkidu knows about its work
    app.click(cx, "restart-terminal");
    app.until_claude_started(cx, 2);
    assert!(until_terminal(cx, &app, "PR #2350"), "the session context never arrived: {}", app.terminal_text(cx));
    assert!(app.terminal_text(cx).contains("SH-21349"));
    common::never_wrote(&fakes);
}

/// Links the PR Claude opens to its session.
#[gpui_kit::test]
fn links_the_pr_claude_opens_to_its_session(cx: &mut TestAppContext) {
    let (app, fakes) = start(cx);
    let session = workspace(cx, &app).session_id;
    app.type_in_terminal(cx, "pr 2329\r");
    assert!(until_terminal(cx, &app, "pull/2329"), "{}", app.terminal_text(cx));
    app.press(cx, "cmd-i");
    assert!(app.until(cx, |w| w.try_find(card("SH-20844")).is_some()), "the board shows SH-20844");
    app.click(cx, card("SH-20844"));
    // linked: on the card, to pick; not picked by itself, as its workspace is on SH-21349's branch
    let made = SharedString::from(format!("claude-option:{session}"));
    let row = made.clone();
    assert!(
        app.until(cx, move |w| w.try_find(row.clone()).is_some()),
        "the PR Claude made isn't linked to its session"
    );
    let (says, picked, new_picked) = cx
        .update_window(app.window, |_, w, _| {
            let option = w.find(made.clone());
            (
                option.description().map(String::from).unwrap_or_default(),
                option.checked(),
                w.find("claude-option:new").checked(),
            )
        })
        .unwrap();
    assert!(says.contains("Created it"), "{says}");
    assert_eq!((picked, new_picked), (Some(false), Some(true)));
    app.press(cx, "escape");
    common::never_wrote(&fakes);
}

/// Runs enkidu inside Claude: its status, a link and an unlink, a note, and the live check.
#[gpui_kit::test]
fn runs_enkidu_inside_claude_its_status_a_link_and_an_unlink_a_note_and_the_live_check(cx: &mut TestAppContext) {
    let (app, fakes) = start(cx);
    let session = format!("session:{}", workspace(cx, &app).session_id);
    // (its answer is longer than the terminal is tall: the end of it is read, where its PR's branch and its ticket
    // still stand)
    app.type_in_terminal(cx, "enkidu status --json\r");
    assert!(
        until_terminal(cx, &app, "\"key\": \"SH-21349\""),
        "enkidu status never answered: {}",
        app.terminal_text(cx)
    );
    assert!(app.terminal_text(cx).contains("\"head\": \"feature/SH-21349-change-719\""), "its PR");

    app.type_in_terminal(cx, "enkidu link pr 2265\r");
    assert!(until_terminal(cx, &app, "Linked pr:example-org/shop#2265"), "{}", app.terminal_text(cx));
    let linked = |app: &App| {
        (kept(app).links.into_iter()).find(|l| l.a == "pr:example-org/shop#2265" && l.b == session && !l.removed)
    };
    assert_eq!(linked(&app).map(|link| link.how).as_deref(), Some("claude"));
    // the Work page shows the session under that PR's ticket
    app.press(cx, "cmd-i");
    assert!(
        app.until(cx, |w| card_says(w, "SH-20890").is_some_and(|says| says.contains("sh-21349"))),
        "the linked session isn't on its ticket's card"
    );
    app.press(cx, "cmd-i");

    app.type_in_terminal(cx, "enkidu unlink pr 2265\r");
    assert!(until_terminal(cx, &app, "Unlinked pr:example-org/shop#2265"), "{}", app.terminal_text(cx));
    assert!(linked(&app).is_none());

    app.type_in_terminal(cx, "enkidu note \"QA asked for a second look\"\r");
    assert!(until_terminal(cx, &app, "Noted on SH-21349"), "{}", app.terminal_text(cx));
    let noted = |e: &enkidu_core::work::model::Event| {
        e.subject == "SH-21349" && e.kind == "note" && e.text == "QA asked for a second look"
    };
    assert!(kept(&app).events.iter().any(noted));

    // the live check, from Claude's terminal: both sources read now and compared with gh and twg
    app.type_in_terminal(cx, "enkidu doctor\r");
    assert!(until_terminal(cx, &app, "Jira for shop"), "enkidu doctor never finished: {}", app.terminal_text(cx));
    let text = app.terminal_text(cx);
    assert!(text.contains("✓ GitHub for shop") && text.contains("✓ Jira for shop"), "{text}");
    assert!(!text.contains('✗'), "{text}");
    common::never_wrote(&fakes);
}

/// Turns Live on and off from Claude's terminal with enkidu live.
#[gpui_kit::test]
fn turns_live_on_and_off_from_claudes_terminal_with_enkidu_live(cx: &mut TestAppContext) {
    let (app, fakes) = start(cx);
    let ws = workspace(cx, &app);
    let live = |app: &App| -> Option<String> {
        app.shared.db.project(&app.project_id).ok().flatten().and_then(|project| project.live_workspace_id)
    };
    let until_live = |cx: &mut TestAppContext, app: &App, wanted: Option<&str>| -> bool {
        for _ in 0..3000 {
            cx.run_until_parked();
            if live(app).as_deref() == wanted {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    };
    assert_eq!(live(&app), None);
    app.type_in_terminal(cx, "enkidu live on\r");
    assert!(until_live(cx, &app, Some(&ws.id)), "enkidu live on never made it Live: {}", app.terminal_text(cx));
    // the window follows: its Live button says so
    assert!(app.until_frame(cx, |frame| frame.projects.iter().any(|p| p.live_workspace_id.as_deref() == Some(&ws.id))));
    app.type_in_terminal(cx, "enkidu live\r");
    assert!(until_terminal(cx, &app, "sh-21349 (this workspace) is Live."), "{}", app.terminal_text(cx));
    app.type_in_terminal(cx, "enkidu live off\r");
    assert!(until_live(cx, &app, None), "enkidu live off never put the main folder back: {}", app.terminal_text(cx));
    assert!(app.until_frame(cx, |frame| frame.projects.iter().all(|p| p.live_workspace_id.is_none())));
    common::never_wrote(&fakes);
}

/// A throwaway Claude folder for this test binary, named for the core (`ENKIDU_CLAUDE_DIRS`): no test here reads or
/// changes a real Claude folder. The event log is read five times a second (the app reads it once a minute).
fn claude_folder() -> std::path::PathBuf {
    static NAMED: std::sync::Once = std::sync::Once::new();
    let folder = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("work-whole-claude-folder");
    NAMED.call_once(|| {
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(folder.join("projects")).unwrap();
        // SAFETY: set once, to the same values, before the one test that reads them starts its core's readers.
        unsafe {
            std::env::set_var("ENKIDU_CLAUDE_DIRS", folder.display().to_string());
            std::env::set_var("ENKIDU_SCAN_EVERY_MS", "200");
        }
    });
    folder
}

/// Adds the hooks to a Claude folder from Settings; a Claude outside Enkidu is linked, then brought in to answer. Here:
/// the hooks added from Settings, a Claude started outside the app with them (it runs this app's own `enkidu event`),
/// the app reading its event log and linking that session to the PR it opened, the session offered on the PR's card,
/// and the hooks taken out again. Bringing such a session into a workspace to answer is tests/work_send.rs,
/// `brings_a_session_from_outside_into_a_new_workspace_on_its_prs_branch`, which starts from the same link.
#[gpui_kit::test]
fn adds_the_hooks_from_settings_and_a_claude_outside_the_app_is_linked_to_the_pr_it_opens(cx: &mut TestAppContext) {
    use enkidu_core::work::claudehooks::is_added;
    let folder = claude_folder();
    let file = folder.join("settings.json");
    let read = |path: &std::path::Path| -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap_or_else(|_| "{}".into())).unwrap()
    };
    let (app, fakes) = start(cx);
    // what Claude outside the app records is read by the core's own reader (the folder's saved sessions too: none)
    enkidu_core::work::start(app.sink.clone(), app.shared.clone());

    // Settings, Claude: the folder's row, then Add hooks
    let toggle = |cx: &mut TestAppContext, app: &App| {
        app.press(cx, "cmd-,");
        assert!(app.until(cx, |w| w.try_find(("settings-nav", 1usize)).is_some()));
        app.click(cx, ("settings-nav", 1usize));
        assert!(app.until(cx, |w| w.try_find(("claude-hooks-toggle", 0usize)).is_some()), "the Claude folder's row");
        cx.update_window(app.window, |_, w, cx| {
            w.render_frame(cx);
            w.scroll("settings-body", gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-3000.))), cx);
            w.render_frame(cx);
            w.click(("claude-hooks-toggle", 0usize), cx);
        })
        .unwrap();
    };
    let close = |cx: &mut TestAppContext, app: &App| {
        app.click(cx, "settings-cancel");
        assert!(app.until(cx, |w| w.try_find("settings-dialog").is_none()));
    };
    assert!(!is_added(&read(&file)));
    toggle(cx, &app);
    let mut added = false;
    for _ in 0..1000 {
        cx.run_until_parked();
        if is_added(&read(&file)) {
            added = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(added, "the hooks never came into {}", file.display());
    let settings = read(&file);
    let mut events: Vec<&str> = settings["hooks"].as_object().expect("hooks").keys().map(String::as_str).collect();
    events.sort();
    assert_eq!(events, ["PostToolUse", "SessionEnd", "SessionStart", "Stop", "UserPromptSubmit"]);
    let command = settings["hooks"]["SessionStart"][0]["hooks"][0]["command"].as_str().unwrap_or_default();
    // (the build folder may have any name, or be a link: only its end is looked at)
    assert!(command.contains("/debug/enkidu'") && command.ends_with(" event --app enkidu"), "{command}");
    close(cx, &app);

    // Claude started outside the app (in another terminal, say), in the main folder, with those settings: it opens
    // PR #2177
    let fake = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../enkidu-testkit/fakes/fake-claude.mjs");
    let mut outside = std::process::Command::new("node")
        .arg(&fake)
        .args(["--settings", &file.to_string_lossy(), "--session-id", "outside-1"])
        .current_dir(&app.project)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", app.home.path())
        .env("CLAUDE_CONFIG_DIR", &folder)
        .env("ENKIDU_DATA_DIR", &app.shared.data_dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("node runs the fake Claude");
    {
        use std::io::Write as _;
        outside.stdin.take().expect("its input").write_all(b"pr 2177\nexit\n").unwrap();
    }
    let ended = outside.wait_with_output().expect("the fake Claude ends");
    let printed = String::from_utf8_lossy(&ended.stdout).to_string();
    assert!(ended.status.success(), "{printed}\n{}", String::from_utf8_lossy(&ended.stderr));
    assert!(printed.contains("pull/2177"), "{printed}");

    // the app reads its event log and links the session to the PR it made
    let linked = |app: &App| {
        (kept(app).links.iter()).any(|l| l.a == "pr:example-org/shop#2177" && l.b == "session:outside-1" && !l.removed)
    };
    let mut seen = false;
    for _ in 0..2000 {
        cx.run_until_parked();
        if linked(&app) {
            seen = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(seen, "the outside session was never linked to its PR");
    // on that PR's card it is there to pick, as the Claude that created it
    app.press(cx, "cmd-i");
    let mut on_card = None;
    for _ in 0..1000 {
        cx.run_until_parked();
        on_card = cx.update(|cx| {
            let work = enkidu_app::stores::Stores::work(cx).read(cx);
            let index = work.snapshot.as_ref()?;
            let group = work.groups.iter().find(|group| group.prs.iter().any(|pr| pr.of(index).number == 2177))?;
            group.sessions.iter().any(|session| session.of(index).id == "outside-1").then(|| group.id.clone())
        });
        if on_card.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let id = on_card.expect("the session is on the card of PR #2177");
    let page = app.shell.read_with(cx, |shell, _| shell.work_page().expect("the Work page").clone());
    cx.update_window(app.window, |_, w, cx| page.update(cx, |page, cx| page.open(&id, w, cx))).unwrap();
    assert!(
        app.until(cx, |w| w.try_find("claude-option:outside-1").is_some()),
        "the session isn't offered on its card"
    );
    let says = cx
        .update_window(app.window, |_, w, _| w.find("claude-option:outside-1").description().map(String::from))
        .unwrap()
        .unwrap_or_default();
    assert!(says.contains("Created it"), "{says}");
    app.press(cx, "escape");
    assert!(app.until(cx, |w| w.try_find("task-dialog").is_none()));
    app.press(cx, "cmd-i");

    // the hooks come out again the same way
    toggle(cx, &app);
    let mut removed = false;
    for _ in 0..1000 {
        cx.run_until_parked();
        if !is_added(&read(&file)) {
            removed = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(removed, "the hooks never left {}", file.display());
    assert!(read(&file).get("hooks").is_none(), "{}", read(&file));
    close(cx, &app);
    common::never_wrote(&fakes);
}

/// Presses Refresh on the Work page and waits until both sources were tried again; gives the index then.
fn refresh(cx: &mut TestAppContext, app: &App) -> std::sync::Arc<enkidu_core::work::model::Snapshot> {
    // a read is told apart from the one before by its time, in milliseconds
    std::thread::sleep(Duration::from_millis(5));
    let since = enkidu_core::db::now_ms();
    app.click(cx, "work-refresh");
    app.until_work(cx, |work| !work.checking && work.health.iter().all(|h| h.last_try.unwrap_or(0) >= since))
}

/// Sends new failing checks to the Claude in the PR's workspace once turned on, once per commit, never on a failed read
/// (the window's side of the workflow is `enkidu_app::work::queued`; its three store tests are enkidu-model's
/// `work/queued.rs`).
#[gpui_kit::test]
fn sends_new_failing_checks_to_the_claude_in_the_prs_workspace_once_turned_on_once_per_commit_never_on_a_failed_read(
    cx: &mut TestAppContext,
) {
    const SAYS: &str = "Checks are failing on the latest commit of PR #2350";
    let (app, fakes) = start(cx);
    let ws = workspace(cx, &app);
    // what reached that Claude (sent with Enter), and what stands in its terminal (typed in, sent or not)
    let sent = |app: &App| -> usize {
        let log = format!("{}.prompts", app.claude_log.display());
        (std::fs::read_to_string(log).unwrap_or_default().lines())
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .filter(|said| said["sessionId"] == ws.session_id.as_str())
            .filter(|said| said["prompt"].as_str().is_some_and(|prompt| prompt.contains(SAYS)))
            .count()
    };
    let terminal = |cx: &mut TestAppContext| {
        cx.update(|cx| cx.global::<enkidu_app::stores::Stores>().terminals.read(cx).active_view(&ws.id))
            .expect("the workspace's terminal")
    };
    let typed = |cx: &mut TestAppContext| -> usize {
        cx.run_until_parked();
        let view = terminal(cx);
        view.read_with(cx, |view, _| view.terminal().screen().text()).matches(SAYS).count()
    };
    let until_typed = |cx: &mut TestAppContext, count: usize| -> bool {
        for _ in 0..3000 {
            if typed(cx) == count {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    };
    let enter = |cx: &mut TestAppContext| {
        let view = terminal(cx);
        view.read_with(cx, |view, _| view.terminal().send(b"\r".to_vec()));
    };
    let until_sent = |cx: &mut TestAppContext, app: &App, count: usize| -> bool {
        for _ in 0..3000 {
            cx.run_until_parked();
            if sent(app) == count {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    };
    let failing = |work: &enkidu_core::work::model::Snapshot| {
        (work.items.iter()).any(|i| {
            i.pr.as_deref() == Some("example-org/shop#2350") && i.state == "open" && i.id.starts_with("checks:")
        })
    };
    let runs = |app: &App| app.shared.db.work_workflow_runs(&app.project_id, 8).unwrap_or_default();
    let quiet = |cx: &mut TestAppContext| {
        // long enough for a send that shouldn't happen to happen
        for _ in 0..40 {
            cx.run_until_parked();
            std::thread::sleep(Duration::from_millis(20));
        }
    };

    // off by default: a push whose pipeline failed only shows on the page
    fakes.gh_fail("2350=e2e0000000000000000000000000000000000001");
    app.press(cx, "cmd-i");
    assert!(app.until(cx, |w| w.try_find("work-refresh").is_some()), "the Work page");
    assert!(failing(&refresh(cx, &app)), "the failure never showed");
    quiet(cx);
    assert_eq!((typed(cx), sent(&app)), (0, 0));

    // turned on in the project's settings; the failure already there isn't new, so it stays where it is
    // (Settings by its key, then the project's page: the Work page stays on screen behind it)
    app.press(cx, "cmd-,");
    assert!(app.until(cx, |w| w.try_find(("settings-nav", 3usize)).is_some()));
    app.click(cx, ("settings-nav", 3usize));
    assert!(app.until(cx, |w| w.try_find("default-base").is_some()), "the project's page never showed");
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        w.scroll("settings-body", gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-6000.))), cx);
        w.render_frame(cx);
        w.click("work-send-failed", cx);
    })
    .unwrap();
    app.click(cx, "save-settings");
    assert!(app.until(cx, |w| w.try_find("settings-dialog").is_none()), "Save closes Settings");
    assert!(app.shared.project_settings(&app.project_id).unwrap().work.send_failed_checks);
    refresh(cx, &app);
    quiet(cx);
    assert_eq!((typed(cx), sent(&app)), (0, 0), "a failure that was there before isn't new");

    // the next push fails: typed into the prompt of the Claude in its workspace, never sent, and the run says so
    fakes.gh_fail("2350=e2e0000000000000000000000000000000000002");
    refresh(cx, &app);
    assert!(until_typed(cx, 1), "the failure never reached the prompt of the workspace's Claude");
    let mut done = None;
    for _ in 0..2000 {
        cx.run_until_parked();
        done = runs(&app).into_iter().find(|run| run.workflow == "failed_checks" && run.state == "done");
        if done.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(done.map(|run| run.text).as_deref(), Some("In Claude's prompt in sh-21349, for you to send."));
    // a workflow never takes you away, and never sends
    assert!(app.until(cx, |w| w.try_find("work-page").is_some()), "the Work page stays on screen");
    quiet(cx);
    assert_eq!(sent(&app), 0, "nothing is sent until you press Enter");
    // it says so on the item's timeline and in a toast
    let noted = kept(&app).events.iter().any(|e| e.kind == "note" && e.text.contains("by the failing-checks workflow"));
    assert!(noted, "{:?}", kept(&app).events.first());
    let toasts: Vec<String> = cx
        .update_window(app.window, |_, w, cx| {
            w.render_frame(cx);
            w.find_all("toast-detail").iter().filter_map(|e| e.label().map(String::from)).collect()
        })
        .unwrap();
    assert!(toasts.iter().any(|detail| detail.contains("the failing-checks workflow typed it in")), "{toasts:?}");
    // you press Enter there: it goes
    enter(cx);
    assert!(until_sent(cx, &app, 1), "the failure never reached the workspace's Claude");
    // read again: not typed in twice
    refresh(cx, &app);
    quiet(cx);
    assert_eq!(typed(cx), 1);

    // a failed read decides nothing; the next good read finds the new failure and sends it
    fakes.gh("logged-out");
    fakes.gh_fail("2350=e2e0000000000000000000000000000000000003");
    let broken = refresh(cx, &app);
    assert!(broken.health.iter().any(|h| h.source == "github" && h.state == "error"));
    quiet(cx);
    assert_eq!(typed(cx), 1);
    fakes.gh("ok");
    let read = refresh(cx, &app);
    assert!(read.health.iter().all(|h| h.state == "ok"), "{:?}", read.health);
    assert!(until_typed(cx, 2), "the failure after the good read never reached Claude's prompt");
    enter(cx);
    assert!(until_sent(cx, &app, 2), "the second failure was never sent");
    fakes.gh_fail("");
    common::never_wrote(&fakes);
}

/// Scrolls Settings' page until the field is whole inside it, clicks it, and puts `text` in place of what it has.
fn fill(cx: &mut TestAppContext, app: &App, id: &'static str, text: &str) {
    for _ in 0..80 {
        cx.run_until_parked();
        let way = cx
            .update_window(app.window, |_, w, cx| {
                w.render_frame(cx);
                let body = w.find("settings-body").bounds();
                let field = w.find(id).bounds();
                if field.top() < body.top() {
                    1.
                } else if field.bottom() > body.bottom() {
                    -1.
                } else {
                    0.
                }
            })
            .unwrap();
        if way == 0. {
            break;
        }
        let by = gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(60. * way)));
        cx.update_window(app.window, |_, w, cx| w.scroll("settings-body", by, cx)).unwrap();
    }
    app.click(cx, id);
    app.press(cx, "cmd-a");
    app.typed(cx, text);
    cx.run_until_parked();
}

/// Writes the morning summary at its time, once a day, with a read-only Claude that ignores the project's own flags.
#[gpui_kit::test]
fn writes_the_morning_summary_at_its_time_once_a_day_with_a_read_only_claude_that_ignores_the_projects_own_flags(
    cx: &mut TestAppContext,
) {
    let (app, fakes) = start(cx);
    let printed = |app: &App| -> Vec<serde_json::Value> {
        let print = |start: &serde_json::Value| {
            start["argv"].as_array().is_some_and(|argv| argv.iter().any(|arg| arg == "-p" || arg == "--print"))
        };
        app.claude_starts().into_iter().filter(print).collect()
    };
    let before = printed(&app).len();
    let own = app.claude_starts()[0]["env"]["CLAUDE_CONFIG_DIR"].clone();
    // the project's own Claude flags skip permissions; the summary must never use them
    app.press(cx, "cmd-,");
    assert!(app.until(cx, |w| w.try_find(("settings-nav", 3usize)).is_some()));
    app.click(cx, ("settings-nav", 3usize));
    assert!(app.until(cx, |w| w.try_find("default-base").is_some()), "the project's page never showed");
    fill(cx, &app, "project-claude-args", "--dangerously-skip-permissions --verbose");
    // always past: due at once
    fill(cx, &app, "work-morning-at", "00:00");
    app.click(cx, "work-morning");
    app.click(cx, "save-settings");
    assert!(app.until(cx, |w| w.try_find("settings-dialog").is_none()), "Save closes Settings");
    let saved = app.shared.project_settings(&app.project_id).unwrap();
    assert_eq!(
        (saved.work.morning_summary, saved.work.morning_at.as_str(), saved.claude.args[0].as_str()),
        (true, "00:00", "--dangerously-skip-permissions")
    );

    // at its time (or the first time the app runs after it): the core's own clock starts it, once
    let today = enkidu_core::work::workflows::local_now().0;
    let runs = |app: &App| -> Vec<enkidu_core::work::workflows::Run> {
        (app.shared.db.work_workflow_runs(&app.project_id, 8).unwrap_or_default().into_iter())
            .filter(|run| run.workflow == "morning" && run.subject == today)
            .collect()
    };
    let mut ran = None;
    for _ in 0..6000 {
        cx.run_until_parked();
        ran = runs(&app).into_iter().find(|run| run.state == "done");
        if ran.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let ran = ran.unwrap_or_else(|| panic!("the morning summary never ran: {:?}", runs(&app)));
    let tickets = kept(&app).tickets.len();
    assert!(tickets > 0);
    assert_eq!(
        ran.text,
        format!("Claude wrote {tickets} lines on the tickets' timelines. Summarized {tickets} tickets.")
    );

    // how Claude ran: print mode in the main folder, restricted, only Bash and the read tools, enkidu allowed, none of
    // your flags
    let prints = printed(&app);
    assert_eq!(prints.len(), before + 1);
    let claude = prints.last().unwrap();
    assert_eq!(claude["cwd"].as_str(), Some(app.project.to_str().unwrap()));
    let argv: Vec<&str> = claude["argv"].as_array().unwrap().iter().filter_map(|arg| arg.as_str()).collect();
    let after = |flag: &str| argv.iter().position(|arg| *arg == flag).and_then(|at| argv.get(at + 1)).copied();
    for flag in ["--restricted", "--strict-mcp-config", "--no-session-persistence"] {
        assert!(argv.contains(&flag), "{flag} in {argv:?}");
    }
    assert_eq!(after("--tools"), Some("Bash,Read,Grep,Glob"));
    assert!(after("--allowedTools").is_some_and(|allowed| allowed.split(',').any(|tool| tool == "Bash(enkidu:*)")));
    for flag in ["--dangerously-skip-permissions", "--verbose", "--model", "--settings", "--resume", "--session-id"] {
        assert!(!argv.contains(&flag), "{flag} must not be in {argv:?}");
    }
    let env = &claude["env"];
    assert!(!own.is_null() && env["CLAUDE_CONFIG_DIR"] == own, "the same Claude account as your own Claude's");
    assert_eq!((env["ENKIDU_TOKEN"].as_str(), env["ENKIDU_WORKFLOW"].as_str()), (Some("(set)"), Some("morning")));
    assert!(env.get("ENKIDU_SESSION").is_none(), "it is no workspace's session");
    // one line per ticket, on its timeline
    let lines = (kept(&app).events.iter())
        .filter(|e| e.kind == "note" && e.text.ends_with("summarized by the fake Claude"))
        .count();
    assert_eq!(lines, tickets);
    // the window said so. Its toast follows the core's news, which is sent after the run is kept as done (what was
    // waited for above), so it is waited for: one look at once missed it on 15 Nov, with every test running at a time
    let toasts = |w: &gpui_kit::Window| -> Vec<String> {
        w.find_all("toast").iter().filter_map(|e| e.label().map(String::from)).collect()
    };
    assert!(
        app.until(cx, |w| toasts(w).iter().any(|title| title == "Morning summary written")),
        "{:?}",
        cx.update_window(app.window, |_, w, _| toasts(w)).unwrap()
    );

    // once a day: the core's next looks (each read is one) don't run it again
    for _ in 0..2 {
        std::thread::sleep(Duration::from_millis(5));
        let since = enkidu_core::db::now_ms();
        cx.update(enkidu_app::work::store::Work::refresh);
        app.until_work(cx, |work| !work.checking && work.health.iter().all(|h| h.last_try.unwrap_or(0) >= since));
        std::thread::sleep(Duration::from_millis(400));
        cx.run_until_parked();
    }
    assert_eq!(runs(&app).len(), 1, "{:?}", runs(&app));
    assert_eq!(printed(&app).len(), before + 1);
    common::never_wrote(&fakes);
}
