//! Settings (item 4.3a): what the dialog edits, as the text in its fields, made from the core's settings and turned
//! back into them when Save is pressed. The user's settings are on two pages (General, Claude), and each project has a
//! page of its own. A field that can't be read (a quote that isn't closed in Claude's arguments, a line of the
//! environment that isn't `KEY=value`) is said with the name of its field, and nothing is saved.
//!
//! The layout is on the General page too, but it lives with the window (`ui.json`), not in the settings file. A
//! project's page ends with its Work settings (what is read from GitHub and Jira) and its workflows.

use std::collections::BTreeMap;

use enkidu_core::db::Project;
use enkidu_core::settings::{ProjectSettings, UserClaude, UserSettings};
use enkidu_core::work::flows::FlowDefaults;

use crate::text::{format_env, join_args, parse_env, parse_lines, split_args};
use crate::ui::{self, CodeColors, Layout, Ui};

/// A page of the dialog.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Page {
    General,
    Claude,
    /// What a workflow starts from: the words of each kind of task, the minutes, the time (item 8.6).
    Workflows,
    /// What the app needs on this Mac, and how each thing stands (item 21.5; DESIGN.md, "Setup").
    Setup,
    /// A project's page, by the project's id.
    Project(String),
}

impl Page {
    /// What the page is called, at the top of it and in the list of pages.
    pub fn title(&self, projects: &[Project]) -> String {
        match self {
            Page::General => "General".into(),
            Page::Claude => "Claude".into(),
            Page::Workflows => "Workflows".into(),
            Page::Setup => "Setup".into(),
            Page::Project(id) => projects.iter().find(|p| &p.id == id).map(|p| p.name.clone()).unwrap_or_default(),
        }
    }

    /// The pages in the order the list has them: General, Claude, Workflows, Setup, then the projects.
    pub fn all(projects: &[Project]) -> Vec<Page> {
        let mut pages = vec![Page::General, Page::Claude, Page::Workflows, Page::Setup];
        pages.extend(projects.iter().map(|p| Page::Project(p.id.clone())));
        pages
    }
}

fn env_text(env: &BTreeMap<String, String>) -> String {
    let pairs: Vec<(String, String)> = env.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    format_env(&pairs)
}

fn env_of(text: &str, field: &str) -> Result<BTreeMap<String, String>, String> {
    parse_env(text).map(|pairs| pairs.into_iter().collect()).map_err(|why| format!("{field}: {why}"))
}

fn args_of(text: &str, field: &str) -> Result<Vec<String>, String> {
    split_args(text).map_err(|why| format!("{field}: {why}"))
}

/// What the General page edits that lives with the window (`ui.json`), not in the settings file: where the changes
/// go, and how code and the terminal's text look (items 8.3 and 8.4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub layout: Layout,
    pub code_colors: CodeColors,
    /// The size of code in a diff and in an opened file, in px.
    pub code_size: u32,
    /// The size of the terminal's text, in px.
    pub terminal_size: f32,
}

impl Default for Look {
    fn default() -> Look {
        Look::of(&Ui::default())
    }
}

impl Look {
    /// As the window has it now.
    pub fn of(ui: &Ui) -> Look {
        Look {
            layout: ui.layout,
            code_colors: ui.code_colors,
            code_size: ui.code_size,
            terminal_size: ui.terminal_size,
        }
    }

    /// Save was pressed: the window gets it. False when nothing differed.
    pub fn apply(&self, ui: &mut Ui) -> bool {
        let was = Look::of(ui);
        ui.layout = self.layout;
        ui.code_colors = self.code_colors;
        ui.set_code_size(self.code_size);
        ui.set_terminal_size(self.terminal_size);
        Look::of(ui) != was
    }

    /// The code a size up or down, within what the window allows.
    pub fn step_code_size(&mut self, up: bool) {
        let size = if up { self.code_size + 1 } else { self.code_size.saturating_sub(1) };
        self.code_size = size.clamp(ui::MIN_CODE_SIZE, ui::MAX_CODE_SIZE);
    }

