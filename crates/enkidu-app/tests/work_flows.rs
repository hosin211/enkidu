//! Workflows on the whole wiring (item 6.9): a workflow set up in its dialog and run by the core's own runner, each
//! task's message sent to its Claude (the fake one, which answers a workflow's task as a Claude might: it tries to
//! commit, asks `enkidu workflow` where it stands, says how it goes with `enkidu update`, stops, and reports when the
//! Stop hook asks), Live following the task that runs, and the run kept on the Workflows page. The real core with its
//! watcher on the fake `gh` and the fake `twg`, never the network. Each run has its own app.

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{App, said};
use enkidu_core::db::Workspace;
use enkidu_core::state::Shared;
use enkidu_core::work::flows::{Flow, Target};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, SharedString, TestAppContext, px, size};

/// The branch of the fakes' PR #2350, which names SH-21349; and the branch a workspace for SH-21473 is on.
const BRANCH: &str = "feature/SH-21349-change-719";
const OTHER: &str = "feature/SH-21473-text-2-85-chars";
const OTHER_NAME: &str = "sh-21473-text-2-85-chars";

/// The app on the project the recorded answers are about, with the workspace `sh-21349` on the branch of PR #2350
/// (on screen, its Claude up and waiting at its prompt), whatever `more` makes, and the index read through the fakes.
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
    app.until_work(cx, |work| !work.checking && work.health.len() == 2 && !work.tickets.is_empty());
    (app, fakes)
}

/// The core's own runner, as `enkidu_core::start` starts it: it looks every 3 seconds, and at once when a turn's end, a
/// report or Run now wakes it. It ends with the test (and the core's watchers with it).
struct Runner(Arc<Shared>);

impl Runner {
    fn start(app: &App) -> Runner {
        enkidu_core::work::flows::start(app.sink.clone(), app.shared.clone());
        Runner(app.shared.clone())
    }
}

impl Drop for Runner {
    fn drop(&mut self) {
        self.0.work.stop();
        enkidu_core::work::flows::wake();
    }
}

fn workspace_named(app: &App, name: &str) -> Workspace {
    let all = app.shared.db.workspaces().expect("the workspaces");
    all.into_iter().find(|workspace| workspace.name == name).unwrap_or_else(|| panic!("no workspace {name}"))
}

