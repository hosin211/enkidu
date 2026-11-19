//! Toasts (DESIGN.md, "Motion": "Toasts: rise from the corner (300 ms), leave to the right (180 ms)"): news, and what
//! went wrong with how to fix it, one above the other in the window's bottom right corner, above the status line
//! (which they never cover). Each window has its own
//! [`Toasts`], drawn by [`layer`] as one of the window's last children; a screen test reads them by "toast" and
//! "toast-detail".
//!
//! How long one stays: what went wrong stays until you close it; news goes after 4 seconds, or 8 when there is
//! something under its title to read. A long detail (more than 160 characters, or several lines) shows its first three
//! lines with "Show all".
//!
//! They were gpui-kit's notifications until item 7.2a: those take 400 ms to come and 200 to go, sink as they leave,
//! and lie on each other; their times are fixed in the library.

use std::collections::HashMap;
use std::time::Duration;

use enkidu_ui::pieces::raised;
use enkidu_ui::tokens::{self as theme, EASE_OUT, bezier};
use enkidu_ui::work::tinted;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, AnyWindowHandle, App, AsyncApp, Context, Entity, FontWeight, Global,
    Role, SharedString, Window, WindowId, deferred, div, px,
};

/// News without a detail stays this long.
pub const STAYS: Duration = Duration::from_secs(4);
/// News with a detail to read stays this long.
pub const STAYS_WITH_DETAIL: Duration = Duration::from_secs(8);
/// A detail longer than this, or of several lines, is folded to three lines.
const LONG: usize = 160;
/// A toast rises into its place for this long, from [`RISE`] px under it.
pub const COMES: Duration = Duration::from_millis(300);
pub const RISE: f32 = 14.;
/// It leaves for this long, [`LEAVES_BY`] px to the right.
pub const GOES: Duration = Duration::from_millis(180);
pub const LEAVES_BY: f32 = 28.;
/// How wide a toast is, and how far the column stands from the window's corner.
const WIDTH: f32 = 360.;
const CORNER: f32 = 16.;
/// Above dialogs and menus (100), under tooltips (200).
const ABOVE: usize = 150;

/// What a toast is: its sign says so.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// News and where it went, with the blue sign.
    Info,
    /// Something done, with the green sign.
    Success,
    /// What went wrong, with the red sign.
    Error,
}

/// How long a toast stays: one that says what went wrong, until it is closed.
fn stays(kind: Kind, detail: bool) -> Option<Duration> {
    match (kind, detail) {
        (Kind::Error, _) => None,
        (_, true) => Some(STAYS_WITH_DETAIL),
        (_, false) => Some(STAYS),
    }
}

/// Whether a detail is folded until "Show all" is pressed.
fn long(detail: &str) -> bool {
    detail.chars().count() > LONG || detail.contains('\n')
}

struct Toast {
    key: usize,
    kind: Kind,
    title: SharedString,
    detail: Option<SharedString>,
    /// The title in the weight of a title.
    titled: bool,
    unfolded: bool,
    leaving: bool,
}

/// The toasts of one window, the oldest first.
#[derive(Default)]
pub struct Toasts {
    list: Vec<Toast>,
    next: usize,
}

#[derive(Default)]
struct AllToasts(HashMap<WindowId, Entity<Toasts>>);

impl Global for AllToasts {}

impl Toasts {
    /// The window's toasts.
    pub fn of(window: &Window, cx: &mut App) -> Entity<Toasts> {
        let id = window.window_handle().window_id();
        if let Some(toasts) = cx.default_global::<AllToasts>().0.get(&id) {
            return toasts.clone();
        }
        let toasts = cx.new(|_| Toasts::default());
        cx.default_global::<AllToasts>().0.insert(id, toasts.clone());
        toasts
    }

    fn push(
        &mut self,
        kind: Kind,
        title: SharedString,
        detail: Option<SharedString>,
        titled: bool,
        cx: &mut Context<Self>,
    ) {
        let key = self.next;
        self.next += 1;
        let stay = stays(kind, detail.is_some());
        self.list.push(Toast { key, kind, title, detail, titled, unfolded: false, leaving: false });
        cx.notify();
        if let Some(stay) = stay {
            cx.spawn(async move |toasts, cx| {
                cx.background_executor().timer(stay).await;
                let _ = toasts.update(cx, |toasts, cx| toasts.dismiss(key, cx));
            })
            .detach();
        }
    }

