//! The files' column at the right of the stage (items 3.9b and 3.9c; DESIGN.md, "Layout": "The files' column"): one
//! column with two tabs, Changes (with their number) and Files. ⌥⌘1 and ⌥⌘2 show a tab, or fold the column away.
//!
//! Changes is a tree of the folders the changed files are in (item 29.4; `changetree`): each folder says how many
//! it holds and closes with a click, a file's row brings its diff into view. Under the tree stand the files that have
//! remarks but no changes (a click opens the file at its first remark). The file being looked at is marked, and while
//! its folder is closed that folder is. Files is the workspace as git sees it (`files_pane`).

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use enkidu_core::git::Change;
use enkidu_model::changes::{ChangeLists, Same};
use enkidu_model::changetree;
use enkidu_model::frame::Frame;
use enkidu_model::review::{self, split_path};
use enkidu_model::search::kind_words;
use enkidu_model::ui::{Panel, Ui};
use enkidu_ui::pieces::{MenuTrigger, Observed, changes_seen, file_icon, folder_icon, island, mono, tabular, ticking};
use enkidu_ui::tips::Tipped as _;
use enkidu_ui::tokens::{self as theme, bezier};
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    Action, Animation, AnimationExt as _, AnyElement, App, Context, Entity, FontWeight, Render, Role, ScrollStrategy,
    SharedString, UniformListScrollHandle, WeakEntity, Window, div, px, svg, uniform_list,
};

use crate::changes_island::{ChangesIsland, counts, glyph_named};
use crate::files_pane::{self, FilesPane};
use crate::remarks::Remarks;
use crate::search::kind_option;

/// The panel: its two tabs, and what the one that is shown holds.
pub struct ToolPanel {
    frame: Entity<Frame>,
    lists: Entity<ChangeLists>,
    ui: Entity<Ui>,
    remarks: Entity<Remarks>,
    /// The changes island: a row brings a file's diff into view there, or opens a file there.
    island: Entity<ChangesIsland>,
    /// The Files panel's own view: it keeps each workspace's list, its open folders and the filter.
    files: Entity<FilesPane>,
    scroll: UniformListScrollHandle,
    /// Which folders of Changed files are closed, for each workspace: every folder is open until it is clicked or
    /// "Collapse all" is.
    folds: HashMap<String, changetree::Folds>,
    /// The kind of file Changed files keeps to, for each workspace that has one picked.
    kinds: HashMap<String, String>,
    /// Counts every folder closed or opened and every kind picked: the tree is made again when it moves.
    turn: u64,
    /// The tree as it was last made, and what it was made from: a frame that changes nothing it shows draws it again
    /// without walking the list.
    tree: Option<(Made, Rc<Tree>)>,
}

/// What the tree was made from: when it is the same, the tree is.
#[derive(PartialEq)]
struct Made {
    workspace: String,
    /// The list of changes itself: the store hands out the very same one until it changes.
    changes: Same,
    turn: u64,
}

/// The tree as it is drawn, and what the row of tools over it says.
struct Tree {
    rows: Vec<changetree::Row>,
    /// The kinds of file the filter offers, each with how many changed files have it. None when every changed file
    /// is of one kind: there is nothing to pick.
    kinds: Vec<(String, usize)>,
    /// The kind the filter keeps to.
    kind: Option<String>,
    /// A folder is open, so "Collapse all" has something to do; a folder is closed, so "Expand all" has.
    open: bool,
    closed: bool,
}

/// A row under the tree.
enum Under {
    /// The space that parts the tree from what follows.
    Space,
    /// "Remarks on files without changes", above those files.
    Heading,
    /// A file that has remarks and no changes: it opens at `line`, its first remark. `id` is its number among the
    /// panel's files (the changed ones, the heading, then these).
    Remarked { path: String, remarks: usize, line: usize, id: usize },
}

/// What one frame's rows are drawn from: the tree, what stands under it, and what changes from frame to frame (the
/// file being looked at, the files that just turned up, the remarks), looked up for the rows in view alone.
struct Shown {
    workspace: String,
    list: Arc<Vec<Change>>,
    tree: Rc<Tree>,
    under: Vec<Under>,
    /// The file being looked at.
    focused: Option<String>,
    /// The files that turned up while the panel was on screen: each flashes once.
    fresh: HashSet<String>,
    /// How many remarks each file has, and the first line one is on.
    remarked: HashMap<String, (usize, usize)>,
}

