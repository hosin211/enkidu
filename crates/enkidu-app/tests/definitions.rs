//! Going to a definition, in a real window on real git and the real core (items 10.11 and 10.17, phases 12 and
//! 13): a ⌘-click on a name goes to where it is defined, never by itself on a guess, and for Python the language
//! server answers, and nobody else. Here the server is the
//! fake one (`enkidu-core/tests/fakes/fake-lsp.mjs`, never a real one), which names what the test tells it to.
//!
//! With ⌘ held a name the server finds is a link; a ⌘-click goes to its definition; ⌘[ comes back and ⌘] goes
//! forward again; several places are listed, each with its path; what the server doesn't find is no link.
//!
//! This file has one test and its own process: it names the server for the whole process.

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::pointing::{
    FakeServer, clicked, keys, link, look, marked, moved, no_link, on, open, opened, rest_on, until_link,
};
use common::{App, said};
use enkidu_testkit::git;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Modifiers, TestAppContext};

fn write(folder: &Path, path: &str, text: &str) {
    let file = folder.join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, text).unwrap();
}

const VIEWS: &str = "from .base import BaseView\nfrom .models import StockItem\n\n\nclass StockView(BaseView):\n    model = StockItem\n\n    def post(self, item):\n        item.save()\n        return self.get_item(item)\n";
const MODELS: &str = "class StockItem:\n    quantity = 0\n\n    def save(self):\n        pass\n\n\nclass ReturnLine:\n    def save(self):\n        pass\n";
const BASE: &str = "class BaseView:\n    def get_item(self, item):\n        return item\n";

