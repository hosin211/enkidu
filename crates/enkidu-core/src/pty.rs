//! Real terminals (PTYs). Output goes to the UI as `pty-output` events and into a buffer, so a
//! reloaded UI can repaint what was already on screen.

use anyhow::{anyhow, Context, Result};
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use crate::events::{Event, Events};

use crate::state::Shared;

const BUFFER_LIMIT: usize = 400_000;

/// After Esc or Ctrl+C, how long Claude's terminal must stay quiet before Claude counts as back at its prompt. While
/// Claude works, its spinner and timer print several times a second, so a second and a half of quiet is a stop.
const QUIET_AFTER_INTERRUPT: Duration = Duration::from_millis(1500);
/// How long after Esc or Ctrl+C to keep checking. A key that didn't interrupt anything leaves Claude printing.
const WATCH_AFTER_INTERRUPT: Duration = Duration::from_secs(20);

/// Text a Claude was started with goes into its prompt once its session has started (its SessionStart hook), at
/// least this long after, so the hook's answer is back with Claude and it is drawing its prompt.
const SETTLE_BEFORE_DRAFT: Duration = Duration::from_millis(600);
/// ... and once its terminal has been quiet this long: the prompt is drawn.
const QUIET_BEFORE_DRAFT: Duration = Duration::from_millis(400);
/// Past this, the text goes in anyway: the session has started, Claude just keeps printing (a long banner, MCP news).
const WAIT_FOR_DRAFT: Duration = Duration::from_secs(8);

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TerminalInfo {
    pub id: String,
    pub workspace_id: String,
    /// "claude" or "shell"
    pub kind: String,
    pub title: String,
    pub exited: bool,
    pub exit_code: Option<i32>,
}

/// What the terminal printed recently, and the byte count up to its end.
#[derive(Serialize, Clone, Default, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Buffered {
    pub data: String,
    pub end: u64,
}

struct Term {
    info: TerminalInfo,
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    buffer: Arc<Mutex<Buffered>>,
    /// When the program last printed anything.
    last_output: Arc<Mutex<Instant>>,
    /// Text to type into Claude's prompt once its session has started, and whether Enter follows (only a scheduled
    /// workflow's run, which you set up and read beforehand; anything else waits for your Enter).
    draft: Option<(String, bool)>,
}

/// Esc or Ctrl+C reached a Claude terminal: Claude may have stopped without saying so.
pub struct Interrupted {
    pub workspace_id: String,
    last_output: Arc<Mutex<Instant>>,
    /// Just before the key went in, so Claude's answer to it always counts as after.
    pressed: Instant,
}

#[derive(Debug, PartialEq)]
enum AfterInterrupt {
    /// Claude is still printing: look again soon.
    Wait,
    /// Quiet: Claude is back at its prompt.
    Settle,
    /// Nothing to do: a hook already moved the status on, or the key didn't stop anything.
    GiveUp,
}

/// Claude Code runs no hook when you interrupt it (Esc, Ctrl+C), not even Stop, so a workspace would keep saying
/// "working" (or "needs you", when Esc turned down a permission) while Claude waits at its prompt. Interrupted,
/// Claude redraws once ("Interrupted · What should Claude do instead?") and then prints nothing: output after the
/// key, then a moment of quiet, means it stopped. A key that changed nothing either prints nothing (a question
/// still on screen) or keeps the spinner going (still working), so it never counts.
fn after_interrupt(status: Option<&str>, printed_since_key: bool, quiet: Duration, waited: Duration) -> AfterInterrupt {
    if !matches!(status, Some("working" | "needs_you")) {
        return AfterInterrupt::GiveUp;
    }
    if printed_since_key && quiet >= QUIET_AFTER_INTERRUPT {
        return AfterInterrupt::Settle;
    }
    if waited >= WATCH_AFTER_INTERRUPT {
        return AfterInterrupt::GiveUp;
    }
    AfterInterrupt::Wait
}

/// The bytes a terminal sends for Esc and for Ctrl+C. Arrow keys and Option-combinations start with Esc too, but
/// never come alone.
fn is_interrupt_key(data: &str) -> bool {
    data == "\x1b" || data == "\x03"
}

/// Watches a Claude terminal after Esc or Ctrl+C and sets the workspace idle once Claude has gone quiet.
pub fn settle_after_interrupt(app: Events, shared: Arc<Shared>, interrupted: Interrupted) {
    let pressed = interrupted.pressed;
    let _ = std::thread::Builder::new().name("interrupt-watch".into()).spawn(move || loop {
        std::thread::sleep(Duration::from_millis(250));
        let last = *interrupted.last_output.lock().unwrap_or_else(|e| e.into_inner());
        let status = shared.status(&interrupted.workspace_id);
        match after_interrupt(status.as_deref(), last > pressed, last.elapsed(), pressed.elapsed()) {
            AfterInterrupt::Wait => continue,
            AfterInterrupt::Settle => {
                shared.set_status(&app, &interrupted.workspace_id, "idle");
                return;
            }
            AfterInterrupt::GiveUp => return,
        }
    });
}

