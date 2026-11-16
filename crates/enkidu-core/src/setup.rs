//! Setup: what the app needs on a Mac, looked at when Settings asks (DESIGN.md, "Setup"). Each thing is one program:
//! is it there, which version is it, and is someone signed in. Nothing is installed, signed in or changed from here.
//! A program is only asked for its version and for who is signed in (`git --version`, `claude --version`,
//! `claude auth status`, `gh --version`, `gh auth status`, `twg --version`, `twg whoami`), with a time limit, and
//! never on the drawing thread. Account names are returned to be shown; they are never written to a log or a file.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::shellenv::{expand_tilde, which};
use crate::state::Shared;
use crate::work::run::{Failure, Tool};

/// One thing the app needs, or can use.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Thing {
    Git,
    Claude,
    Github,
    Jira,
}

impl Thing {
    /// In the order the page lists them.
    pub const ALL: [Thing; 4] = [Thing::Git, Thing::Claude, Thing::Github, Thing::Jira];

    /// No workspace starts without it. The others are connections you may leave out.
    pub fn needed(self) -> bool {
        matches!(self, Thing::Git | Thing::Claude)
    }
}

/// How a thing stands.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum State {
    Ready,
    /// The program isn't on this Mac, or not where your login shell looks.
    Missing,
    /// It is there, and nobody is signed in.
    SignedOut,
    /// It ran and said something the app can't read.
    Unclear,
}

/// What was found of one thing.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Found {
    pub thing: Thing,
    pub state: State,
    /// The command as it is asked for: `git`, `gh`, `twg`, or the claude command of Settings.
    pub command: String,
    /// Where the program is.
    pub path: Option<String>,
    pub version: Option<String>,
    /// Whether someone is signed in, where the program says so without naming them (Claude).
    pub signed_in: Option<bool>,
    /// The account's folder Claude starts with, as Settings name it (`CLAUDE_CONFIG_DIR`): signing in is for that one.
    pub account_dir: Option<String>,
    /// Who is signed in: gh's logins, the name twg reads as.
    pub accounts: Vec<String>,
    /// What the program itself said, when the app can't read it: its first line.
    pub said: Option<String>,
    /// The command the program itself names to put it right (`twg login --force`).
    pub fix: Option<String>,
}

impl Found {
    fn new(thing: Thing, command: &str) -> Found {
        Found {
            thing,
            state: State::Missing,
            command: command.to_string(),
            path: None,
            version: None,
            signed_in: None,
            account_dir: None,
            accounts: Vec::new(),
            said: None,
            fix: None,
        }
    }

    fn unclear(mut self, said: impl Into<String>) -> Found {
        let said: String = said.into();
        let first = crate::redact::redact_urls(said.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or(""));
        self.state = State::Unclear;
        self.said = (!first.is_empty()).then(|| first.chars().take(200).collect());
        self
    }
}

/// How long a program gets to say its version, and to say who is signed in (twg asks Atlassian).
const VERSION_LIMIT: Duration = Duration::from_secs(10);
const SIGNED_IN_LIMIT: Duration = Duration::from_secs(20);

/// The programs to look at and the PATH they are found on. Tests point these at fakes.
#[derive(Clone, Debug)]
pub struct Tools {
    pub path_env: String,
    pub git: String,
    /// The claude command of Settings, and the environment Claude starts with (the account's folder).
    pub claude: String,
    pub claude_env: BTreeMap<String, String>,
    pub gh: Tool,
    pub twg: Tool,
    /// Whether Apple's developer tools are on this Mac. Without them `/usr/bin/git` is only a stand-in, and running
    /// it makes macOS ask to install them: so it is not run.
    pub developer_tools: fn() -> bool,
}

