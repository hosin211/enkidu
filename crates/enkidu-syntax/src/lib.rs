//! Syntax colors for diffs and opened files (items 3.2e and 3.3).
//!
//! Two schemes ([`Scheme`]). The one in use unless Settings says otherwise is an IDE's (item 8.3): the colors of
//! PyCharm's dark scheme, with rules written for these grammars (`ide.rs`). The other is the quiet one: the Vesper
//! theme, mostly white text with soft peach and mint accents (`vesper.rs`, the theme's own rules). The grammars are
//! syntect's, with the larger set two-face carries (TypeScript, Vue, TOML and SCSS are in it). Vesper was written for
//! Shiki; Shiki and syntect both read TextMate scopes, so its rules apply as they are; the grammars are not the very
//! same ones, so a token here and there is scoped, and so colored, differently.
//!
//! Coloring takes a few milliseconds per hundred lines: call [`colors`] off the drawing thread. What it gave for a
//! text is kept, so the unchanged side of a file that changes again is not colored again.

mod ide;
mod vesper;

use std::collections::HashMap;
use std::hash::{Hash as _, Hasher as _};
use std::ops::Range;
use std::str::FromStr as _;
use std::sync::{Arc, Mutex, OnceLock};

use syntect::easy::HighlightLines;
use syntect::highlighting::{Color, ScopeSelectors, StyleModifier, Theme, ThemeItem, ThemeSettings};
use syntect::parsing::{SyntaxReference, SyntaxSet};
use syntect::util::LinesWithEndings;

/// A color as the theme gives it: red, green, blue and how solid it is, each 0 to 255.
pub type Rgba = (u8, u8, u8, u8);

/// A stretch of a text, in bytes, and its color.
pub type Span = (Range<usize>, Rgba);

/// Texts longer than this are shown without colors: coloring them costs more than it helps.
pub const MAX_LINES: usize = 6000;

/// How many texts' colors are kept.
const KEPT: usize = 512;

fn grammars() -> &'static SyntaxSet {
    static GRAMMARS: OnceLock<SyntaxSet> = OnceLock::new();
    GRAMMARS.get_or_init(two_face::syntax::extra_newlines)
}

/// `#RGB`, `#RGBA`, `#RRGGBB` or `#RRGGBBAA`, as the theme writes its colors.
fn color(hex: &str) -> Option<Color> {
    let digits = hex.strip_prefix('#')?;
    let value = |at: usize, len: usize| {
        let part = digits.get(at..at + len)?;
        let byte = u8::from_str_radix(part, 16).ok()?;
        // one digit stands for two of the same
        Some(if len == 1 { byte * 17 } else { byte })
    };
    let (r, g, b, a) = match digits.len() {
        3 => (value(0, 1)?, value(1, 1)?, value(2, 1)?, 255),
        4 => (value(0, 1)?, value(1, 1)?, value(2, 1)?, value(3, 1)?),
        6 => (value(0, 2)?, value(2, 2)?, value(4, 2)?, 255),
        8 => (value(0, 2)?, value(2, 2)?, value(4, 2)?, value(6, 2)?),
        _ => return None,
    };
    Some(Color { r, g, b, a })
}

/// Rules for where syntect's grammars name a thing differently from the grammars Vesper was written for (VS
/// Code's, which Shiki used): each gives the color the theme gives that thing there. The IDE scheme's rules are
/// written for syntect's grammars as they are, so it needs none.
const ADAPTED: &[(&str, &str)] = &[
    // a JSON key: the theme's `support.type.property-name.json` rules
    ("source.json meta.mapping.key string", "#FFC799"),
    // inline code in Markdown: the theme's `markup.inline.raw.markdown`
    ("text.html.markdown markup.raw.inline", "#A0A0A0"),
];

/// Which colors code is drawn in (item 8.3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Scheme {
    /// An IDE's colors, PyCharm's dark scheme: keywords, strings, numbers, comments, definitions, decorators and
    /// built-in names each in their own color (`ide.rs`).
    #[default]
    Ide,
    /// Vesper: mostly white, with soft peach and mint (`vesper.rs`).
    Quiet,
}

/// A scheme's theme, from its rules. A selector syntect can't read (Vesper has three glob-like ones, for underlines) is
/// left out.
fn theme(scheme: Scheme) -> &'static Theme {
    fn build(name: &str, author: &str, foreground: &str, adapted: &[(&str, &str)], rules: &[(&[&str], &str)]) -> Theme {
        // Two rules for the same scope: in a theme's own home (VS Code, and Shiki after it) the later one wins, in
        // syntect the earlier one. So the rules go in back to front.
        let scopes = adapted
            .iter()
            .copied()
            .chain(
                rules.iter().rev().flat_map(|(selectors, hex)| selectors.iter().map(move |selector| (*selector, *hex))),
            )
            .filter_map(|(selector, hex)| {
                let scope = ScopeSelectors::from_str(selector).ok()?;
                let style = StyleModifier { foreground: Some(color(hex)?), background: None, font_style: None };
                Some(ThemeItem { scope, style })
            })
            .collect();
        Theme {
            name: Some(name.into()),
            author: Some(author.into()),
            settings: ThemeSettings { foreground: color(foreground), ..Default::default() },
            scopes,
        }
    }
    static IDE: OnceLock<Theme> = OnceLock::new();
    static QUIET: OnceLock<Theme> = OnceLock::new();
    match scheme {
        Scheme::Ide => IDE.get_or_init(|| build("Islands Dark", "JetBrains", ide::FOREGROUND, &[], ide::RULES)),
        Scheme::Quiet => {
            QUIET.get_or_init(|| build("Vesper", "Rauno Freiberg", vesper::FOREGROUND, ADAPTED, vesper::RULES))
        }
    }
}

