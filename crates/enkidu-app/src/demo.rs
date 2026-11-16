//! Sample data for pictures and screen tests: three workspace tabs, and terminal output with colors, a wide
//! character, Arabic and a cursor, so a picture shows how the terminal draws.

use gpui_kit::SharedString;

use enkidu_ui::lamp::Status;

/// A workspace tab: its lamp, its name, and what it waits for.
#[derive(Clone, Debug)]
pub struct Tab {
    pub name: SharedString,
    pub status: Status,
    /// The tab's words: "Needs you", "Live" (it is the project's Live workspace) or a count of changed files.
    pub note: Option<SharedString>,
}

pub fn tabs() -> Vec<Tab> {
    vec![
        Tab { name: "stock-fix".into(), status: Status::Working, note: Some("5".into()) },
        Tab { name: "sh-21309".into(), status: Status::NeedsYou, note: Some("Needs you".into()) },
        Tab { name: "pr-2175".into(), status: Status::Done, note: Some("Live".into()) },
    ]
}

/// Output for a kept terminal: what a short Claude Code session might show.
pub fn terminal_output() -> Vec<u8> {
    let coral = "\x1b[38;2;232;132;95m";
    let dim = "\x1b[38;2;124;129;140m";
    let green = "\x1b[32m";
    let red = "\x1b[31m";
    let amber = "\x1b[33m";
    let bold = "\x1b[1m";
    let reset = "\x1b[0m";
    let lines = [
        format!("{coral}✻{reset} {bold}Welcome to Claude Code{reset}"),
        String::new(),
        format!("  {dim}cwd: ~/code/shop{reset}"),
        String::new(),
        format!("{dim}>{reset} fix the double count in stock.py when an order is returned"),
        String::new(),
        format!("{coral}●{reset} I'll read how returns update the stock first."),
        String::new(),
        format!("{coral}●{reset} {bold}Read{reset}(src/orders/stock.py)"),
        format!("  {dim}⎿  Read 214 lines{reset}"),
        String::new(),
        format!("{coral}●{reset} {bold}Update{reset}(src/orders/stock.py)"),
        format!("  {dim}⎿  Updated src/orders/stock.py with 3 additions and 1 removal{reset}"),
        format!("      {red}48 -        item.stock += line.quantity{reset}"),
        format!("      {green}48 +        if not line.restocked:{reset}"),
        format!("      {green}49 +            item.stock += line.quantity{reset}"),
        format!("      {green}50 +            line.restocked = True{reset}"),
        String::new(),
        format!(
            "{coral}●{reset} The returned order is counted once now. Wide 中文 and Arabic مرحبا stay in their cells."
        ),
        format!("{coral}●{reset} Opened https://github.com/acme/shop/pull/12 for review."),
        format!("  {dim}No ligatures: -> => != === <= >= :: // www{reset}"),
        String::new(),
        format!("{amber}✶{reset} Running the tests… {dim}(12s · esc to interrupt){reset}"),
        String::new(),
        format!("{dim}╭────────────────────────────────────────────────────────────╮{reset}"),
        format!("{dim}│{reset} > "),
    ];
    let mut out = lines.join("\r\n");
    // Leave the cursor in the prompt box, after "> ".
    out.push_str("\x1b[?25h");
    out.into_bytes()
}

