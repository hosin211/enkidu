//! The workflow dialog (DESIGN.md, "A Jira board … workflows" and "Workflows: each task its own, and a page of their
//! runs"): the cards you picked as the tasks of a new workflow, or added to one you have; or a workflow you have, to
//! change it.
//!
//! Since 15 Nov it is a chain with the step you picked beside it. Left: "Put
//! them in" (when you have workflows they
//! can go into) and the name, then the chain: how it starts, each task in two lines with the time it starts at the
//! latest, how it ends, and New task. Right, the step picked: the start (when it starts, how long a task gets,
//! whether Live follows the task that runs, "For every task") or a task ("Do", its minutes, "Runs in", its own
//! words, what waits on it, its whole message, yours to read and change). Its foot says the plan in one line, or
//! what is still missing, and holds Cancel, Run now, and the white button that starts it at its time or keeps it.
//! What it does is as it was: only where things stand changed.
//!
//! What it decides is `enkidu_model::work::flow_editor`; here it is drawn. Its fields follow that model both ways: what
//! you type goes into the model at once, and what the model changes by itself (another kind's words, a message made
//! again, a workflow you put the cards into) goes into the field. Only what the dialog put into a field is ever
//! put there again, so a letter you just typed is never taken back.

use std::collections::HashMap;
use std::rc::Rc;

use enkidu_core::work::flows::{Flow, FlowTask};
use enkidu_model::work::flow_editor::{
    EVERY_ASKS, EVERY_HINT, Editor, Ends, KIND_KEPT, LIVE, LIVE_SAYS, START_SAYS, START_TITLE, Step, TaskRow, UNLIKE,
    World, saved_toast,
};
use enkidu_model::work::flows::KIND_DEFS;
use enkidu_ui::pieces::{
    ButtonKind, ButtonSize, MenuFrom, MenuTrigger, Part, Pressed, button, button_with, mono, sentence, switch,
    switch_live, text_area, text_field,
};
use enkidu_ui::tips::Tipped as _;
use enkidu_ui::tokens::{self as theme, alpha};
use enkidu_ui::work::tinted;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input::{InputEvent, InputState, Position, TextareaState};
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, AsyncWindowContext, Context, Div, ElementId, Entity, FocusHandle, FontWeight, IntoElement, Render,
    Role, ScrollHandle, SharedString, Subscription, WeakEntity, Window, div, px, svg,
};

use crate::toast;
use crate::work::chain::{chain_step, chain_words, shown_in_chain};
use crate::work::drafts::Drafts;
use crate::work::page::WorkPage;
use crate::work::store::Work;

/// The dialog's width, its height, and the most of a smaller window's height it takes. At 820 px its chain holds
/// eight tasks before it scrolls; at 680 it held five, the number picked the day another dialog was asked for.
const WIDTH: f32 = 1040.;
const HEIGHT: f32 = 820.;
const TALLEST: f32 = 0.78;
/// The chain's width, at the dialog's left.
const CHAIN: f32 = 360.;
/// How wide the names of the picked step's rows are ("What Claude is told" on one line).
const LABEL: f32 = 124.;

/// What the dialog says, made again when the workflow in it or the index changed.
struct Made {
    rows: Vec<TaskRow>,
    problem: Option<String>,
    title: String,
    says: Option<String>,
    save: String,
    starts: String,
    /// The plan in one line, for the foot.
    plan: Option<String>,
    /// The chain's start and end.
    ends: Ends,
    /// "Put them in": each choice's workflow (empty: a new one) and its words; one choice alone isn't shown.
    into: Vec<(String, String)>,
    /// The store's count of its cards' builds and the dialog's count of its changes, when this was made.
    of: (u64, u64),
}

/// One task's fields, with what the dialog itself last put into each.
struct RowFields {
    words: Entity<TextareaState>,
    put_words: String,
    put_asks: &'static str,
    minutes: Entity<InputState>,
    put_minutes: Option<u32>,
    /// The workflow's minutes, shown while the task has none of its own.
    put_usual: u32,
    /// The whole message's field, while it is shown.
    message: Option<(Entity<TextareaState>, Subscription)>,
    put_message: String,
    open: bool,
    _heard: [Subscription; 2],
}

pub struct FlowEditor {
    page: WeakEntity<WorkPage>,
    work: Entity<Work>,
    drafts: Entity<Drafts>,
    editor: Editor,
    made: Option<Rc<Made>>,
    /// Counts the changes of `editor`.
    edits: u64,
    name: Entity<InputState>,
    every: Entity<TextareaState>,
    time: Entity<InputState>,
    minutes: Entity<InputState>,
    put_name: String,
    put_every: String,
    put_time: String,
    put_minutes: u32,
    /// Each task's fields, by its card.
    rows: HashMap<String, RowFields>,
    /// It is being saved.
    busy: bool,
    chain_scroll: ScrollHandle,
    /// The picked step's own scroll: another step starts at its top.
    step_scroll: ScrollHandle,
    focus: FocusHandle,
    /// The dialog is closing (or closed): nothing closes it twice.
    left: bool,
    _heard: Vec<Subscription>,
}

/// Only digits, three at most: a number of minutes.
fn minutes_only(text: &str, _: &mut App) -> bool {
    text.len() <= 3 && text.bytes().all(|b| b.is_ascii_digit())
}

/// Only what a time of day is written with.
fn clock_only(text: &str, _: &mut App) -> bool {
    text.len() <= 5 && text.bytes().all(|b| b.is_ascii_digit() || b == b':')
}