/// The grammar for one of the names a file's language goes by (`enkidu-diff` gives them from the file's path). None for
/// `text` and for a name without a grammar: such a file is shown without colors.
fn grammar(language: &str) -> Option<&'static SyntaxReference> {
    let set = grammars();
    // what the grammar set knows each by: a file extension, or the grammar's own name
    let (extension, name) = match language {
        "python" => ("py", None),
        "typescript" => ("ts", None),
        "tsx" => ("tsx", None),
        "javascript" => ("js", None),
        // the set has no grammar for JavaScript with JSX: the one for TypeScript with it reads it too
        "jsx" => ("tsx", None),
        "vue" => ("vue", None),
        "html" => ("html", None),
        "jinja" => ("jinja2", None),
        "css" => ("css", None),
        "scss" => ("scss", None),
        "less" => ("less", None),
        "json" => ("json", None),
        "yaml" => ("yaml", None),
        "toml" => ("toml", None),
        "ini" => ("ini", None),
        "markdown" => ("md", None),
        "shellscript" => ("sh", None),
        "sql" => ("sql", None),
        "rust" => ("rs", None),
        "go" => ("go", None),
        "ruby" => ("rb", None),
        "java" => ("java", None),
        "swift" => ("swift", None),
        "php" => ("php", None),
        "xml" => ("xml", None),
        "graphql" => ("graphql", None),
        "dockerfile" => ("Dockerfile", Some("Dockerfile")),
        _ => return None,
    };
    set.find_syntax_by_extension(extension).or_else(|| name.and_then(|name| set.find_syntax_by_name(name)))
}

/// Whether files of this language are colored.
pub fn knows(language: &str) -> bool {
    grammar(language).is_some()
}

/// The colors a scheme marks changed lines with, beside their numbers: a new line, a changed one, and where lines
/// went. An IDE's scheme has them (PyCharm's green, blue and gray); the quiet scheme has none of its own, and
/// the app then uses the colors of its own marks for a new, a changed and a deleted file.
pub fn change_marks(scheme: Scheme) -> Option<[Rgba; 3]> {
    let rgba = |hex: &str| color(hex).map(|c| (c.r, c.g, c.b, c.a));
    match scheme {
        Scheme::Ide => Some([rgba(ide::ADDED_LINES)?, rgba(ide::MODIFIED_LINES)?, rgba(ide::DELETED_LINES)?]),
        Scheme::Quiet => None,
    }
}

/// Reads the grammars and the theme now, so the first diff doesn't wait for them (about 50 ms, once). Call it off
/// the drawing thread.
pub fn warm() {
    let _ = colors(Scheme::default(), "python", "x = 1\n");
}

type Kept = HashMap<(u64, usize), (u64, Arc<Vec<Span>>)>;

fn kept() -> &'static Mutex<(u64, Kept)> {
    static KEPT_COLORS: OnceLock<Mutex<(u64, Kept)>> = OnceLock::new();
    KEPT_COLORS.get_or_init(Default::default)
}

/// The colors of `text`, a whole file or a part of one, in `language`, in a scheme: byte ranges in order, each with its
/// color. Text a scheme leaves at its own plain color is in them too. Empty for a language without a grammar and for a
/// text over [`MAX_LINES`].
pub fn colors(scheme: Scheme, language: &str, text: &str) -> Arc<Vec<Span>> {
    let Some(grammar) = grammar(language) else {
        return Arc::default();
    };
    if text.is_empty() || text.split('\n').nth(MAX_LINES).is_some() {
        return Arc::default();
    }
    let key = {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        scheme.hash(&mut hasher);
        language.hash(&mut hasher);
        text.hash(&mut hasher);
        (hasher.finish(), text.len())
    };
    {
        let mut kept = kept().lock().unwrap_or_else(|e| e.into_inner());
        kept.0 += 1;
        let now = kept.0;
        if let Some((used, spans)) = kept.1.get_mut(&key) {
            *used = now;
            return spans.clone();
        }
    }
    let spans = Arc::new(color_lines(scheme, grammar, text));
    let mut kept = kept().lock().unwrap_or_else(|e| e.into_inner());
    kept.0 += 1;
    let now = kept.0;
    if kept.1.len() >= KEPT {
        // the quarter used longest ago goes
        let mut used: Vec<u64> = kept.1.values().map(|(used, _)| *used).collect();
        used.sort_unstable();
        let oldest = used[used.len() / 4];
        kept.1.retain(|_, (used, _)| *used > oldest);
    }
    kept.1.insert(key, (now, spans.clone()));
    spans
}

