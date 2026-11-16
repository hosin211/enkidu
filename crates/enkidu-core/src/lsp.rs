//! A language server, asked where a name is defined (phase 12).
//!
//! The app's own rules (`goto`) answer from the text of files. A language server knows more: what a variable is,
//! what a class gets from the classes it is built on in a library. So where a server
//! is there for a file's language, it is asked first, with a short limit, and the rules answer only when it has
//! nothing to say in time.
//!
//! A server is a program of its own, spoken to over its standard input and output (the Language Server Protocol).
//! One runs for each workspace that was asked about: started on a thread of its own at the first question (which
//! the rules answer), kept for the few workspaces asked last, and stopped when nobody asked for a while.
//!
//! Nobody ever waits for a server longer than the limit. What is written to it goes through a thread of its own (a
//! busy server doesn't read, and a file's text is more than a pipe holds). While it still owes the answer to a
//! question that was given up on, it is not asked anything: the rules answer at once, and nothing piles up for it
//! to work through. Measured on 14 Nov on a workspace of 40,000 files: ty's first answer took 1.1 s, its later
//! ones 1.7 ms.
//!
//! A server is started in a folder of the app's own data, never in the workspace, without the app's own variables, and
//! it is only ever asked to read. What it is told: the text of the file a question is about, as it is on disk, and
//! which files the watcher saw change.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// How long an answer may take before the rules answer instead. Measured on 14 Nov on two real projects: 95 of 100
/// answers of the Rust servers came in under 5 ms.
pub const ANSWER_WAIT: Duration = Duration::from_millis(150);
/// How long a server has to say hello after its start.
const HELLO_WAIT: Duration = Duration::from_secs(20);
/// A server nobody asked for this long is stopped (it holds up to a few hundred MB once it has read a project).
const IDLE: Duration = Duration::from_secs(600);
/// The most servers kept running: the workspaces asked last.
const KEEP: usize = 3;
/// More changed files than this since the last question, and the server is started afresh instead of being told
/// each of them (a merge, a checkout).
const MAX_TOLD: usize = 200;
/// A server that didn't start, or died, is tried again after this long.
const RETRY: Duration = Duration::from_secs(60);
/// A server that owes an answer for this long is taken for stuck, and started afresh.
const STUCK: Duration = Duration::from_secs(20);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// The program that is a language server, and how it is started.
#[derive(Clone, Debug, PartialEq)]
pub enum Program {
    /// A program and its arguments.
    Direct(PathBuf, Vec<String>),
    /// A command line for the shell.
    Shell(String),
}

/// The file in the app's data folder that names the Python server by hand: a command line (`pyrefly lsp`), or
/// `off` (or nothing) for none. There is no field for it in Settings: it is the way out when the carried server
/// misbehaves on a project, without a new build.
pub const PYTHON_SERVER_FILE: &str = "python-server.txt";

/// The language server for Python on this machine. In this order: what `ENKIDU_PYTHON_SERVER` names (a command
/// line; empty: none), what the file [`PYTHON_SERVER_FILE`] in the app's data folder names, and else the `ty` that
/// stands beside the app's own program, where the install puts it.
pub fn python_program(data_dir: &Path) -> Option<Program> {
    let named = |command: String| match command.trim() {
        "" | "off" => None,
        command => Some(Program::Shell(command.to_string())),
    };
    if let Ok(command) = std::env::var("ENKIDU_PYTHON_SERVER") {
        return named(command);
    }
    if let Ok(command) = std::fs::read_to_string(data_dir.join(PYTHON_SERVER_FILE)) {
        return named(command);
    }
    let beside = std::env::current_exe().ok()?.parent()?.join("ty");
    beside.is_file().then(|| Program::Direct(beside, vec!["server".to_string()]))
}

/// Hands a message to the thread that writes to the server. It never waits for the server to read.
fn send(out: &Sender<Vec<u8>>, message: &Value) -> bool {
    let body = message.to_string();
    out.send(format!("Content-Length: {}\r\n\r\n{}", body.len(), body).into_bytes()).is_ok()
}