/// Opens the dialog on `flow`: a new one made of the cards in `picks`, or one you have (no picks), the keyboard in
/// its name.
pub fn open(
    page: &Entity<WorkPage>,
    work: Entity<Work>,
    drafts: Entity<Drafts>,
    flow: Flow,
    picks: Vec<String>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<FlowEditor> {
    let page = page.downgrade();
    // a workflow without a start time yet is offered the one you set for new workflows (Settings, Workflows)
    let usual = work.read(cx).snapshot.as_ref().map(|index| index.flow_defaults.time.clone()).unwrap_or_default();
    let view = cx.new(|cx| FlowEditor::new(page, work, drafts, Editor::of_at(flow, picks, &usual), window, cx));
    let shown = view.clone();
    window.open_dialog(cx, move |dialog, window, _| {
        let room = window.viewport_size();
        let on_close = shown.clone();
        crate::dialog_parts::surface(dialog)
            .w(px(WIDTH.min(f32::from(room.width) - 32.)))
            // a ninth of the window down
            .margin_top(px(f32::from(room.height) * 0.11))
            .child(shown.clone())
            // ↩ is a field's own key here (a new line in a task's words), never the dialog's OK
            .on_ok(|_, _, _| false)
            .on_close(move |_, _, cx| on_close.update(cx, |editor, cx| editor.closed(cx)))
    });
    let name = view.read(cx).name.clone();
    window.defer(cx, move |window, cx| {
        name.update(cx, |field, cx| {
            // after the name that is there (this gives it the keyboard too): what you type goes on from it
            let end = Position { line: 0, character: field.value().encode_utf16().count() as u32 };
            field.set_cursor_position(end, window, cx);
        })
    });
    view
}

impl FlowEditor {
    fn new(
        page: WeakEntity<WorkPage>,
        work: Entity<Work>,
        drafts: Entity<Drafts>,
        editor: Editor,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let draft = &editor.draft;
        let (put_name, put_every) = (draft.name.clone(), draft.instruction.clone());
        let (put_time, put_minutes) = (editor.time.clone(), draft.minutes);
        let name = cx.new(|cx| InputState::new(window, cx).default_value(put_name.clone()));
        let every = cx.new(|cx| {
            TextareaState::new(window, cx).auto_grow(2, 6).placeholder(EVERY_ASKS).default_value(put_every.clone())
        });
        let time = cx.new(|cx| InputState::new(window, cx).validate(clock_only).default_value(put_time.clone()));
        let minutes =
            cx.new(|cx| InputState::new(window, cx).validate(minutes_only).default_value(put_minutes.to_string()));
        let heard = vec![
            cx.subscribe_in(&name, window, |this, field, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    let value = field.read(cx).value().to_string();
                    if value != this.editor.draft.name {
                        this.put_name = value.clone();
                        this.change(cx, |editor, world| editor.change(world, |flow| flow.name = value));
                    }
                }
            }),
            cx.subscribe_in(&every, window, |this, field, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    let value = field.read(cx).value().to_string();
                    if value != this.editor.draft.instruction {
                        this.put_every = value.clone();
                        this.change(cx, |editor, world| editor.change(world, |flow| flow.instruction = value));
                    }
                }
            }),
            cx.subscribe_in(&time, window, |this, field, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    let value = field.read(cx).value().to_string();
                    if value != this.editor.time {
                        this.put_time = value.clone();
                        this.change(cx, |editor, _| editor.time = value);
                    }
                }
            }),
            cx.subscribe_in(&minutes, window, |this, field, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    // nothing typed is no minutes at all: the foot says what a task gets
                    let value: u32 = field.read(cx).value().trim().parse().unwrap_or(0);
                    if value != this.editor.draft.minutes {
                        this.put_minutes = value;
                        this.change(cx, |editor, world| editor.change(world, |flow| flow.minutes = value));
                    }
                }
            }),
            // the index changed under it (a card's items, a workspace): what it says is made again
            cx.observe(&work, |_, _, cx| cx.notify()),
            // what had the keyboard is gone (a menu that closed, a task that was taken out): the dialog itself
            // takes it, so Esc goes on working
            cx.on_focus_lost(window, |this, window, cx| {
                if !this.left {
                    this.focus.focus(window, cx);
                }
            }),
        ];
        FlowEditor {
            page,
            work,
            drafts,
            editor,
            made: None,
            edits: 0,
            name,
            every,
            time,
            minutes,
            put_name,
            put_every,
            put_time,
            put_minutes,
            rows: HashMap::new(),
            busy: false,
            chain_scroll: ScrollHandle::new(),
            step_scroll: ScrollHandle::new(),
            focus: cx.focus_handle(),
            left: false,
            _heard: heard,
        }
    }

    /// The workflow as it would be saved now (a test reads it).
    pub fn draft(&self) -> &Flow {
        &self.editor.draft
    }

    /// What the dialog holds besides: the picks, where they go, the time and whether it starts at one.
    pub fn editor(&self) -> &Editor {
        &self.editor
    }

    /// Changes what the dialog holds, with the index as it is now, and draws again.
    fn change(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut Editor, &World)) {
        let (store, chosen) = (self.work.clone(), self.drafts.read(cx).chosen());
        {
            let work = store.read(cx);
            let Some(index) = work.snapshot.as_deref() else { return };
            let (workspaces, statuses) = work.built_from();
            let world =
                World { work: index, groups: &work.groups, workspaces, statuses, chosen: &chosen, now: work.now() };
            change(&mut self.editor, &world);
        }
        self.edits += 1;
        cx.notify();
    }

    /// A task's place in the workflow, by its card.
    fn at(&self, subject: &str) -> Option<usize> {
        self.editor.draft.tasks.iter().position(|task| task.subject == subject)
    }

    /// One task changed by what you typed or picked.
    fn change_task(&mut self, subject: &str, cx: &mut Context<Self>, change: impl FnOnce(&mut FlowTask)) {
        if let Some(at) = self.at(subject) {
            self.change(cx, |editor, world| editor.change_task(at, world, change));
        }
    }

    /// Before the dialog is drawn: every task has its fields, every field holds what the workflow says, and what
    /// the dialog says is made from the workflow and the index as they are now.
    fn settle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let draft = &self.editor.draft;
        if self.put_name != draft.name {
            self.put_name = draft.name.clone();
            self.name.update(cx, |field, cx| field.set_value(draft.name.clone(), window, cx));
        }
        if self.put_every != draft.instruction {
            self.put_every = draft.instruction.clone();
            self.every.update(cx, |field, cx| field.set_value(draft.instruction.clone(), window, cx));
        }
        if self.put_time != self.editor.time {
            self.put_time = self.editor.time.clone();
            self.time.update(cx, |field, cx| field.set_value(self.editor.time.clone(), window, cx));
        }
        if self.put_minutes != draft.minutes {
            self.put_minutes = draft.minutes;
            self.minutes.update(cx, |field, cx| field.set_value(draft.minutes.to_string(), window, cx));
        }
        self.rows.retain(|subject, _| draft.tasks.iter().any(|task| &task.subject == subject));
        for task in &draft.tasks {
            let asks = if task.kind == "custom" { enkidu_model::work::flow_editor::OWN_WORDS_ASK } else { "" };
            if !self.rows.contains_key(&task.subject) {
                self.rows.insert(task.subject.clone(), row_fields(task, draft.minutes, asks, window, cx));
            }
            let Some(row) = self.rows.get_mut(&task.subject) else { continue };
            if row.put_words != task.instruction {
                row.put_words = task.instruction.clone();
                row.words.update(cx, |field, cx| field.set_value(task.instruction.clone(), window, cx));
            }
            if row.put_asks != asks {
                row.put_asks = asks;
                row.words.update(cx, |field, cx| field.set_placeholder(asks, window, cx));
            }
            if row.put_minutes != task.minutes {
                row.put_minutes = task.minutes;
                let own = task.minutes.map(|minutes| minutes.to_string()).unwrap_or_default();
                row.minutes.update(cx, |field, cx| field.set_value(own, window, cx));
            }
            if row.put_usual != draft.minutes {
                row.put_usual = draft.minutes;
                row.minutes.update(cx, |field, cx| field.set_placeholder(draft.minutes.to_string(), window, cx));
            }
            match (&row.message, row.open) {
                (_, false) => row.message = None,
                (None, true) => {
                    row.put_message = task.message.clone();
                    let (message, subject) = (task.message.clone(), task.subject.clone());
                    let field = cx.new(|cx| TextareaState::new(window, cx).auto_grow(6, 16).default_value(message));
                    let heard = cx.subscribe_in(&field, window, move |this, field, event: &InputEvent, _, cx| {
                        if matches!(event, InputEvent::Change) {
                            let value = field.read(cx).value().to_string();
                            this.typed_message(&subject, value, cx);
                        }
                    });
                    row.message = Some((field, heard));
                }
                (Some((field, _)), true) => {
                    if row.put_message != task.message {
                        row.put_message = task.message.clone();
                        field.update(cx, |field, cx| field.set_value(task.message.clone(), window, cx));
                    }
                }
            }
        }
        let of = (self.work.read(cx).revision, self.edits);
        if self.made.as_ref().is_none_or(|made| made.of != of) {
            self.made = self.make(of, cx).map(Rc::new);
        }
    }

    /// What the dialog says of the workflow in it.
    fn make(&self, of: (u64, u64), cx: &App) -> Option<Made> {
        let project = (self.page.upgrade())
            .and_then(|page| page.read(cx).projects(cx).get(&self.editor.draft.project_id).cloned());
        let chosen = self.drafts.read(cx).chosen();
        let work = self.work.read(cx);
        let index = work.snapshot.as_deref()?;
        let (workspaces, statuses) = work.built_from();
        let now = work.now();
        let world = World { work: index, groups: &work.groups, workspaces, statuses, chosen: &chosen, now };
        let editor = &self.editor;
        Some(Made {
            rows: editor.rows(&world),
            problem: editor.problem(&world),
            title: editor.title(),
            says: project.as_deref().map(Editor::says),
            save: editor.save_words(now),
            starts: editor.starts_words(now),
            plan: editor.plan_words(now),
            ends: editor.ends(now),
            into: editor.into_choices(&world),
            of,
        })
    }

    /// A task's whole message was typed in: it is yours now, and the workflow's words no longer change it.
    fn typed_message(&mut self, subject: &str, value: String, cx: &mut Context<Self>) {
        let Some(at) = self.at(subject) else { return };
        if self.editor.draft.tasks[at].message == value {
            return;
        }
        if let Some(row) = self.rows.get_mut(subject) {
            row.put_message = value.clone();
        }
        self.change_task(subject, cx, |task| {
            task.message = value;
            task.edited = true;
        });
    }

    /// The dialog closed by itself (Esc, a click beside it): the page has none open now.
    fn closed(&mut self, cx: &mut Context<Self>) {
        self.left = true;
        let _ = self.page.update(cx, |page, cx| page.flow_editor_closed(cx));
    }

    /// Closes the dialog.
    pub(crate) fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.left {
            window.close_dialog(cx);
            self.closed(cx);
        }
    }

    /// Run now, or the white button: the workflow is saved, the dialog closes, and the page lets go of the picks.
    /// One that can't be saved stays open and says why.
    fn save(&mut self, run_now: bool, window: &mut Window, cx: &mut Context<Self>) {
        let ready = self.made.as_ref().is_some_and(|made| made.problem.is_none());
        if self.busy || !ready {
            return;
        }
        let now = self.work.read(cx).now();
        let flow = if run_now { self.editor.to_run_now(now) } else { self.editor.to_save(now) };
        self.busy = true;
        let saving = Work::save_flow(&self.work, flow, cx);
        cx.spawn_in(window, async move |this: WeakEntity<Self>, cx: &mut AsyncWindowContext| {
            let kept = saving.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match kept {
                    Ok(saved) => {
                        let (title, detail) = saved_toast(&saved, this.work.read(cx).now());
                        match detail {
                            Some(detail) => toast::done(window, title, detail, cx),
                            None => toast::good(window, title, cx),
                        }
                        this.close(window, cx);
                        let _ = this.page.update(cx, |page, cx| page.flow_saved(&saved.id, cx));
                    }
                    Err(why) => toast::warning(window, "Couldn't save the workflow", why, cx),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// "New task", under the tasks (item 8.7): a task of your own is made in the workflow's project, in its own
    /// dialog in
    /// front of this one, and joins the workflow's tasks as the last one.
    fn new_own_task(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(frame) = self.page.upgrade().map(|page| page.read(cx).frame().clone()) else { return };
        let project = self.editor.draft.project_id.clone();
        let editor = cx.entity().downgrade();
        let then: crate::work::own_task::Then = Box::new(move |task, _, cx| {
            let card = task.subject();
            let _ = editor.update(cx, |this, cx| {
                this.change(cx, |editor, world| {
                    editor.add_card(&card, world);
                })
            });
        });
        crate::work::own_task::open(self.work.clone(), &frame, None, Some(project), Some(then), window, cx);
    }

    /// "Use these words for every …": a task's words become its kind's own, for the tasks to come.
    fn keep_words(&mut self, at: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(task) = self.editor.draft.tasks.get(at) else { return };
        let (kind, words) = (task.kind.clone(), task.instruction.clone());
        let mut kinds = self.work.read(cx).snapshot.as_ref().map(|index| index.flow_kinds.clone()).unwrap_or_default();
        kinds.insert(kind, words);
        let keeping = Work::save_flow_kinds(&self.work, kinds, cx);
        // the words are the kind's now: the offer goes
        self.edits += 1;
        cx.spawn_in(window, async move |this: WeakEntity<Self>, cx: &mut AsyncWindowContext| {
            let kept = keeping.await;
            let _ = this.update_in(cx, |_, window, cx| match kept {
                Ok(()) => toast::done(window, KIND_KEPT.0, KIND_KEPT.1, cx),
                Err(why) => toast::warning(window, "Couldn't keep those words", why, cx),
            });
        })
        .detach();
        cx.notify();
    }
}

/// A task's fields as it comes into the dialog: its words, and its own minutes.
fn row_fields(
    task: &FlowTask,
    usual: u32,
    asks: &'static str,
    window: &mut Window,
    cx: &mut Context<FlowEditor>,
) -> RowFields {
    let (instruction, own) = (task.instruction.clone(), task.minutes.map(|m| m.to_string()).unwrap_or_default());
    let words =
        cx.new(|cx| TextareaState::new(window, cx).auto_grow(2, 6).placeholder(asks).default_value(instruction));
    let minutes = cx
        .new(|cx| InputState::new(window, cx).validate(minutes_only).placeholder(usual.to_string()).default_value(own));
    let (for_words, for_minutes) = (task.subject.clone(), task.subject.clone());
    let heard = [
        cx.subscribe_in(&words, window, move |this, field, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                let value = field.read(cx).value().to_string();
                let Some(at) = this.at(&for_words) else { return };
                if this.editor.draft.tasks[at].instruction != value {
                    if let Some(row) = this.rows.get_mut(&for_words) {
                        row.put_words = value.clone();
                    }
                    this.change_task(&for_words, cx, |task| task.instruction = value);
                }
            }
        }),
        cx.subscribe_in(&minutes, window, move |this, field, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                // nothing typed: the workflow's minutes
                let value: Option<u32> = field.read(cx).value().trim().parse().ok();
                let Some(at) = this.at(&for_minutes) else { return };
                if this.editor.draft.tasks[at].minutes != value {
                    if let Some(row) = this.rows.get_mut(&for_minutes) {
                        row.put_minutes = value;
                    }
                    this.change_task(&for_minutes, cx, |task| task.minutes = value);
                }
            }
        }),
    ];
    RowFields {
        words,
        put_words: task.instruction.clone(),
        put_asks: asks,
        minutes,
        put_minutes: task.minutes,
        put_usual: usual,
        message: None,
        put_message: String::new(),
        open: false,
        _heard: heard,
    }
}

