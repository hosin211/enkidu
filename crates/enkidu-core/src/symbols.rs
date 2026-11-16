//! Go to symbol: classes, functions and methods across a workspace, by name. See SEARCH.md for why it works this way.
//!
//! Unlike text search this needs an index: fuzzy matching a name ("StoLev" finds StockLevel) can't be done by reading
//! every file per keystroke. The index is the lightweight kind Sublime Text and GitHub's search-based code navigation
//! use: only definitions (name, kind, line, container), found by a one-pass line scanner per language (no parser, no
//! language server). It is built in the background when Go to symbol is first used (parallel, well under a second for
//! tens of thousands of files), kept per workspace in compact form (a few MB), refreshed for just the files the file
//! watcher saw change, and dropped for workspaces you haven't searched lately.

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use serde::Serialize;
use std::collections::{BinaryHeap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Instant, UNIX_EPOCH};

/// Same limit as text search: bigger files are bundles or data, not code you navigate.
const MAX_FILE_BYTES: u64 = 1_000_000;
/// Symbol indexes kept at once (one per workspace); the least recently searched one goes first.
const KEEP_INDEXES: usize = 3;

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
#[repr(u8)]
pub enum Kind {
    Class,
    Interface,
    Struct,
    Enum,
    Trait,
    Type,
    Module,
    Component,
    Function,
    Method,
}

impl Kind {
    /// Types rank above functions, functions above methods, when the names match equally well.
    fn bonus(self) -> u32 {
        match self {
            Kind::Class | Kind::Interface | Kind::Struct | Kind::Enum | Kind::Trait | Kind::Component => 12,
            Kind::Type | Kind::Module => 8,
            Kind::Function => 4,
            Kind::Method => 0,
        }
    }
}

/// A definition as the scanner finds it.
#[derive(Debug, Clone, PartialEq)]
pub struct Def {
    pub name: String,
    pub kind: Kind,
    pub line: u32,
    pub container: Option<String>,
}

fn ident_len(s: &str) -> usize {
    s.char_indices().find(|(_, c)| !(c.is_alphanumeric() || *c == '_' || *c == '$')).map_or(s.len(), |(i, _)| i)
}

fn ident(s: &str) -> &str {
    let s = s.trim_start();
    &s[..ident_len(s)]
}

/// Strips any of the given words (each followed by a space) from the front, in any order.
fn strip_words<'a>(mut s: &'a str, words: &[&str]) -> &'a str {
    'again: loop {
        for w in words {
            if let Some(rest) = s.strip_prefix(w) {
                if let Some(rest) = rest.strip_prefix(' ') {
                    s = rest.trim_start();
                    continue 'again;
                }
            }
        }
        return s;
    }
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// Finds the definitions in one file, by its extension. Unknown kinds of file have none.
pub fn extract(path: &str, text: &str) -> Vec<Def> {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    let mut out = Vec::new();
    match ext.as_str() {
        "py" | "pyi" => python(text, &mut out),
        "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "mts" | "cts" => javascript(text, 0, None, &mut out),
        "vue" | "svelte" => {
            let stem = path.rsplit('/').next().unwrap_or(path).rsplit_once('.').map_or(path, |(s, _)| s);
            if ext == "vue" && !stem.is_empty() {
                out.push(Def { name: stem.to_string(), kind: Kind::Component, line: 1, container: None });
            }
            // only the <script> part is code
            let mut offset = 0u32;
            let mut rest = text;
            while let Some(open) = rest.find("<script") {
                let Some(body_start) = rest[open..].find('>').map(|i| open + i + 1) else { break };
                let body_end = rest[body_start..].find("</script>").map_or(rest.len(), |i| body_start + i);
                let first_line = offset + rest[..body_start].matches('\n').count() as u32;
                javascript(&rest[body_start..body_end], first_line, (ext == "vue").then_some(stem), &mut out);
                offset += rest[..body_end].matches('\n').count() as u32;
                rest = &rest[body_end..];
            }
        }
        "rs" => rust(text, &mut out),
        "go" => go(text, &mut out),
        "rb" => ruby(text, &mut out),
        "java" | "kt" | "kts" | "scala" | "cs" | "swift" | "php" | "dart" | "groovy" => c_like(text, &mut out),
        _ => {}
    }
    out
}

fn python(text: &str, out: &mut Vec<Def>) {
    // open scopes: indentation, name, and whether it is a class
    let mut scopes: Vec<(usize, String, bool)> = Vec::new();
    let mut in_string: Option<&str> = None;
    for (i, line) in text.lines().enumerate() {
        if let Some(q) = in_string {
            if line.matches(q).count() % 2 == 1 {
                in_string = None;
            }
            continue;
        }
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = indent_of(line);
        while scopes.last().is_some_and(|s| s.0 >= indent) {
            scopes.pop();
        }
        let def = if let Some(rest) = trimmed.strip_prefix("class ") {
            Some((true, rest))
        } else if let Some(rest) = trimmed.strip_prefix("def ").or_else(|| trimmed.strip_prefix("async def ")) {
            Some((false, rest))
        } else {
            None
        };
        if let Some((is_class, rest)) = def {
            let name = ident(rest);
            if !name.is_empty() {
                let parent = scopes.last();
                // a function inside a function is a local helper, not something to go to
                if is_class || !parent.is_some_and(|p| !p.2) {
                    let in_class = parent.filter(|p| p.2);
                    out.push(Def {
                        name: name.to_string(),
                        kind: if is_class { Kind::Class } else if in_class.is_some() { Kind::Method } else { Kind::Function },
                        line: i as u32 + 1,
                        container: in_class.map(|p| p.1.clone()),
                    });
                }
                scopes.push((indent, name.to_string(), is_class));
            }
        }
        for q in ["\"\"\"", "'''"] {
            if trimmed.matches(q).count() % 2 == 1 {
                in_string = Some(q);
                break;
            }
        }
    }
}

const JS_NOT_METHODS: [&str; 16] =
    ["if", "for", "while", "switch", "catch", "return", "function", "new", "typeof", "await", "yield", "else", "do", "try", "with", "super"];

