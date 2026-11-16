//! A terminal: the engine, the program's side, and the threads around them.
//!
//! The threads follow the app's smoothness rules:
//! - output reaches the parser through a short queue, from the terminal's own pty ([`Terminal::spawn`]) or from
//!   whoever runs the program (the core's terminals, through a [`Feed`]). When the parser falls behind, the queue
//!   fills and the reader waits, and the program waits with it, as with any terminal;
//! - the parser runs the escape-code parser over each chunk with the terminal locked, a chunk at a time, so the
//!   window can take its copy between two chunks;
//! - the writer hands what is typed, and each new size, to the program from a queue ([`Program`]), so a program
//!   that stops reading never blocks the window.
//!
//! The window copies the screen once a frame with [`Terminal::update`], only the rows that changed. The parser never
//! calls into the window: it sets a flag and calls `wake` when the flag goes up, so a flood of output asks for at
//! most one frame at a time.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Instant;

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermDamage};
use alacritty_terminal::vte::ansi::{CursorShape, CursorStyle, Processor, StdSyncHandler};
use anyhow::{Context, Result};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::keys::{Key, KeyAction, KeyModes, Mods, encode_key, encode_key_action};
use crate::links::{self, Link};
use crate::mouse::{MouseAction, mouse_report};
use crate::palette::Palette;
use crate::paste::paste_bytes;
use crate::screen::{Damage, Modes, Screen};
use crate::wheel::{WheelDelta, wheel_lines, wheel_speed};

/// How many lines of history a terminal keeps.
pub const HISTORY_LINES: usize = 10_000;

/// The terminal's size in cells, and a cell's size in pixels (programs may ask for it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridSize {
    pub cols: u16,
    pub rows: u16,
    pub cell_width: u16,
    pub cell_height: u16,
}

impl GridSize {
    pub fn new(cols: u16, rows: u16) -> GridSize {
        GridSize { cols: cols.max(2), rows: rows.max(1), cell_width: 8, cell_height: 18 }
    }

    fn pty(self) -> PtySize {
        PtySize {
            rows: self.rows,
            cols: self.cols,
            pixel_width: self.cols.saturating_mul(self.cell_width),
            pixel_height: self.rows.saturating_mul(self.cell_height),
        }
    }

    fn window(self) -> WindowSize {
        WindowSize {
            num_lines: self.rows,
            num_cols: self.cols,
            cell_width: self.cell_width,
            cell_height: self.cell_height,
        }
    }
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        usize::from(self.rows)
    }
    fn screen_lines(&self) -> usize {
        usize::from(self.rows)
    }
    fn columns(&self) -> usize {
        usize::from(self.cols)
    }
}

/// What a terminal runs.
#[derive(Clone, Debug)]
pub struct Spawn {
    /// The program and its arguments; None runs the user's own shell as a login shell.
    pub program: Option<(String, Vec<String>)>,
    pub cwd: PathBuf,
    /// The program's whole environment (see [`base_env`]).
    pub env: Vec<(String, String)>,
}

/// The environment for programs in a terminal: this app's own, without what would confuse them, plus the terminal
/// basics.
pub fn base_env() -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = std::env::vars()
        .filter(|(k, _)| {
            !(k == "CLAUDECODE"
                || k.starts_with("CLAUDE_CODE_")
                || k == "CLAUDE_PROJECT_DIR"
                || k.starts_with("ENKIDU_")
                || k.starts_with("VSCODE_")
                || k == "TERM_PROGRAM"
                || k == "TERM_PROGRAM_VERSION")
        })
        .collect();
    let mut set = |key: &str, value: &str| {
        env.retain(|(k, _)| k != key);
        env.push((key.to_string(), value.to_string()));
    };
    set("TERM", "xterm-256color");
    set("COLORTERM", "truecolor");
    set("TERM_PROGRAM", "Enkidu");
    if std::env::var("LANG").map(|l| l.is_empty()).unwrap_or(true) {
        set("LANG", "en_US.UTF-8");
    }
    env
}

/// Called from the parser's thread when there is something new to draw. It must not block.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// The program's side of a terminal: what you type goes to it, and it hears each new size. Called on the terminal's
/// writer thread, never on the window's, so a program that stops reading blocks only that thread.
pub trait Program: Send + 'static {
    /// Writes typed bytes. False when the program is gone; the writer then stops.
    fn write(&mut self, bytes: &[u8]) -> bool;
    /// The terminal has a new size.
    fn resize(&mut self, size: GridSize);
}

/// What the writer thread hands to the program.
enum ToProgram {
    Bytes(Vec<u8>),
    Resize(GridSize),
}

/// Where typed bytes, the terminal's replies and new sizes go.
#[derive(Clone)]
enum Input {
    /// To the writer thread, and from there to the program.
    Queue(mpsc::Sender<ToProgram>),
    /// Kept for a test or a picture (a terminal with no program).
    Kept(Arc<Mutex<Vec<u8>>>),
}

impl Input {
    fn send(&self, bytes: Vec<u8>) {
        match self {
            Input::Queue(queue) => {
                let _ = queue.send(ToProgram::Bytes(bytes));
            }
            Input::Kept(kept) => lock(kept).extend(bytes),
        }
    }

    fn resize(&self, size: GridSize) {
        if let Input::Queue(queue) = self {
            let _ = queue.send(ToProgram::Resize(size));
        }
    }
}

/// The terminal's own pty ([`Terminal::spawn`]).
struct PtyProgram {
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
}

impl Program for PtyProgram {
    fn write(&mut self, bytes: &[u8]) -> bool {
        self.writer.write_all(bytes).and_then(|()| self.writer.flush()).is_ok()
    }

    fn resize(&mut self, size: GridSize) {
        let _ = self.master.resize(size.pty());
    }
}

/// What the threads and the window share.
struct Shared {
    dirty: AtomicBool,
    wake: Wake,
    /// The window's own changes the engine doesn't track (the selection): the next copy is a full one.
    redraw_all: AtomicBool,
    title: Mutex<Option<String>>,
    bell: AtomicBool,
    exited: AtomicBool,
    parsed: AtomicU64,
    /// When the parser last took output from the program.
    last_output: Mutex<Option<Instant>>,
    size: Mutex<GridSize>,
    /// Text a program put on the clipboard (OSC 52), for the window to take.
    clipboard: Mutex<Option<String>>,
    /// The cell the last mouse move was reported on: a move within the same cell isn't reported again.
    last_move: Mutex<Option<(usize, usize)>>,
    /// Wheel travel not yet a whole line, in lines.
    wheel_carried: Mutex<f32>,
}

impl Shared {
    fn changed(&self) {
        if !self.dirty.swap(true, Ordering::AcqRel) {
            (self.wake)();
        }
    }
}

