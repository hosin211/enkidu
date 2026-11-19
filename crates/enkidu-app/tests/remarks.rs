//! Item 3.5b: the remarks store on the whole wiring. A remark is kept by the core's database until it is sent, its
//! workspace's tab and the status line count it, and it is there again after the app is quit and opened.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::{App, said};
use enkidu_app::remarks::{Remarks, RemarksEvent};
use enkidu_app::stores::Stores;
use enkidu_core::db::Remark;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext};

fn remark(workspace: &str, id: &str, line: i64, body: &str) -> Remark {
    Remark {
        id: id.into(),
        workspace_id: workspace.into(),
        path: "src/app.py".into(),
        side: "new".into(),
        start_line: line,
        end_line: line,
        snippet: "    return sum(items)".into(),
        body: body.into(),
        created_at: line,
    }
}

fn store(cx: &mut TestAppContext) -> Entity<Remarks> {
    cx.update(|cx| Stores::remarks(cx))
}

/// What the store has for a workspace, as (id, what was said), once it has `count` of them.
fn until_count(cx: &mut TestAppContext, app: &App, workspace: &str, count: usize) -> Vec<(String, String)> {
    let remarks = store(cx);
    let have = |cx: &mut TestAppContext| {
        remarks.read_with(cx, |remarks, _| {
            remarks.of(workspace).iter().map(|r| (r.id.clone(), r.body.clone())).collect::<Vec<_>>()
        })
    };
    assert!(app.until_frame(cx, |frame| frame.remarks(workspace) == count), "the count never came: {:?}", have(cx));
    have(cx)
}

#[gpui_kit::test]
fn a_remark_is_kept_by_the_database_counted_on_its_tab_and_there_after_a_restart(cx: &mut TestAppContext) {
    let app = common::start_on(cx, enkidu_testkit::shop, |shared, project, _| {
        App::make(shared, project, "stock-fix", "new", "stock-fix", Some("development"));
        App::make(shared, project, "other", "new", "other", Some("development"));
    });
    let ids =
        app.frame.read_with(cx, |f, _| f.workspaces.iter().map(|w| (w.name.clone(), w.id.clone())).collect::<Vec<_>>());
    let id_of =
        |name: &str| ids.iter().find(|(known, _)| known == name).map(|(_, id)| id.clone()).expect("the workspace");
    let (ws, other) = (id_of("stock-fix"), id_of("other"));
    app.click(cx, ("tab", 0usize));
    let remarks = store(cx);
    let events = Rc::new(RefCell::new(Vec::new()));
    let _watch = cx.update(|cx| {
        let events = events.clone();
        cx.subscribe(&remarks, move |_, event: &RemarksEvent, _| events.borrow_mut().push(event.clone()))
    });

    // one remark: in the database, on the tab and in the status line, in the remarks' words
    remarks.update(cx, |remarks, cx| remarks.save(remark(&ws, "r1", 2, "Why not a generator?"), cx));
    assert_eq!(until_count(cx, &app, &ws, 1), [("r1".to_string(), "Why not a generator?".to_string())]);
    assert_eq!(app.shared.db.remarks(&ws).unwrap().len(), 1);
    assert!(app.until(cx, |w| said(w, ("tab-remarks", 0usize)).as_deref() == Some("1 remark not sent yet")));
    cx.update_window(app.window, |_, w, _| {
        assert_eq!(said(w, "review-bar").as_deref(), Some("1 remark not sent yet"), "at the foot of the changes")
    })
    .unwrap();
    assert_eq!(*events.borrow(), [RemarksEvent::Saved("r1".into())]);

    // edited: the same remark with new words; a second one; the other workspace has none of them
    remarks.update(cx, |remarks, cx| {
        remarks.save(remark(&ws, "r1", 2, "Use a generator here"), cx);
        remarks.save(remark(&ws, "r2", 1, "Name it for what it adds up"), cx);
    });
    let have = until_count(cx, &app, &ws, 2);
    assert!(have.contains(&("r1".to_string(), "Use a generator here".to_string())), "{have:?}");
    assert!(app.until(cx, |w| said(w, ("tab-remarks", 0usize)).as_deref() == Some("2 remarks not sent yet")));
    cx.update_window(app.window, |_, w, _| assert_eq!(said(w, ("tab-remarks", 1usize)), None)).unwrap();
    assert_eq!(app.frame.read_with(cx, |f, _| f.remarks(&other)), 0);

    // a remark the database refuses is not in the store, and the store says why
    let mut wrong = remark(&ws, "r3", 5, "On no side at all");
    wrong.side = "middle".into();
    remarks.update(cx, |remarks, cx| remarks.save(wrong, cx));
    for _ in 0..400 {
        cx.run_until_parked();
        if events.borrow().iter().any(|event| matches!(event, RemarksEvent::Failed { .. })) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let failed = events.borrow().iter().find(|event| matches!(event, RemarksEvent::Failed { .. })).cloned();
    assert_eq!(
        failed,
        Some(RemarksEvent::Failed { id: "r3".into(), why: "A remark is on the old or the new side".into() })
    );
    assert_eq!(until_count(cx, &app, &ws, 2).len(), 2);

    // deleted: gone from the store at once and from the database behind it
    remarks.update(cx, |remarks, cx| remarks.delete(&ws, "r2", cx));
    assert_eq!(until_count(cx, &app, &ws, 1).len(), 1);
    for _ in 0..400 {
        if app.shared.db.remarks(&ws).unwrap().len() == 1 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let kept: Vec<String> =
        app.shared.db.remarks(&ws).unwrap().into_iter().map(|r| format!("{}: {}", r.id, r.body)).collect();
    assert_eq!(kept, ["r1: Use a generator here"]);

    // the app is quit and opened again: the remark that wasn't sent is there, on its tab too
    let app = common::restart(cx, app);
    app.click(cx, ("tab", 0usize));
    assert_eq!(until_count(cx, &app, &ws, 1), [("r1".to_string(), "Use a generator here".to_string())]);
    assert!(app.until(cx, |w| said(w, ("tab-remarks", 0usize)).as_deref() == Some("1 remark not sent yet")));

    // cleared (sent, or discarded): none left anywhere, and nothing says "0 remarks"
    let remarks = store(cx);
    remarks.update(cx, |remarks, cx| remarks.clear(&ws, cx));
    assert!(until_count(cx, &app, &ws, 0).is_empty());
    assert!(app.until(cx, |w| w.try_find("review-bar").is_none() && w.try_find(("tab-remarks", 0usize)).is_none()));
    for _ in 0..400 {
        if app.shared.db.remarks(&ws).unwrap().is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(app.shared.db.remarks(&ws).unwrap().is_empty());
}
