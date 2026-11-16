//! Reading git and GitHub facts out of text: ticket keys, PR links, and what a `git`/`gh` command did (from Claude
//! Code's own `gitOperation` record when it has one, else from the command and its output).

use regex::Regex;
use serde_json::Value;
use std::sync::LazyLock;

static KEY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b([A-Z][A-Z0-9]{1,9})-([1-9][0-9]{0,6})\b").unwrap());
static PR_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"https://github\.com/([A-Za-z0-9_.-]+)/([A-Za-z0-9_.-]+)/pull/([0-9]+)\b").unwrap());
static COMMIT_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\[([^\s\]]+)(?: \([^)]*\))? ([0-9a-f]{7,40})\]").unwrap());
static PUSH_REF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^\s*[*+!= -]?\s*(?:\[[^\]]+\]|[0-9a-f]+\.\.\.?[0-9a-f]+)\s+\S+\s+->\s+(\S+)").unwrap());
static PUSH_HINT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Create a pull request for '([^']+)' on GitHub").unwrap());

/// Words shaped like ticket keys that never are (UTF-8, SHA-256, ISO-8601…).
const NOT_KEYS: [&str; 16] =
    ["UTF", "SHA", "ISO", "RFC", "CVE", "HTTP", "TLS", "SSL", "UUID", "ECMA", "PEP", "GPT", "AES", "RSA", "HTML", "CSS"];

/// Ticket keys in a text, in order, each once.
pub fn ticket_keys(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for m in KEY.captures_iter(text) {
        if NOT_KEYS.contains(&&m[1]) {
            continue;
        }
        let key = format!("{}-{}", &m[1], &m[2]);
        if !out.contains(&key) {
            out.push(key);
        }
    }
    out
}

/// The project part of a key: `SH` for `SH-12`.
pub fn key_project(key: &str) -> &str {
    key.split('-').next().unwrap_or(key)
}

/// PRs linked in a text as `owner/repo#N`, in order, each once.
pub fn pr_refs(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for m in PR_URL.captures_iter(text) {
        let r = format!("{}/{}#{}", &m[1], &m[2], &m[3]);
        if !out.contains(&r) {
            out.push(r);
        }
    }
    out
}

/// What git and gh did in one command.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GitOps {
    /// (`owner/repo#N`, or `#N` when the repo is the session's own; what was done: created, edited, commented…)
    pub prs: Vec<(String, String)>,
    pub pushed: Vec<String>,
    /// (sha as git printed it, branch)
    pub commits: Vec<(String, Option<String>)>,
}

impl GitOps {
    pub fn is_empty(&self) -> bool {
        self.prs.is_empty() && self.pushed.is_empty() && self.commits.is_empty()
    }
}

/// Claude Code's own record of a git step (`toolUseResult.gitOperation`):
/// `{push:{branch}, pr:{number,url?,action}, commit:{sha,kind,branch}}`, any of them.
pub fn from_git_operation(op: &Value) -> GitOps {
    let mut ops = GitOps::default();
    if let Some(b) = op.pointer("/push/branch").and_then(Value::as_str).filter(|b| !b.is_empty()) {
        ops.pushed.push(b.to_string());
    }
    if let Some(n) = op.pointer("/pr/number").and_then(Value::as_i64) {
        let action = op.pointer("/pr/action").and_then(Value::as_str).unwrap_or("worked").to_string();
        let target = op
            .pointer("/pr/url")
            .and_then(Value::as_str)
            .and_then(|u| pr_refs(u).into_iter().next())
            .unwrap_or_else(|| format!("#{n}"));
        ops.prs.push((target, action));
    }
    if let Some(sha) = op.pointer("/commit/sha").and_then(Value::as_str).filter(|s| s.len() >= 7) {
        let branch = op.pointer("/commit/branch").and_then(Value::as_str).filter(|b| !b.is_empty()).map(String::from);
        ops.commits.push((sha.to_string(), branch));
    }
    ops
}

