//! The Files panel (item 3.9d; DESIGN.md, "Files is the workspace as git sees it"): tracked and new files, ignored ones
//! left out. Folders first, single-folder chains on one row, an amber dot on folders that hold changes, the change
//! letter on changed files, and a filter that ranks file names first. A click on a file opens it where the diffs are.
//!
//! Git is asked for the list off the drawing thread, when the panel first shows a workspace and again when files
//! come or go (the changes say so first); the tree is built there too. Only the rows in view are drawn, so a folder
//! of 50,000 files costs no more than one of fifty.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use enkidu_core::git::Change;
use enkidu_core::state::Shared;
use enkidu_model::changes::{ChangeLists, Same};
use enkidu_model::filetree::{self, Files, Found, Tree};
use enkidu_model::frame::Frame;
use enkidu_model::review::split_path;
use enkidu_ui::pieces::{Observed, file_icon, folder_icon};
use enkidu_ui::tips::Tipped as _;
use enkidu_ui::tokens::{self as theme, tone};
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Context, Entity, Focusable as _, FontWeight, Render, Role, SharedString, Subscription,
    UniformListScrollHandle, WeakEntity, Window, div, px, svg, uniform_list,
};

use crate::bridge::Bridge;
use crate::changes_island::{ChangesIsland, glyph_named};
use crate::toast;

/// A folder shows this many files at first; the rest come with "Show all".
pub const FOLDER_PAGE: usize = 300;
/// A filter shows this many matches at most, and says how many more there are.
const MATCHES: usize = 200;

/// Where the list of files comes from.
pub enum Source {
    /// The core: git, off the drawing thread.
    Core,
    /// A given list, for pictures and screen tests without a core.
    Given(Vec<String>),
}

/// A workspace's files as git listed them, and what is made of the list once: the files it has now (with the new
/// ones, less the deleted ones), their tree, and their names ready for the filter.
pub struct Listing {
    /// Git's own mark of the list: it is asked "anything new?" with it, and the list doesn't come back when not.
    version: String,
    listed: Arc<Vec<String>>,
    files: Files,
    tree: Tree,
    /// There are more than the 50,000 that are listed.
    truncated: bool,
}

/// A workspace's files that aren't listed after this long are said to be read ("Reading files…"); a quick read goes
/// from nothing straight to the list (DESIGN.md, principle 5).
const READ_IS_SLOW: std::time::Duration = std::time::Duration::from_millis(600);

/// What the panel keeps for one workspace.
#[derive(Default)]
struct Kept {
    listing: Option<Arc<Listing>>,
    /// The files that came or went (from the changes) when the listing was made: when it differs, git is asked
    /// again.
    key: Option<String>,
    reading: bool,
    /// The first read has taken long enough to notice: "Reading files…" is said until it is back.
    slow: bool,
    /// The folders that are open, and the ones that show all their files.
    open: HashSet<String>,
    all: HashSet<String>,
}

/// One row of the panel, as it is drawn.
enum Row {
    Folder {
        /// "src", or a chain of single folders: "a/b/c".
        label: String,
        /// The folder the row opens.
        path: String,
        depth: usize,
        open: bool,
        /// It holds changed files.
        changed: bool,
    },
    File {
        path: String,
        depth: usize,
        /// What happened to it, when it is one of the changed files.
        kind: Option<String>,
        /// It is the file opened where the diffs are.
        current: bool,
        /// A filter's match: its folder is said after its name, and it stands at the left edge.
        found: bool,
    },
    /// "Show all 1,234 files", after a folder's first 300.
    ShowAll { dir: String, count: usize, depth: usize },
}