/// A running server.
struct Server {
    child: Child,
    /// What is to be written to it, in order.
    out: Sender<Vec<u8>>,
    /// Its answers to what it was asked.
    said: Receiver<Value>,
    asked: u64,
    /// The question it still owes an answer to (it was given up on, or not waited for), and since when.
    owes: Option<(u64, Instant)>,
}

impl Server {
    fn start(program: &Program, folder: &Path) -> std::io::Result<Server> {
        let mut command = match program {
            Program::Direct(path, args) => {
                let mut command = Command::new(path);
                command.args(args);
                command
            }
            Program::Shell(line) => {
                let mut command = Command::new("sh");
                command.arg("-c").arg(line);
                command
            }
        };
        std::fs::create_dir_all(folder)?;
        command.current_dir(folder).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        // the app's own variables are none of a server's business
        for (name, _) in std::env::vars() {
            if name.starts_with("ENKIDU_") {
                command.env_remove(name);
            }
        }
        let mut child = command.spawn()?;
        let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            return Err(std::io::Error::other("the server has no input or output"));
        };
        // the writer: whatever is to be said to the server waits here, not in whoever says it. It ends, and closes
        // the server's input, when the server is dropped
        let (out, to_write) = channel::<Vec<u8>>();
        std::thread::spawn(move || {
            while let Ok(bytes) = to_write.recv() {
                if stdin.write_all(&bytes).is_err() || stdin.flush().is_err() {
                    return;
                }
            }
        });
        let (tell, said) = channel();
        let answers = out.clone();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut length = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    let line = line.trim_end();
                    if line.is_empty() {
                        break;
                    }
                    if let Some(number) = line.strip_prefix("Content-Length:") {
                        length = number.trim().parse().unwrap_or(0);
                    }
                }
                let mut body = vec![0u8; length];
                if reader.read_exact(&mut body).is_err() {
                    return;
                }
                let Ok(message) = serde_json::from_slice::<Value>(&body) else { continue };
                match (message.get("method").and_then(Value::as_str), message.get("id")) {
                    // the server asks something of its own: answered at once, with nothing (its settings are the
                    // ones it starts with), so it never waits for a question of ours
                    (Some(method), Some(id)) => {
                        let result = match method {
                            "workspace/configuration" => {
                                let items = message["params"]["items"].as_array().map_or(0, Vec::len);
                                Value::Array(vec![Value::Null; items])
                            }
                            _ => Value::Null,
                        };
                        if !send(&answers, &json!({"jsonrpc": "2.0", "id": id, "result": result})) {
                            return;
                        }
                    }
                    // it tells something (what it finds wrong with a file, its progress): not asked for
                    (Some(_), None) => {}
                    (None, _) => {
                        if tell.send(message).is_err() {
                            return;
                        }
                    }
                }
            }
        });
        Ok(Server { child, out, said, asked: 0, owes: None })
    }

    fn tell(&self, method: &str, params: Value) -> bool {
        send(&self.out, &json!({"jsonrpc": "2.0", "method": method, "params": params}))
    }

    /// Asks without waiting for the answer: the server owes it from now on.
    fn put(&mut self, method: &str, params: Value) -> Option<u64> {
        self.asked += 1;
        let id = self.asked;
        send(&self.out, &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})).then_some(id)?;
        self.owes = Some((id, Instant::now()));
        Some(id)
    }

    /// Asks, and waits for the answer. None when none came in `wait` (the server then owes it, and is told that
    /// nobody waits for it any more), or the server is gone.
    fn ask(&mut self, method: &str, params: Value, wait: Duration) -> Option<Value> {
        let id = self.put(method, params)?;
        let until = Instant::now() + wait;
        loop {
            let left = until.checked_duration_since(Instant::now());
            let Some(message) = left.and_then(|left| self.said.recv_timeout(left).ok()) else {
                self.tell("$/cancelRequest", json!({"id": id}));
                return None;
            };
            if message["id"].as_u64() == Some(id) {
                self.owes = None;
                return Some(message.get("result").cloned().unwrap_or(Value::Null));
            }
        }
    }

    /// Whether it still owes the answer to an earlier question: then it is busy with that, and not asked.
    fn busy(&mut self) -> bool {
        let Some((id, _)) = self.owes else { return false };
        loop {
            match self.said.try_recv() {
                Ok(message) if message["id"].as_u64() == Some(id) => {
                    self.owes = None;
                    return false;
                }
                Ok(_) => {}
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return true,
            }
        }
    }

    fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn uri_of(path: &Path) -> String {
    let mut uri = String::from("file://");
    for byte in path.to_string_lossy().bytes() {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => uri.push(byte as char),
            other => uri.push_str(&format!("%{other:02X}")),
        }
    }
    uri
}

