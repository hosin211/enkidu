//! The changes island (item 3.2d; DESIGN.md, "Layout"): the diffs of every file the workspace on screen has changed,
//! stacked in one scroll, under a band with their count and the switch between one column and side by side.
//!
//! The list of changed files comes from the changes store. Each file's diff is read off the drawing thread, once per
//! version of the file, three at a time with the file being read first, and kept (`enkidu_model::review::FileSet`), so
//! going back to a workspace shows its diffs at once. Each workspace has its own view state, so it keeps its place.

use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use enkidu_core::db::Remark;
use enkidu_core::git::Change;
use enkidu_core::names::in_tree_order;
use enkidu_core::state::Shared;
use enkidu_diff::{
    Diff, DiffAnnotation, DiffChangeMark, DiffColors, DiffDecoration, DiffEvent, DiffFile, DiffHunkSeparator,
    DiffLinePosition, DiffLineRange, DiffMark, DiffMode, DiffName, DiffSide, DiffState,
};
use enkidu_model::changes::ChangeLists;
use enkidu_model::frame::{Changes, Frame};
use enkidu_model::remarks as remark_logic;
use enkidu_model::review::{self, FileSet, Opened, Read, split_path};
use enkidu_model::search::Shown;
use enkidu_model::ui::{CodeColors, DiffLayout, Ui, code_row_height};
use enkidu_ui::pieces::{changes_seen, file_icon, island, line_under, ticking};
use enkidu_ui::segmented::{segment, segmented};
use enkidu_ui::tips::Tipped as _;
use enkidu_ui::tokens::{self as theme, alpha, hsla, tone};
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::input::{InputEvent, Position, TextareaState};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, ClipboardItem, Context, ElementId, Entity, Focusable as _, FontWeight, HighlightStyle,
    IntoElement, MouseButton, Render, Rgba, SharedString, Subscription, Window, div, px, svg,
};

use crate::bridge::Bridge;
use crate::opened_file;
use crate::remark_views::{self, Act, BarActs, BarView, CardView, ComposerView};
use crate::remarks::{Remarks, RemarksEvent};
use crate::toast;

/// How many diffs are read at once.
const AT_ONCE: usize = 3;

/// The key context of the changes island: its own keys work while the keyboard is in it (⌘↩ types the remarks into
/// Claude).
pub const KEYS: &str = "Changes";

/// Where the diffs are read from.
pub enum Source {
    /// The core: git, off the drawing thread.
    Core,
    /// Given patches by path, for pictures and screen tests without a core. A patch that is only
    /// [`STILL_READING`] stands for a read that hasn't come back: its diff, and the file when it is opened, are
    /// never there.
    Given(HashMap<String, String>),
}

/// In a window without a core: what is given as this is still being read, for as long as the window lives.
pub const STILL_READING: &str = "(still reading)";

/// One workspace's review: its view's state and what was read for its files.
struct Review {
    state: Entity<DiffState>,
    files: FileSet<DiffFile>,
    /// The list this follows: the store hands out the very same one until it changes.
    list: Option<Arc<Vec<Change>>>,
    /// What stands in the view for a file without lines to show (not read yet, binary, too large, unreadable): the
    /// same one each time, so the view knows it for the same file.
    stand_ins: HashMap<String, DiffFile>,
    /// The view hasn't been given the files as they are now.
    stale: bool,
    /// The workspace's remarks, each where its code is now.
    placed: Vec<Placed>,
    /// The remarks or the files changed since they were placed.
    unplaced: bool,
    /// The bar at the foot, while remarks wait to be sent.
    bar: Bar,
    /// The file opened in place of the diffs, while one is.
    opened: Option<OpenedFile>,
    /// The changed file that was last asked for (a click in the Changed files panel, "Show changes"): the panel
    /// marks it.
    focus: Option<String>,
    /// Its diff comes to the top of the view as the diffs are next drawn, and again as the diffs above it arrive
    /// (true once it was brought there a first time).
    go_to: Option<(String, bool)>,
    /// A changed file was just asked for: the diffs take the keyboard as they are next drawn (item 8.4).
    wants_keyboard: bool,
    /// Where ⌘-clicks were made, the last one last: ⌘[ goes back there (item 10.11). And where ⌘[ came from, for
    /// ⌘].
    trail: Vec<Spot>,
    ahead: Vec<Spot>,
}

/// A place a ⌘-click left: an opened file at a line, or the diffs at a changed file.
#[derive(Clone, Debug, PartialEq)]
enum Spot {
    File(String, usize),
    Diffs(String),
}

/// How many places a workspace keeps to go back to.
const TRAIL: usize = 30;

/// While a language's server can't be asked yet, the name the pointer rests on is asked about again after this
/// long, this many times at most (a server reads a big project for a second or two after its start).
const ASK_AGAIN_AFTER: std::time::Duration = std::time::Duration::from_millis(200);
const ASK_AGAIN: usize = 50;

/// The name ⌘ is held over: what the core found for it.
struct Pointed {
    workspace: String,
    name: DiffName,
    found: enkidu_core::goto::Found,
}

/// A file opened where the diffs are (item 3.7): read only, with its colors, and a remark on any of its lines. It is
/// drawn by the diff view as one file without changes, which shows every line once.
struct OpenedFile {
    path: String,
    /// The view's state. It is this file's own, so the diffs keep their place behind it.
    state: Entity<DiffState>,
    /// What reading it gave; nothing until the first answer is back (a read takes milliseconds).
    read: Option<Opened<DiffFile>>,
    /// The first read has taken long enough to notice: "Reading the file…" is said until it is back.
    slow: bool,
    /// The version that was read or is being read: its change's signature, empty for a file without changes. When
    /// Claude edits the file, this differs from the list's and the file is read again.
    version: Option<String>,
    /// The view hasn't been given the file as it is now.
    stale: bool,
    /// The line to bring into view once its lines are there.
    go_to: Option<usize>,
    /// The line it was opened at: it stays marked.
    target: Option<usize>,
    /// From Find in files: the line as it was shown there with its matches, to mark them in the real line.
    found: Option<Shown>,
    /// It was just opened: it takes the keyboard as it is first drawn (Escape leaves it, the arrows scroll it).
    wants_keyboard: bool,
    /// The remarks on its lines, each where its code is now.
    placed: Vec<Placed>,
    unplaced: bool,
    /// What differs from the last commit in the text on screen, as the view marks it beside the line numbers (item
    /// 10.3): read from the file's diff of that same version. Empty for a file without changes, an all new one, and
    /// until the diff is read.
    marks: Vec<DiffChangeMark>,
    /// The version of the text on screen (`read`): its marks are read from the diff of that same version.
    shown: Option<String>,
    /// A newer text that waits for its diff, so the text and its marks change in one frame: the version, and what
    /// reading it gave. Without this the marks of the old text would stand on the new one's lines for a moment.
    held: Option<(String, Opened<DiffFile>)>,
    _heard: Subscription,
}

/// What the diff of an opened file says of one version of it (item 10.3).
enum DiffOf {
    /// That version carries no marks: the file has no changes, or is all new.
    Unmarked,
    /// Its diff of that version isn't read yet.
    Waiting,
    /// Read: what differs from the last commit (nothing when the diff has no lines to pair: too large, binary,
    /// unreadable).
    Marks(Vec<DiffChangeMark>),
}

/// What the diff of the file at `path` says of its `version` (the signature its change had when the text was read).
fn diff_of(files: &FileSet<DiffFile>, path: &str, version: &str) -> DiffOf {
    let marked = files.change(path).is_some_and(|change| review::marks_its_changes(&change.kind));
    if !marked {
        return DiffOf::Unmarked;
    }
    match files.read_of(path) {
        Some((sig, Read::Lines(file))) if sig == version => DiffOf::Marks(file.change_marks()),
        Some((sig, _)) if sig == version => DiffOf::Marks(Vec::new()),
        _ => DiffOf::Waiting,
    }
}

impl OpenedFile {
    /// A text of the file comes on screen, with what its diff says of that version.
    fn show(&mut self, version: &str, read: Opened<DiffFile>, said: DiffOf) {
        self.marks = match said {
            DiffOf::Marks(marks) => marks,
            DiffOf::Unmarked | DiffOf::Waiting => Vec::new(),
        };
        self.shown = Some(version.to_string());
        self.held = None;
        self.read = Some(read);
        self.slow = false;
        (self.stale, self.unplaced) = (true, true);
    }

    /// The file's diff was read: a newer text that waited for it comes on screen with its marks; else the text on
    /// screen gets its marks, when the diff is of its version.
    fn diff_was_read(&mut self, files: &FileSet<DiffFile>) {
        if let Some((version, _)) = &self.held {
            let said = diff_of(files, &self.path, version);
            if !matches!(said, DiffOf::Waiting)
                && let Some((version, read)) = self.held.take()
            {
                self.show(&version, read, said);
            }
            return;
        }
        let Some(version) = self.shown.clone() else {
            return;
        };
        if let DiffOf::Marks(marks) = diff_of(files, &self.path, &version) {
            self.marks = marks;
        }
    }
}

impl Review {
    /// The view state on screen: the opened file's while one is open, else the diffs'.
    fn on_screen(&self) -> &Entity<DiffState> {
        self.opened.as_ref().map_or(&self.state, |opened| &opened.state)
    }

    /// The remarks as they stand in the view on screen.
    fn placed_on_screen(&self) -> &[Placed] {
        self.opened.as_ref().map_or(&self.placed, |opened| &opened.placed)
    }

    /// The file at `path` with its lines, when the view on screen shows them.
    fn shown_file(&self, path: &str) -> Option<&DiffFile> {
        match &self.opened {
            Some(opened) => match &opened.read {
                Some(Opened::Text(file, _)) if opened.path == path => Some(file),
                _ => None,
            },
            None => self.files.files().find(|(change, _)| change.path == path).and_then(|(_, read)| match read {
                Some(Read::Lines(file)) => Some(file),
                _ => None,
            }),
        }
    }
}

/// The state of the bar that sends a workspace's remarks to Claude (item 3.6).
#[derive(Default)]
struct Bar {
    /// "Add note" was pressed: a note for Claude goes with the remarks.
    note_open: bool,
    /// Where the note is typed. A text field needs its window, so it is made as the island is next drawn.
    note: Option<(Entity<TextareaState>, Subscription)>,
    /// "Discard" was pressed: Keep them, or Delete them.
    confirming: bool,
}

/// A remark where it stands now: files change while Claude works, so its lines may have moved.
#[derive(Clone)]
struct Placed {
    remark: Remark,
    side: DiffSide,
    start: usize,
    end: usize,
    /// The code it was written on is no longer in the file.
    outdated: bool,
}

/// A remark being written under its lines: a new one, or one whose words are being changed.
struct Writing {
    workspace: String,
    path: String,
    side: DiffSide,
    start: usize,
    end: usize,
    /// The remark whose words are being changed; none for a new one.
    editing: Option<Remark>,
    /// Where it is typed. A text field needs its window, so it is made as the island is next drawn.
    input: Option<(Entity<TextareaState>, Subscription)>,
    /// The id it was sent to the database under, until the answer is back.
    saving: Option<String>,
}