/// Hears what the terminal reports while it parses (with the terminal locked, on the parser's thread).
#[derive(Clone)]
pub(crate) struct Listener {
    input: Input,
    palette: Arc<Palette>,
    shared: Arc<Shared>,
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        match event {
            // Answers to the program's questions (cursor position, device attributes, kitty flags).
            Event::PtyWrite(text) => self.input.send(text.into_bytes()),
            Event::ColorRequest(index, format) => {
                let c = self.palette.slot(index);
                let rgb = alacritty_terminal::vte::ansi::Rgb { r: c.0, g: c.1, b: c.2 };
                self.input.send(format(rgb).into_bytes());
            }
            Event::TextAreaSizeRequest(format) => {
                let size = *lock(&self.shared.size);
                self.input.send(format(size.window()).into_bytes());
            }
            Event::Title(title) => *lock(&self.shared.title) = Some(title),
            Event::ResetTitle => *lock(&self.shared.title) = None,
            Event::Bell => self.shared.bell.store(true, Ordering::Release),
            Event::ClipboardStore(_, text) => *lock(&self.shared.clipboard) = Some(text),
            Event::Exit | Event::ChildExit(_) => self.shared.exited.store(true, Ordering::Release),
            // Reading the clipboard (OSC 52) is refused: a program must not read what you copied.
            Event::ClipboardLoad(..) | Event::MouseCursorDirty | Event::CursorBlinkingChange | Event::Wakeup => {}
        }
    }
}

/// How a selection grows: by character, by word, or by line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectKind {
    Simple,
    Word,
    Line,
}

/// Gives a terminal the output of a program that runs elsewhere ([`Terminal::attached`]), from any thread.
#[derive(Clone)]
pub struct Feed {
    chunks: mpsc::SyncSender<Vec<u8>>,
    shared: Arc<Shared>,
}

impl Feed {
    /// The program printed `bytes`. Waits while the parser is behind (the queue is short), so a flood holds the
    /// program back instead of piling up in memory.
    pub fn push(&self, bytes: Vec<u8>) {
        if !bytes.is_empty() {
            let _ = self.chunks.send(bytes);
        }
    }

    /// The program ended.
    pub fn ended(&self) {
        self.shared.exited.store(true, Ordering::Release);
        self.shared.changed();
    }
}

/// A terminal: the engine, and the program's side. The program runs in the terminal's own pty
/// ([`Terminal::spawn`]), elsewhere ([`Terminal::attached`]), or nowhere ([`Terminal::kept`]).
pub struct Terminal {
    term: Arc<FairMutex<Term<Listener>>>,
    palette: Arc<Palette>,
    input: Input,
    shared: Arc<Shared>,
    /// Ends the program of a terminal that runs its own.
    killer: Option<Mutex<Box<dyn ChildKiller + Send + Sync>>>,
    /// For a terminal with no program: its parser, run by [`Terminal::feed`] on the caller's thread.
    kept_parser: Option<Mutex<Processor<StdSyncHandler>>>,
}

fn term_config() -> Config {
    Config {
        scrolling_history: HISTORY_LINES,
        // A blinking bar, until the program asks for another shape.
        default_cursor_style: CursorStyle { shape: CursorShape::Beam, blinking: true },
        kitty_keyboard: true,
        ..Config::default()
    }
}

fn shared(size: GridSize, wake: Wake) -> Arc<Shared> {
    Arc::new(Shared {
        dirty: AtomicBool::new(true),
        wake,
        redraw_all: AtomicBool::new(false),
        title: Mutex::new(None),
        bell: AtomicBool::new(false),
        exited: AtomicBool::new(false),
        parsed: AtomicU64::new(0),
        last_output: Mutex::new(None),
        size: Mutex::new(size),
        clipboard: Mutex::new(None),
        last_move: Mutex::new(None),
        wheel_carried: Mutex::new(0.0),
    })
}

impl Terminal {
    /// A terminal running `spawn` in a pty of its own.
    pub fn spawn(spawn: Spawn, size: GridSize, palette: Palette, wake: Wake) -> Result<Terminal> {
        let pair = native_pty_system().openpty(size.pty()).context("Can't open a terminal")?;
        let mut cmd = match &spawn.program {
            None => CommandBuilder::new_default_prog(),
            Some((program, args)) => {
                let mut cmd = CommandBuilder::new(program);
                cmd.args(args);
                cmd
            }
        };
        cmd.cwd(&spawn.cwd);
        cmd.env_clear();
        for (k, v) in &spawn.env {
            cmd.env(k, v);
        }
        let mut child = pair.slave.spawn_command(cmd).context("Can't start the terminal's program")?;
        drop(pair.slave);
        let reader = pair.master.try_clone_reader().context("Can't read the terminal")?;
        let writer = pair.master.take_writer().context("Can't write to the terminal")?;
        let killer = child.clone_killer();

        let (mut terminal, chunks) = Terminal::start(size, palette, wake, PtyProgram { writer, master: pair.master })?;
        thread::Builder::new().name("enkidu-term reader".into()).spawn(move || read_loop(reader, chunks))?;
        {
            let shared = terminal.shared.clone();
            thread::Builder::new().name("enkidu-term child".into()).spawn(move || {
                let _ = child.wait();
                shared.exited.store(true, Ordering::Release);
                shared.changed();
            })?;
        }
        terminal.killer = Some(Mutex::new(killer));
        Ok(terminal)
    }

    /// A terminal whose program runs elsewhere: its output comes in through the [`Feed`], and what is typed (and
    /// each new size) goes to `program`.
    pub fn attached(size: GridSize, palette: Palette, wake: Wake, program: impl Program) -> Result<(Terminal, Feed)> {
        let (terminal, chunks) = Terminal::start(size, palette, wake, program)?;
        let feed = Feed { chunks, shared: terminal.shared.clone() };
        Ok((terminal, feed))
    }

    /// The engine with its writer and parser threads; output goes into the returned queue.
    fn start(
        size: GridSize,
        palette: Palette,
        wake: Wake,
        mut program: impl Program,
    ) -> Result<(Terminal, mpsc::SyncSender<Vec<u8>>)> {
        let (queue, typed) = mpsc::channel::<ToProgram>();
        let input = Input::Queue(queue);
        let shared = shared(size, wake);
        let palette = Arc::new(palette);
        let listener = Listener { input: input.clone(), palette: palette.clone(), shared: shared.clone() };
        let term = Arc::new(FairMutex::new(Term::new(term_config(), &size, listener)));

        thread::Builder::new().name("enkidu-term writer".into()).spawn(move || {
            while let Ok(next) = typed.recv() {
                match next {
                    ToProgram::Bytes(bytes) => {
                        if !program.write(&bytes) {
                            break;
                        }
                    }
                    ToProgram::Resize(size) => program.resize(size),
                }
            }
        })?;
        let (chunks_in, chunks_out) = mpsc::sync_channel::<Vec<u8>>(16);
        {
            let term = term.clone();
            let shared = shared.clone();
            thread::Builder::new()
                .name("enkidu-term parser".into())
                .spawn(move || parse_loop(&term, &shared, chunks_out))?;
        }
        Ok((Terminal { term, palette, input, shared, killer: None, kept_parser: None }, chunks_in))
    }

