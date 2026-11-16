//! Test helpers: throwaway git repos made for each test, the fake Claude, the fake `gh` and `twg`, made-up screenshots,
//! and the runner that draws pictures offscreen. Tests never use the real Claude or a real repo.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Runs git in `dir` with no global or system config, so the test sees only what it set up. Panics when git fails.
pub fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?} in {}: {}", dir.display(), String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A throwaway repository `parent/name` on the branch `main` with one commit. Returns its path as git says it
/// (temporary folders on the Mac sit behind a link, `/var` to `/private/var`).
pub fn repo(parent: &Path, name: &str) -> PathBuf {
    let dir = parent.join(name);
    std::fs::create_dir_all(&dir).expect("the repo's folder");
    git(&dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("README.md"), format!("# {name}\n")).expect("a file");
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "First"]);
    PathBuf::from(git(&dir, &["rev-parse", "--show-toplevel"]))
}

/// A throwaway project with an origin: `parent/<name>-origin.git` (a bare repository with `main` and `feature/x`),
/// cloned to `parent/<name>`, where only `main` is local and origin's HEAD is `main`. Returns the clone's path.
pub fn repo_with_origin(parent: &Path, name: &str) -> PathBuf {
    let seed = repo(parent, &format!("{name}-seed"));
    git(&seed, &["branch", "feature/x"]);
    let origin = parent.join(format!("{name}-origin.git"));
    git(parent, &["clone", "-q", "--bare", &seed.to_string_lossy(), &origin.to_string_lossy()]);
    let clone = parent.join(name);
    git(parent, &["clone", "-q", &origin.to_string_lossy(), &clone.to_string_lossy()]);
    PathBuf::from(git(&clone, &["rev-parse", "--show-toplevel"]))
}

/// The sample project, `parent/shop`: origin has `main`, `development`, `feature/remote-only` and `feature/change-245`,
/// all at one commit with `src/app.py`; the clone has only `main` locally, and an untracked `.env` and
/// `.claude/settings.local.json` (both ignored) for new workspaces to copy.
pub fn shop(parent: &Path) -> PathBuf {
    let seed = parent.join("seed");
    std::fs::create_dir_all(seed.join("src")).expect("the seed");
    git(&seed, &["init", "-q", "-b", "main"]);
    std::fs::write(seed.join("src/app.py"), "def total(items):\n    return sum(items)\n").unwrap();
    std::fs::write(seed.join(".gitignore"), ".env\n.claude/settings.local.json\n").unwrap();
    git(&seed, &["add", "."]);
    git(&seed, &["commit", "-q", "-m", "first"]);
    for branch in ["development", "feature/remote-only", "feature/change-245"] {
        git(&seed, &["branch", branch]);
    }
    let origin = parent.join("origin.git");
    git(parent, &["init", "-q", "--bare", "-b", "main", &origin.to_string_lossy()]);
    git(&seed, &["remote", "add", "origin", &origin.to_string_lossy()]);
    git(&seed, &["push", "-q", "origin", "main", "development", "feature/remote-only", "feature/change-245"]);
    let main = parent.join("shop");
    git(parent, &["clone", "-q", &origin.to_string_lossy(), &main.to_string_lossy()]);
    std::fs::write(main.join(".env"), "SECRET=e2e\n").unwrap();
    std::fs::create_dir_all(main.join(".claude")).unwrap();
    std::fs::write(main.join(".claude/settings.local.json"), "{}\n").unwrap();
    PathBuf::from(git(&main, &["rev-parse", "--show-toplevel"]))
}

/// What the recorded GitHub answers are about (`enkidu-core/tests/fixtures/work`): the repository [`shop_on_github`]'s
/// origin says it is.
pub const GITHUB_ORIGIN: &str = "https://github.com/example-org/shop.git";

/// [`shop`] with an origin that looks like GitHub: the Work index reads its PRs (through the fake `gh`, never the
/// network), while git itself still fetches from the local test origin.
pub fn shop_on_github(parent: &Path) -> PathBuf {
    let main = shop(parent);
    let origin = parent.join("origin.git");
    git(&main, &["remote", "set-url", "origin", GITHUB_ORIGIN]);
    git(&main, &["config", &format!("url.{}.insteadOf", origin.to_string_lossy()), GITHUB_ORIGIN]);
    main
}

/// The fake `gh` and the fake `twg` (`enkidu-core/tests/fakes`, run with `node`): they answer from the recorded answers
/// beside them, never touch the network, and log every call. Each reads its mode from the file its environment names.
pub fn fake_gh() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../enkidu-core/tests/fakes/fake-gh.mjs")
        .canonicalize()
        .expect("the fake gh")
}

pub fn fake_twg() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../enkidu-core/tests/fakes/fake-twg.mjs")
        .canonicalize()
        .expect("the fake twg")
}

/// The fake language server (`enkidu-core/tests/fakes/fake-lsp.mjs`, run with `node`): it speaks the protocol, reads no
/// project, logs what it is told, and answers "where is this defined" with what a file the test names holds.
pub fn fake_lsp() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../enkidu-core/tests/fakes/fake-lsp.mjs")
        .canonicalize()
        .expect("the fake language server")
}