pub struct FilesPane {
    frame: Entity<Frame>,
    lists: Entity<ChangeLists>,
    island: Entity<ChangesIsland>,
    source: Source,
    kept: HashMap<String, Kept>,
    /// The filter's field. A text field needs its window, so it is made as the pane is first drawn.
    filter: Option<(Entity<InputState>, Subscription)>,
    /// What the filter found, for the words it was asked with.
    found: Option<(String, Found)>,
    scroll: UniformListScrollHandle,
    /// The workspace the rows were last drawn for: another one starts with an empty filter.
    shown: Option<String>,
    /// The rows as they were last made, and what they were made from. A scrolled frame draws them again without
    /// walking the tree: a folder of 50,000 files would cost every frame otherwise.
    drawn: Option<(Made, Rc<Vec<Row>>)>,
    /// Counts every change to what the rows show (a folder opened, a list read, a filter's answer).
    turn: u64,
}

/// What a set of rows was made from: when it is the same, the rows are.
#[derive(PartialEq)]
struct Made {
    workspace: String,
    turn: u64,
    /// The list of changes itself: the store hands out the very same one until it changes.
    changes: Same,
    current: Option<String>,
}

/// Git's list of a workspace's files, and the tree made of it. Runs off the drawing thread.
fn read_listing(
    shared: &Arc<Shared>,
    workspace: String,
    known: Option<Arc<Listing>>,
    changes: &[Change],
) -> Result<Listing, String> {
    let version = known.as_ref().map(|listing| listing.version.clone());
    let read = enkidu_core::commands::list_files(shared, workspace, version).map_err(|err| err.message)?;
    let (listed, version, truncated) = match (&known, read.unchanged) {
        // the same list as last time: it wasn't sent again
        (Some(known), true) => (known.listed.clone(), known.version.clone(), known.truncated),
        _ => (Arc::new(read.files), read.version, read.truncated),
    };
    Ok(listing(version, listed, truncated, changes))
}

/// What is made of a list of files: the files the workspace has now, their tree, their names for the filter.
fn listing(version: String, listed: Arc<Vec<String>>, truncated: bool, changes: &[Change]) -> Listing {
    let visible = filetree::visible_files(&listed, changes);
    let tree = filetree::build_tree(&visible);
    Listing { version, listed, files: Files::new(visible), tree, truncated }
}

impl FilesPane {
    pub fn new(
        frame: Entity<Frame>,
        lists: Entity<ChangeLists>,
        island: Entity<ChangesIsland>,
        source: Source,
        cx: &mut Context<Self>,
    ) -> FilesPane {
        cx.observe(&frame, |this, _, cx| {
            // a removed workspace's files go with it
            let frame = this.frame.read(cx);
            this.kept.retain(|id, _| frame.workspaces.iter().any(|ws| &ws.id == id));
            cx.notify();
        })
        .detach();
        cx.observe(&lists, |_, _, cx| cx.notify()).detach();
        // the opened file is marked
        cx.observe(&island, |_, _, cx| cx.notify()).detach();
        FilesPane {
            frame,
            lists,
            island,
            source,
            kept: HashMap::new(),
            filter: None,
            found: None,
            scroll: UniformListScrollHandle::new(),
            shown: None,
            drawn: None,
            turn: 0,
        }
    }

    /// How many rows the panel has for the workspace on screen, in view or not (for tests: only the rows in view
    /// are drawn).
    pub fn row_count(&self) -> usize {
        self.drawn.as_ref().map_or(0, |(_, rows)| rows.len())
    }

    fn workspace(&self, cx: &App) -> Option<String> {
        self.frame.read(cx).current_workspace().map(|ws| ws.id.clone())
    }

    fn changes(&self, workspace: &str, cx: &App) -> Arc<Vec<Change>> {
        self.lists.read(cx).of(workspace).cloned().unwrap_or_default()
    }

    /// Asks git for the workspace's files, unless they were read for the set of files as it is now. Editing a file
    /// doesn't change the set; a new or a deleted file does, and the changes say so first.
    fn ensure(&mut self, workspace: &str, cx: &mut Context<Self>) {
        let changes = self.changes(workspace, cx);
        let key = filetree::file_set_key(&changes);
        let kept = self.kept.entry(workspace.to_string()).or_default();
        if kept.reading || kept.key.as_ref() == Some(&key) {
            return;
        }
        self.read(workspace, cx);
    }

