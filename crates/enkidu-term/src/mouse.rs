//! What the mouse sends to a program that asked for it (xterm's mouse tracking).
//!
//! A program turns tracking on with `CSI ? 1000 h` (presses and releases, and the wheel), `1002` (also moves while a
//! button is held) or `1003` (every move), and picks how reports look: the classic `CSI M` and three bytes, `1005`
//! (the same with positions as UTF-8 characters, for wide screens) or `1006` (SGR, `CSI < b ; x ; y M`, which also
//! says which button was released). Shift with the mouse stays the window's, as in xterm: it selects text even while
//! a program takes the mouse.

use crate::keys::Mods;

/// What the program asked to hear from the mouse.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MouseMode {
    /// Nothing: the mouse selects text and the wheel moves through the history.
    #[default]
    Off,
    /// Presses, releases and the wheel (`1000`).
    Click,
    /// Those, and moves while a button is held (`1002`).
    Drag,
    /// Those, and every move (`1003`).
    Motion,
}

impl MouseMode {
    /// The program takes the mouse.
    pub fn on(self) -> bool {
        self != MouseMode::Off
    }
}

/// How the program wants its mouse reports written.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MouseEncoding {
    /// `CSI M` and three bytes; nothing past column or row 223.
    #[default]
    Classic,
    /// The same, with positions as UTF-8 characters up to 2015 (`1005`).
    Utf8,
    /// `CSI < b ; x ; y M`, with `m` for a release (`1006`).
    Sgr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Middle,
    Right,
}

impl Button {
    fn code(self) -> u32 {
        match self {
            Button::Left => 0,
            Button::Middle => 1,
            Button::Right => 2,
        }
    }
}

/// What the mouse did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseAction {
    Press(Button),
    Release(Button),
    /// The pointer moved onto another cell, with this button held, or none.
    Move(Option<Button>),
    /// One line of wheel travel toward the history.
    WheelUp,
    /// One line of wheel travel toward the bottom.
    WheelDown,
}

/// The farthest position each encoding can write (columns and rows count from 1).
const CLASSIC_LIMIT: usize = 223;
const UTF8_LIMIT: usize = 2015;