fn color_lines(scheme: Scheme, grammar: &SyntaxReference, text: &str) -> Vec<Span> {
    let mut lines = HighlightLines::new(grammar, theme(scheme));
    // An IDE colors `self` and `cls` where a method names them as its parameters too. The grammar gives a parameter
    // no kind of its own, so those two words are picked out here (a word inside a string or a comment is never a
    // piece by itself).
    let own_name = (scheme == Scheme::Ide && grammar.name == "Python").then(|| color(ide::SELF)).flatten();
    let mut spans: Vec<Span> = Vec::new();
    let mut at = 0;
    for line in LinesWithEndings::from(text) {
        // a line the grammar can't read ends the colors there: what is colored so far stays
        let Ok(pieces) = lines.highlight_line(line, grammars()) else {
            break;
        };
        for (style, piece) in pieces {
            let range = at..at + piece.len();
            at = range.end;
            let foreground = match own_name {
                Some(own) if piece == "self" || piece == "cls" => own,
                _ => style.foreground,
            };
            let rgba = (foreground.r, foreground.g, foreground.b, foreground.a);
            match spans.last_mut() {
                // blanks take no color of their own, and neighbors of one color are one stretch
                Some((last, color)) if last.end == range.start && (*color == rgba || piece.trim().is_empty()) => {
                    last.end = range.end
                }
                _ if piece.trim().is_empty() => {}
                _ => spans.push((range, rgba)),
            }
        }
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHITE: Rgba = (0xff, 0xff, 0xff, 0xff);
    const GRAY: Rgba = (0xa0, 0xa0, 0xa0, 0xff);
    const PEACH: Rgba = (0xff, 0xc7, 0x99, 0xff);
    const MINT: Rgba = (0x99, 0xff, 0xe4, 0xff);
    const COMMENT: Rgba = (0x8b, 0x8b, 0x8b, 0x94);

    /// The color of the first place `word` stands in `text`.
    fn color_of(language: &str, text: &str, word: &str) -> Option<Rgba> {
        let at = text.find(word).unwrap_or_else(|| panic!("{word:?} is not in the text"));
        let spans = colors(Scheme::Quiet, language, text);
        spans.iter().find(|(range, _)| range.start <= at && at < range.end).map(|(_, color)| *color)
    }

    #[test]
    fn the_theme_is_vesper_and_its_colors_read_in_every_form() {
        assert_eq!(color("#FFF"), Some(Color { r: 255, g: 255, b: 255, a: 255 }));
        assert_eq!(color("#FFFF"), Some(Color { r: 255, g: 255, b: 255, a: 255 }));
        assert_eq!(color("#FFC799"), Some(Color { r: 0xff, g: 0xc7, b: 0x99, a: 255 }));
        assert_eq!(color("#8b8b8b94"), Some(Color { r: 0x8b, g: 0x8b, b: 0x8b, a: 0x94 }));
        assert_eq!((color("FFF"), color("#12"), color("#GGG")), (None, None, None));
        // every rule with a color the theme has, but the selectors syntect can't read
        let theme = theme(Scheme::Quiet);
        let selectors: usize = vesper::RULES.iter().map(|(selectors, _)| selectors.len()).sum();
        assert_eq!(vesper::RULES.len(), 57);
        assert!(theme.scopes.len() + 3 >= selectors, "{} of {selectors} selectors read", theme.scopes.len());
        assert_eq!(theme.settings.foreground, color("#FFF"));
    }

    #[test]
    fn python_is_white_with_gray_words_peach_names_and_mint_strings() {
        let text = "import os\n\n\ndef total(items, rate=0.2):\n    \"\"\"Adds them up.\"\"\"\n    name = 'all'  # every one\n    return sum(items) * rate + 42\n";
        assert_eq!(color_of("python", text, "def"), Some(GRAY));
        assert_eq!(color_of("python", text, "return"), Some(GRAY));
        assert_eq!(color_of("python", text, "import"), Some(GRAY));
        assert_eq!(color_of("python", text, "total"), Some(PEACH), "a function's name");
        assert_eq!(color_of("python", text, "42"), Some(PEACH), "a number");
        assert_eq!(color_of("python", text, "'all'"), Some(MINT), "a string");
        assert_eq!(color_of("python", text, "# every one"), Some(COMMENT));
        assert_eq!(color_of("python", text, "name"), Some(WHITE), "plain code is the theme's white");
    }

    /// Item 3.3: a few languages, each with what a reader looks for in it in the theme's colors.
    #[test]
    fn the_languages_of_his_repos_are_colored() {
        let check = |language: &str, text: &str, expected: &[(&str, Rgba)]| {
            for (word, color) in expected {
                assert_eq!(color_of(language, text, word), Some(*color), "{language}: {word:?}");
            }
        };
        check(
            "javascript",
            "import { a } from './a.js';\nconst total = (items) => items.reduce((s, i) => s + i.price, 0); // sum\nfunction f(x) { return x + 1; }\n",
            &[
                ("const", GRAY),
                ("reduce", PEACH),
                ("'./a.js'", MINT),
                ("// sum", COMMENT),
                ("function", GRAY),
                ("f(x)", PEACH),
                ("total", WHITE),
            ],
        );
        check(
            "typescript",
            "export interface Cart { items: Item[]; total: number }\nconst label: string = 'x';\n",
            &[("interface", GRAY), ("Cart", PEACH), ("number", PEACH), ("'x'", MINT), ("label", WHITE)],
        );
        check(
            "vue",
            "<template>\n  <div class=\"cart\">{{ total }}</div>\n</template>\n<script setup>\nimport { ref } from 'vue';\nconst total = ref(0);\n</script>\n<style scoped>\n.cart { color: red; }\n</style>\n",
            &[
                ("template", PEACH),
                ("div", PEACH),
                ("\"cart\"", MINT),
                ("import", GRAY),
                ("'vue'", MINT),
                ("ref(0)", PEACH),
                (".cart {", PEACH),
            ],
        );
        check(
            "jsx",
            "export function Cart({ items }) {\n  return <ul className=\"cart\">{items.length}</ul>; // list\n}\n",
            &[("function", GRAY), ("Cart", PEACH), ("ul", PEACH), ("\"cart\"", MINT), ("// list", COMMENT)],
        );
        check(
            "html",
            "<div class=\"cart\" id=\"main\"><a href=\"/x\">Link</a><!-- note --></div>\n",
            &[
                ("div", PEACH),
                ("class", PEACH),
                ("\"cart\"", MINT),
                ("href", PEACH),
                ("Link", WHITE),
                ("<!-- note -->", COMMENT),
            ],
        );
        check(
            "css",
            ".cart > a:hover { color: #ff0000; margin: 0 4px; }\n@media (min-width: 600px) { #main { display: none; } }\n",
            &[
                (".cart", PEACH),
                ("color", WHITE),
                ("#ff0000", GRAY),
                ("4px", PEACH),
                ("@media", GRAY),
                ("#main", PEACH),
            ],
        );
        check(
            "scss",
            "$gap: 4px;\n.cart { .row { margin: $gap; } }\n@mixin wide { width: 100%; }\n",
            &[("4px", PEACH), (".cart", PEACH), (".row", PEACH), ("@mixin", GRAY), ("100%", PEACH)],
        );
        check(
            "json",
            "{\n  \"name\": \"shop\",\n  \"count\": 3\n}\n",
            &[("\"name\"", PEACH), ("\"shop\"", MINT), ("\"count\"", PEACH), ("3", PEACH)],
        );
        check(
            "yaml",
            "name: shop\nservices:\n  web:\n    ports: [80, 443]  # both\n",
            &[("name", PEACH), ("shop", MINT), ("services", PEACH), ("443", PEACH), ("# both", COMMENT)],
        );
        check(
            "toml",
            "[package]\nname = \"shop\"\ncount = 3 # three\n",
            &[("package", PEACH), ("name", PEACH), ("\"shop\"", MINT), ("3", PEACH), ("# three", COMMENT)],
        );
        check(
            "markdown",
            "# Title\n\nSome text with `code` and a [link](https://x.y).\n",
            &[("# Title", PEACH), ("Some text", WHITE), ("code", GRAY), ("https://x.y", PEACH)],
        );
        check(
            "shellscript",
            "#!/bin/sh\nfor f in *.txt; do\n  echo \"file\" | grep -c 'x' # count\ndone\n",
            &[("for", GRAY), ("echo", PEACH), ("'x'", MINT), ("# count", COMMENT), ("done", GRAY)],
        );
        check(
            "sql",
            "SELECT id, name FROM orders WHERE total > 10 AND status = 'open'; -- all\n",
            &[
                ("SELECT", PEACH),
                ("FROM", PEACH),
                ("10", PEACH),
                ("'open'", MINT),
                ("-- all", COMMENT),
                ("orders", WHITE),
            ],
        );
    }

    #[test]
    fn every_language_a_path_can_name_has_a_grammar() {
        // the names `enkidu-diff` gives from a file's path
        let names = [
            "python",
            "typescript",
            "tsx",
            "javascript",
            "jsx",
            "vue",
            "html",
            "jinja",
            "css",
            "scss",
            "less",
            "json",
            "yaml",
            "toml",
            "ini",
            "markdown",
            "shellscript",
            "sql",
            "rust",
            "go",
            "ruby",
            "java",
            "swift",
            "php",
            "xml",
            "graphql",
            "dockerfile",
        ];
        let unknown: Vec<&str> = names.into_iter().filter(|name| !knows(name)).collect();
        assert_eq!(unknown, Vec::<&str>::new(), "no grammar");
    }

    #[test]
    fn spans_are_in_order_inside_the_text_and_on_whole_characters() {
        let text = "caf\u{e9} = \"\u{645}\u{631}\u{62d}\u{628}\u{627}\"  # \u{4e2d}\u{6587}\nprint(caf\u{e9})\n";
        let spans = colors(Scheme::Ide, "python", text);
        assert!(!spans.is_empty());
        let mut end = 0;
        for (range, _) in spans.iter() {
            assert!(range.start >= end && range.start < range.end && range.end <= text.len(), "{range:?}");
            assert!(text.is_char_boundary(range.start) && text.is_char_boundary(range.end), "{range:?}");
            end = range.end;
        }
    }

    #[test]
    fn a_file_without_a_grammar_or_too_long_has_no_colors() {
        let colors = |language: &str, text: &str| colors(Scheme::Ide, language, text);
        assert!(colors("text", "plain words\n").is_empty());
        assert!(colors("klingon", "nuqneH\n").is_empty());
        assert!(colors("python", "").is_empty());
        assert!(!knows("text") && knows("python"));
        let long = "x = 1\n".repeat(MAX_LINES + 1);
        assert!(colors("python", &long).is_empty());
        let at_the_limit = "x = 1\n".repeat(MAX_LINES - 1);
        assert!(!colors("python", &at_the_limit).is_empty());
    }

    #[test]
    fn a_text_colored_once_is_not_colored_again() {
        let text = "def kept():\n    return 'the very same'\n";
        let color = |language| colors(Scheme::Ide, language, text);
        let (first, second) = (color("python"), color("python"));
        assert!(Arc::ptr_eq(&first, &second));
        // another language's reading of the same text is its own
        let other = color("javascript");
        assert!(!Arc::ptr_eq(&first, &other));
    }
}

#[cfg(test)]
mod explore {
    use super::*;

    /// Prints each colored stretch of a snippet with a letter for its color: for looking, not a test of anything.
    #[test]
    #[ignore = "prints what the grammars give: cargo test -p enkidu-syntax explore -- --ignored --nocapture"]
    fn what_each_language_gives() {
        let samples: &[(&str, &str)] = &[
            (
                "javascript",
                "import { a } from './a.js';\nconst total = (items) => items.reduce((s, i) => s + i.price, 0); // sum\nfunction f(x) { return `n=${x}` + 1; }\nclass Cart extends Base { add(item) { this.items.push(item); } }\n",
            ),
            (
                "typescript",
                "import type { Item } from './types';\nexport interface Cart { items: Item[]; total: number }\nexport function add(cart: Cart, item: Item): Cart { return { ...cart, total: cart.total + 1 }; }\nconst label: string = 'x';\n",
            ),
            (
                "vue",
                "<template>\n  <div class=\"cart\" :title=\"name\" @click=\"add(item)\">{{ total }}</div>\n</template>\n<script setup>\nimport { ref } from 'vue';\nconst total = ref(0);\n</script>\n<style scoped>\n.cart { color: red; }\n</style>\n",
            ),
            ("html", "<!doctype html>\n<div class=\"cart\" id=\"main\"><a href=\"/x\">Link</a><!-- note --></div>\n"),
            (
                "css",
                ".cart > a:hover { color: #ff0000; margin: 0 4px; }\n@media (min-width: 600px) { #main { display: none; } }\n",
            ),
            (
                "scss",
                "$gap: 4px;\n.cart { &:hover { color: darken($ink, 10%); } .row { margin: $gap; } }\n@mixin wide { width: 100%; }\n",
            ),
            (
                "json",
                "{\n  \"name\": \"shop\",\n  \"count\": 3,\n  \"ok\": true,\n  \"nested\": { \"key\": null }\n}\n",
            ),
            (
                "yaml",
                "name: shop\nservices:\n  web:\n    image: \"nginx:1.25\"\n    ports: [80, 443]\n    enabled: true  # yes\n",
            ),
            ("toml", "[package]\nname = \"shop\"\nversion = \"1.0\"\ncount = 3\nenabled = true # yes\n"),
            (
                "markdown",
                "# Title\n\nSome *italic* and **bold** text with `code` and a [link](https://x.y).\n\n- item one\n\n```py\nx = 1\n```\n",
            ),
            (
                "shellscript",
                "#!/bin/sh\nset -eu\nfor f in *.txt; do\n  echo \"file: $f\" | grep -c 'x' # count\ndone\nexport NAME=value\n",
            ),
            (
                "sql",
                "SELECT id, name FROM orders WHERE total > 10 AND status = 'open' ORDER BY id; -- all\nUPDATE orders SET total = 0;\n",
            ),
        ];
        for (language, text) in samples {
            println!("== {language} (known: {})", knows(language));
            for (range, color) in colors(Scheme::Quiet, language, text).iter() {
                let letter = match color {
                    (0xff, 0xff, 0xff, _) => "white",
                    (0xa0, 0xa0, 0xa0, _) => "gray ",
                    (0xff, 0xc7, 0x99, _) => "peach",
                    (0x99, 0xff, 0xe4, _) => "mint ",
                    (0x8b, 0x8b, 0x8b, _) => "cmnt ",
                    (0xff, 0x80, 0x80, _) => "red  ",
                    _ => "other",
                };
                println!("   {letter} {:?}", &text[range.clone()]);
            }
        }
    }
}

#[cfg(test)]
mod scopes {
    use super::*;
    use syntect::parsing::{ParseState, ScopeStack};

    /// Prints the scopes of the first place `word` stands in a snippet: for writing a rule for a grammar whose
    /// scopes differ from the ones the theme was written for.
    #[test]
    #[ignore = "prints scopes: cargo test -p enkidu-syntax scopes -- --ignored --nocapture"]
    fn of_a_token() {
        let cases: &[(&str, &str, &str)] = &[
            ("json", "{\n  \"name\": \"shop\",\n  \"ok\": true\n}\n", "\"name\""),
            ("json", "{\n  \"name\": \"shop\",\n  \"ok\": true\n}\n", "true"),
            ("markdown", "Some text with `code` here.\n", "code"),
            ("toml", "enabled = true\n", "true"),
            ("vue", "<template>\n  <div :title=\"name\" @click=\"add\">x</div>\n</template>\n", "title"),
            ("python", "x = True\n", "True"),
            ("javascript", "const ok = true;\n", "true"),
            ("python", "class A:\n    def f(self, a):\n        pass\n", "self"),
            ("python", "def f():\n    \"\"\"Doc.\"\"\"\n", "\"\"\""),
            ("python", "name = f\"all {x}\"\n", "f\""),
            ("javascript", "typeof o;\n", "typeof"),
            ("javascript", "const re = /ab+c/g;\n", "+c"),
            ("javascript", "const re = /ab+c/g;\n", "g;"),
            ("typescript", "const label: string = 1;\n", ": string"),
            ("html", "<!doctype html>\n", "doctype"),
            ("vue", "<template>\n  <CartRow :key=\"a\" />\n</template>\n", "CartRow"),
            ("ini", "[server]\nhost = 1\n", "server"),
            ("dockerfile", "CMD [\"a\"]\n", "CMD"),
            ("java", "class A { void f() { this.t = 1; } }\n", "this"),
            ("sql", "SELECT a FROM b WHERE c > 1 AND d = 2;\n", "AND"),
            ("go", "func T(items []Item) int { return 1 }\n", "Item"),
        ];
        for (language, text, word) in cases {
            let grammar = grammar(language).unwrap();
            let mut state = ParseState::new(grammar);
            let mut stack = ScopeStack::new();
            let at = text.find(word).unwrap();
            let mut offset = 0;
            'lines: for line in LinesWithEndings::from(text) {
                let ops = state.parse_line(line, grammars()).unwrap();
                let mut ops = ops.iter().peekable();
                for (ix, _) in line.char_indices() {
                    while let Some((op_at, op)) = ops.peek() {
                        if *op_at > ix {
                            break;
                        }
                        stack.apply(op).unwrap();
                        ops.next();
                    }
                    if offset + ix == at {
                        println!(
                            "{language} {word:?}: {}",
                            stack.as_slice().iter().map(|s| s.to_string()).collect::<Vec<_>>().join("  ")
                        );
                        break 'lines;
                    }
                }
                for (_, op) in ops {
                    stack.apply(op).unwrap();
                }
                offset += line.len();
            }
        }
    }
}