/// One choice of a picker: its value, and what it is called.
struct Choice {
    value: String,
    label: SharedString,
}

/// A choice among a few: what is picked, a chevron, and a menu of the choices, the picked one checked. It says what is
/// picked (its label), what it picks (`what`) and the picked value.
fn picker(
    id: impl Into<ElementId>,
    what: &'static str,
    picked: &str,
    choices: Vec<Choice>,
    on_pick: impl Fn(&str, &mut Window, &mut App) + 'static,
) -> MenuFrom {
    let shown = choices.iter().find(|choice| choice.value == picked).map(|choice| choice.label.clone());
    let shown = shown.unwrap_or_default();
    let (value, on_pick) = (picked.to_string(), Rc::new(on_pick));
    MenuTrigger::new(id, shown.clone())
        .aria_description(what)
        .aria_value(SharedString::from(value.clone()))
        .h(theme::INPUT_HEIGHT)
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(6.))
        .rounded(theme::RADIUS_CONTROL)
        .bg(theme::CANVAS)
        .border_1()
        .border_color(theme::RULE_STRONG)
        .px(px(8.))
        .text_size(px(12.5))
        .text_color(theme::INK)
        .cursor_pointer()
        .hover(|s| s.border_color(alpha(theme::INK, 0.22)))
        .child(div().min_w_0().flex_1().truncate().child(shown))
        .child(svg().path("icons/chevron-down.svg").flex_none().size(px(12.)).text_color(theme::INK_3))
        .menu(gpui_kit::Anchor::TopLeft, move |menu, _, _| {
            let mut menu = menu.min_w(px(220.));
            for choice in &choices {
                let (on_pick, picks) = (on_pick.clone(), choice.value.clone());
                let option = option(
                    SharedString::from(format!("flow-option:{}", choice.value)),
                    choice.label.clone(),
                    choice.value == value,
                );
                menu = menu.item(option.on_click(move |_, window, cx| on_pick(&picks, window, cx)));
            }
            menu
        })
}