/// The report a program gets for `action` at the cell `row`, `col` (both from 0), or None when its mode doesn't ask
/// for that action or its encoding can't write that position.
pub fn mouse_report(
    action: MouseAction,
    row: usize,
    col: usize,
    mods: Mods,
    mode: MouseMode,
    encoding: MouseEncoding,
) -> Option<Vec<u8>> {
    let button = match action {
        MouseAction::Press(button) => button.code(),
        // The classic forms can't say which button went up.
        MouseAction::Release(button) if encoding == MouseEncoding::Sgr => button.code(),
        MouseAction::Release(_) => 3,
        MouseAction::Move(Some(button)) if matches!(mode, MouseMode::Drag | MouseMode::Motion) => 32 + button.code(),
        MouseAction::Move(None) if mode == MouseMode::Motion => 32 + 3,
        MouseAction::Move(_) => return None,
        MouseAction::WheelUp => 64,
        MouseAction::WheelDown => 65,
    };
    if !mode.on() {
        return None;
    }
    let code = button + 4 * u32::from(mods.shift) + 8 * u32::from(mods.alt) + 16 * u32::from(mods.ctrl);
    let (x, y) = (col + 1, row + 1);
    match encoding {
        MouseEncoding::Sgr => {
            let end = if matches!(action, MouseAction::Release(_)) { 'm' } else { 'M' };
            Some(format!("\x1b[<{code};{x};{y}{end}").into_bytes())
        }
        MouseEncoding::Classic => {
            if x > CLASSIC_LIMIT || y > CLASSIC_LIMIT {
                return None;
            }
            Some(vec![0x1b, b'[', b'M', (32 + code) as u8, (32 + x) as u8, (32 + y) as u8])
        }
        MouseEncoding::Utf8 => {
            if x > UTF8_LIMIT || y > UTF8_LIMIT {
                return None;
            }
            let mut out = b"\x1b[M".to_vec();
            for value in [32 + code, (32 + x) as u32, (32 + y) as u32] {
                let mut buf = [0u8; 4];
                out.extend_from_slice(char::from_u32(value)?.encode_utf8(&mut buf).as_bytes());
            }
            Some(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use MouseAction::*;
    use MouseEncoding::*;
    use MouseMode::*;

    fn report(
        action: MouseAction,
        row: usize,
        col: usize,
        mode: MouseMode,
        encoding: MouseEncoding,
    ) -> Option<Vec<u8>> {
        mouse_report(action, row, col, Mods::NONE, mode, encoding)
    }

    #[test]
    fn nothing_goes_to_a_program_that_did_not_ask() {
        assert_eq!(report(Press(Button::Left), 0, 0, Off, Sgr), None);
        assert_eq!(report(WheelUp, 0, 0, Off, Classic), None);
    }

    #[test]
    fn sgr_reports_say_the_button_the_cell_and_whether_it_went_down_or_up() {
        assert_eq!(report(Press(Button::Left), 4, 9, Click, Sgr).unwrap(), b"\x1b[<0;10;5M");
        assert_eq!(report(Release(Button::Left), 4, 9, Click, Sgr).unwrap(), b"\x1b[<0;10;5m");
        assert_eq!(report(Press(Button::Right), 0, 0, Click, Sgr).unwrap(), b"\x1b[<2;1;1M");
        assert_eq!(report(Release(Button::Middle), 0, 0, Click, Sgr).unwrap(), b"\x1b[<1;1;1m");
        // far past what the classic form can write
        assert_eq!(report(Press(Button::Left), 299, 499, Click, Sgr).unwrap(), b"\x1b[<0;500;300M");
    }

    #[test]
    fn the_wheel_is_buttons_64_and_65_and_never_released() {
        assert_eq!(report(WheelUp, 2, 3, Click, Sgr).unwrap(), b"\x1b[<64;4;3M");
        assert_eq!(report(WheelDown, 2, 3, Click, Sgr).unwrap(), b"\x1b[<65;4;3M");
        assert_eq!(report(WheelUp, 2, 3, Click, Classic).unwrap(), b"\x1b[M\x60\x24\x23");
    }

    #[test]
    fn classic_reports_are_three_bytes_and_a_release_is_button_3() {
        assert_eq!(report(Press(Button::Left), 0, 0, Click, Classic).unwrap(), b"\x1b[M\x20\x21\x21");
        assert_eq!(report(Press(Button::Right), 9, 19, Click, Classic).unwrap(), b"\x1b[M\x22\x34\x2a");
        assert_eq!(report(Release(Button::Right), 9, 19, Click, Classic).unwrap(), b"\x1b[M\x23\x34\x2a");
        // column 223 is the last one the form can write
        assert_eq!(report(Press(Button::Left), 0, 222, Click, Classic).unwrap(), vec![0x1b, b'[', b'M', 32, 255, 33]);
        assert_eq!(report(Press(Button::Left), 0, 223, Click, Classic), None);
    }

    #[test]
    fn utf8_reports_write_far_positions_as_characters() {
        assert_eq!(report(Press(Button::Left), 0, 0, Click, Utf8).unwrap(), b"\x1b[M\x20\x21\x21");
        // column 300: 32 + 300 = 332, two bytes in UTF-8
        let far = report(Press(Button::Left), 0, 299, Click, Utf8).unwrap();
        assert_eq!(far, [b"\x1b[M ".as_slice(), "\u{14c}".as_bytes(), b"!"].concat());
        assert_eq!(report(Press(Button::Left), 0, 2015, Click, Utf8), None);
    }

    #[test]
    fn moves_are_sent_only_in_the_modes_that_ask_for_them() {
        // presses only: no moves at all
        assert_eq!(report(Move(Some(Button::Left)), 1, 1, Click, Sgr), None);
        // drag: moves with a button held, as button + 32
        assert_eq!(report(Move(Some(Button::Left)), 1, 1, Drag, Sgr).unwrap(), b"\x1b[<32;2;2M");
        assert_eq!(report(Move(None), 1, 1, Drag, Sgr), None);
        // every move: with no button it is button 3 + 32
        assert_eq!(report(Move(None), 1, 1, Motion, Sgr).unwrap(), b"\x1b[<35;2;2M");
        assert_eq!(report(Move(Some(Button::Right)), 1, 1, Motion, Classic).unwrap(), b"\x1b[M\x42\x22\x22");
    }

    #[test]
    fn option_and_ctrl_are_added_to_the_button() {
        let alt_ctrl = Mods { alt: true, ctrl: true, ..Mods::NONE };
        assert_eq!(
            mouse_report(Press(Button::Left), 0, 0, alt_ctrl, Click, Sgr).unwrap(),
            b"\x1b[<24;1;1M",
            "8 for Option, 16 for Ctrl"
        );
        assert_eq!(mouse_report(WheelDown, 0, 0, Mods::CTRL, Click, Sgr).unwrap(), b"\x1b[<81;1;1M");
    }
}
