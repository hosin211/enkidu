//! Item 2.10: what real programs printed, recorded once in a pty of 80 by 24, replayed through enkidu-term, must give the
//! screen those programs show: vim editing a file, less paging, htop, and the fake Claude working and interrupted.
//! The recordings are in `tests/recordings`; `record_the_programs` makes them again (vim, less and htop from this
//! Mac, the fake Claude from enkidu-testkit), run by hand:
//!
//!   cargo test -p enkidu-term --test replay -- --ignored --nocapture

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use enkidu_term::{GridSize, Palette, Screen, Terminal};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};

const COLS: u16 = 80;
const ROWS: u16 = 24;

fn recordings() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/recordings")
}

/// The file vim edits.
const NOTES: &str = "# Notes\nfirst line\nsecond line\nthird line\n";

/// What `program` prints in a pty of 80 by 24: it starts, `steps` are typed (each after its wait), and the output up
/// to `settle` after the last step is kept; then `quit` is typed and the program ends.
fn record(
    program: &str,
    args: &[&str],
    dir: &Path,
    env: &[(&str, &str)],
    steps: &[(u64, &[u8])],
    settle: u64,
    quit: &[u8],
) -> Vec<u8> {
    let pair = native_pty_system()
        .openpty(PtySize { rows: ROWS, cols: COLS, pixel_width: 0, pixel_height: 0 })
        .expect("a pty");
    let mut cmd = CommandBuilder::new(program);
    cmd.args(args);
    cmd.cwd(dir);
    cmd.env_clear();
    // a home of its own, so no program writes its settings or history into yours
    for (k, v) in [
        ("PATH", "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin"),
        ("HOME", &dir.to_string_lossy()),
        ("TERM", "xterm-256color"),
        ("LANG", "en_US.UTF-8"),
    ] {
        cmd.env(k, v);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = pair.slave.spawn_command(cmd).expect("the program starts");
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().expect("its output");
    let mut writer = pair.master.take_writer().expect("its input");
    let out = Arc::new(Mutex::new(Vec::new()));
    let reading = {
        let out = out.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                out.lock().unwrap().extend_from_slice(&buf[..n]);
            }
        })
    };
    for (wait, keys) in steps {
        std::thread::sleep(Duration::from_millis(*wait));
        writer.write_all(keys).unwrap();
        writer.flush().unwrap();
    }
    std::thread::sleep(Duration::from_millis(settle));
    let kept = out.lock().unwrap().clone();
    writer.write_all(quit).unwrap();
    writer.flush().unwrap();
    let _ = child.wait();
    drop(pair.master);
    let _ = reading.join();
    kept
}

