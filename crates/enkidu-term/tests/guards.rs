//! Item 2.9: the guards, with phase 0's budgets: 100 MB printed in 1.5 s at most
//! while typing into the terminal beside it comes back within 10 ms (the memory guard is in memory_guard.rs, a process
//! of its own). They time real work, so they run only in release builds: `sh scripts/check.sh` runs them there.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use enkidu_term::{GridSize, Palette, Spawn, Terminal, base_env};

/// Each guard wants the Mac to itself.
static ALONE: Mutex<()> = Mutex::new(());

fn start(script: &str, size: GridSize) -> Terminal {
    let spawn = Spawn {
        program: Some(("/bin/sh".into(), vec!["-c".into(), script.into()])),
        cwd: std::env::temp_dir(),
        env: base_env(),
    };
    Terminal::spawn(spawn, size, Palette::enkidu(), Arc::new(|| {})).expect("the program starts")
}

fn wait_for(t: &Terminal, secs: u64, seen: impl Fn(&str) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while !seen(&t.screen().text()) {
        assert!(Instant::now() < deadline, "timed out; the screen shows:\n{}", t.screen().text());
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// One run of the flood with typing beside it: how long 100 MB took, how many bytes were printed, and how long the
/// middle and the slowest letter took to come back.
fn flood_with_typing() -> (Duration, u64, Duration, Duration) {
    // phase 0's flood: colored lines with an error word in them, 100 MB in all, once Enter is pressed
    let flood = r#"read go; perl -e '$l = "\e[32m" . ("0123456789abcdef" x 4) . "\e[0m \e[1;31mERR\e[0m line\n"; $n = int(100_000_000 / length($l) / 1000); for (1..$n) { print $l x 1000 }'; echo; echo FLOOD-DONE; read x"#;
    let left = start(flood, GridSize::new(120, 40));
    let right = start("exec /bin/cat", GridSize::new(80, 24));
    // both ready: cat echoes what it is given
    right.send(b"ready\r".to_vec());
    wait_for(&right, 10, |s| s.matches("ready").count() >= 2);
    std::thread::sleep(Duration::from_millis(200));

    let before = left.parsed();
    let started = Instant::now();
    left.send(b"\r".to_vec());
    let mut round_trips = Vec::new();
    let mut letter = 0u8;
    while !left.screen().text().contains("FLOOD-DONE") {
        assert!(started.elapsed() < Duration::from_secs(30), "the flood never ended");
        // a letter typed into cat, timed until its echo is on the screen beside the flood
        let parsed = right.parsed();
        let typed = Instant::now();
        right.send(vec![b'a' + letter % 26]);
        letter += 1;
        while right.parsed() == parsed {
            assert!(typed.elapsed() < Duration::from_secs(2), "a letter typed beside the flood never came back");
            std::hint::spin_loop();
        }
        round_trips.push(typed.elapsed());
        std::thread::sleep(Duration::from_millis(40));
    }
    // the flood ended with the parser's last chunk, not when this loop happened to look
    let took = left.last_output().expect("output").saturating_duration_since(started);
    let printed = left.parsed() - before;
    round_trips.sort();
    let slowest = *round_trips.last().expect("letters were typed");
    let middle = round_trips[round_trips.len() / 2];
    println!(
        "flood: {:.1} MB in {} ms; typing beside it: {} letters, back in {:.2} ms (middle), {:.2} ms at most",
        printed as f64 / 1e6,
        took.as_millis(),
        round_trips.len(),
        middle.as_secs_f64() * 1e3,
        slowest.as_secs_f64() * 1e3
    );
    (took, printed, middle, slowest)
}

#[test]
#[cfg_attr(debug_assertions, ignore = "a timed guard: it runs in release builds (sh scripts/check.sh)")]
fn a_hundred_megabytes_print_in_a_second_and_a_half_while_typing_beside_it_comes_back_within_10_ms() {
    let _alone = ALONE.lock().unwrap_or_else(|e| e.into_inner());
    // The Mac is often busy with other work (12 Nov 16:35: a load of 16 on 10 cores, and one letter
    // of 19 came back in 27.9 ms while the middle one took 0.03 ms). Other work only ever makes these numbers worse,
    // so one run inside both budgets shows the code can do it; three runs outside them fail the guard.
    let mut runs = Vec::new();
    for _ in 0..3 {
        let (took, printed, middle, slowest) = flood_with_typing();
        assert!(printed >= 100_000_000, "{printed} bytes");
        // a letter that takes the middle of the budget every time is no accident of scheduling
        assert!(middle <= Duration::from_millis(10), "the middle letter took {middle:?}, the budget is 10 ms");
        if took <= Duration::from_millis(1500) && slowest <= Duration::from_millis(10) {
            return;
        }
        runs.push((took, slowest));
    }
    panic!("three runs, none inside the budgets (100 MB in 1.5 s, a letter back in 10 ms): {runs:?}");
}
