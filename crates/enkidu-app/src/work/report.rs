//! Claude's reports in the card's Overview (more of the card's `impl`): how the task ended comes first, in a box of its
//! own: a tinted pill with the words, when and which workspace's Claude (a link to it), Claude's words as Markdown, its
//! screenshots as thumbnails. The earlier reports are a click away, quieter. What it says is
//! `enkidu_model::work::report`; the screenshots are `work::shots`.

use std::rc::Rc;

use enkidu_core::db::Workspace;
use enkidu_core::work::model::Snapshot;
use enkidu_model::work::card::Tab;
use enkidu_model::work::groups::Group;
use enkidu_model::work::markdown::blocks_of;
use enkidu_model::work::report::{LONE_SHOT, LONE_SHOT_LARGE, ReportRow, Reports, shot_columns, shot_columns_large};
use enkidu_model::work::rich::Block;
use enkidu_ui::tokens::{self as theme, alpha, bezier};
use enkidu_ui::work::tinted;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, App, Context, Entity, FontWeight, ObjectFit, Rgba, Role, SharedString,
    Window, canvas, div, img, px, rgb,
};

use crate::work::card::{Card, TASK_WIDTH, heading};
use crate::work::rich::{rich, rich_bright};
use crate::work::shots::{Shots, Small};

/// A card's reports, with each one's words read into blocks once.
pub(super) struct ReportsMade {
    pub(super) said: Reports,
    texts: Vec<Rc<Vec<Block>>>,
}

impl ReportsMade {
    pub(super) fn of(group: &Group, index: &Snapshot, workspaces: &[Workspace], now: i64) -> Option<ReportsMade> {
        let said = Reports::of(group, index, workspaces, now)?;
        let texts = said.rows.iter().map(|row| Rc::new(blocks_of(&row.text))).collect();
        Some(ReportsMade { said, texts })
    }
}

/// How a report's state looks: its icon and its color (done green, review amber, stuck red, an update blue).
pub fn report_look(state: &str) -> (&'static str, Rgba) {
    match state {
        "done" => ("icons/circle-check.svg", theme::LAMP_GREEN),
        "review" => ("icons/eye.svg", theme::LAMP_AMBER),
        "update" => ("icons/message-square-text.svg", theme::ACCENT),
        _ => ("icons/circle-alert.svg", theme::LAMP_RED),
    }
}

impl Card {
    /// Before the card is drawn: the screenshots of the reports on screen are asked for (each once).
    pub(super) fn settle_reports(&mut self, cx: &mut Context<Self>) {
        if self.tab() != Some(&Tab::Overview) {
            return;
        }
        let Some(made) = self.made.reports.clone() else { return };
        let shown = if self.all_reports { made.said.rows.len() } else { 1 };
        for row in made.said.rows.iter().take(shown).filter(|row| !row.images.is_empty()) {
            self.shots.update(cx, |shots, cx| shots.want(row.id, &row.images, cx));
        }
    }

    /// "Claude's report": how the task ended, then the earlier reports behind one click.
    pub(super) fn reports(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let made = self.made.reports.clone()?;
        let said = &made.said;
        let first = self.report(&said.rows[0], &made.texts[0], false, window, cx);
        let earlier: Option<AnyElement> = match (said.more(), self.all_reports) {
            (Some(_), true) => {
                let rows: Vec<AnyElement> = (said.rows.iter().zip(&made.texts).skip(1))
                    .map(|(row, text)| self.report(row, text, true, window, cx))
                    .collect();
                Some(
                    div()
                        .id("task-reports-earlier")
                        .test_support()
                        .aria_label("Earlier reports")
                        .mt(px(16.))
                        .pl(px(16.))
                        .border_l_1()
                        .border_color(theme::LINE)
                        .flex()
                        .flex_col()
                        .gap(px(16.))
                        .children(rows)
                        .into_any_element(),
                )
            }
            (Some(words), false) => Some(
                div()
                    .id("task-reports-more")
                    .test_support()
                    .role(Role::Button)
                    .aria_label(SharedString::from(words.clone()))
                    .mt(px(8.))
                    .flex()
                    .child(
                        div()
                            .text_size(theme::TEXT_SMALL)
                            .line_height(theme::TEXT_SMALL_LINE)
                            .text_color(theme::INK_3)
                            .cursor_pointer()
                            .hover(|s| s.text_color(theme::ACCENT))
                            .child(SharedString::from(words)),
                    )
                    .on_click(cx.listener(|card, _, _, cx| {
                        card.all_reports = true;
                        cx.notify();
                    }))
                    .into_any_element(),
            ),
            (None, _) => None,
        };
        Some(
            div()
                .id("task-reports")
                .test_support()
                .aria_label(said.heading)
                .flex_none()
                .child(heading(said.heading, said.count()))
                .child(first)
                .children(earlier)
                .into_any_element(),
        )
    }

