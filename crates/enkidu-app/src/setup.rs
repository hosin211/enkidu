//! Settings → Setup (more of the Settings view's `impl`; DESIGN.md, "Setup"; items 21.5 and 21.6): what
//! the app needs on this Mac, checked in one place. Two parts, each thing a row on a tile: a lamp, its name, how it
//! stands, one line, and the commands to run as chips a press copies. Under Jira, the projects whose tickets aren't
//! read yet lead to their own pages. "Check again" looks at everything once more. The page saves nothing.
//!
//! The core looks (`enkidu_core::setup`): each thing is asked for on a thread of its own, so a row fills in as soon as
//! its program answered and a slow one (twg asks Atlassian) holds up nothing. What a row says is worked out in
//! `enkidu_model::setup`. On a first launch the page opens by itself, once, with a welcome above it.

use std::collections::{HashMap, HashSet};

use enkidu_core::setup::{Found, Parts, State, Thing};
use enkidu_model::settings::Page;
use enkidu_model::setup::{self as words, Lamp, Row};
use enkidu_ui::pieces::{ButtonKind, ButtonSize, button, mono, with_code};
use enkidu_ui::shimmer::shimmer;
use enkidu_ui::tokens as theme;
use enkidu_ui::work::{SourceLook, source_lamp};
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, ClipboardItem, Context, Div, FontWeight, IntoElement, Role, SharedString, WeakEntity, div, px, svg,
};

use crate::bridge::Bridge;
use crate::settings::{Settings, Source};
use crate::toast;

/// What the Setup page knows.
#[derive(Default)]
pub struct Setup {
    /// What was found of each thing. A thing that isn't here is being looked at, or wasn't asked for yet.
    pub(crate) found: HashMap<Thing, Found>,
    /// The things being looked at now.
    pub(crate) looking: HashSet<Thing>,
    /// The parts this build carries beside its program.
    pub(crate) parts: Option<Parts>,
    /// The projects whose Jira tickets aren't read yet: their ids and names.
    pub(crate) without_jira: Vec<(String, String)>,
    /// The first launch: a welcome stands above the page.
    pub(crate) welcome: bool,
    /// The page was looked at once: it looks by itself only the first time it comes on screen.
    asked: bool,
    /// Counts the looks, so the answer to an older one is dropped.
    round: u32,
}

/// What a dialog without a core shows on the page (a picture, a screen test).
#[derive(Clone, Debug, Default)]
pub struct Given {
    pub found: Vec<Found>,
    pub parts: Option<Parts>,
    pub without_jira: Vec<(String, String)>,
}

impl Settings {
    /// The page came on screen: the first time, everything is looked at.
    pub(crate) fn ensure_setup(&mut self, cx: &mut Context<Self>) {
        if !self.setup.asked {
            self.look_at_setup(cx);
        }
    }

    /// Looks at every thing again (and the first time). Each thing is asked for on its own, so its row fills in as
    /// soon as its program answered.
    pub(crate) fn look_at_setup(&mut self, cx: &mut Context<Self>) {
        self.setup.asked = true;
        self.setup.round += 1;
        let round = self.setup.round;
        match &self.source {
            Source::Given(given) => {
                self.setup.found = given.setup.found.iter().map(|found| (found.thing, found.clone())).collect();
                self.setup.looking.clear();
                self.setup.parts = given.setup.parts;
                self.setup.without_jira = given.setup.without_jira.clone();
            }
            Source::Core => {
                self.setup.found.clear();
                self.setup.looking = Thing::ALL.into_iter().collect();
                for thing in Thing::ALL {
                    let looked = Bridge::ask(cx, move |shared| {
                        enkidu_core::setup::look(thing, &enkidu_core::setup::tools(shared))
                    });
                    cx.spawn(async move |this: WeakEntity<Self>, cx: &mut gpui_kit::AsyncApp| {
                        let found = looked.await.ok();
                        let _ = this.update(cx, |this, cx| {
                            if this.setup.round != round {
                                return;
                            }
                            this.setup.looking.remove(&thing);
                            if let Some(found) = found {
                                this.setup.found.insert(thing, found);
                            }
                            cx.notify();
                        });
                    })
                    .detach();
                }
                let rest = Bridge::ask(cx, |shared| {
                    (enkidu_core::setup::parts(shared), enkidu_core::setup::without_jira(shared))
                });
                cx.spawn(async move |this: WeakEntity<Self>, cx: &mut gpui_kit::AsyncApp| {
                    if let Ok((parts, without_jira)) = rest.await {
                        let _ = this.update(cx, |this, cx| {
                            if this.setup.round == round {
                                this.setup.parts = Some(parts);
                                this.setup.without_jira = without_jira;
                                cx.notify();
                            }
                        });
                    }
                })
                .detach();
            }
        }
        cx.notify();
    }

