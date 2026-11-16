//! Search (DESIGN.md, "Search, like PyCharm's Search Everywhere"; item 4.1a): one popup with three tabs. Files finds
//! names in git's list, Symbols finds definitions in the core's index, Text reads the files. This is what each tab
//! shows, as plain data: its rows, its status, its notes, the options of Find in files, the files opened lately, the
//! rule for Shift tapped twice, and where a match that was shown in the popup sits in the real line.

use std::collections::HashMap;
use std::ops::Range;
use std::time::{Duration, Instant};

use enkidu_core::git::Change;
use enkidu_core::search::{FileHits, LineHit, TextQuery, TextSummary};
use enkidu_core::symbols::{Kind, SymbolHit, SymbolResults};

use crate::filetree::{Files, Found};
use crate::text::thousands;

/// The Files tab lists this many files at most.
pub const FILE_LIMIT: usize = 60;
/// The Symbols tab asks for this many definitions.
pub const SYMBOL_LIMIT: usize = 60;
/// A workspace remembers this many files opened lately.
pub const MAX_RECENT: usize = 15;
/// How long after the last key Symbols asks the core's index.
pub const SYMBOLS_WAIT: Duration = Duration::from_millis(60);
/// How long after the last key Text starts reading the files.
pub const TEXT_WAIT: Duration = Duration::from_millis(180);
/// A wait shorter than this shows no loading words (DESIGN.md, principle 5).
pub const NOTICEABLE: Duration = Duration::from_millis(400);

/// The popup's tabs: file names, definitions (classes, functions, methods), and text inside files.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Tab {
    #[default]
    Files,
    Symbols,
    Text,
}

impl Tab {
    pub const ALL: [Tab; 3] = [Tab::Files, Tab::Symbols, Tab::Text];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Files => "Files",
            Tab::Symbols => "Symbols",
            Tab::Text => "Text",
        }
    }

    /// The key that opens the popup on this tab, as its tab says it.
    pub fn shortcut(self) -> &'static str {
        match self {
            Tab::Files => "⇧⌘O",
            Tab::Symbols => "⌘O",
            Tab::Text => "⇧⌘F",
        }
    }

    /// The tab after this one (`by` 1) or before it (-1), around the ends: what ⇥ and ⇧⇥ go to.
    pub fn step(self, by: i32) -> Tab {
        let at = Tab::ALL.iter().position(|tab| *tab == self).unwrap_or(0) as i32;
        Tab::ALL[(at + by).rem_euclid(Tab::ALL.len() as i32) as usize]
    }

    /// What the field says while it is empty.
    pub fn placeholder(self, workspace: &str) -> String {
        match self {
            Tab::Files => format!("A file in {workspace}"),
            Tab::Symbols => "A class, function or method".into(),
            Tab::Text => format!("Text in {workspace}'s files"),
        }
    }

    /// The popup's width: a line of code with its file's name beside it needs more room than a name.
    pub fn width(self) -> f32 {
        if self == Tab::Text { 920. } else { 680. }
    }
}

/// One of Find in files' three switches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toggle {
    Case,
    Word,
    Regex,
}

impl Toggle {
    pub const ALL: [Toggle; 3] = [Toggle::Case, Toggle::Word, Toggle::Regex];

    pub fn label(self) -> &'static str {
        match self {
            Toggle::Case => "Match case",
            Toggle::Word => "Whole words",
            Toggle::Regex => "Regular expression",
        }
    }

    /// The same keys as VS Code.
    pub fn shortcut(self) -> &'static str {
        match self {
            Toggle::Case => "⌥⌘C",
            Toggle::Word => "⌥⌘W",
            Toggle::Regex => "⌥⌘R",
        }
    }
}

/// Find in files' options. Without "Match case" the case is smart: ignored unless the text has a capital letter.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextOptions {
    pub case_sensitive: Option<bool>,
    pub whole_word: bool,
    pub regex: bool,
    /// Only files of this kind are read (what stands after the last dot of their name, as `py`); every file
    /// without one (item 10.5).
    pub kind: Option<String>,
}

/// How the filter of Find in files says a kind of file: "*.py", and "All files" without one.
pub fn kind_words(kind: Option<&str>) -> String {
    match kind {
        Some(kind) => format!("*.{kind}"),
        None => "All files".into(),
    }
}

impl TextOptions {
    pub fn toggle(&mut self, which: Toggle) {
        match which {
            Toggle::Case => self.case_sensitive = if self.case_sensitive == Some(true) { None } else { Some(true) },
            Toggle::Word => self.whole_word = !self.whole_word,
            Toggle::Regex => self.regex = !self.regex,
        }
    }

    pub fn on(&self, which: Toggle) -> bool {
        match which {
            Toggle::Case => self.case_sensitive == Some(true),
            Toggle::Word => self.whole_word,
            Toggle::Regex => self.regex,
        }
    }

    /// What the core is asked for what was typed; nothing when it is only spaces. The text goes as typed: in Find
    /// in files a space at either end is part of what is looked for.
    pub fn query(&self, typed: &str) -> Option<TextQuery> {
        (!typed.trim().is_empty()).then(|| TextQuery {
            pattern: typed.to_string(),
            case_sensitive: self.case_sensitive,
            whole_word: self.whole_word,
            regex: self.regex,
            kind: self.kind.clone(),
        })
    }
}

/// What Search keeps between openings, while the app runs: the tab used last (⇧⇧ opens it), what was typed (the
/// tabs share it), and Find in files' options.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Kept {
    pub tab: Tab,
    pub query: String,
    pub options: TextOptions,
}

/// Why a file is listed before anything is typed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Changed,
    Recent,
}

impl Group {
    pub fn label(self) -> &'static str {
        match self {
            Group::Changed => "Changed",
            Group::Recent => "Opened lately",
        }
    }
}

/// One row of the results.
#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    /// The heading of the files listed before anything is typed.
    Label(Group),
    File {
        path: String,
        from: Option<Group>,
    },
    Symbol(SymbolHit),
    /// A line with a match, as Find in files lists it (item 10.4): one row for each, with its file and its number.
    Line {
        path: String,
        hit: LineHit,
    },
}

/// A line as Find in files showed it (trimmed, maybe a window of it) and where its matches were, in characters.
#[derive(Clone, Debug, PartialEq)]
pub struct Shown {
    pub text: String,
    pub ranges: Vec<[u32; 2]>,
}

/// Where a picked row goes: the file, the line, and from Find in files the line as it was shown.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub path: String,
    pub line: Option<usize>,
    pub shown: Option<Shown>,
}

impl Row {
    /// A row ↑ and ↓ stop on, and ↩ or a click opens.
    pub fn selectable(&self) -> bool {
        matches!(self, Row::File { .. } | Row::Symbol(_) | Row::Line { .. })
    }

