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
  storage: "Couldn't save data",
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
    case "configure_provider":
      return {
        title,
        message,
        actionLabel: "Configure provider",
        action: () => (onOpenSettingsTab ? onOpenSettingsTab("ai") : bluey.window.open({ label: "settings", route: "ai" })),
      };
  }
}
