//! Text with a shape, as the card draws it in the page's own type: a ticket's description as Jira wrote it (Atlassian
//! Document Format) and a PR's description as GitHub writes it (Markdown) both become the same blocks: paragraphs,
//! headings, lists and checklists, code, quotes, panels, tables and links. Nothing in them is ever run or fetched:
//! links are words that open in the browser (web links only), images and files are named.

use serde_json::Value;

/// What a run of text is drawn as, when it is more than words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chip {
    /// A person named in Jira ("@Mira").
    Mention,
    /// A small status word of Jira's.
    Status,
    /// An image or a file, named and never fetched.
    File,
}

/// A run of text with its marks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub strong: bool,
    pub em: bool,
    pub strike: bool,
    pub underline: bool,
    pub code: bool,
    /// The web page it opens (in the browser, never in the app).
    pub link: Option<String>,
    pub chip: Option<Chip>,
}

impl Span {
    pub fn plain(text: impl Into<String>) -> Span {
        Span { text: text.into(), ..Span::default() }
    }

    fn chip(text: impl Into<String>, chip: Chip) -> Span {
        Span { text: text.into(), chip: Some(chip), ..Span::default() }
    }

    fn link(text: impl Into<String>, url: impl Into<String>) -> Span {
        Span { text: text.into(), link: Some(url.into()), ..Span::default() }
    }
}

/// How a panel of Jira's is tinted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Info,
    Note,
    Success,
    Warning,
    Error,
}

/// A cell of a table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    pub header: bool,
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Paragraph(Vec<Span>),
    /// A heading: a big one (Jira's and Markdown's first two levels), or a small one.
    Heading {
        big: bool,
        spans: Vec<Span>,
    },
    /// A list: numbered from `start`, or with bullets.
    List {
        start: Option<u64>,
        items: Vec<Vec<Block>>,
    },
    /// A checklist: its items, and lists inside it.
    Tasks(Vec<Block>),
    /// One item of a checklist.
    Task {
        done: bool,
        spans: Vec<Span>,
    },
    /// Code, as it was written.
    Code(String),
    Quote(Vec<Block>),
    Panel {
        kind: Panel,
        blocks: Vec<Block>,
    },
    Rule,
    /// Something folded behind its title until it is opened.
    Fold {
        title: String,
        blocks: Vec<Block>,
    },
    /// A table: its rows of cells.
    Table(Vec<Vec<Cell>>),
    /// An image or a file, named ("An image or file, in Jira").
    File(String),
    /// Plain text as it is, its line breaks kept (an older ticket's description).
    Pre(String),
}

/// What a file or an image of Jira's is called: it is never fetched.
pub const JIRA_FILE: &str = "An image or file, in Jira";

fn web(url: &str) -> bool {
    let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://")
}

