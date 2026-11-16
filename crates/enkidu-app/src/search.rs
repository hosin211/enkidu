//! Search (DESIGN.md, "Search, like PyCharm's Search Everywhere"; item 4.1): one popup with three tabs, opened from the
//! keyboard or the palette, never a field on screen. Files (⇧⌘O, ⌘P) finds names in git's list, and before
//! anything is typed offers the changed files and the ones opened lately. Symbols (⌘O) finds classes, functions and
//! methods in the core's index. Text (⇧⌘F) reads the files and lists every line with a match, by file. ⇧⇧ opens
//! the tab used last, ⇥ goes to the next tab, ↑ ↓ and ↩ open a result where the diffs are, at its line. What it
//! shows is worked out in `enkidu_model::search`; nothing here touches git or a file on the drawing thread. What the
//! keyboard opens many times a day is instant: no motion.

use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use enkidu_core::git::Change;
use enkidu_core::search::{FileHits, TextSummary};
use enkidu_core::symbols::{SymbolHit, SymbolResults};
use enkidu_diff::{
    Diff, DiffChangeMark, DiffDecoration, DiffFile, DiffLinePosition, DiffLineRange, DiffMark, DiffSide, DiffState,
};
use enkidu_model::changes::ChangeLists;
use enkidu_model::filetree::{self, Files, Found};
use enkidu_model::frame::Frame;
use enkidu_model::review::{Opened, Read, split_path};
use enkidu_model::search::{
    FILE_LIMIT, Group, Kept, NOTICEABLE, Note, Part, Recent, Row, SYMBOL_LIMIT, SYMBOLS_WAIT, Shown, Status, Symbols,
    TEXT_WAIT, Tab, Target, Text, Toggle, file_rows, files_note, files_status, first_selectable, kind_hue, kind_letter,
    kind_word, kind_words, step,
};
use enkidu_model::ui::{CodeColors, Ui, code_row_height};
use enkidu_ui::lamp::LAMP_FPS;
use enkidu_ui::pieces::{MenuTrigger, Observed, file_icon, kbd, mono};
use enkidu_ui::tips::Tipped as _;
use enkidu_ui::tokens::{self as theme, alpha, hsla, tone};
use futures::StreamExt as _;
use futures::channel::mpsc::unbounded;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::dialog::DialogEntrance;
use gpui_kit::component::input::{IndentInline, Input, InputEvent, InputState, MoveDown, MoveUp, OutdentInline};
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::prelude::*;
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, App, Context, Entity, Font, FontFeatures, FontWeight, HighlightStyle,
    IntoElement, Render, Rgba, Role, ScrollHandle, ScrollStrategy, SharedString, StyledText, Subscription, TextRun,
    Transformation, UniformListScrollHandle, WeakEntity, Window, div, font, percentage, px, svg, uniform_list,
};

use crate::bridge::Bridge;
use crate::changes_island::{glyph_named, read_patch, read_whole, syntax_colors, whole, whole_side};

/// The popup's key context: Find in files' three switches have their keys only here.
pub const KEYS: &str = "Search";

/// A row of the Files and Symbols tabs, and one of the Text tab (its rows are all one height, so only the ones in
/// view are made: a search can list a thousand lines).
const ROW: f32 = 36.;
const LINE_ROW: f32 = 28.;
/// How many kinds of file the filter of Find in files offers: the most common ones.
const KINDS_OFFERED: usize = 8;
/// How long a line of Find in files is the picked one before its file is read for the preview under the rows (item
/// 10.6): ↓ held through the rows reads nothing on its way.
const PREVIEW_WAIT: Duration = Duration::from_millis(80);
/// The preview: its band, and how many lines of the file it shows when the window has the room.
const PREVIEW_HEAD: f32 = 30.;
const PREVIEW_LINES: f32 = 14.;
/// The fewest lines a preview is worth showing; in a window too short for them there is none.
const PREVIEW_LEAST: f32 = 6.;
/// With a preview under them the rows take at most this: seven of them.
const ROWS_OVER_PREVIEW: f32 = 7. * LINE_ROW + 12.;

/// What Search keeps while the app runs (the tab used last, what was typed, Find in files' options), the files
/// opened lately in each workspace, and the result that was just picked.
#[derive(Default)]
pub struct SearchStore {
    pub kept: Kept,
    pub recent: Recent,
    /// A result was picked in this workspace: the window that shows it opens the file (and takes this).
    pub asked: Option<(String, Target)>,
}

/// What the popup asks.
pub struct Given {
    pub files: Vec<String>,
    pub symbols: SymbolResults,
    pub text: Vec<FileHits>,
    pub summary: TextSummary,
    /// For the preview of Find in files: the patch of each changed file, by path, and the text of other files.
    pub patches: HashMap<String, String>,
    pub texts: HashMap<String, String>,
}

/// Where the answers come from.
#[derive(Clone)]
pub enum Source {
    /// The core: git's list, the definitions index and the scan of the files, each off the drawing thread.
    Core,
    /// Given answers, for pictures and screen tests without a core.
    Given(Rc<Given>),
}

/// What the popup needs from the window that opens it.
pub struct Parts {
    pub frame: Entity<Frame>,
    pub lists: Entity<ChangeLists>,
    pub store: Entity<SearchStore>,
    /// How the window is set: the colors of the code in Find in files' lines are the ones Settings names.
    pub ui: Entity<Ui>,
    pub source: Source,
}

impl Parts {
    /// The app's own stores.
    pub fn of_the_app(cx: &App) -> Option<Parts> {
        let stores = cx.try_global::<crate::stores::Stores>()?;
        Some(Parts {
            frame: stores.frame.clone(),
            lists: stores.changes.clone(),
            store: stores.search.clone(),
            ui: stores.ui.clone(),
            source: Source::Core,
        })
    }
}

/// Opens Search over the window on `tab`, or on the tab used last (⇧⇧). Without a workspace on screen there is
/// nothing to search, and nothing opens.
pub fn open(parts: Parts, tab: Option<Tab>, window: &mut Window, cx: &mut App) {
    let on_screen = parts.frame.read(cx).current_workspace().map(|ws| (ws.id.clone(), ws.name.clone()));
    let Some((workspace, name)) = on_screen else { return };
    let view = cx.new(|cx| Search::new(parts, workspace, name, tab, window, cx));
    let shown = view.clone();
    // a little above the middle (12% down the window)
    let top = px(f32::from(window.viewport_size().height) * 0.12);
    window.open_dialog(cx, move |dialog, _, cx| {
        let (on_enter, on_close) = (shown.clone(), shown.clone());
        crate::dialog_parts::surface(dialog)
            // lines of code need more room than names
            .w(px(shown.read(cx).tab.width()))
            .margin_top(top)
            .entrance(DialogEntrance::None)
            .child(shown.clone())
            // Enter opens the row picked; the popup closes itself
            .on_ok(move |_, window, cx| {
                on_enter.update(cx, |search, cx| search.pick_active(window, cx));
                false
            })
            // Escape or a click beside it: a scan that still runs is stopped
            .on_close(move |_, _, cx| on_close.update(cx, |search, cx| search.closed(cx)))
    });
    let query = view.read(cx).query.clone();
    window.defer(cx, move |window, cx| {
        query.update(cx, |query, cx| {
            query.focus(window, cx);
            // what was typed last time is there, selected: typing replaces it
            query.select_all(window, cx);
        })
    });
}

pub struct Search {
    lists: Entity<ChangeLists>,
    store: Entity<SearchStore>,
    ui: Entity<Ui>,
    source: Source,
    /// The workspace that was on screen when the popup opened: its id and its name.
    workspace: String,
    name: String,
    tab: Tab,
    query: Entity<InputState>,
    rows: Rc<Vec<Row>>,
    /// The row ↩ opens.
    active: Option<usize>,
    /// The first row that opens is the one picked, until ↑, ↓ or the pointer picks another: answers arrive after the
    /// question, and the first of them is what ↩ should open.
    first: bool,
    scroll: ScrollHandle,
    lines: UniformListScrollHandle,