    pub fn target(&self) -> Option<Target> {
        match self {
            Row::File { path, .. } => Some(Target { path: path.clone(), line: None, shown: None }),
            Row::Symbol(hit) => Some(Target { path: hit.path.clone(), line: Some(hit.line as usize), shown: None }),
            Row::Line { path, hit } => Some(Target {
                path: path.clone(),
                line: Some(hit.line as usize),
                shown: Some(Shown { text: hit.text.clone(), ranges: hit.ranges.clone() }),
            }),
            Row::Label(_) => None,
        }
    }
}

/// The first row ↩ would open: where a new search or another tab starts.
pub fn first_selectable(rows: &[Row]) -> Option<usize> {
    rows.iter().position(Row::selectable)
}

/// The row ↓ (`by` 1) or ↑ (-1) goes to from `active`: the next one that can be opened, or `active` at the ends.
pub fn step(rows: &[Row], active: Option<usize>, by: i32) -> Option<usize> {
    let Some(from) = active else { return first_selectable(rows) };
    let mut at = from as i64 + i64::from(by);
    while at >= 0 && (at as usize) < rows.len() {
        if rows[at as usize].selectable() {
            return Some(at as usize);
        }
        at += i64::from(by);
    }
    Some(from)
}

/// What a tab says of its search at the right of the tabs: the words, and whether it is still at work (a small
/// turning mark stands before them then).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    pub busy: bool,
    pub words: String,
}

impl Status {
    fn said(words: String) -> Status {
        Status { busy: false, words }
    }

    fn at_work(words: &str) -> Status {
        Status { busy: true, words: words.into() }
    }
}

/// A part of a note: its words, and whether they are code (drawn in the code face).
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub text: String,
    pub code: bool,
}

/// What a tab says in place of results: how to use it, that nothing was found, that it reads, or what went wrong.
#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub parts: Vec<Part>,
    /// Something went wrong (Text draws it in red).
    pub problem: bool,
    /// It only says that something is being read: shown once the wait is long enough to notice.
    pub loading: bool,
}

impl Note {
    fn words(text: impl Into<String>) -> Note {
        Note { parts: vec![Part { text: text.into(), code: false }], problem: false, loading: false }
    }

    fn reading(text: &str) -> Note {
        Note { loading: true, ..Note::words(text) }
    }

    fn problem(text: &str) -> Note {
        Note { problem: true, ..Note::words(text) }
    }

    /// The whole note as one line.
    pub fn said(&self) -> String {
        self.parts.iter().map(|part| part.text.as_str()).collect()
    }
}

// Files

/// The Files tab's rows. With something typed: what the filter found, best first. Before that: the changed files
/// (not the deleted ones), then the files opened lately that are still there, each under its heading.
pub fn file_rows(found: Option<&Found>, changes: &[Change], recent: &[String], visible: Option<&Files>) -> Vec<Row> {
    if let Some(found) = found {
        return found.matches.iter().map(|path| Row::File { path: path.clone(), from: None }).collect();
    }
    let changed: Vec<&str> =
        changes.iter().filter(|change| change.kind != "deleted").map(|change| change.path.as_str()).collect();
    // a file opened lately may be gone since; while the list isn't read yet, nothing says so
    let there = |path: &str| visible.is_none_or(|files| files.paths().iter().any(|file| file == path));
    let lately: Vec<&str> =
        recent.iter().map(String::as_str).filter(|path| !changed.contains(path) && there(path)).collect();
    let mut rows = Vec::new();
    for (group, paths) in [(Group::Changed, changed), (Group::Recent, lately)] {
        if !paths.is_empty() {
            rows.push(Row::Label(group));
            rows.extend(paths.into_iter().map(|path| Row::File { path: path.to_string(), from: Some(group) }));
        }
    }
    rows.truncate(FILE_LIMIT);
    rows
}

/// "12 of 1,234 files" with something typed, "1,234 files" without; nothing until the list is read.
pub fn files_status(found: Option<&Found>, visible: Option<&Files>) -> Status {
    let Some(files) = visible else { return Status::default() };
    let all = thousands(files.len());
    Status::said(match found {
        Some(found) => format!("{} of {all} files", thousands(found.total)),
        None => format!("{all} files"),
    })
}

pub fn files_note(visible: Option<&Files>, rows: &[Row], typed: &str) -> Option<Note> {
    if visible.is_none() {
        return Some(Note::reading("Reading the files…"));
    }
    if !rows.is_empty() {
        return None;
    }
    let typed = typed.trim();
    Some(Note::words(if typed.is_empty() {
        "Type part of a file name.".to_string()
    } else {
        format!("No file matches \u{201c}{typed}\u{201d}.")
    }))
}

// Symbols

/// What the Symbols tab knows: the core's last answer, whether a question is out, and what went wrong.
#[derive(Clone, Debug, Default)]
pub struct Symbols {
    pub results: Option<SymbolResults>,
    pub busy: bool,
    pub error: Option<String>,
}

impl Symbols {
    /// Nothing is typed any more: the answers go, the index's size stays.
    pub fn cleared(&mut self) {
        self.busy = false;
        self.error = None;
        if let Some(results) = &mut self.results {
            results.hits.clear();
            results.total = 0;
        }
    }

    pub fn rows(&self, typed: &str) -> Vec<Row> {
        match &self.results {
            Some(results) if !typed.trim().is_empty() => results.hits.iter().cloned().map(Row::Symbol).collect(),
            _ => Vec::new(),
        }
    }

    pub fn status(&self, typed: &str) -> Status {
        match &self.results {
            _ if self.busy => Status::at_work(if self.results.is_some() { "Searching" } else { "Reading definitions" }),
            Some(results) if !typed.trim().is_empty() => Status::said(format!(
                "{} found in {} definitions",
                thousands(results.total),
                thousands(results.symbols)
            )),
            Some(results) => Status::said(format!(
                "{} definitions in {} files",
                thousands(results.symbols),
                thousands(results.files)
            )),
            None => Status::default(),
        }
    }

    pub fn note(&self, typed: &str) -> Option<Note> {
        let typed = typed.trim();
        if let Some(error) = &self.error {
            return Some(Note::words(error.clone()));
        }
        if typed.is_empty() {
            let part = |text: &str, code: bool| Part { text: text.into(), code };
            return Some(Note {
                parts: vec![
                    part("Type a class, function or method name. Letters in order are enough: ", false),
                    part("slv", true),
                    part(" finds ", false),
                    part("StockLevelView", true),
                    part(".", false),
                ],
                problem: false,
                loading: false,
            });
        }
        match &self.results {
            Some(results) if !self.busy && results.hits.is_empty() => {
                Some(Note::words(format!("No definition matches \u{201c}{typed}\u{201d}.")))
            }
            None if self.busy => {
                Some(Note::reading("Reading the definitions in this workspace (only the first time)."))
            }
            _ => None,
        }
    }
}

