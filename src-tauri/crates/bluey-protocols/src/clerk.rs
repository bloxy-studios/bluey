//! Clerk publishable-key → Frontend API host derivation and the URL allowlist
//! check used by the `auth_fapi_fetch` proxy.
//!
//! A publishable key is `pk_test_` or `pk_live_` followed by
//! `base64("<fapi-host>$")`, e.g. `pk_test_Y2xlcmsuZXhhbXBsZS5jb20k` →
//! `clerk.example.com`.

use base64::Engine;

/// Derive the Frontend API host from a publishable key. Returns `None` for
/// malformed keys.
pub fn fapi_host_from_publishable_key(publishable_key: &str) -> Option<String> {
    let key = publishable_key.trim();
    let encoded = key
        .strip_prefix("pk_test_")
        .or_else(|| key.strip_prefix("pk_live_"))?;
    let bytes = decode_base64(encoded)?;
    let decoded = String::from_utf8(bytes).ok()?;
    let host = decoded.strip_suffix('$').unwrap_or(&decoded).to_string();
    if host.is_empty() || !is_plausible_host(&host) {
        return None;
    }
    Some(host.to_ascii_lowercase())
}

fn decode_base64(encoded: &str) -> Option<Vec<u8>> {
    let engine = base64::engine::general_purpose::STANDARD;
    if let Ok(bytes) = engine.decode(encoded) {
        return Some(bytes);
    }
    base64::engine::general_purpose::STANDARD_NO_PAD
        .decode(encoded.trim_end_matches('='))
        .ok()
}

fn is_plausible_host(host: &str) -> bool {
    !host.is_empty()
        && host.contains('.')
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

/// Whether `url` is an HTTPS URL on exactly `allowed_host` (no port, no
/// userinfo tricks). This is the only shape `auth_fapi_fetch` will proxy.
pub fn is_allowed_fapi_url(url: &str, allowed_host: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    // Reject userinfo (`user@host`) and ports (`host:443`) outright.
    if authority.contains('@') || authority.contains(':') {
        return false;
    }
    !authority.is_empty() && authority.eq_ignore_ascii_case(allowed_host)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn encode_key(prefix: &str, host: &str) -> String {
        let engine = base64::engine::general_purpose::STANDARD;
        format!("{prefix}{}", engine.encode(format!("{host}$")))
    }

    #[test]
    fn derives_hosts_from_test_and_live_keys() {
        let key = encode_key("pk_test_", "adapted-gator-12.clerk.accounts.dev");
        assert_eq!(
            fapi_host_from_publishable_key(&key).as_deref(),
            Some("adapted-gator-12.clerk.accounts.dev")
        );
        let key = encode_key("pk_live_", "clerk.bluey.app");
        assert_eq!(
            fapi_host_from_publishable_key(&key).as_deref(),
            Some("clerk.bluey.app")
        );
    }

    #[test]
    fn tolerates_missing_padding_and_dollar() {
        let engine = base64::engine::general_purpose::STANDARD_NO_PAD;
        let key = format!("pk_test_{}", engine.encode("clerk.example.com$"));
        assert_eq!(
            fapi_host_from_publishable_key(&key).as_deref(),
            Some("clerk.example.com")
        );
        let key = format!("pk_test_{}", engine.encode("clerk.example.com"));
        assert_eq!(
            fapi_host_from_publishable_key(&key).as_deref(),
            Some("clerk.example.com")
        );
    }

    #[test]
    fn rejects_malformed_keys() {
        assert_eq!(fapi_host_from_publishable_key(""), None);
        assert_eq!(fapi_host_from_publishable_key("sk_test_abc"), None);
        assert_eq!(fapi_host_from_publishable_key("pk_test_!!!"), None);
        // decodes but is not a hostname
        let engine = base64::engine::general_purpose::STANDARD;
        let key = format!("pk_test_{}", engine.encode("not a host$"));
        assert_eq!(fapi_host_from_publishable_key(&key), None);
    }

    #[test]
    fn url_allowlist() {
        let host = "clerk.example.com";
        assert!(is_allowed_fapi_url(
            "https://clerk.example.com/v1/client",
            host
        ));
        assert!(is_allowed_fapi_url("https://CLERK.example.com", host));
        assert!(is_allowed_fapi_url(
            "https://clerk.example.com?_is_native=1",
            host
        ));
        assert!(!is_allowed_fapi_url(
            "http://clerk.example.com/v1/client",
            host
        ));
        assert!(!is_allowed_fapi_url(
            "https://evil.com/clerk.example.com",
            host
        ));
        assert!(!is_allowed_fapi_url(
            "https://clerk.example.com.evil.com/x",
            host
        ));
        assert!(!is_allowed_fapi_url(
            "https://clerk.example.com@evil.com/",
            host
        ));
        assert!(!is_allowed_fapi_url(
            "https://clerk.example.com:8443/",
            host
        ));
        assert!(!is_allowed_fapi_url("clerk.example.com/v1", host));
    }
}