    /// The terminal's text a size up or down, within what the window allows.
    pub fn step_terminal_size(&mut self, up: bool) {
        let size = self.terminal_size + if up { 1. } else { -1. };
        self.terminal_size = size.clamp(ui::MIN_TERMINAL_SIZE, ui::MAX_TERMINAL_SIZE);
    }
}

/// A size in px as Settings writes it: "12 px", "12.5 px".
pub fn size_words(size: f32) -> String {
    if size.fract() == 0. { format!("{size:.0} px") } else { format!("{size:.1} px") }
}

/// The General and Claude pages: the user's settings, and what the window keeps of how it looks.
#[derive(Clone, Debug, PartialEq)]
pub struct GeneralForm {
    pub look: Look,
    pub workspaces_dir: String,
    pub command: String,
    pub args: String,
    pub env: String,
}

impl GeneralForm {
    pub fn of(settings: &UserSettings, look: Look) -> GeneralForm {
        GeneralForm {
            look,
            workspaces_dir: settings.workspaces_dir.clone(),
            command: settings.claude.command.clone(),
            args: join_args(&settings.claude.args),
            env: env_text(&settings.claude.env),
        }
    }

    /// The user's settings as the fields say them, or which field can't be read.
    pub fn settings(&self) -> Result<UserSettings, String> {
        Ok(UserSettings {
            workspaces_dir: self.workspaces_dir.trim().to_string(),
            claude: UserClaude {
                command: self.command.trim().to_string(),
                args: args_of(&self.args, "Claude's arguments")?,
                env: env_of(&self.env, "Claude's environment")?,
            },
        })
    }
}

/// The Workflows page (item 8.6): what each kind of task starts from, in words, and what a new workflow
/// starts with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkflowsForm {
    /// "A task gets up to" this many minutes, as typed.
    pub minutes: String,
    /// "Starts at", as typed ("02:00").
    pub time: String,
    /// Each kind of task with words of its own, in the order the workflow dialog lists them: its id, what it is
    /// called, and its words (yours where you changed them, else Enkidu's).
    pub kinds: Vec<(String, String, String)>,
}

impl WorkflowsForm {
    /// The page as it opens: your words where you have some, Enkidu's elsewhere.
    pub fn of(mine: &BTreeMap<String, String>, defaults: &FlowDefaults) -> WorkflowsForm {
        use crate::work::flows::{kind_label, kind_text};
        let kinds = (enkidu_core::work::flows::KINDS.iter())
            .filter(|kind| **kind != "custom")
            .map(|kind| (kind.to_string(), kind_label(kind).to_string(), kind_text(kind, mine)))
            .collect();
        WorkflowsForm { minutes: defaults.minutes.to_string(), time: defaults.time.clone(), kinds }
    }

    /// Enkidu's own words for a kind ("Use Enkidu's words" puts them back in the field).
    pub fn usual(kind: &str) -> String {
        crate::work::flows::kind_text(kind, &BTreeMap::new())
    }

    /// Whether a kind's words are your own now.
    pub fn is_yours(kind: &str, words: &str) -> bool {
        let words = words.trim();
        !words.is_empty() && words != Self::usual(kind).trim()
    }

    /// What Save keeps: your own words for the kinds whose words differ from Enkidu's (an emptied field goes back
    /// to Enkidu's), and what a new workflow starts with. Or which field can't be read.
    pub fn read(&self) -> Result<(BTreeMap<String, String>, FlowDefaults), String> {
        let (least, most) = crate::work::flows::MINUTES;
        let minutes: u32 = (self.minutes.trim().parse().ok())
            .filter(|minutes| (least..=most).contains(minutes))
            .ok_or_else(|| format!("Workflows: a task gets between {least} and {most} minutes"))?;
        let time = self.time.trim();
        if enkidu_core::work::flows::clock(time).is_none() {
            return Err("Workflows: the start time is a time of day, like 02:00".into());
        }
        let mine = (self.kinds.iter())
            .filter(|(kind, _, words)| Self::is_yours(kind, words))
            .map(|(kind, _, words)| (kind.clone(), words.trim().to_string()))
            .collect();
        Ok((mine, FlowDefaults { minutes, time: time.to_string() }))
    }
}

