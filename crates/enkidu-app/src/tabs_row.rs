//! The tabs row (DESIGN.md, "Layout" and "Compact when it has to be"; item 1.17): the project's workspaces as tabs,
//! each with its lamp, its name and what it waits for ("Needs you", Live, the number of changed files); New workspace;
//! and on the right what acts on the workspace on screen.
//!
//! With little room the tabs first drop their extra words (Live keeps its bars, counts keep their numbers, "Needs
//! you" stays), then shrink like a browser's down to 112 px; when even that doesn't fit, the strip scrolls and fades
//! at the cut edge. The tab on screen sits on the pill, which glides to the next one you pick, and is always scrolled
//! into view.
//!
//! The tabs stand in your own order, and a workspace can be put away (DESIGN.md, "Your own order, and workspaces
//! put away"). A tab is dragged onto the tab whose place it takes. "Hide workspace" is in the workspace's menu,
//! which a right-click on any tab opens for that tab's workspace. The ones put away stand behind a small button
//! after the tabs, whose menu brings one back. What is arranged is the frame's (`Frame::move_tab`,
//! `hide_workspace`, `show_workspace`) and is kept with where you were.

use std::collections::HashMap;

use enkidu_core::db::Workspace;
use enkidu_model::frame::Frame;
use enkidu_model::live::LiveState;
use enkidu_model::ui::{Layout, Mode, Ui};
use enkidu_model::work::pr::{PrLamp, TabPr, tab_chip};
use enkidu_model::work::when::since_words;
use enkidu_ui::lamp::{Status, lamp};
use enkidu_ui::live::live_badge;
use enkidu_ui::pieces::{MenuTrigger, Observed, Pressed, icon_button, menu_item};
use enkidu_ui::segmented::{segment_sized, segmented};
use enkidu_ui::tab_pill::scrolling_tab_row;
use enkidu_ui::tips::Tipped as _;
use enkidu_ui::tokens::{self as theme, alpha};
use gpui_kit::base::{ElementExt as _, TestSupportExt as _};
use gpui_kit::component::menu::{ContextMenu, ContextMenuExt as _, PopupMenu, PopupMenuItem};
use gpui_kit::prelude::*;
use gpui_kit::{
    Action, AnyElement, App, BoxShadow, Context, DragMoveEvent, Entity, FocusHandle, Font, FontWeight, IntoElement,
    KeyDownEvent, Pixels, Render, Role, ScrollHandle, SharedString, TextRun, Window, div, linear_color_stop,
    linear_gradient, point, px, svg,
};

use crate::work::store::Work;

/// About what a tab needs to show its name and its words in full: with less room than this for each tab, the tabs are
/// compact.
pub const TAB_ROOM: f32 = 215.;

/// Whether the tabs must be compact: a row `row` wide, whose actions on the right take `actions`, has less than
/// [`TAB_ROOM`] for each of its `tabs` (besides New workspace and the gaps, 64 px).
pub fn compact(row: f32, actions: f32, tabs: usize) -> bool {
    row > 0. && row - actions - 64. < tabs as f32 * TAB_ROOM
}

/// How wide the row and its actions were last drawn.
#[derive(Default, Clone, Copy, PartialEq)]
struct Room {
    row: Pixels,
    actions: Pixels,
}

pub struct WorkspaceTabs {
    frame: Entity<Frame>,
    /// The Work index: a workspace's PR shows on its tab. None in a window without one.
    work: Option<Entity<Work>>,
    /// How the workspace is laid out: the switch on the right shows it and changes it.
    ui: Entity<Ui>,
    scroll: ScrollHandle,
    room: Entity<Room>,
    /// Each tab's focus, by workspace: a click on the tab on screen gives it the keyboard, and then ← and → move it.
    focus: HashMap<String, FocusHandle>,
    /// The pointer is on the Live button: Live turns into Stop.
    live_pointed: bool,
    /// The tab that is being dragged along the row: its workspace, and its place when the drag began.
    dragging: Option<(String, usize)>,
    /// The place of the tab the pointer is over while one is dragged: where the dragged one would land.
    drop_at: Option<usize>,
}

/// A workspace's tab while it is dragged along the row: what the tabs it passes over are told.
#[derive(Clone)]
struct DraggedTab {
    id: String,
    name: SharedString,
    status: Status,
}

/// The tab that follows the pointer while it is dragged: its lamp and its name, lifted off the row.
struct TabGhost {
    name: SharedString,
    status: Status,
}

impl Render for TabGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .h(theme::TAB_HEIGHT)
            .max_w(px(260.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded(theme::TAB_RADIUS)
            .bg(theme::RAISED)
            .border_1()
            .border_color(theme::RULE_STRONG)
            .shadow_lg()
            .text_size(theme::TEXT)
            .font_weight(theme::WEIGHT_TITLE)
            .text_color(theme::INK)
            .child(div().flex_none().child(lamp("tab-ghost-lamp", self.status, 14., 0)))
            .child(div().truncate().child(self.name.clone()))
    }
}