#[test]
#[ignore = "records the programs again, by hand: cargo test -p enkidu-term --test replay -- --ignored --nocapture"]
fn record_the_programs() {
    let dir = std::env::temp_dir().join(format!("enkidu-replay-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("notes.md"), NOTES).unwrap();
    let numbers: String = (1..=100).map(|n| format!("line {n}\n")).collect();
    std::fs::write(dir.join("numbers.txt"), numbers).unwrap();
    let fake = Path::new(env!("CARGO_MANIFEST_DIR")).join("../enkidu-testkit/fakes/fake-claude.mjs");
    let fake = fake.canonicalize().expect("the fake Claude");
    let made = [
        // no vimrc, no viminfo: vim as it comes; down two lines, " hello" at the end of the line, Esc
        (
            "vim",
            record(
                "vim",
                &["-u", "NONE", "-i", "NONE", "-N", "notes.md"],
                &dir,
                &[],
                &[(1200, b"jjA hello\x1b")],
                800,
                b":q!\r",
            ),
        ),
        // no LESS options, no history file; a page forward
        (
            "less",
            record("less", &["numbers.txt"], &dir, &[("LESS", ""), ("LESSHISTFILE", "-")], &[(900, b" ")], 700, b"q"),
        ),
        ("htop", record("htop", &[], &dir, &[], &[], 1800, b"q")),
        // the fake Claude works half a second and stops without a hook, as an interrupted Claude does
        (
            "fake-claude",
            record(
                "node",
                &[&fake.to_string_lossy(), "--session-id", "s-1"],
                &dir,
                &[],
                &[(1200, b"spin 0.5\r")],
                1500,
                b"exit\r",
            ),
        ),
    ];
    std::fs::create_dir_all(recordings()).unwrap();
    for (name, bytes) in made {
        assert!(!bytes.is_empty(), "{name} printed nothing");
        std::fs::write(recordings().join(format!("{name}.bin")), &bytes).unwrap();
        println!("{name}: {} bytes", bytes.len());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A recording fed to a terminal of 80 by 24 in chunks, as a pty would hand it over.
fn replay(name: &str) -> Screen {
    replay_in(name, 1000)
}

fn replay_in(name: &str, chunk: usize) -> Screen {
    let bytes = std::fs::read(recordings().join(format!("{name}.bin"))).expect("the recording");
    let t = Terminal::kept(GridSize::new(COLS, ROWS), Palette::enkidu());
    for part in bytes.chunks(chunk) {
        t.feed(part);
    }
    t.screen()
}

#[test]
fn a_recording_fed_a_byte_at_a_time_gives_the_same_screen() {
    // a pty may split an escape sequence anywhere
    for name in ["vim", "less", "htop", "fake-claude"] {
        let (whole, bytewise) = (replay_in(name, 1000), replay_in(name, 1));
        assert_eq!(whole.cells, bytewise.cells, "{name}");
        assert_eq!(whole.cursor_cell, bytewise.cursor_cell, "{name}");
    }
}

#[test]
fn vim_shows_the_file_with_the_edit_tildes_below_and_the_cursor_on_the_edit() {
    let s = replay("vim");
    assert!(s.modes.alt_screen, "vim draws on the alternate screen");
    assert_eq!(s.row_text(0), "# Notes");
    assert_eq!(s.row_text(1), "first line");
    assert_eq!(s.row_text(2), "second line hello");
    assert_eq!(s.row_text(3), "third line");
    for row in 4..23 {
        assert_eq!(s.row_text(row), "~", "row {row}:\n{}", s.text());
    }
    // after Esc the cursor stands on the last letter typed
    let c = s.cursor_cell.expect("a cursor");
    assert_eq!(c, (2, 16), "\n{}", s.text());
}

#[test]
fn less_shows_the_second_page_and_its_prompt() {
    let s = replay("less");
    assert!(s.modes.alt_screen, "less pages on the alternate screen");
    // a page forward: the window\'s 23 lines after the first 23
    for row in 0..23 {
        assert_eq!(s.row_text(row), format!("line {}", row + 24), "row {row}:\n{}", s.text());
    }
    assert_eq!(s.row_text(23), ":");
}

#[test]
fn htop_shows_its_meters_columns_processes_and_keys() {
    let s = replay("htop");
    assert!(s.modes.alt_screen);
    let text = s.text();
    assert!(text.contains("Tasks:") && text.contains("Load average:"), "\n{text}");
    let header = (0..24).find(|&r| s.row_text(r).trim_start().starts_with("PID")).expect("the column header");
    let columns = s.row_text(header);
    for column in ["USER", "PRI", "RES", "CPU%", "MEM%", "TIME+", "Command"] {
        assert!(columns.contains(column), "{columns}");
    }
    // the header row is drawn on a colored band, and processes follow it
    assert!(s.row(header).iter().any(|c| !c.default_bg), "the column header has its band");
    assert!(s.row_text(header + 1).split_whitespace().next().is_some_and(|pid| pid.parse::<u32>().is_ok()));
    assert!(s.row_text(23).starts_with("F1Help"), "{}", s.row_text(23));
}

#[test]
fn the_fake_claude_shows_its_banner_the_prompt_one_spinner_line_and_the_interruption() {
    let s = replay("fake-claude");
    let text = s.text();
    assert_eq!(s.row_text(0), "Fake Claude · session s-1 · new");
    assert!(s.row(0)[0].bold, "its name is bold");
    assert_eq!(s.row_text(1), "args: --session-id s-1");
    assert_eq!(s.row_text(2), "> spin 0.5", "the prompt and what was typed");
    // the spinner turned on one row, rewritten in place, never a new row each frame
    let working: Vec<String> = (0..24).map(|r| s.row_text(r)).filter(|r| r.contains("Working…")).collect();
    assert_eq!(working.len(), 1, "\n{text}");
    assert!(working[0].ends_with("Working… (esc to interrupt)"), "{}", working[0]);
    assert_eq!(s.row_text(4), "  ⎿  Interrupted · What should Claude do instead?", "\n{text}");
    assert_eq!(s.row_text(5), ">", "the prompt again");
    assert_eq!(s.cursor_cell, Some((5, 2)));
}
