//! Item 4.3: Settings, in a real window on the real core. One dialog with its pages on the left: General, Claude,
//! and one for each project. Nothing is written until Save; then the user's settings and each edited project's are
//! in the core's files, and the layout in the window's own.

mod common;

use common::{App, said};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, ScrollDelta, TestAppContext, Window, point, px};

fn prepare(shared: &std::sync::Arc<enkidu_core::state::Shared>, project: &str, _: &std::path::Path) {
    // what the page's Work part edits: untouched, it must stay as it is
    let mut settings = shared.project_settings(project).unwrap();
    settings.work.jira_keys = vec!["SH".into()];
    settings.work.every_minutes = 7;
    settings.copy_files = vec![".env".into()];
    shared.save_project_settings(project, &settings).unwrap();
    App::make(shared, project, "stock-fix", "new", "stock-fix", Some("development"));
}

/// Throwaway Claude folders for this test binary, named for the core (`ENKIDU_CLAUDE_DIRS`): no test here reads or
/// changes a real Claude folder, and the hooks can be added to these two (item 4.4).
fn claude_folders() -> (std::path::PathBuf, std::path::PathBuf) {
    static NAMED: std::sync::Once = std::sync::Once::new();
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("settings-claude-folders");
    let (plain, work) = (root.join("claude"), root.join("claude-work"));
    NAMED.call_once(|| {
        let _ = std::fs::remove_dir_all(&root);
        for dir in [&plain, &work] {
            std::fs::create_dir_all(dir.join("projects")).unwrap();
        }
        // SAFETY: set once, to the same value, before any core of this test binary is asked for the Claude folders.
        unsafe { std::env::set_var("ENKIDU_CLAUDE_DIRS", format!("{}:{}", plain.display(), work.display())) };
    });
    (plain, work)
}

fn start(cx: &mut TestAppContext) -> App {
    claude_folders();
    let app = common::start_on(cx, enkidu_testkit::shop, prepare);
    assert!(app.until(cx, |w| w.try_find(("tab", 0usize)).is_some()), "the workspace's tab");
    app
}

/// The same with the fake Claude and the hook server, as the app runs: "Claude starts like this" needs both.
fn start_live(cx: &mut TestAppContext) -> App {
    claude_folders();
    let app = common::start_live(cx, enkidu_testkit::shop, prepare);
    assert!(app.until(cx, |w| w.try_find(("tab", 0usize)).is_some()), "the workspace's tab");
    app
}

fn look<R>(cx: &mut TestAppContext, app: &App, at: impl FnOnce(&mut Window) -> R) -> R {
    cx.run_until_parked();
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        at(w)
    })
    .unwrap()
}

/// The page on screen, when the dialog is open.
fn page(window: &Window) -> Option<String> {
    window.try_find("settings-dialog").and_then(|dialog| dialog.description().map(String::from))
}

/// The pages listed on the left, the one on screen with a star.
fn pages(window: &Window) -> Vec<String> {
    (0..12usize)
        .filter_map(|ix| window.try_find(("settings-nav", ix)))
        .map(|item| {
            let star = if item.selected() == Some(true) { " *" } else { "" };
            format!("{}{star}", item.label().unwrap_or_default())
        })
        .collect()
}

/// Scrolls the page until the field is whole inside it, as a hand on the trackpad would.
fn bring(cx: &mut TestAppContext, app: &App, id: impl Into<gpui_kit::ElementId>) {
    let id: gpui_kit::ElementId = id.into();
    for _ in 0..60 {
        let whole = look(cx, app, |w| {
            let body = w.find("settings-body").bounds();
            w.try_find(id.clone()).map(|field| field.bounds()).map(|field| {
                if field.top() < body.top() {
                    1
                } else if field.bottom() > body.bottom() {
                    -1
                } else {
                    0
                }
            })
        });
        let Some(way) = whole else { panic!("{id:?} isn't on this page") };
        if way == 0 {
            return;
        }
        let by = ScrollDelta::Pixels(point(px(0.), px(60. * way as f32)));
        cx.update_window(app.window, |_, w, cx| w.scroll("settings-body", by, cx)).unwrap();
    }
    panic!("{id:?} never came into view");
}

/// Puts `text` in a field in place of what it has.
fn fill(cx: &mut TestAppContext, app: &App, id: &'static str, text: &str) {
    bring(cx, app, id);
    app.click(cx, id);
    app.press(cx, "cmd-a");
    app.typed(cx, text);
    cx.run_until_parked();
}

/// The same for a field of several lines: Enter between them is a new line.
fn fill_lines(cx: &mut TestAppContext, app: &App, id: &'static str, lines: &[String]) {
    bring(cx, app, id);
    app.click(cx, id);
    app.press(cx, "cmd-a");
    for (ix, line) in lines.iter().enumerate() {
        if ix > 0 {
            app.press(cx, "enter");
        }
        app.typed(cx, line);
    }
    cx.run_until_parked();
}

fn closed(cx: &mut TestAppContext, app: &App) -> bool {
    app.until(cx, |w| w.try_find("settings-dialog").is_none())
}

