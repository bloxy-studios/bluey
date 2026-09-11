/**
 * Copy for the subscription-account cards and consent dialogs (ADR 0009).
 * The consent paragraphs are the text recorded in `docs/PROVIDER_ACCOUNTS.md`
 * ("Consent copy") — change them there first. Four facts, always in this
 * order: what is sent · whose limits are used · unofficial · what Bluey does
 * when it stops working.
 */

import type { AccountStatus, ProviderAccount, SubscriptionProviderId, UnavailableReason } from "@/lib/types";

export interface ProviderCopy {
  id: SubscriptionProviderId;
  name: string;
  vendor: string;
  /** Plans the sign-in draws on. */
  plans: string;
  /** The official client whose local sign-in "Import" reads. */
  importLabel: string;
  consent: {
    title: string;
    paragraphs: string[];
    confirm: string;
  };
}

export const PROVIDER_COPY: Record<SubscriptionProviderId, ProviderCopy> = {
  chatgpt: {
    id: "chatgpt",
    name: "ChatGPT",
    vendor: "OpenAI",
    plans: "Free · Plus · Pro",
    importLabel: "Import Codex CLI sign-in",
    consent: {
      title: "Use your ChatGPT subscription with Bluey?",
      paragraphs: [
        "Bluey signs you in to ChatGPT in your browser and then talks to OpenAI the way the Codex CLI does. It sends the questions you ask and the screenshots you choose to send with ⌘↵ — nothing else, nothing automatically.",
        "Usage counts against your ChatGPT plan's Codex limits.",
        "This is not an official API: OpenAI may change or stop it without notice.",
        "If that happens, Bluey stops using this account, tells you, and falls back to your API key.",
      ],
      confirm: "Continue in browser",
    },
  },
  claude: {
    id: "claude",
    name: "Claude",
    vendor: "Anthropic",
    plans: "Pro · Max",
    importLabel: "Import Claude Code sign-in",
    consent: {
      title: "Use your Claude subscription with Bluey?",
      paragraphs: [
        "Bluey signs you in to Claude in your browser and then talks to Anthropic the way Claude Code does. It sends the questions you ask and the screenshots you choose to send with ⌘↵.",
        "Usage counts against your Claude plan's limits (5-hour and weekly windows).",
        "This is not an official API: Anthropic's terms say these sign-in tokens are for Claude Code and its own apps only, and since April 2026 it bills requests it does not recognise as Claude Code to paid Extra usage instead of your plan.",
        "Bluey treats the first such response as a stop signal — it will not keep sending, it tells you, and it falls back to your API key.",
      ],
      confirm: "Continue in browser",
    },
  },
  antigravity: {
    id: "antigravity",
    name: "Google AI",
    vendor: "Google",
    plans: "AI Pro · AI Ultra",
    importLabel: "Import Antigravity sign-in",
    consent: {
      title: "Use your Google AI subscription with Bluey?",
      paragraphs: [
        "Bluey signs you in with Google in your browser and then talks to Google the way the Antigravity IDE does. It sends the questions you ask and the screenshots you choose to send with ⌘↵.",
        "Usage counts against your Google AI plan's Antigravity quota.",
        "This is not an official API, and Google's Antigravity terms (read 2026-09-11) say: “Using third party software, tools, or services to access the Service (e.g. using OpenClaw with Antigravity OAuth) is a breach of this Agreement. Such actions may be grounds for suspension or termination of your Antigravity and/or Gemini CLI accounts.” Accounts have been suspended this way since February 2026. This is your account and your call.",
        "At the first sign of a block Bluey stops using every Google account you connected, tells you, and falls back to your Gemini API key.",
      ],
      confirm: "Continue in browser",
    },
  },
};

export const SUBSCRIPTION_PROVIDER_ORDER: SubscriptionProviderId[] = ["chatgpt", "claude", "antigravity"];

export function providerCopy(providerId: string): ProviderCopy | undefined {
  return (PROVIDER_COPY as Record<string, ProviderCopy>)[providerId];
}

export type StatusTone = "neutral" | "pending" | "success" | "warning" | "danger";

export function unavailableReasonLabel(reason: UnavailableReason): string {
  switch (reason) {
    case "fingerprint_drift":
      return "Provider stopped recognising Bluey";
    case "policy_blocked":
      return "Blocked by the provider";
    case "catalog_unavailable":
      return "Model list unavailable";
    case "extra_usage_billing":
      return "Paused to avoid extra-usage charges";
    case "provider_pending":
      return "Not available in this version yet";
    case "disabled":
      return "Subscription accounts are off";
    case "other":
      return "Unavailable";
  }
}

/** Short time for a reset instant, in the user's locale (`14:32`). */
export function formatResetTime(iso: string): string {
  const time = Date.parse(iso);
  if (!Number.isFinite(time)) return "later";
  return new Date(time).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

/** One-line status for the card's badge. */
export function statusLabel(account: ProviderAccount): { label: string; tone: StatusTone } {
  const { status, identity } = account;
  switch (status.state) {
    case "disconnected":
      return { label: "Not connected", tone: "neutral" };
    case "connecting":
      return { label: "Connecting…", tone: "pending" };
    case "connected":
      return { label: identity?.planLabel ? `${identity.planLabel} · connected` : "Connected", tone: "success" };
    case "needs_reauth":
      return { label: "Sign in again", tone: "warning" };
    case "rate_limited":
      return {
        label: `${status.window ? `${status.window} limit` : "Plan limit"} · resets ${formatResetTime(status.until)}`,
        tone: "warning",
      };
    case "unavailable":
      return { label: unavailableReasonLabel(status.reason), tone: "danger" };
  }
}

/** Whether the card offers "Connect" (as opposed to Cancel / Disconnect). */
export function canConnect(status: AccountStatus): boolean {
  return status.state === "disconnected" || status.state === "needs_reauth" || status.state === "unavailable";
}

/** Relative age of a timestamp, coarse enough for a settings card. */
export function formatAge(iso: string | undefined, now: Date = new Date()): string {
  if (!iso) return "never";
  const at = Date.parse(iso);
  if (!Number.isFinite(at)) return "unknown";
  const seconds = Math.max(0, Math.round((now.getTime() - at) / 1000));
  if (seconds < 60) return "just now";
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 48) return `${hours} h ago`;
  return `${Math.round(hours / 24)} d ago`;
}
