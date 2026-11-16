//! Setup (DESIGN.md, "Setup"): what the app needs on a Mac, as the page says it. The core looks (`enkidu_core::setup`);
//! here each thing becomes a row: its lamp, how it stands in a word or two, one line, and the commands to copy. Code
//! in a line stands between backticks, as everywhere.

use enkidu_core::setup::{Found, Parts, State, Thing};

use crate::text::short_path;

/// The page's first title on a first launch, and what stands under it.
pub const WELCOME: &str = "Welcome to Enkidu";
pub const WELCOME_SAYS: [&str; 2] = [
    "Enkidu runs Claude Code in workspaces of your git projects: a checkout and a branch for each task, with its \
     own Claude.",
    "This is what it needs on this Mac. When Git and Claude Code are ready, close this and add your first project.",
];

pub const NEEDS: &str = "Enkidu needs these";
pub const NEEDS_SAY: &str = "Without them no workspace starts.";
pub const CONNECTIONS: &str = "Connections";
pub const CONNECTIONS_SAY: &str = "For the Work page: your pull requests and your tickets, read with your own logins. Nothing \
                                   is ever written to either, and each can be left out.";
pub const BUILD: &str = "This build";
pub const BUILD_SAYS: &str = "Parts the app carries beside its own program.";
/// Under Jira, before the projects whose tickets aren't read yet.
pub const JIRA_PROJECTS: &str = "Jira is read for a project once its Jira project keys are set. Set them for";
pub const CHECK_AGAIN: &str = "Check again";
/// What a row says while it is looked at.
pub const LOOKING: &str = "Looking…";

/// The lamp before a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lamp {
    /// Green: it is there and signed in.
    Ready,
    /// Red: a workspace can't start without it.
    Needed,
    /// Hollow: a connection that isn't set up. Not a fault.
    Off,
    /// It is being looked at.
    Looking,
}

/// A row of the page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// What the row's parts are named after in the window: `git`, `claude`, `github`, `jira`, `cli`, `python`.
    pub id: &'static str,
    pub name: &'static str,
    pub lamp: Lamp,
    /// "Ready", "Not installed", "Not signed in", "Not set up", "Can't tell"; nothing while it is looked at.
    pub stands: &'static str,
    /// One line under the name, code between backticks.
    pub line: String,
    /// The commands to run, each copied by a press.
    pub commands: Vec<String>,
}

fn id(thing: Thing) -> &'static str {
    match thing {
        Thing::Git => "git",
        Thing::Claude => "claude",
        Thing::Github => "github",
        Thing::Jira => "jira",
    }
}

fn name(thing: Thing) -> &'static str {
    match thing {
        Thing::Git => "Git",
        Thing::Claude => "Claude Code",
        Thing::Github => "GitHub",
        Thing::Jira => "Jira",
    }
}

/// What a thing is for, in one sentence.
fn is_for(thing: Thing) -> &'static str {
    match thing {
        Thing::Git => "Makes each workspace's checkout.",
        Thing::Claude => "Runs in every workspace.",
        Thing::Github => "Your pull requests, reviews and checks on the Work page.",
        Thing::Jira => "Your tickets on the Work page.",
    }
}

/// How each program is put on a Mac, by its maker's own instructions (read 15 Nov 2026).
const INSTALL_GIT: &str = "xcode-select --install";
const INSTALL_CLAUDE: &str = "curl -fsSL https://claude.ai/install.sh | bash";
const INSTALL_GH: &str = "brew install gh";
const INSTALL_TWG: &str = "curl -fsSL --retry 2 https://teamwork-graph.atlassian.com/cli/install | bash";

/// "a", "a and b", "a, b and c".
fn listed(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [first @ .., last] => format!("{} and {last}", first.join(", ")),
    }
}

/// "`gh` 2.103.3" or "`gh`" when it never said its version.
fn program(command: &str, version: Option<&str>) -> String {
    match version {
        Some(version) => format!("`{command}` {version}"),
        None => format!("`{command}`"),
    }
}

/// "`/usr/bin/git`, version 2.50.1": where a program is, with your home folder as `~`.
fn placed(found: &Found) -> String {
    let path = found.path.as_deref().map(short_path).unwrap_or_else(|| found.command.clone());
    match &found.version {
        Some(version) => format!("`{path}`, version {version}"),
        None => format!("`{path}`"),
    }
}

/// The command that signs Claude in: for the account's folder Settings name, when they name one.
fn claude_sign_in(found: &Found) -> String {
    match &found.account_dir {
        Some(dir) => format!("CLAUDE_CONFIG_DIR={dir} claude auth login"),
        None => "claude auth login".to_string(),
    }
}

