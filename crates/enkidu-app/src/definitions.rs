//! The list of a name's definitions (item 10.11): when a ⌘-click finds several, a small popup lists
//! them, the nearest first, for you to pick one. ↑ ↓ and ↩, or a click, go to it; Escape closes the list. One
//! definition never opens this: the click goes straight there.

use enkidu_core::goto::{Found, Place};
use enkidu_model::search::{definitions_words, place_words};
use enkidu_ui::pieces::mono;
use enkidu_ui::tokens::{self as theme, alpha, tone};
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::dialog::DialogEntrance;
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, FocusHandle, Focusable, FontWeight, IntoElement, KeyDownEvent, Render, Role, ScrollHandle,
    SharedString, WeakEntity, Window, div, px,
};

use crate::changes_island::ChangesIsland;

/// A row of the list.
const ROW: f32 = 32.;

/// What the list is opened with: the workspace, where the click was made (the file and the line), and what the
/// core found.
pub struct Choosing {
    pub workspace: String,
    pub from: (String, usize),
    pub found: Found,
}

/// Opens the list over the window.
pub fn open(choosing: Choosing, island: WeakEntity<ChangesIsland>, window: &mut Window, cx: &mut App) {
    let view =
        cx.new(|cx| Definitions { choosing, island, active: 0, focus: cx.focus_handle(), scroll: ScrollHandle::new() });
    let shown = view.clone();
    // where Search stands (12% down the window)
    let top = px(f32::from(window.viewport_size().height) * 0.12);
    window.open_dialog(cx, move |dialog, _, _| {
        let on_enter = shown.clone();
        crate::dialog_parts::surface(dialog)
            .w(px(560.))
            .margin_top(top)
            .entrance(DialogEntrance::None)
            .child(shown.clone())
            // Enter goes to the row that is picked; the list closes itself
            .on_ok(move |_, window, cx| {
                on_enter.update(cx, |list, cx| list.pick(list.active, window, cx));
                false
            })
    });
    let focus = view.read(cx).focus.clone();
    window.defer(cx, move |window, cx| window.focus(&focus, cx));
}

pub struct Definitions {
    choosing: Choosing,
    island: WeakEntity<ChangesIsland>,
    /// The row ↩ goes to.
    active: usize,
    focus: FocusHandle,
    scroll: ScrollHandle,
}

impl Focusable for Definitions {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Definitions {
    fn places(&self) -> &[Place] {
        &self.choosing.found.places
    }

    fn move_by(&mut self, by: i32, cx: &mut Context<Self>) {
        let last = self.places().len().saturating_sub(1) as i32;
        self.active = (self.active as i32 + by).clamp(0, last) as usize;
        self.scroll.scroll_to_item(self.active);
        cx.notify();
    }

    /// Goes to a definition: the list closes, and the island opens the file at its line.
    fn pick(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(place) = self.places().get(ix).cloned() else {
            return;
        };
        window.close_dialog(cx);
        let (workspace, from) = (self.choosing.workspace.clone(), self.choosing.from.clone());
        let _ = self.island.update(cx, |island, cx| island.go_to(&workspace, place, from, cx));
    }

    fn row(&self, ix: usize, place: &Place, cx: &mut Context<Self>) -> impl IntoElement {
        let said = place_words(place);
        let active = self.active == ix;
        // the kind's own tint, as in Search's Symbols; a value has none
        let color = said.hue.map_or(theme::INK_3, |hue| tone(hue, 0.13));
        let label = match &said.within {
            Some(within) => format!("{} {within}", place.name),
            None => place.name.clone(),
        };
        div()
            .id(("definition-row", ix))
            .test_support()
            .role(Role::ListBoxOption)
            .aria_label(SharedString::from(label))
            .aria_description(SharedString::from(format!("{}, {}:{}", said.kind, place.path, place.line)))
            .aria_selected(active)
            .h(px(ROW))
            .w_full()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .px(px(8.))
            .rounded(theme::RADIUS_CONTROL)
            .cursor_pointer()
            .text_size(theme::TEXT)
            .when(active, |row| row.bg(theme::SELECTED))
            .on_mouse_move(cx.listener(move |this, _, _, cx| {
                if this.active != ix {
                    this.active = ix;
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _, window, cx| this.pick(ix, window, cx)))
            .child(
                mono(div())
                    .size(theme::BADGE)
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .rounded(theme::RADIUS_SMALL)
                    .bg(alpha(color, 0.15))
                    .text_color(color)
                    .text_size(px(10.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(said.letter),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_baseline()
                    .gap(px(8.))
                    .child(
                        mono(div())
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.5))
                            .text_color(theme::INK)
                            .child(place.name.clone()),
                    )
                    .children(said.within.map(|within| {
                        div().min_w_0().truncate().text_size(theme::TEXT_SMALL).text_color(theme::INK_3).child(within)
                    })),
            )
            // where it is: the folder, then the file's name and the line
            .child(
                div()
                    .flex_shrink(1.)
                    .min_w_0()
                    .flex()
                    .items_baseline()
                    .gap(px(8.))
                    .text_size(theme::TEXT_STATUS)
                    .children(
                        said.folder.map(|folder| div().min_w_0().truncate().text_color(theme::INK_3).child(folder)),
                    )
                    .child(div().flex_none().text_color(theme::INK_2).child(said.place)),
            )
    }
}

impl Render for Definitions {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let words = definitions_words(self.places().len(), &self.choosing.found.word);
        // at most 58% of a short window, as Search's results
        let room = (f32::from(window.viewport_size().height) * 0.58).min(420.);
        let rows: Vec<_> = (0..self.places().len())
            .map(|ix| self.row(ix, &self.places()[ix].clone(), cx).into_any_element())
            .collect();
        div()
            .id("definitions-popup")
            .test_support()
            .aria_label(SharedString::from(words.clone()))
            .track_focus(&self.focus)
            .relative()
            .w_full()
            .flex()
            .flex_col()
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| match event.keystroke.key.as_str() {
                "down" => this.move_by(1, cx),
                "up" => this.move_by(-1, cx),
                _ => {}
            }))
            .child(
                div()
                    .h(theme::BAR)
                    .flex_none()
                    .flex()
                    .items_center()
                    .px(px(16.))
                    .border_b_1()
                    .border_color(theme::LINE)
                    .text_size(theme::TEXT_SMALL)
                    .text_color(theme::INK_3)
                    .child(words),
            )
            .child(
                div()
                    .id("definitions-list")
                    .max_h(px(room))
                    .flex()
                    .flex_col()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .p(px(8.))
                    .children(rows),
            )
    }
}
