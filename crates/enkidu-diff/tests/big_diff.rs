//! item 3.4: a 700-file diff stays smooth. A merge can put 722 files (+24,213 −5,642) into the changes at once. This
//! times that size, with the real text system and no window: giving the view the files, their arrival three at a time
//! (as the app reads them), scrolling through them, a jump, and one file changing while another is read far down the
//! list.
//!
//! It times real work, so it means something only in a release build, alone: `sh scripts/check.sh` runs it there.
//!
//!     cargo test --release -p enkidu-diff --test big_diff

use std::sync::Arc;
use std::time::{Duration, Instant};

use enkidu_diff::{Diff, DiffFile, DiffLinePosition, DiffMode, DiffSide, DiffState};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Context, Entity, HeadlessAppContext, InputEvent as _, IntoElement,
    ParentElement as _, Render, ScrollDelta, ScrollEvent, Styled as _, Window, div, point, px, size,
};

const FILES: usize = 700;
/// Lines in each file before the change.
const LINES: usize = 150;

/// The budgets, loose as guards should be. A frame at 120 Hz has 8.3 ms and at 60 Hz 16.7 ms; the drawing
/// thread counts as busy from 50 ms (the bridge's watch).
const NEW_LIST: Duration = Duration::from_millis(50);
const ARRIVAL_STEP: Duration = Duration::from_millis(50);
const SCROLL_MIDDLE: Duration = Duration::from_millis(8);
const SCROLL_WORST: Duration = Duration::from_millis(33);
const JUMP: Duration = Duration::from_millis(50);
const ONE_FILE_CHANGED: Duration = Duration::from_millis(50);
/// A file that comes into the list or goes from it, far above the lines being read (item 3.16: before the rows after
/// it were kept, this took 30 ms with 420 files).
const A_FILE_JOINS: Duration = Duration::from_millis(16);

fn path(ix: usize) -> String {
    format!("src/module_{:02}/file_{ix:03}.py", ix % 23)
}

/// One file's diff with the whole file in it, as the app reads it: three places changed, 35 lines added and 8
/// removed (700 of them make +24,500 −5,600). `edit` changes one more line, for a file that changed again.
fn patch(ix: usize, edit: bool) -> String {
    let mut body = String::new();
    let (mut before, mut after) = (0, 0);
    for n in 1..=LINES {
        let line = format!("    value_{n} = compute(item_{ix}, weight={n}, scale=0.{n:03})  # step {n} of the file");
        match n {
            20 | 21 | 22 | 60 | 61 | 110 | 111 | 112 => {
                body.push_str(&format!("-{line}\n"));
                before += 1;
                if n == 22 || n == 61 || n == 112 {
                    // each removed block is replaced by a longer one
                    let added = if n == 22 { 15 } else { 10 };
                    for k in 1..=added {
                        body.push_str(&format!(
                            "+    value_{n}_{k} = compute(item_{ix}, weight={k}, scale=1.{k:03})\n"
                        ));
                        after += 1;
                    }
                }
            }
            90 if edit => {
                body.push_str(&format!("-{line}\n+{line} (edited)\n"));
                before += 1;
                after += 1;
            }
            _ => {
                body.push_str(&format!(" {line}\n"));
                before += 1;
                after += 1;
            }
        }
    }
    let path = path(ix);
    format!("diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -1,{before} +1,{after} @@\n{body}")
}

fn parse(ix: usize, edit: bool) -> DiffFile {
    DiffFile::parse(&patch(ix, edit)).expect("the patch parses").remove(0)
}

struct Host {
    state: Entity<DiffState>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        // as the app shows it: lines wrapped, 20 px rows
        div().size_full().child(Diff::new(&self.state).soft_wrap(true).row_height(px(20.)).size_full())
    }
}

/// How long `work` and the frame after it take on the drawing thread.
fn timed(
    cx: &mut HeadlessAppContext,
    window: AnyWindowHandle,
    work: impl FnOnce(&mut Window, &mut gpui_kit::App),
) -> Duration {
    let started = Instant::now();
    cx.update_window(window, |_, window, cx| {
        work(window, cx);
        window.render_frame(cx);
    })
    .expect("the window");
    started.elapsed()
}

fn ms(duration: Duration) -> String {
    format!("{:.2} ms", duration.as_secs_f64() * 1e3)
}

