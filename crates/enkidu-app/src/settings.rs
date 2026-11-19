//! Settings (items 4.3c to 4.3g): one dialog with its pages listed
//! on the left. General (where the changes go, the workspaces folder), Claude (how it starts), and a page for each
//! project (what a new workspace starts with, Live's commands, Claude in that project). Nothing is written until
//! Save; a field that can't be read, or a file that can't be written, is said beside the buttons and the dialog
//! stays. What the fields mean is worked out in `enkidu_model::settings`; the core reads and writes off the drawing
//! thread. The Claude page also has "Claude outside Enkidu" (item 4.4): which Claude folders have the hooks that
//! tell the app what a Claude started elsewhere works on, exactly what they add, and one button a folder to add them
//! or take them out.

use std::collections::{HashMap, HashSet};

use enkidu_core::commands::ClaudeHooks;
use enkidu_core::settings::{ProjectSettings, UserSettings};
use enkidu_core::work::workflows::Run;
use enkidu_model::frame::Frame;
use enkidu_model::settings::{GeneralForm, Look, Page, ProjectForm, WorkflowsForm, size_words};
use enkidu_model::ui::{CodeColors, Layout, Ui};
use enkidu_ui::pieces::{
    ButtonKind, ButtonSize, Observed, button, icon_button, mono, switch, tabular, text_area, text_field, with_code,
};
use enkidu_ui::tab_pill::{word_segment, word_switch};
use enkidu_ui::tokens as theme;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input::{InputState, TextareaState};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, AsyncWindowContext, Context, Entity, FocusHandle, FontWeight, IntoElement, Render, Role,
    ScrollHandle, SharedString, WeakEntity, Window, div, point, px, svg,
};

use crate::bridge::Bridge;
use crate::toast;

/// Where the settings come from.
pub enum Source {
    /// The core: read when the dialog opens, written by Save.
    Core,
    /// Given settings, for pictures and screen tests without a core. Save changes nothing.
    Given(Box<Given>),
}

/// What a dialog without a core shows.
pub struct Given {
    pub user: UserSettings,
    /// Each project's settings, by its id.
    pub projects: HashMap<String, ProjectSettings>,
    /// How Claude starts in the workspace on screen: its name and the command line.
    pub preview: Option<(String, String)>,
    /// The Claude folders and the hooks for Claude outside the app.
    pub hooks: Option<ClaudeHooks>,
    /// Each project's latest workflow runs, by its id, and the moment their ages are counted from.
    pub runs: HashMap<String, Vec<Run>>,
    pub now: i64,
    /// What the Setup page found.
    pub setup: crate::setup::Given,
}

/// Opens Settings over the window on `page`.
pub fn open(frame: Entity<Frame>, page: Page, source: Source, window: &mut Window, cx: &mut App) {
    open_as(frame, page, source, false, window, cx);
}

/// The first launch (item 21.6): Settings opens on Setup, with a welcome above the page.
pub fn open_welcome(frame: Entity<Frame>, window: &mut Window, cx: &mut App) {
    let source = if cx.has_global::<Bridge>() { Source::Core } else { crate::demo::settings(cx) };
    open_as(frame, Page::Setup, source, true, window, cx);
}

fn open_as(frame: Entity<Frame>, page: Page, source: Source, welcome: bool, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| {
        let mut settings = Settings::new(frame, page, source, window, cx);
        settings.setup.welcome = welcome;
        settings
    });
    window.open_dialog(cx, move |dialog, window, _| {
        // 8% down the window, 820 px wide or the window less 16 px on each side. The builder runs with every frame, so
        // it follows the window
        let room = window.viewport_size();
        crate::dialog_parts::surface(dialog)
            .w(px(820f32.min(f32::from(room.width) - 32.)))
            .margin_top(px(f32::from(room.height) * 0.08))
            .child(view.clone())
            // Enter in a field saves nothing: Save is a press of its own
            .on_ok(|_, _, _| false)
    });
}

/// The fields of the General and Claude pages.
struct General {
    /// What lives with the window: where the changes go, the code's colors, the sizes of code and of the terminal.
    look: Look,
    workspaces_dir: Entity<InputState>,
    command: Entity<InputState>,
    args: Entity<InputState>,
    env: Entity<TextareaState>,
}

/// The fields of the Workflows page (item 8.6).
struct WorkflowFields {
    /// As it was read: Save keeps the page only when something differs.
    read: WorkflowsForm,
    minutes: Entity<InputState>,
    time: Entity<InputState>,
    /// Each kind's words: its id, what it is called, and its field.
    kinds: Vec<(String, String, Entity<TextareaState>)>,
    /// The hearing of what is typed in them: "Use Enkidu's words" is there only while they are your own.
    _heard: Vec<gpui_kit::Subscription>,
}

impl WorkflowFields {
    fn new(form: WorkflowsForm, window: &mut Window, cx: &mut Context<Settings>) -> WorkflowFields {
        let kinds: Vec<(String, String, Entity<TextareaState>)> = (form.kinds.iter())
            .map(|(kind, label, words)| (kind.clone(), label.clone(), lines(words, 2, "", window, cx)))
            .collect();
        let heard = (kinds.iter())
            .map(|(_, _, field)| {
                cx.subscribe(field, |_, _, event: &gpui_kit::component::input::InputEvent, cx| {
                    if matches!(event, gpui_kit::component::input::InputEvent::Change) {
                        cx.notify();
                    }
                })
            })
            .collect();
        WorkflowFields {
            minutes: line(&form.minutes, "60", window, cx),
            time: line(&form.time, "02:00", window, cx),
            kinds,
            read: form,
            _heard: heard,
        }
    }

    fn form(&self, cx: &App) -> WorkflowsForm {
        WorkflowsForm {
            minutes: self.minutes.read(cx).value().to_string(),
            time: self.time.read(cx).value().to_string(),
            kinds: (self.kinds.iter())
                .map(|(kind, label, field)| (kind.clone(), label.clone(), field.read(cx).value().to_string()))
                .collect(),
        }
    }
}