    /// Files: the workspace's files once git listed them, and the set of files they were read for.
    files: Option<Arc<Files>>,
    /// The kinds of file among them, the most common first: what the filter of Find in files offers.
    kinds: Rc<Vec<(String, usize)>>,
    files_key: Option<String>,
    reading_files: bool,
    files_error: Option<String>,
    found: Option<Found>,
    filter_turn: u64,

    symbols: Symbols,
    symbols_turn: u64,
    /// The definitions were asked to be read (once, as the popup opens on Symbols).
    prepared: bool,

    text: Text,
    text_turn: u64,
    /// The file around the picked line of Find in files, under the rows (item 10.6): one view, given another file
    /// as another row is picked.
    preview: Entity<DiffState>,
    /// What it shows now.
    previewed: Option<Previewed>,
    /// The row a read is under way for, and the version of its file.
    preview_wanted: Option<(String, usize, Option<String>)>,
    preview_turn: u64,

    /// Words that only say "reading" show once the wait is long enough to notice.
    waited: bool,
    wait_turn: u64,
    _subscriptions: Vec<Subscription>,
}

impl Search {
    fn new(
        parts: Parts,
        workspace: String,
        name: String,
        tab: Option<Tab>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let Parts { lists, store, ui, source, .. } = parts;
        let tab = store.update(cx, |store, _| {
            if let Some(tab) = tab {
                store.kept.tab = tab;
            }
            store.kept.tab
        });
        let typed = store.read(cx).kept.query.clone();
        let placeholder = tab.placeholder(&name);
        let query = cx.new(|cx| {
            let mut field = InputState::new(window, cx).placeholder(placeholder);
            // the application's own write: it says no "changed"
            field.set_value(typed, window, cx);
            field
        });
        let subscriptions = vec![
            cx.subscribe(&query, |this: &mut Self, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.typed(cx);
                }
            }),
            // Claude works on while the popup is open: the changed files, and the files there are, follow
            cx.observe(&lists, |this, _, cx| {
                if this.tab == Tab::Files {
                    this.ensure_files(cx);
                }
                this.refresh(cx);
            }),
        ];
        let colors = ui.read(cx).code_colors;
        let preview = cx.new(|cx| {
            DiffState::new([], cx).with_syntax_highlighter(move |language, text| syntax_colors(colors, language, text))
        });
        let mut search = Search {
            lists,
            store,
            ui,
            source,
            workspace,
            name,
            tab,
            query,
            rows: Rc::new(Vec::new()),
            active: None,
            first: true,
            scroll: ScrollHandle::new(),
            lines: UniformListScrollHandle::new(),
            files: None,
            kinds: Rc::default(),
            files_key: None,
            reading_files: false,
            files_error: None,
            found: None,
            filter_turn: 0,
            symbols: Symbols::default(),
            symbols_turn: 0,
            prepared: false,
            text: Text::default(),
            text_turn: 0,
            preview,
            previewed: None,
            preview_wanted: None,
            preview_turn: 0,
            waited: false,
            wait_turn: 0,
            _subscriptions: subscriptions,
        };
        search.ask(cx);
        search.refresh(cx);
        search
    }

    /// The tab on screen.
    pub fn tab(&self) -> Tab {
        self.tab
    }

    fn typed_text(&self, cx: &App) -> String {
        self.query.read(cx).value().to_string()
    }

    fn changes(&self, cx: &App) -> Arc<Vec<Change>> {
        self.lists.read(cx).of(&self.workspace).cloned().unwrap_or_default()
    }

    /// Something was typed: every tab shares it, and it is there again the next time the popup opens.
    fn typed(&mut self, cx: &mut Context<Self>) {
        let typed = self.typed_text(cx);
        self.store.update(cx, |store, _| store.kept.query = typed);
        self.ask(cx);
        // a new question starts at its first answer
        self.first = true;
        self.active = first_selectable(&self.rows);
        self.back_to_top();
        cx.notify();
    }

    /// Another tab, by its button, its key, or ⇥.
    fn set_tab(&mut self, tab: Tab, window: &mut Window, cx: &mut Context<Self>) {
        let placeholder = self.tab_placeholder(tab);
        self.query.update(cx, |query, cx| {
            query.set_placeholder(placeholder, window, cx);
            query.focus(window, cx);
            query.select_all(window, cx);
        });
        if tab == self.tab {
            return;
        }
        if self.tab == Tab::Text {
            // the scan of the tab that is left stops
            self.text_turn += 1;
            self.stop_text(cx);
        }
        self.tab = tab;
        self.store.update(cx, |store, _| store.kept.tab = tab);
        self.first = true;
        self.ask(cx);
        self.refresh(cx);
        self.back_to_top();
        cx.notify();
    }

    fn tab_placeholder(&self, tab: Tab) -> SharedString {
        SharedString::from(tab.placeholder(&self.name))
    }

    fn back_to_top(&mut self) {
        self.scroll.scroll_to_item(0);
        self.lines.scroll_to_item(0, ScrollStrategy::Top);
    }

    /// Asks the tab on screen for what is typed now.
    fn ask(&mut self, cx: &mut Context<Self>) {
        match self.tab {
            Tab::Files => {
                self.ensure_files(cx);
                self.filter(cx);
            }
            Tab::Symbols => self.ask_symbols(cx),
            Tab::Text => {
                // the kinds of file its filter offers come from the list of files
                self.ensure_files(cx);
                self.ask_text(cx);
            }
        }
    }

    /// The rows of the tab on screen, from what is known now.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let typed = self.typed_text(cx);
        let rows = match self.tab {
            Tab::Files => {
                let changes = self.changes(cx);
                let found = self.found.as_ref().filter(|_| !typed.trim().is_empty());
                file_rows(found, &changes, self.store.read(cx).recent.of(&self.workspace), self.files.as_deref())
            }
            Tab::Symbols => self.symbols.rows(&typed),
            Tab::Text => self.text.rows(&typed),
        };
        self.rows = Rc::new(rows);
        // until another row was picked, or when the one picked is gone: the first one that opens
        let gone = self.active.is_none_or(|ix| !self.rows.get(ix).is_some_and(Row::selectable));
        if self.first || gone {
            self.active = first_selectable(&self.rows);
        }
        self.follow_preview(cx);
        cx.notify();
    }

    /// From now, words that only say "reading" wait until the wait can be noticed.
    fn begin_wait(&mut self, cx: &mut Context<Self>) {
        self.waited = false;
        self.wait_turn += 1;
        let turn = self.wait_turn;
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            cx.background_executor().timer(NOTICEABLE).await;
            let _ = this.update(cx, |this, cx| {
                if this.wait_turn == turn {
                    this.waited = true;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    // Files

    /// Asks git for the workspace's files, unless they were read for the set of files as it is now (an edit doesn't
    /// change the set; a new or a deleted file does, and the changes say so first).
    fn ensure_files(&mut self, cx: &mut Context<Self>) {
        let changes = self.changes(cx);
        let key = filetree::file_set_key(&changes);
        if self.reading_files || self.files_key.as_ref() == Some(&key) {
            return;
        }
        self.reading_files = true;
        if self.files.is_none() {
            self.begin_wait(cx);
        }
        match &self.source {
            Source::Core => {
                let (workspace, this) = (self.workspace.clone(), cx.weak_entity());
                Bridge::call(
                    cx,
                    move |shared| {
                        let listed =
                            enkidu_core::commands::list_files(shared, workspace, None).map_err(|err| err.message)?;
                        Ok::<Files, String>(Files::new(filetree::visible_files(&listed.files, &changes)))
                    },
                    move |read, cx| {
                        let _ = this.update(cx, |this, cx| this.files_read(key, read, cx));
                    },
                );
            }
            Source::Given(given) => {
                let files = Files::new(filetree::visible_files(&given.files, &changes));
                self.files_read(key, Ok(files), cx);
            }
        }
    }

    fn files_read(&mut self, key: String, read: Result<Files, String>, cx: &mut Context<Self>) {
        self.reading_files = false;
        self.files_key = Some(key);
        match read {
            Ok(files) => {
                self.kinds = Rc::new(files.kinds(KINDS_OFFERED));
                self.files = Some(Arc::new(files));
                self.files_error = None;
            }
            // what was listed before stays
            Err(why) => self.files_error = Some(why),
        }
        self.filter(cx);
        self.refresh(cx);
    }

    /// The names are matched again, off the drawing thread; an answer for words that were typed over since is
    /// dropped.
    fn filter(&mut self, cx: &mut Context<Self>) {
        self.filter_turn += 1;
        let turn = self.filter_turn;
        let typed = self.typed_text(cx);
        let (false, Some(files)) = (typed.trim().is_empty(), self.files.clone()) else {
            self.found = None;
            self.refresh(cx);
            return;
        };
        let matching = cx.background_spawn(async move { files.filter(&typed, FILE_LIMIT) });
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let found = matching.await;
            let _ = this.update(cx, |this, cx| {
                if this.filter_turn == turn {
                    this.found = Some(found);
                    this.refresh(cx);
                }
            });
        })
        .detach();
    }

    // Symbols

    /// Asks the core's index of definitions a moment after the last key. The index is built in the background the
    /// first time the popup opens on this tab, so the first question is quick.
    fn ask_symbols(&mut self, cx: &mut Context<Self>) {
        self.symbols_turn += 1;
        let turn = self.symbols_turn;
        let typed = self.typed_text(cx).trim().to_string();
        if !std::mem::replace(&mut self.prepared, true) && matches!(self.source, Source::Core) {
            let workspace = self.workspace.clone();
            Bridge::call(cx, move |shared| enkidu_core::commands::prepare_symbols(shared, workspace), |_, _| {});
        }
        if typed.is_empty() {
            self.symbols.cleared();
            self.refresh(cx);
            return;
        }
        if let Source::Given(given) = &self.source {
            self.symbols.results = Some(given.symbols.clone());
            self.refresh(cx);
            return;
        }
        let workspace = self.workspace.clone();
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            cx.background_executor().timer(SYMBOLS_WAIT).await;
            let asked = this.update(cx, |this, cx| {
                if this.symbols_turn != turn {
                    return None;
                }
                this.symbols.busy = true;
                if this.symbols.results.is_none() {
                    this.begin_wait(cx);
                }
                cx.notify();
                Some(Bridge::ask(cx, move |shared| {
                    enkidu_core::commands::search_symbols(shared, workspace, typed, Some(SYMBOL_LIMIT))
                }))
            });
            let Ok(Some(answer)) = asked else { return };
            let answer = answer.await;
            let _ = this.update(cx, |this, cx| {
                if this.symbols_turn != turn {
                    return;
                }
                this.symbols.busy = false;
                match answer {
                    Ok(Ok(results)) => {
                        this.symbols.results = Some(results);
                        this.symbols.error = None;
                    }
                    Ok(Err(err)) => this.symbols.error = Some(err.message),
                    Err(_) => {}
                }
                this.refresh(cx);
            });
        })
        .detach();
    }

    // Text

    /// Starts reading the files a moment after the last key. The files with matching lines arrive as they are
    /// read, in path order; a newer scan of the same workspace stops this one in the core.
    fn ask_text(&mut self, cx: &mut Context<Self>) {
        self.text_turn += 1;
        let turn = self.text_turn;
        let typed = self.typed_text(cx);
        let options = self.store.read(cx).kept.options.clone();
        let Some(query) = options.query(&typed) else {
            self.text.cleared();
            self.stop_text(cx);
            self.refresh(cx);
            return;
        };
        if let Source::Given(given) = &self.source {
            // the given lines, of the kind of file that is picked
            let of_kind = |file: &&FileHits| {
                options
                    .kind
                    .as_deref()
                    .is_none_or(|kind| enkidu_core::search::kind_of(&file.path).as_deref() == Some(kind))
            };
            let found: Vec<FileHits> = given.text.iter().filter(of_kind).cloned().collect();
            let lines = found.iter().map(|file| file.lines.len()).sum();
            let summary =
                TextSummary { lines, files: found.len(), matched_files: found.len(), ..given.summary.clone() };
            self.text.started(&typed, &options);
            self.text.arrived(found);
            self.text.finished(Ok(summary));
            self.refresh(cx);
            return;
        }
        let workspace = self.workspace.clone();
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            cx.background_executor().timer(TEXT_WAIT).await;
            let (arrived, mut arriving) = unbounded::<Vec<FileHits>>();
            let asked = this.update(cx, |this, cx| {
                if this.text_turn != turn {
                    return None;
                }
                this.text.started(&typed, &options);
                cx.notify();
                Some(Bridge::ask(cx, move |shared| {
                    enkidu_core::commands::search_text(shared, workspace, query, &move |batch| {
                        let _ = arrived.unbounded_send(batch);
                    })
                }))
            });
            let Ok(Some(answer)) = asked else { return };
            while let Some(batch) = arriving.next().await {
                let _ = this.update(cx, |this, cx| {
                    if this.text_turn == turn {
                        this.text.arrived(batch);
                        this.refresh(cx);
                    }
                });
            }
            let answer = answer.await;
            let _ = this.update(cx, |this, cx| {
                if this.text_turn != turn {
                    return;
                }
                this.text.finished(match answer {
                    Ok(Ok(summary)) => Ok(summary),
                    Ok(Err(err)) => Err(err.message),
                    Err(_) => Err("The search stopped before it was done.".into()),
                });
                this.refresh(cx);
            });
        })
        .detach();
    }

    /// Stops the workspace's scan in the core, if one runs.
    fn stop_text(&mut self, cx: &mut Context<Self>) {
        if matches!(self.source, Source::Core) {
            let workspace = self.workspace.clone();
            Bridge::call(cx, move |shared| enkidu_core::commands::cancel_text_search(shared, workspace), |_, _| {});
        }
    }

    /// One of Find in files' switches, by its button or its key.
    fn toggle(&mut self, which: Toggle, cx: &mut Context<Self>) {
        self.store.update(cx, |store, _| store.kept.options.toggle(which));
        if self.tab == Tab::Text {
            self.ask_text(cx);
        }
        cx.notify();
    }

    // The rows

    fn move_by(&mut self, by: i32, cx: &mut Context<Self>) {
        self.first = false;
        self.active = step(&self.rows, self.active, by);
        if let Some(ix) = self.active {
            // at the first row that opens, the heading above it comes back too
            let to = if Some(ix) == first_selectable(&self.rows) { 0 } else { ix };
            self.scroll.scroll_to_item(to);
            self.lines.scroll_to_item(to, ScrollStrategy::Nearest);
        }
        self.follow_preview(cx);
        cx.notify();
    }

    fn hover(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.active != Some(ix) && self.rows.get(ix).is_some_and(Row::selectable) {
            self.first = false;
            self.active = Some(ix);
            self.follow_preview(cx);
            cx.notify();
        }
    }

    /// Opens the row picked: the popup closes, and the window that shows the workspace opens the file at its line.
    fn pick(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.rows.get(ix).and_then(Row::target) else { return };
        window.close_dialog(cx);
        self.closed(cx);
        let workspace = self.workspace.clone();
        self.store.update(cx, |store, cx| {
            store.asked = Some((workspace, target));
            cx.notify();
        });
    }

    pub fn pick_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.active {
            self.pick(ix, window, cx);
        }
    }

    /// The popup closed: what was asked and not answered yet is dropped, and a scan that runs is stopped.
    pub fn closed(&mut self, cx: &mut Context<Self>) {
        self.symbols_turn += 1;
        self.text_turn += 1;
        self.filter_turn += 1;
        self.preview_turn += 1;
        self.stop_text(cx);
    }

    // The preview

    /// The line of Find in files that is picked: its file, its number, and the line as the row shows it.
    fn picked_line(&self) -> Option<(String, usize, Shown)> {
        if self.tab != Tab::Text {
            return None;
        }
        match self.active.and_then(|ix| self.rows.get(ix)) {
            Some(Row::Line { path, hit }) => {
                Some((path.clone(), hit.line as usize, Shown { text: hit.text.clone(), ranges: hit.ranges.clone() }))
            }
            _ => None,
        }
    }

    /// The preview follows the picked line (item 10.6): the file around it, under the rows. Another line of the
    /// file that is shown is there at once. Another file is read after the row was picked for a moment, off the
    /// drawing thread, so ↓ held through a thousand rows reads only where it stops; an answer for a row that is no
    /// longer the picked one is dropped.
    fn follow_preview(&mut self, cx: &mut Context<Self>) {
        let Some((path, line, shown)) = self.picked_line() else {
            // nothing picked: nothing is waited for, and what was shown goes with the rows
            if self.previewed.take().is_some() || self.preview_wanted.take().is_some() {
                self.preview_turn += 1;
                cx.notify();
            }
            return;
        };
        // what git says of the file now: a changed one carries its marks, and one that Claude wrote again since it
        // was read for the preview is read again
        let change = self.changes(cx).iter().find(|change| change.path == path).cloned();
        let sig = change.as_ref().map(|change| change.sig.clone());
        if let Some(previewed) =
            self.previewed.as_mut().filter(|previewed| previewed.path == path && previewed.sig == sig)
        {
            // the same file as it was read: only the line moves
            self.preview_wanted = None;
            self.preview_turn += 1;
            if previewed.line != line || previewed.shown != shown {
                (previewed.line, previewed.shown) = (line, shown);
                let at = DiffLinePosition::new(path, DiffSide::Modified, line);
                self.preview.update(cx, |state, cx| state.scroll_to_line_centered(at, cx));
                cx.notify();
            }
            return;
        }
        if self
            .preview_wanted
            .as_ref()
            .is_some_and(|(wanted, at, version)| *wanted == path && *at == line && *version == sig)
        {
            return;
        }
        self.preview_wanted = Some((path.clone(), line, sig.clone()));
        self.preview_turn += 1;
        let turn = self.preview_turn;
        let workspace = self.workspace.clone();
        let source = self.source.clone();
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            cx.background_executor().timer(PREVIEW_WAIT).await;
            let still = this.read_with(cx, |this, _| this.preview_turn == turn).unwrap_or(false);
            if !still {
                return;
            }
            let read = match source {
                Source::Core => {
                    let (file, change) = (path.clone(), change.clone());
                    let answer = cx
                        .update(|cx| Bridge::ask(cx, move |shared| read_for_preview(shared, workspace, &file, change)));
                    let Ok(read) = answer.await else { return };
                    read
                }
                Source::Given(given) => given_preview(&given, &path),
            };
            let _ = this.update(cx, |this, cx| {
                if this.preview_turn != turn {
                    return;
                }
                this.preview_wanted = None;
                let (read, marks) = read;
                let file = match &read {
                    Opened::Text(file, _) => Some(file.clone()),
                    _ => None,
                };
                let at = DiffLinePosition::new(path.clone(), DiffSide::Modified, line);
                this.preview.update(cx, |state, cx| {
                    state.set_files(file, cx);
                    state.scroll_to_line_centered(at, cx);
                });
                let kind = change.map(|change| change.kind);
                this.previewed = Some(Previewed { path, line, shown, kind, sig, read, marks });
                cx.notify();
            });
        })
        .detach();
    }

    /// The file around the picked line, under the rows: its name and folder (and what happened to it), then its
    /// lines `height` tall, the line that was found marked, with the marks of what differs from the last commit.
    fn preview_view(&self, previewed: &Previewed, height: f32, cx: &mut Context<Self>) -> AnyElement {
        let (dir, name) = split_path(&previewed.path);
        let (colors, code_size) = {
            let ui = self.ui.read(cx);
            (ui.code_colors, ui.code_size)
        };
        let path = previewed.path.clone();
        let body = match previewed.read.note() {
            Some(words) => crate::opened_file::note(words),
            None => {
                let at = DiffLinePosition::new(path.clone(), DiffSide::Modified, previewed.line);
                let range = DiffLineRange::new(path.clone(), DiffSide::Modified, previewed.line, previewed.line);
                let target = DiffDecoration::new(range).with_background(hsla(alpha(theme::ACCENT, 0.12)));
                // what was found is marked in the line, as the row marks it (when the line still has that text)
                let found: Vec<DiffMark> = (|| {
                    let Opened::Text(file, _) = &previewed.read else { return None };
                    let text = *whole_side(file, DiffSide::Modified)?.get(previewed.line.checked_sub(1)?)?;
                    let marks = enkidu_model::search::ranges_in_line(text, &previewed.shown)?;
                    let color = hsla(alpha(theme::FOCUS, 0.3));
                    Some(marks.into_iter().map(|mark| DiffMark::new(at.clone(), mark, color)).collect())
                })()
                .unwrap_or_default();
                let differs =
                    (!previewed.marks.is_empty()).then(|| (SharedString::from(path.clone()), previewed.marks.clone()));
                Diff::new(&self.preview)
                    .colors(crate::changes_island::colors(colors))
                    .font_size(px(code_size as f32))
                    .row_height(px(code_row_height(code_size) as f32))
                    .soft_wrap(true)
                    .header_visible(false)
                    .decorations([target])
                    .marks(found)
                    .change_marks(differs)
                    .into_any_element()
            }
        };
        div()
            .id("search-preview")
            .test_support()
            .aria_label(SharedString::from(path.clone()))
            .aria_description(SharedString::from(format!("Line {}", previewed.line)))
            .flex_none()
            .flex()
            .flex_col()
            .border_t_1()
            .border_color(theme::RULE_STRONG)
            .child(
                div()
                    .h(px(PREVIEW_HEAD))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(12.))
                    .bg(Rgba { r: 0., g: 0., b: 0., a: 0.18 })
                    .border_b_1()
                    .border_color(theme::RULE)
                    .child(file_icon(&path, 13.))
                    .child(
                        div()
                            .flex_none()
                            .max_w(px(420.))
                            .truncate()
                            .text_size(px(12.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::INK)
                            .child(name.to_string()),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(theme::TEXT_STATUS)
                            .text_color(theme::INK_3)
                            .child(dir.to_string()),
                    )
                    .children(
                        previewed
                            .kind
                            .as_deref()
                            .map(|kind| glyph_named("search-preview-kind", kind).into_any_element()),
                    ),
            )
            .child(div().h(px(height)).bg(theme::CANVAS).child(body))
            .into_any_element()
    }

    /// What the tab on screen says of its search, and what it says in place of results.
    fn words(&self, cx: &App) -> (Status, Option<Note>) {
        let typed = self.typed_text(cx);
        let (status, note) = match self.tab {
            Tab::Files => {
                let found = self.found.as_ref().filter(|_| !typed.trim().is_empty());
                let note = match &self.files_error {
                    Some(why) if self.files.is_none() => Some(Note {
                        parts: vec![Part { text: format!("Couldn't list the files: {why}"), code: false }],
                        problem: true,
                        loading: false,
                    }),
                    _ => files_note(self.files.as_deref(), &self.rows, &typed),
                };
                (files_status(found, self.files.as_deref()), note)
            }
            Tab::Symbols => (self.symbols.status(&typed), self.symbols.note(&typed)),
            Tab::Text => (self.text.status(&typed), self.text.note(&typed)),
        };
        // a quick read goes from nothing straight to the result
        (status, note.filter(|note| !note.loading || self.waited))
    }

    fn tabs(&self, status: &Status, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs = Tab::ALL.into_iter().enumerate().map(|(ix, tab)| {
            let on = tab == self.tab;
            div()
                .id(("search-tab", ix))
                .test_support()
                .role(Role::Tab)
                .aria_label(tab.label())
                .aria_selected(on)
                .h(px(28.))
                .px(px(10.))
                .flex()
                .flex_none()
                .items_center()
                .gap(px(8.))
                .rounded(px(6.))
                .text_size(theme::TEXT_SMALL)
                .font_weight(FontWeight::MEDIUM)
                .cursor_pointer()
                .map(|tab| match on {
                    true => tab.bg(theme::SELECTED).text_color(theme::INK),
                    false => tab.text_color(theme::INK_3).hover(|s| s.bg(theme::HOVER).text_color(theme::INK_2)),
                })
                .child(tab.label())
                .child(
                    div()
                        .text_size(theme::CAPTION)
                        .font_weight(FontWeight::NORMAL)
                        .text_color(theme::INK_3)
                        .child(tab.shortcut()),
                )
                .on_click(cx.listener(move |this, _, window, cx| this.set_tab(tab, window, cx)))
        });
        let said = (!status.words.is_empty()).then(|| {
            div()
                .id("search-status")
                .test_support()
                .aria_label(SharedString::from(status.words.clone()))
                .aria_description(if status.busy { "at work" } else { "done" })
                .flex()
                .flex_none()
                .items_center()
                .gap(px(6.))
                .pr(px(8.))
                .text_size(theme::TEXT_STATUS)
                .text_color(theme::INK_3)
                .when(status.busy, |said| {
                    said.child(
                        svg()
                            .flex_none()
                            .size(px(11.))
                            .path("icons/loader-circle.svg")
                            .text_color(theme::INK_3)
                            .with_animation(
                                "search-busy",
                                Animation::new(std::time::Duration::from_secs(1))
                                    .repeat_synced()
                                    .with_max_fps(LAMP_FPS),
                                |sign, t| sign.with_transformation(Transformation::rotate(percentage(t))),
                            ),
                    )
                })
                .child(status.words.clone())
        });
        div()
            .h(px(40.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(4.))
            .px(px(8.))
            .border_b_1()
            .border_color(theme::RULE)
            .children(tabs)
            .child(div().flex_1())
            .children(said)
    }

    /// The filter of Find in files (item 10.5): a button that says the
    /// kind of file the scan keeps to, and a menu of the kinds this workspace has, the most common first, each with
    /// how many files have it.
    fn kind_filter(&self, cx: &mut Context<Self>) -> AnyElement {
        let picked = self.store.read(cx).kept.options.kind.clone();
        let words = kind_words(picked.as_deref());
        let (kinds, all) = (self.kinds.clone(), self.files.as_ref().map(|files| files.len()));
        let search = cx.weak_entity();
        let set = picked.is_some();
        MenuTrigger::new("search-kind", SharedString::from(words.clone()))
            .tip("The kind of file to search in")
            .h(px(26.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(6.))
            .pl(px(10.))
            .pr(px(7.))
            .mr(px(4.))
            .rounded(px(7.))
            .cursor_pointer()
            .border_1()
            .map(|button| match set {
                // a kind is picked: the scan is not of every file, and the button says so in signal blue
                true => button.border_color(alpha(theme::FOCUS, 0.5)).bg(alpha(theme::FOCUS, 0.10)),
                false => button.border_color(theme::RULE_STRONG).hover(|s| s.bg(theme::HOVER)),
            })
            .child(match set {
                true => mono(div()).text_size(px(12.)).text_color(theme::INK).child(words),
                false => div().text_size(px(12.)).text_color(theme::INK_3).child(words),
            })
            .child(svg().path("icons/chevron-down.svg").flex_none().size(px(12.)).text_color(theme::INK_3))
            .menu(gpui_kit::Anchor::TopRight, move |menu, _, _| {
                let pick = |kind: Option<String>| {
                    let search = search.clone();
                    move |_: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut App| {
                        let _ = search.update(cx, |search, cx| search.set_kind(kind.clone(), window, cx));
                    }
                };
                let every = kind_option("search-kind-option", None, all, picked.is_none());
                let mut menu = menu.min_w(px(210.)).item(every.on_click(pick(None)));
                if !kinds.is_empty() {
                    menu = menu.separator();
                }
                for (kind, count) in kinds.iter() {
                    let picked = picked.as_deref() == Some(kind.as_str());
                    let option = kind_option("search-kind-option", Some(kind), Some(*count), picked);
                    menu = menu.item(option.on_click(pick(Some(kind.clone()))));
                }
                menu
            })
            .into_any_element()
    }

    /// Another kind of file, or every file again: the scan runs anew, and the keyboard is back in the field.
    fn set_kind(&mut self, kind: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.store.update(cx, |store, _| store.kept.options.kind = kind);
        self.query.update(cx, |query, cx| query.focus(window, cx));
        self.first = true;
        if self.tab == Tab::Text {
            self.ask_text(cx);
        }
        self.back_to_top();
        cx.notify();
    }

    fn field(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let options = self.store.read(cx).kept.options.clone();
        let kind = (self.tab == Tab::Text).then(|| self.kind_filter(cx));
        let toggles = (self.tab == Tab::Text).then(|| {
            let switch = |which: Toggle, id: &'static str, icon: &'static str, cx: &mut Context<Self>| {
                let on = options.on(which);
                div()
                    .id(id)
                    .test_support()
                    .role(Role::Button)
                    .aria_label(which.label())
                    .aria_selected(on)
                    .tipped_in(
                        which.label(),
                        match which {
                            Toggle::Case => Box::new(crate::ToggleMatchCase) as Box<dyn gpui_kit::Action>,
                            Toggle::Word => Box::new(crate::ToggleWholeWords),
                            Toggle::Regex => Box::new(crate::ToggleRegex),
                        },
                        KEYS,
                    )
                    .size(px(28.))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .map(|switch| match on {
                        true => switch.bg(alpha(theme::FOCUS, 0.2)).border_1().border_color(alpha(theme::FOCUS, 0.45)),
                        false => switch.hover(|s| s.bg(theme::HOVER)),
                    })
                    .child(svg().path(icon).size(px(15.)).text_color(if on { theme::INK } else { theme::INK_3 }))
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle(which, cx)))
            };
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(2.))
                .child(switch(Toggle::Case, "toggle-case", "icons/case-sensitive.svg", cx))
                .child(switch(Toggle::Word, "toggle-word", "icons/whole-word.svg", cx))
                .child(switch(Toggle::Regex, "toggle-regex", "icons/regex.svg", cx))
        });
        div()
            .h(px(48.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(16.))
            .border_b_1()
            .border_color(theme::RULE)
            .child(svg().path("icons/search.svg").flex_none().size(px(16.)).text_color(theme::INK_3))
            .child(div().flex_1().min_w_0().child(Input::new(&self.query).id("search-input").appearance(false)))
            .children(kind)
            .children(toggles)
    }

    /// A row of the Files or Symbols tab.
    fn row(&self, ix: usize, row: &Row, typed: &str, changes: &[Change], cx: &mut Context<Self>) -> AnyElement {
        let active = self.active == Some(ix);
        match row {
            Row::Label(group) => div()
                .id(("search-label", ix))
                .test_support()
                .aria_label(group.label())
                .flex()
                .flex_none()
                .items_center()
                .gap(px(6.))
                .px(px(10.))
                .pt(px(8.))
                .pb(px(4.))
                .text_size(theme::CAPTION)
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::INK_3)
                .child(
                    svg()
                        .path(if *group == Group::Changed { "icons/file-diff.svg" } else { "icons/clock.svg" })
                        .flex_none()
                        .size(px(11.))
                        .text_color(theme::INK_3),
                )
                .child(group.label())
                .into_any_element(),
            Row::File { path, from } => {
                let (dir, name) = split_path(path);
                let mark = filetree::name_match(name, typed).map(|(from, to)| from..to);
                let kind = changes.iter().find(|change| change.path == *path).map(|change| change.kind.clone());
                self.frame(ix, path, active, cx)
                    .when_some(from.map(Group::label), |row, group| row.aria_description(group))
                    .h(px(ROW))
                    .text_size(theme::TEXT)
                    .child(file_icon(path, 15.))
                    .child(
                        div()
                            .flex_none()
                            .max_w(px(360.))
                            .truncate()
                            .text_color(theme::INK)
                            .child(marked(name, mark.into_iter().collect())),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(theme::TEXT_SMALL)
                            .text_color(theme::INK_3)
                            .child(dir.to_string()),
                    )
                    .children(kind.map(|kind| glyph_named("search-kind", &kind).into_any_element()))
                    .into_any_element()
            }
            Row::Symbol(hit) => {
                let (_, file) = split_path(&hit.path);
                let color = tone(kind_hue(hit.kind), 0.13);
                let place = format!("{file}:{}", hit.line);
                self.frame(ix, &hit.name, active, cx)
                    .aria_description(SharedString::from(symbol_words(hit)))
                    .h(px(ROW))
                    .text_size(theme::TEXT)
                    .child(
                        mono(div())
                            .size(px(18.))
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .rounded(px(5.))
                            .bg(alpha(color, 0.15))
                            .text_color(color)
                            .text_size(px(10.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(kind_letter(hit.kind)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_baseline()
                            .gap(px(6.))
                            .child(mono(div()).min_w_0().truncate().text_size(px(12.5)).text_color(theme::INK).child(
                                marked(
                                    &hit.name,
                                    char_marks(
                                        &hit.name,
                                        hit.indices.iter().map(|at| (*at as usize, *at as usize + 1)),
                                    ),
                                ),
                            ))
                            .children(hit.container.as_ref().map(|container| {
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(theme::TEXT_SMALL)
                                    .text_color(theme::INK_3)
                                    .child(format!("in {container}"))
                            })),
                    )
                    .child(
                        div()
                            .flex_shrink(1.)
                            .min_w_0()
                            .truncate()
                            .text_size(theme::TEXT_STATUS)
                            .text_color(theme::INK_3)
                            .child(place),
                    )
                    .into_any_element()
            }
            // the Text tab's rows are drawn by `line_row`
            Row::Line { .. } => div().into_any_element(),
        }
    }

    /// What every row that opens has in common.
    fn frame(&self, ix: usize, label: &str, active: bool, cx: &mut Context<Self>) -> Observed {
        opens(ix, label, active, cx.weak_entity())
    }

    fn footer(&self) -> impl IntoElement {
        let hint = |keys: &[&'static str], words: &'static str| {
            div().flex().flex_none().items_center().gap(px(6.)).children(keys.iter().map(|key| kbd(*key))).child(words)
        };
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(16.))
            .px(px(16.))
            .py(px(8.))
            .border_t_1()
            .border_color(theme::RULE)
            .bg(Rgba { r: 0., g: 0., b: 0., a: 0.25 })
            // its fill follows the dialog's corner (nothing clips to a rounded corner here)
            .rounded_b(crate::dialog_parts::INNER_RADIUS)
            .text_size(theme::CAPTION)
            .text_color(theme::INK_3)
            .child(hint(&["↑", "↓"], "move"))
            .child(hint(&["↩"], "open"))
            .child(hint(&["⇥"], "next tab"))
            .child(div().flex_1())
            .child(hint(&["⇧⇧"], "opens this"))
    }
}

/// The frame of a row that opens: the pointer picks it as it moves over it, a click opens it.
fn opens(ix: usize, label: &str, active: bool, search: WeakEntity<Search>) -> Observed {
    let (over, clicked) = (search.clone(), search);
    div()
        .id(("search-row", ix))
        .test_support()
        .role(Role::ListBoxOption)
        .aria_label(SharedString::from(label.to_string()))
        .aria_selected(active)
        .w_full()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(10.))
        .px(px(10.))
        .rounded(px(7.))
        .cursor_pointer()
        .when(active, |row| row.bg(theme::SELECTED))
        .on_mouse_move(move |_, _, cx| {
            let _ = over.update(cx, |search, cx| search.hover(ix, cx));
        })
        .on_click(move |_, window, cx| {
            let _ = clicked.update(cx, |search, cx| search.pick(ix, window, cx));
        })
}