pub struct ChangesIsland {
    /// The colors the code on hand was drawn in: when Settings names others, it is all colored again.
    code_colors: CodeColors,
    frame: Entity<Frame>,
    lists: Entity<ChangeLists>,
    ui: Entity<Ui>,
    remarks: Entity<Remarks>,
    source: Source,
    reviews: HashMap<String, Review>,
    writing: Option<Writing>,
    /// Where the files that are opened are remembered (Search offers them before anything is typed).
    opened_lately: Option<Entity<crate::search::SearchStore>>,
    /// The name ⌘ is held over, once the core said where it is defined (item 10.11), and the count of the asks:
    /// an answer for a name the pointer has left is dropped.
    pointed: Option<Pointed>,
    link_turn: u64,
    /// The workspace whose language server was last started ahead of any question (once each time it comes on
    /// screen with changed code).
    prepared: Option<String>,
    /// A name with several definitions was clicked: the list to pick from opens as the island is next drawn.
    choosing: Option<crate::definitions::Choosing>,
    _watching: Vec<Subscription>,
}

/// The files that have remarks and no changes, by path (item 3.5d): a remark left on an opened file, or one whose
/// file's
/// changes were committed since.
fn unchanged_with_remarks(remarks: &[Remark], files: &FileSet<DiffFile>) -> Vec<String> {
    let mut paths: Vec<String> = remarks
        .iter()
        .filter(|remark| files.change(&remark.path).is_none())
        .map(|remark| remark.path.clone())
        .collect();
    paths.sort_unstable();
    paths.dedup();
    paths
}

/// Every line of a side of a file, when the file has them all from the first one on (a diff with the whole file in it
/// does; a stand-in for a file that isn't read has none).
pub(crate) fn whole_side(file: &DiffFile, side: DiffSide) -> Option<Vec<&str>> {
    let lines: Vec<(usize, &str)> = file.side_lines(side).collect();
    lines
        .iter()
        .enumerate()
        .all(|(ix, (number, _))| *number == ix + 1)
        .then(|| lines.into_iter().map(|(_, text)| text).collect())
}

/// Where a remark stands in a file whose lines on its side are `lines` (none when the file can't be read): see
/// `enkidu_model::remarks::locate`.
fn placed_at(remark: &Remark, lines: Option<&[&str]>) -> Placed {
    let at = remark_logic::locate(remark, lines);
    Placed {
        remark: remark.clone(),
        side: side_of(remark),
        start: at.start_line.max(1) as usize,
        end: at.end_line.max(1) as usize,
        outdated: at.outdated,
    }
}

fn side_of(remark: &Remark) -> DiffSide {
    if remark.side == "old" { DiffSide::Original } else { DiffSide::Modified }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_millis() as i64)
}

/// The view's colors, from DESIGN.md's tokens ("Diff, derived from the lamps so the window reads as one system").
/// The marks beside the lines of an opened file that differ from the last commit (item 10.2) follow the code's
/// colors: an IDE's scheme brings its own (PyCharm's green, blue and gray); with the quiet colors they are the
/// tones of the app's own marks for a new, a changed and a deleted file.
pub(crate) fn colors(code: CodeColors) -> DiffColors {
    let own = |kind: &str| {
        let (hue, chroma) = review::kind_hue(kind);
        hsla(tone(hue, chroma))
    };
    let of = |(r, g, b, a): enkidu_syntax::Rgba| {
        hsla(Rgba { r: f32::from(r) / 255., g: f32::from(g) / 255., b: f32::from(b) / 255., a: f32::from(a) / 255. })
    };
    let [mark_added, mark_changed, mark_removed] = match enkidu_syntax::change_marks(scheme(code)) {
        Some(marks) => marks.map(of),
        None => [own("added"), own("modified"), own("deleted")],
    };
    DiffColors {
        mark_added,
        mark_changed,
        mark_removed,
        // a name that leads to its definition, while ⌘ is held over it (item 10.11)
        link: hsla(theme::ACCENT),
        header: hsla(theme::PANE),
        rule: hsla(theme::LINE),
        added: hsla(theme::ADD_BG),
        added_word: hsla(theme::ADD_WORD),
        added_sign: hsla(theme::ADD_INK),
        removed: hsla(theme::DEL_BG),
        removed_word: hsla(theme::DEL_WORD),
        removed_sign: hsla(theme::DEL_INK),
        gap: hsla(theme::GAP_BG),
        selected: hsla(theme::REMARK_BAND),
        note: hsla(theme::WELL),
        // the plus that starts a remark, in the remarks' blue
        add_note: hsla(theme::ACCENT_STRONG),
        add_note_sign: hsla(theme::white(1.0)),
    }
}

/// The scheme that Settings' choice of code colors names.
fn scheme(colors: CodeColors) -> enkidu_syntax::Scheme {
    match colors {
        CodeColors::Ide => enkidu_syntax::Scheme::Ide,
        CodeColors::Quiet => enkidu_syntax::Scheme::Quiet,
    }
}

/// The colors of a part of a file, for the view, which asks on its background thread: from `enkidu-syntax`, in the
/// scheme Settings names (DESIGN.md, "Color": an IDE's colors, or the quiet ones).
pub(crate) fn syntax_colors(colors: CodeColors, language: &str, text: &str) -> Vec<(Range<usize>, HighlightStyle)> {
    enkidu_syntax::colors(scheme(colors), language, text)
        .iter()
        .map(|(range, (r, g, b, a))| {
            let color = Rgba {
                r: f32::from(*r) / 255.,
                g: f32::from(*g) / 255.,
                b: f32::from(*b) / 255.,
                a: f32::from(*a) / 255.,
            };
            (range.clone(), HighlightStyle { color: Some(hsla(color)), ..Default::default() })
        })
        .collect()
}

/// A file's diff, read and parsed. Runs off the drawing thread.
pub(crate) fn read_patch(shared: &Arc<Shared>, workspace: String, change: &Change) -> Read<DiffFile> {
    let untracked = change.kind == "untracked";
    let patch = enkidu_core::commands::get_file_patch(
        shared,
        workspace,
        change.path.clone(),
        change.old_path.clone(),
        untracked,
    );
    match patch {
        Err(err) => Read::Failed(err.message),
        Ok(diff) if diff.binary => Read::Binary,
        Ok(diff) if diff.too_large => Read::TooLarge,
        Ok(diff) => parse(&change.path, &diff.diff),
    }
}

fn parse(path: &str, patch: &str) -> Read<DiffFile> {
    match DiffFile::parse(patch) {
        // one file's patch holds one file; nothing in it means nothing differs any more
        Ok(files) => Read::Lines(files.into_iter().next().unwrap_or_else(|| DiffFile::without_lines(path.to_string()))),
        Err(err) => Read::Failed(err.to_string()),
    }
}

/// A whole file, read to be opened. Runs off the drawing thread.
pub(crate) fn read_whole(shared: &Arc<Shared>, workspace: String, path: &str) -> Opened<DiffFile> {
    match enkidu_core::commands::read_file(shared, workspace, path.to_string()) {
        Err(err) => Opened::Failed(err.message),
        Ok(file) if file.missing => Opened::Missing,
        Ok(file) if file.binary => Opened::Binary { size: file.size },
        Ok(file) if file.too_large => Opened::TooLarge { size: file.size },
        Ok(file) => whole(path, &file.text.unwrap_or_default()),
    }
}

/// A file's text as the view shows it: every line once, with its number.
pub(crate) fn whole(path: &str, text: &str) -> Opened<DiffFile> {
    match text.lines().count() {
        0 => Opened::Empty,
        lines => Opened::Text(DiffFile::unchanged(path.to_string(), text), lines),
    }
}

/// An opened file that isn't read after this long says "Reading the file…"; a quick read goes from nothing straight
/// to the file (DESIGN.md, principle 5).
const READ_IS_SLOW: std::time::Duration = std::time::Duration::from_millis(400);