/// The letter that stands for a kind of definition.
pub fn kind_letter(kind: Kind) -> &'static str {
    match kind {
        Kind::Class => "C",
        Kind::Interface => "I",
        Kind::Struct => "S",
        Kind::Enum => "E",
        Kind::Trait | Kind::Type => "T",
        Kind::Module => "M",
        Kind::Component => "V",
        Kind::Function => "ƒ",
        Kind::Method => "m",
    }
}

/// Each kind's own hue, so classes, functions and methods tell apart at a glance.
pub fn kind_hue(kind: Kind) -> f32 {
    match kind {
        Kind::Class => 250.,
        Kind::Interface | Kind::Trait => 160.,
        Kind::Struct | Kind::Type => 130.,
        Kind::Enum => 70.,
        Kind::Module => 35.,
        Kind::Component => 205.,
        Kind::Function => 300.,
        Kind::Method => 195.,
    }
}

/// A kind in a word, for who can't see its letter.
pub fn kind_word(kind: Kind) -> &'static str {
    match kind {
        Kind::Class => "class",
        Kind::Interface => "interface",
        Kind::Struct => "struct",
        Kind::Enum => "enum",
        Kind::Trait => "trait",
        Kind::Type => "type",
        Kind::Module => "module",
        Kind::Component => "component",
        Kind::Function => "function",
        Kind::Method => "method",
    }
}

// Text

/// What the Text tab knows: the files with matching lines as they arrived, the totals once the scan is over,
/// whether a scan runs, and what went wrong.
#[derive(Clone, Debug, Default)]
pub struct Text {
    pub found: Vec<FileHits>,
    pub summary: Option<TextSummary>,
    pub busy: bool,
    pub error: Option<String>,
    /// The scan that runs has sent nothing yet: its first files take the place of the last scan's.
    first: bool,
    /// The name the scan that runs looks for, when it looks for a plain name: a line that defines it is listed
    /// first. Nothing for a regular expression.
    name: Option<String>,
    /// For each file of `found`, in step with it: which of its lines define that name.
    defining: Vec<Vec<bool>>,
    /// The kind of file the scan that runs keeps to, to say it when nothing is found.
    kind: Option<String>,
}

/// Whether a line, as Find in files shows it, defines `name`: the scanner of the definitions index finds a class,
/// a function or the like of that name on it (small and capital letters count as the same, as in the search).
fn defines(path: &str, line: &str, name: &str) -> bool {
    // only a line that names it can define it, and most lines of most files define nothing
    let holds = line.len() >= name.len()
        && line.as_bytes().windows(name.len()).any(|part| part.eq_ignore_ascii_case(name.as_bytes()));
    holds && enkidu_core::symbols::extract(path, line).iter().any(|def| def.name.eq_ignore_ascii_case(name))
}

impl Text {
    /// Nothing is typed any more.
    pub fn cleared(&mut self) {
        *self = Text::default();
    }

    /// A new scan starts, for what `options` make of what was typed. What the last one found stays on screen until
    /// the new one's first files arrive.
    pub fn started(&mut self, typed: &str, options: &TextOptions) {
        self.busy = true;
        self.error = None;
        self.first = true;
        let name = typed.trim();
        let is_name = !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '$');
        self.name = (is_name && !options.regex).then(|| name.to_string());
        self.kind = options.kind.clone();
    }

    /// Files with matching lines arrived, in path order.
    pub fn arrived(&mut self, batch: Vec<FileHits>) {
        let name = self.name.as_deref();
        let defining = batch.iter().map(|file| {
            file.lines.iter().map(|hit| name.is_some_and(|name| defines(&file.path, &hit.text, name))).collect()
        });
        if std::mem::take(&mut self.first) {
            self.defining = defining.collect();
            self.found = batch;
        } else {
            self.defining.extend(defining);
            self.found.extend(batch);
        }
    }

    /// The scan is over. One that a newer scan stopped changes nothing.
    pub fn finished(&mut self, outcome: Result<TextSummary, String>) {
        match outcome {
            Ok(summary) if summary.cancelled => return,
            Ok(summary) => {
                // it found nothing at all: the last scan's lines go
                if std::mem::take(&mut self.first) {
                    self.found.clear();
                }
                self.summary = Some(summary);
            }
            Err(why) => {
                self.found.clear();
                self.defining.clear();
                self.summary = None;
                self.error = Some(why);
                self.first = false;
            }
        }
        self.busy = false;
    }

    /// One row for each line with a match (item 10.4, as PyCharm's Find in Files lists them): first the lines that
    /// define the name that is looked for, then the others, each group in the order the scan gave (by path, then
    /// by line).
    pub fn rows(&self, typed: &str) -> Vec<Row> {
        if typed.trim().is_empty() {
            return Vec::new();
        }
        let lines = || {
            self.found.iter().enumerate().flat_map(|(file_ix, file)| {
                let defining = self.defining.get(file_ix);
                file.lines.iter().enumerate().map(move |(ix, hit)| {
                    (defining.and_then(|lines| lines.get(ix)).copied().unwrap_or(false), &file.path, hit)
                })
            })
        };
        let row = |(_, path, hit): (bool, &String, &LineHit)| Row::Line { path: path.clone(), hit: hit.clone() };
        let mut rows: Vec<Row> = lines().filter(|(defines, ..)| *defines).map(row).collect();
        rows.extend(lines().filter(|(defines, ..)| !*defines).map(row));
        rows
    }

    /// "6 lines in 4 files". The lines a file has past the ones listed for it are counted in: "50 of 73 lines in
    /// 1 file".
    pub fn status(&self, typed: &str) -> Status {
        if self.busy {
            return Status::at_work("Searching");
        }
        let Some(summary) = self.summary.as_ref().filter(|_| !typed.trim().is_empty()) else {
            return Status::default();
        };
        let files = thousands(summary.matched_files);
        let files_word = if summary.matched_files == 1 { "file" } else { "files" };
        let more: usize = self.found.iter().map(|file| file.more as usize).sum();
        Status::said(if summary.truncated {
            format!("First {} lines; {files} files match", thousands(summary.lines))
        } else if more > 0 {
            let all = summary.lines + more;
            format!("{} of {} lines in {files} {files_word}", thousands(summary.lines), thousands(all))
        } else {
            let lines_word = if summary.lines == 1 { "line" } else { "lines" };
            format!("{} {lines_word} in {files} {files_word}", thousands(summary.lines))
        })
    }

    pub fn note(&self, typed: &str) -> Option<Note> {
        let typed = typed.trim();
        if let Some(error) = &self.error {
            return Some(Note::problem(error));
        }
        if typed.is_empty() {
            return Some(Note::words(
                "Searches the workspace's files as they are on disk now: the ones git tracks, the new ones and its \
                 submodules', not the ignored ones. The buttons on the right keep to one kind of file, match case, \
                 whole words or a regular expression (⌥⌘C, ⌥⌘W, ⌥⌘R).",
            ));
        }
        let summary = self.summary.as_ref().filter(|_| !self.busy && self.found.is_empty())?;
        let skipped = match summary.skipped {
            0 => String::new(),
            count => format!(" {count} large or binary files were skipped."),
        };
        let within = match &self.kind {
            Some(kind) => format!(" in {} files", kind_words(Some(kind))),
            None => String::new(),
        };
        Some(Note::words(format!("Nothing found for \u{201c}{typed}\u{201d}{within}.{skipped}")))
    }
}