/// A sample frame for pictures and screen tests: the project "shop" with these tabs (the one at `selected` on
/// screen), and "enkidu" with one workspace that needs you, so its tab shows the red count.
pub fn frame(tabs: &[Tab], selected: usize) -> enkidu_model::frame::Frame {
    use enkidu_core::db::{Project, Workspace};
    let project = |id: &str, name: &str| Project {
        id: id.into(),
        name: name.into(),
        path: format!("~/code/{name}"),
        live_workspace_id: None,
        created_at: 1,
    };
    let workspace = |id: String, project_id: &str, name: &str, branch: &str| Workspace {
        id,
        project_id: project_id.into(),
        name: name.into(),
        path: format!("~/enkidu/workspaces/{project_id}/{name}"),
        branch: branch.into(),
        base: Some("development".into()),
        session_id: String::new(),
        transcript_path: None,
        claude_config_dir: None,
        created_at: 1,
        adopted: false,
    };
    let mut workspaces = Vec::new();
    let mut statuses = std::collections::HashMap::new();
    for (ix, tab) in tabs.iter().enumerate() {
        let id = format!("w{ix}");
        let branch = if ix == 0 { "fix/stock-double-count".to_string() } else { format!("feature/{}", tab.name) };
        workspaces.push(workspace(id.clone(), "p1", &tab.name, &branch));
        statuses.insert(id, tab.status.core_word().to_string());
    }
    workspaces.push(workspace("r0".into(), "p2", "lamp-motion", "feature/lamp-motion"));
    statuses.insert("r0".into(), "needs_you".into());
    let view = enkidu_model::frame::View {
        project_id: Some("p1".into()),
        selected: [("p1".to_string(), format!("w{selected}"))].into(),
        work: false,
        ..Default::default()
    };
    let mut frame = enkidu_model::frame::Frame::load(
        vec![project("p1", "shop"), project("p2", "enkidu")],
        workspaces,
        statuses,
        &view,
    );
    for (ix, tab) in tabs.iter().enumerate() {
        match tab.note.as_deref() {
            Some("Live") => frame.projects[0].live_workspace_id = Some(format!("w{ix}")),
            Some(count) => {
                if let Ok(files) = count.parse::<usize>() {
                    // about two lines added and one removed a file, so the status line has its counts
                    let changes = enkidu_model::frame::Changes {
                        files,
                        additions: if files == 5 { 13 } else { files as u64 * 2 },
                        deletions: if files == 5 { 6 } else { files as u64 },
                    };
                    frame.changed.insert(format!("w{ix}"), changes);
                }
            }
            None => {}
        }
    }
    frame
}

/// One changed file for a picture: what git says of it, and its diff with the whole file in it. `lines` are the
/// diff's own, each starting with a space (unchanged), `-` (removed) or `+` (added); none for a binary file.
fn changed(path: &str, kind: &str, lines: &[&str]) -> (enkidu_core::git::Change, String) {
    let count = |sign: char| lines.iter().filter(|line| line.starts_with(sign)).count();
    let (same, removed, added) = (count(' '), count('-'), count('+'));
    let (before, after) = (same + removed, same + added);
    let mut patch = format!("diff --git a/{path} b/{path}\n");
    if lines.is_empty() {
        patch.push_str(&format!("Binary files a/{path} and b/{path} differ\n"));
    } else {
        match kind {
            "untracked" | "added" => patch.push_str(&format!("new file mode 100644\n--- /dev/null\n+++ b/{path}\n")),
            "deleted" => patch.push_str(&format!("deleted file mode 100644\n--- a/{path}\n+++ /dev/null\n")),
            _ => patch.push_str(&format!("--- a/{path}\n+++ b/{path}\n")),
        }
        let start = |lines: usize| usize::from(lines > 0);
        patch.push_str(&format!("@@ -{},{before} +{},{after} @@\n", start(before), start(after)));
        for line in lines {
            patch.push_str(line);
            patch.push('\n');
        }
    }
    let change = enkidu_core::git::Change {
        path: path.into(),
        old_path: None,
        kind: kind.into(),
        staged: false,
        additions: (!lines.is_empty()).then_some(added as u64),
        deletions: (!lines.is_empty()).then_some(removed as u64),
        sig: "demo".into(),
    };
    (change, patch)
}

