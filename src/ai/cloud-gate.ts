/**
 * Privacy → "Cloud AI" is the user's master switch for sending context to any
 * configured provider. The engine checks it before every model call; the UI
 * gets a configuration error whose recovery opens the Privacy tab.
 */

import type { BlueyError, Settings } from "@/lib/types";

export const CLOUD_AI_DISABLED_CODE = "privacy.cloud_ai_disabled";

export function cloudAiDisabledError(): BlueyError {
  return {
    kind: "configuration",
    code: CLOUD_AI_DISABLED_CODE,
    message: "Cloud AI is turned off in Privacy settings, so Bluey cannot ask a model right now.",
    recoverable: true,
    recovery: { type: "open_settings", tab: "privacy" },
  };
}

export function cloudAiAllowed(settings: Settings): boolean {
  return settings.privacy.cloudAiEnabled;
}

/** Throws the typed error when cloud AI is disabled. */
export function assertCloudAiAllowed(settings: Settings): void {
  if (!cloudAiAllowed(settings)) throw cloudAiDisabledError();
}
