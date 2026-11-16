//! Each workspace's terminals (item 2.11). The core runs them: their pty, Claude's command (your args and env from
//! Settings, the hooks through `--settings`, the conversation to resume, its name), the watch after Esc and Ctrl+C.
//! Each one is drawn by an enkidu-term terminal attached to it. Their output never passes through the drawing thread:
//! the core's reader threads hand it straight to each terminal's own parser ([`Feeds`]).

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use enkidu_core::events::{Event, Events};
use enkidu_core::pty::TerminalInfo;
use enkidu_core::state::Shared;
use enkidu_term::{Feed, GridSize, Palette, Program, Terminal};
use futures::channel::mpsc::UnboundedSender;
use futures::channel::oneshot;
use gpui_kit::{AppContext as _, AsyncApp, Context, Entity, WeakEntity};

use crate::bridge::Bridge;
use crate::terminal_view::TerminalView;

/// The size the core starts a terminal with; its view gives it its real size as soon as it is drawn.
const FIRST_SIZE: (u16, u16) = (120, 32);

/// What became of text for Claude's prompt, to await: it is there, or why it isn't (see
/// [`Terminals::type_into_claude`]).
pub type Typed = oneshot::Receiver<Result<(), String>>;

/// Where that is said.
type Say = oneshot::Sender<Result<(), String>>;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// Where each core terminal's output goes, from the core's reader threads.
#[derive(Default)]
pub struct Feeds {
    slots: Mutex<HashMap<String, Slot>>,
}

enum Slot {
    /// Being attached: what the core printed meanwhile, in order, as (start, end, bytes).
    Attaching { pending: Vec<(u64, u64, Vec<u8>)> },
    /// Attached: where its output goes, and how far it has been given.
    Fed { feed: Feed, upto: u64 },
}

/// The part of `bytes` (a terminal's output from `start` to `end`) that comes after `upto`.
fn after(upto: u64, start: u64, end: u64, bytes: Vec<u8>) -> Option<Vec<u8>> {
    if end <= upto {
        return None;
    }
    let skip = (upto.saturating_sub(start) as usize).min(bytes.len());
    Some(if skip == 0 { bytes } else { bytes[skip..].to_vec() })
}

impl Feeds {
    /// The core's terminal `id` printed `data`: bytes `start` to `end` of all it printed. Called on the core's reader
    /// thread, which waits here while that terminal's parser is behind: that holds the program back, as any terminal
    /// does. Output of a terminal no view shows yet is left to the core, which keeps it for when one does.
    pub fn output(&self, id: &str, data: String, start: u64, end: u64) {
        let mut slots = lock(&self.slots);
        let (feed, bytes) = match slots.get_mut(id) {
            None => return,
            Some(Slot::Attaching { pending }) => {
                pending.push((start, end, data.into_bytes()));
                return;
            }
            Some(Slot::Fed { feed, upto }) => {
                let Some(bytes) = after(*upto, start, end, data.into_bytes()) else { return };
                *upto = end;
                (feed.clone(), bytes)
            }
        };
        drop(slots);
        feed.push(bytes);
    }

    /// Attaches `feed` to the core's terminal `id`: first what it printed so far (`so_far` reads it; the core keeps
    /// the last 400 KB), then everything after, in order, each byte once. Call it off the drawing thread.
    pub fn attach(&self, id: &str, feed: Feed, so_far: impl FnOnce() -> (Vec<u8>, u64)) {
        lock(&self.slots).insert(id.to_string(), Slot::Attaching { pending: Vec::new() });
        let (printed, end) = so_far();
        let mut slots = lock(&self.slots);
        // closed while it was being attached
        let Some(Slot::Attaching { pending }) = slots.remove(id) else { return };
        feed.push(printed);
        let mut upto = end;
        for (start, end, bytes) in pending {
            if let Some(bytes) = after(upto, start, end, bytes) {
                feed.push(bytes);
                upto = end;
            }
        }
        slots.insert(id.to_string(), Slot::Fed { feed, upto });
    }

    /// The terminal was closed: its output goes nowhere.
    pub fn detach(&self, id: &str) {
        lock(&self.slots).remove(id);
    }
}