/// The changed files the panel has shown for a workspace since it came on screen, and the ones that turned up
/// since (when each did).
struct Seen {
    paths: HashSet<String>,
    fresh: HashMap<String, Instant>,
}

impl ToolPanel {
    pub fn new(
        frame: Entity<Frame>,
        lists: Entity<ChangeLists>,
        ui: Entity<Ui>,
        remarks: Entity<Remarks>,
        island: Entity<ChangesIsland>,
        files: files_pane::Source,
        cx: &mut Context<Self>,
    ) -> ToolPanel {
        cx.observe(&frame, |_, _, cx| cx.notify()).detach();
        cx.observe(&lists, |_, _, cx| cx.notify()).detach();
        cx.observe(&ui, |_, _, cx| cx.notify()).detach();
        cx.observe(&remarks, |_, _, cx| cx.notify()).detach();
        // the file being looked at changes there
        cx.observe(&island, |_, _, cx| cx.notify()).detach();
        let files = cx.new(|cx| FilesPane::new(frame.clone(), lists.clone(), island.clone(), files, cx));
        ToolPanel {
            frame,
            lists,
            ui,
            remarks,
            island,
            files,
            scroll: UniformListScrollHandle::new(),
            folds: HashMap::new(),
            kinds: HashMap::new(),
            turn: 0,
            tree: None,
        }
    }

    /// The Files panel's own view.
    pub fn files(&self) -> &Entity<FilesPane> {
        &self.files
    }

    /// A folder of Changed files closes, or opens again.
    fn toggle_folder(&mut self, workspace: &str, path: &str, cx: &mut Context<Self>) {
        self.folds.entry(workspace.to_string()).or_default().turn(path);
        self.turn += 1;
        cx.notify();
    }

    /// "Collapse all" (`close`) or "Expand all": every folder of Changed files at once. The list goes back to its
    /// top, where the folders that are left stand.
    fn fold_all(&mut self, workspace: &str, close: bool, cx: &mut Context<Self>) {
        let folds = self.folds.entry(workspace.to_string()).or_default();
        if close {
            folds.close_all();
            self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        } else {
            folds.open_all();
        }
        self.turn += 1;
        cx.notify();
    }

    /// Changed files keeps to one kind of file, or lists them all again.
    fn set_kind(&mut self, workspace: &str, kind: Option<String>, cx: &mut Context<Self>) {
        match kind {
            Some(kind) => self.kinds.insert(workspace.to_string(), kind),
            None => self.kinds.remove(workspace),
        };
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        self.turn += 1;
        cx.notify();
    }

