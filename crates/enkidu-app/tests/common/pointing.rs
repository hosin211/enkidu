//! Pointing at code with ⌘ held, as the tests of going to a definition do it: where a name stands on screen, the
//! pointer moved and pressed there, the name that is a link, the file and the line that are open.

use enkidu_app::changes_island::ChangesIsland;
use enkidu_diff::{DiffName, DiffState};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Bounds, Capslock, Entity, InputEvent as _, Modifiers, ModifiersChangedEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, TestAppContext, Window, point, px,
};

use super::{App, said};

pub fn island(cx: &mut TestAppContext, app: &App) -> Entity<ChangesIsland> {
    app.shell.read_with(cx, |shell, _| shell.changes().cloned()).expect("the island")
}

pub fn look<R>(cx: &mut TestAppContext, app: &App, at: impl FnOnce(&mut Window) -> R) -> R {
    cx.run_until_parked();
    cx.update_window(app.window, |_, w, cx| {
        w.render_frame(cx);
        at(w)
    })
    .unwrap()
}

pub fn open(cx: &mut TestAppContext, app: &App, path: &str) {
    let island = island(cx, app);
    cx.update(|cx| island.update(cx, |island, cx| island.open_file(path, None, cx)));
    assert!(app.until(cx, |w| said(w, "file-view").as_deref() == Some(path)), "{path} never opened");
}

/// The view of the code that is on screen.
pub fn code(cx: &mut TestAppContext, app: &App) -> Entity<DiffState> {
    let island = island(cx, app);
    cx.update(|cx| island.read(cx).code_on_screen(cx)).expect("code on screen")
}

/// Where a line of the code on screen is: the one whose text is `text`, whichever side says `which`.
pub fn line(cx: &mut TestAppContext, app: &App, text: &str, which: &str) -> Bounds<Pixels> {
    let found = |w: &Window| {
        w.find_all("source")
            .iter()
            .find(|line| line.label() == Some(text) && line.description().is_some_and(|said| said.starts_with(which)))
            .map(|line| line.bounds())
    };
    assert!(app.until(cx, |w| found(w).is_some()), "no line {text:?} ({which})");
    look(cx, app, |w| found(w).unwrap())
}

/// A point on the name `word` of that line, a little inside it.
pub fn on(cx: &mut TestAppContext, app: &App, text: &str, which: &str, word: &str) -> Point<Pixels> {
    let (bounds, state) = (line(cx, app, text, which), code(cx, app));
    let start = text.find(word).unwrap_or_else(|| panic!("no {word} in {text:?}"));
    look(cx, app, |_| ());
    cx.update(|cx| {
        let state = state.read(cx);
        let y = bounds.center().y;
        let mut x = bounds.left();
        while x < bounds.right() {
            if state.name_at(point(x, y)).is_some_and(|name| name.range.start == start) {
                return point(x + px(1.), y);
            }
            x += px(1.);
        }
        panic!("the name {word} isn't on screen in {text:?}");
    })
}

