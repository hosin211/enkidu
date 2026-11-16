//! The gallery: every token and piece on one screen (item 1.12). It is drawn offscreen to `shots/out/gallery.png` and
//! looked at; later items add their own pieces here.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context, Div, FontWeight, IntoElement, Render, Rgba, SharedString, Window, div, px};

use crate::lamp::{Status, lamp};
use crate::live::{LiveLook, live_badge, live_button};
use crate::pieces::*;
use crate::tab_pill::{Pill, tab_row_of};
use crate::tokens::*;

/// The gallery's view. `changes` counts made-up changes, so the tick and the lamps' pop can be shown moving.
#[derive(Default)]
pub struct Gallery {
    pub changes: u32,
}

fn block(title: &'static str, note: &'static str, body: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(10.))
        .child(
            div()
                .flex()
                .items_baseline()
                .gap(px(10.))
                .child(div().text_size(PANEL_TITLE).font_weight(WEIGHT_TITLE).text_color(INK).child(title))
                .child(div().text_size(TEXT_SMALL).text_color(INK_3).child(note)),
        )
        .child(body)
}

fn row() -> Div {
    div().flex().flex_wrap().items_center().gap(px(14.))
}

fn labelled(label: impl Into<SharedString>, piece: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(6.))
        .child(piece)
        .child(div().text_size(CAPTION).text_color(INK_3).child(label.into()))
}

fn swatch(name: &'static str, color: Rgba, on: Rgba) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(5.))
        .w(px(92.))
        .child(
            div()
                .h(px(40.))
                .rounded(RADIUS_CONTROL)
                .bg(on)
                .child(div().size_full().rounded(RADIUS_CONTROL).bg(color).border_1().border_color(white(0.06))),
        )
        .child(div().text_size(CAPTION).text_color(INK_2).child(name))
}

impl Gallery {
    fn lamps(&self) -> Div {
        let mut lamps = row();
        for (ix, status) in [Status::Idle, Status::Working, Status::NeedsYou, Status::Done].into_iter().enumerate() {
            lamps = lamps.child(labelled(status.label(), lamp(("gallery-lamp", ix), status, 14., self.changes)));
        }
        lamps.child(labelled("12 px, in the status line", lamp("gallery-lamp-small", Status::Working, 12., 0)))
    }

    fn colors(&self) -> Div {
        let layers = [("frame", FRAME), ("canvas", CANVAS), ("chrome", CHROME), ("raised", RAISED), ("tip", TIP)];
        let text = [("ink", INK), ("ink-2", INK_2), ("ink-3", INK_3)];
        let lines = [
            ("rule", RULE),
            ("rule-strong", RULE_STRONG),
            ("hover", HOVER),
            ("soft", SOFT),
            ("selected", SELECTED),
            ("selected-hover", SELECTED_HOVER),
            ("thumb", THUMB),
        ];
        let signals = [
            ("accent", ACCENT),
            ("accent-strong", ACCENT_STRONG),
            ("accent-soft", ACCENT_SOFT),
            ("behind", BEHIND),
            ("claude", CLAUDE),
            ("lamp-amber", LAMP_AMBER),
            ("lamp-red", LAMP_RED),
            ("lamp-green", LAMP_GREEN),
            ("lamp-off", LAMP_OFF),
        ];
        let diff = [
            ("add-bg", ADD_BG),
            ("add-word", ADD_WORD),
            ("add-ink", ADD_INK),
            ("del-bg", DEL_BG),
            ("del-word", DEL_WORD),
            ("del-ink", DEL_INK),
            ("gap-bg", GAP_BG),
            ("remark-band", REMARK_BAND),
        ];
        let set = |items: &[(&'static str, Rgba)], on: Rgba| {
            let mut r = row();
            for (name, color) in items {
                r = r.child(swatch(name, *color, on));
            }
            r
        };
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(set(&layers, FRAME))
            .child(set(&text, CANVAS))
            .child(set(&lines, CANVAS))
            .child(set(&signals, CANVAS))
            .child(set(&diff, CANVAS))
    }

    fn icons(&self) -> Div {
        let mut files = row();
        for path in [
            "stock.py",
            "app.js",
            "View.vue",
            "index.html",
            "site.css",
            "theme.scss",
            "package.json",
            "README.md",
            "settings.yml",
            "deploy.sh",
            "query.sql",
            "logo.png",
            "Dockerfile",
            ".env",
            ".gitignore",
            "pnpm-lock.yaml",
            "notes",
        ] {
            files = files.child(labelled(path, file_icon(path, 15.)));
        }
        files.child(labelled("folder", folder_icon(false, 15.))).child(labelled("open", folder_icon(true, 15.)))
    }

    fn type_scale(&self) -> Div {
        let line = |label: &'static str, size, weight: FontWeight, sample: &'static str| {
            div()
                .flex()
                .items_baseline()
                .gap(px(16.))
                .child(div().w(px(150.)).text_size(CAPTION).text_color(INK_3).child(label))
                .child(div().text_size(size).font_weight(weight).text_color(INK).child(sample))
        };
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(line("Empty title 17/24", EMPTY_TITLE, WEIGHT_TITLE, "No project yet"))
            .child(line("Dialog title 15/20", DIALOG_TITLE, WEIGHT_TITLE, "New workspace"))
            .child(line("Panel title 13/20", PANEL_TITLE, WEIGHT_TITLE, "Changes"))
            .child(line("Body 13/18", TEXT, WEIGHT_BODY, "Claude is working on stock-fix"))
            .child(line("Label 13, 500", TEXT, WEIGHT_LABEL, "stock-fix"))
            .child(line("Secondary 12/16", TEXT_SMALL, WEIGHT_BODY, "from development"))
            .child(line("Status line 11.5/16", TEXT_STATUS, WEIGHT_BODY, "5 changed files"))
            .child(line("Caption 11/14", CAPTION, WEIGHT_BODY, "2 minutes ago"))
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap(px(16.))
                    .child(div().w(px(150.)).text_size(CAPTION).text_color(INK_3).child("Mono 12/20"))
                    .child(
                        mono(div())
                            .text_size(MONO_TEXT)
                            .text_color(INK)
                            .child("fix/stock-double-count -> => != === <= >="),
                    ),
            )
    }