#[cfg(test)]
mod list {
    use super::*;

    #[test]
    #[ignore = "prints the grammar set: cargo test -p enkidu-syntax list -- --ignored --nocapture"]
    fn the_grammars() {
        for syntax in grammars().syntaxes() {
            println!("grammar {:?}: {}", syntax.name, syntax.file_extensions.join(" "));
        }
    }
}

#[cfg(test)]
mod ide_look {
    use super::*;

    /// A letter or two for each color of the IDE scheme; plain code has none.
    fn name(color: Rgba) -> &'static str {
        match (color.0, color.1, color.2) {
            (0xBC, 0xBE, 0xC4) => "",
            (0xCF, 0x8E, 0x6D) => "K",
            (0x6A, 0xAB, 0x73) => "s",
            (0x2A, 0xAC, 0xB8) => "n",
            (0x7A, 0x7E, 0x85) => "c",
            (0x5F, 0x82, 0x6B) => "doc",
            (0x56, 0xA8, 0xF5) => "DEF",
            (0xB3, 0xAE, 0x60) => "dec",
            (0xC7, 0x7D, 0xBB) => "fld",
            (0xD5, 0xB7, 0x78) => "tag",
            (0x2F, 0xBA, 0xA3) => "cmp",
            (0xBA, 0xBA, 0xBA) => "att",
            (0x94, 0x55, 0x8D) => "self",
            (0x88, 0x88, 0xC6) => "bi",
            (0xAA, 0x49, 0x26) => "kwarg",
            (0xB2, 0x00, 0xB2) => "pre",
            (0x42, 0xC3, 0xD4) => "re",
            (0xF7, 0x54, 0x64) => "WRONG",
            _ => "?",
        }
    }

    /// Prints each line of a snippet with its colored stretches named: for looking, not a test of anything.
    #[test]
    #[ignore = "prints the IDE scheme's colors: cargo test -p enkidu-syntax ide_look -- --ignored --nocapture"]
    fn what_the_ide_scheme_gives() {
        let samples: &[(&str, &str)] = &[
            (
                "python",
                "import os\nfrom decimal import Decimal\n\n@transaction.atomic\nclass StockLevel(models.Model):\n    \"\"\"What an outlet has.\"\"\"\n\n    quantity = models.DecimalField(max_digits=12, default=0)\n\n    def __init__(self, item: Item, rate=0.2) -> None:\n        self.item = item  # the item\n        name = f\"all {item.name!r}\\n\"\n        return sum(len(x) for x in items if x is not None) * rate + 42\n\n    async def restock(self, line):\n        if not line.restocked and isinstance(line, OrderLine):\n            await line.save(update_fields=[\"restocked\"])\n        raise ValueError('no')\n",
            ),
            (
                "javascript",
                "import { a } from './a.js';\nconst total = (items) => items.reduce((s, i) => s + i.price, 0); // sum\nfunction f(x) { return `n=${x}` + 1; }\nclass Cart extends Base { add(item) { this.items.push(item); } }\nconst re = /ab+c/g; const o = { key: 1, 'q': null }; new Cart(); typeof o;\n",
            ),
            (
                "typescript",
                "import type { Item } from './types';\nexport interface Cart { items: Item[]; total: number }\nexport function add(cart: Cart, item: Item): Cart { return { ...cart, total: cart.total + 1 }; }\nconst label: string = 'x';\n",
            ),
            (
                "vue",
                "<template>\n  <div class=\"cart\" :title=\"name\" @click=\"add(item)\">{{ total }}</div>\n  <CartRow v-for=\"item in items\" :key=\"item.id\" />\n</template>\n<script setup>\nimport { ref } from 'vue';\nconst total = ref(0);\n</script>\n<style scoped>\n.cart { color: red; }\n</style>\n",
            ),
            (
                "html",
                "<!doctype html>\n<div class=\"cart\" id=\"main\"><a href=\"/x\">Link &amp; more</a><!-- note --></div>\n",
            ),
            (
                "jinja",
                "{% extends \"base.html\" %}\n{% for item in items %}<li>{{ item.name|upper }}</li>{% endfor %}\n{# note #}\n",
            ),
            (
                "css",
                ".cart > a:hover { color: #ff0000; margin: 0 4px !important; }\n@media (min-width: 600px) { #main { display: none; background: url(x.png); } }\n",
            ),
            (
                "scss",
                "$gap: 4px;\n.cart { &:hover { color: darken($ink, 10%); } .row { margin: $gap; } }\n@mixin wide { width: 100%; }\n",
            ),
            (
                "json",
                "{\n  \"name\": \"shop\",\n  \"count\": 3,\n  \"ok\": true,\n  \"nested\": { \"key\": null }\n}\n",
            ),
            (
                "yaml",
                "name: shop\nservices:\n  web:\n    image: \"nginx:1.25\"\n    ports: [80, 443]\n    enabled: true  # yes\n",
            ),
            ("toml", "[package]\nname = \"shop\"\nversion = \"1.0\"\ncount = 3\nenabled = true # yes\n"),
            ("ini", "[server]\nhost = 0.0.0.0\n; note\nport=8080\n"),
            (
                "markdown",
                "# Title\n\nSome *italic* and **bold** text with `code` and a [link](https://x.y).\n\n- item one\n> quoted\n\n```py\nx = 1\n```\n",
            ),
            (
                "shellscript",
                "#!/bin/sh\nset -eu\nfor f in *.txt; do\n  echo \"file: $f\" | grep -c 'x' # count\ndone\nexport NAME=value\nif [ -f \"$1\" ]; then exit 1; fi\n",
            ),
            (
                "sql",
                "SELECT id, name FROM orders WHERE total > 10 AND status = 'open' ORDER BY id; -- all\nUPDATE orders SET total = 0;\n",
            ),
            (
                "dockerfile",
                "FROM python:3.12-slim AS base\nRUN pip install -r requirements.txt # deps\nENV PORT=8000\nCMD [\"gunicorn\", \"shop.wsgi\"]\n",
            ),
            (
                "rust",
                "pub fn total(items: &[Item]) -> u32 { let x = 1; items.iter().map(|i| i.price).sum() } // sum\n#[derive(Debug)]\nstruct Item { price: u32 }\n",
            ),
            (
                "java",
                "@Override\npublic class Cart extends Base { private int total = 0; void add(Item item) { this.total += item.price; } }\n",
            ),
            (
                "go",
                "func Total(items []Item) int { // sum\n\tfor _, i := range items { t += i.Price }\n\treturn t\n}\n",
            ),
        ];
        for (language, text) in samples {
            println!("== {language}");
            let spans = colors(Scheme::Ide, language, text);
            let mut at = 0;
            for line in text.split_inclusive('\n') {
                let (from, to) = (at, at + line.len());
                at = to;
                let parts: Vec<String> = spans
                    .iter()
                    .filter(|(range, _)| range.start < to && range.end > from)
                    .map(|(range, color)| {
                        let piece = &text[range.start.max(from)..range.end.min(to)];
                        let piece = piece.trim_end_matches('\n');
                        if name(*color).is_empty() { piece.to_string() } else { format!("{}:{piece}", name(*color)) }
                    })
                    .collect();
                println!("   {}", parts.join(" ¦ "));
            }
        }
    }
}

