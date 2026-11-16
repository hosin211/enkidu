//! `enkidu`, the app's command: what Claude and you use in a terminal to read and correct the Work index. Its code is
//! in `src/cli.rs`. It reads `ENKIDU_…` only and finds its data folder as the app does, so it can only ever talk to
//! Enkidu (items 1.3 and 1.8).

#[rustfmt::skip]
#[allow(clippy::style, clippy::complexity)]
mod cli;

fn main() {
    cli::main()
}