/// Item 4.3c: the dialog, its pages, and the ways into it.
#[gpui_kit::test]
fn settings_open_on_a_page_from_the_key_the_gear_the_palette_and_a_projects_menu(cx: &mut TestAppContext) {
    let app = start(cx);
    // ⌘,
    app.press(cx, "cmd-,");
    assert!(app.until(cx, |w| page(w).as_deref() == Some("General")), "⌘, opened nothing");
    look(cx, &app, |w| {
        assert_eq!(pages(w), ["General *", "Claude", "Workflows", "shop"]);
        assert_eq!(said(w, "settings-title").as_deref(), Some("General"));
        // 820 px wide with its hairline edge, and no taller than 620
        let dialog = w.find("settings-dialog").bounds();
        assert_eq!(dialog.size.width, gpui_kit::px(818.));
        assert_eq!(dialog.size.height, gpui_kit::px(620.));
    });
    // in a small window it keeps 16 px on each side and is 84% as tall
    cx.simulate_window_resize(app.window, gpui_kit::size(px(760.), px(600.)));
    look(cx, &app, |w| {
        let dialog = w.find("settings-dialog").bounds();
        assert_eq!((dialog.size.width, dialog.size.height), (px(726.), px(504.)));
        assert_eq!(dialog.top(), px(48.) + px(1.), "8% down the window, under the dialog's hairline");
    });
    cx.simulate_window_resize(app.window, gpui_kit::size(px(1360.), px(860.)));
    // the pages on the left
    app.click(cx, ("settings-nav", 1usize));
    assert!(app.until(cx, |w| page(w).as_deref() == Some("Claude") && pages(w)[1] == "Claude *"));
    app.click(cx, ("settings-nav", 3usize));
    assert!(app.until(cx, |w| page(w).as_deref() == Some("shop") && w.try_find("default-base").is_some()));
    // Close, at the top
    app.click(cx, "settings-close");
    assert!(closed(cx, &app), "Close left it open");

    // the gear on the rail; Cancel
    app.click(cx, "settings");
    assert!(app.until(cx, |w| page(w).as_deref() == Some("General")), "the gear opened nothing");
    app.click(cx, "settings-cancel");
    assert!(closed(cx, &app), "Cancel left it open");

    // the palette's "Settings" (item 4.2b); Escape
    app.press(cx, "cmd-k");
    assert!(app.until(cx, |w| w.try_find("command-palette").is_some()));
    app.typed(cx, "settings");
    assert!(app.until(cx, |w| said(w, ("palette-row", 0usize)).as_deref() == Some("Settings")));
    app.press(cx, "enter");
    assert!(app.until(cx, |w| w.try_find("command-palette").is_none() && page(w).as_deref() == Some("General")));
    app.press(cx, "escape");
    assert!(closed(cx, &app), "Escape left it open");

    // "Project settings" in the project's menu: straight to that project's page
    app.click(cx, "project-menu");
    assert!(app.until(cx, |w| w.try_find("project-settings").is_some()), "the project's menu");
    app.click(cx, "project-settings");
    assert!(
        app.until(cx, |w| page(w).as_deref() == Some("shop")
            && pages(w) == ["General", "Claude", "Workflows", "shop *"]),
        "Project settings opened {:?}",
        look(cx, &app, |w| page(w))
    );
    app.press(cx, "escape");
    assert!(closed(cx, &app));

    // and "Project settings" in the workspace's own menu
    app.click(cx, "ws-menu");
    assert!(app.until(cx, |w| w.try_find("ws-project-settings").is_some()), "the workspace's menu");
    app.click(cx, "ws-project-settings");
    assert!(
        app.until(cx, |w| page(w).as_deref() == Some("shop")
            && pages(w) == ["General", "Claude", "Workflows", "shop *"]),
        "the workspace menu's Project settings opened {:?}",
        look(cx, &app, |w| page(w))
    );
}

