//! The status line (DESIGN.md, "Layout"; item 1.18): the window's bottom line says in words what the tabs and lamps say
//! in signs, for the workspace on screen: its branch and where it started, what Claude is doing, Live; then on the
//! right the changed files with their added and removed lines, and the folder, whose path a click copies.

use enkidu_model::frame::Frame;
use enkidu_model::work::page::{status_keys, status_keys_words};
use enkidu_model::work::pr::workspace_line;
use enkidu_model::work::{Source, lamp as source_state, source_health, status_words};
use enkidu_ui::lamp::{Status, lamp};
use enkidu_ui::live::live_bars;
use enkidu_ui::pieces::mono;
use enkidu_ui::shimmer::shimmer;
use enkidu_ui::tips::Tipped as _;
use enkidu_ui::tokens::{self as theme, tone};
use enkidu_ui::work::source_lamp;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, ClipboardItem, Context, Entity, FontWeight, IntoElement, Render, Role, SharedString, Window, div,
    px, svg,
};

use crate::projects_row::short_path;
use crate::tabs_row::changed_words;
use crate::toast;
use crate::work::page::WorkPage;
use crate::work::store::Work;

pub struct StatusLine {
    frame: Entity<Frame>,
    /// The Work index and its page: while the page is on screen, the line says how each source stands, and the
    /// keys that work right now. None in a window without one.
    work: Option<(Entity<Work>, Entity<WorkPage>)>,
}

impl StatusLine {
    pub fn new(frame: Entity<Frame>, work: Option<(Entity<Work>, Entity<WorkPage>)>, cx: &mut Context<Self>) -> Self {
        cx.observe(&frame, |_, _, cx| cx.notify()).detach();
        if let Some((work, page)) = &work {
            cx.observe(work, |_, _, cx| cx.notify()).detach();
            cx.observe(page, |_, _, cx| cx.notify()).detach();
        }
        Self { frame, work }
    }

    /// What the line says while the Work page is on screen: each source's last read, in words, with its lamp; and all
    /// of it as one text. None while the workspace is on screen.
    fn work_parts(&self, cx: &App) -> Option<(Vec<AnyElement>, String)> {
        let (work, page) = self.work.as_ref()?;
        if !self.frame.read(cx).work_open {
            return None;
        }
        let (card_open, flows_open) = {
            let page = page.read(cx);
            (page.card().is_some(), page.flows_open())
        };
        let work = work.read(cx);
        let now = work.now();
        let snapshot = work.snapshot.as_deref();
        let checking = snapshot.is_some_and(|work| work.checking);
        let health = snapshot.map(|work| work.health.as_slice()).unwrap_or_default();
        let mut parts: Vec<AnyElement> = Vec::new();
        let mut said: Vec<String> = Vec::new();
        for source in Source::BOTH {
            let Some(stands) = source_health(health, source) else { continue };
            let (words, wrong) = status_words(&stands, source, now);
            if !parts.is_empty() {
                parts.push(rule());
            }
            said.push(words.clone());
            parts.push(
                div()
                    .id(SharedString::from(format!("work-status-{}", source.key())))
                    .test_support()
                    .aria_label(SharedString::from(words.clone()))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .whitespace_nowrap()
                    .when(wrong, |d| d.font_weight(FontWeight::MEDIUM).text_color(theme::LAMP_RED))
                    .child(source_lamp(
                        SharedString::from(format!("work-status-lamp-{}", source.key())),
                        crate::work::page::look(source_state(Some(&stands), checking)),
                        7.,
                    ))
                    .child(words)
                    .into_any_element(),
            );
        }
        parts.push(div().flex_1().h_full().into_any_element());
        // the keys that work right now: there once something was read
        if !health.is_empty() {
            let keys = div()
                .id("work-keys")
                .test_support()
                .aria_label(SharedString::from(status_keys_words(card_open, flows_open)))
                .flex_none()
                .flex()
                .items_center()
                .gap(px(8.))
                .whitespace_nowrap()
                .children(status_keys(card_open, flows_open).iter().map(|(keys, does)| {
                    div().flex().items_center().gap(px(4.)).children(keys.iter().map(|key| key_cap(key))).child(*does)
                }));
            parts.push(keys.into_any_element());
        }
        Some((parts, said.join(" | ")))
    }
}

