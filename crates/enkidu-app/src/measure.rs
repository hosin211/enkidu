//! Measurements the tracker names (items 0.17 and 0.18), made by the real app in a real window.
//!
//! They run only when `ENKIDU_MEASURE` names one. The window then opens in the background without taking the
//! keyboard, and the app quits by itself, within two minutes at most.
//! `scripts/measure.sh` starts them and reads CPU and memory from outside the app.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use enkidu_term::{Spawn, Terminal};
use gpui_kit::{App, AppContext as _, AsyncApp, Keystroke};

use crate::demo::Tab;
use crate::shell::Shell;
use crate::terminal_view::spawn_view;
use enkidu_ui::lamp::Status;

/// When `main` began, for the time to the first frame.
pub static STARTED: OnceLock<Instant> = OnceLock::new();
static FIRST_FRAME: OnceLock<Duration> = OnceLock::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plan {
    /// Nothing moves on screen; `terminals` login shells run (one on screen).
    Rest { terminals: usize },
    /// One lamp turns on screen, nothing else moves.
    Lamp,
    /// One terminal prints 100 MB while the one beside it is typed into.
    Flood,
    /// The window as you open it: where AppKit puts its close, minimise and zoom buttons (items 0.5b and 1.14).
    Frame,
}

impl Plan {
    pub fn from_env() -> Option<Plan> {
        match std::env::var("ENKIDU_MEASURE").ok()?.as_str() {
            "rest1" => Some(Plan::Rest { terminals: 1 }),
            "rest5" => Some(Plan::Rest { terminals: 5 }),
            "lamp" => Some(Plan::Lamp),
            "flood" => Some(Plan::Flood),
            "frame" => Some(Plan::Frame),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Plan::Rest { terminals: 1 } => "rest1",
            Plan::Rest { .. } => "rest5",
            Plan::Lamp => "lamp",
            Plan::Flood => "flood",
            Plan::Frame => "frame",
        }
    }
}

/// How long a calm plan keeps the window open: `ENKIDU_SECONDS`, 40 by default, never more than 110.
fn seconds() -> u64 {
    std::env::var("ENKIDU_SECONDS").ok().and_then(|s| s.parse().ok()).unwrap_or(40).min(110)
}

fn out_path(plan: Plan) -> PathBuf {
    std::env::var_os("ENKIDU_MEASURE_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("measure/out/{}.json", plan.name())))
}

/// The paints of one terminal's grid while a flood runs: when each ended, and how long its own work took.
struct Paints {
    terminal: usize,
    at: Vec<(Instant, Duration)>,
}

static PAINTS: Mutex<Option<Paints>> = Mutex::new(None);

/// Every frame the window drew (the root view rendered) and every paint of any terminal's grid: the calm plans report
/// them per second, to see how often a lamp redraws the window and whether the cached terminal stays still.
pub static FRAMES: AtomicU64 = AtomicU64::new(0);
/// How many times the top row was drawn again (item 1.18b: with the frame in cached views, a turning lamp redraws
/// only the row with its tab, and the status line), and likewise the rail and the status line.
pub static TAB_ROWS: AtomicU64 = AtomicU64::new(0);
pub static RAILS: AtomicU64 = AtomicU64::new(0);
pub static STATUS_LINES: AtomicU64 = AtomicU64::new(0);
static GRID_PAINTS: AtomicU64 = AtomicU64::new(0);

/// Called by the terminal grid after each paint.
pub fn painted(terminal: &Arc<Terminal>, work: Duration) {
    GRID_PAINTS.fetch_add(1, Ordering::Relaxed);
    if let Some(started) = STARTED.get() {
        FIRST_FRAME.get_or_init(|| started.elapsed());
    }
    let mut paints = PAINTS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(paints) = paints.as_mut()
        && paints.terminal == Arc::as_ptr(terminal) as usize
    {
        paints.at.push((Instant::now(), work));
    }
}

fn tabs(lamp_turns: bool) -> Vec<Tab> {
    let first = if lamp_turns { Status::Working } else { Status::Idle };
    vec![
        Tab { name: "stock-fix".into(), status: first, note: Some("5".into()) },
        Tab { name: "sh-21309".into(), status: Status::Done, note: None },
        Tab { name: "pr-2175".into(), status: Status::Done, note: None },
    ]
}

fn login_shell() -> Spawn {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| "/".into());
    Spawn { program: None, cwd: home, env: enkidu_term::base_env() }
}

