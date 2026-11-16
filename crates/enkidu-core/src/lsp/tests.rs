//! The client of language servers, against the fake one (`tests/fakes/fake-lsp.mjs`, run with `node`), and, when
//! `ENKIDU_LIVE_TY` names the real `ty`, against that one on a throwaway folder (ignored otherwise).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::*;

struct Fake {
    _home: tempfile::TempDir,
    /// The workspace's folder, and the folder servers are started in.
    root: PathBuf,
    folder: PathBuf,
    log: PathBuf,
    mode: PathBuf,
    answer: PathBuf,
    program: Program,
}

fn fake() -> Fake {
    let home = tempfile::tempdir().unwrap();
    let base = home.path().canonicalize().unwrap();
    let (root, folder) = (base.join("work space"), base.join("data/servers"));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("app.py"), "from models import Item\n\nitem = Item()\n").unwrap();
    std::fs::write(root.join("models.py"), "import os\n\n\nclass Item:\n    pass\n").unwrap();
    let (log, mode, answer) = (base.join("log"), base.join("mode"), base.join("answer"));
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fakes/fake-lsp.mjs");
    let program = Program::Shell(format!(
        "FAKE_LSP_LOG='{}' FAKE_LSP_MODE='{}' FAKE_LSP_ANSWER='{}' exec node '{}'",
        log.display(),
        mode.display(),
        answer.display(),
        script.display()
    ));
    Fake { _home: home, root, folder, log, mode, answer, program }
}

impl Fake {
    /// A question about `Item` on the third line of `app.py`.
    fn ask(&self, servers: &Servers, workspace: &str, changed: Option<&HashSet<String>>) -> Option<Vec<Named>> {
        let text = std::fs::read_to_string(self.root.join("app.py")).unwrap();
        let question = Question { path: "app.py", text: &text, language: "python", line: 2, character: 7 };
        servers.definition(workspace, &self.program, &self.folder, &self.root, &|| changed.cloned(), &question)
    }

