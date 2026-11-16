//! The island's terminals (item 2.11; DESIGN.md, "Layout" and "Icons"): a band of tabs, Claude first with its filled
//! coral sparkle, then shells with a green terminal, and New terminal; under it the terminal on screen. While Claude
//! works, the hairline under the tabs runs amber. A terminal whose program ended offers to start it again; a Claude
//! that couldn't start says why.

use std::time::Duration;

use enkidu_model::frame::Frame;
use enkidu_ui::pieces::{
    ButtonKind, ButtonSize, MenuTrigger, button, button_tinted, button_with, island, lit_edge, menu_item_tinted, raised,
};
use enkidu_ui::running_line::running_line;
use enkidu_ui::tokens::{self as theme, alpha, tone};
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, AsyncApp, Context, Entity, FocusHandle, Focusable as _, FontWeight, IntoElement, Render, SharedString,
    StyleRefinement, Task, WeakEntity, Window, div, px, svg,
};

use crate::terminal_view::TerminalView;
use crate::terminals::Terminals;
use crate::toast;

/// "Starting Claude…" shows only when the wait is long enough to notice (DESIGN.md, Principle 5).
const SLOW_START: Duration = Duration::from_millis(800);

/// A shell's green.
fn shell_green() -> gpui_kit::Rgba {
    tone(140., 0.13)
}

pub struct TerminalArea {
    frame: Entity<Frame>,
    terminals: Entity<Terminals>,
    /// Two terminals side by side (the measuring plans).
    split: bool,
    /// The window's own focus, held while no terminal is there to hold the keyboard.
    holder: FocusHandle,
    /// The workspace and the terminal on screen when last looked at.
    shown: Option<(String, Option<String>)>,
    /// The wait for Claude to start has been long enough to say so.
    slow: bool,
    waiting: Option<Task<()>>,
}

/// What the band and the body draw of one terminal.
struct TabDrawn {
    id: String,
    claude: bool,
    label: String,
    exited: bool,
    exit_code: Option<i32>,
    active: bool,
    view: Entity<TerminalView>,
}

impl TerminalArea {
    pub fn new(
        frame: Entity<Frame>,
        terminals: Entity<Terminals>,
        split: bool,
        holder: FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe_in(&frame, window, |this: &mut Self, _, window, cx| this.look(window, cx)).detach();
        cx.observe_in(&terminals, window, |this: &mut Self, _, window, cx| this.look(window, cx)).detach();
        let mut area = TerminalArea { frame, terminals, split, holder, shown: None, slow: false, waiting: None };
        area.look(window, cx);
        area
    }

    /// The view of the terminal on screen.
    /// Whether the keyboard is in this island: in one of its terminals, or held for it while it has none.
    pub fn has_keyboard(&self, window: &Window, cx: &gpui_kit::App) -> bool {
        self.holder.is_focused(window)
            || self.active_view(cx).is_some_and(|view| view.read(cx).focus_handle(cx).contains_focused(window, cx))
    }

    /// The terminal on screen takes the keyboard (the changes left the screen while they had it).
    pub fn take_keyboard(&self, window: &mut Window, cx: &mut gpui_kit::App) {
        match self.active_view(cx) {
            Some(view) => {
                let terminal = view.read(cx).focus_handle(cx);
                terminal.focus(window, cx);
            }
            None => self.holder.focus(window, cx),
        }
    }

    pub fn active_view(&self, cx: &gpui_kit::App) -> Option<Entity<TerminalView>> {
        let workspace = self.frame.read(cx).current_workspace()?.id.clone();
        self.terminals.read(cx).active_view(&workspace)
    }

    /// Something changed: the workspace on screen gets its terminals, a shell that didn't open is said, and the
    /// terminal that came on screen takes the keyboard.
    fn look(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let workspace = self.frame.read(cx).current_workspace().map(|w| w.id.clone());
        if let Some(ws) = &workspace {
            self.terminals.update(cx, |terminals, cx| terminals.ensure(ws, cx));
            if let Some(why) = self.terminals.update(cx, |terminals, _| terminals.take_notice(ws)) {
                toast::warning(window, "Couldn't open a shell", why, cx);
            }
        }
        let area = workspace.as_ref().and_then(|ws| self.terminals.read(cx).of(ws));
        let starting = area.is_none_or(|a| !a.known && a.error.is_none());
        let active = area.and_then(|a| a.active.clone());
        let now = workspace.map(|ws| (ws, active));
        if now != self.shown {
            let was = std::mem::replace(&mut self.shown, now);
            self.give_the_keyboard(was, window, cx);
        }
        self.wait_for_claude(starting && self.shown.is_some(), cx);
        cx.notify();
    }