#[derive(Default)]
pub struct PtyManager {
    terms: Mutex<HashMap<String, Term>>,
}

pub struct SpawnSpec {
    pub workspace_id: String,
    pub kind: String,
    pub title: String,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    pub cols: u16,
    pub rows: u16,
    /// Text to type into Claude's prompt once it has started (see `type_draft_when_ready`)
    pub draft: Option<String>,
    /// Enter follows the draft: a scheduled workflow's run (you set it up and read it beforehand)
    pub send: bool,
}

/// Environment for programs started in a terminal: the app's own env without anything that
/// would make claude think it runs inside another claude, plus terminal basics.
pub fn base_env(path: &str) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = std::env::vars()
        .filter(|(k, _)| {
            !(k == "CLAUDECODE"
                || k.starts_with("CLAUDE_CODE_")
                || k == "CLAUDE_PROJECT_DIR"
                || k.starts_with(crate::places::ENV_PREFIX)
                || k.starts_with("VSCODE_")
                || k == "TERM_PROGRAM"
                || k == "TERM_PROGRAM_VERSION")
        })
        .collect();
    let mut set = |key: &str, value: &str| {
        env.retain(|(k, _)| k != key);
        env.push((key.to_string(), value.to_string()));
    };
    set("PATH", path);
    set("TERM", "xterm-256color");
    set("COLORTERM", "truecolor");
    set("TERM_PROGRAM", "Enkidu");
    if std::env::var("LANG").map(|l| l.is_empty()).unwrap_or(true) {
        set("LANG", "en_US.UTF-8");
    }
    env
}

impl PtyManager {
    fn terms(&self) -> std::sync::MutexGuard<'_, HashMap<String, Term>> {
        self.terms.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn spawn(&self, app: Events, shared: Arc<Shared>, spec: SpawnSpec) -> Result<TerminalInfo> {
        let pair = native_pty_system()
            .openpty(PtySize { rows: spec.rows.max(5), cols: spec.cols.max(20), pixel_width: 0, pixel_height: 0 })
            .context("Can't open a terminal")?;
        let mut cmd = CommandBuilder::new(&spec.program);
        cmd.args(&spec.args);
        cmd.cwd(&spec.cwd);
        cmd.env_clear();
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
        let mut child = pair
            .slave
            .spawn_command(cmd)
            .with_context(|| format!("Can't start {}", spec.program.display()))?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let killer = child.clone_killer();
        let id = uuid::Uuid::new_v4().simple().to_string()[..12].to_string();
        let info = TerminalInfo {
            id: id.clone(),
            workspace_id: spec.workspace_id.clone(),
            kind: spec.kind.clone(),
            title: spec.title.clone(),
            exited: false,
            exit_code: None,
        };
        let buffer = Arc::new(Mutex::new(Buffered::default()));
        let last_output = Arc::new(Mutex::new(Instant::now()));
        let draft = spec.draft.clone().filter(|t| !t.trim().is_empty()).map(|t| (t, spec.send));
        self.terms().insert(
            id.clone(),
            Term {
                info: info.clone(),
                writer,
                master: pair.master,
                killer,
                buffer: buffer.clone(),
                last_output: last_output.clone(),
                draft,
            },
        );

        let out_app = app.clone();
        let out_id = id.clone();
        std::thread::spawn(move || {
            let mut chunk = [0u8; 16 * 1024];
            let mut pending: Vec<u8> = Vec::new();
            loop {
                let n = match reader.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                pending.extend_from_slice(&chunk[..n]);
                let text = take_utf8(&mut pending);
                if text.is_empty() {
                    continue;
                }
                *last_output.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
                // append and emit under one lock, so a chunk is either in the buffer or not yet
                let mut buf = buffer.lock().unwrap_or_else(|e| e.into_inner());
                let start = buf.end;
                buf.end += text.len() as u64;
                buf.data.push_str(&text);
                if buf.data.len() > BUFFER_LIMIT {
                    let mut cut = buf.data.len() - BUFFER_LIMIT;
                    while !buf.data.is_char_boundary(cut) {
                        cut += 1;
                    }
                    buf.data.drain(..cut);
                }
                out_app.send(Event::PtyOutput { id: out_id.clone(), data: text, start, end: buf.end });
                drop(buf);
            }
        });

        let wait_id = id;
        let ws_id = spec.workspace_id;
        let is_claude = spec.kind == "claude";
        std::thread::spawn(move || {
            let code = child.wait().ok().map(|s| s.exit_code() as i32);
            shared.pty.mark_exited(&wait_id, code);
            if is_claude {
                shared.set_status(&app, &ws_id, "idle");
            }
            app.send(Event::PtyExit { id: wait_id, code });
        });
        Ok(info)
    }

