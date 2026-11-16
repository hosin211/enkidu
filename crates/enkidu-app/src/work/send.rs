//! A message for Claude goes into its prompt and is never sent: you read it there and press Enter.
//!
//! A Claude running in the conversation gets it typed in, as one paste. Else Claude starts there (in the picked
//! conversation, or a new one, in the card's workspace or a new one) and gets it typed in once it is up. A branch
//! checked out in another folder comes back as [`Outcome::Taken`], for you to pick what to do ([`Way`]). With
//! nowhere to start Claude, the message goes on the clipboard.

use enkidu_core::commands::CmdError;
use enkidu_core::db::Workspace;
use enkidu_core::workspace::{CreateRequest, Created};
use enkidu_model::frame::Frame;
use enkidu_model::work::claudes::{BranchMode, SendTarget, branch_workspace_name, free_name};
use gpui_kit::{App, AsyncApp, ClipboardItem, Entity, Task};

use crate::bridge::Bridge;
use crate::terminals::Terminals;

/// The branch a card's new workspace wants is checked out in another folder (git checks a branch out once).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Taken {
    pub project_id: String,
    pub branch: String,
    /// The folder it is checked out in.
    pub path: String,
}

/// What you picked for a branch checked out elsewhere: open that folder as the workspace, or a new branch from it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Way {
    Open(String),
    Branch(String),
}

/// What became of the message.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// It is in the prompt of the Claude in this workspace (or Claude starts there and the core types it in), with
    /// what the core warned about while it made the workspace.
    Sent(Workspace, Vec<String>),
    /// Its new workspace's branch is checked out elsewhere: nothing was made.
    Taken(Taken),
    /// Nowhere to start Claude: it is on the clipboard.
    Nowhere,
}

/// The core didn't answer: the app is closing.
const GONE: &str = "The app is closing.";

/// Types `text` into the prompt of the Claude in `workspace`: a running one gets it as one paste; else Claude starts
/// there and the core types it in once its session is up. The answer comes once Claude has it, or says why not.
async fn type_in(terminals: &Entity<Terminals>, workspace: &str, text: &str, cx: &mut AsyncApp) -> Result<(), String> {
    let typed = cx.update(|cx| terminals.update(cx, |terminals, cx| terminals.type_into_claude(workspace, text, cx)));
    typed.await.unwrap_or_else(|_| Err("Claude's terminal is gone.".to_string()))
}

fn said(err: CmdError) -> String {
    err.message
}

