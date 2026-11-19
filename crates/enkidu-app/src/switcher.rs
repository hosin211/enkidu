//! The switcher (DESIGN.md, "Layout": "The switcher"): ⌃⇥ held shows every workspace as a card, the most lately
//! used first, and each card says what is behind it: its project, its lamp and name, where Claude stands in words,
//! the last lines of its terminal, what is changed and its pull request. The blue edge is on the one ⌃⇥ would go
//! to; letting go of ⌃ goes there. It floats over the work, four cards in a row.

use std::collections::HashMap;

use enkidu_core::work::model::Snapshot;
use enkidu_model::frame::{Changes, Frame};
use enkidu_model::switcher::{files_words, order, pr_words, says, tail};
use enkidu_model::work::pr::{PrLamp, tab_chip};
use enkidu_ui::lamp::{Status, lamp};
use enkidu_ui::live::live_badge;
use enkidu_ui::pieces::{float_shadow, kbd, mono, project_hue, tabular};
use enkidu_ui::tokens as theme;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, BoxShadow, FontWeight, Role, SharedString, Window, div, linear_color_stop, linear_gradient, point,
    px, svg,
};

use crate::terminals::Terminals;

/// How many lines of its terminal a card shows.
const TAIL: usize = 6;
/// The sheet is this wide, or the window less 24 px on each side; it stands this far under the window's top.
const WIDTH: f32 = 1272.;
const TOP: f32 = 104.;
/// Four cards in a row, 12 px apart, each this high.
const IN_A_ROW: usize = 4;
const GAP: f32 = 12.;
const CARD_HEIGHT: f32 = 176.;

/// What a card says of a workspace, as it stood when the switcher opened.
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub id: String,
    pub name: String,
    pub project: String,
    /// The project's place on the rail: its badge's color.
    pub project_at: usize,
    pub status: String,
    pub live: bool,
    pub on_screen: bool,
    /// Where Claude stands, in words.
    pub says: String,
    /// The last lines of its Claude's terminal.
    pub tail: Vec<String>,
    pub changes: Changes,
    /// Its pull request in words, and its lamp.
    pub pr: Option<(String, PrLamp)>,
}

/// The cards, the most lately used first. `stood` is the state each workspace was last seen in and since when.
pub fn cards(
    frame: &Frame,
    recent: &[String],
    stood: &HashMap<String, (String, Option<i64>)>,
    terminals: &Terminals,
    work: Option<&Snapshot>,
    now: i64,
    cx: &App,
) -> Vec<Card> {
    let rows = frame.rows();
    let ids: Vec<String> = rows.iter().map(|ws| ws.id.clone()).collect();
    let on_screen = frame.current_workspace().map(|ws| ws.id.clone());
    order(recent, &ids)
        .into_iter()
        .filter_map(|id| {
            let ws = rows.iter().find(|ws| ws.id == id)?;
            let project_at = frame.projects.iter().position(|p| p.id == ws.project_id)?;
            let status = frame.status(&ws.id).to_string();
            let since = stood.get(&ws.id).filter(|(was, _)| *was == status).and_then(|(_, since)| *since);
            // Claude's terminal is the workspace's first; one that was not opened in this run has nothing to show
            let lines: Vec<String> = (terminals.of(&ws.id).and_then(|area| area.tabs.first()))
                .map(|tab| {
                    let screen = tab.view.read(cx).terminal().screen();
                    (0..screen.rows).map(|row| screen.row_text(row)).collect()
                })
                .unwrap_or_default();
            let pr = tab_chip(work, &ws.id, Some(&ws.session_id)).map(|pr| (pr_words(&pr), pr.lamp));
            Some(Card {
                id: ws.id.clone(),
                name: ws.name.clone(),
                project: frame.projects[project_at].name.clone(),
                project_at,
                says: says(&status, since, now),
                status,
                live: frame.is_live(ws),
                on_screen: on_screen.as_deref() == Some(ws.id.as_str()),
                tail: tail(&lines, TAIL),
                changes: frame.changed.get(&ws.id).copied().unwrap_or_default(),
                pr,
            })
        })
        .collect()
}

/// A ring of one color around a box, or just inside it.
fn ring(color: gpui_kit::Rgba, width: f32, inset: bool) -> BoxShadow {
    BoxShadow {
        color: gpui_kit::Hsla::from(color),
        offset: point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: px(width),
        inset,
    }
}

