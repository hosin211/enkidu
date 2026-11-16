//! The Work board, drawn (DESIGN.md, "The Work page"): a column per Jira status, each an island with its ring, its name
//! as Jira writes it and its count; a card per ticket, which says only what you read at a glance. What they say is
//! worked out in `enkidu_model::work::page` (`ColumnView`, `CardFace`); here it is drawn.

use enkidu_model::work::board::{Glyph, StatusLook, Tone};
use enkidu_model::work::page::{CardFace, CardHead, ColumnView, PrState};
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, App, FontWeight, IntoElement, Role, SharedString, Window, div, px,
};

use crate::work::page::Press;
use enkidu_ui::lamp::{Status, lamp};
use enkidu_ui::pieces::{changes_seen, island, lit_edge, mono, tick};
use enkidu_ui::tips::Tipped as _;
use enkidu_ui::tokens::{self as theme, bezier};
use enkidu_ui::work::{
    Ink, PrGlyph, StatusRing, card_surface, needs_badge, pr_glyph, priority_icon, status_ring, tinted, type_icon,
};

/// A column is never narrower than this, nor wider than [`COLUMN_MAX`].
pub const COLUMN_MIN: f32 = 244.;
pub const COLUMN_MAX: f32 = 380.;
/// The space between two columns.
pub const GAP: f32 = 8.;
/// How far above and below a column's view its cards are still drawn, so none pops in while it scrolls.
pub const OVERDRAW: f32 = 240.;
/// How wide an edge of the board fades where the board goes on past it.
pub const FADE: f32 = 36.;

/// Whether the board goes on past its left and its right edge: `columns` of at least [`COLUMN_MIN`] in `room` px,
/// scrolled by `offset` (0, or less once it is scrolled to the right).
pub fn more(columns: usize, room: f32, offset: f32) -> (bool, bool) {
    let needed = columns as f32 * COLUMN_MIN + columns.saturating_sub(1) as f32 * GAP;
    let hidden = needed - room;
    (offset < -2., hidden + offset > 2.)
}

/// A status's look as its ring is drawn.
pub fn ring(look: &StatusLook) -> StatusRing {
    match look.glyph {
        Glyph::Todo => StatusRing::Todo,
        Glyph::Progress => StatusRing::Progress(look.fill),
        Glyph::Hold => StatusRing::Hold,
        Glyph::Done => StatusRing::Done,
    }
}

pub fn pr_state(state: PrState) -> PrGlyph {
    match state {
        PrState::Open => PrGlyph::Open,
        PrState::Draft => PrGlyph::Draft,
        PrState::Merged => PrGlyph::Merged,
        PrState::Closed => PrGlyph::Closed,
    }
}

pub fn ink(tone: Tone) -> Ink {
    match tone {
        Tone::Red => Ink::Red,
        Tone::Amber => Ink::Amber,
        Tone::Green => Ink::Green,
        Tone::Violet => Ink::Violet,
        Tone::Muted => Ink::Muted,
    }
}

/// A card as a row of its column's list: 8 px from the column's edges, and 8 px to the next card.
pub fn in_list(card: AnyElement) -> AnyElement {
    div().px(px(GAP)).pb(px(GAP)).child(card).into_any_element()
}

/// A card's id on screen: `work-card:` and the card's own (`ticket:SH-1`, `pr:owner/repo#2`).
pub fn card_id(id: &str) -> SharedString {
    SharedString::from(format!("work-card:{id}"))
}

