//! Going to a definition with the real `ty`, in a real window on a throwaway repo (never on a real project):
//! what the fake server of `definitions.rs` can't show. A real server works out what a variable is, and sees what
//! Claude writes once the watcher has told it. Ignored unless `ENKIDU_LIVE_TY` names the program:
//!
//! This file has one test and its own process: it names the server for the whole process.

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::pointing::{clicked, look, marked, open, opened, rest_on, until_link};
use common::{App, said};
use enkidu_testkit::git;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Modifiers, TestAppContext};

fn write(folder: &Path, path: &str, text: &str) {
    let file = folder.join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, text).unwrap();
}

const MODELS: &str =
    "class StockItem:\n    def save(self):\n        pass\n\n\nclass ReturnLine:\n    def save(self):\n        pass\n";

/// ENKIDU_LIVE_TY=<the program> cargo test --locked -p enkidu-app --test definitions_live -- --ignored --nocapture
#[gpui_kit::test]
#[ignore]
fn the_real_ty_in_the_window_knows_a_variable_and_follows_what_is_written(cx: &mut TestAppContext) {
    let ty = std::env::var("ENKIDU_LIVE_TY").expect("set ENKIDU_LIVE_TY to the ty program");
    // SAFETY: the only test of this run (the other one of this file is not run with `--ignored`).
    unsafe { std::env::set_var("ENKIDU_PYTHON_SERVER", format!("exec '{ty}' server")) };
    let app = common::start_on(cx, enkidu_testkit::shop, |shared, project, _| {
        App::make(shared, project, "stock-fix", "new", "stock-fix", Some("development"))
    });
    let (_, _, folder) = app.on_screen(cx).expect("on screen");
    let folder = PathBuf::from(folder).canonicalize().unwrap();
    let views = "from orders.models import StockItem\n\n\ndef resolve(pk):\n    item = StockItem()\n    item.save()\n    return item\n";
    write(&folder, "orders/__init__.py", "");
    write(&folder, "orders/views.py", views);
    write(&folder, "orders/models.py", MODELS);
    git(&folder, &["add", "-A"]);
    git(&folder, &["commit", "-q", "-m", "orders"]);
    write(&folder, "src/app.py", "print(1)\n");
    assert!(app.until(cx, |w| said(w, "changes-count").as_deref() == Some("1 file")), "the change never came");
    let workspace = app.frame.read_with(cx, |frame, _| frame.current_workspace().map(|w| w.id.clone())).unwrap();
    let cmd = Modifiers::command();
    open(cx, &app, "orders/views.py");
    let (sets, saves, returns) = ("    item = StockItem()", "    item.save()", "    return item");

    // ty was started when the workspace's Python came on screen. The pointer comes to rest on a variable with ⌘
    // held, and stays: the name becomes a link as soon as ty can be asked
    let started = Instant::now();
    let item = rest_on(cx, &app, returns, "Line 7", "item");
    until_link(cx, &app, returns, "item");
    println!("LIVE a variable was a link {:.0} ms after ⌘ came over it", started.elapsed().as_secs_f64() * 1000.);
    assert!(app.shared.servers.ready(&workspace));
    // a variable: the line that sets it
    clicked(cx, &app, item, cmd);
    assert!(app.until(cx, |w| marked(w).as_deref() == Some(sets)), "{:?}", look(cx, &app, |w| marked(w)));
    // a method of that variable: ty knows it is a StockItem, so the click goes straight to its `save` (two classes
    // have one; the rules would list both)
    let save = rest_on(cx, &app, saves, "Line 6", "save");
    until_link(cx, &app, saves, "save");
    clicked(cx, &app, save, cmd);
    assert!(app.until(cx, |w| opened(w).as_deref() == Some("orders/models.py")), "the click went nowhere");
    assert!(look(cx, &app, |w| w.try_find("definitions-popup").is_none()), "the list opened though ty named one");
    assert!(look(cx, &app, |w| w.find_all("source").iter().any(|l| l.description() == Some("Line 2, marked"))));
    app.press(cx, "cmd-[");
    assert!(app.until(cx, |w| opened(w).as_deref() == Some("orders/views.py")));

    // Claude writes the models file: three lines more at its top. The watcher tells the core, the core tells ty
    // with the next question, and the click lands on the method's new line
    write(&folder, "orders/models.py", &format!("import os\n\nLIMIT = 3\n{MODELS}"));
    assert!(app.until(cx, |w| said(w, "changes-count").as_deref() == Some("2 files")), "the edit never came");
    let asked = Instant::now();
    loop {
        let save = rest_on(cx, &app, saves, "Line 6", "save");
        until_link(cx, &app, saves, "save");
        clicked(cx, &app, save, cmd);
        assert!(app.until(cx, |w| opened(w).as_deref() == Some("orders/models.py")));
        assert!(app.until(cx, |w| marked(w).as_deref() == Some("    def save(self):")));
        if look(cx, &app, |w| w.find_all("source").iter().any(|l| l.description() == Some("Line 5, marked"))) {
            break;
        }
        assert!(asked.elapsed() < Duration::from_secs(10), "ty kept naming the method's old line");
        app.press(cx, "cmd-[");
        assert!(app.until(cx, |w| opened(w).as_deref() == Some("orders/views.py")));
    }
    println!("LIVE the method's new line {:.0} ms after the edit was listed", asked.elapsed().as_secs_f64() * 1000.);
    // the workspace's folder holds what the test wrote and nothing of ty's
    let mut left: Vec<String> = (std::fs::read_dir(folder.join("orders")).unwrap())
        .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    left.sort();
    assert_eq!(left, ["__init__.py", "models.py", "views.py"]);
    assert_eq!(git(&folder, &["status", "--porcelain", "--untracked-files=all"]).lines().count(), 2);
}