    /// A terminal with no program, for tests and pictures: [`Terminal::feed`] gives it output, and what it would
    /// send to a program is kept for [`Terminal::sent`].
    pub fn kept(size: GridSize, palette: Palette) -> Terminal {
        let input = Input::Kept(Arc::new(Mutex::new(Vec::new())));
        let shared = shared(size, Arc::new(|| {}));
        let palette = Arc::new(palette);
        let listener = Listener { input: input.clone(), palette: palette.clone(), shared: shared.clone() };
        let term = Arc::new(FairMutex::new(Term::new(term_config(), &size, listener)));
        Terminal { term, palette, input, shared, killer: None, kept_parser: Some(Mutex::new(Processor::new())) }
    }

    /// Output for a kept terminal, parsed now on the caller's thread. Does nothing for a terminal with a program.
    pub fn feed(&self, bytes: &[u8]) {
        let Some(parser) = &self.kept_parser else { return };
        let mut parser = lock(parser);
        let mut term = self.term.lock();
        parser.advance(&mut *term, bytes);
        if parser.sync_bytes_count() > 0 {
            parser.stop_sync(&mut *term);
        }
        drop(term);
        self.shared.parsed.fetch_add(bytes.len() as u64, Ordering::Relaxed);
        self.shared.changed();
    }

    /// Everything a kept terminal would have sent to a program so far.
    pub fn sent(&self) -> Vec<u8> {
        match &self.input {
            Input::Kept(kept) => lock(kept).clone(),
            Input::Queue(_) => Vec::new(),
        }
    }

    /// Sends bytes to the program, as typed. It never blocks: the writer thread does the writing. Typing brings the
    /// view back to the bottom, as in any terminal.
    pub fn send(&self, bytes: Vec<u8>) {
        if bytes.is_empty() {
            return;
        }
        {
            let mut term = self.term.lock();
            if term.grid().display_offset() != 0 {
                term.scroll_display(Scroll::Bottom);
                drop(term);
                self.shared.changed();
            }
        }
        self.input.send(bytes);
    }

    /// A key press, as the program's modes want it (application cursor keys, the kitty keyboard protocol). Returns
    /// whether it sent anything; Cmd shortcuts send nothing, they are the app's.
    pub fn key(&self, key: &Key, mods: Mods) -> bool {
        self.key_action(key, mods, KeyAction::Press)
    }

    /// A key press, repeat or release (the kitty protocol's event types tell them apart). Shift with Page Up or Page
    /// Down moves the view a page through the history instead. Returns whether the key was used.
    pub fn key_action(&self, key: &Key, mods: Mods, action: KeyAction) -> bool {
        if mods.shift && !mods.cmd && matches!(key, Key::PageUp | Key::PageDown) {
            if action != KeyAction::Release {
                let page = i32::from(self.size().rows.saturating_sub(1).max(1));
                self.scroll(if *key == Key::PageUp { page } else { -page });
            }
            return true;
        }
        let modes = self.modes();
        let keys = KeyModes { app_cursor: modes.app_cursor, kitty: modes.kitty };
        match encode_key_action(key, mods, keys, action) {
            // a release never moves the view
            Some(bytes) if action == KeyAction::Release => self.input.send(bytes),
            Some(bytes) => self.send(bytes),
            None => return false,
        }
        true
    }

    /// A paste: one block, bracketed when the program asked for that.
    pub fn paste(&self, text: &str) {
        self.send(paste_bytes(text, self.modes().bracketed_paste));
    }

    /// The window gained or lost the keyboard: told to a program that asked to hear it (`CSI ? 1004 h`).
    pub fn focus(&self, focused: bool) {
        if self.modes().focus_reports {
            self.input.send(if focused { b"\x1b[I".to_vec() } else { b"\x1b[O".to_vec() });
        }
    }

    /// A mouse press, release or move at a cell on screen. While the program takes the mouse (and Shift isn't
    /// held) the mouse is the program's: it hears what its mode asks for, and this returns true. Otherwise it
    /// returns false, and the window selects text.
    pub fn mouse(&self, action: MouseAction, row: usize, col: usize, mods: Mods) -> bool {
        let modes = self.modes();
        if !modes.mouse.on() || mods.shift {
            *lock(&self.shared.last_move) = None;
            return false;
        }
        if let MouseAction::Move(_) = action {
            let mut last = lock(&self.shared.last_move);
            if *last == Some((row, col)) {
                return true;
            }
            *last = Some((row, col));
        }
        if let Some(bytes) = mouse_report(action, row, col, mods, modes.mouse, modes.mouse_encoding) {
            self.input.send(bytes);
        }
        true
    }

    /// A wheel event at a cell: a trackpad's pixels or a mouse's lines, with rows `line_height` pixels tall. Its whole
    /// lines go as [`Terminal::wheel`] says, the rest waits for the next event; Option scrolls faster, and Ctrl too
    /// over a program that takes the mouse. Returns the lines it moved.
    pub fn scroll_wheel(&self, delta: WheelDelta, line_height: f32, row: usize, col: usize, mods: Mods) -> i32 {
        let speed = wheel_speed(mods, self.modes().mouse.on() && !mods.shift);
        let delta = match delta {
            WheelDelta::Pixels(pixels) => WheelDelta::Pixels(pixels * speed),
            WheelDelta::Lines(lines) => WheelDelta::Lines(lines * speed),
            WheelDelta::Pages(pages) => WheelDelta::Pages(pages * speed),
        };
        let rows = usize::from(self.size().rows);
        let lines = {
            let mut carried = lock(&self.shared.wheel_carried);
            let (lines, rest) = wheel_lines(delta, line_height, rows, *carried);
            *carried = rest;
            lines
        };
        self.wheel(lines, row, col, mods);
        lines
    }

    /// The wheel moved `lines` at a cell, toward the history when positive. A program that takes the mouse hears
    /// one report a line; in the alternate screen of a program that asked for it (`CSI ? 1007 h`) each line is an
    /// arrow key; otherwise the view moves through the history.
    pub fn wheel(&self, lines: i32, row: usize, col: usize, mods: Mods) {
        if lines == 0 {
            return;
        }
        let modes = self.modes();
        let count = lines.unsigned_abs() as usize;
        if modes.mouse.on() && !mods.shift {
            let action = if lines > 0 { MouseAction::WheelUp } else { MouseAction::WheelDown };
            if let Some(one) = mouse_report(action, row, col, mods, modes.mouse, modes.mouse_encoding) {
                self.input.send(one.repeat(count));
            }
        } else if modes.alt_screen && modes.alternate_scroll {
            let key = if lines > 0 { Key::Up } else { Key::Down };
            let keys = KeyModes { app_cursor: modes.app_cursor, kitty: 0 };
            if let Some(one) = encode_key(&key, Mods::NONE, keys) {
                self.input.send(one.repeat(count));
            }
        } else {
            self.scroll(lines);
        }
    }