/// Everything a card says, as one line a test (and a screen reader) can read: what it is, its title, each PR, the
/// Claude on it, how its report ended.
pub fn card_words(face: &CardFace) -> String {
    let mut said: Vec<String> = Vec::new();
    said.push(match &face.head {
        CardHead::Ticket { type_name, key, priority, .. } => {
            let kind = type_name.as_deref().unwrap_or("Ticket");
            match priority {
                Some((_, name)) => format!("{kind} {key}, {name} priority"),
                None => format!("{kind} {key}"),
            }
        }
        CardHead::Pr { state, name } => format!("{} pull request {name}", state.label()),
        CardHead::Own { number, project } => format!("Your task {number} for {project}"),
    });
    said.push(face.title.clone());
    match &face.head {
        CardHead::Ticket { .. } => {
            said.extend(face.prs.iter().map(|pr| format!("#{} {}", pr.number, pr.phrase.text)));
            if face.more > 0 {
                said.push(format!("and {} more", face.more));
            }
        }
        CardHead::Pr { .. } => said.extend(face.prs.first().map(|pr| pr.phrase.text.to_string())),
        CardHead::Own { .. } => {}
    }
    if let Some((workspace, status)) = &face.claude {
        said.push(format!("Claude in {workspace}, {}", Status::from_core(status).label().to_lowercase()));
    }
    if let Some(report) = &face.report {
        said.push(format!("Report: {}", report.label));
    }
    if let Some(flow) = &face.flow {
        said.push(format!("Workflow: {}", flow.text));
    }
    said.join(" | ")
}

/// How a card stands on the board: the keyboard is on it, it is picked, and whether any card is (then every card
/// shows its round box, and a click picks).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CardState {
    pub selected: bool,
    pub picked: bool,
    pub picking: bool,
    /// It just moved to another column: it glows once. The number names that move.
    pub moved: Option<u64>,
}

/// What a click on a card asks for: ⇧ picks every card from the last one picked; ⌘, or any click while you pick,
/// picks this one; a plain click opens it.
pub fn press_of(shift: bool, command: bool, picking: bool) -> Press {
    if shift {
        Press::PickRun
    } else if picking || command {
        Press::Pick
    } else {
        Press::Open
    }
}