    /// Live (DESIGN.md, "Live"): the play button in each of its states at its one width, then Live on a tab.
    fn live(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let mut r = row();
        for (ix, (label, look, words, state, pointed)) in [
            ("Off", LiveLook::Off, "Go live", "off", false),
            ("Off, pointed at", LiveLook::Off, "Go live", "off", true),
            ("Starting", LiveLook::Starting, "Starting…", "starting", false),
            ("Live", LiveLook::Live, "Live", "live", false),
            ("Live, pointed at", LiveLook::Live, "Stop", "live", true),
            ("Stopping", LiveLook::Stopping, "Stopping…", "stopping", false),
            ("Switching elsewhere", LiveLook::Waiting, "Go live", "waiting", false),
        ]
        .into_iter()
        .enumerate()
        {
            let piece = live_button(("gallery-live", ix), look, words, state, pointed, window, cx);
            r = r.child(labelled(label, piece));
        }
        r.child(labelled("On a tab", live_badge("gallery-live-badge", false)))
            .child(labelled("On a narrow tab", live_badge("gallery-live-badge-compact", true)))
    }

    fn buttons(&self) -> Div {
        let mut r = row();
        for (ix, (label, kind)) in [
            ("Create workspace", ButtonKind::Primary),
            ("Open in Claude", ButtonKind::Accent),
            ("Cancel", ButtonKind::Secondary),
            ("Discard", ButtonKind::Ghost),
            ("Remove workspace", ButtonKind::Danger),
        ]
        .into_iter()
        .enumerate()
        {
            r = r.child(button(("gallery-button", ix), label, kind, ButtonSize::Medium));
        }
        r.child(button("gallery-small", "Small", ButtonKind::Secondary, ButtonSize::Small))
            .child(button("gallery-large", "Large", ButtonKind::Primary, ButtonSize::Large))
            .child(icon_button("gallery-icon", "icons/settings.svg", "Settings"))
    }

