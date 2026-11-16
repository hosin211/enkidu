//! What a key press sends to the program in the terminal.
//!
//! By default keys are sent: xterm.js 6 with Option as Meta, and Shift+Enter as Esc and Enter, which Claude Code reads
//! as a new line. When the program turns on the kitty keyboard protocol (`CSI > flags u`, as Claude Code can), keys
//! follow <https://sw.kovidgoyal.net/kitty/keyboard-protocol/> for the flags it asked for, repeats and releases
//! included. Cmd shortcuts send nothing: they belong to the app (copy, paste, tabs).

/// The modifiers held with a key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    /// Option. It works as Meta: Option with a key sends Esc, then the key.
    pub alt: bool,
    pub ctrl: bool,
    /// Cmd: the app's shortcuts, never sent to the program.
    pub cmd: bool,
}

impl Mods {
    pub const NONE: Mods = Mods { shift: false, alt: false, ctrl: false, cmd: false };
    pub const SHIFT: Mods = Mods { shift: true, alt: false, ctrl: false, cmd: false };
    pub const ALT: Mods = Mods { shift: false, alt: true, ctrl: false, cmd: false };
    pub const CTRL: Mods = Mods { shift: false, alt: false, ctrl: true, cmd: false };

    /// The modifier number xterm and the kitty protocol put after a key: 1, plus 1 for Shift, 2 for Option (Alt),
    /// 4 for Ctrl.
    fn param(self) -> u8 {
        1 + u8::from(self.shift) + 2 * u8::from(self.alt) + 4 * u8::from(self.ctrl)
    }
}

/// A key, as the program cares about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Key {
    /// A key that types: `text` is what it types with Shift applied ("A"), `base` the key's own character in lower
    /// case ('a'), which Ctrl, Option as Meta and the kitty protocol use.
    Text {
        text: String,
        base: char,
    },
    Enter,
    Tab,
    Backspace,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    /// F1 to F12.
    F(u8),
}

impl Key {
    /// A typing key from one character, as a test or the app names it: `Key::text('a')`.
    pub fn text(c: char) -> Key {
        Key::Text { text: c.to_string(), base: c.to_ascii_lowercase() }
    }
}

/// Kitty keyboard protocol flags (the bits of `CSI > flags u`).
pub const KITTY_DISAMBIGUATE: u8 = 1;
pub const KITTY_EVENT_TYPES: u8 = 2;
pub const KITTY_ALTERNATE_KEYS: u8 = 4;
pub const KITTY_ALL_KEYS: u8 = 8;
pub const KITTY_ASSOCIATED_TEXT: u8 = 16;

/// The terminal modes that change what keys send.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyModes {
    /// DECCKM: arrows, Home and End send `ESC O A` instead of `ESC [ A`.
    pub app_cursor: bool,
    /// The kitty protocol's flags the program turned on; 0 while it's off.
    pub kitty: u8,
}

/// A key going down, repeating while held, or coming up. Only the kitty protocol's event types (flag 2) tell the
/// three apart; otherwise a repeat is one more press and a release sends nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeyAction {
    #[default]
    Press,
    Repeat,
    Release,
}

/// The bytes a key press sends, or None when it sends nothing.
pub fn encode_key(key: &Key, mods: Mods, modes: KeyModes) -> Option<Vec<u8>> {
    encode_key_action(key, mods, modes, KeyAction::Press)
}

/// The bytes a key press, repeat or release sends, or None when it sends nothing.
pub fn encode_key_action(key: &Key, mods: Mods, modes: KeyModes, action: KeyAction) -> Option<Vec<u8>> {
    if mods.cmd {
        return None;
    }
    if modes.kitty & (KITTY_DISAMBIGUATE | KITTY_ALL_KEYS) != 0 {
        return kitty(key, mods, modes, action);
    }
    match action {
        KeyAction::Release => None,
        KeyAction::Press | KeyAction::Repeat => legacy(key, mods, modes),
    }
}

