/**
 * Global error surfacing: backend errors that do not belong to a specific UI
 * control (`app.error`, `audio.error`, helper crashes) become error toasts with
 * their single recovery button. Per-request AI failures stay inline in the chat
 * turn, and the HUD state pill separately mirrors `AppStatus.error`.
 */

import { showErrorToast, showToast } from "@/components/ui/toast-store";
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

export function startErrorSurface(): Unlisten {
  const offApp = eventBus.on("app.error", (error) => showErrorToast(error));
  const offAudio = eventBus.on("audio.error", (error) => showErrorToast(error));
  const offHelper = eventBus.on("helper.status", (status) => {
    if (status.error) showErrorToast(status.error);
    else if (!status.running) showErrorToast(HELPER_STOPPED_ERROR);
    else if (status.restarted) showToast("Helper restarted", 2000);
  });
  return () => {
    offApp();
    offAudio();
    offHelper();
  };
}