    /// One report of the card (see [`report_view`]): its workspace's name goes to that workspace.
    fn report(
        &self,
        row: &ReportRow,
        text: &[Block],
        quiet: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let card = cx.entity().downgrade();
        let go: ToWorkspace = Rc::new(move |id, window, cx| {
            let _ = card.update(cx, |card, cx| card.go_to_workspace(id, window, cx));
        });
        report_view(row, text, Shown { quiet, large: false }, &self.shots, Some(go), window, cx)
    }
}

/// Where a press on the workspace named in a report goes.
pub(super) type ToWorkspace = Rc<dyn Fn(&str, &mut Window, &mut App)>;

/// How a report is shown: `quiet` for an earlier one or an update along the way (no box, dimmer words, its workspace
/// named but not a link), `large` on a whole page (its screenshots large).
#[derive(Clone, Copy)]
pub(super) struct Shown {
    pub quiet: bool,
    pub large: bool,
}

/// One report: how it ended, when and where, Claude's words and its screenshots: on the card, and in a task's timeline
/// on the Workflows page.
pub(super) fn report_view(
    row: &ReportRow,
    text: &[Block],
    shown: Shown,
    shots: &Entity<Shots>,
    to_workspace: Option<ToWorkspace>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let quiet = shown.quiet;
    let (icon, color) = report_look(&row.state);
    let pill = div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(4.))
        .whitespace_nowrap()
        .rounded_full()
        .px(px(8.))
        .h(theme::CHIP)
        .font_weight(FontWeight::MEDIUM)
        .text_color(color)
        .bg(alpha(color, 0.12))
        .child(tinted(icon, color, 12.))
        .child(row.words);
    let when = match &row.workspace {
        Some(_) => format!("{} by the Claude in", row.when),
        None => row.when.clone(),
    };
    let workspace = row.workspace.as_ref().map(|(id, name)| {
        let go = id.clone();
        let to_workspace = to_workspace.clone().filter(|_| !quiet);
        let named = div()
            .id(SharedString::from(format!("report-workspace:{}", row.id)))
            .test_support()
            .aria_label(SharedString::from(name.clone()))
            .min_w_0()
            .truncate()
            .text_color(theme::INK_2)
            .child(SharedString::from(name.clone()));
        match to_workspace {
            None => named,
            Some(to) => named
                .role(Role::Link)
                .cursor_pointer()
                .hover(|s| s.text_color(theme::ACCENT))
                .on_click(move |_, window, cx| to(&go, window, cx)),
        }
    });
    let head = div()
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(8.))
        .text_size(px(12.5))
        .line_height(px(18.))
        .child(pill)
        .child(
            div()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(4.))
                .text_color(theme::INK_3)
                .child(div().flex_none().whitespace_nowrap().child(SharedString::from(when)))
                .children(workspace),
        )
        // Claude's own mark on what it wrote
        .when(!quiet, |head| {
            head.child(div().ml_auto().flex_none().child(tinted("icons/sparkle-filled.svg", theme::CLAUDE, 12.)))
        });
    let words_id = SharedString::from(format!("report-text:{}", row.id));
    let words = match quiet {
        true => rich(words_id, text, window, cx),
        false => rich_bright(words_id, text, window, cx),
    };
    div()
        .id(SharedString::from(format!("task-report:{}", row.id)))
        .test_support()
        .aria_label(SharedString::from(row.said()))
        .aria_value(SharedString::from(row.state.clone()))
        .flex_none()
        .when(!quiet, |report| {
            report
                .rounded(theme::RADIUS_CARD)
                .border_1()
                .border_color(theme::LINE)
                .bg(theme::PANE)
                .px(px(16.))
                .py(px(12.))
        })
        .child(head)
        .child(div().mt(px(8.)).max_w(px(TASK_WIDTH)).child(words))
        .children(thumbs(row, shown.large, shots, window, cx))
        .into_any_element()
}