/// JavaScript and TypeScript (and the script part of Vue files). `component` names the Vue component, whose
/// object-literal methods (methods: { save() { ... } }) count as its methods.
fn javascript(text: &str, first_line: u32, component: Option<&str>, out: &mut Vec<Def>) {
    let mut classes: Vec<(usize, String)> = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with('*') || trimmed.starts_with("/*") {
            continue;
        }
        let indent = indent_of(line);
        let line_no = first_line + i as u32 + 1;
        if trimmed.starts_with('}') {
            while classes.last().is_some_and(|c| c.0 >= indent) {
                classes.pop();
            }
        }
        let s = strip_words(trimmed, &["export", "default", "declare", "abstract"]);
        let mut push = |name: &str, kind: Kind, container: Option<String>| {
            if !name.is_empty() {
                out.push(Def { name: name.to_string(), kind, line: line_no, container });
            }
        };
        if let Some(rest) = s.strip_prefix("class ") {
            let name = ident(rest);
            push(name, Kind::Class, None);
            if !name.is_empty() && !trimmed.ends_with('}') {
                classes.push((indent, name.to_string()));
            }
            continue;
        }
        if let Some(rest) = s.strip_prefix("interface ") {
            push(ident(rest), Kind::Interface, None);
            continue;
        }
        if let Some(rest) = s.strip_prefix("enum ").or_else(|| s.strip_prefix("const enum ")) {
            push(ident(rest), Kind::Enum, None);
            continue;
        }
        if let Some(rest) = s.strip_prefix("namespace ").or_else(|| s.strip_prefix("module ")) {
            if rest.trim_end().ends_with('{') {
                push(ident(rest), Kind::Module, None);
            }
            continue;
        }
        if let Some(rest) = s.strip_prefix("type ") {
            let name = ident(rest);
            let after = rest.trim_start()[name.len()..].trim_start();
            if after.starts_with('=') || after.starts_with('<') {
                push(name, Kind::Type, None);
            }
            continue;
        }
        let f = strip_words(s, &["async"]);
        if let Some(rest) = f.strip_prefix("function") {
            let rest = rest.trim_start_matches('*').trim_start();
            push(ident(rest), Kind::Function, None);
            continue;
        }
        // top-level const/let/var bound to a function: const save = async (x) => ..., const save = function ...
        if indent == 0 {
            if let Some(rest) = s.strip_prefix("const ").or_else(|| s.strip_prefix("let ")).or_else(|| s.strip_prefix("var ")) {
                let name = ident(rest);
                if let Some(value) = rest.trim_start()[name.len()..].split_once('=').map(|(_, v)| v.trim_start()) {
                    let value = strip_words(value, &["async"]);
                    let arrow = value.contains("=>") && (value.starts_with('(') || value.starts_with('<') || value.split("=>").next().is_some_and(|p| ident(p) == p.trim()));
                    if value.starts_with("function") || arrow {
                        push(name, Kind::Function, None);
                    }
                }
                continue;
            }
        }
        // methods: in a class body, or a Vue component's object literal
        let in_class = classes.last().filter(|c| indent > c.0).map(|c| c.1.clone());
        let container = in_class.or_else(|| component.filter(|_| indent > 0).map(str::to_string));
        if let Some(container) = container {
            let m = strip_words(
                trimmed,
                &["public", "private", "protected", "static", "readonly", "override", "abstract", "async", "get", "set", "declare"],
            );
            let m = m.trim_start_matches('*').trim_start();
            let name = ident(m);
            let after = m[name.len()..].trim_start();
            let opens = trimmed.trim_end().ends_with('{') || trimmed.trim_end().ends_with("=> {");
            if !name.is_empty() && (after.starts_with('(') || after.starts_with('<')) && opens && !JS_NOT_METHODS.contains(&name) {
                push(name, Kind::Method, Some(container));
            }
        }
    }
}

fn rust(text: &str, out: &mut Vec<Def>) {
    // impl and trait blocks: indentation and the type they belong to
    let mut blocks: Vec<(usize, String)> = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        let indent = indent_of(line);
        if trimmed.starts_with('}') {
            while blocks.last().is_some_and(|b| b.0 >= indent) {
                blocks.pop();
            }
            continue;
        }
        let mut s = trimmed;
        if let Some(rest) = s.strip_prefix("pub(") {
            s = rest.split_once(')').map_or(rest, |(_, r)| r.trim_start());
        }
        let s = strip_words(s, &["pub", "async", "const", "unsafe", "extern \"C\"", "default"]);
        let line_no = i as u32 + 1;
        let mut push = |name: &str, kind: Kind, container: Option<String>| {
            if !name.is_empty() {
                out.push(Def { name: name.to_string(), kind, line: line_no, container });
            }
        };
        if let Some(rest) = s.strip_prefix("fn ") {
            let container = blocks.last().filter(|b| indent > b.0).map(|b| b.1.clone());
            push(ident(rest), if container.is_some() { Kind::Method } else { Kind::Function }, container);
        } else if let Some(rest) = s.strip_prefix("struct ") {
            push(ident(rest), Kind::Struct, None);
        } else if let Some(rest) = s.strip_prefix("enum ") {
            push(ident(rest), Kind::Enum, None);
        } else if let Some(rest) = s.strip_prefix("union ") {
            push(ident(rest), Kind::Struct, None);
        } else if let Some(rest) = s.strip_prefix("trait ") {
            let name = ident(rest);
            push(name, Kind::Trait, None);
            if !name.is_empty() && trimmed.trim_end().ends_with('{') {
                blocks.push((indent, name.to_string()));
            }
        } else if let Some(rest) = s.strip_prefix("type ") {
            push(ident(rest), Kind::Type, None);
        } else if let Some(rest) = s.strip_prefix("mod ") {
            push(ident(rest), Kind::Module, None);
        } else if let Some(rest) = s.strip_prefix("macro_rules! ") {
            push(ident(rest), Kind::Function, None);
        } else if s.starts_with("impl") && (s[4..].starts_with(' ') || s[4..].starts_with('<')) {
            // impl Foo, impl<T> Foo<T>, impl Trait for Foo: the methods belong to Foo
            let mut head = &s[4..];
            if head.starts_with('<') {
                let mut depth = 0;
                let end = head.char_indices().find(|(_, c)| {
                    match c {
                        '<' => depth += 1,
                        '>' => depth -= 1,
                        _ => {}
                    }
                    depth == 0
                });
                head = end.map_or("", |(i, _)| &head[i + 1..]);
            }
            let head = head.split('{').next().unwrap_or("").split(" where ").next().unwrap_or("");
            let target = head.rsplit(" for ").next().unwrap_or(head).trim().trim_start_matches('&');
            let name = ident(target.rsplit("::").next().unwrap_or(target));
            if !name.is_empty() && trimmed.trim_end().ends_with('{') {
                blocks.push((indent, name.to_string()));
            }
        }
    }
}