    /// The terminal that came on screen takes the keyboard: always when another workspace came on screen, and for a new
    /// or another tab unless a dialog or the palette has the keyboard.
    fn give_the_keyboard(
        &mut self,
        was: Option<(String, Option<String>)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((workspace, active)) = self.shown.clone() else { return };
        let another_workspace = was.as_ref().map(|w| &w.0) != Some(&workspace);
        let Some(view) = active.and(self.terminals.read(cx).active_view(&workspace)) else {
            if another_workspace {
                self.holder.focus(window, cx);
            }
            return;
        };
        let free = match window.focused(cx) {
            None => true,
            Some(focused) => {
                focused == self.holder
                    || self
                        .terminals
                        .read(cx)
                        .of(&workspace)
                        .is_some_and(|area| area.tabs.iter().any(|t| t.view.read(cx).focus_handle(cx) == focused))
            }
        };
        if another_workspace || free {
            view.read(cx).focus_handle(cx).focus(window, cx);
        }
    }

    /// Starts or stops the wait before "Starting Claude…" is said.
    fn wait_for_claude(&mut self, starting: bool, cx: &mut Context<Self>) {
        if !starting {
            self.slow = false;
            self.waiting = None;
        } else if self.waiting.is_none() && !self.slow {
            self.waiting = Some(cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
                cx.background_executor().timer(SLOW_START).await;
                let _ = this.update(cx, |this, cx| {
                    this.slow = true;
                    this.waiting = None;
                    cx.notify();
                });
            }));
        }
    }

    fn tab(&self, ix: usize, tab: &TabDrawn, workspace: &str, cx: &mut Context<Self>) -> AnyElement {
        let label: SharedString = tab.label.clone().into();
        let said = if tab.exited { format!("{} exited", tab.label) } else { tab.label.clone() };
        let icon = if tab.claude {
            svg()
                .path("icons/sparkle-filled.svg")
                .flex_none()
                .size(px(13.))
                .text_color(alpha(theme::CLAUDE, if tab.exited { 0.5 } else { 1.0 }))
        } else {
            svg().path("icons/square-terminal.svg").flex_none().size(px(13.)).text_color(shell_green())
        };
        let (show_ws, show_id) = (workspace.to_string(), tab.id.clone());
        let (close_ws, close_id) = (workspace.to_string(), tab.id.clone());
        div()
            .id(("term-tab", ix))
            .test_support()
            .role(gpui_kit::Role::Tab)
            .aria_label(SharedString::from(said))
            .aria_selected(tab.active)
            .h(px(26.))
            .flex()
            .flex_none()
            .items_center()
            .rounded(px(8.))
            .text_size(theme::TEXT_SMALL)
            .border_1()
            .map(|d| {
                if tab.active {
                    // the terminal on screen: the chosen fill (DESIGN.md, "States"; the blue is the island's own
                    // edge while the keyboard is in it)
                    d.bg(theme::SELECTED).text_color(theme::INK).border_color(theme::RULE_STRONG)
                } else {
                    d.text_color(theme::INK_2)
                        .border_color(alpha(theme::RULE_STRONG, 0.))
                        .hover(|s| s.bg(theme::HOVER).text_color(theme::INK))
                }
            })
            .child(
                div()
                    .id(("term-pick", ix))
                    .test_support()
                    .aria_label(label.clone())
                    .h_full()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .pl(px(7.5))
                    .pr(px(4.))
                    .font_weight(if tab.active { theme::WEIGHT_TITLE } else { FontWeight::MEDIUM })
                    .cursor_pointer()
                    .child(icon)
                    // an exited terminal's name is dimmer, but never so dim on the tab on screen that the tab
                    // reads as the one not chosen: the word beside it says "exited"
                    .child(
                        div()
                            .when(tab.exited, |d| d.text_color(if tab.active { theme::INK_2 } else { theme::INK_3 }))
                            .child(label.clone()),
                    )
                    .when(tab.exited, |d| {
                        d.child(div().font_weight(FontWeight::NORMAL).text_color(theme::INK_3).child("exited"))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.terminals.update(cx, |terminals, cx| terminals.show(&show_ws, &show_id, cx));
                    })),
            )
            .child(
                div()
                    .id(("term-close", ix))
                    .test_support()
                    .role(gpui_kit::Role::Button)
                    .aria_label(SharedString::from(format!("Close {}", tab.label)))
                    .mr(px(3.))
                    .size(px(16.))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.))
                    .cursor_pointer()
                    .text_color(theme::INK_3)
                    .hover(|s| s.bg(theme::HOVER).text_color(theme::INK))
                    .child(svg().path("icons/x.svg").size(px(10.)).text_color(theme::INK_3))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.terminals.update(cx, |terminals, cx| terminals.close(&close_ws, &close_id, cx));
                    })),
            )
            .into_any_element()
    }

    /// New terminal: a shell, or Claude when none runs here.
    fn add_menu(&self, workspace: &str, has_live_claude: bool) -> impl IntoElement {
        let (terminals, workspace) = (self.terminals.clone(), workspace.to_string());
        MenuTrigger::new("term-add", "New terminal")
            .tip("New terminal")
            .size(px(26.))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(px(8.))
            .cursor_pointer()
            .hover(|s| s.bg(theme::HOVER))
            .child(svg().path("icons/plus.svg").size(px(15.)).text_color(theme::INK_3))
            .menu(gpui_kit::Anchor::TopLeft, move |menu, _, _| {
                let start = |kind: &'static str| {
                    let (terminals, workspace) = (terminals.clone(), workspace.clone());
                    move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut gpui_kit::App| {
                        terminals.update(cx, |terminals, cx| terminals.spawn(&workspace, kind, None, cx));
                    }
                };
                let menu = menu.item(
                    menu_item_tinted("new-shell", "icons/square-terminal.svg", shell_green(), "New shell")
                        .on_click(start("shell")),
                );
                if has_live_claude {
                    return menu;
                }
                menu.item(
                    menu_item_tinted("start-claude", "icons/sparkle-filled.svg", theme::CLAUDE, "Start Claude")
                        .on_click(start("claude")),
                )
            })
    }

    /// One terminal in its place in the island.
    fn pane(view: &Entity<TerminalView>) -> gpui_kit::Div {
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .pl(px(12.))
            .pr(px(6.))
            .py(px(8.))
            .child(view.clone().cached(StyleRefinement::default().size_full()))
    }

    /// The bar over a terminal whose program ended: what happened, and the button that starts it again.
    fn exited_bar(&self, tab: &TabDrawn, workspace: &str, cx: &mut Context<Self>) -> impl IntoElement {
        let who = if tab.claude { "Claude" } else { "The shell" };
        let words = match tab.exit_code {
            Some(code) => format!("{who} exited with code {code}."),
            None => format!("{who} exited."),
        };
        let (ws, id) = (workspace.to_string(), tab.id.clone());
        let again = if tab.claude { "Resume Claude" } else { "Start again" };
        div().absolute().left_0().right_0().bottom(px(20.)).flex().justify_center().child(
            raised("exited-bar")
                .flex()
                .items_center()
                .gap(px(12.))
                .py(px(6.))
                .pl(px(14.))
                .pr(px(6.))
                .child(
                    div()
                        .id("terminal-exited")
                        .test_support()
                        .aria_label(SharedString::from(words.clone()))
                        .text_size(theme::TEXT_SMALL)
                        .text_color(theme::INK_2)
                        .child(words),
                )
                .child(
                    button_with(
                        "restart-terminal",
                        Some("icons/rotate-ccw.svg"),
                        again,
                        None,
                        ButtonKind::Primary,
                        ButtonSize::Small,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.terminals.update(cx, |terminals, cx| terminals.restart(&ws, &id, cx));
                    })),
                ),
        )
    }

    /// What stands in the island while no terminal is open: why Claude didn't start, or the two ways to open one.
    fn nothing_open(&self, error: Option<String>, workspace: &str) -> impl IntoElement {
        let start = |kind: &'static str| {
            let (terminals, ws) = (self.terminals.clone(), workspace.to_string());
            move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut gpui_kit::App| {
                terminals.update(cx, |terminals, cx| terminals.spawn(&ws, kind, None, cx));
            }
        };
        let column = div().size_full().flex().flex_col().items_center().justify_center().gap(px(12.)).px(px(24.));
        let not_installed = error.as_deref() == Some(enkidu_core::commands::CLAUDE_NOT_INSTALLED);
        match error {
            Some(error) => column
                .child(
                    div()
                        .id("claude-error")
                        .test_support()
                        .aria_label(SharedString::from(error.clone()))
                        .max_w(px(440.))
                        .text_size(theme::TEXT)
                        .line_height(px(20.))
                        .text_color(theme::INK)
                        .child(error),
                )
                // why it didn't start is mostly the command or its environment: Settings has both. When Claude Code
                // isn't on this Mac at all, Setup has the way to install it (item 21.7)
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(
                            button(
                                "open-claude-settings",
                                if not_installed { "Open Setup" } else { "Open Claude settings" },
                                ButtonKind::Secondary,
                                ButtonSize::Medium,
                            )
                            .on_click({
                                let frame = self.frame.clone();
                                move |_, window, cx| {
                                    let page = match not_installed {
                                        true => enkidu_model::settings::Page::Setup,
                                        false => enkidu_model::settings::Page::Claude,
                                    };
                                    crate::settings::open_here(frame.clone(), page, window, cx)
                                }
                            }),
                        )
                        .child(
                            button("try-claude-again", "Try again", ButtonKind::Primary, ButtonSize::Medium)
                                .on_click(start("claude")),
                        ),
                ),
            None => column
                .child(
                    div()
                        .id("no-terminal")
                        .test_support()
                        .aria_label("No terminal open")
                        .text_size(theme::TEXT)
                        .text_color(theme::INK_3)
                        .child("No terminal open"),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(
                            button_tinted(
                                "empty-new-shell",
                                "icons/square-terminal.svg",
                                shell_green(),
                                "New shell",
                                ButtonKind::Secondary,
                                ButtonSize::Medium,
                            )
                            .on_click(start("shell")),
                        )
                        .child(
                            button_tinted(
                                "empty-start-claude",
                                "icons/sparkle-filled.svg",
                                theme::CLAUDE,
                                "Start Claude",
                                ButtonKind::Primary,
                                ButtonSize::Medium,
                            )
                            .on_click(start("claude")),
                        ),
                ),
        }
    }
}