/// What a file without lines says under its header.
#[derive(Clone, Debug, PartialEq)]
enum Said {
    /// Its diff is being read: nothing to say yet (a read takes milliseconds).
    Reading,
    Words(&'static str),
    /// It has no changes and is listed for its remarks: these, in line order.
    Remarks(Vec<ElementId>),
    /// Git's own words for why the diff couldn't be read.
    Failed(String),
}

impl ChangesIsland {
    pub fn new(
        frame: Entity<Frame>,
        lists: Entity<ChangeLists>,
        ui: Entity<Ui>,
        remarks: Entity<Remarks>,
        source: Source,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> ChangesIsland {
        cx.observe(&frame, |this, _, cx| this.follow(cx)).detach();
        cx.observe(&lists, |this, _, cx| this.follow(cx)).detach();
        // code is colored in the scheme that was left (item 8.3), and again when Settings picks the other one
        cx.observe(&ui, |this, ui, cx| {
            this.follow_code_colors(ui.read(cx).code_colors, cx);
            cx.notify();
        })
        .detach();
        // the remarks changed: each is placed again where its code is, and the files that are listed only for
        // their remarks may be others now
        cx.observe(&remarks, |this, remarks, cx| {
            let remarks = remarks.read(cx);
            for (ws, review) in &mut this.reviews {
                (review.unplaced, review.stale) = (true, true);
                if let Some(opened) = &mut review.opened {
                    opened.unplaced = true;
                }
                // no remark is left to send: the bar goes, and what it held with it
                if remarks.of(ws).is_empty() {
                    review.bar = Bar::default();
                }
            }
            // a remark whose words were being changed is gone (sent to Claude, or discarded): so is its composer
            let gone = this.writing.as_ref().is_some_and(|writing| {
                (writing.editing.as_ref())
                    .is_some_and(|edited| !remarks.of(&writing.workspace).iter().any(|known| known.id == edited.id))
            });
            if gone {
                this.writing = None;
            }
            cx.notify();
        })
        .detach();
        let saved = cx.subscribe_in(&remarks, window, |this, _, event: &RemarksEvent, window, cx| {
            this.remark_was_saved(event, window, cx)
        });
        // the grammars and the theme are read now, in the background, so the first diff doesn't wait for them
        cx.background_spawn(async { enkidu_syntax::warm() }).detach();
        let code_colors = ui.read(cx).code_colors;
        let mut island = ChangesIsland {
            frame,
            lists,
            ui,
            code_colors,
            remarks,
            source,
            reviews: HashMap::new(),
            writing: None,
            opened_lately: None,
            pointed: None,
            link_turn: 0,
            prepared: None,
            choosing: None,
            _watching: vec![saved],
        };
        island.follow(cx);
        island
    }

    /// The workspace on screen.
    fn workspace(&self, cx: &App) -> Option<String> {
        self.frame.read(cx).current_workspace().map(|w| w.id.clone())
    }

    /// Whether the workspace on screen has something to review. With nothing, the island isn't shown and Claude's
    /// terminal has the whole workspace (DESIGN.md: "Nothing to review, nothing shown").
    pub fn reviewing(&self, cx: &App) -> bool {
        self.workspace(cx).is_some_and(|ws| {
            // a remark that wasn't sent keeps the island there, with or without changes; so does an opened file
            self.lists.read(cx).of(&ws).is_some_and(|list| !list.is_empty())
                || !self.remarks.read(cx).of(&ws).is_empty()
                || self.reviews.get(&ws).is_some_and(|review| review.opened.is_some())
        })
    }

    /// Whether the keyboard is in this island: in the lines on screen, in a remark being written, or in the note
    /// that goes with the remarks.
    pub fn has_keyboard(&self, window: &Window, cx: &App) -> bool {
        let Some(review) = self.workspace(cx).and_then(|ws| self.reviews.get(&ws)) else {
            return false;
        };
        let typing = |field: Option<&Entity<TextareaState>>| {
            field.is_some_and(|field| field.read(cx).focus_handle(cx).contains_focused(window, cx))
        };
        review.on_screen().read(cx).focus_handle(cx).contains_focused(window, cx)
            || typing(self.writing.as_ref().and_then(|writing| writing.input.as_ref()).map(|(input, _)| input))
            || typing(review.bar.note.as_ref().map(|(note, _)| note))
    }

    /// The lines on screen take the keyboard (the terminal left the screen while it had it).
    pub fn take_keyboard(&self, window: &mut Window, cx: &mut App) {
        if let Some(review) = self.workspace(cx).and_then(|ws| self.reviews.get(&ws)) {
            let lines = review.on_screen().read(cx).focus_handle(cx);
            window.focus(&lines, cx);
        }
    }

    /// The remarks store this island draws from.
    pub fn remarks(&self) -> &Entity<Remarks> {
        &self.remarks
    }

    /// The view of the code that is on screen: the opened file's, else the diffs'.
    pub fn code_on_screen(&self, cx: &App) -> Option<Entity<DiffState>> {
        self.reviews.get(&self.workspace(cx)?).map(|review| review.on_screen().clone())
    }

    /// The view state of the workspace on screen, for tests.
    pub fn diff(&self, cx: &App) -> Option<Entity<DiffState>> {
        self.reviews.get(&self.workspace(cx)?).map(|review| review.state.clone())
    }

    /// The store's list for the workspace on screen changed, or another workspace came on screen: follow it, and
    /// read what isn't read.
    fn follow(&mut self, cx: &mut Context<Self>) {
        // a removed workspace's review goes with it
        let frame = self.frame.read(cx);
        self.reviews.retain(|id, _| frame.workspaces.iter().any(|w| &w.id == id));
        let Some(ws) = self.workspace(cx) else {
            return;
        };
        // what git has said of this workspace so far (nothing yet, right after a first launch)
        let list = self.lists.read(cx).of(&ws).cloned();
        let layout = self.ui.read(cx).diff_layout;
        let colors = self.code_colors;
        if !self.reviews.contains_key(&ws) {
            let state = cx.new(|cx| {
                DiffState::new([], cx)
                    .with_side_names(true)
                    .with_mode(mode(layout))
                    .with_syntax_highlighter(move |language, text| syntax_colors(colors, language, text))
            });
            // a press on a line number, a drag over several, or Shift and a click: a remark on those lines
            cx.subscribe(&state, |this, state, event: &DiffEvent, cx| this.heard(state, event, cx)).detach();
            let review = Review {
                state,
                files: FileSet::default(),
                list: None,
                stand_ins: HashMap::new(),
                stale: false,
                placed: Vec::new(),
                unplaced: true,
                bar: Bar::default(),
                opened: None,
                focus: None,
                go_to: None,
                wants_keyboard: false,
                trail: Vec::new(),
                ahead: Vec::new(),
            };
            self.reviews.insert(ws.clone(), review);
        }
        let (Some(review), Some(list)) = (self.reviews.get_mut(&ws), list) else {
            return;
        };
        if review.list.as_ref().is_some_and(|known| Arc::ptr_eq(known, &list)) {
            return;
        }
        review.files.follow(&list);
        review.list = Some(list);
        review.stale = true;
        // its language server is started now, at work on a changed file, so it is there by the first ⌘
        if self.prepared.as_deref() != Some(ws.as_str()) {
            let python = review.files.files().map(|(change, _)| change.path.clone()).find(|path| path.ends_with(".py"));
            if let Some(path) = python {
                self.prepared = Some(ws.clone());
                self.prepare_server(&ws, &path, cx);
            }
        }
        self.read_more(&ws, cx);
        // the opened file may be another version now
        self.read_opened(&ws, cx);
        cx.notify();
    }

    /// The lines whose numbers were pressed get a remark: the composer opens under them. While a new remark is
    /// being written, it moves to the newly chosen lines with what was typed.
    fn write_on(&mut self, range: &DiffLineRange, cx: &mut Context<Self>) {
        let Some(ws) = self.workspace(cx) else {
            return;
        };
        // a remark is on one side: a drag from a removed line to an added one (in one column) is on where it ended
        let (side, start, end) = if range.side() == range.end_side() {
            (range.side(), range.start().min(range.end()), range.start().max(range.end()))
        } else {
            (range.end_side(), range.end(), range.end())
        };
        let path = range.path().to_string();
        match &mut self.writing {
            Some(writing) if writing.editing.is_none() && writing.workspace == ws && writing.saving.is_none() => {
                (writing.path, writing.side, writing.start, writing.end) = (path.clone(), side, start, end);
            }
            // a remark whose words are being changed keeps its lines
            Some(_) => return,
            None => {
                self.writing = Some(Writing {
                    workspace: ws.clone(),
                    path: path.clone(),
                    side,
                    start,
                    end,
                    editing: None,
                    input: None,
                    saving: None,
                });
            }
        }
        if let Some(review) = self.reviews.get(&ws) {
            let lines = DiffLineRange::new(path, side, start, end);
            review.on_screen().update(cx, |state, cx| state.set_selected_lines(Some(lines), cx));
        }
        cx.notify();
    }

    /// The remark being written goes to the database; the composer closes when the store has it.
    fn save_writing(&mut self, cx: &mut Context<Self>) {
        let Some(writing) = &mut self.writing else {
            return;
        };
        let Some((input, _)) = &writing.input else {
            return;
        };
        let body = input.read(cx).value().trim().to_string();
        if body.is_empty() || writing.saving.is_some() {
            return;
        }
        let remark = match &writing.editing {
            Some(remark) => Remark { body, ..remark.clone() },
            None => {
                // the code the remark is about, as it is now: it is how the remark finds its lines again
                let file =
                    self.reviews.get(&writing.workspace).and_then(|review| review.shown_file(&writing.path)).cloned();
                let snippet: Vec<&str> = file
                    .iter()
                    .flat_map(|file| file.side_lines(writing.side))
                    .filter(|(line, _)| (writing.start..=writing.end).contains(line))
                    .map(|(_, text)| text)
                    .collect();
                let now = now_ms();
                Remark {
                    id: remark_logic::new_id(now),
                    workspace_id: writing.workspace.clone(),
                    path: writing.path.clone(),
                    side: if writing.side == DiffSide::Original { "old" } else { "new" }.into(),
                    start_line: writing.start as i64,
                    end_line: writing.end as i64,
                    snippet: snippet.join("\n"),
                    body,
                    created_at: now,
                }
            }
        };
        writing.saving = Some(remark.id.clone());
        self.remarks.update(cx, |remarks, cx| remarks.save(remark, cx));
        cx.notify();
    }

    /// The store has the remark that was being written, or the database refused it.
    fn remark_was_saved(&mut self, event: &RemarksEvent, window: &mut Window, cx: &mut Context<Self>) {
        let saving = self.writing.as_ref().and_then(|writing| writing.saving.clone());
        match event {
            RemarksEvent::Saved(id) if saving.as_ref() == Some(id) => self.stop_writing(window, cx),
            RemarksEvent::Failed { id, why } if saving.as_ref() == Some(id) => {
                // what was typed stays in the composer
                if let Some(writing) = &mut self.writing {
                    writing.saving = None;
                }
                toast::warning(window, "Couldn't save the remark", why.clone(), cx);
                cx.notify();
            }
            // the remarks are in Claude's prompt, and Claude's terminal has the keyboard: Enter there sends them
            RemarksEvent::Typed { workspace, count } => {
                // a note went with them; a remark written meanwhile keeps the bar, without it
                if let Some(review) = self.reviews.get_mut(workspace) {
                    review.bar = Bar::default();
                }
                // and Claude's terminal is on screen, also when the changes had the whole workspace
                self.ui.update(cx, |ui, cx| {
                    if ui.reveal_terminal() {
                        cx.notify();
                    }
                });
                toast::done(window, remark_logic::typed_words(*count), "Press Enter there to send them.", cx);
                cx.notify();
            }
            // nothing was lost: the remarks and the note are where they were
            RemarksEvent::NotTyped { why, .. } => {
                toast::warning(window, "Couldn't send the remarks", why.clone(), cx);
                cx.notify();
            }
            _ => {}
        }
    }

    /// The remarks of the workspace on screen go into Claude's prompt as one message, with the bar's note after
    /// them; you press Enter there to send it (the bar's blue button, ⌘↩ in the changes, the palette).
    pub fn send_remarks(&mut self, cx: &mut Context<Self>) {
        let Some(ws) = self.workspace(cx) else {
            return;
        };
        let note = (self.reviews.get(&ws))
            .and_then(|review| review.bar.note.as_ref())
            .map(|(note, _)| note.read(cx).value().to_string());
        self.remarks.update(cx, |remarks, cx| remarks.send(&ws, note.as_deref(), cx));
    }

    /// What the bar on screen holds changes: `change` says how.
    fn with_bar(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut Bar)) {
        let Some(ws) = self.workspace(cx) else {
            return;
        };
        if let Some(review) = self.reviews.get_mut(&ws) {
            change(&mut review.bar);
            cx.notify();
        }
    }

    /// The note's field closes; the keyboard goes back to the diff.
    fn close_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.with_bar(cx, |bar| (bar.note_open, bar.note) = (false, None));
        if let Some(review) = self.workspace(cx).and_then(|ws| self.reviews.get(&ws)) {
            window.focus(&review.on_screen().read(cx).focus_handle(cx), cx);
        }
    }

    /// Every remark of the workspace on screen is deleted, unsent ("Discard", then "Delete 3").
    fn discard_remarks(&mut self, cx: &mut Context<Self>) {
        let Some(ws) = self.workspace(cx) else {
            return;
        };
        self.remarks.update(cx, |remarks, cx| remarks.clear(&ws, cx));
    }

    /// What a view of the code said: lines to remark on, the name ⌘ is held over, a link that was clicked.
    fn heard(&mut self, state: Entity<DiffState>, event: &DiffEvent, cx: &mut Context<Self>) {
        match event {
            // a press on a line number, a drag over several, or Shift and a click: a remark on those lines
            DiffEvent::SelectionEnded(range) => self.write_on(range, cx),
            DiffEvent::NamePointed(name) => self.name_pointed(state, name.clone(), cx),
            DiffEvent::NameClicked(name) => self.name_clicked(name, cx),
            _ => {}
        }
    }

    // Going to a definition (item 10.11)

    /// With ⌘ held, the pointer came over a name of the code on screen, or left it. The core is asked where that
    /// name is defined (off the drawing thread), and the view is told when it leads somewhere: the name is a link
    /// then. A removed line is of a file that was: only the code as it is now is asked about.
    fn name_pointed(&mut self, state: Entity<DiffState>, name: Option<DiffName>, cx: &mut Context<Self>) {
        self.link_turn += 1;
        self.pointed = None;
        let (Some(name), Some(ws)) = (name, self.workspace(cx)) else {
            return;
        };
        if name.position.side() != DiffSide::Modified || !matches!(self.source, Source::Core) {
            return;
        }
        let turn = self.link_turn;
        self.ask_where(state, name, ws, turn, 0, cx);
    }

