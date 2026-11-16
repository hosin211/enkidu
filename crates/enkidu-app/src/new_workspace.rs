//! The New workspace dialog (DESIGN.md, "Dialogs"; item 1.19): a worktree of the project on its own branch, with Claude
//! in it. A name; a new branch from a base, or a branch that exists; the steps shown while it is made; and when the
//! branch is checked out in another folder, the two ways on: that folder as it is, or a new branch from it. Opened by
//! ⌘N, the + by the tabs and the project's menu.
//!
//! Since item 19.3 it asks first where Claude works: a new workspace, which is all of the above, or the project's
//! main folder, which the core takes in as it
//! is (`adopt_workspace`: nothing is made or copied, and Remove only forgets it).

use std::time::Duration;

use enkidu_core::commands::Branches;
use enkidu_core::db::Project;
use enkidu_core::workspace::{CreateRequest, Created, project_slug};
use enkidu_model::frame::Frame;
use enkidu_model::new_workspace::{
    Draft, MAIN_NAME, MAIN_SAYS, Mode, Place, Steps, WHERE, WHERE_MAIN, WHERE_NEW, branch_items, name_from_branch,
    next_branch, step_label,
};
use enkidu_ui::lamp::LAMP_FPS;
use enkidu_ui::pieces::{ButtonKind, ButtonSize, Observed, Pressed, button, mono};
use enkidu_ui::tab_pill::tab_row;
use enkidu_ui::tokens::{self as theme, alpha};
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::searchable_list::{SearchableListItem, SearchableVec};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::prelude::*;
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, App, AsyncWindowContext, Context, Entity, FocusHandle, FontWeight,
    IntoElement, Render, SharedString, Subscription, Transformation, WeakEntity, Window, div, percentage, px, svg,
};

use crate::bridge::Bridge;
use crate::projects_row::short_path;
use crate::stores::{ProgressLog, Stores};
use crate::toast;

/// A branch to pick: its name, and where it is ("local", or "origin" for one only there).
#[derive(Clone)]
pub struct BranchChoice {
    name: SharedString,
    hint: &'static str,
}

impl SearchableListItem for BranchChoice {
    type Value = SharedString;

    fn title(&self) -> SharedString {
        self.name.clone()
    }

    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div()
            .w_full()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(8.))
            .child(mono(div()).min_w_0().truncate().text_size(px(12.)).child(self.name.clone()))
            .child(div().flex_none().text_size(px(11.)).text_color(theme::INK_3).child(self.hint))
    }

    fn value(&self) -> &SharedString {
        &self.name
    }
}

type BranchList = SelectState<SearchableVec<BranchChoice>>;

/// Which way out of a taken branch is being taken.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Way {
    Open,
    Branch,
}

/// The branch picked is checked out in another folder.
struct Taken {
    branch: String,
    path: String,
    /// The new branch offered from it: the first free "<branch>-2", "-3"…
    new_branch: Entity<InputState>,
}

