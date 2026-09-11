/**
 * Pure `BlueyError` → friendly title / message / recovery action mapping.
 *
 * Shared by `useErrorPresenter` (React), the error toasts (`showErrorToast`) and
 * the HUD state pill, so every surface says the same thing about the same error
 * and offers the same single recovery button.
 */

import { bluey } from "@/lib/tauri/api";
import type { BlueyError } from "@/lib/types";

export interface PresentedError {
  title: string;
  message: string;
  /** Label for the recovery button, when the error is actionable. */
  actionLabel?: string;
  /** Runs the recovery. */
  action?: () => void | Promise<void>;
}

export interface ErrorPresenterOptions {
  /** Called for `retry` recoveries. */
  onRetry?: () => void | Promise<void>;
  /** Override for `open_settings` — e.g. switch tabs inside the Settings window. */
  onOpenSettingsTab?: (tab: string) => void;
}

const KIND_TITLES: Record<BlueyError["kind"], string> = {
  permission: "Permission needed",
  capture: "Screen capture failed",
  audio: "Audio problem",
  transcription: "Transcription problem",
  ai: "The AI request failed",
  storage: "Storage problem",
  authentication: "You're signed out",
  configuration: "Setup needed",
  sidecar: "Helper not responding",
  network: "Network problem",
  research: "Research failed",
  cancelled: "Cancelled",
  not_supported: "Not supported here",
  internal: "Something went wrong",
};

const KIND_MESSAGES: Partial<Record<BlueyError["kind"], string>> = {
  permission: "Bluey is missing a macOS permission it needs for this.",
  ai: "The model didn't answer. This is usually temporary.",
  network: "Bluey couldn't reach the network. Check your connection.",
  authentication: "Sign in again to keep using Bluey.",
  sidecar: "Bluey's native helper stopped. Restarting it usually fixes this.",
  configuration: "Something in Settings needs attention before this works.",
};

/** Code-specific copy (provider errors map to stable codes in Rust; see AI_ARCHITECTURE.md). */
const CODE_COPY: Record<string, { title: string; message: string }> = {
  "config.api_key_invalid": {
    title: "API key rejected",
    message:
      "Google AI Studio rejected the key. Create a new one at aistudio.google.com/apikey and paste it in Settings → AI.",
  },
  "config.model_not_found": {
    title: "Model not available",
    message: "This model isn't available for your key. Pick another model in Settings → AI → Models.",
  },
  "config.missing_key": {
    title: "API key missing",
    message: "This provider has no API key yet. Add one in Settings → AI.",
  },
  "config.no_model": {
    title: "No model assigned",
    message: "Assign a model to this role in Settings → AI → Models, or use the provider's recommended models.",
  },
  "config.unknown_provider": {
    title: "Provider not found",
    message: "This role points at a provider that no longer exists. Pick another provider in Settings → AI.",
  },
  "config.no_preset": {
    title: "No recommended models",
    message: "Bluey has no recommended models for this provider kind. Assign models per role in Settings → AI → Models.",
  },
  "ai.transcription_parse": {
    title: "Couldn't read the transcript",
    message: "The model returned a transcript Bluey couldn't parse. Try the import again.",
  },
  "transcription.no_speech": {
    title: "No speech found",
    message: "Bluey couldn't find any speech in this recording.",
  },
  "not_supported.transcribe_file": {
    title: "Can't transcribe files with this provider",
    message:
      "Only Google Gemini can transcribe recordings. Point Settings → AI → Models → Transcription at Google Gemini and import again.",
  },
  // Informational: cloud speech-to-text fell back to on-device Apple Speech.
  "audio.stt_fallback": {
    title: "Using Apple Speech",
    message: "Cloud transcription isn't available right now, so Bluey is transcribing on-device.",
  },
  "config.http_401": { title: "Credentials rejected", message: "The provider rejected the API key (HTTP 401)." },
  "config.http_403": {
    title: "Access denied",
    message: "The provider refused the request (HTTP 403). Check that the key has access to this model and region.",
  },
  "network.http_5xx": {
    title: "Provider unavailable",
    message: "The provider is having trouble right now. Try again in a minute.",
  },
  "network.timeout": { title: "Request timed out", message: "The provider took too long to answer. Try again." },
  "ai.invalid_request": {
    title: "Request rejected",
    message: "The provider rejected the request. Check the model id and options in Settings → AI.",
  },
  "privacy.cloud_ai_disabled": {
    title: "Cloud AI is off",
    message: "Turn Cloud AI back on in Settings → Privacy to let Bluey ask a model.",
  },
  // Subscription accounts (ADR 0009) — every stop signal names what Bluey did instead.
  "account.needs_reauth": {
    title: "Subscription sign-in expired",
    message: "Reconnect the account to keep using your plan. Bluey uses your API key meanwhile.",
  },
  "account.fingerprint_drift": {
    title: "Provider stopped recognising Bluey",
    message:
      "The provider changed how its own app talks to it, so Bluey paused this account rather than bill your extra usage. Your API key is used meanwhile; a fingerprint re-capture fixes it.",
  },
  "account.extra_usage_blocked": {
    title: "Paused to avoid extra-usage charges",
    message: "The provider started billing requests outside your plan. Bluey stopped and fell back to your API key.",
  },
  "account.policy_blocked": {
    title: "Account blocked by the provider",
    message: "The provider refused this account. Bluey stopped using it; your API key is used instead.",
  },
  "account.catalog_unavailable": {
    title: "Couldn't load the plan's models",
    message: "The provider's model list did not answer. Bluey keeps the last catalog it fetched; refresh from the account card later.",
  },
  "account.disabled": {
    title: "Subscription accounts are off",
    message: "Turn them on in Settings → AI → Accounts, or use an API key.",
  },
  "account.denied": {
    title: "Sign-in not completed",
    message: "The provider did not finish the sign-in. Try again from the account card.",
  },
  "account.not_connected": {
    title: "Account not connected",
    message: "Connect the account in Settings → AI → Accounts first.",
  },
  "account.import_not_found": {
    title: "No existing sign-in found",
    message: "Bluey found no sign-in of the official app on this Mac. Connect in the browser instead.",
  },
  "account.browser_open_failed": {
    title: "Couldn't open the browser",
    message: "Bluey could not open the sign-in page in your default browser. Try again, or copy the link from the account card.",
  },
  "account.unknown_provider": {
    title: "Unknown subscription provider",
    message: "Bluey only knows ChatGPT, Claude and Google AI subscriptions.",
  },
};