    /// Asks the core where a name is defined, and makes it a link when it is somewhere. `turn` is the pointing this
    /// belongs to: an answer for a name the pointer has left is dropped. When the language's server couldn't be
    /// asked yet (it is starting, or at work on an earlier question) the same name is asked about again in a
    /// moment, `tries` times at most, so a name the pointer rests on becomes a link as soon as the server is there.
    fn ask_where(
        &mut self,
        state: Entity<DiffState>,
        name: DiffName,
        ws: String,
        turn: u64,
        tries: usize,
        cx: &mut Context<Self>,
    ) {
        let ask = enkidu_core::goto::Ask {
            path: name.position.path().to_string(),
            line: name.position.line(),
            column: name.column,
        };
        let (workspace, island) = (ws.clone(), cx.weak_entity());
        Bridge::call(
            cx,
            move |shared| enkidu_core::commands::go_to_definition(shared, workspace, ask),
            move |found, cx| {
                let _ = island.update(cx, |island, cx| {
                    // the pointer is on another name by now, or on none
                    if island.link_turn != turn {
                        return;
                    }
                    let Ok(Some(found)) = found else { return };
                    if found.later {
                        if tries < ASK_AGAIN {
                            cx.spawn(async move |island, cx| {
                                cx.background_executor().timer(ASK_AGAIN_AFTER).await;
                                let _ = island.update(cx, |island, cx| {
                                    if island.link_turn == turn {
                                        island.ask_where(state, name, ws, turn, tries + 1, cx);
                                    }
                                });
                            })
                            .detach();
                        }
                        return;
                    }
                    if found.places.is_empty() {
                        return;
                    }
                    let link = DiffName {
                        position: name.position.clone(),
                        range: found.start..found.end,
                        column: name.column,
                    };
                    state.update(cx, |state, cx| state.set_link(Some(link.clone()), cx));
                    island.pointed = Some(Pointed { workspace: ws, name: link, found });
                });
            },
        );
    }

    /// A link was clicked with ⌘ held: one definition is gone to; of several, the list to pick from opens.
    fn name_clicked(&mut self, name: &DiffName, cx: &mut Context<Self>) {
        let known = |pointed: &Pointed| pointed.name.position == name.position && pointed.name.range == name.range;
        let Some(pointed) = self.pointed.take().filter(known) else {
            return;
        };
        let from = (name.position.path().to_string(), name.position.line());
        match pointed.found.places.as_slice() {
            [] => {}
            // the one place the language's server names: straight there
            [place] => self.go_to(&pointed.workspace, place.clone(), from, cx),
            // several: the list, each with its path, to pick from
            _ => {
                // a dialog needs its window: it opens as the island is next drawn
                self.choosing =
                    Some(crate::definitions::Choosing { workspace: pointed.workspace, from, found: pointed.found });
                cx.notify();
            }
        }
    }

    /// Goes to a definition in the workspace on screen: its file opens at its line. The place the click was made
    /// (`from`: the file and the line) is kept for the way back.
    pub fn go_to(&mut self, ws: &str, place: enkidu_core::goto::Place, from: (String, usize), cx: &mut Context<Self>) {
        if self.workspace(cx).as_deref() != Some(ws) {
            return;
        }
        let Some(review) = self.reviews.get_mut(ws) else {
            return;
        };
        let left = match &review.opened {
            Some(opened) => Spot::File(opened.path.clone(), from.1),
            None => Spot::Diffs(from.0),
        };
        review.trail.push(left);
        if review.trail.len() > TRAIL {
            review.trail.remove(0);
        }
        review.ahead.clear();
        self.open_at(&place.path, Some(place.line as usize), None, cx);
    }

    /// ⌘[: back to where the last ⌘-click was made. False when there is nowhere to go back to.
    pub fn go_back(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.step(false, window, cx)
    }

    /// ⌘]: forward again, after a step back.
    pub fn go_forward(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.step(true, window, cx)
    }

    fn step(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(review) = self.workspace(cx).and_then(|ws| self.reviews.get_mut(&ws)) else {
            return false;
        };
        let (from, to) =
            if forward { (&mut review.ahead, &mut review.trail) } else { (&mut review.trail, &mut review.ahead) };
        let Some(spot) = from.pop() else {
            return false;
        };
        // where you are now is on the other side
        if let Some(opened) = &review.opened {
            to.push(Spot::File(opened.path.clone(), opened.target.unwrap_or(1)));
        }
        match spot {
            Spot::File(path, line) => self.open_at(&path, Some(line), None, cx),
            // the diffs, at the file the click was in
            Spot::Diffs(path) => self.focus_change(&path, window, cx),
        }
        true
    }

    /// Where ⌘[ would go from the workspace on screen: the file's name, or the changes.
    fn back_to(&self, cx: &App) -> Option<String> {
        let review = self.reviews.get(&self.workspace(cx)?)?;
        review.trail.last().map(|spot| match spot {
            Spot::File(path, _) => split_path(path).1.to_string(),
            Spot::Diffs(_) => "the changes".to_string(),
        })
    }

    /// ⌘ went down or up while the pointer rests at `pointer`: the code on screen is told, so a name under the
    /// pointer becomes a link (or stops being one) without the pointer moving. The window tells this, because a
    /// change of the keys is told to what has the keyboard, which is often Claude's terminal.
    pub fn keys_changed(&mut self, held: bool, pointer: gpui_kit::Point<gpui_kit::Pixels>, cx: &mut Context<Self>) {
        let Some(review) = self.workspace(cx).and_then(|ws| self.reviews.get(&ws)) else {
            return;
        };
        review.on_screen().clone().update(cx, |state, cx| state.point_with(held, pointer, cx));
    }

    /// Opens a file of the workspace on screen where the diffs are, read only, at `line` when one is given (a
    /// diff's header, "Open file" on a remark's file; the Files panel and Search). The diffs keep their place
    /// behind it.
    pub fn open_file(&mut self, path: &str, line: Option<usize>, cx: &mut Context<Self>) {
        self.open_at(path, line, None, cx);
    }

    /// A file was picked: the panel that stood over the work (the lists are hidden) has done its job and goes.
    fn files_picked(&mut self, cx: &mut Context<Self>) {
        self.ui.update(cx, |ui, cx| {
            if ui.close_files_over() {
                cx.notify();
            }
        });
    }

    /// The files that are opened are remembered there.
    pub fn remember_in(&mut self, store: Entity<crate::search::SearchStore>) {
        self.opened_lately = Some(store);
    }

    /// The same, from Find in files: `found` is the line as the popup showed it, so its matches are marked in the
    /// file too.
    pub fn open_at(&mut self, path: &str, line: Option<usize>, found: Option<Shown>, cx: &mut Context<Self>) {
        let Some(ws) = self.workspace(cx) else {
            return;
        };
        self.files_picked(cx);
        if !self.reviews.contains_key(&ws) {
            return;
        }
        // the file that is open already: only the line moves (a ⌘-click on a name the file itself defines)
        if let Some(opened) = (self.reviews.get_mut(&ws))
            .and_then(|review| review.opened.as_mut())
            .filter(|opened| opened.path == path && opened.read.is_some())
        {
            (opened.target, opened.found) = (line, found);
            if let Some(line) = line {
                let at = DiffLinePosition::new(path.to_string(), DiffSide::Modified, line);
                opened.state.update(cx, |state, cx| state.scroll_to_line_centered(at, cx));
            }
            cx.notify();
            return;
        }
        if let Some(store) = &self.opened_lately {
            store.update(cx, |store, cx| {
                if store.recent.opened(&ws, path) {
                    cx.notify();
                }
            });
        }
        let colors = self.code_colors;
        let state = cx.new(|cx| {
            DiffState::new([], cx).with_syntax_highlighter(move |language, text| syntax_colors(colors, language, text))
        });
        // a press on a line number, a drag over several, or Shift and a click: a remark on those lines
        let heard = cx.subscribe(&state, |this, state, event: &DiffEvent, cx| this.heard(state, event, cx));
        // a remark that was being written in the view the file covers closes with it
        if self.writing.as_ref().is_some_and(|writing| writing.workspace == ws) {
            self.writing = None;
        }
        // the file shows where the changes show, so they have to be on screen
        self.reveal(cx);
        if let Some(review) = self.reviews.get_mut(&ws) {
            review.state.update(cx, |state, cx| state.set_selected_lines(None, cx));
            review.opened = Some(OpenedFile {
                path: path.to_string(),
                state,
                read: None,
                slow: false,
                version: None,
                stale: false,
                go_to: line,
                target: line,
                found,
                wants_keyboard: true,
                placed: Vec::new(),
                unplaced: true,
                marks: Vec::new(),
                shown: None,
                held: None,
                _heard: heard,
            });
        }
        self.read_opened(&ws, cx);
        self.prepare_server(&ws, path, cx);
        cx.notify();
    }

    /// A file of a language that has a server comes on screen: the workspace's server is started now, so it has
    /// read what it needs by the time a name is asked about (the core does nothing when one runs already, and for
    /// a language without a server).
    fn prepare_server(&self, ws: &str, path: &str, cx: &mut Context<Self>) {
        if !matches!(self.source, Source::Core) {
            return;
        }
        let (workspace, path) = (ws.to_string(), path.to_string());
        Bridge::call(cx, move |shared| enkidu_core::commands::prepare_definitions(shared, workspace, path), |_, _| {});
    }

    /// The changes come back on screen when they were put away (the terminal alone, ⌥⌘B).
    fn reveal(&mut self, cx: &mut Context<Self>) {
        self.ui.update(cx, |ui, cx| {
            if ui.reveal_review() {
                cx.notify();
            }
        });
    }

    /// The file opened in the workspace on screen, if one is.
    pub fn opened_file(&self, cx: &App) -> Option<String> {
        let review = self.reviews.get(&self.workspace(cx)?)?;
        review.opened.as_ref().map(|opened| opened.path.clone())
    }

    /// The opened file closes: the diffs are back where they were (or, with nothing to review, Claude has the
    /// room again). False when no file was open.
    pub fn close_file(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(ws) = self.workspace(cx) else {
            return false;
        };
        let Some(review) = self.reviews.get_mut(&ws) else {
            return false;
        };
        if review.opened.take().is_none() {
            return false;
        }
        // the way back and forward was through this file: it ends with it
        review.trail.clear();
        review.ahead.clear();
        // a remark that was being written in it closes with it
        if self.writing.as_ref().is_some_and(|writing| writing.workspace == ws) {
            self.writing = None;
        }
        // the remarks stand in the diffs again
        review.unplaced = true;
        window.focus(&review.state.read(cx).focus_handle(cx), cx);
        cx.notify();
        true
    }

