//! The hooks for Claude started outside Enkidu, added to (and removed from) a Claude folder's `settings.json`
//! only when you press the button in Settings, which shows exactly what they are. Each runs the app's own
//! `enkidu event --app enkidu`, which writes to Enkidu's own event log and nothing else. The file's other
//! content stays as it was (key order included), and the original is kept once as
//! `settings.json.before-enkidu`.
//!
//! Other tools add hooks of their own to the same files. Enkidu's are told apart by the end of their command, and
//! it never changes anyone else's.

use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const EVENTS: [(&str, Option<&str>); 5] =
    [("SessionStart", None), ("UserPromptSubmit", None), ("PostToolUse", Some("Bash")), ("Stop", None), ("SessionEnd", None)];

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HookFile {
    /// The Claude folder (`~/.claude-work`)
    pub dir: String,
    pub path: String,
    pub added: bool,
    pub exists: bool,
}

/// What ends the command of every hook the app adds: by it the app knows its own hooks among the others in a file.
const MARK: &str = " event --app enkidu";

/// The app's own `enkidu`: the one next to its program (inside "Enkidu.app" once installed, in `target/` for
/// a dev build); tests name theirs in `ENKIDU_CLI`.
pub fn cli_path() -> Option<PathBuf> {
    cli_path_from(std::env::var_os("ENKIDU_CLI"), std::env::current_exe().ok())
}

fn cli_path_from(named: Option<std::ffi::OsString>, exe: Option<PathBuf>) -> Option<PathBuf> {
    match named.filter(|p| !p.is_empty()) {
        Some(p) => Some(PathBuf::from(p)),
        None => exe?.parent().map(|d| d.join("enkidu")).filter(|p| p.exists()),
    }
}

/// The folder of the app's own `enkidu` goes first on a terminal's PATH, before the login shell's folders: `enkidu`
/// typed in a terminal of the app is always the app's own.
pub fn cli_on_path(env: &mut [(String, String)]) {
    let own = cli_path().and_then(|cli| cli.parent().map(Path::to_path_buf));
    put_on_path(env, own);
}

/// [`cli_on_path`] with the folder given (none in a build without its command).
fn put_on_path(env: &mut [(String, String)], own: Option<PathBuf>) {
    let Some(own) = own else { return };
    for (k, v) in env.iter_mut() {
        if k == "PATH" {
            *v = format!("{}:{v}", own.display());
        }
    }
}

pub fn command(cli: &Path) -> String {
    format!("{}{MARK}", crate::shellenv::shell_quote(&cli.to_string_lossy()))
}

/// What is added: one hook group per event, each running `enkidu event`.
pub fn adds(cli: &Path) -> Value {
    let mut hooks = serde_json::Map::new();
    for (event, matcher) in EVENTS {
        let mut group = json!({ "hooks": [{ "type": "command", "command": command(cli), "timeout": 5 }] });
        if let Some(m) = matcher {
            group["matcher"] = json!(m);
        }
        hooks.insert(event.to_string(), json!([group]));
    }
    json!({ "hooks": hooks })
}

/// A hook group the app added (its command ends with `event --app enkidu`). Any other group in the file is someone
/// else's and is never counted.
fn ours(group: &Value) -> bool {
    group.get("hooks").and_then(Value::as_array).is_some_and(|hs| {
        !hs.is_empty() && hs.iter().all(|h| h.get("command").and_then(Value::as_str).is_some_and(|c| c.trim_end().ends_with(MARK)))
    })
}

