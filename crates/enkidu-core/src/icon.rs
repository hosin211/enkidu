//! A project's own icon, for its button on the rail (DESIGN.md, "Icons"): looked for in the project's folder, where
//! an IDE or a framework keeps one. Project settings can name another picture, or say that the letter is wanted.
//!
//! Only names are looked at, in a fixed list of places: no folder is walked, and no file is opened here (the window
//! draws the picture).

use std::path::{Path, PathBuf};

use crate::settings::RailSettings;
use crate::shellenv::expand_tilde;

/// The IDE's own place for a project's icon: JetBrains' IDEs show `.idea/icon.svg` (or `.png`) beside the project's
/// name, and take `icon_dark` in a dark window, which this one is.
const IDE: [&str; 4] = [".idea/icon_dark.svg", ".idea/icon_dark.png", ".idea/icon.svg", ".idea/icon.png"];

/// The folders an app or a site keeps its icon in, from the project's own folder; that folder itself first.
const FOLDERS: [&str; 24] = [
    "",
    "assets",
    "public",
    "static",
    "resources",
    "bundle",
    "docs",
    "src",
    "src/assets",
    "app",
    "web",
    "build",
    "web/public",
    "frontend/public",
    "client/public",
    "static/img",
    "static/images",
    "static/assets",
    "static/assets/img",
    "static/assets/images",
    "public/img",
    "public/images",
    "assets/img",
    "assets/images",
];

/// The names an icon goes by: a picture made to be small and square, the kind that stays sharp at any size first.
const ICONS: [&str; 6] = ["icon.svg", "favicon.svg", "icon.png", "apple-touch-icon.png", "favicon.png", "favicon.ico"];

/// A logo is looked for only when no folder has an icon: it is often a wide picture with the name in it, and that
/// can't be read in a small square.
const LOGOS: [&str; 2] = ["logo.svg", "logo.png"];

/// The kinds of picture the window draws.
const KINDS: [&str; 7] = ["svg", "png", "ico", "jpg", "jpeg", "webp", "gif"];

/// A picture over this size is not an icon: it is left alone.
const MOST: u64 = 4 * 1024 * 1024;

/// Whether `path` is a picture the window can draw: a file of a known kind, not empty and not huge.
fn drawable(path: &Path) -> bool {
    let kind = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    kind.is_some_and(|kind| KINDS.contains(&kind.as_str()))
        && std::fs::metadata(path).is_ok_and(|file| file.is_file() && file.len() > 0 && file.len() <= MOST)
}

/// The icon found in a project's folder: the IDE's own first; then an icon, in the first of the folders that has
/// one; then a logo, the same way.
pub fn find(folder: &Path) -> Option<PathBuf> {
    let ide = IDE.iter().map(|place| folder.join(place));
    let named = |names: &'static [&'static str]| {
        FOLDERS.iter().flat_map(move |dir| names.iter().map(move |name| folder.join(dir).join(name)))
    };
    ide.chain(named(&ICONS)).chain(named(&LOGOS)).find(|path| drawable(path))
}

/// The picture a project's button shows, by its settings: none when the letter is wanted; the picture they name
/// (from the project's folder, or a whole path) when it is there; else the one found in the folder.
pub fn of(folder: &Path, settings: &RailSettings) -> Option<PathBuf> {
    if !settings.icon {
        return None;
    }
    let named = settings.picture.trim();
    if !named.is_empty() {
        let path = expand_tilde(named);
        let path = if path.is_absolute() { path } else { folder.join(path) };
        if drawable(&path) {
            return Some(path);
        }
    }
    find(folder)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(folder: &Path, place: &str) -> PathBuf {
        let path = folder.join(place);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"a picture").unwrap();
        path
    }

    #[test]
    fn finds_the_ides_icon_first_then_the_usual_places() {
        let home = tempfile::tempdir().unwrap();
        let folder = home.path();
        assert_eq!(find(folder), None, "nothing there: the letter");
        // a logo is taken when there is no icon: the sharper kind of two in a folder
        put(folder, "docs/logo.png");
        let logo = put(folder, "docs/logo.svg");
        assert_eq!(find(folder), Some(logo));
        // a site's icon, deep in the usual places, comes before a logo, even one in the project's own folder
        put(folder, "logo.png");
        let favicon = put(folder, "static/img/favicon.ico");
        assert_eq!(find(folder), Some(favicon));
        // an icon nearer the project's own folder wins, and a sharper kind in the same folder before a duller one
        put(folder, "public/favicon.ico");
        let public = put(folder, "public/favicon.png");
        assert_eq!(find(folder), Some(public));
        // the IDE's own icon comes before all of them, its dark one first
        let ide = put(folder, ".idea/icon.png");
        assert_eq!(find(folder), Some(ide));
        let dark = put(folder, ".idea/icon_dark.svg");
        assert_eq!(find(folder), Some(dark));
    }

    /// Two layouts a project can have: a site built on an admin template keeps a favicon beside its logos under
    /// `static/assets/images`, and shows the favicon; a project whose only picture is a wide JPG logo keeps its
    /// letter.
    #[test]
    fn a_favicon_under_static_assets_is_found_and_a_wide_jpg_logo_is_not_an_icon() {
        let home = tempfile::tempdir().unwrap();
        let site = home.path().join("site");
        for logo in ["logo.svg", "logo-dark.svg", "logo-white.svg"] {
            put(&site, &format!("static/assets/images/{logo}"));
        }
        let favicon = put(&site, "static/assets/images/favicon.svg");
        assert_eq!(find(&site), Some(favicon));
        let tool = home.path().join("tool");
        put(&tool, "static/img/logo.jpg");
        put(&tool, "static/img/arrow.png");
        assert_eq!(find(&tool), None);
    }

    #[test]
    fn a_folder_an_empty_file_and_another_kind_of_file_are_not_icons() {
        let home = tempfile::tempdir().unwrap();
        let folder = home.path();
        std::fs::create_dir_all(folder.join("icon.png")).unwrap();
        std::fs::write(folder.join("logo.png"), b"").unwrap();
        put(folder, "icon.txt");
        assert_eq!(find(folder), None);
    }

    #[test]
    fn the_settings_name_another_picture_or_ask_for_the_letter() {
        let home = tempfile::tempdir().unwrap();
        let folder = home.path();
        let found = put(folder, "favicon.ico");
        let own = put(folder, "design/mark.png");
        let elsewhere = put(home.path(), "elsewhere/other.svg");
        let with = |icon: bool, picture: &str| RailSettings { icon, picture: picture.into() };
        assert_eq!(of(folder, &with(true, "")), Some(found.clone()), "the one found");
        assert_eq!(of(folder, &with(true, " design/mark.png ")), Some(own), "a path from the project's folder");
        assert_eq!(of(folder, &with(true, elsewhere.to_str().unwrap())), Some(elsewhere), "a whole path");
        assert_eq!(of(folder, &with(true, "design/gone.png")), Some(found), "a picture that is gone: the one found");
        assert_eq!(of(folder, &with(false, "design/mark.png")), None, "the letter is wanted");
    }
}
