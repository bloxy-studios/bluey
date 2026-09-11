//! Token sets as stored in the Keychain, and a single-flight refresh cache.

use std::future::Future;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bluey_protocols::oauth::{expires_at, TokenResponse};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

/// Refresh an access token this long before it expires.
pub const DEFAULT_REFRESH_LEEWAY: Duration = Duration::from_secs(60);

/// Unix seconds now (0 if the clock is before 1970).
pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// What a signed-in account keeps in the Keychain (JSON). The field names are
/// the wire format of existing entries — do not rename them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenSet {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Unix seconds.
    #[serde(default)]
    pub expires_at: Option<u64>,
    #[serde(default)]
    pub id_token: Option<String>,
}

impl TokenSet {
    /// From a token-endpoint response; a refresh response that carries no new
    /// refresh token keeps the previous one (providers rotate optionally).
    pub fn from_response(
        response: TokenResponse,
        previous_refresh: Option<String>,
        now_unix: u64,
    ) -> Self {
        Self {
            access_token: response.access_token,
            refresh_token: response.refresh_token.or(previous_refresh),
            expires_at: expires_at(response.expires_in, now_unix),
            id_token: response.id_token,
        }
    }

    /// Whether the access token expires within `leeway` (tokens without an
    /// expiry never do).
    pub fn is_expiring(&self, leeway: Duration, now_unix: u64) -> bool {
        match self.expires_at {
            Some(at) => at.saturating_sub(leeway.as_secs()) <= now_unix,
            None => false,
        }
    }
}

/// Why [`TokenCache::fresh`] could not hand out a token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshError<E> {
    /// Nothing is stored — the account is disconnected.
    NoTokens,
    /// The access token is expiring and there is no refresh token to renew it.
    NoRefreshToken,
    /// The caller's refresh failed; the previous tokens are kept.
    Refresh(E),
}

/// The current token set of one account with **single-flight** refresh: the
/// slot is locked for the duration of a refresh, so concurrent callers wait
/// for the one in progress and then see the fresh token instead of starting
/// their own.
pub struct TokenCache {
    leeway: Duration,
    slot: Mutex<Option<TokenSet>>,
}

impl TokenCache {
    pub fn new(initial: Option<TokenSet>, leeway: Duration) -> Self {
        Self {
            leeway,
            slot: Mutex::new(initial),
        }
    }

    pub async fn current(&self) -> Option<TokenSet> {
        self.slot.lock().await.clone()
    }

    /// Replace (or clear) the stored tokens — after a sign-in or a sign-out.
    pub async fn replace(&self, tokens: Option<TokenSet>) {
        *self.slot.lock().await = tokens;
    }