/// What the demo workspace has changed, for pictures of the changes island: an edit with unchanged lines to fold, a
/// new file, a small edit, a deleted file and a binary one.
pub fn changes() -> Vec<(enkidu_core::git::Change, String)> {
    let stock = [
        " \"\"\"Stock levels, kept in step with orders and returns.\"\"\"",
        " ",
        " from decimal import Decimal",
        " ",
        " from shop.orders.models import Order, OrderLine",
        "+from shop.orders.signals import stock_changed",
        " from shop.stock.models import StockItem",
        " ",
        " ",
        " def reserve(order: Order) -> None:",
        "     \"\"\"Takes an order's lines out of stock when it is placed.\"\"\"",
        "     for line in order.lines.select_related(\"item\"):",
        "         item = line.item",
        "         item.stock -= line.quantity",
        "         item.save(update_fields=[\"stock\"])",
        " ",
        " ",
        " def total_value(items: list[StockItem]) -> Decimal:",
        "     \"\"\"What the stock on hand is worth, at cost.\"\"\"",
        "     return sum((item.stock * item.cost for item in items), Decimal(0))",
        " ",
        " ",
        " def restock(order: Order) -> None:",
        "-    \"\"\"Puts a returned order's lines back into stock.\"\"\"",
        "+    \"\"\"Puts a returned order's lines back into stock, once.\"\"\"",
        "     for line in order.lines.select_related(\"item\"):",
        "         item = line.item",
        "-        item.stock += line.quantity",
        "-        item.save(update_fields=[\"stock\"])",
        "+        if not line.restocked:",
        "+            item.stock += line.quantity",
        "+            line.restocked = True",
        "+            line.save(update_fields=[\"restocked\"])",
        "+            item.save(update_fields=[\"stock\"])",
        "+            stock_changed.send(sender=StockItem, item=item)",
        " ",
        " ",
        " def low_stock(threshold: int = 5) -> list[StockItem]:",
        "     \"\"\"The items to order again.\"\"\"",
        "     return list(StockItem.objects.filter(stock__lte=threshold).order_by(\"stock\"))",
    ];
    let test = [
        "+from shop.orders.models import Order",
        "+from shop.stock import levels",
        "+",
        "+",
        "+def test_a_returned_order_is_counted_once(order: Order):",
        "+    item = order.lines.first().item",
        "+    before = item.stock",
        "+    levels.restock(order)",
        "+    levels.restock(order)",
        "+    item.refresh_from_db()",
        "+    assert item.stock == before + order.lines.first().quantity",
    ];
    let readme = [
        " # shop",
        " ",
        "-Orders and stock for the shop floor.",
        "+Orders, returns and stock for the shop floor.",
        " ",
        " Run the tests with `pytest`.",
    ];
    let notes =
        ["-Returns are counted twice when an order is returned in two parts.", "-Ask Sam how the tills report it."];
    vec![
        changed("README.md", "modified", &readme),
        changed("docs/returns-bug.md", "deleted", &notes),
        changed("src/orders/stock.py", "modified", &stock),
        changed("static/img/returns.png", "added", &[]),
        changed("tests/test_stock.py", "untracked", &test),
    ]
}

/// The same changes among more of them, for pictures of the Changed files tree (item 29.5): a folder with folders and
/// files of its own, a chain of single folders, and files of several kinds.
pub fn tree_changes() -> Vec<(enkidu_core::git::Change, String)> {
    let more: [(&str, &str, &[&str]); 13] = [
        ("docs/returns.md", "untracked", &["+# Returns", "+", "+A returned order is counted once."]),
        ("pyproject.toml", "modified", &[" [project]", "-version = \"1.4.0\"", "+version = \"1.4.1\""]),
        (
            "src/billing/invoices/pdf/render.py",
            "modified",
            &[
                " def render(invoice):",
                "-    return draw(invoice)",
                "+    return draw(invoice, credit=invoice.returned)",
            ],
        ),
        (
            "src/orders/api/serializers.py",
            "modified",
            &[" class OrderLineSerializer(Serializer):", "+    restocked = BooleanField(read_only=True)"],
        ),
        (
            "src/orders/api/views.py",
            "modified",
            &[
                " class ReturnView(APIView):",
                "-    queryset = Order.objects.all()",
                "+    queryset = Order.objects.prefetch_related(\"lines\")",
            ],
        ),
        (
            "src/orders/models.py",
            "modified",
            &[" class OrderLine(Model):", "+    restocked = BooleanField(default=False)"],
        ),
        (
            "src/orders/signals.py",
            "untracked",
            &["+from django.dispatch import Signal", "+", "+stock_changed = Signal()"],
        ),
        (
            "src/stock/levels.py",
            "modified",
            &[" def on_hand(item):", "-    return item.stock", "+    return max(item.stock, 0)"],
        ),
        (
            "tests/orders/test_returns.py",
            "untracked",
            &["+def test_a_return_in_two_parts(order):", "+    assert order.returned"],
        ),
        (
            "tests/orders/test_signals.py",
            "untracked",
            &["+def test_stock_changed_is_sent_once(order):", "+    assert order.lines.count()"],
        ),
        (
            "web/src/api/returns.ts",
            "modified",
            &[
                " export async function returned(id: number) {",
                "-  return get(`/orders/${id}`);",
                "+  return get(`/orders/${id}/returns`);",
            ],
        ),
        (
            "web/src/components/ReturnForm.vue",
            "modified",
            &[" <template>", "-  <form>", "+  <form @submit.prevent=\"send\">"],
        ),
        (
            "web/src/components/StockBadge.vue",
            "untracked",
            &["+<template>", "+  <span class=\"badge\">{{ count }}</span>", "+</template>"],
        ),
    ];
    let mut all = changes();
    all.extend(more.iter().map(|(path, kind, lines)| changed(path, kind, lines)));
    all
}

