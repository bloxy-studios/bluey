/**
 * Saved-credential state (`secrets_state`, `secrets_health`; Rust
 * `crate::secrets::SecretState` / `health::CredentialHealth`). Names and
 * states only — a secret value never reaches the WebView (ADR 0011).
 */

/**
 * `present`: saved and usable. `locked`: saved, but macOS wants the user's
 * approval before this build reads it (an update or rebuild changed Bluey's
 * code identity). `absent`: not saved.
 */
export type SecretState = "present" | "locked" | "absent";

export type CredentialCategory =
  "provider_key" | "research_key" | "agent_key" | "account_tokens" | "sign_in" | "unknown";

/** One saved Bluey-owned credential (Settings → Privacy → Saved credentials). */
export interface CredentialHealth {
  /** The item's name — what `secrets_allow_access` / `secrets_delete` take. */
  key: string;
  category: CredentialCategory;
  /** What it belongs to: a provider, a subscription, the Bluey sign-in. */
  label: string;
  state: SecretState;
  /** API keys can be removed here; sign-in and account tokens go through
   * Sign out and Disconnect. */
  removable: boolean;
}
