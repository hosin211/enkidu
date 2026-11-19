//! The Work page's header (DESIGN.md, "The header"): the window's top row while the page is on screen. After the
//! word "Work": the board menu, Tasks and Hidden once you hid a task, Needs you with its count, then on the right the
//! filter (/), each source's lamp, Check connections and Refresh.

use enkidu_core::work::model::{Board, Snapshot};
use enkidu_model::work::flows_page::{HEADER_RUNNING, HEADER_WORDS, header_chip};
use enkidu_model::work::page::{TASKS_WORDS, board_kind, board_words, hidden_words, needs_you_words, reads_jira};
use enkidu_model::work::{Source, lamp, lamp_lines, source_health};
use enkidu_ui::lamp::LAMP_FPS;
use enkidu_ui::pieces::{MenuTrigger, changes_seen, icon_button, kbd, tick};
use enkidu_ui::running_line::running_line;
use enkidu_ui::tips::Tipped as _;
use enkidu_ui::tokens::{self as theme, alpha, hsla};
use enkidu_ui::work::source_lamp;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::prelude::*;
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, BoxShadow, Context, FontWeight, IntoElement, Role, SharedString, Toggled,
    Transformation, Window, div, percentage, point, px, svg,
};

use crate::work::page::{WorkPage, look};
use crate::work::store::Work;

/// A chip of the header: pressed in while it is on.
fn chip(id: &'static str, on: bool, words: impl Into<SharedString>) -> enkidu_ui::pieces::Pressed {
    enkidu_ui::press::pressable(div().id(id))
        .test_support()
        .role(Role::Button)
        .aria_label(words)
        .aria_toggled(if on { Toggled::True } else { Toggled::False })
        .h(theme::CONTROL)
        .flex_none()
        .flex()
        .items_center()
        .gap(px(8.))
        .rounded(theme::RADIUS_CONTROL)
        .px(px(8.))
        .text_size(theme::TEXT)
        .cursor_pointer()
        .map(|chip| match on {
            true => chip.bg(theme::SELECTED).font_weight(FontWeight::MEDIUM).text_color(theme::INK),
            false => chip.text_color(theme::INK_2).hover(|s| s.bg(theme::HOVER).text_color(theme::INK)),
        })
}

/// The line between two groups of the header.
fn rule() -> AnyElement {
    div().flex_none().h(px(16.)).w(px(1.)).bg(theme::LINE_STRONG).into_any_element()
}

/// A small red dot with a faint glow.
fn red_dot(size: f32, strength: f32) -> AnyElement {
    div()
        .flex_none()
        .size(px(size))
        .rounded_full()
        .bg(alpha(theme::LAMP_RED, strength))
        .shadow(vec![BoxShadow {
            color: hsla(alpha(theme::LAMP_RED, 0.5)),
            offset: point(px(0.), px(0.)),
            blur_radius: px(6.),
            spread_radius: px(0.),
            inset: false,
        }])
        .into_any_element()
}

/// A board of the menu: a check when it is the one picked, its name, and "sprint" or "kanban" at the right (none for
/// All tasks).
fn board_option(id: SharedString, name: SharedString, kind: Option<SharedString>, picked: bool) -> PopupMenuItem {
    PopupMenuItem::element(move |_, _| {
        div()
            .id(id.clone())
            .test_support()
            .aria_label(name.clone())
            // a board's kind is said with it: "Shop Dev, sprint"
            .aria_description(kind.clone().unwrap_or_default())
            .aria_selected(picked)
            .w_full()
            .flex()
            .items_center()
            .gap(px(8.))
            .map(enkidu_ui::pieces::in_menu)
            .text_color(theme::INK)
            .child(svg().path("icons/check.svg").flex_none().size(theme::MARK).text_color(if picked {
                theme::INK
            } else {
                alpha(theme::INK, 0.)
            }))
            .child(div().min_w_0().flex_1().truncate().child(name.clone()))
            .children(
                kind.clone()
                    .map(|kind| div().flex_none().pl(px(12.)).text_size(px(11.5)).text_color(theme::INK_3).child(kind)),
            )
    })
}