/// Opens the plan's window and runs it. The app quits when the plan ends.
pub fn run(plan: Plan, cx: &mut App) {
    let mut options = crate::window_options(cx, false);
    // A measurement window is never in front, and GPUI draws such a window at about 30 frames a second. Lift that
    // here only, so it draws as the window you work in does (up to the display's 120). ENKIDU_KEEP_CAP=1 keeps it, to
    // measure the window while you work in another app.
    if std::env::var("ENKIDU_KEEP_CAP").map_or(true, |v| v.is_empty() || v == "0") {
        options.inactive_frame_interval = None;
    }
    let opened = match plan {
        Plan::Rest { terminals } => open_calm(options, tabs(false), 0, terminals, cx),
        // Tab 1 is selected, so the status line's lamp is still: only tab 0's lamp turns.
        Plan::Lamp => open_calm(options, tabs(true), 1, 1, cx),
        Plan::Flood => open_flood(options, cx),
        Plan::Frame => open_frame(options, cx),
    };
    if let Err(err) = opened {
        eprintln!("measure {}: {err:#}", plan.name());
        cx.quit();
    }
}

fn open_calm(
    options: gpui_kit::WindowOptions,
    tabs: Vec<Tab>,
    selected: usize,
    terminals: usize,
    cx: &mut App,
) -> anyhow::Result<()> {
    let views = (0..terminals).map(|_| spawn_view(login_shell(), cx)).collect::<anyhow::Result<Vec<_>>>()?;
    let shown = views[0].read(cx).terminal().clone();
    let (handle, _) = gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| Shell::new(tabs, selected, views, false, window, cx))
    })?;
    let plan_seconds = seconds();
    let plan = if terminals == 1 && selected == 1 { Plan::Lamp } else { Plan::Rest { terminals } };
    cx.spawn(async move |cx: &mut AsyncApp| {
        // settle 5 s, then count the frames and grid paints over the rest of the plan
        let settle = plan_seconds.min(5);
        cx.background_executor().timer(Duration::from_secs(settle)).await;
        let (frames, paints, rows, from) = (
            FRAMES.load(Ordering::Relaxed),
            GRID_PAINTS.load(Ordering::Relaxed),
            TAB_ROWS.load(Ordering::Relaxed),
            Instant::now(),
        );
        // a window covered by others (or minimised, or on another Space) isn't drawn at all: the share of the time it
        // was visible says whether the numbers count
        let (mut looks, mut seen) = (0u32, 0u32);
        while from.elapsed() < Duration::from_secs(plan_seconds - settle) {
            cx.background_executor().timer(Duration::from_millis(500)).await;
            looks += 1;
            let visible = cx.update_window(handle, |_, window, _| window.visibility());
            if matches!(visible, Ok(gpui_kit::WindowVisibility::Visible)) {
                seen += 1;
            }
        }
        let visible_share = f64::from(seen) / f64::from(looks.max(1));
        let span = from.elapsed().as_secs_f64().max(0.001);
        let frames_per_s = (FRAMES.load(Ordering::Relaxed) - frames) as f64 / span;
        let paints_per_s = (GRID_PAINTS.load(Ordering::Relaxed) - paints) as f64 / span;
        let rows_per_s = (TAB_ROWS.load(Ordering::Relaxed) - rows) as f64 / span;
        let first = FIRST_FRAME.get().map_or(-1.0, |d| d.as_secs_f64() * 1000.0);
        let size = shown.size();
        write(
            plan,
            &format!(
                "{{\"plan\":\"{}\",\"pid\":{},\"first_frame_ms\":{first:.1},\"terminal\":{{\"cols\":{},\"rows\":{},\"shell_output_bytes\":{},\"shell_exited\":{}}},\"frames_per_s\":{frames_per_s:.1},\"grid_paints_per_s\":{paints_per_s:.1},\"top_row_draws_per_s\":{rows_per_s:.1},\"visible_share\":{visible_share:.2}}}",
                plan.name(),
                std::process::id(),
                size.cols,
                size.rows,
                shown.parsed(),
                shown.has_exited()
            ),
        );
        cx.update(|cx| cx.quit());
    })
    .detach();
    Ok(())
}