/// Where the matches of a line that Find in files showed sit in the real line, as byte ranges. The popup shows a
/// line without its indentation, and only a window of a long one ("…" at the cut ends), with the matches counted
/// in characters of what it showed; this finds that text in the line as it is in the file. None when the line no
/// longer has it (the file changed since).
pub fn ranges_in_line(line: &str, shown: &Shown) -> Option<Vec<Range<usize>>> {
    if shown.ranges.is_empty() {
        return None;
    }
    let lead = usize::from(shown.text.starts_with('…'));
    let mut core = shown.text.strip_prefix('…').unwrap_or(&shown.text);
    if shown.text.chars().count() > lead + 1 {
        core = core.strip_suffix('…').unwrap_or(core);
    }
    let at = line.find(core)?;
    // a character of the shown text, counted from its start, as a byte of the line
    let before = line[..at].chars().count();
    let byte_of = |shown_char: u32| -> Option<usize> {
        let nth = (shown_char as usize + before).checked_sub(lead)?;
        match line.char_indices().nth(nth) {
            Some((byte, _)) => Some(byte),
            None => (nth == line.chars().count()).then_some(line.len()),
        }
    };
    shown.ranges.iter().map(|[start, end]| Some(byte_of(*start)?..byte_of(*end)?)).collect()
}

// The files opened lately

/// The files opened lately in each workspace, the last one first: what the Files tab offers before anything is
/// typed. Kept across launches.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Recent(HashMap<String, Vec<String>>);

impl Recent {
    /// What was kept. Anything that can't be read is nothing kept.
    pub fn read(text: &str) -> Recent {
        Recent(serde_json::from_str(text).unwrap_or_default())
    }

    pub fn write(&self) -> String {
        // by workspace, so the same lists always write the same text
        let sorted: std::collections::BTreeMap<&String, &Vec<String>> = self.0.iter().collect();
        serde_json::to_string(&sorted).unwrap_or_else(|_| "{}".into())
    }

    pub fn of(&self, workspace: &str) -> &[String] {
        self.0.get(workspace).map_or(&[], Vec::as_slice)
    }

    /// A file was opened: it is the first now. False when it was the first already.
    pub fn opened(&mut self, workspace: &str, path: &str) -> bool {
        let list = self.0.entry(workspace.to_string()).or_default();
        if list.first().is_some_and(|first| first == path) {
            return false;
        }
        list.retain(|kept| kept != path);
        list.insert(0, path.to_string());
        list.truncate(MAX_RECENT);
        true
    }

    /// The lists of workspaces that are gone go too. False when none was.
    pub fn keep_only(&mut self, workspaces: &[String]) -> bool {
        let before = self.0.len();
        self.0.retain(|id, _| workspaces.contains(id));
        self.0.len() != before
    }
}

// Shift, tapped twice

/// A tap is a press of Shift let go within this time.
pub const TAP: Duration = Duration::from_millis(350);
/// The second tap ends within this time of the first one's end.
pub const BETWEEN_TAPS: Duration = Duration::from_millis(400);

/// ⇧⇧ opens Search, as in PyCharm. Only two clean taps of Shift count: a Shift held for a capital letter, with
/// another key or modifier, with a click, or held for a while, never does.
#[derive(Clone, Copy, Debug, Default)]
pub struct ShiftTaps {
    /// Shift alone is down since then.
    down: Option<Instant>,
    /// Something else was pressed while it was down.
    other: bool,
    /// When the last clean tap ended.
    last_tap: Option<Instant>,
}

impl ShiftTaps {
    /// The modifier keys changed: whether Shift is down now, and whether any other modifier is. True when this
    /// was the second clean tap.
    pub fn modifiers(&mut self, shift: bool, others: bool, now: Instant) -> bool {
        if others {
            // another modifier with it, or on its own: no tap, and the one before is forgotten
            self.other();
            self.down = if shift { self.down.or(Some(now)) } else { None };
            return false;
        }
        match (self.down, shift) {
            (None, true) => {
                self.down = Some(now);
                self.other = false;
                false
            }
            (Some(since), false) => {
                self.down = None;
                self.released(since, now)
            }
            _ => false,
        }
    }

    /// Any other key went down, or the mouse was pressed.
    pub fn other(&mut self) {
        self.other = true;
        self.last_tap = None;
    }