/// Items 4.3d, 4.3e and 4.3g: the General and Claude pages, saved.
#[gpui_kit::test]
fn the_users_settings_are_saved_and_the_layout_with_the_window(cx: &mut TestAppContext) {
    let app = start_live(cx);
    let before = app.shared.user_settings();
    app.press(cx, "cmd-,");
    assert!(app.until(cx, |w| w.try_find("workspaces-dir").is_some()), "the General page never showed");
    // a dialog's fields are 32 px tall (DESIGN.md)
    look(cx, &app, |w| assert_eq!(w.find("workspaces-dir").bounds().size.height, px(32.)));
    // where the changes go: above the terminal until now
    look(cx, &app, |w| {
        assert_eq!(w.find("settings-layout-stack").selected(), Some(true));
        assert_eq!(w.find("settings-layout-side").selected(), Some(false));
    });
    app.click(cx, "settings-layout-side");
    assert!(app.until(cx, |w| w.find("settings-layout-side").selected() == Some(true)));
    let folder = app.home.path().join("other-trees");
    fill(cx, &app, "workspaces-dir", &format!("  {}  ", folder.display()));

    // the Claude page: the command, its arguments, its environment, and how it starts here
    app.click(cx, ("settings-nav", 1usize));
    assert!(app.until(cx, |w| w.try_find("claude-command").is_some()));
    assert!(
        app.until(cx, |w| said(w, "claude-preview").is_some_and(|command| command.starts_with(&before.claude.command))),
        "how Claude starts: {:?}",
        look(cx, &app, |w| said(w, "claude-preview"))
    );
    fill(cx, &app, "claude-args", "--model opus --append-system-prompt 'be brief");
    // the environment as it is (the fake Claude's own folder and log), and one more line
    let mut env: Vec<String> = before.claude.env.iter().map(|(key, value)| format!("{key}={value}")).collect();
    env.push("FOO = bar".into());
    fill_lines(cx, &app, "claude-env", &env);
    look(cx, &app, |w| assert_eq!(page(w).as_deref(), Some("Claude"), "Enter in the environment closed the dialog"));

    // a quote that isn't closed: said beside the buttons, nothing saved, the dialog stays
    app.click(cx, "save-settings");
    assert!(
        app.until(cx, |w| said(w, "settings-error").as_deref() == Some("Claude's arguments: A quote isn't closed")),
        "{:?}",
        look(cx, &app, |w| said(w, "settings-error"))
    );
    assert_eq!(app.shared.user_settings(), before);
    look(cx, &app, |w| assert_eq!(page(w).as_deref(), Some("Claude")));

    // put right, and saved: the dialog closes and says so
    fill(cx, &app, "claude-args", "--model opus --append-system-prompt 'be brief'");
    app.click(cx, "save-settings");
    assert!(closed(cx, &app), "Save left the dialog open: {:?}", look(cx, &app, |w| said(w, "settings-error")));
    assert!(app.until(cx, |w| said(w, "toast").as_deref() == Some("Settings saved")));
    let after = app.shared.user_settings();
    assert_eq!(after.workspaces_dir, folder.display().to_string(), "the folder, without the spaces around it");
    assert_eq!(after.claude.command, before.claude.command);
    assert_eq!(after.claude.args, ["--model", "opus", "--append-system-prompt", "be brief"]);
    let mut env = before.claude.env.clone();
    env.insert("FOO".into(), "bar".into());
    assert_eq!(after.claude.env, env, "the environment as it was, and FOO");
    // and it is in the settings file: a new core reads the same
    let again = enkidu_core::state::Shared::new(app.home.path().join("data"), std::time::Instant::now()).unwrap();
    assert_eq!(again.user_settings(), after);

    // the layout went to the window, and is kept in its own file
    let ui = app.shell.read_with(cx, |shell, cx| shell.ui().read(cx).clone());
    assert_eq!(ui.layout, enkidu_model::ui::Layout::Side);
    let kept = app.home.path().join("data").join("ui.json");
    for _ in 0..600 {
        cx.executor().advance_clock(std::time::Duration::from_millis(400));
        cx.run_until_parked();
        if std::fs::read_to_string(&kept).is_ok_and(|text| text.contains(r#""layout":"side""#)) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(std::fs::read_to_string(&kept).unwrap_or_default().contains(r#""layout":"side""#), "the layout isn't kept");

    // opened again, the fields say what was saved
    app.press(cx, "cmd-,");
    assert!(
        app.until(cx, |w| w.try_find("settings-layout-side").is_some_and(|side| side.selected() == Some(true))),
        "⌘, after a save opened {:?}",
        look(cx, &app, |w| page(w))
    );
    app.click(cx, ("settings-nav", 1usize));
    assert!(
        app.until(cx, |w| {
            said(w, "claude-preview")
                .is_some_and(|command| command.ends_with("--model opus --append-system-prompt 'be brief'"))
        }),
        "how Claude starts now: {:?}",
        look(cx, &app, |w| said(w, "claude-preview"))
    );
}

/// Items 8.3 and 8.4: the code's colors and the sizes of code and of the terminal's text are chosen on the General
/// page. Nothing is the window's until Save; then it is, it is kept in the window's own file, and Settings says it
/// when it opens again.
#[gpui_kit::test]
fn the_codes_colors_and_the_text_sizes_are_chosen_in_settings_and_kept_with_the_window(cx: &mut TestAppContext) {
    use enkidu_model::ui::CodeColors;
    let app = start(cx);
    let ui = |cx: &mut TestAppContext| app.shell.read_with(cx, |shell, cx| shell.ui().read(cx).clone());
    app.press(cx, "cmd-,");
    assert!(app.until(cx, |w| w.try_find("settings-colors-ide").is_some()), "the Code part never showed");
    look(cx, &app, |w| {
        assert_eq!(w.find("settings-colors-ide").selected(), Some(true), "an IDE's colors until you say otherwise");
        assert_eq!(w.find("settings-colors-quiet").selected(), Some(false));
        assert_eq!(said(w, "settings-code-size").as_deref(), Some("12 px"));
        assert_eq!(said(w, "settings-terminal-size").as_deref(), Some("12.5 px"));
    });
    app.click(cx, "settings-colors-quiet");
    app.click(cx, "settings-code-bigger");
    app.click(cx, "settings-code-bigger");
    app.click(cx, "settings-terminal-smaller");
    assert!(app.until(cx, |w| said(w, "settings-code-size").as_deref() == Some("14 px")));
    look(cx, &app, |w| {
        assert_eq!(said(w, "settings-terminal-size").as_deref(), Some("11.5 px"));
        assert_eq!(w.find("settings-colors-quiet").selected(), Some(true));
    });
    // nothing is the window's until Save
    let before = ui(cx);
    assert_eq!((before.code_colors, before.code_size, before.terminal_size), (CodeColors::Ide, 12, 12.5));
    app.click(cx, "save-settings");
    assert!(closed(cx, &app), "Save left the dialog open: {:?}", look(cx, &app, |w| said(w, "settings-error")));
    let after = ui(cx);
    assert_eq!((after.code_colors, after.code_size, after.terminal_size), (CodeColors::Quiet, 14, 11.5));
    assert_eq!(cx.update(|cx| enkidu_app::terminal_view::text_size(cx)), px(11.5), "the terminal follows");
    // kept in the window's own file
    let kept = app.home.path().join("data").join("ui.json");
    let written = r#""codeSize":14,"codeColors":"quiet","terminalSize":11.5"#;
    for _ in 0..600 {
        cx.executor().advance_clock(std::time::Duration::from_millis(400));
        cx.run_until_parked();
        if std::fs::read_to_string(&kept).is_ok_and(|text| text.contains(written)) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let text = std::fs::read_to_string(&kept).unwrap_or_default();
    assert!(text.contains(written), "not kept: {text}");
    // opened again, the page says what was saved; Cancel after a press leaves the window as it was
    app.press(cx, "cmd-,");
    assert!(app.until(cx, |w| w.try_find("settings-colors-quiet").is_some_and(|quiet| quiet.selected() == Some(true))));
    look(cx, &app, |w| {
        assert_eq!(said(w, "settings-code-size").as_deref(), Some("14 px"));
        assert_eq!(said(w, "settings-terminal-size").as_deref(), Some("11.5 px"));
    });
    app.click(cx, "settings-code-smaller");
    app.click(cx, "settings-colors-ide");
    app.click(cx, "settings-cancel");
    assert!(closed(cx, &app));
    let left = ui(cx);
    assert_eq!((left.code_colors, left.code_size), (CodeColors::Quiet, 14));
}

/// Item 8.6: the Workflows page of Settings has
/// the words each kind of task starts from, and what a new workflow starts with. Saved, the core keeps them; opened
/// again the page shows them; "Use Enkidu's words" puts the built-in ones back.
#[gpui_kit::test]
fn the_words_each_kind_of_task_starts_from_and_a_workflows_start_are_set_once_in_settings(cx: &mut TestAppContext) {
    use enkidu_core::work::flows::FlowDefaults;
    let app = start(cx);
    let mine = "Read the ticket, then ask me before you change anything.";
    let kept = |app: &App| (app.shared.db.work_flow_kinds().unwrap(), app.shared.db.work_flow_defaults().unwrap());
    let until_kept = |cx: &mut TestAppContext, app: &App, kinds: usize, minutes: u32| {
        for _ in 0..600 {
            cx.run_until_parked();
            let (words, defaults) = kept(app);
            if words.len() == kinds && defaults.minutes == minutes {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        kept(app)
    };
    app.press(cx, "cmd-,");
    assert!(app.until(cx, |w| page(w).as_deref() == Some("General")));
    app.click(cx, ("settings-nav", 2usize));
    assert!(app.until(cx, |w| page(w).as_deref() == Some("Workflows") && w.try_find("workflow-minutes").is_some()));
    // Enkidu's own to begin with: a field for every kind that has words, and no way back to offer
    look(cx, &app, |w| {
        for kind in ["start", "continue", "came_back", "review", "checks", "investigate"] {
            assert!(w.try_find(gpui_kit::SharedString::from(format!("workflow-words-{kind}"))).is_some(), "{kind}");
            assert!(w.try_find(gpui_kit::SharedString::from(format!("workflow-usual-{kind}"))).is_none(), "{kind}");
        }
        assert!(w.try_find("workflow-words-custom").is_none(), "Your own words has none of Enkidu's");
    });
    // your own words for "Start it", 45 minutes a task, at half past eleven
    fill(cx, &app, "workflow-words-start", mine);
    assert!(app.until(cx, |w| w.try_find("workflow-usual-start").is_some()), "the way back to Enkidu's words");
    fill(cx, &app, "workflow-minutes", "45");
    // a time that is none is said beside the buttons, and nothing is kept
    fill(cx, &app, "workflow-time", "25:99");
    app.click(cx, "save-settings");
    let wrong = "Workflows: the start time is a time of day, like 02:00";
    assert!(
        app.until(cx, |w| said(w, "settings-error").as_deref() == Some(wrong)),
        "{:?}",
        look(cx, &app, |w| said(w, "settings-error"))
    );
    assert_eq!(kept(&app), (Default::default(), FlowDefaults::default()));
    fill(cx, &app, "workflow-time", "23:30");
    app.click(cx, "save-settings");
    assert!(closed(cx, &app), "Save left the dialog open: {:?}", look(cx, &app, |w| said(w, "settings-error")));
    let (words, defaults) = until_kept(cx, &app, 1, 45);
    assert_eq!(words.get("start").map(String::as_str), Some(mine));
    assert_eq!(defaults, FlowDefaults { minutes: 45, time: "23:30".into() });
    // the window's index has them too: a new workflow's tasks start from them
    let index = app.until_work(cx, |work| work.flow_kinds.len() == 1 && work.flow_defaults.minutes == 45);
    assert_eq!(enkidu_model::work::flows::kind_text("start", &index.flow_kinds), mine);

    // opened again the page says them; one press puts Enkidu's words back, and Save forgets yours
    app.press(cx, "cmd-,");
    assert!(app.until(cx, |w| page(w).as_deref() == Some("General")));
    app.click(cx, ("settings-nav", 2usize));
    assert!(app.until(cx, |w| w.try_find("workflow-usual-start").is_some()), "your words are on the page");
    bring(cx, &app, "workflow-usual-start");
    app.click(cx, "workflow-usual-start");
    assert!(app.until(cx, |w| w.try_find("workflow-usual-start").is_none()));
    app.click(cx, "save-settings");
    assert!(closed(cx, &app));
    let (words, defaults) = until_kept(cx, &app, 0, 45);
    assert!(words.is_empty(), "{words:?}");
    assert_eq!(defaults, FlowDefaults { minutes: 45, time: "23:30".into() }, "what a workflow starts with stays");
}

/// Items 4.3f and 4.3g: a project's page, saved; what it doesn't edit stays.
#[gpui_kit::test]
fn a_projects_settings_are_saved_and_its_work_settings_stay_as_they_were(cx: &mut TestAppContext) {
    let app = start(cx);
    let project = app.project_id.clone();
    let before = app.shared.project_settings(&project).unwrap();
    app.click(cx, "project-menu");
    assert!(app.until(cx, |w| w.try_find("project-settings").is_some()));
    app.click(cx, "project-settings");
    assert!(app.until(cx, |w| w.try_find("default-base").is_some()), "the project's page never showed");

    fill(cx, &app, "default-base", " development ");
    // one path a line
    fill_lines(cx, &app, "copy-files", &[".env".into(), ".claude/settings.local.json".into()]);
    // a script of two lines: Enter is a new line there, it doesn't save
    fill_lines(cx, &app, "setup-script", &["make setup".into(), "make seed".into()]);
    look(cx, &app, |w| assert_eq!(page(w).as_deref(), Some("shop"), "Enter in a field closed the dialog"));
    fill(cx, &app, "live-on", "docker compose restart web");
    fill(cx, &app, "live-off", "docker compose restart web");
    fill(cx, &app, "live-message", "You are live.");
    fill(cx, &app, "project-claude-args", "--permission-mode acceptEdits");
    fill(cx, &app, "project-claude-env", "FAKE_PROJECT_ENV=project env");
    // on the rail: its own icon is shown unless you say so, and another picture can be named or chosen
    assert_eq!(before.rail, enkidu_core::settings::RailSettings { icon: true, picture: String::new() });
    bring(cx, &app, "rail-icon");
    look(cx, &app, |w| assert_eq!(w.find("rail-icon").checked(), Some(true)));
    app.click(cx, "rail-icon");
    look(cx, &app, |w| assert_eq!(w.find("rail-icon").checked(), Some(false)));
    bring(cx, &app, "rail-picture-choose");
    app.click(cx, "rail-picture-choose");
    assert!(cx.did_prompt_for_paths(), "Choose… opens the file picker");
    cx.simulate_path_prompt_response(|options| {
        assert!(options.files && !options.directories && !options.multiple, "one file");
        Some(vec![std::path::PathBuf::from("/pictures/mark.png")])
    });
    assert!(app.until(cx, |w| w.find("rail-picture").value() == Some("/pictures/mark.png")), "the file picked");
    fill(cx, &app, "rail-picture", " art/mark.svg ");

    app.click(cx, "save-settings");
    assert!(closed(cx, &app), "Save left the dialog open: {:?}", look(cx, &app, |w| said(w, "settings-error")));
    let after = app.shared.project_settings(&project).unwrap();
    assert_eq!(after.rail, enkidu_core::settings::RailSettings { icon: false, picture: "art/mark.svg".into() });
    assert_eq!(after.default_base, "development");
    assert_eq!(after.copy_files, [".env", ".claude/settings.local.json"]);
    assert_eq!(after.setup_script, "make setup\nmake seed");
    assert_eq!(
        (after.live.on.as_str(), after.live.off.as_str(), after.live.message.as_str()),
        ("docker compose restart web", "docker compose restart web", "You are live.")
    );
    assert_eq!(after.claude.args, ["--permission-mode", "acceptEdits"]);
    assert_eq!(after.claude.env.get("FAKE_PROJECT_ENV").map(String::as_str), Some("project env"));
    // what wasn't touched on the page (its Work and Workflows parts) is as it was
    assert_eq!(after.work, before.work);
    assert_eq!(after.work.jira_keys, ["SH"]);

    // opened again, the fields say what was saved; a field that can't be read is named with its project
    app.click(cx, "project-menu");
    assert!(app.until(cx, |w| w.try_find("project-settings").is_some()));
    app.click(cx, "project-settings");
    assert!(app.until(cx, |w| w.try_find("project-claude-env").is_some()));
    fill(cx, &app, "project-claude-env", "not a pair");
    app.click(cx, "save-settings");
    assert!(
        app.until(cx, |w| {
            said(w, "settings-error").as_deref() == Some("Claude's environment in shop: Line 1: use KEY=value")
        }),
        "{:?}",
        look(cx, &app, |w| said(w, "settings-error"))
    );
    assert_eq!(app.shared.project_settings(&project).unwrap(), after, "nothing was saved");
}

/// How the workspace is laid out, from where its two islands are: "stack" (the changes above the terminal) or "side"
/// (beside it, the terminal on the left).
fn mode(window: &Window) -> &'static str {
    let changes = window.try_find("island-changes").map(|island| island.bounds());
    let terminal = window.try_find("island-claude").map(|island| island.bounds());
    match (changes, terminal) {
        (Some(changes), Some(terminal)) if changes.bottom() <= terminal.top() => "stack",
        (Some(changes), Some(terminal)) if terminal.right() <= changes.left() => "side",
        (Some(_), Some(_)) => "one over the other",
        _ => "one alone",
    }
}

/// Settings from the gear, the layout and Claude's arguments saved, the file on disk, the fields as saved when opened
/// again, and the layout back.
#[gpui_kit::test]
fn saves_settings_to_disk_and_applies_the_layout_choice(cx: &mut TestAppContext) {
    let app = start_live(cx);
    app.until_claude_started(cx, 1);
    // a change in the workspace, so its changes share the screen with the terminal
    let folder = app.frame.read_with(cx, |f, _| f.current_workspace().map(|w| w.path.clone())).expect("a workspace");
    std::fs::write(std::path::Path::new(&folder).join("notes.md"), "one\n").unwrap();
    assert!(app.until(cx, |w| mode(w) == "stack"), "the changes above the terminal: {}", look(cx, &app, |w| mode(w)));

    app.click(cx, "settings");
    assert!(app.until(cx, |w| w.try_find("settings-layout-side").is_some()), "the gear opened nothing");
    app.click(cx, "settings-layout-side");
    app.click(cx, ("settings-nav", 1usize));
    assert!(app.until(cx, |w| w.try_find("claude-args").is_some()));
    fill(cx, &app, "claude-args", "--model opus --verbose");
    app.click(cx, "save-settings");
    assert!(closed(cx, &app), "Save left the dialog open: {:?}", look(cx, &app, |w| said(w, "settings-error")));
    assert!(app.until(cx, |w| mode(w) == "side"), "the layout chosen: {}", look(cx, &app, |w| mode(w)));
    let toml = std::fs::read_to_string(app.home.path().join("data").join("settings.toml")).unwrap();
    // dark only: there is no theme key in the file
    assert!(!toml.contains("theme"), "{toml}");
    let flat = toml.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        flat.contains(r#"args = ["--model", "opus", "--verbose"]"#)
            || flat.contains(r#"args = [ "--model", "opus", "--verbose", ]"#),
        "the arguments in the file: {toml}"
    );

    // opened again: the field says what was saved
    app.click(cx, "settings");
    assert!(app.until(cx, |w| w.try_find("settings-layout-side").is_some_and(|side| side.selected() == Some(true))));
    app.click(cx, ("settings-nav", 1usize));
    assert!(app.until(cx, |w| w.try_find("claude-args").is_some()));
    let args = look(cx, &app, |w| w.find("claude-args").value().map(String::from));
    assert_eq!(args.as_deref(), Some("--model opus --verbose"));
    // and the layout back
    app.click(cx, ("settings-nav", 0usize));
    assert!(app.until(cx, |w| w.try_find("settings-layout-stack").is_some()));
    app.click(cx, "settings-layout-stack");
    app.click(cx, "save-settings");
    assert!(closed(cx, &app));
    assert!(app.until(cx, |w| mode(w) == "stack"), "the layout back: {}", look(cx, &app, |w| mode(w)));
}

/// The Claude folders listed on the Claude page: each row's settings file, with "Added" when it says so, and what
/// its button says.
fn hook_rows(window: &Window) -> Vec<(String, bool, String)> {
    (0..8usize)
        .filter_map(|ix| {
            Some((window.try_find(("claude-hooks-row", ix))?, window.try_find(("claude-hooks-toggle", ix))?))
        })
        .map(|(row, button)| {
            let path = row.label().unwrap_or_default().to_string();
            (path, row.description() == Some("Added"), button.label().unwrap_or_default().to_string())
        })
        .collect()
}

/// Item 4.4: "Claude outside Enkidu" on throwaway Claude folders: nothing is written until a button is pressed;
/// Add hooks puts the five hooks in that folder's settings and keeps the file as it was beside it; Remove takes them
/// out and leaves the rest; a folder without a settings file gets one.
#[gpui_kit::test]
fn hooks_go_into_a_claude_folder_and_out_again_only_on_a_press(cx: &mut TestAppContext) {
    use enkidu_core::work::claudehooks::is_added;
    let (plain, work) = claude_folders();
    let (file, other) = (plain.join("settings.json"), work.join("settings.json"));
    let kept_copy = plain.join("settings.json.before-enkidu");
    let before = "{\n  \"model\": \"opus\",\n  \"permissions\": { \"allow\": [\"Bash(ls:*)\"] }\n}\n";
    std::fs::write(&file, before).unwrap();
    let read = |path: &std::path::Path| -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    };
    let app = start_live(cx);
    app.press(cx, "cmd-,");
    assert!(app.until(cx, |w| w.try_find(("settings-nav", 1usize)).is_some()));
    app.click(cx, ("settings-nav", 1usize));
    assert!(app.until(cx, |w| hook_rows(w).len() == 2), "the Claude folders: {:?}", look(cx, &app, |w| hook_rows(w)));
    let shown = look(cx, &app, |w| hook_rows(w));
    assert_eq!(
        shown,
        [
            (file.to_string_lossy().to_string(), false, "Add hooks".to_string()),
            (other.to_string_lossy().to_string(), false, "Add hooks".to_string())
        ]
    );
    // looking wrote nothing
    assert_eq!(std::fs::read_to_string(&file).unwrap(), before);
    assert!(!kept_copy.exists() && !other.exists());

    // exactly what would be added: five hooks, each running this app's own enkidu
    bring(cx, &app, "claude-hooks-show");
    app.click(cx, "claude-hooks-show");
    assert!(app.until(cx, |w| w.try_find("claude-hooks-adds").is_some()), "Show what it adds showed nothing");
    look(cx, &app, |w| {
        let adds = said(w, "claude-hooks-adds").unwrap();
        assert_eq!(adds.matches(" event --app enkidu").count(), 5, "{adds}");
        for event in ["SessionStart", "UserPromptSubmit", "PostToolUse", "Stop", "SessionEnd"] {
            assert!(adds.contains(&format!("\"{event}\"")), "{event} in {adds}");
        }
        assert_eq!(said(w, "claude-hooks-show").as_deref(), Some("Hide what it adds"));
    });
    app.click(cx, "claude-hooks-show");
    assert!(app.until(cx, |w| w.try_find("claude-hooks-adds").is_none()));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), before, "showing wrote nothing");

    // Add hooks: the row says Added, the file has them with everything it had, and the file as it was is kept
    bring(cx, &app, ("claude-hooks-toggle", 0usize));
    app.click(cx, ("claude-hooks-toggle", 0usize));
    assert!(
        app.until(cx, |w| hook_rows(w).first().is_some_and(|row| row.1 && row.2 == "Remove")),
        "after Add hooks: {:?}, {:?}",
        look(cx, &app, |w| hook_rows(w)),
        look(cx, &app, |w| said(w, "claude-hooks-error"))
    );
    let with_hooks = read(&file);
    assert!(is_added(&with_hooks), "{with_hooks}");
    assert_eq!(with_hooks["model"], "opus");
    assert_eq!(with_hooks["permissions"], read_text(before)["permissions"]);
    assert_eq!(std::fs::read_to_string(&kept_copy).unwrap(), before, "the file as it was, kept beside it");
    assert!(!other.exists(), "the other folder wasn't touched");

    // Remove: they are gone, the rest is as it was
    app.click(cx, ("claude-hooks-toggle", 0usize));
    assert!(app.until(cx, |w| hook_rows(w).first().is_some_and(|row| !row.1 && row.2 == "Add hooks")));
    let without = read(&file);
    assert!(!is_added(&without));
    assert_eq!(without, read_text(before), "everything else is as it was");

    // a folder with no settings file gets one
    app.click(cx, ("claude-hooks-toggle", 1usize));
    assert!(app.until(cx, |w| hook_rows(w).get(1).is_some_and(|row| row.1)), "{:?}", look(cx, &app, |w| hook_rows(w)));
    assert!(is_added(&read(&other)));
    app.click(cx, ("claude-hooks-toggle", 1usize));
    assert!(app.until(cx, |w| hook_rows(w).get(1).is_some_and(|row| !row.1)));
    // Cancel has nothing to undo: the hooks were added and taken out as the buttons were pressed
    app.click(cx, "settings-cancel");
    assert!(closed(cx, &app));
    assert!(!is_added(&read(&file)) && !is_added(&read(&other)));
}

fn read_text(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap()
}

/// Item 4.4: in Settings your own Claude folders are listed, each with its button, and what the hooks add can be
/// read. (A window with sample data: the real folders are never read by a test, and a press changes nothing there.)
#[gpui_kit::test]
fn the_claude_folders_are_listed_with_their_buttons_and_what_the_hooks_add(cx: &mut TestAppContext) {
    let (window, _) = cx.update(|cx| {
        enkidu_app::init(cx);
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
    let window: gpui_kit::AnyWindowHandle = window;
    let look = |cx: &mut TestAppContext| {
        cx.run_until_parked();
        cx.update_window(window, |_, w, cx| {
            w.render_frame(cx);
            (hook_rows(w), said(w, "claude-hooks-error"))
        })
        .unwrap()
    };
    cx.update_window(window, |_, w, cx| {
        w.render_frame(cx);
        w.press("cmd-,", cx);
        w.render_frame(cx);
        w.click(("settings-nav", 1usize), cx);
    })
    .unwrap();
    let (rows, _) = look(cx);
    assert_eq!(
        rows,
        [
            ("~/.claude/settings.json".to_string(), true, "Remove".to_string()),
            ("~/.claude-work/settings.json".to_string(), false, "Add hooks".to_string())
        ]
    );
    // in the sample a press changes nothing, and what would be added can be read
    cx.update_window(window, |_, w, cx| {
        let down = ScrollDelta::Pixels(point(px(0.), px(-2000.)));
        w.scroll("settings-body", down, cx);
        w.click(("claude-hooks-toggle", 0usize), cx);
        w.click("claude-hooks-show", cx);
    })
    .unwrap();
    let (after, error) = look(cx);
    assert_eq!(after, rows, "nothing changed");
    assert_eq!(error, None);
    cx.update_window(window, |_, w, cx| {
        w.render_frame(cx);
        assert!(w.find("claude-hooks-adds").label().is_some_and(|adds| adds.contains("event --app enkidu")));
    })
    .unwrap();
}

/// Opens the project's page of Settings from the project's own menu.
fn open_project_page(cx: &mut TestAppContext, app: &App) {
    app.click(cx, "project-menu");
    assert!(app.until(cx, |w| w.try_find("project-settings").is_some()));
    app.click(cx, "project-settings");
    assert!(app.until(cx, |w| w.try_find("default-base").is_some()), "the project's page never showed");
}

/// Whether a switch of the page is on.
fn switched(cx: &mut TestAppContext, app: &App, id: &'static str) -> Option<bool> {
    look(cx, app, |w| w.try_find(id).and_then(|e| e.checked()))
}

/// Presses a switch of the page, once it is in view.
fn flip(cx: &mut TestAppContext, app: &App, id: &'static str) {
    bring(cx, app, id);
    app.click(cx, id);
}

/// Item 5.16a: what is read from GitHub and Jira for a project is on its page, and saved with it.
#[gpui_kit::test]
fn a_projects_work_settings_are_on_its_page_and_saved_with_it(cx: &mut TestAppContext) {
    let app = start(cx);
    let project = app.project_id.clone();
    let before = app.shared.project_settings(&project).unwrap();
    assert_eq!((before.work.github, before.work.every_minutes), (true, 7));
    open_project_page(cx, &app);
    // the page says what the core has
    bring(cx, &app, "work-jira-keys");
    let shown = look(cx, &app, |w| {
        let value = |id: &'static str| w.find(id).value().map(String::from).unwrap_or_default();
        (value("work-jira-keys"), value("work-every"), value("work-github-account"), value("work-back"))
    });
    assert_eq!(shown, ("SH".to_string(), "7".to_string(), String::new(), String::new()));
    assert_eq!(switched(cx, &app, "work-github"), Some(true));
    // the project's Jira projects, set like you would
    fill(cx, &app, "work-jira-keys", "sh, OPS");
    flip(cx, &app, "work-github");
    assert_eq!(switched(cx, &app, "work-github"), Some(false));
    fill(cx, &app, "work-github-account", " me-work ");
    fill(cx, &app, "work-back", "QA Failed, Reopened");
    fill_lines(cx, &app, "work-ignored", &["codecov/patch".into(), "docs".into()]);
    fill(cx, &app, "work-every", "999");
    // nothing is written until Save
    assert_eq!(app.shared.project_settings(&project).unwrap(), before);
    app.click(cx, "save-settings");
    assert!(closed(cx, &app), "Save left the dialog open: {:?}", look(cx, &app, |w| said(w, "settings-error")));
    let after = app.shared.project_settings(&project).unwrap();
    assert_eq!(after.work.jira_keys, ["SH", "OPS"], "capitals, whatever was typed");
    assert!(!after.work.github);
    assert_eq!(after.work.github_account, "me-work");
    assert_eq!(after.work.back_to_you, ["QA Failed", "Reopened"]);
    assert_eq!(after.work.ignored_checks, ["codecov/patch", "docs"]);
    assert_eq!(after.work.every_minutes, 240, "never over four hours");
    // the rest of the page is as it was
    assert_eq!((after.copy_files.clone(), after.default_base.clone()), (before.copy_files, before.default_base));
    assert_eq!((after.work.send_failed_checks, after.work.morning_summary), (false, false));
    // opened again, the page says what was saved
    open_project_page(cx, &app);
    bring(cx, &app, "work-jira-keys");
    let shown = look(cx, &app, |w| {
        let value = |id: &'static str| w.find(id).value().map(String::from).unwrap_or_default();
        (value("work-jira-keys"), value("work-every"), value("work-github-account"))
    });
    assert_eq!(shown, ("SH, OPS".to_string(), "240".to_string(), "me-work".to_string()));
    assert_eq!(switched(cx, &app, "work-github"), Some(false));
}

/// Item 5.16: a project's workflows are two switches, both off until turned on, and the morning summary's time; they
/// are saved with the page. The latest runs stand under them, each saying what it did or why it didn't.
#[gpui_kit::test]
fn a_projects_workflows_are_turned_on_and_saved_and_their_latest_runs_are_listed(cx: &mut TestAppContext) {
    let app = start(cx);
    let project = app.project_id.clone();
    // two runs the core kept: failing checks that were sent, and a summary that didn't run
    let now = enkidu_core::db::now_ms();
    let db = &app.shared.db;
    let sent = db.work_workflow_add(&project, "failed_checks", "checks:shop#12:abc", "pending", now - 3 * 3_600_000);
    let sent = sent.unwrap().expect("a run");
    db.work_workflow_finish(sent, "done", "Sent to the Claude in stock-fix.", now - 3 * 3_600_000).unwrap();
    let skipped =
        db.work_workflow_add(&project, "morning", "2026-11-12", "running", now - 120_000).unwrap().expect("a run");
    db.work_workflow_finish(skipped, "skipped", "Didn't run: Jira can't be read: twg isn't signed in.", now - 120_000)
        .unwrap();

    open_project_page(cx, &app);
    bring(cx, &app, "work-morning-at");
    assert_eq!(switched(cx, &app, "work-send-failed"), Some(false), "off until you turn it on");
    assert_eq!(switched(cx, &app, "work-morning"), Some(false));
    assert_eq!(look(cx, &app, |w| w.find("work-morning-at").value().map(String::from)).as_deref(), Some("09:00"));
    assert_eq!(look(cx, &app, |w| said(w, "work-morning-run")).as_deref(), Some("Run now"));
    // the latest runs, the newest first: its lamp's state, the workflow's name, what happened, its age
    bring(cx, &app, ("workflow-run", 1usize));
    let runs = look(cx, &app, |w| {
        (0..4usize)
            .filter_map(|n| w.try_find(("workflow-run", n)))
            .map(|row| {
                (
                    row.description().map(String::from).unwrap_or_default(),
                    row.label().map(String::from).unwrap_or_default(),
                )
            })
            .collect::<Vec<_>>()
    });
    assert_eq!(
        runs,
        [
            (
                "morning skipped".to_string(),
                "Morning summary: Didn't run: Jira can't be read: twg isn't signed in., 2 min ago".to_string()
            ),
            ("failed_checks done".to_string(), "Failing checks: Sent to the Claude in stock-fix., 3 h ago".to_string()),
        ]
    );
    // both turned on, the summary at half past eight
    flip(cx, &app, "work-send-failed");
    flip(cx, &app, "work-morning");
    assert_eq!((switched(cx, &app, "work-send-failed"), switched(cx, &app, "work-morning")), (Some(true), Some(true)));
    fill(cx, &app, "work-morning-at", " 08:30 ");
    let before = app.shared.project_settings(&project).unwrap();
    assert_eq!((before.work.send_failed_checks, before.work.morning_summary), (false, false), "nothing until Save");
    app.click(cx, "save-settings");
    assert!(closed(cx, &app), "Save left the dialog open: {:?}", look(cx, &app, |w| said(w, "settings-error")));
    let after = app.shared.project_settings(&project).unwrap();
    assert!(after.work.send_failed_checks && after.work.morning_summary);
    assert_eq!(after.work.morning_at, "08:30");
    assert_eq!(after.work.jira_keys, ["SH"], "what wasn't touched stays");
    // a switch turned back and forth is no edit: nothing is saved again
    open_project_page(cx, &app);
    flip(cx, &app, "work-morning");
    flip(cx, &app, "work-morning");
    assert_eq!(switched(cx, &app, "work-morning"), Some(true));
}

/// Item 7.5a: how Claude starts and what the hooks add can be selected with the mouse and copied.
#[gpui_kit::test]
fn how_claude_starts_and_what_the_hooks_add_can_be_selected_and_copied(cx: &mut TestAppContext) {
    use gpui_kit::{InputEvent as _, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, point};
    let _folders = claude_folders();
    let app = start_live(cx);
    let command = app.shared.user_settings().claude.command;
    app.press(cx, "cmd-,");
    assert!(app.until(cx, |w| w.try_find(("settings-nav", 1usize)).is_some()));
    app.click(cx, ("settings-nav", 1usize));
    assert!(app.until(cx, |w| said(w, "claude-preview").is_some_and(|said| said.starts_with(&command))));
    // a drag over a place of the window, in two steps as a hand makes it
    let drag = |cx: &mut TestAppContext,
                from: gpui_kit::Point<gpui_kit::Pixels>,
                to: gpui_kit::Point<gpui_kit::Pixels>| {
        cx.run_until_parked();
        cx.update_window(app.window, |_, w, cx| {
            w.render_frame(cx);
            let (modifiers, left) = (Modifiers::default(), MouseButton::Left);
            w.dispatch_event(
                MouseMoveEvent { position: from, pressed_button: None, modifiers }.to_platform_input(),
                cx,
            );
            w.render_frame(cx);
            let down = MouseDownEvent { button: left, position: from, modifiers, click_count: 1, first_mouse: false };
            w.dispatch_event(down.to_platform_input(), cx);
            w.render_frame(cx);
            for position in [point((from.x + to.x) / 2., (from.y + to.y) / 2.), to] {
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
    };
    let selected = |cx: &mut TestAppContext| {
        cx.run_until_parked();
        cx.update_window(app.window, |_, w, cx| {
            w.render_frame(cx);
            gpui_kit::base::TextSelection::selected_text(w, cx)
        })
        .unwrap()
    };

    // how Claude starts: its first line, from its first letter on
    let whole = look(cx, &app, |w| said(w, "claude-preview")).unwrap();
    let block = look(cx, &app, |w| w.find("claude-preview").bounds());
    let y = block.top() + px(10. + 9.);
    drag(cx, point(block.left() + px(12.), y), point(block.right() - px(12.), y));
    let taken = selected(cx);
    assert!(taken.len() > 20 && whole.starts_with(taken.trim_end()), "selected {taken:?} of {whole:?}");
    app.press(cx, "cmd-c");
    assert_eq!(cx.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some(taken.trim()));

    // what the hooks add
    bring(cx, &app, "claude-hooks-show");
    app.click(cx, "claude-hooks-show");
    assert!(app.until(cx, |w| w.try_find("claude-hooks-adds").is_some()), "Show what it adds showed nothing");
    let adds = look(cx, &app, |w| said(w, "claude-hooks-adds")).unwrap();
    bring(cx, &app, "claude-hooks-adds");
    let block = look(cx, &app, |w| w.find("claude-hooks-adds").bounds());
    let y = block.top() + px(10. + 8.);
    drag(cx, point(block.left() + px(12.), y), point(block.right() - px(12.), y));
    let taken = selected(cx);
    assert!(!taken.trim().is_empty() && adds.starts_with(taken.trim_end()), "selected {taken:?} of {adds:?}");
}