fn go(text: &str, out: &mut Vec<Def>) {
    // inside a grouped "type ( ... )" block, each line names a type
    let mut in_types = false;
    for (i, line) in text.lines().enumerate() {
        let line_no = i as u32 + 1;
        if in_types {
            if line.trim_start().starts_with(')') {
                in_types = false;
            } else if line.starts_with('\t') || line.starts_with(' ') {
                go_type(line.trim_start(), line_no, out);
            }
            continue;
        }
        if line.trim_end() == "type (" {
            in_types = true;
            continue;
        }
        if let Some(rest) = line.strip_prefix("func ") {
            if let Some(recv) = rest.strip_prefix('(') {
                // func (s *Store) Save(...): a method of Store
                let Some((recv, after)) = recv.split_once(')') else { continue };
                let ty = recv.split_whitespace().last().unwrap_or("").trim_start_matches('*');
                let ty = ty.split('[').next().unwrap_or(ty);
                let name = ident(after);
                if !name.is_empty() {
                    out.push(Def { name: name.to_string(), kind: Kind::Method, line: line_no, container: Some(ty.to_string()).filter(|t| !t.is_empty()) });
                }
            } else {
                let name = ident(rest);
                if !name.is_empty() {
                    out.push(Def { name: name.to_string(), kind: Kind::Function, line: line_no, container: None });
                }
            }
        } else if let Some(rest) = line.strip_prefix("type ") {
            go_type(rest, line_no, out);
        }
    }
}

fn go_type(rest: &str, line_no: u32, out: &mut Vec<Def>) {
    let name = ident(rest);
    let after = rest.trim_start()[name.len()..].trim_start();
    let kind = if after.starts_with("struct") {
        Kind::Struct
    } else if after.starts_with("interface") {
        Kind::Interface
    } else {
        Kind::Type
    };
    if !name.is_empty() {
        out.push(Def { name: name.to_string(), kind, line: line_no, container: None });
    }
}

fn ruby(text: &str, out: &mut Vec<Def>) {
    let mut scopes: Vec<(usize, String)> = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = indent_of(line);
        if trimmed == "end" || trimmed.starts_with("end ") || trimmed.starts_with("end.") {
            while scopes.last().is_some_and(|s| s.0 >= indent) {
                scopes.pop();
            }
            continue;
        }
        let line_no = i as u32 + 1;
        if let Some(rest) = trimmed.strip_prefix("class ").or_else(|| trimmed.strip_prefix("module ")) {
            if rest.starts_with("<<") {
                continue; // class << self
            }
            let name = rest.split_whitespace().next().unwrap_or("").split('<').next().unwrap_or("");
            let name = name.rsplit("::").next().unwrap_or(name);
            if !name.is_empty() {
                let kind = if trimmed.starts_with("class") { Kind::Class } else { Kind::Module };
                out.push(Def { name: name.to_string(), kind, line: line_no, container: scopes.last().map(|s| s.1.clone()) });
                scopes.push((indent, name.to_string()));
            }
        } else if let Some(rest) = trimmed.strip_prefix("def ") {
            let rest = rest.strip_prefix("self.").unwrap_or(rest);
            let len = rest.find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '?' || c == '!' || c == '=')).unwrap_or(rest.len());
            let name = &rest[..len];
            if !name.is_empty() {
                let container = scopes.last().map(|s| s.1.clone());
                let kind = if container.is_some() { Kind::Method } else { Kind::Function };
                out.push(Def { name: name.to_string(), kind, line: line_no, container });
            }
        }
    }
}

/// Java, Kotlin, C#, Swift, PHP, Scala, Dart: type declarations, plus the function keywords that mark functions.
fn c_like(text: &str, out: &mut Vec<Def>) {
    const MODIFIERS: [&str; 22] = [
        "public", "private", "protected", "internal", "static", "final", "abstract", "sealed", "open", "data", "partial",
        "export", "inline", "value", "enum", "annotation", "override", "suspend", "async", "readonly", "fileprivate", "mutating",
    ];
    for (i, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with('*') || trimmed.starts_with('@') {
            continue;
        }
        let line_no = i as u32 + 1;
        // "enum class Foo" (Kotlin) and "enum Foo" both end up here
        let enumish = trimmed.contains("enum ");
        let s = strip_words(trimmed, &MODIFIERS);
        let found = [
            ("class ", if enumish { Kind::Enum } else { Kind::Class }),
            ("interface ", Kind::Interface),
            ("protocol ", Kind::Interface),
            ("struct ", Kind::Struct),
            ("record ", Kind::Class),
            ("object ", Kind::Class),
            ("trait ", Kind::Trait),
            ("fun ", Kind::Function),
            ("func ", Kind::Function),
            ("function ", Kind::Function),
            ("def ", Kind::Function),
        ]
        .into_iter()
        .find_map(|(kw, kind)| s.strip_prefix(kw).map(|rest| (rest, kind)));
        let (rest, kind) = match found {
            Some(f) => f,
            None if enumish && trimmed.starts_with("enum ") || strip_words(trimmed, &MODIFIERS[..11]).starts_with("enum ") => {
                (strip_words(trimmed, &MODIFIERS[..11]).trim_start_matches("enum ").trim_start(), Kind::Enum)
            }
            None => continue,
        };
        // Kotlin extension functions: fun String.slug() names slug
        let rest = rest.trim_start_matches('<').trim_start();
        let name = ident(rest.rsplit_once('.').filter(|(recv, _)| !recv.contains('(')).map_or(rest, |(_, n)| n));
        if !name.is_empty() {
            out.push(Def { name: name.to_string(), kind, line: line_no, container: None });
        }
    }
}

/// One definition, packed into 16 bytes: its name and container are ranges of the index's `names` text.
#[derive(Clone, Copy)]
struct Rec {
    name: u32,
    cont: u32,
    line: u32,
    name_len: u8,
    cont_len: u8,
    kind: Kind,
}

/// One indexed file: where its path is in `paths`, its run of definitions in `recs`, and its size and modification
/// time when it was read (to tell later whether it changed).
#[derive(Clone, Copy)]
struct Entry {
    path: u32,
    path_len: u32,
    start: u32,
    count: u32,
    len: u64,
    mtime: u64,
}

/// Identifiers longer than this aren't identifiers (generated code, data); they are left out.
const MAX_NAME: usize = 255;

/// Definitions read from a run of files, in one string and one list (offsets relative to its own `names`), so a
/// whole run costs two allocations instead of a few per file.
#[derive(Default)]
struct Batch {
    names: String,
    recs: Vec<Rec>,
    /// Per file read: its position in the list being indexed, size, modification time, and its run in `recs`.
    files: Vec<(usize, u64, u64, u32, u32)>,
}

