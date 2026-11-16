//! A PR's description as GitHub writes it (Markdown), read into the card's blocks: headings, lists and task lists,
//! quotes, code blocks, tables, and in a line code, bold, italics and links. HTML comments (PR templates are full of
//! them) go, other tags leave their text only; links are web links or plain words; images are named, never fetched.

use std::sync::LazyLock;

use regex::Regex;

use super::rich::{Block, Cell, Chip, Span};

fn web(url: &str) -> bool {
    let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://")
}

/// A letter, a digit or an underscore: what makes `*` or `_` part of a word, not a mark.
fn word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Where `open(label](target rest)` ends, counted from just after `open`: the label (no `]` in it; it may be empty
/// only when `empty` allows), the target (no space and no `)`), and the place after the closing `)`.
fn bracketed(text: &str, empty: bool) -> Option<(&str, &str, usize)> {
    let close = text.find(']')?;
    let label = &text[..close];
    if label.is_empty() && !empty {
        return None;
    }
    let rest = text[close + 1..].strip_prefix('(')?;
    let from = close + 2;
    let target_len = rest.find(|c: char| c == ')' || c.is_whitespace()).unwrap_or(rest.len());
    if target_len == 0 {
        return None;
    }
    let end = rest[target_len..].find(')')?;
    Some((label, &rest[..target_len], from + target_len + end + 1))
}

/// Where a mark closes: `body` then `close`, the body one or more characters none of which is `stop` (or a line
/// break when `one_line`). Gives the body and the place after the closing mark.
fn until<'a>(text: &'a str, stop: char, close: &str, one_line: bool) -> Option<(&'a str, usize)> {
    let end = text.find(|c: char| c == stop || (one_line && c == '\n'))?;
    (end > 0 && text[end..].starts_with(close)).then(|| (&text[..end], end + close.len()))
}

/// The words of a line with their marks: `code`, **bold**, *italics*, [links](…), ![images](…), bare links. What
/// wins where two could start: code, images, links, bold, italics, bare links, in that order. `base` is what the
/// words around them already carry (the bold a link is in).
fn inline_into(text: &str, base: &Span, out: &mut Vec<Span>) {
    let with = |words: &str| Span { text: words.to_string(), ..base.clone() };
    let mut plain = String::new();
    let mut at = 0;
    let mut before: Option<char> = None;
    while at < text.len() {
        let rest = &text[at..];
        let Some(first) = rest.chars().next() else { break };
        // what starts here, if anything: the spans it makes and how far it reaches
        let mut made: Option<(Vec<Span>, usize)> = None;
        if first == '`' {
            let ticks = rest.len() - rest.trim_start_matches('`').len();
            let body = &rest[ticks..];
            if let Some(end) = body.find('`').filter(|end| *end > 0)
                && body[end..].starts_with(&"`".repeat(ticks))
            {
                made = Some((vec![Span { code: true, ..with(&body[..end]) }], ticks + end + ticks));
            }
        }
        if made.is_none()
            && let Some(inner) = rest.strip_prefix("![")
            && let Some((alt, _, end)) = bracketed(inner, true)
        {
            let name = if alt.is_empty() { "An image" } else { alt };
            let chip = Span { text: format!("{name}, on GitHub"), chip: Some(Chip::File), ..Span::default() };
            made = Some((vec![chip], 2 + end));
        }
        if made.is_none()
            && let Some(inner) = rest.strip_prefix('[')
            && let Some((label, href, end)) = bracketed(inner, false)
        {
            let mut spans = Vec::new();
            match web(href) {
                true => inline_into(label, &Span { link: Some(href.to_string()), ..base.clone() }, &mut spans),
                // not a web link: its words, plain
                false => spans.push(with(label)),
            }
            made = Some((spans, 1 + end));
        }
        for mark in ["**", "__"] {
            if made.is_none()
                && let Some(inner) = rest.strip_prefix(mark)
                && let Some((body, end)) = until(inner, mark.chars().next().unwrap_or('*'), mark, false)
            {
                let mut spans = Vec::new();
                inline_into(body, &Span { strong: true, ..base.clone() }, &mut spans);
                made = Some((spans, 2 + end));
            }
        }
        for mark in ['*', '_'] {
            // not inside a word, and not a second mark of the same kind
            let free = before.is_none_or(|c| !word(c) && (mark == '_' || c != '*'));
            if made.is_none()
                && free
                && first == mark
                && let Some((body, end)) = until(&rest[1..], mark, &mark.to_string(), true)
                && !rest[1 + end..].chars().next().is_some_and(word)
            {
                made = Some((vec![Span { em: true, ..with(body) }], 1 + end));
            }
        }
        if made.is_none() && (rest.starts_with("https://") || rest.starts_with("http://")) {
            let scheme = rest.find("://").map_or(0, |at| at + 3);
            let end = rest[scheme..]
                .find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | ')'))
                .map_or(rest.len(), |end| scheme + end);
            if end > scheme {
                // the end of a sentence isn't part of the link, as on GitHub
                let url = rest[..end].trim_end_matches(['.', ',', ';', ':', '!', '?']);
                let mut spans = vec![Span { link: Some(url.to_string()), ..with(url) }];
                if url.len() < end {
                    spans.push(with(&rest[url.len()..end]));
                }
                made = Some((spans, end));
            }
        }
        match made {
            Some((spans, len)) => {
                if !plain.is_empty() {
                    out.push(with(&std::mem::take(&mut plain)));
                }
                out.extend(spans.into_iter().filter(|span| !span.text.is_empty()));
                before = text[..at + len].chars().next_back();
                at += len;
            }
            None => {
                plain.push(first);
                before = Some(first);
                at += first.len_utf8();
            }
        }
    }
    if !plain.is_empty() {
        out.push(with(&plain));
    }
}

