//! The bridge between the window and the core (item 1.9).
//!
//! The drawing thread draws, takes input and applies state; it never touches a file, git, the database or a process.
//! So every call from the window into enkidu-core goes through [`Bridge::call`]: the work runs on a thread of its own and
//! only its result comes back to the drawing thread. In debug builds, [`watch_the_drawing_thread`] logs any moment
//! that thread is busy for more than 50 ms.

use std::sync::Arc;
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use enkidu_core::events::Events;
use enkidu_core::state::Shared;
use gpui_kit::{App, AsyncApp, Global};

/// The running core, as the window reaches it: its state, and the sink its news goes to (calls that report progress,
/// like making a workspace, send it there, so the stores hear it).
pub struct Bridge {
    shared: Arc<Shared>,
    events: Events,
}

impl Global for Bridge {}

impl Bridge {
    /// Makes the core reachable from every view of the app. `events` is where the core's news goes.
    pub fn install(cx: &mut App, shared: Arc<Shared>, events: Events) {
        cx.set_global(Bridge { shared, events });
    }

    /// The core and where its news goes, for what must hold them (a terminal's writer thread). Never call into the
    /// core with them on the drawing thread.
    pub fn core(cx: &App) -> (Arc<Shared>, Events) {
        let bridge = cx.global::<Bridge>();
        (bridge.shared.clone(), bridge.events.clone())
    }

    /// Runs `work` with the core on a thread of its own, never the drawing thread, then `done` with what it gave,
    /// back on the drawing thread. If the app is gone by then, `done` doesn't run.
    pub fn call<T: Send + 'static>(
        cx: &mut App,
        work: impl FnOnce(&Arc<Shared>) -> T + Send + 'static,
        done: impl FnOnce(T, &mut App) + 'static,
    ) {
        let answer = Self::ask(cx, work);
        cx.spawn(async move |cx: &mut AsyncApp| {
            if let Ok(out) = answer.await {
                cx.update(|cx| done(out, cx));
            }
        })
        .detach();
    }

    /// The same as [`Bridge::ask`], for a call that reports its progress as the core's news.
    pub fn ask_with_news<T: Send + 'static>(
        cx: &App,
        work: impl FnOnce(&Events, &Arc<Shared>) -> T + Send + 'static,
    ) -> futures::channel::oneshot::Receiver<T> {
        let events = cx.global::<Bridge>().events.clone();
        Self::ask(cx, move |shared| work(&events, shared))
    }

    /// Runs `work` with the core on a thread of its own and hands back its answer to await, for a task that does
    /// more than one thing in order.
    pub fn ask<T: Send + 'static>(
        cx: &App,
        work: impl FnOnce(&Arc<Shared>) -> T + Send + 'static,
    ) -> futures::channel::oneshot::Receiver<T> {
        let shared = cx.global::<Bridge>().shared.clone();
        let (tx, rx) = futures::channel::oneshot::channel();
        std::thread::Builder::new()
            .name("core-call".into())
            .spawn(move || {
                let _ = tx.send(work(&shared));
            })
            .expect("a thread for a call into the core");
        rx
    }
}

/// How long the drawing thread may be busy before a debug build says so.
pub const BUSY: Duration = Duration::from_millis(50);
/// How often the watch asks the drawing thread for a sign.
const EVERY: Duration = Duration::from_millis(20);
/// While the drawing thread still hasn't answered, the watch says so this often.
const STILL: Duration = Duration::from_secs(1);

/// Debug builds only: logs to stderr each moment the drawing thread was busy for more than [`BUSY`].
pub fn watch_the_drawing_thread(cx: &App) {
    if !cfg!(debug_assertions) {
        return;
    }
    watch_app(cx, |busy, still| {
        let words = if still { "is still busy, for" } else { "was busy for" };
        eprintln!("Enkidu: the drawing thread {words} {} ms", busy.as_millis());
    });
}

/// [`watch`] on this app's drawing thread: its signs go through GPUI's own queue for that thread.
pub fn watch_app(cx: &App, said: impl Fn(Duration, bool) + Send + 'static) {
    let main = cx.foreground_executor().to_send();
    watch(move |sign| main.spawn(async move { sign() }).detach(), said);
}

