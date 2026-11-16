//! Trackpad and wheel scrolling, in whole lines.
//!
//! A trackpad scrolls by pixels, a mouse wheel by lines. The terminal moves by lines: its history, or one wheel
//! report per line to a program that takes the mouse (Claude Code in full screen, vim, less with the mouse on), as
//! Warp and iTerm do. xterm.js sent at most one report per wheel event and slowed a trackpad to 30 %, so a swipe that
//! moves a page in Warp moved Claude a few lines; here every line's worth of scrolling is one report.

use crate::keys::Mods;

/// No more lines than this from one event, so a page-sized jump stays a page, not a flood.
pub const MAX_LINES: i32 = 40;

/// Option scrolls this many times faster, and so does Ctrl over a program that takes the mouse (xterm.js's
/// `fastScrollSensitivity`).
pub const FAST_SCROLL: f32 = 5.0;

/// How far one wheel event goes. Positive is toward the history.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WheelDelta {
    /// A trackpad's pixels.
    Pixels(f32),
    /// A mouse wheel's lines.
    Lines(f32),
    /// Whole pages.
    Pages(f32),
}

/// The whole lines a wheel event moves, with rows `line_height` pixels tall and `rows` of them to a page, plus what
/// the events before it left over (`carried`, in lines). Returns the lines to move now and what to carry to the next
/// event. Turning around drops what was carried, so the new direction answers at once.
pub fn wheel_lines(delta: WheelDelta, line_height: f32, rows: usize, carried: f32) -> (i32, f32) {
    let pixels = match delta {
        WheelDelta::Pixels(pixels) => pixels,
        WheelDelta::Lines(lines) => lines * line_height,
        WheelDelta::Pages(pages) => pages * line_height * rows as f32,
    };
    if line_height <= 0.0 || !pixels.is_finite() {
        return (0, carried);
    }
    let before = if carried != 0.0 && carried.signum() == -pixels.signum() { 0.0 } else { carried };
    let total = before + pixels / line_height;
    let whole = total.trunc();
    let lines = (whole as i32).clamp(-MAX_LINES, MAX_LINES);
    (lines, total - whole)
}

/// How much faster a wheel event goes with these modifiers: Option always, Ctrl when the program takes the mouse.
pub fn wheel_speed(mods: Mods, program_takes_mouse: bool) -> f32 {
    if mods.alt || (mods.ctrl && program_takes_mouse) { FAST_SCROLL } else { 1.0 }
}

/// Four tests of the wheel. Positive is toward the history, GPUI's way (a browser counts the other way).
#[cfg(test)]
mod tests {
    use super::*;

    const LINE: f32 = 15.0;

    #[test]
    fn scrolls_a_line_for_every_line_height_of_trackpad_pixels_without_slowing_it_down() {
        assert_eq!(wheel_lines(WheelDelta::Pixels(150.0), LINE, 40, 0.0), (10, 0.0));
        assert_eq!(wheel_lines(WheelDelta::Pixels(-45.0), LINE, 40, 0.0), (-3, 0.0));
    }

    #[test]
    fn adds_up_a_slow_swipes_small_steps() {
        let mut carried = 0.0;
        let mut lines = 0;
        for _ in 0..9 {
            let (step, rest) = wheel_lines(WheelDelta::Pixels(5.0), LINE, 40, carried);
            carried = rest;
            lines += step;
        }
        // 45 pixels: three lines, nothing lost on the way
        assert_eq!(lines, 3);
        assert!(carried.abs() < 1e-4, "{carried}");
    }

    #[test]
    fn answers_a_turn_at_once_and_counts_a_mouses_lines_and_pages_as_they_are() {
        // a little toward the history left over, then the other way: the leftover doesn't eat the first line
        assert_eq!(wheel_lines(WheelDelta::Pixels(-15.0), LINE, 40, 0.9).0, -1);
        assert_eq!(wheel_lines(WheelDelta::Lines(3.0), LINE, 40, 0.0).0, 3);
        assert_eq!(wheel_lines(WheelDelta::Pages(1.0), LINE, 30, 0.0).0, 30);
    }

    #[test]
    fn never_sends_more_than_a_page_sized_burst_from_one_event() {
        assert_eq!(wheel_lines(WheelDelta::Pixels(100_000.0), LINE, 40, 0.0).0, MAX_LINES);
        assert_eq!(wheel_lines(WheelDelta::Pixels(-100_000.0), LINE, 40, 0.0).0, -MAX_LINES);
    }

    #[test]
    fn option_scrolls_faster_and_ctrl_too_over_a_program_that_takes_the_mouse() {
        assert_eq!(wheel_speed(Mods::NONE, true), 1.0);
        assert_eq!(wheel_speed(Mods::ALT, false), FAST_SCROLL);
        assert_eq!(wheel_speed(Mods::CTRL, false), 1.0);
        assert_eq!(wheel_speed(Mods::CTRL, true), FAST_SCROLL);
    }
}
