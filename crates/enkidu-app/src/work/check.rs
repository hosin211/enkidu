//! "Check connections" (DESIGN.md, "The Work page"): a dialog that reads every project's GitHub and Jira now, then asks
//! `gh` and `twg` directly and compares, field by field: one card per source and project, "matches" or each difference
//! in a line; a source that can't be read says why. Opening it starts a check; "Check again" asks again. Nothing is
//! written to either. The check runs in the core, off the drawing thread; what each card says is
//! `enkidu_model::work::check`.

use std::rc::Rc;

use enkidu_core::work::Checked;
use enkidu_model::work::check::{CHECK_NOTHING, CHECK_RUNS, CHECK_SAYS, CheckRow, check_rows};
use enkidu_ui::pieces::{ButtonKind, ButtonSize, button, mono, with_code};
use enkidu_ui::running_line::running_line;
use enkidu_ui::tokens::{self as theme, alpha};
use enkidu_ui::work::tinted;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, AsyncApp, Context, Entity, FocusHandle, FontWeight, Global, IntoElement, Render, ScrollHandle,
    SharedString, Task, WeakEntity, Window, div, px,
};

use crate::bridge::Bridge;

/// Starts a check and hands back its answer to wait for: every source's result, or why none came.
pub type Run = Rc<dyn Fn(&mut App) -> Task<Result<Vec<Checked>, String>>>;

/// Who checks in a window without the core (a screen test, a picture). The app asks the core.
pub struct CheckSource(pub Run);

impl Global for CheckSource {}

/// The dialog's width, and the most of the window's height it takes.
const WIDTH: f32 = 600.;
const TALLEST: f32 = 0.78;

pub struct Check {
    /// The last check's cards; none before the first came back.
    rows: Option<Vec<CheckRow>>,
    /// Why the check itself failed (not a source: the app).
    error: Option<String>,
    running: bool,
    /// Counts the checks asked for: an answer to an earlier one is dropped.
    turn: u64,
    scroll: ScrollHandle,
    focus: FocusHandle,
}

/// Opens the dialog and starts a check.
pub fn open(window: &mut Window, cx: &mut App) -> Entity<Check> {
    let view = cx.new(|cx| {
        let mut check = Check {
            rows: None,
            error: None,
            running: false,
            turn: 0,
            scroll: ScrollHandle::new(),
            focus: cx.focus_handle(),
        };
        check.run(cx);
        check
    });
    let shown = view.clone();
    window.open_dialog(cx, move |dialog, window, _| {
        let room = window.viewport_size();
        crate::dialog_parts::surface(dialog)
            .w(px(WIDTH.min(f32::from(room.width) - 32.)))
            .margin_top(px(f32::from(room.height) * 0.11))
            .child(shown.clone())
            // ↩ checks again; it never closes the dialog
            .on_ok({
                let view = shown.clone();
                move |_, _, cx| {
                    view.update(cx, |check, cx| check.run(cx));
                    false
                }
            })
    });
    let focus = view.read(cx).focus.clone();
    window.defer(cx, move |window, cx| focus.focus(window, cx));
    view
}

