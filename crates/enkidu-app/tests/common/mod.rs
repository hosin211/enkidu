//! What the whole-app tests share: a real core on a temporary folder with its workspaces folder there too, a project
//! made from a throwaway repo with an origin, the stores hearing the core's news, and the window on them. No window
//! appears on screen, and nothing outside the temporary folder is touched.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use enkidu_app::bridge::Bridge;
use enkidu_app::shell::Shell;
use enkidu_core::db::Remark;
use enkidu_core::events::Events;
use enkidu_core::state::Shared;
use enkidu_model::frame::Frame;
use futures::channel::mpsc::unbounded;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, ElementId, Entity, InputEvent as _, Modifiers, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, TestAppContext, Window, WindowBounds, WindowOptions,
    px, size,
};

pub mod pointing;

/// The size of these tests' window. It is wide enough that Claude's terminal has about 130 columns beside the
/// rail and the files' column, so what a Claude prints as it starts stays on its screen.
pub const WINDOW: (f32, f32) = (1616., 860.);

pub struct App {
    /// Where the core's news goes (what Claude's hooks would make it send goes here too).
    pub sink: Events,
    /// With the fake Claude ([`start_live`]): the file where each one says how it was started.
    pub claude_log: PathBuf,
    pub home: tempfile::TempDir,
    /// The project's main folder.
    pub project: PathBuf,
    pub project_id: String,
    pub workspaces: PathBuf,
    pub shared: Arc<Shared>,
    pub frame: Entity<Frame>,
    pub window: AnyWindowHandle,
    pub shell: Entity<Shell>,
}

/// Starts it all on a project with an origin (`enkidu_testkit::repo_with_origin`). `prepare` runs on the core before the
/// stores read it (to make workspaces, set settings).
pub fn start(cx: &mut TestAppContext, prepare: impl FnOnce(&Arc<Shared>, &str, &Path)) -> App {
    start_on(cx, |home| enkidu_testkit::repo_with_origin(home, "shop"), prepare)
}

/// The same on the project `make` makes in the temporary folder.
pub fn start_on(
    cx: &mut TestAppContext,
    make: impl FnOnce(&Path) -> PathBuf,
    prepare: impl FnOnce(&Arc<Shared>, &str, &Path),
) -> App {
    build(cx, make, prepare, false)
}

/// The window as the app opens it: each workspace's terminals are the core's, and its Claude is the fake one
/// (`enkidu-testkit/fakes/fake-claude.mjs`, never the real `claude`), started through the real hook server with the
/// app's own `enkidu`. The fake keeps its conversations in the temporary folder.
pub fn start_live(
    cx: &mut TestAppContext,
    make: impl FnOnce(&Path) -> PathBuf,
    prepare: impl FnOnce(&Arc<Shared>, &str, &Path),
) -> App {
    build(cx, make, prepare, true)
}

/// The app as it opens for the first time on a Mac: a data folder with nothing in it and no project, the live window,
/// the fake Claude. `prepare` runs before the window opens (give the core its fake `gh` and `twg` there).
pub fn start_fresh(cx: &mut TestAppContext, prepare: impl FnOnce(&Arc<Shared>, &str, &Path)) -> App {
    let home = tempfile::tempdir().unwrap();
    launch(cx, home, PathBuf::new(), None, prepare, true)
}

/// The app's own `enkidu`, as `cargo test --workspace` builds it; named once for every test of this binary.
fn own_rh() {
    static NAMED: std::sync::Once = std::sync::Once::new();
    NAMED.call_once(|| {
        let cli = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/enkidu");
        let cli = cli.canonicalize().expect("target/debug/enkidu: build it first with `cargo build -p enkidu-cli`");
        // SAFETY: set once, to the same value, before any terminal of this test binary starts.
        unsafe { std::env::set_var("ENKIDU_CLI", cli) };
        // a shell tab runs your login shell: here a plain one, so no test depends on your own shell's setup
        // SAFETY: as above.
        unsafe { std::env::set_var("SHELL", "/bin/sh") };
    });
}