/// A project's page.
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectForm {
    pub default_base: String,
    pub copy_files: String,
    pub setup_script: String,
    pub live_on: String,
    pub live_off: String,
    pub live_message: String,
    pub args: String,
    pub env: String,
    /// Work: read this project's PRs from GitHub.
    pub work_github: bool,
    /// The `gh` account that reads the repo; empty for the first that can.
    pub work_account: String,
    /// Jira's project keys, with commas or spaces between them: "SH, OPS". Empty: Jira isn't read.
    pub work_jira_keys: String,
    /// The Jira statuses that mean a ticket came back to you, with commas between them.
    pub work_back: String,
    /// The checks that never count, one a line.
    pub work_ignored: String,
    /// Minutes between reads, as typed.
    pub work_every: String,
    /// Workflows: new failing checks go to their Claude; the morning summary and its time ("09:00").
    pub send_failed_checks: bool,
    pub morning_summary: bool,
    pub morning_at: String,
    /// The settings the form was made from: what it doesn't edit is saved as it was.
    read: ProjectSettings,
}

/// The minutes between reads as typed: the number the text starts with, 5 when it starts with none (or is 0), and never
/// under 1 or over 240.
pub fn every_minutes(text: &str) -> u32 {
    let text = text.trim_start();
    let (negative, rest) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    // more digits than a count of minutes can have: far over the most
    let typed: i64 = if digits.len() > 9 { i64::from(u32::MAX) } else { digits.parse().unwrap_or(0) };
    let minutes = match (typed, negative) {
        (0, _) => 5,
        (typed, true) => -typed,
        (typed, false) => typed,
    };
    minutes.clamp(1, 240) as u32
}

impl ProjectForm {
    pub fn of(settings: &ProjectSettings) -> ProjectForm {
        ProjectForm {
            default_base: settings.default_base.clone(),
            copy_files: settings.copy_files.join("\n"),
            setup_script: settings.setup_script.clone(),
            live_on: settings.live.on.clone(),
            live_off: settings.live.off.clone(),
            live_message: settings.live.message.clone(),
            args: join_args(&settings.claude.args),
            env: env_text(&settings.claude.env),
            work_github: settings.work.github,
            work_account: settings.work.github_account.clone(),
            work_jira_keys: settings.work.jira_keys.join(", "),
            work_back: settings.work.back_to_you.join(", "),
            work_ignored: settings.work.ignored_checks.join("\n"),
            work_every: settings.work.every_minutes.to_string(),
            send_failed_checks: settings.work.send_failed_checks,
            morning_summary: settings.work.morning_summary,
            morning_at: settings.work.morning_at.clone(),
            read: settings.clone(),
        }
    }