/// A row of the Text tab (item 10.4, as PyCharm's Find in Files lists a match): the line, colored like code with
/// what was found marked, then its file's name and its number at the right.
fn line_row(ix: usize, row: &Row, active: bool, colors: CodeColors, search: WeakEntity<Search>) -> AnyElement {
    let Row::Line { path, hit } = row else {
        // the other tabs' rows are drawn by `Search::row`
        return div().into_any_element();
    };
    let marks = char_marks(&hit.text, hit.ranges.iter().map(|[from, to]| (*from as usize, *to as usize)));
    let found: Vec<&str> = marks.iter().map(|mark| &hit.text[mark.clone()]).collect();
    let place = format!("{path}:{}, matched: {}", hit.line, found.join(" | "));
    let (_, name) = split_path(path);
    let at = format!("{name} {}", hit.line);
    opens(ix, &hit.text, active, search)
        .aria_description(SharedString::from(place))
        .h(px(LINE_ROW))
        .gap(px(14.))
        .pl(px(12.))
        .text_size(theme::TEXT_SMALL)
        .child(
            mono(div())
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_color(theme::INK_2)
                .child(code_line(path, &hit.text, &marks, colors)),
        )
        .child(
            div()
                .id(("search-place", ix))
                .test_support()
                .aria_label(SharedString::from(at.clone()))
                .flex_none()
                .max_w(px(320.))
                .truncate()
                .text_size(theme::TEXT_STATUS)
                .text_color(if active { theme::INK_2 } else { theme::INK_3 })
                .child(at),
        )
        .into_any_element()
}

