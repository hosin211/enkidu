//! Text with a shape, drawn in the page's own type: what `enkidu_model::work::rich` read from Jira's document or
//! GitHub's Markdown. Everything is drawn as text by us: nothing in it is run or fetched. A link opens in the browser;
//! an image or a file is named.

use std::cell::Cell;
use std::ops::Range;

use enkidu_model::work::rich::{self, Block, Chip, Panel, Span};
use enkidu_ui::pieces::{Observed, mono};
use enkidu_ui::selectable::{selectable, selectable_as};
use enkidu_ui::tokens::{self as theme, alpha, tone};
use enkidu_ui::work::tinted;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Div, Font, FontStyle, FontWeight, Hsla, InteractiveText, Rgba, SharedString, StrikethroughStyle,
    StyledText, TextRun, UnderlineStyle, Window, div, px,
};

use crate::work::card::open_in_browser;

/// What a line's plain words look like: a paragraph's are quiet, a heading's and a table header's bright and strong.
#[derive(Clone, Copy)]
struct Base {
    color: Rgba,
    weight: FontWeight,
}

const BODY: Base = Base { color: theme::INK_2, weight: FontWeight::NORMAL };
const BRIGHT: Base = Base { color: theme::INK, weight: FontWeight::NORMAL };
const HEAD: Base = Base { color: theme::INK, weight: FontWeight::SEMIBOLD };
const QUIET: Base = Base { color: theme::INK_3, weight: FontWeight::NORMAL };

/// Names for the parts that need one (a line of words, a fold), unique under the text's own id.
struct Names {
    under: SharedString,
    next: Cell<usize>,
    /// Counts the lines of words, for each one's own name.
    words: Cell<u64>,
}

impl Names {
    fn next(&self, what: &str) -> SharedString {
        let n = self.next.get();
        self.next.set(n + 1);
        SharedString::from(format!("{}:{what}:{n}", self.under))
    }
}

/// A thin space: the room inside a chip's fill, on each side of its words.
const PAD: &str = "\u{2009}";

/// ⌘C over selected words of such a text: what is copied is what the text says, without the thin spaces that give
/// code and chips their room (a command copied with one in front of it isn't found by a shell). False when
/// nothing is selected: the key is someone else's then. For the root of a page that shows such texts.
pub fn copy_selection(window: &mut Window, cx: &mut App) -> bool {
    let selected = gpui_kit::base::TextSelection::selected_text(window, cx);
    let words = selected.replace(PAD, "");
    let words = words.trim();
    if words.is_empty() {
        return false;
    }
    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(words.to_string()));
    true
}

/// One line of words as it is drawn: the text, its runs, and where its links are.
struct Line {
    text: String,
    runs: Vec<TextRun>,
    links: Vec<(Range<usize>, String)>,
}

fn line(spans: &[Span], base: Base) -> Line {
    let mut made = Line { text: String::new(), runs: Vec::new(), links: Vec::new() };
    for span in spans.iter().filter(|span| !span.text.is_empty()) {
        // code and chips sit on a fill of their own, with a little room on each side
        let filled = span.code || span.chip.is_some();
        let words = if filled { format!("{PAD}{}{PAD}", span.text) } else { span.text.clone() };
        let from = made.text.len();
        made.text.push_str(&words);
        let family = if span.code { theme::MONO_FONT } else { theme::UI_FONT };
        let weight = match (span.strong, span.chip) {
            (true, _) => FontWeight::SEMIBOLD,
            (_, Some(Chip::Status)) => FontWeight::MEDIUM,
            _ => base.weight,
        };
        let color = match (&span.link, span.chip) {
            (Some(_), _) | (_, Some(Chip::Mention)) => theme::ACCENT,
            (_, Some(Chip::Status)) => theme::INK_2,
            (_, Some(Chip::File)) => theme::INK_3,
            _ if span.code || span.strong => theme::INK,
            _ => base.color,
        };
        let fill = match span.chip {
            Some(Chip::Mention) => Some(theme::ACCENT_SOFT),
            Some(Chip::Status | Chip::File) => Some(theme::HOVER),
            None if span.code => Some(theme::HOVER),
            None => None,
        };
        let underline = match (&span.link, span.underline) {
            (Some(_), _) => {
                Some(UnderlineStyle { thickness: px(1.), color: Some(alpha(theme::ACCENT, 0.4).into()), wavy: false })
            }
            (None, true) => Some(UnderlineStyle { thickness: px(1.), color: None, wavy: false }),
            (None, false) => None,
        };
        made.runs.push(TextRun {
            len: words.len(),
            font: Font {
                weight,
                style: if span.em { FontStyle::Italic } else { FontStyle::Normal },
                features: if span.code {
                    gpui_kit::FontFeatures::disable_ligatures()
                } else {
                    gpui_kit::FontFeatures::default()
                },
                ..gpui_kit::font(family)
            },
            color: color.into(),
            background_color: fill.map(Hsla::from),
            underline,
            strikethrough: span.strike.then_some(StrikethroughStyle { thickness: px(1.), color: None }),
        });
        if let Some(url) = &span.link {
            // words that follow each other to the same page are one link
            match made.links.last_mut() {
                Some((range, same)) if range.end == from && same == url => range.end = made.text.len(),
                _ => made.links.push((from..made.text.len(), url.clone())),
            }
        }
    }
    made
}

