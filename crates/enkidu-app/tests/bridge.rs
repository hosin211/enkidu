//! Item 1.9: a call from the window into the core runs off the drawing thread, and its result comes back on it.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::thread::ThreadId;
use std::time::{Duration, Instant};

use enkidu_app::bridge::Bridge;
use gpui_kit::TestAppContext;

#[gpui_kit::test]
fn a_call_into_the_core_runs_off_the_drawing_thread_and_comes_back_on_it(cx: &mut TestAppContext) {
    let data = tempfile::tempdir().unwrap();
    let shared = Arc::new(enkidu_core::state::Shared::new(data.path().to_path_buf(), Instant::now()).unwrap());
    let drawing = std::thread::current().id();
    let got: Rc<RefCell<Option<(ThreadId, usize, ThreadId)>>> = Rc::default();
    {
        let got = got.clone();
        cx.update(|cx| {
            Bridge::install(cx, shared, enkidu_core::events::Events::none());
            Bridge::call(
                cx,
                |shared| (std::thread::current().id(), shared.db.projects().unwrap().len()),
                move |(worker, projects), _| *got.borrow_mut() = Some((worker, projects, std::thread::current().id())),
            );
        });
    }
    // the work runs on a real thread of its own: let the test's executor wait for it
    cx.executor().allow_parking();
    for _ in 0..300 {
        cx.run_until_parked();
        if got.borrow().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let (worker, projects, done_on) = got.borrow().expect("the result came back");
    assert_ne!(worker, drawing, "the work ran on the drawing thread");
    assert_eq!(done_on, drawing, "the result came back on another thread");
    assert_eq!(projects, 0, "a new data folder has no projects");
}