    /// A changed file is asked for (a click in the Changed files panel, "Show changes" in the opened file): the
    /// changes come on screen if they were put away, an opened file closes, and the file's diff comes to the top,
    /// unfolded.
    pub fn focus_change(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ws) = self.workspace(cx) else {
            return;
        };
        self.files_picked(cx);
        self.reveal(cx);
        self.close_file(window, cx);
        if let Some(review) = self.reviews.get_mut(&ws) {
            review.focus = Some(path.to_string());
            // done as the diffs are next drawn: they may not have been given this file yet
            review.go_to = Some((path.to_string(), false));
            // you are looking at this diff now: the keyboard goes there with you, so the arrows scroll it and ⌘+ and ⌘−
            // resize its code, not the terminal's text (item 8.4: "when I click on one of the files that have the
            // changes ... the font size, I should be able to change")
            review.wants_keyboard = true;
            cx.notify();
        }
        // its diff is read before the others, if it wasn't yet
        self.read_more(&ws, cx);
    }

    /// The file the workspace on screen is looking at: the opened one, else the changed file last asked for.
    pub fn focused(&self, cx: &App) -> Option<String> {
        let review = self.reviews.get(&self.workspace(cx)?)?;
        review.opened.as_ref().map(|opened| opened.path.clone()).or_else(|| review.focus.clone())
    }

    /// Reads the opened file, unless it was read as it is now. Claude editing it changes its signature in the
    /// list of changes, and it is read again; what is on screen stays until the new text is there.
    fn read_opened(&mut self, ws: &str, cx: &mut Context<Self>) {
        let Some(review) = self.reviews.get_mut(ws) else {
            return;
        };
        let version = match &review.opened {
            Some(opened) => review.files.change(&opened.path).map(|change| change.sig.clone()).unwrap_or_default(),
            None => return,
        };
        let Some(opened) = review.opened.as_mut().filter(|opened| opened.version.as_ref() != Some(&version)) else {
            return;
        };
        opened.version = Some(version.clone());
        let first = opened.read.is_none();
        let (ws, path) = (ws.to_string(), opened.path.clone());
        let island = cx.weak_entity();
        // a first read that takes long enough to notice says so (DESIGN.md, principle 5)
        if first {
            let (ws, path, version) = (ws.clone(), path.clone(), version.clone());
            cx.spawn(async move |island, cx| {
                cx.background_executor().timer(READ_IS_SLOW).await;
                let _ = island.update(cx, |island, cx| {
                    let opened = (island.reviews.get_mut(&ws))
                        .and_then(|review| review.opened.as_mut())
                        .filter(|opened| opened.path == path && opened.version.as_deref() == Some(version.as_str()));
                    if let Some(opened) = opened.filter(|opened| opened.read.is_none()) {
                        opened.slow = true;
                        cx.notify();
                    }
                });
            })
            .detach();
        }
        match &self.source {
            Source::Core => {
                let (workspace, file) = (ws.clone(), path.clone());
                Bridge::call(
                    cx,
                    move |shared| read_whole(shared, workspace, &file),
                    move |read, cx| {
                        let _ = island.update(cx, |island, cx| island.opened_was_read(&ws, &path, &version, read, cx));
                    },
                );
            }
            // for pictures and screen tests without a core: the file as its given diff leaves it
            Source::Given(patches) => {
                if patches.get(&path).is_some_and(|patch| patch == STILL_READING) {
                    return;
                }
                let after = (patches.get(&path))
                    .and_then(|patch| DiffFile::parse(patch).ok())
                    .and_then(|files| files.into_iter().next())
                    .map(|file| file.side_lines(DiffSide::Modified).map(|(_, line)| format!("{line}\n")).collect());
                let read = after.map_or(Opened::Missing, |text: String| whole(&path, &text));
                cx.defer(move |cx| {
                    let _ = island.update(cx, |island, cx| island.opened_was_read(&ws, &path, &version, read, cx));
                });
            }
        }
    }

    fn opened_was_read(&mut self, ws: &str, path: &str, version: &str, read: Opened<DiffFile>, cx: &mut Context<Self>) {
        let Some(Review { files, opened, .. }) = self.reviews.get_mut(ws) else {
            return;
        };
        // another file was opened meanwhile, or this one changed again: this answer is old
        let Some(opened) =
            opened.as_mut().filter(|opened| opened.path == path && opened.version.as_deref() == Some(version))
        else {
            return;
        };
        match diff_of(files, path, version) {
            // a newer text of a file that is on screen: it waits for its diff, so the text and the marks of what
            // differs change in one frame (item 10.3)
            DiffOf::Waiting if opened.read.is_some() => opened.held = Some((version.to_string(), read)),
            said => {
                opened.show(version, read, said);
                cx.notify();
            }
        }
        // its diff is read before the others
        self.read_more(ws, cx);
    }

    /// The composer closes; the keyboard goes back to the diff.
    fn stop_writing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(writing) = self.writing.take() else {
            return;
        };
        if let Some(review) = self.reviews.get(&writing.workspace) {
            review.on_screen().update(cx, |state, cx| state.set_selected_lines(None, cx));
            window.focus(&review.on_screen().read(cx).focus_handle(cx), cx);
        }
        cx.notify();
    }

    /// A remark's words are changed where it stands: its card becomes the composer.
    fn edit(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(ws) = self.workspace(cx) else {
            return;
        };
        let placed = (self.reviews.get(&ws))
            .and_then(|review| review.placed_on_screen().iter().find(|placed| placed.remark.id == id));
        let Some(placed) = placed.cloned() else {
            return;
        };
        self.writing = Some(Writing {
            workspace: ws,
            path: placed.remark.path.clone(),
            side: placed.side,
            start: placed.start,
            end: placed.end,
            editing: Some(placed.remark),
            input: None,
            saving: None,
        });
        cx.notify();
    }

    /// Where each of the workspace's remarks is now (see `enkidu_model::remarks::locate`).
    fn place(review: &mut Review, remarks: &[Remark]) {
        review.placed = remarks
            .iter()
            .map(|remark| {
                let side = side_of(remark);
                let file = review.files.files().find(|(change, _)| change.path == remark.path);
                let lines = file.and_then(|(_, read)| match read {
                    Some(Read::Lines(file)) => whole_side(file, side),
                    _ => None,
                });
                placed_at(remark, lines.as_deref())
            })
            .collect();
    }

    /// Starts as many reads as there is room for, the file being read first.
    fn read_more(&mut self, ws: &str, cx: &mut Context<Self>) {
        let Some(review) = self.reviews.get_mut(ws) else {
            return;
        };
        // first the opened file (its marks wait for its diff), then the file that was asked for, else the one being
        // read: the first of them that still has to be read
        let opened = review.opened.as_ref().map(|opened| opened.path.clone());
        let asked = review.go_to.as_ref().map(|(path, _)| path.clone());
        let top = review.state.read(cx).top_path().map(|path| path.to_string());
        let first = [opened, asked, top].into_iter().flatten().find(|path| review.files.wants_read(path));
        let room = AT_ONCE.saturating_sub(review.files.reading());
        let wanted = review.files.take_to_read(room, first.as_deref());
        for change in wanted {
            let (ws, path, sig) = (ws.to_string(), change.path.clone(), change.sig.clone());
            match &self.source {
                Source::Core => {
                    let island = cx.weak_entity();
                    let workspace = ws.clone();
                    Bridge::call(
                        cx,
                        move |shared| read_patch(shared, workspace, &change),
                        move |read, cx| {
                            let _ = island.update(cx, |island, cx| island.was_read(&ws, &path, &sig, read, cx));
                        },
                    );
                }
                Source::Given(patches) => {
                    let read = match patches.get(&path) {
                        Some(patch) if patch == STILL_READING => continue,
                        Some(patch) => parse(&path, patch),
                        None => Read::Failed("No diff was given for this file".into()),
                    };
                    // not from inside this loop: the answer starts the next reads
                    let island = cx.weak_entity();
                    cx.defer(move |cx| {
                        let _ = island.update(cx, |island, cx| island.was_read(&ws, &path, &sig, read, cx));
                    });
                }
            }
        }
    }

    fn was_read(&mut self, ws: &str, path: &str, sig: &str, read: Read<DiffFile>, cx: &mut Context<Self>) {
        let Some(review) = self.reviews.get_mut(ws) else {
            return;
        };
        if review.files.read(path, sig, read) {
            review.stale = true;
            // the opened file's own diff: its marks are there now, and a newer text that waited for them
            let Review { files, opened, .. } = review;
            if let Some(opened) = opened.as_mut().filter(|opened| opened.path == path) {
                opened.diff_was_read(files);
            }
            cx.notify();
        }
        self.read_more(ws, cx);
    }

    /// Gives the view the files as they are now. Done as the island is drawn, so a burst of reads that came back
    /// since the last frame costs the view one new list, not one each.
    fn settle(&mut self, ws: &str, cx: &mut Context<Self>) {
        // the lines a new remark is being written on, to mark them again in a new list
        let writing_on = self
            .writing
            .as_ref()
            .filter(|writing| writing.workspace == ws && writing.editing.is_none())
            .map(|writing| DiffLineRange::new(writing.path.clone(), writing.side, writing.start, writing.end));
        let remarks = self.remarks.read(cx).of(ws);
        let Some(review) = self.reviews.get_mut(ws) else {
            return;
        };
        // an opened file is what is on screen: the diffs behind it are settled when they are back
        if let Some(opened) = &mut review.opened {
            let file = match &opened.read {
                Some(Opened::Text(file, _)) => Some(file.clone()),
                _ => None,
            };
            if std::mem::take(&mut opened.stale) {
                // the line it was opened at comes into view once there are lines
                let go_to = file.as_ref().and_then(|_| opened.go_to.take());
                let (path, shown) = (opened.path.clone(), file.clone());
                opened.state.update(cx, |state, cx| {
                    state.set_files(shown, cx);
                    // the view drops its marked lines with the old text
                    if writing_on.is_some() {
                        state.set_selected_lines(writing_on, cx);
                    }
                    if let Some(line) = go_to {
                        state.scroll_to_line_centered(DiffLinePosition::new(path, DiffSide::Modified, line), cx);
                    }
                });
                opened.unplaced = true;
            }
            if std::mem::take(&mut opened.unplaced) {
                // the remarks on its lines as they are now; those on lines a change removed aren't in the file
                let lines = file.as_ref().and_then(|file| whole_side(file, DiffSide::Modified));
                opened.placed = (remarks.iter())
                    .filter(|remark| remark.path == opened.path && remark.side != "old")
                    .map(|remark| placed_at(remark, lines.as_deref()))
                    .collect();
            }
            return;
        }
        let gave = std::mem::take(&mut review.stale);
        if gave {
            let Review { files, stand_ins, state, .. } = review;
            let mut stand_in = |path: &str| {
                stand_ins.entry(path.to_string()).or_insert_with(|| DiffFile::without_lines(path.to_string())).clone()
            };
            let mut shown: Vec<DiffFile> = files
                .files()
                .map(|(change, read)| match read {
                    // the very same parsed file each time: the view keeps what it prepared for it
                    Some(Read::Lines(file)) => file.clone(),
                    _ => stand_in(&change.path),
                })
                .collect();
            // then the files that have remarks and no changes, by path: their remarks are sent with the others
            let elsewhere = unchanged_with_remarks(&remarks, files);
            shown.extend(elsewhere.iter().map(|path| stand_in(path)));
            stand_ins.retain(|path, _| files.change(path).is_some() || elsewhere.iter().any(|known| known == path));
            state.update(cx, |state, cx| {
                state.set_files(shown, cx);
                // the view drops its marked lines with the old list
                if writing_on.is_some() {
                    state.set_selected_lines(writing_on, cx);
                }
            });
            review.unplaced = true;
        }
        if std::mem::take(&mut review.unplaced) {
            Self::place(review, &remarks);
        }
        // a changed file that was asked for comes to the top, unfolded. Where that is isn't final until its diff and
        // the ones above it were read, so it is brought there again as they arrive, and then left alone
        if let Some((path, brought)) = review.go_to.clone() {
            if gave || !brought {
                review.state.update(cx, |state, cx| {
                    state.set_file_collapsed(&path, false, cx);
                    state.scroll_to_file(&path, cx);
                });
            }
            let mut above = review.files.files().map(|(change, read)| (change.path == path, read.is_some()));
            let waiting = match above.position(|(asked, _)| asked) {
                // the file isn't changed any more: nothing to bring
                None => false,
                Some(at) => review.files.files().take(at + 1).any(|(_, read)| read.is_none()),
            };
            review.go_to = waiting.then_some((path, true));
        }
    }

    /// Settings chose other code colors: every diff and opened file on hand is colored again, keeping its place.
    fn follow_code_colors(&mut self, colors: CodeColors, cx: &mut Context<Self>) {
        if std::mem::replace(&mut self.code_colors, colors) == colors {
            return;
        }
        for review in self.reviews.values() {
            let states = [Some(&review.state), review.opened.as_ref().map(|opened| &opened.state)];
            for state in states.into_iter().flatten() {
                state.update(cx, |state, cx| {
                    state.set_syntax_highlighter(move |language, text| syntax_colors(colors, language, text), cx)
                });
            }
        }
    }

    fn set_layout(&mut self, layout: DiffLayout, cx: &mut Context<Self>) {
        self.ui.update(cx, |ui, cx| {
            if ui.diff_layout != layout {
                ui.diff_layout = layout;
                cx.notify();
            }
        });
    }

    /// The band: "Changes", how many files and lines, and the switch between one column and side by side.
    fn band(
        &self,
        totals: Changes,
        layout: DiffLayout,
        typing: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let files = review::files_words(totals.files);
        let of = self.workspace(cx).unwrap_or_default();
        let choice = |id: &'static str, icon: &'static str, title: &'static str, layout: DiffLayout, on: bool| {
            segment(id, icon, title, on)
                .tipped(title, None)
                .on_click(cx.listener(move |this, _, _, cx| this.set_layout(layout, cx)))
        };
        let split = layout == DiffLayout::Split;
        let switch = segmented(
            "diff-layout",
            Some(usize::from(split)),
            vec![
                choice("diff-unified", "icons/rows-2.svg", "One column", DiffLayout::Unified, !split),
                choice("diff-split", "icons/columns-2.svg", "Before and after, side by side", DiffLayout::Split, split),
            ],
            window,
            cx,
        );
        div()
            .id("changes-band")
            .h(theme::BAR)
            .flex_none()
            .flex()
            .items_center()
            .gap(px(8.))
            .pl(theme::INSET)
            .pr(px(12.))
            .relative()
            .child(line_under())
            // its title (while your typing goes to the changes, the pane has the blue edge: DESIGN.md, "States")
            .child(
                div()
                    .id("changes-title")
                    .test_support()
                    .aria_label("Changes")
                    .aria_description(if typing { "has the keyboard" } else { "" })
                    .h_full()
                    .flex()
                    .flex_none()
                    .items_center()
                    .text_size(theme::PANEL_TITLE)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::INK)
                    .child("Changes"),
            )
            // how many files, and their lines (DESIGN.md, "Layout": "The changed files with +/− stand in the bar of
            // the changes"). With remarks only and no changed file there is no count: nothing says "0 files"
            .when(totals.files > 0, |band| {
                band.child(
                    div()
                        .id("changes-count")
                        .test_support()
                        .aria_label(files.clone())
                        .text_size(theme::TEXT_SMALL)
                        .text_color(theme::INK_3)
                        .child(files),
                )
                .child(counts(
                    "changes",
                    &of,
                    Some(totals.additions),
                    Some(totals.deletions),
                    window,
                    cx,
                ))
            })
            .child(div().flex_1())
            .child(switch)
            .child(self.whole_workspace_button(cx))
    }

    /// The button at the end of the band that gives the changes the whole workspace, or brings the terminal back
    /// (⇧⌘E).
    fn whole_workspace_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let full = self.ui.read(cx).review_maximized;
        let (label, icon) = match full {
            true => ("Back to the terminal", "icons/minimize-2.svg"),
            false => ("Review in full width", "icons/maximize-2.svg"),
        };
        div()
            .id("maximize-review")
            .test_support()
            .role(gpui_kit::Role::Button)
            .aria_label(label)
            .tipped(label, Some(Box::new(crate::ToggleFullReview)))
            .size(theme::CONTROL)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(theme::RADIUS_CONTROL)
            .cursor_pointer()
            .group("maximize-review")
            .hover(|s| s.bg(theme::HOVER))
            .child(
                svg()
                    .path(icon)
                    .size(theme::ICON)
                    .text_color(theme::INK_3)
                    .group_hover("maximize-review", |s| s.text_color(theme::INK)),
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.ui.update(cx, |ui, cx| {
                    ui.toggle_full_review();
                    cx.notify();
                })
            }))
    }
}