    fn mark_exited(&self, id: &str, code: Option<i32>) {
        if let Some(term) = self.terms().get_mut(id) {
            term.info.exited = true;
            term.info.exit_code = code;
        }
    }

    /// Types into a terminal. Esc or Ctrl+C typed into Claude comes back as `Interrupted`, to be watched.
    pub fn write(&self, id: &str, data: &str) -> Result<Option<Interrupted>> {
        let mut terms = self.terms();
        let term = terms.get_mut(id).ok_or_else(|| anyhow!("That terminal is gone"))?;
        let pressed = Instant::now();
        term.writer.write_all(data.as_bytes())?;
        term.writer.flush()?;
        let interrupt = term.info.kind == "claude" && !term.info.exited && is_interrupt_key(data);
        Ok(interrupt.then(|| Interrupted {
            workspace_id: term.info.workspace_id.clone(),
            last_output: term.last_output.clone(),
            pressed,
        }))
    }

    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> Result<()> {
        let terms = self.terms();
        let term = terms.get(id).ok_or_else(|| anyhow!("That terminal is gone"))?;
        term.master.resize(PtySize { rows: rows.max(5), cols: cols.max(20), pixel_width: 0, pixel_height: 0 })?;
        Ok(())
    }

    /// Ends the program and forgets the terminal.
    pub fn close(&self, id: &str) {
        if let Some(mut term) = self.terms().remove(id) {
            if !term.info.exited {
                let _ = term.killer.kill();
            }
        }
    }

    pub fn close_workspace(&self, workspace_id: &str) {
        let ids: Vec<String> =
            self.terms().values().filter(|t| t.info.workspace_id == workspace_id).map(|t| t.info.id.clone()).collect();
        for id in ids {
            self.close(&id);
        }
    }

    pub fn list(&self, workspace_id: &str) -> Vec<TerminalInfo> {
        let mut list: Vec<TerminalInfo> =
            self.terms().values().filter(|t| t.info.workspace_id == workspace_id).map(|t| t.info.clone()).collect();
        list.sort_by_key(|t| (t.kind != "claude", t.id.clone()));
        list
    }

    /// The text the running Claude of a workspace was started with, taken: it goes in once.
    fn take_draft(&self, workspace_id: &str) -> Option<(String, (String, bool), Arc<Mutex<Instant>>)> {
        let mut terms = self.terms();
        let term = terms
            .values_mut()
            .find(|t| t.info.workspace_id == workspace_id && t.info.kind == "claude" && !t.info.exited && t.draft.is_some())?;
        let draft = term.draft.take()?;
        Some((term.info.id.clone(), draft, term.last_output.clone()))
    }

    pub fn running_claude(&self, workspace_id: &str) -> bool {
        self.terms().values().any(|t| t.info.workspace_id == workspace_id && t.info.kind == "claude" && !t.info.exited)
    }

    /// The terminal of the workspace's running Claude.
    pub fn claude_terminal(&self, workspace_id: &str) -> Option<String> {
        self.terms().values().find(|t| t.info.workspace_id == workspace_id && t.info.kind == "claude" && !t.info.exited).map(|t| t.info.id.clone())
    }

    pub fn buffer(&self, id: &str) -> Option<Buffered> {
        self.terms().get(id).map(|t| t.buffer.lock().unwrap_or_else(|e| e.into_inner()).clone())
    }

    pub fn close_all(&self) {
        let ids: Vec<String> = self.terms().keys().cloned().collect();
        for id in ids {
            self.close(&id);
        }
    }
}

/// Takes the complete UTF-8 text from the front of `pending`; an unfinished character waits
/// for the next read.
fn take_utf8(pending: &mut Vec<u8>) -> String {
    match std::str::from_utf8(pending) {
        Ok(s) => {
            let s = s.to_string();
            pending.clear();
            s
        }
        Err(e) if e.error_len().is_none() => {
            let valid = e.valid_up_to();
            let s = String::from_utf8_lossy(&pending[..valid]).into_owned();
            pending.drain(..valid);
            s
        }
        Err(_) => {
            let s = String::from_utf8_lossy(pending).into_owned();
            pending.clear();
            s
        }
    }
}

/// Whether the text a Claude was started with goes in now, `waited` after its session started and `quiet` after it
/// last printed.
fn draft_due(quiet: Duration, waited: Duration) -> bool {
    (waited >= SETTLE_BEFORE_DRAFT && quiet >= QUIET_BEFORE_DRAFT) || waited >= WAIT_FOR_DRAFT
}

/// How long after a paste Enter follows, when it does: Claude has taken the paste in by then.
pub const ENTER_AFTER_PASTE: Duration = Duration::from_millis(700);