fn path_of(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let mut bytes = Vec::new();
    let mut raw = rest.bytes();
    while let Some(byte) = raw.next() {
        if byte == b'%' {
            let hex: Vec<u8> = raw.by_ref().take(2).collect();
            bytes.push(u8::from_str_radix(std::str::from_utf8(&hex).ok()?, 16).ok()?);
        } else {
            bytes.push(byte);
        }
    }
    String::from_utf8(bytes).ok().map(PathBuf::from)
}

/// The places an answer to "where is this defined" names: each file, as the server names it, with its line from 1.
fn locations(answer: &Value) -> Vec<(String, usize)> {
    let one = |place: &Value| {
        let uri = place.get("uri").or_else(|| place.get("targetUri"))?.as_str()?;
        let range = place.get("range").or_else(|| place.get("targetSelectionRange"))?;
        Some((uri.to_string(), range["start"]["line"].as_u64()? as usize + 1))
    };
    match answer {
        Value::Array(places) => places.iter().filter_map(one).collect(),
        Value::Object(_) => one(answer).into_iter().collect(),
        _ => Vec::new(),
    }
}

/// What is asked: a place in a file of the workspace.
pub struct Question<'a> {
    /// The file, from the workspace's folder.
    pub path: &'a str,
    /// Its text, as it is on disk now.
    pub text: &'a str,
    /// The language, as the protocol names it ("python").
    pub language: &'a str,
    /// The line, from 0, and the place in it as the protocol counts (in UTF-16 units).
    pub line: usize,
    pub character: usize,
}

impl Question<'_> {
    /// The three things said to a server for one question: the file as it is on disk now, the question, and the
    /// file given back, so nothing stays open in the server that the disk could then differ from.
    fn said(&self, root: &Path) -> (Value, Value, Value) {
        let uri = uri_of(&root.join(self.path));
        let document = json!({"uri": uri, "languageId": self.language, "version": 1, "text": self.text});
        let position = json!({"line": self.line, "character": self.character});
        (
            json!({"textDocument": document}),
            json!({"textDocument": {"uri": uri}, "position": position}),
            json!({"textDocument": {"uri": uri}}),
        )
    }
}

/// A place a server names: the file from the workspace's folder (None: a file outside it, a library's or the
/// language's own), and the line from 1.
pub type Named = (Option<String>, usize);

enum Slot {
    /// Nothing runs: the next question starts a server.
    Idle,
    /// Being started on a thread of its own. Questions meanwhile are the rules'.
    Starting,
    Ready(Box<Running>),
    /// It didn't start, or it died: tried again after a while.
    Failed(Instant),
}

struct Running {
    server: Server,
    asked: Instant,
}

/// The servers that run, by workspace.
pub struct Servers {
    slots: Mutex<HashMap<String, Arc<Mutex<Slot>>>>,
    /// A server nobody asked for this long is stopped.
    idle: Duration,
    /// A server that owes an answer for this long is started afresh.
    stuck: Duration,
}

impl Default for Servers {
    fn default() -> Self {
        Servers { slots: Mutex::default(), idle: IDLE, stuck: STUCK }
    }
}