/// The fields of a project's page.
pub(crate) struct ProjectFields {
    /// As the core has it: what the page doesn't edit is saved as it was.
    read: ProjectForm,
    default_base: Entity<InputState>,
    copy_files: Entity<TextareaState>,
    setup_script: Entity<TextareaState>,
    live_on: Entity<TextareaState>,
    live_off: Entity<TextareaState>,
    live_message: Entity<TextareaState>,
    args: Entity<InputState>,
    env: Entity<TextareaState>,
    /// Work: what is read from GitHub and Jira.
    pub(crate) work_github: bool,
    pub(crate) work_account: Entity<InputState>,
    pub(crate) work_jira_keys: Entity<InputState>,
    pub(crate) work_back: Entity<InputState>,
    pub(crate) work_ignored: Entity<TextareaState>,
    pub(crate) work_every: Entity<InputState>,
    /// Workflows: the two switches and the morning summary's time.
    pub(crate) send_failed_checks: bool,
    pub(crate) morning_summary: bool,
    pub(crate) morning_at: Entity<InputState>,
    /// The rail: the project's own icon is shown, and another picture than the one found in its folder.
    pub(crate) icon: bool,
    icon_picture: Entity<InputState>,
}

fn line(text: &str, placeholder: &'static str, window: &mut Window, cx: &mut App) -> Entity<InputState> {
    let text = text.to_string();
    cx.new(|cx| InputState::new(window, cx).placeholder(placeholder).default_value(text))
}

fn lines(
    text: &str,
    rows: usize,
    placeholder: &'static str,
    window: &mut Window,
    cx: &mut App,
) -> Entity<TextareaState> {
    let text = text.to_string();
    cx.new(|cx| TextareaState::new(window, cx).auto_grow(rows, rows + 8).placeholder(placeholder).default_value(text))
}

impl General {
    fn new(form: &GeneralForm, window: &mut Window, cx: &mut App) -> General {
        General {
            look: form.look,
            workspaces_dir: line(&form.workspaces_dir, "", window, cx),
            command: line(&form.command, "", window, cx),
            args: line(&form.args, "--model opus --permission-mode acceptEdits", window, cx),
            env: lines(&form.env, 3, "CLAUDE_CONFIG_DIR=~/.claude-work", window, cx),
        }
    }

    fn form(&self, cx: &App) -> GeneralForm {
        GeneralForm {
            look: self.look,
            workspaces_dir: self.workspaces_dir.read(cx).value().to_string(),
            command: self.command.read(cx).value().to_string(),
            args: self.args.read(cx).value().to_string(),
            env: self.env.read(cx).value().to_string(),
        }
    }
}

impl ProjectFields {
    fn new(settings: &ProjectSettings, window: &mut Window, cx: &mut App) -> ProjectFields {
        let read = ProjectForm::of(settings);
        ProjectFields {
            default_base: line(&read.default_base, "development", window, cx),
            copy_files: lines(&read.copy_files, 4, "", window, cx),
            setup_script: lines(&read.setup_script, 3, "", window, cx),
            live_on: lines(&read.live_on, 3, "docker compose restart web", window, cx),
            live_off: lines(&read.live_off, 3, "", window, cx),
            live_message: lines(&read.live_message, 2, "", window, cx),
            args: line(&read.args, "", window, cx),
            env: lines(&read.env, 2, "", window, cx),
            work_github: read.work_github,
            work_account: line(&read.work_account, "Any", window, cx),
            work_jira_keys: line(&read.work_jira_keys, "SH", window, cx),
            work_back: line(&read.work_back, "QA Failed, Reopened", window, cx),
            work_ignored: lines(&read.work_ignored, 2, "", window, cx),
            work_every: line(&read.work_every, "5", window, cx),
            send_failed_checks: read.send_failed_checks,
            morning_summary: read.morning_summary,
            morning_at: line(&read.morning_at, "09:00", window, cx),
            icon: read.icon,
            icon_picture: line(&read.icon_picture, "The one found in the project's folder", window, cx),
            read,
        }
    }

    fn form(&self, cx: &App) -> ProjectForm {
        // what the fields say, on the form as it was read (which keeps what the page doesn't edit)
        let mut form = self.read.clone();
        form.default_base = self.default_base.read(cx).value().to_string();
        form.copy_files = self.copy_files.read(cx).value().to_string();
        form.setup_script = self.setup_script.read(cx).value().to_string();
        form.live_on = self.live_on.read(cx).value().to_string();
        form.live_off = self.live_off.read(cx).value().to_string();
        form.live_message = self.live_message.read(cx).value().to_string();
        form.args = self.args.read(cx).value().to_string();
        form.env = self.env.read(cx).value().to_string();
        form.work_github = self.work_github;
        form.work_account = self.work_account.read(cx).value().to_string();
        form.work_jira_keys = self.work_jira_keys.read(cx).value().to_string();
        form.work_back = self.work_back.read(cx).value().to_string();
        form.work_ignored = self.work_ignored.read(cx).value().to_string();
        form.work_every = self.work_every.read(cx).value().to_string();
        form.send_failed_checks = self.send_failed_checks;
        form.morning_summary = self.morning_summary;
        form.morning_at = self.morning_at.read(cx).value().to_string();
        form.icon = self.icon;
        form.icon_picture = self.icon_picture.read(cx).value().to_string();
        form
    }
}

pub struct Settings {
    frame: Entity<Frame>,
    /// How the window is laid out, where it is kept (the layout is on the General page, and lives with the window).
    ui: Option<Entity<Ui>>,
    pub(crate) source: Source,
    page: Page,
    general: Option<General>,
    /// The Workflows page, once it was shown.
    workflows: Option<WorkflowFields>,
    /// What Save read from it, on its way to the Work index.
    flows_to_keep: Option<(std::collections::BTreeMap<String, String>, enkidu_core::work::flows::FlowDefaults)>,
    pub(crate) projects: HashMap<String, ProjectFields>,
    /// Each project's latest workflow runs, once asked for; why "Run now" didn't start; and that it was just pressed.
    pub(crate) runs: HashMap<String, Vec<Run>>,
    pub(crate) runs_error: Option<String>,
    pub(crate) run_asked: bool,
    /// The projects whose settings were asked for.
    asked: HashSet<String>,
    /// How Claude starts in the workspace on screen: the workspace's name and the command line.
    preview: Option<(String, String)>,
    /// The hooks for Claude outside the app: the Claude folders, which have them, and what they add.
    hooks: Option<ClaudeHooks>,
    /// The Claude folder whose hooks are being added or taken out.
    hooks_busy: Option<String>,
    hooks_error: Option<String>,
    /// "Show what it adds" is open.
    hooks_shown: bool,
    error: Option<String>,
    saving: bool,
    scroll: ScrollHandle,
    /// The Setup page: what was found of each thing the app needs.
    pub(crate) setup: crate::setup::Setup,
    /// The dialog's own hold on the keyboard, for when the page on screen has no field that has it: the keyboard
    /// would stay on a field of the page before, which is no longer drawn, and Esc would reach nothing (item 21.12).
    focus: FocusHandle,
}

