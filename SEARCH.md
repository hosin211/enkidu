# Search: how it stays fast on a huge repo without eating memory

This file is what the code's comments and `DESIGN.md` mean by `SEARCH.md`. Its second part holds what came later:
the filter by kind of file, the submodules, the preview under the rows of Find in files, and going to a definition.

Enkidu has one search popup with three tabs, like PyCharm's Search Everywhere:

| Tab | Finds | Keys | How |
|---|---|---|---|
| Files | file names | ⇧⇧ (last tab), ⇧⌘O, ⌘P | git's own index, filtered in the window |
| Symbols | classes, functions, methods | ⌘O, ⌥⌘O | a small definitions index, built on first use |
| Text | anything inside files | ⇧⌘F | no index: a parallel scan, streamed |

⇥ moves between tabs, ↑↓ and ↩ open a result read-only at its line. In Text, ⌥⌘C / ⌥⌘W / ⌥⌘R toggle match case,
whole words and regular expressions (the same keys as VS Code).

The two kinds of search need different answers. Finding text can be done by reading the files fast enough, so it
keeps nothing. Finding a definition by a fuzzy name can't (no file read finds "slv" in StockLevelView), so it keeps a
small index of definitions only.

## What the established tools do

- **VS Code** runs a bundled ripgrep for Find in Files: no index at all
  (its issue tracker refers to "VS Code's ripgrep directory"; a JetBrains-style Find in Files add-on for it also uses
  ripgrep and caps results at 1,000).
- **ripgrep** (Andrew Gallant, burntsushi.net/ripgrep): its speed comes from Rust's regex engine, which pulls literal
  strings out of every pattern and finds them with SIMD (memchr, Teddy), plus searching many files in parallel and
  skipping ignored and binary files. "Every consumer of that library gets all these performance optimizations."
- **Indexes for text** pay off at a different scale. Google Code Search (Russ Cox, swtch.com/~rsc/regexp/regexp4.html)
  used a trigram index "around 20% of the size of the files being indexed" (Linux 3.1.3: 420 MB of source, a 77 MB
  index) that has to be rebuilt as files change. Sourcegraph's Zoekt, the same idea for 19,000 repositories (166 GB),
  used 22 GB of RAM before a 5x optimization (sourcegraph.com/blog/zoekt-memory-optimizations-for-sourcegraph-cloud).
  That is a server for thousands of repos, not a tool for one repo that Claude edits every minute.
- **Symbols from language servers** (Zed, VS Code) are heavy and lazy: Zed users report project symbols that appear
  only after opening the file, stale caches, and slow starts, and ask for an up-front index instead, naming Sublime
  Text's as the reason they stay (github.com/zed-industries/zed/issues/13307).
- **Sublime Text** indexes "every symbol and its location" with low-priority background workers that apply its
  regex-based syntax rules; "even for large projects, indexing should take only a few seconds"
  (sublimetext.com/blog/articles/file-indexing).
- **GitHub's code navigation** is "search-based": each file's definitions are extracted (tree-sitter tag queries),
  with fully qualified names, and looked up by name (github.com/github/code-navigation).
- **Fuzzy ranking**: nucleo (the matcher inside the Helix editor) uses fzf's scoring, is faster than fzf and about six
  times faster than the older Rust option (github.com/helix-editor/nucleo).

## What Enkidu does

**Text (`crates/enkidu-core/src/search.rs`): no index.** It reads the files git knows about (tracked, plus new files that aren't
ignored) with Rust's regex engine on 8 threads, in path order, and streams each finished stretch of results to the
popup, so the first ones show in a few milliseconds while the rest are read. A newer keystroke cancels the running
search. Files over 1 MB (bundles, data dumps) and binary files are skipped and counted. The popup lists the first
1,000 matching lines; the scan goes on without collecting lines, only noting which files match, so the total is exact.
Typing more of the same text then only reads the files the last search found, as long as the file watcher saw no
change in between. Memory: one read buffer per thread while it runs, nothing kept after.

