//! Tooltips (DESIGN.md, "Color": `tip`; "Motion": "Menus, tooltips: grow from their trigger, 160 ms; out in 110; the
//! next tooltip along a toolbar is instant").
//!
//! A tooltip comes 450 ms after the pointer rests on something that has one, on the side of that thing (below it,
//! 6 px away, or above when there is no room), easing in out of it. It goes as the pointer leaves, fading for
//! 110 ms. The next one within a quarter of a second needs no wait and no motion: reading along a toolbar. A press
//! on something sends its tooltip away at once and keeps it away until the pointer has left it, so it never comes
//! up over the dialog or the menu that press opened. Typing and the wheel send it away too.
//!
//! GPUI's own tooltips (`.tooltip(..)`) sit at the pointer, come and go at once, and one that still waits when its
//! button is pressed shows over what the button opened (item 7.2b). These are our own: each window has one
//! [`Tips`], drawn by [`layer`] as the window's last child, and [`Tipped::tipped`] puts a thing's words under it.

use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

use gpui_kit::base::{Placement, Positioner, TestSupportExt as _};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::prelude::*;
use gpui_kit::{
    Action, Animation, AnimationExt as _, AnyElement, App, Bounds, BoxShadow, Context, DispatchPhase, Entity, Global,
    GlobalElementId, HitboxBehavior, MouseDownEvent, MouseMoveEvent, ParentElement, Pixels, Role, ScrollEvent,
    SharedString, Task, Window, WindowId, canvas, deferred, div, point, px,
};

use crate::tokens::{DUR_QUICK, EASE_OUT, INK, INK_2, TIP, alpha, bezier, hex, hsla, white};

const BLACK: gpui_kit::Rgba = hex(0x000000);

/// How long the pointer rests on something before its tooltip comes.
pub const SHOW_AFTER: Duration = Duration::from_millis(450);
/// For this long after a tooltip went, the next one comes at once.
pub const NEXT_WITHIN: Duration = Duration::from_millis(250);
/// How long a tooltip takes to go.
pub const LEAVE: Duration = Duration::from_millis(110);
/// How far from its place a tooltip starts as it eases in, toward what it belongs to.
pub const RISE: f32 = 3.;
/// A tooltip is drawn above everything, dialogs and menus too (gpui-kit's popups are at 100).
const ABOVE_ALL: usize = 200;
/// The space between a tooltip and what it belongs to.
const AWAY: f32 = 6.;

/// Something that has a tooltip, as last drawn.
#[derive(Clone)]
struct Trigger {
    id: GlobalElementId,
    bounds: Bounds<Pixels>,
    words: SharedString,
    action: Option<Rc<Box<dyn Action>>>,
    /// The key context the action's key is bound in, when it isn't bound everywhere.
    context: Option<&'static str>,
    /// Gone once a frame is drawn without it.
    alive: Weak<()>,
}

#[derive(Default)]
enum Shown {
    #[default]
    Nothing,
    /// `eased`: it came after the wait, so it eases in. One that follows another is there at once.
    There {
        trigger: Trigger,
        eased: bool,
    },
    Leaving {
        trigger: Trigger,
        _gone: Task<()>,
    },
}

/// The tooltips of one window.
#[derive(Default)]
pub struct Tips {
    /// What the pointer is on.
    over: Option<GlobalElementId>,
    shown: Shown,
    wait: Option<Task<()>>,
    /// When the last tooltip went.
    went: Option<Instant>,
    /// Pressed: it says nothing again until the pointer has left it.
    pressed: Option<GlobalElementId>,
    /// Counts tooltips, so each one's motion is its own.
    turn: usize,
    /// Something under the pointer already spoke for this move of it.
    claimed: bool,
    /// Hears the window's keys for as long as these tooltips live.
    keys: Option<gpui_kit::Subscription>,
}

#[derive(Default)]
struct AllTips(HashMap<WindowId, Entity<Tips>>);