/// Now, in milliseconds since 1970: when a workspace was put away.
fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_millis() as i64)
}

impl WorkspaceTabs {
    pub fn new(frame: Entity<Frame>, ui: Entity<Ui>, work: Option<Entity<Work>>, cx: &mut Context<Self>) -> Self {
        cx.observe(&frame, |_, _, cx| cx.notify()).detach();
        cx.observe(&ui, |_, _, cx| cx.notify()).detach();
        if let Some(work) = &work {
            cx.observe(work, |_, _, cx| cx.notify()).detach();
        }
        let room = cx.new(|_| Room::default());
        cx.observe(&room, |_, _, cx| cx.notify()).detach();
        Self {
            frame,
            work,
            ui,
            scroll: ScrollHandle::new(),
            room,
            focus: HashMap::new(),
            live_pointed: false,
            dragging: None,
            drop_at: None,
        }
    }

    /// Live for the workspace on screen (DESIGN.md, "Live"): a play button at one width in all its states. A press
    /// takes this workspace Live, or stops it; while Live is switching nothing can be pressed. Its tooltip says what
    /// a press would do.
    fn live_button(&self, ws: &Workspace, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let frame = self.frame.read(cx);
        let state = frame.live_state(ws);
        let project = frame.projects.iter().find(|p| p.id == ws.project_id).map(|p| p.name.clone());
        let tip = SharedString::from(state.tip(&ws.name, project.as_deref(), frame.live_elsewhere(ws)));
        let (project_id, target) = (ws.project_id.clone(), (state != LiveState::Live).then(|| ws.id.clone()));
        let store = self.frame.clone();
        let (pointed, look) = (self.live_pointed, crate::live::look(state));
        enkidu_ui::live::live_button("live-button", look, state.words(pointed), state.name(), pointed, window, cx)
            .on_hover(cx.listener(|this, pointed: &bool, _, cx| {
                if this.live_pointed != *pointed {
                    this.live_pointed = *pointed;
                    cx.notify();
                }
            }))
            // (a long sentence wraps inside the tooltip)
            .tipped(tip.clone(), None)
            .on_click(move |_, window, cx| {
                if state.enabled() {
                    crate::live::switch(store.clone(), project_id.clone(), target.clone(), window, cx);
                }
            })
    }

    /// Where the changes go: beside the terminal, above it, or away (DESIGN.md, "Layout": "Three layouts, switched
    /// from the tabs row"). With the changes alone on screen (⇧⌘E) none of the three is chosen.
    fn layout_switch(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let chosen = self.ui.read(cx).chosen();
        let choice = |id: &'static str, title: &'static str, key: Box<dyn Action>, on: bool, pick: fn(&mut Ui)| {
            let icon: &'static str = match id {
                "layout-side" => "icons/layout-side.svg",
                "layout-stack" => "icons/layout-stack.svg",
                _ => "icons/layout-terminal.svg",
            };
            segment_sized(id, icon, 16., title, on).tipped(title, Some(key)).on_click(cx.listener(
                move |this, _, _, cx| {
                    this.ui.update(cx, |ui, cx| {
                        let was = ui.clone();
                        pick(ui);
                        if *ui != was {
                            cx.notify();
                        }
                    })
                },
            ))
        };
        let (beside, above) = (Box::new(crate::ToggleSideLayout), Box::new(crate::ToggleSideLayout));
        let segments = vec![
            choice("layout-side", "Changes beside the terminal", beside, chosen == Mode::Side, |ui| {
                ui.show(Layout::Side)
            }),
            choice("layout-stack", "Changes above the terminal", above, chosen == Mode::Stack, |ui| {
                ui.show(Layout::Stack)
            }),
            choice(
                "layout-terminal",
                "Terminal only",
                Box::new(crate::ToggleTerminalOnly),
                chosen == Mode::Terminal,
                Ui::terminal_only,
            ),
        ];
        let at = match chosen {
            Mode::Side => Some(0),
            Mode::Stack => Some(1),
            Mode::Terminal => Some(2),
            Mode::Review => None,
        };
        segmented("workspace-layout", at, segments, window, cx)
    }

    /// Whether the tabs are compact now (for tests and the pictures).
    pub fn is_compact(&self, cx: &App) -> bool {
        let room = *self.room.read(cx);
        compact(room.row.into(), room.actions.into(), self.frame.read(cx).tabs().len())
    }

    /// Whether the strip is cut on the left and on the right (it has more tabs that way, behind a fade).
    pub fn cut(&self) -> (bool, bool) {
        let (offset, max) = (self.scroll.offset(), self.scroll.max_offset());
        (offset.x < px(-1.), max.x + offset.x > px(1.))
    }
}