More threads don't help: past about 8, the file system is the limit and threads contend in the kernel (on the
42,000-file repo: 4 threads 1.0 s, 8 threads 0.86 s, 10 threads 1.17 s, 16 threads 1.28 s). At that point the scan
reads about 600 MB/s, the same rate ripgrep reached on the same files (it read fewer: it skips hidden folders).

**Symbols (`crates/enkidu-core/src/symbols.rs`): a definitions-only index, Sublime-style.** A one-pass line scanner per language
(Python, JavaScript/TypeScript and Vue, Rust, Go, Ruby; classes and functions in Java, Kotlin, C#, Swift, PHP, Scala,
Dart) finds definitions with their kind, line and container (a method knows its class). No parser, no language
server. The index is built in the background when the popup first opens, on 8 threads, and stored as four flat lists
(paths, names, definitions, files) sorted by path: 16 bytes per definition plus its name, a 32-byte record per file,
and a handful of allocations in all (a map of strings per file used twice the memory). The file watcher counts
changes and remembers which files changed, so later searches re-read only those; a changed file's old definitions
become dead space that is compacted once there is enough of it. Ranking is nucleo's (fzf scoring), case-insensitive
like PyCharm, with an exact name first and types ahead of functions ahead of methods. Indexes are kept for the 3
workspaces searched last.

**Files: git's index.** `git ls-files --cached` (cached until git's index file changes), plus new files from the
watcher's list of changes, minus files deleted but not committed. The folders are never walked twice: `git status`,
which the watcher runs anyway, already walked them.

## Measured

Two throwaway repos built from real code. "Python" is about the shape of a big Django
monolith; "Python and JS" adds 25,000 files of JavaScript and TypeScript packages, most of them bundles a real repo
wouldn't track. Release build, files already in the OS cache, one search at a time.

| | Python: 13,580 files (12,472 .py) | Python and JS: 41,840 files (500 MB) |
|---|---|---|
| Text, first results | 3 to 12 ms | 4 to 11 ms |
| Text, whole scan | 186 to 206 ms | 840 to 920 ms |
| Text, typing on ("Query", then "QuerySet") | 8 ms, same 52 files as a full scan | 17 ms, same 52 files |
| Symbols, first build (once, in the background) | 0.36 to 0.43 s, 251,578 definitions | 1.07 to 1.18 s, 417,436 definitions |
| Symbols, each keystroke | 6 to 8 ms | 9 to 13 ms |
| Symbols, after Claude edits a file | 22 to 35 µs for that file | 24 to 44 µs |
| Symbols, recheck everything (when the watcher lost track) | 11 ms | 33 to 39 ms |
| Symbols, index size | 8.7 MB | 17.1 MB |
| Process memory right after the first build | +19 MB | +23 MB |
| File list from git | 41 ms, then 5 µs cached | 44 to 52 ms, then 7 to 13 µs |

The process grows more than the index's own size because macOS's allocator keeps some of what the build used for
reuse; it is an upper bound, and searches don't add to it. For comparison, a Python language server on a large Django
project typically uses hundreds of MB.

How these were measured: `ENKIDU_BIG_REPO=<repo> cargo test --release --locked -p enkidu-core --lib -- --ignored --nocapture --test-threads=1`
runs `text_search_speed`, `symbol_index_speed` and `list_files_speed`.

## Limits, and what would come next

- The symbol scanner reads lines, not syntax. It finds the definitions that start a line with their keyword (which
  covers the vast majority in Python and in formatted JS/TS), skips nested helper functions and docstrings, but misses
  methods in Java/C#/Swift (no keyword) and names created at runtime, and can be fooled by unusual formatting. The
  precise upgrade is tree-sitter tag queries per language, what GitHub uses, at the cost of parse time and larger
  grammar packages.
- Finding usages (references) isn't covered; Find in files with Whole words is the practical stand-in.
- If a repo ever makes a text scan take more than a second or two (several GB of tracked text), the next step is an
  on-disk trigram index, memory-mapped so the OS pages it in and out (the Zoekt design), updated per changed file
  from the watcher.

## What came later

