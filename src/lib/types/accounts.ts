/**
 * Provider accounts (ADR 0009): the owner's paid AI subscriptions as credential
 * sources next to API keys. Mirrors `bluey_core::types::accounts`.
 *
 * Nothing here is a credential. The WebView sees an account's status, plan and
 * identity; the tokens live in the macOS Keychain and are read only by Rust.
 */

import type { AIProviderKind } from "./ai";
import type { ModelRole } from "./mode";

/** Reserved ids of the subscription providers, in UI order. */
export const SUBSCRIPTION_PROVIDER_IDS = ["chatgpt", "claude", "antigravity"] as const;
export type SubscriptionProviderId = (typeof SUBSCRIPTION_PROVIDER_IDS)[number];

/** API key in the Keychain, or an OAuth subscription account. */
export type ProviderAuthMethod = "api_key" | "oauth_subscription";

export type UnavailableReason =
  /** The provider stopped recognising Bluey's request fingerprint. */
  | "fingerprint_drift"
  /** The provider refused the account (terms, region, entitlement). */
  | "policy_blocked"
  /** The model catalog endpoint is gone or rejected the pinned client version. */
  | "catalog_unavailable"
  /** The provider bills requests outside the plan — a stop signal, never a retry. */
  | "extra_usage_billing"
  /** The provider's integration is not built into this version yet. */
  | "provider_pending"
  /** Subscription accounts are switched off (feature flag or build feature). */
  | "disabled"
  | "other";

export type ConnectFlowKind = "browser" | "device_code" | "manual_code";

/** What the UI shows while a connection is pending. */
export interface ConnectFlow {
  kind: ConnectFlowKind;
  /** The authorization URL that was opened (for "copy link"). */
  url?: string;
  /** Device-code flows: the code to type in the browser. */
  userCode?: string;
  /** Device-code flows: where to type it. */
  verificationUrl?: string;
  /** When the pending flow expires (ISO 8601). */
  expiresAt: string;
}

export type AccountStatus =
  | { state: "disconnected" }
  | { state: "connecting"; flow: ConnectFlow }
  | { state: "connected" }
  /** The refresh token is gone or was rejected; the user must reconnect. */
  | { state: "needs_reauth" }
  /** A plan window is exhausted; the router skips the account until `until`. */
  | { state: "rate_limited"; until: string; window?: string }
  | { state: "unavailable"; reason: UnavailableReason; detail?: string };

/** What the provider says about the signed-in subscription. Display only. */
export interface AccountIdentity {
  email?: string;
  displayName?: string;
  /** Provider-native tier id (`plus`, `default_claude_max_5x`, `g1-pro`, …). */
  planTier?: string;
  /** Human label for the tier (`ChatGPT Plus`, `Claude Max 5×`, `Google AI Pro`). */
  planLabel?: string;
  /** Provider account / organisation id. */
  accountId?: string;
  /** Antigravity: the Cloud Code companion project. */
  projectId?: string;
}

/** Everything the WebView may know about a subscription account. */
export interface ProviderAccount {
  /** Internal id (`chatgpt`, `claude`, `antigravity`, `antigravity-2`, …). */
  accountId: string;
  /** The reserved provider id this account serves. */
  providerId: string;
  kind: AIProviderKind;
  method: ProviderAuthMethod;
  status: AccountStatus;
  identity?: AccountIdentity;
  connectedAt?: string;
  /** Access-token expiry (ISO 8601), for the dev overlay. */
  expiresAt?: string;
  catalogFetchedAt?: string;
  /** The request-fingerprint module version this build ships for the provider. */
  fingerprintVersion?: string;
  /** When that fingerprint was captured from the official client (ISO date). */
  fingerprintCapturedOn?: string;
}

export interface ModelCapabilities {
  vision: boolean;
  tools: boolean;
  /** Reasoning / thinking levels the model accepts (`minimal`, `low`, …). */
  reasoningLevels: string[];
  streaming: boolean;
  contextWindow?: number;
}

/** One model a subscription exposes. */
export interface CatalogModel {
  id: string;
  label: string;
  capabilities: ModelCapabilities;
  /** Antigravity: `antigravity` vs `gemini_cli`; other providers omit it. */
  quotaPool?: string;
  /** Roles the provider profile recommends this model for. */
  suggestedRoles: ModelRole[];
}

export type CatalogSource =
  | { type: "endpoint" }
  | { type: "probed" }
  | { type: "curated"; version: string }
  | { type: "fixture" };

export interface ProviderModelCatalog {
  accountId: string;
  providerId: string;
  fetchedAt: string;
  source: CatalogSource;
  models: CatalogModel[];
}

export interface AccountConnectOptions {
  /** Antigravity on a Workspace account: the Google Cloud project to use. */
  projectId?: string;
  /** Skip the loopback listener and start with the device-code flow. */
  preferDeviceCode?: boolean;
}

export type BilledTo = "plan" | "extra_usage" | "unknown";

/** Result of `accounts_probe_fingerprint` (developer mode). */
export interface FingerprintProbe {
  accountId: string;
  ok: boolean;
  billedTo: BilledTo;
  fingerprintVersion?: string;
  message?: string;
  checkedAt: string;
}

export function isSubscriptionProviderId(id: string): id is SubscriptionProviderId {
  return (SUBSCRIPTION_PROVIDER_IDS as readonly string[]).includes(id);
}

/** Whether a provider kind is served by a subscription account rather than an API key. */
export function isSubscriptionKind(kind: AIProviderKind): boolean {
  return kind === "chatgpt_codex" || kind === "claude_subscription" || kind === "antigravity_google";
}

/** Whether the router may send requests through this account right now. */
export function isAccountUsable(account: ProviderAccount, now: Date = new Date()): boolean {
  const { status } = account;
  if (status.state === "connected") return true;
  if (status.state === "rate_limited") {
    const until = Date.parse(status.until);
    return Number.isFinite(until) && until <= now.getTime();
  }
  return false;
}
