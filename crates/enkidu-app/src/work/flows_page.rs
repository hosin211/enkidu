//! The Workflows page (DESIGN.md, "Workflows: each task its own, and a page of their runs"). It takes the board's
//! place, as one list and the task (since 15 Nov). Left, the workflow or run shown: its name, which is the menu of your
//! workflows still to run and of every run kept, the newest first; how it went, and what can be done with it; then its
//! tasks as a chain, with what happened at its two ends. Right, the task picked, across the rest of the page, in the
//! order you read it: what it is, how it stands (the one colored box), what it was given (the message, the Jira
//! ticket), and what happened, in time order, with Claude's updates and its report, the screenshots large. ↑ ↓ (or
//! j k) go through the tasks; Esc goes back to the board. What it says is `enkidu_model::work::flows_page`; here it is
//! drawn.

use std::collections::HashMap;
use std::rc::Rc;

use enkidu_model::work::board::{Category, TypeKind, status_look, type_kind};
use enkidu_model::work::flows::TaskTone;
use enkidu_model::work::flows_page::{Bar, DELETE_SAYS, Detail, FlowsModel, Step, TaskDetail, TaskLine, chain_ends};
use enkidu_model::work::markdown::blocks_of;
use enkidu_model::work::rich::{Block, jira_doc};
use enkidu_ui::lamp::LAMP_FPS;
use enkidu_ui::pieces::{ButtonKind, ButtonSize, MenuTrigger, Observed, button, button_with, island, mono};
use enkidu_ui::tips::Tipped as _;
use enkidu_ui::tokens::{self as theme, alpha, white};
use enkidu_ui::work::{tinted, type_icon};
use gpui_kit::base::{ElementExt as _, TestSupportExt as _};
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::prelude::*;
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, App, Context, Div, Entity, FocusHandle, FontWeight, IntoElement, Pixels,
    Render, Rgba, Role, ScrollHandle, SharedString, Transformation, WeakEntity, Window, actions, div,
    linear_color_stop, linear_gradient, percentage, px, svg,
};

use crate::work::card::{open_link, status_pill};
use crate::work::chain::{chain_step, shown_in_chain};
use crate::work::page::WorkPage;
use crate::work::report::{Shown, report_view};
use crate::work::rich::rich;
use crate::work::shots::Shots;
use crate::work::store::Work;

/// The page's own keys work while the keyboard is on it.
pub const KEYS: &str = "WorkFlows";
/// ↑ ↓ and j k go through the tasks only while no text field has the keyboard and no screenshot is shown large.
pub const PLAIN_KEYS: &str = "WorkFlows && !Input && !WorkShot";

actions!(work_flows, [NextTask, PrevTask, LeaveFlows]);

/// How wide the left pane is (the run and its chain), and how narrow it may get in a small window.
const SIDE: (f32, f32) = (348., 290.);
/// A long message or description shows this much of its start; one only a little longer shows whole.
const FOLD: f32 = 170.;
const FOLD_SLACK: f32 = 48.;

/// A text that shows its start until you ask for the rest: whether you did, and how tall all of it is (known once it
/// was drawn).
#[derive(Default)]
struct Fold {
    open: bool,
    height: Option<Pixels>,
}

/// The ticket of the task shown, as its header and its box say it.
struct TicketMade {
    kind: TypeKind,
    status: String,
    hue: Option<f32>,
    url: String,
    /// Its description, read into blocks (none: no description, or an empty one).
    description: Option<Rc<Vec<Block>>>,
}

/// The task shown in the third pane.
struct TaskMade {
    detail: TaskDetail,
    /// Its card is read: "Open the card" opens it.
    on_board: bool,
    ticket: Option<TicketMade>,
    /// Each report's words, read into blocks once, by the report's id.
    texts: HashMap<i64, Rc<Vec<Block>>>,
}

/// What the page says, made again when the index, your pick or the task picked changed.
struct Made {
    model: FlowsModel,
    task: Option<TaskMade>,
    /// The two ends of the chain of a run that is over: when it started, and when and how it ended.
    ends: Option<(String, String)>,
    /// The store's count of its cards' builds, and the page's count of its own changes, when this was made.
    of: (u64, u64),
}

pub struct FlowsPage {
    page: WeakEntity<WorkPage>,
    work: Entity<Work>,
    /// What you picked in the name's menu (`flow:ID` or `run:ID`); none: what the page shows by itself.
    selected: Option<String>,
    /// The task picked in each workflow or run, by its key; else the one it opens on.
    task_of: HashMap<String, String>,
    made: Option<Rc<Made>>,
    /// Counts what you picked here.
    edits: u64,
    /// The task on screen (the workflow or run, and the task's card): another one starts with its folds closed, at
    /// its top, without the pictures of the one before.
    on_screen: Option<(String, String)>,
    message: Fold,
    ticket: Fold,
    /// The screenshots of the reports on screen, and the one shown as large as the window.
    shots: Entity<Shots>,
    focus: FocusHandle,
    detail_scroll: ScrollHandle,
    task_scroll: ScrollHandle,
    /// The task the keyboard just moved to is brought into view as it is drawn.
    reveal: bool,
}

/// A state's icon, in a run's list of tasks and in the task's box.
fn state_icon(state: &str) -> &'static str {
    match state {
        "running" => "icons/loader-circle.svg",
        "done" => "icons/circle-check.svg",
        "review" => "icons/eye.svg",
        "blocked" => "icons/circle-alert.svg",
        "no_report" => "icons/message-square-off.svg",
        "timed_out" => "icons/timer-off.svg",
        "asked" => "icons/message-circle-question-mark.svg",
        "cut_off" => "icons/zap-off.svg",
        "failed" => "icons/circle-x.svg",
        "stopped" => "icons/circle-stop.svg",
        _ => "icons/circle-dashed.svg",
    }
}

