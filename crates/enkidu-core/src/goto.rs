//! Go to definition: where the name under a ⌘-click is defined.
//!
//! The app has no rules of its own for this: such things are already invented, and the right tool is used.
//! A language's own tool answers: its language server (`lsp`). This module only says what is asked and what comes
//! back, and turns what a server names into what the app shows ([`from_server`]).
//!
//! Python has a server (Astral's ty, phases 12 and 13). A language without one in the app (JavaScript,
//! TypeScript, Vue, any other) has no links until its server is added: nothing guesses in its place.

use serde::{Deserialize, Serialize};

use crate::symbols::{Def, Kind};

/// The most places given for one name.
pub const MAX_PLACES: usize = 50;

/// What is asked: a place in a file of the workspace.
#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Ask {
    pub path: String,
    /// The line, counted from 1.
    pub line: usize,
    /// A byte of that line inside the name, or right at its end.
    pub column: usize,
}

/// Where a name is defined.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Place {
    pub path: String,
    pub line: u32,
    pub name: String,
    /// What stands on that line, where it is known (the line that sets a variable has none): for the rows of the
    /// list, when a name has several places.
    pub kind: Option<Kind>,
    /// The class it belongs to.
    pub container: Option<String>,
}

/// The name that was asked about, and where its language's server says it is defined.
#[derive(Serialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Found {
    pub word: String,
    /// Where the name stands in its line, in bytes.
    pub start: usize,
    pub end: usize,
    /// The places that define it: one is gone to, several are listed to pick from, each with its path.
    /// None: the server knows no place in the workspace.
    pub places: Vec<Place>,
    /// The server could not be asked yet (it is starting, or still at work on an earlier question): this is no
    /// answer, and the same question is worth asking again in a moment.
    pub later: bool,
}

/// Whether a file is Python's: the one language whose server the app carries.
pub fn is_python(path: &str) -> bool {
    let ends = path.rsplit_once('.').map(|(_, end)| end.to_ascii_lowercase());
    matches!(ends.as_deref(), Some("py" | "pyi"))
}

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// The stretch of a line a name takes, around a byte of it: letters, digits, `_` and `$`. None between names, and
/// for a number. It only says what is asked about, and what is underlined when the answer comes.
pub fn name_at(line: &str, column: usize) -> Option<(usize, usize)> {
    let mut at = column.min(line.len());
    while !line.is_char_boundary(at) {
        at -= 1;
    }
    let start = line[..at].char_indices().rev().take_while(|(_, c)| is_name_char(*c)).last().map_or(at, |(i, _)| i);
    let end = at + line[at..].char_indices().find(|(_, c)| !is_name_char(*c)).map_or(line.len() - at, |(i, _)| i);
    let word = &line[start..end];
    (!word.is_empty() && !word.starts_with(|c: char| c.is_ascii_digit())).then_some((start, end))
}

/// What a language server named for the name at `ask`, as the app takes it. The name's own line is no place to go
/// to, and neither is a place outside the workspace's folder (a library's, the language's own): the app opens the
/// workspace's files. One place left is gone to, several are listed, none and the name is no link.
///
/// `on_line` says what stands on a line of a file of the workspace, where that is known (the definitions list of
/// Search's Symbols tab): a row of the list then says "save in StockItem", not only "save".
pub fn from_server(
    ask: &Ask,
    word: &str,
    range: (usize, usize),
    named: Vec<(Option<String>, usize)>,
    on_line: &dyn Fn(&str, usize) -> Option<Def>,
) -> Found {
    let mut places: Vec<Place> = Vec::new();
    for (path, line) in named {
        let Some(path) = path else { continue };
        if path == ask.path && line == ask.line {
            continue;
        }
        let place = match on_line(&path, line) {
            Some(def) => Place { path, line: def.line, name: def.name, kind: Some(def.kind), container: def.container },
            None => Place { path, line: line as u32, name: word.to_string(), kind: None, container: None },
        };
        if !places.contains(&place) {
            places.push(place);
        }
    }
    places.truncate(MAX_PLACES);
    Found { word: word.to_string(), start: range.0, end: range.1, places, later: false }
}

/// The answer while the language's server can't be asked yet: no place, and worth asking again.
pub fn not_yet(word: &str, range: (usize, usize)) -> Found {
    Found { word: word.to_string(), start: range.0, end: range.1, places: Vec::new(), later: true }
}

#[cfg(test)]
mod tests;