impl Check {
    /// The results on screen (a test reads them).
    pub fn rows(&self) -> Option<&[CheckRow]> {
        self.rows.as_deref()
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Checks now. While one runs, another isn't started.
    pub fn run(&mut self, cx: &mut Context<Self>) {
        if self.running {
            return;
        }
        self.running = true;
        self.rows = None;
        self.error = None;
        self.turn += 1;
        let turn = self.turn;
        cx.notify();
        // a test's or a picture's own checker, else the core's
        let given = cx.try_global::<CheckSource>().map(|source| source.0.clone());
        let answer = match given {
            Some(check) => check(cx),
            None if cx.has_global::<Bridge>() => {
                let asked = Bridge::ask_with_news(cx, |events, shared| {
                    enkidu_core::commands::work_check(events.clone(), shared).map_err(|err| err.message)
                });
                cx.background_spawn(async move { asked.await.unwrap_or_else(|_| Err("The app is closing.".into())) })
            }
            None => cx.background_spawn(async { Err("Nothing reads GitHub or Jira in this window.".to_string()) }),
        };
        cx.spawn(async move |check: WeakEntity<Self>, cx: &mut AsyncApp| {
            let answer = answer.await;
            let _ = check.update(cx, |check, cx| {
                if check.turn != turn {
                    return;
                }
                check.running = false;
                match answer {
                    Ok(results) => check.rows = Some(check_rows(&results)),
                    Err(why) => check.error = Some(why),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// One source of one project: a round mark, "GitHub for shop", its verdict; then why it can't be read, what was
    /// compared, and each difference in a line.
    fn card(&self, at: usize, row: &CheckRow) -> AnyElement {
        let (icon, color) = match row.ok {
            true => ("icons/check.svg", theme::LAMP_GREEN),
            false => ("icons/x.svg", theme::LAMP_RED),
        };
        let head = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .text_size(theme::TEXT)
            .line_height(px(18.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme::INK)
            .child(
                div()
                    .flex_none()
                    .size(px(18.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(alpha(color, 0.15))
                    .child(tinted(icon, color, 12.)),
            )
            .child(SharedString::from(row.title.clone()))
            .child(
                div()
                    .text_size(px(12.))
                    .font_weight(FontWeight::NORMAL)
                    .text_color(theme::INK_3)
                    .child(SharedString::from(row.verdict.clone())),
            );
        let error = row.error.as_ref().map(|error| {
            div().mt(px(4.)).pl(px(26.)).text_size(px(12.5)).line_height(px(18.)).child(with_code(
                error,
                theme::LAMP_RED,
                theme::LAMP_RED,
            ))
        });
        let compared = (!row.compared.is_empty()).then(|| {
            div()
                .mt(px(4.))
                .pl(px(26.))
                .flex()
                .flex_col()
                .gap(px(2.))
                .text_size(px(12.))
                .line_height(px(17.))
                .children(row.compared.iter().map(|line| div().child(with_code(line, theme::INK_3, theme::INK_2))))
        });
        let differences = (!row.differences.is_empty()).then(|| {
            div()
                .mt(px(6.))
                .pl(px(26.))
                .flex()
                .flex_col()
                .gap(px(2.))
                .text_size(px(12.))
                .line_height(px(17.))
                .text_color(theme::INK_2)
                .children(row.differences.iter().map(|differs| {
                    // "#12 title: the index has Old, GitHub says New", the two values bright
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_x(px(4.))
                        .child(mono(div()).text_size(px(11.5)).child(SharedString::from(differs.thing.clone())))
                        .child(SharedString::from(format!("{}: the index has", differs.field)))
                        .child(div().text_color(theme::INK).child(SharedString::from(format!("{},", differs.index))))
                        .child(SharedString::from(format!("{} says", row.source_name)))
                        .child(div().text_color(theme::INK).child(SharedString::from(differs.direct.clone())))
                }))
        });
        div()
            .id(("check-result", at))
            .test_support()
            .aria_label(SharedString::from(row.said()))
            .aria_description(SharedString::from(format!("{} {}", row.source, row.ok)))
            .flex_none()
            .rounded(px(10.))
            .bg(theme::HOVER)
            .px(px(12.))
            .py(px(10.))
            .child(head)
            .children(error)
            .children(compared)
            .children(differences)
            .into_any_element()
    }
}

impl Render for Check {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tallest = f32::from(window.viewport_size().height) * TALLEST;
        let head = div()
            .flex_none()
            .flex()
            .items_start()
            .justify_between()
            .gap(px(16.))
            .px(px(20.))
            .pt(px(18.))
            .pb(px(4.))
            .child(
                div()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(px(15.))
                            .line_height(px(20.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::INK)
                            .child("Check connections"),
                    )
                    .child(
                        div()
                            .id("check-says")
                            .test_support()
                            .aria_label(CHECK_SAYS)
                            .mt(px(4.))
                            .text_size(px(12.))
                            .line_height(px(16.))
                            .text_color(theme::INK_2)
                            .child(CHECK_SAYS),
                    ),
            )
            .child(crate::dialog_parts::close("check-close").on_click(|_, window, cx| window.close_dialog(cx)));
        let mut body: Vec<AnyElement> = Vec::new();
        if self.running {
            body.push(
                div()
                    .id("check-running")
                    .test_support()
                    .aria_label(CHECK_RUNS)
                    .relative()
                    .flex_none()
                    .overflow_hidden()
                    .rounded(px(10.))
                    .bg(theme::HOVER)
                    .px(px(12.))
                    .py(px(10.))
                    .text_size(px(12.5))
                    .line_height(px(18.))
                    .text_color(theme::INK_2)
                    .child(CHECK_RUNS)
                    // something runs: the line of light along its lower edge
                    .child(running_line("check-running-line").aria_label(CHECK_RUNS).bottom_0())
                    .into_any_element(),
            );
        }
        if let Some(error) = &self.error {
            body.push(
                div()
                    .id("check-error")
                    .test_support()
                    .aria_label(SharedString::from(error.clone()))
                    .flex_none()
                    .text_size(px(12.5))
                    .line_height(px(18.))
                    .text_color(theme::LAMP_RED)
                    .child(SharedString::from(error.clone()))
                    .into_any_element(),
            );
        }
        match &self.rows {
            Some(rows) if rows.is_empty() => body.push(
                div()
                    .id("check-nothing")
                    .test_support()
                    .aria_label(CHECK_NOTHING)
                    .flex_none()
                    .text_size(px(12.5))
                    .line_height(px(18.))
                    .text_color(theme::INK_2)
                    .child(CHECK_NOTHING)
                    .into_any_element(),
            ),
            Some(rows) => body.extend(rows.iter().enumerate().map(|(at, row)| self.card(at, row))),
            None => {}
        }
        let again = button(
            "check-again",
            if self.running { "Checking…" } else { "Check again" },
            ButtonKind::Primary,
            ButtonSize::Medium,
        );
        let again = match self.running {
            true => again.opacity(0.45).cursor_default(),
            false => again.on_click(cx.listener(|check, _, _, cx| check.run(cx))),
        };
        div()
            .id("check-dialog")
            .test_support()
            .aria_label("Check connections")
            .aria_description(if self.running { "checking" } else { "checked" })
            .track_focus(&self.focus)
            .relative()
            .w_full()
            .max_h(px(tallest))
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(head)
            .child(
                div()
                    .id("check-results")
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .px(px(20.))
                    .pt(px(12.))
                    .pb(px(16.))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .children(body),
            )
            .child(crate::dialog_parts::foot().child(again))
            .child(crate::dialog_parts::lit_edge())
    }
}