fn card(
    n: usize,
    card: &Card,
    edge: bool,
    width: f32,
    icon: Option<std::path::PathBuf>,
    pick: impl Fn(usize, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let status = Status::from_core(&card.status);
    let needs = status == Status::NeedsYou;
    let part = |what: &'static str| (what, n);
    let head = div()
        .h(theme::CHIP)
        .flex()
        .items_center()
        .gap(px(8.))
        .min_w_0()
        .whitespace_nowrap()
        // its project's picture, the one the rail shows
        .child(crate::rail::project_picture(&card.project, project_hue(card.project_at), 16., icon))
        .child(div().flex_none().child(lamp(part("switch-lamp"), status, 16., 0)))
        .child(
            div().min_w_0().truncate().font_weight(theme::WEIGHT_TITLE).text_color(theme::INK).child(card.name.clone()),
        )
        .when(card.live, |head| head.child(live_badge(part("switch-live"), false)))
        .child(div().flex_1())
        .when(card.on_screen, |head| {
            head.child(div().flex_none().text_size(theme::TEXT_STATUS).text_color(theme::INK_3).child("on screen"))
        });
    let says = div()
        .id(part("switch-says"))
        .test_support()
        .aria_label(SharedString::from(card.says.clone()))
        .truncate()
        .text_size(px(12.5))
        .line_height(theme::TEXT_LINE)
        .map(|says| match needs {
            true => says.font_weight(FontWeight::SEMIBOLD).text_color(theme::LAMP_RED),
            false => says.text_color(theme::INK),
        })
        .child(card.says.clone());
    // the end of its terminal: its last lines stand at the foot, and what is above them fades away
    let lines = mono(div())
        .id(part("switch-tail"))
        .test_support()
        .aria_label(SharedString::from(card.tail.join("\n")))
        .relative()
        .flex_1()
        .min_h_0()
        .mt(px(4.))
        .pt(px(8.))
        .border_t_1()
        .border_color(theme::LINE)
        .overflow_hidden()
        .flex()
        .flex_col()
        .justify_end()
        .text_size(theme::CAPTION)
        .line_height(px(16.))
        .text_color(theme::INK_2)
        .children(card.tail.iter().map(|line| {
            // (a blank line keeps its height)
            div().flex_none().h(px(16.)).truncate().child(line.clone())
        }))
        .child(div().absolute().left_0().right_0().top(px(1.)).h(px(12.)).bg(linear_gradient(
            180.,
            linear_color_stop(theme::WELL, 0.),
            linear_color_stop(theme::alpha(theme::WELL, 0.), 1.),
        )));
    let changed = files_words(card.changes.files);
    let counts = |sign: &str, lines: u64, color: gpui_kit::Rgba| {
        (lines > 0).then(|| div().text_color(color).child(format!("{sign}{lines}")))
    };
    let foot = div()
        .h(theme::CHIP)
        .flex()
        .items_center()
        .gap(px(8.))
        .whitespace_nowrap()
        .text_size(theme::TEXT_SMALL)
        .text_color(theme::INK_3)
        .children(changed.clone().map(|words| {
            tabular(div())
                .id(part("switch-changes"))
                .test_support()
                .aria_label(SharedString::from(words.clone()))
                .flex()
                .flex_none()
                .items_center()
                .gap(px(8.))
                .child(words)
                .children(counts("+", card.changes.additions, theme::ADD_INK))
                .children(counts("\u{2212}", card.changes.deletions, theme::DEL_INK))
        }))
        .child(div().flex_1())
        .children(card.pr.clone().map(|(words, pr)| {
            let color = match pr {
                PrLamp::Red => theme::LAMP_RED,
                PrLamp::Amber => theme::LAMP_AMBER,
                PrLamp::Green => theme::LAMP_GREEN,
                PrLamp::Violet => theme::oklch(0.72, 0.14, 300.),
                PrLamp::Gray => theme::INK_3,
            };
            let glyph = if pr == PrLamp::Violet { "icons/git-merge.svg" } else { "icons/git-pull-request.svg" };
            div()
                .id(part("switch-pr"))
                .test_support()
                .aria_label(SharedString::from(words.clone()))
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(4.))
                .text_color(color)
                .child(svg().path(glyph).flex_none().size(theme::MARK).text_color(color))
                .child(div().min_w_0().truncate().child(words))
        }));
    div()
        .id(("switch-card", n))
        .test_support()
        .role(Role::ListBoxOption)
        .aria_label(SharedString::from(card.name.clone()))
        .aria_description(SharedString::from(card.says.clone()))
        .aria_value(SharedString::from(card.id.clone()))
        .aria_selected(edge)
        .w(px(width))
        .h(px(CARD_HEIGHT))
        .flex_none()
        .flex()
        .flex_col()
        .gap(px(4.))
        .p(px(12.))
        .rounded(theme::RADIUS_CARD)
        .bg(theme::WELL)
        .text_size(theme::TEXT)
        .cursor_pointer()
        // the one ⌃⇥ would go to: the keyboard's blue edge; the others a line
        .shadow(match edge {
            true => vec![ring(theme::ACCENT, 2., true), ring(theme::alpha(theme::ACCENT, 0.16), 4., false)],
            false => vec![ring(theme::LINE, 1., true)],
        })
        .child(head)
        .child(says)
        .child(lines)
        .child(foot)
        .on_click(move |_, window, cx| pick(n, window, cx))
        .into_any_element()
}