/// Item 8.3: the IDE scheme, which is what you see unless Settings says Quiet.
#[cfg(test)]
mod ide_tests {
    use super::*;

    const PLAIN: Rgba = (0xBC, 0xBE, 0xC4, 0xFF);
    const KEYWORD: Rgba = (0xCF, 0x8E, 0x6D, 0xFF);
    const STRING: Rgba = (0x6A, 0xAB, 0x73, 0xFF);
    const NUMBER: Rgba = (0x2A, 0xAC, 0xB8, 0xFF);
    const COMMENT: Rgba = (0x7A, 0x7E, 0x85, 0xFF);
    const DOC: Rgba = (0x5F, 0x82, 0x6B, 0xFF);
    const DEFINITION: Rgba = (0x56, 0xA8, 0xF5, 0xFF);
    const DECORATOR: Rgba = (0xB3, 0xAE, 0x60, 0xFF);
    const FIELD: Rgba = (0xC7, 0x7D, 0xBB, 0xFF);
    const TAG: Rgba = (0xD5, 0xB7, 0x78, 0xFF);
    const COMPONENT: Rgba = (0x2F, 0xBA, 0xA3, 0xFF);
    const ATTRIBUTE: Rgba = (0xBA, 0xBA, 0xBA, 0xFF);
    const SELF: Rgba = (0x94, 0x55, 0x8D, 0xFF);
    const BUILTIN: Rgba = (0x88, 0x88, 0xC6, 0xFF);
    const KEYWORD_ARGUMENT: Rgba = (0xAA, 0x49, 0x26, 0xFF);
    const PREDEFINED: Rgba = (0xB2, 0x00, 0xB2, 0xFF);