/// As the app runs them: found on your login shell's PATH, Claude as Settings start it, `gh` and `twg` as the Work
/// index runs them.
pub fn tools(shared: &Shared) -> Tools {
    let user = shared.user_settings();
    let sources = shared.work.sources();
    Tools {
        path_env: crate::shellenv::login_path(),
        git: "git".into(),
        claude: user.claude.command.trim().to_string(),
        claude_env: user.claude.env,
        gh: sources.gh,
        twg: sources.twg,
        developer_tools,
    }
}

/// `xcode-select -p` names the developer tools' folder, or fails when there are none. It asks for nothing.
fn developer_tools() -> bool {
    std::process::Command::new("/usr/bin/xcode-select")
        .arg("-p")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// The version in what a program prints for `--version`: the first word that starts with a digit and has a dot
/// ("git version 2.50.1 (Apple Git-155)", "gh version 2.103.3 (2026-10-26)", "2.1.325 (Claude Code)", "1.4.2").
pub fn version_in(text: &str) -> Option<String> {
    text.split(|c: char| c.is_whitespace() || matches!(c, ',' | '(' | ')'))
        .map(|word| word.strip_prefix('v').unwrap_or(word))
        .find(|word| {
            word.contains('.')
                && word.starts_with(|c: char| c.is_ascii_digit())
                && word.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
        })
        .map(String::from)
}

/// The command a sentence names between backticks ("Run `twg login --force`.").
fn command_in(sentence: &str) -> Option<String> {
    let mut parts = sentence.split('`');
    parts.next()?;
    parts.next().map(str::trim).filter(|command| !command.is_empty()).map(String::from)
}

/// The program at `path`, run alone.
fn tool_at(path: &Path, path_env: &str) -> Tool {
    Tool {
        program: path.to_string_lossy().into_owned(),
        prefix: Vec::new(),
        path_env: path_env.to_string(),
        env: Vec::new(),
        max_wait: None,
    }
}

/// The program's version, or why it couldn't say it: `Err(None)` when it can't be started at all.
fn version_of(tool: &Tool, env: &[(&str, &str)]) -> Result<Option<String>, Option<String>> {
    match tool.run(&["--version"], env, VERSION_LIMIT) {
        Ok(out) if out.code == 0 => Ok(version_in(&out.stdout).or_else(|| version_in(&out.stderr))),
        Ok(out) => Err(Some(if out.stderr.trim().is_empty() { out.stdout } else { out.stderr })),
        Err(err) if err.to_string().contains("can't start") => Err(None),
        Err(err) => Err(Some(err.to_string())),
    }
}

fn look_git(tools: &Tools) -> Found {
    let found = Found::new(Thing::Git, &tools.git);
    let Some(path) = which(&tools.git, &tools.path_env) else { return found };
    if path == Path::new("/usr/bin/git") && !(tools.developer_tools)() {
        return found;
    }
    let found = Found { path: Some(path.to_string_lossy().into_owned()), ..found };
    match version_of(&tool_at(&path, &tools.path_env), &[]) {
        Ok(version) => Found { state: State::Ready, version, ..found },
        Err(None) => Found { path: None, ..found },
        Err(Some(said)) => found.unclear(said),
    }
}

fn look_claude(tools: &Tools) -> Found {
    let found = Found {
        account_dir: tools.claude_env.get("CLAUDE_CONFIG_DIR").cloned(),
        ..Found::new(Thing::Claude, &tools.claude)
    };
    let Some(path) = which(&tools.claude, &tools.path_env) else { return found };
    let found = Found { path: Some(path.to_string_lossy().into_owned()), ..found };
    let tool = tool_at(&path, &tools.path_env);
    // as Claude starts in a workspace: the account's folder decides who is signed in
    let env: Vec<(String, String)> = (tools.claude_env.iter())
        .map(|(key, value)| {
            let value = if value.starts_with("~/") {
                expand_tilde(value).to_string_lossy().into_owned()
            } else {
                value.clone()
            };
            (key.clone(), value)
        })
        .collect();
    let env: Vec<(&str, &str)> = env.iter().map(|(key, value)| (key.as_str(), value.as_str())).collect();
    let version = match version_of(&tool, &env) {
        Ok(version) => version,
        Err(None) => return Found { path: None, ..found },
        Err(Some(said)) => return found.unclear(said),
    };
    // `claude auth status` answers in JSON whether someone is signed in. A Claude Code without that command is ready
    // as far as the app can tell; its own terminal says the rest.
    let signed_in = (tool.run(&["auth", "status"], &env, SIGNED_IN_LIMIT).ok())
        .and_then(|out| serde_json::from_str::<serde_json::Value>(out.stdout.trim()).ok())
        .and_then(|answer| answer.get("loggedIn").and_then(serde_json::Value::as_bool));
    let state = if signed_in == Some(false) { State::SignedOut } else { State::Ready };
    Found { state, version, signed_in, ..found }
}

/// What a failed read says, sorted: not there, not signed in, or something else in its own words.
fn failed(found: Found, err: &anyhow::Error) -> Found {
    let failure = Failure::of(err);
    if failure.message.contains("can't start") {
        Found { state: State::Missing, path: None, version: None, ..found }
    } else if failure.message.contains("isn't signed in") {
        Found { state: State::SignedOut, fix: command_in(&failure.fix), ..found }
    } else {
        let fix = command_in(&failure.fix);
        Found { fix, ..found.unclear(failure.message) }
    }
}

/// A program run through its `Tool` (a fake in tests is `node <script>`): where its program is and its version, and
/// whether it answered (so it can be asked who is signed in).
fn found_tool(thing: Thing, command: &str, tool: &Tool) -> (Found, bool) {
    let found = Found::new(thing, command);
    let Some(path) = which(&tool.program, &tool.path_env) else { return (found, false) };
    let found = Found { path: Some(path.to_string_lossy().into_owned()), ..found };
    match version_of(tool, &[]) {
        Ok(version) => (Found { version, ..found }, true),
        Err(None) => (Found { path: None, ..found }, false),
        Err(Some(said)) => (found.unclear(said), false),
    }
}

fn look_github(tools: &Tools) -> Found {
    let (found, answered) = found_tool(Thing::Github, "gh", &tools.gh);
    if !answered {
        return found;
    }
    match crate::work::github::accounts(&tools.gh) {
        Ok(accounts) => Found { state: State::Ready, accounts, ..found },
        Err(err) => failed(found, &err),
    }
}

fn look_jira(tools: &Tools) -> Found {
    let (found, answered) = found_tool(Thing::Jira, "twg", &tools.twg);
    if !answered {
        return found;
    }
    match crate::work::jira::whoami(&tools.twg, "") {
        Ok((_, name)) => Found { state: State::Ready, accounts: vec![name], ..found },
        Err(err) => failed(found, &err),
    }
}

/// Looks at one thing. It runs programs: never call it on the drawing thread.
pub fn look(thing: Thing, tools: &Tools) -> Found {
    match thing {
        Thing::Git => look_git(tools),
        Thing::Claude => look_claude(tools),
        Thing::Github => look_github(tools),
        Thing::Jira => look_jira(tools),
    }
}

/// The parts the app carries beside its own program: its `enkidu` (the hooks and `enkidu` in a terminal run it) and the
/// language server for Python. A build without one says so on the page.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Parts {
    pub cli: bool,
    pub python_server: bool,
}

pub fn parts(shared: &Shared) -> Parts {
    Parts {
        cli: crate::work::claudehooks::cli_path().is_some(),
        python_server: crate::lsp::python_program(&shared.data_dir).is_some(),
    }
}

/// The projects whose Jira tickets aren't read yet (no Jira project key set on their page): their ids and names.
pub fn without_jira(shared: &Shared) -> Vec<(String, String)> {
    (shared.db.projects().unwrap_or_default().into_iter())
        .filter(|project| shared.project_settings(&project.id).is_ok_and(|settings| settings.work.jira_keys.is_empty()))
        .map(|project| (project.id, project.name))
        .collect()
}

#[cfg(test)]
mod tests;