/// A tone's color: bright only for state, red for trouble.
fn tone_color(tone: TaskTone) -> Rgba {
    match tone {
        TaskTone::Quiet => theme::INK_3,
        TaskTone::Accent => theme::ACCENT,
        TaskTone::Green => theme::LAMP_GREEN,
        TaskTone::Amber => theme::LAMP_AMBER,
        TaskTone::Red => theme::LAMP_RED,
    }
}

/// A tone's name, as a test reads it.
fn tone_name(tone: TaskTone) -> &'static str {
    match tone {
        TaskTone::Quiet => "gray",
        TaskTone::Accent => "blue",
        TaskTone::Green => "green",
        TaskTone::Amber => "amber",
        TaskTone::Red => "red",
    }
}

/// A state's icon in its color; the one that runs turns, slowly.
fn state_mark(id: SharedString, state: &str, tone: TaskTone, size: f32) -> AnyElement {
    let icon = tinted(state_icon(state), tone_color(tone), size);
    match state == "running" {
        true => icon
            .with_animation(
                id,
                Animation::new(std::time::Duration::from_secs(2)).repeat_synced().with_max_fps(LAMP_FPS),
                |icon, t| icon.with_transformation(Transformation::rotate(percentage(t))),
            )
            .into_any_element(),
        false => icon.into_any_element(),
    }
}

/// A workflow or a run, as the menu of the page's name lists it.
#[derive(Clone)]
struct Listed {
    id: (&'static str, usize),
    said: SharedString,
    described: SharedString,
    value: SharedString,
    /// It is the one shown: it has the check.
    current: bool,
    /// Its mark: an icon in its color (a workflow still to run), else a lamp of this color (a run).
    icon: Option<&'static str>,
    color: Rgba,
    name: SharedString,
    /// A run's day and time, at the right.
    day: Option<SharedString>,
    /// How it stands, under its name.
    line: SharedString,
}

/// One line of that menu: a check when it is the one shown, its mark, its name with its day at the right, and how
/// it stands under them.
fn listed(item: Listed) -> PopupMenuItem {
    PopupMenuItem::element(move |_, _| {
        let item = item.clone();
        let mark = match item.icon {
            Some(icon) => tinted(icon, item.color, 16.).into_any_element(),
            None => div().size(px(8.)).rounded_full().bg(item.color).into_any_element(),
        };
        div()
            .id(item.id)
            .test_support()
            .aria_label(item.said)
            .aria_description(item.described)
            .aria_value(item.value)
            .aria_selected(item.current)
            .w_full()
            .flex()
            .items_start()
            .gap(px(8.))
            .py(px(4.))
            .child(svg().path("icons/check.svg").flex_none().mt(px(3.)).size(theme::MARK).text_color(if item.current {
                theme::INK
            } else {
                alpha(theme::INK, 0.)
            }))
            .child(div().flex_none().w(px(16.)).h(theme::TEXT_LINE).flex().items_center().justify_center().child(mark))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .text_size(theme::TEXT)
                            .line_height(theme::TEXT_LINE)
                            .text_color(theme::INK)
                            .child(div().min_w_0().flex_1().truncate().child(item.name))
                            .children(item.day.map(|day| {
                                enkidu_ui::pieces::tabular(div())
                                    .flex_none()
                                    .text_size(theme::TEXT_STATUS)
                                    .text_color(theme::INK_3)
                                    .child(day)
                            })),
                    )
                    .child(div().truncate().text_size(theme::TEXT_SMALL).text_color(theme::INK_3).child(item.line)),
            )
    })
}

/// A pane of the page, its content scrolling inside it.
fn pane(id: &'static str, label: impl Into<SharedString>) -> Observed {
    island(id).aria_label(label.into()).h_full().min_h_0()
}

/// The pane at the left stands on the stage like the task beside it, with a line at its right.
fn at_the_left(pane: Observed) -> Observed {
    pane.border_r_1().border_color(theme::LINE)
}

/// A box of the task's page: a title line, then what it holds.
fn boxed(id: &'static str, label: impl Into<SharedString>) -> Observed {
    div()
        .id(id)
        .test_support()
        .aria_label(label.into())
        .min_w_0()
        .flex_1()
        .rounded(theme::RADIUS_CARD)
        .border_1()
        .border_color(theme::LINE)
        .bg(theme::PANE)
        .px(px(16.))
        .py(px(12.))
}

impl FlowsPage {
    pub fn new(page: WeakEntity<WorkPage>, work: Entity<Work>, cx: &mut Context<Self>) -> Self {
        // the index changed (a task ended, a report came): what the page says is made again
        cx.observe(&work, |_, _, cx| cx.notify()).detach();
        // a screenshot shows once it is read
        let shots = cx.new(Shots::wide);
        cx.observe(&shots, |_, _, cx| cx.notify()).detach();
        FlowsPage {
            page,
            work,
            selected: None,
            task_of: HashMap::new(),
            made: None,
            edits: 0,
            on_screen: None,
            message: Fold::default(),
            ticket: Fold::default(),
            shots,
            focus: cx.focus_handle(),
            detail_scroll: ScrollHandle::new(),
            task_scroll: ScrollHandle::new(),
            reveal: false,
        }
    }

