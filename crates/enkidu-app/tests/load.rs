//! Item 3.16: two huge workspaces at once. One throwaway
//! repo of 40,000 files, two workspaces of it, and in each 420 changed files (300 edited, 100 new, 20 deleted), as
//! after a long session of Claude's. The whole app runs on it: the real core, its watcher on both folders, the
//! stores and the window. It times what you would feel: the changes arriving, a switch between the two, more edits
//! one at a time, an edit in the workspace that isn't on screen, scrolling the changes, the Files panel, Search
//! (item 4.1), and opening the app again on all of it.
//!
//! It makes 120,000 files, so it doesn't run with every check: `sh scripts/load.sh` runs it in a release build.
//!
//! Every frame is timed twice: by the clock, and by how long its thread had a processor (its own work). On a calm
//! machine the two are the same. This test often runs beside other work (a load of 10 on 10 cores), where a
//! frame can wait a tenth of a second for a processor; the bounds are on the work, and both numbers are printed.

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::{App, said};
use enkidu_app::shell::Shell;
use enkidu_testkit::git;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Entity, InputEvent as _, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ScrollDelta, TestAppContext, Window, point, px,
};

/// 200 folders of 200 files; fewer with `ENKIDU_LOAD_FOLDERS` (at least 20), for a quick look at one number (the
/// Search part then finds nothing to time and the run ends there).
fn folders() -> usize {
    std::env::var("ENKIDU_LOAD_FOLDERS").ok().and_then(|n| n.parse().ok()).unwrap_or(200)
}
const FILES_EACH: usize = 200;
/// What Claude's long session leaves in a workspace.
const EDITED: usize = 300;
const NEW: usize = 100;
const DELETED: usize = 20;
const CHANGED: usize = EDITED + NEW + DELETED;

fn source(folder: usize, file: usize) -> String {
    format!(
        "\"\"\"Module {folder}, file {file}.\"\"\"\n\n\nclass Thing{folder}x{file}:\n    def total(self, items):\n        \
         return sum(items)\n\n    def count(self, items):\n        return len(items)\n\n\ndef \
         helper_{folder}_{file}(value):\n    return value + {file}\n"
    )
}

/// A throwaway repository of 40,000 Python files in 200 folders, committed on `main`.
fn huge(parent: &Path) -> PathBuf {
    let dir = parent.join("huge");
    for folder in 0..folders() {
        let at = dir.join(format!("src/module_{folder:03}"));
        std::fs::create_dir_all(&at).unwrap();
        for file in 0..FILES_EACH {
            std::fs::write(at.join(format!("file_{file:03}.py")), source(folder, file)).unwrap();
        }
    }
    git(&dir, &["init", "-q", "-b", "main"]);
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "40,000 files"]);
    PathBuf::from(git(&dir, &["rev-parse", "--show-toplevel"]))
}

/// A long session's work in a workspace: 300 files edited, 100 new ones, 20 deleted.
fn work(folder: &Path, mark: &str) {
    for n in 0..EDITED {
        let path = folder.join(format!("src/module_{:03}/file_{:03}.py", n % folders(), n / folders()));
        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push_str(&format!("\n\ndef added_by_{mark}_{n}(value):\n    return value * {n}\n"));
        std::fs::write(path, text).unwrap();
    }
    for n in 0..NEW {
        let lines: String = (0..30).map(|line| format!("{mark}_{n}_{line} = {line}\n")).collect();
        std::fs::write(folder.join(format!("src/module_{:03}/new_{mark}_{n:03}.py", n % folders())), lines).unwrap();
    }
    for n in 0..DELETED {
        std::fs::remove_file(folder.join(format!("src/module_{n:03}/file_199.py"))).unwrap();
    }
}

fn workspace(cx: &mut TestAppContext, app: &App, name: &str) -> (String, PathBuf) {
    app.frame
        .read_with(cx, |f, _| f.workspaces.iter().find(|w| w.name == name).map(|w| (w.id.clone(), w.path.clone())))
        .map(|(id, path)| (id, PathBuf::from(path)))
        .unwrap_or_else(|| panic!("no workspace {name}"))
}

/// How long this thread has had a processor, in milliseconds.
fn worked() -> f64 {
    let mut time = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: the call fills the struct it is given and reads nothing else.
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut time) };
    time.tv_sec as f64 * 1000. + time.tv_nsec as f64 / 1_000_000.
}

/// One frame: how long it took by the clock, and how much of that was its own work, in milliseconds.
#[derive(Clone, Copy, Default, PartialEq)]
struct Frame {
    took: f64,
    work: f64,
}

/// The frames that matter of many: the one that took longest, and the most work any of them did.
#[derive(Clone, Copy, Default)]
struct Longest {
    frame: Frame,
    most_work: f64,
}

impl Longest {
    fn with(self, frame: Frame) -> Longest {
        Longest {
            frame: if frame.took > self.frame.took { frame } else { self.frame },
            most_work: self.most_work.max(frame.work),
        }
    }

    fn max(self, other: Longest) -> Longest {
        Longest { most_work: self.most_work.max(other.most_work), ..self.with(other.frame) }
    }
}

impl std::fmt::Display for Longest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:.1} ms", self.frame.took)?;
        // said only when the clock and the work differ: the frame waited for the machine
        if self.frame.took - self.frame.work > 3. {
            write!(
                f,
                " ({:.1} ms of it its own work; the most work in a frame {:.1} ms)",
                self.frame.work, self.most_work
            )?;
        }
        Ok(())
    }
}

/// Draws a frame.
fn draw(w: &mut Window, cx: &mut gpui_kit::App) -> Frame {
    let (started, before) = (Instant::now(), worked());
    w.render_frame(cx);
    Frame { took: started.elapsed().as_secs_f64() * 1000., work: worked() - before }
}

/// Lets what arrived be applied, draws a frame and looks at it.
fn frame<R>(cx: &mut TestAppContext, app: &App, at: impl FnOnce(&Window) -> R) -> (R, Frame) {
    cx.run_until_parked();
    cx.update_window(app.window, |_, w, cx| {
        let took = draw(w, cx);
        (at(w), took)
    })
    .unwrap()
}