impl Settings {
    fn new(frame: Entity<Frame>, page: Page, source: Source, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let ui = cx.try_global::<crate::stores::Stores>().map(|stores| stores.ui.clone());
        let mut settings = Settings {
            frame,
            ui,
            source,
            page,
            general: None,
            workflows: None,
            flows_to_keep: None,
            projects: HashMap::new(),
            runs: HashMap::new(),
            runs_error: None,
            run_asked: false,
            asked: HashSet::new(),
            preview: None,
            hooks: None,
            hooks_busy: None,
            hooks_error: None,
            hooks_shown: false,
            error: None,
            saving: false,
            scroll: ScrollHandle::new(),
            setup: crate::setup::Setup::default(),
            focus: cx.focus_handle(),
        };
        settings.read_user(window, cx);
        settings.read_hooks(cx);
        settings.ensure_page(window, cx);
        // a workflow's run that starts or ends shows on its project's page at once
        if let Some(work) = cx.try_global::<crate::stores::Stores>().map(|stores| stores.work.clone()) {
            cx.subscribe(&work, |settings, _, _: &crate::work::store::WorkNews, cx| {
                if let Page::Project(id) = settings.page.clone() {
                    settings.read_runs(&id, cx);
                }
            })
            .detach();
        }
        settings
    }

    /// The page on screen.
    pub fn page(&self) -> &Page {
        &self.page
    }

    fn look_now(&self, cx: &App) -> Look {
        self.ui.as_ref().map_or_else(Look::default, |ui| Look::of(ui.read(cx)))
    }

    /// Reads the user's settings, and how Claude starts in the workspace on screen.
    fn read_user(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let look = self.look_now(cx);
        let on_screen = self.frame.read(cx).current_workspace().map(|ws| (ws.id.clone(), ws.name.clone()));
        match &self.source {
            Source::Given(given) => {
                let form = GeneralForm::of(&given.user, look);
                self.preview = given.preview.clone();
                self.general = Some(General::new(&form, window, cx));
            }
            Source::Core => {
                // two questions: the settings are in the core's memory and come back at once; how Claude starts can
                // take a moment (the first time it asks your login shell for PATH: 300 ms in the load test), and the
                // pages must not wait for it
                let read = Bridge::ask(cx, |shared| shared.user_settings());
                cx.spawn_in(window, async move |this: WeakEntity<Self>, cx: &mut AsyncWindowContext| {
                    let Ok(user) = read.await else { return };
                    let _ = this.update_in(cx, |this, window, cx| {
                        let form = GeneralForm::of(&user, look);
                        this.general = Some(General::new(&form, window, cx));
                        cx.notify();
                    });
                })
                .detach();
                if let Some((id, name)) = on_screen {
                    let preview = Bridge::ask(cx, move |shared| enkidu_core::commands::claude_preview(shared, id).ok());
                    cx.spawn(async move |this: WeakEntity<Self>, cx: &mut gpui_kit::AsyncApp| {
                        if let Ok(Some(command)) = preview.await {
                            let _ = this.update(cx, |this, cx| {
                                this.preview = Some((name, command));
                                cx.notify();
                            });
                        }
                    })
                    .detach();
                }
            }
        }
    }

    /// Reads which Claude folders have the hooks for Claude outside the app, and what they add.
    fn read_hooks(&mut self, cx: &mut Context<Self>) {
        match &self.source {
            Source::Given(given) => self.hooks = given.hooks.clone(),
            Source::Core => {
                let read = Bridge::ask(cx, enkidu_core::commands::claude_hooks);
                cx.spawn(async move |this: WeakEntity<Self>, cx: &mut gpui_kit::AsyncApp| {
                    if let Ok(Ok(hooks)) = read.await {
                        let _ = this.update(cx, |this, cx| {
                            this.hooks = Some(hooks);
                            cx.notify();
                        });
                    }
                })
                .detach();
            }
        }
    }

