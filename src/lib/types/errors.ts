/**
 * Typed error contract shared by Rust (`bluey_core::error::BlueyError`) and TS.
 * Rust serializes with `#[serde(rename_all = "camelCase")]` into exactly this shape.
 */

export type BlueyErrorKind =
  | "permission"
  | "capture"
  | "audio"
  | "transcription"
  | "ai"
  | "storage"
  | "authentication"
  | "configuration"
  | "sidecar"
  | "network"
  | "research"
  | "cancelled"
  | "not_supported"
  | "internal";

export type RecoveryAction =
  | { type: "open_system_settings"; pane: PermissionKind }
  | { type: "open_settings"; tab: string }
  | { type: "retry" }
  | { type: "sign_in" }
  | { type: "restart_helper" }
  | { type: "configure_provider" }
  | { type: "none" };

export type PermissionKind = "microphone" | "screenRecording" | "accessibility" | "notifications" | "speechRecognition";

export interface BlueyError {
  kind: BlueyErrorKind;
  /** Stable machine-readable code, e.g. "capture.permission_denied". */
  code: string;
  /** Technical message (safe to log; never contains secrets or user content). */
  message: string;
  /** Whether the user can do something about it. */
  recoverable: boolean;
  /** Suggested recovery. */
  recovery?: RecoveryAction;
  /** Extra structured detail (never secrets). */
  details?: Record<string, unknown>;
}

export function isBlueyError(value: unknown): value is BlueyError {
  return (
    typeof value === "object" &&
    value !== null &&
    "kind" in value &&
    "code" in value &&
    "message" in value
  );
}

export function toBlueyError(value: unknown, fallbackKind: BlueyErrorKind = "internal"): BlueyError {
  if (isBlueyError(value)) return value;
  if (value instanceof Error) {
    return { kind: fallbackKind, code: `${fallbackKind}.unexpected`, message: value.message, recoverable: false };
  }
  return {
    kind: fallbackKind,
    code: `${fallbackKind}.unexpected`,
    message: typeof value === "string" ? value : "Unknown error",
    recoverable: false,
  };
}