    /// Whether there is something new to draw since the last call. Taking it lets the next change wake the window.
    pub fn take_changed(&self) -> bool {
        self.shared.dirty.swap(false, Ordering::AcqRel)
    }

    /// Brings `screen` up to date and says which rows changed: only those are copied again. Start from
    /// `Screen::default()`; the first update copies everything.
    pub fn update(&self, screen: &mut Screen) -> Damage {
        let mut term = self.term.lock();
        let all = self.shared.redraw_all.swap(false, Ordering::AcqRel);
        let damage = match term.damage() {
            TermDamage::Full => Damage::Full,
            _ if all => Damage::Full,
            TermDamage::Partial(lines) => {
                let mut rows: Vec<usize> = lines.map(|line| line.line).collect();
                rows.sort_unstable();
                rows.dedup();
                Damage::Rows(rows)
            }
        };
        term.reset_damage();
        screen.refresh(&term, &self.palette, damage)
    }

    /// What the terminal shows now, copied whole.
    pub fn screen(&self) -> Screen {
        let term = self.term.lock();
        Screen::capture(&term, &self.palette)
    }

    /// The modes the program turned on.
    pub fn modes(&self) -> Modes {
        Modes::of(*self.term.lock().mode())
    }

    pub fn size(&self) -> GridSize {
        *lock(&self.shared.size)
    }

    /// Resizes the terminal; the program hears it from the writer thread. Returns whether the size changed.
    pub fn resize(&self, size: GridSize) -> bool {
        {
            let mut current = lock(&self.shared.size);
            if *current == size {
                return false;
            }
            *current = size;
        }
        self.input.resize(size);
        self.term.lock().resize(size);
        self.shared.changed();
        true
    }

    /// Moves the view through the history: positive lines go back, negative toward the bottom.
    pub fn scroll(&self, lines: i32) {
        if lines == 0 {
            return;
        }
        self.term.lock().scroll_display(Scroll::Delta(lines));
        self.shared.changed();
    }

    /// The window changed the selection, which the engine doesn't count as damage: the next copy is whole.
    fn selection_changed(&self) {
        self.shared.redraw_all.store(true, Ordering::Release);
        self.shared.changed();
    }

    /// Starts a selection at a cell on screen; `right_half` says which half of the cell the pointer is on.
    pub fn select_start(&self, row: usize, col: usize, right_half: bool, kind: SelectKind) {
        let mut term = self.term.lock();
        let point = screen_point(&term, row, col);
        let side = if right_half { Side::Right } else { Side::Left };
        let ty = match kind {
            SelectKind::Simple => SelectionType::Simple,
            SelectKind::Word => SelectionType::Semantic,
            SelectKind::Line => SelectionType::Lines,
        };
        term.selection = Some(Selection::new(ty, point, side));
        drop(term);
        self.selection_changed();
    }

    /// Moves the selection's end to a cell on screen.
    pub fn select_to(&self, row: usize, col: usize, right_half: bool) {
        let mut term = self.term.lock();
        let point = screen_point(&term, row, col);
        let side = if right_half { Side::Right } else { Side::Left };
        if let Some(selection) = term.selection.as_mut() {
            selection.update(point, side);
        }
        drop(term);
        self.selection_changed();
    }

    pub fn select_clear(&self) {
        let mut term = self.term.lock();
        if term.selection.take().is_some() {
            drop(term);
            self.selection_changed();
        }
    }

    /// The link under a cell on screen: a program's hyperlink (OSC 8) or a web address in the text, also one that
    /// wraps onto the next row. Only links that may open count (`links::opens`).
    pub fn link_at(&self, row: usize, col: usize) -> Option<Link> {
        let term = self.term.lock();
        let (rows, cols) = (term.screen_lines(), term.columns());
        if row >= rows || col >= cols {
            return None;
        }
        let offset = term.grid().display_offset() as i32;
        let cell = |r: usize, c: usize| &term.grid()[Line(r as i32 - offset)][Column(c)];
        if let Some(link) = cell(row, col).hyperlink() {
            if !links::opens(link.uri()) {
                return None;
            }
            // the cells around it that carry the same link, in reading order on screen
            let at = |i: usize| (i / cols, i % cols);
            let same = |i: usize| {
                let (r, c) = at(i);
                cell(r, c).hyperlink().as_ref() == Some(&link)
            };
            let here = row * cols + col;
            let mut first = here;
            while first > 0 && same(first - 1) {
                first -= 1;
            }
            let mut last = here;
            while last + 1 < rows * cols && same(last + 1) {
                last += 1;
            }
            return Some(Link { url: link.uri().to_string(), cells: Link::runs((first..=last).map(at)) });
        }
        // a web address: the text of the rows this one wraps with
        let wraps = |r: usize| cell(r, cols - 1).flags.contains(Flags::WRAPLINE);
        let mut top = row;
        while top > 0 && wraps(top - 1) {
            top -= 1;
        }
        let mut bottom = row;
        while bottom + 1 < rows && wraps(bottom) {
            bottom += 1;
        }
        let mut text = Vec::new();
        let mut places = Vec::new();
        for r in top..=bottom {
            for c in 0..cols {
                let cell = cell(r, c);
                if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                    continue;
                }
                text.push(cell.c);
                places.push((r, c));
            }
        }
        links::addresses(&text).into_iter().find_map(|(start, end)| {
            let span = &places[start..end];
            let covers = span
                .iter()
                .any(|&(r, c)| r == row && (c == col || (c + 1 == col && cell(r, c).flags.contains(Flags::WIDE_CHAR))));
            covers.then(|| Link { url: text[start..end].iter().collect(), cells: Link::runs(span.iter().copied()) })
        })
    }

    /// The selected text, if anything is selected. A whole line selected comes without its last new line, so pasting
    /// it into a shell never runs it.
    pub fn selection_text(&self) -> Option<String> {
        let text = self.term.lock().selection_to_string()?;
        let text = text.strip_suffix('\n').map(str::to_string).unwrap_or(text);
        (!text.is_empty()).then_some(text)
    }

    /// The window title the program set.
    pub fn title(&self) -> Option<String> {
        lock(&self.shared.title).clone()
    }

    /// Whether the program rang the bell (`BEL`) since the last call. The window does nothing with it: Claude's lamps
    /// say when it needs you.
    pub fn take_bell(&self) -> bool {
        self.shared.bell.swap(false, Ordering::AcqRel)
    }

    /// Text a program asked to put on the clipboard (OSC 52), taken once.
    pub fn take_clipboard(&self) -> Option<String> {
        lock(&self.shared.clipboard).take()
    }

    /// Bytes of output parsed so far.
    pub fn parsed(&self) -> u64 {
        self.shared.parsed.load(Ordering::Relaxed)
    }

    /// When the parser last took output from the program.
    pub fn last_output(&self) -> Option<Instant> {
        *lock(&self.shared.last_output)
    }

    pub fn has_exited(&self) -> bool {
        self.shared.exited.load(Ordering::Acquire)
    }

    /// Ends the program, when it runs in the terminal's own pty.
    pub fn kill(&self) {
        if let Some(killer) = &self.killer {
            let _ = lock(killer).kill();
        }
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        self.kill();
    }
}