/// A thing's row. `None`: it is being looked at.
pub fn row(thing: Thing, found: Option<&Found>) -> Row {
    let base = Row {
        id: id(thing),
        name: name(thing),
        lamp: Lamp::Looking,
        stands: "",
        line: LOOKING.into(),
        commands: Vec::new(),
    };
    let Some(found) = found else { return base };
    // what is missing: red where no workspace starts without it, hollow for a connection
    let wanting = if thing.needed() { Lamp::Needed } else { Lamp::Off };
    let own_fix = found.fix.clone();
    match (found.state, thing) {
        (State::Ready, Thing::Git) => Row { lamp: Lamp::Ready, stands: "Ready", line: placed(found), ..base },
        (State::Ready, Thing::Claude) => {
            let signed = if found.signed_in == Some(true) { ", signed in" } else { "" };
            let account = found.account_dir.as_deref().map(|dir| format!(" (`{dir}`)")).unwrap_or_default();
            Row { lamp: Lamp::Ready, stands: "Ready", line: format!("{}{signed}{account}", placed(found)), ..base }
        }
        (State::Ready, _) => {
            let who = listed(&found.accounts);
            let signed = if who.is_empty() { String::new() } else { format!(", signed in as {who}") };
            let line = format!("{}{signed}", program(&found.command, found.version.as_deref()));
            Row { lamp: Lamp::Ready, stands: "Ready", line, ..base }
        }
        (State::Missing, Thing::Git) => Row {
            lamp: wanting,
            stands: "Not installed",
            line: format!("{} Install Apple's developer tools, then check again.", is_for(thing)),
            commands: vec![INSTALL_GIT.into()],
            ..base
        },
        // a claude command of your own that isn't found: the place to put it right is the Claude page
        (State::Missing, Thing::Claude) if found.command != "claude" => Row {
            lamp: wanting,
            stands: "Not found",
            line: format!(
                "{} `{}` is the command set on the Claude page, and it isn't on this Mac.",
                is_for(thing),
                found.command
            ),
            ..base
        },
        (State::Missing, Thing::Claude) => Row {
            lamp: wanting,
            stands: "Not installed",
            line: format!("{} Install it, then start it once in a terminal and sign in.", is_for(thing)),
            commands: vec![INSTALL_CLAUDE.into(), "claude".into()],
            ..base
        },
        (State::Missing, Thing::Github) => Row {
            lamp: wanting,
            stands: "Not set up",
            line: format!("{} Install GitHub's command line, then sign in.", is_for(thing)),
            commands: vec![INSTALL_GH.into(), "gh auth login".into()],
            ..base
        },
        (State::Missing, Thing::Jira) => Row {
            lamp: wanting,
            stands: "Not set up",
            line: format!("{} Install Atlassian's command line, then sign in.", is_for(thing)),
            commands: vec![INSTALL_TWG.into(), "twg login".into()],
            ..base
        },
        (State::SignedOut, Thing::Claude) => Row {
            lamp: wanting,
            stands: "Not signed in",
            line: format!("{}. Nobody is signed in: sign in from a terminal, then check again.", placed(found)),
            commands: vec![claude_sign_in(found)],
            ..base
        },
        (State::SignedOut, _) => {
            let command = own_fix
                .unwrap_or_else(|| if thing == Thing::Github { "gh auth login" } else { "twg login" }.to_string());
            let reads = if thing == Thing::Github { "your pull requests" } else { "your tickets" };
            Row {
                lamp: wanting,
                stands: "Not signed in",
                line: format!(
                    "{} is here. Sign in to read {reads}.",
                    program(&found.command, found.version.as_deref())
                ),
                commands: vec![command],
                ..base
            }
        }
        (State::Unclear, _) => {
            let said = found.said.as_deref().unwrap_or("nothing the app can read");
            Row {
                lamp: wanting,
                stands: "Can't tell",
                line: format!("`{}` answered: {said}", found.command),
                commands: own_fix.into_iter().collect(),
                ..base
            }
        }
    }
}

/// The parts this build lacks, each as a row. Nothing for a whole build.
pub fn missing_parts(parts: Parts) -> Vec<Row> {
    let mut rows = Vec::new();
    if !parts.cli {
        rows.push(Row {
            id: "cli",
            name: "The enkidu command",
            lamp: Lamp::Needed,
            stands: "Missing",
            line: "Claude's hooks run it, and so do you and Claude in a terminal. This build has none beside its \
                   program. The install script builds the app with it."
                .into(),
            commands: vec!["sh scripts/install.sh".into()],
        });
    }
    if !parts.python_server {
        rows.push(Row {
            id: "python",
            name: "Python definitions",
            lamp: Lamp::Off,
            stands: "Missing",
            line:
                "⌘-click in Python asks the language server `ty`, and this build carries none. Fetch it, then install \
                   the app again."
                    .into(),
            commands: vec!["sh scripts/fetch-ty.sh".into(), "sh scripts/install.sh".into()],
        });
    }
    rows
}