impl Servers {
    /// Where the workspace's server says the name at a place is defined. None: no server is ready (the first
    /// question starts one), it is busy with an earlier question or with somebody else's, it had no answer in
    /// time, or it is gone. `changed` gives what the watcher saw change in the workspace since it was last asked
    /// (None: too much to say, so the server starts afresh); it is asked only when a server is there to be told,
    /// or is about to start and read everything itself. `folder` is where a server is started: a folder of the
    /// app's own.
    pub fn definition(
        &self,
        workspace: &str,
        program: &Program,
        folder: &Path,
        root: &Path,
        changed: &dyn Fn() -> Option<HashSet<String>>,
        question: &Question,
    ) -> Option<Vec<Named>> {
        let slot = self.slot(workspace);
        // somebody is asking it right now: this question is the rules', at once
        let mut state = match slot.try_lock() {
            Ok(state) => state,
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(TryLockError::WouldBlock) => return None,
        };
        // a server is started with the question that made it start: it works on it as soon as it has said hello,
        // so what it has to read first (a whole project, on a big one) is read before the next question
        let begin = |state: &mut Slot| {
            *state = Slot::Starting;
            let first = question.said(root);
            start(program.clone(), folder.to_path_buf(), root.to_path_buf(), slot.clone(), self.idle, first);
        };
        let running = match &mut *state {
            Slot::Starting => return None,
            Slot::Failed(at) if at.elapsed() < RETRY => return None,
            Slot::Idle | Slot::Failed(_) => {
                // what changed until now is nothing the new server needs to be told: it reads the files itself
                let _ = changed();
                begin(&mut state);
                return None;
            }
            Slot::Ready(running) => running,
        };
        if !running.server.alive() {
            *state = Slot::Failed(Instant::now());
            return None;
        }
        running.asked = Instant::now();
        // still busy with a question that was given up on: not asked, so nothing piles up for it. One that never
        // answers is started afresh
        if running.server.busy() {
            if running.server.owes.is_some_and(|(_, since)| since.elapsed() >= self.stuck) {
                let _ = changed();
                begin(&mut state);
            }
            return None;
        }
        // what changed on disk since it was last told
        match changed() {
            Some(files) if files.len() <= MAX_TOLD => {
                if !files.is_empty() {
                    let events: Vec<Value> = files.iter().flat_map(|path| events_of(root, path)).collect();
                    running.server.tell("workspace/didChangeWatchedFiles", json!({"changes": events}));
                }
            }
            _ => {
                begin(&mut state);
                return None;
            }
        }
        let (open, ask, close) = question.said(root);
        running.server.tell("textDocument/didOpen", open);
        let answer = running.server.ask("textDocument/definition", ask, ANSWER_WAIT);
        running.server.tell("textDocument/didClose", close);
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let named = locations(&answer?).into_iter().map(|(uri, line)| {
            let inside = path_of(&uri).and_then(|path| {
                let path = path.canonicalize().unwrap_or(path);
                path.strip_prefix(&root).ok().map(|rest| rest.to_string_lossy().to_string())
            });
            (inside, line)
        });
        Some(named.collect())
    }

    /// The workspace's slot, made if needed. Servers nobody asked lately are stopped on the way, and so is the one
    /// asked longest ago when a new workspace would make them too many.
    fn slot(&self, workspace: &str) -> Arc<Mutex<Slot>> {
        let mut slots = lock(&self.slots);
        let asked = |slot: &Arc<Mutex<Slot>>| match slot.try_lock().ok().as_deref() {
            Some(Slot::Ready(running)) => Some(running.asked),
            // being asked right now, or being started
            _ => None,
        };
        slots.retain(|id, slot| id == workspace || asked(slot).is_none_or(|at| at.elapsed() < self.idle));
        if !slots.contains_key(workspace) && slots.len() >= KEEP {
            let oldest = slots.iter().filter_map(|(id, slot)| Some((asked(slot)?, id.clone()))).min();
            if let Some((_, id)) = oldest {
                slots.remove(&id);
            }
        }
        slots.entry(workspace.to_string()).or_insert_with(|| Arc::new(Mutex::new(Slot::Idle))).clone()
    }

    /// The workspace's server is stopped (the workspace is gone).
    pub fn stop(&self, workspace: &str) {
        lock(&self.slots).remove(workspace);
    }

    /// Every server is stopped (the app quits).
    pub fn stop_all(&self) {
        lock(&self.slots).clear();
    }

    /// Whether the workspace's server can be asked now: it runs, has said hello, and owes no answer.
    pub fn ready(&self, workspace: &str) -> bool {
        let slot = lock(&self.slots).get(workspace).cloned();
        slot.is_some_and(|slot| match &mut *lock(&slot) {
            Slot::Ready(running) => !running.server.busy(),
            _ => false,
        })
    }