/// Where the core sends its events: a terminal's output goes straight to its parser, everything else to the stores,
/// on the drawing thread.
pub fn sink(stores: UnboundedSender<Event>, feeds: Arc<Feeds>) -> Events {
    Events::new(move |event| match event {
        Event::PtyOutput { id, data, start, end } => feeds.output(&id, data, start, end),
        other => {
            let _ = stores.unbounded_send(other);
        }
    })
}

/// What is typed into a core terminal, and each new size. The core writes it: it watches Claude after Esc and Ctrl+C,
/// so a Claude you interrupted goes back to idle.
struct CoreProgram {
    shared: Arc<Shared>,
    events: Events,
    id: String,
}

impl Program for CoreProgram {
    fn write(&mut self, bytes: &[u8]) -> bool {
        let typed = String::from_utf8_lossy(bytes).into_owned();
        enkidu_core::commands::pty_write(self.events.clone(), &self.shared, self.id.clone(), typed).is_ok()
    }

    fn resize(&mut self, size: GridSize) {
        let _ = enkidu_core::commands::pty_resize(&self.shared, self.id.clone(), size.cols, size.rows);
    }
}

/// One terminal of a workspace: what the core says of it, and its view.
pub struct Tab {
    pub info: TerminalInfo,
    pub view: Entity<TerminalView>,
}

/// A workspace's terminals: Claude first, then its shells.
#[derive(Default)]
pub struct Area {
    pub tabs: Vec<Tab>,
    /// The terminal on screen.
    pub active: Option<String>,
    /// Why Claude didn't start.
    pub error: Option<String>,
    /// Why a shell didn't open: said once, then cleared.
    pub notice: Option<String>,
    /// Text was typed into Claude here: its terminal takes the keyboard, so Enter is right there.
    wants_keyboard: bool,
    /// The core was asked what runs here (its answer may still be on its way).
    asked: bool,
    /// The answer came: `tabs` is what runs.
    pub known: bool,
}

impl Area {
    pub fn active_tab(&self) -> Option<&Tab> {
        self.tabs.iter().find(|t| Some(&t.info.id) == self.active.as_ref())
    }

    pub fn has_live_claude(&self) -> bool {
        self.tabs.iter().any(|t| t.info.kind == "claude" && !t.info.exited)
    }

    /// A tab's name: "Claude", "Shell", or "Shell 2" once there are several.
    pub fn label(&self, tab: &Tab) -> String {
        let all: Vec<(&str, &str)> = self.tabs.iter().map(|t| (t.info.id.as_str(), t.info.kind.as_str())).collect();
        tab_label(&all, &tab.info.id)
    }
}

/// The name of the terminal `id` among a workspace's terminals, given as (id, kind) in their order: "Claude" for
/// Claude, "Shell" for the only shell, "Shell 1", "Shell 2" once there are several.
pub fn tab_label(tabs: &[(&str, &str)], id: &str) -> String {
    if tabs.iter().any(|&(tab, kind)| tab == id && kind == "claude") {
        return "Claude".into();
    }
    let shells: Vec<&str> = tabs.iter().filter(|(_, kind)| *kind == "shell").map(|(tab, _)| *tab).collect();
    match shells.iter().position(|tab| *tab == id) {
        Some(at) if shells.len() > 1 => format!("Shell {}", at + 1),
        _ => "Shell".into(),
    }
}

/// Every workspace's terminals.
pub struct Terminals {
    areas: HashMap<String, Area>,
    /// The live app's: where the core's output goes. None for pictures and tests without a core, where nothing is
    /// ever started.
    feeds: Option<Arc<Feeds>>,
    /// For pictures and tests without a core: these terminals stand in every workspace.
    everywhere: Option<Area>,
    /// Where the list of the workspaces that have a Claude is kept from one launch to the next.
    sessions: Option<PathBuf>,
    /// The Claudes still to bring back at launch, in order.
    coming_back: VecDeque<String>,
    /// The launch's Claudes were asked for (once a launch), and are all back: from then on the kept list follows what
    /// you open and close.
    asked_back: bool,
    back: bool,
}

