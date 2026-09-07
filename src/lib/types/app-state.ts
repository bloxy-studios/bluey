/**
 * Application state machine contract.
 *
 * The Rust backend owns the authoritative state machine (`bluey_core::state`).
 * The frontend mirrors it via the `app.state` event and never invents its own
 * `isLoading` / `isThinking` booleans. `audioActive` is the single orthogonal
 * region: an audio session can be running while a capture/think cycle happens.
 */

import type { BlueyError } from "./errors";

export type AppState =
  | "booting"
  | "auth_required"
  | "ready"
  | "listening"
  | "capturing"
  | "analyzing"
  | "thinking"
  | "response_ready"
  | "error"
  | "paused";

export interface AppStatus {
  state: AppState;
  /** True while an audio session is running (orthogonal to the primary state). */
  audioActive: boolean;
  /** Active session id, if a session is running. */
  sessionId?: string;
  /** Active mode id (global default or session override). */
  modeId: string;
  /** Present when `state === "error"`. */
  error?: BlueyError;
  /** State we will return to after `error`/`paused` is resolved. */
  resumeState?: AppState;
  /** ISO timestamp of the last transition. */
  updatedAt: string;
}

/** Events accepted by the state machine (mirrors `bluey_core::state::AppEvent`). */
export type AppStateEvent =
  | { type: "boot_completed"; authenticated: boolean }
  | { type: "authenticated" }
  | { type: "signed_out" }
  | { type: "audio_started" }
  | { type: "audio_stopped" }
  | { type: "capture_started" }
  | { type: "capture_finished" }
  | { type: "analysis_started" }
  | { type: "thinking_started" }
  | { type: "response_ready" }
  | { type: "response_dismissed" }
  | { type: "failed"; error: BlueyError }
  | { type: "recovered" }
  | { type: "paused" }
  | { type: "resumed" }
  | { type: "session_changed"; sessionId?: string }
  | { type: "mode_changed"; modeId: string };

export const IDLE_STATES: ReadonlySet<AppState> = new Set<AppState>(["ready", "listening"]);
export const BUSY_STATES: ReadonlySet<AppState> = new Set<AppState>([
  "capturing",
  "analyzing",
  "thinking",
]);