fn read(path: &Path) -> Result<Value> {
    match std::fs::read_to_string(path) {
        Ok(text) if text.trim().is_empty() => Ok(json!({})),
        Ok(text) => serde_json::from_str(&text).with_context(|| format!("{} isn't valid JSON; it was left as it is", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(e) => Err(e).with_context(|| format!("Can't read {}", path.display())),
    }
}

pub fn is_added(settings: &Value) -> bool {
    EVENTS.iter().all(|(event, _)| settings.pointer(&format!("/hooks/{event}")).and_then(Value::as_array).is_some_and(|a| a.iter().any(ours)))
}

pub fn status(dirs: &[PathBuf]) -> Vec<HookFile> {
    dirs.iter()
        .map(|d| {
            let path = d.join("settings.json");
            HookFile {
                dir: d.to_string_lossy().to_string(),
                path: path.to_string_lossy().to_string(),
                added: read(&path).map(|v| is_added(&v)).unwrap_or(false),
                exists: path.exists(),
            }
        })
        .collect()
}

fn write(path: &Path, settings: &Value) -> Result<()> {
    // the original is kept once, under a name of the app's own
    let backup = path.with_extension("json.before-enkidu");
    if path.exists() && !backup.exists() {
        std::fs::copy(path, &backup).with_context(|| format!("Can't keep a copy of {}", path.display()))?;
    }
    let tmp = path.with_extension("json.enkidu-tmp");
    std::fs::write(&tmp, format!("{}\n", serde_json::to_string_pretty(settings)?))?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Adds the hooks to a Claude folder's settings (once: adding again changes nothing).
pub fn add(dir: &Path, cli: &Path) -> Result<()> {
    if !dir.is_dir() {
        bail!("{} isn't a Claude folder", dir.display());
    }
    let path = dir.join("settings.json");
    let mut settings = read(&path)?;
    if !settings.is_object() {
        bail!("{} isn't a settings object; it was left as it is", path.display());
    }
    remove_from(&mut settings);
    let hooks = settings.as_object_mut().expect("an object").entry("hooks").or_insert_with(|| json!({}));
    let Some(hooks) = hooks.as_object_mut() else { bail!("\"hooks\" in {} isn't an object; it was left as it is", path.display()) };
    for (event, groups) in adds(cli)["hooks"].as_object().expect("built above") {
        let list = hooks.entry(event.clone()).or_insert_with(|| json!([]));
        if let (Some(list), Some(groups)) = (list.as_array_mut(), groups.as_array()) {
            list.extend(groups.iter().cloned());
        }
    }
    write(&path, &settings)
}

fn remove_from(settings: &mut Value) {
    let Some(hooks) = settings.get_mut("hooks").and_then(Value::as_object_mut) else { return };
    for (event, _) in EVENTS {
        if let Some(list) = hooks.get_mut(event).and_then(Value::as_array_mut) {
            list.retain(|g| !ours(g));
            if list.is_empty() {
                hooks.remove(event);
            }
        }
    }
    if hooks.is_empty() {
        settings.as_object_mut().expect("an object").remove("hooks");
    }
}

/// Takes the hooks out again; everything else stays.
pub fn remove(dir: &Path) -> Result<()> {
    let path = dir.join("settings.json");
    let mut settings = read(&path)?;
    if !settings.is_object() || !path.exists() {
        return Ok(());
    }
    let before = settings.clone();
    remove_from(&mut settings);
    if settings == before {
        // none of ours in it: the file is left exactly as it is, byte for byte
        return Ok(());
    }
    write(&path, &settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_its_hooks_keeps_everything_else_and_takes_them_out_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let original = r#"{
  "model": "opus",
  "permissions": { "allow": ["Bash(git:*)"] },
  "hooks": { "Stop": [{ "hooks": [{ "type": "command", "command": "say done" }] }] },
  "alwaysThinkingEnabled": true
}
"#;
        std::fs::write(&path, original).unwrap();
        let cli = Path::new("/Applications/Enkidu.app/Contents/MacOS/enkidu");
        add(dir.path(), cli).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(is_added(&v));
        let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["model", "permissions", "hooks", "alwaysThinkingEnabled"], "the file's order stays");
        assert_eq!(v["hooks"]["Stop"].as_array().unwrap().len(), 2, "your own Stop hook stays");
        assert_eq!(v["hooks"]["PostToolUse"][0]["matcher"], "Bash");
        assert_eq!(
            v["hooks"]["UserPromptSubmit"][0]["hooks"][0]["command"],
            "'/Applications/Enkidu.app/Contents/MacOS/enkidu' event --app enkidu"
        );
        assert_eq!(std::fs::read_to_string(path.with_extension("json.before-enkidu")).unwrap(), original, "the original is kept");
        // adding again changes nothing
        add(dir.path(), cli).unwrap();
        let again: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(again, v);
        remove(dir.path()).unwrap();
        let back: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let first: Value = serde_json::from_str(original).unwrap();
        assert_eq!(back, first, "taken out, the file is what it was");
        assert!(!is_added(&back));
    }

    #[test]
    fn a_new_file_gets_only_the_hooks_and_a_broken_one_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        add(dir.path(), Path::new("/x/enkidu")).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(dir.path().join("settings.json")).unwrap()).unwrap();
        assert_eq!(v.as_object().unwrap().keys().collect::<Vec<_>>(), ["hooks"]);
        assert!(status(&[dir.path().to_path_buf()])[0].added);
        let broken = tempfile::tempdir().unwrap();
        std::fs::write(broken.path().join("settings.json"), "{ not json").unwrap();
        assert!(add(broken.path(), Path::new("/x/enkidu")).unwrap_err().to_string().contains("left as it is"));
        assert_eq!(std::fs::read_to_string(broken.path().join("settings.json")).unwrap(), "{ not json");
    }

    /// Another tool's hooks in the same file, with that tool's own backup next to the file.
    const WITH_ANOTHER_TOOLS_HOOKS: &str = r#"{
  "model": "opus",
  "hooks": {
    "SessionStart": [{ "hooks": [{ "type": "command", "command": "'/Users/me/.local/bin/notes' event", "timeout": 5 }] }],
    "UserPromptSubmit": [{ "hooks": [{ "type": "command", "command": "'/Users/me/.local/bin/notes' event", "timeout": 5 }] }],
    "PostToolUse": [{ "matcher": "Bash", "hooks": [{ "type": "command", "command": "'/Users/me/.local/bin/notes' event", "timeout": 5 }] }],
    "Stop": [{ "hooks": [{ "type": "command", "command": "'/Users/me/.local/bin/notes' event", "timeout": 5 }] }],
    "SessionEnd": [{ "hooks": [{ "type": "command", "command": "'/Users/me/.local/bin/notes' event", "timeout": 5 }] }]
  }
}
"#;

    #[test]
    fn another_tools_hooks_and_its_backup_are_never_touched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let its_backup = path.with_extension("json.before-notes");
        std::fs::write(&path, WITH_ANOTHER_TOOLS_HOOKS).unwrap();
        std::fs::write(&its_backup, "the other tool's copy").unwrap();
        let before: Value = serde_json::from_str(WITH_ANOTHER_TOOLS_HOOKS).unwrap();
        assert!(!is_added(&before), "the other tool's hooks are not the app's");
        assert!(!status(&[dir.path().to_path_buf()])[0].added);

        // taking ours out of a file that has only the other tool's changes nothing, byte for byte
        remove(dir.path()).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), WITH_ANOTHER_TOOLS_HOOKS, "the file stays as it is");
        assert!(!path.with_extension("json.before-enkidu").exists(), "nothing written at all");

        // adding ours keeps every group that was there exactly and adds one group per event beside them
        add(dir.path(), Path::new("/Applications/Enkidu.app/Contents/MacOS/enkidu")).unwrap();
        let added: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(is_added(&added));
        for (event, _) in EVENTS {
            let groups = added["hooks"][event].as_array().unwrap();
            assert_eq!(groups.len(), 2, "{event}: the other tool's group and ours");
            assert_eq!(groups[0], before["hooks"][event][0], "{event}: the other tool's group, unchanged and first");
            assert!(ours(&groups[1]) && !ours(&groups[0]), "{event}");
        }
        assert_eq!(std::fs::read_to_string(&its_backup).unwrap(), "the other tool's copy", "its backup is never written");
        assert!(path.with_extension("json.before-enkidu").exists(), "ours is kept apart");

        // taking ours out leaves exactly the file as it was
        remove(dir.path()).unwrap();
        let back: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(back, before);
        assert_eq!(std::fs::read_to_string(&its_backup).unwrap(), "the other tool's copy");
    }

    #[test]
    fn the_command_it_names_is_the_apps_own() {
        let exe_dir = tempfile::tempdir().unwrap();
        let exe = exe_dir.path().join("enkidu");
        // no enkidu next to the program and none named: none
        assert_eq!(cli_path_from(None, Some(exe.clone())), None);
        std::fs::write(exe_dir.path().join("enkidu"), "").unwrap();
        assert_eq!(cli_path_from(None, Some(exe.clone())), Some(exe_dir.path().join("enkidu")), "the one next to the program");
        assert_eq!(cli_path_from(Some("/t/enkidu".into()), Some(exe)), Some(PathBuf::from("/t/enkidu")), "a test's own");
    }

    #[test]
    fn a_terminal_finds_the_apps_own_command_first() {
        let path = |own: Option<&str>| {
            let mut env = vec![("PATH".to_string(), "/Users/me/.local/bin:/usr/bin".to_string())];
            put_on_path(&mut env, own.map(PathBuf::from));
            env.remove(0).1
        };
        let own = "/Applications/Enkidu.app/Contents/MacOS";
        assert_eq!(path(Some(own)), format!("{own}:/Users/me/.local/bin:/usr/bin"));
        // a build without its own command: the PATH is the login shell's own
        assert_eq!(path(None), "/Users/me/.local/bin:/usr/bin");
    }
}