/// xterm.js 6's table (`Keyboard.ts`), with Option as Meta, plus Shift+Enter.
fn legacy(key: &Key, mods: Mods, modes: KeyModes) -> Option<Vec<u8>> {
    let m = mods.param();
    let bytes = match key {
        // Shift+Enter (and Option+Enter) send Esc then Enter: Claude Code reads it as a new line in its prompt.
        Key::Enter if mods.shift || mods.alt => b"\x1b\r".to_vec(),
        Key::Enter => b"\r".to_vec(),
        Key::Tab if mods.shift => b"\x1b[Z".to_vec(),
        Key::Tab => b"\t".to_vec(),
        Key::Backspace => {
            let code = if mods.ctrl { 0x08 } else { 0x7f };
            if mods.alt { vec![0x1b, code] } else { vec![code] }
        }
        Key::Escape if mods.alt => b"\x1b\x1b".to_vec(),
        Key::Escape => vec![0x1b],
        Key::Up => cursor_key('A', m, modes.app_cursor),
        Key::Down => cursor_key('B', m, modes.app_cursor),
        Key::Right => cursor_key('C', m, modes.app_cursor),
        Key::Left => cursor_key('D', m, modes.app_cursor),
        Key::Home => cursor_key('H', m, modes.app_cursor),
        Key::End => cursor_key('F', m, modes.app_cursor),
        // Shift with Page Up and Page Down moves the view a page instead ([`crate::Terminal::key`]); Option alone
        // changes nothing.
        Key::PageUp | Key::PageDown if mods.shift => return None,
        Key::PageUp => tilde_key(5, if mods.ctrl { m } else { 1 }),
        Key::PageDown => tilde_key(6, if mods.ctrl { m } else { 1 }),
        // Shift or Ctrl with Insert copy and paste on some systems: nothing goes.
        Key::Insert if mods.shift || mods.ctrl => return None,
        Key::Insert => tilde_key(2, 1),
        Key::Delete => tilde_key(3, m),
        Key::F(n) => function_key(*n, m)?,
        Key::Text { text, base } => {
            if mods.ctrl
                && let Some(code) = ctrl_code(*base)
            {
                let mut out = if mods.alt { vec![0x1b] } else { Vec::new() };
                out.push(code);
                return Some(out);
            }
            if mods.alt {
                // Option as Meta: Esc, then the key's own character (upper case with Shift), never the character
                // the Mac would compose (Option+a is "å" on the US layout).
                let c = if mods.shift { base.to_ascii_uppercase() } else { *base };
                let mut out = vec![0x1b];
                out.extend(c.to_string().as_bytes());
                return Some(out);
            }
            if text.is_empty() {
                return None;
            }
            text.as_bytes().to_vec()
        }
    };
    Some(bytes)
}

fn cursor_key(letter: char, m: u8, app_cursor: bool) -> Vec<u8> {
    if m > 1 {
        format!("\x1b[1;{m}{letter}").into_bytes()
    } else if app_cursor {
        format!("\x1bO{letter}").into_bytes()
    } else {
        format!("\x1b[{letter}").into_bytes()
    }
}

fn tilde_key(number: u8, m: u8) -> Vec<u8> {
    if m > 1 { format!("\x1b[{number};{m}~") } else { format!("\x1b[{number}~") }.into_bytes()
}

fn function_key(n: u8, m: u8) -> Option<Vec<u8>> {
    let bytes = match n {
        1..=4 => {
            let letter = ['P', 'Q', 'R', 'S'][usize::from(n - 1)];
            if m > 1 { format!("\x1b[1;{m}{letter}") } else { format!("\x1bO{letter}") }.into_bytes()
        }
        5..=12 => tilde_key(f_number(n), m),
        _ => return None,
    };
    Some(bytes)
}

/// The number in `CSI number ~` for F5 to F12 (F3 too, in the kitty protocol).
fn f_number(n: u8) -> u8 {
    match n {
        3 => 13,
        5 => 15,
        6..=10 => n + 11,
        _ => n + 12,
    }
}

/// The control code Ctrl and a key send: Ctrl+C is 0x03, Ctrl+V 0x16, Ctrl+[ is Esc.
fn ctrl_code(base: char) -> Option<u8> {
    let c = base.to_ascii_lowercase();
    Some(match c {
        'a'..='z' => c as u8 - b'a' + 1,
        '@' | ' ' | '2' => 0,
        '[' | '3' => 0x1b,
        '\\' | '4' => 0x1c,
        ']' | '5' => 0x1d,
        '^' | '6' => 0x1e,
        '_' | '-' | '/' | '7' => 0x1f,
        '?' | '8' => 0x7f,
        _ => return None,
    })
}

