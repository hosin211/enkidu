//! Apps opened from the Finder get a minimal PATH, so the login shell is asked once for the
//! PATH the user has in their terminal. Programs (claude, the shell, scripts) run with it.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::OnceLock;
use std::time::Duration;

static LOGIN_PATH: OnceLock<String> = OnceLock::new();

pub fn user_shell() -> String {
    std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/bin/zsh".to_string())
}

pub fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

pub fn expand_tilde(path: &str) -> PathBuf {
    if path == "~" {
        return home_dir();
    }
    match path.strip_prefix("~/") {
        Some(rest) => home_dir().join(rest),
        None => PathBuf::from(path),
    }
}

/// PATH from an interactive login shell, merged with the app's own PATH and common tool folders.
pub fn login_path() -> String {
    LOGIN_PATH.get_or_init(resolve_login_path).clone()
}

fn resolve_login_path() -> String {
    let own = std::env::var("PATH").unwrap_or_default();
    let shell = user_shell();
    let (tx, rx) = mpsc::channel();
    let child = Command::new(&shell)
        .args(["-ilc", "printf '__ENKIDU_PATH__%s' \"$PATH\""])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let from_shell = match child {
        Ok(child) => {
            std::thread::spawn(move || {
                let _ = tx.send(child.wait_with_output());
            });
            match rx.recv_timeout(Duration::from_secs(6)) {
                Ok(Ok(out)) => String::from_utf8_lossy(&out.stdout)
                    .rsplit("__ENKIDU_PATH__")
                    .next()
                    .map(|p| p.trim().to_string())
                    .unwrap_or_default(),
                _ => String::new(),
            }
        }
        Err(_) => String::new(),
    };
    let home = home_dir();
    let extras = [
        home.join(".local/bin"),
        home.join(".claude/local"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/bin"),
        PathBuf::from("/usr/sbin"),
        PathBuf::from("/sbin"),
    ];
    let mut parts: Vec<String> = Vec::new();
    let mut push = |p: &str| {
        if !p.is_empty() && !parts.iter().any(|x| x == p) {
            parts.push(p.to_string());
        }
    };
    from_shell.split(':').for_each(&mut push);
    own.split(':').for_each(&mut push);
    for extra in extras {
        push(&extra.to_string_lossy());
    }
    parts.join(":")
}

/// Finds a program the way a shell would, using the given PATH.
pub fn which(program: &str, path: &str) -> Option<PathBuf> {
    if program.contains('/') {
        let candidate = expand_tilde(program);
        return is_executable(&candidate).then_some(candidate);
    }
    path.split(':')
        .filter(|dir| !dir.is_empty())
        .map(|dir| Path::new(dir).join(program))
        .find(|candidate| is_executable(candidate))
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// Quotes a value for use inside an `sh -c` string.
pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_single_quotes() {
        assert_eq!(shell_quote("/tmp/it's here"), "'/tmp/it'\\''s here'");
    }

    #[test]
    fn expands_tilde() {
        assert_eq!(expand_tilde("~/x"), home_dir().join("x"));
        assert_eq!(expand_tilde("/abs"), PathBuf::from("/abs"));
    }

    #[test]
    fn finds_programs_on_path() {
        assert!(which("sh", "/usr/bin:/bin").is_some());
        assert!(which("definitely-not-a-program-rh", "/usr/bin:/bin").is_none());
    }
}