    /// A thing's row as the page says it: what was found, or that it is being looked at.
    fn setup_row(&self, thing: Thing) -> Row {
        words::row(thing, self.setup.found.get(&thing).filter(|_| !self.setup.looking.contains(&thing)))
    }

    /// The Setup page.
    pub(crate) fn setup_page(&self, cx: &mut Context<Self>) -> AnyElement {
        let looking = !self.setup.looking.is_empty();
        let check = button("setup-check", words::CHECK_AGAIN, ButtonKind::Secondary, ButtonSize::Small)
            .when(looking, |check| check.opacity(0.45).cursor_default())
            .on_click(cx.listener(move |this, _, _, cx| {
                if this.setup.looking.is_empty() {
                    this.look_at_setup(cx);
                }
            }));
        let welcome =
            self.setup.welcome.then(|| {
                div()
                    .id("setup-welcome")
                    .test_support()
                    .aria_label(words::WELCOME)
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .pb(px(4.))
                    .child(
                        div()
                            .text_size(px(17.))
                            .line_height(px(24.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::INK)
                            .child(words::WELCOME),
                    )
                    .children(words::WELCOME_SAYS.map(|line| {
                        div().text_size(theme::TEXT).line_height(px(20.)).text_color(theme::INK_2).child(line)
                    }))
            });
        let needs = [Thing::Git, Thing::Claude].map(|thing| self.tile(&self.setup_row(thing), cx));
        let jira_ready = self.setup.found.get(&Thing::Jira).is_some_and(|found| found.state == State::Ready)
            && !self.setup.looking.contains(&Thing::Jira);
        let connections =
            vec![self.tile(&self.setup_row(Thing::Github), cx), self.tile(&self.setup_row(Thing::Jira), cx)];
        let projects = (jira_ready && !self.setup.without_jira.is_empty()).then(|| self.jira_projects(cx));
        let build: Vec<Row> = self.setup.parts.map(words::missing_parts).unwrap_or_default();
        let build = (!build.is_empty()).then(|| {
            part(words::BUILD, words::BUILD_SAYS, None, false)
                .children(build.iter().map(|row| self.tile(row, cx)).collect::<Vec<_>>())
        });
        div()
            .id("setup-page")
            .test_support()
            .aria_label("Setup")
            .aria_description(if looking { "looking" } else { "looked" })
            .flex()
            .flex_col()
            .gap(px(24.))
            .children(welcome)
            .child(part(words::NEEDS, words::NEEDS_SAY, Some(check.into_any_element()), true).children(needs))
            .child(
                part(words::CONNECTIONS, words::CONNECTIONS_SAY, None, false).children(connections).children(projects),
            )
            .children(build)
            .into_any_element()
    }

    /// A thing on its tile: the lamp, its name and how it stands; under them one line; under that the commands.
    fn tile(&self, row: &Row, cx: &mut Context<Self>) -> AnyElement {
        let look = match row.lamp {
            Lamp::Ready => SourceLook::Ok,
            Lamp::Needed => SourceLook::Error,
            Lamp::Off => SourceLook::Off,
            Lamp::Looking => SourceLook::Reading,
        };
        // the lamp carries the color; the word beside it is red only where a workspace can't start without the thing
        let stands_color = if row.lamp == Lamp::Needed { theme::LAMP_RED } else { theme::INK_3 };
        let id = |part: &str| SharedString::from(format!("setup-{part}:{}", row.id));
        let line: AnyElement = match row.lamp {
            // looked at now: the words shimmer, as "Claude is working" does
            Lamp::Looking => {
                div().text_color(theme::INK_3).child(shimmer(id("looking"), row.line.clone())).into_any_element()
            }
            _ => with_code(&row.line, theme::INK_3, theme::INK_2).into_any_element(),
        };
        let commands = (!row.commands.is_empty()).then(|| {
            div().pl(px(16.)).flex().flex_wrap().gap(px(8.)).children(row.commands.iter().enumerate().map(
                |(ix, command)| {
                    let copied = command.clone();
                    mono(div())
                        .id(SharedString::from(format!("setup-copy:{}:{ix}", row.id)))
                        .test_support()
                        .role(Role::Button)
                        .aria_label(SharedString::from(format!("Copy {command}")))
                        .max_w_full()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .rounded(px(6.))
                        .bg(theme::PANE)
                        .px(px(8.))
                        .py(px(4.))
                        .text_size(theme::MONO_TEXT)
                        .line_height(px(16.))
                        .text_color(theme::INK_2)
                        .cursor_pointer()
                        .hover(|chip| chip.text_color(theme::INK))
                        .child(div().min_w_0().truncate().child(command.clone()))
                        .child(svg().path("icons/copy.svg").flex_none().size(px(12.)).text_color(theme::INK_3))
                        .on_click(cx.listener(move |_, _, window, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(copied.clone()));
                            toast::good(window, "Copied", cx);
                        }))
                },
            ))
        });
        div()
            .id(id("row"))
            .test_support()
            .aria_label(row.name)
            .aria_description(SharedString::from(format!("{}: {}", row.stands, row.line)))
            .flex()
            .flex_col()
            .gap(px(4.))
            .rounded(px(8.))
            .bg(theme::PANE)
            .border_1()
            .border_color(theme::LINE)
            .px(px(16.))
            .py(px(12.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(source_lamp(id("lamp"), look, 8.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(theme::TEXT)
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::INK)
                            .child(row.name),
                    )
                    .when(!row.stands.is_empty(), |head| {
                        head.child(
                            div()
                                .id(id("stands"))
                                .test_support()
                                .aria_label(row.stands)
                                .flex_none()
                                .text_size(theme::TEXT_SMALL)
                                .text_color(stands_color)
                                .child(row.stands),
                        )
                    }),
            )
            .child(div().pl(px(16.)).text_size(theme::TEXT_SMALL).line_height(px(16.)).child(line))
            .children(commands)
            .into_any_element()
    }