/// A report's screenshots as thumbnails, 16 to 10, each showing its picture from the top: one alone at most 460 px
/// wide, several in two columns, three in a wide window; on a whole page (`large`) one alone up to 880 px, several
/// in one column, two in a wide window. A press shows it as large as the window.
fn thumbs(row: &ReportRow, large: bool, shots: &Entity<Shots>, window: &Window, cx: &mut App) -> Option<AnyElement> {
    if row.images.is_empty() {
        return None;
    }
    let width = f32::from(window.viewport_size().width);
    let (columns, lone) = match large {
        true => (shot_columns_large(row.images.len(), width), LONE_SHOT_LARGE),
        false => (shot_columns(row.images.len(), width), LONE_SHOT),
    };
    let held = shots.clone();
    let shots = shots.read(cx);
    let cells: Vec<AnyElement> = (row.images.iter().enumerate())
        .map(|(at, name)| {
            let small = shots.small(row.id, name);
            let stands = match small {
                Some(Small::Ready { plays: true, .. }) => "shown, playing",
                Some(Small::Ready { .. }) => "shown",
                Some(Small::Failed) => "can't be shown",
                _ => "being read",
            };
            let (report, names, held) = (row.id, row.images.clone(), held.clone());
            // what kind of file it is, said in its top left corner (DESIGN.md, "Claude's report")
            let kind = match small {
                Some(Small::Ready { kind, .. }) if !kind.is_empty() => Some(*kind),
                _ => None,
            };
            // a recording that plays here is told, each time it is painted, whether it is in sight
            let sight = match small {
                Some(Small::Ready { plays: true, sight, .. }) => Some(sight.clone()),
                _ => None,
            };
            let inside: Option<AnyElement> = match small {
                Some(Small::Ready { picture, .. }) => Some(
                    img(picture.clone())
                        .size_full()
                        .rounded(px(7.))
                        .object_fit(ObjectFit::Cover)
                        // it arrives a moment after the card: it comes in, it doesn't pop
                        .with_animation(
                            SharedString::from(format!("report-image-in:{}:{name}", row.id)),
                            Animation::new(theme::DUR_QUICK).with_easing(|t| bezier(theme::EASE_OUT, t)),
                            |picture, t| picture.opacity(t),
                        )
                        .into_any_element(),
                ),
                Some(Small::Failed) => Some(
                    div()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .px(px(8.))
                        .text_size(px(11.5))
                        .text_color(theme::INK_3)
                        .child(SharedString::from(format!("Can't show {name}")))
                        .into_any_element(),
                ),
                _ => None,
            };
            enkidu_ui::press::pressable(div().id(SharedString::from(format!("report-image:{}:{name}", row.id))))
                .test_support()
                .role(Role::Button)
                .aria_label(SharedString::from(name.clone()))
                .aria_description(stands)
                .relative()
                .aspect_ratio(1.6)
                .overflow_hidden()
                .rounded(px(8.))
                .border_1()
                .border_color(theme::LINE)
                .bg(alpha(theme::PANE, 0.6))
                .cursor_pointer()
                .hover(|s| s.border_color(alpha(theme::ACCENT, 0.6)))
                .children(inside)
                .children(kind.map(|kind| {
                    div()
                        .id(SharedString::from(format!("report-image-kind:{}:{name}", row.id)))
                        .test_support()
                        .aria_label(kind)
                        .absolute()
                        .top(px(8.))
                        .left(px(8.))
                        .h(theme::CHIP)
                        .px(px(8.))
                        .flex()
                        .items_center()
                        .rounded(theme::RADIUS_TAG)
                        .bg(alpha(rgb(0x000000), 0.62))
                        .text_size(theme::CAPTION)
                        .line_height(theme::CAPTION_LINE)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(alpha(rgb(0xffffff), 0.92))
                        .child(kind)
                }))
                .children(sight.map(|sight| {
                    canvas(
                        move |bounds, window, _| sight.painted(window.content_mask().bounds.intersects(&bounds)),
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                }))
                .on_click(move |_, window, cx| {
                    held.update(cx, |shots, cx| shots.open(report, names.clone(), at, window, cx));
                })
                .into_any_element()
        })
        .collect();
    Some(
        div()
            .id(SharedString::from(format!("report-images:{}", row.id)))
            .test_support()
            .aria_label(SharedString::from(row.images.join(", ")))
            .mt(px(12.))
            .grid()
            .grid_cols(columns as u16)
            .gap(px(8.))
            .when(row.images.len() == 1, |grid| grid.max_w(px(lone)))
            .children(cells)
            .into_any_element(),
    )
}