/// A line of a file as Find in files shows it: in the colors its language has in the scheme Settings names (the
/// line is colored alone, so a line inside a docstring reads as code), with what was found marked on top.
fn code_line(path: &str, text: &str, marks: &[Range<usize>], colors: CodeColors) -> StyledText {
    let scheme = match colors {
        CodeColors::Ide => enkidu_syntax::Scheme::Ide,
        CodeColors::Quiet => enkidu_syntax::Scheme::Quiet,
    };
    let spans = enkidu_syntax::colors(scheme, &enkidu_diff::language_of(path), text);
    let runs = colored_runs(text.len(), &spans, marks).into_iter().map(|(range, run)| {
        let style = match run {
            // what was found: full ink on signal blue
            Run::Marked => HighlightStyle {
                color: Some(theme::INK.into()),
                background_color: Some(alpha(theme::FOCUS, 0.25).into()),
                ..Default::default()
            },
            Run::Colored((r, g, b, a)) => {
                let color = Rgba {
                    r: f32::from(r) / 255.,
                    g: f32::from(g) / 255.,
                    b: f32::from(b) / 255.,
                    a: f32::from(a) / 255.,
                };
                HighlightStyle { color: Some(color.into()), ..Default::default() }
            }
        };
        (range, style)
    });
    StyledText::new(text.to_string()).with_highlights(runs)
}