    /// What you picked in the name's menu (`flow:ID` or `run:ID`).
    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    /// The workflow or run shown, and the task shown of it (a test reads them).
    pub fn shown(&self) -> (Option<String>, Option<String>) {
        let made = self.made.as_ref();
        (
            made.and_then(|made| made.model.current.clone()),
            made.and_then(|made| made.task.as_ref()).map(|task| task.detail.subject.clone()),
        )
    }

    /// The screenshots of the reports on screen.
    pub fn shots(&self) -> &Entity<Shots> {
        &self.shots
    }

    /// Shows a workflow or a run (`flow:ID` or `run:ID`): a click in the name's menu, a workflow you just saved or
    /// started.
    pub fn select(&mut self, key: impl Into<String>, cx: &mut Context<Self>) {
        self.selected = Some(key.into());
        self.edits += 1;
        self.detail_scroll = ScrollHandle::new();
        cx.notify();
    }

    /// A task of the workflow or run shown is picked: the pane at the right shows it.
    fn pick_task(&mut self, subject: &str, cx: &mut Context<Self>) {
        let Some(current) = self.made.as_ref().and_then(|made| made.model.current.clone()) else { return };
        self.task_of.insert(current, subject.to_string());
        self.edits += 1;
        cx.notify();
    }

    /// ↓ or ↑: the task after the one shown, or before it; its line comes into view.
    fn step(&mut self, by: isize, cx: &mut Context<Self>) {
        let Some(made) = self.made.clone() else { return };
        let (Some(current), Some(detail)) = (made.model.current.as_ref(), made.model.detail.as_ref()) else { return };
        if let Some(next) = detail.step(self.task_of.get(current).map(String::as_str), by) {
            let next = next.to_string();
            self.reveal = true;
            self.pick_task(&next, cx);
        }
    }

    /// The page leaves the screen (Esc, the header's Workflows, a board filter): the board is back. What you picked
    /// in the name's menu stays; the task picked in each run doesn't.
    pub(crate) fn left(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.task_of.clear();
        self.on_screen = None;
        (self.message, self.ticket) = (Fold::default(), Fold::default());
        self.shots.update(cx, |shots, cx| shots.forget(window, cx));
        self.edits += 1;
        self.made = None;
    }

    /// Where the keyboard goes when the page takes it (it came on screen, or a card in front of it closed).
    pub fn keyboard(&self) -> FocusHandle {
        self.focus.clone()
    }