/// The watch, apart from GPUI so a test can give it a stand-in drawing thread. `post` must run the closure it gets on
/// the drawing thread. `said` hears each stretch over [`BUSY`] once it ends (`still` false), and every [`STILL`]
/// while one goes on (`still` true). The watch ends when `post` drops a closure without running it (the app quit).
pub fn watch(post: impl Fn(Box<dyn FnOnce() + Send>) + Send + 'static, said: impl Fn(Duration, bool) + Send + 'static) {
    std::thread::Builder::new()
        .name("drawing-thread-watch".into())
        .spawn(move || {
            loop {
                let (tx, rx) = channel();
                let asked = Instant::now();
                post(Box::new(move || {
                    let _ = tx.send(Instant::now());
                }));
                let answered = loop {
                    match rx.recv_timeout(STILL) {
                        Ok(at) => break at,
                        Err(RecvTimeoutError::Timeout) => said(asked.elapsed(), true),
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                };
                let busy = answered.saturating_duration_since(asked);
                if busy > BUSY {
                    said(busy, false);
                }
                std::thread::sleep(EVERY);
            }
        })
        .expect("a thread for the drawing thread's watch");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::mpsc::Sender;

    type Job = Box<dyn FnOnce() + Send>;

    /// A stand-in drawing thread: it runs what it is given, in order.
    fn drawing_thread() -> Sender<Job> {
        let (tx, rx) = channel::<Job>();
        std::thread::spawn(move || {
            for job in rx {
                job();
            }
        });
        tx
    }

    #[test]
    fn a_busy_drawing_thread_is_said_and_a_free_one_is_not() {
        let main = Arc::new(Mutex::new(drawing_thread()));
        let heard = Arc::new(Mutex::new(Vec::new()));
        // each sign the watch asked for: when it was put in the drawing thread's line
        let asked = Arc::new(Mutex::new(Vec::<Instant>::new()));
        {
            let (main, heard, asked) = (main.clone(), heard.clone(), asked.clone());
            watch(
                move |job| {
                    // put in the line and written down in one step, so the gate below knows which sign stands behind it
                    let main = main.lock().unwrap();
                    let _ = main.send(job);
                    asked.lock().unwrap().push(Instant::now());
                },
                move |busy, still| heard.lock().unwrap().push((busy, still)),
            );
        }
        // free for a while: a few signs come back at once, nothing is said
        std::thread::sleep(Duration::from_millis(150));
        assert!(heard.lock().unwrap().is_empty(), "{:?}", heard.lock().unwrap());

        // Then busy: the drawing thread waits at a gate, and the gate stays shut for 120 ms from the moment the watch's
        // next sign stands behind it. So what the watch must say is known whatever else the Mac is doing. (A sleep of
        // 200 ms in the drawing thread's place was not: the watch can only count from its own question, and with the
        // whole check running that question once went out 119 ms into the stretch, so it said 81 ms.)
        let (open, gate) = channel::<()>();
        let (before, last_back) = {
            let main = main.lock().unwrap();
            main.send(Box::new(move || {
                let _ = gate.recv();
            }))
            .unwrap();
            let asked = asked.lock().unwrap();
            (asked.len(), *asked.last().expect("the watch has asked by now"))
        };
        let until = |what: &dyn Fn() -> bool| {
            let since = Instant::now();
            while !what() {
                assert!(since.elapsed() < Duration::from_secs(10), "the watch stopped");
                std::thread::sleep(Duration::from_millis(2));
            }
        };
        until(&|| asked.lock().unwrap().len() > before);
        let shut = Duration::from_millis(120);
        std::thread::sleep(shut);
        open.send(()).unwrap();
        until(&|| !heard.lock().unwrap().is_empty());
        let at_most = last_back.elapsed();
        let said = heard.lock().unwrap().clone();
        assert_eq!(said.len(), 1, "{said:?}");
        let (busy, still) = said[0];
        assert!(!still);
        // at least as long as the gate stayed shut behind its sign, and no longer than since the last sign that came back
        assert!(busy >= shut && busy <= at_most, "{busy:?}, at most {at_most:?}");
    }
}
