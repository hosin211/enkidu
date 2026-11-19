//! The empty screens (DESIGN.md, "Empty states say what to do next"; item 1.20): with no project, how to start and the
//! keys to know; with no workspace in the project on screen, what a workspace is and the button for the first one. Both
//! under the mark (DESIGN.md, "The mark").

use enkidu_model::frame::Frame;
use enkidu_ui::mark::mark;
use enkidu_ui::pieces::{ButtonKind, ButtonSize, button_with, kbd};
use enkidu_ui::tokens as theme;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Entity, FontWeight, IntoElement, SharedString, Window, div, px};

/// The keys the first screen lists.
pub const SHORTCUTS: [(&str, &str); 5] = [
    ("⌘K", "Go to a workspace or run a command"),
    ("⌘N", "New workspace"),
    ("⌘1–9", "Go to workspace tab 1 to 9"),
    ("⇧⇧", "Search files, classes and text"),
    ("⌘,", "Settings"),
];

/// What a project without workspaces is told: what a workspace is, and that Claude can work in the main folder too
/// (the New workspace dialog asks which).
pub const NO_WORKSPACES: &str = "A workspace is its own checkout on its own branch, with Claude running in it. Make \
                                 one per task, or let Claude work in the project's main folder as it is.";

/// A screen in the island's place: in its middle a column 420 px wide at most, under the drawing (`art` wide, at
/// `shade` opacity), its lines centred.
fn screen(
    id: &'static str,
    label: impl Into<SharedString>,
    art: f32,
    shade: f32,
    words: Vec<AnyElement>,
) -> impl IntoElement {
    let label: SharedString = label.into();
    div().size_full().flex().items_center().justify_center().px(px(32.)).bg(theme::WELL).child(
        div()
            .id(id)
            .test_support()
            .aria_label(label)
            .max_w(px(420.))
            .flex()
            .flex_col()
            .items_center()
            .text_center()
            .child(div().opacity(shade).child(mark(SharedString::from(format!("{id}-mark")), art)))
            .children(words),
    )
}

fn heading(words: impl Into<SharedString>) -> AnyElement {
    div()
        .mt(px(24.))
        .text_size(px(17.))
        .line_height(px(24.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme::INK)
        .child(words.into())
        .into_any_element()
}

fn line(words: &'static str) -> AnyElement {
    div().mt(px(8.)).text_size(px(13.)).line_height(px(20.)).text_color(theme::INK_2).child(words).into_any_element()
}

/// No project yet: add one.
pub fn no_projects(frame: Entity<Frame>, window: &mut Window) -> impl IntoElement {
    let handle = window.window_handle();
    let shortcuts = div().mt(px(32.)).flex().flex_col().gap(px(8.)).text_size(px(12.)).children(SHORTCUTS.iter().map(
        |(keys, words)| {
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .child(div().w(px(56.)).flex().justify_end().child(kbd(*keys)))
                .child(div().text_color(theme::INK_2).child(*words))
        },
    ));
    screen(
        "empty-projects",
        "Add a project to start",
        70.,
        1.,
        vec![
            heading("Add a project to start"),
            line(
                "Pick the main folder of a git repository. Every task then gets its own workspace: a worktree on its \
                 own branch, with Claude running in it.",
            ),
            div()
                .mt(px(24.))
                .child(
                    button_with(
                        "add-first-project",
                        Some("icons/folder-plus.svg"),
                        "Add project",
                        None,
                        ButtonKind::Primary,
                        ButtonSize::Large,
                    )
                    .on_click(move |_, _, cx| crate::rail::add_project(frame.clone(), handle, cx)),
                )
                .into_any_element(),
            shortcuts.into_any_element(),
        ],
    )
}

/// Every workspace of the project on screen is hidden (item 18.5): the screen says so, and how one comes back. The
/// top row has the button of the hidden ones and the +, so the screen has no button of its own.
pub fn all_hidden(project: &str) -> impl IntoElement {
    let words = format!("Every workspace in {project} is hidden");
    screen(
        "empty-hidden",
        words.clone(),
        60.,
        0.9,
        vec![heading(words), line("The eye in the top row lists them: a click brings one back.")],
    )
}

/// The project on screen has no workspace yet: make the first. The dialog asks where Claude works, so the screen
/// names both answers (item 19.4): a checkout of its own, or the project's main folder.
pub fn no_workspaces(frame: Entity<Frame>, project: &str) -> impl IntoElement {
    let words = format!("Start a workspace in {project}");
    screen(
        "empty-workspaces",
        words.clone(),
        60.,
        0.9,
        vec![
            heading(words),
            line(NO_WORKSPACES),
            div()
                .mt(px(24.))
                .child(
                    button_with(
                        "first-workspace",
                        Some("icons/plus.svg"),
                        "New workspace",
                        Some("⌘N"),
                        ButtonKind::Primary,
                        ButtonSize::Large,
                    )
                    .on_click(move |_, window, cx: &mut App| crate::new_workspace::open(frame.clone(), window, cx)),
                )
                .into_any_element(),
        ],
    )
}