/// One choice in a picker's menu: a check when it is the one picked, then its words.
fn option(id: SharedString, label: SharedString, picked: bool) -> PopupMenuItem {
    PopupMenuItem::element(move |_, _| {
        div()
            .id(id.clone())
            .test_support()
            .aria_label(label.clone())
            .aria_selected(picked)
            .w_full()
            .flex()
            .items_center()
            .gap(px(8.))
            .text_color(theme::INK)
            .child(svg().path("icons/check.svg").flex_none().size(px(13.)).text_color(if picked {
                theme::INK
            } else {
                alpha(theme::INK, 0.)
            }))
            .child(div().min_w_0().flex_1().child(label.clone()))
    })
}

/// A small square button of a task's row: its icon, and what it does.
fn row_button(
    id: impl Into<ElementId>,
    icon: &'static str,
    label: &'static str,
    off: bool,
) -> enkidu_ui::pieces::Pressed {
    let button = enkidu_ui::press::pressable(div().id(id))
        .test_support()
        .role(Role::Button)
        .aria_label(label)
        .flex_none()
        .size(px(24.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.))
        .child(tinted(icon, theme::INK_3, 13.));
    match off {
        // the first can't go earlier, the last can't go later
        true => button.opacity(0.3).cursor_default(),
        false => button.cursor_pointer().hover(|s| s.bg(theme::HOVER)).active(|s| s.bg(theme::SOFT)),
    }
}

