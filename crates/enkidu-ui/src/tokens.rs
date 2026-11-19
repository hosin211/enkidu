//! The design tokens of `DESIGN.md`, in one place: color, type, space, radii and motion (item 1.10). Views use these
//! names, never raw values. A test reads DESIGN.md's tables and checks every color here against them.

use std::time::Duration;

use gpui_kit::{FontWeight, Hsla, Pixels, Rgba, px};

/// A color from `0xRRGGBB`.
pub const fn hex(value: u32) -> Rgba {
    Rgba {
        r: ((value >> 16) & 0xff) as f32 / 255.0,
        g: ((value >> 8) & 0xff) as f32 / 255.0,
        b: (value & 0xff) as f32 / 255.0,
        a: 1.0,
    }
}

/// White at an opacity (the lines and fills).
pub const fn white(alpha: f32) -> Rgba {
    Rgba { r: 1.0, g: 1.0, b: 1.0, a: alpha }
}

/// Black at an opacity (the dimming behind what floats).
pub const fn black(alpha: f32) -> Rgba {
    Rgba { r: 0.0, g: 0.0, b: 0.0, a: alpha }
}

/// A color with its opacity changed.
pub const fn alpha(color: Rgba, a: f32) -> Rgba {
    Rgba { r: color.r, g: color.g, b: color.b, a }
}

/// A color laid over the ground behind it: what is seen, with no opacity left.
pub const fn over(top: Rgba, ground: Rgba) -> Rgba {
    Rgba {
        r: top.a * top.r + (1.0 - top.a) * ground.r,
        g: top.a * top.g + (1.0 - top.a) * ground.g,
        b: top.a * top.b + (1.0 - top.a) * ground.b,
        a: 1.0,
    }
}

pub fn hsla(color: Rgba) -> Hsla {
    color.into()
}

