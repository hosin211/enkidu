//! The projects row (DESIGN.md, "Layout"; item 1.16): room for the traffic lights, then each project as a tab with its
//! badge (its color from its place), a red count on a project not on screen whose workspaces need you, Add project, and
//! on the right the commands and Settings. The project on screen opens its menu; another one comes on screen.

use std::path::PathBuf;

use enkidu_core::db::Project;
use enkidu_model::frame::Frame;
use enkidu_model::work::page::{TabMark, tab_mark};
use enkidu_ui::pieces::{
    MenuTrigger, icon_button, menu_item, menu_item_with_key, project_badge, project_badge_quiet, project_hue,
};
use enkidu_ui::tips::Tipped as _;
use enkidu_ui::tokens::{self as theme, EASE_POP, bezier};
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::prelude::*;
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, AnyWindowHandle, App, AsyncApp, Context, Entity, FontWeight, IntoElement,
    PathPromptOptions, Render, SharedString, Window, div, px, svg,
};

use crate::bridge::Bridge;
use crate::toast;
use crate::work::store::Work;

pub struct ProjectsRow {
    frame: Entity<Frame>,
    /// The Work index: the Work tab, the first of the row, counts what waits on you there. None in a window without
    /// one (some pictures).
    work: Option<Entity<Work>>,
}

impl ProjectsRow {
    pub fn new(frame: Entity<Frame>, work: Option<Entity<Work>>, cx: &mut Context<Self>) -> Self {
        cx.observe(&frame, |_, _, cx| cx.notify()).detach();
        if let Some(work) = &work {
            cx.observe(work, |_, _, cx| cx.notify()).detach();
        }
        Self { frame, work }
    }
}

/// A path as you'd say it (enkidu-model's rule): a home folder becomes `~`.
pub use enkidu_model::text::short_path;

/// How many workspaces wait, in words: the red count's label.
pub fn waiting_words(waiting: usize) -> String {
    if waiting == 1 { "1 workspace needs you".into() } else { format!("{waiting} workspaces need you") }
}

/// Add project: the folder picker, then the core adds it (off the drawing thread) and it comes on screen. What went
/// wrong (not a git repository, a linked worktree, already here) is said in a toast.
pub fn add_project(frame: Entity<Frame>, window: AnyWindowHandle, cx: &mut App) {
    let picked = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some("Add project".into()),
    });
    cx.spawn(async move |cx: &mut AsyncApp| {
        let Ok(Ok(Some(paths))) = picked.await else { return };
        let Some(path) = paths.into_iter().next() else { return };
        let path = path.to_string_lossy().to_string();
        let answer = cx.update(|cx| Bridge::ask(cx, move |shared| enkidu_core::commands::add_project(shared, path)));
        match answer.await {
            Ok(Ok(project)) => cx.update(|cx| {
                frame.update(cx, |frame, cx| {
                    frame.add_project(project);
                    cx.notify();
                })
            }),
            Ok(Err(err)) => toast::say_later(window, err.message, cx),
            Err(_) => {}
        }
    })
    .detach();
}

/// Remove project (one with no workspaces left): the core forgets it, the first one left comes on screen.
fn remove_project(frame: Entity<Frame>, id: String, window: AnyWindowHandle, cx: &mut App) {
    let key = id.clone();
    let answer = Bridge::ask(cx, move |shared| enkidu_core::commands::remove_project(shared, key));
    cx.spawn(async move |cx: &mut AsyncApp| match answer.await {
        Ok(Ok(())) => cx.update(|cx| {
            frame.update(cx, |frame, cx| {
                frame.remove_project(&id);
                cx.notify();
            })
        }),
        Ok(Err(err)) => toast::say_later(window, err.message, cx),
        Err(_) => {}
    })
    .detach();
}

/// The red count on a project that isn't on screen. It pops in (from half its size, fading in) each time it changes.
fn waiting_count(ix: usize, waiting: usize) -> AnyElement {
    red_count(("needs-you", ix).into(), format!("needs-you-pop-{ix}-{waiting}"), waiting, waiting_words(waiting))
}

/// A red count that pops in (from half its size, fading in) each time it changes: `motion` names that change. It
/// has no glow: red already calls, and the top row keeps its strongest mark for where you are (DESIGN.md, "States").
fn red_count(id: gpui_kit::ElementId, motion: String, waiting: usize, words: String) -> AnyElement {
    let pill = enkidu_ui::pieces::tabular(div())
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(theme::LAMP_RED)
        .text_color(theme::white(1.0))
        .font_weight(FontWeight::SEMIBOLD)
        .child(waiting.to_string())
        .with_animation(
            SharedString::from(motion),
            Animation::new(theme::DUR_SLOW).with_easing(|t| bezier(EASE_POP, t)),
            |pill, t| {
                let scale = 0.5 + 0.5 * t;
                pill.h(px(16. * scale))
                    .min_w(px(16. * scale))
                    .px(px(4. * scale))
                    .text_size(px(10. * scale))
                    .opacity(t.clamp(0., 1.))
            },
        );
    div()
        .id(id)
        .test_support()
        .aria_label(SharedString::from(words))
        .flex_none()
        .h(px(16.))
        .min_w(px(16.))
        .flex()
        .items_center()
        .justify_center()
        .child(pill)
        .into_any_element()
}