/// New files in a few languages, for a picture of their syntax colors (item 3.3).
pub fn languages() -> Vec<(enkidu_core::git::Change, String)> {
    let files: [(&str, &[&str]); 11] = [
        (
            "api/cart.ts",
            &[
                "import type { Item } from './types';",
                "",
                "export interface Cart { items: Item[]; total: number }",
                "",
                "// adds an item and keeps the total",
                "export function add(cart: Cart, item: Item): Cart {",
                "  return { items: [...cart.items, item], total: cart.total + item.price * 1.2 };",
                "}",
            ],
        ),
        ("config/app.toml", &["[server]", "host = \"0.0.0.0\"", "port = 8080", "debug = false # never in production"]),
        (
            "config/package.json",
            &[
                "{",
                "  \"name\": \"shop\",",
                "  \"private\": true,",
                "  \"scripts\": { \"dev\": \"vite\", \"count\": 3 }",
                "}",
            ],
        ),
        (
            "deploy/compose.yaml",
            &[
                "services:",
                "  web:",
                "    image: \"nginx:1.25\"",
                "    ports: [80, 443]",
                "    restart: always  # keep it up",
            ],
        ),
        (
            "docs/notes.md",
            &[
                "# Returns",
                "",
                "A returned order is counted **once**. Run `pytest` and see [the plan](https://example.com/plan).",
                "",
                "- stock",
                "- orders",
            ],
        ),
        (
            "scripts/release.sh",
            &[
                "#!/bin/sh",
                "set -eu",
                "for f in dist/*.tgz; do",
                "  echo \"uploading $f\" | tee -a 'release.log' # one by one",
                "done",
            ],
        ),
        (
            "sql/orders.sql",
            &[
                "SELECT id, total FROM orders WHERE status = 'returned' AND total > 10 ORDER BY id; -- the returns",
                "UPDATE orders SET restocked = TRUE WHERE id = 42;",
            ],
        ),
        (
            "web/Cart.vue",
            &[
                "<template>",
                "  <ul class=\"cart\" @click=\"add(item)\">",
                "    <li v-for=\"item in items\" :key=\"item.id\">{{ item.name }}</li>",
                "  </ul>",
                "</template>",
                "",
                "<script setup>",
                "import { ref } from 'vue';",
                "const items = ref([]); // filled by the store",
                "</script>",
                "",
                "<style scoped>",
                ".cart { color: #7aa2ff; margin: 0 4px; }",
                "</style>",
            ],
        ),
        (
            "web/index.html",
            &[
                "<!doctype html>",
                "<div id=\"app\" class=\"shop\"><a href=\"/cart\">Cart</a><!-- mounted by Vue --></div>",
            ],
        ),
        (
            "web/main.js",
            &[
                "import { createApp } from 'vue';",
                "const app = createApp(App); // the shop",
                "app.mount('#app');",
                "export const VERSION = `v${1 + 2}`;",
            ],
        ),
        (
            "web/theme.scss",
            &[
                "$gap: 4px;",
                ".cart {",
                "  .row { margin: $gap; color: darken($ink, 10%); }",
                "}",
                "@mixin wide { width: 100%; }",
            ],
        ),
    ];
    files
        .iter()
        .map(|(path, lines)| {
            let added: Vec<String> = lines.iter().map(|line| format!("+{line}")).collect();
            let added: Vec<&str> = added.iter().map(String::as_str).collect();
            changed(path, "untracked", &added)
        })
        .collect()
}