/// Puts `text` into the prompt of the Claude `target` names. Nothing is sent, and nothing here takes you there:
/// who asked does, once this says where it went.
pub fn deliver(
    frame: Entity<Frame>,
    terminals: Entity<Terminals>,
    target: SendTarget,
    text: String,
    way: Option<Way>,
    cx: &mut App,
) -> Task<Result<Outcome, String>> {
    if let SendTarget::None = target {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        return Task::ready(Ok(Outcome::Nowhere));
    }
    if !cx.has_global::<Bridge>() {
        return Task::ready(Err("No terminals run in this window.".to_string()));
    }
    cx.spawn(async move |cx: &mut AsyncApp| match target {
        SendTarget::Workspace { workspace, .. } => {
            type_in(&terminals, &workspace.id, &text, cx).await?;
            Ok(Outcome::Sent(workspace, Vec::new()))
        }
        SendTarget::Switch { workspace, session } => {
            // the workspace's Claude moves to the picked conversation, or a new one; a busy Claude is never closed
            let id = workspace.id.clone();
            let runs = cx.update(|cx| Bridge::ask(cx, move |shared| enkidu_core::commands::pty_list(shared, id)));
            let running = runs.await.map_err(|_| GONE)?.into_iter().find(|t| t.kind == "claude" && !t.exited);
            if let Some(now) = running {
                let status = cx.update(|cx| frame.read(cx).statuses.get(&workspace.id).cloned());
                let busy = match status.as_deref() {
                    Some("working") => Some("working"),
                    Some("needs_you") => Some("waiting for you"),
                    _ => None,
                };
                if let Some(busy) = busy {
                    return Err(format!(
                        "Claude is {busy} in {}. Pick it, or try again once it's done.",
                        workspace.name
                    ));
                }
                // its tab goes at once; the core is asked again and waited for, so the switch finds no Claude there
                cx.update(|cx| terminals.update(cx, |terminals, cx| terminals.close(&workspace.id, &now.id, cx)));
                let closed =
                    cx.update(|cx| Bridge::ask(cx, move |shared| enkidu_core::commands::pty_close(shared, now.id)));
                closed.await.map_err(|_| GONE)?;
            }
            let (id, to) = (workspace.id.clone(), session.map(|session| session.id));
            let switched = cx
                .update(|cx| Bridge::ask(cx, move |shared| enkidu_core::commands::switch_conversation(shared, id, to)));
            let workspace = switched.await.map_err(|_| GONE)?.map_err(said)?;
            let now = workspace.clone();
            cx.update(|cx| {
                frame.update(cx, |frame, cx| {
                    if let Some(known) = frame.workspaces.iter_mut().find(|known| known.id == now.id) {
                        *known = now;
                    }
                    cx.notify();
                })
            });
            // no Claude runs there now: it starts in that conversation, with the message as its draft
            type_in(&terminals, &workspace.id, &text, cx).await?;
            Ok(Outcome::Sent(workspace, Vec::new()))
        }
        SendTarget::Outside { .. } | SendTarget::New { .. } => {
            let (project_id, name, branch, mode, session_id) = match target {
                SendTarget::Outside { session, project_id, name, branch, mode } => {
                    (project_id, name, branch, mode, Some(session.id))
                }
                SendTarget::New { project_id, name, branch, mode } => (project_id, name, branch, mode, None),
                _ => unreachable!("only a new workspace is made here"),
            };
            let made: Result<Created, CmdError> = match way {
                // that folder, as it is, becomes the workspace (one that already was a workspace is that one)
                Some(Way::Open(path)) => {
                    let project = project_id.clone();
                    let adopted = cx.update(|cx| {
                        Bridge::ask_with_news(cx, move |events, shared| {
                            enkidu_core::commands::adopt_workspace(
                                events.clone(),
                                shared,
                                project,
                                path,
                                session_id,
                                Some(name),
                            )
                        })
                    });
                    adopted.await.map_err(|_| GONE)?
                }
                other => {
                    let request = match other {
                        // a new branch from where the taken one is
                        Some(Way::Branch(new_branch)) => {
                            let known = cx.update(|cx| frame.read(cx).workspaces.clone());
                            CreateRequest {
                                project_id: project_id.clone(),
                                name: free_name(&branch_workspace_name(&new_branch), &project_id, &known),
                                mode: "copy".into(),
                                branch: new_branch,
                                base: Some(branch.clone()),
                                session_id,
                            }
                        }
                        _ => {
                            // a branch named for the ticket that is already there (made before, or on origin) is
                            // checked out, not made again
                            let mut mode = if mode == BranchMode::New { "new" } else { "existing" };
                            if mode == "new" {
                                let project = project_id.clone();
                                let listed = cx.update(|cx| {
                                    Bridge::ask(cx, move |shared| enkidu_core::commands::list_branches(shared, project))
                                });
                                if let Ok(Ok(known)) = listed.await
                                    && (known.local.contains(&branch) || known.remote.contains(&branch))
                                {
                                    mode = "existing";
                                }
                            }
                            CreateRequest {
                                project_id: project_id.clone(),
                                name,
                                mode: mode.into(),
                                branch: branch.clone(),
                                base: None,
                                session_id,
                            }
                        }
                    };
                    let wanted = request.branch.clone();
                    let created = cx.update(|cx| {
                        Bridge::ask_with_news(cx, move |events, shared| {
                            enkidu_core::commands::create_workspace(events.clone(), shared, request)
                        })
                    });
                    match created.await.map_err(|_| GONE)? {
                        Err(CmdError { code, path: Some(path), .. }) if code == "branch_taken" => {
                            return Ok(Outcome::Taken(Taken { project_id, branch: wanted, path }));
                        }
                        made => made,
                    }
                }
            };
            let Created { workspace, warnings } = made.map_err(said)?;
            // Claude starts there with the message before the workspace comes on screen (there, a workspace without
            // a Claude would start one of its own)
            type_in(&terminals, &workspace.id, &text, cx).await?;
            Ok(Outcome::Sent(workspace, warnings))
        }
        SendTarget::None => Ok(Outcome::Nowhere),
    })
}
