//! Item 3.12: the diff view in the changes island, one file at a time. Each test gives the island one changed file with
//! its diff (`Shell::with_changes`, no core). "writes a remark on a line with its code" is in `remark_in_diff.rs`,
//! "says when a file is binary" in `review.rs` (both on real git), and the lamp's labels in enkidu-ui's `lamp.rs`.

use enkidu_app::shell::Shell;
use enkidu_core::git::Change;
use enkidu_model::ui::DiffLayout;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, Pixels, TestAppContext, Window, WindowBounds, WindowOptions, px, size,
};

/// The file of these tests: a loop of four lines became one line.
fn patch(path: &str) -> String {
    format!(
        "diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -1,5 +1,2 @@\n def total(items):\n-    s = 0\n-    \
         for i in items:\n-        s += i\n-    return s\n+    return sum(items)\n"
    )
}

/// A window whose workspace has that one changed file, its diff in `layout`.
fn open(cx: &mut TestAppContext, path: &str, layout: DiffLayout) -> AnyWindowHandle {
    let change = Change {
        path: path.into(),
        old_path: None,
        kind: "modified".into(),
        staged: false,
        additions: Some(1),
        deletions: Some(4),
        sig: format!("sig-{path}"),
    };
    let diff = patch(path);
    let window = cx.update(|cx| {
        enkidu_app::init(cx);
        // what is on screen is where it stays (see tests/common)
        cx.set_reduce_motion(true);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Default::default(),
                size: size(px(1360.), px(860.)),
            })),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| {
                let terminal = std::sync::Arc::new(enkidu_term::Terminal::kept(
                    enkidu_term::GridSize::new(120, 36),
                    enkidu_term::Palette::enkidu(),
                ));
                let view = cx.new(|cx| enkidu_app::terminal_view::TerminalView::new(terminal, None, cx));
                Shell::with_changes(enkidu_app::demo::tabs(), vec![view], vec![(change, diff)], layout, window, cx)
            })
        })
        .expect("a test window")
        .0
    });
    for _ in 0..4 {
        cx.run_until_parked();
        cx.update_window(window, |_, w, cx| w.render_frame(cx)).unwrap();
    }
    window
}

/// The diff's lines on screen: what each says, which line it is, and where it is.
fn lines(window: &Window) -> Vec<(String, String, Bounds<Pixels>)> {
    window
        .find_all("source")
        .iter()
        .map(|line| {
            (
                line.label().unwrap_or_default().to_string(),
                line.description().unwrap_or_default().to_string(),
                line.bounds(),
            )
        })
        .collect()
}

/// Shows a unified diff with removed and added lines.
#[gpui_kit::test]
fn shows_a_unified_diff_with_removed_and_added_lines(cx: &mut TestAppContext) {
    let window = open(cx, "calc/a.py", DiffLayout::Unified);
    cx.update_window(window, |_, w, _| {
        let lines = lines(w);
        let which = |ending: &str| lines.iter().filter(|(_, line, _)| line.ends_with(ending)).count();
        assert_eq!(which(", added"), 1, "{lines:?}");
        assert_eq!(which(", removed"), 4);
        assert!(lines.iter().any(|(text, _, _)| text == "    return sum(items)"));
        // one column: each line under the one before it, the removed ones first
        let tops: Vec<Pixels> = lines.iter().map(|(_, _, bounds)| bounds.top()).collect();
        assert!(tops.windows(2).all(|pair| pair[0] < pair[1]), "{tops:?}");
        let texts: Vec<&str> = lines.iter().map(|(text, _, _)| text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "def total(items):",
                "    s = 0",
                "    for i in items:",
                "        s += i",
                "    return s",
                "    return sum(items)"
            ]
        );
    })
    .unwrap();
}

/// Puts before and after side by side, pairing the replaced lines.
#[gpui_kit::test]
fn puts_before_and_after_side_by_side_pairing_the_replaced_lines(cx: &mut TestAppContext) {
    let window = open(cx, "calc/b.py", DiffLayout::Split);
    cx.update_window(window, |_, w, _| {
        let lines = lines(w);
        let at = |text: &str, which: &str| {
            lines
                .iter()
                .find(|(said, line, _)| said == text && line == which)
                .map(|(_, _, bounds)| *bounds)
                .unwrap_or_else(|| panic!("no line {text:?} ({which}) among {lines:?}"))
        };
        // four rows hold the change: the four removed lines, each on the left
        let removed: Vec<&(String, String, Bounds<Pixels>)> =
            lines.iter().filter(|(_, line, _)| line.ends_with(", removed")).collect();
        assert_eq!(removed.len(), 4);
        // the first removed line faces the line that replaced it
        let (before, after) =
            (at("    s = 0", "Line 2, before, removed"), at("    return sum(items)", "Line 2, after, added"));
        assert_eq!(before.top(), after.top(), "they are on one row");
        assert!(before.right() <= after.left(), "before on the left, after on the right");
        // the line that didn't change is on both sides, on the row above
        let (kept_before, kept_after) =
            (at("def total(items):", "Line 1, before"), at("def total(items):", "Line 1, after"));
        assert_eq!(kept_before.top(), kept_after.top());
        assert!(kept_before.top() < before.top());
        // and the sides are named
        assert!(w.try_find("split-before").is_some() && w.try_find("split-after").is_some());
    })
    .unwrap();
}
