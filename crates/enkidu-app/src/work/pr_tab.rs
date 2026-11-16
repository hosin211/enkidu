//! A PR's tab in the card (DESIGN.md, "The card in front of the board"): what is going on there, all from the same read
//! as the board. Who opened it and how big it is; what the PR says (GitHub's Markdown, drawn as text); the checks; the
//! reviews; the review threads; the conversation; the latest commits. GitHub is a click away from each. What it says is
//! worked out in `enkidu_model::work::prtab`.

use enkidu_model::work::prtab::{Bucket, CheckRow, CommentRow, Person, PrTab, ThreadRow};
use enkidu_ui::lamp::LAMP_FPS;
use enkidu_ui::pieces::{Observed, mono};
use enkidu_ui::tips::Tipped as _;
use enkidu_ui::tokens::{self as theme, alpha};
use enkidu_ui::work::{person_badge, tinted};
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, Context, Div, FontWeight, Role, SharedString, Transformation, Window,
    div, linear_color_stop, linear_gradient, percentage, px, svg,
};

use crate::work::board::ink;
use crate::work::card::{Card, open_in_browser};
use crate::work::rich::rich;

/// What you opened in a PR's tab: the checks that passed, the resolved threads, the whole description.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Opened {
    pub passed: bool,
    pub resolved: bool,
    pub body: bool,
}

/// A PR's description is at most this wide, and a long one shows this much of its start.
const BODY_WIDTH: f32 = 548.;
const BODY_START: f32 = 220.;

fn badge(name: &str, size: f32) -> Div {
    let person = Person::of(name);
    person_badge(person.initials, person.hue, size)
}

/// A small word after a name: "bot", "outdated".
fn tag(word: &'static str) -> Div {
    div()
        .flex_none()
        .rounded(px(4.))
        .bg(theme::THUMB)
        .px(px(4.))
        .text_size(px(10.5))
        .text_color(theme::INK_3)
        .child(word)
}

/// The sign that opens something on GitHub (or a check's own page) in the browser.
fn outside(id: SharedString, label: String, url: String) -> Observed {
    div()
        .id(id.clone())
        .test_support()
        .role(Role::Link)
        .aria_label(SharedString::from(label))
        .aria_description(SharedString::from(url.clone()))
        .flex_none()
        .group(id.clone())
        .cursor_pointer()
        .child(
            svg()
                .path("icons/external-link.svg")
                .size(px(12.))
                .text_color(theme::INK_3)
                .group_hover(id, |s| s.text_color(theme::ACCENT)),
        )
        .on_click(move |_, window, cx| open_in_browser(&url, window, cx))
}

/// A part of the tab: its title, a count beside it when that says something, then what it holds.
fn section(id: &'static str, title: &'static str, count: Option<AnyElement>, said: String) -> Observed {
    div().id(id).test_support().aria_label(title).aria_description(SharedString::from(said)).flex_none().child(
        div()
            .mb(px(8.))
            .flex()
            .items_center()
            .gap(px(8.))
            .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(theme::INK_3).child(title))
            .children(count),
    )
}

/// A quiet button under a list: "Show the 5 that passed or were skipped".
fn more(id: &'static str, words: String) -> enkidu_ui::pieces::Pressed {
    enkidu_ui::press::pressable(div().id(id))
        .test_support()
        .role(Role::Button)
        .aria_label(SharedString::from(words.clone()))
        .mt(px(6.))
        .flex()
        .child(
            div()
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::INK_3)
                .cursor_pointer()
                .hover(|s| s.text_color(theme::ACCENT))
                .child(words),
        )
}

fn check_row(n: usize, check: &CheckRow) -> AnyElement {
    let color = ink(check.bucket.tone()).color();
    let icon = match check.bucket {
        Bucket::Fail => "icons/circle-x.svg",
        Bucket::Cancel | Bucket::Skipping => "icons/circle-minus.svg",
        Bucket::Pending => "icons/circle-dashed.svg",
        Bucket::Pass => "icons/circle-check.svg",
    };
    let sign = tinted(icon, color, 13.);
    // a check that runs turns, in steps, on the lamps' clock
    let sign = match check.bucket {
        Bucket::Pending => sign
            .with_animation(
                ("pr-check-turn", n),
                Animation::new(std::time::Duration::from_millis(2400)).repeat_synced().with_max_fps(LAMP_FPS),
                |sign, t| sign.with_transformation(Transformation::rotate(percentage(t))),
            )
            .into_any_element(),
        _ => sign.into_any_element(),
    };
    let word = check.bucket.word();
    let open = check.url.clone().map(|url| {
        outside(SharedString::from(format!("pr-check-open:{n}")), format!("Open {}", check.name), url)
            .tipped("Open the check", None)
    });
    div()
        .id(("pr-check", n))
        .test_support()
        .aria_label(SharedString::from(format!("{}: {word}", check.name)))
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(8.))
        .rounded(px(6.))
        .px(px(4.))
        .py(px(3.))
        .text_size(px(12.5))
        .hover(|s| s.bg(theme::HOVER))
        .child(sign)
        .child(div().min_w_0().flex_1().truncate().text_color(theme::INK_2).child(check.name.clone()))
        .child(div().flex_none().text_size(px(11.5)).text_color(color).child(word))
        .children(open)
        .into_any_element()
}

