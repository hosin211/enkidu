//! Items 21.5 and 21.6: Setup, the page of Settings that says what the app needs on this Mac and how each thing
//! stands (DESIGN.md, "Setup"). First with sample finds (what each state says, what a press copies, where a project
//! without Jira keys leads), then on the real core with a `claude`, a `gh` and a `twg` that are the fakes (never the
//! real ones), and last the first launch: with no project yet the page opens by itself, once.

mod common;

use common::{App, said};
use enkidu_app::demo::SetupSample;
use enkidu_model::setup::{JIRA_PROJECTS, WELCOME};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext, Window, px};

/// A window with sample data and Settings open on Setup, showing `sample`.
fn sample(cx: &mut TestAppContext, sample: SetupSample) -> AnyWindowHandle {
    let (window, _) = cx.update(|cx| {
        enkidu_app::init(cx);
        cx.set_global(sample);
        gpui_kit::open_window(
            gpui_kit::WindowOptions {
                window_bounds: Some(gpui_kit::WindowBounds::Windowed(gpui_kit::Bounds {
                    origin: Default::default(),
                    size: gpui_kit::size(px(1360.), px(860.)),
                })),
                ..Default::default()
            },
            cx,
            |window, cx| cx.new(|cx| enkidu_app::shell::Shell::demo(window, cx)),
        )
        .expect("a test window")
    });
    let window: AnyWindowHandle = window;
    cx.update_window(window, |_, w, cx| {
        w.render_frame(cx);
        w.press("cmd-,", cx);
        w.render_frame(cx);
        w.click("settings-nav-setup", cx);
    })
    .unwrap();
    window
}

fn look<R>(cx: &mut TestAppContext, window: AnyWindowHandle, at: impl FnOnce(&mut Window) -> R) -> R {
    cx.run_until_parked();
    cx.update_window(window, |_, w, cx| {
        w.render_frame(cx);
        at(w)
    })
    .unwrap()
}

/// A thing's row as the window says it: "Ready: `/usr/bin/git`, version 2.50.1".
fn row(window: &Window, thing: &str) -> Option<String> {
    window.try_find(format!("setup-row:{thing}")).and_then(|row| row.description().map(String::from))
}

/// The commands a row offers, as its chips name them.
fn commands(window: &Window, thing: &str) -> Vec<String> {
    (0..4).filter_map(|ix| said(window, format!("setup-copy:{thing}:{ix}"))).collect()
}

/// The page on screen, when the dialog is open.
fn page(window: &Window) -> Option<String> {
    window.try_find("settings-dialog").and_then(|dialog| dialog.description().map(String::from))
}

fn copied(cx: &mut TestAppContext) -> Option<String> {
    cx.run_until_parked();
    cx.read_from_clipboard().and_then(|copied| copied.text())
}

#[gpui_kit::test]
fn each_thing_says_how_it_stands_and_a_press_copies_the_command_that_puts_it_right(cx: &mut TestAppContext) {
    let window = sample(cx, SetupSample::New);
    look(cx, window, |w| {
        assert_eq!(page(w).as_deref(), Some("Setup"));
        assert_eq!(said(w, "settings-title").as_deref(), Some("Setup"));
        assert_eq!(w.find("settings-nav-setup").selected(), Some(true), "its place in the list is the one marked");
        // what is there
        assert_eq!(row(w, "git").as_deref(), Some("Ready: `/usr/bin/git`, version 2.50.1"));
        assert!(commands(w, "git").is_empty(), "nothing to run where nothing is wrong");
        // what no workspace starts without, and isn't there
        assert_eq!(
            row(w, "claude").as_deref(),
            Some("Not installed: Runs in every workspace. Install it, then start it once in a terminal and sign in.")
        );
        assert_eq!(commands(w, "claude"), ["Copy curl -fsSL https://claude.ai/install.sh | bash", "Copy claude"]);
        // the connections: signed out, and not there at all
        assert_eq!(
            row(w, "github").as_deref(),
            Some("Not signed in: `gh` 2.103.3 is here. Sign in to read your pull requests.")
        );
        assert_eq!(commands(w, "github"), ["Copy gh auth login"]);
        assert_eq!(said(w, "setup-stands:jira").as_deref(), Some("Not set up"));
        assert_eq!(
            commands(w, "jira"),
            ["Copy curl -fsSL --retry 2 https://teamwork-graph.atlassian.com/cli/install | bash", "Copy twg login"]
        );
        // nothing about nothing: no welcome on a page opened from the list, no projects under a Jira that isn't
        // read, no part of the build that isn't missing
        for absent in ["setup-welcome", "setup-jira-projects", "setup-row:cli", "setup-row:python"] {
            assert!(w.try_find(absent).is_none(), "{absent}");
        }
        assert_eq!(w.find("setup-page").description(), Some("looked"));
        assert_eq!(said(w, "setup-check").as_deref(), Some("Check again"));
    });
    // a press on a command copies it
    cx.update_window(window, |_, w, cx| w.click("setup-copy:claude:0", cx)).unwrap();
    assert_eq!(copied(cx).as_deref(), Some("curl -fsSL https://claude.ai/install.sh | bash"));
    cx.update_window(window, |_, w, cx| w.click("setup-copy:github:0", cx)).unwrap();
    assert_eq!(copied(cx).as_deref(), Some("gh auth login"));
}