/// Quiet words that do something when pressed ("See the whole message").
fn link(id: impl Into<ElementId>, words: impl Into<SharedString>) -> enkidu_ui::pieces::Pressed {
    let words: SharedString = words.into();
    enkidu_ui::press::pressable(div().id(id))
        .test_support()
        .role(Role::Button)
        .aria_label(words.clone())
        .flex_none()
        .text_size(theme::TEXT_SMALL)
        .text_color(theme::INK_3)
        .cursor_pointer()
        .hover(|s| s.text_color(theme::ACCENT))
        .child(words)
}

/// A field of the dialog's top, its name before it ("Name", "Put them in").
fn top_row(label: &'static str, control: impl IntoElement) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .min_w_0()
        .child(div().flex_none().w(px(76.)).text_size(theme::TEXT_SMALL).text_color(theme::INK_3).child(label))
        .child(div().min_w_0().flex_1().child(control))
}

/// A row of the step shown at the right: what it sets at the left, its control beside it.
fn step_row(label: &'static str, control: impl IntoElement) -> Div {
    div()
        .flex()
        .items_start()
        .gap(px(12.))
        .min_w_0()
        .child(
            div()
                .flex_none()
                .w(px(LABEL))
                // level with the first line of a 32 px field
                .pt(px(8.))
                .text_size(theme::TEXT_SMALL)
                .line_height(theme::TEXT_SMALL_LINE)
                .text_color(theme::INK_3)
                .child(label),
        )
        .child(div().min_w_0().flex_1().child(control))
}

/// Quiet words beside a control, on its line.
fn beside(words: impl Into<SharedString>) -> Div {
    div().flex_none().text_size(theme::TEXT_SMALL).text_color(theme::INK_3).child(words.into())
}

impl FlowEditor {
    /// Another step of the chain is shown at the right, from its top.
    fn pick_step(&mut self, step: Step, cx: &mut Context<Self>) {
        if self.editor.step != step {
            self.step_scroll = ScrollHandle::new();
            self.change(cx, |editor, _| editor.step = step);
        }
    }

    /// The chain's first step: how the workflow starts, and what every task gets.
    fn chain_start(&self, made: &Made, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let on = self.editor.shown_task().is_none();
        let mark = tinted("icons/clock.svg", if on { theme::INK } else { theme::INK_2 }, 12.).into_any_element();
        let step = chain_step("flow-start", mark, true, false, !made.rows.is_empty())
            .role(Role::Button)
            .aria_label(SharedString::from(made.ends.start.clone()))
            .aria_description(SharedString::from(made.ends.start_line.clone()))
            .aria_selected(on)
            .cursor_pointer();
        shown_in_chain(step, on, window.is_window_active())
            .child(chain_words(
                div().truncate().text_size(theme::TEXT).text_color(theme::INK).child(made.ends.start.clone()),
                Some(
                    div()
                        .truncate()
                        .text_size(theme::TEXT_SMALL)
                        .text_color(theme::INK_3)
                        .child(made.ends.start_line.clone())
                        .into_any_element(),
                ),
            ))
            .on_click(cx.listener(|this, _, _, cx| this.pick_step(Step::Start, cx)))
            .into_any_element()
    }