#[gpui_kit::test]
fn cmd_click_goes_where_the_language_server_says_and_nowhere_when_it_says_nothing(cx: &mut TestAppContext) {
    let fake = FakeServer::named();
    let app = common::start_on(cx, enkidu_testkit::shop, |shared, project, _| {
        App::make(shared, project, "stock-fix", "new", "stock-fix", Some("development"))
    });
    let (_, _, folder) = app.on_screen(cx).expect("on screen");
    let folder = PathBuf::from(folder).canonicalize().unwrap();
    write(&folder, "src/orders/__init__.py", "");
    write(&folder, "src/orders/views.py", VIEWS);
    write(&folder, "src/orders/models.py", MODELS);
    write(&folder, "src/orders/base.py", BASE);
    git(&folder, &["add", "-A"]);
    git(&folder, &["commit", "-q", "-m", "orders"]);
    write(&folder, "src/app.py", "from orders.models import StockItem\n\n\ndef total(items):\n    return StockItem\n");
    assert!(app.until(cx, |w| said(w, "changes-count").as_deref() == Some("1 file")), "the change never came");
    let workspace = app.frame.read_with(cx, |frame, _| frame.current_workspace().map(|w| w.id.clone())).unwrap();
    let (views, models, base) =
        (folder.join("src/orders/views.py"), folder.join("src/orders/models.py"), folder.join("src/orders/base.py"));
    let cmd = Modifiers::command();

    // the workspace's server starts when its Python code comes on screen (a changed file is listed), before any ⌘
    let started = Instant::now();
    while !app.shared.servers.ready(&workspace) {
        assert!(started.elapsed() < Duration::from_secs(20), "no server was started for the Python on screen");
        std::thread::sleep(Duration::from_millis(10));
    }
    open(cx, &app, "src/orders/views.py");
    let uses = "    model = StockItem";

    // without ⌘ a name is only text; with ⌘ over it the server is asked, and what it finds makes the name a link
    fake.says(&[(&models, 1)]);
    let stock_item = on(cx, &app, uses, "Line 6", "StockItem");
    moved(cx, &app, stock_item, Modifiers::default());
    assert_eq!(link(cx, &app, uses), None);
    moved(cx, &app, stock_item, cmd);
    until_link(cx, &app, uses, "StockItem");
    // a name the server finds nothing for is no link: nobody guesses by its name
    fake.says(&[]);
    rest_on(cx, &app, uses, "Line 6", "model");
    no_link(cx, &app, uses, "a name the server found nothing for became a link");
    // nor is the line the name itself stands on a place to go to
    fake.says(&[(&views, 6)]);
    rest_on(cx, &app, uses, "Line 6", "model");
    no_link(cx, &app, uses, "a name became a link to itself");

    // ⌘-click: the file that defines it, at its line
    fake.says(&[(&models, 1)]);
    let stock_item = rest_on(cx, &app, uses, "Line 6", "StockItem");
    until_link(cx, &app, uses, "StockItem");
    clicked(cx, &app, stock_item, cmd);
    assert!(app.until(cx, |w| opened(w).as_deref() == Some("src/orders/models.py")), "⌘-click went nowhere");
    assert!(app.until(cx, |w| marked(w).as_deref() == Some("class StockItem:")), "{:?}", look(cx, &app, |w| marked(w)));
    // the band's first button says where it goes back to
    assert_eq!(look(cx, &app, |w| said(w, "close-file")).as_deref(), Some("Back to views.py"));

    // ⌘[ comes back to the line the click was made on, ⌘] goes forward again
    app.press(cx, "cmd-[");
    assert!(app.until(cx, |w| opened(w).as_deref() == Some("src/orders/views.py")), "⌘[ went nowhere");
    assert!(app.until(cx, |w| marked(w).as_deref() == Some(uses)), "{:?}", look(cx, &app, |w| marked(w)));
    assert_eq!(look(cx, &app, |w| said(w, "close-file")).as_deref(), Some("Back to the changes"));
    app.press(cx, "cmd-]");
    assert!(app.until(cx, |w| opened(w).as_deref() == Some("src/orders/models.py")), "⌘] went nowhere");
    // and the band's button goes back too
    app.click(cx, "close-file");
    assert!(app.until(cx, |w| opened(w).as_deref() == Some("src/orders/views.py")), "the button didn't go back");

    // a method the class gets from the class it is built on: straight to it, in another file
    fake.says(&[(&base, 2)]);
    let calls = "        return self.get_item(item)";
    let get_item = rest_on(cx, &app, calls, "Line 10", "get_item");
    until_link(cx, &app, calls, "get_item");
    clicked(cx, &app, get_item, cmd);
    assert!(app.until(cx, |w| opened(w).as_deref() == Some("src/orders/base.py")), "the base's method");
    assert!(app.until(cx, |w| marked(w).as_deref() == Some("    def get_item(self, item):")));
    app.press(cx, "cmd-[");
    assert!(app.until(cx, |w| opened(w).as_deref() == Some("src/orders/views.py")));

    // a variable: the line that sets it, in the same file (nothing but a language server knows this)
    fake.says(&[(&views, 8)]);
    let saves = "        item.save()";
    let item = rest_on(cx, &app, saves, "Line 9", "item");
    until_link(cx, &app, saves, "item");
    clicked(cx, &app, item, cmd);
    assert!(app.until(cx, |w| marked(w).as_deref() == Some("    def post(self, item):")), "the variable's own line");
    assert_eq!(look(cx, &app, |w| opened(w)).as_deref(), Some("src/orders/views.py"));

    // a name the server names several places for: the list, each with what it is and its path, and no going by
    // itself
    fake.says(&[(&models, 4), (&models, 9)]);
    let save = rest_on(cx, &app, saves, "Line 9", "save");
    until_link(cx, &app, saves, "save");
    clicked(cx, &app, save, cmd);
    assert!(app.until(cx, |w| w.try_find("definitions-popup").is_some()), "no list of the two definitions");
    look(cx, &app, |w| {
        assert_eq!(said(w, "definitions-popup").as_deref(), Some("2 definitions of save"));
        let rows: Vec<(String, String, Option<bool>)> = (0..5usize)
            .filter_map(|ix| w.try_find(("definition-row", ix)))
            .map(|row| {
                (
                    row.label().unwrap_or_default().to_string(),
                    row.description().unwrap_or_default().to_string(),
                    row.selected(),
                )
            })
            .collect();
        assert_eq!(
            rows,
            [
                ("save in StockItem".into(), "method, src/orders/models.py:4".into(), Some(true)),
                ("save in ReturnLine".into(), "method, src/orders/models.py:9".into(), Some(false)),
            ]
        );
        // the file under it is still the one the click was made in
        assert_eq!(opened(w).as_deref(), Some("src/orders/views.py"));
    });
    // ↓ and Enter go to the second
    app.press(cx, "down");
    app.press(cx, "enter");
    assert!(app.until(cx, |w| w.try_find("definitions-popup").is_none()), "the list stayed");
    assert!(app.until(cx, |w| opened(w).as_deref() == Some("src/orders/models.py")));
    assert!(app.until(cx, |w| marked(w).as_deref() == Some("    def save(self):")));
    assert!(look(cx, &app, |w| w.find_all("source").iter().any(|l| l.description() == Some("Line 9, marked"))));
    app.press(cx, "cmd-[");
    assert!(app.until(cx, |w| opened(w).as_deref() == Some("src/orders/views.py")));
    // Escape closes the list and goes nowhere
    let save = rest_on(cx, &app, saves, "Line 9", "save");
    until_link(cx, &app, saves, "save");
    clicked(cx, &app, save, cmd);
    assert!(app.until(cx, |w| w.try_find("definitions-popup").is_some()));
    app.press(cx, "escape");
    assert!(app.until(cx, |w| w.try_find("definitions-popup").is_none()), "Escape left the list open");
    assert_eq!(look(cx, &app, |w| opened(w)).as_deref(), Some("src/orders/views.py"));

    // a place outside the workspace (a library's file): the name is no link
    std::fs::write(
        &fake.answer,
        r#"[{"uri": "file:///usr/lib/python3/site-packages/django/db/models/base.py", "range": {"start": {"line": 800, "character": 8}}}]"#,
    )
    .unwrap();
    rest_on(cx, &app, saves, "Line 9", "save");
    no_link(cx, &app, saves, "a library's method became a link");

    // ⌘ pressed while the pointer rests on a name: the link is there without a move, and gone when ⌘ is let go
    fake.says(&[(&models, 1)]);
    let stock_item = on(cx, &app, uses, "Line 6", "StockItem");
    moved(cx, &app, stock_item, Modifiers::default());
    assert_eq!(link(cx, &app, uses), None);
    keys(cx, &app, cmd);
    until_link(cx, &app, uses, "StockItem");
    keys(cx, &app, Modifiers::default());
    assert_eq!(link(cx, &app, uses), None, "⌘ let go, and the link stayed");

    // the server says nothing in time: no link while it is silent, and nobody answers in its place. Once it
    // answers again, the name the pointer still rests on becomes a link without a move (the app asked again)
    std::fs::write(&fake.mode, "silent").unwrap();
    let stock_item = rest_on(cx, &app, uses, "Line 6", "StockItem");
    no_link(cx, &app, uses, "something answered in a silent server's place");
    std::fs::write(&fake.mode, "ok").unwrap();
    app.shared.servers.stop(&workspace);
    until_link(cx, &app, uses, "StockItem");
    clicked(cx, &app, stock_item, cmd);
    assert!(app.until(cx, |w| opened(w).as_deref() == Some("src/orders/models.py")), "the click went nowhere");
    app.press(cx, "cmd-[");
    assert!(app.until(cx, |w| opened(w).as_deref() == Some("src/orders/views.py")));

    // Escape leaves the file: the way back ends with it
    app.press(cx, "escape");
    assert!(app.until(cx, |w| w.try_find("file-view").is_none() && w.try_find("diff-view").is_some()), "Escape");

    // from a diff: the name on the side as it is now. ⌘[ comes back to the diffs
    let returns = "    return StockItem";
    let in_diff = rest_on(cx, &app, returns, "Line 5, after", "StockItem");
    until_link(cx, &app, returns, "StockItem");
    clicked(cx, &app, in_diff, cmd);
    assert!(app.until(cx, |w| opened(w).as_deref() == Some("src/orders/models.py")), "⌘-click in a diff");
    assert_eq!(look(cx, &app, |w| said(w, "close-file")).as_deref(), Some("Back to the changes"));
    app.press(cx, "cmd-[");
    assert!(
        app.until(cx, |w| w.try_find("file-view").is_none() && w.try_find("diff-view").is_some()),
        "⌘[ to the diffs"
    );

    // what the server was told: the workspace's folder, and for each question the file as it is on disk. It was
    // started in the app's own folder, not in the workspace, and none of the app's variables reached it
    let log = fake.log();
    let servers = app.shared.data_dir.join("servers").canonicalize().unwrap();
    assert_eq!(log[0]["started"].as_str(), Some(servers.to_string_lossy().as_ref()));
    assert_eq!(log[0]["own"], serde_json::json!([]), "the app's own variables reached the server");
    assert_eq!(log[1]["root"].as_str(), Some(format!("file://{}", folder.display()).as_str()));
    let texts: Vec<(&str, &str)> = (log.iter())
        .filter(|entry| entry["method"] == "textDocument/didOpen")
        .map(|entry| (entry["uri"].as_str().unwrap(), entry["text"].as_str().unwrap()))
        .collect();
    assert!(texts.len() > 8, "{}", texts.len());
    let app_py = "from orders.models import StockItem\n\n\ndef total(items):\n    return StockItem\n";
    for (uri, text) in texts {
        let expected = if uri.ends_with("/views.py") { VIEWS } else { app_py };
        assert!(uri.ends_with("/src/orders/views.py") || uri.ends_with("/src/app.py"), "{uri}");
        assert_eq!(text, expected, "{uri}");
    }

    // the workspace is removed: its server ends with it
    let pids: Vec<u64> = fake.log().iter().filter_map(|entry| entry["pid"].as_u64()).collect();
    app.shared.servers.stop(&workspace);
    for pid in pids {
        let started = Instant::now();
        loop {
            let mut kill = std::process::Command::new("kill");
            kill.args(["-0", &pid.to_string()]).stderr(std::process::Stdio::null());
            if !kill.status().is_ok_and(|status| status.success()) {
                break;
            }
            assert!(started.elapsed() < Duration::from_secs(5), "a server outlived its workspace");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
