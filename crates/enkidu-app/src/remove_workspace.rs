//! Remove a workspace (item 1.22b): what goes and what stays, said before anything is deleted. A workspace Enkidu made
//! loses its folder and keeps its branch; a folder opened as a workspace (one made elsewhere) is only forgotten.
//! Uncommitted changes are said first, and only "Remove anyway" deletes them. Enter falls on Cancel, so it only closes
//! the dialog.

use enkidu_core::db::Workspace;
use enkidu_model::frame::Frame;
use enkidu_ui::pieces::{ButtonKind, ButtonSize, Observed, button};
use enkidu_ui::tokens::{self as theme, alpha};
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    AsyncWindowContext, Context, Entity, FontWeight, IntoElement, Render, SharedString, WeakEntity, Window, div, px,
};

use crate::bridge::Bridge;
use crate::projects_row::short_path;
use crate::toast;

pub struct RemoveWorkspace {
    frame: Entity<Frame>,
    workspace: Workspace,
    /// The core said it has uncommitted changes: the next press removes them too.
    dirty: bool,
    busy: bool,
    error: Option<String>,
}

/// Asks before removing `workspace`.
pub fn open(frame: Entity<Frame>, workspace: Workspace, window: &mut Window, cx: &mut gpui_kit::App) {
    let view = cx.new(|_| RemoveWorkspace { frame, workspace, dirty: false, busy: false, error: None });
    window.open_dialog(cx, move |dialog, _, cx| {
        let busy = view.read(cx).busy;
        crate::dialog_parts::surface(dialog)
            .w(px(460.))
            .overlay_closable(!busy)
            .keyboard(!busy)
            .child(view.clone())
            .on_ok(|_, _, _| true)
    });
}

impl RemoveWorkspace {
    fn run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        let (id, force) = (self.workspace.id.clone(), self.dirty);
        let answer = Bridge::ask_with_news(cx, move |events, shared| {
            enkidu_core::commands::remove_workspace(events.clone(), shared, id, force)
        });
        cx.spawn_in(window, async move |this: WeakEntity<Self>, cx: &mut AsyncWindowContext| {
            let Ok(answer) = answer.await else { return };
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match answer {
                    Ok(()) => this.removed(window, cx),
                    Err(err) if err.code == "dirty" => this.dirty = true,
                    Err(err) => this.error = Some(err.message),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn removed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ws = self.workspace.clone();
        // its terminals ended with it
        if let Some(terminals) = cx.try_global::<crate::stores::Stores>().map(|stores| stores.terminals.clone()) {
            terminals.update(cx, |terminals, cx| terminals.workspace_gone(&ws.id, cx));
        }
        if let Some(lists) = cx.try_global::<crate::stores::Stores>().map(|stores| stores.changes.clone()) {
            lists.update(cx, |lists, cx| {
                if lists.forget(&ws.id) {
                    cx.notify();
                }
            });
        }
        self.frame.update(cx, |frame, cx| {
            frame.remove_workspace(&ws.id);
            cx.notify();
        });
        window.close_dialog(cx);
        let detail = if ws.adopted {
            format!("{} is still there, as it was.", short_path(&ws.path))
        } else {
            format!("The branch {} is still there.", ws.branch)
        };
        toast::done(window, format!("Removed {}", ws.name), detail, cx);
    }
}

/// A run of words with some of them brighter (a path, a branch).
fn sentence(id: &'static str, label: String, parts: Vec<(String, bool)>) -> Observed {
    div().id(id).test_support().aria_label(SharedString::from(label)).flex().flex_wrap().gap_x(px(4.)).children(
        parts.into_iter().map(|(words, bright)| {
            div().when(bright, |d| d.text_color(theme::INK).font_weight(FontWeight::MEDIUM)).child(words)
        }),
    )
}

impl Render for RemoveWorkspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ws = &self.workspace;
        let place = short_path(&ws.path);
        let what = if ws.adopted {
            // a folder Enkidu didn't make: only forgotten
            let words = format!(
                "Enkidu forgets it and closes its terminals. The folder {place} and everything in it stay as they are."
            );
            sentence(
                "remove-forgets",
                words,
                vec![
                    ("Enkidu forgets it and closes its terminals. The folder".into(), false),
                    (place.clone(), true),
                    ("and everything in it stay as they are.".into(), false),
                ],
            )
        } else {
            let words =
                format!("This deletes the folder {place} and closes its terminals. The branch {} stays.", ws.branch);
            sentence(
                "remove-deletes",
                words,
                vec![
                    ("This deletes the folder".into(), false),
                    (place.clone(), true),
                    ("and closes its terminals. The branch".into(), false),
                    (ws.branch.clone(), true),
                    ("stays.".into(), false),
                ],
            )
        };
        let enabled = !self.busy;
        let confirm = button(
            "confirm-remove",
            if self.dirty { "Remove anyway" } else { "Remove workspace" },
            ButtonKind::Danger,
            ButtonSize::Medium,
        );
        let confirm = if enabled { confirm } else { confirm.opacity(0.45).cursor_default() };
        let title = format!("Remove {}?", ws.name);
        let close = crate::dialog_parts::close("remove-close").on_click(cx.listener(|this, _, window, cx| {
            if !this.busy {
                window.close_dialog(cx);
            }
        }));
        let body = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .px(px(20.))
            .pt(px(12.))
            .pb(px(16.))
            .text_size(theme::TEXT)
            .line_height(px(20.))
            .text_color(theme::INK_2)
            .when(self.dirty, |d| {
                d.child(
                    div()
                        .id("dirty-warning")
                        .test_support()
                        .aria_label("It has uncommitted changes. Removing it deletes them.")
                        .px(px(12.))
                        .py(px(8.))
                        .rounded(px(8.))
                        .bg(alpha(theme::LAMP_RED, 0.1))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme::LAMP_RED)
                        .child("It has uncommitted changes. Removing it deletes them."),
                )
            })
            .child(what)
            .children(self.error.clone().map(|error| {
                div()
                    .id("remove-error")
                    .test_support()
                    .aria_label(SharedString::from(error.clone()))
                    .text_color(theme::LAMP_RED)
                    .child(error)
            }));
        let foot = crate::dialog_parts::foot()
            .child(button("cancel-remove", "Cancel", ButtonKind::Secondary, ButtonSize::Medium).on_click(cx.listener(
                |this, _, window, cx| {
                    if !this.busy {
                        window.close_dialog(cx);
                    }
                },
            )))
            .child(confirm.on_click(cx.listener(|this, _, window, cx| this.run(window, cx))));
        div()
            .id("remove-dialog")
            .test_support()
            .aria_label(SharedString::from(title.clone()))
            .relative()
            .w_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(crate::dialog_parts::head(title, None, Some(close)))
            .child(body)
            .child(foot)
            .child(crate::dialog_parts::lit_edge())
    }
}