    /// The row of tools over the tree: the kind of file it keeps to, and every folder open or closed at once. Each is
    /// there only while it has something to do: a filter when the changed files are of more than one kind, the two
    /// buttons when the tree has a folder.
    fn tools(&self, workspace: &str, tree: &Rc<Tree>, files: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let folders = tree.open || tree.closed;
        if tree.kinds.is_empty() && !folders {
            return None;
        }
        let panel = cx.weak_entity();
        let filter = (!tree.kinds.is_empty()).then(|| {
            let words = kind_words(tree.kind.as_deref());
            let set = tree.kind.is_some();
            let (tree, panel, workspace) = (tree.clone(), panel.clone(), workspace.to_string());
            MenuTrigger::new("changed-kind", SharedString::from(words.clone()))
                .tip("The kind of file to list")
                .aria_description(if set { "picked" } else { "" })
                .h(theme::CONTROL)
                .flex_none()
                .flex()
                .items_center()
                .gap(px(4.))
                .px(px(8.))
                .rounded(theme::RADIUS_CONTROL)
                .cursor_pointer()
                .border_1()
                .border_color(theme::LINE_STRONG)
                .map(|button| match set {
                    // a kind is picked: not every changed file is listed, and the button says so by being picked
                    true => button.bg(theme::SELECTED),
                    false => button.hover(|s| s.bg(theme::HOVER)),
                })
                .child(match set {
                    true => mono(div()).text_size(theme::TEXT_SMALL).text_color(theme::INK).child(words),
                    false => div().text_size(theme::TEXT_SMALL).text_color(theme::INK_2).child(words),
                })
                .child(svg().path("icons/chevron-down.svg").flex_none().size(theme::MARK).text_color(theme::INK_3))
                .menu(gpui_kit::Anchor::TopLeft, move |menu, _, _| {
                    let pick = |kind: Option<String>| {
                        let (panel, workspace) = (panel.clone(), workspace.clone());
                        move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut App| {
                            let _ = panel.update(cx, |panel, cx| panel.set_kind(&workspace, kind.clone(), cx));
                        }
                    };
                    let every = kind_option("changed-kind-option", None, Some(files), tree.kind.is_none());
                    let mut menu = menu.min_w(px(190.)).item(every.on_click(pick(None))).separator();
                    for (kind, count) in tree.kinds.iter() {
                        let picked = tree.kind.as_deref() == Some(kind.as_str());
                        let option = kind_option("changed-kind-option", Some(kind), Some(*count), picked);
                        menu = menu.item(option.on_click(pick(Some(kind.clone()))));
                    }
                    menu
                })
                .into_any_element()
        });
        let all = |id: &'static str, icon: &'static str, label: &'static str, close: bool, can: bool| {
            let (panel, workspace) = (panel.clone(), workspace.to_string());
            div()
                .id(id)
                .test_support()
                .role(Role::Button)
                .aria_label(label)
                .aria_description(if can { "" } else { "dimmed" })
                .tipped(label, None)
                .size(theme::CONTROL)
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .rounded(theme::RADIUS_CONTROL)
                .group(id)
                .map(|button| match can {
                    true => button.cursor_pointer().hover(|s| s.bg(theme::HOVER)),
                    // nothing left to open, or to close
                    false => button.opacity(0.45).cursor_default(),
                })
                .child(
                    svg()
                        .path(icon)
                        .size(theme::ICON)
                        .text_color(theme::INK_3)
                        .when(can, |icon| icon.group_hover(id, |s| s.text_color(theme::INK))),
                )
                .when(can, |button| {
                    button.on_click(move |_, _, cx| {
                        let _ = panel.update(cx, |panel, cx| panel.fold_all(&workspace, close, cx));
                    })
                })
        };
        Some(
            div()
                .id("changed-tools")
                .test_support()
                .flex_none()
                .h(theme::SUB_BAR)
                .pl(px(8.))
                .pr(px(12.))
                .flex()
                .items_center()
                .children(filter)
                .child(div().flex_1())
                .children(
                    folders
                        .then(|| all("changed-expand", "icons/chevrons-up-down.svg", "Expand all", false, tree.closed)),
                )
                .children(
                    folders.then(|| {
                        all("changed-collapse", "icons/chevrons-down-up.svg", "Collapse all", true, tree.open)
                    }),
                )
                .into_any_element(),
        )
    }

    /// Changed files: the changed files as a tree of their folders, then the files that have remarks only.
    fn changed_files(&mut self, workspace: &str, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let list = self.lists.read(cx).of(workspace).cloned().unwrap_or_default();
        let remarks = self.remarks.read(cx).of(workspace);
        let focused = self.island.read(cx).focused(cx);
        // a file that turns up while you watch flashes once; the ones there when the panel came on screen don't. What
        // was seen lives only while the panel shows this workspace, so opening it again starts still
        let now = cx.background_executor().now();
        let moves = !cx.reduce_motion();
        let seen = window.use_keyed_state(SharedString::from(format!("changed-seen:{workspace}")), cx, |_, _| Seen {
            paths: list.iter().map(|change| change.path.clone()).collect(),
            fresh: HashMap::new(),
        });
        let (fresh, turned_up) = seen.update(cx, |seen, _| {
            let mut turned_up = false;
            for change in list.iter() {
                if seen.paths.insert(change.path.clone()) && moves {
                    seen.fresh.insert(change.path.clone(), now);
                    turned_up = true;
                }
            }
            seen.fresh.retain(|_, since| now.saturating_duration_since(*since) < theme::FLASH);
            (seen.fresh.keys().cloned().collect::<HashSet<String>>(), turned_up)
        });
        if turned_up {
            // once it has settled, the row is drawn without it
            cx.spawn(async move |panel: WeakEntity<Self>, cx| {
                cx.background_executor().timer(theme::FLASH).await;
                let _ = panel.update(cx, |_, cx| cx.notify());
            })
            .detach();
        }
        // how many remarks each file has, and the first line one is on
        let mut remarked: HashMap<String, (usize, usize)> = HashMap::new();
        for remark in remarks.iter() {
            let entry = remarked.entry(remark.path.clone()).or_insert((0, usize::MAX));
            *entry = (entry.0 + 1, entry.1.min(remark.start_line.max(1) as usize));
        }
        // the tree, made again only when the list changed, a folder was closed or opened, or a kind was picked
        let made = Made { workspace: workspace.to_string(), changes: Same(list.clone()), turn: self.turn };
        let tree = match &self.tree {
            Some((from, tree)) if *from == made => tree.clone(),
            _ => {
                // a filter is offered while it would leave something out: the changed files are of more than one kind
                let mut kinds = changetree::kinds(&list);
                if !kinds.iter().any(|(_, count)| *count < list.len()) {
                    kinds.clear();
                }
                // a kind no changed file has any more is forgotten: every file is listed again
                let kind =
                    self.kinds.get(workspace).filter(|kind| kinds.iter().any(|(name, _)| name == *kind)).cloned();
                if kind.is_none() {
                    self.kinds.remove(workspace);
                }
                let every_folder_open = changetree::Folds::default();
                let folds = self.folds.get(workspace).unwrap_or(&every_folder_open);
                let rows = changetree::rows(&list, folds, kind.as_deref());
                let folder = |is_open: bool| {
                    rows.iter().any(|row| matches!(row, changetree::Row::Folder { open, .. } if *open == is_open))
                };
                let (open, closed) = (folder(true), folder(false));
                let tree = Rc::new(Tree { rows, kinds, kind, open, closed });
                self.tree = Some((made, tree.clone()));
                tree
            }
        };
        // the files that have remarks but no changes (a remark left in an opened file), by path
        let changed: HashSet<&str> = list.iter().map(|change| change.path.as_str()).collect();
        let mut elsewhere: Vec<(&str, usize, usize)> = remarked
            .iter()
            .filter(|(path, _)| !changed.contains(path.as_str()))
            .map(|(path, (count, line))| (path.as_str(), *count, *line))
            .collect();
        elsewhere.sort_unstable();
        let mut under = Vec::new();
        if !elsewhere.is_empty() {
            if !tree.rows.is_empty() {
                under.push(Under::Space);
            }
            under.push(Under::Heading);
            under.extend(elsewhere.into_iter().enumerate().map(|(nth, (path, remarks, line))| Under::Remarked {
                path: path.to_string(),
                remarks,
                line,
                id: list.len() + 1 + nth,
            }));
        }
        let count = tree.rows.len() + under.len();
        let tools = self.tools(workspace, &tree, list.len(), cx);
        let shown = Rc::new(Shown { workspace: workspace.to_string(), list, tree, under, focused, fresh, remarked });
        let island = self.island.downgrade();
        let panel = cx.weak_entity();
        let list = uniform_list("changed-rows", count, move |range, window, cx| {
            range.map(|ix| row(ix, &shown, island.clone(), panel.clone(), window, cx)).collect::<Vec<_>>()
        })
        .track_scroll(&self.scroll)
        .size_full()
        .px(px(8.))
        .pt(px(4.))
        .pb(px(12.));
        // a list that only makes the rows in view can't be found by a test: the space it fills can
        let list = div().id("changed-files").test_support().flex_1().min_h_0().child(list);
        // with no changes the panel keeps its tabs and nothing under them (DESIGN.md)
        div().flex_1().min_h_0().flex().flex_col().children(tools).child(list).into_any_element()
    }

    /// The column's bar: its two tabs, as pills. Changes says how many there are.
    fn bar(&self, shown: Panel, changed: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let ticks = changes_seen("panel-count", changed as u64, window, cx);
        // the number ticks into place when it changes while you watch (DESIGN.md, "Motion")
        let count = (changed > 0).then(|| {
            let number = div()
                .id("panel-count")
                .test_support()
                .aria_label(SharedString::from(changed.to_string()))
                .flex_none()
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::INK_3)
                .child(changed.to_string());
            ticking("panel-count-tick", number, ticks)
        });
        let tab =
            |id: &'static str, words: &'static str, panel: Panel, key: Box<dyn Action>, count: Option<AnyElement>| {
                let on = shown == panel;
                div()
                    .id(id)
                    .test_support()
                    .role(Role::Tab)
                    .aria_label(words)
                    .aria_selected(on)
                    .h(theme::CONTROL)
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(8.))
                    .px(px(12.))
                    .rounded_full()
                    .text_size(theme::TEXT)
                    .cursor_pointer()
                    .map(|tab| match on {
                        // the tab that is shown: the chosen fill, its name in ink, semibold
                        true => tab.bg(theme::SELECTED).font_weight(theme::WEIGHT_TITLE).text_color(theme::INK),
                        false => tab
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::INK_2)
                            .hover(|s| s.bg(theme::HOVER).text_color(theme::INK)),
                    })
                    .tipped(words, Some(key))
                    .child(words)
                    .children(count)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.ui.update(cx, |ui, cx| {
                            if ui.show_panel(panel) {
                                cx.notify();
                            }
                        })
                    }))
            };
        let changes = tab("panel-tab-changes", "Changes", Panel::Changes, Box::new(crate::ToggleChangedFiles), count);
        let files = tab("panel-tab-files", "Files", Panel::Files, Box::new(crate::ToggleFiles), None);
        div()
            .id("panel-band")
            .test_support()
            .role(Role::TabList)
            .aria_label(match shown {
                Panel::Changes => "Changes",
                Panel::Files => "Files",
            })
            .h(theme::BAR)
            .flex_none()
            .flex()
            .items_center()
            .gap(px(4.))
            // (a fill starts 8 px from the column's edge)
            .pl(px(8.))
            .pr(px(12.))
            .child(changes)
            .child(files)
            .into_any_element()
    }
}

