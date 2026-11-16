//! The remarks store (item 3.5b): each workspace's remarks on lines, kept by the core's database until they are sent to
//! Claude, so they are there after a restart. A workspace's remarks are read once it is known; adding, editing and
//! deleting one goes to the database off the drawing thread. The frame is told how many each workspace has, for its tab
//! and the status line. Sending them (item 3.6) types them into Claude's prompt as one message, and they are deleted
//! once Claude has them.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use enkidu_core::db::Remark;
use enkidu_model::frame::Frame;
use gpui_kit::{AsyncApp, Context, Entity, EventEmitter, WeakEntity};

use crate::bridge::Bridge;
use crate::terminals::Terminals;

/// What became of a remark that was to be saved, or of the remarks that were to go to Claude.
#[derive(Clone, Debug, PartialEq)]
pub enum RemarksEvent {
    /// It is in the database, and in the store.
    Saved(String),
    /// The database refused it (its words); it is not in the store.
    Failed { id: String, why: String },
    /// A workspace's remarks, this many, are in Claude's prompt and no longer here.
    Typed { workspace: String, count: usize },
    /// They couldn't be typed into Claude (why, in the core's words); they are all still here.
    NotTyped { workspace: String, why: String },
}

pub struct Remarks {
    frame: Entity<Frame>,
    /// Where each workspace's Claude runs: the remarks are typed into its prompt.
    terminals: Entity<Terminals>,
    lists: HashMap<String, Arc<Vec<Remark>>>,
    /// The workspaces whose remarks were asked for from the database.
    asked: HashSet<String>,
    /// The workspaces whose remarks are on their way into Claude's prompt.
    sending: HashSet<String>,
    /// Whether a core is there to keep them (none in a window made for pictures: they live in the store only).
    kept: bool,
}

impl EventEmitter<RemarksEvent> for Remarks {}

impl Remarks {
    /// The store on `frame`. With `kept`, every workspace's remarks are read from the core's database as the
    /// workspace becomes known, and every change is written there.
    pub fn new(frame: Entity<Frame>, terminals: Entity<Terminals>, kept: bool, cx: &mut Context<Self>) -> Remarks {
        cx.observe(&frame, |this, _, cx| this.follow(cx)).detach();
        let mut remarks =
            Remarks { frame, terminals, lists: HashMap::new(), asked: HashSet::new(), sending: HashSet::new(), kept };
        remarks.follow(cx);
        remarks
    }

    /// A workspace's remarks, in the order they were written.
    pub fn of(&self, workspace: &str) -> Arc<Vec<Remark>> {
        self.lists.get(workspace).cloned().unwrap_or_default()
    }

    /// Whether a workspace's remarks are on their way into Claude's prompt.
    pub fn sending(&self, workspace: &str) -> bool {
        self.sending.contains(workspace)
    }

