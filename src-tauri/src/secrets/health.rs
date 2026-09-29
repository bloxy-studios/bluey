//! Credential health (Settings → Privacy → Saved credentials): which saved
//! Bluey-owned credentials this build can use without asking, which macOS
//! holds back until the user allows access, and what each one is for. Names
//! and states only — a value never leaves Rust (ADR 0011).

use std::sync::Arc;

use bluey_core::{BlueyError, BlueyResult};
use serde::Serialize;

use super::{is_account_tokens_key, is_webview_secret_key, key_category};
use super::{SecretState, SecretsStore};
use super::{AGENT_ANTHROPIC_KEY, CLERK_OAUTH_TOKENS_KEY, CLERK_TOKEN_KEY, EXA_KEY, FIRECRAWL_KEY};

/// One saved credential, as the WebView may see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialHealth {
    /// The item's key (a name, never a value): what `secrets_allow_access`
    /// and, for removable keys, `secrets_delete` take.
    pub key: String,
    /// `provider_key`, `research_key`, `agent_key`, `account_tokens` or `sign_in`.
    pub category: &'static str,
    /// What the credential belongs to, for the user.
    pub label: String,
    pub state: SecretState,
    /// Removable from Settings (API keys). Sign-in and account tokens go
    /// through Sign out and Disconnect instead.
    pub removable: bool,
}

/// Display names the health list needs from the rest of the app.
pub trait CredentialLabels {
    /// The configured provider `provider_id`, if it still exists.
    fn provider(&self, provider_id: &str) -> Option<String>;
    /// The provider behind subscription account `account_id`, if connected.
    fn account(&self, account_id: &str) -> Option<String>;
}

/// Describe `states` (from [`super::SecretsStore::states`]) for the WebView.
pub fn describe(
    states: Vec<(String, SecretState)>,
    labels: &dyn CredentialLabels,
) -> Vec<CredentialHealth> {
    states
        .into_iter()
        .map(|(key, state)| CredentialHealth {
            category: key_category(&key),
            label: label(&key, labels),
            removable: is_webview_secret_key(&key),
            state,
            key,
        })
        .collect()
}

fn label(key: &str, labels: &dyn CredentialLabels) -> String {
    match key {
        EXA_KEY => "Exa".into(),
        FIRECRAWL_KEY => "Firecrawl".into(),
        AGENT_ANTHROPIC_KEY => "Anthropic (agent)".into(),
        CLERK_TOKEN_KEY | CLERK_OAUTH_TOKENS_KEY => "Bluey sign-in".into(),
        _ => {
            if let Some(id) = middle(key, "provider:", ":api_key") {
                return labels
                    .provider(id)
                    .unwrap_or_else(|| format!("{id} (removed provider)"));
            }
            if let Some(id) = middle(key, "account:", ":oauth_tokens") {
                return labels
                    .account(id)
                    .map(|provider| format!("{provider} subscription"))
                    .unwrap_or_else(|| "Disconnected subscription".into());
            }
            key.to_string()
        }
    }
}

/// Whether "Allow access" may target `key`: a credential the health list
/// offers it for — an API key from Settings, the Bluey sign-in, or a
/// subscription account's tokens. The only command that takes a Rust-only
/// key from the WebView, and it returns a state, never a value.
pub fn may_allow_access(key: &str) -> bool {
    is_webview_secret_key(key) || key == CLERK_OAUTH_TOKENS_KEY || is_account_tokens_key(key)
}

/// "Allow access" (`secrets_allow_access`): the one read that may show
/// macOS's Keychain prompt, so the user answers it once, on purpose. Only an
/// item the silent probe reports `Locked` is read (the value stays in Rust's
/// cache); any other state is returned as it is.
pub async fn allow_access(secrets: &Arc<SecretsStore>, key: &str) -> BlueyResult<SecretState> {
    if !may_allow_access(key) {
        return Err(BlueyError::invalid_params(
            "this secret is not listed under saved credentials",
        ));
    }
    let state = secrets.state(key).await?;
    if state != SecretState::Locked {
        return Ok(state);
    }
    Ok(match secrets.get(key).await? {
        Some(_) => SecretState::Present,
        None => SecretState::Absent,
    })
}

fn middle<'a>(key: &'a str, prefix: &str, suffix: &str) -> Option<&'a str> {
    key.strip_prefix(prefix)?.strip_suffix(suffix)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::{account_tokens_key, provider_key};

    struct Names;

    impl CredentialLabels for Names {
        fn provider(&self, provider_id: &str) -> Option<String> {
            (provider_id == "openai").then(|| "OpenAI".into())
        }
        fn account(&self, account_id: &str) -> Option<String> {
            (account_id == "claude").then(|| "Claude".into())
        }
    }

    #[test]
    fn names_states_and_removability_but_never_values() {
        let states = vec![
            (provider_key("openai"), SecretState::Present),
            (provider_key("gone"), SecretState::Locked),
            (account_tokens_key("claude"), SecretState::Locked),
            (CLERK_TOKEN_KEY.to_string(), SecretState::Present),
            (EXA_KEY.to_string(), SecretState::Present),
        ];
        let health = describe(states, &Names);
        let view: Vec<_> = health
            .iter()
            .map(|h| (h.category, h.label.as_str(), h.state, h.removable))
            .collect();
        assert_eq!(
            view,
            vec![
                ("provider_key", "OpenAI", SecretState::Present, true),
                (
                    "provider_key",
                    "gone (removed provider)",
                    SecretState::Locked,
                    true
                ),
                (
                    "account_tokens",
                    "Claude subscription",
                    SecretState::Locked,
                    false
                ),
                ("sign_in", "Bluey sign-in", SecretState::Present, false),
                ("research_key", "Exa", SecretState::Present, true),
            ]
        );
        let json = serde_json::to_value(&health[0]).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "key": "provider:openai:api_key",
                "category": "provider_key",
                "label": "OpenAI",
                "state": "present",
                "removable": true,
            })
        );
    }

    #[tokio::test]
    async fn allow_access_reads_only_a_locked_listed_item() {
        use crate::secrets::backend::fake::{CountingFake, Op};
        let exa = EXA_KEY;
        let fake = Arc::new(CountingFake::with_items(&[
            (exa, "saved"),
            (CLERK_OAUTH_TOKENS_KEY, "{}"),
        ]));
        let secrets = Arc::new(SecretsStore::with_backend(fake.clone()));

        for key in ["settings:theme", CLERK_TOKEN_KEY, "account::oauth_tokens"] {
            let error = allow_access(&secrets, key).await.unwrap_err();
            assert_eq!(error.code, "internal.invalid_params", "{key}");
        }
        assert_eq!(fake.count(Op::Probe), 0, "rejected before the store");

        let absent = account_tokens_key("claude");
        assert_eq!(
            allow_access(&secrets, &absent).await.unwrap(),
            SecretState::Absent
        );
        assert_eq!(
            allow_access(&secrets, exa).await.unwrap(),
            SecretState::Present
        );
        assert_eq!(
            fake.reads(),
            0,
            "readable or absent items are never read again"
        );

        // A locked item is read: the prompt the user asked for (here denied).
        fake.lock_item(CLERK_OAUTH_TOKENS_KEY, -25293);
        let denied = allow_access(&secrets, CLERK_OAUTH_TOKENS_KEY).await;
        assert_eq!(denied.unwrap_err().code, "storage.keychain_access_denied");
        assert_eq!(fake.reads(), 1);
    }
}
