//! The terminal behind Enkidu's terminal view. No GPUI in here, so all of it is tested without a window.
//!
//! The engine (`alacritty_terminal`) sits behind this crate's own interface, and none of its types cross it, so it
//! can be swapped:
//!
//! - **bytes in:** from the terminal's own pty ([`Terminal::spawn`]), from a program that runs elsewhere through a
//!   [`Feed`] ([`Terminal::attached`], the core's terminals), or by hand ([`Terminal::kept`], tests and pictures);
//! - **out:** the screen as rows of cells, with the cursor and the modes ([`Screen`]), brought up to date once a
//!   frame with only the rows that changed ([`Terminal::update`], [`Damage`]); the title, the clipboard;
//! - **in:** keys ([`Terminal::key`]), the mouse and the wheel ([`Terminal::mouse`], [`Terminal::wheel`]), paste
//!   ([`Terminal::paste`]) and focus ([`Terminal::focus`]), each as the program's modes want it; what is typed goes
//!   to the program on a writer thread ([`Program`]), never on the window's.

pub mod keys;
pub mod links;
pub mod mouse;
pub mod palette;
pub mod paste;
pub mod screen;
pub mod session;
pub mod wheel;

pub use keys::{Key, KeyAction, KeyModes, Mods, encode_key, encode_key_action};
pub use links::Link;
pub use mouse::{Button, MouseAction, MouseEncoding, MouseMode, mouse_report};
pub use palette::{Palette, Rgb};
pub use paste::{dropped_paths, paste_bytes};
pub use screen::{Cell, Cursor, CursorKind, Damage, Modes, Screen};
pub use session::{Feed, GridSize, Program, SelectKind, Spawn, Terminal, Wake, base_env};
pub use wheel::{FAST_SCROLL, WheelDelta, wheel_lines, wheel_speed};