pub struct NewWorkspace {
    frame: Entity<Frame>,
    project: Project,
    /// The workspaces folder (Settings), once the core said it.
    folder: Option<String>,
    draft: Draft,
    name: Entity<InputState>,
    branch: Entity<InputState>,
    base: Entity<BranchList>,
    existing: Entity<BranchList>,
    branches: Option<Branches>,
    fetching: bool,
    creating: bool,
    steps: Steps,
    /// How much of the stores' progress log was read.
    seen: usize,
    error: Option<String>,
    taken: Option<Taken>,
    way: Option<Way>,
    /// The dialog's own hold on the keyboard, for when no field is shown (the main folder's side): Enter and Esc
    /// reach the dialog from it, where a keyboard left on the name field that is gone would reach nothing.
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

/// Opens the dialog for the project on screen.
pub fn open(frame: Entity<Frame>, window: &mut Window, cx: &mut App) {
    let project = {
        let f = frame.read(cx);
        f.current_project.as_ref().and_then(|id| f.projects.iter().find(|p| &p.id == id)).cloned()
    };
    let Some(project) = project else { return };
    let view = cx.new(|cx| NewWorkspace::new(frame, project, window, cx));
    let shown = view.clone();
    window.open_dialog(cx, move |dialog, _, cx| {
        // while it is being made it can't be closed: Esc and a click outside do nothing
        let busy = shown.read(cx).busy();
        let on_enter = shown.clone();
        crate::dialog_parts::surface(dialog)
            .w(px(540.))
            .overlay_closable(!busy)
            .keyboard(!busy)
            .child(shown.clone())
            // Enter is the dialog's OK: it makes the workspace, and the dialog closes once it is made
            .on_ok(move |_, window, cx| {
                on_enter.update(cx, |view, cx| view.confirm(window, cx));
                false
            })
    });
    // the name takes the keyboard, once the dialog is drawn
    let name = view.read(cx).name.clone();
    window.defer(cx, move |window, cx| name.update(cx, |name, cx| name.focus(window, cx)));
}

impl NewWorkspace {
    fn new(frame: Entity<Frame>, project: Project, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("sh-21309-stock-fix"));
        let branch = cx.new(|cx| InputState::new(window, cx).placeholder("feature/…"));
        let list = |window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| {
                SelectState::new(SearchableVec::new(Vec::<BranchChoice>::new()), None, window, cx).searchable(true)
            })
        };
        let (base, existing) = (list(window, cx), list(window, cx));
        let mut subscriptions = vec![
            cx.subscribe_in(&name, window, |this: &mut Self, _, event: &InputEvent, window, cx| {
                this.name_event(event, window, cx)
            }),
            cx.subscribe_in(&branch, window, |this: &mut Self, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    this.draft.branch = this.branch.read(cx).value().to_string();
                    this.draft.branch_touched = true;
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => this.submit(window, cx),
                _ => {}
            }),
            cx.subscribe_in(
                &base,
                window,
                |this: &mut Self, _, event: &SelectEvent<SearchableVec<BranchChoice>>, _, cx| {
                    let SelectEvent::Confirm(value) = event;
                    this.draft.base = value.as_ref().map(|v| v.to_string()).unwrap_or_default();
                    cx.notify();
                },
            ),
            cx.subscribe_in(
                &existing,
                window,
                |this: &mut Self, _, event: &SelectEvent<SearchableVec<BranchChoice>>, window, cx| {
                    let SelectEvent::Confirm(value) = event;
                    let picked = value.as_ref().map(|v| v.to_string()).unwrap_or_default();
                    let had_name = !this.draft.name.is_empty();
                    this.draft.pick_existing(&picked);
                    if !had_name && !this.draft.name.is_empty() {
                        let named = this.draft.name.clone();
                        this.name.update(cx, |name, cx| name.set_value(named, window, cx));
                    }
                    this.taken = None;
                    cx.notify();
                },
            ),
        ];
        // the steps of making it come as the core's news, through the stores
        let mut seen = 0;
        if let Some(log) = cx.try_global::<Stores>().map(|s| s.progress.clone()) {
            seen = log.read(cx).0.len();
            subscriptions.push(cx.observe(&log, |this: &mut Self, log, cx| this.read_progress(&log, cx)));
        }
        let this = NewWorkspace {
            frame,
            project,
            folder: None,
            draft: Draft::default(),
            name,
            branch,
            base,
            existing,
            branches: None,
            fetching: false,
            creating: false,
            steps: Steps::default(),
            seen,
            error: None,
            taken: None,
            way: None,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        };
        this.load(window, cx);
        this
    }

    /// Being made, or a way out of a taken branch is being taken: the dialog stays.
    pub fn busy(&self) -> bool {
        self.creating || self.way.is_some()
    }

    /// The branches to pick from and the workspaces folder, then fresh from origin when there is one.
    fn load(&self, window: &mut Window, cx: &mut Context<Self>) {
        // pictures draw the dialog with no core behind it
        if !cx.has_global::<Bridge>() {
            return;
        }
        let project_id = self.project.id.clone();
        let first = Bridge::ask(cx, move |shared| {
            let folder = shared.user_settings().workspaces_dir;
            enkidu_core::commands::list_branches(shared, project_id).map(|branches| (folder, branches))
        });
        let project_id = self.project.id.clone();
        cx.spawn_in(window, async move |this: WeakEntity<Self>, cx: &mut AsyncWindowContext| {
            let Ok(answer) = first.await else { return };
            let has_origin = match answer {
                Ok((folder, branches)) => {
                    let has_origin = branches.has_origin;
                    let _ = this.update_in(cx, |this, window, cx| {
                        this.folder = Some(folder);
                        this.fetching = has_origin;
                        this.got_branches(branches, window, cx);
                    });
                    has_origin
                }
                Err(err) => {
                    let _ = this.update(cx, |this, cx| {
                        this.error = Some(err.message);
                        cx.notify();
                    });
                    false
                }
            };
            if !has_origin {
                return;
            }
            let Ok(fresh) = cx.update(|_, cx| {
                Bridge::ask(cx, move |shared| {
                    let _ = enkidu_core::commands::fetch_project(shared, project_id.clone());
                    enkidu_core::commands::list_branches(shared, project_id)
                })
            }) else {
                return;
            };
            let fresh = fresh.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.fetching = false;
                if let Ok(Ok(branches)) = fresh {
                    this.got_branches(branches, window, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn got_branches(&mut self, branches: Branches, window: &mut Window, cx: &mut Context<Self>) {
        let items: Vec<BranchChoice> = branch_items(&branches.local, &branches.remote)
            .into_iter()
            .map(|b| BranchChoice { name: b.name.into(), hint: b.hint })
            .collect();
        for list in [&self.base, &self.existing] {
            let items = items.clone();
            list.update(cx, |list, cx| list.set_items(SearchableVec::new(items), window, cx));
        }
        // a new branch starts from the project's default base, until another is picked
        if self.draft.base.is_empty()
            && let Some(base) = branches.default_base.clone()
        {
            self.draft.base = base.clone();
            self.base.update(cx, |list, cx| list.set_selected_value(&SharedString::from(base), window, cx));
        }
        if !self.draft.existing.is_empty() {
            let existing = SharedString::from(self.draft.existing.clone());
            self.existing.update(cx, |list, cx| list.set_selected_value(&existing, window, cx));
        }
        self.branches = Some(branches);
        cx.notify();
    }

    fn name_event(&mut self, event: &InputEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            InputEvent::Change => {
                self.draft.name = self.name.read(cx).value().to_string();
                // the new branch follows the name until it is typed itself
                if !self.draft.branch_touched {
                    let name = self.draft.name.clone();
                    self.branch.update(cx, |branch, cx| branch.set_value(name, window, cx));
                }
                cx.notify();
            }
            InputEvent::Blur => {
                self.draft.name_touched = true;
                cx.notify();
            }
            InputEvent::PressEnter { .. } => self.submit(window, cx),
            _ => {}
        }
    }

    /// Where Claude works: a new workspace, or the main folder. The keyboard goes with it: to the name of a new
    /// workspace, or to the dialog itself, where the main folder has nothing to type.
    fn set_place(&mut self, place: Place, window: &mut Window, cx: &mut Context<Self>) {
        if self.draft.place != place && !self.busy() {
            self.draft.place = place;
            // what a try at the other answer left behind (its steps, what went wrong, a taken branch) is not this one's
            (self.taken, self.error, self.steps) = (None, None, Steps::default());
            match place {
                Place::Workspace => self.name.update(cx, |name, cx| name.focus(window, cx)),
                Place::Main => window.focus(&self.focus, cx),
            }
            cx.notify();
        }
    }

    fn set_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        if self.draft.mode != mode && !self.busy() {
            self.draft.mode = mode;
            self.taken = None;
            cx.notify();
        }
    }

    /// The core's news of the steps, for this project, since last read.
    fn read_progress(&mut self, log: &Entity<ProgressLog>, cx: &mut Context<Self>) {
        let log = log.read(cx);
        let mut changed = false;
        for step in log.0.iter().skip(self.seen) {
            changed |= self.steps.apply(step, &self.project.id);
        }
        self.seen = log.0.len();
        if changed {
            cx.notify();
        }
    }

    /// Enter: Create workspace, or with a branch checked out elsewhere, Create the new branch offered from it.
    pub fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.taken.is_some() { self.branch_from_taken(window, cx) } else { self.submit(window, cx) }
    }

    /// Create workspace: the core makes it, saying each step; then it comes on screen and the dialog closes. For
    /// the main folder: Open the main folder.
    pub fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.draft.valid() || self.busy() || self.taken.is_some() {
            return;
        }
        self.creating = true;
        self.error = None;
        self.steps = Steps::default();
        if self.draft.place == Place::Main {
            self.open_main(window, cx);
            cx.notify();
            return;
        }
        let request = self.draft.request(&self.project.id);
        let (mode, existing) = (self.draft.mode, self.draft.existing.clone());
        self.make(request, window, cx, move |this, err, window, cx| match err.path.clone() {
            Some(path) if mode == Mode::Existing && err.code == "branch_taken" => {
                this.steps = Steps::default();
                this.taken = Some(this.taken_offer(existing, path, window, cx));
            }
            _ => this.error = Some(err.message),
        });
        cx.notify();
    }

    /// Asks the core to make `request`; what it made comes on screen, what went wrong goes to `failed`.
    fn make(
        &mut self,
        request: CreateRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
        failed: impl FnOnce(&mut Self, enkidu_core::commands::CmdError, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let answer = Bridge::ask_with_news(cx, move |events, shared| {
            enkidu_core::commands::create_workspace(events.clone(), shared, request)
        });
        cx.spawn_in(window, async move |this: WeakEntity<Self>, cx: &mut AsyncWindowContext| {
            let Ok(answer) = answer.await else { return };
            let _ = this.update_in(cx, |this, window, cx| {
                this.creating = false;
                this.way = None;
                match answer {
                    Ok(created) => this.made(created, window, cx),
                    Err(err) => failed(this, err, window, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The project's main folder itself becomes the workspace: the core takes it in as it is (nothing is made or
    /// copied there). One that is open already is the one that comes on screen (the core answers with it).
    fn open_main(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (project_id, path) = (self.project.id.clone(), self.project.path.clone());
        let answer = Bridge::ask_with_news(cx, move |events, shared| {
            let name = Some(MAIN_NAME.to_string());
            enkidu_core::commands::adopt_workspace(events.clone(), shared, project_id, path, None, name)
        });
        cx.spawn_in(window, async move |this: WeakEntity<Self>, cx: &mut AsyncWindowContext| {
            let Ok(answer) = answer.await else { return };
            let _ = this.update_in(cx, |this, window, cx| {
                this.creating = false;
                match answer {
                    Ok(created) => this.made(created, window, cx),
                    Err(err) => this.error = Some(err.message),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn made(&mut self, created: Created, window: &mut Window, cx: &mut Context<Self>) {
        self.frame.update(cx, |frame, cx| {
            frame.add_workspace(created.workspace);
            cx.notify();
        });
        window.close_dialog(cx);
        for warning in created.warnings {
            toast::warning(window, "Workspace created, with a warning", warning, cx);
        }
    }

    fn taken_offer(&self, branch: String, path: String, window: &mut Window, cx: &mut Context<Self>) -> Taken {
        let known: Vec<String> = self.branches.iter().flat_map(|b| b.local.iter().chain(&b.remote).cloned()).collect();
        let offered = next_branch(&branch, &known);
        let new_branch = cx.new(|cx| InputState::new(window, cx).default_value(offered));
        Taken { branch, path, new_branch }
    }

    /// That folder, as it is, becomes the workspace (with the name given, when it is free).
    fn open_taken(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(taken) = &self.taken else { return };
        if self.way.is_some() {
            return;
        }
        self.way = Some(Way::Open);
        self.error = None;
        let (project_id, path) = (self.project.id.clone(), taken.path.clone());
        let name = Some(self.draft.name.trim().to_string()).filter(|n| !n.is_empty());
        let answer = Bridge::ask_with_news(cx, move |events, shared| {
            enkidu_core::commands::adopt_workspace(events.clone(), shared, project_id, path, None, name)
        });
        cx.spawn_in(window, async move |this: WeakEntity<Self>, cx: &mut AsyncWindowContext| {
            let Ok(answer) = answer.await else { return };
            let _ = this.update_in(cx, |this, window, cx| {
                this.way = None;
                match answer {
                    Ok(created) => this.made(created, window, cx),
                    Err(err) => this.error = Some(err.message),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// A new branch from where the taken one is, in a new workspace here.
    fn branch_from_taken(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(taken) = &self.taken else { return };
        let new_branch = taken.new_branch.read(cx).value().trim().to_string();
        if self.way.is_some()
            || new_branch.is_empty()
            || new_branch.contains(char::is_whitespace)
            || new_branch == taken.branch
        {
            return;
        }
        self.way = Some(Way::Branch);
        self.error = None;
        let name = Some(self.draft.name.trim().to_string()).filter(|n| !n.is_empty());
        let request = CreateRequest {
            project_id: self.project.id.clone(),
            name: name.unwrap_or_else(|| name_from_branch(&new_branch)),
            mode: "copy".into(),
            branch: new_branch,
            base: Some(taken.branch.clone()),
            session_id: None,
        };
        self.make(request, window, cx, |this, err, _, _| this.error = Some(err.message));
        cx.notify();
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.busy() {
            window.close_dialog(cx);
        }
    }
}

/// A field: its label above, the control, and a line under it (a hint, or what is wrong).
fn field(label: &'static str, control: impl IntoElement, below: Option<AnyElement>) -> gpui_kit::Div {
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .min_w_0()
        .child(div().text_size(px(12.)).font_weight(FontWeight::MEDIUM).text_color(theme::INK_2).child(label))
        .child(control)
        .children(below)
}

fn hint(id: &'static str, words: impl Into<SharedString>) -> AnyElement {
    let words = words.into();
    div()
        .id(id)
        .test_support()
        .aria_label(words.clone())
        .text_size(px(12.))
        .text_color(theme::INK_3)
        .child(words)
        .into_any_element()
}

/// One answer of a two-way switch in the dialog (where Claude works; a new branch or one that exists): bright when
/// it is the one chosen. Who uses it says what a click does.
fn segment(id: &'static str, words: &'static str, chosen: bool) -> Observed {
    div()
        .id(id)
        .test_support()
        .role(gpui_kit::Role::Tab)
        .aria_label(words)
        .aria_selected(chosen)
        .h(theme::SEGMENT_HEIGHT)
        .px(px(10.))
        .flex()
        .items_center()
        .text_size(px(12.))
        .text_color(if chosen { theme::INK } else { theme::INK_2 })
        .cursor_pointer()
        .child(words)
}

/// The switch itself: its answers in a well, the pill under the one chosen.
fn switch(
    id: &'static str,
    selected: usize,
    segments: Vec<Observed>,
    window: &mut Window,
    cx: &mut App,
) -> gpui_kit::Div {
    div().p(px(2.)).rounded(px(8.)).bg(theme::FRAME).child(tab_row(id, selected, segments, window, cx))
}

/// A step's sign: turning while it runs, a green tick when done, amber when it warned or failed, a dash when skipped.
fn step_sign(id: SharedString, state: &str) -> AnyElement {
    let disc = |color: gpui_kit::Rgba, icon: &'static str, size: f32| {
        div()
            .flex_none()
            .mt(px(1.))
            .size(px(16.))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .bg(color)
            .child(svg().path(icon).size(px(size)).text_color(theme::white(1.0)))
            .into_any_element()
    };
    match state {
        "running" => svg()
            .flex_none()
            .mt(px(1.))
            .size(px(15.))
            .path("icons/loader-circle.svg")
            .text_color(theme::INK_2)
            .with_animation(
                id,
                Animation::new(Duration::from_secs(1)).repeat_synced().with_max_fps(LAMP_FPS),
                |sign, t| sign.with_transformation(Transformation::rotate(percentage(t))),
            )
            .into_any_element(),
        "done" => disc(theme::LAMP_GREEN, "icons/check.svg", 10.),
        "warning" | "error" => disc(theme::LAMP_AMBER, "icons/triangle-alert.svg", 9.),
        _ => svg()
            .flex_none()
            .mt(px(1.))
            .size(px(15.))
            .path("icons/circle-minus.svg")
            .text_color(theme::INK_3)
            .into_any_element(),
    }
}

/// A button that may be greyed out: then it neither looks nor acts pressable.
fn maybe(button: Pressed, enabled: bool) -> Pressed {
    if enabled { button } else { button.opacity(0.45).cursor_default() }
}

impl NewWorkspace {
    /// The dialog's first question: where Claude works. A new workspace (what the rest of the dialog sets up), or
    /// the project's main folder.
    fn where_row(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let answer = |id: &'static str, words: &'static str, place: Place, cx: &mut Context<Self>| -> Observed {
            segment(id, words, self.draft.place == place)
                .on_click(cx.listener(move |this, _, window, cx| this.set_place(place, window, cx)))
        };
        let segments = vec![
            answer("ws-where-new", WHERE_NEW, Place::Workspace, cx),
            answer("ws-where-main", WHERE_MAIN, Place::Main, cx),
        ];
        let selected = usize::from(self.draft.place == Place::Main);
        div()
            .id("ws-where")
            .test_support()
            .aria_label(WHERE)
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .child(div().text_size(px(12.)).font_weight(FontWeight::MEDIUM).text_color(theme::INK).child(WHERE))
            .child(switch("ws-where-switch", selected, segments, window, cx))
    }

    /// The main folder's side of the dialog: the folder, and what working there means.
    fn main_box(&self) -> impl IntoElement {
        let folder = SharedString::from(short_path(&self.project.path));
        div()
            .id("ws-main-box")
            .flex()
            .flex_col()
            .gap(px(10.))
            .p(px(14.))
            .rounded(px(10.))
            .bg(alpha(theme::CHROME, 0.7))
            .border_1()
            .border_color(theme::RULE)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .min_w_0()
                    .child(svg().path("icons/folder-open.svg").flex_none().size(px(14.)).text_color(theme::INK_2))
                    .child(
                        mono(div())
                            .id("ws-main-folder")
                            .test_support()
                            .aria_label(folder.clone())
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.))
                            .text_color(theme::INK)
                            .child(folder),
                    ),
            )
            .child(
                div()
                    .id("ws-main-says")
                    .test_support()
                    .aria_label(MAIN_SAYS)
                    .text_size(px(12.))
                    .line_height(px(18.))
                    .text_color(theme::INK_2)
                    .child(MAIN_SAYS),
            )
    }

    fn branch_box(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let answer = |id: &'static str, words: &'static str, mode: Mode, cx: &mut Context<Self>| -> Observed {
            segment(id, words, self.draft.mode == mode)
                .on_click(cx.listener(move |this, _, _, cx| this.set_mode(mode, cx)))
        };
        let segments = vec![
            answer("ws-mode-new", "New branch", Mode::New, cx),
            answer("ws-mode-existing", "Existing branch", Mode::Existing, cx),
        ];
        let selected = usize::from(self.draft.mode == Mode::Existing);
        let switch = switch("ws-mode", selected, segments, window, cx);
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .child(div().text_size(px(12.)).font_weight(FontWeight::MEDIUM).text_color(theme::INK).child("Branch"))
            .child(switch);
        // what the branch lists hold, said as it is at each moment
        let origin = self.branches.as_ref().map(|b| b.has_origin);
        let (base_hint, existing_hint) = match (origin, self.fetching) {
            (None, _) => ("Reading the branches…", "Reading the branches…"),
            (Some(_), true) => ("Fetching from origin…", "Fetching from origin…"),
            (Some(true), false) => ("Fresh from origin", "Local branches and branches on origin"),
            (Some(false), false) => ("Local branches", "Local branches"),
        };
        let body = match self.draft.mode {
            Mode::New => div()
                .flex()
                .gap(px(12.))
                .child(
                    field(
                        "Name of the new branch",
                        enkidu_ui::pieces::text_field(&self.branch).id("ws-branch-name").disabled(self.busy()),
                        None,
                    )
                    .flex_1(),
                )
                .child(
                    field(
                        "Starts from",
                        div().id("ws-base").test_support().child(
                            Select::new(&self.base)
                                .h(theme::INPUT_HEIGHT)
                                .placeholder("Pick a base branch")
                                .search_placeholder("Find a branch"),
                        ),
                        Some(hint("ws-base-hint", base_hint)),
                    )
                    .flex_1(),
                ),
            Mode::Existing => div().child(field(
                "Branch to work on",
                div().id("ws-existing").child(
                    Select::new(&self.existing)
                        .h(theme::INPUT_HEIGHT)
                        .placeholder("Pick a branch")
                        .search_placeholder("Find a branch"),
                ),
                Some(hint("ws-existing-hint", existing_hint)),
            )),
        };
        div()
            .id("ws-branch-box")
            .flex()
            .flex_col()
            .gap(px(12.))
            .p(px(14.))
            .rounded(px(10.))
            .bg(alpha(theme::CHROME, 0.7))
            .border_1()
            .border_color(theme::RULE)
            .child(header)
            .child(body)
    }

    fn steps_list(&self) -> impl IntoElement {
        div().id("create-steps").test_support().flex().flex_col().gap(px(8.)).children(self.steps.0.iter().map(
            |step| {
                let running = step.state == "running";
                let id = SharedString::from(format!("step-{}", step.step));
                div()
                    .id(id.clone())
                    .test_support()
                    .aria_label(SharedString::from(format!("{}: {}", step_label(&step.step), step.state)))
                    .flex()
                    .items_start()
                    .gap(px(10.))
                    .text_size(px(13.))
                    .child(step_sign(SharedString::from(format!("{id}-sign")), &step.state))
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_color(if running { theme::INK } else { theme::INK_2 })
                                    .child(step_label(&step.step).to_string()),
                            )
                            .when(!running && !step.detail.is_empty(), |d| {
                                d.child(
                                    div()
                                        .mt(px(2.))
                                        .text_size(px(12.))
                                        .line_height(px(16.))
                                        .text_color(theme::INK_3)
                                        .child(step.detail.clone()),
                                )
                            }),
                    )
            },
        ))
    }

    fn taken_panel(&self, taken: &Taken, cx: &mut Context<Self>) -> impl IntoElement {
        let there = self.frame.read(cx).workspaces.iter().find(|w| w.path == taken.path).map(|w| w.name.clone());
        let open_words = match &there {
            Some(name) => format!("Go to {name}"),
            None => "Open that folder as a workspace".to_string(),
        };
        let busy = self.way;
        let open_label = if busy == Some(Way::Open) { "Opening…".to_string() } else { open_words };
        div()
            .id("branch-taken")
            .test_support()
            .aria_label(SharedString::from(format!("{} is checked out in {}", taken.branch, short_path(&taken.path))))
            .flex()
            .flex_col()
            .gap(px(10.))
            .p(px(14.))
            .rounded(px(10.))
            .bg(alpha(theme::CHROME, 0.7))
            .border_1()
            .border_color(theme::RULE)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_x(px(4.))
                    .text_size(px(12.5))
                    .line_height(px(18.))
                    .text_color(theme::INK_2)
                    .child(mono(div()).text_size(px(12.)).text_color(theme::INK).child(taken.branch.clone()))
                    .child("is checked out in")
                    .child(div().text_color(theme::INK).child(short_path(&taken.path)))
                    .child(". Git checks a branch out in one folder only."),
            )
            .child(
                div().child(
                    maybe(button("taken-open", open_label, ButtonKind::Secondary, ButtonSize::Medium), busy.is_none())
                        .on_click(cx.listener(|this, _, window, cx| this.open_taken(window, cx))),
                ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .min_w_0()
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(12.5))
                            .text_color(theme::INK_3)
                            .child("Or a new branch from it:"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(enkidu_ui::pieces::text_field(&taken.new_branch).id("taken-branch")),
                    )
                    .child(
                        maybe(
                            button(
                                "taken-new-branch",
                                if busy == Some(Way::Branch) { "Creating…" } else { "Create" },
                                ButtonKind::Secondary,
                                ButtonSize::Medium,
                            ),
                            busy.is_none(),
                        )
                        .on_click(cx.listener(|this, _, window, cx| this.branch_from_taken(window, cx))),
                    ),
            )
    }
}

impl Render for NewWorkspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let name_line = match (self.draft.name_error(), &self.folder) {
            (Some(rule), _) => Some(
                div()
                    .id("ws-name-error")
                    .test_support()
                    .aria_label(rule)
                    .text_size(px(12.))
                    .text_color(theme::LAMP_RED)
                    .child(rule)
                    .into_any_element(),
            ),
            (None, Some(folder)) => {
                let name = if self.draft.name.is_empty() { "…" } else { self.draft.name.as_str() };
                Some(hint("ws-folder", format!("{}/{}/{name}", short_path(folder), project_slug(&self.project.name))))
            }
            (None, None) => None,
        };
        let valid = self.draft.valid() && !self.busy() && self.taken.is_none();
        let error = self.error.clone().map(|error| {
            div()
                .id("create-error")
                .test_support()
                .aria_label(SharedString::from(error.clone()))
                .px(px(12.))
                .py(px(8.))
                .rounded(px(8.))
                .bg(alpha(theme::LAMP_RED, 0.1))
                .text_size(px(12.))
                .line_height(px(16.))
                .text_color(theme::LAMP_RED)
                .child(error)
        });
        let taken = self.taken.as_ref().map(|taken| self.taken_panel(taken, cx).into_any_element());
        let footer = crate::dialog_parts::foot()
            .child(
                div()
                    .flex_1()
                    .text_size(px(12.))
                    .text_color(theme::INK_3)
                    .when(!self.busy() && self.taken.is_none(), |d| d.child(self.draft.place.enter())),
            )
            .child(
                maybe(button("ws-cancel", "Cancel", ButtonKind::Secondary, ButtonSize::Medium), !self.busy())
                    .on_click(cx.listener(|this, _, window, cx| this.close(window, cx))),
            )
            .child(
                maybe(
                    button(
                        "create-workspace",
                        self.draft.place.button(self.creating),
                        ButtonKind::Primary,
                        ButtonSize::Medium,
                    ),
                    valid,
                )
                .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
            );
        let says = self.draft.place.says(&self.project.name);
        // while it is being made it can't be closed
        let close = crate::dialog_parts::close("ws-close")
            .when(self.busy(), |close| close.opacity(0.45).cursor_default())
            .on_click(cx.listener(|this, _, window, cx| this.close(window, cx)));
        let body = div()
            .flex()
            .flex_col()
            .gap(px(20.))
            .px(px(20.))
            .pt(px(12.))
            .pb(px(16.))
            .child(self.where_row(window, cx))
            .map(|body| match self.draft.place {
                Place::Workspace => body
                    .child(field(
                        "Name",
                        enkidu_ui::pieces::text_field(&self.name).id("ws-name").disabled(self.busy()),
                        name_line,
                    ))
                    .child(self.branch_box(window, cx)),
                Place::Main => body.child(self.main_box()),
            })
            .when(!self.steps.0.is_empty(), |d| d.child(self.steps_list()))
            .children(taken)
            .children(error);
        div()
            .id("new-workspace-dialog")
            .test_support()
            .aria_label("New workspace")
            // what it is for, which follows where Claude works
            .aria_description(SharedString::from(says.clone()))
            .track_focus(&self.focus)
            .relative()
            .w_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(crate::dialog_parts::head("New workspace", Some(says.into()), Some(close)))
            .child(body)
            .child(footer)
            .child(crate::dialog_parts::lit_edge())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_folder_is_named_as_the_core_names_it() {
        // the hint takes the core's own slug, so it names the folder the core will make
        assert_eq!(project_slug("shop"), "shop");
        assert_eq!(project_slug("my shop!"), "my-shop-");
        assert_eq!(project_slug("!!!"), "project");
    }
}
