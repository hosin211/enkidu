use enkidu_app::measure::{self, Plan};
use enkidu_app::shell::Shell;
use enkidu_app::window_state;
use gpui_kit::{AppContext as _, WindowOptions};

fn main() {
    let _ = measure::STARTED.set(std::time::Instant::now());
    let plan = Plan::from_env();
    gpui_kit::application().with_assets(enkidu_ui::assets::Assets).run(move |cx| {
        enkidu_app::init(cx);
        // debug builds say when the drawing thread is busy for more than 50 ms (item 1.9)
        enkidu_app::bridge::watch_the_drawing_thread(cx);
        cx.on_window_closed(|cx, _| cx.quit()).detach();
        if let Some(plan) = plan {
            // A measurement: the window opens in the background, without the keyboard, and the app quits by itself.
            measure::run(plan, cx);
            return;
        }
        let options = enkidu_app::window_options(cx, true);
        // `ENKIDU_VIEW=design`: the gallery of every token and piece (item 1.12)
        if std::env::var("ENKIDU_VIEW").is_ok_and(|v| v == "design") {
            if gpui_kit::open_window(options, cx, |_, cx| cx.new(|_| enkidu_ui::gallery::Gallery::default())).is_err() {
                cx.quit();
            }
            cx.activate(true);
            return;
        }
        // the core and the stores on it: what the database remembers, then the core's news (item 1.15)
        let (frame, failed) = match enkidu_app::stores::start(cx) {
            Ok(frame) => (frame, None),
            Err(err) => {
                eprintln!("Enkidu: the core could not start: {err:#}");
                (cx.new(|_| enkidu_model::frame::Frame::default()), Some(format!("Enkidu could not start: {err:#}")))
            }
        };
        let core_started = failed.is_none();
        // the window opens where and how big it was left (item 1.14)
        let options = WindowOptions { window_bounds: Some(window_state::restore(cx)), ..options };
        let opened = gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| {
                if let Some(path) = window_state::path() {
                    window_state::remember(path, window, cx);
                }
                // each workspace's terminals are the core's; without a core there is nothing to run
                if core_started {
                    Shell::live(frame.clone(), window, cx)
                } else {
                    Shell::kept(frame.clone(), window, cx)
                }
            })
        });
        // a core that could not start is said in the window, not only in a log
        if let (Ok((handle, _)), Some(message)) = (&opened, failed) {
            let _ = handle.update(cx, |_, window, cx| enkidu_app::toast::say(window, message, cx));
        }
        if let Err(err) = opened {
            eprintln!("Enkidu: the window could not open: {err:#}");
            cx.quit();
            return;
        }
        cx.activate(true);
    });
}
