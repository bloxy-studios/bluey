//! Randomness for OAuth flows, from the operating system's RNG.

use bluey_protocols::oauth;
use rand::RngCore;

/// `N` random bytes.
pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    rand::rng().fill_bytes(&mut bytes);
    bytes
}

/// 16 random bytes, base64url (22 characters) — `state`, `nonce`.
pub fn random_token() -> String {
    oauth::base64url(&random_bytes::<16>())
}

/// A fresh PKCE `(code_verifier, code_challenge)` pair from 32 random bytes.
pub fn pkce() -> (String, String) {
    oauth::pkce_pair(&random_bytes::<32>())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_tokens_are_url_safe_and_unique() {
        let a = random_token();
        let b = random_token();
        assert_ne!(a, b);
        assert_eq!(a.len(), 22);
        assert!(a
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn pkce_pairs_are_fresh_and_well_formed() {
        let (verifier, challenge) = pkce();
        let (other, _) = pkce();
        assert_ne!(verifier, other);
        assert_eq!(verifier.len(), 43);
        assert_eq!(challenge.len(), 43);
        let (_, expected) = oauth::pkce_pair_from(verifier.as_bytes());
        // The challenge is over the verifier *text*, not the random bytes.
        assert_ne!(challenge, expected);
    }
}
