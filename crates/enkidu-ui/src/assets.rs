//! Files the app carries inside itself: its fonts and its icons.
//!
//! The icons are Lucide's 1.43, and 1.68 for the Work page's (ISC licence, `assets/icons/LICENSE-LUCIDE`), redrawn at
//! DESIGN.md's 1.75 stroke (`chevrons-down-up` as gpui-kit 0.9.1 carries it), plus the lamp's ring. The three layout
//! pictures (`layout-*`) are the app's own drawings: small windows whose filled part is the changes. gpui-kit only
//! builds in the icons its own components use, so the app carries the ones it draws.

use std::borrow::Cow;

use gpui_kit::{App, AssetSource, Result, SharedString};

/// The app's assets: its own icons first, then gpui-kit's for its components.
pub struct Assets;

macro_rules! icons {
    ($($name:literal),* $(,)?) => {
        fn own_icon(path: &str) -> Option<&'static [u8]> {
            match path {
                $(concat!("icons/", $name, ".svg") => Some(include_bytes!(concat!("../assets/icons/", $name, ".svg"))),)*
                _ => None,
            }
        }
    };
}

icons![
    "check",
    "braces",
    "case-sensitive",
    "clock",
    "regex",
    "whole-word",
    "arrow-down",
    "arrow-left",
    "arrow-right",
    "arrow-up",
    "chevron-down",
    "chevron-right",
    "chevrons-up-down",
    "chevrons-down-up",
    "columns-2",
    "circle-minus",
    "command",
    "container",
    "copy",
    "database",
    "ellipsis",
    "file",
    "file-archive",
    "file-braces",
    "file-code",
    "file-diff",
    "file-cog",
    "file-headphone",
    "file-image",
    "file-key",
    "file-lock",
    "file-spreadsheet",
    "file-terminal",
    "file-text",
    "file-video-camera",
    "folder",
    "folder-git-2",
    "folder-open",
    "folder-plus",
    "folder-tree",
    "git-branch",
    "keyboard",
    "layout-side",
    "layout-stack",
    "layout-terminal",
    "loader-circle",
    "maximize-2",
    "message-square",
    "minus",
    "minimize-2",
    "lamp-ring",
    "panel-left",
    "panel-right",
    "panels-top-left",
    "pencil",
    "play",
    "play-fill",
    "refresh-cw",
    "plus",
    "search",
    "send",
    "settings",
    "sliders-horizontal",
    "sparkle",
    "rotate-ccw",
    "rows-2",
    "sparkle-filled",
    "sparkles",
    "text-search",
    "square-fill",
    "square-terminal",
    "trash",
    "triangle-alert",
    // the Work page's (phase 5): from Lucide 1.68
    "alarm-clock",
    "at-sign",
    "bookmark",
    "bug",
    "calendar-clock",
    "chevron-left",
    "chevron-up",
    "chevrons-down",
    "chevrons-up",
    "circle",
    "circle-alert",
    "circle-check",
    "circle-dashed",
    "circle-stop",
    "circle-x",
    "corner-down-right",
    "external-link",
    "eye",
    "eye-off",
    "git-commit-horizontal",
    "git-merge",
    "git-pull-request",
    "git-pull-request-arrow",
    "git-pull-request-closed",
    "git-pull-request-draft",
    "image",
    "inbox",
    "info",
    "link",
    "message-circle",
    "message-circle-question-mark",
    "message-square-dashed",
    "message-square-off",
    "message-square-text",
    "paperclip",
    "radio",
    "reply",
    "shield-check",
    "signal",
    "square",
    "square-check",
    "square-kanban",
    "square-plus",
    "timer-off",
    "undo-2",
    "user-round",
    "unlink",
    "zap",
    "zap-off",
    "x",
];

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match own_icon(path) {
            Some(bytes) => Ok(Some(Cow::Borrowed(bytes))),
            None => gpui_kit::assets::Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        gpui_kit::assets::Assets.list(path)
    }
}

/// JetBrains Mono, carried inside the app (SIL Open Font License, `assets/fonts/OFL.txt`).
pub fn load_fonts(cx: &mut App) -> Result<()> {
    cx.text_system().add_fonts(vec![
        Cow::Borrowed(include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/JetBrainsMono-Bold.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/JetBrainsMono-Italic.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/JetBrainsMono-BoldItalic.ttf").as_slice()),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_the_window_draws_is_carried() {
        for name in
            ["command", "container", "file-code", "folder-open", "git-branch", "keyboard", "lamp-ring", "sparkles"]
        {
            let path = format!("icons/{name}.svg");
            let bytes = Assets.load(&path).ok().flatten().unwrap_or_default();
            assert!(bytes.starts_with(b"<svg"), "{path} is missing");
        }
    }
}