/// The files git would list in the demo workspace, for pictures of the Files panel: the changed ones are among them.
pub fn files() -> Vec<String> {
    [
        ".gitignore",
        "README.md",
        "docs/returns-bug.md",
        "docs/stock-notes.md",
        "manage.py",
        "pyproject.toml",
        "src/orders/__init__.py",
        "src/orders/models.py",
        "src/orders/serializers.py",
        "src/orders/signals.py",
        "src/orders/stock.py",
        "src/orders/views.py",
        "src/shop/settings/base.py",
        "src/shop/settings/local.py",
        "src/shop/urls.py",
        "src/stock/models.py",
        "static/css/shop.css",
        "static/img/logo.svg",
        "tests/conftest.py",
        "web/app/components/cart/CartLine.vue",
    ]
    .map(String::from)
    .to_vec()
}

/// What Search answers in a picture: the workspace's files, the definitions that "slv" finds, and the lines that
/// have "stock" in them.
pub fn search(files: Vec<String>) -> crate::search::Given {
    use enkidu_core::search::{FileHits, LineHit, TextSummary};
    use enkidu_core::symbols::{Kind, SymbolHit, SymbolResults};
    let symbol = |name: &str, kind: Kind, container: Option<&str>, path: &str, line: u32, indices: &[u32]| SymbolHit {
        name: name.into(),
        kind,
        container: container.map(String::from),
        path: path.into(),
        line,
        indices: indices.to_vec(),
    };
    // a line and where "stock" is in it, whatever its case, counted in characters
    let line = |line: u32, text: &str| {
        let lower = text.to_lowercase();
        let ranges = lower
            .match_indices("stock")
            .map(|(at, found)| {
                let from = lower[..at].chars().count() as u32;
                [from, from + found.chars().count() as u32]
            })
            .collect();
        LineHit { line, text: text.into(), ranges }
    };
    crate::search::Given {
        files,
        symbols: SymbolResults {
            hits: vec![
                symbol("StockLevelView", Kind::Class, None, "src/orders/views.py", 18, &[0, 5, 10]),
                symbol("stock_level_value", Kind::Function, None, "src/orders/stock.py", 41, &[0, 6, 12]),
                symbol("save_level", Kind::Method, Some("StockLevelView"), "src/orders/views.py", 52, &[0, 5, 2]),
                symbol("StockLevelSerializer", Kind::Class, None, "src/orders/serializers.py", 9, &[0, 5, 10]),
                symbol("ShopLiveSettings", Kind::Struct, None, "src/shop/settings/local.py", 4, &[0, 4, 6]),
                symbol("CartLineView", Kind::Component, None, "web/app/components/cart/CartLine.vue", 1, &[4, 8]),
            ],
            total: 6,
            symbols: 1_284,
            files: 20,
            ..Default::default()
        },
        text: vec![
            FileHits {
                path: "README.md".into(),
                lines: vec![line(3, "Orders, returns and stock for the shop floor.")],
                more: 0,
            },
            FileHits {
                path: "src/orders/stock.py".into(),
                lines: vec![
                    line(6, "from shop.orders.signals import stock_changed"),
                    line(7, "from shop.stock.models import StockItem"),
                    line(24, "\"\"\"Puts a returned order's lines back into stock, once.\"\"\""),
                    line(28, "item.stock += line.quantity"),
                    line(31, "item.save(update_fields=[\"stock\"])"),
                ],
                more: 4,
            },
            FileHits {
                path: "src/stock/models.py".into(),
                lines: vec![
                    line(12, "class StockItem(models.Model):"),
                    line(15, "stock = models.DecimalField(max_digits=12, decimal_places=3)"),
                ],
                more: 0,
            },
            FileHits { path: "tests/conftest.py".into(), lines: vec![line(8, "def stock_item(db):")], more: 0 },
        ],
        summary: TextSummary { lines: 13, files: 4, searched: 20, matched_files: 4, ms: 3, ..Default::default() },
        // for the preview under the rows: the changed files as their patches leave them, the others as they are
        patches: changes().into_iter().map(|(change, patch)| (change.path, patch)).collect(),
        texts: [
            ("README.md", "# Shop\n\nOrders, returns and stock for the shop floor.\n\n## Run it\n\n    make dev\n"),
            (
                "src/stock/models.py",
                "from decimal import Decimal\n\nfrom django.db import models\n\nfrom shop.core.models import Library\n\n\nclass \
                 Supplier(models.Model):\n    name = models.CharField(max_length=120)\n\n\nclass \
                 StockItem(models.Model):\n    library = models.ForeignKey(Library, on_delete=models.CASCADE)\n    name = \
                 models.CharField(max_length=120)\n    stock = models.DecimalField(max_digits=12, decimal_places=3)\n    cost = \
                 models.DecimalField(max_digits=12, decimal_places=4, default=Decimal(0))\n\n    def __str__(self):\n        \
                 return self.name\n",
            ),
            (
                "tests/conftest.py",
                "from decimal import Decimal\n\nimport pytest\n\nfrom shop.stock.models import StockItem\n\n@pytest.fixture\ndef \
                 stock_item(db):\n    return StockItem.objects.create(name=\"Flour\", stock=Decimal(\"12.5\"))\n",
            ),
        ]
        .into_iter()
        .map(|(path, text)| (path.to_string(), text.to_string()))
        .collect(),
    }
}

