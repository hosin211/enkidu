//! The palette (⌘K; DESIGN.md, "The palette (⌘K) reaches every workspace of every project"; item 1.21), in its
//! first form: every workspace of every project with its lamp, project, branch, Live and key, then the actions that
//! work so far, narrowed as you type. ↑ and ↓ move (around the ends), Enter or a click goes there, Esc or ⌘K
//! closes it. What the keyboard opens many times a day is instant: no motion.

use enkidu_model::frame::Frame;
use enkidu_model::palette::{Group, Pick, Row, flat, groups, narrow, step};
use enkidu_model::ui::{DiffLayout, Layout, Panel, Ui};
use enkidu_ui::lamp::{Status, lamp};
use enkidu_ui::live::live_badge;
use enkidu_ui::pieces::on_screen_bar;
use enkidu_ui::tokens::{self as theme, tone};
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::dialog::DialogEntrance;
use gpui_kit::component::input::{InputEvent, InputState, MoveDown, MoveUp};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, Context, Entity, FontWeight, IntoElement, Render, Role, ScrollHandle, SharedString, Subscription,
    Window, div, px, svg,
};

pub struct Palette {
    frame: Entity<Frame>,
    query: Entity<InputState>,
    /// Every row, as the frame was when the palette opened.
    all: Vec<Group>,
    /// The rows that match what is typed.
    shown: Vec<Group>,
    selected: usize,
    scroll: ScrollHandle,
    /// How the window is laid out, in a window that keeps it.
    ui: Option<Entity<Ui>>,
    _subscriptions: Vec<Subscription>,
}

/// Opens the palette over the window.
pub fn open(frame: Entity<Frame>, window: &mut Window, cx: &mut gpui_kit::App) {
    let view = cx.new(|cx| Palette::new(frame, window, cx));
    let shown = view.clone();
    window.open_dialog(cx, move |dialog, _, _| {
        let on_enter = shown.clone();
        crate::dialog_parts::surface(dialog)
            .w(px(600.))
            .margin_top(px(110.))
            .entrance(DialogEntrance::None)
            .child(shown.clone())
            // Enter goes to the row picked; the palette closes itself first, as a pick may open another dialog
            .on_ok(move |_, window, cx| {
                on_enter.update(cx, |palette, cx| palette.run(window, cx));
                false
            })
    });
    let query = view.read(cx).query.clone();
    window.defer(cx, move |window, cx| query.update(cx, |query, cx| query.focus(window, cx)));
}

