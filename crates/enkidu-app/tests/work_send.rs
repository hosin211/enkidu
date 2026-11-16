//! "Tell Claude" on the whole wiring (items 5.13 and 5.18b): the real core with its watcher on the fake `gh` and the
//! fake `twg`, the window as the app opens it, and the fake Claude in each workspace's terminal, which records every
//! prompt that was sent to it. A card's message goes into a Claude's prompt and is never sent: you press Enter there.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{App, said};
use enkidu_core::state::Shared;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, SharedString, TestAppContext, px, size};

/// The branch of the fakes' PR #2350, which names SH-21349.
const BRANCH: &str = "feature/SH-21349-change-719";

/// The prompts that were sent to a fake Claude, oldest first.
fn sent(app: &App) -> Vec<String> {
    let log = format!("{}.prompts", app.claude_log.display());
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter_map(|said| said["prompt"].as_str().map(String::from))
        .collect()
}

fn until_sent(cx: &mut TestAppContext, app: &App, count: usize) -> Vec<String> {
    for _ in 0..800 {
        cx.run_until_parked();
        if sent(app).len() >= count {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    sent(app)
}

/// The app on the project the recorded answers are about, with a workspace on the branch of SH-21349's PR (on
/// screen, its Claude up and waiting at its prompt), whatever `more` makes, and the index read through the fakes.
fn start(cx: &mut TestAppContext, more: impl FnOnce(&Arc<Shared>, &str)) -> (App, common::Fakes) {
    let app = common::start_live(cx, enkidu_testkit::shop_on_github, |shared, project, _| {
        common::reads_jira(shared, project);
        App::make(shared, project, "sh-21349", "new", BRANCH, Some("main"));
        more(shared, project);
    });
    let fakes = app.start_work();
    cx.simulate_window_resize(app.window, size(px(2600.), px(1600.)));
    app.until_claude_started(cx, 1);
    assert!(app.until_terminal(cx, "\n>"), "Claude's prompt");
    (app, fakes)
}

fn card(key: &str) -> SharedString {
    SharedString::from(format!("work-card:ticket:{key}"))
}

/// Opens the Work page and a ticket's card in front of its board.
fn open_card(cx: &mut TestAppContext, app: &App, key: &str) {
    app.press(cx, "cmd-i");
    app.until_work(cx, |work| !work.checking && work.health.len() == 2 && !work.tickets.is_empty());
    assert!(app.until(cx, |w| w.try_find(card(key)).is_some()), "the board shows {key}");
    app.click(cx, card(key));
    assert!(app.until(cx, |w| w.try_find("task-composer").is_some()), "the card of {key} is open");
}

/// Where the open card's message goes, in words, and what its button says.
fn goes(cx: &mut TestAppContext, app: &App) -> (String, String) {
    cx.run_until_parked();
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        (said(w, "task-target").unwrap_or_default(), said(w, "task-send").unwrap_or_default())
    })
    .unwrap()
}

/// What the last toast says: its title and the line under it.
fn toast(cx: &mut TestAppContext, app: &App) -> (Option<String>, Option<String>) {
    cx.run_until_parked();
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        let titles: Vec<String> = w.find_all("toast").iter().filter_map(|e| e.label().map(String::from)).collect();
        let details: Vec<String> =
            w.find_all("toast-detail").iter().filter_map(|e| e.label().map(String::from)).collect();
        (titles.last().cloned(), details.last().cloned())
    })
    .unwrap()
}