/// The action that picks the tab at `ix` (⌘1 for the first), for the first nine.
pub fn tab_action(ix: usize) -> Option<Box<dyn Action>> {
    Some(match ix {
        0 => Box::new(crate::SelectTab1),
        1 => Box::new(crate::SelectTab2),
        2 => Box::new(crate::SelectTab3),
        3 => Box::new(crate::SelectTab4),
        4 => Box::new(crate::SelectTab5),
        5 => Box::new(crate::SelectTab6),
        6 => Box::new(crate::SelectTab7),
        7 => Box::new(crate::SelectTab8),
        8 => Box::new(crate::SelectTab9),
        _ => return None,
    })
}

/// How many files a workspace changed, in words (the label of the count on its tab).
pub fn changed_words(changed: usize) -> String {
    if changed == 1 { "1 changed file".into() } else { format!("{changed} changed files") }
}

/// How many remarks a workspace has that weren't sent to Claude yet, in words (the label of the count on its tab).
pub fn unsent_words(remarks: usize) -> String {
    format!("{} not sent yet", enkidu_model::remarks::remarks_words(remarks))
}

/// How strong a done lamp is on a tab that is not on screen (DESIGN.md, "States": the ranks of the top rows).
const DONE_ELSEWHERE: f32 = 0.55;

/// A tab's name is cut to no less than 2.5 em of the 13 px face.
const NAME_MIN: Pixels = px(32.5);

/// How wide `text` is drawn in the UI face at `size` and `weight`, its figures tabular as a tab's counts are.
fn text_width(window: &Window, text: &str, size: Pixels, weight: FontWeight) -> Pixels {
    let font = Font { weight, features: enkidu_ui::pieces::tabular_figures(), ..gpui_kit::font(theme::UI_FONT) };
    let run = TextRun { len: text.len(), font, ..Default::default() };
    window.text_system().layout_line(text, size, &[run], None).width
}

/// How wide a tab's words are drawn (0 without words), the same pieces as [`tab_words`] lays out.
fn words_width(status: Status, live: bool, remarks: usize, changed: usize, compact: bool, window: &Window) -> Pixels {
    let small = px(11.);
    if status == Status::NeedsYou {
        return text_width(window, status.label(), small, FontWeight::SEMIBOLD);
    }
    // Live: 6 px in, the bars (three of 2.5 px, 2 px apart), and with room 6 px, the word and 8 px out
    let bars = px(2.5 * 3. + 2. * 2.);
    if live {
        return match compact {
            true => px(6.) + bars + px(6.),
            false => px(6.) + bars + px(6.) + text_width(window, "Live", small, FontWeight::SEMIBOLD) + px(8.),
        };
    }
    // unsent remarks: 6 px in, with room the icon and 4 px, the count, 6 px out
    if remarks > 0 {
        let icon = if compact { px(0.) } else { px(10. + 4.) };
        return px(6.) + icon + text_width(window, &remarks.to_string(), small, FontWeight::SEMIBOLD) + px(6.);
    }
    if changed == 0 {
        return px(0.);
    }
    let icon = if compact { px(0.) } else { px(11. + 4.) };
    icon + text_width(window, &changed.to_string(), small, FontWeight::NORMAL)
}

/// The narrowest a tab may get: 112 px, or its padding, lamp, name cut to [`NAME_MIN`], words and PR when they need
/// more, so a narrow tab never runs into the next one.
fn tab_min_width(words: Pixels, pr: Pixels) -> Pixels {
    let after = |part: Pixels| if part > px(0.) { px(8.) + part } else { px(0.) };
    (px(10. + 14. + 8.) + NAME_MIN + after(words) + after(pr) + px(10.)).max(px(112.))
}

/// How wide a tab's PR is drawn: its dot, and with room 4 px and its number in the code face at 11 px.
fn pr_width(pr: Option<&TabPr>, compact: bool) -> Pixels {
    match (pr, compact) {
        (None, _) => px(0.),
        (Some(_), true) => px(6.),
        (Some(pr), false) => px(6. + 4. + 6.6 * (pr.number.to_string().len() + 1) as f32),
    }
}