fn mode(layout: DiffLayout) -> DiffMode {
    match layout {
        DiffLayout::Unified => DiffMode::Unified,
        DiffLayout::Split => DiffMode::Split,
    }
}

/// Lines added and removed, green and red. Nothing for none (DESIGN.md: no counts of zero). `of` says whose lines
/// they are (the workspace, and the file for a file's): each number ticks into place when it changes while it is on
/// screen, never when it first shows (DESIGN.md, "Motion").
pub(crate) fn counts(
    id: &'static str,
    of: &str,
    additions: Option<u64>,
    deletions: Option<u64>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let mut count = |what: &'static str, sign: &str, lines: Option<u64>, color: Rgba| {
        lines.filter(|lines| *lines > 0).map(|lines| {
            let words = format!("{sign}{lines}");
            let key = SharedString::from(format!("{id}-{what}:{of}"));
            let changes = changes_seen(key.clone(), lines, window, cx);
            let number = div()
                .id(SharedString::from(format!("{id}-{what}")))
                .test_support()
                .aria_label(words.clone())
                .text_color(color)
                .child(words);
            ticking(key, number, changes)
        })
    };
    let (added, removed) =
        (count("added", "+", additions, theme::ADD_INK), count("removed", "\u{2212}", deletions, theme::DEL_INK));
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.))
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .children(added)
        .children(removed)
        .into_any_element()
}

/// A small square that says what happened to the file: a letter on its own tint, as git colors it.
pub(crate) fn glyph(kind: &str) -> impl IntoElement {
    glyph_named("file-kind", kind)
}

/// The same under another id (a row of the Changed files panel has one too).
pub(crate) fn glyph_named(id: &'static str, kind: &str) -> impl IntoElement {
    let (hue, chroma) = review::kind_hue(kind);
    let color = tone(hue, chroma);
    let label = review::kind_label(kind);
    div()
        .id(id)
        .test_support()
        .aria_label(label)
        .tipped(label, None)
        .size(px(16.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(theme::RADIUS_SMALL)
        .bg(alpha(color, 0.15))
        .text_color(color)
        .text_size(px(10.))
        .font_weight(FontWeight::BOLD)
        .child(review::kind_letter(kind))
}

/// Opens a file where the diffs are, at a line when one is given.
type OpenFile = Rc<dyn Fn(&str, Option<usize>, &mut App)>;

/// What a file's header holds: its icon, its name, then its folder, what happened to it, its lines, and the button
/// that opens the whole file. `remark_line` is the first line a remark is on, for a file listed only for its remarks.
fn header(
    file: &DiffFile,
    list: &[Change],
    remarks: usize,
    // the first line a remark is on, for a file listed only for its remarks; and how a file is opened
    (remark_line, open): (Option<usize>, OpenFile),
    workspace: &str,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let path = file.path().clone();
    // the list is in the order a tree of its folders lists it (`ChangeLists` keeps it so)
    let change = list.binary_search_by(|change| in_tree_order(&change.path, &path)).ok().map(|ix| &list[ix]);
    let (dir, name) = review::split_path(&path);
    let (dir, name) = (dir.to_string(), name.to_string());
    let full = match change.and_then(|change| change.old_path.as_ref()) {
        Some(old) => format!("{old} \u{2192} {path}"),
        None => path.to_string(),
    };
    div()
        .w_full()
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(file_icon(&path, 16.))
        .child(
            div()
                .id("file-head")
                .test_support()
                .aria_label(path.clone())
                .tipped(full.clone(), None)
                .flex()
                .flex_1()
                .min_w_0()
                .items_baseline()
                .gap(px(8.))
                .child(
                    div()
                        .flex_shrink_0()
                        .max_w_full()
                        .truncate()
                        .text_size(theme::TEXT)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::INK)
                        .child(name),
                )
                .when(!dir.is_empty(), |head| {
                    head.child(div().min_w_0().truncate().text_size(px(12.)).text_color(theme::INK_3).child(dir))
                }),
        )
        // the remarks on this file that weren't sent yet, in their blue
        .when(remarks > 0, |head| {
            let words = remark_logic::remarks_words(remarks);
            head.child(
                div()
                    .id("file-remarks")
                    .test_support()
                    .aria_label(words.clone())
                    .flex_none()
                    .h(theme::CHIP)
                    .flex()
                    .items_center()
                    .px(px(8.))
                    .rounded_full()
                    .bg(theme::REMARK_BAND)
                    .text_size(theme::TEXT_STATUS)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::ACCENT)
                    .child(words),
            )
        })
        // a file listed only for its remarks says so where the letter of its change would be
        .when(change.is_none(), |head| {
            head.child(
                div()
                    .id("file-unchanged")
                    .test_support()
                    .aria_label("No changes")
                    .flex_none()
                    .text_size(px(12.))
                    .text_color(theme::INK_3)
                    .child("No changes"),
            )
        })
        .children(change.map(|change| glyph(&change.kind)))
        .children(change.map(|change| {
            let of = format!("{workspace}:{}", change.path);
            counts("file", &of, change.additions, change.deletions, window, cx)
        }))
        // the whole file, to leave remarks on lines the diff doesn't show (a deleted file has none)
        .when(change.is_none_or(|change| change.kind != "deleted"), |head| {
            let label = SharedString::from(format!("Open {path}"));
            let (path, open) = (path.to_string(), open.clone());
            head.child(
                div()
                    .id("open-file")
                    .test_support()
                    .role(gpui_kit::Role::Button)
                    .aria_label(label)
                    .tipped("Open the whole file, to leave remarks on any line", None)
                    .size(theme::CONTROL)
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .rounded(theme::RADIUS_CONTROL)
                    .cursor_pointer()
                    .group("open-file")
                    .hover(|s| s.bg(theme::HOVER))
                    // a press here is the button's, not the start of a text selection in the code under it
                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                        gpui_kit::base::GlobalState::suppress_text_selection(cx)
                    })
                    .on_click(move |_, _, cx| open(&path, remark_line, cx))
                    .child(
                        svg()
                            .path("icons/file-text.svg")
                            .size(theme::ICON)
                            .text_color(theme::INK_3)
                            .group_hover("open-file", |s| s.text_color(theme::INK)),
                    ),
            )
        })
        .into_any_element()
}

/// What stands under the header of a file without lines.
fn notice(said: &Said, draw: &DrawNote, window: &mut Window, cx: &mut App) -> AnyElement {
    let (words, color) = match said {
        Said::Reading => return div().h(px(28.)).into_any_element(),
        Said::Remarks(ids) => {
            let cards: Vec<AnyElement> =
                ids.iter().map(|id| div().id(id.clone()).child(draw(id, window, cx)).into_any_element()).collect();
            return div()
                .id("unchanged-remarks")
                .px(px(12.))
                .py(px(8.))
                .flex()
                .flex_col()
                .gap(px(8.))
                // (a card's buttons keep their own presses; its words can be selected)
                .children(cards)
                .into_any_element();
        }
        Said::Words(words) => (words.to_string(), theme::INK_3),
        Said::Failed(why) => (why.clone(), theme::LAMP_RED),
    };
    div()
        .id("file-note")
        .test_support()
        .aria_label(words.clone())
        .px(px(16.))
        .py(px(16.))
        .text_size(px(12.))
        .text_color(color)
        .child(words)
        .into_any_element()
}

/// How the note with an id is drawn: under a line of a diff, or under the header of a file without changes.
type DrawNote = dyn Fn(&ElementId, &mut Window, &mut App) -> AnyElement;

/// What stands under a line: a remark's card, or the composer.
enum Note {
    Card(CardView),
    Composer,
}