    /// Adds the hooks to a Claude folder's settings, or takes them out: one folder at a time.
    fn set_hooks(&mut self, dir: String, on: bool, cx: &mut Context<Self>) {
        if self.hooks.is_none() || self.hooks_busy.is_some() || matches!(self.source, Source::Given(_)) {
            return;
        }
        self.hooks_busy = Some(dir.clone());
        self.hooks_error = None;
        cx.notify();
        let done = Bridge::ask(cx, move |shared| enkidu_core::commands::set_claude_hooks(shared, dir, on));
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut gpui_kit::AsyncApp| {
            let answer = done.await;
            let _ = this.update(cx, |this, cx| {
                this.hooks_busy = None;
                match answer {
                    Ok(Ok(hooks)) => this.hooks = Some(hooks),
                    Ok(Err(err)) => this.hooks_error = Some(err.message),
                    Err(_) => this.hooks_error = Some("The hooks could not be changed.".into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// A project's page reads that project's settings the first time it is shown.
    fn ensure_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // the Workflows page: what the Work index has of your words for each kind, and of what a workflow starts with
        if self.page == Page::Workflows && self.workflows.is_none() {
            let work = cx.try_global::<crate::stores::Stores>().map(|stores| stores.work.clone());
            let form = match work.as_ref().and_then(|work| work.read(cx).snapshot.clone()) {
                Some(index) => WorkflowsForm::of(&index.flow_kinds, &index.flow_defaults),
                None => WorkflowsForm::of(&Default::default(), &Default::default()),
            };
            self.workflows = Some(WorkflowFields::new(form, window, cx));
        }
        // the Setup page looks at everything the first time it comes on screen
        if self.page == Page::Setup {
            self.ensure_setup(cx);
        }
        let Page::Project(id) = self.page.clone() else { return };
        // its workflows' latest runs, each time the page comes on screen
        self.runs_error = None;
        self.read_runs(&id, cx);
        if !self.asked.insert(id.clone()) {
            return;
        }
        match &self.source {
            Source::Given(given) => {
                let settings = given.projects.get(&id).cloned().unwrap_or_default();
                self.projects.insert(id, ProjectFields::new(&settings, window, cx));
            }
            Source::Core => {
                let asked = id.clone();
                let read = Bridge::ask(cx, move |shared| enkidu_core::commands::get_project_settings(shared, asked));
                cx.spawn_in(window, async move |this: WeakEntity<Self>, cx: &mut AsyncWindowContext| {
                    let Ok(read) = read.await else { return };
                    let _ = this.update_in(cx, |this, window, cx| {
                        match read {
                            Ok(settings) => {
                                this.projects.insert(id, ProjectFields::new(&settings, window, cx));
                            }
                            Err(err) => this.error = Some(err.message),
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
        }
    }

    pub(crate) fn show(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        if self.page == page {
            return;
        }
        self.page = page;
        self.scroll.set_offset(point(px(0.), px(0.)));
        // the field that had the keyboard is on the page that just left: the dialog takes it, so Esc still closes
        window.focus(&self.focus, cx);
        self.ensure_page(window, cx);
        cx.notify();
    }

    fn set_layout(&mut self, layout: Layout, cx: &mut Context<Self>) {
        if let Some(general) = &mut self.general {
            general.look.layout = layout;
            cx.notify();
        }
    }

    fn set_code_colors(&mut self, colors: CodeColors, cx: &mut Context<Self>) {
        if let Some(general) = &mut self.general {
            general.look.code_colors = colors;
            cx.notify();
        }
    }

    /// A press on the minus or the plus beside a size: the code's, or the terminal's.
    fn step_size(&mut self, terminal: bool, up: bool, cx: &mut Context<Self>) {
        if let Some(general) = &mut self.general {
            if terminal {
                general.look.step_terminal_size(up);
            } else {
                general.look.step_code_size(up);
            }
            cx.notify();
        }
    }

    /// Save: the user's settings and every project's page that was edited go to the core, the look to the window.
    /// What can't be read or written is said, and the dialog stays.
    pub fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(general) = &self.general else { return };
        if self.saving {
            return;
        }
        let form = general.form(cx);
        let look = form.look;
        let user = match form.settings() {
            Ok(user) => user,
            Err(why) => return self.failed(why, cx),
        };
        // the Workflows page, when it was shown and something on it differs: your words for the kinds, and what a new
        // workflow starts with
        let flows = match &self.workflows {
            Some(fields) => {
                let now = fields.form(cx);
                match (now != fields.read).then(|| now.read()).transpose() {
                    Ok(flows) => flows,
                    Err(why) => return self.failed(why, cx),
                }
            }
            None => None,
        };
        self.flows_to_keep = flows;
        let mut edited: Vec<(String, ProjectSettings)> = Vec::new();
        let names: HashMap<String, String> =
            self.frame.read(cx).projects.iter().map(|p| (p.id.clone(), p.name.clone())).collect();
        for (id, fields) in &self.projects {
            let form = fields.form(cx);
            if !form.edited() {
                continue;
            }
            match form.settings(names.get(id).map_or("this project", String::as_str)) {
                Ok(settings) => edited.push((id.clone(), settings)),
                Err(why) => return self.failed(why, cx),
            }
        }
        self.error = None;
        if matches!(self.source, Source::Given(_)) {
            self.saved(look, window, cx);
            return;
        }
        self.saving = true;
        cx.notify();
        let written = Bridge::ask(cx, move |shared| {
            enkidu_core::commands::save_user_settings(shared, user).map_err(|err| err.message)?;
            for (id, settings) in edited {
                enkidu_core::commands::save_project_settings(shared, id, settings).map_err(|err| err.message)?;
            }
            Ok::<(), String>(())
        });
        cx.spawn_in(window, async move |this: WeakEntity<Self>, cx: &mut AsyncWindowContext| {
            let outcome = written.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.saving = false;
                match outcome {
                    Ok(Ok(())) => this.saved(look, window, cx),
                    Ok(Err(why)) => this.failed(why, cx),
                    Err(_) => this.failed("The settings could not be saved.".into(), cx),
                }
            });
        })
        .detach();
    }

    fn failed(&mut self, why: String, cx: &mut Context<Self>) {
        self.error = Some(why);
        cx.notify();
    }

    fn saved(&mut self, look: Look, window: &mut Window, cx: &mut Context<Self>) {
        // what a workflow starts from lives with the Work index (the app's database): the page and the workflow dialog
        // have it at once, and the core keeps it
        if let Some((kinds, defaults)) = self.flows_to_keep.take()
            && let Some(work) = cx.try_global::<crate::stores::Stores>().map(|stores| stores.work.clone())
        {
            crate::work::store::Work::save_flow_kinds(&work, kinds, cx).detach();
            crate::work::store::Work::save_flow_defaults(&work, defaults, cx).detach();
        }
        // where the changes go and how code looks live with the window, not in the settings file
        if let Some(ui) = &self.ui {
            ui.update(cx, |ui, cx| {
                if look.apply(ui) {
                    cx.notify();
                }
            });
        }
        // a project's own icon may be another one now, or its letter
        crate::stores::Stores::read_icons(cx);
        window.close_dialog(cx);
        toast::good(window, "Settings saved", cx);
    }

    /// Choose…: a file picker for the project's picture; the file picked stands in the field.
    fn choose_picture(&mut self, project: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(field) = self.projects.get(project).map(|fields| fields.icon_picture.clone()) else { return };
        let picked = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose".into()),
        });
        cx.spawn_in(window, async move |_, cx: &mut AsyncWindowContext| {
            let Ok(Ok(Some(paths))) = picked.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            let path = path.to_string_lossy().to_string();
            let _ = cx.update(|window, cx| field.update(cx, |field, cx| field.set_value(path, window, cx)));
        })
        .detach();
    }

    /// "On the rail": the project's own icon, or its letter.
    fn rail_part(
        &self,
        id: &str,
        fields: &ProjectFields,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let (for_switch, for_picker) = (id.to_string(), id.to_string());
        let shown = switch("rail-icon", "Show the project's own icon", fields.icon, window, cx)
            .on_click(cx.listener(move |this, _, _, cx| this.flip(&for_switch, |fields| &mut fields.icon, cx)));
        let choose = button("rail-picture-choose", "Choose…", ButtonKind::Secondary, ButtonSize::Large)
            .on_click(cx.listener(move |this, _, window, cx| this.choose_picture(&for_picker, window, cx)));
        section(
            "On the rail",
            "The project's button on the rail shows its own icon when it has one, else its letter.",
            vec![
                crate::settings_work::switch_row("Show the project's own icon", shown),
                field(
                    "Another picture",
                    Some(
                        "Optional: a PNG, SVG or ICO file, as a path from the project's folder or a whole one. Empty: \
                         the icon found in the project's folder (.idea/icon.svg, a favicon, an app's icon).",
                    ),
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(div().flex_1().min_w_0().child(text_field(&fields.icon_picture).id("rail-picture")))
                        .child(choose),
                ),
            ],
            false,
        )
    }

    fn nav(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let frame = self.frame.read(cx);
        let (projects, on_screen) = (frame.projects.clone(), frame.current_project.clone());
        let item = |id: gpui_kit::ElementId,
                    page: Page,
                    icon: &'static str,
                    label: String,
                    hint: bool,
                    cx: &mut Context<Self>| {
            let active = page == self.page;
            div()
                .id(id)
                .test_support()
                .role(Role::Tab)
                .aria_label(SharedString::from(label.clone()))
                .aria_selected(active)
                .h(theme::LIST_ROW)
                .flex()
                .flex_none()
                .items_center()
                .gap(px(8.))
                .px(px(8.))
                .rounded(px(6.))
                .text_size(theme::TEXT)
                .cursor_pointer()
                .map(|item| match active {
                    true => item.bg(theme::SELECTED).font_weight(FontWeight::MEDIUM).text_color(theme::INK),
                    false => item.text_color(theme::INK_2).hover(|s| s.bg(theme::HOVER).text_color(theme::INK)),
                })
                .child(svg().path(icon).flex_none().size(theme::ICON).text_color(if active {
                    theme::INK
                } else {
                    theme::INK_3
                }))
                .child(div().flex_1().min_w_0().truncate().child(label))
                .when(hint, |item| {
                    item.child(div().flex_none().text_size(theme::CAPTION).text_color(theme::INK_3).child("current"))
                })
                .on_click(cx.listener(move |this, _, window, cx| this.show(page.clone(), window, cx)))
        };
        let mut list = vec![
            item(
                ("settings-nav", 0usize).into(),
                Page::General,
                "icons/sliders-horizontal.svg",
                "General".into(),
                false,
                cx,
            )
            .into_any_element(),
            item(("settings-nav", 1usize).into(), Page::Claude, "icons/sparkle.svg", "Claude".into(), false, cx)
                .into_any_element(),
            item(
                ("settings-nav", 2usize).into(),
                Page::Workflows,
                "icons/calendar-clock.svg",
                "Workflows".into(),
                false,
                cx,
            )
            .into_any_element(),
            // Setup has a name of its own, so the projects after it keep their numbers
            item("settings-nav-setup".into(), Page::Setup, "icons/circle-check.svg", "Setup".into(), false, cx)
                .into_any_element(),
        ];
        if !projects.is_empty() {
            list.push(
                div()
                    .px(px(8.))
                    .pt(px(16.))
                    .pb(px(8.))
                    .text_size(theme::TEXT_SMALL)
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::INK_3)
                    .child("Projects")
                    .into_any_element(),
            );
        }
        for (ix, project) in projects.iter().enumerate() {
            let current = on_screen.as_deref() == Some(project.id.as_str());
            let page = Page::Project(project.id.clone());
            list.push(
                item(
                    ("settings-nav", 3 + ix).into(),
                    page,
                    "icons/folder-git-2.svg",
                    project.name.clone(),
                    current,
                    cx,
                )
                .into_any_element(),
            );
        }
        div()
            .w(px(232.))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .px(px(8.))
            .py(px(16.))
            .border_r_1()
            .border_color(theme::LINE)
            .bg(theme::PANE)
            // its fill follows the dialog's corner (nothing clips to a rounded corner here)
            .rounded_l(crate::dialog_parts::INNER_RADIUS)
            .child(
                div()
                    .px(px(8.))
                    .pb(px(16.))
                    .text_size(theme::DIALOG_TITLE)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::INK)
                    .child("Settings"),
            )
            .children(list)
    }

    fn general_page(&self, general: &General, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let choice = |id: &'static str, words: &'static str, layout: Layout, cx: &mut Context<Self>| -> Observed {
            word_segment(id, words, general.look.layout == layout)
                .on_click(cx.listener(move |this, _, _, cx| this.set_layout(layout, cx)))
        };
        let choices = vec![
            choice("settings-layout-stack", "Above the terminal", Layout::Stack, cx),
            choice("settings-layout-side", "Beside the terminal", Layout::Side, cx),
        ];
        let selected = usize::from(general.look.layout == Layout::Side);
        let switch = div().flex().child(word_switch("settings-layout", selected, choices, window, cx));
        // the code's colors (item 8.3): an IDE's, or the quiet ones
        let colors = |id: &'static str, words: &'static str, colors: CodeColors, cx: &mut Context<Self>| -> Observed {
            word_segment(id, words, general.look.code_colors == colors)
                .on_click(cx.listener(move |this, _, _, cx| this.set_code_colors(colors, cx)))
        };
        let color_choices = vec![
            colors("settings-colors-ide", "Like an IDE", CodeColors::Ide, cx),
            colors("settings-colors-quiet", "Quiet", CodeColors::Quiet, cx),
        ];
        let color_switch = div().flex().child(word_switch(
            "settings-colors",
            usize::from(general.look.code_colors == CodeColors::Quiet),
            color_choices,
            window,
            cx,
        ));
        // the two sizes (item 8.4): a minus, the size in words, a plus
        let size = |name: &'static str, terminal: bool, size: f32, cx: &mut Context<Self>| {
            let step = |id: String, icon: &'static str, label: String, up: bool, cx: &mut Context<Self>| {
                icon_button(SharedString::from(id), icon, label)
                    .on_click(cx.listener(move |this, _, _, cx| this.step_size(terminal, up, cx)))
            };
            let words = size_words(size);
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(step(
                    format!("settings-{name}-smaller"),
                    "icons/minus.svg",
                    format!("Smaller {name} text"),
                    false,
                    cx,
                ))
                .child(
                    tabular(div())
                        .id(SharedString::from(format!("settings-{name}-size")))
                        .test_support()
                        .aria_label(SharedString::from(words.clone()))
                        .w(px(56.))
                        .flex()
                        .justify_center()
                        .text_size(theme::TEXT)
                        .text_color(theme::INK)
                        .child(words),
                )
                .child(step(
                    format!("settings-{name}-bigger"),
                    "icons/plus.svg",
                    format!("Bigger {name} text"),
                    true,
                    cx,
                ))
        };
        let sizes = div()
            .flex()
            .gap(px(40.))
            .child(field("Code", None, size("code", false, general.look.code_size as f32, cx)))
            .child(field("Terminal", None, size("terminal", true, general.look.terminal_size, cx)));
        div()
            .flex()
            .flex_col()
            .gap(px(24.))
            .child(section(
                "Layout",
                "Can also be switched from the top row, the command palette or the keyboard.",
                vec![field("Changes", Some("⌥⌘L switches. Above the terminal gives diffs the full width."), switch)],
                true,
            ))
            .child(section(
                "Code",
                "How code looks in a diff and in an opened file, and the size of text there and in the terminal.",
                vec![
                    field(
                        "Colors",
                        Some("Like an IDE gives keywords, strings, numbers and definitions each their own color, as PyCharm does. Quiet keeps code nearly white."),
                        color_switch,
                    ),
                    field(
                        "Text size",
                        Some("⌘+ and ⌘− change the one the keyboard is in, and ⌘0 puts it back."),
                        sizes,
                    ),
                ],
                false,
            ))
            .child(section(
                "Workspaces",
                "Each project gets a folder in here, with one folder per workspace.",
                vec![field("Workspaces folder", None, text_field(&general.workspaces_dir).id("workspaces-dir"))],
                false,
            ))
            .into_any_element()
    }

    /// The Workflows page (item 8.6): what a new workflow starts with, and the words each kind of task starts from.
    fn workflows_page(&self, fields: &WorkflowFields, cx: &mut Context<Self>) -> AnyElement {
        let short = |state: &Entity<InputState>, id: &'static str| div().w(px(96.)).child(text_field(state).id(id));
        let starts = div()
            .flex()
            .gap(px(32.))
            .child(field(
                "A task gets up to",
                Some("Minutes, from 5 to 480."),
                short(&fields.minutes, "workflow-minutes"),
            ))
            .child(field("Starts at", Some("A time of day, like 02:00."), short(&fields.time, "workflow-time")));
        let kinds: Vec<gpui_kit::Div> = (fields.kinds.iter())
            .map(|(kind, label, words)| {
                // while the words are your own, one press puts Enkidu's back
                let usual = WorkflowsForm::is_yours(kind, &words.read(cx).value()).then(|| {
                    let (kind, field) = (kind.clone(), words.clone());
                    div().flex().child(
                        div()
                            .id(SharedString::from(format!("workflow-usual-{kind}")))
                            .test_support()
                            .role(Role::Button)
                            .aria_label("Use Enkidu's words")
                            .text_size(theme::TEXT_SMALL)
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::INK_3)
                            .cursor_pointer()
                            .hover(|s| s.text_color(theme::ACCENT))
                            .child("Use Enkidu's words")
                            .on_click(cx.listener(move |_, _, window, cx| {
                                let usual = WorkflowsForm::usual(&kind);
                                field.update(cx, |field, cx| field.set_value(usual, window, cx));
                                cx.notify();
                            })),
                    )
                });
                let area = div()
                    .id(SharedString::from(format!("workflow-words-{kind}")))
                    .test_support()
                    .child(text_area(words, false));
                field_named(label.clone(), None, div().flex().flex_col().gap(px(8.)).child(area).children(usual))
            })
            .collect();
        div()
            .flex()
            .flex_col()
            .gap(px(24.))
            .child(section(
                "A new workflow",
                "What the workflow dialog starts with. Each workflow can still say otherwise.",
                vec![starts],
                true,
            ))
            .child(section(
                "What each kind of task starts from",
                "When you make a workflow and pick what a task does, its words start from these. Change them here once; you can still change them for one task in the workflow. A field you empty goes back to Enkidu's words.",
                kinds,
                false,
            ))
            .into_any_element()
    }

    fn claude_page(&self, general: &General, cx: &mut Context<Self>) -> AnyElement {
        let preview = self.preview.as_ref().map(|(workspace, command)| {
            field_named(
                format!("Claude starts like this in {workspace}"),
                None,
                mono(div())
                    .id("claude-preview")
                    .test_support()
                    .aria_label(SharedString::from(command.clone()))
                    .px(px(12.))
                    .py(px(12.))
                    .rounded(px(8.))
                    .bg(theme::PANE)
                    .text_size(theme::TEXT_STATUS)
                    .line_height(px(18.))
                    .text_color(theme::INK_2)
                    // (it can be selected and copied)
                    .child(enkidu_ui::selectable::words("claude-preview-words", command.clone())),
            )
        });
        let starts = vec![
            div()
                .flex()
                .gap(px(12.))
                // a third and two thirds
                .child(div().flex_1().min_w_0().child(field(
                    "Command",
                    None,
                    text_field(&general.command).id("claude-command"),
                )))
                .child(div().flex_1().flex_grow(2.).min_w_0().child(field(
                    "Arguments",
                    Some("Used in every project, before the project's own."),
                    text_field(&general.args).id("claude-args"),
                ))),
            field(
                "Environment",
                Some("KEY=value, one per line. CLAUDE_CONFIG_DIR picks another Claude account."),
                area(&general.env, "claude-env", true),
            ),
        ];
        div()
            .flex()
            .flex_col()
            .gap(px(24.))
            .child(
                section(
                    "How Claude starts",
                    "Enkidu adds only its own flags (its hooks, the session, the name and a note on what its command, \
                     enkidu, is for), then yours.",
                    starts,
                    true,
                )
                .children(preview),
            )
            .children(self.hooks.as_ref().map(|hooks| self.hooks_section(hooks, cx)))
            .into_any_element()
    }

    /// "Claude outside Enkidu" (item 4.4): a row for each Claude folder with its
    /// settings file, "Added" once the hooks are there, and one button; under them, exactly what is added.
    fn hooks_section(&self, hooks: &ClaudeHooks, cx: &mut Context<Self>) -> gpui_kit::Div {
        let small = |words: &str| {
            div().text_size(theme::TEXT_SMALL).line_height(px(16.)).child(with_code(words, theme::INK_3, theme::INK_2))
        };
        let description = small(
            "Claude you start yourself (an alias in another terminal, say) can tell Enkidu which PRs and tickets it works on, so \
             they show on the Work page with their session. This adds five hooks to that folder's settings.json, each \
             running `enkidu event`, which writes only to Enkidu's own event log. The file is first kept as \
             settings.json.before-enkidu.",
        );
        let mut parts: Vec<AnyElement> = Vec::new();
        if hooks.cli.is_none() {
            // a broken build: there is nothing the hooks could run
            let words = "This build has no `enkidu` command of its own. `sh scripts/install.sh` puts it inside Enkidu.";
            parts.push(small(words).id("claude-hooks-no-cli").test_support().aria_label(words).into_any_element());
        } else {
            let busy = self.hooks_busy.clone();
            let can = busy.is_none();
            let rows = hooks.files.iter().enumerate().map(|(ix, file)| {
                let (dir, added) = (file.dir.clone(), file.added);
                let words = match (busy.as_deref() == Some(file.dir.as_str()), added) {
                    (true, _) => "…",
                    (false, true) => "Remove",
                    (false, false) => "Add hooks",
                };
                div()
                    .id(("claude-hooks-row", ix))
                    .test_support()
                    .aria_label(SharedString::from(file.path.clone()))
                    .when(added, |row| row.aria_description("Added"))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .rounded(px(8.))
                    .bg(theme::HOVER)
                    .px(px(12.))
                    .py(px(8.))
                    .child(
                        mono(div())
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(theme::MONO_TEXT)
                            .text_color(theme::INK_2)
                            .child(enkidu_model::text::short_path(&file.path)),
                    )
                    .when(added, |row| {
                        row.child(
                            div().flex_none().text_size(theme::TEXT_SMALL).text_color(theme::LAMP_GREEN).child("Added"),
                        )
                    })
                    .child(
                        button(("claude-hooks-toggle", ix), words, ButtonKind::Secondary, ButtonSize::Small)
                            .when(!can, |button| button.opacity(0.45).cursor_default())
                            .on_click(cx.listener(move |this, _, _, cx| this.set_hooks(dir.clone(), !added, cx))),
                    )
            });
            let rows: Vec<AnyElement> = rows.map(IntoElement::into_any_element).collect();
            let shown = self.hooks_shown;
            let show = if shown { "Hide what it adds" } else { "Show what it adds" };
            let list = div()
                .id("claude-hooks")
                .test_support()
                .flex()
                .flex_col()
                .gap(px(8.))
                .children(rows)
                .child(
                    div().flex().child(
                        div()
                            .id("claude-hooks-show")
                            .test_support()
                            .role(Role::Button)
                            .aria_label(show)
                            .text_size(theme::TEXT_SMALL)
                            .line_height(theme::TEXT_SMALL_LINE)
                            .text_color(theme::INK_3)
                            .hover(|s| s.text_color(theme::INK))
                            .cursor_pointer()
                            .child(show)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.hooks_shown = !this.hooks_shown;
                                cx.notify();
                            })),
                    ),
                )
                .children(hooks.adds.clone().filter(|_| shown).map(|adds| {
                    mono(div())
                        .id("claude-hooks-adds")
                        .test_support()
                        .aria_label(SharedString::from(adds.clone()))
                        .max_h(px(220.))
                        .overflow_y_scroll()
                        .rounded(px(8.))
                        .bg(theme::PANE)
                        .px(px(12.))
                        .py(px(12.))
                        .text_size(px(11.))
                        .line_height(px(16.))
                        .text_color(theme::INK_2)
                        .child(enkidu_ui::selectable::words("claude-hooks-adds-words", adds))
                }));
            parts.push(list.into_any_element());
        }
        if let Some(error) = &self.hooks_error {
            parts.push(
                div()
                    .id("claude-hooks-error")
                    .test_support()
                    .aria_label(SharedString::from(error.clone()))
                    .text_size(theme::TEXT_SMALL)
                    .text_color(theme::LAMP_RED)
                    .child(error.clone())
                    .into_any_element(),
            );
        }
        section_with("Claude outside Enkidu", description.into_any_element(), parts, false)
    }

    fn project_page(
        &self,
        id: &str,
        fields: &ProjectFields,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let code = |state: &Entity<TextareaState>, id: &'static str| area(state, id, true);
        div()
            .flex()
            .flex_col()
            .gap(px(24.))
            .child(section(
                "New workspaces",
                "What every new workspace of this project starts with.",
                vec![
                    field(
                        "Default base branch",
                        Some("Empty: development if it exists, else origin's default branch."),
                        text_field(&fields.default_base).id("default-base"),
                    ),
                    field(
                        "Files to copy",
                        Some(
                            "Paths or globs from the main folder, one per line. Existing files are never overwritten.",
                        ),
                        code(&fields.copy_files, "copy-files"),
                    ),
                    field(
                        "Setup script",
                        Some("Runs in each new workspace with your login shell."),
                        code(&fields.setup_script, "setup-script"),
                    ),
                ],
                true,
            ))
            .child(section(
                "Live",
                "Go live copies a workspace's changes into the main folder, where your dev environment runs, and \
                 keeps copying them while it is Live. Stop puts the main folder back as it was.",
                vec![
                    div()
                        .flex()
                        .gap(px(12.))
                        .child(div().flex_1().min_w_0().child(field(
                            "After the copy",
                            Some(
                                "Optional. Runs in the main folder, to restart a server say. {path} is the workspace.",
                            ),
                            code(&fields.live_on, "live-on"),
                        )))
                        .child(div().flex_1().min_w_0().child(field(
                            "After putting it back",
                            Some("Optional. Runs once the main folder is back as it was."),
                            code(&fields.live_off, "live-off"),
                        ))),
                    field(
                        "Told to Claude in the Live workspace",
                        None,
                        area(&fields.live_message, "live-message", false),
                    ),
                ],
                false,
            ))
            .child(section(
                "Claude in this project",
                "Added after the general Claude settings.",
                vec![
                    field("Arguments", None, text_field(&fields.args).id("project-claude-args")),
                    field(
                        "Environment",
                        Some("Overrides the general environment."),
                        code(&fields.env, "project-claude-env"),
                    ),
                ],
                false,
            ))
            .child(self.rail_part(id, fields, window, cx))
            // what is read from GitHub and Jira for the Work page, and the project's workflows
            .child(self.work_part(id, fields, window, cx))
            .child(self.workflows_part(id, fields, window, cx))
            .into_any_element()
    }
}