/// The commands of a shell line, each as its words: quotes are taken off (so a quoted `'gh pr create'` stays one
/// word), `;`, `&&`, `||`, `|`, `(`, `)`, backticks and new lines separate commands, and a heredoc's text
/// (`cat > body.md <<'EOF' … EOF`) is skipped whole.
fn commands(line: &str) -> Vec<Vec<String>> {
    let chars: Vec<char> = line.chars().collect();
    let mut out: Vec<Vec<String>> = vec![Vec::new()];
    let mut word = String::new();
    let mut quote: Option<char> = None;
    // heredocs whose text starts at the next new line
    let mut heredocs: Vec<String> = Vec::new();
    fn end(word: &mut String, out: &mut [Vec<String>]) {
        if !word.is_empty() {
            out.last_mut().expect("never empty").push(std::mem::take(word));
        }
    }
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match quote {
            Some(q) if c == q => quote = None,
            Some('"') if c == '\\' => {
                i += 1;
                word.extend(chars.get(i));
            }
            Some(_) => word.push(c),
            None => match c {
                '\'' | '"' => quote = Some(c),
                '\\' => {
                    i += 1;
                    word.extend(chars.get(i));
                }
                '<' if chars.get(i + 1) == Some(&'<') => {
                    end(&mut word, &mut out);
                    if chars.get(i + 2) == Some(&'<') {
                        // a here-string: the word after it is this command's text
                        i += 3;
                        continue;
                    }
                    i += 2;
                    if chars.get(i) == Some(&'-') {
                        i += 1;
                    }
                    while chars.get(i).is_some_and(|c| *c == ' ' || *c == '\t') {
                        i += 1;
                    }
                    let mut delim = String::new();
                    while let Some(&d) = chars.get(i) {
                        if d.is_whitespace() || matches!(d, ';' | '&' | '|' | ')') {
                            break;
                        }
                        if d != '\'' && d != '"' {
                            delim.push(d);
                        }
                        i += 1;
                    }
                    if !delim.is_empty() {
                        heredocs.push(delim);
                    }
                    continue;
                }
                '\n' => {
                    end(&mut word, &mut out);
                    out.push(Vec::new());
                    for delim in heredocs.drain(..) {
                        // skip lines up to and with the one that is only the delimiter
                        loop {
                            let start = i + 1;
                            if start >= chars.len() {
                                i = chars.len();
                                break;
                            }
                            let stop = chars[start..].iter().position(|c| *c == '\n').map_or(chars.len(), |p| start + p);
                            let text: String = chars[start..stop].iter().collect();
                            i = stop;
                            if text.trim_start_matches('\t').trim_end() == delim {
                                break;
                            }
                        }
                    }
                }
                ';' | '&' | '|' | '(' | ')' | '`' => {
                    end(&mut word, &mut out);
                    out.push(Vec::new());
                }
                c if c.is_whitespace() => end(&mut word, &mut out),
                c => word.push(c),
            },
        }
        i += 1;
    }
    end(&mut word, &mut out);
    out.retain(|c| !c.is_empty());
    out
}

fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(k, _)| {
        !k.is_empty() && !k.starts_with(|c: char| c.is_ascii_digit()) && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// Does the shell line run `words` as a command (`gh pr create`, `git push`)? `GH_TOKEN=x gh pr create`,
/// `cd x && git -C y push` count; `grep 'gh pr create' notes` and `echo git push` don't.
pub fn runs(line: &str, words: &[&str]) -> bool {
    commands(line).iter().any(|cmd| {
        let mut w = cmd.iter().map(String::as_str).skip_while(|w| is_assignment(w) || matches!(*w, "command" | "env" | "time" | "sudo" | "exec" | "nohup" | "$"));
        let mut want = words.iter();
        let Some(first) = want.next() else { return false };
        if w.next() != Some(*first) {
            return false;
        }
        // git's own options before the subcommand: `git -C dir push`, `git --no-pager commit`
        let mut rest: Vec<&str> = w.collect();
        if *first == "git" {
            let mut i = 0;
            while i < rest.len() && rest[i].starts_with('-') {
                i += if matches!(rest[i], "-C" | "-c") { 2 } else { 1 };
            }
            rest.drain(..i.min(rest.len()));
        }
        want.enumerate().all(|(i, x)| rest.get(i) == Some(x))
    })
}

/// Does this shell line do something the index cares about (make a PR, commit, push)?
pub fn interesting_command(line: &str) -> bool {
    runs(line, &["gh", "pr"]) || runs(line, &["git", "commit"]) || runs(line, &["git", "push"]) || runs(line, &["git", "cherry-pick"])
}

/// Keeps only the parts of Claude Code's `gitOperation` that the command really did. Its record is a guess from the
/// command and its output, and a command that merely quotes `gh pr create` next to a PR link fools it.
pub fn confirmed(ops: GitOps, command: &str) -> GitOps {
    let did = |action: &str| match action {
        "created" => runs(command, &["gh", "pr", "create"]),
        "edited" => runs(command, &["gh", "pr", "edit"]),
        "commented" => runs(command, &["gh", "pr", "comment"]) || runs(command, &["gh", "pr", "review"]),
        "ready" => runs(command, &["gh", "pr", "ready"]),
        "merged" => runs(command, &["gh", "pr", "merge"]),
        "closed" => runs(command, &["gh", "pr", "close"]),
        "reopened" => runs(command, &["gh", "pr", "reopen"]),
        _ => false,
    };
    GitOps {
        prs: ops.prs.into_iter().filter(|(_, action)| did(action)).collect(),
        pushed: if runs(command, &["git", "push"]) { ops.pushed } else { Vec::new() },
        commits: if runs(command, &["git", "commit"]) || runs(command, &["git", "cherry-pick"]) { ops.commits } else { Vec::new() },
    }
}

/// When there is no `gitOperation`: what the command's output says it did.
pub fn from_command_output(command: &str, stdout: &str, stderr: &str) -> GitOps {
    let mut ops = GitOps::default();
    if runs(command, &["gh", "pr", "create"]) {
        for r in pr_refs(stdout) {
            ops.prs.push((r, "created".into()));
        }
    }
    if runs(command, &["git", "commit"]) || runs(command, &["git", "cherry-pick"]) {
        for m in COMMIT_LINE.captures_iter(stdout) {
            ops.commits.push((m[2].to_string(), Some(m[1].to_string()).filter(|b| b != "detached")));
        }
    }
    if runs(command, &["git", "push"]) {
        let both = format!("{stderr}\n{stdout}");
        for m in PUSH_REF.captures_iter(&both).chain(PUSH_HINT.captures_iter(&both)) {
            let b = m[1].trim_start_matches("refs/heads/").to_string();
            if !ops.pushed.contains(&b) {
                ops.pushed.push(b);
            }
        }
    }
    ops
}

/// Branches that many unrelated sessions sit on: a session there isn't working on a PR from it.
pub fn long_lived(branch: &str, base: &str) -> bool {
    matches!(branch, "" | "HEAD" | "main" | "master" | "development" | "develop" | "dev" | "staging" | "production" | "prod" | "trunk")
        || branch.starts_with("release/")
        || (!base.is_empty() && branch == base)
}

/// What the person typed: Claude Code's own notes (reminders, command output) taken out.
pub fn typed_text(content: &Value) -> String {
    let raw = match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    };
    let mut text = raw;
    for tag in ["system-reminder", "local-command-stdout", "local-command-stderr", "bash-stdout", "bash-stderr"] {
        text = drop_tag(&text, tag);
    }
    text
}