/// The files' column stands on the frame, at full height, with no ground of its own.
fn island_frame() -> Observed {
    island("island-panel").size_full()
}

/// A step of the tree: how far a folder's rows stand in from its own.
const STEP: f32 = 16.;

/// How many remarks a file has, in their blue.
fn remark_count(count: usize) -> impl IntoElement {
    let words = count.to_string();
    tabular(div())
        .id("row-remarks")
        .test_support()
        .aria_label(SharedString::from(words.clone()))
        .flex_none()
        .h(theme::BADGE)
        .min_w(theme::BADGE)
        .flex()
        .items_center()
        .justify_center()
        .px(px(4.))
        .rounded_full()
        .bg(theme::REMARK_BAND)
        .text_size(theme::CAPTION)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme::ACCENT)
        .child(words)
}

/// A row's own look: 28 px as in the Files tab, rounded, lit under the pointer. `marked` is the row of the file being
/// looked at: it has the chosen fill (its name is semibold).
fn row_frame(id: (&'static str, usize), label: &str, marked: bool) -> Observed {
    div()
        .id(id)
        .test_support()
        .role(Role::TreeItem)
        .aria_label(SharedString::from(label.to_string()))
        .aria_selected(marked)
        .relative()
        .h(theme::LIST_ROW)
        .w_full()
        .flex()
        .items_center()
        .rounded(theme::RADIUS_CONTROL)
        .cursor_pointer()
        .map(|row| if marked { row.bg(theme::SELECTED) } else { row.hover(|s| s.bg(theme::HOVER)) })
}

/// What just turned up flashes white and settles (DESIGN.md, "Motion"): the light over a row.
fn flash(of: &str) -> impl IntoElement {
    let light = div().id("row-flash").test_support().absolute().inset_0().rounded(theme::RADIUS_CONTROL);
    light.bg(theme::SELECTED_HOVER).with_animation(
        SharedString::from(format!("row-flash:{of}")),
        Animation::new(theme::FLASH).with_easing(|t| bezier(theme::EASE_OUT, t)),
        |light, t| light.opacity(1. - t),
    )
}

fn row(
    ix: usize,
    shown: &Shown,
    island: WeakEntity<ChangesIsland>,
    panel: WeakEntity<ToolPanel>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let focused = shown.focused.as_deref();
    match shown.tree.rows.get(ix) {
        Some(changetree::Row::Folder { label, path, depth, open, files }) => {
            // closed, it stands for what it holds: the file being looked at, a file that just turned up
            let marked = !*open && focused.is_some_and(|file| changetree::holds(path, file));
            let fresh = !*open && shown.fresh.iter().any(|file| changetree::holds(path, file));
            let count = review::files_words(*files);
            let (asked, workspace) = (path.clone(), shown.workspace.clone());
            row_frame(("change-folder", ix), path, marked)
                .aria_description(SharedString::from(format!("{}, {count}", if *open { "open" } else { "closed" })))
                .tipped(path.clone(), None)
                // a step is 16 px; the arrow stands in a 16 px slot, 4 px before the folder's icon
                .gap(px(8.))
                .pl(px(8. + *depth as f32 * STEP))
                .pr(px(8.))
                .text_size(theme::TEXT)
                .text_color(theme::INK_2)
                .when(fresh, |row| row.child(flash(path)))
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
                .child(
                    tabular(div())
                        .id("folder-files")
                        .test_support()
                        .aria_label(SharedString::from(count.clone()))
                        .flex_none()
                        .text_size(theme::TEXT_SMALL)
                        .text_color(theme::INK_3)
                        .child(count),
                )
                .on_click(move |_, _, cx| {
                    let _ = panel.update(cx, |panel, cx| panel.toggle_folder(&workspace, &asked, cx));
                })
                .into_any_element()
        }
        Some(changetree::Row::File { change, depth }) => {
            let Some(file) = shown.list.get(*change) else {
                return div().h(theme::LIST_ROW).into_any_element();
            };
            let path = file.path.as_str();
            let (_, name) = split_path(path);
            // "old → new" for a file that was renamed, else its path
            let full = match &file.old_path {
                Some(old) => format!("{old} \u{2192} {path}"),
                None => path.to_string(),
            };
            let remarks = shown.remarked.get(path).map_or(0, |(count, _)| *count);
            let asked = path.to_string();
            let on_screen = focused == Some(path);
            row_frame(("change-row", *change), path, on_screen)
                .tipped(full, None)
                .gap(px(8.))
                // a file's icon stands under its folder's, past the arrow's slot
                .pl(px(8. + *depth as f32 * STEP + 20.))
                .pr(px(8.))
                .when(shown.fresh.contains(path), |row| row.child(flash(path)))
                .child(file_icon(path, 16.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(theme::TEXT)
                        // its name is bright only while it is the file on screen (DESIGN.md, "States")
                        .map(|name| match on_screen {
                            true => name.font_weight(theme::WEIGHT_TITLE).text_color(theme::INK),
                            false => name.text_color(theme::INK_2),
                        })
                        .child(name.to_string()),
                )
                .when(remarks > 0, |row| row.child(remark_count(remarks)))
                .child(counts(
                    "row",
                    &format!("{}:{path}", shown.workspace),
                    file.additions,
                    file.deletions,
                    window,
                    cx,
                ))
                .child(glyph_named("row-kind", &file.kind))
                // its diff comes into view where the changes are
                .on_click(move |_, window, cx| {
                    let _ = island.update(cx, |island, cx| island.focus_change(&asked, window, cx));
                })
                .into_any_element()
        }
        None => match shown.under.get(ix - shown.tree.rows.len()) {
            Some(Under::Heading) => div()
                .h(theme::LIST_ROW)
                .w_full()
                .flex()
                .items_end()
                .px(px(8.))
                .pb(px(4.))
                .text_size(theme::TEXT_STATUS)
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::INK_3)
                .child("Remarks on files without changes")
                .into_any_element(),
            Some(Under::Remarked { path, remarks, line, id }) => {
                let (dir, name) = split_path(path);
                let (asked, line) = (path.clone(), *line);
                row_frame(("remark-file-row", *id), path, focused == Some(path.as_str()))
                    .tipped(path.clone(), None)
                    .gap(px(8.))
                    .px(px(8.))
                    .child(file_icon(path, 16.))
                    .child(
                        div()
                            .min_w_0()
                            .flex_shrink_0()
                            .max_w_full()
                            .truncate()
                            .text_size(theme::TEXT)
                            .text_color(theme::INK_2)
                            .child(name.to_string()),
                    )
                    // its folder after its name: these files stand outside the tree
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(theme::TEXT_SMALL)
                            .text_color(theme::INK_3)
                            .child(dir.to_string()),
                    )
                    .child(remark_count(*remarks))
                    // the file opens where the changes are, at its first remark
                    .on_click(move |_, _, cx| {
                        let _ = island.update(cx, |island, cx| island.open_file(&asked, Some(line), cx));
                    })
                    .into_any_element()
            }
            Some(Under::Space) | None => div().h(theme::LIST_ROW).into_any_element(),
        },
    }
}

impl Render for ToolPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let workspace = self.frame.read(cx).current_workspace().map(|ws| ws.id.clone());
        let (Some(shown), Some(workspace)) = (self.ui.read(cx).panel_shown(), workspace) else {
            return div().into_any_element();
        };
        // with the lists hidden it stands over the work: what floats is true black
        let over = self.ui.read(cx).lists_hidden;
        let changed = self.lists.read(cx).of(&workspace).map_or(0, |list| list.len());
        let bar = self.bar(shown, changed, window, cx);
        let body = match shown {
            Panel::Changes => self.changed_files(&workspace, window, cx),
            Panel::Files => div().flex_1().min_h_0().child(self.files.clone()).into_any_element(),
        };
        island_frame().when(over, |panel| panel.bg(theme::SHEET)).child(bar).child(body).into_any_element()
    }
}
