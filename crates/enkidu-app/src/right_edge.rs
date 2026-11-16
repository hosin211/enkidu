//! The right edge of a workspace (items 3.9b and 3.9c; DESIGN.md, "The right edge is a tool bar (like an IDE's)"): a
//! bar of two buttons, Changed files (with their number) and Files. Each opens its panel beside the bar, full height; a
//! second click, or ⌥⌘1 / ⌥⌘2, closes it.
//!
//! Changed files is a tree of the folders the changed files are in (item 29.4; `changetree`): each folder says how many
//! it holds and closes with a click, a file's row brings its diff into view. Under the tree stand the files that have
//! remarks but no changes (a click opens the file at its first remark). The file being looked at is marked, and while
//! its folder is closed that folder is. Files is the workspace as git sees it (`files_pane`).

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use enkidu_core::git::Change;
use enkidu_model::changes::ChangeLists;
use enkidu_model::changetree;
use enkidu_model::frame::{Changes, Frame};
use enkidu_model::review::{self, split_path};
use enkidu_model::search::kind_words;
use enkidu_model::ui::{Panel, Ui};
use enkidu_ui::pieces::{
    MenuTrigger, Observed, Pressed, file_icon, folder_icon, island, lit_edge, mono, on_screen_bar,
};
use enkidu_ui::press::pressable;
use enkidu_ui::tips::Tipped as _;
use enkidu_ui::tokens::{self as theme, alpha, bezier, hex, hsla, white};
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    Action, Animation, AnimationExt as _, AnyElement, App, BoxShadow, Context, Entity, FontWeight, Render, Rgba, Role,
    ScrollStrategy, SharedString, UniformListScrollHandle, WeakEntity, Window, div, point, px, svg, uniform_list,
};

use crate::changes_island::{ChangesIsland, counts, glyph_named};
use crate::files_pane::{self, FilesPane};
use crate::remarks::Remarks;
use crate::search::kind_option;

/// The bar at the right edge: the buttons that open the panels.
pub struct ToolRail {
    frame: Entity<Frame>,
    lists: Entity<ChangeLists>,
    ui: Entity<Ui>,
}

impl ToolRail {
    pub fn new(frame: Entity<Frame>, lists: Entity<ChangeLists>, ui: Entity<Ui>, cx: &mut Context<Self>) -> ToolRail {
        cx.observe(&frame, |_, _, cx| cx.notify()).detach();
        cx.observe(&lists, |_, _, cx| cx.notify()).detach();
        cx.observe(&ui, |_, _, cx| cx.notify()).detach();
        ToolRail { frame, lists, ui }
    }
}

fn ring(color: Rgba, y: f32, spread: f32, inset: bool) -> BoxShadow {
    BoxShadow {
        color: hsla(color),
        offset: point(px(0.), px(y)),
        blur_radius: px(0.),
        spread_radius: px(spread),
        inset,
    }
}