    /// The running server's program (its process id), for whoever measures what it holds.
    pub fn program(&self, workspace: &str) -> Option<u32> {
        let slot = lock(&self.slots).get(workspace).cloned()?;
        let state = lock(&slot);
        if let Slot::Ready(running) = &*state {
            Some(running.server.child.id())
        } else {
            None
        }
    }

    /// Whether a server runs or is being started for the workspace: nothing more has to be done to have one.
    pub fn held_for(&self, workspace: &str) -> bool {
        let slot = lock(&self.slots).get(workspace).cloned();
        slot.is_some_and(|slot| matches!(slot.try_lock().as_deref(), Ok(Slot::Starting | Slot::Ready(_)) | Err(_)))
    }

    /// How many servers are held (running or being started).
    pub fn held(&self) -> usize {
        lock(&self.slots).len()
    }
}

/// What a changed file is to a server: gone, or there. A file that is there is said both new and changed: the
/// watcher doesn't say which, and a server that knows it already takes "new" as "look again".
fn events_of(root: &Path, path: &str) -> Vec<Value> {
    let file = root.join(path);
    let uri = uri_of(&file);
    match file.exists() {
        true => vec![json!({"uri": uri, "type": 1}), json!({"uri": uri, "type": 2})],
        false => vec![json!({"uri": uri, "type": 3})],
    }
}

/// Starts a server for a workspace on a thread of its own and puts it in its slot once it has said hello, already
/// at work on `first`, the question that made it start (nobody waits for that answer: it only makes the server
/// read what it needs). The same thread then looks now and then whether anybody still asks: a server nobody asked
/// for `idle` is stopped, and the next question starts one again.
fn start(
    program: Program,
    folder: PathBuf,
    root: PathBuf,
    slot: Arc<Mutex<Slot>>,
    idle: Duration,
    first: (Value, Value, Value),
) {
    std::thread::spawn(move || {
        let ready = Server::start(&program, &folder).ok().and_then(|mut server| {
            let root = root.canonicalize().unwrap_or(root);
            let root_uri = uri_of(&root);
            let hello = server.ask(
                "initialize",
                json!({
                    "processId": std::process::id(),
                    "rootUri": root_uri,
                    "rootPath": root,
                    "workspaceFolders": [{"uri": root_uri, "name": "workspace"}],
                    "capabilities": {
                        "general": {"positionEncodings": ["utf-16"]},
                        "textDocument": {
                            "definition": {"linkSupport": true},
                            "synchronization": {"didSave": false},
                        },
                        "workspace": {
                            "configuration": true,
                            "workspaceFolders": true,
                            "didChangeWatchedFiles": {"dynamicRegistration": true},
                        },
                    },
                }),
                HELLO_WAIT,
            )?;
            let defines = &hello["capabilities"]["definitionProvider"];
            if defines.is_null() || *defines == Value::Bool(false) {
                return None;
            }
            server.tell("initialized", json!({}));
            let (open, ask, close) = first;
            server.tell("textDocument/didOpen", open);
            server.put("textDocument/definition", ask);
            server.tell("textDocument/didClose", close);
            Some(server)
        });
        let Some(server) = ready else {
            *lock(&slot) = Slot::Failed(Instant::now());
            return;
        };
        *lock(&slot) = Slot::Ready(Box::new(Running { server, asked: Instant::now() }));
        // (from here the slot is held only while it is looked at: a workspace that is removed drops its server at
        // once, not when this thread next wakes)
        let kept = Arc::downgrade(&slot);
        drop(slot);
        loop {
            std::thread::sleep((idle / 4).clamp(Duration::from_millis(50), Duration::from_secs(30)));
            let Some(slot) = kept.upgrade() else { return };
            let mut state = lock(&slot);
            match &*state {
                Slot::Ready(running) if running.asked.elapsed() >= idle => {
                    *state = Slot::Idle;
                    return;
                }
                Slot::Ready(_) => {}
                // stopped, or started afresh by a thread of its own
                _ => return,
            }
        }
    });
}

#[cfg(test)]
mod tests;
