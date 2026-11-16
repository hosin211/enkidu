//! The terminal's colors: the sixteen of the app's own theme (tuned to the window's lamps), xterm's 256, and true
//! color.

use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, NamedColor};

/// A color in 8-bit red, green and blue.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub const fn hex(value: u32) -> Rgb {
        Rgb((value >> 16) as u8, (value >> 8) as u8, value as u8)
    }

    /// This color moved toward `other` by `amount` (0 keeps it, 1 is `other`).
    pub fn mix(self, other: Rgb, amount: f32) -> Rgb {
        let blend = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * amount).round() as u8;
        Rgb(blend(self.0, other.0), blend(self.1, other.1), blend(self.2, other.2))
    }
}

/// The engine's color as ours (kept private, so no engine type crosses this crate's interface).
fn from_engine(c: alacritty_terminal::vte::ansi::Rgb) -> Rgb {
    Rgb(c.r, c.g, c.b)
}

/// The colors a terminal draws with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Palette {
    pub foreground: Rgb,
    pub background: Rgb,
    pub cursor: Rgb,
    /// Black, red, green, yellow, blue, magenta, cyan, white, then their bright forms.
    pub ansi: [Rgb; 16],
}

impl Palette {
    /// The app's terminal theme: Claude's output reads like the rest of the window.
    pub fn enkidu() -> Palette {
        Palette {
            foreground: Rgb::hex(0xe4e6eb),
            background: Rgb::hex(0x111217),
            cursor: Rgb::hex(0x7aa2ff),
            ansi: [
                Rgb::hex(0x1e2027),
                Rgb::hex(0xff6369),
                Rgb::hex(0x3dd68c),
                Rgb::hex(0xffb547),
                Rgb::hex(0x6f9dff),
                Rgb::hex(0xc59bff),
                Rgb::hex(0x5cd6e8),
                Rgb::hex(0xd5d8de),
                Rgb::hex(0x6c717d),
                Rgb::hex(0xff8d92),
                Rgb::hex(0x6fe3a8),
                Rgb::hex(0xffcb7a),
                Rgb::hex(0x9cbcff),
                Rgb::hex(0xdcbdfb),
                Rgb::hex(0x8de8f4),
                Rgb::hex(0xffffff),
            ],
        }
    }

    /// One of xterm's 256 colors: the sixteen, then a 6x6x6 cube, then 24 grays.
    pub fn indexed(&self, index: u8) -> Rgb {
        match index {
            0..=15 => self.ansi[usize::from(index)],
            16..=231 => {
                let i = index - 16;
                let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
                Rgb(level(i / 36), level((i / 6) % 6), level(i % 6))
            }
            _ => {
                let gray = 8 + (index - 232) * 10;
                Rgb(gray, gray, gray)
            }
        }
    }

    /// The color slot `index` as alacritty numbers them (0 to 255, then foreground 256, background 257, cursor 258),
    /// for replies to a program that asks for a color.
    pub fn slot(&self, index: usize) -> Rgb {
        match index {
            0..=255 => self.indexed(index as u8),
            i if i == NamedColor::Background as usize => self.background,
            i if i == NamedColor::Cursor as usize => self.cursor,
            _ => self.foreground,
        }
    }

    fn named(&self, color: NamedColor) -> Rgb {
        let dim = |c: Rgb| c.mix(self.background, 0.34);
        match color {
            NamedColor::Foreground | NamedColor::BrightForeground => self.foreground,
            NamedColor::Background => self.background,
            NamedColor::Cursor => self.cursor,
            NamedColor::DimForeground => dim(self.foreground),
            NamedColor::DimBlack => dim(self.ansi[0]),
            NamedColor::DimRed => dim(self.ansi[1]),
            NamedColor::DimGreen => dim(self.ansi[2]),
            NamedColor::DimYellow => dim(self.ansi[3]),
            NamedColor::DimBlue => dim(self.ansi[4]),
            NamedColor::DimMagenta => dim(self.ansi[5]),
            NamedColor::DimCyan => dim(self.ansi[6]),
            NamedColor::DimWhite => dim(self.ansi[7]),
            other => self.ansi[(other as usize).min(15)],
        }
    }

    /// The color a cell asks for, after any color the program set itself (OSC 4, 10, 11).
    pub(crate) fn resolve(&self, color: Color, overrides: &Colors) -> Rgb {
        match color {
            Color::Spec(rgb) => from_engine(rgb),
            Color::Indexed(i) => overrides[usize::from(i)].map(from_engine).unwrap_or_else(|| self.indexed(i)),
            Color::Named(named) => overrides[named].map(from_engine).unwrap_or_else(|| self.named(named)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_256_colors_follow_xterm() {
        let p = Palette::enkidu();
        assert_eq!(p.indexed(1), Rgb::hex(0xff6369));
        assert_eq!(p.indexed(16), Rgb(0, 0, 0));
        assert_eq!(p.indexed(196), Rgb(255, 0, 0));
        assert_eq!(p.indexed(231), Rgb(255, 255, 255));
        assert_eq!(p.indexed(232), Rgb(8, 8, 8));
        assert_eq!(p.indexed(255), Rgb(238, 238, 238));
    }
}