/// Git and Claude Code are both ready: a workspace can start.
pub fn can_start(git: Option<&Found>, claude: Option<&Found>) -> bool {
    [git, claude].iter().all(|found| found.is_some_and(|found| found.state == State::Ready))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(thing: Thing, state: State) -> Found {
        let (command, path, version) = match thing {
            Thing::Git => ("git", "/usr/bin/git", "2.50.1"),
            Thing::Claude => ("claude", "/Users/demo/.local/bin/claude", "2.1.325"),
            Thing::Github => ("gh", "/opt/homebrew/bin/gh", "2.103.3"),
            Thing::Jira => ("twg", "/Users/demo/.local/bin/twg", "1.4.2"),
        };
        let there = state != State::Missing;
        Found {
            thing,
            state,
            command: command.into(),
            path: there.then(|| path.to_string()),
            version: there.then(|| version.to_string()),
            signed_in: None,
            account_dir: None,
            accounts: Vec::new(),
            said: None,
            fix: None,
        }
    }

    #[test]
    fn a_thing_being_looked_at_says_so_and_claims_nothing() {
        let row = row(Thing::Claude, None);
        assert_eq!(
            (row.id, row.name, row.lamp, row.stands, row.line.as_str()),
            ("claude", "Claude Code", Lamp::Looking, "", LOOKING)
        );
        assert!(row.commands.is_empty());
    }

    #[test]
    fn what_is_ready_says_where_it_is_its_version_and_who_is_signed_in() {
        let git = row(Thing::Git, Some(&found(Thing::Git, State::Ready)));
        assert_eq!((git.lamp, git.stands, git.line.as_str()), (Lamp::Ready, "Ready", "`/usr/bin/git`, version 2.50.1"));
        // your home folder is said as ~
        let mut claude = found(Thing::Claude, State::Ready);
        claude.signed_in = Some(true);
        assert_eq!(row(Thing::Claude, Some(&claude)).line, "`~/.local/bin/claude`, version 2.1.325, signed in");
        // an older Claude Code can't say whether: the row doesn't claim it
        claude.signed_in = None;
        assert_eq!(row(Thing::Claude, Some(&claude)).line, "`~/.local/bin/claude`, version 2.1.325");
        // another account's folder is named
        claude.signed_in = Some(true);
        claude.account_dir = Some("~/.claude-work".into());
        assert_eq!(
            row(Thing::Claude, Some(&claude)).line,
            "`~/.local/bin/claude`, version 2.1.325, signed in (`~/.claude-work`)"
        );
        let mut github = found(Thing::Github, State::Ready);
        github.accounts = vec!["me-dev".into()];
        assert_eq!(row(Thing::Github, Some(&github)).line, "`gh` 2.103.3, signed in as me-dev");
        github.accounts = vec!["me-dev".into(), "me-work".into()];
        assert_eq!(row(Thing::Github, Some(&github)).line, "`gh` 2.103.3, signed in as me-dev and me-work");
        github.accounts.push("third".into());
        assert_eq!(row(Thing::Github, Some(&github)).line, "`gh` 2.103.3, signed in as me-dev, me-work and third");
        let mut jira = found(Thing::Jira, State::Ready);
        jira.accounts = vec!["Me Dev".into()];
        let jira = row(Thing::Jira, Some(&jira));
        assert_eq!((jira.lamp, jira.line.as_str()), (Lamp::Ready, "`twg` 1.4.2, signed in as Me Dev"));
        assert!(git.commands.is_empty() && jira.commands.is_empty(), "nothing to run where nothing is wrong");
    }

    #[test]
    fn what_is_missing_says_what_it_is_for_and_the_commands_that_put_it_there() {
        let git = row(Thing::Git, Some(&found(Thing::Git, State::Missing)));
        assert_eq!((git.lamp, git.stands), (Lamp::Needed, "Not installed"), "no workspace starts without it: red");
        assert_eq!(git.line, "Makes each workspace's checkout. Install Apple's developer tools, then check again.");
        assert_eq!(git.commands, ["xcode-select --install"]);
        let claude = row(Thing::Claude, Some(&found(Thing::Claude, State::Missing)));
        assert_eq!((claude.lamp, claude.stands), (Lamp::Needed, "Not installed"));
        assert_eq!(claude.commands, ["curl -fsSL https://claude.ai/install.sh | bash", "claude"]);
        // a connection that isn't there is not a fault: hollow, "Not set up"
        let github = row(Thing::Github, Some(&found(Thing::Github, State::Missing)));
        assert_eq!((github.lamp, github.stands), (Lamp::Off, "Not set up"));
        assert_eq!(
            github.line,
            "Your pull requests, reviews and checks on the Work page. Install GitHub's command line, then sign in."
        );
        assert_eq!(github.commands, ["brew install gh", "gh auth login"]);
        let jira = row(Thing::Jira, Some(&found(Thing::Jira, State::Missing)));
        assert_eq!((jira.lamp, jira.stands), (Lamp::Off, "Not set up"));
        assert_eq!(
            jira.commands,
            ["curl -fsSL --retry 2 https://teamwork-graph.atlassian.com/cli/install | bash", "twg login"]
        );
    }

    #[test]
    fn a_claude_command_of_your_own_that_is_not_there_points_at_the_claude_page() {
        let mut claude = found(Thing::Claude, State::Missing);
        claude.command = "~/bin/claude-nightly".into();
        let row = row(Thing::Claude, Some(&claude));
        assert_eq!((row.lamp, row.stands), (Lamp::Needed, "Not found"));
        assert_eq!(
            row.line,
            "Runs in every workspace. `~/bin/claude-nightly` is the command set on the Claude page, and it isn't on this Mac."
        );
        assert!(row.commands.is_empty(), "installing Claude Code would not help");
    }

    #[test]
    fn what_is_signed_out_names_the_command_that_signs_in() {
        let mut claude = found(Thing::Claude, State::SignedOut);
        claude.signed_in = Some(false);
        let said = row(Thing::Claude, Some(&claude));
        assert_eq!((said.lamp, said.stands), (Lamp::Needed, "Not signed in"));
        assert_eq!(
            said.line,
            "`~/.local/bin/claude`, version 2.1.325. Nobody is signed in: sign in from a terminal, then check again."
        );
        assert_eq!(said.commands, ["claude auth login"]);
        // for the account's folder Settings name
        claude.account_dir = Some("~/.claude-work".into());
        assert_eq!(row(Thing::Claude, Some(&claude)).commands, ["CLAUDE_CONFIG_DIR=~/.claude-work claude auth login"]);
        let github = row(Thing::Github, Some(&found(Thing::Github, State::SignedOut)));
        assert_eq!((github.lamp, github.stands), (Lamp::Off, "Not signed in"));
        assert_eq!(github.line, "`gh` 2.103.3 is here. Sign in to read your pull requests.");
        assert_eq!(github.commands, ["gh auth login"]);
        // the program's own word for it wins
        let mut jira = found(Thing::Jira, State::SignedOut);
        jira.fix = Some("twg login --force".into());
        let jira = row(Thing::Jira, Some(&jira));
        assert_eq!(jira.line, "`twg` 1.4.2 is here. Sign in to read your tickets.");
        assert_eq!(jira.commands, ["twg login --force"]);
    }

    #[test]
    fn what_cant_be_read_is_said_in_the_programs_own_words() {
        let mut jira = found(Thing::Jira, State::Unclear);
        jira.said = Some("No connection to Jira".into());
        let row_ = row(Thing::Jira, Some(&jira));
        assert_eq!(
            (row_.lamp, row_.stands, row_.line.as_str()),
            (Lamp::Off, "Can't tell", "`twg` answered: No connection to Jira")
        );
        let mut claude = found(Thing::Claude, State::Unclear);
        claude.said = None;
        let row_ = row(Thing::Claude, Some(&claude));
        assert_eq!((row_.lamp, row_.line.as_str()), (Lamp::Needed, "`claude` answered: nothing the app can read"));
    }

    #[test]
    fn a_whole_build_has_no_rows_of_its_own_and_a_workspace_can_start_with_git_and_claude() {
        assert!(missing_parts(Parts { cli: true, python_server: true }).is_empty());
        let rows = missing_parts(Parts { cli: false, python_server: false });
        assert_eq!(
            rows.iter().map(|row| (row.id, row.lamp)).collect::<Vec<_>>(),
            [("cli", Lamp::Needed), ("python", Lamp::Off)]
        );
        assert_eq!(rows[1].commands, ["sh scripts/fetch-ty.sh", "sh scripts/install.sh"]);
        let (git, claude) = (found(Thing::Git, State::Ready), found(Thing::Claude, State::Ready));
        assert!(can_start(Some(&git), Some(&claude)));
        assert!(!can_start(Some(&git), None), "still looking");
        assert!(!can_start(Some(&git), Some(&found(Thing::Claude, State::SignedOut))));
    }
}