fn screen_point<T>(term: &Term<T>, row: usize, col: usize) -> Point {
    let offset = term.grid().display_offset() as i32;
    let row = row.min(term.screen_lines().saturating_sub(1)) as i32;
    let col = col.min(term.columns().saturating_sub(1));
    Point::new(Line(row - offset), Column(col))
}

fn read_loop(mut reader: Box<dyn Read + Send>, chunks: mpsc::SyncSender<Vec<u8>>) {
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        match reader.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if chunks.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        }
    }
}

fn parse_loop(term: &FairMutex<Term<Listener>>, shared: &Shared, chunks: mpsc::Receiver<Vec<u8>>) {
    let mut parser: Processor<StdSyncHandler> = Processor::new();
    loop {
        // A program that began a synchronized update and went quiet is drawn once the update's time is up.
        let chunk = match parser.sync_timeout().sync_timeout() {
            Some(deadline) => match chunks.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(chunk) => chunk,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    parser.stop_sync(&mut *term.lock());
                    shared.changed();
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            },
            None => match chunks.recv() {
                Ok(chunk) => chunk,
                Err(_) => break,
            },
        };
        parser.advance(&mut *term.lock(), &chunk);
        shared.parsed.fetch_add(chunk.len() as u64, Ordering::Relaxed);
        *lock(&shared.last_output) = Some(Instant::now());
        shared.changed();
    }
    if parser.sync_bytes_count() > 0 {
        parser.stop_sync(&mut *term.lock());
    }
    shared.exited.store(true, Ordering::Release);
    shared.changed();
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kept(cols: u16, rows: u16) -> Terminal {
        Terminal::kept(GridSize::new(cols, rows), Palette::enkidu())
    }

    #[test]
    fn output_shows_on_screen_with_its_colors() {
        let t = kept(20, 4);
        t.feed(b"plain \x1b[31mred\x1b[0m \x1b[38;2;1;2;3mrgb\x1b[0m\r\nnext");
        let s = t.screen();
        assert_eq!(s.row_text(0), "plain red rgb");
        assert_eq!(s.row_text(1), "next");
        assert_eq!(s.row(0)[6].fg, Palette::enkidu().ansi[1]);
        assert_eq!(s.row(0)[10].fg, crate::Rgb(1, 2, 3));
        assert!(s.row(0)[0].default_bg);
    }

    #[test]
    fn answers_the_programs_questions() {
        let t = kept(20, 4);
        // where is the cursor?
        t.feed(b"ab\x1b[6n");
        assert_eq!(t.sent(), b"\x1b[1;3R");
    }

    #[test]
    fn tells_a_program_the_background_color_so_it_can_pick_a_dark_theme() {
        let t = kept(20, 4);
        t.feed(b"\x1b]11;?\x07");
        let reply = String::from_utf8(t.sent()).unwrap();
        assert!(reply.starts_with("\x1b]11;rgb:1111/1212/1717"), "{reply:?}");
    }

    #[test]
    fn turns_on_the_kitty_keyboard_protocol_when_the_program_asks() {
        let t = kept(20, 4);
        assert_eq!(t.modes().kitty, 0);
        t.feed(b"\x1b[>1u");
        assert_eq!(t.modes().kitty, 1);
        t.feed(b"\x1b[?u");
        assert_eq!(t.sent(), b"\x1b[?1u");
        t.feed(b"\x1b[<u");
        assert_eq!(t.modes().kitty, 0);
    }

    #[test]
    fn keeps_history_and_scrolls_through_it() {
        let t = kept(10, 3);
        for i in 0..10 {
            t.feed(format!("line {i}\r\n").as_bytes());
        }
        assert_eq!(t.screen().row_text(0), "line 8");
        t.scroll(3);
        let s = t.screen();
        assert_eq!(s.display_offset, 3);
        assert_eq!(s.row_text(0), "line 5");
        // typing brings the view back down
        t.send(b"x".to_vec());
        assert_eq!(t.screen().display_offset, 0);
    }

    #[test]
    fn the_view_keeps_its_lines_while_output_arrives() {
        let t = kept(10, 3);
        for i in 0..20 {
            t.feed(format!("line {i}\r\n").as_bytes());
        }
        t.scroll(5);
        let seen = t.screen().text();
        assert_eq!(seen, "line 13\nline 14\nline 15");
        for i in 20..30 {
            t.feed(format!("line {i}\r\n").as_bytes());
        }
        let s = t.screen();
        assert_eq!(s.text(), seen, "the same lines stay in view");
        assert_eq!(s.display_offset, 15);
    }

    #[test]
    fn the_history_keeps_ten_thousand_lines_and_the_view_stops_at_its_top() {
        let t = kept(12, 3);
        let mut out = String::new();
        for i in 0..10_100 {
            out.push_str(&format!("line {i}\r\n"));
        }
        t.feed(out.as_bytes());
        let s = t.screen();
        assert_eq!(s.history, HISTORY_LINES);
        t.scroll(20_000);
        let s = t.screen();
        assert_eq!(s.display_offset, HISTORY_LINES);
        // the oldest lines kept: 10,100 printed, a blank row under the last, 3 rows on screen
        assert_eq!(s.row_text(0), "line 98");
    }

    #[test]
    fn the_alternate_screen_has_no_history_and_leaving_it_brings_the_main_one_back() {
        let t = kept(10, 3);
        for i in 0..10 {
            t.feed(format!("line {i}\r\n").as_bytes());
        }
        let main = t.screen();
        t.feed(b"\x1b[?1049h\x1b[2J\x1b[Hfull screen\x1b[?1007l");
        let s = t.screen();
        assert_eq!((s.history, s.modes.alt_screen), (0, true));
        // a program that turned the wheel's arrows off: the wheel has nowhere to go
        t.wheel(3, 0, 0, Mods::NONE);
        assert_eq!((t.screen().display_offset, t.sent().len()), (0, 0));
        t.feed(b"\x1b[?1049l");
        let back = t.screen();
        assert_eq!(back.text(), main.text());
        assert_eq!(back.history, main.history);
    }

    #[test]
    fn the_program_sets_the_title_keeps_a_stack_of_them_and_rings_the_bell() {
        let t = kept(20, 3);
        assert_eq!(t.title(), None);
        t.feed(b"\x1b]0;vim notes.md\x07");
        assert_eq!(t.title().as_deref(), Some("vim notes.md"));
        // saved, changed, restored (`CSI 22 t`, `CSI 23 t`)
        t.feed(b"\x1b[22t\x1b]2;make test\x1b\\");
        assert_eq!(t.title().as_deref(), Some("make test"));
        t.feed(b"\x1b[23t");
        assert_eq!(t.title().as_deref(), Some("vim notes.md"));
        assert!(!t.take_bell());
        t.feed(b"\x07");
        assert!(t.take_bell());
        assert!(!t.take_bell(), "taken once");
    }

    #[test]
    fn the_program_picks_the_cursor_shape_and_whether_it_blinks() {
        use crate::screen::CursorKind;
        let t = kept(20, 3);
        let cursor = |t: &Terminal| t.screen().cursor.map(|c| (c.kind, c.blinking));
        assert_eq!(cursor(&t), Some((CursorKind::Bar, true)), "a blinking bar");
        t.feed(b"\x1b[2 q");
        assert_eq!(cursor(&t), Some((CursorKind::Block, false)));
        t.feed(b"\x1b[3 q");
        assert_eq!(cursor(&t), Some((CursorKind::Underline, true)));
        t.feed(b"\x1b[6 q");
        assert_eq!(cursor(&t), Some((CursorKind::Bar, false)));
        t.feed(b"\x1b[0 q");
        assert_eq!(cursor(&t), Some((CursorKind::Bar, true)), "back to the default");
        t.feed(b"\x1b[?25l");
        assert_eq!(cursor(&t), None, "hidden, as Claude Code hides it");
        t.feed(b"\x1b[?25h");
        assert!(cursor(&t).is_some());
    }

    #[test]
    fn a_hidden_cursor_still_has_its_cell() {
        let t = kept(20, 3);
        t.feed(b"> \x1b[?25l");
        let s = t.screen();
        assert_eq!((s.cursor, s.cursor_cell), (None, Some((0, 2))));
    }

    #[test]
    fn emoji_take_two_cells_and_keep_their_variation_selector() {
        let t = kept(12, 2);
        t.feed("a🙂b❤\u{fe0f}c".as_bytes());
        let s = t.screen();
        assert!(s.row(0)[1].wide && s.row(0)[2].spacer);
        assert_eq!(s.row(0)[3].ch, 'b');
        assert_eq!(s.row_text(0), "a🙂b❤c");
        let heart = s.row(0).iter().find(|c| c.ch == '❤').unwrap();
        assert_eq!(heart.extra.as_deref(), Some(['\u{fe0f}'].as_slice()), "drawn with its selector, as an emoji");
    }

    #[test]
    fn arabic_keeps_a_cell_a_letter_in_the_order_it_was_written() {
        let t = kept(20, 2);
        t.feed("مرحبا|x".as_bytes());
        let s = t.screen();
        let cells: String = s.row(0)[..7].iter().map(|c| c.ch).collect();
        assert_eq!(cells, "مرحبا|x");
        assert!(s.row(0)[..5].iter().all(|c| !c.wide && !c.spacer));
    }

    #[test]
    fn a_synchronized_update_waits_for_its_end_or_its_time_on_the_parser_thread() {
        let (t, feed) =
            Terminal::attached(GridSize::new(20, 3), Palette::enkidu(), Arc::new(|| {}), Elsewhere::default()).unwrap();
        feed.push(b"\x1b[?2026hpart".to_vec());
        thread::sleep(std::time::Duration::from_millis(40));
        assert_eq!(t.screen().row_text(0), "", "half an update isn't shown");
        feed.push(b"ial\x1b[?2026l".to_vec());
        assert!(eventually(|| t.screen().row_text(0) == "partial"));
        // an update that never ends is shown once its time is up
        feed.push(b"\r\n\x1b[?2026hstuck".to_vec());
        thread::sleep(std::time::Duration::from_millis(40));
        assert_eq!(t.screen().row_text(1), "");
        assert!(eventually(|| t.screen().row_text(1) == "stuck"), "after the update's time");
    }

    #[test]
    fn selects_and_copies_text() {
        let t = kept(20, 3);
        t.feed(b"hello world\r\nsecond line");
        t.select_start(0, 6, false, SelectKind::Simple);
        t.select_to(0, 10, true);
        assert_eq!(t.selection_text().as_deref(), Some("world"));
        assert!(t.screen().row(0)[6].selected);
        t.select_start(1, 2, false, SelectKind::Line);
        assert_eq!(t.selection_text().as_deref(), Some("second line"));
        t.select_clear();
        assert_eq!(t.selection_text(), None);
    }

    #[test]
    fn wide_characters_take_two_cells() {
        let t = kept(10, 2);
        t.feed("a中b".as_bytes());
        let s = t.screen();
        assert!(s.row(0)[1].wide);
        assert!(s.row(0)[2].spacer);
        assert_eq!(s.row(0)[3].ch, 'b');
        assert_eq!(s.row_text(0), "a中b");
    }

    #[test]
    fn a_synchronized_update_is_drawn_whole() {
        let t = kept(10, 2);
        t.feed(b"\x1b[?2026hpart");
        t.feed(b"ial\x1b[?2026l");
        assert_eq!(t.screen().row_text(0), "partial");
    }

    #[test]
    fn the_environment_never_carries_the_apps_own_variables() {
        // SAFETY: nothing reads these two names: they are this test's own.
        unsafe {
            std::env::set_var("ENKIDU_ONLY_IN_THIS_TEST", "http://127.0.0.1:9");
            std::env::set_var("ENKIDU_ONLY_IN_THIS_TEST_TOO", "secret");
        }
        let env = base_env();
        assert!(env.iter().all(|(k, _)| !k.starts_with("ENKIDU_")));
        assert!(env.iter().any(|(k, v)| k == "TERM" && v == "xterm-256color"));
        unsafe {
            std::env::remove_var("ENKIDU_ONLY_IN_THIS_TEST");
            std::env::remove_var("ENKIDU_ONLY_IN_THIS_TEST_TOO");
        }
    }

    #[test]
    fn only_the_rows_that_changed_are_copied_again() {
        let t = kept(20, 5);
        let mut screen = Screen::default();
        assert_eq!(t.update(&mut screen), Damage::Full, "the first copy is whole");
        t.feed(b"hello");
        assert_eq!(t.update(&mut screen), Damage::Rows(vec![0]));
        assert_eq!(screen.row_text(0), "hello");
        t.feed(b"\r\nworld");
        // the new text, and the row the cursor left
        assert_eq!(t.update(&mut screen), Damage::Rows(vec![0, 1]));
        assert_eq!(screen.row_text(1), "world");
        // nothing new: only the cursor's row
        assert_eq!(t.update(&mut screen), Damage::Rows(vec![1]));
    }

    #[test]
    fn output_that_scrolls_a_new_selection_and_a_new_size_copy_everything() {
        let t = kept(10, 3);
        let mut screen = Screen::default();
        t.update(&mut screen);
        t.feed(b"1\r\n2\r\n3\r\n4");
        assert_eq!(t.update(&mut screen), Damage::Full, "the text moved up");
        t.update(&mut screen);
        t.select_start(0, 0, false, SelectKind::Line);
        assert_eq!(t.update(&mut screen), Damage::Full, "the selection");
        assert!(screen.row(0)[0].selected);
        t.resize(GridSize::new(12, 4));
        assert_eq!(t.update(&mut screen), Damage::Full, "the size");
        assert_eq!((screen.cols, screen.rows), (12, 4));
    }

    #[test]
    fn the_rows_copied_again_always_match_a_whole_copy() {
        // what a full-screen program prints: moves, colors, erasing, a scroll region, insert and delete
        let steps: [&[u8]; 12] = [
            b"\x1b[2J\x1b[Htop line\r\nsecond",
            b"\x1b[3;5H\x1b[31mred\x1b[0m",
            b"\x1b[1;1H\x1b[K",
            b"\x1b[2;3r\x1b[2;1H\x1bM",
            b"\x1b[r\x1b[5;1Hbottom",
            b"\x1b[4;2H\x1b[2@ins",
            b"\x1b[4;1H\x1b[1P",
            b"\x1b[?1049h\x1b[Halt screen",
            b"\x1b[3;3H\x1b[7mreverse\x1b[27m",
            b"\x1b[?1049l",
            "\x1b[2;1H中文 wide".as_bytes(),
            b"\r\n\r\n\r\n\r\nscrolled",
        ];
        let t = kept(16, 6);
        let mut screen = Screen::default();
        t.update(&mut screen);
        for step in steps {
            t.feed(step);
            t.update(&mut screen);
            let whole = t.screen();
            assert_eq!(screen.cells, whole.cells, "after {:?}:\n{}", String::from_utf8_lossy(step), whole.text());
            assert_eq!(screen.cursor, whole.cursor);
        }
    }

    #[test]
    fn keys_follow_the_programs_modes() {
        let t = kept(20, 4);
        assert!(t.key(&Key::Up, Mods::NONE));
        t.feed(b"\x1b[?1h");
        assert!(t.key(&Key::Up, Mods::NONE));
        assert!(!t.key(&Key::text('c'), Mods { cmd: true, ..Mods::NONE }), "Cmd is the app's");
        t.feed(b"\x1b[>1u");
        assert!(t.key(&Key::Escape, Mods::NONE));
        assert_eq!(t.sent(), b"\x1b[A\x1bOA\x1b[27u");
    }

    #[test]
    fn shift_with_page_up_and_page_down_moves_the_view_a_page() {
        let t = kept(10, 4);
        for i in 0..20 {
            t.feed(format!("line {i}\r\n").as_bytes());
        }
        assert!(t.key(&Key::PageUp, Mods::SHIFT));
        assert_eq!(t.screen().display_offset, 3, "a page is the rows less one");
        assert!(t.key_action(&Key::PageUp, Mods::SHIFT, KeyAction::Release));
        assert_eq!(t.screen().display_offset, 3, "the release moves nothing");
        t.key(&Key::PageDown, Mods::SHIFT);
        assert_eq!(t.screen().display_offset, 0);
        assert_eq!(t.sent(), b"", "nothing went to the program");
    }

    #[test]
    fn with_event_types_on_a_release_reaches_the_program_and_the_view_stays() {
        let t = kept(10, 3);
        for i in 0..10 {
            t.feed(format!("line {i}\r\n").as_bytes());
        }
        t.feed(b"\x1b[>3u");
        t.scroll(2);
        assert!(t.key_action(&Key::Escape, Mods::NONE, KeyAction::Release));
        assert_eq!(t.screen().display_offset, 2);
        assert!(!t.key_action(&Key::text('a'), Mods::NONE, KeyAction::Release), "text is never released");
        assert_eq!(t.sent(), b"\x1b[27;1:3u");
    }

    #[test]
    fn focus_is_told_only_to_a_program_that_asked() {
        let t = kept(20, 4);
        t.focus(true);
        assert_eq!(t.sent(), b"");
        t.feed(b"\x1b[?1004h");
        t.focus(false);
        t.focus(true);
        assert_eq!(t.sent(), b"\x1b[O\x1b[I");
    }

    #[test]
    fn the_mouse_is_the_programs_while_it_listens_and_shift_keeps_it_the_windows() {
        let t = kept(40, 10);
        let left = crate::mouse::Button::Left;
        assert!(!t.mouse(MouseAction::Press(left), 1, 2, Mods::NONE), "no program listens: the window selects");
        t.feed(b"\x1b[?1000h\x1b[?1006h");
        assert!(t.mouse(MouseAction::Press(left), 1, 2, Mods::NONE));
        assert!(
            t.mouse(MouseAction::Move(Some(left)), 1, 3, Mods::NONE),
            "a drag isn't asked for, still the program's"
        );
        assert!(t.mouse(MouseAction::Release(left), 1, 3, Mods::NONE));
        assert!(!t.mouse(MouseAction::Press(left), 1, 2, Mods::SHIFT), "Shift selects");
        assert_eq!(t.sent(), b"\x1b[<0;3;2M\x1b[<0;4;2m");
        // moves with a button held, each cell once
        t.feed(b"\x1b[?1002h");
        t.mouse(MouseAction::Move(Some(left)), 4, 4, Mods::NONE);
        t.mouse(MouseAction::Move(Some(left)), 4, 4, Mods::NONE);
        t.mouse(MouseAction::Move(Some(left)), 4, 5, Mods::NONE);
        assert!(t.sent().ends_with(b"\x1b[<32;5;5M\x1b[<32;6;5M"), "{:?}", String::from_utf8_lossy(&t.sent()));
    }

    #[test]
    fn the_wheel_moves_the_history_or_becomes_arrows_or_reports() {
        let t = kept(10, 3);
        for i in 0..10 {
            t.feed(format!("line {i}\r\n").as_bytes());
        }
        t.wheel(2, 0, 0, Mods::NONE);
        assert_eq!(t.screen().display_offset, 2, "the history");
        assert_eq!(t.sent(), b"");
        t.wheel(-2, 0, 0, Mods::NONE);
        // a full-screen program in the alternate screen: arrows, one a line
        t.feed(b"\x1b[?1049h");
        t.wheel(3, 0, 0, Mods::NONE);
        assert_eq!(t.sent(), b"\x1b[A\x1b[A\x1b[A");
        // one that takes the mouse: one report a line
        t.feed(b"\x1b[?1000h\x1b[?1006h");
        t.wheel(-2, 1, 4, Mods::NONE);
        assert!(t.sent().ends_with(b"\x1b[<65;5;2M\x1b[<65;5;2M"));
    }

    #[test]
    fn a_slow_swipe_adds_up_and_option_scrolls_five_times_faster() {
        let t = kept(10, 3);
        for i in 0..100 {
            t.feed(format!("line {i}\r\n").as_bytes());
        }
        // three small trackpad moves of half a line make one line
        for _ in 0..3 {
            t.scroll_wheel(WheelDelta::Pixels(10.0), 20.0, 0, 0, Mods::NONE);
        }
        assert_eq!(t.screen().display_offset, 1);
        t.scroll_wheel(WheelDelta::Lines(2.0), 20.0, 0, 0, Mods::ALT);
        assert_eq!(t.screen().display_offset, 11, "Option: 2 lines times 5");
        t.scroll_wheel(WheelDelta::Lines(-1.0), 20.0, 0, 0, Mods::CTRL);
        assert_eq!(t.screen().display_offset, 10, "Ctrl over the history: as it is");
        // over a program that takes the mouse, Ctrl is fast too, and each line is one report with Ctrl in it
        t.feed(b"\x1b[?1049h\x1b[?1000h\x1b[?1006h");
        assert_eq!(t.scroll_wheel(WheelDelta::Lines(1.0), 20.0, 0, 0, Mods::CTRL), 5);
        assert_eq!(t.sent(), b"\x1b[<80;1;1M".repeat(5));
    }

    #[test]
    fn a_programs_hyperlink_covers_its_text_and_only_the_web_opens() {
        let t = kept(40, 3);
        t.feed(b"\x1b]8;;https://example.com/docs\x1b\\Docs here\x1b]8;;\x1b\\ plain\r\n");
        t.feed(b"\x1b]8;;file:///Applications/Calculator.app\x1b\\run me\x1b]8;;\x1b\\");
        let link = t.link_at(0, 2).expect("the hyperlink");
        assert_eq!(link, Link { url: "https://example.com/docs".into(), cells: vec![(0, 0, 8)] });
        assert_eq!(t.link_at(0, 12), None, "plain text");
        assert_eq!(t.link_at(1, 1), None, "a link to an app never opens");
    }

    #[test]
    fn a_web_address_in_the_text_is_a_link_also_when_it_wraps() {
        let t = kept(40, 3);
        t.feed(b"PR: https://github.com/acme/shop/pull/12.");
        let link = t.link_at(0, 10).expect("the address");
        assert_eq!(link.url, "https://github.com/acme/shop/pull/12", "without the full stop");
        assert_eq!(link.cells, [(0, 4, 39)]);
        assert_eq!(t.link_at(0, 40), None);
        assert_eq!(t.link_at(0, 1), None);
        let t = kept(20, 4);
        t.feed(b"go https://example.com/a/very/long/path end");
        let link = t.link_at(1, 3).expect("the wrapped address");
        assert_eq!(link.url, "https://example.com/a/very/long/path");
        assert_eq!(link.cells, [(0, 3, 19), (1, 0, 18)]);
        assert_eq!(t.link_at(0, 5), Some(link), "the same link from its first row");
        assert_eq!(t.link_at(2, 1), None);
    }

    #[test]
    fn a_paste_is_one_bracketed_block_when_the_program_asks() {
        let t = kept(20, 4);
        t.paste("a\nb");
        t.feed(b"\x1b[?2004h");
        t.paste("c\nd");
        assert_eq!(t.sent(), b"a\rb\x1b[200~c\rd\x1b[201~");
    }

    /// A program that runs elsewhere: what reaches it, and a gate that can hold its writes.
    #[derive(Clone, Default)]
    struct Elsewhere {
        typed: Arc<Mutex<Vec<u8>>>,
        sizes: Arc<Mutex<Vec<(u16, u16)>>>,
        gate: Arc<Mutex<()>>,
    }

    impl Program for Elsewhere {
        fn write(&mut self, bytes: &[u8]) -> bool {
            let _open = lock(&self.gate);
            lock(&self.typed).extend_from_slice(bytes);
            true
        }

        fn resize(&mut self, size: GridSize) {
            lock(&self.sizes).push((size.cols, size.rows));
        }
    }

    fn eventually(done: impl Fn() -> bool) -> bool {
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        while Instant::now() < deadline {
            if done() {
                return true;
            }
            thread::sleep(std::time::Duration::from_millis(2));
        }
        false
    }

    #[test]
    fn an_attached_terminal_shows_what_its_feed_brings_and_types_into_its_program() {
        let program = Elsewhere::default();
        let woken = Arc::new(AtomicU64::new(0));
        let wake: Wake = {
            let woken = woken.clone();
            Arc::new(move || {
                woken.fetch_add(1, Ordering::Relaxed);
            })
        };
        let (t, feed) = Terminal::attached(GridSize::new(30, 5), Palette::enkidu(), wake, program.clone()).unwrap();
        // the window takes the first frame; from then on, new output wakes it
        assert!(t.take_changed());
        // output from another thread, parsed on the terminal's own
        let from = feed.clone();
        thread::spawn(move || from.push(b"from the core\x1b[6n".to_vec())).join().unwrap();
        assert!(eventually(|| t.screen().row_text(0) == "from the core"));
        assert!(woken.load(Ordering::Relaxed) >= 1, "the window was woken");
        // the terminal's answer and what is typed go to the program
        t.send(b"ls\r".to_vec());
        assert!(eventually(|| *lock(&program.typed) == b"\x1b[1;14Rls\r"));
        t.resize(GridSize::new(40, 6));
        assert!(eventually(|| *lock(&program.sizes) == [(40, 6)]));
        feed.ended();
        assert!(t.has_exited());
    }

    #[test]
    fn a_program_that_stops_reading_never_blocks_the_window() {
        let program = Elsewhere::default();
        let (t, feed) =
            Terminal::attached(GridSize::new(30, 5), Palette::enkidu(), Arc::new(|| {}), program.clone()).unwrap();
        let held = lock(&program.gate);
        let started = Instant::now();
        for _ in 0..1000 {
            t.send(vec![b'x'; 4096]);
        }
        t.resize(GridSize::new(50, 8));
        feed.push(b"still drawing".to_vec());
        assert!(eventually(|| t.screen().row_text(0) == "still drawing"));
        let mut screen = Screen::default();
        t.update(&mut screen);
        assert!(started.elapsed() < std::time::Duration::from_secs(1), "{:?}", started.elapsed());
        drop(held);
        assert!(eventually(|| lock(&program.typed).len() == 1000 * 4096));
    }
}