impl Render for TerminalArea {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // the island the keyboard is in has the signal-blue edge (DESIGN.md, Principle 2): this one while a terminal of
        // its has the keyboard, or the window itself holds it because no terminal is there
        // text was typed into Claude here: its terminal takes the keyboard, so Enter is right there. Done as the
        // terminal is drawn: while the changes have the whole workspace it waits until the terminal is back
        if let Some(ws) = self.frame.read(cx).current_workspace().map(|w| w.id.clone())
            && let Some(view) = self.terminals.read(cx).active_view(&ws)
            && self.terminals.update(cx, |terminals, _| terminals.take_keyboard_wish(&ws))
        {
            let terminal = view.read(cx).focus_handle(cx);
            terminal.focus(window, cx);
        }
        // (no part has the keyboard's edge while the window is behind another app: DESIGN.md, "States")
        let focused = self.has_keyboard(window, cx) && window.is_window_active();
        let frame = self.frame.read(cx);
        let Some(workspace) = frame.current_workspace().map(|w| w.id.clone()) else {
            return island("island-claude", focused).size_full().child(lit_edge());
        };
        let working = frame.status(&workspace) == "working";
        let (tabs, known, error, has_live_claude) = match self.terminals.read(cx).of(&workspace) {
            Some(area) => (
                area.tabs
                    .iter()
                    .map(|tab| TabDrawn {
                        id: tab.info.id.clone(),
                        claude: tab.info.kind == "claude",
                        label: area.label(tab),
                        exited: tab.info.exited,
                        exit_code: tab.info.exit_code,
                        active: area.active.as_ref() == Some(&tab.info.id),
                        view: tab.view.clone(),
                    })
                    .collect::<Vec<_>>(),
                area.known,
                area.error.clone(),
                area.has_live_claude(),
            ),
            None => (Vec::new(), false, None, false),
        };