    /// The toast leaves: to the right for 180 ms, then it is gone.
    fn dismiss(&mut self, key: usize, cx: &mut Context<Self>) {
        let Some(toast) = self.list.iter_mut().find(|toast| toast.key == key && !toast.leaving) else { return };
        toast.leaving = true;
        cx.notify();
        cx.spawn(async move |toasts, cx| {
            cx.background_executor().timer(GOES).await;
            let _ = toasts.update(cx, |toasts, cx| {
                toasts.list.retain(|toast| toast.key != key);
                cx.notify();
            });
        })
        .detach();
    }

    fn card(&self, toast: &Toast, cx: &mut Context<Self>) -> AnyElement {
        let key = toast.key;
        let (sign, color) = match toast.kind {
            Kind::Error => ("icons/triangle-alert.svg", theme::LAMP_RED),
            Kind::Success => ("icons/circle-check.svg", theme::LAMP_GREEN),
            Kind::Info => ("icons/info.svg", theme::ACCENT),
        };
        let title = div()
            .id("toast")
            .test_support()
            .aria_label(toast.title.clone())
            .text_size(theme::TEXT)
            .line_height(px(18.))
            .text_color(theme::INK)
            .when(toast.titled, |title| title.font_weight(FontWeight::MEDIUM))
            .child(toast.title.clone());
        let folds = toast.detail.as_ref().is_some_and(|detail| long(detail));
        let shown = toast.unfolded;
        let detail = toast.detail.clone().map(|detail| {
            // the detail: 11 px in the code face, a step brighter than the line under it
            enkidu_ui::pieces::mono(div())
                .id("toast-detail")
                .test_support()
                .aria_label(detail.clone())
                .mt(px(4.))
                .text_size(px(11.))
                .line_height(px(16.))
                .text_color(theme::INK_2)
                // folded: three lines of it (a clamp by lines would count each line of several on its own)
                .when(folds && !shown, |detail| detail.max_h(px(48.)).overflow_hidden())
                .child(enkidu_ui::selectable::words(("toast-detail-words", key), detail))
        });
        let unfold = folds.then(|| {
            let words = if shown { "Show less" } else { "Show all" };
            div().flex().mt(px(4.)).child(
                div()
                    .id("toast-unfold")
                    .test_support()
                    .role(Role::Button)
                    .aria_label(words)
                    .text_size(theme::TEXT_SMALL)
                    .line_height(theme::TEXT_SMALL_LINE)
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::INK_3)
                    .hover(|s| s.text_color(theme::INK))
                    .cursor_pointer()
                    .child(words)
                    .on_click(cx.listener(move |toasts, _, _, cx| {
                        if let Some(toast) = toasts.list.iter_mut().find(|toast| toast.key == key) {
                            toast.unfolded = !toast.unfolded;
                            cx.notify();
                        }
                    })),
            )
        });
        let close = div()
            .id(("toast-close", key))
            .test_support()
            .role(Role::Button)
            .aria_label("Dismiss")
            .flex_none()
            .mr(px(-4.))
            .size(px(20.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .cursor_pointer()
            .hover(|s| s.bg(theme::HOVER))
            .child(tinted("icons/x.svg", theme::INK_3, 12.))
            .on_click(cx.listener(move |toasts, _, _, cx| toasts.dismiss(key, cx)));
        let card = raised(("toast-card", key))
            .role(Role::Status)
            .w_full()
            .px(px(16.))
            .py(px(12.))
            // it takes the mouse: what is under a toast isn't pressed through it
            .occlude()
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(8.))
                    .child(tinted(sign, color, 16.).mt(px(1.)))
                    .child(div().min_w_0().flex_1().child(title).children(detail).children(unfold))
                    .child(close),
            );
        // it rises from under its place as it comes, and goes to the right; with "Reduce motion" it is in its
        // place at once and gone at once (`with_animation` draws the end)
        let moved = div().relative().w_full();
        match toast.leaving {
            false => moved
                .child(card)
                .with_animation(
                    ("toast-in", key),
                    Animation::new(COMES).with_easing(|t| bezier(EASE_OUT, t)),
                    |toast, t| toast.top(px(RISE * (1. - t))).opacity(t),
                )
                .into_any_element(),
            true => moved
                .child(card)
                .with_animation(
                    ("toast-out", key),
                    Animation::new(GOES).with_easing(|t| bezier(EASE_OUT, t)),
                    |toast, t| toast.left(px(LEAVES_BY * t)).opacity(1. - t),
                )
                .into_any_element(),
        }
    }
}