impl Batch {
    fn add(&mut self, at: usize, len: u64, mtime: u64, defs: Vec<Def>) {
        let start = self.recs.len() as u32;
        // a class's methods share its name as container: store it once per file
        let mut containers: Vec<(String, u32, u8)> = Vec::new();
        for d in defs {
            if d.name.len() > MAX_NAME || d.container.as_ref().is_some_and(|c| c.len() > MAX_NAME) {
                continue;
            }
            let name = self.names.len() as u32;
            self.names.push_str(&d.name);
            let (cont, cont_len) = match d.container {
                Some(c) => match containers.iter().find(|x| x.0 == c) {
                    Some(&(_, off, len)) => (off, len),
                    None => {
                        let off = self.names.len() as u32;
                        let len = c.len() as u8;
                        self.names.push_str(&c);
                        containers.push((c, off, len));
                        (off, len)
                    }
                },
                None => (0, 0),
            };
            self.recs.push(Rec { name, cont, line: d.line, name_len: d.name.len() as u8, cont_len, kind: d.kind });
        }
        self.files.push((at, len, mtime, start, self.recs.len() as u32 - start));
    }
}

fn has_symbols(path: &str) -> bool {
    let ext = path.rsplit('.').next().unwrap_or("");
    matches!(
        ext,
        "py" | "pyi" | "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "mts" | "cts" | "vue" | "svelte" | "rs" | "go" | "rb" | "java"
            | "kt" | "kts" | "scala" | "cs" | "swift" | "php" | "dart" | "groovy"
    )
}

fn stat(path: &Path) -> Option<(u64, u64)> {
    let m = std::fs::metadata(path).ok().filter(|m| m.is_file())?;
    let mtime = m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos() as u64);
    Some((m.len(), mtime))
}

/// Reads one file and finds its definitions. None when it is gone. Big, binary and minified files have none.
fn index_file(root: &Path, rel: &str, buf: &mut Vec<u8>) -> Option<(u64, u64, Vec<Def>)> {
    let path = root.join(rel);
    let (len, mtime) = stat(&path)?;
    if len > MAX_FILE_BYTES {
        return Some((len, mtime, Vec::new()));
    }
    buf.clear();
    std::fs::File::open(&path).ok()?.read_to_end(buf).ok()?;
    if memchr::memchr(0, &buf[..buf.len().min(8192)]).is_some() {
        return Some((len, mtime, Vec::new()));
    }
    // minified bundles have a few enormous lines; they hold no definitions worth listing
    let lines = memchr::memchr_iter(b'\n', buf).count().max(1);
    if buf.len() / lines > 400 {
        return Some((len, mtime, Vec::new()));
    }
    let text = String::from_utf8_lossy(buf);
    Some((len, mtime, extract(rel, &text)))
}

/// A full build reads every file and frees what it read; macOS's allocator keeps freed memory for reuse, so the app
/// would look bigger than it is. This hands it back (malloc_zone_pressure_relief, from <malloc/malloc.h>).
fn release_freed_memory() {
    #[cfg(target_os = "macos")]
    {
        extern "C" {
            fn malloc_zone_pressure_relief(zone: *mut std::ffi::c_void, goal: usize) -> usize;
        }
        // SAFETY: a null zone means every zone; goal 0 means release as much as possible. It only frees unused pages.
        unsafe {
            malloc_zone_pressure_relief(std::ptr::null_mut(), 0);
        }
    }
}

/// What a search returns per definition.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SymbolHit {
    pub name: String,
    pub kind: Kind,
    pub container: Option<String>,
    pub path: String,
    pub line: u32,
    /// Positions in `name` that matched, for highlighting.
    pub indices: Vec<u32>,
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct SymbolResults {
    pub hits: Vec<SymbolHit>,
    /// How many definitions matched in all.
    pub total: usize,
    pub symbols: usize,
    pub files: usize,
    /// Bytes the index takes (its own data, not counting allocator overhead).
    pub bytes: usize,
    /// Time spent bringing the index up to date for this search (0 when it already was).
    pub refresh_ms: u64,
    pub search_ms: u64,
}

/// One workspace's definitions. Everything lives in four flat lists (paths, names, definitions, files), sorted by
/// path: tens of thousands of files cost a handful of allocations, not a few each, which is most of the memory a
/// map-of-strings index wastes. A changed file's new definitions go at the end; the old ones become dead space,
/// compacted away once there is enough of it.
pub struct SymbolIndex {
    root: PathBuf,
    entries: Vec<Entry>,
    paths: String,
    names: String,
    recs: Vec<Rec>,
    /// Definitions still referenced by an entry (the rest of `recs` is dead).
    live: usize,
    /// The file watcher's change counter when the index was last brought up to date.
    generation: u64,
    built: bool,
}

enum Seen {
    Unchanged,
    Read,
    Gone,
}

impl SymbolIndex {
    pub fn new(root: &Path) -> Self {
        SymbolIndex {
            root: root.to_path_buf(),
            entries: Vec::new(),
            paths: String::new(),
            names: String::new(),
            recs: Vec::new(),
            live: 0,
            generation: u64::MAX,
            built: false,
        }
    }

    fn path(&self, e: &Entry) -> &str {
        &self.paths[e.path as usize..(e.path + e.path_len) as usize]
    }

    fn find(&self, path: &str) -> Result<usize, usize> {
        self.entries.binary_search_by(|e| self.path(e).cmp(path))
    }

    fn name(&self, r: &Rec) -> &str {
        &self.names[r.name as usize..r.name as usize + r.name_len as usize]
    }

    fn container(&self, r: &Rec) -> Option<&str> {
        (r.cont_len > 0).then(|| &self.names[r.cont as usize..r.cont as usize + r.cont_len as usize])
    }

    pub fn symbols(&self) -> usize {
        self.live
    }

    pub fn files(&self) -> usize {
        self.entries.len()
    }

    /// What the index holds in memory: its four lists, at their allocated size.
    pub fn bytes(&self) -> usize {
        self.paths.capacity() + self.names.capacity() + self.recs.capacity() * std::mem::size_of::<Rec>() + self.entries.capacity() * std::mem::size_of::<Entry>()
    }

    /// Appends a batch's names and definitions; returns where its definitions start and the shift for its names.
    fn append(&mut self, batch: &Batch) -> (u32, u32) {
        let names_base = self.names.len() as u32;
        let recs_base = self.recs.len() as u32;
        self.names.push_str(&batch.names);
        self.recs.extend(batch.recs.iter().map(|r| Rec { name: r.name + names_base, cont: if r.cont_len > 0 { r.cont + names_base } else { 0 }, ..*r }));
        (recs_base, names_base)
    }

    fn add_path(&mut self, path: &str) -> (u32, u32) {
        let at = self.paths.len() as u32;
        self.paths.push_str(path);
        (at, path.len() as u32)
    }

