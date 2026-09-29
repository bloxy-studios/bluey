//! Credential health (Settings → Privacy → Saved credentials): which saved
//! Bluey-owned credentials this build can use without asking, which macOS
//! holds back until the user allows access, and what each one is for. Names
//! and states only — a value never leaves Rust (ADR 0011).

use serde::Serialize;

use super::{is_webview_secret_key, key_category, SecretState};
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
}