    /// A task in the chain: its number on the rail, its key and title, the time it starts at the latest; under
    /// them its kind of work, its minutes and where it runs.
    fn chain_task(&self, at: usize, row: &TaskRow, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let number = enkidu_ui::pieces::tabular(div())
            .text_size(px(11.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(if row.shown { theme::INK } else { theme::INK_2 })
            .child((at + 1).to_string())
            .into_any_element();
        // its kind of work stands out of the line: the one that differs from the others is seen at once
        let rest = row.summary.strip_prefix(row.kind_label.as_str()).unwrap_or(&row.summary).to_string();
        let subject = row.subject.clone();
        let step = chain_step(("flow-task", at), number, true, true, true)
            .role(Role::Button)
            .aria_label(SharedString::from(row.said(at + 1)))
            .aria_description(SharedString::from(row.kind.clone()))
            .aria_value(SharedString::from(row.subject.clone()))
            .aria_selected(row.shown)
            .cursor_pointer();
        shown_in_chain(step, row.shown, window.is_window_active())
            .child(chain_words(
                div()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        mono(div())
                            .flex_none()
                            .text_size(theme::TEXT_SMALL)
                            .text_color(theme::INK_2)
                            .child(row.name.clone()),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .text_size(theme::TEXT)
                            .text_color(theme::INK)
                            .child(row.title.clone()),
                    ),
                Some(
                    div()
                        .id(("flow-task-line", at))
                        .test_support()
                        .aria_label(SharedString::from(row.summary.clone()))
                        .min_w_0()
                        .flex()
                        .text_size(theme::TEXT_SMALL)
                        .child(div().flex_none().text_color(theme::INK_2).child(row.kind_label.clone()))
                        .child(div().min_w_0().truncate().text_color(theme::INK_3).child(rest))
                        .into_any_element(),
                ),
            ))
            .children(row.starts.clone().map(|starts| {
                enkidu_ui::pieces::tabular(div())
                    .id(("flow-task-starts", at))
                    .test_support()
                    .aria_label(SharedString::from(starts.clone()))
                    .flex_none()
                    .pt(px(10.))
                    .text_size(theme::TEXT_STATUS)
                    .text_color(theme::INK_3)
                    .child(starts)
            }))
            .on_click(cx.listener(move |this, _, _, cx| this.pick_step(Step::Task(subject.clone()), cx)))
            .into_any_element()
    }

    /// The chain's last step: when it is over at the latest. Nothing is set here, so it can't be picked.
    fn chain_end(made: &Made) -> AnyElement {
        chain_step("flow-end", tinted("icons/circle-stop.svg", theme::INK_3, 12.).into_any_element(), true, true, false)
            .aria_label(SharedString::from(made.ends.end.clone()))
            .aria_description(made.ends.end_line.unwrap_or_default())
            .child(chain_words(
                div().truncate().text_size(theme::TEXT).text_color(theme::INK_2).child(made.ends.end.clone()),
                made.ends.end_line.map(|line| {
                    div()
                        .truncate()
                        .text_size(theme::TEXT_SMALL)
                        .text_color(theme::INK_3)
                        .child(line)
                        .into_any_element()
                }),
            ))
            .into_any_element()
    }

    /// The left of the dialog: where the cards go and the workflow's name, then its chain from the start to the
    /// end, and New task under it.
    fn chain(&self, made: &Made, window: &Window, cx: &mut Context<Self>) -> Div {
        let this = cx.entity().downgrade();
        // "Put them in": only when you have a workflow they can go into
        let into = (made.into.len() > 1).then(|| {
            let choices: Vec<Choice> = (made.into.iter())
                .map(|(id, words)| Choice { value: id.clone(), label: words.clone().into() })
                .collect();
            top_row(
                "Put them in",
                picker("flow-into", "Put them in", &self.editor.into, choices, move |id, _, cx| {
                    let _ = this.update(cx, |this, cx| this.change(cx, |editor, world| editor.put_into(id, world)));
                }),
            )
        });
        let tasks: Vec<AnyElement> =
            made.rows.iter().enumerate().map(|(at, row)| self.chain_task(at, row, window, cx)).collect();
        let count = made.rows.len();
        div()
            .flex_none()
            .w(px(CHAIN))
            .min_h_0()
            .flex()
            .flex_col()
            .border_r_1()
            .border_color(theme::RULE)
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .pl(px(20.))
                    .pr(px(16.))
                    .pt(px(12.))
                    .pb(px(12.))
                    .children(into)
                    .child(top_row("Name", text_field(&self.name).id("flow-name").aria_label("Name"))),
            )
            .child(
                div()
                    .id("flow-chain")
                    .test_support()
                    .aria_label("The workflow, from its start to its end")
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .pl(px(10.))
                    .pr(px(8.))
                    .pb(px(16.))
                    .overflow_y_scroll()
                    .track_scroll(&self.chain_scroll)
                    .child(self.chain_start(made, window, cx))
                    .child(
                        div()
                            .id("flow-tasks")
                            .test_support()
                            .aria_label("The tasks, in this order")
                            .aria_value(SharedString::from(count.to_string()))
                            .flex_none()
                            .flex()
                            .flex_col()
                            .children(tasks),
                    )
                    .when(count > 0, |chain| chain.child(Self::chain_end(made)))
                    // a task of your own, made here: it joins the tasks as the last one (item 8.7)
                    .child(
                        div().flex_none().mt(px(8.)).pl(px(42.)).flex().child(
                            button_with(
                                "flow-new-task",
                                Some("icons/plus.svg"),
                                "New task",
                                None,
                                ButtonKind::Secondary,
                                ButtonSize::Small,
                            )
                            .tipped("A task of your own for this workflow: something that is not in Jira", None)
                            .on_click(cx.listener(|this, _, window, cx| this.new_own_task(window, cx))),
                        ),
                    ),
            )
    }

