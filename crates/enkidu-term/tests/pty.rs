//! A terminal with a real pty and real programs: what they print shows on screen, what is typed reaches them, a
//! resize reaches them, and the threads keep the window free.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use enkidu_term::{GridSize, Palette, Rgb, Screen, Spawn, Terminal, base_env};

fn start(program: Option<(&str, &[&str])>, size: GridSize) -> Terminal {
    let spawn = Spawn {
        program: program.map(|(p, args)| (p.to_string(), args.iter().map(|a| a.to_string()).collect())),
        cwd: std::env::temp_dir(),
        env: base_env(),
    };
    Terminal::spawn(spawn, size, Palette::enkidu(), Arc::new(|| {})).expect("the program starts")
}

fn sh(script: &str) -> Terminal {
    start(Some(("/bin/sh", &["-c", script])), GridSize::new(80, 24))
}

/// Waits up to `secs` for the screen to satisfy `done`, and returns it.
fn wait_for(t: &Terminal, secs: u64, done: impl Fn(&Screen) -> bool) -> Screen {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        let screen = t.screen();
        if done(&screen) {
            return screen;
        }
        assert!(Instant::now() < deadline, "timed out; the screen shows:\n{}", screen.text());
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_program_draws_with_256_colors_true_color_and_its_cursor() {
    let t = sh(r"printf '\033[38;5;196mR\033[38;2;10;20;30mT\033[0m\n'; printf '\033[3;5Hx'; read line");
    let s = wait_for(&t, 10, |s| s.row_text(0) == "RT" && s.row_text(2).trim() == "x");
    assert_eq!(s.row(0)[0].fg, Rgb(255, 0, 0), "color 196 of the 256");
    assert_eq!(s.row(0)[1].fg, Rgb(10, 20, 30), "true color");
    let cursor = s.cursor.expect("a cursor");
    assert_eq!((cursor.row, cursor.col), (2, 5));
}

#[test]
fn typing_reaches_the_program() {
    let t = sh(r#"read line; echo "got:$line"; read x"#);
    std::thread::sleep(Duration::from_millis(100));
    t.send(b"hello\r".to_vec());
    wait_for(&t, 10, |s| s.text().contains("got:hello"));
}

#[test]
fn the_program_sees_the_new_size_after_a_resize() {
    let t = sh("read x; stty size; read y");
    assert!(t.resize(GridSize::new(100, 30)));
    std::thread::sleep(Duration::from_millis(100));
    t.send(b"\r".to_vec());
    wait_for(&t, 10, |s| s.text().contains("30 100"));
    assert_eq!(t.screen().cols, 100);
}

/// The user's own login shell, as the app starts it. It reads the user's shell files, so it runs only when asked:
/// `cargo test -p enkidu-term --test pty -- --ignored`.
#[test]
#[ignore]
fn the_login_shell_starts_and_runs_a_command() {
    let t = start(None, GridSize::new(120, 30));
    std::thread::sleep(Duration::from_millis(1500));
    t.send(b"echo rh-$((6*7))\r".to_vec());
    let s = wait_for(&t, 20, |s| s.text().lines().any(|l| l.trim() == "enkidu-42"));
    let shell = std::env::var("SHELL").unwrap_or_default();
    println!("login shell {shell}: answered enkidu-42; the screen shows:\n{}", s.text());
}

#[test]
fn typing_never_waits_for_a_program_that_stopped_reading() {
    // The program never reads, so the pty's input fills up after a few kilobytes.
    let t = sh("sleep 20");
    let chunk = vec![b'x'; 64 * 1024];
    let started = Instant::now();
    for _ in 0..64 {
        t.send(chunk.clone());
    }
    let took = started.elapsed();
    println!("4 MB typed into a program that doesn't read: send() returned after {took:?} in all");
    assert!(took < Duration::from_millis(200), "typing waited {took:?}");
    t.kill();
}

#[test]
fn output_is_parsed_on_its_own_thread_while_the_window_takes_copies() {
    // About 11 MB of output, as fast as the program can write it.
    let lines = 300_000u64;
    let t = sh(&format!("yes 0123456789abcdefghijklmnopqrstuvwxyz | head -n {lines}"));
    let expected = lines * 38; // 36 characters, then the pty's \r\n
    let started = Instant::now();
    let mut slowest = Duration::ZERO;
    let mut copies = 0u32;
    while t.parsed() < expected {
        // What the window does once a frame: take a copy of the screen.
        let before = Instant::now();
        let screen = t.screen();
        slowest = slowest.max(before.elapsed());
        copies += 1;
        assert_eq!(screen.cols, 80);
        assert!(started.elapsed() < Duration::from_secs(60), "parsed only {} bytes", t.parsed());
        std::thread::sleep(Duration::from_millis(1));
    }
    let took = started.elapsed();
    let mb = t.parsed() as f64 / 1_000_000.0;
    println!(
        "{mb:.1} MB parsed in {took:?} ({:.0} MB/s, debug build); {copies} screen copies, the slowest {slowest:?}",
        mb / took.as_secs_f64()
    );
    assert!(slowest < Duration::from_millis(25), "a copy of the screen waited {slowest:?}");
    assert_eq!(wait_for(&t, 5, |s| s.row_text(22).len() == 36).row_text(22), "0123456789abcdefghijklmnopqrstuvwxyz");
}

#[test]
fn the_terminal_runs_in_the_folder_it_is_given() {
    let dir: PathBuf = std::env::temp_dir();
    let t = start(Some(("/bin/sh", &["-c", "pwd; read x"])), GridSize::new(120, 5));
    let real = std::fs::canonicalize(&dir).unwrap();
    wait_for(&t, 10, |s| {
        let shown = s.row_text(0);
        shown == real.to_string_lossy() || shown == dir.to_string_lossy().trim_end_matches('/')
    });
}