    /// The project's settings as the fields say them, or which field can't be read. `name` is the project's, to
    /// say whose field it is.
    pub fn settings(&self, name: &str) -> Result<ProjectSettings, String> {
        let mut settings = self.read.clone();
        settings.default_base = self.default_base.trim().to_string();
        settings.copy_files = parse_lines(&self.copy_files);
        settings.setup_script = self.setup_script.clone();
        settings.live.on = self.live_on.clone();
        settings.live.off = self.live_off.clone();
        settings.live.message = self.live_message.clone();
        settings.claude.args = args_of(&self.args, &format!("Claude's arguments in {name}"))?;
        settings.claude.env = env_of(&self.env, &format!("Claude's environment in {name}"))?;
        let between_commas = |text: &str| -> Vec<String> {
            text.split(',').map(str::trim).filter(|part| !part.is_empty()).map(String::from).collect()
        };
        settings.work.github = self.work_github;
        settings.work.github_account = self.work_account.trim().to_string();
        // Jira's keys are capitals, whatever was typed, with commas or spaces between them
        settings.work.jira_keys = (self.work_jira_keys.split(|c: char| c.is_whitespace() || c == ','))
            .map(|key| key.trim().to_uppercase())
            .filter(|key| !key.is_empty())
            .collect();
        settings.work.back_to_you = between_commas(&self.work_back);
        settings.work.ignored_checks = parse_lines(&self.work_ignored);
        settings.work.every_minutes = every_minutes(&self.work_every);
        settings.work.send_failed_checks = self.send_failed_checks;
        settings.work.morning_summary = self.morning_summary;
        settings.work.morning_at = self.morning_at.trim().to_string();
        Ok(settings)
    }