impl Terminals {
    /// The app's: terminals the core runs. `sessions` is the file that keeps which workspaces have a Claude.
    pub fn live(feeds: Arc<Feeds>, sessions: Option<PathBuf>) -> Terminals {
        Terminals {
            areas: HashMap::new(),
            feeds: Some(feeds),
            everywhere: None,
            sessions,
            coming_back: VecDeque::new(),
            asked_back: false,
            back: false,
        }
    }

    /// At launch every workspace's Claude comes back, not only the one on screen: the first time all of them, later
    /// the ones that had a Claude when the app last ran; the one on screen first, then one after another, so they
    /// don't all start in the same instant. Once a launch.
    pub fn bring_back(&mut self, workspaces: Vec<String>, on_screen: Option<String>, cx: &mut Context<Self>) {
        if self.feeds.is_none() || self.asked_back {
            return;
        }
        self.asked_back = true;
        let path = self.sessions.clone();
        let kept = Bridge::ask(cx, move |_| {
            let text = std::fs::read_to_string(path?).ok()?;
            serde_json::from_str::<Vec<String>>(&text).ok()
        });
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let kept = kept.await.ok().flatten();
            let _ = this.update(cx, |this, cx| {
                let back = enkidu_model::sessions::to_bring_back(&workspaces, kept.as_deref(), on_screen.as_deref());
                this.coming_back = back.into();
                this.next_back(cx);
            });
        })
        .detach();
    }

    /// Starts the next Claude of the launch, or, when all are back, begins to keep the list.
    fn next_back(&mut self, cx: &mut Context<Self>) {
        while let Some(workspace) = self.coming_back.pop_front() {
            // the one on screen started its own already
            if !self.areas.get(&workspace).is_some_and(|area| area.asked) {
                self.ensure(&workspace, cx);
                return;
            }
        }
        // all were asked for: the launch is over once none is still starting, so the list is never kept with one missing
        let starting = self.areas.values().any(|area| area.asked && !area.known);
        if !self.back && !starting {
            self.back = true;
            self.keep_sessions(cx);
        }
    }

    /// A workspace's terminals are known now: the launch goes on with the next one.
    fn settled(&mut self, cx: &mut Context<Self>) {
        if self.asked_back && !self.back {
            self.next_back(cx);
        }
    }

    /// Writes which workspaces have a Claude (one that exited still counts: the next launch starts it again; a shell
    /// alone doesn't). Nothing is written while the launch's Claudes are still coming back, so none is forgotten.
    fn keep_sessions(&self, cx: &mut Context<Self>) {
        let Some(path) = self.sessions.clone().filter(|_| self.back) else { return };
        let mut list: Vec<String> = self
            .areas
            .iter()
            .filter(|(_, area)| area.tabs.iter().any(|t| t.info.kind == "claude"))
            .map(|(id, _)| id.clone())
            .collect();
        list.sort();
        Bridge::call(
            cx,
            move |_| {
                let text = serde_json::to_string(&list).unwrap_or_default();
                let tmp = path.with_extension("json.tmp");
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                if let Err(err) = std::fs::write(&tmp, text).and_then(|()| std::fs::rename(&tmp, &path)) {
                    eprintln!("Enkidu: which workspaces have a Claude could not be kept: {err}");
                }
            },
            |(), _| {},
        );
    }

    /// For pictures and tests without a core: `views` stand in every workspace, the first as Claude, the rest as
    /// shells. Nothing is started or closed.
    pub fn sample(views: Vec<Entity<TerminalView>>) -> Terminals {
        let tabs: Vec<Tab> = views
            .into_iter()
            .enumerate()
            .map(|(ix, view)| Tab {
                info: TerminalInfo {
                    id: format!("sample-{ix}"),
                    workspace_id: String::new(),
                    kind: if ix == 0 { "claude" } else { "shell" }.into(),
                    title: if ix == 0 { "Claude" } else { "Shell" }.into(),
                    exited: false,
                    exit_code: None,
                },
                view,
            })
            .collect();
        let active = tabs.first().map(|t| t.info.id.clone());
        let area = Area { tabs, active, known: true, asked: true, ..Area::default() };
        Terminals {
            areas: HashMap::new(),
            feeds: None,
            everywhere: Some(area),
            sessions: None,
            coming_back: VecDeque::new(),
            asked_back: false,
            back: false,
        }
    }

    /// A workspace's terminals, once anything is known of them.
    pub fn of(&self, workspace: &str) -> Option<&Area> {
        self.areas.get(workspace).or(self.everywhere.as_ref())
    }

    /// The view of the terminal on screen in a workspace.
    pub fn active_view(&self, workspace: &str) -> Option<Entity<TerminalView>> {
        self.of(workspace)?.active_tab().map(|t| t.view.clone())
    }

    /// A workspace came on screen: the terminals the core already runs for it show, or its Claude starts.
    pub fn ensure(&mut self, workspace: &str, cx: &mut Context<Self>) {
        let id = workspace.to_string();
        self.learn(workspace, cx, move |this, cx| {
            if this.areas.get(&id).is_none_or(|area| area.tabs.is_empty()) {
                this.spawn(&id, "claude", None, cx);
            }
        });
    }

    /// Asks the core, once, what already runs in a workspace, and shows it; then `then` goes on from there.
    fn learn(
        &mut self,
        workspace: &str,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, &mut Context<Self>) + 'static,
    ) {
        if self.feeds.is_none() {
            return;
        }
        let area = self.areas.entry(workspace.to_string()).or_default();
        if area.asked {
            return;
        }
        area.asked = true;
        let id = workspace.to_string();
        let running = Bridge::ask(cx, {
            let id = id.clone();
            move |shared| enkidu_core::commands::pty_list(shared, id)
        });
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let Ok(running) = running.await else { return };
            let _ = this.update(cx, |this, cx| {
                let found = !running.is_empty();
                if found {
                    let tabs: Vec<Tab> = running.into_iter().filter_map(|info| this.adopt(info, cx)).collect();
                    let area = this.areas.entry(id).or_default();
                    area.active = tabs.first().map(|t| t.info.id.clone());
                    area.tabs = tabs;
                    area.known = true;
                }
                then(this, cx);
                if found {
                    this.keep_sessions(cx);
                    this.settled(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Text for a workspace's Claude goes into its prompt as one paste and is never sent: you read it there and press
    /// Enter. A Claude that isn't running starts, and the core types the text in once its session is up (never as a
    /// first message on its command line, which Claude would send).
    ///
    /// What comes back says what became of it: the text is in the prompt of a running Claude (or Claude started and
    /// the core has it to type in), or why not (Claude's command couldn't start, in the core's words). Who must know
    /// awaits it (remarks are only cleared once Claude has them); who needn't lets it go.
    pub fn type_into_claude(&mut self, workspace: &str, text: &str, cx: &mut Context<Self>) -> Typed {
        let (say, typed) = oneshot::channel();
        self.type_and_say(workspace, text, say, cx);
        typed
    }

    fn type_and_say(&mut self, workspace: &str, text: &str, say: Say, cx: &mut Context<Self>) {
        if self.feeds.is_none() {
            let _ = say.send(Err("No terminals run in this window.".into()));
            return;
        }
        if !self.areas.get(workspace).is_some_and(|area| area.asked) {
            // a workspace not opened yet: first what runs there
            let (id, text) = (workspace.to_string(), text.to_string());
            self.learn(workspace, cx, move |this, cx| this.type_and_say(&id, &text, say, cx));
            return;
        }
        let claude = self.areas.get(workspace).and_then(|area| {
            area.tabs
                .iter()
                .find(|t| t.info.kind == "claude" && !t.info.exited)
                .map(|t| (t.info.id.clone(), t.view.clone()))
        });
        match claude {
            Some((id, view)) => {
                view.read(cx).terminal().send(enkidu_term::paste_bytes(text, true));
                self.show(workspace, &id, cx);
                let _ = say.send(Ok(()));
            }
            None => self.spawn_and_say(workspace, "claude", Some(text.to_string()), Some(say), cx),
        }
        if let Some(area) = self.areas.get_mut(workspace) {
            area.wants_keyboard = true;
        }
        cx.notify();
    }

    /// Starts Claude or a shell in a workspace. `draft` goes into Claude's prompt once it has started, never sent.
    pub fn spawn(&mut self, workspace: &str, kind: &str, draft: Option<String>, cx: &mut Context<Self>) {
        self.spawn_and_say(workspace, kind, draft, None, cx);
    }

    /// The same; `say` hears whether it started.
    fn spawn_and_say(
        &mut self,
        workspace: &str,
        kind: &str,
        draft: Option<String>,
        say: Option<Say>,
        cx: &mut Context<Self>,
    ) {
        if self.feeds.is_none() {
            return;
        }
        let area = self.areas.entry(workspace.to_string()).or_default();
        area.asked = true;
        if kind == "claude" {
            area.error = None;
        }
        cx.notify();
        let (id, kind) = (workspace.to_string(), kind.to_string());
        let made = Bridge::ask_with_news(cx, {
            let (id, kind) = (id.clone(), kind.clone());
            move |events, shared| {
                let (cols, rows) = FIRST_SIZE;
                enkidu_core::commands::spawn_terminal(events, shared, &id, &kind, cols, rows, draft, false)
            }
        });
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let Ok(made) = made.await else { return };
            let started = made.as_ref().map(|_| ()).map_err(|err| err.message.clone());
            let _ = this.update(cx, |this, cx| {
                match made {
                    Ok(info) => this.add(info, true, cx),
                    Err(err) => {
                        let area = this.areas.entry(id).or_default();
                        area.known = true;
                        if kind == "claude" {
                            area.error = Some(err.message);
                            // no Claude came for the text that was to be typed in: nothing takes the keyboard for it
                            area.wants_keyboard = false;
                        } else {
                            area.notice = Some(err.message);
                        }
                    }
                }
                this.keep_sessions(cx);
                this.settled(cx);
                cx.notify();
            });
            if let Some(say) = say {
                let _ = say.send(started);
            }
        })
        .detach();
    }

    /// A terminal the core runs joins its workspace's tabs: Claude first (in place of a Claude that exited), shells
    /// after. `show` puts it on screen.
    fn add(&mut self, info: TerminalInfo, show: bool, cx: &mut Context<Self>) {
        let workspace = info.workspace_id.clone();
        if self.areas.get(&workspace).is_some_and(|a| a.tabs.iter().any(|t| t.info.id == info.id)) {
            return;
        }
        let Some(tab) = self.adopt(info, cx) else { return };
        let old: Vec<String> = match self.areas.get(&workspace) {
            Some(area) if tab.info.kind == "claude" => area
                .tabs
                .iter()
                .filter(|t| t.info.kind == "claude" && t.info.exited)
                .map(|t| t.info.id.clone())
                .collect(),
            _ => Vec::new(),
        };
        for id in &old {
            self.forget(id, cx);
        }
        let area = self.areas.entry(workspace).or_default();
        area.tabs.retain(|t| !old.contains(&t.info.id));
        area.known = true;
        if show || area.active.is_none() || area.active.as_ref().is_some_and(|a| old.contains(a)) {
            area.active = Some(tab.info.id.clone());
        }
        if tab.info.kind == "claude" {
            area.tabs.insert(0, tab);
        } else {
            area.tabs.push(tab);
        }
    }

    /// The view of a terminal the core runs: an enkidu-term terminal attached to it, given what it printed so far and
    /// everything after.
    fn adopt(&mut self, info: TerminalInfo, cx: &mut Context<Self>) -> Option<Tab> {
        let feeds = self.feeds.clone()?;
        let (shared, events) = Bridge::core(cx);
        let (tx, rx) = futures::channel::mpsc::unbounded::<()>();
        let wake: enkidu_term::Wake = Arc::new(move || {
            let _ = tx.unbounded_send(());
        });
        let program = CoreProgram { shared, events, id: info.id.clone() };
        let (cols, rows) = FIRST_SIZE;
        let attached = Terminal::attached(GridSize::new(cols, rows), Palette::enkidu(), wake, program);
        let (terminal, feed) = match attached {
            Ok(attached) => attached,
            Err(err) => {
                eprintln!("Enkidu: a terminal's threads could not start: {err:#}");
                return None;
            }
        };
        let view = cx.new(|cx| TerminalView::new(Arc::new(terminal), Some(rx), cx));
        let id = info.id.clone();
        Bridge::call(
            cx,
            move |shared| {
                feeds.attach(&id, feed, || {
                    let so_far = shared.pty.buffer(&id).unwrap_or_default();
                    (so_far.data.into_bytes(), so_far.end)
                })
            },
            |(), _| {},
        );
        Some(Tab { info, view })
    }

    /// Ends a terminal's program and stops its output.
    fn forget(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(feeds) = &self.feeds {
            feeds.detach(id);
            let id = id.to_string();
            Bridge::call(cx, move |shared| enkidu_core::commands::pty_close(shared, id), |(), _| {});
        }
    }

    /// Closes a terminal: its program ends and its tab goes.
    pub fn close(&mut self, workspace: &str, id: &str, cx: &mut Context<Self>) {
        if self.feeds.is_none() {
            return;
        }
        self.forget(id, cx);
        if let Some(area) = self.areas.get_mut(workspace) {
            area.tabs.retain(|t| t.info.id != id);
            if area.active.as_deref() == Some(id) {
                area.active = area.tabs.first().map(|t| t.info.id.clone());
            }
        }
        self.keep_sessions(cx);
        cx.notify();
    }

    /// Starts a terminal that exited again: Claude resumes its conversation, a shell starts anew.
    pub fn restart(&mut self, workspace: &str, id: &str, cx: &mut Context<Self>) {
        let kind = self
            .areas
            .get(workspace)
            .and_then(|a| a.tabs.iter().find(|t| t.info.id == id))
            .map_or_else(|| "claude".to_string(), |t| t.info.kind.clone());
        self.close(workspace, id, cx);
        self.spawn(workspace, &kind, None, cx);
    }

    /// A workspace was removed (the core ended its terminals): their output stops and its tabs are forgotten.
    pub fn workspace_gone(&mut self, workspace: &str, cx: &mut Context<Self>) {
        if let (Some(area), Some(feeds)) = (self.areas.remove(workspace), &self.feeds) {
            for tab in &area.tabs {
                feeds.detach(&tab.info.id);
            }
            self.keep_sessions(cx);
            cx.notify();
        }
    }

    /// For pictures: the sample terminal at `index` has exited with `code`.
    pub fn sample_exited(mut self, index: usize, code: i32) -> Terminals {
        if let Some(tab) = self.everywhere.as_mut().and_then(|area| area.tabs.get_mut(index)) {
            tab.info.exited = true;
            tab.info.exit_code = Some(code);
        }
        self
    }

    /// For a picture: Claude that couldn't start, with why.
    pub fn sample_error(mut self, why: &str) -> Terminals {
        if let Some(area) = self.everywhere.as_mut() {
            area.error = Some(why.to_string());
        }
        self
    }

    /// Puts another of a workspace's terminals on screen.
    pub fn show(&mut self, workspace: &str, id: &str, cx: &mut Context<Self>) {
        if let Some(area) = self.areas.get_mut(workspace)
            && area.tabs.iter().any(|t| t.info.id == id)
            && area.active.as_deref() != Some(id)
        {
            area.active = Some(id.to_string());
            cx.notify();
        }
    }

    /// Whether text was typed into this workspace's Claude since the last call: its terminal then takes the keyboard.
    /// A Claude that is still starting isn't on screen yet (one that ended may still be): the wish waits for it.
    pub fn take_keyboard_wish(&mut self, workspace: &str) -> bool {
        self.areas.get_mut(workspace).is_some_and(|area| {
            area.active_tab().is_some_and(|tab| tab.info.kind == "claude" && !tab.info.exited)
                && std::mem::take(&mut area.wants_keyboard)
        })
    }

    /// The notice of a shell that didn't open, taken to be said once.
    pub fn take_notice(&mut self, workspace: &str) -> Option<String> {
        self.areas.get_mut(workspace)?.notice.take()
    }

    /// The core's news about terminals: one's program ended, or a workflow started one.
    pub fn apply(&mut self, event: &Event, cx: &mut Context<Self>) {
        match event {
            Event::PtyExit { id, code } => {
                let tab = self.areas.values_mut().flat_map(|a| a.tabs.iter_mut()).find(|t| &t.info.id == id);
                if let Some(tab) = tab {
                    tab.info.exited = true;
                    tab.info.exit_code = *code;
                    cx.notify();
                }
            }
            // a workflow started a workspace's Claude: it is that workspace's Claude tab. A workspace not opened yet
            // finds it when it opens.
            Event::PtySpawned(info) if self.areas.get(&info.workspace_id).is_some_and(|a| a.known) => {
                let show = info.kind == "claude";
                self.add(info.clone(), show, cx);
                self.keep_sessions(cx);
                cx.notify();
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use enkidu_term::Screen;
    use std::time::{Duration, Instant};

    struct Nowhere;
    impl Program for Nowhere {
        fn write(&mut self, _: &[u8]) -> bool {
            true
        }
        fn resize(&mut self, _: GridSize) {}
    }

    fn terminal() -> (Terminal, Feed) {
        Terminal::attached(GridSize::new(40, 4), Palette::enkidu(), Arc::new(|| {}), Nowhere).unwrap()
    }

    fn shows(t: &Terminal, text: &str) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut screen = Screen::default();
        while Instant::now() < deadline {
            t.update(&mut screen);
            if screen.row_text(0) == text {
                return true;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        false
    }

    #[test]
    fn a_terminal_gets_what_was_printed_before_it_was_attached_then_the_rest_each_byte_once() {
        let feeds = Feeds::default();
        let (t, feed) = terminal();
        // printed before any view: the core keeps it, nothing is fed
        feeds.output("t1", "early ".into(), 0, 6);
        // while attaching, more arrives: part of it is already in what the core kept, part is new
        feeds.attach("t1", feed, || {
            feeds.output("t1", "middle ".into(), 6, 13);
            feeds.output("t1", "late ".into(), 13, 18);
            (b"early middle ".to_vec(), 13)
        });
        feeds.output("t1", "end".into(), 18, 21);
        // told again by mistake: not shown twice
        feeds.output("t1", "end".into(), 18, 21);
        assert!(shows(&t, "early middle late end"));
    }

    #[test]
    fn output_that_starts_before_what_a_terminal_has_is_cut_to_the_new_part() {
        assert_eq!(after(10, 4, 14, b"0123456789".to_vec()).unwrap(), b"6789");
        assert_eq!(after(10, 10, 13, b"abc".to_vec()).unwrap(), b"abc");
        assert_eq!(after(10, 2, 10, b"8 bytes.".to_vec()), None);
    }

    #[test]
    fn a_closed_terminal_takes_no_more_output_and_others_keep_theirs() {
        let feeds = Feeds::default();
        let (one, feed_one) = terminal();
        let (two, feed_two) = terminal();
        feeds.attach("one", feed_one, || (Vec::new(), 0));
        feeds.attach("two", feed_two, || (Vec::new(), 0));
        feeds.output("one", "first".into(), 0, 5);
        feeds.output("two", "second".into(), 0, 6);
        assert!(shows(&one, "first") && shows(&two, "second"));
        feeds.detach("one");
        feeds.output("one", " more".into(), 5, 10);
        feeds.output("two", " more".into(), 6, 11);
        assert!(shows(&two, "second more"));
        assert!(shows(&one, "first"), "nothing came after it was closed");
    }

    #[test]
    fn shells_are_numbered_once_there_are_two() {
        let one = [("c", "claude"), ("s1", "shell")];
        assert_eq!((tab_label(&one, "c"), tab_label(&one, "s1")), ("Claude".into(), "Shell".into()));
        let two = [("c", "claude"), ("s1", "shell"), ("s2", "shell")];
        assert_eq!((tab_label(&two, "s1"), tab_label(&two, "s2")), ("Shell 1".into(), "Shell 2".into()));
        assert_eq!(tab_label(&two, "c"), "Claude");
    }
}