/// A line's words with their marks. Words that follow each other with the same marks are one run.
pub fn inline(text: &str) -> Vec<Span> {
    let mut read = Vec::new();
    inline_into(text, &Span::default(), &mut read);
    let marks = |span: &Span| Span { text: String::new(), ..span.clone() };
    let mut out: Vec<Span> = Vec::new();
    for span in read {
        match out.last_mut() {
            Some(last) if last.chip.is_none() && span.chip.is_none() && marks(last) == marks(&span) => {
                last.text.push_str(&span.text);
            }
            _ => out.push(span),
        }
    }
    out
}

/// A block of a Markdown text before its words are read.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Raw {
    Heading { level: usize, text: String },
    Code(String),
    List { ordered: bool, start: u64, items: Vec<(String, Option<bool>)> },
    Quote(String),
    Table(Vec<Vec<String>>),
    Rule,
    Paragraph(String),
}

static COMMENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<!--.*?-->").expect("a pattern"));
static BREAK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<br\s*/?>").expect("a pattern"));
static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)</?[a-z][^>]*>").expect("a pattern"));
static NEWLINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\r\n?").expect("a pattern"));
static HEADING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(#{1,6})\s+(.*?)\s*#*\s*$").expect("a pattern"));
static ITEM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(?:[-*+]|(\d+)[.)])\s+(?:\[([ xX])\]\s+)?(.*)$").expect("a pattern"));
static QUOTE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*>\s?(.*)$").expect("a pattern"));
static ROW: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*\|.*\|\s*$").expect("a pattern"));
static SHAPE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*\|[\s:|-]+\|\s*$").expect("a pattern"));
static CONTINUES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s{2,}\S").expect("a pattern"));

/// A line of three or more of one of `-`, `*` or `_`, with nothing else but spaces.
fn rule(line: &str) -> bool {
    let marks: Vec<char> = line.chars().filter(|c| !c.is_whitespace()).collect();
    marks.len() >= 3 && matches!(marks[0], '-' | '*' | '_') && marks.iter().all(|c| *c == marks[0])
}