/// A line of words with their marks; one with links hears a click on them.
fn words(spans: &[Span], base: Base, names: &Names) -> Option<AnyElement> {
    let line = line(spans, base);
    if line.text.is_empty() {
        return None;
    }
    let said = line.text.replace(PAD, "");
    // the words can be selected and copied: each line takes part in the window's selection, and a drag goes on from one
    // into the next
    let order = names.words.get();
    names.words.set(order + 1);
    let piece = SharedString::from(format!("{}:words:{order}", names.under));
    let text = SharedString::from(line.text);
    let styled = StyledText::new(text.clone()).with_runs(line.runs);
    if line.links.is_empty() {
        return Some(selectable(piece, text, styled).into_any_element());
    }
    let (ranges, urls): (Vec<Range<usize>>, Vec<String>) = line.links.into_iter().unzip();
    let id = names.next("links");
    let pages = urls.join(" ");
    let layout = styled.layout().clone();
    let linked = InteractiveText::new(SharedString::from(format!("{id}:text")), styled).on_click(
        ranges,
        move |ix, window, cx| {
            if let Some(url) = urls.get(ix) {
                open_in_browser(url, window, cx);
            }
        },
    );
    Some(
        div()
            .id(id)
            .test_support()
            .aria_label(SharedString::from(said))
            .aria_description(SharedString::from(pages))
            .child(selectable_as(piece, text, layout, linked))
            .into_any_element(),
    )
}

/// An image or a file, named: its sign, then what it is.
fn file_chip(name: &str) -> Div {
    div().flex().child(
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.))
            .rounded(px(5.))
            .bg(theme::HOVER)
            .px(px(6.))
            .py(px(1.))
            .text_size(px(11.5))
            .text_color(theme::INK_3)
            .child(tinted("icons/image.svg", theme::INK_3, 11.))
            .child(name.to_string()),
    )
}

/// A panel's edge and fill, by its kind.
fn panel_colors(kind: Panel) -> (Rgba, Rgba) {
    let violet = tone(300., 0.12);
    match kind {
        Panel::Info => (alpha(theme::ACCENT, 0.6), theme::ACCENT_SOFT),
        Panel::Note => (alpha(violet, 0.6), alpha(violet, 0.1)),
        Panel::Success => (alpha(theme::LAMP_GREEN, 0.6), alpha(theme::LAMP_GREEN, 0.1)),
        Panel::Warning => (alpha(theme::LAMP_AMBER, 0.6), alpha(theme::LAMP_AMBER, 0.1)),
        Panel::Error => (alpha(theme::LAMP_RED, 0.6), alpha(theme::LAMP_RED, 0.1)),
    }
}

/// A column of blocks, `gap` px apart.
fn column(gap: f32, blocks: Vec<AnyElement>) -> Div {
    div().min_w_0().flex().flex_col().gap(px(gap)).children(blocks)
}

