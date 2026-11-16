//! Setup's checks, against programs made for the test: a `git` and a `claude` that are small shell scripts in a
//! folder of their own, and the fake `gh` and `twg`. The real `claude`, `gh` and `twg` are never run here.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use super::*;

fn fake(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fakes").join(name)
}

/// A folder with programs of our own in it, to stand as the whole PATH.
fn bin(programs: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (name, script) in programs {
        let path = dir.path().join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    dir
}

const GIT: &str = r#"[ "$1" = "--version" ] && echo "git version 2.99.1 (Test Git-1)""#;
/// Signed in when the account's folder holds a file `signed-in`, as the real one answers by its folder.
const CLAUDE: &str = r#"case "$1" in
  --version) echo "2.1.325 (Claude Code)" ;;
  auth) if [ -f "$CLAUDE_CONFIG_DIR/signed-in" ]; then echo '{"loggedIn": true, "email": "someone@example.com", "orgName": "Example"}'
        else echo '{"loggedIn": false, "authMethod": "none"}'; exit 1; fi ;;
esac"#;

fn gh(mode: &str) -> Tool {
    let mut tool =
        Tool::new(&format!("node {}", fake("fake-gh.mjs").display()), &std::env::var("PATH").unwrap_or_default());
    tool.env = vec![("FAKE_GH_MODE".to_string(), mode.to_string())];
    tool
}

fn twg(mode: &str) -> Tool {
    let mut tool =
        Tool::new(&format!("node {}", fake("fake-twg.mjs").display()), &std::env::var("PATH").unwrap_or_default());
    tool.env = vec![("FAKE_TWG_MODE".to_string(), mode.to_string())];
    tool
}

/// Everything there and signed in, with the folder `bin` as the PATH of git and claude.
fn tools(bin: &Path) -> Tools {
    let account = bin.join("account");
    std::fs::create_dir_all(&account).unwrap();
    std::fs::write(account.join("signed-in"), "").unwrap();
    Tools {
        path_env: bin.to_string_lossy().into_owned(),
        git: "git".into(),
        claude: "claude".into(),
        claude_env: [("CLAUDE_CONFIG_DIR".to_string(), account.to_string_lossy().into_owned())].into(),
        gh: gh("ok"),
        twg: twg("ok"),
        developer_tools: || true,
    }
}

#[test]
fn reads_the_version_each_program_prints_in_its_own_way() {
    assert_eq!(version_in("git version 2.50.1 (Apple Git-155)").as_deref(), Some("2.50.1"));
    assert_eq!(
        version_in("gh version 2.103.3 (2026-10-26)\nhttps://github.com/cli/cli/releases/tag/v2.103.3").as_deref(),
        Some("2.103.3")
    );
    assert_eq!(version_in("2.1.325 (Claude Code)").as_deref(), Some("2.1.325"));
    assert_eq!(version_in("1.4.2\n").as_deref(), Some("1.4.2"));
    assert_eq!(version_in("tool v0.9.0-rc1").as_deref(), Some("0.9.0-rc1"));
    assert_eq!(version_in("no number here"), None);
    assert_eq!(command_in("Run `twg login --force`.").as_deref(), Some("twg login --force"));
    assert_eq!(command_in("Check the internet connection"), None);
}

#[test]
fn everything_there_and_signed_in_is_ready_with_where_it_is_its_version_and_who() {
    let bin = bin(&[("git", GIT), ("claude", CLAUDE)]);
    let tools = tools(bin.path());
    let git = look(Thing::Git, &tools);
    assert_eq!((git.state, git.version.as_deref()), (State::Ready, Some("2.99.1")));
    assert_eq!(git.path.as_deref(), Some(bin.path().join("git").to_str().unwrap()));
    let claude = look(Thing::Claude, &tools);
    assert_eq!(
        (claude.state, claude.version.as_deref(), claude.signed_in),
        (State::Ready, Some("2.1.325"), Some(true))
    );
    // Claude is asked whether, never who: what it says about the account goes nowhere
    assert!(claude.accounts.is_empty());
    assert!(!serde_json::to_string(&claude).unwrap().contains("someone@example.com"));
    let github = look(Thing::Github, &tools);
    assert_eq!((github.state, github.version.as_deref()), (State::Ready, Some("2.103.3")));
    assert_eq!(github.accounts, ["me-dev"]);
    let jira = look(Thing::Jira, &tools);
    assert_eq!((jira.state, jira.version.as_deref()), (State::Ready, Some("1.4.2")));
    assert_eq!(jira.accounts, ["Me Dev"]);
    assert!(Thing::Git.needed() && Thing::Claude.needed() && !Thing::Github.needed() && !Thing::Jira.needed());
}

#[test]
fn a_program_that_is_not_on_this_mac_is_missing() {
    // a PATH with nothing in it
    let empty = bin(&[]);
    let mut tools = tools(empty.path());
    tools.gh = Tool::new("gh-that-is-not-there", &tools.path_env);
    tools.twg = Tool::new("twg-that-is-not-there", &tools.path_env);
    for thing in Thing::ALL {
        let found = look(thing, &tools);
        assert_eq!((found.state, found.path, found.version), (State::Missing, None, None), "{thing:?}");
    }
    // a claude command of your own that names a file that isn't there
    tools.claude = "/nowhere/claude".into();
    assert_eq!(look(Thing::Claude, &tools).state, State::Missing);
}