    /// Brings the index up to date. `files` is the workspace's file list, sorted (tracked plus new); `changed` names
    /// the files the watcher saw change since the last time (None: it lost track, or this is the first build, so
    /// every file is checked against its size and modification time).
    pub fn refresh(&mut self, files: &[&str], changed: Option<&HashSet<String>>, generation: u64) {
        match (changed, self.built) {
            (Some(changed), true) => self.refresh_changed(files, changed),
            _ => {
                self.refresh_all(files);
                release_freed_memory();
            }
        }
        if self.recs.len() - self.live > 50_000.max(self.live / 3) {
            self.compact();
        }
        self.built = true;
        self.generation = generation;
    }

    fn refresh_all(&mut self, files: &[&str]) {
        let mut listed: Vec<&str> = files.iter().copied().filter(|p| has_symbols(p)).collect();
        listed.sort_unstable();
        listed.dedup();
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).saturating_sub(1).clamp(1, 8);
        const RUN: usize = 64;
        let next = AtomicUsize::new(0);
        // Each run of files goes into these the moment it is read and is dropped right away, so the build never
        // holds more than a few runs on top of the index itself.
        #[derive(Default)]
        struct Fresh {
            names: String,
            recs: Vec<Rec>,
            /// Per file read: its position in `listed`, size, modification time, and its run in `recs`.
            read: Vec<(usize, u64, u64, u32, u32)>,
            /// Per run: where it starts in `listed`, and what happened to each of its files.
            seen: Vec<(usize, Vec<Seen>)>,
        }
        let fresh = Mutex::new(Fresh::default());
        {
            let this = &*self;
            let listed = &listed;
            let fresh = &fresh;
            std::thread::scope(|scope| {
                for _ in 0..threads {
                    scope.spawn(|| {
                        let mut buf = Vec::new();
                        loop {
                            let from = next.fetch_add(RUN, Ordering::Relaxed);
                            if from >= listed.len() {
                                break;
                            }
                            let mut batch = Batch::default();
                            let mut seen = Vec::with_capacity(RUN);
                            for (k, &path) in listed[from..(from + RUN).min(listed.len())].iter().enumerate() {
                                let known = this.find(path).ok().map(|i| this.entries[i]);
                                if let Some(known) = known {
                                    if stat(&this.root.join(path)) == Some((known.len, known.mtime)) {
                                        seen.push(Seen::Unchanged);
                                        continue;
                                    }
                                }
                                match index_file(&this.root, path, &mut buf) {
                                    Some((len, mtime, defs)) => {
                                        batch.add(from + k, len, mtime, defs);
                                        seen.push(Seen::Read);
                                    }
                                    None => seen.push(Seen::Gone),
                                }
                            }
                            let mut f = fresh.lock().unwrap_or_else(|e| e.into_inner());
                            let names_base = f.names.len() as u32;
                            let recs_base = f.recs.len() as u32;
                            f.names.push_str(&batch.names);
                            f.recs.extend(batch.recs.iter().map(|r| Rec { name: r.name + names_base, cont: if r.cont_len > 0 { r.cont + names_base } else { 0 }, ..*r }));
                            f.read.extend(batch.files.iter().map(|&(at, len, mtime, start, count)| (at, len, mtime, recs_base + start, count)));
                            f.seen.push((from, seen));
                        }
                        // the read buffer goes back to the system here, not when the index is dropped
                        drop(buf);
                    });
                }
            });
        }
        let mut fresh = fresh.into_inner().unwrap_or_else(|e| e.into_inner());
        fresh.seen.sort_unstable_by_key(|s| s.0);
        fresh.read.sort_unstable_by_key(|r| r.0);

        // Unchanged files keep their definitions where they are; when there are none (the first build), the fresh
        // lists simply become the index's, with nothing copied.
        let any_kept = fresh.seen.iter().any(|(_, seen)| seen.iter().any(|s| matches!(s, Seen::Unchanged)));
        let recs_base = if any_kept {
            let batch = Batch { names: std::mem::take(&mut fresh.names), recs: std::mem::take(&mut fresh.recs), files: Vec::new() };
            self.append(&batch).0
        } else {
            self.names = std::mem::take(&mut fresh.names);
            self.recs = std::mem::take(&mut fresh.recs);
            0
        };