fn text_of(value: &Value, key: &str) -> Option<String> {
    match value.get(key) {
        Some(Value::String(text)) => Some(text.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        Some(Value::Bool(b)) => Some(b.to_string()),
        _ => None,
    }
}

fn kind(node: &Value) -> &str {
    node.get("type").and_then(Value::as_str).unwrap_or("")
}

fn content(node: &Value) -> &[Value] {
    node.get("content").and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

fn attrs(node: &Value) -> &Value {
    node.get("attrs").unwrap_or(&Value::Null)
}

/// All the text under a node, as it is.
fn plain_of(node: &Value) -> String {
    let own = node.get("text").and_then(Value::as_str).unwrap_or("");
    content(node).iter().fold(own.to_string(), |mut all, child| {
        all.push_str(&plain_of(child));
        all
    })
}

/// A run of Jira's text with its marks: bold, italic, code, struck, underlined, linked.
fn adf_text(node: &Value) -> Span {
    let mut span = Span::plain(node.get("text").and_then(Value::as_str).unwrap_or(""));
    for mark in node.get("marks").and_then(Value::as_array).map_or(&[][..], Vec::as_slice) {
        match kind(mark) {
            "strong" => span.strong = true,
            "em" => span.em = true,
            "strike" => span.strike = true,
            "underline" => span.underline = true,
            "code" => span.code = true,
            // a link that isn't to the web stays plain text
            "link" => span.link = text_of(attrs(mark), "href").filter(|href| web(href)),
            _ => {}
        }
    }
    span
}

/// A day as Jira's date chip names it: "8 Nov 2026".
fn day(at: i64) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let date = super::when::local(at);
    format!("{} {} {}", date.day, MONTHS[(date.month as usize).clamp(1, 12) - 1], date.year)
}

fn adf_inline(nodes: &[Value], out: &mut Vec<Span>) {
    for node in nodes {
        let a = attrs(node);
        match kind(node) {
            "text" => out.push(adf_text(node)),
            "hardBreak" => out.push(Span::plain("\n")),
            "mention" => out.push(Span::chip(text_of(a, "text").unwrap_or_else(|| "@someone".into()), Chip::Mention)),
            "emoji" => {
                out.push(Span::plain(text_of(a, "text").or_else(|| text_of(a, "shortName")).unwrap_or_default()))
            }
            "inlineCard" => {
                if let Some(url) = text_of(a, "url").filter(|url| web(url)) {
                    out.push(Span::link(url.clone(), url));
                }
            }
            "status" => out.push(Span::chip(text_of(a, "text").unwrap_or_default(), Chip::Status)),
            "date" => {
                let at = text_of(a, "timestamp").and_then(|at| at.parse::<i64>().ok()).filter(|at| *at != 0);
                out.extend(at.map(|at| Span::plain(day(at))));
            }
            "mediaInline" => out.push(Span::chip(JIRA_FILE, Chip::File)),
            // something newer than this page knows: its text, so nothing is lost
            _ if !content(node).is_empty() => adf_inline(content(node), out),
            _ => out.extend(node.get("text").and_then(Value::as_str).map(Span::plain)),
        }
    }
    out.retain(|span| !span.text.is_empty());
}

fn spans(node: &Value) -> Vec<Span> {
    let mut out = Vec::new();
    adf_inline(content(node), &mut out);
    out
}

fn adf_blocks(nodes: &[Value]) -> Vec<Block> {
    let mut out = Vec::new();
    for node in nodes {
        let a = attrs(node);
        match kind(node) {
            "paragraph" => {
                let spans = spans(node);
                if !spans.is_empty() {
                    out.push(Block::Paragraph(spans));
                }
            }
            "heading" => {
                let level = a.get("level").and_then(Value::as_f64).unwrap_or(3.);
                out.push(Block::Heading { big: level <= 2., spans: spans(node) });
            }
            "bulletList" | "orderedList" => {
                let ordered = kind(node) == "orderedList";
                let start = a.get("order").and_then(Value::as_u64).filter(|order| *order != 0).unwrap_or(1);
                let items = (content(node).iter())
                    .map(|item| match kind(item) {
                        "listItem" => adf_blocks(content(item)),
                        _ => adf_blocks(std::slice::from_ref(item)),
                    })
                    .collect();
                out.push(Block::List { start: ordered.then_some(start), items });
            }
            "listItem" => out.extend(adf_blocks(content(node))),
            "taskList" => out.push(Block::Tasks(adf_blocks(content(node)))),
            "taskItem" => {
                let done = a.get("state").and_then(Value::as_str) == Some("DONE");
                let mut spans = spans(node);
                // a thing that is done is struck through, as in Jira
                spans.iter_mut().for_each(|span| span.strike |= done);
                out.push(Block::Task { done, spans });
            }
            "codeBlock" => out.push(Block::Code(plain_of(node))),
            "blockquote" => out.push(Block::Quote(adf_blocks(content(node)))),
            "panel" => {
                let kind = match a.get("panelType").and_then(Value::as_str) {
                    Some("note") => Panel::Note,
                    Some("success") => Panel::Success,
                    Some("warning") => Panel::Warning,
                    Some("error") => Panel::Error,
                    _ => Panel::Info,
                };
                out.push(Block::Panel { kind, blocks: adf_blocks(content(node)) });
            }
            "rule" => out.push(Block::Rule),
            "expand" | "nestedExpand" => {
                let title = text_of(a, "title").filter(|title| !title.is_empty()).unwrap_or_else(|| "More".into());
                out.push(Block::Fold { title, blocks: adf_blocks(content(node)) });
            }
            "table" => {
                let rows = (content(node).iter())
                    .map(|row| {
                        (content(row).iter())
                            .map(|cell| Cell { header: kind(cell) == "tableHeader", blocks: adf_blocks(content(cell)) })
                            .collect()
                    })
                    .collect();
                out.push(Block::Table(rows));
            }
            "mediaSingle" | "mediaGroup" => out.push(Block::File(JIRA_FILE.to_string())),
            "blockCard" | "embedCard" => {
                if let Some(url) = text_of(a, "url").filter(|url| web(url)) {
                    out.push(Block::Paragraph(vec![Span::link(url.clone(), url)]));
                }
            }
            // something newer than this page knows: its text, so nothing is lost
            _ if !content(node).is_empty() => out.extend(adf_blocks(content(node))),
            _ => out.extend(
                (node.get("text").and_then(Value::as_str))
                    .filter(|text| !text.is_empty())
                    .map(|text| Block::Paragraph(vec![Span::plain(text)])),
            ),
        }
    }
    out
}

/// A Jira description as blocks. Older tickets have plain text instead of a document: it is shown as it is.
pub fn jira_doc(description: &Value) -> Vec<Block> {
    match description {
        Value::String(text) => vec![Block::Pre(text.clone())],
        doc => adf_blocks(content(doc)),
    }
}

fn spans_text(spans: &[Span], out: &mut String) {
    spans.iter().for_each(|span| out.push_str(&span.text));
}

fn blocks_text(blocks: &[Block], out: &mut Vec<String>) {
    for block in blocks {
        match block {
            Block::Paragraph(spans) | Block::Heading { spans, .. } | Block::Task { spans, .. } => {
                let mut line = String::new();
                spans_text(spans, &mut line);
                out.push(line);
            }
            Block::List { items, .. } => items.iter().for_each(|item| blocks_text(item, out)),
            Block::Tasks(blocks) | Block::Quote(blocks) | Block::Panel { blocks, .. } => blocks_text(blocks, out),
            Block::Fold { title, blocks } => {
                out.push(title.clone());
                blocks_text(blocks, out);
            }
            Block::Table(rows) => {
                rows.iter().flatten().for_each(|cell| blocks_text(&cell.blocks, out));
            }
            Block::Code(text) | Block::File(text) | Block::Pre(text) => out.push(text.clone()),
            Block::Rule => {}
        }
    }
}

/// Everything the blocks say, a line per paragraph, heading, list item or cell: what a screen reader (and a test)
/// reads.
pub fn plain(blocks: &[Block]) -> String {
    let mut lines = Vec::new();
    blocks_text(blocks, &mut lines);
    lines.join("\n")
}

/// Every web page the blocks link to, in their order.
pub fn links(blocks: &[Block]) -> Vec<String> {
    fn of(blocks: &[Block], out: &mut Vec<String>) {
        for block in blocks {
            match block {
                Block::Paragraph(spans) | Block::Heading { spans, .. } | Block::Task { spans, .. } => {
                    out.extend(spans.iter().filter_map(|span| span.link.clone()));
                }
                Block::List { items, .. } => items.iter().for_each(|item| of(item, out)),
                Block::Tasks(blocks)
                | Block::Quote(blocks)
                | Block::Panel { blocks, .. }
                | Block::Fold { blocks, .. } => of(blocks, out),
                Block::Table(rows) => rows.iter().flatten().for_each(|cell| of(&cell.blocks, out)),
                Block::Code(_) | Block::File(_) | Block::Pre(_) | Block::Rule => {}
            }
        }
    }
    let mut out = Vec::new();
    of(blocks, &mut out);
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn text(words: &str) -> Value {
        json!({ "type": "text", "text": words })
    }

    fn marked(words: &str, mark: Value) -> Value {
        json!({ "type": "text", "text": words, "marks": [mark] })
    }

    fn doc(content: Value) -> Value {
        json!({ "type": "doc", "content": content })
    }

    fn para(content: Value) -> Value {
        json!({ "type": "paragraph", "content": content })
    }

    /// Draws Jira's document: headings, lists, checklists, code, tables and links.
    #[test]
    fn reads_jiras_document_headings_lists_checklists_code_tables_and_links() {
        let description = doc(json!([
            { "type": "heading", "attrs": { "level": 2 }, "content": [text("What to build")] },
            para(json!([
                text("Use "),
                marked("decimal_separator", json!({ "type": "code" })),
                text(" from the outlet, "),
                marked("bold", json!({ "type": "strong" })),
            ])),
            { "type": "bulletList", "content": [{ "type": "listItem", "content": [para(json!([text("one")]))] }] },
            { "type": "orderedList", "attrs": { "order": 3 },
              "content": [{ "type": "listItem", "content": [para(json!([text("third")]))] }] },
            { "type": "taskList", "content": [
                { "type": "taskItem", "attrs": { "state": "DONE" }, "content": [text("done thing")] }] },
            { "type": "codeBlock", "content": [text("print(1)")] },
            { "type": "table", "content": [{ "type": "tableRow", "content": [
                { "type": "tableHeader", "content": [para(json!([text("Column")]))] }] }] },
            para(json!([
                marked("the spec", json!({ "type": "link", "attrs": { "href": "https://example.com/spec" } })),
                { "type": "mention", "attrs": { "text": "@Mira" } },
            ])),
            { "type": "mediaSingle", "content": [{ "type": "media", "attrs": { "id": "m1" } }] },
            { "type": "someNewBlock", "content": [para(json!([text("kept anyway")]))] },
        ]));
        let blocks = jira_doc(&description);
        let said = |words: &str| vec![Span::plain(words)];
        assert_eq!(blocks[0], Block::Heading { big: true, spans: said("What to build") });
        let Block::Paragraph(spans) = &blocks[1] else { panic!("{:?}", blocks[1]) };
        assert_eq!((spans[1].text.as_str(), spans[1].code), ("decimal_separator", true));
        assert_eq!((spans[3].text.as_str(), spans[3].strong), ("bold", true));
        assert_eq!(blocks[2], Block::List { start: None, items: vec![vec![Block::Paragraph(said("one"))]] });
        assert_eq!(blocks[3], Block::List { start: Some(3), items: vec![vec![Block::Paragraph(said("third"))]] });
        let done = Span { text: "done thing".into(), strike: true, ..Span::default() };
        assert_eq!(blocks[4], Block::Tasks(vec![Block::Task { done: true, spans: vec![done] }]));
        assert_eq!(blocks[5], Block::Code("print(1)".into()));
        let header = Cell { header: true, blocks: vec![Block::Paragraph(said("Column"))] };
        assert_eq!(blocks[6], Block::Table(vec![vec![header]]));
        // a link opens in the browser through the app, never in the window
        let Block::Paragraph(spans) = &blocks[7] else { panic!("{:?}", blocks[7]) };
        assert_eq!(spans[0], Span::link("the spec", "https://example.com/spec"));
        assert_eq!(spans[1], Span::chip("@Mira", Chip::Mention));
        assert_eq!(blocks[8], Block::File("An image or file, in Jira".into()));
        // a block this page doesn't know keeps its text
        assert_eq!(blocks[9], Block::Paragraph(said("kept anyway")));
        assert_eq!(blocks.len(), 10);
        assert_eq!(links(&blocks), ["https://example.com/spec"]);
        let all = plain(&blocks);
        assert!(all.starts_with("What to build\nUse decimal_separator from the outlet, bold\none\nthird\n"), "{all}");
    }

    /// Never runs what a description says, and links only to the web.
    #[test]
    fn never_runs_what_a_description_says_and_links_only_to_the_web() {
        let description = doc(json!([para(json!([
            text("<img src=x onerror=alert(1)>"),
            marked("click", json!({ "type": "link", "attrs": { "href": "javascript:alert(1)" } })),
        ]))]));
        let blocks = jira_doc(&description);
        // a tag is its own letters, and what isn't a web link is plain text
        assert_eq!(blocks, [Block::Paragraph(vec![Span::plain("<img src=x onerror=alert(1)>"), Span::plain("click")])]);
        assert!(links(&blocks).is_empty());
        // a card that isn't a web link is dropped, as a block and in a line
        let cards = doc(json!([
            { "type": "blockCard", "attrs": { "url": "file:///etc/passwd" } },
            para(json!([{ "type": "inlineCard", "attrs": { "url": "HTTPS://example.com/Page" } }])),
        ]));
        assert_eq!(links(&jira_doc(&cards)), ["HTTPS://example.com/Page"]);
    }

    /// Draws every recorded description (from the real Jira, anonymized) with all of its text.
    #[test]
    fn reads_every_recorded_description_with_all_of_its_text() {
        let recorded = include_str!("../../../enkidu-core/tests/fixtures/work/twg-describe.json");
        let answer: Value = serde_json::from_str(recorded).expect("the recorded answer");
        let docs: Vec<&Value> = (answer["data"]["items"].as_array().expect("its items").iter())
            .filter(|item| item["ok"] == json!(true) && !item["data"]["description"].is_null())
            .map(|item| &item["data"]["description"])
            .collect();
        assert_eq!(docs.len(), 41);
        fn words(node: &Value, out: &mut Vec<String>) {
            if kind(node) == "text"
                && let Some(text) = node.get("text").and_then(Value::as_str).filter(|text| !text.is_empty())
            {
                out.push(text.to_string());
            }
            content(node).iter().for_each(|child| words(child, out));
        }
        let mut kinds = std::collections::BTreeSet::new();
        fn seen(blocks: &[Block], kinds: &mut std::collections::BTreeSet<&'static str>) {
            for block in blocks {
                let name = match block {
                    Block::Paragraph(_) => "paragraph",
                    Block::Heading { .. } => "heading",
                    Block::List { items, .. } => {
                        items.iter().for_each(|item| seen(item, kinds));
                        "list"
                    }
                    Block::Tasks(blocks) => {
                        seen(blocks, kinds);
                        "tasks"
                    }
                    Block::Task { .. } => "task",
                    Block::Code(_) => "code",
                    Block::Quote(_) => "quote",
                    Block::Panel { .. } => "panel",
                    Block::Rule => "rule",
                    Block::Fold { blocks, .. } => {
                        seen(blocks, kinds);
                        "fold"
                    }
                    Block::Table(_) => "table",
                    Block::File(_) => "file",
                    Block::Pre(_) => "pre",
                };
                kinds.insert(name);
            }
        }
        for description in docs {
            let blocks = jira_doc(description);
            // every run of text of the document is in what is drawn, none between two blocks lost
            let all = plain(&blocks).replace('\n', "");
            let mut expected = Vec::new();
            words(description, &mut expected);
            for word in expected {
                assert!(all.contains(&word.replace('\n', "")), "{word:?} is lost from {all}");
            }
            seen(&blocks, &mut kinds);
        }
        let met: Vec<&str> = kinds.into_iter().collect();
        assert_eq!(met, ["code", "file", "fold", "heading", "list", "paragraph", "table", "task", "tasks"]);
    }

    /// Shows an older plain-text description as it is.
    #[test]
    fn shows_an_older_plain_text_description_as_it_is() {
        let blocks = jira_doc(&json!("line one\nline two"));
        assert_eq!(blocks, [Block::Pre("line one\nline two".into())]);
        assert_eq!(plain(&blocks), "line one\nline two");
    }

    /// What else Jira writes: a line break, a person, a status, a date, an emoji, quotes, panels, a fold, a rule.
    #[test]
    fn reads_the_rest_of_what_jira_writes() {
        let at = super::super::when::moment(2026, 11, 8, 12, 0);
        let description = doc(json!([
            para(json!([
                text("one"),
                { "type": "hardBreak" },
                text("two "),
                { "type": "status", "attrs": { "text": "BLOCKED" } },
                { "type": "emoji", "attrs": { "shortName": ":tada:", "text": "🎉" } },
                { "type": "date", "attrs": { "timestamp": at.to_string() } },
                { "type": "mediaInline", "attrs": { "id": "f1" } },
                { "type": "mention", "attrs": {} },
            ])),
            { "type": "blockquote", "content": [para(json!([text("quoted")]))] },
            { "type": "panel", "attrs": { "panelType": "warning" }, "content": [para(json!([text("careful")]))] },
            { "type": "panel", "attrs": { "panelType": "new-kind" }, "content": [para(json!([text("fyi")]))] },
            { "type": "expand", "attrs": { "title": "" }, "content": [para(json!([text("inside")]))] },
            { "type": "rule" },
            { "type": "paragraph", "content": [] },
            { "type": "heading", "attrs": { "level": 4 }, "content": [text("Small")] },
            { "type": "taskList", "content": [
                { "type": "taskItem", "attrs": { "state": "TODO" }, "content": [text("to do")] },
                { "type": "taskList", "content": [
                    { "type": "taskItem", "attrs": { "state": "TODO" }, "content": [text("under it")] }] }] },
        ]));
        let blocks = jira_doc(&description);
        let Block::Paragraph(spans) = &blocks[0] else { panic!("{:?}", blocks[0]) };
        let said: Vec<&str> = spans.iter().map(|span| span.text.as_str()).collect();
        assert_eq!(said, ["one", "\n", "two ", "BLOCKED", "🎉", "8 Nov 2026", JIRA_FILE, "@someone"]);
        assert_eq!(
            spans.iter().map(|span| span.chip).collect::<Vec<_>>(),
            [None, None, None, Some(Chip::Status), None, None, Some(Chip::File), Some(Chip::Mention)]
        );
        let p = |words: &str| vec![Block::Paragraph(vec![Span::plain(words)])];
        assert_eq!(blocks[1], Block::Quote(p("quoted")));
        assert_eq!(blocks[2], Block::Panel { kind: Panel::Warning, blocks: p("careful") });
        assert_eq!(blocks[3], Block::Panel { kind: Panel::Info, blocks: p("fyi") });
        assert_eq!(blocks[4], Block::Fold { title: "More".into(), blocks: p("inside") });
        assert_eq!(blocks[5], Block::Rule);
        assert_eq!(blocks[6], Block::Heading { big: false, spans: vec![Span::plain("Small")] });
        let task = |words: &str| Block::Task { done: false, spans: vec![Span::plain(words)] };
        assert_eq!(blocks[7], Block::Tasks(vec![task("to do"), Block::Tasks(vec![task("under it")])]));
        assert_eq!(blocks.len(), 8, "an empty paragraph is nothing");
    }
}
