//! The app's own places (items 1.3 and 1.4): its identity, its data folder, the folder new workspaces go to, and the
//! prefix of every environment name it reads.
//!
//! Only the standard library is used here, so these rules can be checked on their own (`rustc --test`).

use std::path::{Path, PathBuf};

/// The app's identity. Its data folder is named after it.
pub const APP_ID: &str = "io.github.hosin211.enkidu";
/// Every environment name the app reads, or gives the programs it starts, begins with this.
pub const ENV_PREFIX: &str = "ENKIDU_";
/// Where new workspaces go, unless Settings name another folder.
pub const WORKSPACES_DIR: &str = "~/enkidu/workspaces";
/// The name tests give their own, throwaway Claude folders in (`a:b`), so nothing reads or changes the real ones.
pub const CLAUDE_DIRS_ENV: &str = "ENKIDU_CLAUDE_DIRS";

/// The home folder, from `HOME` (as `dirs::home_dir` reads it on macOS).
pub fn home() -> PathBuf {
    std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

/// The app's data folder: `ENKIDU_DATA_DIR` when it is set (tests, measurements), else
/// `~/Library/Application Support/io.github.hosin211.enkidu`.
pub fn data_dir() -> PathBuf {
    data_dir_from(std::env::var("ENKIDU_DATA_DIR").ok().as_deref(), &home())
}

/// [`data_dir`] without the environment: the named folder, else the default one under `home`.
pub fn data_dir_from(named: Option<&str>, home: &Path) -> PathBuf {
    match named {
        Some(dir) if !dir.trim().is_empty() => PathBuf::from(dir.trim()),
        _ => home.join("Library/Application Support").join(APP_ID),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_data_folder_is_the_apps_own() {
        let home = Path::new("/Users/me");
        assert_eq!(
            data_dir_from(None, home),
            PathBuf::from("/Users/me/Library/Application Support/io.github.hosin211.enkidu")
        );
        assert_eq!(data_dir_from(Some("  "), home), data_dir_from(None, home), "an empty name is no name");
        assert_eq!(data_dir_from(Some("/tmp/t1"), home), PathBuf::from("/tmp/t1"));
    }

    #[test]
    fn every_name_the_app_reads_carries_its_prefix() {
        assert!(CLAUDE_DIRS_ENV.starts_with(ENV_PREFIX));
        assert!(WORKSPACES_DIR.starts_with("~/"), "new workspaces go under the home folder");
    }
}