/// The tab's PR, quietly: its checks as a dot and its number, in ink and semibold when something on it waits on you
/// (not blue: in this row blue is spent once, on the tab on screen; DESIGN.md, "States"). Compact, the dot alone.
fn pr_chip(ix: usize, pr: &TabPr, compact: bool) -> AnyElement {
    let dot = match pr.lamp {
        PrLamp::Red => theme::LAMP_RED,
        PrLamp::Amber => theme::LAMP_AMBER,
        PrLamp::Green => theme::LAMP_GREEN,
        PrLamp::Violet => theme::oklch(0.72, 0.14, 300.),
        PrLamp::Gray => alpha(theme::INK_3, 0.6),
    };
    let lamp = match pr.lamp {
        PrLamp::Red => "red",
        PrLamp::Amber => "amber",
        PrLamp::Green => "green",
        PrLamp::Violet => "violet",
        PrLamp::Gray => "gray",
    };
    let title = SharedString::from(pr.title.clone());
    enkidu_ui::pieces::mono(div())
        .id(("tab-pr", ix))
        .test_support()
        .aria_label(title.clone())
        .aria_description(if pr.waiting { "waits on you" } else { "nothing waits" })
        .aria_value(lamp)
        .flex_none()
        .flex()
        .items_center()
        .gap(px(4.))
        .text_size(px(11.))
        .line_height(px(16.))
        .text_color(theme::INK_3)
        .tipped(title, None)
        .child(div().flex_none().size(px(6.)).rounded_full().bg(dot))
        .when(!compact, |chip| {
            chip.child(
                div()
                    .when(pr.waiting, |number| number.font_weight(FontWeight::SEMIBOLD).text_color(theme::INK))
                    .child(format!("#{}", pr.number)),
            )
        })
        .into_any_element()
}

/// What a tab says after its name, most urgent first; compact, only what can't be left out (words become signs).
fn tab_words(
    ix: usize,
    status: Status,
    live: bool,
    remarks: usize,
    changed: usize,
    compact: bool,
) -> Option<AnyElement> {
    if status == Status::NeedsYou {
        return Some(
            enkidu_ui::pieces::tabular(div())
                .id(("tab-words", ix))
                .test_support()
                .aria_label(status.label())
                .flex_none()
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::LAMP_RED)
                .child(status.label())
                .into_any_element(),
        );
    }
    if live {
        return Some(live_badge(("tab-words", ix), compact).into_any_element());
    }
    // remarks you wrote and haven't sent to Claude: in ink on a soft chip (blue is spent once in this row, on the
    // tab on screen)
    if remarks > 0 {
        return Some(
            enkidu_ui::pieces::tabular(div())
                .id(("tab-words", ix))
                .test_support()
                .aria_label(unsent_words(remarks))
                .flex_none()
                .flex()
                .items_center()
                .gap(px(4.))
                .h(px(16.))
                .px(px(6.))
                .rounded_full()
                .bg(theme::SOFT)
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::INK)
                .when(!compact, |d| {
                    d.child(svg().path("icons/message-square.svg").flex_none().size(px(10.)).text_color(theme::INK_2))
                })
                .child(remarks.to_string())
                .into_any_element(),
        );
    }
    (changed > 0).then(|| {
        enkidu_ui::pieces::tabular(div())
            .id(("tab-words", ix))
            .test_support()
            .aria_label(changed_words(changed))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(4.))
            .text_size(px(11.))
            .text_color(theme::INK_3)
            .when(!compact, |d| {
                d.child(svg().path("icons/file-diff.svg").flex_none().size(px(11.)).text_color(theme::INK_3))
            })
            .child(changed.to_string())
            .into_any_element()
    })
}

/// The edge where the strip is cut: the frame's color fading out over 28 px.
fn fade(id: &'static str, left: bool) -> Observed {
    let (solid, clear) = (linear_color_stop(theme::FRAME, 0.), linear_color_stop(alpha(theme::FRAME, 0.), 1.));
    let (from, to) = if left {
        (solid, clear)
    } else {
        (linear_color_stop(alpha(theme::FRAME, 0.), 0.), linear_color_stop(theme::FRAME, 1.))
    };
    div()
        .id(id)
        .test_support()
        .absolute()
        .top_0()
        .bottom_0()
        .w(px(28.))
        .when(left, |d| d.left_0())
        .when(!left, |d| d.right_0())
        .bg(linear_gradient(90., from, to))
}

impl WorkspaceTabs {
    /// A key on a tab that has the keyboard: ← and → move to the tab before or after (around the ends), Enter and
    /// Space put it on screen.
    fn tab_key(&mut self, ix: usize, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        if keystroke.modifiers.modified() {
            return;
        }
        let tabs: Vec<String> = self.frame.read(cx).tabs().iter().map(|w| w.id.clone()).collect();
        let count = tabs.len();
        if count == 0 || ix >= count {
            return;
        }
        match keystroke.key.as_str() {
            "left" | "right" => {
                let next = if keystroke.key == "right" { (ix + 1) % count } else { (ix + count - 1) % count };
                if let Some(handle) = self.focus.get(&tabs[next]) {
                    handle.focus(window, cx);
                }
                cx.stop_propagation();
            }
            "enter" | "space" => {
                self.frame.update(cx, |frame, cx| {
                    if frame.select_workspace(&tabs[ix]) {
                        cx.notify();
                    }
                });
                cx.stop_propagation();
            }
            _ => {}
        }
    }
}