    /// The start, shown at the right: when it starts, how long a task gets, whether Live follows the task that
    /// runs, and what every task is told. Set once, for the whole workflow.
    fn start_step(&self, made: &Made, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let editor = &self.editor;
        let scheduled = switch("flow-scheduled", "Start at a time", editor.scheduled, window, cx)
            .on_click(cx.listener(|this, _, _, cx| this.change(cx, |editor, _| editor.scheduled = !editor.scheduled)));
        let live =
            switch_live("flow-live", LIVE, editor.draft.live, window, cx).on_click(cx.listener(|this, _, _, cx| {
                this.change(cx, |editor, world| editor.change(world, |flow| flow.live = !flow.live))
            }));
        let line = || div().h(theme::INPUT_HEIGHT).flex().items_center().gap(px(8.)).min_w_0();
        let head = div()
            .flex_none()
            .child(
                div()
                    .text_size(theme::DIALOG_TITLE)
                    .line_height(theme::DIALOG_TITLE_LINE)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::INK)
                    .child(START_TITLE),
            )
            .child(div().mt(px(2.)).text_size(theme::TEXT_SMALL).text_color(theme::INK_3).child(START_SAYS));
        let starts =
            step_row(
                "Starts at",
                line()
                    .child(scheduled)
                    .child(div().flex_none().w(px(84.)).child(
                        text_field(&self.time).id("flow-time").aria_label("Starts at").disabled(!editor.scheduled),
                    ))
                    .child(
                        div()
                            .id("flow-starts")
                            .test_support()
                            .aria_label(SharedString::from(made.starts.clone()))
                            .min_w_0()
                            .truncate()
                            .text_size(theme::TEXT_SMALL)
                            .text_color(theme::INK_3)
                            .child(made.starts.clone()),
                    ),
            );
        let minutes = step_row(
            "A task gets up to",
            line()
                .child(
                    div()
                        .flex_none()
                        .w(px(64.))
                        .child(text_field(&self.minutes).id("flow-minutes").aria_label("Minutes for each task")),
                )
                .child(div().flex_none().text_size(theme::TEXT).text_color(theme::INK).child("min"))
                .child(beside("unless it says otherwise")),
        );
        let follows = step_row(
            "Live",
            div()
                .min_h(theme::INPUT_HEIGHT)
                .flex()
                .items_center()
                .gap(px(10.))
                .min_w_0()
                .child(div().flex_none().child(live))
                // one sentence that wraps as a whole: what Live does, then what that means, quieter
                .child(div().min_w_0().flex_1().text_size(theme::TEXT).line_height(theme::TEXT_LINE).child(sentence(
                    &[
                        Part { words: LIVE, color: theme::INK.into(), strong: false },
                        Part { words: &format!(" {LIVE_SAYS}"), color: theme::INK_3.into(), strong: false },
                    ],
                    theme::INK,
                ))),
        );
        let every = step_row(
            "For every task",
            div()
                .child(
                    div()
                        .id("flow-instruction")
                        .test_support()
                        .child(text_area(&self.every, false).aria_label("For every task")),
                )
                .child(
                    div()
                        .mt(px(6.))
                        .text_size(theme::TEXT_SMALL)
                        .line_height(theme::TEXT_SMALL_LINE)
                        .text_color(theme::INK_3)
                        .child(EVERY_HINT),
                ),
        );
        // what is different about a workflow, under what it sets
        let unlike = step_row(
            "",
            div().text_size(theme::TEXT_SMALL).line_height(px(17.)).text_color(theme::INK_3).child(UNLIKE),
        );
        vec![
            head.into_any_element(),
            starts.into_any_element(),
            minutes.into_any_element(),
            follows.into_any_element(),
            every.into_any_element(),
            unlike.into_any_element(),
        ]
    }

    /// A task, shown at the right: its title, key and place with the buttons that move it or take it out; then
    /// what it does and for how long, where it runs, what Claude is told, what goes with that, and its whole
    /// message while you look at it.
    fn task_step(&self, at: usize, count: usize, row: &TaskRow, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(fields) = self.rows.get(&row.subject) else { return Vec::new() };
        let this = cx.entity().downgrade();
        let head = div()
            .flex_none()
            .flex()
            .items_start()
            .gap(px(4.))
            .min_w_0()
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .child(
                        div()
                            .line_clamp(2)
                            .text_size(theme::DIALOG_TITLE)
                            .line_height(theme::DIALOG_TITLE_LINE)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::INK)
                            .child(row.title.clone()),
                    )
                    .child(
                        div()
                            .id("flow-step-place")
                            .test_support()
                            .aria_label(SharedString::from(format!("{}, task {} of {count}", row.name, at + 1)))
                            .mt(px(2.))
                            .flex()
                            .items_center()
                            .text_size(theme::TEXT_SMALL)
                            .text_color(theme::INK_3)
                            .child(mono(div()).flex_none().text_color(theme::INK_2).child(row.name.clone()))
                            .child(format!(", task {} of {count}", at + 1)),
                    ),
            )
            .child(match row.first {
                true => row_button(("flow-task-earlier", at), "icons/arrow-up.svg", "Earlier", true),
                false => row_button(("flow-task-earlier", at), "icons/arrow-up.svg", "Earlier", false)
                    .on_click(cx.listener(move |this, _, _, cx| this.change(cx, |editor, _| editor.move_task(at, -1)))),
            })
            .child(match row.last {
                true => row_button(("flow-task-later", at), "icons/arrow-down.svg", "Later", true),
                false => row_button(("flow-task-later", at), "icons/arrow-down.svg", "Later", false)
                    .on_click(cx.listener(move |this, _, _, cx| this.change(cx, |editor, _| editor.move_task(at, 1)))),
            })
            .child(
                row_button(("flow-task-remove", at), "icons/x.svg", "Remove", false)
                    .tipped("Take it out of the workflow", None)
                    .on_click(cx.listener(move |this, _, _, cx| this.change(cx, |editor, _| editor.remove(at)))),
            );
        let kinds: Vec<Choice> =
            KIND_DEFS.iter().map(|kind| Choice { value: kind.id.to_string(), label: kind.label.into() }).collect();
        let kind = picker(("flow-task-kind", at), "Do", &row.kind, kinds, {
            let this = this.clone();
            move |kind, _, cx| {
                let _ = this.update(cx, |this, cx| this.change(cx, |editor, world| editor.set_kind(at, kind, world)));
            }
        });
        let places: Vec<Choice> = (row.choices.iter())
            .map(|choice| Choice { value: choice.value.clone(), label: choice.label.clone().into() })
            .collect();
        let targets = row.choices.clone();
        let place = picker(("flow-task-target", at), "Runs in", &row.target, places, {
            let this = this.clone();
            move |value, _, cx| {
                let Some(target) = targets.iter().find(|choice| choice.value == value).map(|c| c.target.clone()) else {
                    return;
                };
                let _ = this.update(cx, |this, cx| {
                    this.change(cx, |editor, world| editor.change_task(at, world, |task| task.target = target));
                });
            }
        });
        let does = step_row(
            "Do",
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .min_w_0()
                .child(div().flex_none().w(px(210.)).child(kind))
                .child(beside("for up to"))
                .child(div().flex_none().w(px(64.)).child(
                    text_field(&fields.minutes).id(("flow-task-minutes", at)).aria_label("Minutes for this task"),
                ))
                .child(beside("min")),
        );
        let runs = step_row("Runs in", place);
        let waits = row.waits.clone().map(|waits| {
            div()
                .id(("flow-task-waits", at))
                .test_support()
                .aria_label(SharedString::from(waits.clone()))
                .mt(px(6.))
                .min_w_0()
                .text_size(theme::TEXT_SMALL)
                .line_height(theme::TEXT_SMALL_LINE)
                .text_color(theme::INK_3)
                .child(waits)
        });
        let keep = row.make_default.clone().map(|words| {
            link(("flow-task-default", at), words)
                .on_click(cx.listener(move |this, _, window, cx| this.keep_words(at, window, cx)))
        });
        let subject = row.subject.clone();
        let toggle = link(("flow-task-message-toggle", at), row.message_words(fields.open)).on_click(cx.listener(
            move |this, _, _, cx| {
                if let Some(row) = this.rows.get_mut(&subject) {
                    row.open = !row.open;
                    cx.notify();
                }
            },
        ));
        let told = step_row(
            "What Claude is told",
            div()
                .child(
                    div().id(("flow-task-words", at)).test_support().child(
                        text_area(&fields.words, false)
                            .aria_label(SharedString::from(format!("What to do on {}", row.name)))
                            .text_size(px(12.5))
                            .line_height(px(18.)),
                    ),
                )
                .children(waits)
                .child(
                    div()
                        .mt(px(6.))
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_x(px(16.))
                        .gap_y(px(4.))
                        .min_w_0()
                        .child(toggle)
                        .children(keep),
                ),
        );
        let message = fields.message.as_ref().map(|(field, _)| {
            step_row(
                "The whole message",
                div()
                    .child(
                        div()
                            .id(("flow-task-message", at))
                            .test_support()
                            // the field says what it is itself, so what it shows can be read with its name
                            .child(
                                text_area(field, true)
                                    .aria_label(SharedString::from(format!("The message for {}", row.name)))
                                    .text_size(px(11.5))
                                    .line_height(px(17.)),
                            ),
                    )
                    .when(row.edited, |message| {
                        message.child(div().mt(px(6.)).flex().child(
                            link(("flow-task-undo", at), "Undo my edits").on_click(cx.listener(
                                move |this, _, _, cx| this.change(cx, |editor, world| editor.undo_edits(at, world)),
                            )),
                        ))
                    }),
            )
            .into_any_element()
        });
        let mut step =
            vec![head.into_any_element(), does.into_any_element(), runs.into_any_element(), told.into_any_element()];
        step.extend(message);
        step
    }
}