#[test]
fn the_stand_in_git_of_a_mac_without_developer_tools_is_missing_and_is_never_run() {
    // /usr/bin/git on this Mac would answer if it were run: with no developer tools it isn't
    let mut tools = tools(bin(&[]).path());
    tools.path_env = "/usr/bin".into();
    tools.developer_tools = || false;
    let found = look(Thing::Git, &tools);
    assert_eq!((found.state, found.path, found.version), (State::Missing, None, None));
    tools.developer_tools = || true;
    assert_eq!(look(Thing::Git, &tools).state, State::Ready);
}

#[test]
fn claude_that_is_there_but_signed_out_says_so_and_an_older_one_is_ready_as_far_as_it_can_tell() {
    let bin = bin(&[("git", GIT), ("claude", CLAUDE)]);
    let mut tools = tools(bin.path());
    // another account's folder, where nobody is signed in
    let other = bin.path().join("other-account");
    std::fs::create_dir_all(&other).unwrap();
    tools.claude_env.insert("CLAUDE_CONFIG_DIR".into(), other.to_string_lossy().into_owned());
    let found = look(Thing::Claude, &tools);
    assert_eq!(
        (found.state, found.signed_in, found.version.as_deref()),
        (State::SignedOut, Some(false), Some("2.1.325"))
    );
    // a Claude Code from before `auth status`
    let old = self::bin(&[(
        "claude",
        r#"[ "$1" = "--version" ] && echo "1.0.20 (Claude Code)" && exit 0; echo "error: unknown command 'auth'" >&2; exit 1"#,
    )]);
    tools.path_env = old.path().to_string_lossy().into_owned();
    let found = look(Thing::Claude, &tools);
    assert_eq!((found.state, found.signed_in, found.version.as_deref()), (State::Ready, None, Some("1.0.20")));
    // one that can't even say its version
    let broken = self::bin(&[("claude", "echo 'dyld: Library not loaded' >&2; exit 134")]);
    tools.path_env = broken.path().to_string_lossy().into_owned();
    let found = look(Thing::Claude, &tools);
    assert_eq!((found.state, found.said.as_deref()), (State::Unclear, Some("dyld: Library not loaded")));
}

#[test]
fn gh_and_twg_signed_out_name_the_command_that_signs_in() {
    let bin = bin(&[]);
    let mut tools = tools(bin.path());
    tools.gh = gh("logged-out");
    tools.twg = twg("logged-out");
    let github = look(Thing::Github, &tools);
    assert_eq!((github.state, github.fix.as_deref()), (State::SignedOut, Some("gh auth login")));
    assert!(github.accounts.is_empty() && github.version.is_some() && github.path.is_some());
    let jira = look(Thing::Jira, &tools);
    assert_eq!((jira.state, jira.fix.as_deref()), (State::SignedOut, Some("twg login --force")));
}

#[test]
fn a_program_that_answers_with_something_else_is_unclear_in_its_own_words() {
    let bin = bin(&[]);
    let mut tools = tools(bin.path());
    tools.twg = twg("garbage");
    let jira = look(Thing::Jira, &tools);
    assert_eq!(jira.state, State::Unclear, "{jira:?}");
    assert!(jira.said.is_some() && jira.version.as_deref() == Some("1.4.2"));
    tools.twg = twg("network");
    let jira = look(Thing::Jira, &tools);
    assert_eq!((jira.state, jira.said.as_deref()), (State::Unclear, Some("No connection to Jira")));
}

/// A live, read-only look at this Mac's own programs (item 21.13):
///
///   ENKIDU_LIVE_SETUP=1 cargo test --locked -p enkidu-core --lib setup::tests::live -- --ignored --nocapture
///
/// It asks git, gh and twg for their version and for who is signed in (never the real `claude`, which no test runs)
/// and prints how each stands with the number of accounts, never their names.
#[test]
#[ignore = "runs this Mac's own git, gh and twg: set ENKIDU_LIVE_SETUP"]
fn live_this_macs_own_git_gh_and_twg_answer_in_a_way_the_app_reads() {
    if std::env::var_os("ENKIDU_LIVE_SETUP").is_none() {
        println!("not asked for (ENKIDU_LIVE_SETUP)");
        return;
    }
    let path_env = crate::shellenv::login_path();
    let tools = Tools {
        path_env: path_env.clone(),
        git: "git".into(),
        claude: "claude".into(),
        claude_env: Default::default(),
        gh: Tool::new("gh", &path_env),
        twg: Tool::new("twg", &path_env),
        developer_tools: super::developer_tools,
    };
    let home = crate::shellenv::home_dir().to_string_lossy().into_owned();
    for thing in [Thing::Git, Thing::Github, Thing::Jira] {
        let started = std::time::Instant::now();
        let found = look(thing, &tools);
        let path = found.path.as_deref().map(|path| path.replace(&home, "~"));
        println!(
            "LIVE setup {thing:?}: {:?}, version {:?}, at {path:?}, {} signed in, {} ms",
            found.state,
            found.version,
            found.accounts.len(),
            started.elapsed().as_millis()
        );
        assert_ne!(found.state, State::Unclear, "{thing:?} said something the app can't read");
        assert!(found.state == State::Missing || found.version.is_some(), "{thing:?} has no version the app reads");
    }
}