/// Waits for a toast with this title (others may still be on screen) and gives the line under it.
fn until_toast(cx: &mut TestAppContext, app: &App, title: &str) -> Option<String> {
    for _ in 0..800 {
        cx.run_until_parked();
        let found = cx
            .update_window(app.window, |_, w, cx| {
                w.render_frame(cx);
                let titles: Vec<String> =
                    w.find_all("toast").iter().filter_map(|e| e.label().map(String::from)).collect();
                let details: Vec<String> =
                    w.find_all("toast-detail").iter().filter_map(|e| e.label().map(String::from)).collect();
                titles.iter().position(|said| said == title).map(|at| details.get(at).cloned())
            })
            .unwrap();
        if let Some(detail) = found {
            return detail;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("no toast says {title:?}; the last one says {:?}", toast(cx, app));
}

/// The index as the core's own database has it.
fn kept(app: &App) -> enkidu_core::work::model::Snapshot {
    enkidu_core::work::snapshot(&app.shared).expect("the core's index")
}

/// What is kept of a card's draft.
fn draft(cx: &mut TestAppContext, app: &App, card: &str) -> enkidu_model::work::compose::Draft {
    cx.run_until_parked();
    app.shell.read_with(cx, |shell, cx| shell.work_page().expect("the Work page").read(cx).drafts().read(cx).of(card))
}

/// Send to Claude: "types into the prompt of the Claude running where the work is, never presses Enter, takes you
/// there, and notes it"; and "sends what waits on a PR to the Claude running in its workspace".
#[gpui_kit::test]
fn types_into_the_prompt_of_the_claude_running_where_the_work_is_never_presses_enter_takes_you_there_and_notes_it(
    cx: &mut TestAppContext,
) {
    let (app, fakes) = start(cx, |_, _| {});
    let session = app.frame.read_with(cx, |frame, _| frame.workspaces[0].session_id.clone());
    open_card(cx, &app, "SH-21349");
    assert_eq!(
        goes(cx, &app),
        (
            "goes into the prompt of the Claude in sh-21349; you press Enter there".to_string(),
            "Open in Claude".to_string()
        )
    );
    // your words about the task, and an instruction for the first thing that needs you
    app.press(cx, "s");
    app.typed(cx, "Finish this before the release.");
    app.press(cx, "escape");
    app.press(cx, "n");
    app.typed(cx, "Keep the old name as an alias.");
    app.press(cx, "escape");
    let starts = app.claude_starts().len();
    app.click(cx, "task-send");
    // you are taken to that Claude: the card and the page are gone, its workspace is on screen
    assert!(app.until(cx, |w| w.try_find("work-page").is_none() && w.try_find("task-dialog").is_none()));
    assert_eq!(app.on_screen(cx).map(|(name, ..)| name).as_deref(), Some("sh-21349"));
    // in Claude's prompt as one paste, not sent: nothing reached Claude until you press Enter
    assert!(app.until_terminal(cx, "Finish this before the release."), "the message is in Claude's prompt");
    std::thread::sleep(Duration::from_millis(400));
    cx.run_until_parked();
    assert!(sent(&app).is_empty(), "nothing was sent: {:?}", sent(&app));
    assert_eq!(app.claude_starts().len(), starts, "the Claude that ran there got it: none was started");
    assert_eq!(
        toast(cx, &app),
        (Some("In Claude's prompt in sh-21349".to_string()), Some("Press Enter there to send it.".to_string()))
    );
    // the keyboard is on that Claude: Enter, pressed by you, sends it
    app.press(cx, "enter");
    let prompts = until_sent(cx, &app, 1);
    assert_eq!(prompts.len(), 1, "one message, sent once");
    let prompt = &prompts[0];
    // the card's links, your words, and your instruction for the item
    for line in [
        "SH-21349:",
        "PR #2350",
        "\n\nFinish this before the release.\n\nAddress ",
        "   What to do: Keep the old name as an alias.",
        "don't post anything to GitHub or Jira",
    ] {
        assert!(prompt.contains(line), "{line:?} is missing from {prompt}");
    }
    // the card's timeline says where it went, and the card is linked to that Claude by you
    let mut noted = false;
    for _ in 0..600 {
        let index = kept(&app);
        let said = |event: &enkidu_core::work::model::Event| {
            event.kind == "note" && event.text == "Put in Claude's prompt in sh-21349: Finish this before the release."
        };
        let linked = index.links.iter().any(|link| {
            link.a == "ticket:SH-21349" && link.b == format!("session:{session}") && link.how == "you" && !link.removed
        });
        noted = index.events.iter().any(said) && linked;
        if noted {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(noted, "the note and the link are in the core's index: {:?}", kept(&app).events.first());
    // your words are sent: the draft is cleared, and the card stays with the Claude it went to
    let left = draft(cx, &app, "ticket:SH-21349");
    assert_eq!((left.note.as_str(), left.items.len(), left.text.clone()), ("", 0, None));
    assert_eq!(left.claude.as_deref(), Some(session.as_str()));
    // nothing was written to GitHub or Jira for it
    let (gh, twg) = (fakes.gh_log(), fakes.twg_log());
    assert!(!gh.contains("mutation") && !gh.contains("fake-token"), "{gh}");
    assert!(!twg.contains(" comment add") && !twg.contains(" transition"), "{twg}");
    common::never_wrote(&fakes);
}

/// Runs git in a folder of the test's own and gives what it printed.
fn git(folder: &std::path::Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git").args(args).current_dir(folder).output().expect("git runs");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// The fake Claudes started in the folder of the workspace `name`, oldest first: (session, resumed, its arguments).
fn started_in(app: &App, name: &str) -> Vec<(String, bool, String)> {
    (app.claude_starts().iter())
        .filter(|start| start["cwd"].as_str().is_some_and(|cwd| cwd.ends_with(&format!("/{name}"))))
        .map(|start| {
            let argv: Vec<&str> =
                start["argv"].as_array().map(|a| a.iter().filter_map(|v| v.as_str()).collect()).unwrap_or_default();
            (start["sessionId"].as_str().unwrap_or_default().to_string(), start["resumed"] == true, argv.join(" "))
        })
        .collect()
}

fn until_started_in(cx: &mut TestAppContext, app: &App, name: &str, count: usize) -> Vec<(String, bool, String)> {
    for _ in 0..1500 {
        cx.run_until_parked();
        if started_in(app, name).len() >= count {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    started_in(app, name)
}

/// The workspace named `name`, as the window has it.
fn workspace(cx: &mut TestAppContext, app: &App, name: &str) -> Option<enkidu_core::db::Workspace> {
    cx.run_until_parked();
    app.frame.read_with(cx, |frame, _| frame.workspaces.iter().find(|w| w.name == name).cloned())
}

/// Writes what Claude should do about the open card's task.
fn tell(cx: &mut TestAppContext, app: &App, words: &str) {
    app.press(cx, "s");
    app.typed(cx, words);
    app.press(cx, "escape");
}

/// Waits until the card's message took you to the Claude of the workspace `name`.
fn until_taken_to(cx: &mut TestAppContext, app: &App, name: &str) {
    let gone = app.until(cx, |w| w.try_find("work-page").is_none() && w.try_find("task-dialog").is_none());
    assert!(gone, "the page never took you to Claude; it said {:?}", toast(cx, app));
    for _ in 0..800 {
        if app.on_screen(cx).is_some_and(|(on, ..)| on == name) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("{name} never came on screen; on screen: {:?}", app.on_screen(cx));
}

/// Send to Claude: starts Claude there when it isn't running, with the message to type into its prompt once it is up.
#[gpui_kit::test]
fn starts_claude_there_when_it_isnt_running_with_the_message_typed_into_its_prompt_once_it_is_up(
    cx: &mut TestAppContext,
) {
    // another workspace is on screen: the Claude of the card's workspace isn't running
    let (app, fakes) =
        start(cx, |shared, project| App::make(shared, project, "scratch", "new", "scratch", Some("main")));
    let scratch = workspace(cx, &app, "scratch").expect("the second workspace").id;
    app.frame.update(cx, |frame, cx| {
        frame.select_workspace(&scratch);
        cx.notify();
    });
    assert_eq!(until_started_in(cx, &app, "scratch", 1).len(), 1, "scratch's Claude started as it came on screen");
    // the first workspace's Claude ran while it was on screen: it is closed, as you would close its tab
    let ours = workspace(cx, &app, "sh-21349").expect("the card's workspace");
    let running = enkidu_core::commands::pty_list(&app.shared, ours.id.clone());
    cx.update(|cx| {
        let terminals = enkidu_app::stores::Stores::terminals(cx);
        for tab in running {
            terminals.update(cx, |terminals, cx| terminals.close(&ours.id, &tab.id, cx));
        }
    });
    for _ in 0..400 {
        cx.run_until_parked();
        if enkidu_core::commands::pty_list(&app.shared, ours.id.clone()).iter().all(|tab| tab.exited) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let before = started_in(&app, "sh-21349").len();
    open_card(cx, &app, "SH-21349");
    assert_eq!(goes(cx, &app).0, "goes into the prompt of the Claude in sh-21349; you press Enter there");
    tell(cx, &app, "Look at the failing check.");
    // ⌘↩ from the card sends, as the button does
    app.press(cx, "cmd-enter");
    until_taken_to(cx, &app, "sh-21349");
    // Claude starts there, in the workspace's own conversation, with nothing to send on its command line
    let starts = until_started_in(cx, &app, "sh-21349", before + 1);
    assert_eq!(starts.len(), before + 1, "one Claude started there");
    let (session, _, argv) = starts.last().expect("the start");
    assert_eq!(session, &ours.session_id);
    assert!(!argv.contains("Look at the failing check."), "the message is never a first message: {argv}");
    // and gets the message typed into its prompt once it is up: not sent
    assert!(app.until_terminal(cx, "Look at the failing check."), "the message is in its prompt");
    std::thread::sleep(Duration::from_millis(400));
    cx.run_until_parked();
    assert!(sent(&app).is_empty(), "nothing was sent: {:?}", sent(&app));
    app.press(cx, "enter");
    let prompts = until_sent(cx, &app, 1);
    assert!(prompts[0].contains("SH-21349:") && prompts[0].contains("Look at the failing check."), "{prompts:?}");
    common::never_wrote(&fakes);
}

/// Starts a workspace named for a ticket no Claude has worked on: its own branch, a Claude, and the message in its
/// prompt. Then starts a new conversation in the card's workspace when you pick a new Claude, and resumes the earlier
/// one when you pick it (an idle Claude closes first, a busy one never).
#[gpui_kit::test]
fn starts_a_workspace_for_a_ticket_no_claude_has_worked_on_then_a_new_conversation_there_and_the_old_one_again(
    cx: &mut TestAppContext,
) {
    let (app, fakes) = start(cx, |_, _| {});
    let name = "sh-21473-text-2-85-chars";
    let branch = "feature/SH-21473-text-2-85-chars";
    open_card(cx, &app, "SH-21473");
    assert_eq!(
        goes(cx, &app),
        (
            format!("a new workspace, {name}, on {branch} (a new branch), with a new Claude and this in its prompt"),
            "Start workspace".to_string()
        )
    );
    assert!(app.until(cx, |w| w.try_find("detail-sessions").is_some()), "the project has a workspace to pick instead");
    tell(cx, &app, "Start with the tests.");
    app.click(cx, "task-send");
    until_taken_to(cx, &app, name);
    // its own branch, made from the project's base
    let made = workspace(cx, &app, name).expect("the new workspace");
    assert_eq!(made.branch, branch);
    assert_eq!(git(std::path::Path::new(&made.path), &["rev-parse", "--abbrev-ref", "HEAD"]), branch);
    // Claude starts there with nothing to send, and gets the message in its prompt once it says it is up
    let starts = until_started_in(cx, &app, name, 1);
    assert_eq!(starts.len(), 1, "one Claude started there, not one more as it came on screen");
    let (first, resumed, argv) = starts[0].clone();
    assert_eq!((first.as_str(), resumed), (made.session_id.as_str(), false));
    assert!(!argv.contains("Start with the tests."), "{argv}");
    assert!(app.until_terminal(cx, "Start with the tests."), "the message is in its prompt");
    assert_eq!(toast(cx, &app), (Some(format!("Started {name}")), Some("Press Enter there to send it.".to_string())));
    // nothing goes before you press Enter
    std::thread::sleep(Duration::from_millis(400));
    cx.run_until_parked();
    assert!(sent(&app).is_empty(), "{:?}", sent(&app));
    app.press(cx, "enter");
    let prompts = until_sent(cx, &app, 1);
    assert!(prompts[0].contains("SH-21473") && prompts[0].contains("Start with the tests."), "{prompts:?}");
    // the card's timeline says a workspace was started for it
    let mut noted = false;
    for _ in 0..600 {
        let said = format!("Started {name} for it; the message is in its Claude's prompt: Start with the tests.");
        noted = kept(&app).events.iter().any(|event| event.kind == "note" && event.text == said);
        if noted {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(noted, "the note is on the card's timeline");

    // its Claude answered (its Stop hook): done, so it may close. The card is with it now: it is picked
    let status = |cx: &mut TestAppContext, app: &App| -> Option<String> {
        cx.run_until_parked();
        app.frame.read_with(cx, |frame, _| frame.statuses.get(&made.id).cloned())
    };
    for _ in 0..800 {
        if status(cx, &app).as_deref() == Some("done") {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(status(cx, &app).as_deref(), Some("done"), "Claude finished its turn");
    open_card(cx, &app, "SH-21473");
    let option = |id: &str| SharedString::from(format!("claude-option:{id}"));
    let picked = option(&first);
    assert!(app.until(cx, move |w| w.try_find(picked.clone()).is_some_and(|e| e.checked() == Some(true))));
    // a Claude at work is never closed for a new conversation
    app.click(cx, option("new"));
    assert_eq!(
        goes(cx, &app),
        (
            format!("a new conversation in {name} (the one there now stays saved), with this in its prompt"),
            "Start Claude".to_string()
        )
    );
    app.frame.update(cx, |frame, cx| {
        frame.statuses.insert(made.id.clone(), "working".into());
        cx.notify();
    });
    tell(cx, &app, "Again.");
    app.click(cx, "task-send");
    let refused = until_toast(cx, &app, "Couldn't put it in Claude's prompt");
    assert_eq!(refused, Some(format!("Claude is working in {name}. Pick it, or try again once it's done.")));
    assert!(app.until(cx, |w| w.try_find("task-dialog").is_some()), "the card stays, with your words");
    assert_eq!(started_in(&app, name).len(), 1, "nothing was closed or started");
    assert_eq!(draft(cx, &app, "ticket:SH-21473").note, "Again.");
    // idle again: the Claude there closes first (its conversation stays saved), and a new one starts in a
    // conversation of its own, with the message in its prompt
    app.frame.update(cx, |frame, cx| {
        frame.statuses.insert(made.id.clone(), "done".into());
        cx.notify();
    });
    app.press(cx, "s");
    app.press(cx, "cmd-a");
    app.typed(cx, "Fresh start, please.");
    app.press(cx, "escape");
    app.click(cx, "task-send");
    until_taken_to(cx, &app, name);
    let starts = until_started_in(cx, &app, name, 2);
    assert_eq!(starts.len(), 2, "a new Claude started there");
    let (fresh, resumed, _) = starts[1].clone();
    assert!(fresh != first && !resumed, "a conversation of its own: {fresh} {resumed}");
    assert_eq!(workspace(cx, &app, name).map(|w| w.session_id), Some(fresh.clone()), "the workspace is in it now");
    assert!(app.until_terminal(cx, "Fresh start, please."), "the message is in its prompt");
    assert_eq!(draft(cx, &app, "ticket:SH-21473").claude.as_deref(), Some(fresh.as_str()), "the card stays with it");
    app.press(cx, "enter");
    let prompts = until_sent(cx, &app, 2);
    assert!(prompts[1].contains("Fresh start, please."), "{prompts:?}");
    for _ in 0..800 {
        if status(cx, &app).as_deref() == Some("done") {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    // the first conversation is still there to pick: it is resumed in the same folder
    open_card(cx, &app, "SH-21473");
    let old = option(&first);
    assert!(app.until(cx, move |w| w.try_find(old.clone()).is_some()), "the first conversation is on the card");
    app.click(cx, option(&first));
    assert_eq!(
        goes(cx, &app),
        (
            format!("resumes its conversation in {name}, with this in its prompt; you press Enter there"),
            "Start Claude".to_string()
        )
    );
    tell(cx, &app, "Back to the first one.");
    app.click(cx, "task-send");
    until_taken_to(cx, &app, name);
    let starts = until_started_in(cx, &app, name, 3);
    assert_eq!(starts.len(), 3);
    assert_eq!((starts[2].0.as_str(), starts[2].1), (first.as_str(), true), "the old conversation, resumed");
    assert!(app.until_terminal(cx, "Back to the first one."), "the message is in its prompt");
    common::never_wrote(&fakes);
}

/// A commit of its own on a new branch of the project's main folder, which is left on the branch it was on. Gives
/// the commit.
fn branch_with_a_commit(app: &App, branch: &str) -> String {
    let was = git(&app.project, &["rev-parse", "--abbrev-ref", "HEAD"]);
    git(&app.project, &["checkout", "-q", "-b", branch]);
    std::fs::write(app.project.join("made-before.txt"), "made before\n").unwrap();
    git(&app.project, &["add", "made-before.txt"]);
    git(
        &app.project,
        &["-c", "user.name=Test", "-c", "user.email=test@example.com", "commit", "-q", "-m", "Made before"],
    );
    let tip = git(&app.project, &["rev-parse", "HEAD"]);
    git(&app.project, &["checkout", "-q", &was]);
    tip
}

/// Send to Claude: checks out a ticket's branch that is already there instead of making it again.
#[gpui_kit::test]
fn checks_out_a_tickets_branch_that_is_already_there_instead_of_making_it_again(cx: &mut TestAppContext) {
    let (app, fakes) = start(cx, |_, _| {});
    let (name, branch) = ("sh-21473-text-2-85-chars", "feature/SH-21473-text-2-85-chars");
    // the branch named for the ticket was made before, and has a commit the base doesn't
    let tip = branch_with_a_commit(&app, branch);
    open_card(cx, &app, "SH-21473");
    tell(cx, &app, "Go on where the branch is.");
    app.click(cx, "task-send");
    until_taken_to(cx, &app, name);
    let made = workspace(cx, &app, name).expect("the new workspace");
    let folder = std::path::Path::new(&made.path);
    assert_eq!(git(folder, &["rev-parse", "--abbrev-ref", "HEAD"]), branch);
    assert_eq!(git(folder, &["rev-parse", "HEAD"]), tip, "the branch that was there is checked out, not made again");
    assert!(app.until_terminal(cx, "Go on where the branch is."), "the message is in its Claude's prompt");
    common::never_wrote(&fakes);
}

/// The card's new workspace wants a branch the project's main folder has checked out: the card asks what to do.
fn taken(cx: &mut TestAppContext) -> (App, common::Fakes, &'static str, &'static str) {
    let (app, fakes) = start(cx, |_, _| {});
    let (name, branch) = ("sh-21473-text-2-85-chars", "feature/SH-21473-text-2-85-chars");
    git(&app.project, &["checkout", "-q", "-b", branch]);
    open_card(cx, &app, "SH-21473");
    tell(cx, &app, "Start with the tests.");
    app.click(cx, "task-send");
    assert!(app.until(cx, |w| w.try_find("branch-taken").is_some()), "the card asks: {:?}", toast(cx, &app));
    (app, fakes, name, branch)
}

/// Opens the folder a card's branch is checked out in, with its session, when you say so.
#[gpui_kit::test]
fn opens_the_folder_a_cards_branch_is_checked_out_in_when_you_say_so(cx: &mut TestAppContext) {
    let (app, fakes, name, branch) = taken(cx);
    // a choice, not an error: it says where the branch is, and nothing was started
    let said = cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        said(w, "branch-taken").unwrap_or_default()
    });
    let said = said.unwrap();
    let folder = app.project.file_name().and_then(|name| name.to_str()).unwrap_or_default().to_string();
    assert!(said.starts_with(&format!("{branch} is checked out in ")) && said.ends_with(&folder), "{said}");
    assert_eq!(toast(cx, &app), (None, None), "a choice, not an error");
    assert!(workspace(cx, &app, name).is_none() && started_in(&app, &folder).is_empty(), "nothing started yet");
    assert!(app.until(cx, |w| w.try_find("task-dialog").is_some()));
    let offered = cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        (said_of(w, "taken-open"), said_of(w, "taken-new-branch"))
    });
    assert_eq!(offered.unwrap(), (Some("Open that folder as a workspace".to_string()), Some("Create".to_string())));
    // that folder, as it is, becomes the card's workspace, with Claude in it and the message in its prompt
    app.click(cx, "taken-open");
    until_taken_to(cx, &app, name);
    let opened = workspace(cx, &app, name).expect("the folder as a workspace");
    let real = |path: &std::path::Path| path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    assert_eq!(real(std::path::Path::new(&opened.path)), real(&app.project));
    assert_eq!(opened.branch, branch);
    assert_eq!(until_started_in(cx, &app, &folder, 1).len(), 1, "Claude started in that folder");
    assert!(app.until_terminal(cx, "Start with the tests."), "the message is in its prompt");
    assert!(sent(&app).is_empty(), "nothing was sent");
    assert_eq!(draft(cx, &app, "ticket:SH-21473").claude.as_deref(), Some(opened.session_id.as_str()));
    common::never_wrote(&fakes);
}

fn said_of(window: &gpui_kit::Window, id: &'static str) -> Option<String> {
    said(window, id)
}

/// Item 5.13: the other way out of a taken branch, a new branch from it.
#[gpui_kit::test]
fn or_starts_the_cards_workspace_on_a_new_branch_from_the_one_that_is_taken(cx: &mut TestAppContext) {
    let (app, fakes, name, branch) = taken(cx);
    // the first free name is offered: Create makes a workspace on it, starting where the taken branch is
    let tip = git(&app.project, &["rev-parse", "HEAD"]);
    app.click(cx, "taken-new-branch");
    let new_name = format!("{name}-2");
    until_taken_to(cx, &app, &new_name);
    let made = workspace(cx, &app, &new_name).expect("the new workspace");
    assert_eq!(made.branch, format!("{branch}-2"));
    let folder = std::path::Path::new(&made.path);
    assert_eq!(git(folder, &["rev-parse", "--abbrev-ref", "HEAD"]), format!("{branch}-2"));
    assert_eq!(git(folder, &["rev-parse", "HEAD"]), tip, "it starts where the taken branch is");
    assert_eq!(until_started_in(cx, &app, &new_name, 1).len(), 1);
    assert!(app.until_terminal(cx, "Start with the tests."), "the message is in its Claude's prompt");
    common::never_wrote(&fakes);
}

/// Send to Claude: brings a session from outside Enkidu into a new workspace on its PR's branch.
#[gpui_kit::test]
fn brings_a_session_from_outside_into_a_new_workspace_on_its_prs_branch(cx: &mut TestAppContext) {
    let (app, fakes) = start(cx, |_, _| {});
    // PR #2329's branch is in the repository, and a Claude that ran in Warp made that PR
    let (name, branch, session) =
        ("sh-20844-change-390", "feature/SH-20844-change-390", "8b9a2f10-55d1-4e0b-9a51-2c1d7e6f0a11");
    git(&app.project, &["branch", branch]);
    let now = enkidu_core::db::now_ms();
    let seen = enkidu_core::work::store::SessionSeen {
        id: session.into(),
        config_dir: app.home.path().join("claude-config").to_string_lossy().into(),
        cwd: app.project.to_string_lossy().into(),
        title: Some("change 390".into()),
        workspace_id: None,
        first_at: now - 7_200_000,
        last_at: now - 3_600_000,
        source: "hook".into(),
    };
    app.shared.db.work_session_seen(&seen).expect("the session is kept");
    app.shared
        .db
        .work_mention(session, "pr:example-org/shop#2329", "hook:pr-create", "created", now - 3_600_000)
        .unwrap();
    enkidu_core::work::relink(&app.shared).expect("linked");
    enkidu_core::work::changed(&app.sink);
    open_card(cx, &app, "SH-20844");
    let row = SharedString::from(format!("claude-option:{session}"));
    let picked = row.clone();
    assert!(
        app.until(cx, move |w| w.try_find(picked.clone()).is_some_and(|e| e.checked() == Some(true))),
        "the Claude that made the PR is on the card, and picked"
    );
    let (words, button) = goes(cx, &app);
    assert!(words.starts_with("goes into its session from "), "{words}");
    assert!(words.ends_with(&format!(", brought into a new workspace on {branch}; you press Enter there")), "{words}");
    assert_eq!(button, "Open in Claude");
    tell(cx, &app, "Units?");
    app.click(cx, "task-send");
    until_taken_to(cx, &app, name);
    // a workspace on the PR's own branch, whose conversation is that session
    let made = workspace(cx, &app, name).expect("the new workspace");
    assert_eq!((made.branch.as_str(), made.session_id.as_str()), (branch, session));
    assert_eq!(git(std::path::Path::new(&made.path), &["rev-parse", "--abbrev-ref", "HEAD"]), branch);
    let starts = until_started_in(cx, &app, name, 1);
    assert_eq!(starts.len(), 1);
    assert_eq!(starts[0].0, session, "Claude starts there in that session");
    assert!(app.until_terminal(cx, "Units?"), "the message is in its prompt");
    assert!(sent(&app).is_empty(), "nothing was sent");
    common::never_wrote(&fakes);
}

/// Shows what Claude reports with enkidu report, its screenshot copied in, on the card and in the task (item 5.14 on
/// the whole wiring: `enkidu` run by the fake Claude, the core's own copy of the picture, the card).
#[gpui_kit::test]
fn shows_what_claude_reports_with_enkidu_report_its_screenshot_copied_in_on_the_card_and_in_the_task(
    cx: &mut TestAppContext,
) {
    let (app, fakes) = start(cx, |_, _| ());
    // Claude made a screenshot in a folder of its own and reports with it
    let shot = app.home.path().join("after.png");
    std::fs::write(&shot, enkidu_testkit::screenshot_png(640, 400, 1)).expect("Claude's screenshot");
    app.type_in_terminal(
        cx,
        &format!(
            "enkidu report review --ticket SH-21349 \"Renamed the field. Checked the API answer with three orders.\" \
             --image {}\r",
            shot.display()
        ),
    );
    assert!(app.until_terminal(cx, "Reported on SH-21349"), "{}", app.terminal_text(cx));
    assert!(app.terminal_text(cx).contains("Finished, wants your review, with 1 screenshot"));
    let report = (enkidu_core::work::snapshot(&app.shared).expect("the core's index").reports.into_iter())
        .find(|report| report.subject == "SH-21349")
        .expect("the report is in the app's database");
    assert_eq!(report.images, ["1-after.png"]);
    // what Claude leaves in its folder can go: the report keeps its own copy
    std::fs::remove_file(&shot).expect("the screenshot Claude made");

    // on the card: how it ended
    app.press(cx, "cmd-i");
    assert!(
        app.until(cx, |w| w.try_find("card-report:ticket:SH-21349").is_some()),
        "the card carries the report's mark"
    );
    let mark = cx.update_window(app.window, |_, w, _| said(w, "card-report:ticket:SH-21349")).unwrap();
    assert_eq!(mark.as_deref(), Some("Review"));
    // in the task: Claude's words, by the Claude of its workspace
    app.click(cx, card("SH-21349"));
    let (box_id, text_id) = (format!("task-report:{}", report.id), format!("report-text:{}", report.id));
    assert!(app.until(cx, |w| w.try_find(SharedString::from(box_id.clone())).is_some()), "the report is in the task");
    let (head, words) = cx
        .update_window(app.window, |_, w, _| {
            let say = |id: &str| w.find(SharedString::from(id.to_string())).label().map(String::from);
            (say(&box_id).unwrap_or_default(), say(&text_id).unwrap_or_default())
        })
        .unwrap();
    assert!(
        head.starts_with("Finished, wants your review, ") && head.ends_with(" by the Claude in sh-21349"),
        "{head}"
    );
    assert_eq!(words, "Renamed the field. Checked the API answer with three orders.");
    // the screenshot came over and shows as a picture, read from the app's own folder
    let thumb = SharedString::from(format!("report-image:{}:1-after.png", report.id));
    let shown = |w: &gpui_kit::Window, id: &SharedString| {
        w.try_find(id.clone()).and_then(|e| e.description().map(String::from))
    };
    let small = thumb.clone();
    assert!(app.until(cx, move |w| shown(w, &small).as_deref() == Some("shown")), "the screenshot never showed");
    // large, it is the whole picture at its own size; Esc closes it and the task stays
    app.click(cx, thumb);
    assert!(
        app.until(cx, |w| {
            w.try_find("report-lightbox-image").and_then(|e| e.description().map(String::from)).as_deref()
                == Some("whole")
        }),
        "the screenshot never showed large"
    );
    let picture = cx.update_window(app.window, |_, w, _| w.find("report-lightbox-image").bounds().size).unwrap();
    assert_eq!(picture, size(px(640.), px(400.)));
    app.press(cx, "escape");
    assert!(app.until(cx, |w| w.try_find("report-lightbox").is_none() && w.try_find("task-dialog").is_some()));
    common::never_wrote(&fakes);
}

/// How the newest run on a project's page of Settings stands ("morning skipped") and what it says.
fn newest_run(cx: &mut TestAppContext, app: &App) -> (String, String) {
    cx.run_until_parked();
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        let row = w.try_find(("workflow-run", 0usize));
        let stands = row.as_ref().and_then(|row| row.description().map(String::from)).unwrap_or_default();
        (stands, row.and_then(|row| row.label().map(String::from)).unwrap_or_default())
    })
    .unwrap()
}

fn until_newest_run(cx: &mut TestAppContext, app: &App, stands: &str) -> String {
    for _ in 0..6000 {
        let (now, says) = newest_run(cx, app);
        if now == stands {
            return says;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("the newest run never stood as {stands}: {:?}", newest_run(cx, app));
}

/// Runs the morning summary by hand from Settings, and doesn't when Jira can't be read (item 5.16 on the whole wiring:
/// the core reads through the fake gh and twg, then runs the fake Claude read-only in the main folder; its run shows on
/// the project's page as it ends, and the window says it in a toast).
#[gpui_kit::test]
fn runs_the_morning_summary_by_hand_from_settings_and_doesnt_when_jira_cant_be_read(cx: &mut TestAppContext) {
    let (app, fakes) = start(cx, |_, _| ());
    app.until_work(cx, |work| !work.checking && work.health.len() == 2 && !work.tickets.is_empty());
    let printed = |app: &App| -> Vec<serde_json::Value> {
        let print = |start: &serde_json::Value| {
            start["argv"].as_array().is_some_and(|argv| argv.iter().any(|arg| arg == "-p" || arg == "--print"))
        };
        app.claude_starts().into_iter().filter(print).collect()
    };
    let before = printed(&app).len();
    // Jira signed out: the summary doesn't run, and says why
    fakes.twg("logged-out");
    app.click(cx, ("project", 0usize));
    assert!(app.until(cx, |w| w.try_find("project-settings").is_some()));
    app.click(cx, "project-settings");
    assert!(app.until(cx, |w| w.try_find("default-base").is_some()), "the project's page never showed");
    let to_the_end = |cx: &mut TestAppContext, app: &App| {
        cx.run_until_parked();
        cx.update_window(app.window, |_, w, cx| {
            w.render_frame(cx);
            w.scroll("settings-body", gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-6000.))), cx)
        })
        .unwrap();
    };
    to_the_end(cx, &app);
    assert_eq!(
        cx.update_window(app.window, |_, w, _| said(w, "work-morning-run")).unwrap().as_deref(),
        Some("Run now")
    );
    app.click(cx, "work-morning-run");
    let says = until_newest_run(cx, &app, "morning skipped");
    assert!(says.starts_with("Morning summary: Didn't run: Jira can't be read: twg isn't signed in"), "{says}");
    assert_eq!(printed(&app).len(), before, "no Claude ran");
    assert_eq!(
        until_toast(cx, &app, "Morning summary").map(|detail| detail.contains("Jira can't be read")),
        Some(true)
    );

    // signed in again: it runs, read-only in the main folder, and writes one line per ticket
    fakes.twg("ok");
    to_the_end(cx, &app);
    app.click(cx, "work-morning-run");
    let says = until_newest_run(cx, &app, "morning done");
    let tickets = enkidu_core::work::snapshot(&app.shared).expect("the core's index").tickets.len();
    assert!(tickets > 0);
    assert_eq!(
        says,
        format!(
            "Morning summary: Claude wrote {tickets} lines on the tickets' timelines. Summarized {tickets} tickets., just now"
        )
    );
    let runs = printed(&app);
    assert_eq!(runs.len(), before + 1, "one read-only Claude ran");
    let claude = runs.last().unwrap();
    assert_eq!(claude["cwd"].as_str(), Some(app.project.to_str().unwrap()), "in the project's main folder");
    assert_eq!(
        until_toast(cx, &app, "Morning summary written"),
        Some(format!("Claude wrote {tickets} lines on the tickets' timelines. Summarized {tickets} tickets."))
    );
    // one line per ticket, on its timeline
    let events = enkidu_core::work::snapshot(&app.shared).expect("the core's index").events;
    let lines = events.iter().filter(|e| e.kind == "note" && e.text.ends_with("summarized by the fake Claude")).count();
    assert_eq!(lines, tickets);
    app.press(cx, "escape");
    assert!(app.until(cx, |w| w.try_find("work-morning-run").is_none()), "Esc closes Settings");
    common::never_wrote(&fakes);
}

/// Item 8.5 on the whole wiring: a task of your own goes to Claude as a ticket's card does. "Start workspace" makes a
/// workspace named for the task on a new branch; Claude starts there with what the task is, what you wrote about it
/// and your words typed into its prompt, never sent; that Claude is the card's from then on, and the card's timeline
/// says so under its own name.
#[gpui_kit::test]
fn a_task_of_your_own_starts_a_workspace_named_for_it_with_its_words_in_claudes_prompt(cx: &mut TestAppContext) {
    let (app, _fakes) = start(cx, |shared, project| {
        shared
            .db
            .work_save_own_task(None, project, "Try the small Docker setup", "The runner has 4 GB.", "todo", 1)
            .unwrap();
    });
    let name = "task-1-try-the-small-docker-setup";
    let branch = "feature/task-1-try-the-small-docker-setup";
    app.press(cx, "cmd-i");
    app.until_work(cx, |work| !work.checking && work.health.len() == 2 && work.own_tasks.len() == 1);
    let own = SharedString::from("work-card:own:1");
    let shown = own.clone();
    assert!(app.until(cx, move |w| w.try_find(shown.clone()).is_some()), "the board shows the task");
    app.click(cx, own);
    assert!(app.until(cx, |w| w.try_find("task-composer").is_some()), "its card is open");
    assert_eq!(
        goes(cx, &app),
        (
            format!("a new workspace, {name}, on {branch} (a new branch), with a new Claude and this in its prompt"),
            "Start workspace".to_string()
        )
    );
    tell(cx, &app, "Start with the compose file.");
    app.click(cx, "task-send");
    until_taken_to(cx, &app, name);
    let made = workspace(cx, &app, name).expect("the new workspace");
    assert_eq!(made.branch, branch);
    assert_eq!(git(std::path::Path::new(&made.path), &["rev-parse", "--abbrev-ref", "HEAD"]), branch);
    assert_eq!(until_started_in(cx, &app, name, 1).len(), 1);
    assert!(app.until_terminal(cx, "Start with the compose file."), "the message is in its prompt");
    // nothing goes before you press Enter
    std::thread::sleep(Duration::from_millis(400));
    cx.run_until_parked();
    assert!(sent(&app).is_empty(), "{:?}", sent(&app));
    app.press(cx, "enter");
    let prompts = until_sent(cx, &app, 1);
    let wanted = "A task of mine (not in Jira): Try the small Docker setup\n\nThe runner has 4 GB.\n\nStart with the compose file.";
    assert!(prompts[0].starts_with(wanted), "{prompts:?}");
    assert!(!prompts[0].contains("Ticket:"), "{prompts:?}");
    // that Claude is linked to the task by you, and the task's own timeline says a workspace was started for it
    let session = format!("session:{}", made.session_id);
    let mut told = (false, false);
    for _ in 0..600 {
        let index = kept(&app);
        let linked = index.links.iter().any(|l| l.a == "own:1" && l.b == session && l.detail == "linked by you");
        let noted = (index.events.iter())
            .any(|e| e.subject == "own:1" && e.kind == "note" && e.text.starts_with(&format!("Started {name} for it")));
        told = (linked, noted);
        if told == (true, true) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(told, (true, true), "linked by you, and noted on its timeline");
}