    /// The color, in the IDE scheme, of the first place `word` stands in `text`.
    fn color_of(language: &str, text: &str, word: &str) -> Option<Rgba> {
        let at = text.find(word).unwrap_or_else(|| panic!("{word:?} is not in the text"));
        let spans = colors(Scheme::Ide, language, text);
        spans.iter().find(|(range, _)| range.start <= at && at < range.end).map(|(_, color)| *color)
    }

    fn check(language: &str, text: &str, expected: &[(&str, Rgba)]) {
        for (word, color) in expected {
            assert_eq!(color_of(language, text, word), Some(*color), "{language}: {word:?}");
        }
    }

    #[test]
    fn python_is_colored_the_way_pycharm_colors_it() {
        let text = "import os\n\n@transaction.atomic\nclass StockLevel(models.Model):\n    \"\"\"What an outlet has.\"\"\"\n\n    quantity = models.DecimalField(max_digits=12, default=0)\n\n    def __init__(self, item: Item, rate=0.2) -> None:\n        self.item = item  # the item\n        return sum(len(x) for x in items if x is not None) * rate + 42\n\n    def restock(cls, line):\n        if not line.restocked and isinstance(line, OrderLine):\n            raise ValueError('no')\n";
        check(
            "python",
            text,
            &[
                // "the word class is being highlighted and then the word method"
                ("class", KEYWORD),
                ("def __init__", KEYWORD),
                ("restock", DEFINITION),
                ("import", KEYWORD),
                ("return", KEYWORD),
                ("None", KEYWORD),
                ("not line", KEYWORD),
                ("and isinstance", KEYWORD),
                ("@transaction.atomic", DECORATOR),
                ("\"\"\"What", DOC),
                ("What an outlet", DOC),
                ("'no'", STRING),
                ("12", NUMBER),
                ("0.2", NUMBER),
                ("# the item", COMMENT),
                ("__init__", PREDEFINED),
                // as a parameter and where it is used, and `cls` like it
                ("self, item", SELF),
                ("self.item", SELF),
                ("cls", SELF),
                ("sum", BUILTIN),
                ("len", BUILTIN),
                ("isinstance", BUILTIN),
                ("ValueError", BUILTIN),
                ("max_digits", KEYWORD_ARGUMENT),
                // a class's name, a call, a plain name and a sign stay plain
                ("StockLevel", PLAIN),
                ("DecimalField", PLAIN),
                ("quantity", PLAIN),
                ("= models", PLAIN),
                ("rate=", PLAIN),
            ],
        );
    }