/// Points the core at the fake Claude: a `claude` of its own in the temporary folder, its conversations and its log
/// there too.
fn fake_claude(shared: &Arc<Shared>, home: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let fake = Path::new(env!("CARGO_MANIFEST_DIR")).join("../enkidu-testkit/fakes/fake-claude.mjs");
    let fake = fake.canonicalize().expect("the fake Claude");
    let bin = home.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let claude = bin.join("claude");
    std::fs::write(&claude, format!("#!/bin/sh\nexec node '{}' \"$@\"\n", fake.display())).unwrap();
    std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
    let log = home.join("fake-claude.log");
    let mut user = shared.user.lock().unwrap();
    user.claude.command = claude.to_string_lossy().into();
    user.claude.env.insert("CLAUDE_CONFIG_DIR".into(), home.join("claude-config").to_string_lossy().into());
    user.claude.env.insert("FAKE_CLAUDE_LOG".into(), log.to_string_lossy().into());
    log
}

fn build(
    cx: &mut TestAppContext,
    make: impl FnOnce(&Path) -> PathBuf,
    prepare: impl FnOnce(&Arc<Shared>, &str, &Path),
    live: bool,
) -> App {
    let home = tempfile::tempdir().unwrap();
    let project = make(home.path());
    launch(cx, home, project, None, prepare, live)
}

/// The app started again on what `app` left, as after quitting it and opening it again: the same folders and database,
/// a new core, new stores and a new window. The first core's terminals end before that, as when the app quits.
pub fn restart(cx: &mut TestAppContext, app: App) -> App {
    restart_as(cx, app, true)
}

/// The same; with `live` false the new window has a terminal that runs nothing (no Claude at all).
pub fn restart_as(cx: &mut TestAppContext, app: App, live: bool) -> App {
    app.shared.pty.close_all();
    let _ = cx.update_window(app.window, |_, w, _| w.remove_window());
    cx.run_until_parked();
    let App { home, project, project_id, .. } = app;
    launch(cx, home, project, Some(project_id), |_, _, _| {}, live)
}

/// A core on `home`'s data folder, the stores on it and the window. `known` is the project's id when the database
/// already has it (a restart).
fn launch(
    cx: &mut TestAppContext,
    home: tempfile::TempDir,
    project: PathBuf,
    known: Option<String>,
    prepare: impl FnOnce(&Arc<Shared>, &str, &Path),
    live: bool,
) -> App {
    let workspaces = home.path().join("workspaces");
    let data = home.path().join("data");
    let shared = Arc::new(Shared::new(data.clone(), Instant::now()).unwrap());
    {
        let mut user = shared.user.lock().unwrap();
        user.workspaces_dir = workspaces.to_string_lossy().into();
        // never the real claude in a test: a live window gets the fake one below, any other window none at all
        user.claude.command = "/nowhere/claude-is-never-run-in-a-test".into();
    }
    // no folder given: a Mac that has no project yet (`start_fresh`)
    let fresh = project.as_os_str().is_empty();
    let project_id = known.unwrap_or_else(|| match fresh {
        true => String::new(),
        false => {
            enkidu_core::commands::add_project(&shared, project.to_string_lossy().into())
                .expect("the project is added")
                .id
        }
    });
    let (news, events) = unbounded();
    // as in the app: a terminal's output goes straight to its parser, the rest to the stores
    let feeds = Arc::new(enkidu_app::terminals::Feeds::default());
    let sink = enkidu_app::terminals::sink(news, feeds.clone());
    let claude_log = if live {
        own_rh();
        let server = enkidu_core::hooks::start(sink.clone(), shared.clone()).expect("the hook server");
        *shared.hooks.lock().unwrap() = Some(server);
        fake_claude(&shared, home.path())
    } else {
        PathBuf::new()
    };
    // the core's watch on every workspace's folder, its following of a folder that moves to another branch, and a Live
    // workspace's changes going on into the main folder, as `enkidu_core::start` sets them up
    shared.changes.start(sink.clone()).expect("the watch on the folders");
    let follower = Arc::downgrade(&shared);
    shared.changes.after_refresh(move |app, id| {
        if let Some(shared) = follower.upgrade() {
            enkidu_core::work::follow_branch(app, &shared, id);
            enkidu_core::live::follow(app, &shared, id);
        }
    });
    prepare(&shared, &project_id, home.path());
    let frame = cx.update(|cx| {
        enkidu_app::init(cx);
        // the whole-app tests run as with "Reduce motion" on: what is on screen is where it stays, so a test can
        // measure it without moving the clock. The tests of the motion itself turn it back off (tests/motion.rs)
        cx.set_reduce_motion(true);
        Bridge::install(cx, shared.clone(), sink.clone());
        // what outlives a launch is kept in the data folder, as in the app
        enkidu_app::stores::install_for_tests(events, feeds, enkidu_app::stores::Places::in_dir(&data), cx)
    });
    let on = frame.clone();
    let (window, shell) = cx
        .update(|cx| {
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: Default::default(),
                        size: size(px(WINDOW.0), px(WINDOW.1)),
                    })),
                    ..Default::default()
                },
                cx,
                |window, cx| cx.new(|cx| if live { Shell::live(on, window, cx) } else { Shell::kept(on, window, cx) }),
            )
        })
        .expect("a test window");
    // the window is in front, as the app's is (inputs hear that they lost the keyboard only then)
    cx.update_window(window, |_, w, _| w.activate_window()).unwrap();
    cx.executor().allow_parking();
    let app = App { sink, claude_log, home, project, project_id, workspaces, shared, frame, window, shell };
    assert!(until(cx, app.window, |_| true));
    assert!(app.until_frame(cx, |f| f.loaded), "the first read came back");
    app
}