/// A view on its own state in a window of the app's first size, drawn offscreen.
fn open(cx: &mut HeadlessAppContext) -> (Entity<DiffState>, AnyWindowHandle) {
    let state = cx.update(|cx| cx.new(|cx| DiffState::new([], cx).with_side_names(true).with_mode(DiffMode::Split)));
    let window: AnyWindowHandle = cx
        .open_window(size(px(1360.), px(860.)), {
            let state = state.clone();
            move |_, cx| cx.new(|_| Host { state })
        })
        .expect("a window drawn offscreen")
        .into();
    for _ in 0..3 {
        timed(cx, window, |_, _| {});
    }
    (state, window)
}

/// 400 swipes of 300 px, each with its frame: the middle frame and the worst.
fn scroll(cx: &mut HeadlessAppContext, window: AnyWindowHandle) -> (Duration, Duration) {
    let mut frames = Vec::new();
    for _ in 0..400 {
        // the wheel event itself, then one frame (the test helper `scroll` would draw three)
        frames.push(timed(cx, window, |window, cx| {
            let swipe = ScrollEvent {
                position: point(px(680.), px(430.)),
                delta: ScrollDelta::Pixels(point(px(0.), px(-300.))),
                ..Default::default()
            };
            window.dispatch_event(swipe.to_platform_input(), cx);
        }));
    }
    frames.sort();
    (frames[frames.len() / 2], frames[frames.len() - 1])
}

fn top(cx: &mut HeadlessAppContext, state: &Entity<DiffState>) -> String {
    cx.update(|cx| state.read(cx).top_path().map(|path| path.to_string())).unwrap_or_default()
}

