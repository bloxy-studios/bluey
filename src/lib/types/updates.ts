/**
 * In-app updates (mirrors `bluey_core::types::updates`, docs/UPDATES.md).
 *
 * Rust owns the cycle (check → available → download + install → ready →
 * relaunch) and publishes every transition as `update.status`; the WebView
 * mirrors it in `updatesStore` and renders it in the HUD pill and Settings →
 * General → Updates.
 */

import type { BlueyError } from "./errors";

/** `latest` follows stable releases; `nightly` follows the prerelease built from `main`. */
export type UpdateChannel = "latest" | "nightly";

export const UPDATE_CHANNELS: readonly UpdateChannel[] = ["latest", "nightly"];

export const UPDATE_CHANNEL_LABELS: Record<UpdateChannel, string> = {
  latest: "Latest",
  nightly: "Nightly",
};

export type UpdatePhase =
  | "idle"
  | "checking"
  | "up_to_date"
  /** A newer version exists (`available`); automatic mode moves on by itself. */
  | "available"
  | "downloading"
  /** Downloaded, verified and installed on disk — relaunch to run it. */
  | "ready"
  | "error";

export interface AvailableUpdate {
  version: string;
  channel: UpdateChannel;
  notes?: string;
  /** RFC 3339, from the feed's `pub_date`. */
  publishedAt?: string;
}

export interface UpdateProgress {
  downloaded: number;
  total?: number;
}

export interface UpdateStatus {
  phase: UpdatePhase;
  currentVersion: string;
  channel: UpdateChannel;
  automatic: boolean;
  /** False in builds that cannot replace themselves (dev builds, the browser mock). */
  supported: boolean;
  available?: AvailableUpdate;
  progress?: UpdateProgress;
  error?: BlueyError;
  /** RFC 3339 of the last completed check. */
  lastCheckedAt?: string;
}