/// As the app last left it: only the workspaces named had a Claude, so only theirs come back at launch; another one
/// gets its Claude when it is opened. Call it from a `prepare`, after the workspaces are made.
pub fn only_these_had_claude(shared: &Arc<Shared>, home: &Path, names: &[&str]) {
    let all = shared.db.workspaces().unwrap();
    let ids: Vec<String> = all.iter().filter(|w| names.contains(&w.name.as_str())).map(|w| w.id.clone()).collect();
    std::fs::write(home.join("data").join("sessions.json"), serde_json::to_string(&ids).unwrap()).unwrap();
}

/// Runs the app until `seen` holds for the window's last frame (the core answers on threads of its own).
pub fn until(cx: &mut TestAppContext, window: AnyWindowHandle, seen: impl Fn(&Window) -> bool) -> bool {
    for _ in 0..800 {
        cx.run_until_parked();
        let ok = cx
            .update_window(window, |_, w, cx| {
                w.render_frame(cx);
                seen(w)
            })
            .unwrap();
        if ok {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

/// Whether a terminal's text holds `words`, whatever rows the terminal broke them over: a line longer than the
/// terminal is wide goes on in the next row, and the blanks at a row's end are not kept. Words that name a line's
/// start themselves (they hold a line break) are looked for as they are.
#[allow(dead_code)]
pub fn holds(text: &str, words: &str) -> bool {
    let squashed = |text: &str| text.split_whitespace().collect::<String>();
    text.contains(words) || (!words.contains('\n') && squashed(text).contains(&squashed(words)))
}

pub fn said(window: &Window, id: impl Into<ElementId>) -> Option<String> {
    window.try_find(id).and_then(|e| e.label().map(String::from))
}

impl App {
    pub fn until(&self, cx: &mut TestAppContext, seen: impl Fn(&Window) -> bool) -> bool {
        until(cx, self.window, seen)
    }

    pub fn until_frame(&self, cx: &mut TestAppContext, done: impl Fn(&Frame) -> bool) -> bool {
        for _ in 0..800 {
            cx.run_until_parked();
            if self.frame.read_with(cx, |f, _| done(f)) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    pub fn press(&self, cx: &mut TestAppContext, key: &str) {
        cx.update_window(self.window, |_, w, cx| w.press(key, cx)).unwrap();
    }

    pub fn typed(&self, cx: &mut TestAppContext, text: &str) {
        cx.update_window(self.window, |_, w, cx| w.input(text, cx)).unwrap();
    }

    pub fn click(&self, cx: &mut TestAppContext, id: impl Into<ElementId>) {
        let id = id.into();
        cx.update_window(self.window, |_, w, cx| {
            w.render_frame(cx);
            w.click(id, cx)
        })
        .unwrap();
    }

    /// The workspace on screen: its name, branch and folder.
    pub fn on_screen(&self, cx: &mut TestAppContext) -> Option<(String, String, String)> {
        self.frame
            .read_with(cx, |f, _| f.current_workspace().map(|w| (w.name.clone(), w.branch.clone(), w.path.clone())))
    }

    /// The tabs shown, by name.
    pub fn tabs(&self, cx: &mut TestAppContext) -> Vec<String> {
        cx.update_window(self.window, |_, w, cx| {
            w.render_frame(cx);
            (0..20usize)
                .map_while(|ix| w.try_find(("tab", ix)))
                .map(|t| t.label().unwrap_or_default().to_string())
                .collect()
        })
        .unwrap()
    }

    /// What the terminal on screen shows, as text; empty while the workspace has none.
    pub fn terminal_text(&self, cx: &mut TestAppContext) -> String {
        cx.run_until_parked();
        self.shell
            .read_with(cx, |shell, cx| {
                shell.terminal_on_screen(cx).map(|view| view.read(cx).terminal().screen().text())
            })
            .unwrap_or_default()
    }

    /// Waits until the terminal on screen shows `words` (see [`holds`]).
    pub fn until_terminal(&self, cx: &mut TestAppContext, words: &str) -> bool {
        for _ in 0..800 {
            if holds(&self.terminal_text(cx), words) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    /// Types into the terminal on screen, as the keyboard would.
    pub fn type_in_terminal(&self, cx: &mut TestAppContext, text: &str) {
        let view = self.shell.read_with(cx, |shell, cx| shell.terminal_on_screen(cx)).expect("a terminal on screen");
        view.read_with(cx, |view, _| view.terminal().send(text.as_bytes().to_vec()));
    }

    /// How each fake Claude said it was started, oldest first (`start_live`).
    pub fn claude_starts(&self) -> Vec<serde_json::Value> {
        std::fs::read_to_string(&self.claude_log)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }

    /// Waits until `count` fake Claudes have started, and gives the last one's words.
    pub fn until_claude_started(&self, cx: &mut TestAppContext, count: usize) -> serde_json::Value {
        for _ in 0..800 {
            cx.run_until_parked();
            let starts = self.claude_starts();
            if starts.len() >= count {
                return starts[count - 1].clone();
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("{count} fake Claudes never started; the terminal shows:\n{}", self.terminal_text(cx));
    }

    /// Makes a workspace through the core (as the dialog would).
    pub fn make(shared: &Arc<Shared>, project_id: &str, name: &str, mode: &str, branch: &str, base: Option<&str>) {
        let request = enkidu_core::workspace::CreateRequest {
            project_id: project_id.into(),
            name: name.into(),
            mode: mode.into(),
            branch: branch.into(),
            base: base.map(String::from),
            session_id: None,
        };
        enkidu_core::commands::create_workspace(Events::none(), shared, request).expect("the workspace is made");
    }
}

// Remarks on lines (items 3.5c and on): pressing a line number in the diff, and what the database keeps.

/// Where the line number that says `label` is (the middle of it).
pub fn line_number(window: &Window, label: &str) -> Option<Point<Pixels>> {
    // every line number has the id ("line", its place in its side): look through the first forty
    (0..40usize)
        .flat_map(|ix| window.find_all(("line", ix)))
        .find(|number| number.label() == Some(label))
        .map(|number| number.bounds().center())
}

pub fn at(cx: &mut TestAppContext, app: &App, label: &str) -> Point<Pixels> {
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        line_number(w, label).unwrap_or_else(|| panic!("no line number says {label:?}"))
    })
    .unwrap()
}

/// A press and a release of the mouse, moving from `from` to `to` in between (the same place: a click).
pub fn press(cx: &mut TestAppContext, app: &App, from: Point<Pixels>, to: Point<Pixels>, shift: bool) {
    let modifiers = if shift { Modifiers::shift() } else { Modifiers::default() };
    let left = MouseButton::Left;
    cx.update_window(app.window, |_, w, cx| {
        let hover = MouseMoveEvent { position: from, pressed_button: None, modifiers };
        w.dispatch_event(hover.to_platform_input(), cx);
        let down = MouseDownEvent { button: left, position: from, modifiers, click_count: 1, first_mouse: false };
        w.dispatch_event(down.to_platform_input(), cx);
        w.render_frame(cx);
        if from != to {
            let moved = MouseMoveEvent { position: to, pressed_button: Some(left), modifiers };
            w.dispatch_event(moved.to_platform_input(), cx);
            w.render_frame(cx);
        }
        let up = MouseUpEvent { button: left, position: to, modifiers, click_count: 1 };
        w.dispatch_event(up.to_platform_input(), cx);
        w.render_frame(cx);
    })
    .unwrap();
}

pub fn click_line(cx: &mut TestAppContext, app: &App, label: &str) {
    let place = at(cx, app, label);
    press(cx, app, place, place, false);
}

/// The remarks the database has for the workspace, oldest first.
pub fn kept(app: &App, ws: &str) -> Vec<Remark> {
    app.shared.db.remarks(ws).unwrap()
}

pub fn until_kept(cx: &mut TestAppContext, app: &App, ws: &str, count: usize) -> Vec<Remark> {
    for _ in 0..600 {
        cx.run_until_parked();
        if kept(app, ws).len() == count {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    kept(app, ws)
}

// The Work index (phase 5): the core's watcher on the fake gh and the fake twg, never the real ones.

/// One test core's fake `gh` and `twg`: the mode each answers in (a file, so a test can break a source and fix it
/// while the app runs) and the log of every call. The core's watcher ends when this goes away.
pub struct Fakes {
    dir: PathBuf,
    shared: Arc<Shared>,
}

impl Fakes {
    /// The fake gh's mode from now on: ok, logged-out, garbage and the rest (`enkidu-core/tests/fakes/fake-gh.mjs`).
    pub fn gh(&self, mode: &str) {
        std::fs::write(self.dir.join("gh.mode"), mode).unwrap();
    }

    /// The fake gh's next read finds the PR's latest commit failing its checks: `2350=<sha>` (empty: none fails).
    pub fn gh_fail(&self, on: &str) {
        std::fs::write(self.dir.join("gh.fail"), on).unwrap();
    }

    /// The fake twg's mode from now on.
    pub fn twg(&self, mode: &str) {
        std::fs::write(self.dir.join("twg.mode"), mode).unwrap();
    }

    /// Every call the fake gh got, a line each (never its environment).
    pub fn gh_log(&self) -> String {
        std::fs::read_to_string(self.dir.join("gh.log")).unwrap_or_default()
    }

    pub fn twg_log(&self) -> String {
        std::fs::read_to_string(self.dir.join("twg.log")).unwrap_or_default()
    }
}

impl Drop for Fakes {
    fn drop(&mut self) {
        self.shared.work.stop();
    }
}

/// Refresh reads again at once in this test binary (the app waits 30 s between two reads of a project).
fn no_wait_between_reads() {
    static SET: std::sync::Once = std::sync::Once::new();
    SET.call_once(|| {
        // SAFETY: set once, to the same value, before any watcher of this test binary starts.
        unsafe { std::env::set_var("ENKIDU_WORK_MIN_GAP_MS", "0") };
    });
}

/// Gives a test's core its fake `gh` and `twg`, in `home`'s own folder of fakes. From a `prepare` it is there before
/// the window opens, so nothing the window asks at once (Setup, on a first launch) reaches a real one.
pub fn give_fakes(shared: &Arc<Shared>, home: &Path) -> Fakes {
    no_wait_between_reads();
    let dir = home.join("fakes");
    std::fs::create_dir_all(&dir).unwrap();
    let fakes = Fakes { dir: dir.clone(), shared: shared.clone() };
    fakes.gh("ok");
    fakes.twg("ok");
    let path = std::env::var("PATH").unwrap_or_default();
    let tool = |script: PathBuf, name: &str, upper: &str| {
        let mut tool = enkidu_core::work::run::Tool::new(&format!("node {}", script.display()), &path);
        let file = |kind: &str| dir.join(format!("{name}.{kind}")).to_string_lossy().into_owned();
        tool.env = vec![
            (format!("FAKE_{upper}_MODE_FILE"), file("mode")),
            (format!("FAKE_{upper}_LOG"), file("log")),
            (format!("FAKE_{upper}_FAIL_FILE"), file("fail")),
        ];
        tool
    };
    let gh = tool(enkidu_testkit::fake_gh(), "gh", "GH");
    let twg = tool(enkidu_testkit::fake_twg(), "twg", "TWG");
    shared.work.use_sources(enkidu_core::work::watch::Sources { gh, twg });
    fakes
}

/// A project's Jira projects, as you would set them in its settings (call it from a `prepare`): `SH` and `OPS`, the
/// ones the recorded answers are about.
pub fn reads_jira(shared: &Arc<Shared>, project_id: &str) {
    let mut settings = shared.project_settings(project_id).unwrap();
    settings.work.jira_keys = vec!["SH".into(), "OPS".into()];
    shared.save_project_settings(project_id, &settings).unwrap();
}

impl App {
    /// Starts the core's Work watcher, as `enkidu_core::start` does, on fakes of this test's own: `gh` and `twg` answer
    /// from the recorded answers (`enkidu-core/tests/fixtures/work`), touch no network, and each has its mode and its log
    /// in this test's folder.
    pub fn start_work(&self) -> Fakes {
        let fakes = self.fakes();
        self.start_watcher();
        fakes
    }

    /// Gives this test's core its fake `gh` and `twg`, before anything reads them (`start_watcher` does).
    pub fn fakes(&self) -> Fakes {
        give_fakes(&self.shared, self.home.path())
    }

    /// The core's Work watcher, as `enkidu_core::start` starts it. Give the core its fakes first (`fakes`).
    pub fn start_watcher(&self) {
        enkidu_core::work::start_watcher(self.sink.clone(), self.shared.clone());
    }

    /// The Work index as the window's store has it now.
    pub fn work(&self, cx: &mut TestAppContext) -> Option<Arc<enkidu_core::work::model::Snapshot>> {
        cx.run_until_parked();
        cx.update(|cx| enkidu_app::stores::Stores::work(cx).read(cx).snapshot.clone())
    }

    /// Waits until the store's index is as `seen` wants it (the core reads on threads of its own).
    pub fn until_work(
        &self,
        cx: &mut TestAppContext,
        seen: impl Fn(&enkidu_core::work::model::Snapshot) -> bool,
    ) -> Arc<enkidu_core::work::model::Snapshot> {
        for _ in 0..2000 {
            if let Some(work) = self.work(cx).filter(|work| seen(work)) {
                return work;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        // what the store shows beside what the core has: which of the two is behind
        let brief = |health: &[enkidu_core::work::model::Health], checking: bool| {
            let lines: Vec<String> = health
                .iter()
                .map(|h| format!("{} {} tried {:?} ok {:?} {:?}", h.source, h.state, h.last_try, h.last_ok, h.error))
                .collect();
            format!("checking {checking}; {}", lines.join("; "))
        };
        let store = self.work(cx).map(|w| brief(&w.health, w.checking));
        let core = enkidu_core::work::snapshot(&self.shared).map(|w| brief(&w.health, w.checking));
        let wanted = self.shared.work.wanted();
        panic!(
            "the Work index never came as expected.\n the store: {store:?}\n the core: {core:?}\n wanted: {wanted:?}"
        );
    }
}

/// Nothing was ever written: only read commands ever reached the fake `gh` and the fake `twg`, and no token was ever an
/// argument (it travels in the environment only).
pub fn never_wrote(fakes: &Fakes) {
    let gh = fakes.gh_log();
    assert!(!gh.is_empty(), "the fake gh was asked");
    for write in ["mutation", " pr comment", " pr review", " pr merge", " pr edit", " pr close", "-X POST", "--method"]
    {
        assert!(!gh.contains(write), "nothing is ever written to GitHub ({write})");
    }
    assert!(!gh.contains("fake-token"), "no token is ever an argument");
    for line in fakes.twg_log().lines() {
        let call = line.split(" | ").nth(1).unwrap_or("");
        let reads = [
            "whoami",
            "jira workitem query",
            "jira workitem get",
            "jira workitem bulk-get",
            "jira workitem comment query",
            "jira board query",
            "jira board scope query",
        ];
        let read = reads.iter().any(|read| call == *read || call.starts_with(&format!("{read} ")));
        assert!(read, "only reads ever reach Jira: {call}");
    }
}