impl Global for AllTips {}

impl Tips {
    /// The window's tooltips.
    pub fn of(window: &Window, cx: &mut App) -> Entity<Tips> {
        let id = window.window_handle().window_id();
        if let Some(tips) = cx.default_global::<AllTips>().0.get(&id) {
            return tips.clone();
        }
        let tips = cx.new(|_| Tips::default());
        // a key typed anywhere in the window sends its tooltip away (heard before any binding, wherever the
        // keyboard is)
        let heard = tips.downgrade();
        let keys = cx.intercept_keystrokes(move |_, window, cx| {
            if window.window_handle().window_id() == id {
                let _ = heard.update(cx, |tips, cx| tips.away(cx));
            }
        });
        tips.update(cx, |tips, _| tips.keys = Some(keys));
        cx.default_global::<AllTips>().0.insert(id, tips.clone());
        tips
    }

    /// What the tooltip on screen says, if one is (a leaving one no longer counts).
    pub fn says(&self) -> Option<SharedString> {
        match &self.shown {
            Shown::There { trigger, .. } => Some(trigger.words.clone()),
            _ => None,
        }
    }

    /// The pointer moved: it is on `trigger` or it isn't. Of two things under it, one inside the other, the inner
    /// one hears first and speaks (a PR's dot on a tab).
    fn pointer(&mut self, trigger: &Trigger, on_it: bool, window: &mut Window, cx: &mut Context<Self>) {
        if on_it {
            if self.claimed {
                return;
            }
            self.claimed = true;
        }
        let was = self.over.as_ref() == Some(&trigger.id);
        match (was, on_it) {
            (false, true) => self.came(trigger.clone(), window, cx),
            (true, false) => self.left(&trigger.id, window, cx),
            _ => {}
        }
    }

    fn came(&mut self, trigger: Trigger, window: &mut Window, cx: &mut Context<Self>) {
        self.over = Some(trigger.id.clone());
        if self.pressed.as_ref() == Some(&trigger.id) {
            return;
        }
        let now = cx.background_executor().now();
        let lately = self.went.is_some_and(|went| now.saturating_duration_since(went) < NEXT_WITHIN);
        if lately || !matches!(self.shown, Shown::Nothing) {
            // reading along a row: the next one is there at once
            self.wait = None;
            self.put(trigger, false, cx);
            return;
        }
        let id = trigger.id.clone();
        self.wait = Some(cx.spawn_in(window, async move |tips, cx| {
            cx.background_executor().timer(SHOW_AFTER).await;
            let _ = tips.update(cx, |tips, cx| {
                if tips.over.as_ref() == Some(&id) && trigger.alive.upgrade().is_some() {
                    tips.put(trigger, true, cx);
                }
            });
        }));
    }

    fn put(&mut self, trigger: Trigger, eased: bool, cx: &mut Context<Self>) {
        self.turn += 1;
        self.shown = Shown::There { trigger, eased };
        cx.notify();
    }

    fn left(&mut self, id: &GlobalElementId, window: &mut Window, cx: &mut Context<Self>) {
        self.over = None;
        self.wait = None;
        if self.pressed.as_ref() == Some(id) {
            self.pressed = None;
        }
        let Shown::There { trigger, .. } = &self.shown else { return };
        if &trigger.id != id {
            return;
        }
        let trigger = trigger.clone();
        self.went = Some(cx.background_executor().now());
        self.turn += 1;
        let turn = self.turn;
        let gone = cx.spawn_in(window, async move |tips, cx| {
            cx.background_executor().timer(LEAVE).await;
            let _ = tips.update(cx, |tips, cx| {
                if tips.turn == turn {
                    tips.shown = Shown::Nothing;
                    cx.notify();
                }
            });
        });
        self.shown = Shown::Leaving { trigger, _gone: gone };
        cx.notify();
    }