/// The flood: 100 MB of colored lines on the left; on the right, `cat`, typed into every 400 ms.
fn open_flood(options: gpui_kit::WindowOptions, cx: &mut App) -> anyhow::Result<()> {
    let script = r#"read go; perl -e '$l = "\e[32m" . ("0123456789abcdef" x 4) . "\e[0m \e[1;31mERR\e[0m line\n"; $n = int(100_000_000 / length($l) / 1000); for (1..$n) { print $l x 1000 }'; echo; echo FLOOD-DONE; read x"#;
    let left =
        spawn_view(Spawn { program: Some(("/bin/sh".into(), vec!["-c".into(), script.into()])), ..login_shell() }, cx)?;
    let right = spawn_view(Spawn { program: Some(("/bin/cat".into(), vec![])), ..login_shell() }, cx)?;
    let flood = left.read(cx).terminal().clone();
    let typed = right.read(cx).terminal().clone();
    let (window, _) = gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| Shell::new(tabs(false), 0, vec![right.clone(), left.clone()], true, window, cx))
    })?;
    cx.spawn(async move |cx: &mut AsyncApp| {
        let report = flood_run(&flood, &typed, window, cx).await;
        write(Plan::Flood, &report);
        cx.update(|cx| cx.quit());
    })
    .detach();
    Ok(())
}

async fn flood_run(
    flood: &Arc<Terminal>,
    typed: &Arc<Terminal>,
    window: gpui_kit::AnyWindowHandle,
    cx: &mut AsyncApp,
) -> String {
    let executor = cx.background_executor().clone();
    let timer = |ms: u64| executor.timer(Duration::from_millis(ms));
    timer(3000).await;
    *PAINTS.lock().unwrap_or_else(|e| e.into_inner()) =
        Some(Paints { terminal: Arc::as_ptr(flood) as usize, at: Vec::new() });
    let parsed_before = flood.parsed();
    let started = Instant::now();
    flood.send(b"\r".to_vec());

    let mut latencies: Vec<f64> = Vec::new();
    let mut missed = 0;
    let mut n = 0;
    let deadline = started + Duration::from_secs(95);
    let mut done_at = None;
    while Instant::now() < deadline {
        if flood.screen().text().contains("FLOOD-DONE") {
            // The flood ended with the parser's last chunk, not when this loop happened to look.
            done_at =
                Some(flood.last_output().map_or_else(|| started.elapsed(), |t| t.saturating_duration_since(started)));
            break;
        }
        // Type "pingN" and Enter into the right terminal, as key presses through the window.
        n += 1;
        let word = format!("ping{n}");
        let typed_at = Instant::now();
        let key = |key: String, text: Option<String>| Keystroke { modifiers: Default::default(), key, key_char: text };
        let keys: Vec<Keystroke> =
            word.chars().map(|c| key(c.to_string(), Some(c.to_string()))).chain([key("enter".into(), None)]).collect();
        // Through the window itself, as the platform delivers a key press (never through the root view, which is
        // being drawn).
        let _ = cx.update_window(window, move |_, window, cx| {
            for keystroke in keys {
                window.dispatch_keystroke(keystroke, cx);
            }
        });
        let mut seen = false;
        while typed_at.elapsed() < Duration::from_secs(2) {
            if typed.screen().text().contains(&word) {
                latencies.push(typed_at.elapsed().as_secs_f64() * 1000.0);
                seen = true;
                break;
            }
            timer(2).await;
        }
        if !seen {
            missed += 1;
        }
        timer(30).await;
    }
    let paints = PAINTS.lock().unwrap_or_else(|e| e.into_inner()).take().map(|p| p.at).unwrap_or_default();
    let seconds = done_at.unwrap_or_else(|| started.elapsed()).as_secs_f64();
    let bytes = flood.parsed() - parsed_before;
    let intervals: Vec<f64> = paints.windows(2).map(|w| (w[1].0 - w[0].0).as_secs_f64() * 1000.0).collect();
    let work: Vec<f64> = paints.iter().map(|p| p.1.as_secs_f64() * 1000.0).collect();
    format!(
        "{{\"plan\":\"flood\",\"finished\":{},\"seconds\":{seconds:.2},\"bytes\":{bytes},\"mb_per_s\":{:.1},\"frames\":{},\"fps\":{:.1},\"frame_interval_ms\":{},\"grid_draw_ms\":{},\"typing_latency_ms\":{},\"typed\":{n},\"typing_missed\":{missed}}}",
        done_at.is_some(),
        bytes as f64 / 1_000_000.0 / seconds,
        paints.len(),
        paints.len() as f64 / seconds,
        stats(&intervals),
        stats(&work),
        stats(&latencies),
    )
}

