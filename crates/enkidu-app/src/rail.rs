//! The rail (DESIGN.md, "Layout": "The rail is everything you have"): Work first, with a red count when something
//! waits on you; then every project as its own icon or its letter; then the + that adds a project; and at the foot
//! the command palette and Settings.
//!
//! A click on a project goes to it, on the workspace you had there. The project on screen has the chosen fill, and
//! its name stands in the top row, where it opens the project's menu ([`project_menu`]). A project whose tabs are
//! not on screen has a red count when one of its Claudes needs you.

use std::collections::HashMap;
use std::path::PathBuf;

use enkidu_core::db::Project;
use enkidu_model::frame::Frame;
use enkidu_model::work::page::{TabMark, tab_mark};
use enkidu_ui::pieces::{MenuTrigger, menu_item, menu_item_with_key, project_badge, project_hue, tabular};
use enkidu_ui::tips::Tipped as _;
use enkidu_ui::tokens::{self as theme, EASE_POP, bezier};
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::prelude::*;
use gpui_kit::{
    Action, Animation, AnimationExt as _, AnyElement, AnyWindowHandle, App, AsyncApp, BoxShadow, Context, ElementId,
    Entity, FontWeight, IntoElement, PathPromptOptions, Render, Role, SharedString, Stateful, Window, div, img, point,
    px, svg,
};

use crate::bridge::Bridge;
use crate::stores::Icons;
use crate::tabs_row::short_path;
use crate::toast;
use crate::work::store::Work;

pub struct Rail {
    frame: Entity<Frame>,
    /// The Work index: Work's button counts what waits on you there. None in a window without one (some pictures).
    work: Option<Entity<Work>>,
    /// Each project's own icon, as the stores found it. None in a window without stores.
    icons: Option<Entity<Icons>>,
    /// The icons given to a window made for pictures, by project.
    given: HashMap<String, PathBuf>,
}

/// How many of a project's Claudes need you, in words (the label of the red count on its button).
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

/// Remove project, in red; while the project still has workspaces it says so and can't be chosen.
fn remove_item(workspaces: usize) -> PopupMenuItem {
    let blocked = workspaces > 0;
    let words = if blocked { "Remove its workspaces first" } else { "Remove project" };
    let color = if blocked { theme::INK_3 } else { theme::LAMP_RED };
    menu_item("remove-project", "icons/trash.svg", words, color).disabled(blocked)
}