impl Render for Toasts {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.list.is_empty() {
            return div().into_any_element();
        }
        let cards: Vec<AnyElement> = self.list.iter().map(|toast| self.card(toast, cx)).collect();
        deferred(
            div()
                .id("toasts")
                .absolute()
                // 8 px above the stage's foot: the status line's 28 px under the stage stay free
                .bottom(theme::STATUS_LINE + px(8.))
                .right(px(CORNER))
                .w(px(WIDTH))
                .flex()
                .flex_col()
                .gap(px(8.))
                .children(cards),
        )
        .with_priority(ABOVE)
        .into_any_element()
    }
}

/// The window's toasts, for the window's root to draw among its last children (the root fills the window).
pub fn layer(window: &mut Window, cx: &mut App) -> impl IntoElement + use<> {
    div().absolute().inset_0().child(Toasts::of(window, cx))
}

/// Says what went wrong, `words`, in a toast in `window` (with the red warning sign).
pub fn say(window: &mut Window, words: impl Into<SharedString>, cx: &mut App) {
    push(Kind::Error, words.into(), None, false, window, cx);
}

/// The same, from work that finished off the drawing thread.
pub fn say_later(window: AnyWindowHandle, words: impl Into<SharedString>, cx: &mut AsyncApp) {
    let words = words.into();
    let _ = cx.update_window(window, |_, window, cx| say(window, words, cx));
}

/// Good news in one line ("Settings saved"), with the green sign.
pub fn good(window: &mut Window, words: impl Into<SharedString>, cx: &mut App) {
    push(Kind::Success, words.into(), None, true, window, cx);
}

/// News and where it went ("Path copied", "~/code/shop"), with the blue sign.
pub fn note(window: &mut Window, title: impl Into<SharedString>, detail: impl Into<SharedString>, cx: &mut App) {
    push(Kind::Info, title.into(), Some(detail.into()), true, window, cx);
}

/// Something done, and what it did ("stock-fix is Live", what was copied where), with the green sign.
pub fn done(window: &mut Window, title: impl Into<SharedString>, detail: impl Into<SharedString>, cx: &mut App) {
    push(Kind::Success, title.into(), Some(detail.into()), true, window, cx);
}

/// Something that went wrong, and why ("Live didn't switch", the command's own words), with the red sign.
pub fn warning(window: &mut Window, title: impl Into<SharedString>, detail: impl Into<SharedString>, cx: &mut App) {
    push(Kind::Error, title.into(), Some(detail.into()), true, window, cx);
}

/// One toast in the window's bottom right corner, under the ones already there.
fn push(
    kind: Kind,
    title: SharedString,
    detail: Option<SharedString>,
    titled: bool,
    window: &mut Window,
    cx: &mut App,
) {
    Toasts::of(window, cx).update(cx, |toasts, cx| toasts.push(kind, title, detail, titled, cx));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_went_wrong_stays_and_news_goes_sooner_without_a_detail() {
        assert_eq!(stays(Kind::Error, false), None);
        assert_eq!(stays(Kind::Error, true), None);
        assert_eq!(stays(Kind::Success, false), Some(Duration::from_secs(4)));
        assert_eq!(stays(Kind::Info, true), Some(Duration::from_secs(8)));
        assert_eq!(stays(Kind::Success, true), Some(Duration::from_secs(8)));
    }

    #[test]
    fn a_detail_of_several_lines_or_over_160_characters_is_folded() {
        assert!(!long("~/code/shop"));
        assert!(long("Copied 1 file.\n\n$ on (stock-fix)"));
        assert!(!long(&"x".repeat(160)));
        assert!(long(&"x".repeat(161)));
    }

    #[test]
    fn a_toast_comes_and_goes_by_the_designs_times() {
        let design = include_str!("../../../DESIGN.md");
        assert!(design.contains("| Toasts | rise from the corner (300 ms), leave to the right (180 ms) |"));
        assert_eq!((COMES, GOES), (Duration::from_millis(300), Duration::from_millis(180)));
    }
}