    /// Under Jira: the projects whose tickets aren't read yet, each a quiet button that goes to its page.
    fn jira_projects(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("setup-jira-projects")
            .test_support()
            .aria_label(words::JIRA_PROJECTS)
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.))
            .pl(px(4.))
            .text_size(theme::TEXT_SMALL)
            .line_height(px(16.))
            .text_color(theme::INK_3)
            .child(words::JIRA_PROJECTS)
            .children(self.setup.without_jira.iter().enumerate().map(|(ix, (project, name))| {
                let page = Page::Project(project.clone());
                button(("setup-jira-project", ix), name.clone(), ButtonKind::Secondary, ButtonSize::Small)
                    .on_click(cx.listener(move |this, _, window, cx| this.show(page.clone(), window, cx)))
            }))
            .into_any_element()
    }
}

/// A part of the page: its title, one line under it, and what stands at the title's right (Check again).
fn part(title: &'static str, says: &'static str, right: Option<AnyElement>, first: bool) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .when(!first, |part| part.pt(px(24.)).border_t_1().border_color(theme::LINE))
        .child(
            div()
                .flex()
                .items_start()
                .gap(px(16.))
                .pb(px(4.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(
                            div()
                                .text_size(theme::TEXT)
                                .line_height(px(20.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme::INK)
                                .child(title),
                        )
                        .child(
                            div()
                                .text_size(theme::TEXT_SMALL)
                                .line_height(px(16.))
                                .text_color(theme::INK_3)
                                .child(says),
                        ),
                )
                .children(right),
        )
}