#[gpui_kit::test]
fn a_project_whose_tickets_arent_read_leads_to_its_page_and_a_build_says_what_it_lacks(cx: &mut TestAppContext) {
    let window = sample(cx, SetupSample::Partly);
    look(cx, window, |w| {
        assert_eq!(row(w, "jira").as_deref(), Some("Ready: `twg` 1.4.2, signed in as Me Dev"));
        assert_eq!(row(w, "claude").as_deref(), Some("Ready: `~/.local/bin/claude`, version 2.1.325, signed in"));
        // under Jira: the project that reads none of it yet
        assert_eq!(said(w, "setup-jira-projects").as_deref(), Some(JIRA_PROJECTS));
        assert_eq!(said(w, ("setup-jira-project", 0usize)).as_deref(), Some("enkidu"));
        assert!(w.try_find(("setup-jira-project", 1usize)).is_none(), "shop reads its tickets already");
        // this build carries its enkidu and lacks the Python server: only the second is said
        assert!(w.try_find("setup-row:cli").is_none());
        let python = row(w, "python");
        assert!(python.as_deref().is_some_and(|said| said.starts_with("Missing: ⌘-click in Python")), "{python:?}");
        assert_eq!(commands(w, "python"), ["Copy sh scripts/fetch-ty.sh", "Copy sh scripts/install.sh"]);
    });
    // the project's name goes to its own page, where the keys are typed (it stands under the connections: the page
    // scrolls to it)
    cx.update_window(window, |_, w, cx| {
        let down = gpui_kit::ScrollDelta::Pixels(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(-2000.)));
        w.scroll("settings-body", down, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, w, cx| {
        w.render_frame(cx);
        w.click(("setup-jira-project", 0usize), cx)
    })
    .unwrap();
    look(cx, window, |w| {
        assert_eq!(page(w).as_deref(), Some("enkidu"));
        assert!(w.try_find("setup-page").is_none());
    });
    // and back: what was found is still there, nothing is asked again by itself
    cx.update_window(window, |_, w, cx| w.click("settings-nav-setup", cx)).unwrap();
    look(cx, window, |w| assert_eq!(row(w, "jira").as_deref(), Some("Ready: `twg` 1.4.2, signed in as Me Dev")));
}

/// Whether the window's own file says that Setup was shown on this Mac. It is written a moment after the change, by
/// the test's clock.
fn kept_as_seen(cx: &mut TestAppContext, app: &App) -> bool {
    let file = app.home.path().join("data").join("ui.json");
    let seen = || std::fs::read_to_string(&file).is_ok_and(|kept| kept.contains(r#""setupSeen":true"#));
    for _ in 0..600 {
        cx.executor().advance_clock(std::time::Duration::from_millis(400));
        cx.run_until_parked();
        if seen() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    seen()
}

/// Waits until every row was looked at, and `seen` holds.
fn looked(cx: &mut TestAppContext, app: &App, seen: impl Fn(&Window) -> bool) -> bool {
    app.until(cx, |w| w.try_find("setup-page").is_some_and(|page| page.description() == Some("looked")) && seen(w))
}

fn no_dialog(cx: &mut TestAppContext, app: &App) -> bool {
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        w.try_find("settings-dialog").is_none()
    })
    .unwrap()
}

#[gpui_kit::test]
fn the_page_asks_the_programs_themselves_and_check_again_asks_them_again(cx: &mut TestAppContext) {
    // the live window: its Claude is the fake one, and the core's gh and twg are the fakes before anything asks them
    let app = common::start_live(cx, enkidu_testkit::shop, |shared, _, home| drop(common::give_fakes(shared, home)));
    let fakes = app.fakes();
    let starts = app.claude_starts().len();
    app.press(cx, "cmd-,");
    assert!(app.until(cx, |w| w.try_find("settings-nav-setup").is_some()));
    app.click(cx, "settings-nav-setup");
    assert!(looked(cx, &app, |w| said(w, "setup-stands:jira").is_some()), "every program answered");
    let (git, claude, github, jira) = cx
        .update_window(app.window, |_, w, cx| {
            w.render_frame(cx);
            (row(w, "git"), row(w, "claude"), row(w, "github"), row(w, "jira"))
        })
        .unwrap();
    // git is this Mac's own (every test uses it); its version is whatever it is
    assert!(git.as_deref().is_some_and(|said| said.starts_with("Ready: `") && said.contains("`, version ")), "{git:?}");
    let signed_in = |said: &str| said.starts_with("Ready: ") && said.contains(", version 2.1.325, signed in");
    assert!(claude.as_deref().is_some_and(signed_in), "{claude:?}");
    assert_eq!(github.as_deref(), Some("Ready: `gh` 2.103.3, signed in as me-dev"));
    assert_eq!(jira.as_deref(), Some("Ready: `twg` 1.4.2, signed in as Me Dev"));
    // Jira is ready, and this project has no Jira project key yet: it is named, and leads to its page
    let named = cx.update_window(app.window, |_, w, _| said(w, ("setup-jira-project", 0usize))).unwrap();
    assert_eq!(named.as_deref(), Some("shop"));

    // everyone signs out; nothing on the page changes by itself
    fakes.gh("logged-out");
    fakes.twg("logged-out");
    let account = app.home.path().join("claude-config");
    std::fs::create_dir_all(&account).unwrap();
    std::fs::write(account.join("signed-out"), "").unwrap();
    cx.run_until_parked();
    let stands = cx.update_window(app.window, |_, w, _| said(w, "setup-stands:github")).unwrap();
    assert_eq!(stands.as_deref(), Some("Ready"));
    // Check again asks every program once more
    app.click(cx, "setup-check");
    assert!(
        looked(cx, &app, |w| said(w, "setup-stands:github").as_deref() == Some("Not signed in")),
        "after Check again: {:?}",
        cx.update_window(app.window, |_, w, _| row(w, "github")).unwrap()
    );
    let (claude, claude_runs, github_runs, jira, jira_runs, projects) = cx
        .update_window(app.window, |_, w, cx| {
            w.render_frame(cx);
            let projects = w.try_find("setup-jira-projects").is_some();
            (
                row(w, "claude"),
                commands(w, "claude"),
                commands(w, "github"),
                row(w, "jira"),
                commands(w, "jira"),
                projects,
            )
        })
        .unwrap();
    let signed_out = |said: &str| said.starts_with("Not signed in: ") && said.contains("Nobody is signed in");
    assert!(claude.as_deref().is_some_and(signed_out), "{claude:?}");
    // signing in is for the account's folder Claude starts with here
    let sign_in = format!("CLAUDE_CONFIG_DIR={} claude auth login", account.display());
    assert_eq!(claude_runs, [format!("Copy {sign_in}")]);
    assert_eq!(github_runs, ["Copy gh auth login"]);
    assert_eq!(jira.as_deref(), Some("Not signed in: `twg` 1.4.2 is here. Sign in to read your tickets."));
    // twg's own word for it
    assert_eq!(jira_runs, ["Copy twg login --force"]);
    assert!(!projects, "a Jira that isn't read has no projects to name");
    app.click(cx, "setup-copy:claude:0");
    assert_eq!(copied(cx), Some(sign_in));

    // looking never started a Claude, wrote nothing to GitHub or Jira, and asked them only who is signed in
    assert_eq!(app.claude_starts().len(), starts, "no session was started to look");
    common::never_wrote(&fakes);
    let asked: Vec<String> =
        fakes.twg_log().lines().map(|line| line.split(" | ").nth(1).unwrap_or("").to_string()).collect();
    assert!(asked.iter().all(|call| call.starts_with("whoami")), "{asked:?}");
    assert!(fakes.gh_log().lines().all(|line| line.contains("auth status")), "{}", fakes.gh_log());
}

#[gpui_kit::test]
fn on_a_first_launch_setup_opens_by_itself_once(cx: &mut TestAppContext) {
    // a Mac that has no project yet: the fakes are given before the window opens, since the page asks at once
    let app = common::start_fresh(cx, |shared, _, home| drop(common::give_fakes(shared, home)));
    assert!(app.until(cx, |w| w.try_find("setup-welcome").is_some()), "Setup opened by itself");
    let (shown, welcome) = cx.update_window(app.window, |_, w, _| (page(w), said(w, "setup-welcome"))).unwrap();
    assert_eq!((shown.as_deref(), welcome.as_deref()), (Some("Setup"), Some(WELCOME)));
    assert!(looked(cx, &app, |w| row(w, "github").is_some_and(|said| said.starts_with("Ready: "))));
    // it is kept that Setup was shown here
    assert!(kept_as_seen(cx, &app), "ui.json never said so");
    // the welcome saves nothing: its one button is Done
    let foot = |w: &Window| (w.try_find("setup-done").is_some(), w.try_find("save-settings").is_some());
    assert_eq!(cx.update_window(app.window, |_, w, _| foot(w)).unwrap(), (true, false));
    // closed, the first screen is there with its one button
    app.click(cx, "setup-done");
    assert!(app.until(cx, |w| w.try_find("settings-dialog").is_none() && w.try_find("add-first-project").is_some()));

    // the next launch: no project still, and the page stays where it is, in Settings
    let app = common::restart(cx, app);
    assert!(app.until(cx, |w| w.try_find("add-first-project").is_some()));
    for _ in 0..20 {
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(no_dialog(cx, &app), "it opened again");
    // from Settings it is the page it always is: no welcome
    drop(app.fakes());
    app.press(cx, "cmd-,");
    assert!(app.until(cx, |w| w.try_find("settings-nav-setup").is_some()));
    app.click(cx, "settings-nav-setup");
    assert!(app.until(cx, |w| w.try_find("setup-page").is_some()));
    assert!(cx.update_window(app.window, |_, w, _| w.try_find("setup-welcome").is_none()).unwrap());
    assert_eq!(
        cx.update_window(app.window, |_, w, _| foot(w)).unwrap(),
        (false, true),
        "Cancel and Save, as on every page"
    );
}

#[gpui_kit::test]
fn with_a_project_already_there_setup_never_opens_by_itself(cx: &mut TestAppContext) {
    let app = common::start_live(cx, enkidu_testkit::shop, |shared, _, home| drop(common::give_fakes(shared, home)));
    assert!(kept_as_seen(cx, &app), "a Mac with a project is past the welcome");
    assert!(no_dialog(cx, &app));
}

/// Item 21.12, found by the load run: with the keyboard in a field of one page, a click to a page that has no field
/// (Setup) left the keyboard on the field that was no longer drawn, and Esc closed nothing. The dialog holds the
/// keyboard itself when the page changes.
#[gpui_kit::test]
fn esc_still_closes_settings_after_a_page_without_a_field_took_the_place_of_one_with_the_keyboard(
    cx: &mut TestAppContext,
) {
    let app = common::start_on(cx, enkidu_testkit::shop, |shared, _, home| drop(common::give_fakes(shared, home)));
    let _fakes = app.fakes();
    app.press(cx, "cmd-,");
    assert!(app.until(cx, |w| w.try_find(("settings-nav", 3usize)).is_some()));
    // the keyboard into a field of the project's page (typed into, so it has it), then over to Setup
    app.click(cx, ("settings-nav", 3usize));
    assert!(app.until(cx, |w| w.try_find("default-base").is_some()));
    app.click(cx, "default-base");
    for letter in ["m", "a", "i", "n"] {
        app.typed(cx, letter);
        cx.run_until_parked();
    }
    app.click(cx, "settings-nav-setup");
    assert!(looked(cx, &app, |w| page(w).as_deref() == Some("Setup")));
    app.press(cx, "escape");
    assert!(app.until(cx, |w| w.try_find("settings-dialog").is_none()), "Esc closed nothing");
}