fn git(dir: &str, args: &[&str]) -> String {
    let out = std::process::Command::new("git").arg("-C").arg(dir).args(args).output().expect("git runs");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Every prompt that was sent to a fake Claude, oldest first: the conversation it went to, and its words.
fn sent(app: &App) -> Vec<(String, String)> {
    let log = format!("{}.prompts", app.claude_log.display());
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter_map(|said| Some((said["sessionId"].as_str()?.to_string(), said["prompt"].as_str()?.to_string())))
        .collect()
}

/// The index as the core's own database has it.
fn kept(app: &App) -> enkidu_core::work::model::Snapshot {
    enkidu_core::work::snapshot(&app.shared).expect("the core's index")
}

fn flow_named(app: &App, name: &str) -> Option<Flow> {
    kept(app).flows.into_iter().find(|flow| flow.name == name)
}

/// The workspace that is Live now, as the window has it.
fn live_now(cx: &mut TestAppContext, app: &App) -> Option<String> {
    cx.run_until_parked();
    let project = app.project_id.clone();
    app.frame.read_with(cx, |frame, _| {
        frame.projects.iter().find(|p| p.id == project).and_then(|p| p.live_workspace_id.clone())
    })
}

/// Waits (up to `seconds`) until the workflow `name` is in `state`, noting every move of Live as the window hears
/// it. The workflow as it stands at the end, whether it got there or not.
fn until_flow(
    cx: &mut TestAppContext,
    app: &App,
    name: &str,
    state: &str,
    seconds: u64,
    live_seen: &mut Vec<Option<String>>,
) -> Option<Flow> {
    let started = Instant::now();
    loop {
        let live = live_now(cx, app);
        if live_seen.last() != Some(&live) {
            live_seen.push(live);
        }
        let flow = flow_named(app, name);
        if flow.as_ref().is_some_and(|flow| flow.state == state) || started.elapsed() > Duration::from_secs(seconds) {
            return flow;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The run is over in the core; the window hears of Live's last move a moment later. Waits for it (the window's Live
/// is the core's), and notes it.
fn live_settled(cx: &mut TestAppContext, app: &App, live_seen: &mut Vec<Option<String>>) {
    let started = Instant::now();
    loop {
        let core = app.shared.db.project(&app.project_id).ok().flatten().and_then(|project| project.live_workspace_id);
        let live = live_now(cx, app);
        if live_seen.last() != Some(&live) {
            live_seen.push(live.clone());
        }
        if live == core || started.elapsed() > Duration::from_secs(10) {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// What a workspace's terminal shows, also while the Work page is on screen.
fn terminal_of(cx: &mut TestAppContext, workspace: &str) -> String {
    cx.run_until_parked();
    let view = cx.update(|cx| cx.global::<enkidu_app::stores::Stores>().terminals.read(cx).active_view(workspace));
    view.map(|view| view.read_with(cx, |view, _| view.terminal().screen().text())).unwrap_or_default()
}

fn card(key: &str) -> SharedString {
    SharedString::from(format!("work-card:ticket:{key}"))
}

/// Opens the Work page, picks these tickets' cards, and opens the workflow dialog for them.
fn workflow_for(cx: &mut TestAppContext, app: &App, keys: &[&str]) {
    app.press(cx, "cmd-i");
    for key in keys {
        assert!(app.until(cx, |w| w.try_find(card(key)).is_some()), "the board shows {key}");
    }
    let page = app.shell.read_with(cx, |shell, _| shell.work_page().expect("the Work page").clone());
    page.update(cx, |page, cx| {
        page.clear_picks(cx);
        for key in keys {
            page.pick(&format!("ticket:{key}"), cx);
        }
    });
    assert!(app.until(cx, |w| w.try_find("pick-workflow").is_some()), "the bar of the picked cards");
    app.click(cx, "pick-workflow");
    assert!(app.until(cx, |w| w.try_find("flow-editor").is_some()), "the workflow dialog");
}

/// A task's place in the dialog, by its ticket.
fn row(cx: &mut TestAppContext, app: &App, key: &str) -> usize {
    let subject = format!("ticket:{key}");
    cx.run_until_parked();
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        (0..12usize).find(|n| w.try_find(("flow-task", *n)).is_some_and(|row| row.value() == Some(subject.as_str())))
    })
    .unwrap()
    .unwrap_or_else(|| panic!("no task for {key} in the dialog"))
}

/// Shows a step of the dialog's chain at its right: a task by its place, or the start (when it runs, and what every
/// task gets). The dialog draws the setup of the step that is picked only (design 2, item 17.4).
fn step(cx: &mut TestAppContext, app: &App, at: Option<usize>) {
    match at {
        Some(at) => {
            app.click(cx, ("flow-task", at));
            assert!(app.until(cx, |w| w.try_find(("flow-task-kind", at)).is_some()), "the task's setup is shown");
        }
        None => {
            app.click(cx, "flow-start");
            assert!(app.until(cx, |w| w.try_find("flow-minutes").is_some()), "the start's setup is shown");
        }
    }
}

/// The step a control of the dialog belongs to: a task's own (`("flow-task-words", 2)`), the start's, or none (the
/// name, which is always there).
fn step_of(id: &gpui_kit::ElementId) -> Option<Option<usize>> {
    match id {
        gpui_kit::ElementId::NamedInteger(name, at) if name.starts_with("flow-task-") => Some(Some(*at as usize)),
        gpui_kit::ElementId::Name(name)
            if ["flow-minutes", "flow-time", "flow-instruction"].contains(&name.as_ref()) =>
        {
            Some(None)
        }
        _ => None,
    }
}

/// Where a task runs, as its picker says: the value picked.
fn target(cx: &mut TestAppContext, app: &App, at: usize) -> Option<String> {
    step(cx, app, Some(at));
    cx.run_until_parked();
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        w.try_find(("flow-task-target", at)).and_then(|picker| picker.value().map(String::from))
    })
    .unwrap()
}

/// Picks a choice of one of the dialog's pickers, of the task at its place.
fn choose(cx: &mut TestAppContext, app: &App, picker: (&'static str, usize), value: &str) {
    step(cx, app, Some(picker.1));
    app.click(cx, picker);
    let option = SharedString::from(format!("flow-option:{value}"));
    assert!(app.until(cx, |w| w.try_find(option.clone()).is_some()), "the choice {value}");
    app.click(cx, option);
}

/// Replaces what a field of the dialog holds, with its step shown first.
fn fill(cx: &mut TestAppContext, app: &App, id: impl Into<gpui_kit::ElementId>, text: &str) {
    let id = id.into();
    if let Some(at) = step_of(&id) {
        step(cx, app, at);
    }
    app.click(cx, id);
    app.press(cx, "cmd-a");
    app.typed(cx, text);
    cx.run_until_parked();
}

/// "Run now": the dialog closes, and the core has the workflow.
fn run_now(cx: &mut TestAppContext, app: &App, name: &str) {
    app.click(cx, "flow-run-now");
    assert!(app.until(cx, |w| w.try_find("flow-editor").is_none()), "the dialog closed");
    let started = Instant::now();
    while flow_named(app, name).is_none() && started.elapsed() < Duration::from_secs(10) {
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(flow_named(app, name).is_some(), "the core kept {name}");
}

/// The Workflows page with the run of the workflow `flow` shown: the page opened if it isn't, the run picked.
fn show_run(cx: &mut TestAppContext, app: &App, flow: &Flow) -> String {
    if !cx.update_window(app.window, |_, w, _| w.try_find("work-page").is_some()).unwrap() {
        app.press(cx, "cmd-i");
    }
    assert!(app.until(cx, |w| w.try_find("work-flows").is_some()), "Workflows in the header");
    if !cx.update_window(app.window, |_, w, _| w.try_find("flows-page").is_some()).unwrap() {
        app.click(cx, "work-flows");
    }
    assert!(app.until(cx, |w| w.try_find("flows-page").is_some()), "the Workflows page");
    let run = kept(app).flow_runs.into_iter().find(|run| run.flow_id == flow.id).expect("the run is kept");
    let key = format!("run:{}", run.id);
    let wanted = format!("{} ", run.id);
    // the runs are in the menu of the page's name (design 2, item 17.5)
    app.click(cx, "flows-switch");
    let item = |w: &gpui_kit::Window| {
        (0..20usize).find(|n| {
            w.try_find(("flow-run-item", *n))
                .is_some_and(|item| item.description().is_some_and(|d| d.starts_with(&wanted)))
        })
    };
    assert!(app.until(cx, |w| item(w).is_some()), "the run is listed");
    let at = cx.update_window(app.window, |_, w, _| item(w)).unwrap().expect("its place");
    app.click(cx, ("flow-run-item", at));
    assert!(
        app.until(cx, |w| w.try_find("flow-detail").is_some_and(|detail| detail.value() == Some(key.as_str()))),
        "the run is shown"
    );
    key
}

/// The lines of the run shown: each task's state, and whether it is the one picked.
fn lines(cx: &mut TestAppContext, app: &App) -> Vec<String> {
    cx.run_until_parked();
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        (0..12usize)
            .filter_map(|n| w.try_find(("flow-line", n)))
            .map(|line| format!("{} {}", line.value().unwrap_or_default(), line.selected() == Some(true)))
            .collect()
    })
    .unwrap()
}

fn says(cx: &mut TestAppContext, app: &App, id: impl Into<gpui_kit::ElementId>) -> Option<String> {
    let id = id.into();
    cx.run_until_parked();
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        said(w, id)
    })
    .unwrap()
}

/// Runs a workflow: each task's message sent to its Claude, Live following it, the Stop hook asking for the report,
/// then the next task.
#[gpui_kit::test]
fn runs_a_workflow_each_tasks_message_sent_to_its_claude_live_following_it_the_stop_hook_asking_for_the_report(
    cx: &mut TestAppContext,
) {
    let (app, fakes) = start(cx, |shared, project| App::make(shared, project, OTHER_NAME, "new", OTHER, Some("main")));
    let (first, second) = (workspace_named(&app, "sh-21349"), workspace_named(&app, OTHER_NAME));
    // where each workspace's branch is now: a workflow leaves it there, its work uncommitted
    let paths = [first.path.clone(), second.path.clone()];
    let heads: Vec<String> = paths.iter().map(|path| git(path, &["rev-parse", "HEAD"])).collect();
    workflow_for(cx, &app, &["SH-21349", "SH-21473"]);
    let (at_72, at_45) = (row(cx, &app, "SH-21349"), row(cx, &app, "SH-21473"));
    assert_eq!(target(cx, &app, at_72), Some(format!("workspace:{}", first.id)));
    assert_eq!(target(cx, &app, at_45), Some(format!("workspace:{}", second.id)));
    // SH-21473 set up its own way: your own words, and a new conversation in its workspace
    choose(cx, &app, ("flow-task-kind", at_45), "custom");
    fill(cx, &app, ("flow-task-words", at_45), "E2E: only touch fake-output.txt, nothing else.");
    choose(cx, &app, ("flow-task-target", at_45), &format!("conversation:{}", second.id));
    assert_eq!(target(cx, &app, at_45), Some(format!("conversation:{}", second.id)));
    fill(cx, &app, "flow-name", "E2E night");
    fill(cx, &app, "flow-minutes", "5");
    let before = sent(&app).len();
    let conversation_before = second.session_id.clone();
    let mut live_seen = vec![live_now(cx, &app)];
    let _runner = Runner::start(&app);
    run_now(cx, &app, "E2E night");

    // each task's message reaches its Claude whole and sent; the Stop hook asks for the report, the fake makes it,
    // screenshot and all; then the next task runs, and the end
    let flow = until_flow(cx, &app, "E2E night", "done", 120, &mut live_seen);
    let flow = flow.filter(|flow| flow.state == "done").unwrap_or_else(|| {
        panic!("the workflow never finished: {:?}\n{}", flow_named(&app, "E2E night"), terminal_of(cx, &first.id))
    });
    let prompts: Vec<(String, String)> = sent(&app).into_iter().skip(before).collect();
    let to_72 = prompts.iter().find(|(_, prompt)| prompt.contains("--ticket SH-21349")).expect("SH-21349's message");
    assert!(to_72.1.lines().next().is_some_and(|line| line.starts_with("SH-21349: ")), "{}", to_72.1);
    for said in ["workflow \"E2E night\"", "This workspace is Live while you work", "Don't commit and don't push"] {
        assert!(to_72.1.contains(said), "{said:?} is missing from:\n{}", to_72.1);
    }
    // in the board's order, each in its own workspace, each ended by its report
    let by_subject: Vec<(String, String, Option<String>)> =
        flow.tasks.iter().map(|task| (task.subject.clone(), task.state.clone(), task.workspace_id.clone())).collect();
    let ended = |key: &str| by_subject.iter().find(|(subject, _, _)| subject == &format!("ticket:{key}")).cloned();
    assert_eq!(ended("SH-21349").map(|(_, state, ws)| (state, ws)), Some(("done".into(), Some(first.id.clone()))));
    assert_eq!(ended("SH-21473").map(|(_, state, ws)| (state, ws)), Some(("done".into(), Some(second.id.clone()))));
    // SH-21473's own words reached its Claude, in a new conversation of its workspace
    let to_45 = prompts.iter().find(|(_, prompt)| prompt.contains("--ticket SH-21473")).expect("SH-21473's message");
    assert!(to_45.1.contains("E2E: only touch fake-output.txt, nothing else."), "{}", to_45.1);
    assert_ne!(to_45.0, conversation_before, "a new conversation");
    // each task's report, and the update each Claude gave along the way (`enkidu update`), screenshots and all
    let index = kept(&app);
    let fake: Vec<_> = index.reports.iter().filter(|report| report.text.starts_with("Fake Claude")).collect();
    let mut reported: Vec<&str> =
        fake.iter().filter(|report| report.state != "update").map(|report| report.subject.as_str()).collect();
    reported.sort();
    assert_eq!(reported, ["SH-21349", "SH-21473"]);
    assert!(fake.iter().filter(|report| report.state != "update").all(|report| report.images.len() == 1));
    let updates: Vec<_> = fake.iter().filter(|report| report.state == "update").collect();
    let mut updated: Vec<&str> = updates.iter().map(|report| report.subject.as_str()).collect();
    updated.sort();
    updated.dedup();
    assert_eq!(updated, ["SH-21349", "SH-21473"]);
    assert!(updates.iter().all(|report| report.text == "Fake Claude: halfway there." && report.images.len() == 1));
    // the hand-off: a task ends when Claude has reported and its turn has ended, and that turn's end wakes the
    // runner (which otherwise looks every 3 seconds), so the next task starts within the same second
    for task in &flow.tasks {
        let key = task.subject.trim_start_matches("ticket:");
        let reported =
            fake.iter().find(|report| report.state != "update" && report.subject == key).expect("its report");
        let ended = task.ended_at.expect("it ended");
        assert!(
            ended >= reported.at && ended - reported.at < 1500,
            "{key} ended {} ms after its report",
            ended - reported.at
        );
        // said with `--nocapture`, for the phase's report
        println!("RUN {key}: ended {} ms after its report", ended - reported.at);
    }
    let handed =
        flow.tasks[1].started_at.expect("the second started") - flow.tasks[0].ended_at.expect("the first ended");
    assert!((0..1500).contains(&handed), "the next task started {handed} ms after the first ended");
    let whole = flow.tasks[1].ended_at.unwrap_or_default() - flow.tasks[0].started_at.unwrap_or_default();
    println!("RUN the next task started {handed} ms after the first ended; both tasks took {whole} ms");
    // Live moved to each task as it ran, in the tasks' order, then back to what it was before (nothing)
    let order: Vec<Option<String>> = flow.tasks.iter().map(|task| task.workspace_id.clone()).collect();
    let mut expected = vec![None];
    expected.extend(order);
    expected.push(None);
    live_settled(cx, &app, &mut live_seen);
    assert_eq!(live_seen, expected, "every move of Live");
    assert_eq!(live_now(cx, &app), None);
    // nothing was committed: the fake tried, Enkidu's hook refused it; and Claude saw where it stood in the
    // workflow (`enkidu workflow`)
    let place = flow.tasks.iter().position(|task| task.subject == "ticket:SH-21349").expect("its place") + 1;
    let terminal = terminal_of(cx, &first.id);
    assert!(
        terminal.replace('\n', "").contains("Denied: Enkidu's workflow \"E2E night\" keeps every change uncommitted"),
        "{terminal}"
    );
    let stands = format!("Workflow \"E2E night\" (running): task {place} of 2, SH-21349 (");
    assert!(terminal.replace('\n', "").contains(&stands) && terminal.contains(" min left of 5."), "{terminal}");
    // each branch is where it was, the work in the diff
    let now: Vec<String> = paths.iter().map(|path| git(path, &["rev-parse", "HEAD"])).collect();
    assert_eq!(now, heads);
    for path in &paths {
        assert!(git(path, &["status", "--porcelain"]).contains("fake-output.txt"), "{path}");
    }
    // the tickets' timelines say what the workflow did
    let events: Vec<&str> = (index.events.iter())
        .filter(|event| event.kind == "workflow" && event.subject == "SH-21349")
        .map(|event| event.text.as_str())
        .collect();
    for said in ["Workflow \"E2E night\" started it in sh-21349", "Workflow \"E2E night\": Finished, nothing needs you"]
    {
        assert!(events.contains(&said), "{said:?} is missing from {events:?}");
    }

    // the Workflows page, in place of the board: the run is kept in its list; opened, how each task went
    show_run(cx, &app, &flow);
    assert_eq!(says(cx, &app, "flow-words").as_deref(), Some("Finished: 2 done"));
    assert_eq!(says(cx, &app, "flow-task-count").as_deref(), Some("2 tasks"));
    // a task picked: how it ended, the message it got, and what happened (Claude's update and report, screenshots
    // included); then its card, on top of the page
    let its = flow.tasks.iter().position(|task| task.subject == "ticket:SH-21473").expect("its place");
    app.click(cx, ("flow-line", its));
    assert!(
        app.until(cx, |w| w.try_find("run-task").is_some_and(|task| task.value() == Some("ticket:SH-21473"))),
        "the task's page"
    );
    let outcome = says(cx, &app, "run-task-outcome").unwrap_or_default();
    assert!(outcome.starts_with("Done, nothing needs you"), "{outcome}");
    let message = cx
        .update_window(app.window, |_, w, _| w.find("run-task-message").value().map(String::from))
        .unwrap()
        .unwrap_or_default();
    assert!(message.contains("E2E: only touch fake-output.txt, nothing else."), "{message}");
    let of_45: Vec<_> = fake.iter().filter(|report| report.subject == "SH-21473").collect();
    let (update, report) = (
        of_45.iter().find(|report| report.state == "update").expect("its update"),
        of_45.iter().find(|report| report.state != "update").expect("its report"),
    );
    let text = |id: i64| SharedString::from(format!("report-text:{id}"));
    assert_eq!(says(cx, &app, text(update.id)).as_deref(), Some("Fake Claude: halfway there."));
    assert_eq!(says(cx, &app, text(report.id)).as_deref(), Some("Fake Claude: made the change and checked it."));
    // both screenshots are there, and shown once they are read
    for (id, name) in [(update.id, &update.images[0]), (report.id, &report.images[0])] {
        let shot = SharedString::from(format!("report-image:{id}:{name}"));
        assert!(
            app.until(cx, |w| w.try_find(shot.clone()).is_some_and(|shot| shot.description() == Some("shown"))),
            "the screenshot {name} of report {id}"
        );
    }
    app.click(cx, "run-task-card");
    // the card shows the report too: it is on screen twice now, in the card and behind it on the page
    let reported = SharedString::from(format!("task-report:{}", report.id));
    assert!(app.until(cx, |w| w.try_find("task-dialog").is_some() && w.find_all(reported.clone()).len() == 2));
    app.press(cx, "escape");
    assert!(app.until(cx, |w| w.try_find("task-dialog").is_none()));
    // the page is still there; Esc goes back to the board
    assert!(cx.update_window(app.window, |_, w, _| w.try_find("flows-page").is_some()).unwrap());
    app.press(cx, "escape");
    assert!(app.until(cx, |w| w.try_find("flows-page").is_none() && w.try_find("work-board").is_some()));
    common::never_wrote(&fakes);
}

/// Runs a task that has a workspace in a new one when you pick that: a new branch from its own, the old workspace
/// untouched, a new tab.
#[gpui_kit::test]
fn runs_a_task_that_has_a_workspace_in_a_new_one_when_you_pick_that_a_new_branch_from_its_own_the_old_one_untouched(
    cx: &mut TestAppContext,
) {
    let (app, fakes) = start(cx, |_, _| {});
    let old = workspace_named(&app, "sh-21349");
    // something uncommitted in the old workspace, to see that it stays
    std::fs::write(std::path::Path::new(&old.path).join("notes.txt"), "mine\n").expect("a file in the workspace");
    let (head, status) = (git(&old.path, &["rev-parse", "HEAD"]), git(&old.path, &["status", "--porcelain"]));
    assert!(status.contains("notes.txt"));
    workflow_for(cx, &app, &["SH-21349"]);
    let at = row(cx, &app, "SH-21349");
    // by default it runs in its own workspace; a new workspace picked instead
    assert_eq!(target(cx, &app, at), Some(format!("workspace:{}", old.id)));
    choose(cx, &app, ("flow-task-target", at), "new");
    assert_eq!(target(cx, &app, at).as_deref(), Some("new"));
    fill(cx, &app, "flow-name", "E2E fresh");
    fill(cx, &app, "flow-minutes", "5");
    let mut live_seen = Vec::new();
    let _runner = Runner::start(&app);
    run_now(cx, &app, "E2E fresh");
    let flow = until_flow(cx, &app, "E2E fresh", "done", 180, &mut live_seen);
    let flow = flow.filter(|flow| flow.state == "done").unwrap_or_else(|| {
        panic!("the workflow never finished: {:?}", flow_named(&app, "E2E fresh"));
    });
    let task = &flow.tasks[0];
    assert_eq!(task.state, "done", "{:?}", task.note);
    let made_id = task.workspace_id.clone().expect("the workspace it ran in");
    assert_ne!(made_id, old.id);
    // a new workspace, on a new branch starting where the ticket's branch is (git checks a branch out in one folder)
    let made = app.shared.db.workspace(&made_id).expect("the database").expect("the workspace made for it");
    assert_eq!(made.branch, format!("{BRANCH}-2"));
    assert_eq!(
        git(&made.path, &["rev-parse", &format!("{BRANCH}-2~0")]),
        head,
        "it starts where the ticket's branch is"
    );
    // the window has it as a new tab
    assert!(app.until_frame(cx, |frame| frame.workspaces.iter().any(|workspace| workspace.id == made_id)));
    // the old workspace is as it was: its branch, its uncommitted changes
    assert_eq!(git(&old.path, &["rev-parse", "HEAD"]), head);
    assert_eq!(git(&old.path, &["status", "--porcelain"]), status);
    assert_eq!(git(&old.path, &["branch", "--show-current"]), BRANCH);
    // the task keeps the workspace made for it: running it again goes on there
    assert_eq!(task.target, Target::Workspace { workspace_id: made_id });
    common::never_wrote(&fakes);
}

/// Stops a run at Claude's usage limit: the task cut off in Claude Code's words, its lamp asking for you, the next task
/// not run.
#[gpui_kit::test]
fn stops_a_run_at_claudes_usage_limit_the_task_cut_off_in_claude_codes_words_the_next_task_not_run(
    cx: &mut TestAppContext,
) {
    let (app, fakes) = start(cx, |shared, project| App::make(shared, project, OTHER_NAME, "new", OTHER, Some("main")));
    let cut = workspace_named(&app, OTHER_NAME);
    workflow_for(cx, &app, &["SH-21349", "SH-21473"]);
    let (at_72, at_45) = (row(cx, &app, "SH-21349"), row(cx, &app, "SH-21473"));
    // SH-21473 runs first; its words make the fake Claude hit the usage limit
    assert!(at_45 < at_72, "SH-21473 comes first on the board");
    choose(cx, &app, ("flow-task-kind", at_45), "custom");
    fill(cx, &app, ("flow-task-words", at_45), "E2E: FAKE_LIMIT, then nothing else.");
    fill(cx, &app, "flow-name", "E2E limit");
    fill(cx, &app, "flow-minutes", "5");
    let before = sent(&app).len();
    let mut live_seen = vec![live_now(cx, &app)];
    let _runner = Runner::start(&app);
    run_now(cx, &app, "E2E limit");
    // the StopFailure hook ends the task at once (not after its 5 minutes), and the limit stops the run
    let flow = until_flow(cx, &app, "E2E limit", "stopped", 90, &mut live_seen);
    let flow = flow.filter(|flow| flow.state == "stopped").unwrap_or_else(|| {
        panic!("the run never stopped: {:?}\n{}", flow_named(&app, "E2E limit"), terminal_of(cx, &cut.id))
    });
    let limit = "You've hit your limit · resets 3am (Europe/Paris)";
    let by_subject: Vec<(&str, &str, Option<&str>)> =
        flow.tasks.iter().map(|task| (task.subject.as_str(), task.state.as_str(), task.note.as_deref())).collect();
    assert_eq!(by_subject, [("ticket:SH-21473", "cut_off", Some(limit)), ("ticket:SH-21349", "waiting", None)]);
    let after: Vec<(String, String)> = sent(&app).into_iter().skip(before).collect();
    assert!(
        !after.iter().any(|(_, prompt)| prompt.contains("--ticket SH-21349")),
        "the next task's Claude got nothing"
    );
    // Live followed the task, then went back; the lamp of the Claude that was cut off asks for you
    live_settled(cx, &app, &mut live_seen);
    assert_eq!(live_seen, [None, Some(cut.id.clone()), None]);
    assert!(app.until_frame(cx, |frame| frame.statuses.get(&cut.id).map(String::as_str) == Some("needs_you")));
    let index = kept(&app);
    let events: Vec<&str> = (index.events.iter())
        .filter(|event| event.kind == "workflow" && event.subject == "SH-21473")
        .map(|event| event.text.as_str())
        .collect();
    let said = format!("Workflow \"E2E limit\": Claude was cut off: {limit}");
    assert!(events.contains(&said.as_str()), "{events:?}");
    // the run on the Workflows page: stopped, opening on the task that was cut off; the next one never ran
    show_run(cx, &app, &flow);
    assert_eq!(says(cx, &app, "flow-words").as_deref(), Some("Stopped: 1 cut off"));
    assert_eq!(lines(cx, &app), ["cut_off true", "stopped false"]);
    let outcome = says(cx, &app, "run-task-outcome").unwrap_or_default();
    assert!(outcome.contains("You've hit your limit · resets 3am"), "{outcome}");
    app.press(cx, "escape");
    assert!(app.until(cx, |w| w.try_find("flows-page").is_none()));
    common::never_wrote(&fakes);
}

/// Item 8.7 on the whole wiring: New task, under the workflow dialog's tasks, makes a task of your own in the
/// workflow's
/// project. It joins the tasks as the last one, in your own words. The workflow runs it in a new workspace named for
/// it; its message names it by its number; Claude's report lands on its card, and the task ends done.
#[gpui_kit::test]
fn a_task_of_your_own_made_in_the_workflow_dialog_joins_the_workflow_and_runs(cx: &mut TestAppContext) {
    let (app, _fakes) = start(cx, |_, _| {});
    workflow_for(cx, &app, &["SH-21349"]);
    app.click(cx, "flow-new-task");
    assert!(
        app.until(cx, |w| w.try_find("own-task-dialog").is_some()),
        "the task's dialog, in front of the workflow's"
    );
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        assert!(w.try_find(("own-task-project", 0usize)).is_none(), "its project is the workflow's: nothing to pick");
    })
    .unwrap();
    app.click(cx, "own-task-title");
    app.typed(cx, "Write fake-output.txt");
    app.click(cx, "own-task-save");
    assert!(
        app.until(cx, |w| w.try_find("own-task-dialog").is_none() && w.try_find("flow-editor").is_some()),
        "only the task's dialog closes"
    );
    // the core has the task, in the workflow's project, to do
    let own = app.until_work(cx, |work| work.own_tasks.len() == 1).own_tasks[0].clone();
    assert_eq!((own.id, own.project_id.as_str(), own.state.as_str()), (1, app.project_id.as_str(), "todo"));
    // it is the dialog's last task
    let at = {
        let mut found = None;
        for _ in 0..400 {
            cx.run_until_parked();
            found = cx
                .update_window(app.window, |_, w, cx| {
                    w.render_frame(cx);
                    (0..12usize).find(|n| w.try_find(("flow-task", *n)).is_some_and(|row| row.value() == Some("own:1")))
                })
                .unwrap();
            if found.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        found.expect("the new task's row in the dialog")
    };
    assert_eq!(at, 1, "after the ticket's");
    // made here, it is the step shown, to set it up
    assert!(app.until(cx, |w| w.try_find(("flow-task-kind", 1usize)).is_some()), "the new task's setup is shown");
    // only the new task runs here (the ticket's own run is another test's): its words, a name, five minutes
    step(cx, &app, Some(0));
    app.click(cx, ("flow-task-remove", 0usize));
    fill(cx, &app, ("flow-task-words", 0usize), "E2E: only touch fake-output.txt, nothing else.");
    fill(cx, &app, "flow-name", "Own night");
    fill(cx, &app, "flow-minutes", "5");
    let before = sent(&app).len();
    let mut live_seen = vec![live_now(cx, &app)];
    let _runner = Runner::start(&app);
    run_now(cx, &app, "Own night");
    let flow = until_flow(cx, &app, "Own night", "done", 120, &mut live_seen);
    let flow = flow
        .filter(|flow| flow.state == "done")
        .unwrap_or_else(|| panic!("the workflow never finished: {:?}", flow_named(&app, "Own night")));
    assert_eq!(flow.tasks.len(), 1);
    let task = &flow.tasks[0];
    assert_eq!((task.subject.as_str(), task.kind.as_str(), task.state.as_str()), ("own:1", "custom", "done"));
    // in a new workspace named for the task, on its own new branch
    let made = workspace_named(&app, "task-1-write-fake-output-txt");
    assert_eq!(made.branch, "feature/task-1-write-fake-output-txt");
    assert_eq!(task.workspace_id.as_deref(), Some(made.id.as_str()));
    // its message says what it is and names it by its number; the report landed on its card
    let prompts: Vec<(String, String)> = sent(&app).into_iter().skip(before).collect();
    let message = &prompts.iter().find(|(_, prompt)| prompt.contains("--task 1")).expect("the task's message").1;
    assert!(message.starts_with("Task 1: Write fake-output.txt\nIt is a task of mine, not in Jira."), "{message}");
    assert!(message.contains("E2E: only touch fake-output.txt, nothing else."), "{message}");
    let index = kept(&app);
    let reports: Vec<(&str, &str)> = (index.reports.iter())
        .filter(|report| report.state != "update")
        .map(|report| (report.subject.as_str(), report.state.as_str()))
        .collect();
    assert_eq!(reports, [("own:1", "done")]);
    // and its timeline says what the workflow did with it
    let told: Vec<&str> = index.events.iter().filter(|e| e.subject == "own:1").map(|e| e.kind.as_str()).collect();
    assert!(told.contains(&"workflow") && told.contains(&"report") && told.contains(&"own"), "{told:?}");
    live_settled(cx, &app, &mut live_seen);
}