/// The sheet with its cards, the blue edge on the one at `at`. `pick` is told a card that was clicked.
pub fn sheet(
    cards: &[Card],
    at: usize,
    // the projects that have an icon of their own, by their place in the projects' row
    icons: &HashMap<usize, std::path::PathBuf>,
    scroll: &gpui_kit::ScrollHandle,
    window: &Window,
    pick: impl Fn(usize, &mut Window, &mut App) + Clone + 'static,
) -> AnyElement {
    let room = window.viewport_size();
    let width = WIDTH.min(f32::from(room.width) - 48.);
    // a card's width: four in a row inside the sheet's 16 px, 12 px apart
    let inside = width - 32. - 2.;
    let card_width = ((inside - GAP * (IN_A_ROW as f32 - 1.)) / IN_A_ROW as f32).floor();
    // the rows that fit: the sheet never goes under the window's foot, and the row with the blue edge is in view
    let rows = cards.len().div_ceil(IN_A_ROW).max(1) as f32;
    let tallest = (f32::from(room.height) - TOP - 24. - 32. - 2. - 12. - 20.).max(CARD_HEIGHT);
    let height = (rows * CARD_HEIGHT + (rows - 1.) * GAP).min(tallest);
    scroll.scroll_to_item(at);
    let hint = |keys: &[&'static str], words: &'static str| {
        div().flex().flex_none().items_center().gap(px(8.)).children(keys.iter().map(|key| kbd(*key))).child(words)
    };
    let sheet = div()
        .id("switcher")
        .test_support()
        .role(Role::ListBox)
        .aria_label("Every workspace, the most lately used first")
        .w(px(width))
        .p(px(16.))
        .rounded(theme::RADIUS_DIALOG)
        .bg(theme::SHEET)
        .border_1()
        .border_color(theme::LINE_STRONG)
        .shadow(float_shadow())
        .occlude()
        .child(
            div()
                .id("switch-cards")
                .h(px(height))
                .overflow_y_scroll()
                .track_scroll(scroll)
                .flex()
                .flex_wrap()
                .content_start()
                .gap(px(GAP))
                .children(cards.iter().enumerate().map(|(n, one)| {
                    card(n, one, n == at, card_width, icons.get(&one.project_at).cloned(), pick.clone())
                })),
        )
        .child(
            div()
                .mt(px(12.))
                .h(theme::CHIP)
                .flex()
                .items_center()
                .gap(px(16.))
                .text_size(theme::TEXT_SMALL)
                .text_color(theme::INK_3)
                .child(hint(&["⌃", "⇥"], "the next one"))
                .child(hint(&["⇧", "⇥"], "back"))
                .child(div().text_color(theme::INK_2).child("Let go of ⌃ to go there."))
                .child(div().flex_1())
                .child(hint(&["Esc"], "stay here")),
        );
    // centred over the stage, under the top row
    div().absolute().left_0().right_0().top(px(TOP)).flex().justify_center().child(sheet).into_any_element()
}