    /// Away at once: a press, a key, the wheel, or what it belonged to is gone.
    fn away(&mut self, cx: &mut Context<Self>) {
        self.wait = None;
        self.went = None;
        if !matches!(self.shown, Shown::Nothing) {
            self.shown = Shown::Nothing;
            cx.notify();
        }
    }

    fn pressed(&mut self, id: GlobalElementId, cx: &mut Context<Self>) {
        self.pressed = Some(id);
        self.away(cx);
    }

    /// The trigger whose tooltip is waited for or shown.
    fn subject(&self) -> Option<&Trigger> {
        match &self.shown {
            Shown::There { trigger, .. } | Shown::Leaving { trigger, .. } => Some(trigger),
            Shown::Nothing => None,
        }
    }
}

/// How a tooltip looks as it comes, stays and goes.
#[derive(Clone, Copy, PartialEq)]
enum Look {
    In,
    There,
    Out,
}

impl Render for Tips {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let (trigger, look) = match &self.shown {
            Shown::Nothing => return div().into_any_element(),
            Shown::There { trigger, eased: true } => (trigger, Look::In),
            Shown::There { trigger, eased: false } => (trigger, Look::There),
            Shown::Leaving { trigger, .. } => (trigger, Look::Out),
        };
        let key = trigger
            .action
            .as_ref()
            .and_then(|action| Kbd::binding_for_action(action.as_ref().as_ref(), trigger.context, window));
        let words = trigger.words.clone();
        let body = div()
            .id("tip")
            .test_support()
            .role(Role::Tooltip)
            .aria_label(words.clone())
            .flex()
            .items_center()
            .gap(px(8.))
            .max_w(px(320.))
            .rounded(px(7.))
            .bg(TIP)
            .px(px(8.))
            .py(px(4.))
            .text_size(px(12.))
            .line_height(px(16.))
            .text_color(INK)
            .shadow(tip_shadow())
            .child(words)
            .children(key.map(|key| {
                div()
                    .flex_none()
                    .rounded(px(4.))
                    .bg(white(0.07))
                    .px(px(4.))
                    .text_size(px(11.))
                    .text_color(INK_2)
                    .child(key.appearance(false))
            }));
        // it eases in out of what it belongs to: from above when it hangs under it
        let under = trigger.bounds.bottom() + px(48.) < window.viewport_size().height;
        let from = if under { -RISE } else { RISE };
        let turn = self.turn;
        let moved: AnyElement = match look {
            Look::There => div().child(body).into_any_element(),
            Look::In => div()
                .relative()
                .child(body)
                .with_animation(
                    SharedString::from(format!("tip-in-{turn}")),
                    Animation::new(DUR_QUICK).with_easing(|t| bezier(EASE_OUT, t)),
                    move |tip, t| tip.top(px(from * (1. - t))).opacity(t),
                )
                .into_any_element(),
            Look::Out => div()
                .child(body)
                .with_animation(SharedString::from(format!("tip-out-{turn}")), Animation::new(LEAVE), |tip, t| {
                    tip.opacity(1. - t)
                })
                .into_any_element(),
        };
        let placed = Positioner::side(trigger.bounds)
            .placement(if under { Placement::Bottom } else { Placement::Top })
            .offset(px(AWAY))
            .margin(px(8.))
            .child(moved);
        deferred(placed).with_priority(ABOVE_ALL).into_any_element()
    }
}

/// A hairline of light around it and a soft shadow under it.
fn tip_shadow() -> Vec<BoxShadow> {
    let shadow = |color, y: f32, blur: f32, spread: f32| BoxShadow {
        color: hsla(color),
        offset: point(px(0.), px(y)),
        blur_radius: px(blur),
        spread_radius: px(spread),
        inset: false,
    };
    vec![
        shadow(white(0.085), 0., 0., 1.),
        shadow(alpha(BLACK, 0.35), 2., 6., 0.),
        shadow(alpha(BLACK, 0.6), 12., 32., -6.),
    ]
}