/// The ring on a tab that has the keyboard (DESIGN.md, "States"): signal blue, 2 px wide, standing 2 px inside the
/// tab's edge, so it is never taken for the blue edge of the tab on screen (that tab keeps its edge, and has the
/// ring inside it as well).
///
/// The rings are inset shadows without blur, painted one over the other: inside the tab, so the strip never cuts
/// them where it scrolls (it cut the old ring, which stood outside), and nothing is painted under the tab (GPUI
/// paints an outer shadow under the whole of what casts it: the old ring filled the tab with blue).
fn focus_ring(on_screen: bool) -> Vec<BoxShadow> {
    let ring = |color: gpui_kit::Rgba, reach: f32| BoxShadow {
        color: color.into(),
        offset: point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: px(reach),
        inset: true,
    };
    let edge = f32::from(theme::ON_SCREEN_EDGE);
    match on_screen {
        // over the pill: the ring, the pill's own fill as the gap, and the pill's blue edge again
        true => {
            vec![ring(theme::ACCENT, edge + 4.), ring(theme::SELECTED_ON_FRAME, edge + 2.), ring(theme::ACCENT, edge)]
        }
        // the ring, and the row's own color as the gap
        false => vec![ring(theme::ACCENT, 4.), ring(theme::FRAME, 2.)],
    }
}

