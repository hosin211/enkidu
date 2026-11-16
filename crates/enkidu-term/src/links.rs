//! Links in the terminal: a program's hyperlinks (OSC 8) and web addresses in the text, also when the text wraps
//! onto the next row. ⌘ and a click opens one in the browser. Only `http`, `https` and `mailto` open: a link to a file
//! or to another app could run something.

/// A link on screen: where it goes, and the cells it covers, a run per row: (row, first column, last column).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub url: String,
    pub cells: Vec<(usize, usize, usize)>,
}

impl Link {
    /// Whether the link covers the cell at `row`, `col`.
    pub fn covers(&self, row: usize, col: usize) -> bool {
        self.cells.iter().any(|&(r, first, last)| r == row && (first..=last).contains(&col))
    }

    /// The runs of cells, from (row, column) positions in reading order.
    pub(crate) fn runs(positions: impl IntoIterator<Item = (usize, usize)>) -> Vec<(usize, usize, usize)> {
        let mut runs: Vec<(usize, usize, usize)> = Vec::new();
        for (row, col) in positions {
            match runs.last_mut() {
                Some(run) if run.0 == row && run.2 + 1 >= col => run.2 = run.2.max(col),
                _ => runs.push((row, col, col)),
            }
        }
        runs
    }
}

const SCHEMES: [&str; 3] = ["https://", "http://", "mailto:"];

/// Whether a link may open: the web and mail only, with something after the scheme.
pub fn opens(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    SCHEMES.iter().any(|scheme| lower.starts_with(scheme) && lower.len() > scheme.len())
}

/// A character that can't be part of an address in running text.
fn ends_address(c: char) -> bool {
    c.is_whitespace()
        || c.is_control()
        || matches!(c, '<' | '>' | '"' | '\'' | '`' | '{' | '}' | '|' | '\\' | '^')
        // box drawing, as Claude Code draws its panels with
        || ('\u{2500}'..='\u{257f}').contains(&c)
}

/// The web addresses in a run of text, as (start, end) in characters, the end excluded. An address loses the
/// punctuation that ends a sentence, and a closing bracket it never opened.
pub(crate) fn addresses(text: &[char]) -> Vec<(usize, usize)> {
    let lower: Vec<char> = text.iter().map(char::to_ascii_lowercase).collect();
    let at = |i: usize, scheme: &str| scheme.chars().enumerate().all(|(k, c)| lower.get(i + k) == Some(&c));
    let mut found = Vec::new();
    let mut i = 0;
    while i < text.len() {
        let starts_word = i == 0 || !text[i - 1].is_alphanumeric();
        let Some(scheme) = SCHEMES.iter().find(|s| starts_word && at(i, s)) else {
            i += 1;
            continue;
        };
        let body = i + scheme.chars().count();
        let mut end = body;
        while end < text.len() && !ends_address(text[end]) {
            end += 1;
        }
        loop {
            match text[end - 1] {
                '.' | ',' | ';' | ':' | '!' | '?' if end > body => end -= 1,
                close @ (')' | ']') if end > body => {
                    let open = if close == ')' { '(' } else { '[' };
                    let opened = text[i..end].iter().filter(|&&c| c == open).count();
                    let closed = text[i..end].iter().filter(|&&c| c == close).count();
                    if closed > opened { end -= 1 } else { break }
                }
                _ => break,
            }
        }
        if end > body {
            found.push((i, end));
        }
        i = end.max(i + 1);
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(text: &str) -> Vec<String> {
        let chars: Vec<char> = text.chars().collect();
        addresses(&chars).into_iter().map(|(a, b)| chars[a..b].iter().collect()).collect()
    }

    #[test]
    fn finds_web_and_mail_addresses_in_running_text() {
        assert_eq!(found("see https://github.com/acme/shop/pull/12 now"), ["https://github.com/acme/shop/pull/12"]);
        assert_eq!(found("HTTP://Example.com/a?b=1&c=2#d"), ["HTTP://Example.com/a?b=1&c=2#d"]);
        assert_eq!(found("mail mailto:team@example.com today"), ["mailto:team@example.com"]);
        assert_eq!(found("two: http://a.io and https://b.io/x"), ["http://a.io", "https://b.io/x"]);
        assert_eq!(found("https://ar.wikipedia.org/wiki/مرحبا"), ["https://ar.wikipedia.org/wiki/مرحبا"]);
    }

    #[test]
    fn an_address_ends_where_the_sentence_or_the_bracket_around_it_does() {
        assert_eq!(found("Opened https://example.com/pr/1."), ["https://example.com/pr/1"]);
        assert_eq!(found("(see https://example.com/x)"), ["https://example.com/x"]);
        assert_eq!(
            found("https://en.wikipedia.org/wiki/Rust_(programming_language)"),
            ["https://en.wikipedia.org/wiki/Rust_(programming_language)"]
        );
        assert_eq!(found("\"https://example.com/q\""), ["https://example.com/q"]);
        assert_eq!(found("│ https://example.com/box │"), ["https://example.com/box"]);
        assert_eq!(found("<https://example.com/angle>"), ["https://example.com/angle"]);
    }

    #[test]
    fn a_scheme_alone_or_inside_a_word_is_no_address() {
        assert!(found("https:// nothing").is_empty());
        assert!(found("xhttps://example.com").is_empty());
        assert!(found("ftp://example.com file:///etc/hosts").is_empty());
    }

    #[test]
    fn only_the_web_and_mail_open() {
        assert!(opens("https://example.com"));
        assert!(opens("HTTP://example.com"));
        assert!(opens("mailto:a@b.c"));
        assert!(!opens("file:///Applications/Calculator.app"));
        assert!(!opens("vscode://file/x"));
        assert!(!opens("https://"));
    }

    #[test]
    fn cells_in_reading_order_become_a_run_per_row() {
        assert_eq!(Link::runs([(0, 5), (0, 6), (0, 7), (1, 0), (1, 1)]), [(0, 5, 7), (1, 0, 1)]);
    }
}