fn drop_tag(text: &str, tag: &str) -> String {
    let (open, close) = (format!("<{tag}>"), format!("</{tag}>"));
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(&open) {
        out.push_str(&rest[..start]);
        match rest[start..].find(&close) {
            Some(end) => rest = &rest[start + end + close.len()..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

/// `2026-11-05T14:03:22.123Z` (or with an offset) as milliseconds since 1970.
pub fn parse_time(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || (b[10] != b'T' && b[10] != b' ') || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let num = |from: usize, to: usize| s.get(from..to)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, sec) = (num(0, 4)?, num(5, 7)?, num(8, 10)?, num(11, 13)?, num(14, 16)?, num(17, 19)?);
    let mut i = 19;
    let mut ms = 0;
    if b.get(i) == Some(&b'.') {
        i += 1;
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        let frac = &s[start..i];
        ms = frac.chars().chain("000".chars()).take(3).collect::<String>().parse::<i64>().ok()?;
    }
    let offset_min = match b.get(i) {
        Some(b'Z') | None => 0,
        Some(sign @ (b'+' | b'-')) => {
            let hh = num(i + 1, i + 3)?;
            let mm = s.get(i + 3..).map(|r| r.trim_start_matches(':')).and_then(|r| r.get(..2)).and_then(|r| r.parse::<i64>().ok()).unwrap_or(0);
            let total = hh * 60 + mm;
            if *sign == b'+' {
                total
            } else {
                -total
            }
        }
        _ => return None,
    };
    // days from civil (Howard Hinnant's algorithm)
    let (y2, m2) = if mo <= 2 { (y - 1, mo + 9) } else { (y, mo - 3) };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * m2 + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(((days * 86400 + h * 3600 + mi * 60 + sec) - offset_min * 60) * 1000 + ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn finds_ticket_keys_and_pr_links() {
        let text = "SH-21533 is like SH-20698, see https://github.com/Org/shop/pull/2395 and \
                    https://github.com/Org/shop/pull/new/feature/x, UTF-8, SHA-256, SH-21533 again, sh-1, SH-0";
        assert_eq!(ticket_keys(text), ["SH-21533", "SH-20698"]);
        assert_eq!(pr_refs(text), ["Org/shop#2395"], "the create-a-PR link after a push is not a PR");
        assert_eq!(key_project("OPS-6294"), "OPS");
    }

    #[test]
    fn reads_claude_codes_git_records() {
        let created = from_git_operation(&json!({"pr":{"number":2395,"url":"https://github.com/Org/shop/pull/2395","action":"created"}}));
        assert_eq!(created.prs, [("Org/shop#2395".to_string(), "created".to_string())]);
        let ready = from_git_operation(&json!({"pr":{"number":12,"action":"ready"}}));
        assert_eq!(ready.prs, [("#12".to_string(), "ready".to_string())], "no URL: the session's own repo");
        let both = from_git_operation(&json!({"commit":{"sha":"898c2622f6","kind":"committed","branch":"feature/a"},"push":{"branch":"feature/a"}}));
        assert_eq!(both.commits, [("898c2622f6".to_string(), Some("feature/a".to_string()))]);
        assert_eq!(both.pushed, ["feature/a"]);
        assert!(from_git_operation(&json!({"branch":{"ref":"x","action":"merged"}})).is_empty());
    }

    #[test]
    fn reads_what_git_and_gh_printed() {
        let pr = from_command_output("gh pr create --base development --title x", "https://github.com/Org/shop/pull/2414\n", "");
        assert_eq!(pr.prs, [("Org/shop#2414".to_string(), "created".to_string())]);
        let commit = from_command_output("git add -A && git commit -m 'x'", "[feature/SH-1-a 3f2c1ab] Fix totals\n 2 files changed", "");
        assert_eq!(commit.commits, [("3f2c1ab".to_string(), Some("feature/SH-1-a".to_string()))]);
        let root = from_command_output("git commit -m init", "[main (root-commit) 0a1b2c3] init", "");
        assert_eq!(root.commits[0].0, "0a1b2c3");
        let push = from_command_output(
            "git push -u origin HEAD:hotfix/x",
            "",
            "remote: Create a pull request for 'hotfix/x' on GitHub by visiting:\nTo github.com:Org/shop.git\n * [new branch]      HEAD -> hotfix/x\n",
        );
        assert_eq!(push.pushed, ["hotfix/x"]);
        let again = from_command_output("git push", "", "To github.com:Org/shop.git\n   1a2b3c4..5d6e7f8  feature/b -> feature/b\n");
        assert_eq!(again.pushed, ["feature/b"]);
        assert!(from_command_output("git status", "[x 1234567] y", "").is_empty());
    }

    #[test]
    fn a_command_counts_only_when_it_runs() {
        assert!(runs("gh pr create --base development", &["gh", "pr", "create"]));
        assert!(runs("GH_TOKEN=\"$(gh auth token --user x)\" gh pr create --fill", &["gh", "pr", "create"]));
        assert!(runs("gh auth switch --user x >/dev/null 2>&1; gh pr create --repo o/r", &["gh", "pr", "create"]));
        assert!(runs("cd /code/shop && git -C sub --no-pager push -u origin HEAD", &["git", "push"]));
        assert!(runs("(git commit -m 'gh pr create')", &["git", "commit"]));
        assert!(!runs("command grep -F 'gh pr create' notes.jsonl | head", &["gh", "pr", "create"]));
        assert!(!runs("echo git push", &["git", "push"]));
        assert!(!runs("rg \"git commit\" src", &["git", "commit"]));
        assert!(!runs("(git commit -m 'gh pr create')", &["gh", "pr", "create"]), "quoted text inside another command");
        // a heredoc's text (with an apostrophe and markdown backticks) is not commands
        let body = "cat > .context/pr_body.md <<'EOF'\n<h3> Jira </h3>\nIt can't fail; run `gh pr create` by hand\nEOF\ngh pr create --body-file .context/pr_body.md";
        assert!(runs(body, &["gh", "pr", "create"]));
        assert_eq!(commands(body).len(), 2, "{:?}", commands(body));
        let tabs = "cat <<-END > x\n\tgit push\n\tEND\necho done";
        assert!(!runs(tabs, &["git", "push"]));
        assert!(runs("git commit -m \"$(cat <<'EOF'\nFix it, don't break\nEOF\n)\"", &["git", "commit"]));
        assert!(!runs("cat <<< 'git push'", &["git", "push"]), "a here-string is text too");
    }

    #[test]
    fn claude_codes_guess_must_match_the_command() {
        // the command only searched for the words, and its output held a PR link
        let guess = from_git_operation(&json!({"pr":{"number":2395,"url":"https://github.com/Org/shop/pull/2395","action":"created"}}));
        assert!(confirmed(guess.clone(), "command grep -F 'gh pr create' t.jsonl | jq .").is_empty());
        assert_eq!(confirmed(guess, "gh pr create --fill").prs.len(), 1);
        let push = from_git_operation(&json!({"push":{"branch":"feature/a"}}));
        assert!(confirmed(push.clone(), "rg 'git push' notes").is_empty());
        assert_eq!(confirmed(push, "git push -u origin feature/a").pushed, ["feature/a"]);
        assert!(from_command_output("rg 'gh pr create' x", "https://github.com/Org/shop/pull/1", "").is_empty());
        let edited = from_git_operation(&json!({"pr":{"number":7,"url":"https://github.com/Org/shop/pull/7","action":"edited"}}));
        assert_eq!(confirmed(edited.clone(), "gh pr edit 7 --add-label x").prs.len(), 1);
        assert!(confirmed(edited, "gh pr view 7 --json title").is_empty(), "reading a PR is not working on it");
    }

    #[test]
    fn keeps_only_what_the_person_typed() {
        let typed = typed_text(&json!([
            {"type":"text","text":"<system-reminder>SH-1 from memory</system-reminder>Please fix SH-21533"},
            {"type":"image","source":{}}
        ]));
        assert_eq!(typed.trim(), "Please fix SH-21533");
        assert_eq!(typed_text(&json!("<bash-stdout>SH-2</bash-stdout>see SH-3")), "see SH-3");
    }

    #[test]
    fn long_lived_branches_dont_link() {
        assert!(long_lived("development", ""));
        assert!(long_lived("release/1.8", ""));
        assert!(long_lived("trunk-x", "trunk-x"));
        assert!(!long_lived("hotfix/round-totals", "development"));
    }

    #[test]
    fn parses_times() {
        assert_eq!(parse_time("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_time("2026-11-05T14:03:22.123Z"), Some(1_793_887_402_123));
        assert_eq!(parse_time("2026-11-05T15:03:22.123+01:00"), Some(1_793_887_402_123));
        assert_eq!(parse_time("2026-11-05T14:03:22.1Z"), Some(1_793_887_402_100));
        assert_eq!(parse_time("yesterday"), None);
    }
}