/// The name of the project on screen, in the top row: it opens the project's menu (its folder, New workspace,
/// Project settings, Show in Finder, Remove project). `at` is the project's place on the rail.
pub(crate) fn project_menu(
    store: Entity<Frame>,
    at: usize,
    project: &Project,
    frame: &Frame,
    window: &Window,
) -> impl IntoElement + use<> {
    let place = short_path(&project.path);
    let workspaces = frame.workspaces.iter().filter(|w| w.project_id == project.id).count();
    let (id, path) = (project.id.clone(), PathBuf::from(&project.path));
    let handle = window.window_handle();
    MenuTrigger::new("project-menu", format!("{}: project menu", project.name))
        .tip(place.clone())
        .aria_value(SharedString::from(at.to_string()))
        .h(theme::CONTROL)
        .flex_none()
        .max_w(px(240.))
        .px(px(8.))
        .flex()
        .items_center()
        .gap(px(4.))
        .rounded(theme::RADIUS_CONTROL)
        .cursor_pointer()
        .hover(|s| s.bg(theme::HOVER))
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_size(px(13.5))
                .font_weight(theme::WEIGHT_TITLE)
                .text_color(theme::INK)
                .child(project.name.clone()),
        )
        .child(svg().path("icons/chevron-down.svg").flex_none().size(theme::MARK).text_color(theme::INK_3))
        .menu(gpui_kit::Anchor::TopLeft, move |menu, _, _| {
            let (path, frame, id) = (path.clone(), store.clone(), id.clone());
            let (for_new, in_project) = (frame.clone(), id.clone());
            let (for_settings, its_page) = (frame.clone(), enkidu_model::settings::Page::Project(id.clone()));
            menu.item(enkidu_ui::pieces::menu_label(place.clone()))
                .item(
                    menu_item_with_key("new-workspace-menu", "icons/plus.svg", "New workspace", "⌘N", theme::INK)
                        .on_click(move |_, window, cx| {
                            crate::new_workspace::open_in(for_new.clone(), &in_project, window, cx)
                        }),
                )
                .item(
                    menu_item("project-settings", "icons/sliders-horizontal.svg", "Project settings", theme::INK)
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
}

/// A project's picture, `side` px: its own icon when `icon` names one, else its letter on its color (its letter
/// too while the picture is read, or when it can't be). It is the same wherever a project is shown: on the rail,
/// on a card of the switcher, in the dialog of a task of your own.
pub(crate) fn project_picture(name: &str, hue: f32, side: f32, icon: Option<PathBuf>) -> AnyElement {
    let Some(path) = icon else { return project_badge(name, hue, side).into_any_element() };
    let (loading, failed) = (name.to_string(), name.to_string());
    img(path)
        .flex_none()
        .size(px(side))
        // (a badge's own corner: 8 px on the rail, 4 px beside a name)
        .rounded(px(if side >= 24. { 8. } else { 4. }))
        .with_loading(move || project_badge(&loading, hue, side).into_any_element())
        .with_fallback(move || project_badge(&failed, hue, side).into_any_element())
        .into_any_element()
}

/// A red count at the corner of a button of the rail, with a ring in the frame's color around it. It pops in (from
/// half its size, fading in) each time it changes: `motion` names that change. It has no glow: red already calls
/// (DESIGN.md, "States").
fn red_count(id: ElementId, motion: String, said: String, words: String) -> AnyElement {
    let ring = BoxShadow {
        color: theme::PANE.into(),
        offset: point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: px(2.),
        inset: false,
    };
    let pill = tabular(div())
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(theme::LAMP_RED)
        .shadow(vec![ring])
        .text_color(theme::white(1.0))
        .font_weight(FontWeight::BOLD)
        .child(said)
        .with_animation(
            SharedString::from(motion),
            Animation::new(theme::DUR_SLOW).with_easing(|t| bezier(EASE_POP, t)),
            |pill, t| {
                let scale = 0.5 + 0.5 * t;
                pill.h(px(16. * scale))
                    .min_w(px(16. * scale))
                    .px(px(4. * scale))
                    .text_size(px(11. * scale))
                    .opacity(t.clamp(0., 1.))
            },
        );
    div()
        .id(id)
        .test_support()
        .aria_label(SharedString::from(words))
        .absolute()
        .top_0()
        .right_0()
        .h(theme::BADGE)
        .min_w(theme::BADGE)
        .flex()
        .items_center()
        .justify_center()
        .child(pill)
        .into_any_element()
}

/// A button of the rail: 40 px with a 10 px corner, what it holds in its middle. The one for what is on screen has
/// the chosen fill.
fn button(id: impl Into<ElementId>, label: impl Into<SharedString>, on: bool) -> enkidu_ui::pieces::Pressed {
    enkidu_ui::press::pressable(div().id(id))
        .test_support()
        .role(Role::Button)
        .aria_label(label.into())
        .aria_selected(on)
        .relative()
        .size(theme::RAIL_BUTTON)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(theme::RADIUS_CARD)
        .cursor_pointer()
        .group("rail-button")
        .map(|button| match on {
            true => button.bg(theme::SELECTED),
            false => button.hover(|s| s.bg(theme::HOVER)).active(|s| s.bg(theme::SOFT)),
        })
}

/// The sign in a button of the rail: bright on the one for what is on screen, and under the pointer.
fn sign(icon: &'static str, on: bool) -> gpui_kit::Svg {
    svg()
        .path(icon)
        .size(theme::ICON)
        .text_color(if on { theme::INK } else { theme::INK_3 })
        .group_hover("rail-button", |s| s.text_color(theme::INK))
}

impl Rail {
    pub fn new(frame: Entity<Frame>, work: Option<Entity<Work>>, cx: &mut Context<Self>) -> Self {
        cx.observe(&frame, |_, _, cx| cx.notify()).detach();
        if let Some(work) = &work {
            cx.observe(work, |_, _, cx| cx.notify()).detach();
        }
        let icons = cx.try_global::<crate::stores::Stores>().map(|stores| stores.icons.clone());
        if let Some(icons) = &icons {
            cx.observe(icons, |_, _, cx| cx.notify()).detach();
        }
        Self { frame, work, icons, given: HashMap::new() }
    }

    /// For pictures: these projects show these icons.
    pub fn show_icons(&mut self, icons: HashMap<String, PathBuf>, cx: &mut Context<Self>) {
        self.given = icons;
        cx.notify();
    }

    /// The picture a project's button shows, if it has one of its own.
    pub(crate) fn icon_of(&self, project: &str, cx: &App) -> Option<PathBuf> {
        (self.given.get(project).cloned())
            .or_else(|| self.icons.as_ref().and_then(|icons| icons.read(cx).of.get(project).cloned()))
    }

    /// The first button, once there is a project: the Work page (it spans the projects), with a red count of what
    /// waits on you while the page is closed, or a red mark when a source can't be read. A click opens the page, or
    /// goes back.
    fn work_button(&self, open: bool, cx: &mut Context<Self>) -> Option<AnyElement> {
        let work = self.work.as_ref()?;
        let mark = {
            let work = work.read(cx);
            tab_mark(work.snapshot.as_deref(), &work.groups)
        };
        let words = mark.words();
        let frame = self.frame.clone();
        let button = button("work-tab", SharedString::from(words.clone()), open)
            .aria_toggled(if open { gpui_kit::Toggled::True } else { gpui_kit::Toggled::False })
            .tipped_beside(words, Some(Box::new(crate::ToggleWork)))
            .child(sign("icons/inbox.svg", open))
            .map(|button| match mark {
                TabMark::Broken => button.child(red_count(
                    "work-tab-broken".into(),
                    "work-tab-broken-pop".into(),
                    "!".into(),
                    "A source can't be read".into(),
                )),
                TabMark::Waiting { items, .. } if !open => button.child(red_count(
                    "work-tab-count".into(),
                    format!("work-tab-pop-{items}"),
                    items.to_string(),
                    items.to_string(),
                )),
                _ => button,
            })
            .on_click(move |_, _, cx| {
                frame.update(cx, |frame, cx| {
                    if frame.toggle_work() {
                        cx.notify();
                    }
                })
            });
        Some(button.into_any_element())
    }

    /// A project: its own icon or its letter. A click goes to it. `on`: it is the project on screen, with its tabs
    /// in the top row; any other one says with a red count how many of its Claudes need you.
    fn project(&self, at: usize, project: &Project, frame: &Frame, on: bool, cx: &mut Context<Self>) -> AnyElement {
        let waiting = if on { 0 } else { frame.waiting(&project.id) };
        let id = project.id.clone();
        let (name, hue, side) = (project.name.clone(), project_hue(at), f32::from(theme::RAIL_PICTURE));
        let picture = match self.icon_of(&project.id, cx) {
            Some(path) => div()
                .id(("project-icon", at))
                .test_support()
                .aria_label(SharedString::from(format!("{name}'s own icon")))
                .aria_description(SharedString::from(short_path(&path.to_string_lossy())))
                .flex_none()
                .size(theme::RAIL_PICTURE)
                .child(project_picture(&name, hue, side, Some(path)))
                .into_any_element(),
            None => project_picture(&name, hue, side, None),
        };
        button(("project", at), SharedString::from(project.name.clone()), on)
            .aria_description(SharedString::from(short_path(&project.path)))
            .tipped_beside(project.name.clone(), None)
            .child(picture)
            .when(waiting > 0, |button| {
                button.child(red_count(
                    ("project-waiting", at).into(),
                    format!("project-waiting-pop-{at}-{waiting}"),
                    waiting.to_string(),
                    waiting_words(waiting),
                ))
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.frame.update(cx, |frame, cx| {
                    if frame.select_project(&id) {
                        cx.notify();
                    }
                })
            }))
            .into_any_element()
    }

    /// One of the two buttons at the foot: the command palette, Settings.
    fn foot_button(
        id: &'static str,
        icon: &'static str,
        words: &'static str,
        key: Box<dyn Action>,
    ) -> enkidu_ui::pieces::Pressed {
        button(id, words, false).tipped_beside(words, Some(key)).child(sign(icon, false))
    }
}

/// The hairline under Work: it spans the projects under it.
fn rule() -> Stateful<gpui_kit::Div> {
    div().id("rail-rule").flex_none().w(px(24.)).h(px(1.)).bg(theme::LINE_STRONG)
}

impl Render for Rail {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::measure::RAILS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let frame = self.frame.read(cx).clone();
        let mut above: Vec<AnyElement> = Vec::new();
        let mut projects: Vec<AnyElement> = Vec::new();
        // nothing until the database was read; with no project yet the screen beside has the one Add project
        if frame.loaded && !frame.projects.is_empty() {
            if let Some(work) = self.work_button(frame.work_open, cx) {
                above.push(work);
                above.push(rule().into_any_element());
            }
            for (at, project) in frame.projects.iter().enumerate() {
                // behind the Work page no project is the one on screen: each button brings its project back
                let on = frame.current_project.as_deref() == Some(project.id.as_str()) && !frame.work_open;
                projects.push(self.project(at, project, &frame, on, cx));
            }
            let (store, handle) = (self.frame.clone(), window.window_handle());
            projects.push(
                button("add-project", "Add project", false)
                    .tipped_beside("Add project", None)
                    .child(sign("icons/plus.svg", false))
                    .on_click(move |_, _, cx| add_project(store.clone(), handle, cx))
                    .into_any_element(),
            );
        }
        let (for_palette, for_settings) = (self.frame.clone(), self.frame.clone());
        div()
            .id("rail")
            .test_support()
            .aria_label("Projects")
            .w(theme::RAIL)
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(8.))
            .pt(px(4.))
            .pb(px(8.))
            .children(above)
            .child(
                div()
                    .id("rail-projects")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(8.))
                    .children(projects),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        Self::foot_button("palette", "icons/command.svg", "Commands", Box::new(crate::TogglePalette))
                            .on_click(move |_, window, cx| crate::palette::open(for_palette.clone(), window, cx)),
                    )
                    .child(
                        Self::foot_button("settings", "icons/settings.svg", "Settings", Box::new(crate::OpenSettings))
                            .on_click(move |_, window, cx| {
                                let page = enkidu_model::settings::Page::General;
                                crate::settings::open_here(for_settings.clone(), page, window, cx)
                            }),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_projects_count_in_words() {
        assert_eq!(waiting_words(1), "1 workspace needs you");
        assert_eq!(waiting_words(3), "3 workspaces need you");
    }
}