impl WorkspaceTabs {
    fn tab(
        &self,
        ix: usize,
        ws: &Workspace,
        frame: &Frame,
        selected: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> ContextMenu<Pressed> {
        let compact = self.is_compact(cx);
        let status = Status::from_core(frame.status(&ws.id));
        let (live, remarks, changed) = (frame.is_live(ws), frame.remarks(&ws.id), frame.changed_files(&ws.id));
        let words = tab_words(ix, status, live, remarks, changed, compact);
        // the workspace's PR, if it has one: its number and its checks' lamp
        let pr = (self.work.as_ref())
            .and_then(|work| tab_chip(work.read(cx).snapshot.as_deref(), &ws.id, Some(&ws.session_id)));
        let tab_min =
            tab_min_width(words_width(status, live, remarks, changed, compact, window), pr_width(pr.as_ref(), compact));
        let tip = match &ws.base {
            Some(base) => format!("{}, from {base}", ws.branch),
            None => ws.branch.clone(),
        };
        let id = ws.id.clone();
        // while a tab is dragged: this one is the one held (its place in the row dims), or the one the pointer is
        // over, which shows where the held one would land: at its right edge when it comes from the left, at its
        // left edge when it comes from the right
        let held = self.dragging.as_ref().is_some_and(|(dragged, _)| dragged == &ws.id);
        let lands = match (&self.dragging, self.drop_at) {
            (Some((_, from)), Some(at)) if at == ix && *from != ix => Some(*from < ix),
            _ => None,
        };
        let dragged = DraggedTab { id: ws.id.clone(), name: ws.name.clone().into(), status };
        let (this, menu_frame, menu_ws) = (cx.entity().downgrade(), self.frame.clone(), ws.clone());
        // it answers a press, as every tab and button does
        enkidu_ui::press::pressable(div().id(("tab", ix)))
            .test_support()
            .role(Role::Tab)
            .aria_label(SharedString::from(ws.name.clone()))
            .aria_selected(selected)
            // (what a test reads: whether it is the one held, or where the held one would land beside it)
            .aria_description(match (held, lands) {
                (true, _) => "held",
                (_, Some(true)) => "lands after",
                (_, Some(false)) => "lands before",
                _ => "",
            })
            .relative()
            .when(held, |d| d.opacity(0.35))
            .children(lands.map(|after| {
                let line = div().absolute().top(px(6.)).bottom(px(6.)).w(px(2.)).rounded_full().bg(theme::ACCENT);
                if after { line.right_0() } else { line.left_0() }
            }))
            // dragged along the row, it takes the place of the tab it is let go on
            .on_drag(dragged, move |dragged, _, _, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.dragging = Some((dragged.id.clone(), ix));
                    cx.notify();
                });
                cx.new(|_| TabGhost { name: dragged.name.clone(), status: dragged.status })
            })
            .on_drag_move(cx.listener(move |this, event: &DragMoveEvent<DraggedTab>, _, cx| {
                let over = event.bounds.contains(&event.event.position);
                if over && this.drop_at != Some(ix) {
                    this.drop_at = Some(ix);
                    cx.notify();
                } else if !over && this.drop_at == Some(ix) {
                    this.drop_at = None;
                    cx.notify();
                }
            }))
            .on_drop(cx.listener(move |this, dragged: &DraggedTab, _, cx| {
                (this.dragging, this.drop_at) = (None, None);
                this.frame.update(cx, |frame, cx| {
                    if frame.move_tab(&dragged.id, ix) {
                        cx.notify();
                    }
                });
                cx.notify();
            }))
            .track_focus(&self.focus[&ws.id])
            .focus_visible(move |s| s.shadow(focus_ring(selected)))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| this.tab_key(ix, event, window, cx)))
            .h(theme::TAB_HEIGHT)
            // it shrinks like a browser's tab down to 112 px, or to what it must show when that is more
            .min_w(tab_min)
            .max_w(px(260.))
            .flex_shrink(1.)
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded(theme::TAB_RADIUS)
            .text_size(theme::TEXT)
            .text_color(if selected { theme::INK } else { theme::INK_2 })
            .cursor_pointer()
            .when(!selected, |d| d.hover(|s| s.bg(theme::HOVER).text_color(theme::INK)))
            .tipped(tip, tab_action(ix))
            .child(
                div()
                    .flex_none()
                    // "done" is the least urgent thing a workspace says: quiet on a tab that is not on screen
                    .when(status == Status::Done && !selected, |d| d.opacity(DONE_ELSEWHERE))
                    .child(lamp(SharedString::from(format!("tab-lamp-{id}")), status, 14., frame.pops(&ws.id))),
            )
            .child(
                div()
                    .min_w(NAME_MIN)
                    .flex_shrink(1.)
                    .truncate()
                    .when(selected, |d| d.font_weight(theme::WEIGHT_TITLE))
                    .child(ws.name.clone()),
            )
            .children(words)
            .children(pr.as_ref().map(|pr| pr_chip(ix, pr, compact)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.frame.update(cx, |frame, cx| {
                    if frame.select_workspace(&id) {
                        cx.notify();
                    }
                })
            }))
            // a right-click: the workspace's menu, for this tab's workspace (it need not be on screen)
            .context_menu(move |menu, _, _| workspace_items(menu, menu_frame.clone(), menu_ws.clone()))
    }

    /// The workspaces of the project on screen that are put away, behind a small button after the tabs: an eye
    /// with a line through it and their number. Its menu lists them, each with its lamp, its name and since when;
    /// a click brings one back to its place among the tabs, and on screen.
    fn hidden_button(&self, hidden: &[&Workspace], frame: &Frame) -> AnyElement {
        let count = hidden.len();
        let words = match count {
            1 => "1 hidden workspace".to_string(),
            count => format!("{count} hidden workspaces"),
        };
        let now = now_ms();
        let rows: Vec<HiddenRow> = (hidden.iter())
            .map(|ws| HiddenRow {
                id: ws.id.clone(),
                name: ws.name.clone().into(),
                status: Status::from_core(frame.status(&ws.id)),
                since: since_words(frame.hidden.get(&ws.id).copied().unwrap_or(now), now).into(),
            })
            .collect();
        let frame = self.frame.clone();
        MenuTrigger::new("hidden-workspaces", SharedString::from(words.clone()))
            .tip(words)
            .aria_value(SharedString::from(count.to_string()))
            .h(theme::PROJECT_TAB_HEIGHT)
            .flex_none()
            .flex()
            .items_center()
            .gap(px(5.))
            .rounded(theme::RADIUS_CONTROL)
            .px(px(7.))
            .text_size(theme::TEXT_SMALL)
            .text_color(theme::INK_3)
            .cursor_pointer()
            .hover(|s| s.bg(theme::HOVER).text_color(theme::INK_2))
            .child(svg().path("icons/eye-off.svg").flex_none().size(px(14.)).text_color(theme::INK_3))
            .child(enkidu_ui::pieces::tabular(div()).child(count.to_string()))
            .menu(gpui_kit::Anchor::TopLeft, move |menu, _, _| {
                let mut menu = menu.min_w(px(280.)).max_h(px(420.)).scrollable(true).label("Hidden workspaces");
                for (at, row) in rows.iter().enumerate() {
                    let (frame, id) = (frame.clone(), row.id.clone());
                    menu = menu.item(hidden_row(at, row.clone()).on_click(move |_, _, cx| {
                        frame.update(cx, |frame, cx| {
                            if frame.show_workspace(&id) {
                                cx.notify();
                            }
                        })
                    }));
                }
                menu
            })
            .into_any_element()
    }
}

/// A workspace that is put away, as the menu of the hidden ones lists it.
#[derive(Clone)]
struct HiddenRow {
    id: String,
    name: SharedString,
    status: Status,
    /// "since Mon 2 Nov".
    since: SharedString,
}

/// One line of that menu: its lamp, its name, and since when it is put away.
fn hidden_row(at: usize, row: HiddenRow) -> PopupMenuItem {
    PopupMenuItem::element(move |_, _| {
        div()
            .id(("hidden-workspace", at))
            .test_support()
            .aria_label(row.name.clone())
            .aria_description(row.since.clone())
            .aria_value(SharedString::from(row.id.clone()))
            .w_full()
            .flex()
            .items_center()
            .gap(px(8.))
            .text_color(theme::INK)
            .child(div().flex_none().child(lamp(("hidden-lamp", at), row.status, 14., 0)))
            .child(div().min_w_0().flex_1().truncate().child(row.name.clone()))
            .child(div().flex_none().pl(px(12.)).text_size(px(11.5)).text_color(theme::INK_3).child(row.since.clone()))
    })
}