/// One button of the bar: its icon, lifted while its panel is open, with how many (the changed files) on a badge.
fn rail_button(
    id: &'static str,
    icon: &'static str,
    label: &'static str,
    key: Box<dyn Action>,
    open: bool,
    badge: Option<usize>,
) -> Pressed {
    let tip = if open { format!("Hide {}", label.to_lowercase()) } else { label.to_string() };
    pressable(div().id(id))
        .test_support()
        .role(Role::Button)
        .aria_label(label)
        .aria_selected(open)
        .tipped(tip, Some(key))
        .relative()
        .size(px(32.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(9.))
        .cursor_pointer()
        .group("rail-button")
        .map(|button| match open {
            true => button
                .bg(theme::CANVAS)
                .shadow(vec![ring(theme::RULE_STRONG, 0., 1., false), ring(white(0.07), 1., 0., true)]),
            false => button.hover(|s| s.bg(theme::HOVER)),
        })
        .child(
            svg()
                .path(icon)
                .size(px(16.))
                .text_color(if open { theme::INK } else { theme::INK_3 })
                .group_hover("rail-button", |s| s.text_color(theme::INK)),
        )
        .children(badge.filter(|count| *count > 0).map(|count| {
            let words = if count > 99 { "99+".to_string() } else { count.to_string() };
            // it pops in from half its size each time the number changes (DESIGN.md, "Motion": "Badges pop in with a
            // small overshoot")
            let pill = enkidu_ui::pieces::tabular(div())
                .id("rail-count-pill")
                .test_support()
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(hex(0xd9dce3))
                // a ring in the frame's color parts it from the button under it
                .shadow(vec![ring(theme::FRAME, 0., 2., false)])
                .font_weight(FontWeight::BOLD)
                .text_color(hex(0x0a0b0e))
                .child(words.clone())
                .with_animation(
                    SharedString::from(format!("rail-count-pop-{count}")),
                    Animation::new(theme::DUR_SLOW).with_easing(|t| bezier(theme::EASE_POP, t)),
                    |pill, t| {
                        let scale = 0.5 + 0.5 * t;
                        pill.h(px(15. * scale))
                            .min_w(px(15. * scale))
                            .px(px(3. * scale))
                            .text_size(px(9.5 * scale))
                            .opacity(t.clamp(0., 1.))
                    },
                );
            div()
                .id("rail-count")
                .test_support()
                .aria_label(SharedString::from(words))
                .absolute()
                .top(px(-4.))
                .right(px(-4.))
                .h(px(15.))
                .min_w(px(15.))
                .flex()
                .items_center()
                .justify_center()
                .child(pill)
        }))
}

impl Render for ToolRail {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let open = self.ui.read(cx).right_panel;
        let changed = (self.frame.read(cx).current_workspace())
            .and_then(|ws| self.lists.read(cx).of(&ws.id))
            .map_or(0, |list| list.len());
        let toggle = |panel: Panel| {
            cx.listener(move |this: &mut Self, _, _, cx| {
                this.ui.update(cx, |ui, cx| {
                    ui.toggle_panel(panel);
                    cx.notify();
                })
            })
        };
        div()
            .id("tool-rail")
            .w(px(enkidu_model::ui::RAIL_WIDTH))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(6.))
            .pt(px(2.))
            .child(
                rail_button(
                    "rail-changes",
                    "icons/file-diff.svg",
                    "Changed files",
                    Box::new(crate::ToggleChangedFiles),
                    open == Some(Panel::Changes),
                    Some(changed),
                )
                .on_click(toggle(Panel::Changes)),
            )
            .child(
                rail_button(
                    "rail-files",
                    "icons/folder-tree.svg",
                    "Files",
                    Box::new(crate::ToggleFiles),
                    open == Some(Panel::Files),
                    None,
                )
                .on_click(toggle(Panel::Files)),
            )
    }
}