/// A part of a page: its title, a line on what it is, and its fields. Every part after the first has a hairline
/// above it.
pub(crate) fn section(
    title: &'static str,
    description: &'static str,
    fields: Vec<gpui_kit::Div>,
    first: bool,
) -> gpui_kit::Div {
    let description =
        div().text_size(theme::TEXT_SMALL).line_height(px(16.)).text_color(theme::INK_3).child(description);
    let fields = fields.into_iter().map(IntoElement::into_any_element).collect();
    section_with(title, description.into_any_element(), fields, first)
}

/// The same with the line under the title and the fields as they come (words with code in them, rows).
pub(crate) fn section_with(
    title: &'static str,
    description: AnyElement,
    fields: Vec<AnyElement>,
    first: bool,
) -> gpui_kit::Div {
    div()
        .flex()
        .flex_col()
        .gap(px(16.))
        .when(!first, |section| section.pt(px(24.)).border_t_1().border_color(theme::LINE))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(
                    div()
                        .text_size(theme::TEXT)
                        .line_height(px(20.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::INK)
                        .child(title),
                )
                .child(description),
        )
        .children(fields)
}

/// A field of several lines, under `id`; `code` puts it in the code face (a script, an environment, paths).
pub(crate) fn area(state: &Entity<TextareaState>, id: &'static str, code: bool) -> Observed {
    div().id(id).test_support().child(text_area(state, code))
}