impl WorkPage {
    /// The filter's field, made when it is first drawn (a text field needs its window).
    fn filter_field(&mut self, window: &mut Window, cx: &mut Context<Self>) -> gpui_kit::Entity<InputState> {
        if let Some((field, _)) = &self.filter {
            return field.clone();
        }
        // Escape is the page's: it empties the field, then leaves it (`WorkPage::back`)
        let field = cx.new(|cx| InputState::new(window, cx).placeholder("Filter"));
        let heard = cx.subscribe_in(&field, window, |page, field, event: &InputEvent, window, cx| match event {
            InputEvent::Change => {
                let typed = field.read(cx).value().to_string();
                if typed != page.query {
                    // a filter of the board: the board is back
                    if !typed.is_empty() && page.flows_open() {
                        page.show_flows(false, window, cx);
                        // the keyboard stays in the field you are typing in
                        field.update(cx, |field, cx| field.focus(window, cx));
                    }
                    page.query = typed;
                    page.rebuild(cx);
                }
            }
            // Enter leaves the field for the board
            InputEvent::PressEnter { .. } => page.take_keyboard(window, cx),
            _ => {}
        });
        self.filter = Some((field.clone(), heard));
        field
    }

    /// The keyboard is in the filter's field.
    pub(crate) fn filtering(&self, window: &Window, cx: &gpui_kit::App) -> bool {
        use gpui_kit::Focusable as _;
        self.filter.as_ref().is_some_and(|(field, _)| field.read(cx).focus_handle(cx).is_focused(window))
    }