/// Draws frames until `seen` holds: how long that took, and the longest frame on the way (the window stays in use
/// while the news arrives, so no frame may take long).
fn until(cx: &mut TestAppContext, app: &App, what: &str, seen: impl Fn(&Window) -> bool) -> (Duration, Longest) {
    let started = Instant::now();
    let mut longest = Longest::default();
    loop {
        let (there, took) = frame(cx, app, &seen);
        longest = longest.with(took);
        if there {
            return (started.elapsed(), longest);
        }
        assert!(started.elapsed() < Duration::from_secs(120), "{what}: not after two minutes");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// How often the window is drawn while nothing happens, in `LOOKS` updates of it, 100 ms of the clock apart (four
/// seconds). A test window is drawn after an update only when something asked for a frame, so this counts what
/// asks: a cursor that blinks may, twice a second; anything else would cost a processor while you only read.
fn drawn_at_rest(cx: &mut TestAppContext, window: gpui_kit::AnyWindowHandle, shell: &Entity<Shell>) -> u64 {
    let drawn = |cx: &mut TestAppContext| shell.read_with(cx, |shell, _| shell.drawn());
    // what is still on its way ends first: motion, by the clock this test holds
    for _ in 0..3 {
        cx.executor().advance_clock(Duration::from_millis(2_500));
        cx.run_until_parked();
        cx.update_window(window, |_, w, cx| w.render_frame(cx)).unwrap();
    }
    // and what the core still reads on its own threads, in real time: at rest means no frame for a second and a half
    let (started, mut quiet, mut last) = (Instant::now(), Instant::now(), drawn(cx));
    while quiet.elapsed() < Duration::from_millis(1_500) {
        assert!(started.elapsed() < Duration::from_secs(120), "the window never came to rest: it draws by itself");
        std::thread::sleep(Duration::from_millis(20));
        cx.run_until_parked();
        cx.update_window(window, |_, _, _| {}).unwrap();
        let now = drawn(cx);
        if now != last {
            (last, quiet) = (now, Instant::now());
        }
    }
    let before = drawn(cx);
    for _ in 0..LOOKS {
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        cx.update_window(window, |_, _, _| {}).unwrap();
    }
    drawn(cx) - before
}

/// The looks of [`drawn_at_rest`], and how many frames a cursor's blink may ask for in them (it blinks about twice a
/// second; something that drew by itself would draw with every look).
const LOOKS: usize = 40;
const BLINKS: u64 = 12;

fn ms(time: Duration) -> f64 {
    time.as_secs_f64() * 1000.
}

/// This process's memory in MB, as `ps` says it.
fn memory() -> f64 {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .expect("ps runs");
    String::from_utf8_lossy(&out.stdout).trim().parse::<f64>().unwrap_or(0.) / 1024.
}

/// What the process holds allocated and not freed, in MB: the bytes in use in every malloc zone. What `ps` says
/// moves with the Mac: it counts memory that was freed and not yet given back, and leaves out what the system
/// packed away while the Mac was busy (it read 60 MB and 98 MB in two runs for the same 62 MB in use). This
/// doesn't move. (A picture's pixels on the GPU are not in it: a test window has none.)
fn heap() -> f64 {
    let mut stats =
        libc::malloc_statistics_t { blocks_in_use: 0, size_in_use: 0, max_size_in_use: 0, size_allocated: 0 };
    // SAFETY: a null zone asks for all zones, and the call only fills the struct it is given.
    unsafe { libc::malloc_zone_statistics(std::ptr::null_mut(), &mut stats) };
    stats.size_in_use as f64 / (1024. * 1024.)
}

/// A switch to another workspace: how long by the clock until it was on screen with its changes counted, and the
/// window's own work for it (the click, which draws five frames in a test, and the frames after). The clock also
/// counts what the test's thread ran meanwhile for the background (diffs still being read, which the app reads on
/// other threads), so it says more about the moment than about the switch: the bound is on the work.
struct Switched {
    took: Duration,
    work: f64,
}

/// Clicks a tab and draws until that workspace is on screen with its changes counted.
fn switch(cx: &mut TestAppContext, app: &App, tab: usize, name: &str, count: &str) -> Switched {
    frame(cx, app, |_| ());
    let trace = std::env::var("ENKIDU_LOAD_TRACE").is_ok();
    let clicked = Instant::now();
    let before = worked();
    cx.update_window(app.window, |_, w, cx| w.click(("tab", tab), cx)).unwrap();
    let mut work = worked() - before;
    if trace {
        println!("LOAD   switch: the click itself {:.1} ms ({work:.1} ms work)", ms(clicked.elapsed()));
    }
    loop {
        let (started, before) = (Instant::now(), worked());
        let (listed, drawn) = frame(cx, app, |w| {
            w.try_find("island-claude").is_some() && said(w, "changes-count").as_deref() == Some(count)
        });
        if trace {
            println!(
                "LOAD   switch: a round {:.1} ms ({:.1} ms work), its frame {:.1} ms ({:.1} ms work), listed {listed}",
                ms(started.elapsed()),
                worked() - before,
                drawn.took,
                drawn.work
            );
        }
        work += drawn.work;
        let on = app.frame.read_with(cx, |f, _| f.current_workspace().is_some_and(|w| w.name == name));
        if listed && on {
            return Switched { took: clicked.elapsed(), work };
        }
        assert!(clicked.elapsed() < Duration::from_secs(30), "{name} never came on screen with {count}");
    }
}

/// Whether a line that says `text` is on screen in the diffs.
fn has_line(window: &Window, text: &str) -> bool {
    window.find_all("source").iter().any(|line| line.label() == Some(text))
}

/// Scrolls the changes by 900 px a step: a frame in the middle, and the longest.
fn scrolling(cx: &mut TestAppContext, app: &App, steps: usize) -> (f64, Longest) {
    scrolling_in(cx, app, "diff-view", 900., steps)
}

/// Scrolls `what` by `by` px a step: a frame in the middle, and the longest.
fn scrolling_in(cx: &mut TestAppContext, app: &App, what: &'static str, by: f32, steps: usize) -> (f64, Longest) {
    let mut frames: Vec<Frame> = Vec::new();
    for _ in 0..steps {
        cx.run_until_parked();
        cx.update_window(app.window, |_, w, cx| {
            w.scroll(what, ScrollDelta::Pixels(point(px(0.), px(-by))), cx);
            frames.push(draw(w, cx));
        })
        .unwrap();
    }
    let longest = frames.iter().fold(Longest::default(), |longest, frame| longest.with(*frame));
    frames.sort_by(|a, b| a.took.partial_cmp(&b.took).unwrap());
    (frames[frames.len() / 2].took, longest)
}

#[cfg_attr(debug_assertions, ignore = "a load test on 120,000 files: `sh scripts/load.sh` runs it in a release build")]
#[gpui_kit::test]
fn two_huge_workspaces_stay_quick(cx: &mut TestAppContext) {
    // `ENKIDU_LOAD_TY` names the real `ty` (`scripts/load.sh` names the one the app carries): the ⌘ lines below
    // need a language server, since nothing else says where a name is defined (phase 13). Without it they are
    // left out, and the run says so
    let ty = std::env::var("ENKIDU_LOAD_TY").ok();
    if let Some(ty) = &ty {
        // SAFETY: the tests of this file run one after the other, and only this one asks about Python.
        unsafe { std::env::set_var("ENKIDU_PYTHON_SERVER", format!("exec '{ty}' server")) };
    }
    let begun = Instant::now();
    let app = common::start_on(cx, huge, |shared, project, _| {
        App::make(shared, project, "one", "new", "one", Some("main"));
        App::make(shared, project, "two", "new", "two", Some("main"));
    });
    println!(
        "LOAD the repo of 40,000 files, its two workspaces and the app on them: {:.1} s",
        begun.elapsed().as_secs_f64()
    );
    app.click(cx, ("tab", 0usize));
    assert!(app.until_frame(cx, |f| f.current_workspace().is_some_and(|w| w.name == "one")));
    let (_, one) = workspace(cx, &app, "one");
    let (_, two) = workspace(cx, &app, "two");
    let (_, rest) = frame(cx, &app, |_| ());
    let (clean, clean_heap) = (memory(), heap());
    println!(
        "LOAD two clean workspaces: a frame {:.2} ms, the process {clean:.0} MB ({clean_heap:.0} MB of it in use)",
        rest.took
    );

    // Claude's long session in the workspace on screen: 420 changed files arrive
    let written = Instant::now();
    work(&one, "one");
    let writing = written.elapsed();
    let want = format!("{CHANGED} files");
    let (listed, longest_arrival) = until(cx, &app, "the changes of the workspace on screen", |w| {
        said(w, "changes-count").as_deref() == Some(want.as_str())
    });
    println!(
        "LOAD {CHANGED} changed files in the workspace on screen (written in {:.0} ms): listed {:.0} ms after the \
         last was written, the longest frame meanwhile {longest_arrival}",
        ms(writing),
        ms(listed)
    );
    let (first, longest_first) =
        until(cx, &app, "the first diff", |w| w.find_all("source").len() > 5 && !w.find_all("file-head").is_empty());
    println!("LOAD its first diff on screen {:.0} ms later, the longest frame {longest_first}", ms(first));

    // the same in the workspace that isn't on screen: its tab learns of it
    let written = Instant::now();
    work(&two, "two");
    let writing = written.elapsed();
    let words = format!("{CHANGED} changed files");
    let (hidden, longest_hidden) = until(cx, &app, "the changes of the workspace that isn't on screen", |w| {
        said(w, ("tab-changed", 1usize)).as_deref() == Some(words.as_str())
    });
    println!(
        "LOAD {CHANGED} changed files in the workspace that isn't on screen (written in {:.0} ms): on its tab {:.0} \
         ms after the last was written, the longest frame meanwhile {longest_hidden}",
        ms(writing),
        ms(hidden)
    );

    // from one to the other and back, both full: the best of three
    let (mut there, mut back, mut switch_work) = (Duration::MAX, Duration::MAX, 0f64);
    for _ in 0..3 {
        let (to, from) = (switch(cx, &app, 1, "two", &want), switch(cx, &app, 0, "one", &want));
        (there, back) = (there.min(to.took), back.min(from.took));
        switch_work = switch_work.max(to.work).max(from.work);
    }
    println!(
        "LOAD a switch to the other workspace, its changes listed: {:.1} ms; back: {:.1} ms (the best of three, while \
         the diffs are still being read); the window's own work for a switch, at most {switch_work:.1} ms",
        ms(there),
        ms(back)
    );
    // for a look at where a switch's time goes (sample the test meanwhile): many more switches, when asked
    if let Some(more) = std::env::var("ENKIDU_LOAD_SWITCHES").ok().and_then(|n| n.parse::<usize>().ok()) {
        println!("LOAD   switching {more} times there and back");
        let started = Instant::now();
        for _ in 0..more {
            switch(cx, &app, 1, "two", &want);
            switch(cx, &app, 0, "one", &want);
        }
        println!("LOAD   a switch among {} more: {:.1} ms", 2 * more, ms(started.elapsed()) / (2 * more) as f64);
    }

    // scrolling through the changes while their diffs are still being read, then once they are
    let (middle, longest_reading) = scrolling(cx, &app, 60);
    println!(
        "LOAD scrolling the changes while they are read: a frame {middle:.2} ms in the middle, {longest_reading} at \
         worst"
    );
    // every diff read: the last file by path has its lines when it is scrolled to (an edited file in the last
    // folder; on a smaller repo, a new file there)
    let last_line = match folders() {
        200.. => "    return value * 199".to_string(),
        few => format!("one_{}_29 = 29", (99 - (few - 1)) / few * few + few - 1),
    };
    let read = Instant::now();
    loop {
        cx.run_until_parked();
        let at_end = cx
            .update_window(app.window, |_, w, cx| {
                w.scroll("diff-view", ScrollDelta::Pixels(point(px(0.), px(-200_000.))), cx);
                w.render_frame(cx);
                has_line(w, &last_line)
            })
            .unwrap();
        if at_end {
            break;
        }
        assert!(read.elapsed() < Duration::from_secs(120), "the last diffs were never read");
        std::thread::sleep(Duration::from_millis(20));
    }
    println!(
        "LOAD every diff of the workspace was read {:.1} s after the scrolling (0.0: they were read by then)",
        read.elapsed().as_secs_f64()
    );
    let (middle_read, longest_read) = scrolling(cx, &app, 60);
    println!("LOAD scrolling them once read: a frame {middle_read:.2} ms in the middle, {longest_read} at worst");

    // Claude goes on working in the workspace on screen, once all is calm: more edits, one at a time. Every other
    // one is to a file that is changed already (the list keeps its length), the rest are new files (it grows)
    for _ in 0..150 {
        frame(cx, &app, |_| ());
        std::thread::sleep(Duration::from_millis(10));
    }
    let rounds: usize = std::env::var("ENKIDU_LOAD_EDITS").ok().and_then(|n| n.parse().ok()).unwrap_or(10);
    let trace = std::env::var("ENKIDU_LOAD_TRACE").is_ok();
    let seen = |w: &Window| (said(w, "changes-count"), said(w, "changes-added"), said(w, "changes-removed"));
    let (mut edits, mut longest_edit): (Vec<f64>, Longest) = (Vec::new(), Longest::default());
    for n in 0..rounds {
        let (before, _) = frame(cx, &app, seen);
        let written = Instant::now();
        if n % 2 == 0 {
            let path = one.join("src/module_000/file_000.py");
            let mut text = std::fs::read_to_string(&path).unwrap();
            text.push_str(&format!("\n# edit {n}\n"));
            std::fs::write(path, text).unwrap();
        } else {
            std::fs::write(one.join(format!("more-{n:03}.md")), format!("# More {n}\n")).unwrap();
        }
        let (_, longest) = until(cx, &app, "one more edit", |w| seen(w) != before);
        edits.push(ms(written.elapsed()));
        longest_edit = longest_edit.max(longest);
        if trace {
            println!(
                "LOAD trace: edit {n} ({}) listed after {:.0} ms, the longest frame until then {longest}",
                if n % 2 == 0 { "an edit" } else { "a new file" },
                ms(written.elapsed())
            );
        }
        // its diff is read again, and the frames that follow are counted too
        for after in 0..30 {
            let (_, took) = frame(cx, &app, |_| ());
            longest_edit = longest_edit.with(took);
            if trace && took.took > 6. {
                println!(
                    "LOAD trace:   frame {after} after it was listed took {:.1} ms ({:.1} ms of work)",
                    took.took, took.work
                );
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    edits.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let (edit, edit_worst) = (edits[edits.len() / 2], edits[edits.len() - 1]);
    println!(
        "LOAD {rounds} more edits there, one at a time: listed after {edit:.0} ms in the middle, {edit_worst:.0} ms \
         at worst; the longest frame {longest_edit}"
    );

    // Changed files as a tree (item 29.6): the 420 files under their 201 folders. Every folder closed at once and
    // opened at once, then the tree scrolled to its end
    let first_file = |w: &Window| w.try_find(("change-row", 0usize)).is_some();
    assert!(frame(cx, &app, first_file).0, "the tree of changed files isn't on screen");
    let pressed = Instant::now();
    app.click(cx, "changed-collapse");
    let closed = format!("closed, {CHANGED} files");
    let (_, longest_collapse) = until(cx, &app, "Collapse all", |w| {
        w.try_find(("change-folder", 0usize)).is_some_and(|src| src.description() == Some(closed.as_str()))
    });
    let collapse = pressed.elapsed();
    let pressed = Instant::now();
    app.click(cx, "changed-expand");
    let (_, longest_expand) = until(cx, &app, "Expand all", first_file);
    let expand = pressed.elapsed();
    let (tree_frame, longest_tree) = scrolling_in(cx, &app, "changed-files", 600., 40);
    println!(
        "LOAD Changed files as a tree ({CHANGED} files under their folders): Collapse all {:.1} ms (the longest frame \
         {longest_collapse}), Expand all {:.1} ms (the longest frame {longest_expand}); scrolled to its end, a frame \
         {tree_frame:.2} ms, the longest {longest_tree}",
        ms(collapse),
        ms(expand)
    );
    let tree_work = longest_collapse.max(longest_expand).max(longest_tree).most_work;
    assert!(tree_work < 33., "a frame of the tree of changed files worked {tree_work:.1} ms");

    // the Files panel on 40,000 files, and its filter
    let opened = Instant::now();
    app.press(cx, "alt-cmd-2");
    let (_, longest_files) = until(cx, &app, "the Files panel", |w| {
        said(w, "files-count").is_some() && w.try_find(("folder-row", 0usize)).is_some()
    });
    let files = opened.elapsed();
    let count = frame(cx, &app, |w| said(w, "files-count")).0.unwrap_or_default();
    println!("LOAD the Files panel ({count}): open after {:.0} ms, the longest frame {longest_files}", ms(files));
    app.click(cx, "files-filter");
    let typed = Instant::now();
    app.typed(cx, "file_150");
    let (_, longest_filter) = until(cx, &app, "the filter", |w| {
        w.try_find(("file-row", 0usize))
            .is_some_and(|row| row.label().is_some_and(|path| path.ends_with("file_150.py")))
    });
    let filter = typed.elapsed();
    println!("LOAD its filter on a name: the matches after {:.0} ms, the longest frame {longest_filter}", ms(filter));
    let (full, full_heap) = (memory(), heap());
    println!(
        "LOAD both workspaces full and read: the process {full:.0} MB ({:.0} MB more than clean), {full_heap:.0} MB of \
         it in use ({:.0} MB more than clean)",
        full - clean,
        full_heap - clean_heap
    );

    // Search on the same workspace (item 4.1): its Files, its Symbols (the first question reads every definition),
    // and its Text (a scan of every file). It asks a moment after the last key, by the clock this test holds
    let first_row = |w: &Window| w.try_find(("search-row", 0usize)).and_then(|row| row.label().map(String::from));
    // Find in files lists one row for each line (item 10.4): its first row is a line
    let line_row = |w: &Window| w.try_find(("search-row", 0usize)).and_then(|row| row.label().map(String::from));
    let status = |w: &Window| said(w, "search-status").unwrap_or_default();
    let type_in = |cx: &mut TestAppContext, words: &str| {
        app.press(cx, "cmd-a");
        app.typed(cx, words);
        cx.executor().advance_clock(Duration::from_millis(250));
    };
    let opened = Instant::now();
    app.press(cx, "cmd-p");
    let (_, longest_open) = until(cx, &app, "Search's popup", |w| w.try_find("search-popup").is_some());
    let popup = opened.elapsed();
    let (_, longest_list) = until(cx, &app, "Search's list of files", |w| status(w).ends_with(" files"));
    let listed = opened.elapsed();
    println!(
        "LOAD Search (⌘P): the popup after {:.1} ms, {} after {:.0} ms; the longest frame {}",
        ms(popup),
        frame(cx, &app, status).0,
        ms(listed),
        longest_open.max(longest_list)
    );
    let typed = Instant::now();
    type_in(cx, "file_150");
    let (_, longest_name) =
        until(cx, &app, "a file's name", |w| first_row(w).is_some_and(|path| path.ends_with("/file_150.py")));
    println!(
        "LOAD Search, a name typed: the matches ({}) after {:.0} ms, the longest frame {longest_name}",
        frame(cx, &app, status).0,
        ms(typed.elapsed())
    );
    // Symbols
    app.press(cx, "tab");
    let asked = Instant::now();
    type_in(cx, "thing150x7");
    let (_, longest_symbols) =
        until(cx, &app, "the first definition", |w| first_row(w).is_some_and(|name| name.starts_with("Thing150x7")));
    let first_symbol = asked.elapsed();
    println!(
        "LOAD Search, Symbols, the first question (every definition is read): {:.0} ms ({}), the longest frame \
         {longest_symbols}",
        ms(first_symbol),
        frame(cx, &app, status).0
    );
    let mut later: Vec<f64> = Vec::new();
    let mut longest_symbol = Longest::default();
    for name in ["helper_19_19", "thing7x150", "helper_150_7", "thing199x1", "helper_3_33"] {
        let asked = Instant::now();
        type_in(cx, name);
        let want = name.to_lowercase();
        let (_, longest) = until(cx, &app, "a later definition", |w| {
            first_row(w).is_some_and(|found| found.to_lowercase().starts_with(&want))
        });
        later.push(ms(asked.elapsed()));
        longest_symbol = longest_symbol.max(longest);
    }
    later.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!(
        "LOAD Search, Symbols, a later question: {:.0} ms in the middle, {:.0} ms at worst, the longest frame \
         {longest_symbol}",
        later[later.len() / 2],
        later[later.len() - 1]
    );
    // Text: one line in 40,000 files, then a text that every file has
    app.press(cx, "tab");
    let asked = Instant::now();
    type_in(cx, "helper_150_7(");
    let (_, longest_one) = until(cx, &app, "the one line", |w| status(w).starts_with("1 line in 1 file"));
    println!(
        "LOAD Search, Text, one line among 40,000 files: {:.0} ms ({}), the longest frame {longest_one}",
        ms(asked.elapsed()),
        frame(cx, &app, status).0
    );
    let asked = Instant::now();
    type_in(cx, "return sum(items)");
    let (_, longest_rows) =
        until(cx, &app, "the first lines", |w| line_row(w).is_some_and(|line| line == "return sum(items)"));
    let first_lines = asked.elapsed();
    let (_, longest_all) = until(cx, &app, "the whole scan", |w| status(w).starts_with("First 1,000 lines"));
    println!(
        "LOAD Search, Text, a line every file has: the first lines after {:.0} ms, the scan done after {:.0} ms \
         ({}), the longest frame {}",
        ms(first_lines),
        ms(asked.elapsed()),
        frame(cx, &app, status).0,
        longest_rows.max(longest_all)
    );
    // moving through a thousand lines. The file under the rows (item 10.6) is read only for a row that was the picked
    // one for a moment: while ↓ is held it stays the file it was (the one line found before)
    let previewed = |w: &Window| w.try_find("search-preview").and_then(|under| under.label().map(String::from));
    let shown_before = frame(cx, &app, previewed).0;
    let mut moves: Vec<Frame> = Vec::new();
    for _ in 0..60 {
        app.press(cx, "down");
        moves.push(frame(cx, &app, |_| ()).1);
    }
    let longest_move = moves.iter().fold(Longest::default(), |longest, frame| longest.with(*frame));
    moves.sort_by(|a, b| a.took.partial_cmp(&b.took).unwrap());
    println!(
        "LOAD Search, Text, ↓ through the thousand lines: a frame {:.2} ms in the middle, {longest_move} at worst",
        moves[moves.len() / 2].took
    );
    // ↓ was held just now, so no file was read for the rows passed; after a rest on a row, that row's file is
    // there. Then ↓ with a rest on each of thirty rows, each one another file
    assert_eq!(frame(cx, &app, previewed).0, shown_before, "↓ held through the rows read a file for the preview");
    let rested = Instant::now();
    cx.executor().advance_clock(Duration::from_millis(100));
    let (_, longest_preview) =
        until(cx, &app, "the file under the rows", |w| previewed(w).is_some() && previewed(w) != shown_before);
    let first_preview = rested.elapsed();
    let mut follows: Vec<f64> = Vec::new();
    let mut longest_follow = Longest::default();
    for _ in 0..30 {
        let before = frame(cx, &app, previewed).0;
        let pressed = Instant::now();
        app.press(cx, "down");
        cx.executor().advance_clock(Duration::from_millis(100));
        let (_, longest) = until(cx, &app, "the next row's file", |w| previewed(w).is_some() && previewed(w) != before);
        follows.push(ms(pressed.elapsed()));
        longest_follow = longest_follow.max(longest);
    }
    follows.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!(
        "LOAD Search, Text, the file under the rows: there {:.0} ms after a rest on its row, the longest frame \
         {longest_preview}; ↓ with a rest on each of 30 rows, each another file: its file after {:.0} ms in the middle, \
         {:.0} ms at worst, the longest frame {longest_follow}",
        ms(first_preview),
        follows[follows.len() / 2],
        follows[follows.len() - 1]
    );
    let longest_search = [
        longest_open,
        longest_list,
        longest_name,
        longest_symbols,
        longest_symbol,
        longest_one,
        longest_rows,
        longest_all,
        longest_move,
        longest_preview,
        longest_follow,
    ]
    .into_iter()
    .fold(Longest::default(), Longest::max);
    app.press(cx, "escape");
    let (_, _) = until(cx, &app, "Search closed", |w| w.try_find("search-popup").is_none());

    // An opened file (items 10.3, 10.10 and 10.11): 5,002 lines that all differ from the last commit, so every row
    // carries its mark. It is scrolled; then ⌘ is held while the pointer crosses its code, and a ⌘-click goes to a
    // definition in another of the 40,000 files
    let big = "src/module_000/file_000.py";
    let body: String = (0..5_000)
        .map(|n| match n % 50 {
            7 => format!("    total_{n} = helper_150_7(total_{n})\n"),
            _ => format!("    total_{n} = {n} * 2  # was {n}\n"),
        })
        .collect();
    let (_, _, here) = app.on_screen(cx).expect("a workspace on screen");
    // (the file imports the name that is ⌘-clicked below: the code itself says where it is defined, so the click
    // goes straight there. A name only found by its name is listed, not gone to: item 10.17)
    let head = "from src.module_150.file_007 import helper_150_7\ndef big(total_0):\n";
    std::fs::write(Path::new(&here).join(big), format!("{head}{body}")).unwrap();
    let island = app.shell.read_with(cx, |shell, _| shell.changes().cloned()).expect("the island");
    let asked = Instant::now();
    cx.update(|cx| island.update(cx, |island, cx| island.open_file(big, Some(10), cx)));
    let (_, longest_text) =
        until(cx, &app, "the opened file", |w| said(w, "file-lines").as_deref() == Some("5,002 lines"));
    let text_after = asked.elapsed();
    let (_, longest_marks) = until(cx, &app, "its marks", |w| w.try_find(("change-mark", 10usize)).is_some());
    let marks_after = asked.elapsed();
    let mut frames: Vec<Frame> = Vec::new();
    for _ in 0..40 {
        cx.run_until_parked();
        cx.update_window(app.window, |_, w, cx| {
            w.scroll("file-view", ScrollDelta::Pixels(point(px(0.), px(-900.))), cx);
            frames.push(draw(w, cx));
        })
        .unwrap();
    }
    let longest_wheel = frames.iter().fold(Longest::default(), |longest, frame| longest.with(*frame));
    frames.sort_by(|a, b| a.took.partial_cmp(&b.took).unwrap());
    println!(
        "LOAD an opened file of 5,002 lines, every one changed: its lines after {:.0} ms, its marks after {:.0} ms, \
         the longest frame {}; scrolling it, a mark on every row: a frame {:.2} ms in the middle, {longest_wheel} at \
         worst",
        ms(text_after),
        ms(marks_after),
        longest_text.max(longest_marks),
        frames[frames.len() / 2].took
    );
    let (mut crossing, mut longest_link, mut longest_there) =
        (Longest::default(), Longest::default(), Longest::default());
    if ty.is_none() {
        println!("LOAD ⌘-click: not measured, no language server was named (ENKIDU_LOAD_TY)");
    }
    if ty.is_some() {
        // ⌘ held while the pointer crosses the code: each name it comes over is asked of the core, off this thread, and
        // the core asks the workspace's language server (started when the file came on screen)
        let cmd = Modifiers::command();
        let code = frame(cx, &app, |w| w.find("file-view").bounds()).0;
        for step in 0..40 {
            let part = step as f32 / 40.;
            let position = point(
                code.left() + px(90.) + px(500.) * part,
                code.top() + px(30.) + (code.size.height - px(60.)) * part,
            );
            cx.update_window(app.window, |_, w, cx| {
                w.dispatch_event(
                    MouseMoveEvent { position, pressed_button: None, modifiers: cmd }.to_platform_input(),
                    cx,
                );
                crossing = crossing.with(draw(w, cx));
            })
            .unwrap();
            crossing = crossing.with(frame(cx, &app, |_| ()).1);
        }
        println!("LOAD ⌘ held while the pointer crosses its code, 40 moves: the longest frame {crossing}");
        // a ⌘-click: the name on line 9 is defined in one of the 40,000 files
        cx.update(|cx| island.update(cx, |island, cx| island.open_file(big, Some(10), cx)));
        let calls = "    total_7 = helper_150_7(total_7)";
        let (_, _) = until(cx, &app, "line 9", |w| w.find_all("source").iter().any(|line| line.label() == Some(calls)));
        let row = frame(cx, &app, |w| {
            w.find_all("source").iter().find(|line| line.label() == Some(calls)).map(|line| line.bounds()).unwrap()
        })
        .0;
        let state = cx.update(|cx| island.read(cx).code_on_screen(cx)).expect("the code on screen");
        let on_name = cx.update(|cx| {
            let (state, y) = (state.read(cx), row.center().y);
            let mut x = row.left();
            while x < row.right() {
                if state.name_at(point(x, y)).is_some_and(|name| name.range.start == calls.find("helper").unwrap()) {
                    return point(x + px(1.), y);
                }
                x += px(1.);
            }
            panic!("helper_150_7 isn't on screen");
        });
        let asked = Instant::now();
        cx.update_window(app.window, |_, w, cx| {
            let hover = MouseMoveEvent { position: on_name, pressed_button: None, modifiers: cmd };
            w.dispatch_event(hover.to_platform_input(), cx);
        })
        .unwrap();
        loop {
            longest_link = longest_link.with(frame(cx, &app, |_| ()).1);
            if cx.update(|cx| state.read(cx).link().is_some()) {
                break;
            }
            assert!(asked.elapsed() < Duration::from_secs(60), "the name never became a link");
            std::thread::sleep(Duration::from_millis(1));
            // (while the server can't be asked yet the app asks again after a moment, by the clock this test holds)
            cx.executor().advance_clock(Duration::from_millis(20));
        }
        let linked = asked.elapsed();
        let clicked = Instant::now();
        cx.update_window(app.window, |_, w, cx| {
            let left = MouseButton::Left;
            let down =
                MouseDownEvent { button: left, position: on_name, modifiers: cmd, click_count: 1, first_mouse: false };
            w.dispatch_event(down.to_platform_input(), cx);
            let up = MouseUpEvent { button: left, position: on_name, modifiers: cmd, click_count: 1 };
            w.dispatch_event(up.to_platform_input(), cx);
        })
        .unwrap();
        let there = "src/module_150/file_007.py";
        let (_, to_there) = until(cx, &app, "the definition's file", |w| {
            said(w, "file-view").as_deref() == Some(there)
                && w.find_all("source").iter().any(|line| line.label() == Some("def helper_150_7(value):"))
        });
        longest_there = to_there;
        println!(
            "LOAD ⌘ over a name defined in another of the 40,000 files: a link after {:.1} ms; ⌘-click: its definition \
         on screen after {:.0} ms; the longest frame {}",
            ms(linked),
            ms(clicked.elapsed()),
            longest_link.max(longest_there)
        );
        {
            // the server that answered: what it holds after these questions
            let (id, _) = workspace(cx, &app, "one");
            let waited = Instant::now();
            while !app.shared.servers.ready(&id) {
                assert!(waited.elapsed() < Duration::from_secs(60), "the language server never was ready");
                std::thread::sleep(Duration::from_millis(5));
            }
            let held = app.shared.servers.program(&id).map(|pid| {
                let out = std::process::Command::new("ps").args(["-o", "rss=", "-p", &pid.to_string()]).output();
                out.ok().and_then(|out| String::from_utf8_lossy(&out.stdout).trim().parse::<f64>().ok()).unwrap_or(0.)
                    / 1024.
            });
            println!(
                "LOAD the workspace's language server (ty) holds {:.0} MB after these questions",
                held.unwrap_or(0.)
            );
        }
    }
    let longest_file = [longest_text, longest_marks, longest_wheel, crossing, longest_link, longest_there]
        .into_iter()
        .fold(Longest::default(), Longest::max);
    // back to the changes
    app.press(cx, "escape");
    let (_, _) = until(cx, &app, "the file closed", |w| w.try_find("file-view").is_none());

    // Settings over the same window (item 4.3), both workspaces full: the dialog, a project's page (its settings are
    // read from disk) and typing in a field
    let opened = Instant::now();
    app.press(cx, "cmd-,");
    let (_, longest_general) = until(cx, &app, "Settings", |w| w.try_find("workspaces-dir").is_some());
    let general = opened.elapsed();
    let asked = Instant::now();
    app.click(cx, ("settings-nav", 3usize));
    let (_, longest_page) = until(cx, &app, "a project's page", |w| w.try_find("default-base").is_some());
    let page = asked.elapsed();
    app.click(cx, "default-base");
    let mut keys: Vec<Frame> = Vec::new();
    for letter in "development".chars() {
        app.typed(cx, &letter.to_string());
        keys.push(frame(cx, &app, |_| ()).1);
    }
    let longest_settings = keys.iter().fold(longest_general.max(longest_page), |longest, frame| longest.with(*frame));
    keys.sort_by(|a, b| a.took.partial_cmp(&b.took).unwrap());
    println!(
        "LOAD Settings (⌘,): the General page after {:.1} ms, a project's page after {:.1} ms, a letter typed there \
         a frame {:.2} ms in the middle; the longest frame {longest_settings}",
        ms(general),
        ms(page),
        keys[keys.len() / 2].took
    );
    // Setup (item 21.5), in the same dialog: the page is there at once with every row being looked at, then each row
    // as its program answered (the fake gh and twg, this Mac's git, and no claude in this window). The programs run
    // off the drawing thread: the window has to stay quick meanwhile
    let _fakes = app.fakes();
    let asked = Instant::now();
    app.click(cx, "settings-nav-setup");
    let (_, longest_shown) = until(cx, &app, "Setup", |w| w.try_find("setup-page").is_some());
    let shown = asked.elapsed();
    let (_, longest_looked) = until(cx, &app, "Setup looked", |w| {
        w.try_find("setup-page").is_some_and(|page| page.description() == Some("looked"))
    });
    let looked = asked.elapsed();
    let setup_still = frame(cx, &app, |_| ()).1;
    let longest_setup = longest_shown.max(longest_looked).with(setup_still);
    println!(
        "LOAD Setup: the page {:.1} ms after the click, every program answered after {:.0} ms, a frame of it at rest \
         {:.2} ms; the longest frame {longest_setup}",
        ms(shown),
        ms(looked),
        setup_still.took
    );
    app.press(cx, "escape");
    let (_, _) = until(cx, &app, "Settings closed", |w| w.try_find("settings-dialog").is_none());

    // Live with the workspace on screen and all its changed files (item 4.5): the copy into the huge main folder, and
    // the main folder put back. The core copies off the drawing thread: the window has to stay quick meanwhile
    let live_is = |w: &Window, state: &str| w.try_find("live-button").is_some_and(|b| b.description() == Some(state));
    let asked = Instant::now();
    app.click(cx, "live-button");
    let (_, longest_on) = until(cx, &app, "Live", |w| live_is(w, "live"));
    let went_live = asked.elapsed();
    let asked = Instant::now();
    app.click(cx, "live-button");
    let (_, longest_off) = until(cx, &app, "Live stopped", |w| live_is(w, "off"));
    let longest_live = longest_on.max(longest_off);
    println!(
        "LOAD Live with its changed files ({}): Live after {:.0} ms, the main folder back {:.0} ms after Stop; the \
         longest frame {longest_live}",
        frame(cx, &app, |w| said(w, "changes-count")).0.unwrap_or_default(),
        ms(went_live),
        ms(asked.elapsed())
    );

    // the huge main folder itself opened as a workspace (item 19.5): the dialog's question, its other side, then the
    // main folder as a third tab. The core
    // takes the folder in off the drawing thread and git lists what is changed there: the window stays in use
    let was_on =
        app.frame.read_with(cx, |f, _| f.current_workspace().map(|w| w.id.clone())).expect("a workspace on screen");
    app.press(cx, "cmd-n");
    let (_, longest_asked) = until(cx, &app, "the New workspace dialog", |w| w.try_find("ws-where-main").is_some());
    let clicked = Instant::now();
    app.click(cx, "ws-where-main");
    let (_, longest_side) = until(cx, &app, "the main folder's side", |w| w.try_find("ws-main-says").is_some());
    let side = clicked.elapsed();
    let asked = Instant::now();
    app.press(cx, "enter");
    let (_, longest_opened) = until(cx, &app, "the main folder on screen", |w| {
        w.try_find("new-workspace-dialog").is_none() && w.try_find(("tab", 2usize)).is_some()
    });
    let opened = asked.elapsed();
    let main_id = app
        .frame
        .read_with(cx, |f, _| f.current_workspace().filter(|w| f.is_main_folder(w)).map(|w| w.id.clone()))
        .expect("the main folder is the workspace on screen");
    let (no_live, still_main) = frame(cx, &app, |w| w.try_find("live-button").is_none());
    assert!(no_live, "the main folder has no Go live");
    let longest_main = longest_asked.max(longest_side).max(longest_opened);
    println!(
        "LOAD the main folder of 40,000 files as a workspace: its side of the dialog {:.1} ms after the click, its \
         tab on screen {:.0} ms after Enter, a frame with it on screen {}; the longest frame {longest_main}",
        ms(side),
        ms(opened),
        Longest::default().with(still_main)
    );
    // forgotten again (the folder stays), so what follows is measured on the two workspaces, as before
    enkidu_core::commands::remove_workspace(app.sink.clone(), &app.shared, main_id.clone(), false)
        .expect("the main folder is forgotten");
    app.frame.update(cx, |f, cx| {
        f.remove_workspace(&main_id);
        f.select_workspace(&was_on);
        cx.notify();
    });
    let (_, _) = until(cx, &app, "back on the workspace it was on", |w| {
        w.try_find(("tab", 2usize)).is_none() && w.try_find("live-button").is_some()
    });

    // the app closed and opened again on all of it: the changes are there as last seen
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    std::thread::sleep(Duration::from_millis(300));
    let again = Instant::now();
    let app = common::restart_as(cx, app, false);
    let (launch, longest_launch) = until(cx, &app, "the app opened again", |w| {
        w.try_find(("tab", 1usize)).is_some() && w.try_find("changes-count").is_some()
    });
    let launched = again.elapsed();
    let (counted, _) = frame(cx, &app, |w| said(w, "changes-count"));
    println!(
        "LOAD opened again on both: the window with its changes listed ({}) after {:.0} ms ({:.0} ms of it drawing \
         until they showed), the longest frame {longest_launch}",
        counted.unwrap_or_default(),
        ms(launched),
        ms(launch)
    );
    let idle = drawn_at_rest(cx, app.window, &app.shell);
    println!(
        "LOAD at rest on that window: drawn {idle} times in {} s while nothing happens (a blinking cursor alone may \
         ask for a frame)",
        LOOKS / 10
    );
    assert!(idle <= BLINKS, "the window was drawn {idle} times in {LOOKS} looks while nothing happened");

    // what you would notice: a switch, an edit, a frame while scrolling, and no frame whose own work is long
    assert!(
        switch_work < 60.,
        "the window worked {switch_work:.1} ms for a switch ({there:?} and {back:?} by the clock)"
    );
    assert!(edit < 900., "one more edit took {edit:.0} ms to be listed, in the middle of {rounds}");
    assert!(middle.max(middle_read) < 8., "a scrolled frame took {middle:.2} and {middle_read:.2} ms in the middle");
    for (what, longest) in [
        ("the changes arrived", longest_arrival),
        ("the first diff showed", longest_first),
        ("the other workspace's changes arrived", longest_hidden),
        ("the changes were scrolled while they were read", longest_reading),
        ("the changes were scrolled", longest_read),
        ("one more edit was listed", longest_edit),
        ("the Files panel opened", longest_files),
        ("the filter answered", longest_filter),
        ("Search answered", longest_search),
        ("a file was open with its marks, and ⌘ was held over it", longest_file),
        ("Settings were open", longest_settings),
        ("Setup looked at this Mac", longest_setup),
        ("Live switched", longest_live),
        ("the main folder was opened as a workspace", longest_main),
        ("the app opened again", longest_launch),
    ] {
        assert!(longest.most_work < 33., "a frame worked {:.1} ms while {what}", longest.most_work);
    }
}

// The Work board under load (phase 5): an index ten times a usual one, on the board without a core behind it.

/// The sample index forty times over: 520 tickets, 360 PRs and 3,200 waiting items, in the sample's nine statuses
/// (about sixty cards a column).
fn big_index() -> enkidu_core::work::model::Snapshot {
    use enkidu_core::work::model::{Item, Pr, Session, Snapshot, Ticket};
    let base: Snapshot = serde_json::from_str(enkidu_testkit::WORK_SAMPLE).expect("the sample Work index");
    let mut work = Snapshot { health: base.health.clone(), ..Default::default() };
    for n in 0..40 {
        let key = |old: &str| old.replace("SH-", &format!("X{n}-"));
        let reference = |old: &str| old.replace("demo-org/", &format!("org{n}/"));
        work.tickets.extend(base.tickets.iter().cloned().map(|t| Ticket { key: key(&t.key), ..t }));
        for (old, project) in &base.ticket_projects {
            work.ticket_projects.insert(key(old), project.clone());
        }
        work.prs.extend(base.prs.iter().cloned().map(|p| Pr { repo: reference(&p.repo), ..p }));
        work.sessions.extend(base.sessions.iter().cloned().map(|s| Session { id: format!("{}-{n}", s.id), ..s }));
        for round in 0..8 {
            work.items.extend(base.items.iter().cloned().map(|i| Item {
                id: format!("{}-{n}-{round}", i.id),
                pr: i.pr.as_deref().map(reference),
                ticket: i.ticket.as_deref().map(key),
                ..i
            }));
        }
        work.links.extend(base.links.iter().cloned().map(|mut l| {
            l.a = reference(&key(&l.a));
            l.b = if l.b.starts_with("session:") { format!("{}-{n}", l.b) } else { reference(&key(&l.b)) };
            l
        }));
    }
    work
}

#[cfg_attr(debug_assertions, ignore = "a load test: `sh scripts/load.sh` runs it in a release build")]
#[gpui_kit::test]
fn the_work_board_stays_quick_with_an_index_ten_times_his(cx: &mut TestAppContext) {
    // the test runner's own line ends here, so every line of numbers starts at the margin
    println!();
    use enkidu_app::work::store::Work;
    use enkidu_core::db::{Project, Workspace};
    use enkidu_model::frame::{Frame as Model, View};

    let index = big_index();
    let (tickets, prs, items) = (index.tickets.len(), index.prs.len(), index.items.len());
    let project = |id: &str, name: &str| Project {
        id: id.into(),
        name: name.into(),
        path: format!("/code/{name}"),
        live_workspace_id: None,
        created_at: 1,
    };
    let workspaces: Vec<Workspace> = (0..10)
        .map(|n| Workspace {
            id: format!("w{n}"),
            project_id: "p1".into(),
            name: format!("task-{n}"),
            path: format!("/w/task-{n}"),
            branch: format!("feature/X{n}-21309-export"),
            base: Some("development".into()),
            session_id: format!("s-w{n}"),
            transcript_path: None,
            claude_config_dir: None,
            created_at: 1,
            adopted: false,
        })
        .collect();
    let view = View { project_id: Some("p1".into()), work: true, ..Default::default() };
    let model =
        Model::load(vec![project("p1", "shop"), project("p2", "enkidu")], workspaces, Default::default(), &view);
    // the cards are built from the index
    let (built, groups) = {
        let (started, before) = (Instant::now(), worked());
        let work = Work::given(index.clone(), &model, enkidu_testkit::WORK_SAMPLE_NOW);
        (Frame { took: ms(started.elapsed()), work: worked() - before }, work.groups.len())
    };
    println!(
        "LOAD work: {tickets} tickets, {prs} PRs and {items} waiting items make {groups} cards in {:.1} ms",
        built.work
    );
    let (window, shell, frame, store) = cx.update(|cx| {
        enkidu_app::init(cx);
        cx.set_reduce_motion(true);
        let work = cx.new(|_| Work::given(index, &model, enkidu_testkit::WORK_SAMPLE_NOW));
        let frame = cx.new(|_| model);
        let (on, given) = (frame.clone(), work.clone());
        let options = gpui_kit::WindowOptions {
            window_bounds: Some(gpui_kit::WindowBounds::Windowed(gpui_kit::Bounds {
                origin: Default::default(),
                size: gpui_kit::size(px(1360.), px(860.)),
            })),
            ..Default::default()
        };
        let (window, shell) =
            gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Shell::with_work(on, given, window, cx)))
                .expect("a test window");
        (window, shell, frame, work)
    });
    let draw_one = |cx: &mut TestAppContext| -> Frame {
        cx.run_until_parked();
        cx.update_window(window, |_, w, cx| draw(w, cx)).unwrap()
    };
    // the page as it opens: its first frame lays out every card of every column
    let first = draw_one(cx);
    let shown =
        shell.read_with(cx, |shell, cx| shell.work_page().map_or(0, |page| page.read(cx).board().shown().len()));
    println!(
        "LOAD work: the board with {shown} cards on it, first frame {:.1} ms ({:.1} ms its own work)",
        first.took, first.work
    );
    // a frame when nothing changed (a lamp elsewhere turned): the page is drawn again as it is
    let mut still = Longest::default();
    for _ in 0..20 {
        shell.update(cx, |_, cx| cx.notify());
        still = still.with(draw_one(cx));
    }
    println!("LOAD work: a frame of the board at rest {still}");
    let idle = drawn_at_rest(cx, window, &shell);
    println!("LOAD work: at rest the board is drawn {idle} times in {} s while nothing happens", LOOKS / 10);
    assert_eq!(idle, 0, "the board was drawn while nothing happened");
    // nothing piles up with the frames: the app stays open all day and draws millions of them. A hundred bytes
    // left by each frame would show here as 0.2 MB
    let before = heap();
    for _ in 0..2_000 {
        shell.update(cx, |_, cx| cx.notify());
        draw_one(cx);
    }
    let after = heap();
    println!(
        "LOAD work: 2,000 more frames of the board: {before:.1} MB in use before, {after:.1} MB after ({:+.2} MB)",
        after - before
    );
    assert!(after - before < 1., "2,000 frames of the board left {:.2} MB behind", after - before);
    // a column of sixty cards scrolled by the wheel
    let mut wheel = Longest::default();
    for _ in 0..40 {
        cx.run_until_parked();
        let took = cx
            .update_window(window, |_, w, cx| {
                w.scroll(("work-cards", 3usize), ScrollDelta::Pixels(point(px(0.), px(-90.))), cx);
                draw(w, cx)
            })
            .unwrap();
        wheel = wheel.with(took);
    }
    println!("LOAD work: scrolling a column of sixty cards, the longest frame {wheel}");
    // the board swiped sideways
    let mut swipe = Longest::default();
    for _ in 0..30 {
        cx.run_until_parked();
        let took = cx
            .update_window(window, |_, w, cx| {
                w.scroll("work-board", ScrollDelta::Pixels(point(px(-40.), px(0.))), cx);
                draw(w, cx)
            })
            .unwrap();
        swipe = swipe.with(took);
    }
    println!("LOAD work: swiping the board sideways, the longest frame {swipe}");
    // a Claude starts working: the cards are built again from the lamps, and the board with them
    let mut lamps = Longest::default();
    for n in 0..10 {
        let (started, before) = (Instant::now(), worked());
        frame.update(cx, |frame, cx| {
            frame.statuses.insert(format!("w{n}"), "working".into());
            cx.notify();
        });
        cx.run_until_parked();
        cx.update_window(window, |_, w, cx| w.render_frame(cx)).unwrap();
        lamps = lamps.with(Frame { took: ms(started.elapsed()), work: worked() - before });
    }
    println!("LOAD work: a lamp changes (cards and board built again, one frame) {lamps}");
    assert!(store.read_with(cx, |work, _| work.groups.len()) == groups);
    // the card in front of the board: the one with the most waiting on it, each thing with its own field
    let page = shell.read_with(cx, |shell, _| shell.work_page().expect("the Work page").clone());
    let (heavy, waiting) = page.read_with(cx, |page, _| {
        let cards = page.board().columns.iter().flat_map(|column| column.cards.iter());
        cards.max_by_key(|card| card.waiting).map(|card| (card.id.clone(), card.waiting)).expect("a card")
    });
    let opened = {
        let (started, before) = (Instant::now(), worked());
        cx.update_window(window, |_, w, cx| page.update(cx, |page, cx| page.open(&heavy, w, cx))).unwrap();
        cx.run_until_parked();
        // its first frame makes the fields and measures the description; the second is the card as it stays
        cx.update_window(window, |_, w, cx| {
            w.render_frame(cx);
            w.render_frame(cx);
        })
        .unwrap();
        Frame { took: ms(started.elapsed()), work: worked() - before }
    };
    assert!(page.read_with(cx, |page, _| page.card() == Some(heavy.as_str())), "the card is open");
    println!(
        "LOAD work: opening the card with {waiting} things that need you, to its second frame {:.1} ms ({:.1} ms its own work)",
        opened.took, opened.work
    );
    let mut card_still = Longest::default();
    for _ in 0..20 {
        shell.update(cx, |_, cx| cx.notify());
        card_still = card_still.with(draw_one(cx));
    }
    println!("LOAD work: a frame with that card open, at rest {card_still}");
    // j through everything that needs you there, each brought into view
    let mut through = Longest::default();
    for _ in 0..waiting {
        cx.run_until_parked();
        let took = cx
            .update_window(window, |_, w, cx| {
                w.press("j", cx);
                draw(w, cx)
            })
            .unwrap();
        through = through.with(took);
    }
    println!("LOAD work: j through its {waiting} items, the longest frame {through}");
    // an instruction typed for the item the keyboard is on: every letter is kept with the card's draft
    cx.update_window(window, |_, w, cx| w.press("n", cx)).unwrap();
    let mut typing = Longest::default();
    for letter in "Fix it as they say, then answer.".chars() {
        cx.run_until_parked();
        let took = cx
            .update_window(window, |_, w, cx| {
                w.input(&letter.to_string(), cx);
                draw(w, cx)
            })
            .unwrap();
        typing = typing.with(took);
    }
    println!("LOAD work: typing an instruction there, the longest letter {typing}");
    // Esc leaves the field; e marks the item done: the index changes at once, the cards, the board and the card
    // are built again
    cx.update_window(window, |_, w, cx| w.press("escape", cx)).unwrap();
    let mut done = Longest::default();
    for _ in 0..5 {
        // what e does, asked of the store itself: a key press of the test's own would draw frames of its own. A
        // test build draws the window once at the end of the update, so this is the change and its one frame
        let on = page
            .read_with(cx, |page, cx| {
                page.card_view().and_then(|card| card.read(cx).current_item().map(|item| item.id.clone()))
            })
            .expect("the keyboard is on an item");
        let (started, before) = (Instant::now(), worked());
        cx.update(|cx| Work::item_set(&store, &on, enkidu_app::work::store::ItemState::Done, window, cx));
        done = done.with(Frame { took: ms(started.elapsed()), work: worked() - before });
        cx.run_until_parked();
    }
    let left = page.read_with(cx, |page, cx| page.card_view().map(|card| card.read(cx).needs_count()));
    assert_eq!(left, Some(waiting - 5), "five items were marked done");
    println!("LOAD work: an item marked done (index, cards, board and card built again, one frame) {done}");

    // the workflow dialog for twelve cards, the ones with the most waiting on them: a chain of twelve tasks, each
    // with its own fields (drawn for the step that is picked) and its whole message made from what waits on its card
    cx.update_window(window, |_, w, cx| w.press("escape", cx)).unwrap();
    cx.run_until_parked();
    assert!(page.read_with(cx, |page, _| page.card().is_none()), "the card is closed");
    let heaviest: Vec<String> = page.read_with(cx, |page, _| {
        let mut cards: Vec<_> = page.board().columns.iter().flat_map(|column| column.cards.iter()).collect();
        cards.sort_by_key(|card| std::cmp::Reverse(card.waiting));
        cards.iter().take(12).map(|card| card.id.clone()).collect()
    });
    page.update(cx, |page, cx| heaviest.iter().for_each(|id| page.pick(id, cx)));
    let flow_opened = {
        let (started, before) = (Instant::now(), worked());
        cx.update_window(window, |_, w, cx| page.update(cx, |page, cx| page.workflow_for_picked(w, cx))).unwrap();
        cx.run_until_parked();
        // its first frame makes every task's fields; the second is the dialog as it stays
        cx.update_window(window, |_, w, cx| {
            w.render_frame(cx);
            w.render_frame(cx);
        })
        .unwrap();
        Frame { took: ms(started.elapsed()), work: worked() - before }
    };
    let (tasks, letters) = page.read_with(cx, |page, cx| {
        let draft = page.flow_editor().expect("the workflow dialog").read(cx).draft().clone();
        (draft.tasks.len(), draft.tasks.iter().map(|task| task.message.len()).sum::<usize>())
    });
    assert_eq!(tasks, 12, "every picked card is a task");
    println!(
        "LOAD work: the workflow dialog for {tasks} tasks ({} KB of messages), to its second frame {:.1} ms ({:.1} ms its own work)",
        letters / 1024,
        flow_opened.took,
        flow_opened.work
    );
    let mut flow_still = Longest::default();
    for _ in 0..20 {
        shell.update(cx, |_, cx| cx.notify());
        flow_still = flow_still.with(draw_one(cx));
    }
    println!("LOAD work: a frame with that dialog open, at rest {flow_still}");
    // its name typed (the keyboard starts there): every letter makes all twelve messages again
    let mut naming = Longest::default();
    for letter in " for the review".chars() {
        cx.run_until_parked();
        let took = cx
            .update_window(window, |_, w, cx| {
                w.input(&letter.to_string(), cx);
                draw(w, cx)
            })
            .unwrap();
        naming = naming.with(took);
    }
    let named = page.read_with(cx, |page, cx| page.flow_editor().map(|editor| editor.read(cx).draft().name.clone()));
    assert_eq!(named.as_deref(), Some("Night run for the review"));
    println!("LOAD work: typing its name (twelve messages made again), the longest letter {naming}");
    // from step to step down the chain: each task's own setup drawn as it is picked (item 17.4). Twelve tasks are
    // more than the chain's pane holds, so it is scrolled a little further before each click
    let mut stepping = Longest::default();
    for n in 0..tasks {
        cx.run_until_parked();
        let took = cx
            .update_window(window, |_, w, cx| {
                w.scroll("flow-chain", ScrollDelta::Pixels(point(px(0.), px(-50.))), cx);
                w.render_frame(cx);
                w.click(("flow-task", n), cx);
                draw(w, cx)
            })
            .unwrap();
        stepping = stepping.with(took);
    }
    let on_last = cx.update_window(window, |_, w, _| w.try_find(("flow-task-words", tasks - 1)).is_some()).unwrap();
    assert!(on_last, "the last task's setup is shown");
    println!("LOAD work: going from step to step through its {tasks} tasks, the longest frame {stepping}");
    // a task's own words typed: the first one's, the chain back at its top
    cx.update_window(window, |_, w, cx| {
        w.scroll("flow-chain", ScrollDelta::Pixels(point(px(0.), px(4000.))), cx);
        w.render_frame(cx);
        w.click(("flow-task", 0usize), cx);
        w.render_frame(cx);
        w.click(("flow-task-words", 0usize), cx);
    })
    .unwrap();
    let mut wording = Longest::default();
    for letter in " Only the header.".chars() {
        cx.run_until_parked();
        let took = cx
            .update_window(window, |_, w, cx| {
                w.input(&letter.to_string(), cx);
                draw(w, cx)
            })
            .unwrap();
        wording = wording.with(took);
    }
    let worded = page.read_with(cx, |page, cx| {
        page.flow_editor().map(|editor| editor.read(cx).draft().tasks[0].instruction.contains("Only the header."))
    });
    assert_eq!(worded, Some(true), "the words went into the task");
    println!("LOAD work: typing a task's own words, the longest letter {wording}");
    // the chain scrolled through its twelve tasks
    let mut flow_wheel = Longest::default();
    for _ in 0..40 {
        cx.run_until_parked();
        let took = cx
            .update_window(window, |_, w, cx| {
                w.scroll("flow-chain", ScrollDelta::Pixels(point(px(0.), px(-90.))), cx);
                draw(w, cx)
            })
            .unwrap();
        flow_wheel = flow_wheel.with(took);
    }
    println!("LOAD work: scrolling the dialog's chain through its tasks, the longest frame {flow_wheel}");

    // the Workflows page: twenty runs kept, each of those twelve tasks with its whole message, three workflows
    // still to run, and on the task shown five updates and a report with two screenshots, each a whole screen
    cx.update_window(window, |_, w, cx| w.click("flow-cancel", cx)).unwrap();
    cx.run_until_parked();
    assert!(page.read_with(cx, |page, _| page.flow_editor().is_none()), "the dialog is closed");
    let now = enkidu_testkit::WORK_SAMPLE_NOW;
    let minute = 60_000i64;
    let picture = std::sync::Arc::new(enkidu_testkit::busy_screenshot_png(2880, 1800, 3));
    cx.update(|cx| {
        let picture = picture.clone();
        cx.set_global(enkidu_app::work::shots::ShotSource(std::sync::Arc::new(move |_, _| Ok(picture.to_vec()))));
    });
    let (runs_kept, tasks_each) = store.update(cx, |work, cx| {
        use enkidu_core::work::flows::{Flow, FlowRun, FlowTask};
        let index = std::sync::Arc::make_mut(work.snapshot.as_mut().expect("the index"));
        let chosen = std::collections::HashMap::new();
        let cards: Vec<&enkidu_model::work::groups::Group> =
            heaviest.iter().filter_map(|id| work.groups.iter().find(|group| &group.id == id)).collect();
        let night = enkidu_model::work::flows::new_flow(
            &cards,
            index,
            &[],
            &std::collections::HashMap::new(),
            &chosen,
            now,
            &index.flow_kinds,
        );
        let states = ["done", "review", "blocked", "done", "timed_out", "done", "no_report", "done", "asked", "done", "cut_off", "stopped"];
        let run = |n: i64| -> FlowRun {
            let at = now - n * 24 * 60 * minute;
            let tasks: Vec<FlowTask> = (night.tasks.iter().zip(states).enumerate())
                .map(|(k, (task, state))| FlowTask {
                    state: state.to_string(),
                    started_at: Some(at + k as i64 * 20 * minute),
                    ended_at: Some(at + k as i64 * 20 * minute + 19 * minute),
                    ..task.clone()
                })
                .collect();
            let flow = Flow { id: "f1".into(), name: format!("Night run {n}"), state: "done".into(), start_at: Some(at), tasks, ..night.clone() };
            FlowRun { id: 100 - n, flow_id: "f1".into(), started_at: Some(at), ended_at: at + 240 * minute, state: "done".into(), flow }
        };
        index.flow_runs = (1..=20).map(run).collect();
        index.flows = (0..3)
            .map(|n| Flow {
                id: format!("due{n}"),
                name: format!("Tonight {n}"),
                state: "scheduled".into(),
                start_at: Some(now + (n + 1) * 60 * minute),
                ..night.clone()
            })
            .collect();
        // on the task the newest run opens on (its second, for your review): five updates and its report
        let first = &index.flow_runs[0].flow.tasks[1];
        let subject = first.subject.trim_start_matches("ticket:").to_string();
        let from = first.started_at.unwrap_or(now);
        index.reports = (0..6)
            .map(|k| enkidu_core::work::reports::Report {
                id: 50 - k,
                subject: subject.clone(),
                session_id: None,
                workspace_id: None,
                state: if k == 0 { "review".into() } else { "update".into() },
                text: "Read the comments and found the cause.\n\n- The writer ignores the outlet's setting.\n- The tests cover it now.".into(),
                images: if k == 0 { vec!["1-list.png".into(), "2-excel.png".into()] } else { Vec::new() },
                at: from + (18 - 3 * k) * minute,
            })
            .collect();
        let made = (index.flow_runs.len(), index.flow_runs[0].flow.tasks.len());
        work.regroup();
        cx.notify();
        made
    });
    cx.run_until_parked();
    let flows_opened = {
        let (started, before) = (Instant::now(), worked());
        // the page comes on screen and is drawn twice: its first frame measures the message and the ticket (they
        // fold), the second is the page as it stays. The screenshots are read meanwhile, off the drawing thread: in
        // a test that reading runs on this thread once it is let (the next step), so it isn't in this time
        cx.update_window(window, |_, w, cx| {
            page.update(cx, |page, cx| page.show_flows(true, w, cx));
            w.render_frame(cx);
            w.render_frame(cx);
        })
        .unwrap();
        Frame { took: ms(started.elapsed()), work: worked() - before }
    };
    let shown_first = page.read_with(cx, |page, cx| page.flows().read(cx).shown());
    assert_eq!(shown_first.0.as_deref(), Some("run:99"), "the newest run is shown");
    println!(
        "LOAD work: the Workflows page with {runs_kept} runs of {tasks_each} tasks, to its second frame {:.1} ms ({:.1} ms its own work)",
        flows_opened.took, flows_opened.work
    );
    // its two screenshots arrive, read and shrunk off the drawing thread: the frames meanwhile, and the one that
    // first shows them
    let (arrived, arriving) = {
        let seen = |w: &Window| {
            ["report-image:50:1-list.png", "report-image:50:2-excel.png"]
                .iter()
                .all(|id| w.try_find(*id).is_some_and(|shot| shot.description() == Some("shown")))
        };
        let (started, mut longest) = (Instant::now(), Longest::default());
        loop {
            cx.run_until_parked();
            let (there, took) = cx
                .update_window(window, |_, w, cx| {
                    let took = draw(w, cx);
                    (seen(w), took)
                })
                .unwrap();
            longest = longest.with(took);
            if there {
                break (started.elapsed(), longest);
            }
            assert!(started.elapsed() < Duration::from_secs(120), "the screenshots never showed");
            std::thread::sleep(Duration::from_millis(2));
        }
    };
    println!(
        "LOAD work: its report's two large screenshots shown after {:.0} ms, the longest frame meanwhile {arriving}",
        ms(arrived)
    );
    let mut flows_still = Longest::default();
    for _ in 0..20 {
        shell.update(cx, |_, cx| cx.notify());
        flows_still = flows_still.with(draw_one(cx));
    }
    println!("LOAD work: a frame of the Workflows page at rest {flows_still}");
    // ↓ through the run's twelve tasks: each one's page made and drawn
    let mut tasks_down = Longest::default();
    for _ in 0..tasks_each {
        cx.run_until_parked();
        let took = cx
            .update_window(window, |_, w, cx| {
                w.press("down", cx);
                draw(w, cx)
            })
            .unwrap();
        tasks_down = tasks_down.with(took);
    }
    let last = page.read_with(cx, |page, cx| page.flows().read(cx).shown().1);
    assert!(last.is_some_and(|subject| Some(&subject) == heaviest.last()), "the keyboard reached the last task");
    println!("LOAD work: ↓ through its {tasks_each} tasks, the longest frame {tasks_down}");
    // another run picked, and another: each from the menu of the page's name, which lists all 23 (item 17.5). A
    // menu is built as it opens and carries out a pick as it closes, so each is timed from the click to the frame
    // that shows it, with what the click put off in between
    let clicked = |cx: &mut TestAppContext, id: gpui_kit::ElementId| -> Frame {
        cx.run_until_parked();
        let (started, before) = (Instant::now(), worked());
        cx.update_window(window, |_, w, cx| w.click(id, cx)).unwrap();
        cx.run_until_parked();
        cx.update_window(window, |_, w, cx| w.render_frame(cx)).unwrap();
        Frame { took: ms(started.elapsed()), work: worked() - before }
    };
    let (mut menu_opened, mut runs_picked) = (Longest::default(), Longest::default());
    // the runs near the menu's top: the ones further down are reached with the wheel (a test of the page's own)
    for n in 1..5usize {
        menu_opened = menu_opened.with(clicked(cx, "flows-switch".into()));
        runs_picked = runs_picked.with(clicked(cx, ("flow-run-item", n).into()));
        let picked = page.read_with(cx, |page, cx| page.flows().read(cx).shown().0);
        assert_eq!(picked, Some(format!("run:{}", 99 - n)), "the run that was clicked is shown");
    }
    println!(
        "LOAD work: opening the menu of {} workflows and runs, from the click to its frame, the longest {menu_opened}",
        runs_kept + 3
    );
    println!("LOAD work: picking another run there, from the click to its frame, the longest {runs_picked}");
    // the run's chain scrolled
    let mut runs_wheel = Longest::default();
    for _ in 0..30 {
        cx.run_until_parked();
        let took = cx
            .update_window(window, |_, w, cx| {
                w.scroll("flow-detail-list", ScrollDelta::Pixels(point(px(0.), px(-60.))), cx);
                draw(w, cx)
            })
            .unwrap();
        runs_wheel = runs_wheel.with(took);
    }
    println!("LOAD work: scrolling a run's chain, the longest frame {runs_wheel}");
    println!("LOAD work: memory {:.0} MB ({:.0} MB of it in use)", memory(), heap());
    // bounds on the work itself: twice a frame of 60 a second at most, for an index ten times a usual one
    assert!(built.work < 30., "building the cards took {:.1} ms", built.work);
    assert!(still.most_work < 33., "a frame of the board at rest took {:.1} ms of work", still.most_work);
    assert!(wheel.most_work < 33., "a frame while scrolling a column took {:.1} ms of work", wheel.most_work);
    assert!(swipe.most_work < 33., "a frame while swiping the board took {:.1} ms of work", swipe.most_work);
    assert!(lamps.most_work < 60., "a lamp's change took {:.1} ms of work", lamps.most_work);
    assert!(opened.work < 100., "opening the heaviest card took {:.1} ms of work", opened.work);
    assert!(card_still.most_work < 33., "a frame with the card open took {:.1} ms of work", card_still.most_work);
    assert!(through.most_work < 33., "a frame while moving through its items took {:.1} ms of work", through.most_work);
    assert!(typing.most_work < 33., "a typed letter took {:.1} ms of work", typing.most_work);
    assert!(done.most_work < 60., "marking an item done took {:.1} ms of work", done.most_work);
    assert!(flow_opened.work < 100., "opening the workflow dialog took {:.1} ms of work", flow_opened.work);
    assert!(flow_still.most_work < 33., "a frame with the dialog open took {:.1} ms of work", flow_still.most_work);
    assert!(naming.most_work < 33., "a letter of its name took {:.1} ms of work", naming.most_work);
    assert!(stepping.most_work < 33., "a step of the chain took {:.1} ms of work", stepping.most_work);
    assert!(wording.most_work < 33., "a letter of a task's words took {:.1} ms of work", wording.most_work);
    assert!(
        flow_wheel.most_work < 33.,
        "a frame while scrolling the dialog took {:.1} ms of work",
        flow_wheel.most_work
    );
    assert!(flows_opened.work < 100., "opening the Workflows page took {:.1} ms of work", flows_opened.work);
    assert!(arriving.most_work < 33., "a frame while its screenshots came took {:.1} ms of work", arriving.most_work);
    assert!(flows_still.most_work < 33., "a frame of the Workflows page took {:.1} ms of work", flows_still.most_work);
    assert!(tasks_down.most_work < 33., "a frame while going through its tasks took {:.1} ms", tasks_down.most_work);
    assert!(menu_opened.most_work < 33., "opening the menu of runs took {:.1} ms of work", menu_opened.most_work);
    assert!(runs_picked.most_work < 33., "picking another run took {:.1} ms of work", runs_picked.most_work);
    assert!(runs_wheel.most_work < 33., "a frame while scrolling a run's chain took {:.1} ms", runs_wheel.most_work);
}

/// Item 5.14 under load: a report with twelve screenshots, each a whole Retina screen (2,880 by 1,800). What is
/// timed: reading and shrinking one off the drawing thread, the card's frames before and after they arrive, one
/// shown as large as the window, → through all twelve, and what the pictures weigh.
#[cfg_attr(debug_assertions, ignore = "a load test: `sh scripts/load.sh` runs it in a release build")]
#[gpui_kit::test]
fn a_report_with_twelve_screenshots_stays_quick(cx: &mut TestAppContext) {
    println!();
    use enkidu_app::work::shots::{ShotSource, large_of, small_of};
    use enkidu_app::work::store::Work;
    use enkidu_core::db::{Project, Workspace};
    use enkidu_model::frame::{Frame as Model, View};

    const SHOTS: usize = 12;
    let picture = std::sync::Arc::new(enkidu_testkit::busy_screenshot_png(2880, 1800, 1));
    // a recording too, as Claude makes them of a page: 40 pictures of 1,120 by 688, a block moving over stripes
    const PICTURES: u32 = 40;
    let recording = std::sync::Arc::new({
        let mut bytes = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new_with_speed(&mut bytes, 30);
            for n in 0..PICTURES {
                let frame = image::RgbaImage::from_fn(1120, 688, |x, y| match (x / 20 == n, (x / 40 + y / 40) % 2) {
                    (true, _) => image::Rgba([30, 90, 220, 255]),
                    (false, 0) => image::Rgba([245, 245, 245, 255]),
                    (false, _) => image::Rgba([225, 228, 232, 255]),
                });
                let delay = image::Delay::from_numer_denom_ms(120, 1);
                encoder.encode_frame(image::Frame::from_parts(frame, 0, 0, delay)).unwrap();
            }
        }
        bytes
    });
    // one of them read and shrunk, as the app does it off the drawing thread
    let timed = |work: &dyn Fn()| {
        let mut best = f64::MAX;
        for _ in 0..3 {
            let before = worked();
            work();
            best = best.min(worked() - before);
        }
        best
    };
    let small =
        timed(&|| drop(small_of(&picture, enkidu_model::work::report::THUMB_WIDTH_OF_MANY).expect("a thumbnail")));
    let large = timed(&|| drop(large_of(&picture, (3456, 2234)).expect("the whole picture")));
    println!(
        "LOAD report: a screenshot of 2,880 by 1,800 ({:.1} MB as a PNG) is read and shrunk to its thumbnail in {small:.0} ms, \
         and read whole for the large view in {large:.0} ms, both off the drawing thread",
        picture.len() as f64 / 1_048_576.
    );

    let names: Vec<String> = (1..=SHOTS).map(|n| format!("{n}-screen.png")).collect();
    let mut index: enkidu_core::work::model::Snapshot =
        serde_json::from_str(enkidu_testkit::WORK_SAMPLE).expect("the sample Work index");
    index.reports = vec![enkidu_core::work::reports::Report {
        id: 7,
        subject: "SH-21309".into(),
        session_id: Some("s-w2".into()),
        workspace_id: Some("w2".into()),
        state: "review".into(),
        text: "Fixed the CSV header.\n\nChecked the export with 3 orders.".into(),
        images: names.clone(),
        at: enkidu_testkit::WORK_SAMPLE_NOW - 60_000,
    }];
    // and on another ticket a report with three recordings: they play in their thumbnails
    let flows: Vec<String> = (1..=3).map(|n| format!("{n}-flow.gif")).collect();
    index.reports.push(enkidu_core::work::reports::Report {
        id: 8,
        subject: "SH-21376".into(),
        session_id: None,
        workspace_id: None,
        state: "review".into(),
        text: "Recorded the three flows.".into(),
        images: flows.clone(),
        at: enkidu_testkit::WORK_SAMPLE_NOW - 120_000,
    });
    let project = Project {
        id: "p1".into(),
        name: "shop".into(),
        path: "/code/shop".into(),
        live_workspace_id: None,
        created_at: 1,
    };
    let workspace = Workspace {
        id: "w2".into(),
        project_id: "p1".into(),
        name: "sh-21309-export".into(),
        path: "/w/sh-21309-export".into(),
        branch: "feature/SH-21309-export".into(),
        base: Some("development".into()),
        session_id: "s-w2".into(),
        transcript_path: None,
        claude_config_dir: None,
        created_at: 1,
        adopted: false,
    };
    let view = View { project_id: Some("p1".into()), work: true, ..Default::default() };
    let model = Model::load(vec![project], vec![workspace], Default::default(), &view);
    let (window, shell) = cx.update(|cx| {
        enkidu_app::init(cx);
        cx.set_reduce_motion(true);
        let (given, moving) = (picture.clone(), recording.clone());
        cx.set_global(ShotSource(std::sync::Arc::new(move |_, name| match name.ends_with(".gif") {
            true => Ok(moving.to_vec()),
            false => Ok(given.to_vec()),
        })));
        let work = cx.new(|_| Work::given(index, &model, enkidu_testkit::WORK_SAMPLE_NOW));
        let frame = cx.new(|_| model);
        let options = gpui_kit::WindowOptions {
            window_bounds: Some(gpui_kit::WindowBounds::Windowed(gpui_kit::Bounds {
                origin: Default::default(),
                size: gpui_kit::size(px(1360.), px(860.)),
            })),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Shell::with_work(frame, work, window, cx)))
            .expect("a test window")
    });
    cx.run_until_parked();
    cx.update_window(window, |_, w, cx| w.render_frame(cx)).unwrap();
    let before = memory();
    let page = shell.read_with(cx, |shell, _| shell.work_page().expect("the Work page").clone());
    // the card opens at once, its screenshots' places empty: nothing was read yet
    let opened = {
        let (started, before) = (Instant::now(), worked());
        cx.update_window(window, |_, w, cx| {
            page.update(cx, |page, cx| page.open("ticket:SH-21309", w, cx));
            w.render_frame(cx);
            w.render_frame(cx);
        })
        .unwrap();
        Frame { took: ms(started.elapsed()), work: worked() - before }
    };
    let state = |cx: &mut TestAppContext, name: &str| -> Option<String> {
        let id = gpui_kit::SharedString::from(format!("report-image:7:{name}"));
        cx.update_window(window, |_, w, _| w.try_find(id).and_then(|e| e.description().map(String::from))).unwrap()
    };
    assert_eq!(state(cx, &names[0]).as_deref(), Some("being read"));
    println!(
        "LOAD report: the card opens before any screenshot is read, to its second frame {:.1} ms ({:.1} ms its own work)",
        opened.took, opened.work
    );
    // they are read one after another (here on the test's own thread; in the app on a thread of its own)
    let (started, at) = (Instant::now(), worked());
    cx.run_until_parked();
    let read = Frame { took: ms(started.elapsed()), work: worked() - at };
    let arrive = cx.update_window(window, |_, w, cx| draw(w, cx)).unwrap();
    assert!(names.iter().all(|name| state(cx, name).as_deref() == Some("shown")), "all twelve show");
    println!(
        "LOAD report: its {SHOTS} screenshots read and shrunk in {:.0} ms off the drawing thread, then the frame that \
         shows them all {:.1} ms",
        read.work, arrive.work
    );
    let mut still = Longest::default();
    for _ in 0..20 {
        shell.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        still = still.with(cx.update_window(window, |_, w, cx| draw(w, cx)).unwrap());
    }
    println!("LOAD report: a frame of the card with its {SHOTS} thumbnails, at rest {still}");
    let with_thumbs = memory();
    // one as large as the window: the layer is there in the next frame, with the thumbnail until the whole one is read
    // (what a press on a thumbnail does, asked of the store itself: a click of the test's own draws frames of its own)
    let card = page.read_with(cx, |page, _| page.card_view().expect("the card is open").clone());
    let shots = card.read_with(cx, |card, _| card.shots().clone());
    let shown_large = {
        let (started, before) = (Instant::now(), worked());
        cx.update_window(window, |_, w, cx| {
            shots.update(cx, |shots, cx| shots.open(7, names.clone(), 0, w, cx));
            w.render_frame(cx);
        })
        .unwrap();
        Frame { took: ms(started.elapsed()), work: worked() - before }
    };
    let large_says = |cx: &mut TestAppContext| -> Option<String> {
        cx.update_window(window, |_, w, _| {
            w.try_find("report-lightbox-image").and_then(|e| e.description().map(String::from))
        })
        .unwrap()
    };
    assert_eq!(large_says(cx).as_deref(), Some("its thumbnail meanwhile"));
    cx.run_until_parked();
    let whole = cx.update_window(window, |_, w, cx| draw(w, cx)).unwrap();
    assert_eq!(large_says(cx).as_deref(), Some("whole"));
    println!(
        "LOAD report: a screenshot shown as large as the window, the press and its frame {:.1} ms; the frame when the \
         whole picture is there {:.1} ms",
        shown_large.work, whole.work
    );
    // → through all twelve, each read again at the screen's size
    let mut through = Longest::default();
    for _ in 0..SHOTS {
        let took = cx
            .update_window(window, |_, w, cx| {
                let (started, before) = (Instant::now(), worked());
                shots.update(cx, |shots, cx| shots.step(1, w, cx));
                w.render_frame(cx);
                Frame { took: ms(started.elapsed()), work: worked() - before }
            })
            .unwrap();
        through = through.with(took);
        cx.run_until_parked();
        through = through.with(cx.update_window(window, |_, w, cx| draw(w, cx)).unwrap());
    }
    println!("LOAD report: → through its {SHOTS} screenshots, the longest frame {through}");
    let with_large = memory();
    // Esc closes the large one, and the card closed lets every picture go
    cx.update_window(window, |_, w, cx| {
        w.press("escape", cx);
        w.render_frame(cx);
    })
    .unwrap();
    assert!(large_says(cx).is_none());
    assert!(shots.read_with(cx, |shots, _| shots.small(7, &names[0]).is_some()));
    println!(
        "LOAD report: memory {before:.0} MB before the card, {with_thumbs:.0} MB with its {SHOTS} thumbnails, \
         {with_large:.0} MB with one shown large"
    );
    // a recording shown large plays from its file, a picture at a time, at its own size: three seconds of it, a
    // frame for each of its pictures, and what it weighs meanwhile (read whole, its 40 pictures are 118 MB)
    cx.update(|cx| cx.set_reduce_motion(false));
    // (its thumbnail first, as on the card: the large view takes the picture's size from it)
    let flow = vec!["13-flow.gif".to_string()];
    cx.update(|cx| shots.update(cx, |shots, cx| shots.want(7, &flow, cx)));
    cx.run_until_parked();
    assert!(shots.read_with(cx, |shots, _| matches!(
        shots.small(7, &flow[0]),
        Some(enkidu_app::work::shots::Small::Ready { whole: (1120, 688), .. })
    )));
    let before_playing = memory();
    let recording_shown = {
        let (started, before) = (Instant::now(), worked());
        cx.update_window(window, |_, w, cx| {
            shots.update(cx, |shots, cx| shots.open(7, flow.clone(), 0, w, cx));
            w.render_frame(cx);
        })
        .unwrap();
        Frame { took: ms(started.elapsed()), work: worked() - before }
    };
    let first_picture = Instant::now();
    while large_says(cx).as_deref() != Some("whole, playing") {
        assert!(first_picture.elapsed() < Duration::from_secs(60), "the recording never played: {:?}", large_says(cx));
        cx.run_until_parked();
        cx.update_window(window, |_, w, cx| draw(w, cx)).unwrap();
    }
    let first_picture = ms(first_picture.elapsed());
    let (mut playing, mut most) = (Longest::default(), before_playing);
    for _ in 0..25 {
        cx.executor().advance_clock(Duration::from_millis(120));
        cx.run_until_parked();
        playing = playing.with(cx.update_window(window, |_, w, cx| draw(w, cx)).unwrap());
        most = most.max(memory());
    }
    assert_eq!(large_says(cx).as_deref(), Some("whole, playing"));
    cx.update_window(window, |_, w, cx| {
        w.press("escape", cx);
        w.render_frame(cx);
    })
    .unwrap();
    assert!(large_says(cx).is_none());
    println!(
        "LOAD report: a recording of 1,120 by 688 in {PICTURES} pictures shown large: the press and its frame {:.1} ms, its \
         first picture after {first_picture:.0} ms; while it plays, a frame for each picture {playing}; it weighs {:.0} MB \
         more at most (read whole it would be 118 MB)",
        recording_shown.work,
        most - before_playing
    );
    // the drawing thread's own work: a frame of 60 a second at most; reading is off it, and bounded too
    assert!(recording_shown.work < 50., "showing the recording took {:.1} ms of work", recording_shown.work);
    assert!(playing.most_work < 33., "a frame of the playing recording took {:.1} ms of work", playing.most_work);
    assert!(most - before_playing < 60., "the playing recording weighs {:.0} MB", most - before_playing);
    assert!(opened.work < 100., "opening the card took {:.1} ms of work", opened.work);
    assert!(arrive.work < 33., "the frame that shows the thumbnails took {:.1} ms of work", arrive.work);
    assert!(still.most_work < 33., "a frame with the thumbnails took {:.1} ms of work", still.most_work);
    assert!(shown_large.work < 50., "showing one large took {:.1} ms of work", shown_large.work);
    assert!(whole.work < 33., "the frame with the whole picture took {:.1} ms of work", whole.work);
    assert!(through.most_work < 50., "a step to the next screenshot took {:.1} ms of work", through.most_work);
    assert!(small < 400. && large < 400., "reading a screenshot took {small:.0} ms small, {large:.0} ms large");
    assert!(with_large - before < 160., "the pictures weigh {:.0} MB", with_large - before);

    // three recordings that play in their thumbnails at once (phase 26): the card with them, three seconds of it
    let before_three = memory();
    cx.update_window(window, |_, w, cx| {
        page.update(cx, |page, cx| page.open("ticket:SH-21376", w, cx));
        w.render_frame(cx);
    })
    .unwrap();
    let playing_all = |cx: &mut TestAppContext| {
        cx.update_window(window, |_, w, _| {
            flows.iter().all(|name| {
                let id = gpui_kit::SharedString::from(format!("report-image:8:{name}"));
                w.try_find(id).is_some_and(|shot| shot.description() == Some("shown, playing"))
            })
        })
        .unwrap()
    };
    let started = Instant::now();
    while !playing_all(cx) {
        assert!(started.elapsed() < Duration::from_secs(60), "the three recordings never played");
        cx.run_until_parked();
        cx.update_window(window, |_, w, cx| draw(w, cx)).unwrap();
    }
    let (mut three, mut most_three) = (Longest::default(), before_three);
    for _ in 0..25 {
        cx.executor().advance_clock(Duration::from_millis(120));
        cx.run_until_parked();
        three = three.with(cx.update_window(window, |_, w, cx| draw(w, cx)).unwrap());
        most_three = most_three.max(memory());
    }
    println!(
        "LOAD report: three recordings of 1,120 by 688 playing in their thumbnails at once: a frame of the card with them \
         {three}; they weigh {:.0} MB more at most",
        most_three - before_three
    );
    assert!(three.most_work < 33., "a frame with three playing thumbnails took {:.1} ms of work", three.most_work);
    assert!(most_three - before_three < 120., "three playing thumbnails weigh {:.0} MB", most_three - before_three);
}