impl Palette {
    fn new(frame: Entity<Frame>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Go to a workspace or run a command"));
        let subscriptions = vec![cx.subscribe(&query, |this: &mut Self, query, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.shown = narrow(&this.all, &query.read(cx).value());
                this.selected = 0;
                this.scroll.scroll_to_item(0);
                cx.notify();
            }
        })];
        // how the window is laid out, where it is kept: the palette offers the layouts
        let ui = cx.try_global::<crate::stores::Stores>().map(|stores| stores.ui.clone());
        let all = groups(frame.read(cx), ui.as_ref().map(|ui| ui.read(cx)));
        Palette {
            frame,
            ui,
            query,
            shown: all.clone(),
            all,
            selected: 0,
            scroll: ScrollHandle::new(),
            _subscriptions: subscriptions,
        }
    }

    fn move_by(&mut self, by: i32, cx: &mut Context<Self>) {
        let count = flat(&self.shown).len();
        self.selected = step(self.selected, by, count);
        // the list's children are each group's heading, then its rows
        let mut child = 0;
        let mut row = 0;
        'find: for group in &self.shown {
            child += 1;
            for _ in &group.rows {
                if row == self.selected {
                    break 'find;
                }
                row += 1;
                child += 1;
            }
        }
        self.scroll.scroll_to_item(child);
        cx.notify();
    }

    /// Goes to the row picked: the palette closes, then the workspace comes on screen or the action runs.
    pub fn run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pick) = flat(&self.shown).get(self.selected).map(|row| row.pick.clone()) else { return };
        window.close_dialog(cx);
        let frame = self.frame.clone();
        match pick {
            Pick::Workspace(id) => frame.update(cx, |frame, cx| {
                if frame.select_workspace(&id) {
                    cx.notify();
                }
            }),
            Pick::NextWorkspace | Pick::PreviousWorkspace => frame.update(cx, |frame, cx| {
                if frame.cycle_workspace(if pick == Pick::NextWorkspace { 1 } else { -1 }) {
                    cx.notify();
                }
            }),
            Pick::NewWorkspace => crate::new_workspace::open(frame, window, cx),
            Pick::Settings => crate::settings::open_here(frame, enkidu_model::settings::Page::General, window, cx),
            // Search on its tab, for the workspace on screen
            Pick::Search(tab) => {
                if let Some(parts) = crate::search::Parts::of_the_app(cx) {
                    crate::search::open(parts, Some(tab), window, cx);
                }
            }
            // into Claude's prompt, never sent; the changes island says what became of them
            Pick::TypeRemarks(ws) => {
                if let Some(remarks) = cx.try_global::<crate::stores::Stores>().map(|stores| stores.remarks.clone()) {
                    remarks.update(cx, |remarks, cx| remarks.send(&ws, None, cx));
                }
            }
            // Live for the workspace on screen: on, or off again
            Pick::Live { project, workspace } => crate::live::switch(frame, project, workspace, window, cx),
            Pick::AddProject => crate::rail::add_project(frame, window.window_handle(), cx),
            // the layouts, as their keys and the switch in the top row set them
            Pick::Layout(_)
            | Pick::TerminalOnly
            | Pick::FullReview
            | Pick::DiffLayout
            | Pick::Panel(_)
            | Pick::Lists => {
                let Some(ui) = self.ui.clone() else { return };
                ui.update(cx, |ui, cx| {
                    match pick {
                        Pick::Layout(layout) => ui.show(layout),
                        Pick::TerminalOnly => ui.toggle_terminal_only(),
                        Pick::FullReview => ui.toggle_full_review(),
                        Pick::Panel(panel) => ui.toggle_panel(panel),
                        Pick::Lists => ui.toggle_lists(),
                        _ => {
                            ui.diff_layout = match ui.diff_layout {
                                DiffLayout::Split => DiffLayout::Unified,
                                DiffLayout::Unified => DiffLayout::Split,
                            }
                        }
                    }
                    cx.notify();
                });
            }
        }
    }

    fn row(&self, ix: usize, row: &Row, in_front: bool, cx: &mut Context<Self>) -> AnyElement {
        let selected = ix == self.selected;
        let lead: AnyElement = match (&row.status, &row.pick) {
            (Some(status), _) => {
                lamp(SharedString::from(format!("palette-lamp-{ix}")), Status::from_core(status), 16., 0)
                    .into_any_element()
            }
            (None, pick) => {
                let (icon, color) = match pick {
                    Pick::NewWorkspace => ("icons/plus.svg", theme::ACCENT),
                    Pick::TypeRemarks(_) => ("icons/send.svg", theme::ACCENT),
                    Pick::Settings => ("icons/settings.svg", theme::INK_2),
                    Pick::Live { workspace: Some(_), .. } => ("icons/play-fill.svg", theme::LAMP_GREEN),
                    Pick::Live { workspace: None, .. } => ("icons/square-fill.svg", theme::LAMP_GREEN),
                    Pick::Search(enkidu_model::search::Tab::Files) => ("icons/search.svg", tone(250., 0.13)),
                    Pick::Search(enkidu_model::search::Tab::Symbols) => ("icons/braces.svg", tone(300., 0.13)),
                    Pick::Search(enkidu_model::search::Tab::Text) => ("icons/text-search.svg", tone(195., 0.13)),
                    Pick::NextWorkspace => ("icons/arrow-down.svg", theme::INK_2),
                    Pick::PreviousWorkspace => ("icons/arrow-up.svg", theme::INK_2),
                    Pick::Layout(Layout::Side) => ("icons/layout-side.svg", theme::INK_2),
                    Pick::Layout(Layout::Stack) => ("icons/layout-stack.svg", theme::INK_2),
                    Pick::TerminalOnly => ("icons/square-terminal.svg", theme::INK_2),
                    Pick::FullReview => ("icons/maximize-2.svg", theme::INK_2),
                    Pick::DiffLayout => ("icons/columns-2.svg", theme::INK_2),
                    Pick::Panel(Panel::Changes) => ("icons/file-diff.svg", theme::INK_2),
                    Pick::Panel(Panel::Files) => ("icons/folder-tree.svg", theme::INK_2),
                    Pick::Lists => ("icons/panel-left.svg", theme::INK_2),
                    _ => ("icons/folder-plus.svg", tone(155., 0.13)),
                };
                svg().path(icon).flex_none().size(theme::ICON).text_color(color).into_any_element()
            }
        };
        div()
            .id(("palette-row", ix))
            .test_support()
            .role(Role::ListBoxOption)
            .aria_label(SharedString::from(row.name.clone()))
            .aria_selected(selected)
            .relative()
            .h(theme::DIALOG_CONTROL)
            .flex_none()
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded(theme::RADIUS_CONTROL)
            .text_size(theme::TEXT)
            .text_color(theme::INK)
            .when(selected, |d| d.bg(theme::SELECTED))
            .cursor_pointer()
            .child(lead)
            .child(div().flex_none().max_w(px(220.)).truncate().child(row.name.clone()))
            .children(row.detail.clone().map(|detail| {
                div().min_w_0().flex_shrink(1.).truncate().text_size(px(12.)).text_color(theme::INK_3).child(detail)
            }))
            .when(row.live, |d| d.child(live_badge(("palette-live", ix), false)))
            // what is on screen (the workspace, the layout) has the blue bar at its left, as the file on screen has
            // in the lists: the fill is only where the arrow keys are (DESIGN.md, "States")
            .when(row.current, |d| {
                d.child(on_screen_bar(9., in_front).id(("palette-current", ix)).test_support().aria_label("Current"))
            })
            .child(div().flex_1())
            .children(
                row.key.map(|key| div().flex_none().pl(px(12.)).text_size(px(12.)).text_color(theme::INK_3).child(key)),
            )
            // the pointer picks a row as it moves over it; a click goes there
            .on_mouse_move(cx.listener(move |this, _, _, cx| {
                if this.selected != ix {
                    this.selected = ix;
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.selected = ix;
                this.run(window, cx);
            }))
            .into_any_element()
    }
}