impl ProjectsRow {
    /// The first tab of the row, once there is a project: the Work page (it spans projects), with a red count of
    /// what waits on you while the page is closed, or a red mark when a source can't be read. A click opens the
    /// page, or goes back.
    fn work_tab(&self, active: bool, cx: &mut Context<Self>) -> Option<AnyElement> {
        let work = self.work.as_ref()?;
        let mark = {
            let work = work.read(cx);
            tab_mark(work.snapshot.as_deref(), &work.groups)
        };
        let words = mark.words();
        let frame = self.frame.clone();
        let tab = div()
            .id("work-tab")
            .test_support()
            .role(gpui_kit::Role::Button)
            .aria_label(SharedString::from(words.clone()))
            .aria_toggled(if active { gpui_kit::Toggled::True } else { gpui_kit::Toggled::False })
            .h(theme::PROJECT_TAB_HEIGHT)
            .flex_none()
            .flex()
            .items_center()
            .gap(px(6.))
            .rounded(theme::RADIUS_CONTROL)
            .pl(px(6.))
            .pr(px(8.))
            .text_size(theme::TEXT)
            .cursor_pointer()
            .map(|tab| match active {
                // an inset highlight along the top edge
                true => tab
                    .bg(theme::SELECTED)
                    .border_t_1()
                    .border_color(theme::HIGHLIGHT)
                    .font_weight(theme::WEIGHT_TITLE)
                    .text_color(theme::INK),
                false => tab.text_color(theme::INK_2).hover(|s| s.bg(theme::HOVER).text_color(theme::INK)),
            })
            .tipped(words, Some(Box::new(crate::ToggleWork)))
            .child(svg().path("icons/inbox.svg").flex_none().size(px(15.)).text_color(if active {
                theme::ACCENT
            } else {
                theme::INK_2
            }))
            .child("Work")
            .map(|tab| match mark {
                TabMark::Broken => tab.child(
                    div()
                        .id("work-tab-broken")
                        .test_support()
                        .aria_label("A source can't be read")
                        .flex_none()
                        .size(px(16.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(theme::LAMP_RED)
                        .text_size(px(10.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme::white(1.0))
                        .child("!"),
                ),
                TabMark::Waiting { items, .. } if !active => tab.child(red_count(
                    "work-tab-count".into(),
                    format!("work-tab-pop-{items}"),
                    items,
                    items.to_string(),
                )),
                _ => tab,
            })
            .on_click(move |_, _, cx| {
                frame.update(cx, |frame, cx| {
                    if frame.toggle_work() {
                        cx.notify();
                    }
                })
            });
        // a hairline sets it apart from the projects it spans
        let rule = div().flex_none().mx(px(6.)).h(px(16.)).w(px(1.)).bg(theme::RULE_STRONG);
        Some(div().flex_none().flex().items_center().child(tab).child(rule).into_any_element())
    }

    fn tab(&self, ix: usize, project: &Project, frame: &Frame, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        // behind the Work page no project is the one on screen: each is a tab that brings it back
        let active = frame.current_project.as_deref() == Some(project.id.as_str()) && !frame.work_open;
        let place = short_path(&project.path);
        // the project on screen keeps its color; another one's badge is gray (DESIGN.md, "States")
        let badge = match active {
            true => project_badge(&project.name, project_hue(ix), 18.),
            false => project_badge_quiet(&project.name, 18.),
        };
        let name = div()
            .min_w_0()
            .truncate()
            .text_size(theme::TEXT)
            .text_color(if active { theme::INK } else { theme::INK_2 })
            .when(active, |d| d.font_weight(theme::WEIGHT_TITLE))
            .child(project.name.clone());
        if active {
            let workspaces = frame.workspaces.iter().filter(|w| w.project_id == project.id).count();
            let (frame_entity, id, path) = (self.frame.clone(), project.id.clone(), PathBuf::from(&project.path));
            let handle = window.window_handle();
            return MenuTrigger::new(("project", ix), format!("{}: project menu", project.name))
                .h(theme::PROJECT_TAB_HEIGHT)
                .min_w_0()
                .max_w(px(220.))
                .pl(px(4.))
                .pr(px(8.))
                .flex()
                .items_center()
                .gap(px(6.))
                .rounded(theme::RADIUS_CONTROL)
                .bg(theme::SELECTED)
                // an inset highlight along the top edge
                .border_t_1()
                .border_color(theme::HIGHLIGHT)
                .cursor_pointer()
                .hover(|s| s.bg(theme::SELECTED_HOVER))
                .child(badge)
                .child(name)
                .child(svg().path("icons/chevron-down.svg").flex_none().size(px(12.)).text_color(theme::INK_3))
                .menu(gpui_kit::Anchor::TopLeft, move |menu, _, _| {
                    let (path, frame, id) = (path.clone(), frame_entity.clone(), id.clone());
                    let for_new = frame.clone();
                    let (for_settings, its_page) = (frame.clone(), enkidu_model::settings::Page::Project(id.clone()));
                    menu.label(place.clone())
                        .item(
                            menu_item_with_key(
                                "new-workspace-menu",
                                "icons/plus.svg",
                                "New workspace",
                                "⌘N",
                                theme::INK,
                            )
                            .on_click(move |_, window, cx| crate::new_workspace::open(for_new.clone(), window, cx)),
                        )
                        .item(
                            menu_item(
                                "project-settings",
                                "icons/sliders-horizontal.svg",
                                "Project settings",
                                theme::INK,
                            )
                            .on_click(move |_, window, cx| {
                                crate::settings::open_here(for_settings.clone(), its_page.clone(), window, cx)
                            }),
                        )
                        .item(
                            menu_item("show-in-finder", "icons/folder-open.svg", "Show in Finder", theme::INK)
                                .on_click(move |_, _, cx| cx.reveal_path(&path)),
                        )
                        .separator()
                        .item(
                            remove_item(workspaces)
                                .on_click(move |_, _, cx| remove_project(frame.clone(), id.clone(), handle, cx)),
                        )
                })
                .into_any_element();
        }
        let id = project.id.clone();
        let waiting = frame.waiting(&project.id);
        enkidu_ui::press::pressable(div().id(("project", ix)))
            .test_support()
            .aria_label(project.name.clone())
            .h(theme::PROJECT_TAB_HEIGHT)
            .min_w_0()
            .max_w(px(220.))
            .pl(px(4.))
            .pr(px(8.))
            .flex()
            .items_center()
            .gap(px(6.))
            .rounded(theme::RADIUS_CONTROL)
            .cursor_pointer()
            .hover(|s| s.bg(theme::HOVER))
            .tipped(place.clone(), None)
            .child(badge)
            .child(name)
            .when(waiting > 0, |d| d.child(waiting_count(ix, waiting)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.frame.update(cx, |frame, cx| {
                    if frame.select_project(&id) {
                        cx.notify();
                    }
                })
            }))
            .into_any_element()
    }
}

/// Remove project, in red; while the project still has workspaces it says so and can't be chosen.
fn remove_item(workspaces: usize) -> PopupMenuItem {
    let blocked = workspaces > 0;
    let words = if blocked { "Remove its workspaces first" } else { "Remove project" };
    let color = if blocked { theme::INK_3 } else { theme::LAMP_RED };
    menu_item("remove-project", "icons/trash.svg", words, color).disabled(blocked)
}

impl Render for ProjectsRow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let frame = self.frame.read(cx).clone();
        let tabs: Vec<AnyElement> =
            frame.projects.iter().enumerate().map(|(ix, p)| self.tab(ix, p, &frame, window, cx)).collect();
        let (frame_entity, handle) = (self.frame.clone(), window.window_handle());
        div()
            .id("projects-row")
            .test_support()
            .aria_label("Projects")
            .w_full()
            .h(theme::PROJECTS_ROW)
            .flex_none()
            .flex()
            .items_center()
            // room on the left for the traffic lights, which the window draws itself
            .pl(px(84.))
            .pr(px(10.))
            .gap(px(2.))
            .children(self.work_tab(frame.work_open, cx).filter(|_| !frame.projects.is_empty()))
            .children(tabs)
            .child(
                icon_button("add-project", "icons/plus.svg", "Add project")
                    .tipped("Add project", None)
                    .on_click(move |_, _, cx| add_project(frame_entity.clone(), handle, cx)),
            )
            .child(div().flex_1())
            .child({
                let frame = self.frame.clone();
                icon_button("palette", "icons/command.svg", "Commands")
                    .tipped("Commands", Some(Box::new(crate::TogglePalette)))
                    .on_click(move |_, window, cx| crate::palette::open(frame.clone(), window, cx))
            })
            .child({
                let frame = self.frame.clone();
                icon_button("settings", "icons/settings.svg", "Settings")
                    .tipped("Settings", Some(Box::new(crate::OpenSettings)))
                    .on_click(move |_, window, cx| {
                        crate::settings::open_here(frame.clone(), enkidu_model::settings::Page::General, window, cx)
                    })
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_and_counts_in_words() {
        let home = std::env::var("HOME").unwrap();
        assert_eq!(short_path(&format!("{home}/code/shop")), "~/code/shop");
        assert_eq!(short_path("/opt/shop"), "/opt/shop");
        assert_eq!(waiting_words(1), "1 workspace needs you");
        assert_eq!(waiting_words(3), "3 workspaces need you");
    }
}