/// The blocks of a Markdown text, line by line.
fn raw_blocks(markdown: &str) -> Vec<Raw> {
    // HTML comments go whole (PR templates), other tags leave their text
    let text = COMMENT.replace_all(markdown, "");
    let text = BREAK.replace_all(&text, "\n");
    let text = TAG.replace_all(&text, "");
    let text = NEWLINE.replace_all(&text, "\n");
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out: Vec<Raw> = Vec::new();
    let mut para: Vec<&str> = Vec::new();
    fn flush(para: &mut Vec<&str>, out: &mut Vec<Raw>) {
        if !para.is_empty() {
            out.push(Raw::Paragraph(para.join(" ")));
        }
        para.clear();
    }
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        i += 1;
        let opened = line.trim_start();
        if let Some(fence) = ["```", "~~~"].into_iter().find(|fence| opened.starts_with(fence)) {
            flush(&mut para, &mut out);
            let mut code: Vec<&str> = Vec::new();
            while i < lines.len() && !lines[i].trim_start().starts_with(fence) {
                code.push(lines[i]);
                i += 1;
            }
            // past the closing fence
            i += 1;
            out.push(Raw::Code(code.join("\n")));
            continue;
        }
        if let Some(heading) = HEADING.captures(line) {
            flush(&mut para, &mut out);
            out.push(Raw::Heading { level: heading[1].len(), text: heading[2].to_string() });
            continue;
        }
        if rule(line) {
            flush(&mut para, &mut out);
            out.push(Raw::Rule);
            continue;
        }
        if let Some(item) = ITEM.captures(line) {
            flush(&mut para, &mut out);
            let number = item.get(1).map(|number| number.as_str().parse::<u64>().unwrap_or(1));
            let ordered = number.is_some();
            let entry = (item[3].to_string(), item.get(2).map(|mark| mark.as_str().eq_ignore_ascii_case("x")));
            match out.last_mut() {
                Some(Raw::List { ordered: same, items, .. }) if *same == ordered => items.push(entry),
                _ => out.push(Raw::List { ordered, start: number.unwrap_or(1), items: vec![entry] }),
            }
            continue;
        }
        if let Some(quote) = QUOTE.captures(line) {
            flush(&mut para, &mut out);
            match out.last_mut() {
                Some(Raw::Quote(text)) => {
                    text.push('\n');
                    text.push_str(&quote[1]);
                }
                _ => out.push(Raw::Quote(quote[1].to_string())),
            }
            continue;
        }
        if ROW.is_match(line) {
            flush(&mut para, &mut out);
            // the line under the header (|---|---|) only shapes the table
            if SHAPE.is_match(line) {
                continue;
            }
            let inside = line.trim();
            let cells: Vec<String> =
                inside[1..inside.len() - 1].split('|').map(|cell| cell.trim().to_string()).collect();
            match out.last_mut() {
                Some(Raw::Table(rows)) => rows.push(cells),
                _ => out.push(Raw::Table(vec![cells])),
            }
            continue;
        }
        if line.trim().is_empty() {
            flush(&mut para, &mut out);
            continue;
        }
        // a line that continues a list item
        if para.is_empty()
            && CONTINUES.is_match(line)
            && let Some(Raw::List { items, .. }) = out.last_mut()
            && let Some((text, _)) = items.last_mut()
        {
            text.push(' ');
            text.push_str(line.trim());
            continue;
        }
        para.push(line.trim());
    }
    flush(&mut para, &mut out);
    out
}

fn paragraph(text: &str) -> Vec<Block> {
    let spans = inline(text);
    if spans.is_empty() { Vec::new() } else { vec![Block::Paragraph(spans)] }
}