        // the new list of entries, in path order
        let mut entries = Vec::with_capacity(listed.len());
        let mut live = 0usize;
        let mut read = fresh.read.iter().peekable();
        for (from, seen) in &fresh.seen {
            for (k, how) in seen.iter().enumerate() {
                let path = listed[from + k];
                match how {
                    Seen::Unchanged => {
                        if let Ok(i) = self.find(path) {
                            live += self.entries[i].count as usize;
                            entries.push(self.entries[i]);
                        }
                    }
                    Seen::Read => {
                        let Some(&&(_, len, mtime, start, count)) = read.peek().filter(|r| r.0 == from + k) else { continue };
                        read.next();
                        let (p, pl) = match self.find(path) {
                            Ok(i) => (self.entries[i].path, self.entries[i].path_len),
                            Err(_) => self.add_path(path),
                        };
                        live += count as usize;
                        entries.push(Entry { path: p, path_len: pl, start: recs_base + start, count, len, mtime });
                    }
                    Seen::Gone => {}
                }
            }
        }
        self.entries = entries;
        self.live = live;
        self.shrink();
    }

    fn refresh_changed(&mut self, files: &[&str], changed: &HashSet<String>) {
        let mut buf = Vec::new();
        let mut changed: Vec<&String> = changed.iter().collect();
        changed.sort_unstable();
        for path in changed {
            let path = path.as_str();
            let listed = has_symbols(path) && files.binary_search(&path).is_ok();
            let known = self.find(path);
            let now = if listed { stat(&self.root.join(path)) } else { None };
            if let (Ok(i), Some(now)) = (known, now) {
                if (self.entries[i].len, self.entries[i].mtime) == now {
                    continue;
                }
            }
            let read = if listed { index_file(&self.root, path, &mut buf) } else { None };
            match (known, read) {
                (Ok(i), Some((len, mtime, defs))) => {
                    let mut batch = Batch::default();
                    batch.add(0, len, mtime, defs);
                    let (recs_base, _) = self.append(&batch);
                    let (_, _, _, start, count) = batch.files[0];
                    let e = &mut self.entries[i];
                    self.live = self.live - e.count as usize + count as usize;
                    *e = Entry { start: recs_base + start, count, len, mtime, ..*e };
                }
                (Err(at), Some((len, mtime, defs))) => {
                    let mut batch = Batch::default();
                    batch.add(0, len, mtime, defs);
                    let (recs_base, _) = self.append(&batch);
                    let (_, _, _, start, count) = batch.files[0];
                    let (p, pl) = self.add_path(path);
                    self.live += count as usize;
                    self.entries.insert(at, Entry { path: p, path_len: pl, start: recs_base + start, count, len, mtime });
                }
                (Ok(i), None) => {
                    self.live -= self.entries[i].count as usize;
                    self.entries.remove(i);
                }
                (Err(_), None) => {}
            }
        }
    }

    /// Copies the live paths, names and definitions into fresh lists, leaving the dead space behind.
    fn compact(&mut self) {
        let old_paths = std::mem::take(&mut self.paths);
        let old_names = std::mem::take(&mut self.names);
        let old_recs = std::mem::take(&mut self.recs);
        let mut paths = String::with_capacity(old_paths.len());
        let mut names = String::with_capacity(old_names.len() / 2);
        let mut recs = Vec::with_capacity(self.live);
        for e in &mut self.entries {
            let p = &old_paths[e.path as usize..(e.path + e.path_len) as usize];
            e.path = paths.len() as u32;
            paths.push_str(p);
            let start = recs.len() as u32;
            let mut containers: Vec<(u32, u32)> = Vec::new();
            for r in &old_recs[e.start as usize..(e.start + e.count) as usize] {
                let name = names.len() as u32;
                names.push_str(&old_names[r.name as usize..r.name as usize + r.name_len as usize]);
                let cont = if r.cont_len == 0 {
                    0
                } else if let Some(&(_, new)) = containers.iter().find(|c| c.0 == r.cont) {
                    new
                } else {
                    let new = names.len() as u32;
                    names.push_str(&old_names[r.cont as usize..r.cont as usize + r.cont_len as usize]);
                    containers.push((r.cont, new));
                    new
                };
                recs.push(Rec { name, cont, ..*r });
            }
            e.start = start;
        }
        self.paths = paths;
        self.names = names;
        self.recs = recs;
        self.shrink();
    }

    fn shrink(&mut self) {
        self.paths.shrink_to_fit();
        self.names.shrink_to_fit();
        self.recs.shrink_to_fit();
        self.entries.shrink_to_fit();
    }

    /// The best matches for `query`, ranked like fzf (nucleo's scoring), with types ahead of functions ahead of
    /// methods when names match equally well, and an exact name first.
    pub fn search(&self, query: &str, limit: usize) -> (Vec<SymbolHit>, usize) {
        let query = query.trim();
        if query.is_empty() || limit == 0 {
            return (Vec::new(), 0);
        }
        // like PyCharm's Go to Symbol: case doesn't matter, capitals and word starts still score higher
        let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
        let mut matcher = Matcher::new(Config::DEFAULT);
        let mut chars = Vec::new();
        // a min-heap of the best `limit`: higher score, then the shorter name, then by file and position
        type Ranked = std::cmp::Reverse<(u32, std::cmp::Reverse<usize>, std::cmp::Reverse<u32>, std::cmp::Reverse<u32>)>;
        let mut best: BinaryHeap<Ranked> = BinaryHeap::new();
        let mut total = 0;
        for (fi, e) in self.entries.iter().enumerate() {
            for ri in e.start..e.start + e.count {
                let r = &self.recs[ri as usize];
                let name = self.name(r);
                let Some(score) = pattern.score(Utf32Str::new(name, &mut chars), &mut matcher) else { continue };
                total += 1;
                let exact = if name.eq_ignore_ascii_case(query) { 1000 } else { 0 };
                let item = std::cmp::Reverse((score + r.kind.bonus() + exact, std::cmp::Reverse(name.len()), std::cmp::Reverse(fi as u32), std::cmp::Reverse(ri)));
                if best.len() < limit {
                    best.push(item);
                } else if best.peek().is_some_and(|worst| item < *worst) {
                    best.pop();
                    best.push(item);
                }
            }
        }
        let mut ranked: Vec<_> = best.into_iter().map(|std::cmp::Reverse(x)| x).collect();
        ranked.sort_by(|a, b| b.cmp(a));
        let mut indices = Vec::new();
        let hits = ranked
            .into_iter()
            .map(|(_, _, std::cmp::Reverse(fi), std::cmp::Reverse(ri))| {
                let r = &self.recs[ri as usize];
                let name = self.name(r);
                indices.clear();
                pattern.indices(Utf32Str::new(name, &mut chars), &mut matcher, &mut indices);
                indices.sort_unstable();
                indices.dedup();
                SymbolHit {
                    name: name.to_string(),
                    kind: r.kind,
                    container: self.container(r).map(str::to_string),
                    path: self.path(&self.entries[fi as usize]).to_string(),
                    line: r.line,
                    indices: indices.clone(),
                }
            })
            .collect();
        (hits, total)
    }

    /// The definitions of the file at `path`, in the order they stand in it. Empty for a file the index doesn't
    /// hold.
    pub fn in_file(&self, path: &str) -> Vec<Def> {
        let Ok(at) = self.find(path) else { return Vec::new() };
        let e = &self.entries[at];
        self.recs[e.start as usize..(e.start + e.count) as usize]
            .iter()
            .map(|r| Def {
                name: self.name(r).to_string(),
                kind: r.kind,
                line: r.line,
                container: self.container(r).map(str::to_string),
            })
            .collect()
    }

    /// Whether the index was built: before that it has nothing to say.
    pub fn is_built(&self) -> bool {
        self.built
    }

    /// Brings the index up to date when the watcher saw a change since the last time, as a search does.
    pub fn bring_up_to_date(&mut self, files: &[&str], changes: impl FnOnce(u64) -> (u64, Option<HashSet<String>>)) {
        let (generation, changed) = changes(self.generation);
        if !self.built || generation != self.generation {
            self.refresh(files, changed.as_ref(), generation);
        }
    }
}

/// The symbol indexes of the workspaces searched lately (at most KEEP_INDEXES).
#[derive(Default)]
pub struct SymbolIndexes {
    indexes: Mutex<Vec<(String, Arc<Mutex<SymbolIndex>>)>>,
}

impl SymbolIndexes {
    /// The workspace's index, created empty if needed, and marked as the most recently used.
    pub fn get(&self, workspace_id: &str, root: &Path) -> Arc<Mutex<SymbolIndex>> {
        let mut list = self.indexes.lock().unwrap_or_else(|e| e.into_inner());
        let found = list.iter().position(|(id, _)| id == workspace_id);
        let entry = match found {
            Some(i) => list.remove(i),
            None => (workspace_id.to_string(), Arc::new(Mutex::new(SymbolIndex::new(root)))),
        };
        let index = entry.1.clone();
        list.insert(0, entry);
        list.truncate(KEEP_INDEXES);
        index
    }

    #[cfg(test)]
    pub fn has(&self, workspace_id: &str) -> bool {
        self.indexes.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|(id, _)| id == workspace_id)
    }

    pub fn remove(&self, workspace_id: &str) {
        self.indexes.lock().unwrap_or_else(|e| e.into_inner()).retain(|(id, _)| id != workspace_id);
    }
}

