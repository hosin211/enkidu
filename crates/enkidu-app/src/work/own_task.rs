//! A task of your own (item 8.5): the dialog that
//! makes one, and changes or deletes one that is there. A title, what it is about if you want to say, the project it
//! is for, and its state: To do, In progress or Done. Nothing is read from anywhere and nothing is sent anywhere: the
//! task lives in the app's own database, and its card stands on the Work board with the tickets.

use enkidu_core::commands::OwnTaskDraft;
use enkidu_core::work::model::OwnTask;
use enkidu_model::frame::Frame;
use enkidu_model::work::board::own_state;
use enkidu_ui::pieces::{ButtonKind, ButtonSize, Observed, button, project_hue, text_field};
use enkidu_ui::tab_pill::{word_segment, word_switch};
use enkidu_ui::tokens as theme;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input::{InputState, TextareaState};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, AsyncWindowContext, Context, Entity, FontWeight, IntoElement, Render, Role, SharedString, WeakEntity, Window,
    div, px,
};

use crate::toast;
use crate::work::store::Work;

/// The states in the order the dialog and the card show them.
pub const STATES: [&str; 3] = ["todo", "doing", "done"];

/// What is done with a task once it is kept (the workflow dialog puts a new one among its tasks).
pub type Then = Box<dyn Fn(&OwnTask, &mut Window, &mut App)>;

pub struct OwnTaskDialog {
    work: Entity<Work>,
    /// The task being changed; none for a new one.
    editing: Option<OwnTask>,
    /// The projects a new one can be for: each one's id, name and place in the projects row (its color).
    projects: Vec<(String, String, usize)>,
    /// The one picked, by its place in `projects`.
    project: usize,
    /// `todo`, `doing` or `done`.
    state: String,
    title: Entity<InputState>,
    notes: Entity<TextareaState>,
    /// Delete was pressed: Keep it, or Delete it.
    confirming: bool,
    busy: bool,
    error: Option<String>,
    then: Option<Then>,
}

/// Opens the dialog: for a new task (`editing` none) in one of the frame's projects (`only`, when it must be that
/// one: a workflow's), or for one that is there. Hands back the view, or none when there is no project to make a
/// task for.
pub fn open(
    work: Entity<Work>,
    frame: &Entity<Frame>,
    editing: Option<OwnTask>,
    only: Option<String>,
    then: Option<Then>,
    window: &mut Window,
    cx: &mut App,
) -> Option<Entity<OwnTaskDialog>> {
    let (projects, on_screen) = {
        let frame = frame.read(cx);
        let wanted = editing.as_ref().map(|task| task.project_id.clone()).or(only);
        let projects: Vec<(String, String, usize)> = (frame.projects.iter().enumerate())
            .filter(|(_, project)| wanted.as_ref().is_none_or(|id| &project.id == id))
            .map(|(ix, project)| (project.id.clone(), project.name.clone(), ix))
            .collect();
        (projects, frame.current_project.clone())
    };
    if projects.is_empty() && editing.is_none() {
        return None;
    }
    // a new task is for the project on screen, unless you pick another
    let project = projects.iter().position(|(id, ..)| Some(id) == on_screen.as_ref()).unwrap_or(0);
    let view = cx.new(|cx| {
        let (title, notes, state) = match &editing {
            Some(task) => (task.title.clone(), task.notes.clone(), task.state.clone()),
            None => (String::new(), String::new(), "todo".to_string()),
        };
        OwnTaskDialog {
            work,
            editing,
            projects,
            project,
            state,
            title: cx.new(|cx| InputState::new(window, cx).placeholder("What you want to do").default_value(title)),
            notes: cx.new(|cx| {
                TextareaState::new(window, cx)
                    .auto_grow(4, 12)
                    .placeholder("What it is about, how you will know it is done. Claude gets this with the task.")
                    .default_value(notes)
            }),
            confirming: false,
            busy: false,
            error: None,
            then,
        }
    });
    let shown = view.clone();
    window.open_dialog(cx, move |dialog, _, cx| {
        let busy = shown.read(cx).busy;
        let on_enter = shown.clone();
        crate::dialog_parts::surface(dialog)
            .w(px(520.))
            .overlay_closable(!busy)
            .keyboard(!busy)
            .child(shown.clone())
            // Enter in the title keeps it; the dialog closes once it is kept
            .on_ok(move |_, window, cx| {
                on_enter.update(cx, |view, cx| view.save(window, cx));
                false
            })
    });
    // the title takes the keyboard, once the dialog is drawn
    let title = view.read(cx).title.clone();
    window.defer(cx, move |window, cx| title.update(cx, |title, cx| title.focus(window, cx)));
    Some(view)
}