impl Render for ChangesIsland {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(ws) = self.workspace(cx) else {
            return island("island-changes").size_full();
        };
        self.settle(&ws, cx);
        // a name with several definitions was clicked: the list to pick from opens (a dialog needs its window)
        if let Some(choosing) = self.choosing.take() {
            let island = cx.weak_entity();
            window.defer(cx, move |window, cx| crate::definitions::open(choosing, island, window, cx));
        }
        let back_to = self.back_to(cx);
        // the composer's text field is made here: a text field needs its window
        if let Some(writing) = self.writing.as_mut().filter(|writing| writing.input.is_none()) {
            let words = writing.editing.as_ref().map(|remark| remark.body.clone()).unwrap_or_default();
            // the cursor goes after a remark's words when they are changed
            let end = Position {
                line: words.matches('\n').count() as u32,
                character: words.rsplit('\n').next().map_or(0, |last| last.encode_utf16().count() as u32),
            };
            let input = cx.new(|cx| {
                TextareaState::new(window, cx)
                    .auto_grow(2, 10)
                    .placeholder("What should Claude do here?")
                    .default_value(words)
            });
            let heard = cx.subscribe(&input, |this, _, event: &InputEvent, cx| match event {
                // ⌘↩ saves
                InputEvent::PressEnter { secondary: true, .. } => this.save_writing(cx),
                // "Add remark" can be pressed once something is typed
                InputEvent::Change => cx.notify(),
                _ => {}
            });
            // and the keyboard goes into it
            input.update(cx, |input, cx| input.set_cursor_position(end, window, cx));
            writing.input = Some((input, heard));
        }
        // a file that was just opened takes the keyboard
        if let Some(opened) = self.reviews.get_mut(&ws).and_then(|review| review.opened.as_mut())
            && std::mem::take(&mut opened.wants_keyboard)
        {
            window.focus(&opened.state.read(cx).focus_handle(cx), cx);
        }
        // and so do the diffs when a changed file was just asked for (a click in the Changed files panel)
        if let Some(review) = self.reviews.get_mut(&ws).filter(|review| review.opened.is_none())
            && std::mem::take(&mut review.wants_keyboard)
        {
            window.focus(&review.state.read(cx).focus_handle(cx), cx);
        }
        // the same for the note that goes with the remarks, once "Add note" was pressed
        if let Some(review) =
            self.reviews.get_mut(&ws).filter(|review| review.bar.note_open && review.bar.note.is_none())
        {
            let note = cx.new(|cx| {
                TextareaState::new(window, cx)
                    .auto_grow(2, 6)
                    .placeholder("A note for Claude that goes with the remarks")
            });
            let heard = cx.subscribe(&note, |this, _, event: &InputEvent, cx| match event {
                // ⌘↩ types the remarks into Claude, with the note
                InputEvent::PressEnter { secondary: true, .. } => this.send_remarks(cx),
                // Escape closes the note only while it is empty
                InputEvent::Change => cx.notify(),
                _ => {}
            });
            note.update(cx, |note, cx| note.focus(window, cx));
            review.bar.note = Some((note, heard));
        }
        let layout = self.ui.read(cx).diff_layout;
        let Some(review) = self.reviews.get(&ws) else {
            return island("island-changes").size_full();
        };
        // what is on screen: the opened file, or the diffs
        let opened = review.opened.as_ref();
        let state = review.on_screen().clone();
        if opened.is_none() && state.read(cx).mode() != mode(layout) {
            state.update(cx, |state, cx| state.set_mode(mode(layout), cx));
        }
        let list = review.list.clone().unwrap_or_default();
        let totals = Changes::of(&list);
        // in the diffs: the files without lines, and what each says
        let mut said: HashMap<String, Said> = match opened {
            Some(_) => HashMap::new(),
            None => (review.files.files())
                .filter_map(|(change, read)| {
                    let said = match read {
                        Some(Read::Lines(_)) => return None,
                        None => Said::Reading,
                        Some(Read::Binary) => Said::Words("Binary file, not shown."),
                        Some(Read::TooLarge) => Said::Words("This change is too large to show here."),
                        Some(Read::Failed(why)) => Said::Failed(why.clone()),
                    };
                    Some((change.path.clone(), said))
                })
                .collect(),
        };

        // the remarks: each under the last of its lines (one whose code is gone, above its file's lines), and the
        // composer under the lines being remarked on
        let writing = self.writing.as_ref().filter(|writing| writing.workspace == ws);
        let composing = writing.and_then(|writing| {
            let (input, _) = writing.input.as_ref()?;
            Some(Rc::new(ComposerView {
                label: remark_views::capitalized(&remark_logic::line_label(writing.start as i64, writing.end as i64)),
                input: input.clone(),
                save: if writing.editing.is_some() { "Save" } else { "Add remark" },
                busy: writing.saving.is_some(),
            }))
        });
        let edited = writing.and_then(|writing| writing.editing.as_ref()).map(|remark| remark.id.clone());
        let mut annotations = Vec::new();
        let mut notes: HashMap<ElementId, Note> = HashMap::new();
        let mut counts: HashMap<String, usize> = HashMap::new();
        // in the diffs: the remarks on files without changes, by file and line; they stand under their file's header
        let mut unchanged: HashMap<String, Vec<(usize, ElementId)>> = HashMap::new();
        for placed in review.placed_on_screen() {
            *counts.entry(placed.remark.path.clone()).or_default() += 1;
            let id = ElementId::Name(placed.remark.id.clone().into());
            // its file's lines are in the view: the opened file's, or a changed file's diff
            if opened.is_some() || review.files.change(&placed.remark.path).is_some() {
                let (path, line) = (
                    placed.remark.path.clone(),
                    DiffLinePosition::new(placed.remark.path.clone(), placed.side, placed.end),
                );
                annotations.push(if placed.outdated {
                    DiffAnnotation::file(id.clone(), path)
                } else {
                    DiffAnnotation::line(id.clone(), line)
                });
            } else {
                unchanged.entry(placed.remark.path.clone()).or_default().push((placed.start, id.clone()));
            }
            let note = if edited.as_ref() == Some(&placed.remark.id) {
                Note::Composer
            } else {
                Note::Card(CardView {
                    id: placed.remark.id.clone(),
                    label: remark_views::capitalized(&remark_logic::line_label(placed.start as i64, placed.end as i64)),
                    removed: placed.side == DiffSide::Original,
                    many: placed.start != placed.end,
                    outdated: placed.outdated,
                    snippet: placed.remark.snippet.clone(),
                    body: placed.remark.body.clone(),
                })
            };
            notes.insert(id, note);
        }
        if let Some(writing) = writing.filter(|writing| writing.editing.is_none()) {
            let id = ElementId::Name("new-remark".into());
            let line = DiffLinePosition::new(writing.path.clone(), writing.side, writing.end);
            annotations.push(DiffAnnotation::line(id.clone(), line));
            notes.insert(id, Note::Composer);
        }
        // the files listed only for their remarks, which can be opened to see the lines they are about
        let mut elsewhere: HashMap<String, usize> = HashMap::new();
        for (path, mut remarks) in unchanged {
            remarks.sort_by_key(|(line, _)| *line);
            elsewhere.insert(path.clone(), remarks.first().map_or(1, |(line, _)| *line));
            said.insert(path, Said::Remarks(remarks.into_iter().map(|(_, id)| id).collect()));
        }
        let (notes, counts, said, elsewhere) = (Rc::new(notes), Rc::new(counts), Rc::new(said), Rc::new(elsewhere));
        let island_ = cx.weak_entity();
        // what a button does, on the island
        let act = |does: fn(&mut ChangesIsland, &mut Window, &mut Context<ChangesIsland>)| -> Act {
            let island_ = island_.clone();
            Rc::new(move |window, cx| {
                let _ = island_.update(cx, |island, cx| does(island, window, cx));
            })
        };
        // how a note is drawn, under a line or under the header of a file without changes
        let draw: Rc<DrawNote> = {
            let (island_, remarks, ws) = (island_.clone(), self.remarks.clone(), ws.clone());
            let (save, cancel) = (
                act(|island, _, cx| island.save_writing(cx)),
                act(|island, window, cx| island.stop_writing(window, cx)),
            );
            Rc::new(move |id: &ElementId, _: &mut Window, cx: &mut App| -> AnyElement {
                match notes.get(id) {
                    Some(Note::Card(view)) => {
                        let edit: Act = {
                            let (island_, id) = (island_.clone(), view.id.clone());
                            Rc::new(move |_, cx| {
                                let _ = island_.update(cx, |island, cx| island.edit(&id, cx));
                            })
                        };
                        let delete: Act = {
                            let (remarks, ws, id) = (remarks.clone(), ws.clone(), view.id.clone());
                            Rc::new(move |_, cx| remarks.update(cx, |remarks, cx| remarks.delete(&ws, &id, cx)))
                        };
                        remark_views::card(view, edit, delete)
                    }
                    Some(Note::Composer) => match &composing {
                        Some(view) => remark_views::composer(view, save.clone(), cancel.clone(), cx),
                        None => div().into_any_element(),
                    },
                    None => div().into_any_element(),
                }
            })
        };

        // the bar at the foot, while remarks wait to be sent
        let waiting = self.remarks.read(cx).of(&ws);
        let bar = (!waiting.is_empty()).then(|| {
            let view = BarView {
                waiting: remark_logic::waiting_words(&waiting),
                count: waiting.len(),
                note: review.bar.note.as_ref().map(|(note, _)| note.clone()),
                note_open: review.bar.note_open,
                confirming: review.bar.confirming,
                sending: self.remarks.read(cx).sending(&ws),
            };
            let acts = BarActs {
                add_note: act(|island, _, cx| island.with_bar(cx, |bar| bar.note_open = true)),
                close_note: act(|island, window, cx| island.close_note(window, cx)),
                discard: act(|island, _, cx| island.with_bar(cx, |bar| bar.confirming = true)),
                keep: act(|island, _, cx| island.with_bar(cx, |bar| bar.confirming = false)),
                delete: act(|island, _, cx| island.discard_remarks(cx)),
                send: act(|island, _, cx| island.send_remarks(cx)),
            };
            remark_views::bar(&view, acts, window, cx)
        });