/// A color given in OKLCH (`oklch(L C H)`, `H` in degrees), as the icons' and the badges' colors are, turned into sRGB
/// by OKLab's own formulas; values outside sRGB are clipped.
pub fn oklch(lightness: f32, chroma: f32, hue: f32) -> Rgba {
    let (a, b) = (chroma * hue.to_radians().cos(), chroma * hue.to_radians().sin());
    let l = (lightness + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
    let m = (lightness - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
    let s = (lightness - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
    let gamma = |x: f32| {
        let x = x.clamp(0.0, 1.0);
        if x <= 0.003_130_8 { 12.92 * x } else { 1.055 * x.powf(1.0 / 2.4) - 0.055 }
    };
    Rgba {
        r: gamma(4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s),
        g: gamma(-1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s),
        b: gamma(-0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s),
        a: 1.0,
    }
}

/// The color of small icons: one lightness and chroma for every hue, so no file type or project shouts louder than
/// another.
pub fn tone(hue: f32, chroma: f32) -> Rgba {
    oklch(0.77, chroma, hue)
}

// The parts (DESIGN.md, "Color"): the stage, what you read and write, is true black; the frame around it stands a
// hair above.
/// The stage.
pub const WELL: Rgba = hex(0x000000);
/// The frame: the top row, the rail, the files' column, the status line.
pub const PANE: Rgba = hex(0x0a0a0a);
/// What floats: true black too. Its edge ([`LINE_STRONG`]) and the dimming of what is behind it set it apart.
pub const SHEET: Rgba = hex(0x000000);
/// A card of the Work board when pointed at: the frame's black with 4 % of white over it.
pub const CARD_HOVER: Rgba = over(white(0.04), PANE);
pub const TIP: Rgba = hex(0x292929);
/// The edge of the pane your typing goes to, and of a card or a row the arrow keys are on: 3.6 to 1 against the
/// frame.
pub const KEY_EDGE: Rgba = alpha(ACCENT, 0.60);

// Text.
pub const INK: Rgba = hex(0xededed);
pub const INK_2: Rgba = hex(0xa8a8a8);
pub const INK_3: Rgba = hex(0x878787);
/// Text on a white (primary) button.
pub const ON_INK: Rgba = hex(0x0a0a0a);

// Lines and fills.
/// The stage's edge and the line between two panes: seen at a glance, 1.71 to 1 on the frame and 1.59 on the stage.
pub const LINE: Rgba = white(0.19);
/// A control's edge, and the edge of what floats: 2.31 to 1 on the frame.
pub const LINE_STRONG: Rgba = white(0.27);
/// Under the pointer.
pub const HOVER: Rgba = white(0.05);
/// A soft fill between the pointer's and the chosen one's: a ghost button while it is pressed, a neutral chip.
pub const SOFT: Rgba = white(0.08);
/// What is chosen: a row, a tab, a chip (DESIGN.md, "States": always stronger than [`HOVER`]).
pub const SELECTED: Rgba = white(0.14);
/// The pointer on what is chosen.
pub const SELECTED_HOVER: Rgba = white(0.18);
pub const THUMB: Rgba = white(0.14);

// What is yours, and Claude's.
/// What is yours: the edge of the tab on screen and of the pane you type in, the cursor, a field's ring, remarks.
pub const ACCENT: Rgba = hex(0x7daafd);
/// Where you are: the edge of the tab on screen, the bar of the row on screen. Blue is what is yours.
pub const HERE: Rgba = ACCENT;
/// The button that types into Claude, and a ticked box: white on it stands at 4.66 to 1.
pub const ACCENT_STRONG: Rgba = hex(0x376ceb);
pub const ACCENT_SOFT: Rgba = alpha(ACCENT, 0.16);
pub const FOCUS: Rgba = ACCENT;
/// What the blue marks turn to while the window is behind another app.
pub const BEHIND: Rgba = hex(0x6e6e6e);
pub const CLAUDE: Rgba = hex(0xea8b60);
/// The blue of the app's icon: the second wedge of the mark on the empty screens lights in it.
pub const MARK_BLUE: Rgba = hex(0x4f7dff);

// The signal lamps: Claude's state.
pub const LAMP_AMBER: Rgba = hex(0xf9b73f);
pub const LAMP_RED: Rgba = hex(0xfc5858);
pub const LAMP_GREEN: Rgba = hex(0x3cd788);
pub const LAMP_BLUE: Rgba = ACCENT;
pub const LAMP_OFF: Rgba = white(0.20);

// The diff, from the lamps so the window reads as one system.
pub const ADD_BG: Rgba = alpha(LAMP_GREEN, 0.12);
pub const ADD_WORD: Rgba = alpha(LAMP_GREEN, 0.27);
pub const ADD_INK: Rgba = hex(0x7de4a7);
pub const DEL_BG: Rgba = alpha(LAMP_RED, 0.12);
pub const DEL_WORD: Rgba = alpha(LAMP_RED, 0.27);
pub const DEL_INK: Rgba = hex(0xfe8a88);
pub const GAP_BG: Rgba = white(0.03);
pub const REMARK_BAND: Rgba = alpha(ACCENT, 0.16);

/// Behind what floats: the window dimmed (GPUI has no blur of the window's own content).
pub const SCRIM: Rgba = black(0.60);

// The terminal: the palette's colors on the stage's own black, its cursor blue.
pub const TERMINAL_SELECTION: Rgba = alpha(ACCENT, 0.25);
pub const TERMINAL_CURSOR: Rgba = ACCENT;

// Type (DESIGN.md, "Type").
pub const UI_FONT: &str = ".SystemUIFont";
pub const MONO_FONT: &str = "JetBrains Mono";
/// Body 13/18.
pub const TEXT: Pixels = px(13.);
pub const TEXT_LINE: Pixels = px(18.);
/// Secondary 12/16.
pub const TEXT_SMALL: Pixels = px(12.);
pub const TEXT_SMALL_LINE: Pixels = px(16.);
/// The smallest, 11.5/16.
pub const TEXT_STATUS: Pixels = px(11.5);
pub const TEXT_STATUS_LINE: Pixels = px(16.);
/// Captions, 11/14.
pub const CAPTION: Pixels = px(11.);
pub const CAPTION_LINE: Pixels = px(14.);
/// A panel's title, 13/20 semibold.
pub const PANEL_TITLE: Pixels = px(13.);
pub const PANEL_TITLE_LINE: Pixels = px(20.);
/// A dialog's title, 15/20 semibold.
pub const DIALOG_TITLE: Pixels = px(15.);
pub const DIALOG_TITLE_LINE: Pixels = px(20.);
/// An empty screen's title, 17/24 semibold.
pub const EMPTY_TITLE: Pixels = px(17.);
pub const EMPTY_TITLE_LINE: Pixels = px(24.);
pub const WEIGHT_BODY: FontWeight = FontWeight::NORMAL;
/// Labels.
pub const WEIGHT_LABEL: FontWeight = FontWeight::MEDIUM;
/// Titles, the Live button and what is on screen.
pub const WEIGHT_TITLE: FontWeight = FontWeight::SEMIBOLD;
/// Code in the diff, 12/20.
pub const MONO_TEXT: Pixels = px(12.);
pub const MONO_LINE: Pixels = px(20.);
pub const TERMINAL_TEXT: Pixels = px(12.5);
/// Terminal rows are the font's own height times this.
pub const TERMINAL_LINE_SCALE: f32 = 1.22;

// Space, size, shape (DESIGN.md, "Space, size, shape"). A 4 px grid, and one scale: 4, 8, 12, 16, 24, 32.
pub const GRID: Pixels = px(4.);
/// The top row: the project's name, its workspaces as tabs, and what acts on the one on screen.
pub const TOP_ROW: Pixels = px(44.);
/// A bar: a pane's bar, the bar of the files' column.
pub const BAR: Pixels = px(40.);
/// A bar inside a pane: the tools over a tree, a file's header, a board column's head.
pub const SUB_BAR: Pixels = px(32.);
/// A row of a list or a tree.
pub const LIST_ROW: Pixels = px(28.);
/// A control. One in a dialog is [`DIALOG_CONTROL`].
pub const CONTROL: Pixels = px(28.);
/// The status line, at the window's foot.
pub const STATUS_LINE: Pixels = px(28.);
/// A chip, a pill and a key.
pub const CHIP: Pixels = px(20.);
/// A badge and a file's letter.
pub const BADGE: Pixels = px(16.);
/// Text starts this far from a part's edge.
pub const INSET: Pixels = px(16.);
/// An icon that leads a row or is a button, and a small mark beside words.
pub const ICON: Pixels = px(16.);
pub const MARK: Pixels = px(12.);
/// The rail at the left: its width, a button on it, and a project's picture in a button.
pub const RAIL: Pixels = px(56.);
pub const RAIL_BUTTON: Pixels = px(40.);
pub const RAIL_PICTURE: Pixels = px(24.);
/// The files' column at the right of the stage, until its width is dragged.
pub const PANEL: Pixels = px(304.);
/// Where no column stands, the stage ends this far before the window's right edge.
pub const STAGE_MARGIN: Pixels = px(8.);
/// A control in a dialog: a field, a button, a switch with words.
pub const DIALOG_CONTROL: Pixels = px(32.);
/// The blue bar at the left of the row of what is on screen, where a list has a picked row as well.
pub const SELECTED_BAR: Pixels = px(2.);
/// The blue edge of the tab on screen and of the pane your typing goes to.
pub const ON_SCREEN_EDGE: Pixels = px(1.5);

// Radii by hierarchy.
/// Keys, a file's letter, a ticked box: what is 16 px or so.
pub const RADIUS_SMALL: Pixels = px(4.);
/// Chips.
pub const RADIUS_TAG: Pixels = px(6.);
/// Controls and rows.
pub const RADIUS_CONTROL: Pixels = px(8.);
/// Cards, remark cards, a button on the rail.
pub const RADIUS_CARD: Pixels = px(10.);
/// The stage.
pub const RADIUS_STAGE: Pixels = px(12.);
/// What floats: menus, dialogs, the palette, Search, the switcher, toasts.
pub const RADIUS_DIALOG: Pixels = px(14.);

// Motion (DESIGN.md, "Motion"): strong ease-out curves, under 300 ms, only transform and opacity.
/// `cubic-bezier(0.23, 1, 0.32, 1)`
pub const EASE_OUT: [f32; 4] = [0.23, 1.0, 0.32, 1.0];
/// `cubic-bezier(0.77, 0, 0.175, 1)`
pub const EASE_IN_OUT: [f32; 4] = [0.77, 0.0, 0.175, 1.0];
/// `cubic-bezier(0.34, 1.56, 0.64, 1)`: a slight overshoot.
pub const EASE_POP: [f32; 4] = [0.34, 1.56, 0.64, 1.0];
pub const DUR_PRESS: Duration = Duration::from_millis(120);
pub const DUR_QUICK: Duration = Duration::from_millis(160);
pub const DUR_BASE: Duration = Duration::from_millis(220);
pub const DUR_SLOW: Duration = Duration::from_millis(320);
/// A press scales to this.
pub const PRESS_SCALE: f32 = 0.97;
/// How far a dialog travels as it comes in, in px.
pub const DIALOG_TRAVEL: f32 = 8.;
/// Menus and tooltips leave in this time.
pub const MENU_OUT: Duration = Duration::from_millis(110);
/// A dialog rises this far as it fades in.
pub const DIALOG_RISE: Pixels = px(8.);
/// The needs-you ping goes out this often.
pub const PING_EVERY: Duration = Duration::from_millis(1700);
/// A lamp's pop on a new state.
pub const POP: Duration = Duration::from_millis(460);
/// A new changed file's flash.
pub const FLASH: Duration = Duration::from_millis(1800);
pub const TOAST_IN: Duration = Duration::from_millis(300);
pub const TOAST_OUT: Duration = Duration::from_millis(180);

/// Where the traffic lights sit: their top left corner, so they are centred in the top row, 16 px from the window's
/// edge. What stands after them in the row starts at [`AFTER_LIGHTS`].
pub const TRAFFIC_LIGHTS: (f32, f32) = (16., 15.);
pub const AFTER_LIGHTS: Pixels = px(84.);

/// The value a cubic-bezier curve gives at `t` (0 to 1), as CSS draws it: the curve's x is solved for `t`, its y
/// returned. For GPUI animations, which take an easing function.
pub fn bezier(curve: [f32; 4], t: f32) -> f32 {
    let [x1, y1, x2, y2] = curve;
    let at = |a: f32, b: f32, s: f32| {
        let u = 1.0 - s;
        3.0 * u * u * s * a + 3.0 * u * s * s * b + s * s * s
    };
    let t = t.clamp(0.0, 1.0);
    // x(s) rises from 0 to 1: find s with x(s) = t by halving
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..30 {
        let mid = (lo + hi) / 2.0;
        if at(x1, x2, mid) < t {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    at(y1, y2, (lo + hi) / 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DESIGN: &str = include_str!("../../../DESIGN.md");

    fn named(token: &str) -> Option<Rgba> {
        Some(match token {
            "well" => WELL,
            "pane" => PANE,
            "sheet" => SHEET,
            "tip" => TIP,
            "ink" => INK,
            "ink-2" => INK_2,
            "ink-3" => INK_3,
            "line" => LINE,
            "line-strong" => LINE_STRONG,
            "hover" => HOVER,
            "soft" => SOFT,
            "selected" => SELECTED,
            "selected-hover" => SELECTED_HOVER,
            "thumb" => THUMB,
            "behind" => BEHIND,
            "accent" => ACCENT,
            "accent-strong" => ACCENT_STRONG,
            "accent-soft" => ACCENT_SOFT,
            "claude" => CLAUDE,
            "lamp-amber" => LAMP_AMBER,
            "lamp-red" => LAMP_RED,
            "lamp-green" => LAMP_GREEN,
            "lamp-off" => LAMP_OFF,
            "add-bg" => ADD_BG,
            "add-word" => ADD_WORD,
            "add-ink" => ADD_INK,
            "del-bg" => DEL_BG,
            "del-word" => DEL_WORD,
            "del-ink" => DEL_INK,
            "gap-bg" => GAP_BG,
            "remark-band" => REMARK_BAND,
            _ => return None,
        })
    }

    /// A value as DESIGN.md writes it: `#7AA2FF`, `white at 7%`, `signal blue at 30%`, `green at 10%`, or a bare
    /// `11%` that keeps the base of the value before it.
    fn read(value: &str, base: &mut Rgba) -> Rgba {
        let value = value.trim().trim_matches('`');
        if let Some(h) = value.strip_prefix('#') {
            *base = hex(u32::from_str_radix(h, 16).unwrap());
            return *base;
        }
        let (name, percent) = match value.rsplit_once(" at ") {
            Some((name, percent)) => (Some(name), percent),
            None => (None, value),
        };
        if let Some(name) = name {
            *base = match name {
                "white" => white(1.0),
                "black" => black(1.0),
                "blue" => ACCENT,
                "green" => LAMP_GREEN,
                "red" => LAMP_RED,
                other => panic!("a color DESIGN.md names that this test doesn't know: {other}"),
            };
        }
        alpha(*base, percent.trim_end_matches('%').parse::<f32>().unwrap() / 100.0)
    }

    fn close(a: Rgba, b: Rgba) -> bool {
        [a.r - b.r, a.g - b.g, a.b - b.b, a.a - b.a].iter().all(|d| d.abs() < 0.002)
    }

    #[test]
    fn every_color_in_design_md_is_a_token_with_its_value() {
        let mut checked = Vec::new();
        // the tables: | `a` / `b` | `#x` / white at 7% | use |
        for line in DESIGN.lines().filter(|l| l.starts_with("| `")) {
            let cells: Vec<&str> = line.split('|').map(str::trim).collect();
            let names: Vec<&str> = cells[1].split(" / ").map(|n| n.trim_matches('`')).collect();
            let values: Vec<&str> = cells[2].split(" / ").collect();
            if names.iter().any(|n| named(n).is_none()) {
                continue; // a table of something else (motion, the Work page)
            }
            assert_eq!(names.len(), values.len(), "{line}");
            let mut base = white(1.0);
            for (name, value) in names.iter().zip(values) {
                let want = read(value, &mut base);
                assert!(close(named(name).unwrap(), want), "{name}: DESIGN.md says {value}");
                checked.push(name.to_string());
            }
        }
        // the diff's paragraph: `add-bg` green at 10%, `add-ink #62E0A2`, ... (wrapped over several lines)
        let diff = DESIGN
            .lines()
            .skip_while(|l| !l.starts_with("Diff, derived from the lamps"))
            .take_while(|l| !l.trim().is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        for part in diff.split([',', ';']) {
            let Some(start) = part.find('`') else { continue };
            let rest = &part[start + 1..];
            let (name, value) = match rest.split_once('`') {
                Some((name, _)) if name.contains(' ') => name.split_once(' ').unwrap(),
                Some((name, value)) => (name, value.trim().trim_end_matches('.')),
                None => continue,
            };
            let mut base = white(1.0);
            let want = read(value, &mut base);
            assert!(close(named(name).expect(name), want), "{name}: DESIGN.md says {value}");
            checked.push(name.to_string());
        }
        checked.sort();
        checked.dedup();
        assert_eq!(checked.len(), 31, "{checked:?}");
    }

    /// The contrast between two opaque colors as WCAG counts it: 1 is none, 21 is black on white.
    fn contrast(a: Rgba, b: Rgba) -> f32 {
        let lin = |c: f32| if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
        let lum = |c: Rgba| 0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b);
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    /// DESIGN.md, "States": what says where you are and where your typing goes can be seen (3 to 1 or more against
    /// what is beside it, as WCAG 1.4.11 asks of a state), what is chosen is stronger than what the pointer is on,
    /// and a line between two things is seen at a glance.
    #[test]
    fn the_marks_can_be_seen() {
        // what a mark stands on: the stage, the frame, what floats
        for (name, ground) in [("the stage", WELL), ("the frame", PANE), ("what floats", SHEET)] {
            // the blue of where you are and of the keyboard, and both while the window is behind
            let (here, blue, gray) = (contrast(HERE, ground), contrast(ACCENT, ground), contrast(BEHIND, ground));
            assert!(here >= 3.0, "where you are on {name}: {here}");
            assert!(blue >= 3.0, "the keyboard's blue on {name}: {blue}");
            assert!(gray >= 3.0, "the gray of a window behind on {name}: {gray}");
            // rest, pointer, chosen, the pointer on what is chosen: each stronger than the one before
            let (pointed, chosen, both) = (over(HOVER, ground), over(SELECTED, ground), over(SELECTED_HOVER, ground));
            let apart = contrast(chosen, pointed);
            assert!(apart >= 1.2, "chosen against pointed at on {name}: {apart}");
            assert!(contrast(both, ground) > contrast(chosen, ground), "{name}");
            assert!(contrast(over(SOFT, ground), ground) > contrast(pointed, ground), "{name}");
            assert!(contrast(chosen, ground) > contrast(over(SOFT, ground), ground), "{name}");
            // where two things meet, and a control's edge
            let (line, edge) = (contrast(over(LINE, ground), ground), contrast(over(LINE_STRONG, ground), ground));
            assert!(line >= 1.5, "a line on {name}: {line}");
            assert!(edge >= 2.0, "a control's edge on {name}: {edge}");
        }
        // the frame stands a hair above the stage, and never as much as a line: the stage's edge parts them
        let step = contrast(PANE, WELL);
        assert!((1.04..1.2).contains(&step), "the frame against the stage: {step}");
        assert_eq!(HERE, ACCENT, "blue is what is yours: where you are is blue");
        // the pane you type in, and a card or a row under the arrow keys, wear the keyboard's edge
        assert!(contrast(over(KEY_EDGE, PANE), PANE) >= 3.0);
        assert!(contrast(over(KEY_EDGE, WELL), WELL) >= 3.0);
        // hints can be read on every ground, and the name of what is on screen stands out from the others
        for ground in [WELL, PANE, SHEET] {
            assert!(contrast(INK_3, ground) >= 4.5, "{}", contrast(INK_3, ground));
        }
        assert!(contrast(INK, INK_2) >= 1.9);
        // white words on the button that types into Claude
        assert!(contrast(hex(0xffffff), ACCENT_STRONG) >= 4.5);
    }

    #[test]
    fn the_curves_and_times_are_design_mds() {
        let motion = DESIGN.lines().find(|l| l.starts_with("Tokens: `--ease-out`")).unwrap();
        assert!(motion.contains("cubic-bezier(0.23, 1, 0.32, 1)") && EASE_OUT == [0.23, 1.0, 0.32, 1.0]);
        assert!(motion.contains("cubic-bezier(0.77, 0, 0.175, 1)") && EASE_IN_OUT == [0.77, 0.0, 0.175, 1.0]);
        let times = DESIGN.lines().skip_while(|l| !l.starts_with("Tokens: `--ease-out`")).nth(1).unwrap();
        let both = format!("{motion} {times}");
        for (name, d) in [("press", DUR_PRESS), ("quick", DUR_QUICK), ("base", DUR_BASE), ("slow", DUR_SLOW)] {
            let ms = d.as_millis().to_string();
            assert!(both.contains(&format!("--dur-{name}` {ms}")), "--dur-{name} {ms} ms: {both}");
        }
        // the curve: 0 and 1 at its ends, and well past halfway early (a strong ease-out)
        assert!(bezier(EASE_OUT, 0.0).abs() < 1e-3 && (bezier(EASE_OUT, 1.0) - 1.0).abs() < 1e-3);
        assert!(bezier(EASE_OUT, 0.25) > 0.6, "{}", bezier(EASE_OUT, 0.25));
        assert!(bezier(EASE_POP, 0.6) > 1.0, "the pop overshoots: {}", bezier(EASE_POP, 0.6));
    }

    /// The motion table's own numbers, row by row (the rows without a number are tested where they move).
    #[test]
    fn the_motion_tables_numbers_are_ours() {
        let table = DESIGN.split("\n## Motion").nth(1).unwrap().split("\n## ").next().unwrap();
        let ms = |d: Duration| d.as_millis();
        for (said, ours) in [
            ("| A press (`press::pressable`) | scale 0.97, 120 ms |", PRESS_SCALE == 0.97 && ms(DUR_PRESS) == 120),
            (
                "| Menus, tooltips | grow from their trigger, 160 ms; out in 110;",
                ms(DUR_QUICK) == 160 && ms(crate::pieces::MENU_LEAVE) == 110 && ms(crate::tips::LEAVE) == 110,
            ),
            ("| Dialogs | rise 8 px and fade, 220 ms,", DIALOG_TRAVEL == 8. && ms(DUR_BASE) == 220),
            ("| A switch's thumb | glides 220 ms to the new choice |", ms(DUR_BASE) == 220),
            ("| Needs you | a ping goes out from the red lamp every 1.7 s |", ms(PING_EVERY) == 1700),
            ("| A new state | the lamp pops once (460 ms, a small overshoot) |", ms(POP) == 460),
            ("the changes rise in (220 ms)", ms(DUR_BASE) == 220),
            ("its row flashes white and settles (1.8 s)", ms(FLASH) == 1800),
            ("| Counts, +/− | tick into place (220 ms) when they change, never on first show |", ms(DUR_BASE) == 220),
        ] {
            assert!(table.contains(said), "DESIGN.md no longer says {said:?}");
            assert!(ours, "ours differs from {said:?}");
        }
        // the pop overshoots a little and comes back: never an ease-in
        assert!(bezier(EASE_POP, 0.6) > 1.0 && (bezier(EASE_POP, 1.0) - 1.0).abs() < 1e-3);
        assert!(table.contains("Only\ntransform and opacity move; never ease-in."));
    }

    #[test]
    fn the_type_sizes_and_weights_are_design_mds() {
        // the section as one line: its sentences wrap
        let said_of_type = DESIGN.split("\n## Type").nth(1).unwrap().split("\n## ").next().unwrap();
        let said_of_type = said_of_type.split_whitespace().collect::<Vec<_>>().join(" ");
        for (said, size, line) in [
            ("Body 13/18", TEXT, TEXT_LINE),
            ("secondary 12/16", TEXT_SMALL, TEXT_SMALL_LINE),
            ("the smallest 11.5/16", TEXT_STATUS, TEXT_STATUS_LINE),
            ("caption 11/14", CAPTION, CAPTION_LINE),
            ("panel title 13/20 semibold", PANEL_TITLE, PANEL_TITLE_LINE),
            ("dialog title 15/20 semibold", DIALOG_TITLE, DIALOG_TITLE_LINE),
            ("empty-state title 17/24 semibold", EMPTY_TITLE, EMPTY_TITLE_LINE),
            ("12/20 in the diff", MONO_TEXT, MONO_LINE),
        ] {
            assert!(said_of_type.contains(said), "DESIGN.md no longer says {said:?}");
            let numbers = said.split(' ').find(|word| word.contains('/')).unwrap();
            let (text, row) = numbers.split_once('/').unwrap();
            let want: (f32, f32) = (text.parse().unwrap(), row.parse().unwrap());
            assert_eq!((f32::from(size), f32::from(line)), want, "{said}");
        }
        assert!(said_of_type.contains("12.5 in the terminal") && f32::from(TERMINAL_TEXT) == 12.5);
        assert!(said_of_type.contains("JetBrains Mono") && MONO_FONT == "JetBrains Mono");
        assert!(
            said_of_type.contains("Weights 400, 500 for labels, 600 for titles, the Live button and what is on screen")
        );
        assert_eq!([WEIGHT_BODY.0, WEIGHT_LABEL.0, WEIGHT_TITLE.0], [400., 500., 600.]);
    }

    #[test]
    fn oklch_gives_the_known_srgb_colors() {
        // sRGB's own primaries and white, as OKLab gives them
        for (l, c, h, want) in [
            (1.0, 0.0, 0.0, hex(0xffffff)),
            (0.0, 0.0, 0.0, hex(0x000000)),
            (0.627_955, 0.257_683, 29.233_885, hex(0xff0000)),
            (0.866_440, 0.294_827, 142.495_34, hex(0x00ff00)),
            (0.452_014, 0.313_214, 264.052_02, hex(0x0000ff)),
        ] {
            let got = oklch(l, c, h);
            // within 1.5 of 255 on each channel
            let near = [got.r - want.r, got.g - want.g, got.b - want.b].iter().all(|d| d.abs() < 0.006);
            assert!(near, "oklch({l} {c} {h}) gave {got:?}");
        }
        // Python's blue sits light and blue
        let python = tone(250.0, 0.13);
        assert!(python.b > python.r && python.b > python.g && python.r > 0.3, "{python:?}");
    }

    #[test]
    fn sizes_and_radii_are_design_mds() {
        let space = DESIGN.split("## Space, size, shape").nth(1).unwrap().split("\n## ").next().unwrap();
        let space = space.split_whitespace().collect::<Vec<_>>().join(" ");
        for (said, value) in [
            ("The top row 44", TOP_ROW),
            ("a bar 40", BAR),
            ("a bar inside a pane 32", SUB_BAR),
            ("a row 28", LIST_ROW),
            ("a control 28", CONTROL),
            ("the status line 28", STATUS_LINE),
            ("a control in a dialog 32", DIALOG_CONTROL),
            ("and a key 20", CHIP),
            ("a file's letter 16", BADGE),
            ("text starts 16 px", INSET),
            ("The rail is 56 px wide", RAIL),
            ("its buttons are 40 px", RAIL_BUTTON),
            ("a project's picture in one 24", RAIL_PICTURE),
            ("The files' column is 304", PANEL),
            ("ends 8 px before the window's right edge", STAGE_MARGIN),
            ("4 (keys", RADIUS_SMALL),
            ("6 (chips)", RADIUS_TAG),
            ("8 (controls", RADIUS_CONTROL),
            ("10 (cards", RADIUS_CARD),
            ("12 (the stage)", RADIUS_STAGE),
            ("14 (what floats", RADIUS_DIALOG),
        ] {
            assert!(space.contains(said), "DESIGN.md no longer says {said:?}");
            let number = said.chars().filter(|c| c.is_ascii_digit()).collect::<String>();
            assert_eq!(f32::from(value), number.parse::<f32>().unwrap(), "{said}");
        }
        assert!(space.contains("one scale for every space: 4, 8, 12, 16, 24 or 32 px") && f32::from(GRID) == 4.0);
        // an icon in one of two sizes
        let icons = DESIGN.split("\n## Icons").nth(1).unwrap().split("\n## ").next().unwrap();
        assert!(icons.contains("16 px where an icon leads a row or is a button, 12 px for a small mark"));
        assert_eq!((f32::from(ICON), f32::from(MARK)), (16.0, 12.0));
    }
}
