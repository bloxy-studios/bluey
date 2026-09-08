/**
 * Session controls shared by the HUD session menu (and anything else that
 * needs them). Every action is guarded: failures become error toasts instead
 * of unhandled rejections. Pausing a session also pauses the audio pipeline so
 * nothing is transcribed meanwhile; resuming restarts it.
 */

import { showErrorToast } from "@/components/ui/toast-store";
import { bluey } from "@/lib/tauri/api";
import { toBlueyError } from "@/lib/types";
import { useAppStore } from "@/stores/appStore";

async function guarded(run: () => Promise<unknown>): Promise<boolean> {
  try {
    await run();
    return true;
  } catch (error) {
    showErrorToast(toBlueyError(error, "storage"));
    return false;
  }
}

function audioActive(): boolean {
  return useAppStore.getState().status?.audioActive ?? false;
}

export function startSession(title?: string): Promise<boolean> {
  return guarded(() => bluey.session.start(title ? { title } : {}));
}

export function pauseSession(): Promise<boolean> {
  return guarded(async () => {
    await bluey.session.pause();
    if (audioActive()) await bluey.audio.pause();
  });
}

export function resumeSession(): Promise<boolean> {
  return guarded(async () => {
    await bluey.session.resume();
    if (audioActive()) await bluey.audio.resume();
  });
}

export function endSession(): Promise<boolean> {
  return guarded(() => bluey.session.end());
}

export function openSessionHistory(): Promise<boolean> {
  return guarded(() => bluey.window.open({ label: "settings", route: "sessions" }));
}