    /// Before the page is drawn: what it says is made from the index as it is now and what you picked; another task
    /// on screen starts with its folds closed, at its top; and the screenshots of its reports are asked for.
    fn settle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let of = (self.work.read(cx).revision, self.edits);
        if self.made.as_ref().is_none_or(|made| made.of != of) {
            self.made = Some(Rc::new(self.make(of, cx)));
        }
        let Some(made) = self.made.clone() else { return };
        let on_screen = (made.model.current.clone()).zip(made.task.as_ref().map(|task| task.detail.subject.clone()));
        if self.on_screen != on_screen {
            self.on_screen = on_screen;
            (self.message, self.ticket) = (Fold::default(), Fold::default());
            self.task_scroll = ScrollHandle::new();
            self.shots.update(cx, |shots, cx| shots.forget(window, cx));
        }
        if let Some(task) = &made.task {
            for (_, step) in &task.detail.steps {
                if let Step::Report(report) = step
                    && !report.images.is_empty()
                {
                    self.shots.update(cx, |shots, cx| shots.want(report.id, &report.images, cx));
                }
            }
        }
    }

    fn make(&self, of: (u64, u64), cx: &App) -> Made {
        let work = self.work.read(cx);
        let Some(index) = work.snapshot.as_deref() else {
            return Made { model: FlowsModel::default(), task: None, ends: None, of };
        };
        let now = work.now();
        let model = FlowsModel::of(index, self.selected.as_deref(), now);
        let picked = model.current.as_ref().and_then(|current| self.task_of.get(current));
        let task = model.detail.as_ref().and_then(|detail| {
            let task = detail.task(picked.map(String::as_str))?;
            let detail = TaskDetail::of(&detail.shown, task, index, work.built_from().0, now);
            let group = work.groups.iter().find(|group| group.id == task.subject);
            let ticket = group.and_then(|group| group.ticket(index)).map(|ticket| TicketMade {
                kind: type_kind(ticket.issue_type.as_deref()),
                status: ticket.status.clone(),
                hue: status_look(&ticket.status, Category::of(&ticket.category)).hue,
                url: ticket.url.clone(),
                description: (ticket.description.as_ref())
                    .map(jira_doc)
                    .filter(|blocks| !blocks.is_empty())
                    .map(Rc::new),
            });
            let texts = (detail.steps.iter())
                .filter_map(|(_, step)| match step {
                    Step::Report(report) => Some((report.id, Rc::new(blocks_of(&report.text)))),
                    _ => None,
                })
                .collect();
            Some(TaskMade { detail, on_board: group.is_some(), ticket, texts })
        });
        let ends = model.current.as_deref().and_then(|key| chain_ends(key, index, work.built_from().0));
        Made { model, task, ends, of }
    }

    /// Esc: back to the board.
    fn leave(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let page = self.page.clone();
        // after this update: the page lets this view go, and tells it so
        window.defer(cx, move |window, cx| {
            let _ = page.update(cx, |page, cx| page.show_flows(false, window, cx));
        });
    }

    /// The name of what is shown, which is the menu of your workflows (design 2: the list of runs is no pane of
    /// its own): under "Workflows" the ones still to run, a rule, then every run kept, the newest first. The one
    /// shown has the check; a long menu scrolls.
    fn switcher(&self, made: &Rc<Made>, detail: &Detail, cx: &mut Context<Self>) -> AnyElement {
        let (made, page) = (made.clone(), cx.entity().downgrade());
        MenuTrigger::new("flows-switch", SharedString::from(detail.name.clone()))
            .tip("Your workflows, and every run of them")
            .aria_description("Your workflows, and every run of them")
            .min_w_0()
            .h(theme::DIALOG_CONTROL)
            .flex()
            .items_center()
            .gap(px(4.))
            .rounded(theme::RADIUS_CONTROL)
            .px(px(8.))
            .text_size(px(16.))
            .line_height(px(22.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme::INK)
            .cursor_pointer()
            .hover(|s| s.bg(theme::HOVER))
            .child(div().min_w_0().truncate().child(detail.name.clone()))
            .child(svg().path("icons/chevron-down.svg").flex_none().size(theme::MARK).text_color(theme::INK_3))
            .menu(gpui_kit::Anchor::TopLeft, move |menu, _, _| {
                let pick = |key: &str| {
                    let (page, key) = (page.clone(), key.to_string());
                    move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut App| {
                        let _ = page.update(cx, |page, cx| page.select(key.clone(), cx));
                    }
                };
                let model = &made.model;
                let mut menu = menu
                    .min_w(px(340.))
                    .max_w(px(440.))
                    .max_h(px(480.))
                    .scrollable(true)
                    .item(enkidu_ui::pieces::menu_label("Workflows"));
                for (at, flow) in model.coming.iter().enumerate() {
                    // the one that runs is on air; the ones due have their time; the ones kept wait
                    let (icon, color) = match flow.state.as_str() {
                        "running" => ("icons/radio.svg", theme::ACCENT),
                        "scheduled" => ("icons/calendar-clock.svg", theme::INK_2),
                        _ => ("icons/calendar-clock.svg", theme::INK_3),
                    };
                    let item = Listed {
                        id: ("flow-item", at),
                        said: SharedString::from(format!("{}: {}", flow.name, flow.words)),
                        described: SharedString::from(flow.id.clone()),
                        value: SharedString::from(flow.state.clone()),
                        current: flow.current,
                        icon: Some(icon),
                        color,
                        name: SharedString::from(flow.name.clone()),
                        day: None,
                        line: SharedString::from(flow.words.clone()),
                    };
                    menu = menu.item(listed(item).on_click(pick(&flow.key)));
                }
                if !model.coming.is_empty() && !model.runs.is_empty() {
                    menu = menu.separator();
                }
                for (at, run) in model.runs.iter().enumerate() {
                    let item = Listed {
                        id: ("flow-run-item", at),
                        said: SharedString::from(format!("{}, {}: {}", run.name, run.day, run.line)),
                        described: SharedString::from(format!("{} {}", run.id, tone_name(run.tone))),
                        value: SharedString::from(run.state.clone()),
                        current: run.current,
                        icon: None,
                        color: match run.tone {
                            TaskTone::Quiet => theme::LAMP_OFF,
                            tone => tone_color(tone),
                        },
                        name: SharedString::from(run.name.clone()),
                        day: Some(SharedString::from(run.day.clone())),
                        line: SharedString::from(run.line.clone()),
                    };
                    menu = menu.item(listed(item).on_click(pick(&run.key)));
                }
                menu
            })
            .into_any_element()
    }

    /// A segment per task, in its state's color: a run at a glance.
    fn bar(bar: &Bar) -> AnyElement {
        div()
            .id("flow-bar")
            .test_support()
            .aria_label(SharedString::from(bar.label.clone()))
            .mt(px(8.))
            .h(px(4.))
            .flex()
            .gap(px(4.))
            .children(bar.segments.iter().map(|tone| {
                div().h_full().flex_1().rounded_full().bg(match tone {
                    Some(tone) => tone_color(*tone),
                    None => white(0.10),
                })
            }))
            .into_any_element()
    }

    /// An end of the chain: when it started, or when and how it ended. Nothing is behind it to show.
    fn end_of_chain(id: &'static str, words: &str, above: bool, below: bool) -> AnyElement {
        let dot = div().size(px(8.)).rounded_full().border_1().border_color(theme::INK_3).into_any_element();
        chain_step(id, dot, false, above, below)
            .aria_label(SharedString::from(words.to_string()))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .py(px(12.))
                    .text_size(theme::TEXT_SMALL)
                    .line_height(theme::TEXT_SMALL_LINE)
                    .text_color(theme::INK_3)
                    .child(words.to_string()),
            )
            .into_any_element()
    }

    /// A task in the run's chain: its state on the rail as a colored icon, then its number and key, its state in
    /// a word, its time, its title, its kind and how many notes Claude left. Picked, the page on the right shows it.
    #[allow(clippy::too_many_arguments)]
    fn line(
        &self,
        at: usize,
        line: &TaskLine,
        picked: bool,
        above: bool,
        below: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let color = tone_color(line.tone);
        let subject = line.subject.clone();
        let mark = state_mark(SharedString::from(format!("flow-line-turn:{at}")), &line.state, line.tone, 15.);
        let step = chain_step(("flow-line", at), mark, false, above, below)
            .role(Role::Button)
            .aria_label(SharedString::from(line.said(at + 1)))
            .aria_description(SharedString::from(line.kind.clone()))
            .aria_value(SharedString::from(line.state.clone()))
            .aria_selected(picked)
            .cursor_pointer();
        shown_in_chain(step, picked, window.is_window_active())
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .py(px(8.))
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .text_size(theme::TEXT_STATUS)
                            .child(
                                enkidu_ui::pieces::tabular(div())
                                    .flex_none()
                                    .text_color(theme::INK_3)
                                    .child((at + 1).to_string()),
                            )
                            .child(mono(div()).flex_none().text_color(theme::INK_2).child(line.name.clone()))
                            .child(
                                div().flex_none().font_weight(FontWeight::MEDIUM).text_color(color).child(line.short),
                            )
                            .child(div().flex_1())
                            .child(
                                enkidu_ui::pieces::tabular(div())
                                    .flex_none()
                                    .text_color(theme::INK_3)
                                    .child(line.took.clone()),
                            ),
                    )
                    .child(
                        div()
                            .line_clamp(2)
                            .text_size(theme::TEXT)
                            .line_height(theme::TEXT_LINE)
                            .text_color(theme::INK)
                            .child(line.title.clone()),
                    )
                    .child(
                        div()
                            .line_height(theme::TEXT_SMALL_LINE)
                            .truncate()
                            .text_size(theme::TEXT_STATUS)
                            .text_color(theme::INK_3)
                            .child(line.kind.clone()),
                    ),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.pick_task(&subject, cx)))
            .into_any_element()
    }

    /// The left pane: the workflow or run shown. On top, standing still: its name (the menu of the others), when,
    /// how it went, what can be done with it. Under a rule, scrolling: its tasks as a chain, with what happened at
    /// its two ends.
    fn detail(&self, made: &Rc<Made>, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(detail) = &made.model.detail else {
            return at_the_left(pane("flow-detail", "No workflow"))
                .w(px(SIDE.0))
                .min_w(px(SIDE.1))
                .flex_shrink(1.)
                .into_any_element();
        };
        let shown = made.task.as_ref().map(|task| task.detail.subject.as_str());
        let lamp = detail.lamp.map(|tone| {
            div().flex_none().size(px(8.)).rounded_full().bg(match tone {
                TaskTone::Quiet => theme::LAMP_OFF,
                tone => tone_color(tone),
            })
        });
        let buttons = self.buttons(detail, cx);
        let head = div()
            .flex_none()
            .px(px(16.))
            .pt(px(16.))
            .pb(px(16.))
            // the name hangs a little to the left, so its letters stand over the lines under it
            .child(div().ml(px(-8.)).flex().min_w_0().child(self.switcher(made, detail, cx)))
            .child(
                div()
                    .id("flow-when")
                    .test_support()
                    .aria_label(SharedString::from(detail.when.clone()))
                    .mt(px(4.))
                    .text_size(theme::TEXT_SMALL)
                    .text_color(theme::INK_3)
                    .child(detail.when.clone()),
            )
            .child(
                div()
                    .id("flow-words")
                    .test_support()
                    .aria_label(SharedString::from(detail.words.clone()))
                    .aria_value(detail.lamp.map(tone_name).unwrap_or_default())
                    .mt(px(8.))
                    .flex()
                    .items_start()
                    .gap(px(8.))
                    .text_size(theme::TEXT)
                    .line_height(theme::TEXT_LINE)
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::INK)
                    .children(lamp.map(|lamp| div().flex_none().h(theme::TEXT_LINE).flex().items_center().child(lamp)))
                    .child(div().min_w_0().child(detail.words.clone())),
            )
            .children(detail.bar.as_ref().map(Self::bar))
            .children(detail.every.clone().map(|every| {
                div()
                    .id("flow-every")
                    .test_support()
                    .aria_label(SharedString::from(every.clone()))
                    .mt(px(8.))
                    .line_clamp(3)
                    .text_size(theme::TEXT_SMALL)
                    .line_height(theme::TEXT_SMALL_LINE)
                    .text_color(theme::INK_3)
                    .child(every)
            }))
            .when(!buttons.is_empty(), |head| {
                head.child(div().mt(px(12.)).flex().flex_wrap().items_center().gap(px(8.)).children(buttons))
            });
        let (started, ended) = match &made.ends {
            Some((started, ended)) => (Some(started.as_str()), Some(ended.as_str())),
            None => (None, None),
        };
        let last = detail.lines.len().saturating_sub(1);
        let lines: Vec<AnyElement> = (detail.lines.iter().enumerate())
            .map(|(at, line)| {
                let picked = shown == Some(line.subject.as_str());
                // the rail runs from the first thing in the chain to the last: an end, or a task
                let (above, below) = (at > 0 || started.is_some(), at < last || ended.is_some());
                self.line(at, line, picked, above, below, window, cx)
            })
            .collect();
        // after ↑ or ↓ the task picked is brought into view
        if self.reveal
            && let Some(at) = detail.lines.iter().position(|line| shown == Some(line.subject.as_str()))
        {
            // the count, and the chain's start when it has one, come before the lines
            self.detail_scroll.scroll_to_item(at + 1 + usize::from(started.is_some()));
        }
        let tasks = !detail.lines.is_empty();
        at_the_left(pane("flow-detail", SharedString::from(detail.name.clone())))
            .aria_value(SharedString::from(detail.key.clone()))
            .w(px(SIDE.0))
            .min_w(px(SIDE.1))
            .flex_shrink(1.)
            .child(head)
            .child(div().flex_none().h(px(1.)).bg(theme::LINE))
            .child(
                div()
                    .id("flow-detail-list")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .p(px(8.))
                    .overflow_y_scroll()
                    .track_scroll(&self.detail_scroll)
                    .child(
                        div()
                            .id("flow-task-count")
                            .test_support()
                            .aria_label(SharedString::from(detail.count.clone()))
                            .flex_none()
                            .px(px(8.))
                            .pb(px(8.))
                            .text_size(theme::TEXT_SMALL)
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::INK_3)
                            .child(detail.count.clone()),
                    )
                    .children(started.map(|words| Self::end_of_chain("flow-started", words, false, tasks)))
                    .children(lines)
                    .children(ended.map(|words| Self::end_of_chain("flow-ended", words, tasks, false))),
            )
            .into_any_element()
    }

    /// What can be done with the workflow or run shown: run it (again), change it, stop it, delete it.
    fn buttons(&self, detail: &Detail, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut buttons: Vec<AnyElement> = Vec::new();
        let small = |id: &'static str, icon: Option<&'static str>, words: &'static str, kind: ButtonKind| {
            button_with(id, icon, words, None, kind, ButtonSize::Small)
        };
        let run = |id: &'static str, words: &'static str, flow: &str, cx: &mut Context<Self>| {
            let flow = flow.to_string();
            small(id, Some("icons/play.svg"), words, ButtonKind::Secondary)
                .on_click(cx.listener(move |this, _, window, cx| {
                    // the page follows the workflow that starts
                    this.select(format!("flow:{flow}"), cx);
                    Work::run_flow_now(&this.work, &flow, window.window_handle(), cx);
                }))
                .into_any_element()
        };
        if let Some(flow) = &detail.buttons.run_again {
            buttons.push(run("flow-run-again", "Run it again", flow, cx));
        }
        if let Some(flow) = &detail.buttons.run_now {
            buttons.push(run("flow-run", "Run now", flow, cx));
        }
        if let Some(flow) = detail.buttons.change.clone() {
            buttons.push(
                small("flow-edit", None, "Change", ButtonKind::Ghost)
                    .on_click(cx.listener(move |this, _, window, cx| this.change(&flow, window, cx)))
                    .into_any_element(),
            );
        }
        if let Some((flow, says)) = detail.buttons.stop.clone() {
            buttons.push(
                small("flow-stop", Some("icons/square.svg"), "Stop", ButtonKind::Ghost)
                    .tipped(says, None)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        Work::stop_flow(&this.work, &flow, window.window_handle(), cx)
                    }))
                    .into_any_element(),
            );
        }
        if let Some(flow) = detail.buttons.delete.clone() {
            buttons.push(
                div()
                    .id("flow-delete")
                    .test_support()
                    .role(Role::Button)
                    .aria_label("Delete")
                    .flex_none()
                    .size(theme::CONTROL)
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::HOVER))
                    .active(|s| s.bg(theme::SOFT))
                    .tipped(DELETE_SAYS, None)
                    .child(tinted("icons/trash.svg", theme::INK_2, 12.))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        Work::delete_flow(&this.work, &flow, window.window_handle(), cx)
                    }))
                    .into_any_element(),
            );
        }
        buttons
    }

    /// Change: the workflow opens in its dialog, as you have it now.
    fn change(&mut self, flow: &str, window: &mut Window, cx: &mut Context<Self>) {
        let found = (self.work.read(cx).snapshot.as_ref())
            .and_then(|index| index.flows.iter().find(|have| have.id == flow).cloned());
        let (Some(flow), page) = (found, self.page.clone()) else { return };
        window.defer(cx, move |window, cx| {
            let _ = page.update(cx, |page, cx| page.edit_flow(flow, Vec::new(), window, cx));
        });
    }

    /// A text that shows its start, fading out, with words under it for the rest; a short one shows whole.
    fn folded(
        &self,
        which: fn(&mut FlowsPage) -> &mut Fold,
        now: &Fold,
        id: &'static str,
        more: &'static str,
        content: AnyElement,
        cx: &mut Context<Self>,
    ) -> Div {
        // until it was drawn once its height isn't known: it starts as a long one does, without the words under it
        let long = now.height.is_some_and(|height| height > px(FOLD + FOLD_SLACK));
        let folded = !now.open && (long || now.height.is_none());
        let page = cx.entity().downgrade();
        let whole = div().relative().flex_none().child(content).on_prepaint(move |bounds, _, cx| {
            let _ = page.update(cx, |page, cx| {
                let fold = which(page);
                if fold.height != Some(bounds.size.height) {
                    fold.height = Some(bounds.size.height);
                    cx.notify();
                }
            });
        });
        let fade = (folded && long).then(|| {
            div().absolute().left_0().right_0().bottom_0().h(px(56.)).bg(linear_gradient(
                180.,
                linear_color_stop(alpha(theme::PANE, 0.), 0.),
                linear_color_stop(theme::PANE, 1.),
            ))
        });
        let words = if now.open { "Show less" } else { more };
        let toggle = long.then(|| {
            div().mt(px(4.)).flex().child(
                div()
                    .id(id)
                    .test_support()
                    .role(Role::Button)
                    .aria_label(words)
                    .text_size(theme::TEXT_SMALL)
                    .line_height(theme::TEXT_SMALL_LINE)
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::INK_3)
                    .cursor_pointer()
                    .hover(|s| s.text_color(theme::ACCENT))
                    .child(words)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let fold = which(this);
                        fold.open = !fold.open;
                        cx.notify();
                    })),
            )
        });
        div()
            .child(
                div()
                    .relative()
                    .when(folded, |text| text.max_h(px(FOLD)).overflow_hidden())
                    .child(whole)
                    .children(fade),
            )
            .children(toggle)
    }

    /// What happened to the task, in time order: the time on the left, a dot in the step's color, then the step:
    /// the message sent, Claude's updates, a report asked for, its report with the screenshots large, or how it
    /// ended when no report came.
    fn happened(&self, task: &TaskMade, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let steps = &task.detail.steps;
        if steps.is_empty() {
            return None;
        }
        let last = steps.len() - 1;
        let rows: Vec<AnyElement> = (steps.iter().enumerate())
            .map(|(at, (clock, step))| {
                let what: AnyElement = match step {
                    Step::Report(report) => {
                        let text = task.texts.get(&report.id).cloned().unwrap_or_default();
                        let shown = Shown { quiet: report.state == "update", large: true };
                        report_view(report, &text, shown, &self.shots, None, window, cx)
                    }
                    Step::Sent(text) | Step::Ended(text) => div()
                        .pt(px(1.))
                        .text_size(px(12.5))
                        .line_height(theme::TEXT_LINE)
                        .text_color(theme::INK_2)
                        .child(text.clone())
                        .into_any_element(),
                    Step::Asked(text) => div()
                        .pt(px(1.))
                        .text_size(px(12.5))
                        .line_height(theme::TEXT_LINE)
                        .text_color(theme::LAMP_AMBER)
                        .child(*text)
                        .into_any_element(),
                };
                let said = match step {
                    Step::Report(report) => report.said(),
                    Step::Sent(text) | Step::Ended(text) => text.clone(),
                    Step::Asked(text) => text.to_string(),
                };
                div()
                    .id(("run-task-step", at))
                    .test_support()
                    .aria_label(SharedString::from(said))
                    .aria_description(SharedString::from(clock.clone()))
                    .aria_value(SharedString::from(step.kind().to_string()))
                    .flex()
                    .gap(px(8.))
                    .child(
                        enkidu_ui::pieces::tabular(div())
                            .flex_none()
                            .w(px(44.))
                            .line_height(theme::TEXT_LINE)
                            .text_size(theme::TEXT_STATUS)
                            .text_color(theme::INK_3)
                            .flex()
                            .justify_end()
                            .child(clock.clone()),
                    )
                    .child(
                        div()
                            .relative()
                            .flex_none()
                            .w(px(16.))
                            .flex()
                            .justify_center()
                            // the line down to the next step, under the dot
                            .when(at < last, |column| {
                                column.child(div().absolute().top(px(8.)).bottom_0().w(px(1.)).bg(theme::LINE_STRONG))
                            })
                            .child(
                                div()
                                    .relative()
                                    .flex_none()
                                    .mt(px(1.))
                                    .size(px(16.))
                                    .rounded_full()
                                    .border_4()
                                    .border_color(theme::WELL)
                                    .bg(tone_color(step.tone())),
                            ),
                    )
                    .child(div().min_w_0().flex_1().pb(px(16.)).child(what))
                    .into_any_element()
            })
            .collect();
        Some(
            div()
                .id("run-task-happened")
                .test_support()
                .aria_label("What happened")
                .flex_none()
                .mt(px(24.))
                .child(
                    div()
                        .text_size(theme::TEXT)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::INK)
                        .child("What happened"),
                )
                .child(div().mt(px(12.)).flex().flex_col().children(rows))
                .into_any_element(),
        )
    }

    /// The pane at the right: the task picked, across the rest of the page.
    fn task(&self, made: &Made, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let empty = || pane("flow-task-pane", "No task").flex_1().min_w_0().into_any_element();
        let (Some(task), Some(_)) = (&made.task, &made.model.detail) else { return empty() };
        let detail = &task.detail;
        let color = tone_color(detail.tone);
        let subject = detail.subject.clone();
        // what it is: its ticket's type, its key, where the ticket stands, its place in the run; and where to go
        let card = task.on_board.then(|| {
            button("run-task-card", "Open the card", ButtonKind::Ghost, ButtonSize::Small).on_click(cx.listener(
                move |this, _, window, cx| {
                    let (page, card) = (this.page.clone(), subject.clone());
                    window.defer(cx, move |window, cx| {
                        let _ = page.update(cx, |page, cx| page.open(&card, window, cx));
                    });
                },
            ))
        });
        let workspace = detail.workspace.clone().map(|(id, name)| {
            button_with(
                "run-task-workspace",
                Some("icons/external-link.svg"),
                format!("Open {name}"),
                None,
                ButtonKind::Ghost,
                ButtonSize::Small,
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                let (page, workspace) = (this.page.clone(), id.clone());
                window.defer(cx, move |window, cx| {
                    let _ = page.update(cx, |page, cx| page.left_flows_for_workspace(&workspace, window, cx));
                });
            }))
        });
        let head = div()
            .min_w_0()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_x(px(8.))
            .gap_y(px(4.))
            .text_size(theme::TEXT_SMALL)
            .text_color(theme::INK_3)
            .children(task.ticket.as_ref().map(|ticket| type_icon(ticket.kind.name(), 16.)))
            .child(mono(div()).flex_none().whitespace_nowrap().text_color(theme::INK_2).child(detail.name.clone()))
            .children(task.ticket.as_ref().map(|ticket| status_pill(&ticket.status, ticket.hue, false)))
            .child(
                div()
                    .id("run-task-place")
                    .test_support()
                    .aria_label(SharedString::from(detail.place.clone()))
                    .flex_none()
                    .whitespace_nowrap()
                    .child(detail.place.clone()),
            )
            .child(div().flex_1())
            .children(card)
            .children(workspace);
        let title = div()
            .id("run-task-title")
            .test_support()
            .aria_label(SharedString::from(detail.title.clone()))
            .mt(px(8.))
            .text_size(px(20.))
            .line_height(px(28.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme::INK)
            .child(detail.title.clone());
        let facts = div()
            .id("run-task-facts")
            .test_support()
            .aria_label(SharedString::from(detail.facts.clone()))
            .mt(px(4.))
            .text_size(px(12.5))
            .text_color(theme::INK_3)
            .child(detail.facts.clone());
        // the one colored box: how it stands, and the line that matters most
        let outcome = div()
            .id("run-task-outcome")
            .test_support()
            .aria_label(SharedString::from(detail.outcome_said()))
            .aria_value(SharedString::from(detail.state.clone()))
            .mt(px(16.))
            .flex()
            .items_start()
            .gap(px(8.))
            .rounded(theme::RADIUS_CARD)
            .px(px(16.))
            .py(px(12.))
            .bg(match detail.tone {
                TaskTone::Quiet => theme::PANE,
                _ => alpha(color, 0.10),
            })
            .child(div().flex_none().mt(px(2.)).child(state_mark(
                "run-task-turn".into(),
                &detail.state,
                detail.tone,
                16.,
            )))
            .child(
                div()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(px(14.))
                            .line_height(px(20.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(color)
                            .child(detail.outcome.clone()),
                    )
                    .children(detail.outcome_line.clone().map(|line| {
                        div().text_size(px(12.5)).line_height(theme::TEXT_LINE).text_color(theme::INK_2).child(line)
                    }))
                    .children(detail.said.clone().map(|said| {
                        div()
                            .mt(px(4.))
                            .max_w(px(720.))
                            .line_clamp(2)
                            .text_size(theme::TEXT)
                            .line_height(px(19.))
                            .text_color(theme::INK)
                            .child(format!("“{said}”"))
                    })),
            );
        // what it was given: the message, and the Jira ticket beside it when there is room
        let title_line = |words: &'static str| {
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .text_size(px(12.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::INK)
                .child(words)
        };
        let sent = mono(div())
            .mt(px(8.))
            .text_size(px(11.5))
            .line_height(theme::TEXT_SMALL_LINE)
            .text_color(theme::INK_2)
            .child(detail.message.clone())
            .into_any_element();
        let message = boxed("run-task-message", detail.message_heading)
            .aria_description(SharedString::from(detail.message_at.clone().unwrap_or_default()))
            .aria_value(SharedString::from(detail.message.clone()))
            .child(title_line(detail.message_heading).children(detail.message_at.clone().map(|at| {
                enkidu_ui::pieces::tabular(div()).font_weight(FontWeight::NORMAL).text_color(theme::INK_3).child(at)
            })))
            .child(self.folded(
                |page| &mut page.message,
                &self.message,
                "run-task-message-more",
                "Show the whole message",
                sent,
                cx,
            ));
        let described = task.ticket.as_ref().and_then(|ticket| Some((ticket, ticket.description.clone()?)));
        let ticket = described.map(|(ticket, blocks)| {
            let url = ticket.url.clone();
            let link = (!url.is_empty()).then(|| open_link("run-task-jira", "Open in Jira", url));
            let doc = div().mt(px(8.)).child(rich("run-ticket-doc", &blocks, window, cx)).into_any_element();
            boxed("run-task-ticket", "The Jira ticket").child(title_line("The Jira ticket").children(link)).child(
                self.folded(
                    |page| &mut page.ticket,
                    &self.ticket,
                    "run-task-ticket-more",
                    "Show the whole description",
                    doc,
                    cx,
                ),
            )
        });
        // side by side when the pane is wide enough for two, else one under the other
        let room = f32::from(window.viewport_size().width) - SIDE.0;
        let given = div()
            .mt(px(16.))
            .flex()
            .items_start()
            .gap(px(12.))
            .when(room < 880., |given| given.flex_col().items_stretch())
            .child(message)
            .children(ticket);
        let happened = self.happened(task, window, cx);
        pane("flow-task-pane", "The task")
            .flex_1()
            .min_w_0()
            .child(
                div()
                    .id("flow-task-scroll")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .px(px(32.))
                    .py(px(24.))
                    .overflow_y_scroll()
                    .track_scroll(&self.task_scroll)
                    .child(
                        div()
                            .id("run-task")
                            .test_support()
                            .aria_label(SharedString::from(format!("{}: {}", detail.name, detail.title)))
                            .aria_description(SharedString::from(detail.state.clone()))
                            .aria_value(SharedString::from(detail.subject.clone()))
                            .child(head)
                            .child(title)
                            .child(facts)
                            .child(outcome)
                            .child(given)
                            .children(happened),
                    ),
            )
            .into_any_element()
    }
}

impl Render for FlowsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.settle(window, cx);
        let root = div()
            .id("flows-page")
            .test_support()
            .aria_label("Workflows")
            .key_context(KEYS)
            .track_focus(&self.focus)
            // ⌘C on words selected in it (a ticket's text, a report)
            .on_action(|_: &gpui_kit::base::input::Copy, window, cx| {
                if !crate::work::rich::copy_selection(window, cx) {
                    cx.propagate();
                }
            })
            .on_action(cx.listener(|this, _: &NextTask, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &PrevTask, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &LeaveFlows, window, cx| this.leave(window, cx)))
            .size_full()
            .flex();
        let Some(made) = self.made.clone() else { return root };
        let detail = self.detail(&made, window, cx);
        let task = self.task(&made, window, cx);
        self.reveal = false;
        root.child(detail)
            .child(task)
            // a screenshot shown as large as the window lies over the page (out of the row: it is no third pane)
            .child(div().absolute().child(self.shots.clone()))
    }
}