    #[test]
    fn the_languages_of_his_repos_are_colored_like_an_ide() {
        check(
            "javascript",
            "import { a } from './a.js';\nconst total = (items) => items.reduce((s, i) => s + i.price, 0); // sum\nfunction f(x) { return `n=${x}` + 1; }\nclass Cart extends Base { add(item) { this.items.push(item); } }\ntypeof f;\n",
            &[
                ("import", KEYWORD),
                ("const", KEYWORD),
                ("function", KEYWORD),
                ("class", KEYWORD),
                ("extends", KEYWORD),
                ("this", KEYWORD),
                ("typeof", KEYWORD),
                ("'./a.js'", STRING),
                ("`n=", STRING),
                ("${", KEYWORD),
                ("0)", NUMBER),
                ("// sum", COMMENT),
                ("f(x)", DEFINITION),
                ("reduce", DEFINITION),
                ("add(item)", DEFINITION),
                ("price", FIELD),
                ("Cart", PLAIN),
                ("=> items", PLAIN),
                ("+ i.price", PLAIN),
            ],
        );
        check(
            "typescript",
            "export interface Cart { items: Item[]; total: number }\nexport function add(cart: Cart): Cart { return cart; }\nconst label: string = 'x';\n",
            &[
                ("interface", KEYWORD),
                ("number", KEYWORD),
                ("string", KEYWORD),
                ("add", DEFINITION),
                ("'x'", STRING),
                ("Cart", PLAIN),
                (": Item", PLAIN),
            ],
        );
        check(
            "vue",
            "<template>\n  <div class=\"cart\" @click=\"add(item)\">{{ total }}</div>\n  <CartRow :key=\"item.id\" />\n</template>\n<script setup>\nimport { ref } from 'vue';\nconst total = ref(0);\n</script>\n<style scoped>\n.cart { color: red; }\n</style>\n",
            &[
                ("template", TAG),
                ("div", TAG),
                ("class=", ATTRIBUTE),
                ("\"cart\"", STRING),
                ("CartRow", COMPONENT),
                ("import", KEYWORD),
                ("'vue'", STRING),
                ("ref(0)", DEFINITION),
                (".cart", TAG),
                ("color", ATTRIBUTE),
                ("red", STRING),
            ],
        );
        check(
            "html",
            "<div class=\"cart\" id=\"main\"><a href=\"/x\">Link &amp; more</a><!-- note --></div>\n",
            &[
                ("div", TAG),
                ("class", ATTRIBUTE),
                ("\"cart\"", STRING),
                ("&amp;", DEFINITION),
                ("<!-- note -->", COMMENT),
                ("Link", PLAIN),
            ],
        );
        check(
            "css",
            ".cart > a:hover { color: #ff0000; margin: 0 4px !important; }\n@media (min-width: 600px) { #main { display: none; } }\n",
            &[
                (".cart", TAG),
                ("color", ATTRIBUTE),
                ("#ff0000", DEFINITION),
                ("4px", NUMBER),
                ("!important", KEYWORD),
                ("@media", KEYWORD),
                ("#main", TAG),
                ("none", STRING),
            ],
        );
        check(
            "scss",
            "$gap: 4px;\n.cart { .row { margin: $gap; color: darken($ink, 10%); } }\n@mixin wide { width: 100%; }\n",
            &[("$gap", FIELD), (".cart", TAG), ("margin", ATTRIBUTE), ("darken", DEFINITION), ("@mixin", KEYWORD)],
        );
        check(
            "json",
            "{\n  \"name\": \"shop\",\n  \"count\": 3,\n  \"ok\": true\n}\n",
            &[("\"name\"", FIELD), ("\"shop\"", STRING), ("3", NUMBER), ("true", KEYWORD)],
        );
        check(
            "yaml",
            "name: shop\nservices:\n  web:\n    image: \"nginx:1.25\"\n    ports: [80, 443]\n    enabled: true  # yes\n",
            &[
                ("name", KEYWORD),
                ("shop", STRING),
                ("\"nginx:1.25\"", STRING),
                ("80", NUMBER),
                ("true", KEYWORD),
                ("# yes", COMMENT),
            ],
        );
        check(
            "toml",
            "[package]\nname = \"shop\"\ncount = 3\n",
            &[("package", TAG), ("name", KEYWORD), ("\"shop\"", STRING), ("3", NUMBER)],
        );
        check(
            "markdown",
            "# Title\n\nSome text with `code` and a [link](https://x.y).\n",
            &[("# Title", KEYWORD), ("[link]", DEFINITION), ("code", PLAIN), ("Some text", PLAIN)],
        );
        check(
            "shellscript",
            "#!/bin/sh\nfor f in *.txt; do\n  echo \"file: $f\" # count\ndone\n",
            &[("for", KEYWORD), ("done", KEYWORD), ("\"file: ", STRING), ("$f", FIELD), ("# count", COMMENT)],
        );
        check(
            "sql",
            "SELECT id FROM orders WHERE total > 10 AND status = 'open'; -- all\n",
            &[("SELECT", KEYWORD), ("AND", KEYWORD), ("10", NUMBER), ("'open'", STRING), ("-- all", COMMENT)],
        );
    }

