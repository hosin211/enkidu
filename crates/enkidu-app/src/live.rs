//! Live in the window (item 4.5; DESIGN.md, "Live"): the play button at the right of the tabs row, at one width in
//! every state so nothing beside it moves, and the switch itself. Go live copies the workspace's changes into the
//! project's main folder, where the dev environment runs, and they follow every edit; Stop puts the main folder back.
//! The core does the copying, off the drawing thread; a toast says what it did, and what the project's own command
//! printed.

use enkidu_core::commands::LiveRequest;
use enkidu_model::frame::Frame;
use enkidu_model::live::{self, LiveState};
use enkidu_ui::live::LiveLook;
use gpui_kit::{App, AppContext as _, AsyncApp, Entity, Window};

use crate::bridge::Bridge;
use crate::toast;

/// How a state of the button looks.
pub fn look(state: LiveState) -> LiveLook {
    match state {
        LiveState::Off => LiveLook::Off,
        LiveState::Starting => LiveLook::Starting,
        LiveState::Live => LiveLook::Live,
        LiveState::Stopping => LiveLook::Stopping,
        LiveState::Waiting => LiveLook::Waiting,
    }
}

/// Takes `project`'s Live to `workspace` (none: stops it). A press while it is switching does nothing. The toast
/// says what happened to the main folder, then what the project's own command printed; when it couldn't switch,
/// why, and the window shows what is Live now (the core may have put back the one that was Live before it failed).
pub fn switch(frame: Entity<Frame>, project: String, workspace: Option<String>, window: &mut Window, cx: &mut App) {
    // a window without a core (a picture) has nothing to switch
    if !cx.has_global::<Bridge>() {
        return;
    }
    let started = frame.update(cx, |frame, cx| {
        let started = frame.live_switching.start(&project, workspace.as_deref());
        if started {
            cx.notify();
        }
        started
    });
    if !started {
        return;
    }
    let asked = Bridge::ask_with_news(cx, {
        let project = project.clone();
        move |events, shared| {
            let request = LiveRequest { project_id: project, workspace_id: workspace };
            let outcome = enkidu_core::commands::set_live(events.clone(), shared, request);
            // what is Live now as the database has it, when it failed
            let now = outcome.is_err().then(|| shared.db.projects().ok()).flatten();
            (outcome, now)
        }
    });
    let handle = window.window_handle();
    cx.spawn(async move |cx: &mut AsyncApp| {
        let answer = asked.await;
        // the window's state first, whether or not the window is still there to say it
        let said = cx.update(|cx| {
            frame.update(cx, |frame, cx| {
                frame.live_switching.end(&project);
                cx.notify();
                match answer {
                    Ok((Ok(outcome), _)) => {
                        if let Some(p) = frame.projects.iter_mut().find(|p| p.id == project) {
                            p.live_workspace_id = outcome.live_workspace_id.clone();
                        }
                        let name = (outcome.live_workspace_id.as_ref())
                            .and_then(|id| frame.workspaces.iter().find(|w| &w.id == id))
                            .map(|w| w.name.clone());
                        Some(Ok(live::switched(name.as_deref(), &outcome.note, &outcome.output)))
                    }
                    Ok((Err(err), now)) => {
                        for read in now.unwrap_or_default() {
                            if let Some(p) = frame.projects.iter_mut().find(|p| p.id == read.id) {
                                p.live_workspace_id = read.live_workspace_id;
                            }
                        }
                        Some(Err(err.message))
                    }
                    Err(_) => None,
                }
            })
        });
        let _ = cx.update_window(handle, |_, window, cx| match said {
            Some(Ok((title, Some(detail)))) => toast::done(window, title, detail, cx),
            Some(Ok((title, None))) => toast::good(window, title, cx),
            Some(Err(why)) => toast::warning(window, live::DID_NOT_SWITCH, why, cx),
            None => {}
        });
    })
    .detach();
}