pub fn moved(cx: &mut TestAppContext, app: &App, position: Point<Pixels>, modifiers: Modifiers) {
    cx.update_window(app.window, |_, w, cx| {
        w.dispatch_event(MouseMoveEvent { position, pressed_button: None, modifiers }.to_platform_input(), cx);
        w.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
}

pub fn clicked(cx: &mut TestAppContext, app: &App, position: Point<Pixels>, modifiers: Modifiers) {
    let left = MouseButton::Left;
    cx.update_window(app.window, |_, w, cx| {
        let down = MouseDownEvent { button: left, position, modifiers, click_count: 1, first_mouse: false };
        w.dispatch_event(down.to_platform_input(), cx);
        w.render_frame(cx);
        w.dispatch_event(MouseUpEvent { button: left, position, modifiers, click_count: 1 }.to_platform_input(), cx);
        w.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
}

pub fn keys(cx: &mut TestAppContext, app: &App, modifiers: Modifiers) {
    cx.update_window(app.window, |_, w, cx| {
        let event = ModifiersChangedEvent { modifiers, capslock: Capslock { on: false } };
        w.dispatch_event(event.to_platform_input(), cx);
        w.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
}

/// The name that is a link in the code on screen, as its text.
pub fn link(cx: &mut TestAppContext, app: &App, text: &str) -> Option<String> {
    let state = code(cx, app);
    cx.run_until_parked();
    cx.update(|cx| state.read(cx).link().cloned()).map(|name: DiffName| text[name.range].to_string())
}

/// Waits for the core's answer: the name under the pointer is a link.
pub fn until_link(cx: &mut TestAppContext, app: &App, text: &str, word: &str) {
    for _ in 0..500 {
        if link(cx, app, text).as_deref() == Some(word) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
        // (while a language's server can't be asked yet the app asks again after a moment, by the clock this test
        // holds)
        cx.executor().advance_clock(std::time::Duration::from_millis(100));
    }
    panic!("{word} never became a link in {text:?}: {:?}", link(cx, app, text));
}

/// The pointer comes to rest on a name with ⌘ held, after having been somewhere else (a name is asked about once
/// while the pointer stays on it).
pub fn rest_on(cx: &mut TestAppContext, app: &App, text: &str, which: &str, word: &str) -> Point<Pixels> {
    let at = on(cx, app, text, which, word);
    moved(cx, app, point_at(2., 2.), Modifiers::command());
    moved(cx, app, at, Modifiers::command());
    at
}

fn point_at(x: f32, y: f32) -> Point<Pixels> {
    point(px(x), px(y))
}

/// No link shows for a while: the core answered, and named nothing (or went on having no answer).
pub fn no_link(cx: &mut TestAppContext, app: &App, text: &str, why: &str) {
    for _ in 0..6 {
        std::thread::sleep(std::time::Duration::from_millis(80));
        cx.executor().advance_clock(std::time::Duration::from_millis(250));
        cx.run_until_parked();
    }
    assert_eq!(link(cx, app, text), None, "{why}");
}

/// The fake language server's files (`enkidu-core/tests/fakes/fake-lsp.mjs`): what it logs, its mode, and what it
/// answers. A test's process has one: naming it is for the whole process.
pub struct FakeServer {
    _folder: tempfile::TempDir,
    pub log: std::path::PathBuf,
    pub mode: std::path::PathBuf,
    pub answer: std::path::PathBuf,
}

impl FakeServer {
    /// Makes the files and names the fake as the server for Python, for this process. It names nothing until told.
    pub fn named() -> FakeServer {
        let folder = tempfile::tempdir().unwrap();
        let (log, mode, answer) = (folder.path().join("log"), folder.path().join("mode"), folder.path().join("answer"));
        let command = format!(
            "FAKE_LSP_LOG='{}' FAKE_LSP_MODE='{}' FAKE_LSP_ANSWER='{}' exec node '{}'",
            log.display(),
            mode.display(),
            answer.display(),
            enkidu_testkit::fake_lsp().display()
        );
        // SAFETY: set once, before the app of this one-test process starts and before any thread reads it.
        unsafe { std::env::set_var("ENKIDU_PYTHON_SERVER", command) };
        FakeServer { _folder: folder, log, mode, answer }
    }

    /// The server will name these places: each a file and its line from 1.
    pub fn says(&self, places: &[(&std::path::Path, usize)]) {
        let places: Vec<serde_json::Value> = (places.iter())
            .map(|(file, line)| {
                serde_json::json!({
                    "uri": format!("file://{}", file.display()),
                    "range": {"start": {"line": line - 1, "character": 4}, "end": {"line": line - 1, "character": 8}},
                })
            })
            .collect();
        std::fs::write(&self.answer, serde_json::Value::Array(places).to_string()).unwrap();
    }

    /// What the server was told, in order.
    pub fn log(&self) -> Vec<serde_json::Value> {
        let text = std::fs::read_to_string(&self.log).unwrap_or_default();
        text.lines().map(|line| serde_json::from_str(line).unwrap()).collect()
    }
}

/// The line that is marked in the opened file (the one it was opened at), as its text.
pub fn marked(window: &Window) -> Option<String> {
    window
        .find_all("source")
        .iter()
        .find(|line| line.description().is_some_and(|said| said.ends_with(", marked")))
        .map(|line| line.label().unwrap_or_default().to_string())
}

pub fn opened(window: &Window) -> Option<String> {
    said(window, "file-view")
}