        let band = div()
            .id("terminal-tabs")
            .test_support()
            .role(gpui_kit::Role::TabList)
            .aria_label("Terminals")
            .relative()
            .h(theme::BAND)
            .flex_none()
            .flex()
            .items_center()
            .gap(px(4.))
            .px(px(5.))
            .bg(theme::CHROME)
            .border_b_1()
            .border_color(theme::RULE)
            // while Claude works, the line under the tabs runs
            .when(working, |band| band.child(running_line("claude-activity")))
            .children(tabs.iter().enumerate().map(|(ix, tab)| self.tab(ix, tab, &workspace, cx)).collect::<Vec<_>>())
            .child(self.add_menu(&workspace, has_live_claude));

        let mut body = div().relative().flex_1().min_h_0().flex().flex_row();
        let active = tabs.iter().find(|t| t.active);
        if self.split && tabs.len() > 1 {
            body = body
                .child(Self::pane(&tabs[0].view))
                .child(div().w(px(1.)).h_full().bg(theme::RULE))
                .child(Self::pane(&tabs[1].view));
        } else if let Some(tab) = active {
            body = body.child(Self::pane(&tab.view));
            if tab.exited {
                body = body.child(self.exited_bar(tab, &workspace, cx));
            }
        } else if tabs.is_empty() && (known || error.is_some()) {
            body = body.child(self.nothing_open(error, &workspace));
        } else if self.slow {
            body = body.child(
                div().size_full().flex().items_center().justify_center().child(
                    div()
                        .id("starting-claude")
                        .test_support()
                        .aria_label("Starting Claude…")
                        .text_size(theme::TEXT)
                        .text_color(theme::INK_3)
                        .child("Starting Claude…"),
                ),
            );
        }
        island("island-claude", focused).size_full().child(band).child(body).child(lit_edge())
    }
}