impl Render for WorkspaceTabs {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::measure::TAB_ROWS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let frame = self.frame.read(cx).clone();
        let project = frame.current_project.as_ref().and_then(|id| frame.projects.iter().find(|p| &p.id == id));
        let tabs = frame.tabs();
        // the ones that are put away: behind their button, after the tabs
        let hidden = frame.hidden_tabs();
        for ws in &tabs {
            self.focus.entry(ws.id.clone()).or_insert_with(|| cx.focus_handle());
        }
        self.focus.retain(|id, _| tabs.iter().any(|w| &w.id == id));
        // a drag that ended, on a tab or anywhere else: nothing is held any more
        if !cx.has_active_drag() {
            (self.dragging, self.drop_at) = (None, None);
        }
        // no workspaces yet: no row at all (the empty screen under it says what to do). With every workspace put
        // away the row stays: the button of the hidden ones is how one comes back
        let Some(project) = project.filter(|_| !tabs.is_empty() || !hidden.is_empty()) else {
            return div().id("tabs-row").into_any_element();
        };
        let current = frame.current_workspace().map(|w| w.id.clone());
        let current_ws = frame.current_workspace().cloned();
        // the main folder itself has no Live: Live copies a workspace's changes into that folder (item 19.4)
        let can_go_live = current_ws.as_ref().is_some_and(|ws| !frame.is_main_folder(ws));
        let selected = tabs.iter().position(|w| Some(&w.id) == current.as_ref()).unwrap_or(0);
        let views: Vec<ContextMenu<Pressed>> =
            tabs.iter().enumerate().map(|(ix, ws)| self.tab(ix, ws, &frame, ix == selected, window, cx)).collect();
        let hidden_button = (!hidden.is_empty()).then(|| self.hidden_button(&hidden, &frame));
        let (cut_left, cut_right) = self.cut();
        let strip = scrolling_tab_row("workspace-tabs", selected, views, &self.scroll, window, cx)
            .size_full()
            // a wheel scroll moves the fades at the cut edges
            .on_scroll(cx.listener(|_, _, _, cx| cx.notify()));
        let (room, room_for_actions) = (self.room.clone(), self.room.clone());
        div()
            .id("tabs-row")
            .test_support()
            .w_full()
            .h(theme::TABS_ROW)
            .flex_none()
            .flex()
            .items_center()
            .px(theme::GAP)
            .gap(px(4.))
            .child(
                div()
                    .id("tab-strip")
                    .test_support()
                    .role(Role::TabList)
                    .aria_label(SharedString::from(format!("Workspaces in {}", project.name)))
                    .relative()
                    .min_w_0()
                    .flex_shrink(1.)
                    .flex()
                    .child(strip)
                    .when(cut_left, |d| d.child(fade("tab-fade-left", true)))
                    .when(cut_right, |d| d.child(fade("tab-fade-right", false))),
            )
            .children(hidden_button)
            .child({
                let frame = self.frame.clone();
                icon_button(
                    "new-workspace",
                    "icons/plus.svg",
                    SharedString::from(format!("New workspace in {}", project.name)),
                )
                .tipped(format!("New workspace in {}", project.name), Some(Box::new(crate::NewWorkspace)))
                .on_click(move |_, window, cx| crate::new_workspace::open(frame.clone(), window, cx))
            })
            .child(div().flex_1().min_w(px(16.)))
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    // what acts on the workspace on screen: nothing of it while every workspace is put away
                    .when(current_ws.is_some(), |actions| {
                        actions
                            .child(self.layout_switch(window, cx))
                            .child(div().flex_none().h(px(16.)).w(px(1.)).bg(theme::RULE))
                    })
                    .children(current_ws.as_ref().filter(|_| can_go_live).map(|ws| self.live_button(ws, window, cx)))
                    .children(current_ws.map(|ws| workspace_menu(self.frame.clone(), ws)))
                    .on_prepaint(move |bounds, _, cx| {
                        room_for_actions.update(cx, |room, cx| {
                            if room.actions != bounds.size.width {
                                room.actions = bounds.size.width;
                                cx.notify();
                            }
                        })
                    }),
            )
            .on_prepaint(move |bounds, _, cx| {
                room.update(cx, |room, cx| {
                    if room.row != bounds.size.width {
                        room.row = bounds.size.width;
                        cx.notify();
                    }
                })
            })
            .into_any_element()
    }
}