/// Claude said its session started (its SessionStart hook): the text it was started with goes into its prompt as
/// one paste once it has drawn the prompt, never with Enter: you read it there and send it yourself (a scheduled
/// workflow's run, which you set up beforehand, is sent). Claude runs the hook only once it trusts the folder, so the
/// text never lands in that question.
pub fn type_draft_when_ready(shared: Arc<Shared>, workspace_id: &str) {
    let Some((id, (draft, send), last_output)) = shared.pty.take_draft(workspace_id) else { return };
    let started = Instant::now();
    let _ = std::thread::Builder::new().name("claude-draft".into()).spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(100));
            let quiet = last_output.lock().unwrap_or_else(|e| e.into_inner()).elapsed();
            if draft_due(quiet, started.elapsed()) {
                break;
            }
        }
        if let Err(e) = paste(&shared, &id, &draft, send) {
            eprintln!("enkidu: couldn't type into Claude's prompt: {e:#}");
        }
    });
}

/// Types text into a Claude's prompt as one paste; Enter follows a moment later only when `send` (a scheduled
/// workflow's run).
pub fn paste(shared: &Shared, id: &str, text: &str, send: bool) -> Result<()> {
    shared.pty.write(id, &format!("\x1b[200~{text}\x1b[201~"))?;
    if send {
        std::thread::sleep(ENTER_AFTER_PASTE);
        shared.pty.write(id, "\r")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_the_text_claude_was_started_with_once_it_has_drawn_its_prompt() {
        let ms = Duration::from_millis;
        assert!(!draft_due(ms(3000), ms(200)), "just after the session started, quiet or not: the hook's answer is still on its way");
        assert!(!draft_due(ms(100), ms(1500)), "still printing");
        assert!(draft_due(ms(450), ms(900)), "quiet a moment, after the session started");
        assert!(draft_due(ms(20), ms(8000)), "printing on and on: in it goes anyway");
    }

    #[test]
    fn utf8_split_across_reads_is_kept_whole() {
        let bytes = "héllo ✓".as_bytes();
        let (a, b) = bytes.split_at(bytes.len() - 1);
        let mut pending = a.to_vec();
        let first = take_utf8(&mut pending);
        assert_eq!(first, "héllo ");
        pending.extend_from_slice(b);
        assert_eq!(take_utf8(&mut pending), "✓");
        assert!(pending.is_empty());
    }

    #[test]
    fn only_esc_and_ctrl_c_alone_are_interrupts() {
        assert!(is_interrupt_key("\x1b"));
        assert!(is_interrupt_key("\x03"));
        // arrows, Option+key and Shift+Enter (a new line in Claude's prompt) start with Esc but are not it
        for other in ["\x1b[A", "\x1bb", "\x1b\r", "q", "\x1b\x1b"] {
            assert!(!is_interrupt_key(other), "{other:?}");
        }
    }

    #[test]
    fn an_interrupted_claude_goes_idle_once_it_has_redrawn_and_gone_quiet() {
        let ms = Duration::from_millis;
        use AfterInterrupt::*;
        // still printing (the spinner of a Claude that kept working), or not answered yet: look again
        assert_eq!(after_interrupt(Some("working"), true, ms(200), ms(800)), Wait);
        assert_eq!(after_interrupt(Some("working"), false, ms(4000), ms(300)), Wait);
        // redrew after the key, then quiet: back at its prompt
        assert_eq!(after_interrupt(Some("working"), true, ms(1600), ms(1900)), Settle);
        assert_eq!(after_interrupt(Some("needs_you"), true, ms(1500), ms(1700)), Settle);
        // Esc that changed nothing on a question: no redraw, so the question stays "needs you"
        assert_eq!(after_interrupt(Some("needs_you"), false, ms(9000), ms(5000)), Wait);
        assert_eq!(after_interrupt(Some("needs_you"), false, ms(30000), ms(20000)), GiveUp);
        // a hook already said what happened, or Claude was not busy: leave it
        for status in [Some("done"), Some("idle"), None] {
            assert_eq!(after_interrupt(status, true, ms(5000), ms(5000)), GiveUp);
        }
    }

    #[test]
    fn base_env_drops_nested_claude_markers() {
        std::env::set_var("CLAUDECODE", "1");
        std::env::set_var("CLAUDE_CODE_ENTRYPOINT", "cli");
        let env = base_env("/usr/bin:/bin");
        assert!(!env.iter().any(|(k, _)| k == "CLAUDECODE" || k == "CLAUDE_CODE_ENTRYPOINT"));
        assert!(env.iter().any(|(k, v)| k == "PATH" && v == "/usr/bin:/bin"));
        assert!(env.iter().any(|(k, v)| k == "TERM" && v == "xterm-256color"));
    }
}