    /// Asks until a server is ready for the workspace (the first question starts it and is not answered).
    fn ready(&self, servers: &Servers, workspace: &str) {
        let nothing = HashSet::new();
        assert_eq!(self.ask(servers, workspace, Some(&nothing)), None, "the question that starts a server");
        let started = Instant::now();
        while !servers.ready(workspace) {
            assert!(started.elapsed() < Duration::from_secs(20), "the fake server never said hello");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn says(&self, places: &str) {
        std::fs::write(&self.answer, places).unwrap();
    }

    /// What the server was told, in order.
    fn log(&self) -> Vec<Value> {
        let text = std::fs::read_to_string(&self.log).unwrap_or_default();
        text.lines().map(|line| serde_json::from_str(line).unwrap()).collect()
    }

    /// The same once `seen` holds (the server writes its log as it reads).
    fn log_when(&self, seen: impl Fn(&[Value]) -> bool) -> Vec<Value> {
        let started = Instant::now();
        loop {
            let log = self.log();
            if seen(&log) {
                return log;
            }
            assert!(started.elapsed() < Duration::from_secs(10), "the server's log never said it: {log:?}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn place(&self, file: &str, line: usize) -> String {
        format!(
            "{{\"uri\": \"{}\", \"range\": {{\"start\": {{\"line\": {line}, \"character\": 6}}, \"end\": {{\"line\": \
             {line}, \"character\": 10}}}}}}",
            uri_of(&self.root.join(file))
        )
    }
}

fn methods(log: &[Value]) -> Vec<String> {
    log.iter().filter_map(|entry| entry["method"].as_str().map(String::from)).collect()
}

fn alive(pid: u64) -> bool {
    let mut kill = std::process::Command::new("kill");
    kill.args(["-0", &pid.to_string()]).stderr(std::process::Stdio::null());
    kill.status().is_ok_and(|status| status.success())
}

/// Item 12.1: the first question about a workspace starts its server and is the rules'; the next ones are the
/// server's. It is started in the app's own folder, without the app's own variables, and told only the file the
/// question is about.
#[test]
fn the_first_question_starts_a_server_and_the_next_ones_are_its_own() {
    let fake = fake();
    // (a variable of the app's own, that no server should see)
    std::env::set_var("ENKIDU_TEST_MARK", "1");
    let servers = Servers::default();
    fake.ready(&servers, "w");
    let nothing = HashSet::new();

    // one place in the workspace: its file from the workspace's folder, its line from 1
    fake.says(&format!("[{}]", fake.place("models.py", 3)));
    assert_eq!(fake.ask(&servers, "w", Some(&nothing)), Some(vec![(Some("models.py".into()), 4)]));
    // a place alone (not in a list), and one said with the words servers use for a link
    fake.says(&fake.place("models.py", 3));
    assert_eq!(fake.ask(&servers, "w", Some(&nothing)), Some(vec![(Some("models.py".into()), 4)]));
    let link = format!(
        "[{{\"targetUri\": \"{}\", \"targetRange\": {{}}, \"targetSelectionRange\": {{\"start\": {{\"line\": 0, \
         \"character\": 0}}}}}}]",
        uri_of(&fake.root.join("models.py"))
    );
    fake.says(&link);
    assert_eq!(fake.ask(&servers, "w", Some(&nothing)), Some(vec![(Some("models.py".into()), 1)]));
    // several places, one of them outside the workspace (a library's file): that one has no path here
    fake.says(&format!(
        "[{}, {{\"uri\": \"file:///usr/lib/python/os.py\", \"range\": {{\"start\": {{\"line\": 9, \"character\": 0}}}}}}]",
        fake.place("models.py", 3)
    ));
    assert_eq!(fake.ask(&servers, "w", Some(&nothing)), Some(vec![(Some("models.py".into()), 4), (None, 10)]));
    // nothing found
    fake.says("null");
    assert_eq!(fake.ask(&servers, "w", Some(&nothing)), Some(Vec::new()));

    let log = fake.log_when(|log| methods(log).iter().filter(|method| *method == "textDocument/didClose").count() == 6);
    // started in the folder of the app's own, not in the workspace; none of the app's variables reached it
    assert_eq!(log[0]["started"].as_str(), Some(fake.folder.to_string_lossy().as_ref()));
    assert_eq!(log[0]["own"], serde_json::json!([]), "the app's own variables reached the server");
    // it was told the workspace's folder, and that the app says which files change
    assert_eq!(log[1]["method"], "initialize");
    assert_eq!(log[1]["root"].as_str(), Some(uri_of(&fake.root).as_str()));
    assert!(uri_of(&fake.root).ends_with("/work%20space"), "{}", uri_of(&fake.root));
    assert_eq!(log[1]["watches"]["dynamicRegistration"], true);
    assert_eq!(log[2]["method"], "initialized");
    // each question: the file as it is on disk, the place, and the file given back. The first of them is the
    // question that made the server start: it was given to it as soon as it had said hello, and nobody waited
    assert_eq!(
        methods(&log[3..6]),
        ["textDocument/didOpen", "textDocument/definition", "textDocument/didClose"],
        "{log:?}"
    );
    assert_eq!(log[3]["uri"].as_str(), Some(uri_of(&fake.root.join("app.py")).as_str()));
    assert_eq!(log[3]["language"], "python");
    assert_eq!(log[3]["text"], "from models import Item\n\nitem = Item()\n");
    assert_eq!(log[4]["position"], serde_json::json!({"line": 2, "character": 7}));
    // nothing else was ever said to it: no file was changed, nothing was written
    assert_eq!(methods(&log).len(), 2 + 6 * 3, "{log:?}");
}

/// Item 12.2: a server that is slow, silent, dead or mute changes nothing but who answers: the question comes back
/// empty within the limit, and the rules are asked.
#[test]
fn a_slow_a_silent_or_a_dead_server_only_leaves_the_question_to_the_rules() {
    let fake = fake();
    let servers = Servers::default();
    fake.ready(&servers, "w");
    let nothing = HashSet::new();
    fake.says(&format!("[{}]", fake.place("models.py", 3)));

    // slow: no answer within the limit, and the question returns then, not when the answer comes
    std::fs::write(&fake.mode, "slow").unwrap();
    let asked = Instant::now();
    assert_eq!(fake.ask(&servers, "w", Some(&nothing)), None);
    let waited = asked.elapsed();
    assert!(waited >= ANSWER_WAIT && waited < ANSWER_WAIT + Duration::from_millis(400), "waited {waited:?}");
    // while it still owes that answer it is not asked anything: the next questions come back at once, and nothing
    // piles up for it to work through. It was told that nobody waits for the first one any more
    std::fs::write(&fake.mode, "ok").unwrap();
    fake.says(&format!("[{}]", fake.place("models.py", 0)));
    let asked = Instant::now();
    for _ in 0..20 {
        assert_eq!(fake.ask(&servers, "w", Some(&nothing)), None, "a busy server was asked");
    }
    assert!(asked.elapsed() < Duration::from_millis(100), "twenty questions waited {:?}", asked.elapsed());
    let log = fake.log_when(|log| methods(log).contains(&"$/cancelRequest".to_string()));
    assert_eq!(methods(&log).iter().filter(|method| *method == "textDocument/definition").count(), 2, "{log:?}");
    // its late answer is not taken for the next question's
    std::thread::sleep(Duration::from_millis(1100));
    assert_eq!(fake.ask(&servers, "w", Some(&nothing)), Some(vec![(Some("models.py".into()), 1)]));

    // it dies when asked: nothing, and it is not asked again for a while (nor started over and over)
    std::fs::write(&fake.mode, "dies").unwrap();
    assert_eq!(fake.ask(&servers, "w", Some(&nothing)), None);
    std::thread::sleep(Duration::from_millis(200));
    std::fs::write(&fake.mode, "ok").unwrap();
    assert_eq!(fake.ask(&servers, "w", Some(&nothing)), None, "a dead server answered");
    assert!(!servers.ready("w"));
    let starts = |log: &[Value]| log.iter().filter(|entry| entry.get("started").is_some()).count();
    assert_eq!(fake.ask(&servers, "w", Some(&nothing)), None);
    assert_eq!(starts(&fake.log()), 1, "a server that died was started again at once");

    // silent: it never answers. It is kept and left alone, then started afresh once it has owed its answer long
    // enough to be taken for stuck
    let patient = Servers { stuck: Duration::from_millis(600), ..Servers::default() };
    std::fs::write(&fake.mode, "ok").unwrap();
    fake.ready(&patient, "s");
    std::fs::write(&fake.mode, "silent").unwrap();
    assert_eq!(fake.ask(&patient, "s", Some(&nothing)), None);
    assert_eq!(fake.ask(&patient, "s", Some(&nothing)), None);
    assert!(patient.program("s").is_some() && !patient.ready("s"), "a server that only is slow is kept");
    let before = starts(&fake.log());
    std::thread::sleep(Duration::from_millis(700));
    std::fs::write(&fake.mode, "ok").unwrap();
    assert_eq!(fake.ask(&patient, "s", Some(&nothing)), None, "the question that starts it afresh");
    fake.log_when(|log| starts(log) == before + 1);
    let waited = Instant::now();
    while !patient.ready("s") {
        assert!(waited.elapsed() < Duration::from_secs(20), "the fresh server never was ready");
        std::thread::sleep(Duration::from_millis(10));
    }
    fake.says("null");
    assert_eq!(fake.ask(&patient, "s", Some(&nothing)), Some(Vec::new()));
    drop(patient);

    // one that can't be started at all, and one that doesn't find definitions: never ready, never asked
    std::fs::write(&fake.mode, "dead").unwrap();
    assert_eq!(fake.ask(&servers, "gone", Some(&nothing)), None);
    // (the fake reads its mode as it starts: it is given the time to start and end before the mode changes)
    std::thread::sleep(Duration::from_millis(500));
    std::fs::write(&fake.mode, "mute").unwrap();
    let hellos = |log: &[Value]| methods(log).iter().filter(|method| *method == "initialize").count();
    let so_far = hellos(&fake.log());
    assert_eq!(fake.ask(&servers, "mute", Some(&nothing)), None);
    let mute = fake.log_when(|log| hellos(log) == so_far + 1);
    std::thread::sleep(Duration::from_millis(300));
    assert!(!servers.ready("gone") && !servers.ready("mute"));
    assert_eq!(fake.ask(&servers, "mute", Some(&nothing)), None);
    assert_eq!(fake.ask(&servers, "gone", Some(&nothing)), None);
    assert_eq!(methods(&fake.log()).len(), methods(&mute).len(), "a server that finds no definitions was asked");
    let missing = Program::Direct(fake.root.join("no-such-program"), Vec::new());
    let text = "x = 1\n";
    let question = Question { path: "app.py", text, language: "python", line: 0, character: 0 };
    let none = || Some(HashSet::new());
    assert_eq!(servers.definition("none", &missing, &fake.folder, &fake.root, &none, &question), None);
    std::thread::sleep(Duration::from_millis(200));
    assert!(!servers.ready("none"));
}

/// Item 12.3: what the watcher saw change is told to the server before the question; too much to tell, and it is
/// started afresh. A server's own questions (its settings, which files to watch) are answered at once.
#[test]
fn what_changed_on_disk_is_told_first_and_too_much_starts_the_server_afresh() {
    let fake = fake();
    std::fs::write(&fake.mode, "asks").unwrap();
    let servers = Servers::default();
    fake.ready(&servers, "w");
    // what it asked right after hello was answered without any question of ours being asked
    let log = fake.log_when(|log| log.iter().filter(|entry| entry.get("answered").is_some()).count() == 2);
    let answered: Vec<&Value> = log.iter().filter(|entry| entry.get("answered").is_some()).collect();
    assert_eq!(answered[0]["result"], serde_json::json!([null, null]), "its settings: one answer for each asked");
    assert_eq!(answered[1]["result"], Value::Null);

    // a file that changed and one that is gone, then the question
    std::fs::write(&fake.mode, "ok").unwrap();
    fake.says("null");
    let changed: HashSet<String> = ["models.py".to_string(), "old/gone.py".to_string()].into();
    assert_eq!(fake.ask(&servers, "w", Some(&changed)), Some(Vec::new()));
    let log = fake.log_when(|log| methods(log).contains(&"textDocument/didClose".to_string()));
    let said = methods(&log);
    let told = said.iter().position(|method| method == "workspace/didChangeWatchedFiles").expect("told what changed");
    assert_eq!(said[told + 1], "textDocument/didOpen", "{said:?}");
    let told = log.iter().find(|entry| entry["method"] == "workspace/didChangeWatchedFiles").unwrap();
    let mut events: Vec<(String, u64)> = (told["changes"].as_array().unwrap().iter())
        .map(|event| (event["uri"].as_str().unwrap().to_string(), event["type"].as_u64().unwrap()))
        .collect();
    events.sort();
    let (models, gone) = (uri_of(&fake.root.join("models.py")), uri_of(&fake.root.join("old/gone.py")));
    let mut expected = vec![(models.clone(), 1), (models, 2), (gone, 3)];
    expected.sort();
    assert_eq!(events, expected, "a file that is there is said new and changed, one that is gone, gone");
    // nothing changed: nothing is told
    assert_eq!(fake.ask(&servers, "w", Some(&HashSet::new())), Some(Vec::new()));
    let count = |log: &[Value]| methods(log).iter().filter(|m| *m == "workspace/didChangeWatchedFiles").count();
    assert_eq!(count(&fake.log()), 1);

    // too much changed to say (a checkout): the server is started afresh, and this question is the rules'
    let first = fake.log()[0]["pid"].as_u64().unwrap();
    assert_eq!(fake.ask(&servers, "w", None), None);
    let log = fake.log_when(|log| log.iter().filter(|entry| entry.get("started").is_some()).count() == 2);
    assert!(!alive(first), "the old server was left running");
    let started = Instant::now();
    while !servers.ready("w") {
        assert!(started.elapsed() < Duration::from_secs(20));
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(fake.ask(&servers, "w", Some(&HashSet::new())), Some(Vec::new()));
    assert_eq!(count(&log), 1, "the fresh server was told about files");
    // and as many files as it takes to be too much
    let many: HashSet<String> = (0..MAX_TOLD + 1).map(|n| format!("f{n}.py")).collect();
    assert_eq!(fake.ask(&servers, "w", Some(&many)), None);
    fake.log_when(|log| log.iter().filter(|entry| entry.get("started").is_some()).count() == 3);
}

/// Item 12.4: servers run for the workspaces asked last and no more, and each is stopped with its workspace and
/// when the app quits. A stopped server's program is gone.
#[test]
fn servers_are_kept_for_the_workspaces_asked_last_and_end_with_them() {
    let fake = fake();
    let servers = Servers::default();
    for workspace in ["a", "b", "c"] {
        fake.ready(&servers, workspace);
    }
    assert_eq!(servers.held(), KEEP);
    let pids = |log: &[Value]| -> Vec<u64> { log.iter().filter_map(|entry| entry["pid"].as_u64()).collect() };
    let first_three = pids(&fake.log());
    assert_eq!(first_three.len(), 3);
    assert!(first_three.iter().all(|pid| alive(*pid)));
    // "a" is asked again, so "b" is the one asked longest ago when a fourth workspace asks
    std::thread::sleep(Duration::from_millis(20));
    fake.says("null");
    assert_eq!(fake.ask(&servers, "a", Some(&HashSet::new())), Some(Vec::new()));
    fake.ready(&servers, "d");
    assert_eq!(servers.held(), KEEP);
    assert!(servers.ready("a") && !servers.ready("b") && servers.ready("c") && servers.ready("d"));
    let gone = |pid: u64| {
        let started = Instant::now();
        while alive(pid) {
            assert!(started.elapsed() < Duration::from_secs(5), "a stopped server's program still runs");
            std::thread::sleep(Duration::from_millis(10));
        }
    };
    gone(first_three[1]);
    // a workspace that is removed
    servers.stop("c");
    assert_eq!(servers.held(), 2);
    gone(first_three[2]);
    // the app quits
    let all = pids(&fake.log());
    servers.stop_all();
    assert_eq!(servers.held(), 0);
    all.into_iter().for_each(gone);
}

/// Item 12.6: which program is the Python server: the variable (tests), then the file in the app's data folder (the
/// way out by hand), then the one carried beside the app's program. No test of this crate sets the variable, and no
/// `ty` stands beside a test's program.
#[test]
fn the_python_server_is_named_by_a_file_of_the_apps_own_or_is_the_one_carried() {
    assert!(std::env::var_os("ENKIDU_PYTHON_SERVER").is_none(), "a test named a server for the whole crate");
    let data = tempfile::tempdir().unwrap();
    assert_eq!(python_program(data.path()), None, "no server carried beside a test's program");
    let file = data.path().join(PYTHON_SERVER_FILE);
    std::fs::write(&file, "pyrefly lsp\n").unwrap();
    assert_eq!(python_program(data.path()), Some(Program::Shell("pyrefly lsp".into())));
    for off in ["off\n", "", "  \n"] {
        std::fs::write(&file, off).unwrap();
        assert_eq!(python_program(data.path()), None, "{off:?}");
    }
}

/// Item 12.5: a server nobody asks is stopped by itself (it holds memory), and the next question starts one again.
#[test]
fn a_server_nobody_asks_is_stopped_and_started_again_by_the_next_question() {
    let fake = fake();
    let servers = Servers { idle: Duration::from_millis(400), ..Servers::default() };
    fake.ready(&servers, "w");
    let pid = fake.log()[0]["pid"].as_u64().unwrap();
    fake.says("null");
    // asked now and then, it stays
    for _ in 0..4 {
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(fake.ask(&servers, "w", Some(&HashSet::new())), Some(Vec::new()), "it was stopped while asked");
    }
    assert!(alive(pid));
    // left alone, it is stopped: its program ends
    let left = Instant::now();
    while alive(pid) {
        assert!(left.elapsed() < Duration::from_secs(5), "a server nobody asked kept running");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(left.elapsed() >= Duration::from_millis(300), "it was stopped before its time: {:?}", left.elapsed());
    assert!(!servers.ready("w"));
    // the next question starts one again
    fake.ready(&servers, "w");
    assert_eq!(fake.ask(&servers, "w", Some(&HashSet::new())), Some(Vec::new()));
    let starts = fake.log().iter().filter(|entry| entry.get("started").is_some()).count();
    assert_eq!(starts, 2);
}

/// The real `ty` on a throwaway folder (never on a real project): ENKIDU_LIVE_TY=<the program> cargo test --locked
/// -p enkidu-core --lib lsp::tests::the_real_ty -- --ignored --nocapture. It checks what the fake can't: that the
/// questions are ones a real server answers, and that a file changed on disk is seen once the server is told.
#[test]
#[ignore]
fn the_real_ty_finds_a_class_a_variable_and_what_changed_on_disk() {
    let ty = std::env::var("ENKIDU_LIVE_TY").expect("set ENKIDU_LIVE_TY to the ty program");
    let home = tempfile::tempdir().unwrap();
    let base = home.path().canonicalize().unwrap();
    let (root, folder) = (base.join("work space"), base.join("servers"));
    std::fs::create_dir_all(root.join("orders")).unwrap();
    let models = "class StockItem:\n    quantity = 0\n\n    def save(self):\n        pass\n";
    let views =
        "from orders.models import StockItem\n\n\ndef resolve(pk):\n    item = StockItem()\n    item.save()\n    \
                 return item\n";
    std::fs::write(root.join("orders/__init__.py"), "").unwrap();
    std::fs::write(root.join("orders/models.py"), models).unwrap();
    std::fs::write(root.join("orders/views.py"), views).unwrap();
    let program = Program::Direct(PathBuf::from(ty), vec!["server".into()]);
    let servers = Servers::default();
    let ask = |line: usize, word: &str, changed: Option<&HashSet<String>>| {
        let text = std::fs::read_to_string(root.join("orders/views.py")).unwrap();
        let character = text.lines().nth(line).unwrap().find(word).unwrap();
        let question = Question { path: "orders/views.py", text: &text, language: "python", line, character };
        servers.definition("w", &program, &folder, &root, &|| changed.cloned(), &question)
    };
    let nothing = HashSet::new();
    let started = Instant::now();
    assert_eq!(ask(4, "StockItem", Some(&nothing)), None, "the question that starts it");
    while !servers.ready("w") {
        assert!(started.elapsed() < Duration::from_secs(20), "ty never said hello");
        std::thread::sleep(Duration::from_millis(5));
    }
    println!("LIVE ty said hello {:.0} ms after the first question", started.elapsed().as_secs_f64() * 1000.);
    // its first answer may take longer than the limit (it reads the project then): asked until one comes
    let until = |line: usize, word: &str, changed: Option<&HashSet<String>>| {
        let asked = Instant::now();
        loop {
            if let Some(places) = ask(line, word, changed.filter(|_| asked.elapsed() < Duration::from_millis(1))) {
                return places;
            }
            assert!(asked.elapsed() < Duration::from_secs(20), "ty never answered");
        }
    };
    let asked = Instant::now();
    assert_eq!(until(4, "StockItem", Some(&nothing)), [(Some("orders/models.py".into()), 1)], "a class, by its import");
    println!("LIVE its first answer {:.0} ms after it was asked", asked.elapsed().as_secs_f64() * 1000.);
    // a method of a variable whose class it works out, and a variable's own line: what the rules can't know
    let asked = Instant::now();
    assert_eq!(ask(5, "save", Some(&nothing)), Some(vec![(Some("orders/models.py".into()), 4)]));
    println!("LIVE a later answer in {:.2} ms", asked.elapsed().as_secs_f64() * 1000.);
    assert_eq!(ask(6, "item", Some(&nothing)), Some(vec![(Some("orders/views.py".into()), 5)]));

    // Claude writes the models file: the method moves down three lines. Told that the file changed, it says the
    // new line; a new file is found too
    std::fs::write(root.join("orders/models.py"), format!("import os\n\nLIMIT = 3\n{models}")).unwrap();
    std::fs::write(root.join("orders/stock.py"), "def level():\n    return 1\n").unwrap();
    let with_stock = format!("from orders.stock import level\n{views}");
    std::fs::write(root.join("orders/views.py"), &with_stock).unwrap();
    let changed: HashSet<String> =
        ["orders/models.py", "orders/stock.py", "orders/views.py"].into_iter().map(String::from).collect();
    assert_eq!(until(6, "save", Some(&changed)), [(Some("orders/models.py".into()), 7)], "the file as it is now");
    assert_eq!(until(0, "level", Some(&nothing)), [(Some("orders/stock.py".into()), 1)], "a file made after its start");
    // the folder it was given is as it was: three Python files and nothing else
    let mut left: Vec<String> = (std::fs::read_dir(root.join("orders")).unwrap())
        .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    left.sort();
    assert_eq!(left, ["__init__.py", "models.py", "stock.py", "views.py"]);
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1, "the server left something in the workspace");
}

/// The real `ty` asked many times about one long file, as ⌘ held while the pointer crosses code asks: ENKIDU_LIVE_TY=
/// <the program> cargo test --release --locked -p enkidu-core --lib lsp::tests::the_real_ty_on_a_long -- --ignored
/// --nocapture. It prints how long the answers take and how many came after the limit.
#[test]
#[ignore]
fn the_real_ty_on_a_long_file_asked_again_and_again() {
    let ty = std::env::var("ENKIDU_LIVE_TY").expect("set ENKIDU_LIVE_TY to the ty program");
    let home = tempfile::tempdir().unwrap();
    let base = home.path().canonicalize().unwrap();
    let (root, folder) = (base.join("workspace"), base.join("servers"));
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/__init__.py"), "").unwrap();
    std::fs::write(root.join("src/helpers.py"), "def helper(value):\n    return value\n").unwrap();
    let body: String = (0..5_000).map(|n| format!("    total_{n} = helper(total_{})\n", n.max(1) - 1)).collect();
    let text = format!("from src.helpers import helper\n\n\ndef big(total_0):\n{body}");
    std::fs::write(root.join("src/big.py"), &text).unwrap();
    // with `ENKIDU_LIVE_FILES`, as many more small files beside it (the load test's repo has 40,000)
    let more: usize = std::env::var("ENKIDU_LIVE_FILES").ok().and_then(|n| n.parse().ok()).unwrap_or(0);
    for n in 0..more {
        let at = root.join(format!("src/module_{:03}", n / 200));
        std::fs::create_dir_all(&at).unwrap();
        let text = format!("class Thing{n}:\n    def total(self, items):\n        return sum(items)\n");
        std::fs::write(at.join(format!("file_{:03}.py", n % 200)), text).unwrap();
    }
    let program = Program::Direct(PathBuf::from(ty), vec!["server".into()]);
    let servers = Servers::default();
    let nothing = || Some(HashSet::new());
    let ask = |line: usize| {
        let character = text.lines().nth(line).unwrap().find("helper").unwrap();
        let question = Question { path: "src/big.py", text: &text, language: "python", line, character };
        servers.definition("w", &program, &folder, &root, &nothing, &question)
    };
    let started = Instant::now();
    assert_eq!(ask(10), None);
    while !servers.ready("w") {
        assert!(started.elapsed() < Duration::from_secs(20), "ty never said hello");
        std::thread::sleep(Duration::from_millis(2));
    }
    println!(
        "LIVE {more} more files: ty was ready {:.0} ms after the first question",
        started.elapsed().as_secs_f64() * 1000.
    );
    let (mut took, mut late, mut first) = (Vec::new(), 0, None);
    for n in 0..60 {
        let asked = Instant::now();
        match ask(10 + n * 70) {
            Some(places) => {
                assert_eq!(places, [(Some("src/helpers.py".into()), 1)]);
                first.get_or_insert(started.elapsed());
            }
            None => late += 1,
        }
        took.push(asked.elapsed().as_secs_f64() * 1000.);
    }
    took.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!(
        "LIVE a file of 5,004 lines, 60 questions one after the other: {late} came after the limit; an answer {:.1} ms \
         in the middle, {:.1} ms at worst; the first answer in time {:.0} ms after the first question",
        took[took.len() / 2],
        took[took.len() - 1],
        first.map_or(f64::NAN, |first: Duration| first.as_secs_f64() * 1000.)
    );
}
