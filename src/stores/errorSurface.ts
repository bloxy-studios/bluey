/**
 * Global error surfacing: backend errors that do not belong to a specific UI
 * control (`app.error`, `audio.error`, helper crashes) become error toasts with
 * their single recovery button. Per-request AI failures stay inline in the chat
 * turn, and the HUD state pill separately mirrors `AppStatus.error`.
 */

import { showErrorToast, showToast, useToastStore } from "@/components/ui/toast-store";
import { presentError } from "@/lib/errors/present";
import { eventBus } from "@/lib/tauri/event-bus";
import type { Unlisten } from "@/lib/tauri/transport";
import type { BlueyError } from "@/lib/types";

export const HELPER_STOPPED_ERROR: BlueyError = {
  kind: "sidecar",
  code: "sidecar.helper_stopped",
  message: "The native helper is not running.",
  recoverable: true,
  recovery: { type: "restart_helper" },
};

/**
 * Expected changes of transcription route, not failures: listening fell back
 * to on-device Apple Speech (no cloud key, Privacy → Cloud AI off), or Apple
 * Speech has no on-device model for the locale and uses Apple's servers.
 */
const AUDIO_NOTICE_CODES = new Set(["audio.stt_fallback", "audio.speech_server"]);
const NOTICE_DURATION_MS = 4000;

/** An info notice, and each reason only once per app run rather than on every start. */
function showAudioNotice(error: BlueyError, shown: Set<string>): void {
  if (shown.has(error.message)) return;
  shown.add(error.message);
  useToastStore.getState().push({
    message: presentError(error).message,
    variant: "info",
    key: error.code,
    durationMs: NOTICE_DURATION_MS,
  });
}

export function startErrorSurface(): Unlisten {
  const shownNotices = new Set<string>();
  const offApp = eventBus.on("app.error", (error) => showErrorToast(error));
  const offAudio = eventBus.on("audio.error", (error) => {
    if (AUDIO_NOTICE_CODES.has(error.code)) showAudioNotice(error, shownNotices);
    else showErrorToast(error);
  });
  const offHelper = eventBus.on("helper.status", (status) => {
    if (status.error) showErrorToast(status.error);
    // Exited, and the supervisor is already starting a replacement.
    else if (!status.running && status.restarted) showToast("Restarting the native helper…", 2000);
    else if (!status.running) showErrorToast(HELPER_STOPPED_ERROR);
    else if (status.restarted) showToast("Helper restarted", 2000);
  });
  return () => {
    offApp();
    offAudio();
    offHelper();
  };
}
