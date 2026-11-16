//! A copy of what the terminal shows. It is taken under the terminal's lock in a moment (a few thousand cells) and
//! drawn without the lock, so the parser never waits for drawing. After the first copy only the rows that changed
//! are copied again, and the window hears which ones ([`Damage`]).

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::selection::SelectionRange;
use alacritty_terminal::term::cell::{Cell as EngineCell, Flags};
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor};

use crate::mouse::{MouseEncoding, MouseMode};
use crate::palette::{Palette, Rgb};

/// One cell as it is drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    pub ch: char,
    /// Zero-width characters drawn on top of `ch` (combining marks, variation selectors).
    pub extra: Option<Box<[char]>>,
    pub fg: Rgb,
    pub bg: Rgb,
    /// The background is the terminal's own, so nothing needs painting under the cell.
    pub default_bg: bool,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    /// A wide character (CJK, most emoji) that takes this cell and the next.
    pub wide: bool,
    /// The second half of a wide character: nothing of its own to draw.
    pub spacer: bool,
    pub selected: bool,
}

impl Cell {
    fn blank(palette: &Palette) -> Cell {
        Cell {
            ch: ' ',
            extra: None,
            fg: palette.foreground,
            bg: palette.background,
            default_bg: true,
            bold: false,
            italic: false,
            underline: false,
            strike: false,
            wide: false,
            spacer: false,
            selected: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorKind {
    Block,
    Bar,
    Underline,
    HollowBlock,
}

/// Where the cursor shows, in the rows and columns on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub row: usize,
    pub col: usize,
    pub kind: CursorKind,
    /// It sits on a wide character, so it is two cells wide.
    pub wide: bool,
    /// It blinks while the terminal has the keyboard, until a program asks for a steady cursor (`CSI 2 q`, `4 q`, `6
    /// q`).
    pub blinking: bool,
}

/// The modes the program turned on that the window acts on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modes {
    /// DECCKM: the arrows send `ESC O A` instead of `ESC [ A`.
    pub app_cursor: bool,
    pub bracketed_paste: bool,
    /// What the program wants to hear from the mouse. While it listens, the mouse is the program's (Shift keeps it
    /// the window's, to select text).
    pub mouse: MouseMode,
    pub mouse_encoding: MouseEncoding,
    pub alt_screen: bool,
    /// In the alternate screen without the mouse, the wheel becomes arrow keys.
    pub alternate_scroll: bool,
    /// The program wants to hear when the window gains and loses the keyboard (`CSI ? 1004 h`).
    pub focus_reports: bool,
    /// The kitty keyboard protocol's flags, 0 while it's off.
    pub kitty: u8,
}

impl Modes {
    pub(crate) fn of(mode: TermMode) -> Modes {
        let kitty = [
            (TermMode::DISAMBIGUATE_ESC_CODES, 1),
            (TermMode::REPORT_EVENT_TYPES, 2),
            (TermMode::REPORT_ALTERNATE_KEYS, 4),
            (TermMode::REPORT_ALL_KEYS_AS_ESC, 8),
            (TermMode::REPORT_ASSOCIATED_TEXT, 16),
        ]
        .iter()
        .filter(|(flag, _)| mode.contains(*flag))
        .fold(0u8, |bits, (_, bit)| bits | bit);
        let mouse = if mode.contains(TermMode::MOUSE_MOTION) {
            MouseMode::Motion
        } else if mode.contains(TermMode::MOUSE_DRAG) {
            MouseMode::Drag
        } else if mode.contains(TermMode::MOUSE_REPORT_CLICK) {
            MouseMode::Click
        } else {
            MouseMode::Off
        };
        let mouse_encoding = if mode.contains(TermMode::SGR_MOUSE) {
            MouseEncoding::Sgr
        } else if mode.contains(TermMode::UTF8_MOUSE) {
            MouseEncoding::Utf8
        } else {
            MouseEncoding::Classic
        };
        Modes {
            app_cursor: mode.contains(TermMode::APP_CURSOR),
            bracketed_paste: mode.contains(TermMode::BRACKETED_PASTE),
            mouse,
            mouse_encoding,
            alt_screen: mode.contains(TermMode::ALT_SCREEN),
            alternate_scroll: mode.contains(TermMode::ALTERNATE_SCROLL),
            focus_reports: mode.contains(TermMode::FOCUS_IN_OUT),
            kitty,
        }
    }
}

/// Which rows of the screen changed since the last copy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Damage {
    /// All of them: the first copy, a new size, text that moved (output scrolled it, or the view moved through the
    /// history), a new selection, new colors.
    Full,
    /// Only these rows, top to bottom. None at all: nothing changed.
    Rows(Vec<usize>),
}

impl Damage {
    /// Nothing changed.
    pub fn is_empty(&self) -> bool {
        matches!(self, Damage::Rows(rows) if rows.is_empty())
    }

    /// Whether `row` changed.
    pub fn has(&self, row: usize) -> bool {
        match self {
            Damage::Full => true,
            Damage::Rows(rows) => rows.binary_search(&row).is_ok(),
        }
    }
}

/// What the terminal shows right now.
#[derive(Clone, Debug, Default)]
pub struct Screen {
    pub cols: usize,
    pub rows: usize,
    /// Row after row, `cols` cells each.
    pub cells: Vec<Cell>,
    pub cursor: Option<Cursor>,
    /// The cursor's cell, also while the program hides the cursor (Claude Code hides it and keeps it at its prompt):
    /// where an input method's composition shows.
    pub cursor_cell: Option<(usize, usize)>,
    /// How many lines back in the history the view is (0: at the bottom, following the output).
    pub display_offset: usize,
    /// Lines of history above the screen.
    pub history: usize,
    pub modes: Modes,
}