    /// `/`: the keyboard goes to the filter.
    pub(crate) fn focus_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let field = self.filter_field(window, cx);
        field.update(cx, |field, cx| field.focus(window, cx));
    }

    /// The board menu: All tasks, then Jira's boards, each with "sprint" or "kanban". It opens with the boards read
    /// last, and reads them again for the next time.
    fn board_menu(&self, index: &Snapshot, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let (name, words) = board_words(index.board.as_ref());
        let picked = index.board.as_ref().map(|pick| pick.board.id.clone());
        let (store, handle) = (self.work.clone(), window.window_handle());
        let page = cx.entity().downgrade();
        MenuTrigger::new("work-board-pick", SharedString::from(name.clone()))
            .tip(words)
            .h(theme::CONTROL)
            .min_w_0()
            .max_w(px(240.))
            .flex_shrink(1.)
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded(theme::RADIUS_CONTROL)
            .px(px(8.))
            .text_size(theme::TEXT)
            .text_color(theme::INK_2)
            .cursor_pointer()
            .hover(|s| s.bg(theme::HOVER).text_color(theme::INK))
            // a board of Jira's is picked: its sign is in ink then
            .child(
                svg().path("icons/square-kanban.svg").flex_none().size(theme::ICON).text_color(if picked.is_some() {
                    theme::INK
                } else {
                    theme::INK_3
                }),
            )
            .child(div().truncate().child(name))
            .child(svg().path("icons/chevron-down.svg").flex_none().size(px(12.)).text_color(theme::INK_3))
            .menu(gpui_kit::Anchor::TopLeft, move |menu, _, cx| {
                // the boards as they are now; they are read again as the menu opens, for the next time
                let (boards, loading): (Vec<Board>, bool) = {
                    let work = store.read(cx);
                    (work.snapshot.as_ref().map(|index| index.boards.clone()).unwrap_or_default(), work.boards_loading)
                };
                Work::load_boards(&store, Some(handle), cx);
                let pick = |board: Option<Board>| {
                    let (store, page) = (store.clone(), page.clone());
                    move |_: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut gpui_kit::App| {
                        // a board picked is a view of the board: it is back
                        let _ = page.update(cx, |page, cx| page.show_flows(false, window, cx));
                        Work::pick_board(&store, board.clone(), handle, cx)
                    }
                };
                let all = board_option("work-board-option:".into(), "All tasks".into(), None, picked.is_none());
                let mut menu = menu.min_w(px(260.)).item(all.on_click(pick(None)));
                if boards.is_empty() {
                    // nothing read yet: it says that they are being read (they are there the next time it opens)
                    return if loading || cx.has_global::<crate::bridge::Bridge>() {
                        menu.separator().item(enkidu_ui::pieces::menu_label("Reading Jira's boards"))
                    } else {
                        menu
                    };
                }
                menu = menu.separator().item(enkidu_ui::pieces::menu_label("Jira boards"));
                for board in boards {
                    let id = SharedString::from(format!("work-board-option:{}", board.id));
                    let kind = SharedString::from(board_kind(&board.kind).to_string());
                    let is_picked = picked.as_deref() == Some(board.id.as_str());
                    let option = board_option(id, board.name.clone().into(), Some(kind), is_picked);
                    menu = menu.item(option.on_click(pick(Some(board))));
                }
                menu
            })
            .into_any_element()
    }

    /// One source's lamp: its state at a glance, the details when pointed at.
    fn source_button(&self, source: Source, index: &Snapshot, now: i64, cx: &gpui_kit::App) -> AnyElement {
        let stands = source_health(&index.health, source);
        let state = lamp(stands.as_ref(), index.checking);
        let lines = lamp_lines(&index.health, source, &self.projects(cx), now).join("\n");
        let wrong = state == enkidu_model::work::Lamp::Error;
        div()
            .id(SharedString::from(format!("health-{}", source.key())))
            .test_support()
            .aria_label(source.name())
            .aria_description(SharedString::from(lines.clone()))
            .aria_value(state.name())
            .h(theme::CONTROL)
            .flex_none()
            .flex()
            .items_center()
            .gap(px(8.))
            .text_size(theme::TEXT_SMALL)
            .text_color(if wrong { theme::LAMP_RED } else { theme::INK_2 })
            .tipped(lines, None)
            .child(source_lamp(SharedString::from(format!("health-lamp-{}", source.key())), look(state), 8.))
            .child(source.name())
            .into_any_element()
    }

    /// The header, in the window's top row, after the word "Work". Before anything was read it has only the
    /// sources' lamps, Check connections and Refresh.
    pub(crate) fn header(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let (index, now) = {
            let work = self.work.read(cx);
            (work.snapshot.clone(), work.now())
        };
        let empty = Snapshot::default();
        let index = index.as_deref().unwrap_or(&empty);
        let first_run = enkidu_model::work::page::first_run(Some(index));
        let (board, needs_you) = (self.board.clone(), self.board.needs_you);
        let lists_hidden = self.ui.read(cx).lists_hidden;
        let mut row = div()
            .id("work-header")
            .test_support()
            .h(theme::TOP_ROW)
            .flex_none()
            .w_full()
            .flex()
            .items_center()
            .gap(px(8.))
            // the row starts after the window's traffic lights. With the lists hidden, the button that brings them
            // back stands first (a button's box stands 8 px before its sign)
            .pl(theme::AFTER_LIGHTS - px(if lists_hidden { 8. } else { 0. }))
            .pr(px(12.))
            .children(lists_hidden.then(|| crate::tabs_row::show_lists_button(self.ui.clone())))
            .child(
                div()
                    .id("work-title")
                    .test_support()
                    .aria_label("Work")
                    .flex_none()
                    .mr(px(12.))
                    .text_size(px(13.5))
                    .font_weight(theme::WEIGHT_TITLE)
                    .text_color(theme::INK)
                    .child("Work"),
            );
        // which Jira board: there once Jira is read
        if !first_run && reads_jira(index) {
            // Jira's boards are read once ahead of the menu, so its first opening has them (the menu is made as it
            // opens, and each opening reads them again for the next)
            if index.boards.is_empty() && !std::mem::replace(&mut self.boards_asked, true) {
                let store = self.work.clone();
                cx.defer(move |cx| Work::load_boards(&store, None, cx));
            }
            row = row.child(self.board_menu(index, window, cx)).child(rule());
        }
        // the board or the tasks you hid: there once you hid one
        if !first_run && board.away > 0 {
            let hidden = hidden_words(board.away, board.away_waiting);
            row = row
                .child(
                    chip("work-show-tasks", !board.show_hidden, "Tasks")
                        .tipped(TASKS_WORDS, None)
                        .child("Tasks")
                        .on_click(cx.listener(|page, _, _, cx| page.show_hidden(false, cx))),
                )
                .child(
                    chip("work-show-hidden", board.show_hidden, "Hidden")
                        .aria_description(SharedString::from(hidden.clone()))
                        .aria_value(SharedString::from(board.away.to_string()))
                        .tipped(hidden, None)
                        .child(
                            svg()
                                .path("icons/eye-off.svg")
                                .flex_none()
                                .size(theme::ICON)
                                .text_color(if board.show_hidden { theme::INK_2 } else { theme::INK_3 }),
                        )
                        .child("Hidden")
                        .child(
                            enkidu_ui::pieces::tabular(div())
                                .text_size(px(12.))
                                .text_color(theme::INK_3)
                                .child(board.away.to_string()),
                        )
                        .when(board.away_waiting > 0, |chip| {
                            chip.child(div().id("hidden-waiting").test_support().child(red_dot(6., 1.)))
                        })
                        .on_click(cx.listener(|page, _, _, cx| page.show_hidden(true, cx))),
                );
            if !board.show_hidden && board.needs_count > 0 {
                row = row.child(rule());
            }
        }
        if !first_run && board.needs_count > 0 && !board.show_hidden {
            let count = board.needs_count;
            let changes = changes_seen("work-needs-count", count as u64, window, cx);
            row = row.child(
                chip("work-needs-you", needs_you, "Needs you")
                    .aria_value(SharedString::from(count.to_string()))
                    .tipped(needs_you_words(needs_you), None)
                    .child(red_dot(7., if needs_you { 1. } else { 0.8 }))
                    .child("Needs you")
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::INK_2)
                            .child(tick("work-needs-tick", count.to_string(), changes)),
                    )
                    .on_click(cx.listener(|page, _, window, cx| page.toggle_needs_you(window, cx))),
            );
        }
        // your workflows: there once you made one (or a run of one is kept)
        if let Some(running) = header_chip(index).filter(|_| !first_run) {
            let open = self.flows_open();
            row = row.child(
                chip("work-flows", open, "Workflows")
                    .relative()
                    .aria_description(if running { "a workflow is running" } else { "" })
                    .aria_value(SharedString::from(index.flows.len().to_string()))
                    .tipped(if running { HEADER_RUNNING } else { HEADER_WORDS }, None)
                    .child(
                        svg().path("icons/calendar-clock.svg").flex_none().size(theme::ICON).text_color(if running {
                            theme::ACCENT
                        } else {
                            theme::INK_3
                        }),
                    )
                    .child("Workflows")
                    // while one runs, the line of light along the chip's lower edge
                    .when(running, |chip| {
                        chip.child(running_line("work-flows-running").aria_label("A workflow is running").bottom_0())
                    })
                    .on_click(cx.listener(move |page, _, window, cx| page.show_flows(!open, window, cx))),
            );
        }
        row = row.child(div().flex_1().min_w(px(12.)).h_full());
        // a task of your own (item 8.5): there once there is a project to make it for
        if !first_run && self.has_projects(cx) {
            row = row.child(
                chip("work-new-task", false, "New task")
                    .tipped("A task of your own, for one of your projects: something that is not in Jira", None)
                    .child(svg().path("icons/plus.svg").flex_none().size(theme::ICON).text_color(theme::INK_3))
                    .child("New task")
                    .on_click(cx.listener(|page, _, window, cx| page.new_own_task(window, cx))),
            );
        }
        // the filter: keys, titles, branches and names, in the view you are in
        if !first_run {
            let field = self.filter_field(window, cx);
            let lit = self.filtering(window, cx);
            let typed = !field.read(cx).value().is_empty();
            row = row.child(
                div()
                    .id("work-filter")
                    .test_support()
                    .h(theme::CONTROL)
                    .w(px(208.))
                    .flex_shrink(1.)
                    .flex()
                    .items_center()
                    .rounded(theme::RADIUS_CONTROL)
                    .bg(theme::WELL)
                    .border_1()
                    // the keyboard is in it: the field's blue ring
                    .border_color(if lit { theme::ACCENT } else { theme::LINE_STRONG })
                    // (the field brings 8 px itself: its words stand 8 px after the sign)
                    .px(px(8.))
                    .child(svg().path("icons/search.svg").flex_none().size(theme::ICON).text_color(theme::INK_3))
                    .child(
                        div().flex_1().min_w_0().child(enkidu_ui::pieces::bare_field(&field, px(20.)).id("work-query")),
                    )
                    .when(!typed, |filter| filter.child(kbd("/"))),
            );
        }
        row = row
            .child(self.source_button(Source::Github, index, now, cx))
            .child(self.source_button(Source::Jira, index, now, cx))
            .child(
                icon_button("work-check", "icons/shield-check.svg", "Check connections")
                    .tipped("Check connections: read GitHub and Jira, then compare with gh and twg", None)
                    .on_click(|_, window, cx| {
                        crate::work::check::open(window, cx);
                    }),
            );
        // Refresh: its arrows turn while a read runs
        let refresh = enkidu_ui::press::pressable(div().id("work-refresh"))
            .test_support()
            .role(Role::Button)
            .aria_label("Read GitHub and Jira now")
            .size(theme::CONTROL)
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(theme::RADIUS_CONTROL)
            .text_color(theme::INK_3)
            .cursor_pointer()
            .group("work-refresh")
            .hover(|s| s.bg(theme::HOVER))
            .tipped("Read GitHub and Jira now", None)
            .on_click(|_, _, cx| Work::refresh(cx));
        let arrows = svg()
            .path("icons/refresh-cw.svg")
            .size(theme::ICON)
            .text_color(theme::INK_3)
            .group_hover("work-refresh", |s| s.text_color(theme::INK));
        let refresh = match index.checking {
            true => refresh.child(arrows.with_animation(
                "work-refresh-turn",
                Animation::new(std::time::Duration::from_millis(1400)).repeat_synced().with_max_fps(LAMP_FPS),
                |arrows, t| arrows.with_transformation(Transformation::rotate(percentage(t))),
            )),
            false => refresh.child(arrows),
        };
        row.child(refresh).into_any_element()
    }
}