/// The sample Work index (kept beside this as `fixtures/work/sample.ts`): invented tickets and PRs of a small shop,
/// with a bit of everything that can wait on you. It is what the tests of the Work page stand on, as JSON in the shape
/// the core sends (`enkidu_core::work::model::Snapshot`), made once with `node --experimental-strip-types` at
/// [`WORK_SAMPLE_NOW`] for the home folder [`WORK_SAMPLE_HOME`] (the command is in `fixtures/work/README.md`).
pub const WORK_SAMPLE: &str = include_str!("../fixtures/work/sample.json");
/// A ticket's description with every kind of block Jira writes (a link first, a person, a status, headings, lists
/// inside lists, a numbered list from 9, a checklist, code, a quote, two panels, a table, a file, a fold, a rule,
/// marks), as Jira's document format. Ours, for the pictures and the screen tests of the card.
pub const WORK_RICH_DOC: &str = include_str!("../fixtures/work/rich-doc.json");
/// The moment the sample was made for: 8 Nov 2026, 10:00 UTC, in milliseconds.
pub const WORK_SAMPLE_NOW: i64 = 1_794_132_000_000;
/// The home folder in the sample's paths.
pub const WORK_SAMPLE_HOME: &str = "/Users/demo";

/// A made-up screenshot, as a PNG: a page with a colored top band, a side column and rows of bars, in the tint `seed`
/// picks. For tests and pictures of Claude's reports; nothing real is shown in it.
pub fn screenshot_png(width: u32, height: u32, seed: u32) -> Vec<u8> {
    png_of(made_up_page(width, height, seed))
}

/// The same page with fine grain over a quarter of it, so it weighs what a real screenshot full of words does (about
/// 2 MB at 2,880 by 1,800, where the plain one is 0.1 MB): for timing how long one takes to read.
pub fn busy_screenshot_png(width: u32, height: u32, seed: u32) -> Vec<u8> {
    let mut page = made_up_page(width, height, seed);
    let mut next = 0x9e37_79b9_u32 ^ seed.wrapping_mul(0x85eb_ca6b).wrapping_add(1);
    for (x, y, pixel) in page.enumerate_pixels_mut() {
        // the middle of the page, where the words would be
        if x * 100 >= width * 22 && x * 100 < width * 72 && y * 100 >= height * 22 && y * 100 < height * 72 {
            next ^= next << 13;
            next ^= next >> 17;
            next ^= next << 5;
            let grain = (next % 48) as i16 - 24;
            for part in pixel.0.iter_mut() {
                *part = (*part as i16 + grain).clamp(0, 255) as u8;
            }
        }
    }
    png_of(page)
}

fn png_of(page: image::RgbImage) -> Vec<u8> {
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgb8(page)
        .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
        .expect("a PNG of the made-up screenshot");
    bytes
}

fn made_up_page(width: u32, height: u32, seed: u32) -> image::RgbImage {
    const TINTS: [[u8; 3]; 4] = [[79, 125, 255], [61, 214, 140], [255, 181, 71], [232, 132, 95]];
    let tint = TINTS[seed as usize % TINTS.len()];
    let mut page = image::RgbImage::from_pixel(width, height, image::Rgb([247, 248, 250]));
    // a part of the page, in thousandths of its width and height
    let mut fill = |left: u32, top: u32, right: u32, bottom: u32, color: [u8; 3]| {
        let at = |part: u32, whole: u32| ((whole as u64 * part as u64 / 1000) as u32).min(whole);
        for y in at(top, height)..at(bottom, height) {
            for x in at(left, width)..at(right, width) {
                page.put_pixel(x, y, image::Rgb(color));
            }
        }
    };
    // the top band, the side column and its entries
    fill(0, 0, 1000, 80, tint);
    fill(0, 80, 180, 1000, [234, 236, 240]);
    for row in 0..6 {
        fill(20, 120 + row * 60, 150, 150 + row * 60, [208, 212, 220]);
    }
    // a heading, rows of words, a button
    fill(220, 130, 620, 170, [40, 44, 52]);
    for row in 0..9 {
        let wide = 620 + (row * 37 + seed * 53) % 300;
        fill(220, 220 + row * 70, wide, 250 + row * 70, [196, 200, 208]);
    }
    fill(820, 120, 960, 180, tint);
    page
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_repo_with_an_origin_has_its_branches_there() {
        let parent = tempfile::tempdir().unwrap();
        let dir = repo_with_origin(parent.path(), "shop");
        assert_eq!(git(&dir, &["branch", "--format=%(refname:short)"]), "main");
        let remote = git(&dir, &["branch", "-r", "--format=%(refname:short)"]);
        assert!(remote.contains("origin/feature/x") && remote.contains("origin/main"), "{remote}");
    }

    #[test]
    fn a_throwaway_repo_has_one_commit_on_main() {
        let parent = tempfile::tempdir().unwrap();
        let dir = repo(parent.path(), "shop");
        assert!(dir.ends_with("shop"));
        assert_eq!(git(&dir, &["rev-parse", "--abbrev-ref", "HEAD"]), "main");
        assert_eq!(git(&dir, &["rev-list", "--count", "HEAD"]), "1");
    }
}
