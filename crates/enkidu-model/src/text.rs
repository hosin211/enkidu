//! Text the settings and the frame turn into lists and back: Claude's arguments split and joined as a POSIX shell
//! would, `KEY=value` lines, lists of lines, and a home folder said as `~`.

/// A count with a comma at every thousand: "1,234".
pub fn thousands(count: usize) -> String {
    let digits = count.to_string();
    let mut grouped = String::new();
    for (ix, digit) in digits.chars().enumerate() {
        if ix > 0 && (digits.len() - ix).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

/// Splits a command line the way a POSIX shell would: quotes and backslashes, no expansion.
pub fn split_args(input: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_token = false;
    let mut quote: Option<char> = None;
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match quote {
            Some('\'') => {
                if c == '\'' {
                    quote = None;
                } else {
                    cur.push(c);
                }
            }
            Some(_) => {
                if c == '"' {
                    quote = None;
                } else if c == '\\' && i + 1 < chars.len() && "\"\\$`".contains(chars[i + 1]) {
                    i += 1;
                    cur.push(chars[i]);
                } else {
                    cur.push(c);
                }
            }
            None => {
                if c == '\'' || c == '"' {
                    quote = Some(c);
                    in_token = true;
                } else if c == '\\' && i + 1 < chars.len() {
                    i += 1;
                    cur.push(chars[i]);
                    in_token = true;
                } else if c.is_whitespace() {
                    if in_token {
                        out.push(std::mem::take(&mut cur));
                    }
                    in_token = false;
                } else {
                    cur.push(c);
                    in_token = true;
                }
            }
        }
        i += 1;
    }
    if quote.is_some() {
        return Err("A quote isn't closed".into());
    }
    if in_token {
        out.push(cur);
    }
    Ok(out)
}

/// The opposite of [`split_args`]: quotes only what needs it.
pub fn join_args(args: &[String]) -> String {
    let plain = |a: &str| a.chars().all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c));
    args.iter()
        .map(|a| match a.as_str() {
            "" => "''".to_string(),
            a if plain(a) => a.to_string(),
            a => format!("'{}'", a.replace('\'', "'\\''")),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `KEY=value` lines, in order (a key given twice keeps its first place and its last value). Blank lines and `#`
/// comments are left out; a line that isn't a pair, or a name that won't do, is said with its line number.
pub fn parse_env(text: &str) -> Result<Vec<(String, String)>, String> {
    let mut env: Vec<(String, String)> = Vec::new();
    for (n, raw) in text.split('\n').enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some(eq) = line.find('=').filter(|eq| *eq > 0) else {
            return Err(format!("Line {}: use KEY=value", n + 1));
        };
        let key = line[..eq].trim();
        let name_ok = key.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !name_ok {
            return Err(format!("Line {}: \"{key}\" isn't a valid name", n + 1));
        }
        let value = line[eq + 1..].trim().to_string();
        match env.iter_mut().find(|(k, _)| k == key) {
            Some(pair) => pair.1 = value,
            None => env.push((key.to_string(), value)),
        }
    }
    Ok(env)
}

/// The opposite of [`parse_env`].
pub fn format_env(env: &[(String, String)]) -> String {
    env.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("\n")
}

/// Lines to a list, without blanks.
pub fn parse_lines(text: &str) -> Vec<String> {
    text.split('\n').map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect()
}

/// A path as you'd say it: a home folder (`/Users/<name>` or `/home/<name>`) becomes `~`.
pub fn short_path(path: &str) -> String {
    for root in ["/Users/", "/home/"] {
        if let Some(rest) = path.strip_prefix(root) {
            let name_end = rest.find('/').unwrap_or(rest.len());
            if name_end > 0 {
                return format!("~{}", &rest[name_end..]);
            }
        }
    }
    path.to_string()
}

#[cfg(test)]
mod tests {
    // How a line of arguments is split: nine tests.
    use super::*;
    use crate::new_workspace::{name_from_branch, valid_name};

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn split_args_splits_like_a_shell() {
        assert_eq!(
            split_args("--model opus  --permission-mode acceptEdits").unwrap(),
            strings(&["--model", "opus", "--permission-mode", "acceptEdits"])
        );
        assert_eq!(
            split_args(r#"--append-system-prompt "be brief, it's late" 'a b' c\ d"#).unwrap(),
            strings(&["--append-system-prompt", "be brief, it's late", "a b", "c d"])
        );
        assert_eq!(split_args(r#"--x """#).unwrap(), strings(&["--x", ""]));
        assert_eq!(split_args("   ").unwrap(), Vec::<String>::new());
    }

    #[test]
    fn split_args_rejects_an_open_quote() {
        assert!(split_args(r#"--x "open"#).unwrap_err().contains("quote"));
    }

    #[test]
    fn split_args_round_trips_through_join_args() {
        let args = strings(&["--model", "opus", "--name", "it's here", "", "a=b"]);
        assert_eq!(split_args(&join_args(&args)).unwrap(), args);
        assert_eq!(join_args(&strings(&["--model", "opus"])), "--model opus");
    }

    #[test]
    fn env_parses_key_value_lines() {
        assert_eq!(
            parse_env("CLAUDE_CONFIG_DIR=~/.claude-work\n# note\n\nA=b=c").unwrap(),
            [("CLAUDE_CONFIG_DIR".to_string(), "~/.claude-work".to_string()), ("A".to_string(), "b=c".to_string())]
        );
        assert_eq!(format_env(&[("A".into(), "1".into()), ("B".into(), "2".into())]), "A=1\nB=2");
    }

    #[test]
    fn env_names_the_bad_line() {
        assert!(parse_env("A=1\nnot a pair").unwrap_err().contains("Line 2"));
        assert!(parse_env("1BAD=x").unwrap_err().contains("isn't a valid name"));
    }

    #[test]
    fn lines_split_without_blanks() {
        assert_eq!(parse_lines(" .env \n\n CLAUDE.local.md\n"), strings(&[".env", "CLAUDE.local.md"]));
    }

    #[test]
    fn names_suggests_a_workspace_name_from_a_branch() {
        assert_eq!(name_from_branch("feature/SH-21309-stock export"), "SH-21309-stock-export");
        assert_eq!(name_from_branch("main"), "main");
        assert!(valid_name(&name_from_branch("fix/__weird__")));
    }

    #[test]
    fn names_validates_names_like_the_core() {
        assert!(valid_name("sh-21309_fix.2"));
        for bad in ["", "-x", "a b", "x/y", ".hidden", &"a".repeat(65)] {
            assert!(!valid_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn paths_shortens_home_paths() {
        assert_eq!(short_path("/Users/sam/enkidu/ws"), "~/enkidu/ws");
        assert_eq!(short_path("/opt/x"), "/opt/x");
        assert_eq!(short_path("/home/ann"), "~");
    }
}
