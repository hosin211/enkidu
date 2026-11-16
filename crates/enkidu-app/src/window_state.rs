//! The window remembers its size and place (DESIGN.md, Principle 8: "Window size and position ... survive a
//! restart"; item 1.14). It lives in `window.json` in the app's own data folder: read once as the app
//! starts, and written off the drawing thread a moment after the window last moved or changed size.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::{App, AppContext as _, Bounds, Context, Pixels, Window, WindowBounds, point, px, size};
use serde::{Deserialize, Serialize};

/// The smallest window (DESIGN.md's layout needs this much).
pub const MIN: (f32, f32) = (900., 560.);
/// A first window's size.
pub const FIRST: (f32, f32) = (1360., 860.);
/// How long the window must stay still before its place is written.
const SETTLE: Duration = Duration::from_millis(400);

/// The window as it was left.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Saved {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    #[serde(default)]
    pub maximized: bool,
    #[serde(default)]
    pub fullscreen: bool,
}

/// `window.json` in the app's data folder.
pub fn path() -> Option<PathBuf> {
    Some(enkidu_core::places::data_dir().join("window.json"))
}

pub fn load(path: &Path) -> Option<Saved> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// Written aside and moved into place, so a crash mid-write never leaves half a file.
pub fn save(path: &Path, saved: &Saved) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string(saved).map_err(std::io::Error::other)?)?;
    std::fs::rename(tmp, path)
}

/// What to keep of the window as it is now. Maximized and full screen keep the size to go back to.
pub fn of(bounds: WindowBounds) -> Saved {
    let (rect, maximized, fullscreen) = match bounds {
        WindowBounds::Windowed(rect) => (rect, false, false),
        WindowBounds::Maximized(rect) => (rect, true, false),
        WindowBounds::Fullscreen(rect) => (rect, false, true),
    };
    Saved {
        x: f32::from(rect.origin.x),
        y: f32::from(rect.origin.y),
        width: f32::from(rect.size.width),
        height: f32::from(rect.size.height),
        maximized,
        fullscreen,
    }
}

/// Where the window opens: as it was left, when enough of it is still on a display to grab (its top 100 by 40
/// points); otherwise `first`, centred. Never smaller than [`MIN`].
pub fn bounds(saved: Option<Saved>, displays: &[Bounds<Pixels>], first: Bounds<Pixels>) -> WindowBounds {
    let Some(saved) = saved else { return WindowBounds::Windowed(first) };
    let rect = Bounds {
        origin: point(px(saved.x), px(saved.y)),
        size: size(px(saved.width.max(MIN.0)), px(saved.height.max(MIN.1))),
    };
    let grip = Bounds { origin: rect.origin, size: size(px(100.), px(40.)) };
    if !displays.iter().any(|d| d.contains(&grip.origin) && d.contains(&grip.bottom_right())) {
        return WindowBounds::Windowed(first);
    }
    match (saved.fullscreen, saved.maximized) {
        (true, _) => WindowBounds::Fullscreen(rect),
        (_, true) => WindowBounds::Maximized(rect),
        _ => WindowBounds::Windowed(rect),
    }
}

/// Where the app's window opens, from `window.json` and the displays there are now.
pub fn restore(cx: &App) -> WindowBounds {
    let first = Bounds::centered(None, size(px(FIRST.0), px(FIRST.1)), cx);
    let displays: Vec<Bounds<Pixels>> = cx.displays().iter().map(|d| d.bounds()).collect();
    bounds(path().as_deref().and_then(load), &displays, first)
}

/// Keeps the window's place in `path`: a moment after it last moved or changed size, written off the drawing thread.
pub fn remember<V: 'static>(path: PathBuf, window: &mut Window, cx: &mut Context<V>) {
    let moves = Rc::new(Cell::new(0u64));
    cx.observe_window_bounds(window, move |_, window, cx| {
        let this_move = moves.get() + 1;
        moves.set(this_move);
        let saved = of(window.window_bounds());
        let (moves, path) = (moves.clone(), path.clone());
        cx.spawn(async move |_, cx| {
            cx.background_executor().timer(SETTLE).await;
            if moves.get() == this_move {
                cx.background_spawn(async move {
                    if let Err(err) = save(&path, &saved) {
                        eprintln!("Enkidu: the window's place could not be kept: {err}");
                    }
                })
                .detach();
            }
        })
        .detach();
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
        Bounds { origin: point(px(x), px(y)), size: size(px(w), px(h)) }
    }

    const FIRST_RECT: (f32, f32, f32, f32) = (280., 120., 1360., 860.);

    #[test]
    fn the_window_opens_as_it_was_left() {
        let displays = [rect(0., 0., 1920., 1080.)];
        let first = rect(FIRST_RECT.0, FIRST_RECT.1, FIRST_RECT.2, FIRST_RECT.3);
        assert_eq!(bounds(None, &displays, first), WindowBounds::Windowed(first), "the first time: centred");
        let left = Saved { x: 40., y: 60., width: 1500., height: 900., maximized: false, fullscreen: false };
        assert_eq!(bounds(Some(left), &displays, first), WindowBounds::Windowed(rect(40., 60., 1500., 900.)));
        let max = Saved { maximized: true, ..left };
        assert_eq!(bounds(Some(max), &displays, first), WindowBounds::Maximized(rect(40., 60., 1500., 900.)));
        let full = Saved { fullscreen: true, ..left };
        assert_eq!(bounds(Some(full), &displays, first), WindowBounds::Fullscreen(rect(40., 60., 1500., 900.)));
        let tiny = Saved { width: 300., height: 200., ..left };
        assert_eq!(
            bounds(Some(tiny), &displays, first),
            WindowBounds::Windowed(rect(40., 60., 900., 560.)),
            "never smaller"
        );
    }

    #[test]
    fn a_window_left_on_a_display_that_is_gone_opens_centred() {
        let first = rect(FIRST_RECT.0, FIRST_RECT.1, FIRST_RECT.2, FIRST_RECT.3);
        // it was on a second display to the right, now unplugged
        let gone = Saved { x: 2100., y: 100., width: 1400., height: 900., maximized: false, fullscreen: false };
        assert_eq!(bounds(Some(gone), &[rect(0., 0., 1920., 1080.)], first), WindowBounds::Windowed(first));
        // with the second display there, it opens there
        let two = [rect(0., 0., 1920., 1080.), rect(1920., 0., 2560., 1440.)];
        assert_eq!(bounds(Some(gone), &two, first), WindowBounds::Windowed(rect(2100., 100., 1400., 900.)));
        // only its bottom corner still showing is not enough to grab it
        let corner = Saved { x: -1300., y: 900., ..gone };
        assert_eq!(bounds(Some(corner), &[rect(0., 0., 1920., 1080.)], first), WindowBounds::Windowed(first));
    }

    #[test]
    fn what_is_kept_is_read_back_and_a_broken_file_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deeper/window.json");
        let saved = of(WindowBounds::Maximized(rect(10., 20., 1400., 900.)));
        assert_eq!(saved, Saved { x: 10., y: 20., width: 1400., height: 900., maximized: true, fullscreen: false });
        save(&path, &saved).unwrap();
        assert_eq!(load(&path), Some(saved));
        assert!(!path.with_extension("json.tmp").exists(), "nothing left aside");
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(load(&path), None);
        assert_eq!(load(&dir.path().join("none.json")), None);
    }
}