/// Brings the index up to date (if the watcher saw changes) and searches it.
pub fn search(index: &Mutex<SymbolIndex>, files: &[&str], changes: impl FnOnce(u64) -> (u64, Option<HashSet<String>>), query: &str, limit: usize) -> SymbolResults {
    let mut index = index.lock().unwrap_or_else(|e| e.into_inner());
    let t = Instant::now();
    let (generation, changed) = changes(index.generation);
    if !index.built || generation != index.generation {
        index.refresh(files, changed.as_ref(), generation);
    }
    let refresh_ms = t.elapsed().as_millis() as u64;
    let t = Instant::now();
    let (hits, total) = index.search(query, limit);
    SymbolResults {
        hits,
        total,
        symbols: index.symbols(),
        files: index.files(),
        bytes: index.bytes(),
        refresh_ms,
        search_ms: t.elapsed().as_millis() as u64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(defs: &[Def]) -> Vec<String> {
        defs.iter()
            .map(|d| match &d.container {
                Some(c) => format!("{:?} {c}.{}:{}", d.kind, d.name, d.line),
                None => format!("{:?} {}:{}", d.kind, d.name, d.line),
            })
            .collect()
    }

    #[test]
    fn python_classes_methods_and_functions_without_locals_or_docstrings() {
        let src = "import os\n\nclass StockLevel(models.Model):\n    \"\"\"\n    class NotReal:\n    def nope(self):\n    \"\"\"\n    quantity = 1\n\n    def save(self):\n        def helper():\n            pass\n        return 1\n\n    @property\n    async def total(self):\n        pass\n\n    class Meta:\n        ordering = [\"id\"]\n\n\ndef apply_movement(m):\n    pass\n";
        assert_eq!(
            names(&extract("orders/stock.py", src)),
            [
                "Class StockLevel:3",
                "Method StockLevel.save:10",
                "Method StockLevel.total:16",
                "Class StockLevel.Meta:19",
                "Function apply_movement:23"
            ]
        );
    }

    #[test]
    fn javascript_typescript_and_vue() {
        let ts = "export default class Store {\n  private items = [];\n  async save(item: Item): Promise<void> {\n    if (x) {\n    }\n  }\n  get count(): number {\n    return 1;\n  }\n}\nexport interface Item { id: number }\nexport type Id = string;\nexport const enum Color { Red }\nexport async function load(id) {\n}\nconst format = (x) => x;\nconst value = 3;\nfunction* walk() {}\n";
        assert_eq!(
            names(&extract("src/store.ts", ts)),
            [
                "Class Store:1",
                "Method Store.save:3",
                "Method Store.count:7",
                "Interface Item:11",
                "Type Id:12",
                "Enum Color:13",
                "Function load:14",
                "Function format:16",
                "Function walk:18"
            ]
        );
        let vue = "<template>\n  <div>{{ total }}</div>\n</template>\n\n<script>\nexport default {\n  methods: {\n    refresh() {\n      this.load();\n    },\n  },\n};\n</script>\n";
        assert_eq!(names(&extract("src/components/StockTable.vue", vue)), ["Component StockTable:1", "Method StockTable.refresh:8"]);
    }

    #[test]
    fn rust_go_ruby_and_class_languages() {
        let rs = "pub struct Hub {\n}\nimpl<T: Clone> Hub {\n    pub fn new() -> Self {\n    }\n}\nimpl Drop for Hub {\n    fn drop(&mut self) {}\n}\npub(crate) async fn run() {}\nenum Mode { A }\ntrait Search {\n    fn find(&self);\n}\n";
        assert_eq!(
            names(&extract("src/hub.rs", rs)),
            ["Struct Hub:1", "Method Hub.new:4", "Method Hub.drop:8", "Function run:10", "Enum Mode:11", "Trait Search:12", "Method Search.find:13"]
        );
        let go = "package store\n\ntype Store struct {\n}\n\nfunc (s *Store) Save(x int) error {\n}\n\nfunc New() *Store {\n}\n\ntype (\n\tID string\n\tRepo interface {\n\t}\n)\n";
        assert_eq!(
            names(&extract("store.go", go)),
            ["Struct Store:3", "Method Store.Save:6", "Function New:9", "Type ID:13", "Interface Repo:14"]
        );
        let rb = "module Billing\n  class Invoice < Base\n    def total\n    end\n    def self.build!\n    end\n  end\nend\n";
        assert_eq!(names(&extract("app/invoice.rb", rb)), ["Module Billing:1", "Class Billing.Invoice:2", "Method Invoice.total:3", "Method Invoice.build!:5"]);
        let kt = "data class Order(val id: Int)\nenum class Status { OPEN }\nfun String.slug(): String = this\ninterface Repo {}\n";
        assert_eq!(names(&extract("Order.kt", kt)), ["Class Order:1", "Enum Status:2", "Function slug:3", "Interface Repo:4"]);
    }

    #[test]
    fn finds_by_fuzzy_name_with_types_first_and_refreshes_only_changed_files() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("orders")).unwrap();
        std::fs::write(root.join("orders/stock.py"), "class StockLevel:\n    def stock_level_total(self):\n        pass\n\ndef stock_lever():\n    pass\n").unwrap();
        std::fs::write(root.join("orders/views.py"), "class StockLevelView:\n    pass\n").unwrap();
        std::fs::write(root.join("README.md"), "# StockLevel\n").unwrap();
        let files = ["README.md", "orders/stock.py", "orders/views.py"];
        let mut index = SymbolIndex::new(root);
        index.refresh(&files, None, 1);
        assert_eq!(index.symbols(), 4);
        assert_eq!(index.files(), 2, "only code files are indexed");

        // the exact name first, then the class whose name starts the same, before functions
        let (hits, total) = index.search("StockLevel", 10);
        assert_eq!(hits[0].name, "StockLevel");
        assert_eq!(hits[0].kind, Kind::Class);
        assert_eq!(hits[1].name, "StockLevelView");
        // case doesn't matter: the method stock_level_total matches too (stock_lever has no final l)
        assert_eq!(total, 3);
        assert_eq!(index.search("stocklevel", 10).0[0].name, "StockLevel");
        // camel-case initials and letters in order
        let (hits, _) = index.search("slv", 10);
        assert_eq!(hits[0].name, "StockLevelView");
        assert_eq!(hits[0].indices, [0, 5, 10]);
        // a method carries its class
        let (hits, _) = index.search("stock_level_total", 10);
        assert_eq!((hits[0].container.as_deref(), hits[0].line), (Some("StockLevel"), 2));

        // one file changes: only it is read again
        std::fs::write(root.join("orders/views.py"), "class StockLevelView:\n    pass\n\nclass StockReport:\n    pass\n").unwrap();
        let changed: HashSet<String> = ["orders/views.py".to_string()].into();
        index.refresh(&files, Some(&changed), 2);
        assert_eq!(index.search("StockReport", 5).0[0].line, 4);
        // a file removed from the list drops out
        index.refresh(&["orders/stock.py"], Some(&["orders/views.py".to_string()].into()), 3);
        assert!(index.search("StockReport", 5).0.is_empty());
    }

    #[test]
    fn a_definition_takes_sixteen_bytes() {
        assert_eq!(std::mem::size_of::<Rec>(), 16);
        assert_eq!(std::mem::size_of::<Entry>(), 32);
    }

    /// Lengths are kept in one byte: a name (or a class name) longer than 255 bytes is left out instead of wrapping
    /// around, which would cut a name in the wrong place.
    #[test]
    fn names_too_long_for_a_byte_are_left_out_not_cut() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let long = "L".repeat(300);
        let just = "J".repeat(255);
        std::fs::write(
            root.join("gen.py"),
            format!("class {long}:\n    def inside_long(self):\n        pass\n\nclass {just}:\n    def inside_just(self):\n        pass\n\ndef normal():\n    pass\n"),
        )
        .unwrap();
        let mut index = SymbolIndex::new(root);
        index.refresh(&["gen.py"], None, 1);
        assert!(index.search("LLLL", 5).0.is_empty(), "a 300-byte class name is not indexed");
        assert!(index.search("inside_long", 5).0.is_empty(), "nor its methods, whose class name is too long to keep");
        let inside = index.search("inside_just", 5).0;
        assert_eq!(inside[0].container.as_deref(), Some(just.as_str()), "255 bytes still fit, whole");
        assert_eq!(index.search(&just, 5).0[0].name, just);
        assert_eq!(index.search("normal", 5).0[0].name, "normal");
    }

    #[test]
    fn dead_space_from_edits_is_compacted_without_changing_results() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(root.join("a.py"), "class Alpha:\n    def run(self):\n        pass\n").unwrap();
        std::fs::write(root.join("b.py"), "def beta():\n    pass\n").unwrap();
        let files = ["a.py", "b.py"];
        let mut index = SymbolIndex::new(root);
        index.refresh(&files, None, 1);
        // Claude edits a.py a few times: each edit leaves the old definitions behind as dead space
        for n in 0..5 {
            std::fs::write(root.join("a.py"), format!("class Alpha:\n    def run(self):\n        pass\n    def step{n}(self):\n        pass\n")).unwrap();
            index.refresh(&files, Some(&["a.py".to_string()].into()), 2 + n);
        }
        assert!(index.recs.len() > index.live, "dead definitions are still in the list");
        let before: Vec<_> = ["run", "step4", "beta", "Alpha"].iter().map(|q| index.search(q, 5).0).collect();
        index.compact();
        assert_eq!(index.recs.len(), index.live);
        let after: Vec<_> = ["run", "step4", "beta", "Alpha"].iter().map(|q| index.search(q, 5).0).collect();
        assert_eq!(before, after);
        assert_eq!(after[0][0].container.as_deref(), Some("Alpha"));
        assert!(index.search("step3", 5).0.iter().all(|h| h.name != "step3"), "an old version's method is gone");
    }

    #[test]
    fn keeps_only_the_latest_few_workspaces() {
        let indexes = SymbolIndexes::default();
        for id in ["a", "b", "c", "d"] {
            indexes.get(id, Path::new("/tmp"));
        }
        assert!(!indexes.has("a"), "the least recently searched one went");
        assert!(indexes.has("d"));
        indexes.get("b", Path::new("/tmp"));
        indexes.get("e", Path::new("/tmp"));
        assert!(indexes.has("b") && !indexes.has("c"));
    }

    /// Run on a big repo: ENKIDU_BIG_REPO=<path> cargo test --release symbol_index_speed -- --ignored --nocapture
    #[test]
    #[ignore]
    fn symbol_index_speed() {
        let root = PathBuf::from(std::env::var("ENKIDU_BIG_REPO").expect("set ENKIDU_BIG_REPO"));
        let tracked = crate::git::tracked_files(&root).unwrap();
        let files: Vec<&str> = tracked.iter().map(String::as_str).collect();
        // the process's own memory, as Activity Monitor counts it (footprint), before and after
        let footprint = || {
            let out = std::process::Command::new("footprint").args(["-p", &std::process::id().to_string()]).output().ok();
            out.and_then(|o| {
                let text = String::from_utf8_lossy(&o.stdout).to_string();
                let line = text.lines().find(|l| l.contains("Footprint:"))?.to_string();
                let mut parts = line.split_whitespace().skip_while(|w| *w != "Footprint:").skip(1);
                let value: f64 = parts.next()?.parse().ok()?;
                let unit = parts.next().unwrap_or("MB");
                Some(match unit { "GB" => value * 1024.0, "KB" => value / 1024.0, _ => value })
            })
        };
        let before = footprint();
        let mut index = SymbolIndex::new(&root);
        let t = Instant::now();
        index.refresh(&files, None, 1);
        let build = t.elapsed();
        let after = footprint();
        println!("ENKIDU_SYMBOLS memory: {before:?} MB before, {after:?} MB after building");
        let t = Instant::now();
        index.refresh(&files, None, 2);
        let check = t.elapsed();
        let t = Instant::now();
        let one: HashSet<String> = [files[files.len() / 2].to_string()].into();
        index.refresh(&files, Some(&one), 3);
        let single = t.elapsed();
        println!(
            "ENKIDU_SYMBOLS build {:?}, recheck all (nothing changed) {:?}, one changed file {:?}: {} symbols in {} files, {:.1} MB",
            build,
            check,
            single,
            index.symbols(),
            index.files(),
            index.bytes() as f64 / 1048576.0
        );
        let searches: Vec<&str> = vec!["QuerySet", "qs", "get_queryset", "Admin", "useState", "zzqx"];
        let report = |index: &SymbolIndex| {
            for q in &searches {
                let t = Instant::now();
                let (hits, total) = index.search(q, 50);
                println!(
                    "ENKIDU_SYMBOLS \"{q}\": {:?}, {total} matches, top: {}",
                    t.elapsed(),
                    hits.iter().take(3).map(|h| format!("{} ({:?}, {}:{})", h.name, h.kind, h.path.rsplit('/').next().unwrap_or(""), h.line)).collect::<Vec<_>>().join(", ")
                );
            }
        };
        report(&index);
    }
}