fn thread_row(n: usize, thread: &ThreadRow) -> AnyElement {
    let sign = match thread.resolved {
        true => tinted("icons/message-square-dashed.svg", theme::INK_3, 13.),
        false => tinted("icons/message-square.svg", theme::ACCENT, 13.),
    };
    let said = format!("{}: {}", thread.who, thread.text);
    let open = thread.url.clone().map(|url| {
        outside(SharedString::from(format!("pr-thread-open:{n}")), "Open the thread on GitHub".to_string(), url)
    });
    let text = gpui_kit::StyledText::new(said.clone()).with_runs(vec![
        gpui_kit::TextRun {
            len: thread.who.len() + 1,
            font: gpui_kit::Font { weight: FontWeight::MEDIUM, ..gpui_kit::font(theme::UI_FONT) },
            color: theme::INK.into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        },
        gpui_kit::TextRun {
            len: said.len() - thread.who.len() - 1,
            font: gpui_kit::font(theme::UI_FONT),
            color: theme::INK_2.into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        },
    ]);
    div()
        .id(("pr-thread", n))
        .test_support()
        .aria_label(SharedString::from(said))
        .aria_description(SharedString::from(
            [
                Some(if thread.resolved { "Resolved".to_string() } else { "Open".to_string() }),
                thread.place.clone(),
                thread.outdated.then(|| "outdated".to_string()),
                thread.replies.clone(),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(", "),
        ))
        .rounded(px(10.))
        .border_1()
        .border_color(theme::RULE)
        .px(px(12.))
        .py(px(10.))
        .when(thread.resolved, |row| row.opacity(0.7))
        .child(
            div()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(8.))
                .text_size(px(12.))
                .child(sign)
                .children(thread.place.clone().map(|place| {
                    mono(div()).min_w_0().truncate().text_size(px(11.5)).text_color(theme::INK_2).child(place)
                }))
                .when(thread.outdated, |head| head.child(tag("outdated")))
                .child(div().flex_1())
                .children(open),
        )
        .child(div().mt(px(6.)).text_size(px(12.5)).line_height(px(18.)).line_clamp(4).child(text))
        .children(
            thread
                .replies
                .clone()
                .map(|replies| div().mt(px(6.)).text_size(px(11.5)).text_color(theme::INK_3).child(replies)),
        )
        .into_any_element()
}

fn comment_row(n: usize, comment: &CommentRow) -> AnyElement {
    let who = comment.who.clone().unwrap_or_else(|| "Someone".into());
    let face = match &comment.who {
        Some(name) => badge(name, 20.).into_any_element(),
        None => tinted("icons/message-circle.svg", theme::INK_3, 16.).into_any_element(),
    };
    div()
        .id(("pr-comment", n))
        .test_support()
        .aria_label(SharedString::from(format!("{who}: {}", comment.text)))
        .aria_description(SharedString::from(comment.when.clone()))
        .flex()
        .gap(px(10.))
        .when(comment.bot, |row| row.opacity(0.75))
        .child(face)
        .child(
            div()
                .min_w_0()
                .flex_1()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .text_size(px(12.))
                        .child(div().font_weight(FontWeight::MEDIUM).text_color(theme::INK).child(who))
                        .when(comment.bot, |head| head.child(tag("bot")))
                        .child(div().text_color(theme::INK_3).child(comment.when.clone()))
                        .child(div().flex_1())
                        .child(outside(
                            SharedString::from(format!("pr-comment-open:{n}")),
                            "Open the comment on GitHub".to_string(),
                            comment.url.clone(),
                        )),
                )
                .child(
                    div()
                        .mt(px(2.))
                        .text_size(px(12.5))
                        .line_height(px(18.))
                        .text_color(theme::INK_2)
                        .line_clamp(5)
                        .child(comment.text.clone()),
                ),
        )
        .into_any_element()
}

