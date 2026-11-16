//! Strict reading of GitHub's and Jira's answers: a field that should be there and isn't fails the whole read,
//! with a message that says which one. A read is all or nothing; a guess never replaces real data.

use anyhow::{anyhow, Result};
use serde_json::Value;

pub fn field<'a>(v: &'a Value, path: &str, source: &str) -> Result<&'a Value> {
    let mut cur = v;
    for part in path.split('.') {
        cur = cur.get(part).ok_or_else(|| anyhow!("{source}'s answer has no `{path}`"))?;
    }
    Ok(cur)
}

pub fn string(v: &Value, path: &str, source: &str) -> Result<String> {
    field(v, path, source)?
        .as_str()
        .map(String::from)
        .ok_or_else(|| anyhow!("{source}'s `{path}` isn't text"))
}

/// A field that may be null (or text): GitHub and Jira send null for "none".
pub fn opt_string(v: &Value, path: &str, source: &str) -> Result<Option<String>> {
    let f = field(v, path, source)?;
    if f.is_null() {
        return Ok(None);
    }
    f.as_str().map(|s| Some(s.to_string())).ok_or_else(|| anyhow!("{source}'s `{path}` isn't text"))
}

pub fn int(v: &Value, path: &str, source: &str) -> Result<i64> {
    field(v, path, source)?.as_i64().ok_or_else(|| anyhow!("{source}'s `{path}` isn't a number"))
}

pub fn opt_int(v: &Value, path: &str, source: &str) -> Result<Option<i64>> {
    let f = field(v, path, source)?;
    if f.is_null() {
        return Ok(None);
    }
    f.as_i64().map(Some).ok_or_else(|| anyhow!("{source}'s `{path}` isn't a number"))
}

pub fn boolean(v: &Value, path: &str, source: &str) -> Result<bool> {
    field(v, path, source)?.as_bool().ok_or_else(|| anyhow!("{source}'s `{path}` isn't true or false"))
}

pub fn array<'a>(v: &'a Value, path: &str, source: &str) -> Result<&'a Vec<Value>> {
    field(v, path, source)?.as_array().ok_or_else(|| anyhow!("{source}'s `{path}` isn't a list"))
}

/// Parses a tool's printed answer; an empty or broken answer is a failed read.
pub fn parse(text: &str, source: &str) -> Result<Value> {
    if text.trim().is_empty() {
        return Err(anyhow!("{source} answered nothing"));
    }
    serde_json::from_str(text).map_err(|e| anyhow!("{source}'s answer isn't valid JSON ({e})"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn missing_or_wrong_fields_fail_with_their_name() {
        let v = json!({ "a": { "b": "x", "n": 3, "null": null, "list": [1] } });
        assert_eq!(string(&v, "a.b", "GitHub").unwrap(), "x");
        assert_eq!(int(&v, "a.n", "GitHub").unwrap(), 3);
        assert_eq!(opt_string(&v, "a.null", "GitHub").unwrap(), None);
        assert_eq!(array(&v, "a.list", "GitHub").unwrap().len(), 1);
        assert!(string(&v, "a.c", "GitHub").unwrap_err().to_string().contains("no `a.c`"));
        assert!(string(&v, "a.n", "Jira").unwrap_err().to_string().contains("Jira's `a.n` isn't text"));
        assert!(parse("", "twg").unwrap_err().to_string().contains("answered nothing"));
        assert!(parse("{oops", "gh").unwrap_err().to_string().contains("isn't valid JSON"));
    }
}