/// A field: its name, the control, and a line under it on what it takes.
pub(crate) fn field(label: &'static str, hint: Option<&'static str>, control: impl IntoElement) -> gpui_kit::Div {
    field_named(label.to_string(), hint, control)
}

fn field_named(label: String, hint: Option<&'static str>, control: impl IntoElement) -> gpui_kit::Div {
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .min_w_0()
        .child(div().text_size(theme::TEXT_SMALL).font_weight(FontWeight::MEDIUM).text_color(theme::INK).child(label))
        .child(control)
        .children(
            hint.map(|hint| {
                div().text_size(theme::TEXT_SMALL).line_height(px(16.)).text_color(theme::INK_3).child(hint)
            }),
        )
}

impl Render for Settings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let projects = self.frame.read(cx).projects.clone();
        let title = self.page.title(&projects);
        // at most 620 px, or 84% of a short window
        let height = (f32::from(window.viewport_size().height) * 0.84).min(620.);
        let body: Option<AnyElement> = match (&self.page, &self.general) {
            (Page::General, Some(general)) => Some(self.general_page(general, window, cx)),
            (Page::Claude, Some(general)) => Some(self.claude_page(general, cx)),
            (Page::Workflows, _) => self.workflows.as_ref().map(|fields| self.workflows_page(fields, cx)),
            (Page::Setup, _) => Some(self.setup_page(cx)),
            (Page::Project(id), _) => self.projects.get(id).map(|fields| self.project_page(id, fields, window, cx)),
            _ => None,
        };
        let error = self.error.clone().map(|error| {
            div()
                .id("settings-error")
                .test_support()
                .aria_label(SharedString::from(error.clone()))
                .flex_1()
                .min_w_0()
                .text_size(theme::TEXT_SMALL)
                .text_color(theme::LAMP_RED)
                .child(error)
        });
        let ready = self.general.is_some() && !self.saving;
        // the first launch's welcome saves nothing: its Setup page has one button, Done. Every other page, and Setup
        // opened from the list, has Cancel and Save
        let foot: Vec<AnyElement> = if self.setup.welcome && self.page == Page::Setup {
            vec![
                button("setup-done", "Done", ButtonKind::Primary, ButtonSize::Large)
                    .on_click(|_, window, cx| window.close_dialog(cx))
                    .into_any_element(),
            ]
        } else {
            vec![
                button("settings-cancel", "Cancel", ButtonKind::Secondary, ButtonSize::Large)
                    .on_click(|_, window, cx| window.close_dialog(cx))
                    .into_any_element(),
                button("save-settings", "Save", ButtonKind::Primary, ButtonSize::Large)
                    .when(!ready, |save| save.opacity(0.45).cursor_default())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if ready {
                            this.save(window, cx);
                        }
                    }))
                    .into_any_element(),
            ]
        };
        div()
            .id("settings-dialog")
            .test_support()
            .aria_label("Settings")
            .aria_description(SharedString::from(title.clone()))
            .track_focus(&self.focus)
            .relative()
            .h(px(height))
            .w_full()
            .flex()
            .overflow_hidden()
            .child(self.nav(cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .h(px(48.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_between()
                            .pl(px(24.))
                            .pr(px(12.))
                            .border_b_1()
                            .border_color(theme::LINE)
                            .child(
                                div()
                                    .id("settings-title")
                                    .test_support()
                                    .aria_label(SharedString::from(title.clone()))
                                    .text_size(theme::DIALOG_TITLE)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme::INK)
                                    .child(title),
                            )
                            .child(
                                div()
                                    .id("settings-close")
                                    .test_support()
                                    .role(Role::Button)
                                    .aria_label("Close")
                                    .size(theme::CONTROL)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(6.))
                                    .cursor_pointer()
                                    .hover(|s| s.bg(theme::HOVER))
                                    .child(svg().path("icons/x.svg").size(theme::ICON).text_color(theme::INK_3))
                                    .on_click(|_, window, cx| window.close_dialog(cx)),
                            ),
                    )
                    .child(
                        div()
                            .id("settings-body")
                            .test_support()
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll)
                            .px(px(24.))
                            .py(px(24.))
                            .children(body),
                    )
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap(px(8.))
                            .px(px(24.))
                            .py(px(16.))
                            .border_t_1()
                            .border_color(theme::LINE)
                            .children(error)
                            .children(foot),
                    ),
            )
    }
}

/// Opens Settings on the app's own settings: the core's, or in a window without a core (a picture) sample ones.
pub fn open_here(frame: Entity<Frame>, page: Page, window: &mut Window, cx: &mut App) {
    let source = if cx.has_global::<Bridge>() { Source::Core } else { crate::demo::settings(cx) };
    open(frame, page, source, window, cx);
}