    /// A token set that is not expiring at `now_unix`, refreshing through
    /// `refresh(current, refresh_token)` when it is. Exactly one refresh runs
    /// no matter how many callers arrive while it is in flight.
    pub async fn fresh<E, F, Fut>(
        &self,
        now_unix: u64,
        refresh: F,
    ) -> Result<TokenSet, RefreshError<E>>
    where
        F: FnOnce(TokenSet, String) -> Fut,
        Fut: Future<Output = Result<TokenSet, E>>,
    {
        let mut slot = self.slot.lock().await;
        let Some(current) = slot.as_ref() else {
            return Err(RefreshError::NoTokens);
        };
        if !current.is_expiring(self.leeway, now_unix) {
            return Ok(current.clone());
        }
        let Some(refresh_token) = current.refresh_token.clone() else {
            return Err(RefreshError::NoRefreshToken);
        };
        let fresh = refresh(current.clone(), refresh_token)
            .await
            .map_err(RefreshError::Refresh)?;
        *slot = Some(fresh.clone());
        Ok(fresh)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use bluey_protocols::oauth::parse_token_response;
    use pretty_assertions::assert_eq;

    use super::*;

    fn tokens(expires_at: Option<u64>, refresh: Option<&str>) -> TokenSet {
        TokenSet {
            access_token: "a".into(),
            refresh_token: refresh.map(String::from),
            expires_at,
            id_token: None,
        }
    }

    #[test]
    fn token_sets_track_expiry_with_leeway() {
        let set = tokens(Some(1_000), None);
        assert!(!set.is_expiring(Duration::from_secs(60), 900));
        assert!(set.is_expiring(Duration::from_secs(60), 940));
        assert!(set.is_expiring(Duration::from_secs(60), 2_000));
        assert!(!tokens(None, None).is_expiring(Duration::from_secs(60), u64::MAX));
    }

    #[test]
    fn a_refresh_response_without_a_new_refresh_token_keeps_the_old_one() {
        let response = parse_token_response(r#"{"access_token":"new","expires_in":60}"#).unwrap();
        let set = TokenSet::from_response(response, Some("old-rt".into()), 1_000);
        assert_eq!(set.access_token, "new");
        assert_eq!(set.refresh_token.as_deref(), Some("old-rt"));
        assert_eq!(set.expires_at, Some(1_060));
        let rotated =
            parse_token_response(r#"{"access_token":"n","refresh_token":"rt2"}"#).unwrap();
        let set = TokenSet::from_response(rotated, Some("old".into()), 1_000);
        assert_eq!(set.refresh_token.as_deref(), Some("rt2"));
        assert_eq!(set.expires_at, None);
    }

    #[test]
    fn the_keychain_json_shape_is_stable() {
        let stored = r#"{"access_token":"at","refresh_token":"rt","expires_at":1700000000,"id_token":"a.b.c"}"#;
        let set: TokenSet = serde_json::from_str(stored).unwrap();
        assert_eq!(set.refresh_token.as_deref(), Some("rt"));
        assert_eq!(set.expires_at, Some(1_700_000_000));
        assert_eq!(serde_json::to_string(&set).unwrap(), stored);
        // Older entries without optional fields still load.
        let minimal: TokenSet = serde_json::from_str(r#"{"access_token":"at"}"#).unwrap();
        assert_eq!(minimal, tokens(None, None).clone_with_access("at"));
    }

    impl TokenSet {
        fn clone_with_access(&self, access: &str) -> Self {
            Self {
                access_token: access.into(),
                ..self.clone()
            }
        }
    }

    #[tokio::test]
    async fn a_valid_token_is_returned_without_refreshing() {
        let cache = TokenCache::new(
            Some(tokens(Some(10_000), Some("rt"))),
            DEFAULT_REFRESH_LEEWAY,
        );
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let set = cache
            .fresh(5_000, |_, _| async move {
                counted.fetch_add(1, Ordering::SeqCst);
                Ok::<_, ()>(tokens(Some(20_000), Some("rt2")))
            })
            .await
            .unwrap();
        assert_eq!(set.access_token, "a");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn concurrent_callers_share_one_refresh() {
        let cache = Arc::new(TokenCache::new(
            Some(tokens(Some(1_000), Some("rt"))),
            DEFAULT_REFRESH_LEEWAY,
        ));
        let calls = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for _ in 0..3 {
            let cache = cache.clone();
            let calls = calls.clone();
            handles.push(tokio::spawn(async move {
                cache
                    .fresh(990, |current, refresh_token| async move {
                        assert_eq!(current.access_token, "a");
                        assert_eq!(refresh_token, "rt");
                        calls.fetch_add(1, Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_millis(200)).await;
                        Ok::<_, ()>(TokenSet {
                            access_token: "fresh".into(),
                            refresh_token: Some("rt2".into()),
                            expires_at: Some(5_000),
                            id_token: None,
                        })
                    })
                    .await
            }));
        }
        for handle in handles {
            let set = handle.await.unwrap().unwrap();
            assert_eq!(set.access_token, "fresh");
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            cache.current().await.unwrap().refresh_token.as_deref(),
            Some("rt2")
        );
    }

    #[tokio::test]
    async fn refresh_failures_keep_the_previous_tokens() {
        let cache = TokenCache::new(
            Some(tokens(Some(1_000), Some("rt"))),
            DEFAULT_REFRESH_LEEWAY,
        );
        let outcome = cache
            .fresh(999, |_, _| async { Err::<TokenSet, _>("invalid_grant") })
            .await;
        assert_eq!(outcome, Err(RefreshError::Refresh("invalid_grant")));
        assert_eq!(cache.current().await, Some(tokens(Some(1_000), Some("rt"))));

        let without_refresh =
            TokenCache::new(Some(tokens(Some(1_000), None)), DEFAULT_REFRESH_LEEWAY);
        assert_eq!(
            without_refresh
                .fresh(999, |_, _| async { Ok::<TokenSet, ()>(tokens(None, None)) })
                .await,
            Err(RefreshError::NoRefreshToken)
        );

        let empty = TokenCache::new(None, DEFAULT_REFRESH_LEEWAY);
        assert_eq!(
            empty
                .fresh(0, |_, _| async { Ok::<TokenSet, ()>(tokens(None, None)) })
                .await,
            Err(RefreshError::NoTokens)
        );
        empty.replace(Some(tokens(None, None))).await;
        assert!(empty.current().await.is_some());
    }
}