/// How a stretch of a line is drawn: as what was found, or in a color of the code's.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Run {
    Marked,
    Colored(enkidu_syntax::Rgba),
}

/// The runs a line of `len` bytes is drawn in: its colors, cut where a mark starts and ends, with each mark over
/// them. In order, none over another (the text element takes them so); a stretch without a color has no run.
fn colored_runs(
    len: usize,
    spans: &[(Range<usize>, enkidu_syntax::Rgba)],
    marks: &[Range<usize>],
) -> Vec<(Range<usize>, Run)> {
    let ends = |range: &Range<usize>| [range.start, range.end];
    let mut cuts: Vec<usize> = spans
        .iter()
        .flat_map(|(span, _)| ends(span))
        .chain(marks.iter().flat_map(ends))
        .filter(|at| *at < len)
        .collect();
    cuts.extend([0, len]);
    cuts.sort_unstable();
    cuts.dedup();
    let (mut span_ix, mut mark_ix) = (0, 0);
    let mut runs: Vec<(Range<usize>, Run)> = Vec::new();
    for pair in cuts.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        while marks.get(mark_ix).is_some_and(|mark| mark.end <= from) {
            mark_ix += 1;
        }
        while spans.get(span_ix).is_some_and(|(span, _)| span.end <= from) {
            span_ix += 1;
        }
        let run = if marks.get(mark_ix).is_some_and(|mark| mark.start <= from) {
            Run::Marked
        } else if let Some((_, color)) = spans.get(span_ix).filter(|(span, _)| span.start <= from) {
            Run::Colored(*color)
        } else {
            continue;
        };
        match runs.last_mut() {
            Some((range, last)) if range.end == from && *last == run => range.end = to,
            _ => runs.push((from..to, run)),
        }
    }
    runs
}

