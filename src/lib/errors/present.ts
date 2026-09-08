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

/** Turns a BlueyError into a friendly title/message + recovery action. */
export function presentError(error: BlueyError, options: ErrorPresenterOptions = {}): PresentedError {
  const { onRetry, onOpenSettingsTab } = options;
  const title = KIND_TITLES[error.kind] ?? "Something went wrong";
  const message = KIND_MESSAGES[error.kind] ?? error.message;

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