    /// Reads the workspace's files again, off the drawing thread.
    fn read(&mut self, workspace: &str, cx: &mut Context<Self>) {
        let changes = self.changes(workspace, cx);
        let key = filetree::file_set_key(&changes);
        let kept = self.kept.entry(workspace.to_string()).or_default();
        kept.reading = true;
        let known = kept.listing.clone();
        let (pane, ws) = (cx.weak_entity(), workspace.to_string());
        // a first read that takes long enough to notice says so (DESIGN.md, principle 5)
        if known.is_none() {
            let ws = ws.clone();
            cx.spawn(async move |pane, cx| {
                cx.background_executor().timer(READ_IS_SLOW).await;
                let _ = pane.update(cx, |pane, cx| {
                    if let Some(kept) = pane.kept.get_mut(&ws).filter(|kept| kept.reading && kept.listing.is_none()) {
                        kept.slow = true;
                        cx.notify();
                    }
                });
            })
            .detach();
        }
        match &self.source {
            Source::Core => {
                let id = ws.clone();
                Bridge::call(
                    cx,
                    move |shared| read_listing(shared, id, known, &changes),
                    move |read, cx| {
                        let _ = pane.update(cx, |pane, cx| pane.was_read(&ws, key, read, cx));
                    },
                );
            }
            Source::Given(paths) => {
                // (a list that is only `STILL_READING` stands for a read that hasn't come back)
                if paths.len() == 1 && paths[0] == crate::changes_island::STILL_READING {
                    cx.notify();
                    return;
                }
                let read = Ok(listing("given".into(), Arc::new(paths.clone()), false, &changes));
                cx.defer(move |cx| {
                    let _ = pane.update(cx, |pane, cx| pane.was_read(&ws, key, read, cx));
                });
            }
        }
        cx.notify();
    }

    fn was_read(&mut self, workspace: &str, key: String, read: Result<Listing, String>, cx: &mut Context<Self>) {
        let Some(kept) = self.kept.get_mut(workspace) else {
            return;
        };
        kept.reading = false;
        kept.slow = false;
        kept.key = Some(key);
        match read {
            Ok(listing) => kept.listing = Some(Arc::new(listing)),
            Err(why) => {
                // what was listed before stays; it is said once, where it can be read
                if let Some(window) = cx.active_window() {
                    let _ = cx.update_window(window, |_, window, cx| {
                        toast::warning(window, "Couldn't list the files", why, cx)
                    });
                }
            }
        }
        // what the filter found was found in the old list
        self.found = None;
        self.turn += 1;
        self.filter_again(cx);
        cx.notify();
    }

    /// A folder opens, or closes.
    fn toggle_folder(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(kept) = self.workspace(cx).and_then(|ws| self.kept.get_mut(&ws)) else {
            return;
        };
        if !kept.open.remove(path) {
            kept.open.insert(path.to_string());
        }
        self.turn += 1;
        cx.notify();
    }

    /// A folder shows all its files, past the first 300.
    fn show_all(&mut self, dir: &str, cx: &mut Context<Self>) {
        if let Some(kept) = self.workspace(cx).and_then(|ws| self.kept.get_mut(&ws)) {
            kept.all.insert(dir.to_string());
            self.turn += 1;
            cx.notify();
        }
    }

    /// A file was opened from elsewhere (Search): its folders open here, so it is there in the tree afterwards.
    pub fn reveal(&mut self, path: &str, cx: &mut Context<Self>) {
        if let Some(kept) = self.workspace(cx).and_then(|ws| self.kept.get_mut(&ws)) {
            kept.open.extend(filetree::parent_folders(path));
            self.turn += 1;
            cx.notify();
        }
    }