/// The tab's parts, top to bottom; the card's left side puts 24 px between them.
pub fn pr_tab(pr: &PrTab, opened: Opened, window: &mut Window, cx: &mut Context<Card>) -> Vec<AnyElement> {
    let mut parts: Vec<AnyElement> = Vec::new();
    // who, when, how big
    let url = pr.url.clone();
    let mut said: Vec<String> = Vec::new();
    let mut top = div()
        .id("pr-details")
        .test_support()
        .flex_none()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_x(px(16.))
        .gap_y(px(6.))
        .text_size(px(12.5))
        .text_color(theme::INK_3);
    if let Some((author, when)) = &pr.opened {
        said.push(format!("{author} opened it {when}"));
        top = top.child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(badge(author, 16.))
                .child(div().text_color(theme::INK_2).child(author.clone()))
                .child(format!("opened it {when}")),
        );
    }
    if let Some((added, removed, files)) = &pr.size {
        let words = pr.size_words().unwrap_or_default();
        said.push(words.clone());
        top = top.child(
            div()
                .id("pr-size")
                .test_support()
                .aria_label(SharedString::from(words))
                .flex()
                .items_center()
                .gap(px(4.))
                .child(div().text_color(theme::ADD_INK).child(added.clone()))
                .child(div().text_color(theme::DEL_INK).child(removed.clone()))
                .child(files.clone()),
        );
    }
    said.push(pr.commits.clone());
    top = top.child(pr.commits.clone());
    for label in &pr.labels {
        said.push(label.clone());
        top = top.child(
            div()
                .rounded_full()
                .border_1()
                .border_color(theme::RULE_STRONG)
                .px(px(8.))
                .py(px(1.))
                .text_size(px(11.5))
                .text_color(theme::INK_2)
                .child(label.clone()),
        );
    }
    top = top
        .aria_label(SharedString::from(format!("PR #{}", pr.number)))
        .aria_description(SharedString::from(said.join(", ")));
    top = top.child(div().flex_1()).child(
        div()
            .id("pr-open")
            .test_support()
            .role(Role::Link)
            .aria_label("Open on GitHub")
            .aria_description(SharedString::from(url.clone()))
            .group("pr-open")
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.))
            .text_size(px(12.))
            .cursor_pointer()
            .hover(|s| s.text_color(theme::ACCENT))
            .child("Open on GitHub")
            .child(
                svg()
                    .path("icons/external-link.svg")
                    .size(px(11.))
                    .text_color(theme::INK_3)
                    .group_hover("pr-open", |s| s.text_color(theme::ACCENT)),
            )
            .on_click(move |_, window, cx| open_in_browser(&url, window, cx)),
    );
    parts.push(top.into_any_element());

    // what the PR says: GitHub's Markdown, drawn as text; a long one shows its start
    if let Some(blocks) = &pr.body {
        let folded = pr.long_body && !opened.body;
        let fade = folded.then(|| {
            let (clear, solid) = (alpha(theme::RAISED, 0.), theme::RAISED);
            div().absolute().left_0().right_0().bottom_0().h(px(56.)).bg(linear_gradient(
                180.,
                linear_color_stop(clear, 0.),
                linear_color_stop(solid, 1.),
            ))
        });
        let words = if opened.body { "Show less" } else { "Show the whole description" };
        parts.push(
            section("pr-body", "What the PR says", None, if folded { "Its start" } else { "All of it" }.to_string())
                .child(
                    div()
                        .id("pr-body-text")
                        .test_support()
                        .relative()
                        .when(folded, |text| text.max_h(px(BODY_START)).overflow_hidden())
                        .child(div().flex_none().max_w(px(BODY_WIDTH)).child(rich("pr-markdown", blocks, window, cx)))
                        .children(fade),
                )
                .children(pr.long_body.then(|| {
                    more("pr-body-more", words.to_string()).on_click(cx.listener(|card, _, _, cx| {
                        card.pr_opened.body = !card.pr_opened.body;
                        cx.notify();
                    }))
                }))
                .into_any_element(),
        );
    }

    // the checks, counted by how they stand; the ones that passed behind a click while others didn't
    if !pr.check_counts.is_empty() {
        let counts = div().flex().items_center().gap(px(8.)).text_size(px(11.5)).children(pr.check_counts.iter().map(
            |(bucket, n)| {
                div().text_color(ink(bucket.tone()).color()).child(format!("{n} {}", bucket.word().to_lowercase()))
            },
        ));
        let all = opened.passed || pr.checks.is_empty();
        let rows = pr.checks.iter().chain(pr.passed.iter().filter(|_| all)).enumerate().map(|(n, c)| check_row(n, c));
        let hidden = (!all && !pr.passed.is_empty()).then(|| {
            more("pr-checks-more", format!("Show the {} that passed or were skipped", pr.passed.len())).on_click(
                cx.listener(|card, _, _, cx| {
                    card.pr_opened.passed = true;
                    cx.notify();
                }),
            )
        });
        parts.push(
            section("pr-checks-list", "Checks", Some(counts.into_any_element()), pr.check_words())
                .child(div().flex().flex_col().gap(px(2.)).children(rows))
                .children(hidden)
                .into_any_element(),
        );
    }

    // each reviewer's latest review, and who is asked and hasn't yet
    if !pr.reviews.is_empty() || !pr.requested.is_empty() {
        let reviews = pr.reviews.iter().enumerate().map(|(n, review)| {
            let said = format!("{} {} {}", review.who, review.word, review.when);
            div()
                .id(("pr-review", n))
                .test_support()
                .aria_label(SharedString::from(said.trim_end().to_string()))
                .flex()
                .items_center()
                .gap(px(8.))
                .text_size(px(12.5))
                .child(badge(&review.who, 18.))
                .child(div().text_color(theme::INK).child(review.who.clone()))
                .when(review.bot, |row| row.child(tag("bot")))
                .child(div().font_weight(FontWeight::MEDIUM).text_color(ink(review.tone).color()).child(review.word))
                .child(div().flex_1())
                .child(div().text_size(px(11.5)).text_color(theme::INK_3).child(review.when.clone()))
                .into_any_element()
        });
        let asked = pr.requested.iter().enumerate().map(|(n, who)| {
            div()
                .id(("pr-requested", n))
                .test_support()
                .aria_label(SharedString::from(format!("{who} asked to review, hasn't yet")))
                .flex()
                .items_center()
                .gap(px(8.))
                .text_size(px(12.5))
                .child(
                    div()
                        .size(px(18.))
                        .flex_none()
                        .rounded_full()
                        .border_1()
                        .border_dashed()
                        .border_color(theme::RULE_STRONG),
                )
                .child(div().text_color(theme::INK_2).child(who.clone()))
                .child(div().text_color(theme::INK_3).child("asked to review, hasn't yet"))
                .into_any_element()
        });
        let said = format!("{} reviewed, {} asked", pr.reviews.len(), pr.requested.len());
        parts.push(
            section("pr-reviews", "Reviews", None, said)
                .child(div().flex().flex_col().gap(px(6.)).children(reviews).children(asked))
                .into_any_element(),
        );
    }

    // every review thread: the open ones, the resolved ones behind a click
    if !pr.open_threads.is_empty() || !pr.resolved_threads.is_empty() {
        let words = pr.thread_words();
        let count = div().text_size(px(11.5)).text_color(theme::INK_3).child(words.clone());
        let rows = (pr.open_threads.iter())
            .chain(pr.resolved_threads.iter().filter(|_| opened.resolved))
            .enumerate()
            .map(|(n, thread)| thread_row(n, thread));
        let hidden = (!opened.resolved && !pr.resolved_threads.is_empty()).then(|| {
            more("pr-threads-more", format!("Show the {} resolved", pr.resolved_threads.len())).on_click(cx.listener(
                |card, _, _, cx| {
                    card.pr_opened.resolved = true;
                    cx.notify();
                },
            ))
        });
        parts.push(
            section("pr-threads", "Review threads", Some(count.into_any_element()), words)
                .child(div().flex().flex_col().gap(px(8.)).children(rows))
                .children(hidden)
                .into_any_element(),
        );
    }

    // the conversation
    if !pr.comments.is_empty() {
        let count = div().text_size(px(11.5)).text_color(theme::INK_3).child(pr.comments.len().to_string());
        let rows = pr.comments.iter().enumerate().map(|(n, comment)| comment_row(n, comment));
        parts.push(
            section("pr-conversation", "Conversation", Some(count.into_any_element()), pr.comments.len().to_string())
                .child(div().flex().flex_col().gap(px(10.)).children(rows))
                .into_any_element(),
        );
    }

    // the latest commits
    if !pr.recent_commits.is_empty() {
        let count = div().text_size(px(11.5)).text_color(theme::INK_3).child(pr.commits_shown.clone());
        let headlines: Vec<&str> = pr.recent_commits.iter().map(|(_, headline)| headline.as_str()).collect();
        let rows = pr.recent_commits.iter().map(|(oid, headline)| {
            div()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(8.))
                .text_size(px(12.5))
                .child(tinted("icons/git-commit-horizontal.svg", theme::INK_3, 13.))
                .child(mono(div()).flex_none().text_size(px(11.5)).text_color(theme::INK_3).child(oid.clone()))
                .child(div().min_w_0().truncate().text_color(theme::INK_2).child(headline.clone()))
        });
        parts.push(
            section("pr-commits", "Latest commits", Some(count.into_any_element()), headlines.join("\n"))
                .child(div().flex().flex_col().gap(px(2.)).children(rows))
                .into_any_element(),
        );
    }
    parts
}
