//! Sends what the failing-checks workflow queued (DESIGN.md, "Settings → a project → Workflows"). The core decides
//! what to queue, right after a full read of GitHub, and hands each run out once; the window owns the terminals, so it
//! types each into the prompt of the Claude that worked on it (never sent: you press Enter there), writes a line on the
//! item's timeline, says so in a toast, and tells the core how it went. It never acts on stale data, never starts a
//! Claude of its own, and never takes you anywhere. What to do with each is decided in `enkidu_model::work::queued`.

use enkidu_model::work::claudes::SendTarget;
use enkidu_model::work::queued::{
    NO_LONGER_WAITS, NOT_REACHED, Queued, queued_done, queued_note, queued_send, queued_toast,
};
use gpui_kit::{App, AsyncApp};

use crate::bridge::Bridge;
use crate::stores::{Notice, NoticeKind, Stores};
use crate::work::send::{Outcome, deliver};

/// Asks the core for what it queued and sends each: after every read of the index. One round at a time; what is
/// queued meanwhile is taken by the round after the next read.
pub fn run(cx: &mut App) {
    if !cx.has_global::<Bridge>() {
        return;
    }
    let Some(stores) = cx.try_global::<Stores>() else { return };
    let (work, frame, terminals) = (stores.work.clone(), stores.frame.clone(), stores.terminals.clone());
    let (drafts, notices) = (stores.drafts.clone(), stores.notices.clone());
    if work.read(cx).sending_queued {
        return;
    }
    work.update(cx, |work, _| work.sending_queued = true);
    let taken = Bridge::ask(cx, |shared| enkidu_core::commands::work_workflow_take(shared).unwrap_or_default());
    cx.spawn(async move |cx: &mut AsyncApp| {
        for run in taken.await.unwrap_or_default() {
            // on the index as it is now
            let decided = cx.update(|cx| {
                let (store, frame) = (work.read(cx), frame.read(cx));
                let Some(index) = store.snapshot.as_deref() else { return Queued::Skip(NO_LONGER_WAITS) };
                let chosen = drafts.read(cx).chosen();
                let (workspaces, statuses) = (&frame.workspaces, &frame.statuses);
                queued_send(&run.subject, &run.project_id, index, &store.groups, workspaces, statuses, &chosen)
            });
            let (state, text) = match decided {
                Queued::Skip(why) => ("skipped", why.to_string()),
                Queued::Fail(why) => ("failed", why.to_string()),
                Queued::Type { workspace, text, subject, title } => {
                    let target = SendTarget::Workspace { workspace: *workspace, session: None };
                    let typed = cx.update(|cx| deliver(frame.clone(), terminals.clone(), target, text, None, cx));
                    match typed.await {
                        Ok(Outcome::Sent(workspace, _)) => {
                            let note = queued_note(&workspace.name, &title);
                            let noted = cx.update(|cx| {
                                Bridge::ask_with_news(cx, move |events, shared| {
                                    enkidu_core::commands::work_note(events.clone(), shared, subject, note)
                                })
                            });
                            let _ = noted.await;
                            let (toast, detail) = queued_toast(&workspace.name, &title);
                            cx.update(|cx| {
                                notices.update(cx, |notices, cx| {
                                    notices.0.push(Notice { kind: NoticeKind::Done, title: toast, detail });
                                    cx.notify();
                                })
                            });
                            ("done", queued_done(&workspace.name))
                        }
                        _ => ("failed", NOT_REACHED.to_string()),
                    }
                }
            };
            let id = run.id;
            let finished = cx.update(|cx| {
                Bridge::ask_with_news(cx, move |events, shared| {
                    enkidu_core::commands::work_workflow_finish(events.clone(), shared, id, state.to_string(), text)
                })
            });
            let _ = finished.await;
        }
        cx.update(|cx| work.update(cx, |work, _| work.sending_queued = false));
    })
    .detach();
}