    fn released(&mut self, since: Instant, now: Instant) -> bool {
        if self.other {
            return false;
        }
        if now.saturating_duration_since(since) > TAP {
            self.last_tap = None;
            return false;
        }
        match self.last_tap.take() {
            Some(last) if now.saturating_duration_since(last) < BETWEEN_TAPS => true,
            _ => {
                self.last_tap = Some(now);
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(path: &str, kind: &str) -> Change {
        Change {
            path: path.into(),
            old_path: None,
            kind: kind.into(),
            staged: false,
            additions: Some(1),
            deletions: Some(0),
            sig: path.into(),
        }
    }

    fn files(paths: &[&str]) -> Files {
        Files::new(paths.iter().map(|path| path.to_string()).collect())
    }

    fn file(path: &str, from: Option<Group>) -> Row {
        Row::File { path: path.into(), from }
    }

    #[test]
    fn the_tabs_go_around_with_their_keys_and_words() {
        assert_eq!(Tab::default(), Tab::Files);
        assert_eq!(Tab::ALL.map(Tab::label), ["Files", "Symbols", "Text"]);
        assert_eq!(Tab::ALL.map(Tab::shortcut), ["⇧⌘O", "⌘O", "⇧⌘F"]);
        assert_eq!(Tab::Files.step(1), Tab::Symbols);
        assert_eq!(Tab::Text.step(1), Tab::Files, "⇥ on the last tab goes to the first");
        assert_eq!(Tab::Files.step(-1), Tab::Text, "⇧⇥ on the first goes to the last");
        assert_eq!(Tab::Files.placeholder("stock-fix"), "A file in stock-fix");
        assert_eq!(Tab::Symbols.placeholder("stock-fix"), "A class, function or method");
        assert_eq!(Tab::Text.placeholder("stock-fix"), "Text in stock-fix's files");
        assert_eq!((Tab::Files.width(), Tab::Symbols.width(), Tab::Text.width()), (680., 680., 920.));
    }

    #[test]
    fn before_typing_files_lists_the_changed_files_then_the_ones_opened_lately() {
        let visible = files(&["a.py", "docs/guide.md", "src/app.py", "src/new.py"]);
        let changes =
            [change("src/app.py", "modified"), change("gone.py", "deleted"), change("src/new.py", "untracked")];
        let recent: Vec<String> = ["docs/guide.md", "src/app.py", "removed-since.py"].map(String::from).to_vec();
        let rows = file_rows(None, &changes, &recent, Some(&visible));
        assert_eq!(
            rows,
            [
                Row::Label(Group::Changed),
                file("src/app.py", Some(Group::Changed)),
                file("src/new.py", Some(Group::Changed)),
                // a changed file isn't listed twice, and one that is gone isn't listed
                Row::Label(Group::Recent),
                file("docs/guide.md", Some(Group::Recent)),
            ]
        );
        assert_eq!((Group::Changed.label(), Group::Recent.label()), ("Changed", "Opened lately"));
        // until the list is read, nothing says that a file is gone
        let unread = file_rows(None, &[], &recent, None);
        assert_eq!(unread.len(), 4);
        // nothing changed and nothing opened: no heading of nothing
        assert!(file_rows(None, &[], &[], Some(&visible)).is_empty());
        // at most 60 rows
        let many: Vec<Change> = (0..80).map(|n| change(&format!("f{n}.py"), "modified")).collect();
        assert_eq!(file_rows(None, &many, &[], None).len(), FILE_LIMIT);
    }

    #[test]
    fn typed_files_lists_what_the_filter_found_and_says_how_many() {
        let visible = files(&["a.py", "docs/app-notes.md", "src/app.py"]);
        let found = visible.filter("app", FILE_LIMIT);
        let rows = file_rows(Some(&found), &[change("a.py", "modified")], &[], Some(&visible));
        assert_eq!(rows, [file("src/app.py", None), file("docs/app-notes.md", None)]);
        assert_eq!(files_status(Some(&found), Some(&visible)).words, "2 of 3 files");
        assert_eq!(files_status(None, Some(&visible)).words, "3 files");
        assert_eq!(files_status(None, None), Status::default(), "nothing until the list is read");
        // thousands are grouped: the files whose number has a 1 in it, of all
        let big = Files::new((0..12_345).map(|n| format!("f{n}.py")).collect());
        let with_a_one = (0..12_345).filter(|n: &i32| n.to_string().contains('1')).count();
        assert!(with_a_one > 1000);
        assert_eq!(
            files_status(Some(&big.filter("f1", 5)), Some(&big)).words,
            format!("{} of 12,345 files", thousands(with_a_one))
        );

        assert_eq!(files_note(Some(&visible), &rows, "app"), None);
        assert_eq!(files_note(Some(&visible), &[], " zzz ").unwrap().said(), "No file matches \u{201c}zzz\u{201d}.");
        assert_eq!(files_note(Some(&visible), &[], "").unwrap().said(), "Type part of a file name.");
        let reading = files_note(None, &[], "app").unwrap();
        assert_eq!((reading.said().as_str(), reading.loading), ("Reading the files…", true));
    }

    #[test]
    fn up_and_down_stop_on_the_rows_that_open() {
        let rows = [
            Row::Label(Group::Changed),
            file("a.py", Some(Group::Changed)),
            Row::Label(Group::Recent),
            file("b.py", Some(Group::Recent)),
            file("c.py", Some(Group::Recent)),
        ];
        assert_eq!(first_selectable(&rows), Some(1));
        assert_eq!(step(&rows, Some(1), 1), Some(3), "down skips the heading");
        assert_eq!(step(&rows, Some(3), -1), Some(1));
        assert_eq!(step(&rows, Some(1), -1), Some(1), "up at the first row stays");
        assert_eq!(step(&rows, Some(4), 1), Some(4), "down at the last row stays");
        assert_eq!(step(&rows, None, 1), Some(1));
        assert_eq!(first_selectable(&[Row::Label(Group::Changed)]), None);
        assert_eq!(step(&[], None, 1), None);
    }

    fn hit(name: &str, kind: Kind, container: Option<&str>, path: &str, line: u32) -> SymbolHit {
        SymbolHit {
            name: name.into(),
            kind,
            container: container.map(String::from),
            path: path.into(),
            line,
            indices: vec![0],
        }
    }

    #[test]
    fn symbols_says_what_it_reads_what_it_found_and_how_to_ask() {
        let mut symbols = Symbols::default();
        // nothing typed: how to ask, with the example in the code face
        let hint = symbols.note("").unwrap();
        assert_eq!(
            hint.said(),
            "Type a class, function or method name. Letters in order are enough: slv finds StockLevelView."
        );
        let code: Vec<&str> = hint.parts.iter().filter(|part| part.code).map(|part| part.text.as_str()).collect();
        assert_eq!(code, ["slv", "StockLevelView"]);
        assert_eq!(symbols.status(""), Status::default());

        // the first question: the definitions are being read
        symbols.busy = true;
        assert_eq!(symbols.status("slv"), Status { busy: true, words: "Reading definitions".into() });
        let reading = symbols.note("slv").unwrap();
        assert!(reading.loading);
        assert_eq!(reading.said(), "Reading the definitions in this workspace (only the first time).");

        // its answer
        symbols.busy = false;
        symbols.results = Some(SymbolResults {
            hits: vec![
                hit("StockLevelView", Kind::Class, None, "src/views.py", 12),
                hit("save", Kind::Method, Some("StockLevelView"), "src/views.py", 40),
            ],
            total: 2,
            symbols: 251_578,
            files: 13_580,
            ..Default::default()
        });
        assert_eq!(symbols.rows("slv").len(), 2);
        assert_eq!(symbols.status("slv").words, "2 found in 251,578 definitions");
        assert_eq!(symbols.note("slv"), None);
        let target = symbols.rows("slv")[1].target().unwrap();
        assert_eq!((target.path.as_str(), target.line, target.shown), ("src/views.py", Some(40), None));

        // a later question
        symbols.busy = true;
        assert_eq!(symbols.status("slvx"), Status { busy: true, words: "Searching".into() });
        assert_eq!(symbols.note("slvx"), None, "the last answer stays while the next one comes");
        symbols.busy = false;
        symbols.results.as_mut().unwrap().hits.clear();
        symbols.results.as_mut().unwrap().total = 0;
        assert_eq!(symbols.note(" slvx ").unwrap().said(), "No definition matches \u{201c}slvx\u{201d}.");

        // the field emptied: the index's size is said, and nothing is listed
        symbols.cleared();
        assert!(symbols.rows("").is_empty());
        assert_eq!(symbols.status("").words, "251,578 definitions in 13,580 files");

        // what went wrong is said as it is
        symbols.error = Some("That workspace is gone".into());
        assert_eq!(symbols.note("slv").unwrap().said(), "That workspace is gone");
    }

    #[test]
    fn each_kind_of_definition_has_its_letter_and_its_hue() {
        let kinds = [
            Kind::Class,
            Kind::Interface,
            Kind::Struct,
            Kind::Enum,
            Kind::Trait,
            Kind::Type,
            Kind::Module,
            Kind::Component,
            Kind::Function,
            Kind::Method,
        ];
        assert_eq!(kinds.map(kind_letter), ["C", "I", "S", "E", "T", "T", "M", "V", "ƒ", "m"]);
        assert_eq!(kinds.map(kind_hue), [250., 160., 130., 70., 160., 130., 35., 205., 300., 195.]);
        assert_eq!(kind_word(Kind::Component), "component");
    }

    fn line(number: u32, text: &str, ranges: &[[u32; 2]]) -> LineHit {
        LineHit { line: number, text: text.into(), ranges: ranges.to_vec() }
    }

    fn summary(lines: usize, matched_files: usize, ms: u64) -> TextSummary {
        TextSummary { lines, files: matched_files, matched_files, ms, ..Default::default() }
    }

    #[test]
    fn text_lists_one_row_for_each_line_as_they_arrive_the_defining_line_first() {
        let mut text = Text::default();
        assert_eq!(
            text.note("").unwrap().said(),
            "Searches the workspace's files as they are on disk now: the ones git tracks, the new ones and its \
             submodules', not the ignored ones. The buttons on the right keep to one kind of file, match case, whole \
             words or a regular expression (⌥⌘C, ⌥⌘W, ⌥⌘R)."
        );
        let plain = TextOptions::default();
        text.started("StockLevel", &plain);
        assert_eq!(text.status("StockLevel"), Status { busy: true, words: "Searching".into() });
        assert_eq!(text.note("StockLevel"), None, "no words while it reads");
        // the scan gives the files by path: the notes, then the file that defines the name, then one that uses it
        text.arrived(vec![FileHits {
            path: "docs/api.md".into(),
            lines: vec![line(40, "`StockLevel` holds what is in stock", &[[1, 11]])],
            more: 0,
        }]);
        text.arrived(vec![
            FileHits {
                path: "src/orders/stock.py".into(),
                lines: vec![
                    line(4, "class StockLevel(models.Model):", &[[6, 16]]),
                    line(9, "return StockLevel.objects.all()", &[[7, 17]]),
                ],
                more: 0,
            },
            FileHits {
                path: "src/orders/views.py".into(),
                lines: vec![line(2, "from .stock import StockLevel", &[[19, 29]])],
                more: 3,
            },
        ]);
        let places = |text: &Text, typed: &str| -> Vec<String> {
            (text.rows(typed).iter())
                .map(|row| match row {
                    Row::Line { path, hit } => format!("{path}:{}", hit.line),
                    other => format!("{other:?}"),
                })
                .collect()
        };
        // one row for each line, nothing else: no heading for a file, no "3 more lines". The line that defines the
        // name stands first, the others as the scan gave them
        assert_eq!(
            places(&text, "StockLevel"),
            ["src/orders/stock.py:4", "docs/api.md:40", "src/orders/stock.py:9", "src/orders/views.py:2"]
        );
        let rows = text.rows("StockLevel");
        assert!(rows.iter().all(Row::selectable));
        assert_eq!(first_selectable(&rows), Some(0));
        assert_eq!(step(&rows, Some(0), 1), Some(1), "every row is a line that opens");
        // ↩ on a line goes to it, with the line as it was shown
        let target = rows[0].target().unwrap();
        assert_eq!((target.path.as_str(), target.line), ("src/orders/stock.py", Some(4)));
        assert_eq!(target.shown, Some(Shown { text: "class StockLevel(models.Model):".into(), ranges: vec![[6, 16]] }));

        // the lines a file has past the ones listed for it are counted, and no time is said
        text.finished(Ok(summary(4, 3, 14)));
        assert_eq!(text.status("StockLevel"), Status { busy: false, words: "4 of 7 lines in 3 files".into() });
        assert_eq!(text.status(""), Status::default());

        // small letters find the same definition; what isn't a name (a space in it, a regular expression) has no
        // defining line, and the rows are as the scan gave them
        let by_path = ["docs/api.md:40", "src/orders/stock.py:4", "src/orders/stock.py:9", "src/orders/views.py:2"];
        let found = text.found.clone();
        text.started("stocklevel", &plain);
        text.arrived(found.clone());
        assert_eq!(places(&text, "stocklevel")[0], "src/orders/stock.py:4");
        text.started("class StockLevel", &plain);
        text.arrived(found.clone());
        assert_eq!(places(&text, "class StockLevel"), by_path);
        text.started("StockLevel", &TextOptions { regex: true, ..TextOptions::default() });
        text.arrived(found);
        assert_eq!(places(&text, "StockLevel"), by_path);

        // the next scan: the lines stay until its first files arrive, then they are replaced
        text.started("StockLevels", &plain);
        assert_eq!(text.rows("StockLevels").len(), 4);
        text.arrived(vec![FileHits {
            path: "src/app.py".into(),
            lines: vec![line(4, "StockLevels = 1", &[[0, 11]])],
            more: 0,
        }]);
        assert_eq!(places(&text, "StockLevels"), ["src/app.py:4"]);
        text.finished(Ok(summary(1, 1, 3)));
        assert_eq!(text.status("StockLevels").words, "1 line in 1 file");

        // a scan that finds nothing empties the list and says so, with what it skipped and the kind it kept to
        text.started("zzz", &plain);
        text.finished(Ok(TextSummary { skipped: 2, ms: 5, ..Default::default() }));
        assert!(text.rows("zzz").is_empty());
        assert_eq!(text.status("zzz").words, "0 lines in 0 files");
        assert_eq!(
            text.note(" zzz ").unwrap().said(),
            "Nothing found for \u{201c}zzz\u{201d}. 2 large or binary files were skipped."
        );
        text.started("zzz", &plain);
        text.finished(Ok(TextSummary::default()));
        assert_eq!(text.note("zzz").unwrap().said(), "Nothing found for \u{201c}zzz\u{201d}.");
        text.started("zzz", &TextOptions { kind: Some("vue".into()), ..TextOptions::default() });
        text.finished(Ok(TextSummary::default()));
        assert_eq!(text.note("zzz").unwrap().said(), "Nothing found for \u{201c}zzz\u{201d} in *.vue files.");
    }

    #[test]
    fn text_says_when_it_lists_only_the_first_lines_and_what_went_wrong() {
        let plain = TextOptions::default();
        let mut text = Text::default();
        text.started("x", &plain);
        text.arrived(vec![FileHits { path: "a.py".into(), lines: vec![line(1, "x", &[[0, 1]])], more: 0 }]);
        text.finished(Ok(TextSummary {
            lines: 1000,
            matched_files: 4321,
            truncated: true,
            ms: 840,
            ..Default::default()
        }));
        assert_eq!(text.status("x").words, "First 1,000 lines; 4,321 files match");

        // a scan that a newer one stopped changes nothing, and the newer one is still at work
        text.started("x", &plain);
        text.finished(Ok(TextSummary { cancelled: true, ..Default::default() }));
        assert!(text.busy);
        assert_eq!(text.rows("x").len(), 1);

        // a pattern the engine refuses: its words, in red, and no lines
        text.finished(Err("regex parse error: unclosed group".into()));
        let note = text.note("(x").unwrap();
        assert!(note.problem);
        assert_eq!(note.said(), "regex parse error: unclosed group");
        assert!(text.rows("(x").is_empty() && !text.busy);
        assert_eq!(text.status("(x"), Status::default());
        // the next scan starts clean
        text.started("x", &plain);
        assert_eq!(text.note("x"), None);

        text.cleared();
        assert!(text.found.is_empty() && text.summary.is_none() && !text.busy);
    }

    #[test]
    fn the_options_toggle_keep_to_a_kind_of_file_and_the_text_goes_as_typed() {
        let mut options = TextOptions::default();
        assert!(options.query("   ").is_none(), "only spaces is nothing to look for");
        let query = options.query(" sum(").unwrap();
        assert_eq!(query.pattern, " sum(", "a space at the start is part of it");
        assert_eq!((query.case_sensitive, query.whole_word, query.regex), (None, false, false));
        assert_eq!(query.kind, None, "every file");

        options.toggle(Toggle::Case);
        assert_eq!(options.case_sensitive, Some(true));
        assert!(options.on(Toggle::Case) && !options.on(Toggle::Word) && !options.on(Toggle::Regex));
        options.toggle(Toggle::Case);
        assert_eq!(options.case_sensitive, None, "off is smart case again, not \"ignore case\"");
        options.toggle(Toggle::Word);
        options.toggle(Toggle::Regex);
        let query = options.query("sum").unwrap();
        assert!(query.whole_word && query.regex);
        assert_eq!(Toggle::ALL.map(Toggle::label), ["Match case", "Whole words", "Regular expression"]);
        assert_eq!(Toggle::ALL.map(Toggle::shortcut), ["⌥⌘C", "⌥⌘W", "⌥⌘R"]);

        // one kind of file: the core reads only those
        options.kind = Some("py".into());
        assert_eq!(options.query("sum").unwrap().kind.as_deref(), Some("py"));
        assert_eq!((kind_words(Some("py")), kind_words(None)), ("*.py".into(), "All files".into()));
    }

    #[test]
    fn a_match_shown_in_the_popup_is_found_again_in_the_real_line() {
        let shown = |text: &str, ranges: &[[u32; 2]]| Shown { text: text.into(), ranges: ranges.to_vec() };
        // the popup shows a line without its indentation
        let real = "        return sum(items)";
        let ranges = ranges_in_line(real, &shown("return sum(items)", &[[7, 10]])).unwrap();
        assert_eq!((ranges.len(), ranges[0].clone()), (1, 15..18));
        assert_eq!(&real[ranges[0].clone()], "sum");
        // two matches on one line
        let real = "\tsum(a) + sum(b)";
        let ranges = ranges_in_line(real, &shown("sum(a) + sum(b)", &[[0, 3], [9, 12]])).unwrap();
        assert_eq!(ranges.iter().map(|range| &real[range.clone()]).collect::<Vec<_>>(), ["sum", "sum"]);
        // a window of a long line: "…" at both cut ends, counted as one character at the start
        let real = format!("{}needle{}", "a".repeat(100), "b".repeat(300));
        let window = format!("…{}needle{}…", "a".repeat(60), "b".repeat(180));
        let ranges = ranges_in_line(&real, &shown(&window, &[[61, 67]])).unwrap();
        assert_eq!(&real[ranges[0].clone()], "needle");
        // letters that take more than one byte before the match
        let real = "    título = \"añadir\"  # añadir";
        let ranges = ranges_in_line(real, &shown("título = \"añadir\"  # añadir", &[[10, 16], [21, 27]])).unwrap();
        assert_eq!(ranges.iter().map(|range| &real[range.clone()]).collect::<Vec<_>>(), ["añadir", "añadir"]);
        // a match that ends the line
        let real = "  x = total";
        let ranges = ranges_in_line(real, &shown("x = total", &[[4, 9]])).unwrap();
        assert_eq!((ranges.len(), ranges[0].clone()), (1, 6..11));
        // the file changed since: the line doesn't have that text any more
        assert_eq!(ranges_in_line("    return len(items)", &shown("return sum(items)", &[[7, 10]])), None);
        // nothing was marked
        assert_eq!(ranges_in_line("x", &shown("x", &[])), None);
    }

    #[test]
    fn the_files_opened_lately_are_kept_fifteen_a_workspace_the_last_one_first() {
        let mut recent = Recent::default();
        assert!(recent.of("w1").is_empty());
        assert!(recent.opened("w1", "a.py"));
        assert!(recent.opened("w1", "b.py"));
        assert!(!recent.opened("w1", "b.py"), "the first one opened again changes nothing");
        assert!(recent.opened("w1", "a.py"));
        assert_eq!(recent.of("w1"), ["a.py", "b.py"], "opened again, it moves to the front and isn't listed twice");
        for n in 0..20 {
            recent.opened("w2", &format!("f{n}.py"));
        }
        assert_eq!(recent.of("w2").len(), MAX_RECENT);
        assert_eq!(recent.of("w2")[0], "f19.py");
        assert_eq!(recent.of("w2")[14], "f5.py");

        // kept as text, and read back the same
        let text = recent.write();
        assert_eq!(Recent::read(&text), recent);
        assert!(text.starts_with(r#"{"w1":["a.py","b.py"],"w2":["f19.py""#), "{text}");
        assert_eq!(Recent::read("not json"), Recent::default());
        assert_eq!(Recent::read(r#"{"w1": "broken"}"#), Recent::default());

        // a workspace that is gone is forgotten
        assert!(recent.keep_only(&["w1".to_string()]));
        assert!(recent.of("w2").is_empty());
        assert!(!recent.keep_only(&["w1".to_string()]));
    }

    #[test]
    fn only_two_clean_taps_of_shift_open_search() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        // two taps: down and up, down and up
        let mut taps = ShiftTaps::default();
        assert!(!taps.modifiers(true, false, at(0)));
        assert!(!taps.modifiers(false, false, at(60)));
        assert!(!taps.modifiers(true, false, at(180)));
        assert!(taps.modifiers(false, false, at(240)), "the second tap opens it");
        // and a third tap alone doesn't open it again
        assert!(!taps.modifiers(true, false, at(300)));
        assert!(!taps.modifiers(false, false, at(340)));

        // a Shift held for a capital letter never does
        let mut taps = ShiftTaps::default();
        taps.modifiers(true, false, at(0));
        taps.other();
        assert!(!taps.modifiers(false, false, at(80)));
        taps.modifiers(true, false, at(150));
        assert!(!taps.modifiers(false, false, at(200)), "one clean tap after a capital letter");

        // a key typed between the taps
        let mut taps = ShiftTaps::default();
        taps.modifiers(true, false, at(0));
        taps.modifiers(false, false, at(50));
        taps.other();
        taps.modifiers(true, false, at(120));
        assert!(!taps.modifiers(false, false, at(170)));

        // a Shift held for a while (a drag, a pause) is no tap
        let mut taps = ShiftTaps::default();
        taps.modifiers(true, false, at(0));
        assert!(!taps.modifiers(false, false, at(400)));
        taps.modifiers(true, false, at(450));
        assert!(!taps.modifiers(false, false, at(500)), "the long press before doesn't count as the first tap");

        // two taps too far apart
        let mut taps = ShiftTaps::default();
        taps.modifiers(true, false, at(0));
        taps.modifiers(false, false, at(50));
        taps.modifiers(true, false, at(500));
        assert!(!taps.modifiers(false, false, at(550)));
        // but the last one is a first tap of its own
        taps.modifiers(true, false, at(650));
        assert!(taps.modifiers(false, false, at(700)));

        // Shift with another modifier (⇧⌘O) is no tap, whichever goes down first
        let mut taps = ShiftTaps::default();
        taps.modifiers(true, false, at(0));
        taps.modifiers(true, true, at(20));
        taps.modifiers(true, false, at(60));
        assert!(!taps.modifiers(false, false, at(90)));
        taps.modifiers(false, true, at(100));
        taps.modifiers(true, true, at(120));
        taps.modifiers(false, true, at(140));
        assert!(!taps.modifiers(false, false, at(160)));
        taps.modifiers(true, false, at(200));
        assert!(!taps.modifiers(false, false, at(240)), "one clean tap after them");
    }
}

// Going to a definition

/// What heads the list of a name's definitions, when a ⌘-click finds several (item 10.11).
pub fn definitions_words(count: usize, word: &str) -> String {
    match count {
        1 => format!("1 definition of {word}"),
        _ => format!("{count} definitions of {word}"),
    }
}

/// A definition as that list says it: its kind in a word and a letter (a value a class or a file sets has "=" and
/// no hue), the class it belongs to, its file's name with its line, and the folder the file is in.
/// Two `models.py` are told apart by their folders.
#[derive(Clone, Debug, PartialEq)]
pub struct PlaceWords {
    pub kind: &'static str,
    pub letter: &'static str,
    pub hue: Option<f32>,
    pub within: Option<String>,
    pub place: String,
    /// None for a file at the workspace's top.
    pub folder: Option<String>,
}

/// The most letters of a folder the list shows. A longer one keeps its end: the folders nearest the file.
const FOLDER_SHOWN: usize = 40;

pub fn place_words(place: &enkidu_core::goto::Place) -> PlaceWords {
    let (folder, name) = crate::review::split_path(&place.path);
    PlaceWords {
        kind: place.kind.map_or("value", kind_word),
        letter: place.kind.map_or("=", kind_letter),
        hue: place.kind.map(kind_hue),
        within: place.container.as_ref().map(|class| format!("in {class}")),
        place: format!("{name}:{}", place.line),
        folder: (!folder.is_empty()).then(|| folder_end(folder, FOLDER_SHOWN)),
    }
}

/// A folder's path: whole when it has at most `most` letters, else its last folders after "…/".
fn folder_end(folder: &str, most: usize) -> String {
    if folder.chars().count() <= most {
        return folder.to_string();
    }
    let (mut kept, mut letters) = (Vec::new(), 2);
    for part in folder.rsplit('/') {
        let more = part.chars().count() + usize::from(!kept.is_empty());
        if letters + more > most && !kept.is_empty() {
            break;
        }
        kept.push(part);
        letters += more;
    }
    kept.reverse();
    format!("…/{}", kept.join("/"))
}

#[cfg(test)]
mod definition_tests {
    use super::*;
    use enkidu_core::goto::Place;

    #[test]
    fn a_definition_is_said_by_its_kind_its_class_and_its_file_with_its_line() {
        assert_eq!(definitions_words(3, "save"), "3 definitions of save");
        // a single place found by its name alone is listed too, not gone to
        assert_eq!(definitions_words(1, "year"), "1 definition of year");
        let method = Place {
            path: "src/orders/models.py".into(),
            line: 50,
            name: "save".into(),
            kind: Some(Kind::Method),
            container: Some("StockItem".into()),
        };
        let said = place_words(&method);
        assert_eq!((said.kind, said.letter), ("method", "m"));
        assert_eq!((said.within.as_deref(), said.place.as_str()), (Some("in StockItem"), "models.py:50"));
        assert_eq!(said.folder.as_deref(), Some("src/orders"), "the folder tells two `models.py` apart");
        assert!(said.hue.is_some());
        // a value a file sets: no kind the index knows
        let value = Place { path: "constants.py".into(), line: 4, name: "DAYS".into(), kind: None, container: None };
        let said = place_words(&value);
        assert_eq!((said.kind, said.letter, said.hue), ("value", "=", None));
        assert_eq!((said.within, said.place.as_str()), (None, "constants.py:4"));
        assert_eq!(said.folder, None, "a file at the workspace's top has no folder to say");
        // a long folder keeps its end, the folders nearest the file
        assert_eq!(folder_end("apps/orders/views", 40), "apps/orders/views");
        assert_eq!(
            folder_end("backend/services/warehouse/apps/stock_levels/api/v2/views", 40),
            "…/apps/stock_levels/api/v2/views"
        );
        assert_eq!(folder_end("a_single_folder_whose_name_is_longer_than_the_room_there_is", 20).chars().count(), 61);
    }
}