/// One run of it all. What it says is what missed.
fn measure(cx: &mut HeadlessAppContext) -> Vec<String> {
    // reading and parsing is off the drawing thread in the app: timed to know it, with no budget
    let started = Instant::now();
    let files: Vec<DiffFile> = (0..FILES).map(|ix| parse(ix, false)).collect();
    let parsing = started.elapsed();
    let (added, removed) = files.iter().fold((0, 0), |(a, d), file| (a + file.additions(), d + file.deletions()));

    // 1. the files arrive three at a time, the rest standing in without lines until they are read
    let (state, window) = open(cx);
    let stand_ins: Vec<DiffFile> = (0..FILES).map(|ix| DiffFile::without_lines(path(ix))).collect();
    let mut steps = Vec::new();
    for read in (0..=FILES).step_by(3).chain([FILES]) {
        let shown: Vec<DiffFile> =
            (0..FILES).map(|ix| if ix < read { files[ix].clone() } else { stand_ins[ix].clone() }).collect();
        let state = state.clone();
        steps.push(timed(cx, window, move |_, cx| state.update(cx, |state, cx| state.set_files(shown, cx))));
    }
    steps.sort();
    let (arrival_middle, arrival_worst, arrival_all) =
        (steps[steps.len() / 2], steps[steps.len() - 1], steps.iter().sum::<Duration>());

    // the word marks are prepared in the background (run here, in turn)
    let started = Instant::now();
    cx.run_until_parked();
    let preparing = started.elapsed();
    timed(cx, window, |_, _| {});

    // 2. every file at once, as when a workspace with its diffs kept comes back on screen
    let fresh = cx.update(|cx| cx.new(|cx| DiffState::new([], cx).with_side_names(true).with_mode(DiffMode::Split)));
    let all = files.clone();
    let started = Instant::now();
    cx.update(|cx| fresh.update(cx, |state, cx| state.set_files(all, cx)));
    let new_list = started.elapsed();

    // 3. scrolling through them; and through seven files of the same kind, to see what the other 693 cost a frame
    let (scroll_middle, scroll_worst) = scroll(cx, window);
    let reached = top(cx, &state);
    let (few, few_window) = open(cx);
    let seven: Vec<DiffFile> = files[..7].to_vec();
    cx.update(|cx| few.update(cx, |state, cx| state.set_files(seven, cx)));
    cx.run_until_parked();
    let (few_middle, _) = scroll(cx, few_window);

    // 4. a jump far down the list, to a line there
    let far = path(650);
    let jump = {
        let (state, far) = (state.clone(), far.clone());
        timed(cx, window, move |_, cx| {
            state.update(cx, |state, cx| state.scroll_to_line(DiffLinePosition::new(far, DiffSide::Modified, 70), cx))
        })
    };
    let at = top(cx, &state);

    // 5. one file changes while you read another far down: the list is given again with that one file new
    let mut changed = files.clone();
    changed[3] = parse(3, true);
    let with_one_changed = changed.clone();
    let one_changed = {
        let state = state.clone();
        timed(cx, window, move |_, cx| state.update(cx, |state, cx| state.set_files(changed, cx)))
    };
    let still_at = top(cx, &state);

    // 6. a new file comes first in the list while you read far down (every file after it moves one place), and
    // goes again
    let first = "diff --git a/a-new.md b/a-new.md\nnew file mode 100644\n--- /dev/null\n+++ b/a-new.md\n@@ -0,0 +1,3 @@\n+one\n+two\n+three\n";
    let mut joined = DiffFile::parse(first).expect("the patch parses");
    joined.extend(with_one_changed.iter().cloned());
    let a_file_joined = {
        let state = state.clone();
        timed(cx, window, move |_, cx| state.update(cx, |state, cx| state.set_files(joined, cx)))
    };
    let after_joining = top(cx, &state);
    let a_file_left = {
        let state = state.clone();
        timed(cx, window, move |_, cx| state.update(cx, |state, cx| state.set_files(with_one_changed, cx)))
    };
    let after_leaving = top(cx, &state);

    println!(
        "big_diff: {FILES} files, +{added} \u{2212}{removed}; parsed in {} (off the drawing thread in the app)",
        ms(parsing)
    );
    println!(
        "big_diff: arriving three at a time, {} steps: {} in the middle, {} at worst, {} in all; word marks prepared \
         in {} (in the background)",
        steps.len(),
        ms(arrival_middle),
        ms(arrival_worst),
        ms(arrival_all),
        ms(preparing)
    );
    println!("big_diff: all {FILES} given at once: {}", ms(new_list));
    println!(
        "big_diff: scrolling, 400 frames: {} in the middle, {} at worst (reached {reached}); with 7 files {} in the \
         middle",
        ms(scroll_middle),
        ms(scroll_worst),
        ms(few_middle)
    );
    println!("big_diff: a jump to {far}: {}; one file changed meanwhile: {}", ms(jump), ms(one_changed));
    println!(
        "big_diff: a new file came first in the list meanwhile: {}; it went again: {}",
        ms(a_file_joined),
        ms(a_file_left)
    );

    let mut missed = Vec::new();
    let mut check = |what: &str, took: Duration, budget: Duration| {
        if took > budget {
            missed.push(format!("{what} took {}, the budget is {}", ms(took), ms(budget)));
        }
    };
    check("a step of the arrival", arrival_worst, ARRIVAL_STEP);
    check("the whole list given at once", new_list, NEW_LIST);
    check("a frame of scrolling (the middle one)", scroll_middle, SCROLL_MIDDLE);
    check("a frame of scrolling (the worst)", scroll_worst, SCROLL_WORST);
    check("the jump", jump, JUMP);
    check("one file changing", one_changed, ONE_FILE_CHANGED);
    check("a file coming first in the list", a_file_joined, A_FILE_JOINS);
    check("a file going from the list", a_file_left, A_FILE_JOINS);
    if (added, removed) != (FILES * 35, FILES * 8) {
        missed.push(format!("the diff isn't the size it should be: +{added} \u{2212}{removed}"));
    }
    if at != far || still_at != far || after_joining != far || after_leaving != far {
        missed.push(format!(
            "the view isn't where it was sent: at {at:?}, then {still_at:?}, {after_joining:?} and {after_leaving:?}, not {far:?}"
        ));
    }
    if reached == path(0) {
        missed.push("scrolling went nowhere".to_string());
    }
    missed
}

fn main() {
    if cfg!(debug_assertions) {
        println!("big_diff: a timed guard, it runs in a release build (sh scripts/check.sh)");
        return;
    }
    let mut cx = HeadlessAppContext::with_platform(
        gpui_kit::platform::current_platform(true).text_system(),
        Arc::new(gpui_kit::assets::Assets),
        gpui_kit::platform::current_headless_renderer,
    );
    cx.update(|cx| {
        gpui_kit::init(cx);
        enkidu_diff::init(cx);
    });
    // The Mac is often busy with other work, which only ever makes these numbers worse: one run inside every
    // budget shows the code can do it; three runs outside fail the guard (as the terminal's flood guard does).
    let mut missed = Vec::new();
    for _ in 0..3 {
        missed = measure(&mut cx);
        if missed.is_empty() {
            return;
        }
    }
    for miss in &missed {
        eprintln!("big_diff: MISSED: {miss}");
    }
    std::process::exit(1);
}