    /// Types a workspace's remarks into its Claude's prompt as one message (`enkidu_model::remarks::message`), with
    /// `note` after them. Never sent: you read it there and press Enter. A Claude that isn't running starts and gets
    /// it typed in once it is up. The remarks are deleted here only when Claude has them
    /// ([`RemarksEvent::Typed`]); if Claude couldn't start they all stay ([`RemarksEvent::NotTyped`]).
    pub fn send(&mut self, workspace: &str, note: Option<&str>, cx: &mut Context<Self>) {
        let list = self.of(workspace);
        if list.is_empty() || !self.sending.insert(workspace.to_string()) {
            return;
        }
        cx.notify();
        let message = enkidu_model::remarks::message(&list, note);
        let typed = self.terminals.update(cx, |terminals, cx| terminals.type_into_claude(workspace, &message, cx));
        let workspace = workspace.to_string();
        cx.spawn(async move |store: WeakEntity<Self>, cx: &mut AsyncApp| {
            let typed = typed.await.unwrap_or_else(|_| Err("Claude's terminal is gone.".to_string()));
            let _ = store.update(cx, |store, cx| {
                store.sending.remove(&workspace);
                match typed {
                    Ok(()) => {
                        store.went(&workspace, &list, cx);
                        cx.emit(RemarksEvent::Typed { workspace, count: list.len() });
                    }
                    Err(why) => cx.emit(RemarksEvent::NotTyped { workspace, why }),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// `sent` went to Claude: they are deleted. One written while they were on their way stays.
    fn went(&mut self, workspace: &str, sent: &[Remark], cx: &mut Context<Self>) {
        let left: Vec<Remark> = (self.of(workspace).iter())
            .filter(|remark| !sent.iter().any(|gone| gone.id == remark.id))
            .cloned()
            .collect();
        if left.is_empty() {
            self.clear(workspace, cx);
            return;
        }
        self.set(workspace, left, cx);
        if self.kept {
            for id in sent.iter().map(|remark| remark.id.clone()) {
                Bridge::call(cx, move |shared| enkidu_core::commands::delete_remark(shared, id), |_, _| {});
            }
        }
    }

    /// The workspaces changed: a new one's remarks are read, a removed one's are forgotten (the database drops them
    /// with the workspace).
    fn follow(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<String> = self.frame.read(cx).workspaces.iter().map(|w| w.id.clone()).collect();
        self.lists.retain(|id, _| ids.contains(id));
        self.asked.retain(|id| ids.contains(id));
        if !self.kept {
            return;
        }
        for id in ids {
            if !self.asked.insert(id.clone()) {
                continue;
            }
            let store = cx.weak_entity();
            Bridge::call(
                cx,
                {
                    let id = id.clone();
                    move |shared| enkidu_core::commands::list_remarks(shared, id)
                },
                move |read, cx| {
                    // a database that can't be read leaves the workspace without remarks
                    let Ok(read) = read else { return };
                    let _ = store.update(cx, |store, cx| {
                        // what was added while this was read is newer than the read
                        let mut list = read;
                        for remark in store.of(&id).iter() {
                            match list.iter_mut().find(|known| known.id == remark.id) {
                                Some(known) => *known = remark.clone(),
                                None => list.push(remark.clone()),
                            }
                        }
                        store.set(&id, list, cx);
                    });
                },
            );
        }
    }

    fn set(&mut self, workspace: &str, list: Vec<Remark>, cx: &mut Context<Self>) {
        let count = list.len();
        self.lists.insert(workspace.to_string(), Arc::new(list));
        self.frame.update(cx, |frame, cx| {
            if frame.set_remarks(workspace, count) {
                cx.notify();
            }
        });
        cx.notify();
    }

    /// Saves a remark: a new one, or one that is there under the same id with new words. It is in the store once the
    /// database has it ([`RemarksEvent::Saved`]); if the database refuses, it isn't, and the event says why.
    pub fn save(&mut self, remark: Remark, cx: &mut Context<Self>) {
        if !self.kept {
            self.put(remark, cx);
            return;
        }
        let store = cx.weak_entity();
        let saved = remark.clone();
        Bridge::call(
            cx,
            move |shared| enkidu_core::commands::save_remark(shared, saved),
            move |result, cx| {
                let _ = store.update(cx, |store, cx| match result {
                    Ok(()) => store.put(remark, cx),
                    Err(err) => cx.emit(RemarksEvent::Failed { id: remark.id, why: err.message }),
                });
            },
        );
    }

    fn put(&mut self, remark: Remark, cx: &mut Context<Self>) {
        let mut list = self.of(&remark.workspace_id).to_vec();
        match list.iter_mut().find(|known| known.id == remark.id) {
            Some(known) => *known = remark.clone(),
            None => list.push(remark.clone()),
        }
        self.set(&remark.workspace_id, list, cx);
        cx.emit(RemarksEvent::Saved(remark.id));
    }

    /// Deletes one remark, from the store at once and from the database behind it.
    pub fn delete(&mut self, workspace: &str, id: &str, cx: &mut Context<Self>) {
        let list: Vec<Remark> = self.of(workspace).iter().filter(|remark| remark.id != id).cloned().collect();
        self.set(workspace, list, cx);
        if self.kept {
            let id = id.to_string();
            Bridge::call(cx, move |shared| enkidu_core::commands::delete_remark(shared, id), |_, _| {});
        }
    }

    /// Deletes every remark of a workspace: they were sent to Claude, or discarded.
    pub fn clear(&mut self, workspace: &str, cx: &mut Context<Self>) {
        self.set(workspace, Vec::new(), cx);
        if self.kept {
            let workspace = workspace.to_string();
            Bridge::call(cx, move |shared| enkidu_core::commands::clear_remarks(shared, workspace), |_, _| {});
        }
    }
}