    fn small_things(&self) -> Div {
        row()
            .child(chip("Draft", INK_2))
            .child(chip("Needs review", LAMP_AMBER))
            .child(chip("Merged", ACCENT))
            .child(chip("Failing", LAMP_RED))
            .child(kbd("⌘K"))
            .child(kbd("⇧⌘]"))
            .child(kbd("Esc"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(TEXT_STATUS)
                    .text_color(INK_2)
                    .child(tick("gallery-count", format!("{} changed files", 5 + self.changes), self.changes))
                    .child(div().text_color(LAMP_GREEN).child(tick(
                        "gallery-add",
                        format!("+{}", 13 + 2 * self.changes),
                        self.changes,
                    )))
                    .child(div().text_color(LAMP_RED).child(tick("gallery-del", "−6", 0))),
            )
            // tabular figures: both rows are as wide as each other, digit for digit; beside them, the face's own
            .child(
                div()
                    .id("gallery-tabular")
                    .flex()
                    .items_center()
                    .gap(px(16.))
                    .text_size(TEXT)
                    .text_color(INK_2)
                    .child(tabular(div().flex().flex_col().child("1,111,111").child("8,888,888")))
                    .child(div().flex().flex_col().text_color(INK_3).child("1,111,111").child("8,888,888")),
            )
    }

    fn layers(&self) -> Div {
        row()
            .items_start()
            .child(
                island("gallery-island", false)
                    .w(px(260.))
                    .h(px(120.))
                    .child(div().p(px(14.)).text_color(INK_2).child("An island"))
                    .child(lit_edge()),
            )
            .child(
                island("gallery-island-focused", true)
                    .w(px(260.))
                    .h(px(120.))
                    .child(div().p(px(14.)).text_color(INK_2).child("The island the keyboard is in"))
                    .child(lit_edge()),
            )
            .child(
                raised("gallery-raised")
                    .w(px(260.))
                    .h(px(120.))
                    .child(div().p(px(14.)).text_color(INK_2).child("A menu, a toast, a card"))
                    .child(lit_edge()),
            )
    }

    fn projects(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let mut badges = row();
        for (place, name) in
            ["shop", "enkidu", "crm", "ten-bridges", "fieldnotes", "flashcards", "dataloader"].into_iter().enumerate()
        {
            badges = badges.child(labelled(name, project_badge(name, project_hue(place), 16.)));
        }
        let selected = self.changes as usize % 3;
        let tabs: Vec<Observed> =
            [("stock-fix", Status::Working), ("sh-21309", Status::NeedsYou), ("pr-2175", Status::Done)]
                .into_iter()
                .enumerate()
                .map(|(ix, (name, status))| {
                    gpui_kit::div()
                        .id(("gallery-tab", ix))
                        .test_support()
                        .h(TAB_HEIGHT)
                        .px(px(10.))
                        .flex()
                        .items_center()
                        .gap(px(7.))
                        .rounded(TAB_RADIUS)
                        .child(lamp(("gallery-tab-lamp", ix), status, 14., 0))
                        .child(
                            div()
                                .text_size(TEXT)
                                .text_color(if ix == selected { INK } else { INK_2 })
                                .when(ix == selected, |name| name.font_weight(WEIGHT_TITLE))
                                .child(name),
                        )
                })
                .collect();
        div().flex().flex_col().gap(px(14.)).child(badges).child(
            // the workspace on screen: the chosen fill and the blue edge of where you are
            div().p(px(6.)).rounded(ISLAND_RADIUS).bg(FRAME).child(tab_row_of(
                Pill::OnScreen,
                "gallery-tabs",
                selected,
                tabs,
                window,
                cx,
            )),
        )
    }
}

impl Render for Gallery {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let blocks: Vec<AnyElement> = vec![
            block(
                "Signal lamps",
                "Idle is a hollow ring; working turns; needs you pings; done is green.",
                self.lamps(),
            )
            .into_any_element(),
            block(
                "Color",
                "Layers step up from the frame; the lamps are Claude's, signal blue is yours.",
                self.colors(),
            )
            .into_any_element(),
            block("Icons", "One lightness for every hue, so no file type shouts louder than another.", self.icons())
                .into_any_element(),
            block(
                "Type",
                "The system face for the interface, JetBrains Mono for code, ligatures off.",
                self.type_scale(),
            )
            .into_any_element(),
            block("Buttons", "Primary is white like a key; the one that types into Claude is blue.", self.buttons())
                .into_any_element(),
            block(
                "Live",
                "A play button of one width, so nothing beside it moves. Live is green with the bars playing.",
                self.live(window, cx),
            )
            .into_any_element(),
            block("Chips, keys, counts", "Counts tick into place when they change.", self.small_things())
                .into_any_element(),
            block("Layers", "Every layer has an edge and a lit top edge.", self.layers()).into_any_element(),
            block("Projects and workspaces", "A project's color comes from its place.", self.projects(window, cx))
                .into_any_element(),
        ];
        div()
            .id("gallery")
            .size_full()
            .overflow_y_scroll()
            .bg(CANVAS)
            .text_color(INK)
            .font_family(UI_FONT)
            .text_size(TEXT)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(28.))
                    .p(px(28.))
                    .child(
                        div()
                            .text_size(EMPTY_TITLE)
                            .font_weight(WEIGHT_TITLE)
                            .text_color(INK)
                            .child("Enkidu design system"),
                    )
                    .children(blocks),
            )
            .child(crate::tips::layer(window, cx))
    }
}