    #[test]
    fn every_rule_of_the_ide_scheme_is_read_and_its_plain_color_is_the_schemes() {
        let theme = theme(Scheme::Ide);
        let selectors: usize = ide::RULES.iter().map(|(selectors, _)| selectors.len()).sum();
        assert_eq!(theme.scopes.len(), selectors, "a selector syntect can't read colors nothing");
        assert_eq!(theme.settings.foreground, color("#BCBEC4"));
        // text no rule colors is in the spans too, in the scheme's plain color
        assert_eq!(color_of("python", "name = other\n", "name"), Some(PLAIN));
    }

    #[test]
    fn each_scheme_keeps_its_own_colors_for_the_same_text() {
        let text = "def both():\n    return 'kept apart'\n";
        let (ide, quiet) = (colors(Scheme::Ide, "python", text), colors(Scheme::Quiet, "python", text));
        assert_ne!(ide, quiet);
        // and each is still what was kept for it
        assert!(Arc::ptr_eq(&ide, &colors(Scheme::Ide, "python", text)));
        assert!(Arc::ptr_eq(&quiet, &colors(Scheme::Quiet, "python", text)));
        let at = text.find("def").unwrap();
        let color_at = |spans: &[Span]| spans.iter().find(|(range, _)| range.contains(&at)).map(|(_, color)| *color);
        assert_eq!(color_at(&ide), Some(KEYWORD));
        assert_eq!(color_at(&quiet), Some((0xa0, 0xa0, 0xa0, 0xff)), "Vesper's gray");
    }
}