/// The kitty protocol: keys that would be ambiguous become `CSI code ; modifiers u`, other keys keep their CSI form
/// with the modifiers (and the event type) added, text stays text unless the program asked for every key.
fn kitty(key: &Key, mods: Mods, modes: KeyModes, action: KeyAction) -> Option<Vec<u8>> {
    let all = modes.kitty & KITTY_ALL_KEYS != 0;
    let events = modes.kitty & KITTY_EVENT_TYPES != 0;
    let m = mods.param();
    let event = match action {
        KeyAction::Press => 1,
        KeyAction::Repeat if events => 2,
        KeyAction::Repeat => 1,
        KeyAction::Release if events => 3,
        KeyAction::Release => return None,
    };
    // `;modifiers` and `:event`, left out when they say nothing (unless something follows them)
    let tail = |keep: bool| -> String {
        match (m, event) {
            (1, 1) if !keep => String::new(),
            (m, 1) => format!(";{m}"),
            (m, e) => format!(";{m}:{e}"),
        }
    };
    // a key that keeps its legacy bytes: sent on press and repeat, never released
    let plain = |bytes: Vec<u8>| (event != 3).then_some(bytes);
    let csi_u = |code: u32| Some(format!("\x1b[{code}{}u", tail(false)).into_bytes());
    let tilde = |number: u8| Some(format!("\x1b[{number}{}~", tail(false)).into_bytes());
    // arrows, Home, End, F1, F2, F4: `CSI letter` alone, `CSI 1 ; modifiers letter` with more to say
    let letter = |letter: char, cursor: bool| -> Option<Vec<u8>> {
        let text = if m == 1 && event == 1 {
            if cursor && modes.app_cursor { format!("\x1bO{letter}") } else { format!("\x1b[{letter}") }
        } else {
            format!("\x1b[1{}{letter}", tail(true))
        };
        Some(text.into_bytes())
    };
    match key {
        Key::Enter if all || m > 1 => csi_u(13),
        Key::Enter => plain(b"\r".to_vec()),
        Key::Tab if all || m > 1 => csi_u(9),
        Key::Tab => plain(b"\t".to_vec()),
        Key::Backspace if all || m > 1 => csi_u(127),
        Key::Backspace => plain(vec![0x7f]),
        Key::Escape => csi_u(27),
        Key::Up => letter('A', true),
        Key::Down => letter('B', true),
        Key::Right => letter('C', true),
        Key::Left => letter('D', true),
        Key::Home => letter('H', true),
        Key::End => letter('F', true),
        Key::PageUp => tilde(5),
        Key::PageDown => tilde(6),
        Key::Insert => tilde(2),
        Key::Delete => tilde(3),
        // F3 is `CSI 13 ~`: `CSI 1 ; m R` would read as a cursor position report.
        Key::F(3) => tilde(13),
        Key::F(n @ (1 | 2 | 4)) => letter(['P', 'Q', 'R', 'S'][usize::from(n - 1)], false),
        Key::F(n @ 5..=12) => tilde(f_number(*n)),
        Key::F(_) => None,
        Key::Text { text, base } => {
            // Text with no modifier, or Shift alone, is just text, unless the program asked for every key.
            if !all && !mods.ctrl && !mods.alt {
                return if text.is_empty() { None } else { plain(text.as_bytes().to_vec()) };
            }
            let code = base.to_ascii_lowercase() as u32;
            let mut out = format!("\x1b[{code}");
            if modes.kitty & KITTY_ALTERNATE_KEYS != 0
                && mods.shift
                && let Some(shifted) =
                    text.chars().next().filter(|c| c.to_ascii_lowercase() as u32 == code && *c as u32 != code)
            {
                out.push_str(&format!(":{}", shifted as u32));
            }
            let with_text = all
                && modes.kitty & KITTY_ASSOCIATED_TEXT != 0
                && !mods.ctrl
                && !mods.alt
                && !text.is_empty()
                && event != 3;
            out.push_str(&tail(with_text));
            if with_text {
                let points: Vec<String> = text.chars().map(|c| (c as u32).to_string()).collect();
                out.push_str(&format!(";{}", points.join(":")));
            }
            out.push('u');
            Some(out.into_bytes())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use KeyAction::{Press, Release, Repeat};

    const NONE: Mods = Mods::NONE;
    const SHIFT: Mods = Mods::SHIFT;
    const ALT: Mods = Mods::ALT;
    const CTRL: Mods = Mods::CTRL;
    const SHIFT_ALT: Mods = Mods { shift: true, alt: true, ctrl: false, cmd: false };
    const CTRL_ALT: Mods = Mods { shift: false, alt: true, ctrl: true, cmd: false };
    const CTRL_SHIFT: Mods = Mods { shift: true, alt: false, ctrl: true, cmd: false };

    fn text(t: &str, base: char) -> Key {
        Key::Text { text: t.into(), base }
    }

    /// Checks a table of keys against the bytes they must send in `modes`.
    fn table(modes: KeyModes, rows: &[(Key, Mods, KeyAction, Option<&[u8]>)]) {
        for (key, mods, action, want) in rows {
            let got = encode_key_action(key, *mods, modes, *action);
            assert_eq!(
                got.as_deref(),
                *want,
                "{key:?} with {mods:?}, {action:?}, in {modes:?}: got {:?}, want {:?}",
                got.as_deref().map(String::from_utf8_lossy),
                want.map(String::from_utf8_lossy)
            );
        }
    }

    #[test]
    fn every_key_goes_out_as_its_bytes() {
        let rows: &[(Key, Mods, KeyAction, Option<&[u8]>)] = &[
            // Enter: Shift and Option make a new line in Claude's prompt
            (Key::Enter, NONE, Press, Some(b"\r")),
            (Key::Enter, SHIFT, Press, Some(b"\x1b\r")),
            (Key::Enter, ALT, Press, Some(b"\x1b\r")),
            (Key::Enter, CTRL, Press, Some(b"\r")),
            (Key::Enter, CTRL_ALT, Press, Some(b"\x1b\r")),
            (Key::Tab, NONE, Press, Some(b"\t")),
            (Key::Tab, SHIFT, Press, Some(b"\x1b[Z")),
            (Key::Tab, ALT, Press, Some(b"\t")),
            (Key::Tab, SHIFT_ALT, Press, Some(b"\x1b[Z")),
            (Key::Backspace, NONE, Press, Some(b"\x7f")),
            (Key::Backspace, SHIFT, Press, Some(b"\x7f")),
            (Key::Backspace, ALT, Press, Some(b"\x1b\x7f")),
            (Key::Backspace, CTRL, Press, Some(b"\x08")),
            (Key::Backspace, CTRL_ALT, Press, Some(b"\x1b\x08")),
            (Key::Escape, NONE, Press, Some(b"\x1b")),
            (Key::Escape, ALT, Press, Some(b"\x1b\x1b")),
            (Key::Escape, CTRL, Press, Some(b"\x1b")),
            // arrows, Home and End: CSI 1 ; modifiers with any modifier
            (Key::Up, NONE, Press, Some(b"\x1b[A")),
            (Key::Down, NONE, Press, Some(b"\x1b[B")),
            (Key::Right, NONE, Press, Some(b"\x1b[C")),
            (Key::Left, NONE, Press, Some(b"\x1b[D")),
            (Key::Left, SHIFT, Press, Some(b"\x1b[1;2D")),
            (Key::Left, ALT, Press, Some(b"\x1b[1;3D")),
            (Key::Right, SHIFT_ALT, Press, Some(b"\x1b[1;4C")),
            (Key::Up, CTRL, Press, Some(b"\x1b[1;5A")),
            (Key::Down, CTRL_SHIFT, Press, Some(b"\x1b[1;6B")),
            (Key::Left, CTRL_ALT, Press, Some(b"\x1b[1;7D")),
            (Key::Home, NONE, Press, Some(b"\x1b[H")),
            (Key::End, NONE, Press, Some(b"\x1b[F")),
            (Key::Home, SHIFT, Press, Some(b"\x1b[1;2H")),
            (Key::End, CTRL, Press, Some(b"\x1b[1;5F")),
            // Page Up and Down: Ctrl adds the modifiers, Option alone nothing, Shift moves the view instead
            (Key::PageUp, NONE, Press, Some(b"\x1b[5~")),
            (Key::PageDown, NONE, Press, Some(b"\x1b[6~")),
            (Key::PageUp, ALT, Press, Some(b"\x1b[5~")),
            (Key::PageDown, CTRL, Press, Some(b"\x1b[6;5~")),
            (Key::PageUp, CTRL_ALT, Press, Some(b"\x1b[5;7~")),
            (Key::PageUp, SHIFT, Press, None),
            (Key::PageDown, CTRL_SHIFT, Press, None),
            (Key::Insert, NONE, Press, Some(b"\x1b[2~")),
            (Key::Insert, ALT, Press, Some(b"\x1b[2~")),
            (Key::Insert, SHIFT, Press, None),
            (Key::Insert, CTRL, Press, None),
            (Key::Delete, NONE, Press, Some(b"\x1b[3~")),
            (Key::Delete, SHIFT, Press, Some(b"\x1b[3;2~")),
            (Key::Delete, ALT, Press, Some(b"\x1b[3;3~")),
            (Key::Delete, CTRL, Press, Some(b"\x1b[3;5~")),
            // function keys
            (Key::F(1), NONE, Press, Some(b"\x1bOP")),
            (Key::F(2), NONE, Press, Some(b"\x1bOQ")),
            (Key::F(3), NONE, Press, Some(b"\x1bOR")),
            (Key::F(4), NONE, Press, Some(b"\x1bOS")),
            (Key::F(1), SHIFT, Press, Some(b"\x1b[1;2P")),
            (Key::F(4), CTRL, Press, Some(b"\x1b[1;5S")),
            (Key::F(5), NONE, Press, Some(b"\x1b[15~")),
            (Key::F(6), NONE, Press, Some(b"\x1b[17~")),
            (Key::F(7), NONE, Press, Some(b"\x1b[18~")),
            (Key::F(8), NONE, Press, Some(b"\x1b[19~")),
            (Key::F(9), NONE, Press, Some(b"\x1b[20~")),
            (Key::F(10), NONE, Press, Some(b"\x1b[21~")),
            (Key::F(11), NONE, Press, Some(b"\x1b[23~")),
            (Key::F(12), NONE, Press, Some(b"\x1b[24~")),
            (Key::F(5), SHIFT, Press, Some(b"\x1b[15;2~")),
            (Key::F(12), CTRL_ALT, Press, Some(b"\x1b[24;7~")),
            (Key::F(13), NONE, Press, None),
            // text, and Ctrl with it: Ctrl+C stops, Ctrl+V lets Claude Code read an image from the clipboard
            (text("a", 'a'), NONE, Press, Some(b"a")),
            (text("A", 'a'), SHIFT, Press, Some(b"A")),
            (text("ب", 'f'), NONE, Press, Some("ب".as_bytes())),
            (text("é", 'e'), NONE, Press, Some("é".as_bytes())),
            (text("c", 'c'), CTRL, Press, Some(b"\x03")),
            (text("v", 'v'), CTRL, Press, Some(b"\x16")),
            (text("d", 'd'), CTRL, Press, Some(b"\x04")),
            (text("C", 'c'), CTRL_SHIFT, Press, Some(b"\x03")),
            (text(" ", ' '), CTRL, Press, Some(b"\x00")),
            (text("2", '2'), CTRL, Press, Some(b"\x00")),
            (text("[", '['), CTRL, Press, Some(b"\x1b")),
            (text("3", '3'), CTRL, Press, Some(b"\x1b")),
            (text("\\", '\\'), CTRL, Press, Some(b"\x1c")),
            (text("]", ']'), CTRL, Press, Some(b"\x1d")),
            (text("6", '6'), CTRL, Press, Some(b"\x1e")),
            (text("_", '_'), CTRL, Press, Some(b"\x1f")),
            (text("-", '-'), CTRL, Press, Some(b"\x1f")),
            (text("/", '/'), CTRL, Press, Some(b"\x1f")),
            (text("8", '8'), CTRL, Press, Some(b"\x7f")),
            // Option as Meta: Esc and the key's own character, never what the Mac composes
            (text("å", 'a'), ALT, Press, Some(b"\x1ba")),
            (text("∫", 'b'), ALT, Press, Some(b"\x1bb")),
            (text("ƒ", 'f'), ALT, Press, Some(b"\x1bf")),
            (text("Å", 'a'), SHIFT_ALT, Press, Some(b"\x1bA")),
            (text("≥", '.'), ALT, Press, Some(b"\x1b.")),
            (text("!", '!'), ALT, Press, Some(b"\x1b!")),
            (text(" ", ' '), ALT, Press, Some(b"\x1b ")),
            (text("c", 'c'), CTRL_ALT, Press, Some(b"\x1b\x03")),
            (text(" ", ' '), CTRL_ALT, Press, Some(b"\x1b\x00")),
            // Cmd is the app's
            (text("c", 'c'), Mods { cmd: true, ..NONE }, Press, None),
            (Key::Enter, Mods { cmd: true, ..NONE }, Press, None),
            // a repeat is one more press, a release sends nothing
            (text("a", 'a'), NONE, Repeat, Some(b"a")),
            (Key::Up, NONE, Repeat, Some(b"\x1b[A")),
            (text("a", 'a'), NONE, Release, None),
            (Key::Escape, NONE, Release, None),
        ];
        table(KeyModes::default(), rows);
    }

    #[test]
    fn application_cursor_keys_send_ss3_without_modifiers() {
        let app = KeyModes { app_cursor: true, kitty: 0 };
        table(
            app,
            &[
                (Key::Up, NONE, Press, Some(b"\x1bOA")),
                (Key::Down, NONE, Press, Some(b"\x1bOB")),
                (Key::Right, NONE, Press, Some(b"\x1bOC")),
                (Key::Left, NONE, Press, Some(b"\x1bOD")),
                (Key::Home, NONE, Press, Some(b"\x1bOH")),
                (Key::End, NONE, Press, Some(b"\x1bOF")),
                (Key::Up, SHIFT, Press, Some(b"\x1b[1;2A")),
                (Key::Left, ALT, Press, Some(b"\x1b[1;3D")),
                (Key::PageUp, NONE, Press, Some(b"\x1b[5~")),
            ],
        );
    }

    #[test]
    fn with_the_kitty_protocol_on_ambiguous_keys_become_csi_u() {
        let modes = KeyModes { app_cursor: false, kitty: KITTY_DISAMBIGUATE };
        table(
            modes,
            &[
                (Key::Enter, NONE, Press, Some(b"\r")),
                (Key::Enter, SHIFT, Press, Some(b"\x1b[13;2u")),
                (Key::Enter, ALT, Press, Some(b"\x1b[13;3u")),
                (Key::Enter, CTRL, Press, Some(b"\x1b[13;5u")),
                (Key::Tab, NONE, Press, Some(b"\t")),
                (Key::Tab, SHIFT, Press, Some(b"\x1b[9;2u")),
                (Key::Backspace, NONE, Press, Some(b"\x7f")),
                (Key::Backspace, ALT, Press, Some(b"\x1b[127;3u")),
                (Key::Backspace, CTRL, Press, Some(b"\x1b[127;5u")),
                (Key::Escape, NONE, Press, Some(b"\x1b[27u")),
                (Key::Escape, SHIFT, Press, Some(b"\x1b[27;2u")),
                (Key::Up, NONE, Press, Some(b"\x1b[A")),
                (Key::Up, SHIFT, Press, Some(b"\x1b[1;2A")),
                (Key::Home, NONE, Press, Some(b"\x1b[H")),
                (Key::End, CTRL, Press, Some(b"\x1b[1;5F")),
                (Key::PageUp, NONE, Press, Some(b"\x1b[5~")),
                (Key::PageUp, ALT, Press, Some(b"\x1b[5;3~")),
                (Key::PageDown, CTRL, Press, Some(b"\x1b[6;5~")),
                (Key::Insert, NONE, Press, Some(b"\x1b[2~")),
                (Key::Delete, SHIFT, Press, Some(b"\x1b[3;2~")),
                (Key::F(1), NONE, Press, Some(b"\x1b[P")),
                (Key::F(2), NONE, Press, Some(b"\x1b[Q")),
                (Key::F(3), NONE, Press, Some(b"\x1b[13~")),
                (Key::F(4), NONE, Press, Some(b"\x1b[S")),
                (Key::F(1), SHIFT, Press, Some(b"\x1b[1;2P")),
                (Key::F(3), SHIFT, Press, Some(b"\x1b[13;2~")),
                (Key::F(5), NONE, Press, Some(b"\x1b[15~")),
                (Key::F(12), ALT, Press, Some(b"\x1b[24;3~")),
                (text("a", 'a'), NONE, Press, Some(b"a")),
                (text("A", 'a'), SHIFT, Press, Some(b"A")),
                (text("é", 'e'), NONE, Press, Some("é".as_bytes())),
                (text("c", 'c'), CTRL, Press, Some(b"\x1b[99;5u")),
                (text("v", 'v'), CTRL, Press, Some(b"\x1b[118;5u")),
                (text("å", 'a'), ALT, Press, Some(b"\x1b[97;3u")),
                (text("Å", 'a'), SHIFT_ALT, Press, Some(b"\x1b[97;4u")),
                (text("C", 'c'), CTRL_SHIFT, Press, Some(b"\x1b[99;6u")),
                (text("c", 'c'), CTRL_ALT, Press, Some(b"\x1b[99;7u")),
                (text(" ", ' '), CTRL, Press, Some(b"\x1b[32;5u")),
                // without event types: a repeat is a press, a release nothing
                (text("c", 'c'), CTRL, Repeat, Some(b"\x1b[99;5u")),
                (text("c", 'c'), CTRL, Release, None),
            ],
        );
    }

    #[test]
    fn with_event_types_repeats_and_releases_are_told_apart() {
        let modes = KeyModes { app_cursor: false, kitty: KITTY_DISAMBIGUATE | KITTY_EVENT_TYPES };
        table(
            modes,
            &[
                (text("c", 'c'), CTRL, Press, Some(b"\x1b[99;5u")),
                (text("c", 'c'), CTRL, Repeat, Some(b"\x1b[99;5:2u")),
                (text("c", 'c'), CTRL, Release, Some(b"\x1b[99;5:3u")),
                (Key::Escape, NONE, Release, Some(b"\x1b[27;1:3u")),
                (Key::Enter, SHIFT, Release, Some(b"\x1b[13;2:3u")),
                (Key::Up, NONE, Repeat, Some(b"\x1b[1;1:2A")),
                (Key::Up, NONE, Release, Some(b"\x1b[1;1:3A")),
                (Key::PageUp, NONE, Release, Some(b"\x1b[5;1:3~")),
                (Key::F(1), NONE, Release, Some(b"\x1b[1;1:3P")),
                // keys sent as text or legacy bytes: never released
                (text("a", 'a'), NONE, Repeat, Some(b"a")),
                (text("a", 'a'), NONE, Release, None),
                (Key::Enter, NONE, Release, None),
                (Key::Backspace, NONE, Repeat, Some(b"\x7f")),
            ],
        );
    }

    #[test]
    fn with_every_key_reported_text_keys_are_codes_too() {
        let all = KeyModes { app_cursor: false, kitty: KITTY_DISAMBIGUATE | KITTY_ALL_KEYS };
        table(
            all,
            &[
                (text("a", 'a'), NONE, Press, Some(b"\x1b[97u")),
                (text("A", 'a'), SHIFT, Press, Some(b"\x1b[97;2u")),
                (Key::Enter, NONE, Press, Some(b"\x1b[13u")),
                (Key::Tab, NONE, Press, Some(b"\x1b[9u")),
                (Key::Backspace, NONE, Press, Some(b"\x1b[127u")),
                (text("a", 'a'), NONE, Release, None),
            ],
        );
        let with_events =
            KeyModes { app_cursor: false, kitty: KITTY_DISAMBIGUATE | KITTY_ALL_KEYS | KITTY_EVENT_TYPES };
        table(with_events, &[(text("a", 'a'), NONE, Release, Some(b"\x1b[97;1:3u"))]);
        let with_text = KeyModes {
            app_cursor: false,
            kitty: KITTY_DISAMBIGUATE | KITTY_ALL_KEYS | KITTY_ALTERNATE_KEYS | KITTY_ASSOCIATED_TEXT,
        };
        table(
            with_text,
            &[
                (text("A", 'a'), SHIFT, Press, Some(b"\x1b[97:65;2;65u")),
                (text("a", 'a'), NONE, Press, Some(b"\x1b[97;1;97u")),
                (text("c", 'c'), CTRL, Press, Some(b"\x1b[99;5u")),
            ],
        );
    }

    #[test]
    fn the_kitty_protocol_keeps_application_cursor_keys() {
        let modes = KeyModes { app_cursor: true, kitty: KITTY_DISAMBIGUATE };
        table(
            modes,
            &[
                (Key::Up, NONE, Press, Some(b"\x1bOA")),
                (Key::Up, CTRL, Press, Some(b"\x1b[1;5A")),
                // the function keys never were cursor keys
                (Key::F(1), NONE, Press, Some(b"\x1b[P")),
            ],
        );
    }
}
