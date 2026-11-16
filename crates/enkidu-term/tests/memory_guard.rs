//! Item 2.9: the guard on memory, with phase 0's budget: at most 15 MB for each
//! terminal more. A process of its own, so nothing another test freed or kept moves the numbers; it runs only in
//! release builds (`sh scripts/check.sh`).

use std::sync::Arc;
use std::time::{Duration, Instant};

use enkidu_term::{GridSize, Palette, Spawn, Terminal, base_env};

fn start(script: &str) -> Terminal {
    let spawn = Spawn {
        program: Some(("/bin/sh".into(), vec!["-c".into(), script.into()])),
        cwd: std::env::temp_dir(),
        env: base_env(),
    };
    Terminal::spawn(spawn, GridSize::new(120, 36), Palette::enkidu(), Arc::new(|| {})).expect("the program starts")
}

fn ready(t: &Terminal, word: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !t.screen().text().contains(word) {
        assert!(Instant::now() < deadline, "timed out; the screen shows:\n{}", t.screen().text());
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// This process's memory as macOS counts it for Activity Monitor (its physical footprint).
fn footprint() -> u64 {
    let mut info: libc::rusage_info_v2 = unsafe { std::mem::zeroed() };
    // SAFETY: the buffer is a rusage_info_v2, the flavor asked for.
    let done = unsafe {
        libc::proc_pid_rusage(
            libc::getpid(),
            libc::RUSAGE_INFO_V2,
            (&mut info as *mut libc::rusage_info_v2).cast::<libc::rusage_info_t>(),
        )
    };
    assert_eq!(done, 0, "proc_pid_rusage");
    info.ri_phys_footprint
}

#[test]
#[cfg_attr(debug_assertions, ignore = "a guard on memory: it runs in release builds (sh scripts/check.sh)")]
fn each_terminal_costs_at_most_15_mb() {
    // one terminal first, so what the first one alone sets up (threads, the pty system) isn't counted for each
    let first = start("seq 1 60; echo ready-first; read x");
    ready(&first, "ready-first");
    let before = footprint();
    let five: Vec<Terminal> = (0..5).map(|i| start(&format!("seq 1 60; echo ready-{i}; read x"))).collect();
    for (i, t) in five.iter().enumerate() {
        ready(t, &format!("ready-{i}"));
    }
    let more = footprint().saturating_sub(before);
    let each = more / five.len() as u64;
    println!("memory: {:.1} MB for five terminals more, {:.1} MB each", more as f64 / 1e6, each as f64 / 1e6);
    assert!(each <= 15_000_000, "each terminal takes {:.1} MB, the budget is 15 MB", each as f64 / 1e6);
}