/// The moment the pictures' workflow runs are as old as they say: 8 Nov 2026, 10:00 UTC.
const SETTINGS_NOW: i64 = 1_794_132_000_000;

/// The sample project's latest workflow runs: a summary that runs, one that didn't for a signed-out Jira, one that
/// was written, and failing checks sent to a Claude.
fn workflow_runs() -> Vec<enkidu_core::work::workflows::Run> {
    let run = |id: i64, workflow: &str, state: &str, text: &str, minutes: i64| enkidu_core::work::workflows::Run {
        id,
        project_id: "p1".into(),
        workflow: workflow.into(),
        subject: "2026-11-08".into(),
        state: state.into(),
        at: SETTINGS_NOW - minutes * 60_000,
        text: text.into(),
    };
    vec![
        run(4, "morning", "running", "", 0),
        run(
            3,
            "morning",
            "skipped",
            "Didn't run: Jira can't be read: twg isn't signed in. Run `twg login --force`.",
            26,
        ),
        run(2, "failed_checks", "done", "Sent `tests (3.12)` of PR #2414 to the Claude in sh-21309-export.", 190),
        run(1, "morning", "done", "Claude wrote 9 lines on the tickets' timelines. Summarized 9 tickets.", 24 * 60 + 5),
    ]
}

/// The settings a picture's dialog shows: one account's Claude, and the sample project's own.
/// Which state of Setup a window without a core shows (pictures and screen tests set it before Settings opens).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SetupSample {
    /// Everything there and signed in.
    #[default]
    Ready,
    /// A Mac that is new to it: Git there, Claude Code not installed, `gh` signed out, `twg` missing.
    New,
    /// Claude Code and GitHub ready, Jira ready with a project that reads none of it yet, and a build that lacks
    /// its Python server.
    Partly,
}

impl gpui_kit::Global for SetupSample {}