        // your typing goes here: the bar's title says so (no part has the keyboard's mark while the window is behind
        // another app: DESIGN.md, "States")
        let focused = self.has_keyboard(window, cx) && window.is_window_active();
        // code is 12/20 (DESIGN.md, "Type") until you change its size (item 8.4); the lines keep that share
        let code_size = self.ui.read(cx).code_size;
        let lines = Diff::new(&state)
            .colors(colors(self.code_colors))
            .font_size(px(code_size as f32))
            .row_height(px(code_row_height(code_size) as f32))
            .soft_wrap(true)
            .annotations(annotations)
            .render_annotation({
                let draw = draw.clone();
                move |annotation, window, cx| draw(annotation.id(), window, cx)
            })
            // the plus beside a line under the pointer: a remark on that line (or on the lines that are marked)
            .on_add_annotation({
                let island_ = island_.clone();
                move |range, _, cx| {
                    let _ = island_.update(cx, |island, cx| island.write_on(range, cx));
                }
            })
            .size_full()
            .border_0()
            .bg(theme::WELL);
        let (band, body) = match opened {
            // an opened file: its own band, and its lines (or why it has none)
            Some(opened) => {
                let view = opened_file::BandView {
                    path: opened.path.clone(),
                    kind: review.files.change(&opened.path).map(|change| change.kind.clone()),
                    lines: opened.read.as_ref().and_then(|read| read.lines()),
                    remarks: opened.placed.len(),
                    has_review: !list.is_empty() || !waiting.is_empty(),
                    back_to,
                    typing: focused,
                };
                let path = opened.path.clone();
                let acts = opened_file::BandActs {
                    // back to where a ⌘-click was made, when one led here; else the file closes
                    close: act(|island, window, cx| {
                        if !island.go_back(window, cx) {
                            island.close_file(window, cx);
                        }
                    }),
                    show_changes: {
                        let (island_, path) = (island_.clone(), path.clone());
                        Rc::new(move |window, cx| {
                            let _ = island_.update(cx, |island, cx| island.focus_change(&path, window, cx));
                        })
                    },
                    copy_path: {
                        let path = path.clone();
                        Rc::new(move |window, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(path.clone()));
                            toast::note(window, "Path copied", path.clone(), cx);
                        })
                    },
                    reveal: {
                        let (ws, path) = (ws.clone(), path.clone());
                        Rc::new(move |_, cx| {
                            let (ws, path) = (ws.clone(), path.clone());
                            Bridge::call(
                                cx,
                                move |shared| enkidu_core::commands::reveal_file(shared, ws, path),
                                |_, _| {},
                            );
                        })
                    },
                };
                let said = match (&opened.read, opened.slow) {
                    (None, true) => Some("Reading the file…".to_string()),
                    (read, _) => read.as_ref().and_then(|read| read.note()),
                };
                let body = match said {
                    Some(words) => opened_file::note(words),
                    None => {
                        // the line it was opened at stays marked, in the remarks' blue
                        let target = opened.target.map(|line| {
                            let range = DiffLineRange::new(path.clone(), DiffSide::Modified, line, line);
                            DiffDecoration::new(range).with_background(hsla(alpha(theme::ACCENT, 0.12)))
                        });
                        // opened from Find in files: what was found on that line is marked there, as the popup
                        // marked it (when the line still has that text)
                        let found: Vec<DiffMark> = (|| {
                            let (line, shown) = (opened.target?, opened.found.as_ref()?);
                            let Some(Opened::Text(file, _)) = &opened.read else { return None };
                            let text = *whole_side(file, DiffSide::Modified)?.get(line.checked_sub(1)?)?;
                            let at = DiffLinePosition::new(path.clone(), DiffSide::Modified, line);
                            let marks = enkidu_model::search::ranges_in_line(text, shown)?;
                            let color = hsla(alpha(theme::FOCUS, 0.3));
                            Some(marks.into_iter().map(|mark| DiffMark::new(at.clone(), mark, color)).collect())
                        })()
                        .unwrap_or_default();
                        // and what differs from the last commit, beside the line numbers (item 10.3)
                        let differs = (!opened.marks.is_empty())
                            .then(|| (SharedString::from(path.clone()), opened.marks.clone()));
                        lines
                            .header_visible(false)
                            .decorations(target)
                            .marks(found)
                            .change_marks(differs)
                            .into_any_element()
                    }
                };
                let body = div()
                    .id("file-view")
                    .test_support()
                    .aria_label(SharedString::from(path))
                    .flex_1()
                    .min_h_0()
                    .child(body);
                (opened_file::band(&view, acts), body)
            }
            // the diffs, every changed file under its header
            None => {
                let open = {
                    let island_ = island_.clone();
                    move |path: &str, line: Option<usize>, cx: &mut App| {
                        let _ = island_.update(cx, |island, cx| island.open_file(path, line, cx));
                    }
                };
                let lines = lines
                    // a file's diff is its whole file in one hunk: nothing is left out before it, so nothing says so
                    .hunk_separator(DiffHunkSeparator::LineInfo)
                    .render_header({
                        let ws = ws.clone();
                        move |file, window, cx| {
                            let path = file.path().as_str();
                            let remarks = counts.get(path).copied().unwrap_or(0);
                            let opens = (elsewhere.get(path).copied(), Rc::new(open.clone()) as OpenFile);
                            header(file, &list, remarks, opens, &ws, window, cx)
                        }
                    })
                    .render_notice({
                        let draw = draw.clone();
                        move |file, window, cx| {
                            said.get(file.path().as_str()).map(|said| notice(said, &*draw, window, cx))
                        }
                    });
                let body = div().id("diff-view").test_support().flex_1().min_h_0().child(lines);
                (self.band(totals, layout, focused, window, cx).into_any_element(), body)
            }
        };
        island("island-changes")
            .size_full()
            // ⌘↩ with the keyboard in the changes types the remarks into Claude (a text field's own ⌘↩ comes first)
            .key_context(KEYS)
            .on_action(cx.listener(|this, _: &crate::TypeRemarks, _, cx| this.send_remarks(cx)))
            // Escape there leaves an opened file (a text field's own Escape comes first, and a terminal's is its own)
            .on_action(cx.listener(|this, _: &crate::CloseFile, window, cx| {
                if !this.close_file(window, cx) {
                    cx.propagate();
                }
            }))
            // ⌘[ and ⌘]: back to where a ⌘-click was made, and forward again (item 10.11)
            .on_action(cx.listener(|this, _: &crate::GoBack, window, cx| {
                if !this.go_back(window, cx) {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|this, _: &crate::GoForward, window, cx| {
                if !this.go_forward(window, cx) {
                    cx.propagate();
                }
            }))
            .child(band)
            .child(body)
            .children(bar)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Items 3.2e and 8.3: what the view is given for a file's text is the colors of the scheme Settings names, as
    /// the view wants them: an IDE's, or the quiet ones.
    #[test]
    fn the_views_colors_are_the_chosen_schemes() {
        let text = "def total(items):\n    return sum(items)  # all\n";
        let in_scheme = |colors: CodeColors| {
            let colors = syntax_colors(colors, "python", text);
            move |word: &str| color_at(&colors, text, word)
        };
        fn color_at(colors: &[(Range<usize>, HighlightStyle)], text: &str, word: &str) -> (u8, u8, u8, u8) {
            let at = text.find(word).unwrap();
            let (_, style) = colors.iter().find(|(range, _)| range.start <= at && at < range.end).expect(word);
            let rgba = gpui_kit::Rgba::from(style.color.expect("a color"));
            let byte = |part: f32| (part * 255.).round() as u8;
            (byte(rgba.r), byte(rgba.g), byte(rgba.b), byte(rgba.a))
        }
        // an IDE's: the keyword, the definition, the built-in name and the comment each in their own color
        let color_of = in_scheme(CodeColors::Ide);
        assert_eq!(color_of("def"), (0xcf, 0x8e, 0x6d, 0xff), "a keyword is orange");
        assert_eq!(color_of("total"), (0x56, 0xa8, 0xf5, 0xff), "a function's name is blue where it is defined");
        assert_eq!(color_of("sum"), (0x88, 0x88, 0xc6, 0xff), "a built-in name");
        assert_eq!(color_of("# all"), (0x7a, 0x7e, 0x85, 0xff), "a comment is gray");
        assert_eq!(color_of("items"), (0xbc, 0xbe, 0xc4, 0xff), "plain code");
        // the quiet ones
        let color_of = in_scheme(CodeColors::Quiet);
        let colors = syntax_colors(CodeColors::Quiet, "python", text);
        assert_eq!(color_of("def"), (0xa0, 0xa0, 0xa0, 0xff), "a keyword is gray");
        assert_eq!(color_of("total"), (0xff, 0xc7, 0x99, 0xff), "a function's name is peach");
        assert_eq!(color_of("# all"), (0x8b, 0x8b, 0x8b, 0x94), "a comment is dim and lets the row show through");
        // nothing but colors: no weights, no underlines
        assert!(colors.iter().all(|(_, style)| style.font_weight.is_none() && style.underline.is_none()));
        // a file without a grammar gets none
        assert!(syntax_colors(CodeColors::Ide, "text", "plain words\n").is_empty());
    }

    /// Item 10.2: the marks beside the lines that differ from the last commit are in the colors of the code's
    /// scheme: PyCharm's own with an IDE's colors, the tones of the app's file badges with the quiet ones.
    #[test]
    fn the_marks_of_what_differs_follow_the_codes_colors() {
        let bytes = |color: gpui_kit::Hsla| {
            let rgba = gpui_kit::Rgba::from(color);
            let byte = |part: f32| (part * 255.).round() as u8;
            (byte(rgba.r), byte(rgba.g), byte(rgba.b))
        };
        let ide = colors(CodeColors::Ide);
        assert_eq!(bytes(ide.mark_added), (0x54, 0x91, 0x59), "the scheme's ADDED_LINES_COLOR");
        assert_eq!(bytes(ide.mark_changed), (0x37, 0x5f, 0xad), "its MODIFIED_LINES_COLOR");
        assert_eq!(bytes(ide.mark_removed), (0x86, 0x8a, 0x91), "its DELETED_LINES_COLOR");
        let quiet = colors(CodeColors::Quiet);
        let badge = |kind: &str| {
            let (hue, chroma) = review::kind_hue(kind);
            bytes(hsla(tone(hue, chroma)))
        };
        assert_eq!(bytes(quiet.mark_added), badge("added"));
        assert_eq!(bytes(quiet.mark_changed), badge("modified"));
        assert_eq!(bytes(quiet.mark_removed), badge("deleted"));
        // the rest of the view's colors are the same in both
        assert_eq!((ide.added, ide.removed, ide.header), (quiet.added, quiet.removed, quiet.header));
    }

    /// Item 10.3: an opened file's marks are read from its diff of the very version that is on screen. A file that
    /// is all new carries none, a diff that isn't read yet is waited for, and one without lines gives none.
    #[test]
    fn an_opened_files_marks_come_from_its_diff_of_the_same_version() {
        let change = |path: &str, kind: &str, sig: &str| Change {
            path: path.into(),
            old_path: None,
            kind: kind.into(),
            staged: false,
            additions: None,
            deletions: None,
            sig: sig.into(),
        };
        let patch = "--- a/a.py\n+++ b/a.py\n@@ -1,3 +1,3 @@\n one\n-two\n+TWO\n three\n";
        let lines = || Read::Lines(DiffFile::parse(patch).unwrap().remove(0));
        let mut files: FileSet<DiffFile> = FileSet::default();
        files.follow(&[change("a.py", "modified", "v1"), change("new.py", "untracked", "n1")]);
        // nothing is read yet
        assert!(matches!(diff_of(&files, "a.py", "v1"), DiffOf::Waiting));
        // an all new file never has marks, and neither has a file that isn't changed
        assert!(matches!(diff_of(&files, "new.py", "n1"), DiffOf::Unmarked));
        assert!(matches!(diff_of(&files, "other.py", ""), DiffOf::Unmarked));
        // read: the marks of that version
        assert!(files.read("a.py", "v1", lines()));
        let DiffOf::Marks(marks) = diff_of(&files, "a.py", "v1") else { panic!("read") };
        assert_eq!(marks, [DiffChangeMark { kind: enkidu_diff::DiffChangeKind::Changed, start: 2, end: 2 }]);
        // Claude edits the file: the list has a new version, and the diff on hand is the old one's
        files.follow(&[change("a.py", "modified", "v2"), change("new.py", "untracked", "n1")]);
        assert!(matches!(diff_of(&files, "a.py", "v2"), DiffOf::Waiting));
        assert!(matches!(diff_of(&files, "a.py", "v1"), DiffOf::Marks(_)), "the text on screen keeps its own");
        // a diff that can't be shown has no lines to compare: read, without marks
        assert!(files.read("a.py", "v2", Read::TooLarge));
        assert!(matches!(diff_of(&files, "a.py", "v2"), DiffOf::Marks(marks) if marks.is_empty()));
    }
}
