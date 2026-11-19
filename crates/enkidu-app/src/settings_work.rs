//! A project's Work and Workflows settings (more of the Settings view's `impl`; DESIGN.md, "Settings → a project →
//! Workflows").
//!
//! Work: whether this project's PRs are read from GitHub and with which account, which Jira projects' tickets
//! belong to it, the statuses that mean a ticket came back to you, the checks that never count, and how often it is
//! read. Workflows: two switches, each with one line on what it does, both off until turned on; the morning summary
//! has its time and "Run now" beside it; under them the latest runs, each with its lamp, what happened or why not,
//! and its age. All of it is saved with the rest of the page, by Save. "Run now" starts the summary at once with what
//! is saved.

use enkidu_model::work::runs::{
    FAILED_CHECKS, FAILED_CHECKS_SAYS, MORNING, MORNING_SAYS, RunLamp, WORKFLOWS_SAY, morning_runs, run_rows,
};
use enkidu_ui::pieces::{ButtonKind, ButtonSize, button, switch, text_field, with_code};
use enkidu_ui::shimmer::shimmer;
use enkidu_ui::tokens as theme;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context, Div, FontWeight, IntoElement, SharedString, WeakEntity, Window, div, px};

use crate::bridge::Bridge;
use crate::settings::{ProjectFields, Settings, Source, area, field, section_with};

/// What the Work part says under its title.
const WORK_SAYS: &str = "Your PRs on GitHub and your tickets in Jira, on the Work page (⌘I). Read with gh and twg and \
                         your own logins; nothing is ever written to either.";

/// A switch with its name before it: the name on the left, the switch at the right end of the row.
pub(crate) fn switch_row(label: &'static str, control: impl IntoElement) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(16.))
        .text_size(theme::TEXT_SMALL)
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme::INK)
        .child(label)
        .child(control)
}

/// Two fields side by side, each taking half.
fn pair(left: Div, right: Div) -> Div {
    div().flex().gap(px(12.)).child(div().flex_1().min_w_0().child(left)).child(div().flex_1().min_w_0().child(right))
}

/// The line under a switch on what it does (code between backticks).
fn says(words: &'static str) -> Div {
    div().text_size(theme::TEXT_SMALL).line_height(px(16.)).child(with_code(words, theme::INK_3, theme::INK_2))
}

impl Settings {
    /// Reads a project's latest workflow runs: when its page comes on screen, and whenever the core says something
    /// about Work changed (a run started or ended).
    pub(crate) fn read_runs(&mut self, project: &str, cx: &mut Context<Self>) {
        match &self.source {
            Source::Given(given) => {
                self.runs.insert(project.to_string(), given.runs.get(project).cloned().unwrap_or_default());
            }
            Source::Core => {
                let (asked, id) = (project.to_string(), project.to_string());
                let read = Bridge::ask(cx, move |shared| enkidu_core::commands::work_workflow_runs(shared, asked));
                cx.spawn(async move |this: WeakEntity<Self>, cx: &mut gpui_kit::AsyncApp| {
                    if let Ok(Ok(runs)) = read.await {
                        let _ = this.update(cx, |this, cx| {
                            if this.runs.get(&id) != Some(&runs) {
                                this.runs.insert(id, runs);
                                cx.notify();
                            }
                        });
                    }
                })
                .detach();
            }
        }
    }