impl Render for FlowEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.settle(window, cx);
        // one height, so the dialog stands still as you go from step to step
        let tall = (f32::from(window.viewport_size().height) * TALLEST).min(HEIGHT);
        let root = div()
            .id("flow-editor")
            .test_support()
            .track_focus(&self.focus)
            .relative()
            .w_full()
            .h(px(tall))
            .flex()
            .flex_col()
            .overflow_hidden();
        let Some(made) = self.made.clone() else { return root.aria_label("A workflow") };
        let head = div()
            .flex_none()
            .flex()
            .items_start()
            .justify_between()
            .gap(px(16.))
            .px(px(20.))
            .pt(px(18.))
            .pb(px(12.))
            .border_b_1()
            .border_color(theme::RULE)
            .child(
                div()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(theme::DIALOG_TITLE)
                            .line_height(theme::DIALOG_TITLE_LINE)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::INK)
                            .truncate()
                            .child(made.title.clone()),
                    )
                    .children(made.says.clone().map(|says| {
                        div()
                            .id("flow-says")
                            .test_support()
                            .aria_label(SharedString::from(says.clone()))
                            .mt(px(4.))
                            .text_size(theme::TEXT_SMALL)
                            .line_height(theme::TEXT_SMALL_LINE)
                            .text_color(theme::INK_2)
                            .child(says)
                    })),
            )
            .child(
                crate::dialog_parts::close("flow-close")
                    .on_click(cx.listener(|this, _, window, cx| this.close(window, cx))),
            );
        let chain = self.chain(&made, window, cx);
        // the step that is picked in the chain: the start, or one task
        let shown = self.editor.shown_task().and_then(|at| Some((at, made.rows.get(at)?)));
        let (said, step) = match shown {
            Some((at, row)) => (row.said(at + 1), self.task_step(at, made.rows.len(), row, cx)),
            None => (START_TITLE.to_string(), self.start_step(&made, window, cx)),
        };
        let step = div()
            .id("flow-step")
            .test_support()
            .aria_label(SharedString::from(said))
            .min_w_0()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(px(14.))
            .px(px(24.))
            .pt(px(16.))
            .pb(px(20.))
            .overflow_y_scroll()
            .track_scroll(&self.step_scroll)
            .children(step);
        let body = div().id("flow-body").test_support().flex_1().min_h_0().flex().child(chain).child(step);
        let off = made.problem.is_some() || self.busy;
        let dim = |button: Pressed| if off { button.opacity(0.45).cursor_default() } else { button };
        let run = dim(button_with(
            "flow-run-now",
            Some("icons/play.svg"),
            "Run now",
            None,
            ButtonKind::Secondary,
            ButtonSize::Medium,
        ));
        let save = dim(button_with(
            "flow-save",
            Some("icons/calendar-clock.svg"),
            made.save.clone(),
            None,
            ButtonKind::Primary,
            ButtonSize::Medium,
        ));
        let (run, save) = match off {
            true => (run, save),
            false => (
                run.on_click(cx.listener(|this, _, window, cx| this.save(true, window, cx))),
                save.on_click(cx.listener(|this, _, window, cx| this.save(false, window, cx))),
            ),
        };
        // where the eye is when it goes for the buttons: what is still missing, else the plan in one line
        let says = match (made.problem.clone(), made.plan.clone()) {
            (Some(problem), _) => Some(
                div()
                    .id("flow-problem")
                    .test_support()
                    .aria_label(SharedString::from(problem.clone()))
                    .truncate()
                    .text_size(theme::TEXT_SMALL)
                    .text_color(theme::LAMP_AMBER)
                    .child(problem),
            ),
            (None, Some(plan)) => Some(
                div()
                    .id("flow-plan")
                    .test_support()
                    .aria_label(SharedString::from(plan.clone()))
                    .truncate()
                    .text_size(theme::TEXT_SMALL)
                    .text_color(theme::INK_2)
                    .child(plan),
            ),
            (None, None) => None,
        };
        let foot = crate::dialog_parts::foot()
            .child(div().min_w_0().flex_1().children(says))
            .child(
                button("flow-cancel", "Cancel", ButtonKind::Ghost, ButtonSize::Medium)
                    .on_click(cx.listener(|this, _, window, cx| this.close(window, cx))),
            )
            .child(run)
            .child(save);
        root.aria_label(SharedString::from(made.title.clone()))
            .aria_description(if self.busy { "saving" } else { "" })
            .child(head)
            .child(body)
            .child(foot)
            .child(crate::dialog_parts::lit_edge())
    }
}
