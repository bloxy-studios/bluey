//! Minimal `.env` parser shared by `build.rs` (which bakes the *public* Clerk
//! settings into the binary) and the runtime loader in [`crate::secrets::load_dotenv`].
//!
//! Deliberately std-only: `build.rs` includes this file with `#[path]`, so it
//! must not depend on anything from the crate or its dependencies.
//!
//! Accepted syntax mirrors what Vite and Bun accept for the same files:
//! `KEY=value`, an optional `export ` prefix, blank lines and `#` comments,
//! single- or double-quoted values (quotes stripped), and an inline ` # comment`
//! after an unquoted value. Empty assignments (`KEY=`) are skipped so a
//! placeholder line — `.env.example` is full of them — never masks a value from
//! another file or from the process environment.

/// Parse the `KEY=value` pairs of a `.env` file, in file order.
pub fn parse(contents: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for raw in contents.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line
            .strip_prefix("export ")
            .map(str::trim_start)
            .unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        let value = clean_value(value.trim());
        if value.is_empty() {
            continue;
        }
        pairs.push((key.to_string(), value.to_string()));
    }
    pairs
}

/// Strip one pair of matching quotes, or — for an unquoted value — an inline
/// comment introduced by whitespace and `#`.
fn clean_value(value: &str) -> &str {
    let bytes = value.as_bytes();
    if bytes.len() >= 2 {
        let (first, last) = (bytes[0], bytes[bytes.len() - 1]);
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return &value[1..value.len() - 1];
        }
    }
    if value.starts_with('#') {
        return "";
    }
    match value.find(" #").or_else(|| value.find("\t#")) {
        Some(index) => value[..index].trim_end(),
        None => value,
    }
}

#[cfg(test)]
mod tests {
    use super::parse;

    fn map(contents: &str) -> Vec<(String, String)> {
        parse(contents)
    }

    #[test]
    fn parses_plain_quoted_and_exported_assignments() {
        let pairs = map(concat!(
            "# comment\n",
            "\n",
            "GEMINI_API_KEY=abc123\n",
            "export BLUEY_AI_PROVIDER=gemini\n",
            "VITE_CLERK_PUBLISHABLE_KEY=\"pk_test_Zm9v\"\n",
            "BLUEY_CLERK_ACCOUNT_PORTAL_URL='https://accounts.example.com/user'\n",
            "  SPACED  =  value with spaces  \n",
        ));
        assert_eq!(
            pairs,
            vec![
                ("GEMINI_API_KEY".to_string(), "abc123".to_string()),
                ("BLUEY_AI_PROVIDER".to_string(), "gemini".to_string()),
                (
                    "VITE_CLERK_PUBLISHABLE_KEY".to_string(),
                    "pk_test_Zm9v".to_string()
                ),
                (
                    "BLUEY_CLERK_ACCOUNT_PORTAL_URL".to_string(),
                    "https://accounts.example.com/user".to_string()
                ),
                ("SPACED".to_string(), "value with spaces".to_string()),
            ]
        );
    }

    #[test]
    fn skips_empty_values_invalid_keys_and_lines_without_equals() {
        let pairs = map("BLUEY_CLERK_OAUTH_CLIENT_ID=\nEMPTY_QUOTED=\"\"\nnot a pair\nBAD-KEY=x\n=novalue\nOK=1\n");
        assert_eq!(pairs, vec![("OK".to_string(), "1".to_string())]);
    }

    #[test]
    fn strips_inline_comments_only_outside_quotes_and_keeps_equals_in_values() {
        let pairs = map(concat!(
            "A=value # trailing comment\n",
            "B=\"quoted # not a comment\"\n",
            "C=#comment-only\n",
            "D=base64==\n",
            "E=https://example.com/?a=1&b=2\n",
        ));
        assert_eq!(
            pairs,
            vec![
                ("A".to_string(), "value".to_string()),
                ("B".to_string(), "quoted # not a comment".to_string()),
                ("D".to_string(), "base64==".to_string()),
                ("E".to_string(), "https://example.com/?a=1&b=2".to_string()),
            ]
        );
    }

    #[test]
    fn handles_crlf_and_unbalanced_quotes() {
        let pairs = map("A=1\r\nB=\"unbalanced\r\nC='x\"\r\n");
        assert_eq!(
            pairs,
            vec![
                ("A".to_string(), "1".to_string()),
                ("B".to_string(), "\"unbalanced".to_string()),
                ("C".to_string(), "'x\"".to_string()),
            ]
        );
    }
}