    /// "Run now": the morning summary starts at once, with what is saved. It ends on its own, and its run says how.
    fn run_now(&mut self, project: String, cx: &mut Context<Self>) {
        let running = self.runs.get(&project).is_some_and(|runs| morning_runs(runs));
        if running || self.run_asked || matches!(self.source, Source::Given(_)) {
            return;
        }
        self.run_asked = true;
        self.runs_error = None;
        cx.notify();
        let id = project.clone();
        let started = Bridge::ask_with_news(cx, move |events, shared| {
            enkidu_core::commands::work_morning_now(events.clone(), shared, id).map_err(|err| err.message)
        });
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut gpui_kit::AsyncApp| {
            let answer = started.await;
            let _ = this.update(cx, |this, cx| {
                this.run_asked = false;
                match answer {
                    Ok(Ok(())) => {}
                    Ok(Err(why)) => this.runs_error = Some(why),
                    Err(_) => this.runs_error = Some("The morning summary could not be started.".into()),
                }
                this.read_runs(&project, cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Turns one of a project's switches.
    pub(crate) fn flip(&mut self, project: &str, which: fn(&mut ProjectFields) -> &mut bool, cx: &mut Context<Self>) {
        if let Some(fields) = self.projects.get_mut(project) {
            let on = which(fields);
            *on = !*on;
            cx.notify();
        }
    }

    /// "Work": what is read from GitHub and Jira for this project.
    pub(crate) fn work_part(
        &self,
        id: &str,
        fields: &ProjectFields,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let project = id.to_string();
        let github = switch("work-github", "Read this project's PRs from GitHub", fields.work_github, window, cx)
            .on_click(cx.listener(move |this, _, _, cx| this.flip(&project, |fields| &mut fields.work_github, cx)));
        let description = div()
            .text_size(theme::TEXT_SMALL)
            .line_height(px(16.))
            .text_color(theme::INK_3)
            .child(WORK_SAYS)
            .into_any_element();
        let rows: Vec<AnyElement> = vec![
            switch_row("Read this project's PRs from GitHub", github).into_any_element(),
            pair(
                field(
                    "GitHub account",
                    Some("Empty: the first account gh is signed in with that can read the repo."),
                    text_field(&fields.work_account).id("work-github-account"),
                ),
                field(
                    "Jira project keys",
                    Some("Tickets of these Jira projects belong here. Empty: Jira isn't read."),
                    text_field(&fields.work_jira_keys).id("work-jira-keys"),
                ),
            )
            .into_any_element(),
            field(
                "Back to you",
                Some(
                    "Jira statuses that mean a ticket came back to you, comma separated. Empty: a move back along \
                     the usual way, like QA Review to In Progress.",
                ),
                text_field(&fields.work_back).id("work-back"),
            )
            .into_any_element(),
            pair(
                field(
                    "Checks that never count",
                    Some("One per line, as GitHub names them."),
                    area(&fields.work_ignored, "work-ignored", false),
                ),
                field(
                    "Read every (minutes)",
                    Some("And right after Claude pushes, and on Refresh."),
                    text_field(&fields.work_every).id("work-every"),
                ),
            )
            .into_any_element(),
        ];
        section_with("Work", description, rows, false)
    }

    /// "Workflows": the two switches, the morning summary's time and Run now, and the latest runs.
    pub(crate) fn workflows_part(
        &self,
        id: &str,
        fields: &ProjectFields,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let (for_checks, for_morning, for_run) = (id.to_string(), id.to_string(), id.to_string());
        let checks = switch("work-send-failed", FAILED_CHECKS, fields.send_failed_checks, window, cx).on_click(
            cx.listener(move |this, _, _, cx| this.flip(&for_checks, |fields| &mut fields.send_failed_checks, cx)),
        );
        let morning = switch("work-morning", MORNING, fields.morning_summary, window, cx).on_click(
            cx.listener(move |this, _, _, cx| this.flip(&for_morning, |fields| &mut fields.morning_summary, cx)),
        );
        let runs = self.runs.get(id).map(Vec::as_slice).unwrap_or_default();
        let running = morning_runs(runs) || self.run_asked;
        let run = button(
            "work-morning-run",
            if running { "Running…" } else { "Run now" },
            ButtonKind::Secondary,
            ButtonSize::Large,
        );
        let run = match running {
            true => run.opacity(0.45).cursor_default(),
            false => run.on_click(cx.listener(move |this, _, _, cx| this.run_now(for_run.clone(), cx))),
        };
        let now = match &self.source {
            Source::Given(given) => given.now,
            Source::Core => enkidu_core::db::now_ms(),
        };
        let latest = run_rows(runs, now);
        let description = div()
            .text_size(theme::TEXT_SMALL)
            .line_height(px(16.))
            .text_color(theme::INK_3)
            .child(WORKFLOWS_SAY)
            .into_any_element();
        let mut rows: Vec<AnyElement> = vec![
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(switch_row(FAILED_CHECKS, checks))
                .child(says(FAILED_CHECKS_SAYS))
                .into_any_element(),
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(switch_row(MORNING, morning))
                .child(says(MORNING_SAYS))
                .into_any_element(),
            div()
                .flex()
                .items_end()
                .gap(px(8.))
                .child(div().w(px(96.)).child(field("At", None, text_field(&fields.morning_at).id("work-morning-at"))))
                .child(run)
                .into_any_element(),
        ];
        if let Some(error) = &self.runs_error {
            rows.push(
                div()
                    .id("work-morning-error")
                    .test_support()
                    .aria_label(SharedString::from(error.clone()))
                    .text_size(theme::TEXT_SMALL)
                    .text_color(theme::LAMP_RED)
                    .child(SharedString::from(error.clone()))
                    .into_any_element(),
            );
        }
        if !latest.is_empty() {
            rows.push(
                div()
                    .id("workflow-runs")
                    .test_support()
                    .aria_label("The latest runs")
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .children(latest.iter().enumerate().map(|(n, row)| {
                        let lamp = match row.lamp {
                            RunLamp::Amber => theme::LAMP_AMBER,
                            RunLamp::Green => theme::LAMP_GREEN,
                            RunLamp::Gray => theme::INK_3,
                            RunLamp::Red => theme::LAMP_RED,
                        };
                        let words: AnyElement = match row.running {
                            // something in progress: its words shimmer, as "Claude is working" does
                            true => shimmer(("workflow-run-words", n), row.words.clone()).into_any_element(),
                            false => with_code(&row.words, theme::INK_3, theme::INK_2).into_any_element(),
                        };
                        div()
                            .id(("workflow-run", n))
                            .test_support()
                            .aria_label(SharedString::from(row.said()))
                            .aria_description(SharedString::from(format!("{} {}", row.workflow, row.state)))
                            .flex()
                            .items_start()
                            .gap(px(8.))
                            .text_size(theme::TEXT_SMALL)
                            .line_height(theme::TEXT_SMALL_LINE)
                            .child(div().flex_none().mt(px(4.)).size(px(8.)).rounded_full().bg(lamp))
                            .child(
                                div()
                                    .flex_none()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme::INK_2)
                                    .child(row.name),
                            )
                            .child(div().flex_1().min_w_0().text_color(theme::INK_3).child(words))
                            .child(
                                div().flex_none().text_color(theme::INK_3).child(SharedString::from(row.age.clone())),
                            )
                    }))
                    .into_any_element(),
            );
        }
        section_with("Workflows", description, rows, false)
    }
}