/// p50, p95, p99 and the largest, as JSON.
fn stats(values: &[f64]) -> String {
    if values.is_empty() {
        return "null".into();
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let at = |q: f64| sorted[((sorted.len() as f64 - 1.0) * q).round() as usize];
    format!(
        "{{\"count\":{},\"p50\":{:.2},\"p95\":{:.2},\"p99\":{:.2},\"max\":{:.2}}}",
        sorted.len(),
        at(0.5),
        at(0.95),
        at(0.99),
        sorted[sorted.len() - 1]
    )
}

fn write(plan: Plan, json: &str) {
    let path = out_path(plan);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(err) = std::fs::write(&path, format!("{json}\n")) {
        eprintln!("measure: can't write {}: {err}", path.display());
    }
    println!("{json}");
}

/// The window as you open it, then where its own buttons are, in points from its top left corner.
fn open_frame(options: gpui_kit::WindowOptions, cx: &mut App) -> anyhow::Result<()> {
    // where `window.json` in the data folder says, as the app opens its window
    let options = gpui_kit::WindowOptions { window_bounds: Some(crate::window_state::restore(cx)), ..options };
    let (window, _) = gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Shell::demo(window, cx)))?;
    cx.spawn(async move |cx: &mut AsyncApp| {
        cx.background_executor().timer(Duration::from_secs(1)).await;
        let found = cx.update_window(window, |_, window, _| traffic_lights(window)).ok().flatten();
        let json = match found {
            Some(buttons) => format!("{{\"plan\":\"frame\",{buttons}}}"),
            None => "{\"plan\":\"frame\",\"error\":\"AppKit gave no window buttons\"}".to_string(),
        };
        write(Plan::Frame, &json);
        cx.update(|cx| cx.quit());
    })
    .detach();
    Ok(())
}

/// The close, minimise and zoom buttons' frames as AppKit has them, measured from the window's top left, and the
/// window's own size: `"close":{"x":..,"top":..,"w":..,"h":..},...`.
fn traffic_lights(window: &gpui_kit::Window) -> Option<String> {
    use objc2_app_kit::{NSView, NSWindowButton};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    // the trait's, not GPUI's own `window_handle` (its window id)
    let handle = HasWindowHandle::window_handle(window).ok()?;
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else { return None };
    // SAFETY: GPUI's window hands out its own live NSView, on the main thread, where this runs.
    let view: &NSView = unsafe { appkit.ns_view.cast().as_ref() };
    let ns_window = view.window()?;
    let frame = ns_window.frame();
    // where GPUI has it, from the top left of the display
    let at = window.bounds();
    let mut parts = vec![format!(
        "\"window\":{{\"x\":{:.1},\"y\":{:.1},\"w\":{:.1},\"h\":{:.1}}}",
        f32::from(at.origin.x),
        f32::from(at.origin.y),
        frame.size.width,
        frame.size.height
    )];
    for (name, which) in [
        ("close", NSWindowButton::CloseButton),
        ("minimize", NSWindowButton::MiniaturizeButton),
        ("zoom", NSWindowButton::ZoomButton),
    ] {
        let button = ns_window.standardWindowButton(which)?;
        // in the window's own coordinates, which count up from its bottom
        let at = button.convertRect_toView(button.bounds(), None);
        let top = frame.size.height - (at.origin.y + at.size.height);
        parts.push(format!(
            "\"{name}\":{{\"x\":{:.1},\"top\":{:.1},\"w\":{:.1},\"h\":{:.1}}}",
            at.origin.x, top, at.size.width, at.size.height
        ));
    }
    Some(parts.join(","))
}