/// What the sample Setup page found.
pub fn setup(sample: SetupSample) -> crate::setup::Given {
    use enkidu_core::setup::{Found, Parts, State, Thing};
    let found = |thing: Thing, state: State, command: &str, path: &str, version: &str| Found {
        thing,
        state,
        command: command.into(),
        path: (state != State::Missing).then(|| path.to_string()),
        version: (state != State::Missing).then(|| version.to_string()),
        signed_in: None,
        account_dir: None,
        accounts: Vec::new(),
        said: None,
        fix: None,
    };
    let git = found(Thing::Git, State::Ready, "git", "/usr/bin/git", "2.50.1");
    let claude = |state: State| Found {
        signed_in: (state == State::Ready).then_some(true),
        ..found(Thing::Claude, state, "claude", "/Users/demo/.local/bin/claude", "2.1.325")
    };
    let github = |state: State, accounts: &[&str]| Found {
        accounts: accounts.iter().map(|name| name.to_string()).collect(),
        ..found(Thing::Github, state, "gh", "/opt/homebrew/bin/gh", "2.103.3")
    };
    let jira = |state: State, accounts: &[&str]| Found {
        accounts: accounts.iter().map(|name| name.to_string()).collect(),
        ..found(Thing::Jira, state, "twg", "/Users/demo/.local/bin/twg", "1.4.2")
    };
    let whole = Parts { cli: true, python_server: true };
    match sample {
        SetupSample::Ready => crate::setup::Given {
            found: vec![
                git,
                claude(State::Ready),
                github(State::Ready, &["me-dev", "me-work"]),
                jira(State::Ready, &["Me Dev"]),
            ],
            parts: Some(whole),
            without_jira: Vec::new(),
        },
        SetupSample::New => crate::setup::Given {
            found: vec![git, claude(State::Missing), github(State::SignedOut, &[]), jira(State::Missing, &[])],
            parts: Some(whole),
            without_jira: Vec::new(),
        },
        SetupSample::Partly => crate::setup::Given {
            found: vec![git, claude(State::Ready), github(State::Ready, &["me-dev"]), jira(State::Ready, &["Me Dev"])],
            parts: Some(Parts { cli: true, python_server: false }),
            without_jira: vec![("p2".into(), "enkidu".into())],
        },
    }
}

pub fn settings(cx: &gpui_kit::App) -> crate::settings::Source {
    use enkidu_core::settings::{ProjectSettings, UserSettings};
    let mut user = UserSettings { workspaces_dir: "~/enkidu/workspaces".into(), ..UserSettings::default() };
    user.claude.command = "claude".into();
    user.claude.args = vec!["--model".into(), "opus".into()];
    user.claude.env.insert("CLAUDE_CONFIG_DIR".into(), "~/.claude-work".into());
    let mut shop = ProjectSettings {
        default_base: "development".into(),
        copy_files: vec![".env".into(), ".claude/settings.local.json".into()],
        setup_script: "make setup".into(),
        ..ProjectSettings::default()
    };
    shop.live.on = "docker compose restart web".into();
    shop.live.message = "You are live in the shop's dev environment.".into();
    shop.claude.args = vec!["--permission-mode".into(), "acceptEdits".into()];
    shop.work.jira_keys = vec!["SH".into()];
    shop.work.back_to_you = vec!["QA Failed".into(), "Reopened".into()];
    shop.work.ignored_checks = vec!["codecov/patch".into()];
    shop.work.morning_summary = true;
    shop.work.morning_at = "08:30".into();
    let preview = "claude --settings ~/Library/…/hooks/stock-fix.json --session-id 3f6c… --name stock-fix \
                   --append-system-prompt '…' --model opus --permission-mode acceptEdits";
    crate::settings::Source::Given(Box::new(crate::settings::Given {
        user,
        projects: [("p1".to_string(), shop)].into_iter().collect(),
        preview: Some(("stock-fix".into(), preview.into())),
        hooks: Some(claude_hooks()),
        runs: [("p1".to_string(), workflow_runs())].into_iter().collect(),
        now: SETTINGS_NOW,
        setup: setup(cx.try_global::<SetupSample>().copied().unwrap_or_default()),
    }))
}

/// The Claude folders a picture's Settings list: one that has the hooks, one that has not.
pub fn claude_hooks() -> enkidu_core::commands::ClaudeHooks {
    use enkidu_core::work::claudehooks::{HookFile, adds, command};
    let cli = std::path::Path::new("/Applications/Enkidu.app/Contents/MacOS/enkidu");
    let file = |dir: &str, added: bool| HookFile {
        dir: dir.into(),
        path: format!("{dir}/settings.json"),
        added,
        exists: true,
    };
    enkidu_core::commands::ClaudeHooks {
        cli: Some(cli.to_string_lossy().into()),
        command: Some(command(cli)),
        files: vec![file("~/.claude", true), file("~/.claude-work", false)],
        adds: Some(serde_json::to_string_pretty(&adds(cli)).unwrap_or_default()),
    }
}