/// A key in the status line: a hairline box around its sign.
fn key_cap(key: &'static str) -> AnyElement {
    div()
        .flex_none()
        .rounded(px(4.))
        .border_1()
        .border_color(theme::RULE_STRONG)
        .px(px(4.))
        .text_size(px(10.5))
        .line_height(px(14.))
        .child(key)
        .into_any_element()
}

/// What Claude is doing, as the status line says it.
pub fn sentence(status: Status) -> &'static str {
    match status {
        Status::Idle => "Claude is idle",
        Status::Working => "Claude is working",
        Status::NeedsYou => "Claude needs you",
        Status::Done => "Claude is done",
    }
}

/// The hairline between the line's parts.
fn rule() -> AnyElement {
    div().flex_none().h(px(12.)).w(px(1.)).bg(theme::RULE_STRONG).into_any_element()
}

impl Render for StatusLine {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bare = div()
            .w_full()
            .h(theme::STATUS_LINE)
            .flex_none()
            .flex()
            .items_center()
            .gap(px(12.))
            .overflow_hidden()
            .pl(px(18.))
            .pr(px(12.))
            .text_size(theme::TEXT_STATUS)
            .line_height(theme::TEXT_STATUS_LINE)
            .text_color(theme::INK_3);
        // the Work page on screen: how each source stands
        if let Some((parts, said)) = self.work_parts(cx) {
            return bare
                .id("work-status-line")
                .test_support()
                .aria_label(SharedString::from(said))
                .children(parts)
                .into_any_element();
        }
        let line = bare.id("status-line").test_support();
        let frame = self.frame.read(cx);
        // no workspace on screen: the line keeps its place and says nothing
        let Some(ws) = frame.current_workspace().cloned() else {
            return line.into_any_element();
        };
        let status = Status::from_core(frame.status(&ws.id));
        let pops = frame.pops(&ws.id);
        let live = frame.is_live(&ws).then(|| frame.projects.iter().find(|p| p.id == ws.project_id)).flatten();
        let live = live.map(|p| format!("Live in {}'s dev environment", p.name));
        let changes = frame.changes(&ws.id).filter(|c| c.files > 0);
        let remarks = frame.remarks(&ws.id);
        // what waits on you on this workspace's PR: "PR #2395: checks failing, 2 remarks waiting"
        let pr = self.work.as_ref().and_then(|(work, _)| {
            let line = workspace_line(work.read(cx).snapshot.as_deref(), &ws.id, Some(&ws.session_id))?;
            Some((line.text, line.severe, line.more))
        });
        let pr = pr.map(|(text, severe, more)| {
            let color = if severe { theme::LAMP_RED } else { theme::FOCUS };
            let rest = match more {
                0 => String::new(),
                1 => ", and 1 more PR".to_string(),
                n => format!(", and {n} more PRs"),
            };
            div()
                .id("status-work")
                .test_support()
                .aria_label(SharedString::from(format!("{text}{rest}")))
                .aria_description(if severe { "blocks" } else { "waits on you" })
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(6.))
                .whitespace_nowrap()
                .font_weight(FontWeight::MEDIUM)
                .text_color(color)
                .child(svg().path("icons/git-pull-request.svg").flex_none().size(px(12.)).text_color(color))
                .child(div().min_w_0().truncate().child(text))
                .when(more > 0, |item| {
                    item.child(div().flex_none().font_weight(FontWeight::NORMAL).text_color(theme::INK_3).child(rest))
                })
        });

        let tip = match &ws.base {
            Some(base) => format!("Branch {}, started from {base}", ws.branch),
            None => format!("Branch {}", ws.branch),
        };
        let branch = div()
            .id("status-branch-item")
            .flex_none()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(6.))
            .whitespace_nowrap()
            .tipped(tip.clone(), None)
            .child(svg().path("icons/git-branch.svg").flex_none().size(px(12.)).text_color(tone(35., 0.13)))
            .child(
                mono(div())
                    .id("status-branch")
                    .test_support()
                    .aria_label(SharedString::from(ws.branch.clone()))
                    .max_w(px(260.))
                    .truncate()
                    .text_size(px(11.))
                    .text_color(theme::INK_2)
                    .child(ws.branch.clone()),
            )
            .children(ws.base.clone().map(|base| {
                let words = format!("from {base}");
                div()
                    .id("status-base")
                    .test_support()
                    .aria_label(SharedString::from(words.clone()))
                    .truncate()
                    .child(words)
            }));

        let words = sentence(status);
        let claude = div()
            .id("status-claude")
            .test_support()
            .aria_label(words)
            .flex_none()
            .flex()
            .items_center()
            .gap(px(6.))
            .whitespace_nowrap()
            .when(status == Status::NeedsYou, |d| d.font_weight(FontWeight::MEDIUM).text_color(theme::LAMP_RED))
            .child(lamp("status-lamp", status, 12., pops))
            .child(match status {
                // words for something in progress shimmer
                Status::Working => shimmer("status-shimmer", words).into_any_element(),
                _ => div().child(words).into_any_element(),
            });

        let live = live.map(|words| {
            div()
                .id("status-live")
                .test_support()
                .aria_label(SharedString::from(words.clone()))
                .flex_none()
                .flex()
                .items_center()
                .gap(px(6.))
                .whitespace_nowrap()
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::LAMP_GREEN)
                .child(live_bars("status-live-bars", 9.))
                .child(words)
        });

        let changes = changes.map(|changes| {
            let counts = [
                (changes.additions > 0).then(|| (format!("+{}", changes.additions), theme::ADD_INK)),
                (changes.deletions > 0).then(|| (format!("−{}", changes.deletions), theme::DEL_INK)),
            ];
            let said: Vec<String> = counts.iter().flatten().map(|(text, _)| text.clone()).collect();
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap(px(6.))
                .whitespace_nowrap()
                .child(
                    div()
                        .id("status-changes")
                        .test_support()
                        .aria_label(SharedString::from(changed_words(changes.files)))
                        .child(changed_words(changes.files)),
                )
                .when(!said.is_empty(), |d| {
                    d.child(
                        enkidu_ui::pieces::tabular(div())
                            .id("status-counts")
                            .test_support()
                            .aria_label(SharedString::from(said.join(" ")))
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .children(
                                counts.into_iter().flatten().map(|(text, color)| div().text_color(color).child(text)),
                            ),
                    )
                })
        });

        // the remarks you wrote and haven't sent to Claude yet, in their blue
        let unsent = (remarks > 0).then(|| {
            let words = format!("{} not sent", enkidu_model::remarks::remarks_words(remarks));
            div()
                .id("status-remarks")
                .test_support()
                .aria_label(SharedString::from(words.clone()))
                .flex_none()
                .flex()
                .items_center()
                .gap(px(6.))
                .whitespace_nowrap()
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::ACCENT)
                .child(svg().path("icons/message-square.svg").flex_none().size(px(11.)).text_color(theme::ACCENT))
                .child(words)
        });

        let place = short_path(&ws.path);
        let path = ws.path.clone();
        let copied = place.clone();
        let folder = div()
            .id("status-path")
            .test_support()
            .role(Role::Button)
            .aria_label(SharedString::from(place.clone()))
            .flex_shrink(1.)
            .min_w_0()
            .max_w(px(320.))
            .truncate()
            .px(px(4.))
            .rounded(px(4.))
            .cursor_pointer()
            .hover(|s| s.bg(theme::HOVER).text_color(theme::INK_2))
            .tipped("Copy the folder's path", None)
            .on_click(move |_, window, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(path.clone()));
                toast::note(window, "Path copied", copied.clone(), cx);
            })
            .child(place);

        line.child(branch)
            .child(rule())
            .child(claude)
            .when_some(live, |d, live| d.child(rule()).child(live))
            .when_some(pr, |d, pr| d.child(rule()).child(pr))
            .child(div().flex_1().min_w(px(16.)).h_full())
            .when_some(unsent, |d, unsent| d.child(unsent).child(rule()))
            .when_some(changes, |d, changes| d.child(changes).child(rule()))
            .child(folder)
            .into_any_element()
    }
}