function formatRetry(ms: unknown): string {
  if (typeof ms !== "number" || !Number.isFinite(ms) || ms <= 0) return "";
  const seconds = Math.ceil(ms / 1000);
  return seconds >= 120 ? ` Retry in about ${Math.round(seconds / 60)} minutes.` : ` Retry in ${seconds}s.`;
}

/** Title + message for an error, from the most specific source available. */
export function describeError(error: BlueyError): { title: string; message: string } {
  const details = error.details ?? {};
  if (error.code === "network.http_429") {
    if (details.dailyQuota === true) {
      return {
        title: "Daily quota reached",
        message:
          "You've used today's free-tier quota. It resets at midnight Pacific — or enable billing in Google AI Studio.",
      };
    }
    return {
      title: "Rate limited",
      message: `The provider is rate-limiting requests.${formatRetry(details.retryAfterMs)}`,
    };
  }
  if (error.code.startsWith("ai.blocked_")) {
    const reason = error.code.slice("ai.blocked_".length).replace(/_/g, " ");
    return {
      title: "Answer refused",
      message: `The model declined to answer this request (${reason}). Rephrase and try again.`,
    };
  }
  if (error.code === "internal.invalid_params") {
    // Rust writes a specific, user-readable message for these ("unsupported recording format …",
    // "the recording is empty", "… larger than 2 GB", "`X` is not a valid shortcut"); keep it. The
    // import title is reserved for the recording checks so other invalid-parameter errors stay honest.
    return {
      title: /recording/i.test(error.message) ? "Can't import this file" : "Bluey couldn't do that",
      message: error.message,
    };
  }
  if (error.code === "account.rate_limited") {
    const window = typeof details.window === "string" && details.window ? ` ${details.window}` : "";
    const until = typeof details.until === "string" ? Date.parse(details.until) : Number.NaN;
    const resets = Number.isFinite(until)
      ? ` It resets at ${new Date(until).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}.`
      : "";
    return {
      title: "Plan limit reached",
      message: `Your subscription's${window} window is used up.${resets} Bluey uses your API key meanwhile.`,
    };
  }
  if (error.code === "account.provider_pending") {
    // Rust names the provider and the PR that lands it.
    return { title: "Not available yet", message: error.message };
  }
  const specific = CODE_COPY[error.code];
  if (specific) return specific;
  return {
    title: KIND_TITLES[error.kind] ?? "Something went wrong",
    message: KIND_MESSAGES[error.kind] ?? error.message,
  };
}

/** Turns a BlueyError into a friendly title/message + recovery action. */
export function presentError(error: BlueyError, options: ErrorPresenterOptions = {}): PresentedError {
  const { onRetry, onOpenSettingsTab } = options;
  const { title, message } = describeError(error);

  const recovery = error.recovery;
  if (!recovery || recovery.type === "none") return { title, message };

  switch (recovery.type) {
    case "open_system_settings":
      return {
        title,
        message,
        actionLabel: "Open System Settings",
        action: () => bluey.permissions.openSettings({ kind: recovery.pane }),
      };
    case "open_settings":
      return {
        title,
        message,
        actionLabel: "Open Settings",
        action: () =>
          onOpenSettingsTab ? onOpenSettingsTab(recovery.tab) : bluey.window.open({ label: "settings", route: recovery.tab }),
      };
    case "retry":
      return onRetry ? { title, message, actionLabel: "Retry", action: onRetry } : { title, message };
    case "sign_in":
      return {
        title,
        message,
        actionLabel: "Sign in",
        action: () => bluey.window.open({ label: "settings", route: "profile" }),
      };
    case "restart_helper":
      return {
        title,
        message,
        actionLabel: "Restart helper",
        action: () => bluey.dev.restartHelper(),
      };
    case "reconnect_account":
      return {
        title,
        message,
        actionLabel: "Reconnect",
        action: async () => {
          await bluey.accounts.connect({ providerId: recovery.providerId });
        },
      };
    case "use_api_key":
      return {
        title,
        message,
        actionLabel: "Use API key instead",
        action: () => (onOpenSettingsTab ? onOpenSettingsTab("ai") : bluey.window.open({ label: "settings", route: "ai" })),
      };
    case "configure_provider":
      return {
        title,
        message,
        actionLabel: "Configure provider",
        action: () => (onOpenSettingsTab ? onOpenSettingsTab("ai") : bluey.window.open({ label: "settings", route: "ai" })),
      };
  }
}