/// The panel the bar opened.
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
    /// The list of changes, by where it is kept: the store hands out the very same one until it changes.
    changes: usize,
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
                .h(px(24.))
                .flex_none()
                .flex()
                .items_center()
                .gap(px(6.))
                .pl(px(9.))
                .pr(px(6.))
                .rounded(px(7.))
                .cursor_pointer()
                .border_1()
                .map(|button| match set {
                    // a kind is picked: not every changed file is listed, and the button says so in signal blue
                    true => button.border_color(alpha(theme::FOCUS, 0.5)).bg(alpha(theme::FOCUS, 0.10)),
                    false => button.border_color(theme::RULE_STRONG).hover(|s| s.bg(theme::HOVER)),
                })
                .child(match set {
                    true => mono(div()).text_size(px(12.)).text_color(theme::INK).child(words),
                    false => div().text_size(px(12.)).text_color(theme::INK_3).child(words),
                })
                .child(svg().path("icons/chevron-down.svg").flex_none().size(px(12.)).text_color(theme::INK_3))
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
                .size(px(24.))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .rounded(px(7.))
                .group(id)
                .map(|button| match can {
                    true => button.cursor_pointer().hover(|s| s.bg(theme::HOVER)),
                    // nothing left to open, or to close
                    false => button.opacity(0.45).cursor_default(),
                })
                .child(
                    svg()
                        .path(icon)
                        .size(px(13.))
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
                .h(px(24.))
                .mx(px(8.))
                .mt(px(8.))
                .flex()
                .items_center()
                .gap(px(2.))
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
        let totals = Changes::of(&list);
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
        let made = Made { workspace: workspace.to_string(), changes: Arc::as_ptr(&list) as usize, turn: self.turn };
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
        let band = band("Changed files")
            // how many, and their lines: with the panel open, here and not on the changes' own band
            .when(totals.files > 0, |band| {
                let files = review::files_words(totals.files);
                band.child(
                    enkidu_ui::pieces::tabular(div())
                        .id("changes-count")
                        .test_support()
                        .aria_label(SharedString::from(files.clone()))
                        .text_size(px(12.))
                        .text_color(theme::INK_3)
                        .child(files),
                )
                .child(counts(
                    "changes",
                    workspace,
                    Some(totals.additions),
                    Some(totals.deletions),
                    window,
                    cx,
                ))
            });
        let list = uniform_list("changed-rows", count, move |range, window, cx| {
            range.map(|ix| row(ix, &shown, island.clone(), panel.clone(), window, cx)).collect::<Vec<_>>()
        })
        .track_scroll(&self.scroll)
        .size_full()
        .px(px(6.))
        .pt(px(6.))
        .pb(px(8.));
        // a list that only makes the rows in view can't be found by a test: the space it fills can
        let list = div().id("changed-files").test_support().flex_1().min_h_0().child(list);
        // with no changes the panel keeps its title and nothing under it (DESIGN.md)
        island_frame().child(band).children(tools).child(list).child(lit_edge()).into_any_element()
    }

    /// Files: the workspace as git sees it (`files_pane`).
    fn files_panel(&self) -> AnyElement {
        island_frame().child(self.files.clone()).child(lit_edge()).into_any_element()
    }
}

/// A panel's band: its title, then what the panel adds.
pub(crate) fn band(title: &'static str) -> Observed {
    div()
        .id("panel-band")
        .test_support()
        .aria_label(title)
        .h(theme::BAND)
        .flex_none()
        .flex()
        .items_center()
        .gap(px(8.))
        .pl(px(12.))
        .pr(px(5.))
        .bg(theme::CHROME)
        .border_b_1()
        .border_color(theme::RULE)
        .child(div().text_size(theme::TEXT).font_weight(FontWeight::SEMIBOLD).text_color(theme::INK).child(title))
}

/// The panel is an island of its own, at full height.
fn island_frame() -> Observed {
    island("island-panel", false).size_full()
}

/// How many remarks a file has, in their blue.
fn remark_count(count: usize) -> impl IntoElement {
    let words = count.to_string();
    enkidu_ui::pieces::tabular(div())
        .id("row-remarks")
        .test_support()
        .aria_label(SharedString::from(words.clone()))
        .flex_none()
        .px(px(6.))
        .rounded_full()
        .bg(theme::REMARK_BAND)
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme::ACCENT)
        .child(words)
}