impl OwnTaskDialog {
    fn draft(&self, cx: &App) -> OwnTaskDraft {
        OwnTaskDraft {
            id: self.editing.as_ref().map(|task| task.id),
            project_id: match &self.editing {
                Some(task) => task.project_id.clone(),
                None => self.projects.get(self.project).map(|(id, ..)| id.clone()).unwrap_or_default(),
            },
            title: self.title.read(cx).value().to_string(),
            notes: self.notes.read(cx).value().to_string(),
            state: self.state.clone(),
        }
    }

    /// The name of the project the task is for.
    fn project_name(&self) -> String {
        match &self.editing {
            Some(task) => task.project.clone(),
            None => self.projects.get(self.project).map(|(_, name, _)| name.clone()).unwrap_or_default(),
        }
    }

    /// Add task, or Save: the task is kept, and the dialog closes. What can't be kept is said, and it stays.
    pub fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let draft = self.draft(cx);
        if draft.title.trim().is_empty() {
            self.error = Some("Give the task a title.".into());
            cx.notify();
            return;
        }
        self.busy = true;
        self.error = None;
        let new = draft.id.is_none();
        let keeping = Work::save_own_task(&self.work, draft, self.project_name(), cx);
        cx.spawn_in(window, async move |this: WeakEntity<Self>, cx: &mut AsyncWindowContext| {
            let kept = keeping.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match kept {
                    Ok(task) => {
                        window.close_dialog(cx);
                        if new {
                            toast::done(window, "Added to the board", task.title.clone(), cx);
                        }
                        if let Some(then) = &this.then {
                            then(&task, window, cx);
                        }
                    }
                    Err(why) => this.error = Some(why),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Delete it: the dialog closes, and the task is gone from the board. Its card, if it is open under this dialog,
    /// closes by itself once the board has it gone: so this dialog goes first, or the card would close it in its
    /// place and stay.
    fn delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(task) = self.editing.clone() else { return };
        if self.busy {
            return;
        }
        self.busy = true;
        window.close_dialog(cx);
        let deleting = Work::delete_own_task(&self.work, task.id, cx);
        cx.spawn_in(window, async move |_, cx: &mut AsyncWindowContext| {
            let gone = deleting.await;
            let _ = cx.update(|window, cx| match gone {
                Ok(()) => toast::done(window, "Deleted your task", task.title.clone(), cx),
                Err(why) => toast::warning(window, "Couldn't delete the task", why, cx),
            });
        })
        .detach();
    }

    fn set_state(&mut self, state: &str, cx: &mut Context<Self>) {
        if self.state != state {
            self.state = state.to_string();
            cx.notify();
        }
    }
}

/// A field's name above it.
fn label(words: &'static str) -> gpui_kit::Div {
    div().mb(px(8.)).text_size(theme::TEXT_SMALL).font_weight(FontWeight::MEDIUM).text_color(theme::INK).child(words)
}

/// The three states as a row to pick from: To do, In progress, Done. `pick` is told the one pressed.
pub fn state_row(
    id: &'static str,
    state: &str,
    pick: impl Fn(&'static str, &mut Window, &mut App) + Clone + 'static,
    window: &mut Window,
    cx: &mut App,
) -> gpui_kit::Div {
    let choices: Vec<Observed> = STATES
        .iter()
        .map(|choice| {
            let (on, words) = (*choice == state, own_state(choice).1);
            let pick = pick.clone();
            word_segment(SharedString::from(format!("{id}-{choice}")), words, on)
                .on_click(move |_, window, cx| pick(choice, window, cx))
        })
        .collect();
    let selected = STATES.iter().position(|choice| *choice == state).unwrap_or(0);
    div().flex().child(word_switch(id, selected, choices, window, cx))
}

impl Render for OwnTaskDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let new = self.editing.is_none();
        let says = match new {
            true => "Something you want to do that is not in Jira. It stands on the Work board with your tickets.",
            false => "Yours, not Jira's: it changes only here.",
        };
        // the project: picked for a new task when there is more than one, said for one that is there
        let project = match (&self.editing, self.projects.len()) {
            (None, 2..) => {
                let picked = self.project;
                let choices = self.projects.iter().enumerate().map(|(at, (id, name, place))| {
                    let on = at == picked;
                    // its picture, the one the rail shows
                    let icon = crate::stores::Stores::icon_of(id, cx);
                    div()
                        .id(("own-task-project", at))
                        .test_support()
                        .role(Role::RadioButton)
                        .aria_label(SharedString::from(name.clone()))
                        .aria_selected(on)
                        .h(theme::DIALOG_CONTROL)
                        .px(px(12.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .rounded(theme::RADIUS_CONTROL)
                        .border_1()
                        .border_color(if on {
                            enkidu_ui::tokens::alpha(theme::ACCENT, 0.55)
                        } else {
                            theme::LINE_STRONG
                        })
                        .when(on, |choice| choice.bg(theme::ACCENT_SOFT))
                        .text_size(theme::TEXT_SMALL)
                        .text_color(if on { theme::INK } else { theme::INK_2 })
                        .cursor_pointer()
                        .child(crate::rail::project_picture(name, project_hue(*place), 16., icon))
                        .child(name.clone())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.project = at;
                            cx.notify();
                        }))
                });
                Some(div().child(label("Project")).child(div().flex().flex_wrap().gap(px(8.)).children(choices)))
            }
            _ => None,
        };
        let view = cx.entity().downgrade();
        let state = state_row(
            "own-task-state",
            &self.state,
            move |state, _, cx| {
                let _ = view.update(cx, |this, cx| this.set_state(state, cx));
            },
            window,
            cx,
        );
        let enabled = !self.busy;
        let error = self.error.clone().map(|why| {
            div()
                .id("own-task-error")
                .test_support()
                .aria_label(SharedString::from(why.clone()))
                .flex_1()
                .min_w_0()
                .text_size(theme::TEXT_SMALL)
                .text_color(theme::LAMP_AMBER)
                .child(why)
        });
        // Delete asks once more before it does it
        let delete = (!new).then(|| match self.confirming {
            false => div().flex_1().flex().child(
                button("own-task-delete", "Delete", ButtonKind::Ghost, ButtonSize::Large)
                    .text_color(theme::LAMP_RED)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.confirming = true;
                        cx.notify();
                    })),
            ),
            true => div()
                .flex_1()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(div().text_size(theme::TEXT_SMALL).text_color(theme::INK_2).child("Delete this task?"))
                .child(button("own-task-keep", "Keep it", ButtonKind::Secondary, ButtonSize::Large).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.confirming = false;
                        cx.notify();
                    }),
                ))
                .child(
                    button("own-task-delete-it", "Delete it", ButtonKind::Danger, ButtonSize::Large)
                        .on_click(cx.listener(|this, _, window, cx| this.delete(window, cx))),
                ),
        });
        let save_words = if new { "Add task" } else { "Save" };
        div()
            .id("own-task-dialog")
            .test_support()
            .aria_label(if new { "A task of your own" } else { "Your task" })
            .relative()
            .flex()
            .flex_col()
            .child(crate::dialog_parts::head(
                if new { "A task of your own" } else { "Your task" },
                Some(says.into()),
                Some(crate::dialog_parts::close("own-task-close").on_click(|_, window, cx| window.close_dialog(cx))),
            ))
            .child(
                div()
                    .px(px(24.))
                    .pt(px(12.))
                    .pb(px(24.))
                    .flex()
                    .flex_col()
                    .gap(px(16.))
                    .child(div().child(label("Title")).child(text_field(&self.title).id("own-task-title")))
                    .child(div().child(label("What it is about (optional)")).child(crate::settings::area(
                        &self.notes,
                        "own-task-notes",
                        false,
                    )))
                    .children(project)
                    .child(div().child(label("State")).child(state)),
            )
            .child(
                crate::dialog_parts::foot()
                    .children(delete)
                    .children(error)
                    .child(
                        button("own-task-cancel", "Cancel", ButtonKind::Secondary, ButtonSize::Large)
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        button("own-task-save", save_words, ButtonKind::Primary, ButtonSize::Large)
                            .when(!enabled, |save| save.opacity(0.45).cursor_default())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if enabled {
                                    this.save(window, cx);
                                }
                            })),
                    ),
            )
    }
}