/// A card: its first line (type, key and priority, or a PR's state and name; the badge of what waits on you), its
/// title on up to three lines, a line per PR, and the Claude on it with its lamp. A round box on its corner picks
/// it: there when the card is pointed at, and on every card while you pick.
pub fn card(
    face: &CardFace,
    state: CardState,
    on_press: impl Fn(Press, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let id = &face.id;
    let named = |what: &str| SharedString::from(format!("{what}:{id}"));
    let CardState { selected, picked, picking, moved } = state;
    let on_press = std::rc::Rc::new(on_press);
    // the round box that picks the card; with ⇧, every card from the last one picked
    let group = named("card");
    let pick_box = {
        let on_press = on_press.clone();
        let words = if picked { "Unpick this card" } else { "Pick this card" };
        div()
            .id(named("card-pick"))
            .test_support()
            .role(Role::CheckBox)
            .aria_label(words)
            .aria_toggled(if picked { gpui_kit::Toggled::True } else { gpui_kit::Toggled::False })
            .absolute()
            .left(px(8.))
            .top(px(8.))
            .size(px(20.))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .border_1()
            .cursor_pointer()
            .map(|pick| match picked {
                true => pick.border_color(theme::ACCENT).bg(theme::ACCENT_STRONG).child(tinted(
                    "icons/check.svg",
                    theme::white(1.0),
                    12.,
                )),
                false => pick
                    .border_color(theme::RULE_STRONG)
                    .bg(theme::CHROME)
                    .hover(|s| s.border_color(enkidu_ui::tokens::alpha(theme::ACCENT, 0.7))),
            })
            // out of sight until the card is pointed at, unless you are picking
            .when(!picked && !picking, |pick| pick.opacity(0.).group_hover(group.clone(), |s| s.opacity(1.)))
            .on_click(move |event, window, cx| {
                cx.stop_propagation();
                let press = if event.modifiers().shift { Press::PickRun } else { Press::Pick };
                on_press(press, window, cx)
            })
    };
    let head = match &face.head {
        CardHead::Ticket { kind, key, priority, .. } => div()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(type_icon(kind.name(), 13.))
            .child(mono(div()).truncate().text_size(px(11.5)).text_color(theme::INK_3).child(key.clone()))
            .children(priority.as_ref().map(|(level, _)| priority_icon(*level, 12.))),
        CardHead::Pr { state, name } => div()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(pr_glyph(pr_state(*state), 13.))
            .child(mono(div()).truncate().text_size(px(11.5)).text_color(theme::INK_3).child(name.clone())),
        // a task of your own (item 8.5): signal blue is what is yours (DESIGN.md, "Color"); then the project it is for
        CardHead::Own { project, .. } => div()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(tinted("icons/user-round.svg", theme::ACCENT, 13.))
            .child(
                div()
                    .flex_none()
                    .text_size(px(11.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::ACCENT)
                    .child("Your task"),
            )
            .child(div().truncate().text_size(px(11.5)).text_color(theme::INK_3).child(project.clone())),
    };
    // in a workflow that is due or runs: when it starts, or that it runs now
    let flow = face.flow.as_ref().map(|flow| {
        let color = if flow.running { theme::ACCENT } else { theme::INK_3 };
        div()
            .id(named("card-flow"))
            .test_support()
            .aria_label(SharedString::from(flow.text.clone()))
            .aria_description(SharedString::from(flow.words.clone()))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(4.))
            .text_size(px(11.))
            .line_height(px(16.))
            .text_color(color)
            .tipped(flow.words.clone(), None)
            .child(tinted("icons/calendar-clock.svg", color, 11.))
            .child(flow.text.clone())
    });
    let first = div()
        .h(px(20.))
        .flex()
        .items_center()
        .gap(px(6.))
        // room for the round box while it is there
        .when(picked || picking, |line| line.pl(px(26.)))
        .child(head)
        .child(div().flex_1().min_w(px(8.)))
        .children(flow)
        .when(face.waiting > 0, |line| {
            let words = face.needs.clone();
            line.child(needs_badge(named("card-needs"), face.waiting, face.blocked, words.clone()).tipped(words, None))
        });
    let title = div()
        .mt(px(4.))
        .text_size(theme::TEXT)
        .line_height(px(18.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme::INK)
        .line_clamp(3)
        .child(face.title.clone());
    let prs = match &face.head {
        // a ticket's PRs: its state's glyph, its number, its state in a word or two
        CardHead::Ticket { .. } if !face.prs.is_empty() => Some(
            div()
                .mt(px(8.))
                .flex()
                .flex_col()
                .gap(px(4.))
                .children(face.prs.iter().map(|pr| {
                    let words = format!("#{} {}", pr.number, pr.phrase.text);
                    div()
                        .id(SharedString::from(format!("card-pr:{id}:{}", pr.number)))
                        .test_support()
                        .aria_label(SharedString::from(words))
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .text_size(px(12.))
                        .line_height(px(16.))
                        .child(pr_glyph(pr_state(pr.state), 12.))
                        .child(
                            mono(div())
                                .flex_none()
                                .text_size(px(11.5))
                                .text_color(theme::INK_2)
                                .child(format!("#{}", pr.number)),
                        )
                        .child(div().truncate().text_color(ink(pr.phrase.tone).color()).child(pr.phrase.text))
                }))
                .when(face.more > 0, |lines| {
                    lines.child(
                        div()
                            .pl(px(18.))
                            .text_size(px(11.5))
                            .text_color(theme::INK_3)
                            .child(format!("and {} more", face.more)),
                    )
                }),
        ),
        // a PR without a ticket: its state alone (its number is in the first line)
        CardHead::Pr { .. } => face.prs.first().map(|pr| {
            div()
                .mt(px(8.))
                .text_size(px(12.))
                .line_height(px(16.))
                .text_color(ink(pr.phrase.tone).color())
                .child(pr.phrase.text)
        }),
        _ => None,
    };
    let foot = (face.claude.is_some() || face.report.is_some()).then(|| {
        div()
            .mt(px(8.))
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(8.))
            .children(face.claude.as_ref().map(|(workspace, status)| {
                let status = Status::from_core(status);
                div()
                    .id(named("card-claude"))
                    .test_support()
                    .aria_label(SharedString::from(workspace.clone()))
                    .aria_description(status.label())
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(12.))
                    .line_height(px(16.))
                    .text_color(theme::INK_2)
                    .child(tinted("icons/sparkle-filled.svg", theme::CLAUDE, 11.))
                    .child(div().truncate().child(workspace.clone()))
                    .child(lamp(named("card-lamp"), status, 11., 0))
            }))
            // how Claude's latest report says the task ended
            .children(face.report.as_ref().map(|report| {
                let (icon, color) = super::report::report_look(&report.state);
                div()
                    .id(named("card-report"))
                    .test_support()
                    .aria_label(report.label)
                    .aria_description(SharedString::from(report.words.clone()))
                    .ml_auto()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .text_size(px(11.5))
                    .line_height(px(16.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(color)
                    .tipped(report.words.clone(), None)
                    .child(tinted(icon, color, 12.))
                    .child(report.label)
                    .when(report.images > 0, |mark| {
                        mark.child(
                            div()
                                .ml(px(2.))
                                .flex()
                                .items_center()
                                .gap(px(2.))
                                .font_weight(FontWeight::NORMAL)
                                .text_color(theme::INK_3)
                                .child(tinted("icons/image.svg", theme::INK_3, 11.))
                                .child(report.images.to_string()),
                        )
                    })
            }))
    });
    // it answers a press
    let surface = enkidu_ui::press::pressable(card_surface(card_id(id), selected, face.stale))
        .test_support()
        .role(Role::Button)
        .aria_label(SharedString::from(face.name().to_string()))
        .aria_description(SharedString::from(card_words(face)))
        .aria_selected(selected)
        .px(px(12.))
        .pt(px(8.))
        .pb(px(10.))
        .when(face.stale, |card| card.tipped("From the last good read: this source can't be read right now", None))
        .group(group)
        .child(pick_box)
        .child(first)
        .child(title)
        .children(prs)
        .children(foot)
        .on_click(move |event, window, cx| {
            let keys = event.modifiers();
            on_press(press_of(keys.shift, keys.platform, picking), window, cx)
        });
    match moved {
        // it just moved to another column: it glows once, so you see it move
        Some(turn) => surface
            .with_animation(
                SharedString::from(format!("card-moved:{id}:{turn}")),
                Animation::new(enkidu_ui::work::CARD_MOVED).with_easing(|t| bezier(theme::EASE_OUT, t)),
                enkidu_ui::work::card_moved,
            )
            .into_any_element(),
        None => surface.into_any_element(),
    }
}

/// A column: an island with the status's ring, its name as Jira writes it and its count, then its cards, which
/// scroll on their own (`cards`: the column's list, see [`in_list`]).
pub fn column(ix: usize, column: &ColumnView, cards: AnyElement, window: &mut Window, cx: &mut App) -> AnyElement {
    let count = column.cards.len();
    // the count ticks when it changes while you watch
    let changes = changes_seen(SharedString::from(format!("work-count-{}", column.id)), count as u64, window, cx);
    let words = if count == 1 { "1 card".to_string() } else { format!("{count} cards") };
    island(("work-column", ix), false)
        .aria_label(SharedString::from(column.name.clone()))
        .aria_description(SharedString::from(words))
        .flex_1()
        .min_w(px(COLUMN_MIN))
        .max_w(px(COLUMN_MAX))
        .h_full()
        .child(
            div()
                .h(px(40.))
                .flex_none()
                .flex()
                .items_center()
                .gap(px(8.))
                .pl(px(14.))
                .pr(px(12.))
                .child(status_ring(ring(&column.look), column.look.hue, 14.))
                .child(
                    div()
                        .truncate()
                        .text_size(theme::TEXT)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::INK)
                        .child(column.name.clone()),
                )
                .child(div().flex_none().text_size(px(12.)).text_color(theme::INK_3).child(tick(
                    SharedString::from(format!("work-count-tick-{}", column.id)),
                    count.to_string(),
                    changes,
                ))),
        )
        // its cards: a list that draws only the ones in view (`cards`), found by tests through this
        .child(div().id(("work-cards", ix)).test_support().flex_1().min_h_0().child(cards))
        .child(lit_edge())
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_edge_fades_where_the_board_goes_on_past_it() {
        // nine columns need 2,260 px; in 1,344 the board goes on to the right
        assert_eq!(more(9, 1344., 0.), (false, true));
        // scrolled a little: both ways; scrolled to its end: only to the left
        assert_eq!(more(9, 1344., -300.), (true, true));
        assert_eq!(more(9, 1344., -916.), (true, false));
        // everything fits: no edge fades
        assert_eq!(more(5, 1344., 0.), (false, false));
        assert_eq!(more(9, 2384., 0.), (false, false));
        assert_eq!(more(0, 1344., 0.), (false, false));
    }
}
