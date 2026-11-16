//! From a GPUI key press to the key the terminal sends (`enkidu_term::keys`).

use enkidu_term::{Key, Mods};
use gpui_kit::Keystroke;

/// The terminal key and modifiers for a key press, or None for a key the terminal ignores (a lone modifier).
pub fn terminal_key(keystroke: &Keystroke) -> Option<(Key, Mods)> {
    let m = &keystroke.modifiers;
    let mods = Mods { shift: m.shift, alt: m.alt, ctrl: m.control, cmd: m.platform };
    let key = match keystroke.key.as_str() {
        "enter" => Key::Enter,
        "tab" => Key::Tab,
        "backspace" => Key::Backspace,
        "escape" => Key::Escape,
        "up" => Key::Up,
        "down" => Key::Down,
        "left" => Key::Left,
        "right" => Key::Right,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        "insert" => Key::Insert,
        "delete" => Key::Delete,
        "space" => Key::Text { text: keystroke.key_char.clone().unwrap_or_else(|| " ".into()), base: ' ' },
        name if name.len() > 1 && name.starts_with('f') && name[1..].parse::<u8>().is_ok() => {
            Key::F(name[1..].parse().ok()?)
        }
        name => {
            let mut chars = name.chars();
            let base = chars.next()?;
            if chars.next().is_some() {
                // A named key the terminal doesn't send (a lone modifier, media keys): type its text, if any.
                let text = keystroke.key_char.clone()?;
                let first = text.chars().next()?;
                return Some((Key::Text { text, base: first }, mods));
            }
            let text = keystroke
                .key_char
                .clone()
                .unwrap_or_else(|| if m.shift { base.to_uppercase().collect() } else { base.to_string() });
            Key::Text { text, base: base.to_ascii_lowercase() }
        }
    };
    Some((key, mods))
}

#[cfg(test)]
mod tests {
    use super::*;
    use enkidu_term::{KeyModes, encode_key};

    fn bytes(spec: &'static str) -> Option<Vec<u8>> {
        let keystroke = Keystroke::parse(spec).expect("a key spec");
        let (key, mods) = terminal_key(&keystroke)?;
        encode_key(&key, mods, KeyModes::default())
    }

    #[test]
    fn gpui_key_presses_become_the_bytes_claude_code_reads() {
        assert_eq!(bytes("enter").unwrap(), b"\r");
        assert_eq!(bytes("shift-enter").unwrap(), b"\x1b\r");
        assert_eq!(bytes("escape").unwrap(), b"\x1b");
        assert_eq!(bytes("ctrl-c").unwrap(), b"\x03");
        assert_eq!(bytes("ctrl-v").unwrap(), b"\x16");
        assert_eq!(bytes("up").unwrap(), b"\x1b[A");
        assert_eq!(bytes("alt-b").unwrap(), b"\x1bb");
        assert_eq!(bytes("a").unwrap(), b"a");
        assert_eq!(bytes("f5").unwrap(), b"\x1b[15~");
        assert_eq!(bytes("cmd-c"), None);
    }

    #[test]
    fn every_key_the_mac_names_reaches_the_terminal() {
        let rows: [(&'static str, &[u8]); 30] = [
            ("tab", b"\t"),
            ("shift-tab", b"\x1b[Z"),
            ("backspace", b"\x7f"),
            ("ctrl-backspace", b"\x08"),
            ("alt-backspace", b"\x1b\x7f"),
            ("alt-enter", b"\x1b\r"),
            ("alt-escape", b"\x1b\x1b"),
            ("down", b"\x1b[B"),
            ("left", b"\x1b[D"),
            ("right", b"\x1b[C"),
            ("alt-left", b"\x1b[1;3D"),
            ("shift-up", b"\x1b[1;2A"),
            ("home", b"\x1b[H"),
            ("end", b"\x1b[F"),
            ("pageup", b"\x1b[5~"),
            ("pagedown", b"\x1b[6~"),
            ("insert", b"\x1b[2~"),
            ("delete", b"\x1b[3~"),
            ("f1", b"\x1bOP"),
            ("f4", b"\x1bOS"),
            ("f12", b"\x1b[24~"),
            ("shift-f12", b"\x1b[24;2~"),
            ("space", b" "),
            ("ctrl-space", b"\x00"),
            ("alt-space", b"\x1b "),
            ("ctrl-d", b"\x04"),
            ("ctrl-alt-c", b"\x1b\x03"),
            ("alt-shift-b", b"\x1bB"),
            ("ctrl-]", b"\x1d"),
            ("1", b"1"),
        ];
        for (spec, want) in rows {
            assert_eq!(bytes(spec).as_deref(), Some(want), "{spec}");
        }
        assert_eq!(bytes("f13"), None, "F13 and up send nothing");
        assert_eq!(bytes("shift"), None, "a lone modifier sends nothing");
    }
}
