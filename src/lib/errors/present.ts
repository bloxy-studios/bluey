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

/** A provider answered HTTP 400. Rust usually quotes the provider's reason — see `describeError`. */
const REQUEST_REJECTED = {
  title: "Request rejected",
  message: "The provider rejected the request. Check the model id and options in Settings → AI.",
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
  // Google AI (Antigravity) sign-in needs Google's desktop-app OAuth client secret compiled in;
  // without it Rust stops before the browser opens (docs/PROVIDER_ACCOUNTS.md › Google AI).
  "config.antigravity_client_secret": {
    title: "Google sign-in isn't set up in this build",
    message:
      "This build has no Antigravity OAuth client secret, so Bluey can't open the Google sign-in. Add BLUEY_ANTIGRAVITY_CLIENT_SECRET to .env.local and rebuild (docs/PROVIDER_ACCOUNTS.md › Google AI), or keep using your Gemini API key.",
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
  "not_supported.link_scheme": {
    title: "Can't open this link",
    message: "Bluey only opens web (http/https) and email links.",
  },
  "internal.open_link": {
    title: "Couldn't open the link",
    message: "Your default browser didn't open it. Copy the link and open it yourself.",
  },
  // Informational: cloud speech-to-text fell back to on-device Apple Speech.
  "audio.stt_fallback": {
    title: "Using Apple Speech",
    message: "Cloud transcription isn't available right now, so Bluey is transcribing on-device.",
  },
  // The Apple route promises on-device; the locale has no on-device model.
  "audio.speech_server": {
    title: "Transcribing on Apple's servers",
    message:
      "This Mac has no on-device speech model for your language, so Apple Speech sends audio to Apple to transcribe it.",
  },
  // Cloud speech-to-text lost its connection; it reconnects on its own.
  "audio.stt_degraded": {
    title: "Reconnecting transcription",
    message: "Bluey lost its connection to cloud transcription and is reconnecting. Speech in the meantime isn't transcribed.",
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
  "ai.invalid_request": REQUEST_REJECTED,
  // The reply never became an answer (`src/lib/errors/answers.ts`); regenerating is the recovery.
  "ai.truncated": {
    title: "Answer was cut short",
    message: "The model ran out of room before finishing, even with extra space. Regenerate for the full answer.",
  },
  "ai.unreadable_output": {
    title: "Couldn't read the answer",
    message: "The model's reply wasn't in a form Bluey can show. Regenerate to try again.",
  },
  // In-app updates (docs/UPDATES.md) — the current version keeps running in every case.
  "ai.prepare_failed": {
    title: "Couldn't prepare a suggestion",
    message: "Bluey heard the question but no answer came through. Regenerate to try again.",
  },
  "update.check_failed": {
    title: "Couldn't check for updates",
    message: "Bluey couldn't reach the update feed. It tries again in a few hours, or use Check now in Settings → General.",
  },
  "update.install_failed": {
    title: "Update couldn't be installed",
    message: "The download or install failed and the current version keeps running. Try again from Settings → General → Updates.",
  },
  "update.unsupported": {
    title: "Updates aren't available in this build",
    message: "Development builds don't update themselves. Install a release from the download page.",
  },
  "update.nothing_pending": {
    title: "Nothing to install",
    message: "No update has been found yet. Use Check now in Settings → General → Updates.",
  },
  "privacy.cloud_ai_disabled": {
    title: "Cloud AI is off",
    message: "Turn Cloud AI back on in Settings → Privacy to let Bluey ask a model.",
  },
  // Subscription accounts (ADR 0009) — every stop signal names what Bluey did instead.
  "account.needs_reauth": {
    title: "Subscription sign-in expired",
    message: "Reconnect the account to keep using your plan.",
  },
  "account.fingerprint_drift": {
    title: "Provider stopped recognising Bluey",
    message:
      "The provider changed how its own app talks to it, so Bluey paused this account rather than bill your extra usage. A fingerprint re-capture fixes it.",
  },
  "account.extra_usage_blocked": {
    title: "Paused to avoid extra-usage charges",
    message: "The provider started billing requests outside your plan, so Bluey stopped using this account.",
  },
  "account.policy_blocked": {
    title: "Account blocked by the provider",
    message: "The provider refused this account. Bluey stopped using it.",
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
  "auth.sign_in_required": {
    title: "Sign in first",
    message: "Sign in to Bluey before you start listening.",
  },
  "account.not_connected": {
    title: "Account not connected",
    message: "Connect the account in Settings → AI → Accounts first.",
  },
  "account.import_not_found": {
    title: "No existing sign-in found",
    message: "Bluey found no sign-in of the official app on this Mac. Connect in the browser instead.",
  },
  // Keychain (ADR 0011): a saved credential exists but macOS wants the user's approval — after an
  // update or a rebuild Bluey is a new app to the Keychain until it is allowed again.
  "storage.keychain_access_denied": {
    title: "macOS blocked a saved credential",
    message:
      "Bluey changed since this credential was saved, so macOS asks again. Retry and choose Always Allow — or re-enter the key in Settings.",
  },
  "storage.keychain_interaction_not_allowed": {
    title: "Credential needs your approval",
    message:
      "macOS wants your approval before Bluey uses a saved credential. Open Settings → Privacy → Saved credentials and choose Allow access.",
  },
  "storage.keychain_unavailable": {
    title: "Keychain unavailable",
    message: "Bluey can't reach your login keychain. Unlock your Mac's login keychain, then try again.",
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

/** Rust's messages start lowercase ("the Gemini API rejected …"); toasts read as sentences. */
function sentenceCase(text: string): string {
  return text.length > 0 ? text.charAt(0).toUpperCase() + text.slice(1) : text;
}

function formatRetry(ms: unknown): string {
  if (typeof ms !== "number" || !Number.isFinite(ms) || ms <= 0) return "";
  const seconds = Math.ceil(ms / 1000);
  return seconds >= 120 ? ` Retry in about ${Math.round(seconds / 60)} minutes.` : ` Retry in ${seconds}s.`;
}

/** Title + message for an error, from the most specific source available. */
function describeCopy(error: BlueyError): { title: string; message: string } {
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
      message: `Your subscription's${window} window is used up.${resets}`,
    };
  }
  if (error.code === "account.provider_pending") {
    // Rust names the provider and the PR that lands it.
    return { title: "Not available yet", message: error.message };
  }
  if (error.code === "account.needs_reauth" && details.imported === true) {
    // Claude Code / Codex rotate refresh tokens: Bluey never refreshes an imported session.
    return {
      title: "Imported sign-in expired",
      message:
        "Import the sign-in again from the official app (macOS asks once to share it), or reconnect in the browser.",
    };
  }
  if (error.code === "account.import_denied") {
    // Rust names the app whose sign-in macOS refused to share ("… reading Claude Code's sign-in …").
    return { title: "macOS blocked the import", message: error.message };
  }
  if (error.code === "config.model_not_found" && typeof details.model === "string") {
    // Azure / OpenAI-compatible / Anthropic name the model or deployment they could not find.
    return {
      title: "Model not available",
      message: `"${details.model}" isn't available on this provider. Check the model or deployment name in Settings → AI.`,
    };
  }
  if (error.code === "ai.invalid_request") {
    // Rust names the provider and quotes its reason ("ChatGPT rejected the request: Invalid
    // schema …") — that is what fixes the request, so it outranks the generic copy.
    const detail = error.message.trim();
    return {
      title: REQUEST_REJECTED.title,
      message: detail.length > 0 ? sentenceCase(detail) : REQUEST_REJECTED.message,
    };
  }
  const specific = CODE_COPY[error.code];
  if (specific) return specific;
  return {
    title: KIND_TITLES[error.kind] ?? "Something went wrong",
    message: KIND_MESSAGES[error.kind] ?? error.message,
  };
}

/** Account stop signals: what Bluey does instead depends on whether an API-key provider stands in. */
const ACCOUNT_STOP_CODES = new Set([
  "account.needs_reauth",
  "account.rate_limited",
  "account.fingerprint_drift",
  "account.extra_usage_blocked",
  "account.policy_blocked",
]);

/** `config.provider_unusable`: a role IS assigned, but Rust names why its provider can't serve it. */
function describeUnusableProvider(details: Record<string, unknown>): { title: string; message: string } {
  const name = typeof details.providerName === "string" ? details.providerName : "This provider";
  const role = typeof details.role === "string" ? `the ${details.role} role` : "this role";
  const cause = typeof details.cause === "string" ? details.cause : "";
  const noStandIn = "and no API-key provider can stand in";
  if (cause === "missing_key") {
    return {
      title: "API key missing",
      message: `${name} has no API key yet, so ${role} can't answer. Add the key in Settings → AI, or assign the role to another provider.`,
    };
  }
  if (cause === "disabled") {
    return {
      title: "Provider turned off",
      message: `${name} is turned off, but ${role} still uses it. Turn it back on or pick another provider in Settings → AI.`,
    };
  }
  if (cause === "not_configured") {
    return {
      title: "Provider not found",
      message: `${role.charAt(0).toUpperCase()}${role.slice(1)} points at a provider that no longer exists. Pick another provider in Settings → AI.`,
    };
  }
  if (cause === "account_needs_reauth") {
    return {
      title: "Subscription sign-in expired",
      message: `${name} needs you to sign in again, ${noStandIn}. Reconnect it or add an API key in Settings → AI.`,
    };
  }
  if (cause === "account_rate_limited") {
    return {
      title: "Plan limit reached",
      message: `${name}'s plan window is used up, ${noStandIn}. Add an API key in Settings → AI, or wait for the reset.`,
    };
  }
  if (cause.startsWith("account_")) {
    return {
      title: "Subscription account unavailable",
      message: `${name} can't answer right now, ${noStandIn}. Reconnect it or add an API key in Settings → AI.`,
    };
  }
  return {
    title: "Provider can't be used",
    message: `${name} can't serve this request. Pick another provider for ${role} in Settings → AI.`,
  };
}

export function describeError(error: BlueyError): { title: string; message: string } {
  const details = error.details ?? {};
  if (error.code === "config.provider_unusable") return describeUnusableProvider(details);
  const copy = describeCopy(error);
  if (!ACCOUNT_STOP_CODES.has(error.code)) return copy;
  // Promise the API key only when Rust found one that now answers instead (PROV-001).
  const fallback =
    typeof details.fallbackProviderId === "string"
      ? " Bluey uses your API key meanwhile."
      : " Add an API key in Settings → AI so Bluey can keep answering meanwhile.";
  return { title: copy.title, message: `${copy.message}${fallback}` };
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