/// The window's tooltip, for the window's root to draw as its last child. It also sends a tooltip away when
/// something is typed, the wheel turns, or what it belonged to is no longer drawn.
pub fn layer(window: &mut Window, cx: &mut App) -> impl IntoElement + use<> {
    let tips = Tips::of(window, cx);
    let (watch, drawn) = (tips.clone(), tips.clone());
    div().absolute().size_0().child(drawn).child(
        canvas(
            |_, _, _| {},
            move |_, _, window, cx| {
                // what it belonged to left the screen: it goes too. Asked once this frame's elements are settled
                let alive = watch.read(cx).subject().map(|trigger| trigger.alive.clone());
                if alive.is_some_and(|alive| alive.upgrade().is_none()) {
                    let gone = watch.clone();
                    window.defer(cx, move |_, cx| gone.update(cx, |tips, cx| tips.away(cx)));
                }

                let turned = watch.clone();
                window.on_mouse_event(move |_: &ScrollEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture {
                        turned.update(cx, |tips, cx| tips.away(cx));
                    }
                });
                // each move of the pointer is heard by what is under it, the innermost first
                let moved = watch.clone();
                window.on_mouse_event(move |_: &MouseMoveEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture {
                        moved.update(cx, |tips, _| tips.claimed = false);
                    }
                });
            },
        )
        .absolute()
        .size_0(),
    )
}

/// Something that can have a tooltip: anything that holds children.
pub trait Tipped: ParentElement + Sized {
    /// Its tooltip: `words`, and the key of `action` beside them when it has one. A test finds it as "tip" and reads
    /// its label.
    fn tipped(self, words: impl Into<SharedString>, action: Option<Box<dyn Action>>) -> Self {
        tip_on(self, words.into(), action, None)
    }

    /// The same for an action whose key is bound in the key context `context` only (the opened file's Escape,
    /// Search's toggles): the key is looked up there, wherever the keyboard is.
    fn tipped_in(self, words: impl Into<SharedString>, action: Box<dyn Action>, context: &'static str) -> Self {
        tip_on(self, words.into(), Some(action), Some(context))
    }
}

impl<T: ParentElement> Tipped for T {}

fn tip_on<E: ParentElement>(
    element: E,
    words: SharedString,
    action: Option<Box<dyn Action>>,
    context: Option<&'static str>,
) -> E {
    {
        let action = action.map(Rc::new);
        element.child(
            canvas(
                |bounds, window, _| {
                    let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
                    // kept while it is drawn frame after frame, dropped after the first frame without it
                    let (id, alive) = window.with_global_id("tip-of".into(), |id, window| {
                        let alive = window.with_element_state(id, |kept: Option<Rc<()>>, _| {
                            let kept = kept.unwrap_or_default();
                            (Rc::downgrade(&kept), kept)
                        });
                        (id.clone(), alive)
                    });
                    (hitbox, id, alive)
                },
                move |bounds, (hitbox, id, alive), window, cx| {
                    let tips = Tips::of(window, cx);
                    let trigger = Trigger { id: id.clone(), bounds, words, action, context, alive };
                    window.on_mouse_event({
                        let (tips, hitbox) = (tips.clone(), hitbox.clone());
                        move |_: &MouseMoveEvent, phase, window, cx| {
                            if phase == DispatchPhase::Bubble {
                                // while something is dragged (a workspace's tab along its row) no tooltip comes:
                                // the pointer is busy, and a tip would stand over what it carries
                                let on_it = hitbox.is_hovered(window) && !cx.has_active_drag();
                                tips.update(cx, |tips, cx| tips.pointer(&trigger, on_it, window, cx));
                            }
                        }
                    });
                    window.on_mouse_event(move |_: &MouseDownEvent, phase, window, cx| {
                        if phase == DispatchPhase::Bubble && hitbox.is_hovered(window) {
                            tips.update(cx, |tips, cx| tips.pressed(id.clone(), cx));
                        }
                    });
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        )
    }
}
