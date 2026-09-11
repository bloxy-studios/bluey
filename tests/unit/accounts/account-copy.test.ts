import { describe, expect, it } from "vitest";

import {
  canConnect,
  formatAge,
  statusLabel,
  unavailableReasonLabel,
} from "@/features/settings/accounts/account-copy";
import { isAccountUsable, isSubscriptionKind, isSubscriptionProviderId, type ProviderAccount } from "@/lib/types";

function account(status: ProviderAccount["status"], planLabel?: string): ProviderAccount {
  return {
    accountId: "chatgpt",
    providerId: "chatgpt",
    kind: "chatgpt_codex",
    method: "oauth_subscription",
    status,
    identity: planLabel ? { planLabel } : undefined,
  };
}

describe("account types and card copy", () => {
  it("knows the subscription kinds and reserved ids", () => {
    expect(isSubscriptionProviderId("claude")).toBe(true);
    expect(isSubscriptionProviderId("gemini")).toBe(false);
    expect(isSubscriptionKind("antigravity_google")).toBe(true);
    expect(isSubscriptionKind("google_gemini")).toBe(false);
  });

  it("only connected accounts and expired rate limits are usable", () => {
    const now = new Date("2026-09-11T12:00:00Z");
    expect(isAccountUsable(account({ state: "connected" }), now)).toBe(true);
    expect(isAccountUsable(account({ state: "disconnected" }), now)).toBe(false);
    expect(isAccountUsable(account({ state: "needs_reauth" }), now)).toBe(false);
    expect(isAccountUsable(account({ state: "rate_limited", until: "2026-09-11T14:32:00Z" }), now)).toBe(false);
    expect(isAccountUsable(account({ state: "rate_limited", until: "2026-09-11T11:00:00Z" }), now)).toBe(true);
    expect(isAccountUsable(account({ state: "rate_limited", until: "garbage" }), now)).toBe(false);
  });

  it("labels every status for the card badge", () => {
    expect(statusLabel(account({ state: "disconnected" }))).toEqual({ label: "Not connected", tone: "neutral" });
    expect(statusLabel(account({ state: "connected" }, "ChatGPT Plus"))).toEqual({
      label: "ChatGPT Plus · connected",
      tone: "success",
    });
    expect(statusLabel(account({ state: "connected" })).label).toBe("Connected");
    expect(statusLabel(account({ state: "needs_reauth" })).tone).toBe("warning");
    expect(statusLabel(account({ state: "rate_limited", until: "2026-09-11T14:32:00Z", window: "5h" })).label).toMatch(
      /^5h limit · resets /,
    );
    expect(statusLabel(account({ state: "unavailable", reason: "policy_blocked" }))).toEqual({
      label: "Blocked by the provider",
      tone: "danger",
    });
    expect(unavailableReasonLabel("extra_usage_billing")).toBe("Paused to avoid extra-usage charges");
  });

  it("offers Connect from disconnected, expired and unavailable states only", () => {
    expect(canConnect({ state: "disconnected" })).toBe(true);
    expect(canConnect({ state: "needs_reauth" })).toBe(true);
    expect(canConnect({ state: "unavailable", reason: "fingerprint_drift" })).toBe(true);
    expect(canConnect({ state: "connected" })).toBe(false);
    expect(canConnect({ state: "connecting", flow: { kind: "browser", expiresAt: "2026-09-11T12:10:00Z" } })).toBe(false);
    expect(canConnect({ state: "rate_limited", until: "2026-09-11T14:32:00Z" })).toBe(false);
  });

  it("formats catalog ages coarsely", () => {
    const now = new Date("2026-09-11T12:00:00Z");
    expect(formatAge(undefined, now)).toBe("never");
    expect(formatAge("2026-09-11T11:59:50Z", now)).toBe("just now");
    expect(formatAge("2026-09-11T11:45:00Z", now)).toBe("15 min ago");
    expect(formatAge("2026-09-11T09:00:00Z", now)).toBe("3 h ago");
    expect(formatAge("2026-09-08T12:00:00Z", now)).toBe("3 d ago");
    expect(formatAge("not a date", now)).toBe("unknown");
  });
});