/// The workspace on screen's menu, behind the … at the right of the row; its items are [`workspace_items`].
fn workspace_menu(frame: Entity<Frame>, ws: Workspace) -> impl IntoElement {
    MenuTrigger::new("ws-menu", "Workspace actions")
        .tip("Workspace actions")
        .size(theme::PROJECT_TAB_HEIGHT)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(theme::RADIUS_CONTROL)
        .cursor_pointer()
        .hover(|s| s.bg(theme::HOVER))
        .child(svg().path("icons/ellipsis.svg").size(px(16.)).text_color(theme::INK_2))
        .menu(gpui_kit::Anchor::TopRight, move |menu, _, _| workspace_items(menu, frame.clone(), ws.clone()))
}

/// A workspace's menu: its name, Show in Finder, Copy path, Copy branch name, Project settings, then Hide workspace and
/// Remove workspace…, which asks first. The … at the right of the row opens it for the workspace on screen, a
/// right-click on a tab for that tab's workspace.
fn workspace_items(menu: PopupMenu, frame: Entity<Frame>, ws: Workspace) -> PopupMenu {
    let (path, branch, place) = (ws.path.clone(), ws.branch.clone(), crate::projects_row::short_path(&ws.path));
    let (hiding, hidden) = (frame.clone(), ws.id.clone());
    let (frame, removed) = (frame.clone(), ws.clone());
    menu.label(ws.name.clone())
        .item(menu_item("ws-show-in-finder", "icons/folder-open.svg", "Show in Finder", theme::INK).on_click({
            let path = path.clone();
            move |_, _, cx| cx.reveal_path(std::path::Path::new(&path))
        }))
        .item(menu_item("ws-copy-path", "icons/copy.svg", "Copy path", theme::INK).on_click({
            let path = path.clone();
            move |_, window, cx| {
                cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(path.clone()));
                crate::toast::note(window, "Path copied", place.clone(), cx);
            }
        }))
        .item(menu_item("ws-copy-branch", "icons/git-branch.svg", "Copy branch name", theme::INK).on_click(
            move |_, window, cx| {
                cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(branch.clone()));
                crate::toast::note(window, "Branch name copied", branch.clone(), cx);
            },
        ))
        .item(
            menu_item("ws-project-settings", "icons/sliders-horizontal.svg", "Project settings", theme::INK).on_click(
                {
                    let (frame, page) = (frame.clone(), enkidu_model::settings::Page::Project(ws.project_id.clone()));
                    move |_, window, cx| crate::settings::open_here(frame.clone(), page.clone(), window, cx)
                },
            ),
        )
        .separator()
        // put away: its tab goes, and nothing else of it (the button after the tabs brings it back)
        .item(menu_item("hide-workspace", "icons/eye-off.svg", "Hide workspace", theme::INK).on_click(
            move |_, _, cx| {
                hiding.update(cx, |frame, cx| {
                    if frame.hide_workspace(&hidden, now_ms()) {
                        cx.notify();
                    }
                })
            },
        ))
        .item(
            menu_item("remove-workspace", "icons/trash.svg", "Remove workspace…", theme::LAMP_RED).on_click(
                move |_, window, cx| crate::remove_workspace::open(frame.clone(), removed.clone(), window, cx),
            ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_go_compact_when_each_has_less_than_its_room() {
        // a 1360 px row with 150 px of actions has room for 5 tabs at 215 px
        assert!(!compact(1360., 150., 5));
        assert!(compact(1360., 150., 6));
        assert!(!compact(0., 0., 3), "not measured yet: in full");
        assert_eq!(changed_words(1), "1 changed file");
        assert_eq!(changed_words(5), "5 changed files");
    }

    /// The keyboard's ring is drawn inside the tab: an outer shadow is cut where the strip scrolls, and GPUI paints
    /// it under the whole tab. It is blue, 2 px wide and 2 px from the tab's edge; on the tab on screen the pill's
    /// own edge is painted last, so it stays.
    #[test]
    fn the_keyboards_ring_stands_inside_the_tab() {
        let blue: gpui_kit::Hsla = theme::ACCENT.into();
        for on_screen in [false, true] {
            let ring = focus_ring(on_screen);
            assert!(ring.iter().all(|shadow| shadow.inset && shadow.blur_radius == px(0.)), "{on_screen}");
            // painted one over the other: each ring reaches less far in than the one before
            assert!(ring.windows(2).all(|pair| pair[0].spread_radius > pair[1].spread_radius), "{on_screen}");
            assert_eq!(ring[0].color, blue, "the ring itself");
            assert_eq!(ring[0].spread_radius - ring[1].spread_radius, px(2.), "2 px wide");
            assert!(ring[0].spread_radius <= px(6.), "clear of the tab's words: its padding is 10 px");
        }
        let (plain, on_screen) = (focus_ring(false), focus_ring(true));
        assert_eq!(plain[1].spread_radius, px(2.), "2 px from the edge");
        assert_eq!(on_screen.last().map(|edge| (edge.color, edge.spread_radius)), Some((blue, theme::ON_SCREEN_EDGE)));
    }
}