/// What the preview shows (item 10.6): the row it was read for, what reading its file gave, and what differs from
/// the last commit in it.
struct Previewed {
    path: String,
    line: usize,
    /// The line as the row shows it, to mark what was found in the real line.
    shown: Shown,
    /// What happened to the file ("modified", "untracked"…), when it is one of the changed files.
    kind: Option<String>,
    /// Its change's signature when it was read: another one since means Claude wrote the file again.
    sig: Option<String>,
    read: Opened<DiffFile>,
    marks: Vec<DiffChangeMark>,
}

/// A file for the preview, with the marks of what differs from the last commit when it is a changed file (its diff
/// is read right after its text). Runs off the drawing thread.
fn read_for_preview(
    shared: &Arc<enkidu_core::state::Shared>,
    workspace: String,
    path: &str,
    change: Option<Change>,
) -> (Opened<DiffFile>, Vec<DiffChangeMark>) {
    let text = read_whole(shared, workspace.clone(), path);
    let marks = match change.filter(|change| enkidu_model::review::marks_its_changes(&change.kind)) {
        Some(change) if matches!(text, Opened::Text(..)) => match read_patch(shared, workspace, &change) {
            Read::Lines(file) => file.change_marks(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    };
    (text, marks)
}

/// The same from given answers, for pictures and screen tests without a core: a changed file is what its given
/// patch leaves, another file its given text.
fn given_preview(given: &Given, path: &str) -> (Opened<DiffFile>, Vec<DiffChangeMark>) {
    if let Some(file) = given.patches.get(path).and_then(|patch| DiffFile::parse(patch).ok()?.into_iter().next()) {
        let text: String = file.side_lines(DiffSide::Modified).map(|(_, line)| format!("{line}\n")).collect();
        let marks = if file.status() == enkidu_diff::DiffFileStatus::Added { Vec::new() } else { file.change_marks() };
        return (whole(path, &text), marks);
    }
    match given.texts.get(path) {
        Some(text) => (whole(path, text), Vec::new()),
        None => (Opened::Missing, Vec::new()),
    }
}

/// A kind of file in a filter's menu: a check when it is the one picked, "*.py" (or "All files"), and how many
/// files have it at the right. `of` names the menu it is in (Find in files', Changed files').
pub(crate) fn kind_option(of: &'static str, kind: Option<&str>, count: Option<usize>, picked: bool) -> PopupMenuItem {
    let id = SharedString::from(format!("{of}:{}", kind.unwrap_or_default()));
    let words = SharedString::from(kind_words(kind));
    let files = count.map(|count| SharedString::from(enkidu_model::text::thousands(count)));
    let is_kind = kind.is_some();
    PopupMenuItem::element(move |_, _| {
        let name = match is_kind {
            true => mono(div()).text_size(px(12.)),
            false => div(),
        };
        div()
            .id(id.clone())
            .test_support()
            .aria_label(words.clone())
            .aria_description(files.clone().unwrap_or_default())
            .aria_selected(picked)
            .w_full()
            .flex()
            .items_center()
            .gap(px(8.))
            .text_color(theme::INK)
            .child(svg().path("icons/check.svg").flex_none().size(px(13.)).text_color(if picked {
                theme::INK
            } else {
                alpha(theme::INK, 0.)
            }))
            .child(name.min_w_0().flex_1().truncate().child(words.clone()))
            .children(
                files.clone().map(|files| {
                    div().flex_none().pl(px(12.)).text_size(px(11.5)).text_color(theme::INK_3).child(files)
                }),
            )
    })
}

/// A definition said in words: "method in StockLevelView, src/views.py:40".
fn symbol_words(hit: &SymbolHit) -> String {
    let within = hit.container.as_ref().map(|container| format!(" in {container}")).unwrap_or_default();
    format!("{}{within}, {}:{}", kind_word(hit.kind), hit.path, hit.line)
}

/// Text with some of it marked: signal blue behind, in full ink.
fn marked(text: &str, marks: Vec<Range<usize>>) -> StyledText {
    let style = HighlightStyle {
        color: Some(theme::INK.into()),
        background_color: Some(alpha(theme::FOCUS, 0.25).into()),
        ..Default::default()
    };
    StyledText::new(text.to_string()).with_highlights(marks.into_iter().map(|mark| (mark, style)))
}

/// Stretches of characters (from, to) as byte ranges of `text`, in order, joined where they touch. One that reaches
/// past the text is left out.
fn char_marks(text: &str, stretches: impl IntoIterator<Item = (usize, usize)>) -> Vec<Range<usize>> {
    let bytes: Vec<usize> = text.char_indices().map(|(byte, _)| byte).chain([text.len()]).collect();
    let mut marks: Vec<Range<usize>> = stretches
        .into_iter()
        .filter(|(from, to)| from < to && *to < bytes.len())
        .map(|(from, to)| bytes[from]..bytes[to])
        .collect();
    marks.sort_by_key(|mark| mark.start);
    let mut joined: Vec<Range<usize>> = Vec::new();
    for mark in marks {
        match joined.last_mut() {
            Some(last) if mark.start <= last.end => last.end = last.end.max(mark.end),
            _ => joined.push(mark),
        }
    }
    joined
}

impl Render for Search {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (status, note) = self.words(cx);
        let typed = self.typed_text(cx);
        // the results take at most 460 px, or 58% of a short window
        let viewport = f32::from(window.viewport_size().height);
        let room = (viewport * 0.58).min(460.);
        // under the rows of Find in files, the file around the picked line, when the window has the room (item 10.6).
        // The popup stands 12% down the window; its tabs, its field and its foot take 122 px
        let row_height = code_row_height(self.ui.read(cx).code_size) as f32;
        let rows_over = (self.rows.len() as f32 * LINE_ROW + 12.).min(ROWS_OVER_PREVIEW);
        let preview =
            self.previewed.as_ref().filter(|_| self.tab == Tab::Text && !self.rows.is_empty()).and_then(|previewed| {
                let space = viewport * 0.88 - 24. - 122. - PREVIEW_HEAD - rows_over;
                let lines = (space / row_height).floor().min(PREVIEW_LINES);
                (lines >= PREVIEW_LEAST).then(|| self.preview_view(previewed, lines * row_height + 6., cx))
            });
        let note = note.map(|note| {
            // one text that wraps, its code in the code face
            let color = if note.problem { theme::LAMP_RED } else { theme::INK_3 };
            let runs: Vec<TextRun> = (note.parts.iter())
                .map(|part| TextRun {
                    len: part.text.len(),
                    font: match part.code {
                        true => Font { features: FontFeatures::disable_ligatures(), ..font(theme::MONO_FONT) },
                        false => font(theme::UI_FONT),
                    },
                    color: if part.code { theme::INK_2.into() } else { color.into() },
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                })
                .collect();
            div()
                .id("search-note")
                .test_support()
                .aria_label(SharedString::from(note.said()))
                .px(px(12.))
                .py(px(24.))
                .text_center()
                .text_size(theme::TEXT)
                .line_height(px(20.))
                .child(StyledText::new(note.said()).with_runs(runs))
        });
        let rows = self.rows.clone();
        let results: AnyElement = if self.tab == Tab::Text && !rows.is_empty() {
            let (active, search) = (self.active, cx.weak_entity());
            let colors = self.ui.read(cx).code_colors;
            // with the preview under them the rows take seven rows at most, and no more than they are
            let height = match preview {
                Some(_) => rows_over,
                None => (rows.len() as f32 * LINE_ROW + 12.).min(room).max(88.),
            };
            let list = uniform_list("search-line-rows", rows.len(), {
                let rows = rows.clone();
                move |range, _, _| {
                    range
                        .map(|ix| line_row(ix, &rows[ix], active == Some(ix), colors, search.clone()))
                        .collect::<Vec<_>>()
                }
            })
            .track_scroll(&self.lines)
            .size_full();
            // a list that only makes the rows in view can't be found by a test: the space it fills can
            div().id("search-lines").test_support().h(px(height)).p(px(6.)).child(list).into_any_element()
        } else {
            let changes = self.changes(cx);
            let list: Vec<AnyElement> =
                rows.iter().enumerate().map(|(ix, row)| self.row(ix, row, &typed, &changes, cx)).collect();
            div()
                .id("search-list")
                .min_h(px(88.))
                .max_h(px(room))
                .flex()
                .flex_col()
                .overflow_y_scroll()
                .track_scroll(&self.scroll)
                .p(px(6.))
                .children(note)
                .children(list)
                .into_any_element()
        };
        div()
            .id("search-popup")
            .test_support()
            .aria_label("Search")
            .aria_description(self.tab.label())
            .key_context(KEYS)
            .relative()
            .w_full()
            .flex()
            .flex_col()
            // ↑ and ↓ move through the rows before the field would move its cursor; ⇥ and ⇧⇥ change the tab before
            // the field would indent
            .capture_action(cx.listener(|this, _: &MoveUp, _, cx| {
                this.move_by(-1, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &MoveDown, _, cx| {
                this.move_by(1, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &IndentInline, window, cx| {
                this.set_tab(this.tab.step(1), window, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &OutdentInline, window, cx| {
                this.set_tab(this.tab.step(-1), window, cx);
                cx.stop_propagation();
            }))
            // the keys that open Search go to their tab while it is open
            .on_action(cx.listener(|this, _: &crate::SearchFiles, window, cx| this.set_tab(Tab::Files, window, cx)))
            .on_action(cx.listener(|this, _: &crate::SearchSymbols, window, cx| this.set_tab(Tab::Symbols, window, cx)))
            .on_action(cx.listener(|this, _: &crate::SearchText, window, cx| this.set_tab(Tab::Text, window, cx)))
            .on_action(cx.listener(|this, _: &crate::ToggleMatchCase, _, cx| {
                if this.tab == Tab::Text {
                    this.toggle(Toggle::Case, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &crate::ToggleWholeWords, _, cx| {
                if this.tab == Tab::Text {
                    this.toggle(Toggle::Word, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &crate::ToggleRegex, _, cx| {
                if this.tab == Tab::Text {
                    this.toggle(Toggle::Regex, cx);
                }
            }))
            .child(self.tabs(&status, cx))
            .child(self.field(cx))
            .child(results)
            .children(preview)
            .child(self.footer())
            .child(crate::dialog_parts::lit_edge())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn characters_that_matched_become_byte_ranges_joined_where_they_touch() {
        // "slv" in StockLevelView: S, L, V
        assert_eq!(char_marks("StockLevelView", [(0, 1), (5, 6), (10, 11)]), [0..1, 5..6, 10..11]);
        // letters next to each other are one mark
        assert_eq!(char_marks("total", [(0, 1), (1, 2), (2, 3)]), vec![(0..3)]);
        // out of order, and overlapping
        assert_eq!(char_marks("sum(items)", [(4, 9), (0, 3), (2, 5)]), vec![(0..9)]);
        // letters of more than one byte before the match
        let text = "título = añadir";
        let marks = char_marks(text, [(9, 15)]);
        assert_eq!(&text[marks[0].clone()], "añadir");
        // a stretch that reaches past the text, or is empty, is left out
        assert_eq!(char_marks("abc", [(1, 9), (2, 2), (0, 3)]), vec![(0..3)]);
    }

    /// Item 10.4: a line of Find in files is drawn in the code's colors with what was found marked over them.
    #[test]
    fn a_lines_colors_are_cut_by_its_marks_into_runs_that_never_lie_over_each_other() {
        let (orange, blue) = ((0xcf, 0x8e, 0x6d, 0xff), (0x56, 0xa8, 0xf5, 0xff));
        // "class StockLevel(Base):": the keyword, then the name; "Level" was found, inside the name
        let spans = vec![(0..5, orange), (6..16, blue)];
        let runs = colored_runs(23, &spans, std::slice::from_ref(&(11..16)));
        assert_eq!(
            runs,
            [(0..5, Run::Colored(orange)), (6..11, Run::Colored(blue)), (11..16, Run::Marked)],
            "the name is cut where the mark starts; the stretch without a color has no run"
        );
        // a mark over two colors and the gap between them is one run
        assert_eq!(
            colored_runs(23, &spans, std::slice::from_ref(&(3..8))),
            [(0..3, Run::Colored(orange)), (3..8, Run::Marked), (8..16, Run::Colored(blue))]
        );
        // no colors at all (a file without a grammar): only the marks; no marks: only the colors
        assert_eq!(colored_runs(10, &[], &[2..4, 6..10]), [(2..4, Run::Marked), (6..10, Run::Marked)]);
        assert_eq!(colored_runs(23, &spans, &[]), [(0..5, Run::Colored(orange)), (6..16, Run::Colored(blue))]);
        // two colors that touch stay two runs, the same color twice in a row becomes one
        assert_eq!(colored_runs(4, &[(0..2, orange), (2..4, blue)], &[]).len(), 2);
        assert_eq!(colored_runs(4, &[(0..2, orange), (2..4, orange)], &[]), [(0..4, Run::Colored(orange))]);
        // whatever is given, the runs are in order and none starts before the last one ended
        let runs = colored_runs(12, &[(0..4, orange), (4..9, blue), (9..12, orange)], &[1..2, 3..10]);
        assert!(runs.windows(2).all(|pair| pair[0].0.end <= pair[1].0.start), "{runs:?}");
        assert_eq!(runs.last().unwrap().0.end, 12);
    }

    /// The line as the popup draws it: the colors of its language, in the scheme Settings names.
    #[test]
    fn a_line_of_find_in_files_is_colored_like_code() {
        let text = "class StockLevel(Base):";
        let spans =
            enkidu_syntax::colors(enkidu_syntax::Scheme::Ide, &enkidu_diff::language_of("src/orders/stock.py"), text);
        let runs = colored_runs(text.len(), &spans, std::slice::from_ref(&(6..16)));
        // the keyword in an IDE's orange, the name that was found marked
        assert_eq!((runs[0].0.start, runs[0].1), (0, Run::Colored((0xcf, 0x8e, 0x6d, 0xff))));
        assert!(runs[0].0.end <= 6, "the keyword's color ends before the name: {runs:?}");
        assert!(runs.contains(&(6..16, Run::Marked)), "{runs:?}");
        // a file without a grammar has no colors: its line is drawn in the row's own ink
        assert!(
            enkidu_syntax::colors(enkidu_syntax::Scheme::Ide, &enkidu_diff::language_of("notes.txt"), "plain words")
                .is_empty()
        );
    }
}
