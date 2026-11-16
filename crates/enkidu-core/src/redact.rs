//! Git remote URLs can carry tokens, so anything git prints is cleaned before it is
//! logged or shown in the app.

/// Replaces every URL (`scheme://...`) and scp-style remote (`user@host:path`) with `<url>`.
pub fn redact_urls(text: &str) -> String {
    redact_scp(&redact_schemes(text))
}

fn redact_schemes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find("://") {
        let before = &rest[..pos];
        let scheme_start = before
            .char_indices()
            .rev()
            .take_while(|(_, c)| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
            .last()
            .map(|(i, _)| i)
            .unwrap_or(before.len());
        out.push_str(&before[..scheme_start]);
        let after = &rest[pos + 3..];
        let end = after
            .find(|c: char| c.is_whitespace() || matches!(c, '\'' | '"' | '>' | ')' | ','))
            .unwrap_or(after.len());
        out.push_str("<url>");
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

fn redact_scp(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut token = String::new();
    let flush = |token: &mut String, out: &mut String| {
        if looks_like_scp_remote(token) {
            out.push_str("<url>");
        } else {
            out.push_str(token);
        }
        token.clear();
    };
    for c in text.chars() {
        if c.is_whitespace() || matches!(c, '\'' | '"') {
            flush(&mut token, &mut out);
            out.push(c);
        } else {
            token.push(c);
        }
    }
    flush(&mut token, &mut out);
    out
}

fn looks_like_scp_remote(token: &str) -> bool {
    let Some((user_host, path)) = token.split_once(':') else {
        return false;
    };
    let Some((user, host)) = user_host.split_once('@') else {
        return false;
    };
    !user.is_empty()
        && host.contains('.')
        && !path.is_empty()
        && user_host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '@' | '.' | '-' | '_'))
}

#[cfg(test)]
mod tests {
    use super::redact_urls;

    #[test]
    fn hides_https_urls_with_tokens() {
        let text = "From https://x-access-token:ghp_secret@github.com/org/repo\n * branch main";
        let out = redact_urls(text);
        assert_eq!(out, "From <url>\n * branch main");
        assert!(!out.contains("ghp_secret"));
    }

    #[test]
    fn hides_scp_style_remotes() {
        assert_eq!(redact_urls("fatal: git@github.com:org/repo.git not found"), "fatal: <url> not found");
    }

    #[test]
    fn keeps_normal_text() {
        let text = "Preparing worktree (new branch 'feature/a')\nHEAD is now at abc123 time: 10:30";
        assert_eq!(redact_urls(text), text);
    }

    #[test]
    fn handles_quoted_urls_and_file_urls() {
        assert_eq!(redact_urls("remote 'file:///tmp/origin.git' x"), "remote '<url>' x");
    }
}