/// A Markdown text as the card's blocks.
pub fn blocks_of(markdown: &str) -> Vec<Block> {
    (raw_blocks(markdown).into_iter())
        .map(|raw| match raw {
            Raw::Heading { level, text } => Block::Heading { big: level <= 2, spans: inline(&text) },
            Raw::Code(text) => Block::Code(text),
            // a list whose every item has a box is a checklist
            Raw::List { items, .. } if items.iter().all(|(_, task)| task.is_some()) => Block::Tasks(
                (items.iter())
                    .map(|(text, task)| Block::Task { done: *task == Some(true), spans: inline(text) })
                    .collect(),
            ),
            Raw::List { ordered, start, items } => Block::List {
                start: ordered.then_some(start),
                items: items.iter().map(|(text, _)| paragraph(text)).collect(),
            },
            Raw::Quote(text) => Block::Quote(paragraph(&text)),
            Raw::Table(rows) => Block::Table(
                (rows.iter().enumerate())
                    .map(|(r, row)| row.iter().map(|cell| Cell { header: r == 0, blocks: paragraph(cell) }).collect())
                    .collect(),
            ),
            Raw::Rule => Block::Rule,
            Raw::Paragraph(text) => Block::Paragraph(inline(&text)),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::work::rich::{links, plain};

    fn said(words: &str) -> Vec<Block> {
        vec![Block::Paragraph(vec![Span::plain(words)])]
    }

    /// Draws GitHub's Markdown: headings, lists, task lists, code, quotes, tables and links.
    #[test]
    fn reads_githubs_markdown_headings_lists_task_lists_code_quotes_tables_and_links() {
        let text = [
            "## What changes",
            "- An **Export to CSV** action, `orders/export.py`",
            "- Cancelled orders are _left out_",
            "",
            "1. Filter the list",
            "2. Export",
            "",
            "- [x] Columns as asked",
            "- [ ] Fast enough",
            "",
            "> A quote",
            "",
            "```python",
            "print(1)",
            "```",
            "",
            "| Column | Kind |",
            "|---|---|",
            "| total | number |",
            "",
            "See [the spec](https://example.com/spec) and https://example.com/bare.",
            "![a screenshot](https://example.com/shot.png)",
        ]
        .join("\n");
        let blocks = blocks_of(&text);
        assert_eq!(blocks[0], Block::Heading { big: true, spans: vec![Span::plain("What changes")] });
        let Block::List { start: None, items } = &blocks[1] else { panic!("{:?}", blocks[1]) };
        let Block::Paragraph(first) = &items[0][0] else { panic!("{:?}", items[0]) };
        assert_eq!((first[1].text.as_str(), first[1].strong), ("Export to CSV", true));
        assert_eq!((first[3].text.as_str(), first[3].code), ("orders/export.py", true));
        let Block::Paragraph(second) = &items[1][0] else { panic!("{:?}", items[1]) };
        assert_eq!((second[1].text.as_str(), second[1].em), ("left out", true));
        assert_eq!(blocks[2], Block::List { start: Some(1), items: vec![said("Filter the list"), said("Export")] });
        let task = |done: bool, words: &str| Block::Task { done, spans: vec![Span::plain(words)] };
        assert_eq!(blocks[3], Block::Tasks(vec![task(true, "Columns as asked"), task(false, "Fast enough")]));
        assert_eq!(blocks[4], Block::Quote(said("A quote")));
        assert_eq!(blocks[5], Block::Code("print(1)".into()));
        let Block::Table(rows) = &blocks[6] else { panic!("{:?}", blocks[6]) };
        let cells: Vec<Vec<(bool, String)>> =
            rows.iter().map(|row| row.iter().map(|cell| (cell.header, plain(&cell.blocks))).collect()).collect();
        assert_eq!(
            cells,
            [
                vec![(true, "Column".to_string()), (true, "Kind".to_string())],
                vec![(false, "total".to_string()), (false, "number".to_string())]
            ]
        );
        // lines with no blank line between them are one paragraph; the image is named, never fetched
        let Block::Paragraph(last) = &blocks[7] else { panic!("{:?}", blocks[7]) };
        let texts: Vec<&str> = last.iter().map(|span| span.text.as_str()).collect();
        assert_eq!(texts, ["See ", "the spec", " and ", "https://example.com/bare", ". ", "a screenshot, on GitHub"]);
        assert_eq!(last[5].chip, Some(Chip::File));
        assert_eq!(last[5].link, None);
        assert_eq!(links(&blocks), ["https://example.com/spec", "https://example.com/bare"]);
        assert_eq!(blocks.len(), 8);
    }

    /// Drops the template's HTML comments, shows other tags' text only, and never runs anything.
    #[test]
    fn drops_the_templates_html_comments_shows_other_tags_text_only_and_never_runs_anything() {
        let text =
            "Hello<!-- a\nreminder -->\n<details><summary>More</summary>inside</details>\n<script>alert(1)</script>";
        let blocks = blocks_of(text);
        // lines with no blank line between them are one paragraph, as in any Markdown
        assert_eq!(blocks, said("Hello Moreinside alert(1)"));
        assert!(!format!("{blocks:?}").contains("reminder"));
        // not a web link: plain text; a tag is dropped, never drawn
        let blocks = blocks_of("[x](javascript:alert(1)) <img src=x onerror=alert(1)>");
        assert!(links(&blocks).is_empty());
        assert_eq!(plain(&blocks), "x)");
    }

    /// Draws every recorded description with all of its words.
    #[test]
    fn reads_every_recorded_description_with_all_of_its_words() {
        let recorded = include_str!("../../../enkidu-core/tests/fixtures/work/gh-open-prs.json");
        let answer: serde_json::Value = serde_json::from_str(recorded).expect("the recorded answer");
        let bodies: Vec<&str> = (answer["data"]["search"]["nodes"].as_array().expect("its PRs").iter())
            .map(|node| node["body"].as_str().expect("a body"))
            .collect();
        assert_eq!(bodies.len(), 6);
        let word = Regex::new(r"Text \d+ \(\d+ chars\)").expect("a pattern");
        let mut kinds = std::collections::BTreeSet::new();
        for body in bodies {
            let blocks = blocks_of(body);
            let all = plain(&blocks);
            // every fake text of the recording is there ("Text 12 (40 chars)")
            for found in word.find_iter(body) {
                assert!(all.contains(found.as_str()), "{:?} is lost from {all}", found.as_str());
            }
            kinds.extend(raw_blocks(body).iter().map(|raw| match raw {
                Raw::Heading { .. } => "heading",
                Raw::Code(_) => "code",
                Raw::List { .. } => "list",
                Raw::Quote(_) => "quote",
                Raw::Table(_) => "table",
                Raw::Rule => "rule",
                Raw::Paragraph(_) => "paragraph",
            }));
        }
        for kind in ["code", "heading", "list", "paragraph"] {
            assert!(kinds.contains(kind), "{kind} in {kinds:?}");
        }
    }

    /// The marks of a line, where the order of the marks decides.
    #[test]
    fn a_lines_marks_win_in_their_order_and_never_inside_a_word() {
        let texts = |text: &str| -> Vec<(String, &'static str)> {
            (inline(text).into_iter())
                .map(|span| {
                    let how = match (&span.link, span.code, span.strong, span.em) {
                        (Some(_), ..) => "link",
                        (_, true, ..) => "code",
                        (_, _, true, _) => "strong",
                        (_, _, _, true) => "em",
                        _ => "",
                    };
                    (span.text, how)
                })
                .collect()
        };
        let s = |text: &str, how: &'static str| (text.to_string(), how);
        // code wins over what is inside it, with as many backticks as it opened with
        assert_eq!(texts("`a*b*` and ``c`d``"), [s("a*b*", "code"), s(" and `", ""), s("c", "code"), s("d``", "")]);
        assert_eq!(texts("``x``"), [s("x", "code")]);
        // bold with either mark, italics with either mark, never inside a word
        assert_eq!(
            texts("**b** __b__ *i* _i_"),
            [s("b", "strong"), s(" ", ""), s("b", "strong"), s(" ", ""), s("i", "em"), s(" ", ""), s("i", "em")]
        );
        assert_eq!(texts("snake_case_name and 2*3*4"), [s("snake_case_name and 2*3*4", "")]);
        // a link's words keep their marks, and bold around a link stays bold
        let spans = inline("**see [the `spec`](https://example.com/a b)**");
        assert_eq!(spans.len(), 3);
        assert!(spans.iter().all(|span| span.strong));
        assert_eq!(spans[1].link.as_deref(), Some("https://example.com/a"));
        assert!(spans[2].code && spans[2].link.is_some());
        // a bare link ends before the sentence's own end, and before a closing bracket
        assert_eq!(
            texts("(https://example.com/x), then http://a.b/c?d=1!"),
            [
                s("(", ""),
                s("https://example.com/x", "link"),
                s("), then ", ""),
                s("http://a.b/c?d=1", "link"),
                s("!", "")
            ]
        );
        // an image without a name, and a link that isn't to the web
        assert_eq!(inline("![](https://example.com/a.png)")[0].text, "An image, on GitHub");
        assert_eq!(texts("[mail](mailto:a@b.c)"), [s("mail", "")]);
        assert_eq!(texts("a * b * c"), [s("a ", ""), s(" b ", "em"), s(" c", "")]);
    }

    /// The shapes of blocks the reader knows.
    #[test]
    fn reads_fences_rules_numbered_lists_quotes_over_lines_and_lines_that_continue_an_item() {
        let text = "Title\n===\n\n~~~\na\n  b\n~~~\n\n3) three\n4) four\n   goes on\n\n* * *\n> one\n> two\n\n# Big #\n#### Small\n- a\n1. b\n\n|a|b|\n|:-|-:|\n|1|2|\nlast";
        let blocks = blocks_of(text);
        let kinds: Vec<String> = (blocks.iter())
            .map(|block| match block {
                Block::Paragraph(_) => format!("p:{}", plain(std::slice::from_ref(block))),
                Block::Heading { big, .. } => {
                    format!("h{}:{}", if *big { "1" } else { "3" }, plain(std::slice::from_ref(block)))
                }
                Block::Code(text) => format!("code:{text}"),
                Block::List { start, items } => format!("list:{start:?}:{}", items.len()),
                Block::Quote(_) => format!("quote:{}", plain(std::slice::from_ref(block))),
                Block::Table(rows) => format!("table:{}", rows.len()),
                Block::Rule => "rule".to_string(),
                other => format!("{other:?}"),
            })
            .collect();
        assert_eq!(
            kinds,
            [
                "p:Title ===",
                "code:a\n  b",
                "list:Some(3):2",
                "rule",
                "quote:one\ntwo",
                "h1:Big",
                "h3:Small",
                "list:None:1",
                "list:Some(1):1",
                "table:2",
                "p:last"
            ]
        );
        let Block::List { items, .. } = &blocks[2] else { panic!() };
        assert_eq!(plain(&items[1]), "four goes on");
        // a code block that never closes takes the rest
        assert_eq!(blocks_of("```\nopen\nstill"), [Block::Code("open\nstill".into())]);
        // Windows line ends and <br> are line breaks
        assert_eq!(blocks_of("a\r\nb<br/>c\rd"), said("a b c d"));
        assert!(blocks_of("  \n\n").is_empty());
    }
}
