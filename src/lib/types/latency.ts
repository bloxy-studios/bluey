/**
 * The ⌘↵ fast-path trace (ADR 0010 §2, `docs/LATENCY.md`). Mirrors
 * `bluey_core::types::latency`.
 *
 * Every `t*` value is **milliseconds on Bluey's monotonic clock** (ms since the
 * Rust process started). The WebView cannot read that clock, so it reports
 * `TraceStamps` — offsets relative to the moment the `context_build_snapshot`
 * reply arrived — and Rust merges the halves by `requestId`.
 */

export interface LatencyTrace {
  requestId: string;
  /** `shortcut_capture`, `shortcut_generate`, `typed`, `follow_up`, `regenerate`, `detected_event`, `prepare`, `bench`, `internal`. */
  trigger: string;
  /** The trigger moment: the global shortcut keydown, else the HUD submit. */
  tShortcut?: number;
  tCaptureDone?: number;
  tSnapshotReady?: number;
  tRetrievalDone?: number;
  tPromptBuilt?: number;
  tRequestSent?: number;
  tResponseHeaders?: number;
  tFirstToken?: number;
  tFirstPaint?: number;
  tDone?: number;
  /** The snapshot reply's IPC cost (round trip minus native build), measured here. */
  ipcMs?: number;
  /** Decoded size of the screenshot that travelled with the request. */
  imageBytes?: number;
  /** Long edge of the screenshot in pixels. */
  imagePx?: number;
  promptTokens?: number;
  providerId?: string;
  model?: string;
}

/** The WebView's half: offsets (ms) relative to the anchor; absolute Rust-clock values where noted. */
export interface TraceStamps {
  trigger?: string;
  /** `SnapshotTrace.replyMs` of the snapshot this ask used (Rust clock). */
  anchorMs?: number;
  ipcMs?: number;
  /** `shortcut.triggered.monoMs` (Rust clock) when the ask came from a global shortcut. */
  shortcutMs?: number;
  /** `SnapshotTrace.captureDoneMs` (Rust clock). */
  captureDoneMs?: number;
  imageBytes?: number;
  imagePx?: number;
  askStartedMs?: number;
  snapshotReadyMs?: number;
  retrievalDoneMs?: number;
  promptBuiltMs?: number;
  /** The moment `ai_stream` was invoked. */
  streamInvokedMs?: number;
  firstPaintMs?: number;
  doneMs?: number;
}

/** What the native snapshot builder observed, on the Rust clock (`ContextSnapshot.trace`). */
export interface SnapshotTrace {
  startedMs: number;
  captureDoneMs?: number;
  /** Stamped right before the reply left Rust — the WebView's anchor. */
  replyMs: number;
  imageBytes?: number;
  imagePx?: number;
}

export type BenchStage =
  | "capture"
  | "snapshot_ready"
  | "retrieval"
  | "prompt_built"
  | "request_sent"
  | "response_headers"
  | "first_token"
  | "first_paint"
  | "done";

export interface BenchRow {
  stage: BenchStage | string;
  label: string;
  samples: number;
  p50Ms?: number;
  p95Ms?: number;
}

export interface BenchReport {
  provider: string;
  model?: string;
  iterations: number;
  discarded: number;
  failures: number;
  fixture: boolean;
  rows: BenchRow[];
  imageBytesP50?: number;
  promptTokensP50?: number;
  localTotalP50Ms?: number;
  localTotalP95Ms?: number;
  /** The table as the PR template wants it pasted. */
  markdown: string;
  ranAt: string;
}

export interface BenchOptions {
  iterations: number;
  /** `mock` or a configured provider id. */
  provider: string;
  /** A screen image (JPEG / PNG, or its base64 text) standing in for the live capture. */
  fixture?: string;
}