/// A row's own look: 26 px as in the Files panel, rounded, lit under the pointer. `marked` is the row of the file
/// being looked at: filled, with the bar at its left (gray while the window is behind another app: `in_front` says
/// whether it is the front one).
fn row_frame(id: (&'static str, usize), label: &str, marked: bool, in_front: bool) -> Observed {
    div()
        .id(id)
        .test_support()
        .role(Role::TreeItem)
        .aria_label(SharedString::from(label.to_string()))
        .aria_selected(marked)
        .relative()
        .h(theme::TREE_ROW)
        .w_full()
        .flex()
        .items_center()
        .rounded(px(6.))
        .cursor_pointer()
        .map(|row| if marked { row.bg(theme::SELECTED) } else { row.hover(|s| s.bg(theme::HOVER)) })
        .when(marked, |row| row.child(on_screen_bar(6., in_front)))
}

/// What just turned up lights up in signal blue and settles (DESIGN.md, "Motion"): the light over a row.
fn flash(of: &str) -> impl IntoElement {
    div().id("row-flash").test_support().absolute().inset_0().rounded(px(6.)).bg(theme::ACCENT_SOFT).with_animation(
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
    let in_front = window.is_window_active();
    let focused = shown.focused.as_deref();
    match shown.tree.rows.get(ix) {
        Some(changetree::Row::Folder { label, path, depth, open, files }) => {
            // closed, it stands for what it holds: the file being looked at, a file that just turned up
            let marked = !*open && focused.is_some_and(|file| changetree::holds(path, file));
            let fresh = !*open && shown.fresh.iter().any(|file| changetree::holds(path, file));
            let count = review::files_words(*files);
            let (asked, workspace) = (path.clone(), shown.workspace.clone());
            row_frame(("change-folder", ix), path, marked, in_front)
                .aria_description(SharedString::from(format!("{}, {count}", if *open { "open" } else { "closed" })))
                .tipped(path.clone(), None)
                .gap(px(4.))
                .pl(px(4. + *depth as f32 * 12.))
                .pr(px(8.))
                .text_size(theme::TEXT)
                .text_color(theme::INK_2)
                .when(fresh, |row| row.child(flash(path)))
                .child(
                    svg()
                        .path(if *open { "icons/chevron-down.svg" } else { "icons/chevron-right.svg" })
                        .flex_none()
                        .size(px(13.))
                        .text_color(theme::INK_3),
                )
                .child(folder_icon(*open, 14.))
                .child(div().ml(px(2.)).min_w_0().truncate().child(label.clone()))
                .child(
                    enkidu_ui::pieces::tabular(div())
                        .id("folder-files")
                        .test_support()
                        .aria_label(SharedString::from(count.clone()))
                        .flex_none()
                        .ml(px(2.))
                        .text_size(px(11.5))
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
                return div().h(theme::TREE_ROW).into_any_element();
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
            row_frame(("change-row", *change), path, focused == Some(path), in_front)
                .tipped(full, None)
                .gap(px(6.))
                // a file's icon lines up with the folder icons beside it, past their chevrons
                .pl(px(21. + *depth as f32 * 12.))
                .pr(px(6.))
                .when(shown.fresh.contains(path), |row| row.child(flash(path)))
                .child(file_icon(path, 14.))
                .child(
                    div()
                        .ml(px(2.))
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(theme::TEXT)
                        .text_color(theme::INK)
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
                .h(theme::TREE_ROW)
                .w_full()
                .flex()
                .items_end()
                .px(px(8.))
                .pb(px(5.))
                .text_size(px(11.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::INK_3)
                .child("Remarks on files without changes")
                .into_any_element(),
            Some(Under::Remarked { path, remarks, line, id }) => {
                let (dir, name) = split_path(path);
                let (asked, line) = (path.clone(), *line);
                row_frame(("remark-file-row", *id), path, focused == Some(path.as_str()), in_front)
                    .tipped(path.clone(), None)
                    .gap(px(6.))
                    .pl(px(8.))
                    .pr(px(6.))
                    .child(file_icon(path, 14.))
                    .child(
                        div()
                            .ml(px(2.))
                            .min_w_0()
                            .flex_shrink_0()
                            .max_w_full()
                            .truncate()
                            .text_size(theme::TEXT)
                            .text_color(theme::INK)
                            .child(name.to_string()),
                    )
                    // its folder after its name: these files stand outside the tree
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.5))
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
            Some(Under::Space) | None => div().h(theme::TREE_ROW).into_any_element(),
        },
    }
}

impl Render for ToolPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let workspace = self.frame.read(cx).current_workspace().map(|ws| ws.id.clone());
        match (self.ui.read(cx).right_panel, workspace) {
            (Some(Panel::Changes), Some(workspace)) => self.changed_files(&workspace, window, cx),
            (Some(Panel::Files), Some(_)) => self.files_panel(),
            _ => div().into_any_element(),
        }
    }
}