**Which files are read.** The files git tracks, the new ones it doesn't ignore (unversioned: the list of changes is
asked with `--untracked-files=all`, so a new file in a new folder is there), and the same two kinds inside each
submodule that is checked out (`FileLists::in_submodules`: what a submodule tracks is asked again when its own index
file changes, its new files when the watcher's change counter moved; a workspace without `.gitmodules` costs one look
at the folder). Never what git ignores: one project measured here ignores 239,778 files, almost all of them
`node_modules`, uploaded media and Python caches.

**A kind of file** (`TextQuery.kind`, the button beside the field): the list is cut to the paths that end with that
kind before the scan, so a filtered scan reads fewer files. Typing more of the same text still reads only the files
the last scan found, inside the same kind.

**Find in files' rows and its preview** (`crates/enkidu-app/src/search.rs`, `crates/enkidu-model/src/search.rs`): one row for
each line, the line that defines the searched name first. Under the rows, the file around the picked line: a second
diff view, given another file after the row was the picked one for 80 ms. Its file is read, and colored, off the
drawing thread; an answer for a row that is no longer the picked one is dropped. Coloring costs 60 to 80 µs a line of
Python and is kept for the 512 texts colored last.

**Going to a definition: the language's server, and nothing of our own** (`crates/enkidu-core/src/lsp.rs`,
`crates/enkidu-core/src/goto.rs`; phases 12 and 13). Such things are already invented, and the right tool is used:
the app has no rules of its own for finding a definition.

For a Python file the core asks a language server where the name is defined: `ty server`, the program
`scripts/fetch-ty.sh` fetches and `scripts/install.sh` puts beside the app's own program. By hand, without a
new build: a file `python-server.txt` in the app's data folder holding another command line (`pyrefly lsp`), or
`off` for none; there is no field for it in Settings. (`ENKIDU_PYTHON_SERVER` does the same for tests.) A language
without a server in the app has no links.

One server runs for each workspace asked lately, three at most. It is started on a thread of its own when the
workspace's Python code comes on screen, or at the first question, and stopped after ten minutes without one.
Each question gives the server the file's text as it is on disk, asks, and gives the file back; before it, the
server is told which files the watcher saw change (ty answers from what it read before until it is told: checked
live). It has 150 ms. Nobody waits longer: what is written to a server goes through a thread of its own, one
question is asked at a time, and while the server still owes the answer to a question that was given up on it is
not asked at all. Then the core says "later", and the app asks again every 200 ms while the pointer rests on the
name (50 times at most), so the name becomes a link when the server is there. A server starts at work on the
question that made it start: on a workspace of 40,000 files ty read for 1.6 s then, and answered in 1.7 ms in the
middle and 8.9 ms at worst after.

What it names is where the click goes: one place, or a list of several. A place outside the workspace's folder (a
library's, Python's own) is dropped, and a name with only such places is no link. The rows of the list say what
stands on each line (a method, its class) from the definitions list of the Symbols tab above, which is still the
app's own scanner: moving that list to the language server too is agreed and not built.

The server is started in the app's data folder, without the app's own variables, and is only asked to read: on
two real projects git saw the same files before and after 2,000 questions, and ty opened no network connection.
If the app is killed, the server ends by itself when its input closes.

Measured on 14 Nov 2026 on two real Django projects: ty says hello in 28 ms (0.8 s the very
first time), answers in 0.1 ms in the middle and under 5 ms 95 times of 100, and holds 11 MB at its start and 171
to 192 MB once every Python file of the project was asked about.

Measured on 14 Nov 2026, release build, on a throwaway repo in the shape of a Django project with a Vue front end
(9,758 files, 78 MB of Python):

| | |
|---|---|
| Symbols, first build | 254 ms, 101,736 definitions in 9,137 files |
| Symbols, recheck everything | 9.7 ms |
| Symbols, after one file changed | 45 µs |
| Symbols, index size | 4.6 MB |
| Symbols, a fuzzy question | 1.9 to 3.8 ms |
| Text, whole scan | 153 to 298 ms, first results after 4 to 7 ms |
| File list from git | 72 ms, then 13 µs |