impl Render for Palette {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut list: Vec<AnyElement> = Vec::new();
        let mut ix = 0;
        let in_front = window.is_window_active();
        for group in self.shown.clone() {
            list.push(
                div()
                    .px(px(8.))
                    .pt(px(8.))
                    .pb(px(4.))
                    .text_size(theme::CAPTION)
                    .line_height(px(16.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::INK_3)
                    .child(group.heading)
                    .into_any_element(),
            );
            for row in &group.rows {
                list.push(self.row(ix, row, in_front, cx));
                ix += 1;
            }
        }
        let nothing = self.shown.is_empty().then(|| {
            div()
                .id("palette-empty")
                .test_support()
                .aria_label("Nothing matches")
                .px(px(12.))
                .py(px(24.))
                .text_center()
                .text_size(theme::TEXT)
                .text_color(theme::INK_3)
                .child("Nothing matches")
        });
        div()
            .id("command-palette")
            .test_support()
            .aria_label("Command palette")
            .relative()
            .w_full()
            .flex()
            .flex_col()
            // ↑ and ↓ move through the rows before the field would move its cursor; ⌘K again closes the palette
            .capture_action(cx.listener(|this, _: &MoveUp, _, cx| {
                this.move_by(-1, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &MoveDown, _, cx| {
                this.move_by(1, cx);
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|_, _: &crate::TogglePalette, window, cx| window.close_dialog(cx)))
            .child(
                div()
                    .h(px(48.))
                    .flex_none()
                    .flex()
                    .items_center()
                    // (the field brings 8 px itself: its words stand 12 px after the sign)
                    .gap(px(4.))
                    .pl(theme::INSET)
                    .pr(px(8.))
                    .border_b_1()
                    .border_color(theme::LINE)
                    .child(svg().path("icons/search.svg").flex_none().size(theme::ICON).text_color(theme::INK_3))
                    .child(
                        div().flex_1().child(
                            enkidu_ui::pieces::bare_field(&self.query, theme::DIALOG_CONTROL).id("palette-query"),
                        ),
                    ),
            )
            .child(
                div()
                    .id("palette-list")
                    .flex()
                    .flex_col()
                    .max_h(px(420.))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .p(px(8.))
                    .children(list)
                    .children(nothing),
            )
    }
}