    /// Whether a field was edited (a page that was only looked at is not saved again).
    pub fn edited(&self) -> bool {
        *self != ProjectForm::of(&self.read)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(id: &str, name: &str) -> Project {
        Project {
            id: id.into(),
            name: name.into(),
            path: format!("/code/{name}"),
            live_workspace_id: None,
            created_at: 1,
        }
    }

    #[test]
    fn the_pages_are_general_claude_workflows_setup_and_one_for_each_project() {
        let projects = [project("p1", "shop"), project("p2", "tools")];
        let pages = Page::all(&projects);
        let (first, second) = (Page::Project("p1".into()), Page::Project("p2".into()));
        assert_eq!(pages, [Page::General, Page::Claude, Page::Workflows, Page::Setup, first, second]);
        let titles: Vec<String> = pages.iter().map(|page| page.title(&projects)).collect();
        assert_eq!(titles, ["General", "Claude", "Workflows", "Setup", "shop", "tools"]);
        assert_eq!(Page::all(&[]), [Page::General, Page::Claude, Page::Workflows, Page::Setup]);
    }

    /// Item 8.6: the Workflows page shows what each kind of task starts from, and Save
    /// keeps only what differs from Enkidu's own.
    #[test]
    fn the_workflows_page_edits_each_kinds_words_and_what_a_new_workflow_starts_with() {
        use crate::work::flows::kind_text;
        let none = BTreeMap::new();
        let form = WorkflowsForm::of(&none, &FlowDefaults::default());
        assert_eq!((form.minutes.as_str(), form.time.as_str()), ("60", "02:00"));
        let listed: Vec<(&str, &str)> =
            form.kinds.iter().map(|(kind, label, _)| (kind.as_str(), label.as_str())).collect();
        assert_eq!(
            listed,
            [
                ("start", "Start it"),
                ("continue", "Carry on"),
                ("came_back", "Fix what came back"),
                ("review", "Answer the review"),
                ("checks", "Fix the failing checks"),
                ("investigate", "Investigate only"),
            ],
            "every kind but Your own words, which has none"
        );
        assert!(form.kinds.iter().all(|(kind, _, words)| *words == kind_text(kind, &none) && !words.is_empty()));
        // untouched, nothing is yours: Save keeps no words, and the usual start
        assert_eq!(form.read(), Ok((BTreeMap::new(), FlowDefaults::default())));
        // your own words for one kind, another emptied, the minutes and the time changed
        let mut edited = form.clone();
        edited.kinds[0].2 = "  Read the ticket, then ask me before you change anything.  ".into();
        edited.kinds[2].2 = "   ".into();
        edited.minutes = " 45 ".into();
        edited.time = "23:30".into();
        let (mine, defaults) = edited.read().unwrap();
        let kept = BTreeMap::from([(
            "start".to_string(),
            "Read the ticket, then ask me before you change anything.".to_string(),
        )]);
        assert_eq!(mine, kept, "an emptied field goes back to Enkidu's words");
        assert_eq!(defaults, FlowDefaults { minutes: 45, time: "23:30".into() });
        // opened again, the page shows your words there and Enkidu's elsewhere
        let again = WorkflowsForm::of(&mine, &defaults);
        assert_eq!(again.kinds[0].2, "Read the ticket, then ask me before you change anything.");
        assert_eq!(again.kinds[2].2, kind_text("came_back", &none));
        assert_eq!((again.minutes.as_str(), again.time.as_str()), ("45", "23:30"));
        assert!(
            WorkflowsForm::is_yours("start", &again.kinds[0].2)
                && !WorkflowsForm::is_yours("review", &again.kinds[3].2)
        );
        assert_eq!(WorkflowsForm::usual("start"), kind_text("start", &none));
        // what can't be read is said, and nothing is kept
        let wrong = |minutes: &str, time: &str| {
            WorkflowsForm { minutes: minutes.into(), time: time.into(), ..form.clone() }.read().unwrap_err()
        };
        assert_eq!(wrong("2", "02:00"), "Workflows: a task gets between 5 and 480 minutes");
        assert_eq!(wrong("an hour", "02:00"), "Workflows: a task gets between 5 and 480 minutes");
        assert_eq!(wrong("60", "25:00"), "Workflows: the start time is a time of day, like 02:00");
        assert_eq!(wrong("60", "tonight"), "Workflows: the start time is a time of day, like 02:00");
    }

    /// Items 8.3 and 8.4: the code's colors and the two sizes are on the General page, and live with the window.
    #[test]
    fn the_look_is_read_from_the_window_and_put_back_when_saved() {
        let mut ui = Ui::default();
        let mut look = Look::of(&ui);
        assert_eq!(look, Look::default());
        assert_eq!((look.code_colors, look.code_size, look.terminal_size), (CodeColors::Ide, 12, 12.5));
        assert!(!look.apply(&mut ui), "nothing was changed, so nothing is written");
        look.code_colors = CodeColors::Quiet;
        look.layout = Layout::Side;
        look.step_code_size(true);
        look.step_code_size(true);
        look.step_terminal_size(false);
        assert!(look.apply(&mut ui));
        assert_eq!(
            (ui.code_colors, ui.code_size, ui.terminal_size, ui.layout),
            (CodeColors::Quiet, 14, 11.5, Layout::Side)
        );
        assert!(!look.apply(&mut ui));
        // the steps stop at the window's own bounds
        for _ in 0..40 {
            look.step_code_size(false);
            look.step_terminal_size(true);
        }
        assert_eq!((look.code_size, look.terminal_size), (ui::MIN_CODE_SIZE, ui::MAX_TERMINAL_SIZE));
        assert_eq!(
            (size_words(12.), size_words(12.5), size_words(9.)),
            ("12 px".into(), "12.5 px".into(), "9 px".into())
        );
    }

    #[test]
    fn the_general_form_shows_the_users_settings_as_text_and_reads_them_back() {
        let mut settings = UserSettings { workspaces_dir: "~/enkidu/workspaces".into(), ..UserSettings::default() };
        settings.claude.command = "claude".into();
        settings.claude.args =
            vec!["--model".into(), "opus".into(), "--append-system-prompt".into(), "be brief".into()];
        settings.claude.env.insert("CLAUDE_CONFIG_DIR".into(), "~/.claude-work".into());
        settings.claude.env.insert("ANTHROPIC_LOG".into(), "debug".into());
        let form = GeneralForm::of(&settings, Look { layout: Layout::Side, ..Look::default() });
        assert_eq!(form.look.layout, Layout::Side);
        assert_eq!(form.args, "--model opus --append-system-prompt 'be brief'");
        assert_eq!(form.env, "ANTHROPIC_LOG=debug\nCLAUDE_CONFIG_DIR=~/.claude-work");
        // unedited, it reads back as it was
        assert_eq!(form.settings().unwrap(), settings);

        // edited: spaces around a folder and a command go, the arguments are split as a shell would
        let edited = GeneralForm {
            workspaces_dir: "  ~/work/trees ".into(),
            command: " /opt/bin/claude ".into(),
            args: "--model \"sonnet 5\" --verbose".into(),
            env: "# the work account\nCLAUDE_CONFIG_DIR = ~/.claude-work\n\nFOO=1\nFOO=2".into(),
            ..form.clone()
        };
        let read = edited.settings().unwrap();
        assert_eq!(read.workspaces_dir, "~/work/trees");
        assert_eq!(read.claude.command, "/opt/bin/claude");
        assert_eq!(read.claude.args, ["--model", "sonnet 5", "--verbose"]);
        let env: Vec<(&str, &str)> = read.claude.env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        assert_eq!(env, [("CLAUDE_CONFIG_DIR", "~/.claude-work"), ("FOO", "2")]);
    }

    #[test]
    fn a_field_that_cant_be_read_is_named_and_nothing_is_made() {
        let form = GeneralForm::of(&UserSettings::default(), Look::default());
        let open_quote = GeneralForm { args: "--append-system-prompt 'be brief".into(), ..form.clone() };
        assert_eq!(open_quote.settings().unwrap_err(), "Claude's arguments: A quote isn't closed");
        let bad_line = GeneralForm { env: "A=1\nnot a pair".into(), ..form.clone() };
        assert_eq!(bad_line.settings().unwrap_err(), "Claude's environment: Line 2: use KEY=value");
        let bad_name = GeneralForm { env: "2FAST=1".into(), ..form };
        assert_eq!(bad_name.settings().unwrap_err(), "Claude's environment: Line 1: \"2FAST\" isn't a valid name");

        let project = ProjectForm { args: "\"".into(), ..ProjectForm::of(&ProjectSettings::default()) };
        assert_eq!(project.settings("shop").unwrap_err(), "Claude's arguments in shop: A quote isn't closed");
        let project = ProjectForm { env: "=x".into(), ..ProjectForm::of(&ProjectSettings::default()) };
        assert_eq!(project.settings("shop").unwrap_err(), "Claude's environment in shop: Line 1: use KEY=value");
    }

    #[test]
    fn a_projects_form_edits_its_own_fields_and_keeps_the_rest_as_it_was() {
        let mut settings = ProjectSettings {
            default_base: "development".into(),
            copy_files: vec![".env".into(), ".claude/settings.local.json".into()],
            setup_script: "make setup\n".into(),
            ..ProjectSettings::default()
        };
        settings.live.on = "docker compose restart web".into();
        settings.live.message = "You are live.".into();
        settings.claude.args = vec!["--permission-mode".into(), "acceptEdits".into()];
        settings.claude.env.insert("FAKE_PROJECT_ENV".into(), "project env".into());
        // what the page's Work and Workflows parts edit: untouched, it stays as it was
        settings.work.github = true;
        settings.work.jira_keys = vec!["SH".into()];
        settings.work.back_to_you = vec!["QA Failed".into(), "Reopened".into()];
        settings.work.ignored_checks = vec!["codecov/patch".into()];
        settings.work.every_minutes = 7;
        settings.work.morning_at = "08:30".into();

        let form = ProjectForm::of(&settings);
        assert_eq!(form.copy_files, ".env\n.claude/settings.local.json");
        assert_eq!(form.args, "--permission-mode acceptEdits");
        assert_eq!(form.env, "FAKE_PROJECT_ENV=project env");
        assert!(!form.edited());
        assert_eq!(form.settings("shop").unwrap(), settings, "unedited, it reads back as it was");

        let edited = ProjectForm {
            default_base: " main ".into(),
            copy_files: ".env\n\n  config/local.py  \n".into(),
            setup_script: "  make setup && make seed\n".into(),
            live_off: "docker compose restart web".into(),
            args: String::new(),
            ..form
        };
        assert!(edited.edited());
        let read = edited.settings("shop").unwrap();
        assert_eq!(read.default_base, "main");
        assert_eq!(read.copy_files, [".env", "config/local.py"], "one path a line, blanks left out");
        assert_eq!(read.setup_script, "  make setup && make seed\n", "a script is kept as it was typed");
        assert_eq!(
            (read.live.on.as_str(), read.live.off.as_str()),
            ("docker compose restart web", "docker compose restart web")
        );
        assert!(read.claude.args.is_empty());
        // the Work settings weren't touched: they are saved as they were
        assert_eq!(read.work, settings.work);
    }

    /// A project's Work and Workflows settings, into the form and back.
    #[test]
    fn a_projects_form_edits_what_is_read_from_github_and_jira_and_its_workflows() {
        let mut settings = ProjectSettings::default();
        settings.work.jira_keys = vec!["SH".into(), "OPS".into()];
        settings.work.back_to_you = vec!["QA Failed".into(), "Reopened".into()];
        settings.work.ignored_checks = vec!["codecov/patch".into(), "lint (optional)".into()];
        let form = ProjectForm::of(&settings);
        assert!(form.work_github && !form.send_failed_checks && !form.morning_summary, "as a new project stands");
        assert_eq!((form.work_jira_keys.as_str(), form.work_back.as_str()), ("SH, OPS", "QA Failed, Reopened"));
        assert_eq!(form.work_ignored, "codecov/patch\nlint (optional)");
        assert_eq!((form.work_every.as_str(), form.morning_at.as_str()), ("5", "09:00"));
        assert!(!form.edited());

        let edited = ProjectForm {
            work_github: false,
            work_account: "  me-work ".into(),
            // as you would type them
            work_jira_keys: "sh, OPS".into(),
            work_back: " QA Failed ,, Reopened , ".into(),
            work_ignored: "codecov/patch\n\n  docs  \n".into(),
            work_every: "12".into(),
            send_failed_checks: true,
            morning_summary: true,
            morning_at: " 08:30 ".into(),
            ..form.clone()
        };
        assert!(edited.edited());
        let work = edited.settings("shop").unwrap().work;
        assert!(!work.github && work.send_failed_checks && work.morning_summary);
        assert_eq!(work.github_account, "me-work");
        assert_eq!(work.jira_keys, ["SH", "OPS"], "capitals, whatever was typed");
        assert_eq!(work.back_to_you, ["QA Failed", "Reopened"]);
        assert_eq!(work.ignored_checks, ["codecov/patch", "docs"]);
        assert_eq!((work.every_minutes, work.morning_at.as_str()), (12, "08:30"));
        // keys with spaces between them, or one a line
        let spaced = ProjectForm { work_jira_keys: "sh ops\nweb".into(), ..form.clone() };
        assert_eq!(spaced.settings("shop").unwrap().work.jira_keys, ["SH", "OPS", "WEB"]);
        // none: Jira isn't read
        let none = ProjectForm { work_jira_keys: " , ".into(), ..form };
        assert!(none.settings("shop").unwrap().work.jira_keys.is_empty());
    }

    #[test]
    fn the_minutes_between_reads_are_a_number_between_1_and_240() {
        assert_eq!((every_minutes("7"), every_minutes(" 12 min"), every_minutes("+3")), (7, 12, 3));
        // nothing readable, or none at all: the usual five
        assert_eq!((every_minutes(""), every_minutes("soon"), every_minutes("0")), (5, 5, 5));
        // never under one, never over four hours
        assert_eq!((every_minutes("-3"), every_minutes("999"), every_minutes("99999999999999999999")), (1, 240, 240));
        assert_eq!(every_minutes("2.5"), 2);
    }
}