    /// A file is picked: it opens where the diffs are. Picked from the filter, its folders open too, so it is
    /// there in the tree afterwards.
    fn pick(&mut self, path: &str, found: bool, cx: &mut Context<Self>) {
        if found && let Some(kept) = self.workspace(cx).and_then(|ws| self.kept.get_mut(&ws)) {
            kept.open.extend(filetree::parent_folders(path));
            self.turn += 1;
        }
        let path = path.to_string();
        self.island.update(cx, |island, cx| island.open_file(&path, None, cx));
        cx.notify();
    }

    /// What is typed in the filter now.
    fn query(&self, cx: &App) -> String {
        self.filter.as_ref().map(|(field, _)| field.read(cx).value().trim().to_string()).unwrap_or_default()
    }

    /// The filter's words changed (or the list did): the files are matched again, off the drawing thread. What
    /// comes back for words that were typed over since is dropped.
    fn filter_again(&mut self, cx: &mut Context<Self>) {
        let query = self.query(cx);
        if query.is_empty() {
            self.found = None;
            self.turn += 1;
            cx.notify();
            return;
        }
        let Some(listing) = self.workspace(cx).and_then(|ws| self.kept.get(&ws)).and_then(|kept| kept.listing.clone())
        else {
            return;
        };
        let asked = query.clone();
        let matching = cx.background_spawn(async move { listing.files.filter(&query, MATCHES) });
        cx.spawn(async move |pane: WeakEntity<Self>, cx| {
            let found = matching.await;
            let _ = pane.update(cx, |pane, cx| {
                if pane.query(cx) == asked {
                    pane.found = Some((asked, found));
                    pane.turn += 1;
                    pane.scroll.scroll_to_item(0, gpui_kit::ScrollStrategy::Top);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// The rows to draw: the filter's matches while something is typed, else the tree with its open folders.
    fn rows(&self, listing: &Listing, kept: &Kept, changes: &[Change], current: Option<&str>) -> Vec<Row> {
        let kinds: HashMap<&str, &str> = changes.iter().map(|c| (c.path.as_str(), c.kind.as_str())).collect();
        let file = |path: String, depth: usize, found: bool| Row::File {
            kind: kinds.get(path.as_str()).map(|kind| kind.to_string()),
            current: current == Some(path.as_str()),
            path,
            depth,
            found,
        };
        if let Some((_, found)) = &self.found {
            return found.matches.iter().map(|path| file(path.clone(), 0, true)).collect();
        }
        let changed: HashSet<String> = changes.iter().flat_map(|c| filetree::parent_folders(&c.path)).collect();
        let mut rows = Vec::new();
        // each open folder's children after it, one step in; folders first
        let mut walk = vec![(String::new(), 0usize, 0usize)];
        while let Some((dir, depth, at)) = walk.pop() {
            let Some(node) = listing.tree.get(&dir) else { continue };
            if let Some(name) = node.dirs.get(at) {
                // come back for the next folder (and then the files) after this one's own rows
                walk.push((dir.clone(), depth, at + 1));
                let (label, path) = filetree::compact_folder(&listing.tree, &dir, name);
                let open = kept.open.contains(&path);
                rows.push(Row::Folder { label, changed: changed.contains(&path), path: path.clone(), depth, open });
                if open {
                    walk.push((path, depth + 1, 0));
                }
                continue;
            }
            let all = kept.all.contains(&dir) || node.files.len() <= FOLDER_PAGE;
            let shown = if all { node.files.len() } else { FOLDER_PAGE };
            for name in &node.files[..shown] {
                let path = if dir.is_empty() { name.clone() } else { format!("{dir}/{name}") };
                rows.push(file(path, depth, false));
            }
            if shown < node.files.len() {
                rows.push(Row::ShowAll { dir, count: node.files.len(), depth });
            }
        }
        rows
    }
}

/// A step of the tree: how far a folder's rows stand in from its own.
const STEP: f32 = 16.;

/// "1,234 files", as the row over the tree says it.
fn files_words(count: usize, more: bool) -> String {
    let lines = enkidu_model::review::lines_words(count);
    let number = lines.split(' ').next().unwrap_or_default();
    format!("{number}{} {}", if more { "+" } else { "" }, if count == 1 { "file" } else { "files" })
}

fn row_frame(id: (&'static str, usize), label: &str) -> Observed {
    div()
        .id(id)
        .test_support()
        .role(Role::TreeItem)
        .aria_label(SharedString::from(label.to_string()))
        .relative()
        .h(theme::LIST_ROW)
        .w_full()
        .flex()
        .items_center()
        .rounded(theme::RADIUS_CONTROL)
        .cursor_pointer()
}

fn row(ix: usize, row: &Row, pane: WeakEntity<FilesPane>) -> AnyElement {
    match row {
        Row::Folder { label, path, depth, open, changed } => {
            let asked = path.clone();
            row_frame(("folder-row", ix), path)
                .aria_description(if *open { "open" } else { "closed" })
                // a step is 16 px; the arrow stands in a 16 px slot, 4 px before the folder's icon
                .gap(px(8.))
                .pl(px(8. + *depth as f32 * STEP))
                .pr(px(8.))
                .text_size(theme::TEXT)
                .text_color(theme::INK_2)
                .hover(|s| s.bg(theme::HOVER).text_color(theme::INK))
                .child(
                    div().flex_none().w(theme::ICON).mr(px(-4.)).flex().justify_center().child(
                        svg()
                            .path(if *open { "icons/chevron-down.svg" } else { "icons/chevron-right.svg" })
                            .size(theme::MARK)
                            .text_color(theme::INK_3),
                    ),
                )
                .child(folder_icon(*open, 16.))
                .child(div().min_w_0().truncate().child(label.clone()))
                .when(*changed, |row| {
                    row.child(div().flex_1()).child(
                        div()
                            .id("folder-changed")
                            .test_support()
                            .aria_label("Has changes")
                            .tipped("Has changes", None)
                            .flex_none()
                            .size(px(6.))
                            .rounded_full()
                            .bg(tone(80., 0.1)),
                    )
                })
                .on_click(move |_, _, cx| {
                    let _ = pane.update(cx, |pane, cx| pane.toggle_folder(&asked, cx));
                })
                .into_any_element()
        }
        Row::File { path, depth, kind, current, found } => {
            let (dir, name) = split_path(path);
            let (asked, found) = (path.clone(), *found);
            let tip = path.clone();
            row_frame(("file-row", ix), path)
                .aria_selected(*current)
                .tipped(tip.clone(), None)
                .gap(px(8.))
                // a file's icon stands under its folder's, past the arrow's slot
                .pl(px(if found { 8. } else { 8. + *depth as f32 * STEP + 20. }))
                .pr(px(8.))
                // the file that is open has the chosen fill, and its name is semibold
                .map(|row| if *current { row.bg(theme::SELECTED) } else { row.hover(|s| s.bg(theme::HOVER)) })
                .child(file_icon(path, 16.))
                .child(
                    div()
                        .min_w_0()
                        .flex_shrink_0()
                        .max_w_full()
                        .truncate()
                        .text_size(theme::TEXT)
                        // bright is for the file on screen alone: a changed file has its letter at the right
                        // (DESIGN.md, "States": chosen never rests on brightness that means something else too)
                        .map(|name| match *current {
                            true => name.font_weight(theme::WEIGHT_TITLE).text_color(theme::INK),
                            false => name.text_color(theme::INK_2),
                        })
                        .child(name.to_string()),
                )
                .when(found && !dir.is_empty(), |row| {
                    row.child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(theme::TEXT_SMALL)
                            .text_color(theme::INK_3)
                            .child(dir.to_string()),
                    )
                })
                .children(
                    kind.as_deref()
                        .map(|kind| div().flex_1().flex().justify_end().child(glyph_named("row-kind", kind))),
                )
                .on_click(move |_, _, cx| {
                    let _ = pane.update(cx, |pane, cx| pane.pick(&asked, found, cx));
                })
                .into_any_element()
        }
        Row::ShowAll { dir, count, depth } => {
            let words = format!("Show all {}", files_words(*count, false));
            let asked = dir.clone();
            row_frame(("show-all", ix), &words)
                .pl(px(8. + *depth as f32 * STEP + 20.))
                .pr(px(8.))
                .text_size(theme::TEXT_SMALL)
                .text_color(theme::INK_3)
                .hover(|s| s.bg(theme::HOVER).text_color(theme::INK))
                .child(words)
                .on_click(move |_, _, cx| {
                    let _ = pane.update(cx, |pane, cx| pane.show_all(&asked, cx));
                })
                .into_any_element()
        }
    }
}

/// A line under the list: why it is short, or that there is more.
fn note(words: String) -> impl IntoElement {
    div()
        .id("files-note")
        .test_support()
        .aria_label(SharedString::from(words.clone()))
        .flex_none()
        .px(px(12.))
        .py(px(8.))
        .text_size(px(12.))
        .line_height(px(16.))
        .text_color(theme::INK_3)
        .child(words)
}

impl Render for FilesPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(workspace) = self.workspace(cx) else {
            return div().into_any_element();
        };
        // the filter's field is made here: a text field needs its window
        if self.filter.is_none() {
            let field = cx.new(|cx| InputState::new(window, cx).placeholder("Filter files").clean_on_escape());
            let heard = cx.subscribe(&field, |this, _, event: &InputEvent, cx| match event {
                InputEvent::Change => this.filter_again(cx),
                // Enter opens the best match
                InputEvent::PressEnter { .. } => {
                    let first = this.found.as_ref().and_then(|(_, found)| found.matches.first().cloned());
                    if let Some(path) = first {
                        this.pick(&path, true, cx);
                    }
                }
                _ => {}
            });
            self.filter = Some((field, heard));
        }
        // another workspace came on screen: its own files, and nothing typed
        if self.shown.as_ref() != Some(&workspace) {
            self.shown = Some(workspace.clone());
            self.found = None;
            self.turn += 1;
            if let Some((field, _)) = &self.filter
                && !field.read(cx).value().is_empty()
            {
                field.update(cx, |field, cx| field.set_value("", window, cx));
            }
        }
        self.ensure(&workspace, cx);
        let Some((field, _)) = self.filter.as_ref().map(|(field, heard)| (field.clone(), heard)) else {
            return div().into_any_element();
        };
        let changes = self.changes(&workspace, cx);
        let current = self.island.read(cx).opened_file(cx);
        let kept = self.kept.get(&workspace);
        let listing = kept.and_then(|kept| kept.listing.clone());
        let typed = self.query(cx);
        // the rows, made again only when something they show changed
        let made = Made {
            workspace: workspace.clone(),
            turn: self.turn,
            changes: Same(changes.clone()),
            current: current.clone(),
        };
        let rows = match &self.drawn {
            Some((from, rows)) if *from == made => rows.clone(),
            _ => {
                let rows = Rc::new(match (&listing, kept) {
                    (Some(listing), Some(kept)) => self.rows(listing, kept, &changes, current.as_deref()),
                    _ => Vec::new(),
                });
                self.drawn = Some((made, rows.clone()));
                rows
            }
        };
        // what stands under the list
        let filtered = self.found.as_ref().filter(|_| !typed.is_empty());
        let under = match (filtered, &listing) {
            (Some((_, found)), _) if found.matches.is_empty() => {
                Some(format!("No file matches \u{201c}{typed}\u{201d}."))
            }
            (Some((_, found)), _) if found.total > found.matches.len() => {
                Some(format!("{} more. Type more of the name to narrow it down.", found.total - found.matches.len()))
            }
            (None, Some(listing)) if listing.truncated => Some("Only the first 50,000 files are listed.".to_string()),
            (None, None) if kept.is_some_and(|kept| kept.slow) => Some("Reading files…".to_string()),
            _ => None,
        };
        let pane = cx.weak_entity();
        let count = rows.len();
        let list = uniform_list("files-rows", count, {
            let pane = pane.clone();
            move |range, _, _| range.map(|ix| row(ix, &rows[ix], pane.clone())).collect::<Vec<_>>()
        })
        .track_scroll(&self.scroll)
        .size_full()
        .px(px(8.))
        .pt(px(4.))
        .pb(px(12.));
        // a list that only makes the rows in view can't be found by a test: the space it fills can
        let list = div().id("files-tree").test_support().flex_1().min_h_0().child(list);
        let lit = field.read(cx).focus_handle(cx).contains_focused(window, cx);
        // how many files, and reading them again
        let count = listing.as_ref().map(|listing| {
            let words = files_words(listing.files.len(), listing.truncated);
            div()
                .id("files-count")
                .test_support()
                .aria_label(SharedString::from(words.clone()))
                .flex_none()
                .text_size(theme::TEXT_SMALL)
                .text_color(theme::INK_3)
                .child(words)
        });
        let refresh = div()
            .id("files-refresh")
            .test_support()
            .role(Role::Button)
            .aria_label("Read the files again")
            .tipped("Read the files again", None)
            .size(theme::CONTROL)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(theme::RADIUS_CONTROL)
            .cursor_pointer()
            .group("files-refresh")
            .hover(|s| s.bg(theme::HOVER))
            .child(
                svg()
                    .path("icons/refresh-cw.svg")
                    .size(theme::ICON)
                    .text_color(theme::INK_3)
                    .group_hover("files-refresh", |s| s.text_color(theme::INK)),
            )
            .on_click(cx.listener(|this, _, _, cx| {
                if let Some(workspace) = this.workspace(cx) {
                    this.read(&workspace, cx);
                }
            }));
        let filter = div()
            .h(theme::CONTROL)
            .flex_1()
            .min_w_0()
            .flex()
            .items_center()
            // (the field brings 8 px itself: its words stand 8 px after the sign)
            .pl(px(8.))
            .pr(px(4.))
            .rounded(theme::RADIUS_CONTROL)
            .bg(theme::WELL)
            .border_1()
            .border_color(if lit { theme::ACCENT } else { theme::LINE_STRONG })
            .child(svg().path("icons/search.svg").flex_none().size(theme::ICON).text_color(theme::INK_3))
            .child(div().flex_1().min_w_0().child(enkidu_ui::pieces::bare_field(&field, px(20.)).id("files-filter")))
            .when(!typed.is_empty(), |filter| {
                let field = field.clone();
                let pane = pane.clone();
                filter.child(
                    div()
                        .id("files-clear")
                        .test_support()
                        .role(Role::Button)
                        .aria_label("Clear the filter")
                        .size(theme::BADGE)
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .rounded(theme::RADIUS_SMALL)
                        .cursor_pointer()
                        .group("files-clear")
                        .hover(|s| s.bg(theme::HOVER))
                        .child(
                            svg()
                                .path("icons/x.svg")
                                .size(px(12.))
                                .text_color(theme::INK_3)
                                .group_hover("files-clear", |s| s.text_color(theme::INK)),
                        )
                        // the field keeps the keyboard, to type the next name. Emptying it this way says nothing, so the
                        // panel is told: the tree is back
                        .on_click(move |_, window, cx| {
                            field.update(cx, |field, cx| {
                                field.set_value("", window, cx);
                                field.focus(window, cx);
                            });
                            let _ = pane.update(cx, |pane, cx| pane.filter_again(cx));
                        }),
                )
            });
        // the row over the tree: the filter, how many files, and reading them again
        let tools = div()
            .id("files-tools")
            .test_support()
            .flex_none()
            .h(theme::SUB_BAR)
            .pl(px(8.))
            .pr(px(12.))
            .flex()
            .items_center()
            .gap(px(8.))
            .child(filter)
            .children(count)
            .child(refresh);
        div()
            .size_full()
            .flex()
            .flex_col()
            .font_weight(FontWeight::NORMAL)
            .child(tools)
            .child(list)
            .children(under.map(note))
            .into_any_element()
    }
}
