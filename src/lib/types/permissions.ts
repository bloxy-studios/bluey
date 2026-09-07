/** Permission contract (mirrors `bluey_core::types::permissions`). */

import type { RecoveryAction } from "./errors";

export type PermissionStatus = "granted" | "denied" | "not_determined" | "restricted" | "unknown";

export interface PermissionState {
  microphone: PermissionStatus;
  screenRecording: PermissionStatus;
  accessibility: PermissionStatus;
  notifications: PermissionStatus;
  speechRecognition: PermissionStatus;
  checkedAt: string;
}

export interface CaptureProtection {
  supported: boolean;
  enabled: boolean;
  /** Honest description of what the platform guarantees. */
  note: string;
}

export interface SetupCheck {
  id: "screen" | "microphone" | "accessibility" | "ai" | "helper" | "systemAudio";
  label: string;
  ok: boolean;
  detail: string;
  /** What to do if it failed. */
  fix?: string;
  recovery?: RecoveryAction;
}