fn draw(blocks: &[Block], base: Base, names: &Names, window: &mut Window, cx: &mut App) -> Vec<AnyElement> {
    let mut out: Vec<AnyElement> = Vec::new();
    for block in blocks {
        match block {
            Block::Paragraph(spans) => out.extend(words(spans, base, names)),
            Block::Heading { big, spans } => {
                let line = div().children(words(spans, HEAD, names));
                out.push(match big {
                    true => line.pt(px(4.)).text_size(px(14.5)).line_height(px(21.)).into_any_element(),
                    false => line.text_size(px(13.5)).line_height(px(20.)).into_any_element(),
                });
            }
            Block::List { start, items } => {
                // the marks hang in a gutter of their own, wide enough for the last number
                let last = start.map(|start| start + items.len().saturating_sub(1) as u64);
                let gutter = last.map_or(20., |last| 14. + 7. * last.to_string().len() as f32);
                let rows = items.iter().enumerate().map(|(n, item)| {
                    let mark = match start {
                        Some(start) => format!("{}.", start + n as u64),
                        None => "•".to_string(),
                    };
                    div()
                        .flex()
                        .items_start()
                        .child(
                            div()
                                .w(px(gutter))
                                .flex_none()
                                .pr(px(6.))
                                .flex()
                                .justify_end()
                                .text_color(theme::INK_3)
                                .child(mark),
                        )
                        .child(column(4., draw(item, base, names, window, cx)).flex_1())
                        .into_any_element()
                });
                out.push(column(4., rows.collect()).into_any_element());
            }
            Block::Tasks(tasks) => out.push(column(4., draw(tasks, base, names, window, cx)).into_any_element()),
            Block::Task { done, spans } => {
                let (icon, color) = match done {
                    true => ("icons/square-check.svg", theme::LAMP_GREEN),
                    false => ("icons/square.svg", theme::INK_3),
                };
                let said = if *done { QUIET } else { base };
                out.push(
                    div()
                        .flex()
                        .items_start()
                        .gap(px(8.))
                        .child(tinted(icon, color, 13.).mt(px(3.)))
                        .child(div().min_w_0().flex_1().children(words(spans, said, names)))
                        .into_any_element(),
                );
            }
            Block::Code(text) => out.push(
                mono(div())
                    .rounded(px(8.))
                    .border_1()
                    .border_color(theme::RULE)
                    .bg(alpha(theme::FRAME, 0.6))
                    .px(px(12.))
                    .py(px(8.))
                    .text_size(px(12.))
                    .line_height(px(18.))
                    .text_color(theme::INK)
                    .child(text.clone())
                    .into_any_element(),
            ),
            Block::Quote(blocks) => out.push(
                column(8., draw(blocks, QUIET, names, window, cx))
                    .border_l_2()
                    .border_color(theme::RULE_STRONG)
                    .pl(px(12.))
                    .text_color(theme::INK_3)
                    .into_any_element(),
            ),
            Block::Panel { kind, blocks } => {
                let (edge, fill) = panel_colors(*kind);
                out.push(
                    column(8., draw(blocks, base, names, window, cx))
                        .rounded(px(8.))
                        .border_l_2()
                        .border_color(edge)
                        .bg(fill)
                        .px(px(12.))
                        .py(px(8.))
                        .into_any_element(),
                );
            }
            Block::Rule => out.push(div().h(px(1.)).flex_none().bg(theme::RULE).into_any_element()),
            Block::Fold { title, blocks } => {
                let id = names.next("fold");
                // closed until it is opened, and it stays as it was left while the text is on screen
                let open = window.use_keyed_state(id.clone(), cx, |_, _| false);
                let is_open = *open.read(cx);
                let icon = if is_open { "icons/chevron-down.svg" } else { "icons/chevron-right.svg" };
                let inside = is_open.then(|| column(8., draw(blocks, base, names, window, cx)).mt(px(8.)));
                out.push(
                    div()
                        .rounded(px(8.))
                        .border_1()
                        .border_color(theme::RULE)
                        .px(px(12.))
                        .py(px(8.))
                        .child(
                            div()
                                .id(id)
                                .test_support()
                                .role(gpui_kit::Role::Button)
                                .aria_label(SharedString::from(title.clone()))
                                .aria_expanded(is_open)
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .text_color(theme::INK)
                                .cursor_pointer()
                                .child(tinted(icon, theme::INK_3, 12.))
                                .child(title.clone())
                                .on_click(move |_, _, cx| {
                                    open.update(cx, |open, cx| {
                                        *open = !*open;
                                        cx.notify();
                                    })
                                }),
                        )
                        .children(inside)
                        .into_any_element(),
                );
            }
            Block::Table(rows) => {
                let columns = rows.iter().map(Vec::len).max().unwrap_or(0).max(1);
                let mut cells: Vec<AnyElement> = Vec::new();
                for (r, row) in rows.iter().enumerate() {
                    for c in 0..columns {
                        let cell = row.get(c);
                        let header = cell.is_some_and(|cell| cell.header);
                        let said = if header { HEAD } else { base };
                        let inside = cell.map(|cell| draw(&cell.blocks, said, names, window, cx)).unwrap_or_default();
                        cells.push(
                            column(4., inside)
                                .px(px(10.))
                                .py(px(6.))
                                .when(header, |cell| cell.bg(theme::HOVER))
                                .when(r + 1 < rows.len(), |cell| cell.border_b_1().border_color(theme::RULE))
                                .into_any_element(),
                        );
                    }
                }
                out.push(
                    div()
                        .rounded(px(8.))
                        .border_1()
                        .border_color(theme::RULE)
                        .overflow_hidden()
                        .grid()
                        .grid_cols(columns as u16)
                        .text_size(px(12.5))
                        .children(cells)
                        .into_any_element(),
                );
            }
            Block::File(name) => out.push(file_chip(name).into_any_element()),
            Block::Pre(text) => out.push(div().child(text.clone()).into_any_element()),
        }
    }
    out
}

/// The blocks, drawn: 13 px on 20, quiet ink, 10 px between two blocks. `id` names the text; all it says is its
/// label.
pub fn rich(id: impl Into<SharedString>, blocks: &[Block], window: &mut Window, cx: &mut App) -> Observed {
    rich_in(id, blocks, BODY, window, cx)
}

/// The same in bright ink: words that are what you opened this for (Claude's report).
pub fn rich_bright(id: impl Into<SharedString>, blocks: &[Block], window: &mut Window, cx: &mut App) -> Observed {
    rich_in(id, blocks, BRIGHT, window, cx)
}

fn rich_in(id: impl Into<SharedString>, blocks: &[Block], base: Base, window: &mut Window, cx: &mut App) -> Observed {
    let id: SharedString = id.into();
    let names = Names { under: id.clone(), next: Cell::new(0), words: Cell::new(0) };
    div()
        .id(id)
        .test_support()
        .aria_label(SharedString::from(rich::plain(blocks)))
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(10.))
        .text_size(px(13.))
        .line_height(px(20.))
        .text_color(base.color)
        .children(draw(blocks, base, &names, window, cx))
}