impl Screen {
    /// Copies everything `term` shows. Call it with the terminal locked.
    pub(crate) fn capture<T: EventListener>(term: &Term<T>, palette: &Palette) -> Screen {
        let mut screen = Screen::default();
        screen.refresh(term, palette, Damage::Full);
        screen
    }

    /// Copies the rows `damage` names again (all of them when the size changed), and the cursor and the modes. Call
    /// it with the terminal locked. Returns the damage it applied.
    pub(crate) fn refresh<T: EventListener>(&mut self, term: &Term<T>, palette: &Palette, damage: Damage) -> Damage {
        let content = term.renderable_content();
        let cols = term.columns();
        let rows = term.screen_lines();
        let damage = if cols != self.cols || rows != self.rows { Damage::Full } else { damage };
        if damage == Damage::Full {
            self.cols = cols;
            self.rows = rows;
            self.cells = vec![Cell::blank(palette); cols * rows];
        }
        let offset = content.display_offset;
        let copy = RowCopy { palette, colors: content.colors, selection: content.selection, offset };
        match &damage {
            Damage::Full => (0..rows).for_each(|row| copy.row(term, row, self.row_mut(row))),
            Damage::Rows(changed) => {
                changed.iter().filter(|&&row| row < rows).for_each(|&row| copy.row(term, row, self.row_mut(row)));
            }
        }

        self.cursor = {
            let point = content.cursor.point;
            let row = point.line.0 + offset as i32;
            let kind = match content.cursor.shape {
                CursorShape::Hidden => None,
                CursorShape::Block => Some(CursorKind::Block),
                CursorShape::Beam => Some(CursorKind::Bar),
                CursorShape::Underline => Some(CursorKind::Underline),
                CursorShape::HollowBlock => Some(CursorKind::HollowBlock),
            };
            kind.filter(|_| row >= 0 && (row as usize) < rows && point.column.0 < cols).map(|kind| Cursor {
                row: row as usize,
                col: point.column.0,
                kind,
                wide: self.cells[row as usize * cols + point.column.0].wide,
                blinking: term.cursor_style().blinking,
            })
        };
        self.cursor_cell = {
            let point = content.cursor.point;
            let row = point.line.0 + offset as i32;
            (row >= 0 && (row as usize) < rows && point.column.0 < cols).then_some((row as usize, point.column.0))
        };
        self.display_offset = offset;
        self.history = term.grid().history_size();
        self.modes = Modes::of(content.mode);
        damage
    }

    fn row_mut(&mut self, row: usize) -> &mut [Cell] {
        &mut self.cells[row * self.cols..(row + 1) * self.cols]
    }

    pub fn row(&self, row: usize) -> &[Cell] {
        &self.cells[row * self.cols..(row + 1) * self.cols]
    }

    /// A row as text, without the blanks at its end. Wide characters count once.
    pub fn row_text(&self, row: usize) -> String {
        let text: String = self.row(row).iter().filter(|c| !c.spacer).map(|c| c.ch).collect();
        text.trim_end().to_string()
    }

    /// Everything on screen as text, a line per row, without blank rows at the end.
    pub fn text(&self) -> String {
        let lines: Vec<String> = (0..self.rows).map(|r| self.row_text(r)).collect();
        lines.join("\n").trim_end().to_string()
    }
}

/// What copying a row needs to know besides the row.
struct RowCopy<'a> {
    palette: &'a Palette,
    colors: &'a Colors,
    selection: Option<SelectionRange>,
    offset: usize,
}

impl RowCopy<'_> {
    fn row<T>(&self, term: &Term<T>, row: usize, out: &mut [Cell]) {
        let line = Line(row as i32 - self.offset as i32);
        let cells = &term.grid()[line];
        for (col, slot) in out.iter_mut().enumerate() {
            let selected = self.selection.is_some_and(|range| range.contains(Point::new(line, Column(col))));
            *slot = self.cell(&cells[Column(col)], selected);
        }
    }

    fn cell(&self, cell: &EngineCell, selected: bool) -> Cell {
        let flags = cell.flags;
        let bold = flags.contains(Flags::BOLD);
        // Bold text in one of the eight base colors uses its bright form, as in xterm.js.
        let fg_color = match cell.fg {
            Color::Named(named) if bold && (named as usize) < 8 => Color::Named(named.to_bright()),
            Color::Indexed(i) if bold && i < 8 => Color::Indexed(i + 8),
            other => other,
        };
        let mut fg = self.palette.resolve(fg_color, self.colors);
        let mut bg = self.palette.resolve(cell.bg, self.colors);
        let mut default_bg = cell.bg == Color::Named(NamedColor::Background);
        if flags.contains(Flags::DIM) {
            fg = fg.mix(bg, 0.34);
        }
        if flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
            default_bg = false;
        }
        if flags.contains(Flags::HIDDEN) {
            fg = bg;
        }
        Cell {
            ch: if flags.contains(Flags::WIDE_CHAR_SPACER) { ' ' } else { cell.c },
            extra: cell.zerowidth().filter(|z| !z.is_empty()).map(|z| z.to_vec().into_boxed_slice()),
            fg,
            bg,
            default_bg,
            bold,
            italic: flags.contains(Flags::ITALIC),
            underline: flags.intersects(Flags::ALL_UNDERLINES),
            strike: flags.contains(Flags::STRIKEOUT),
            wide: flags.contains(Flags::WIDE_CHAR),
            spacer: flags.contains(Flags::WIDE_CHAR_SPACER),
            selected,
        }
    }
}
