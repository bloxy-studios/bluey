//! Mirrors `src/lib/types/latency.ts` — the ⌘↵ fast-path trace (ADR 0010 §2,
//! `docs/LATENCY.md`). The merge rule and the percentile math live in
//! [`crate::latency`]; these are the wire shapes.

use serde::{Deserialize, Serialize};

/// One request's stage timestamps, in **milliseconds on Bluey's monotonic
/// clock** (ms since the app process started — never wall-clock). `None` means
/// the stage did not happen for this request or nobody observed it.
///
/// `t_shortcut` is the trigger moment: the global shortcut keydown when the ask
/// came from ⌘↵ / ⌘⇧↵, else the moment the HUD submitted the question.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LatencyTrace {
    pub request_id: String,
    /// `shortcut_capture`, `shortcut_generate`, `typed`, `follow_up`,
    /// `regenerate`, `detected_event`, `prepare`, `bench`, `internal`.
    pub trigger: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub t_shortcut: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub t_capture_done: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub t_snapshot_ready: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub t_retrieval_done: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub t_prompt_built: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub t_request_sent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub t_response_headers: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub t_first_token: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub t_first_paint: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub t_done: Option<f64>,
    /// The snapshot reply's IPC cost (round trip minus native build), measured by the WebView.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ipc_ms: Option<f64>,
    /// Decoded size of the screenshot that travelled with the request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_bytes: Option<u64>,
    /// Long edge of the screenshot in pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_px: Option<u32>,
    /// Provider-reported input tokens, else the engine's estimate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// Mirrors `TraceStamps` — the WebView's half of the trace. Times are
/// **offsets in ms relative to the anchor**: the moment the
/// `context_build_snapshot` reply arrived in the WebView (negative before it).
/// `anchor_ms`, `shortcut_ms` and `capture_done_ms` are absolute values on the
/// Rust clock, copied from what Rust handed over.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TraceStamps {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<String>,
    /// `SnapshotTrace::reply_ms` of the snapshot this ask used; `None` when no
    /// native snapshot was built (the request's arrival anchors instead).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor_ms: Option<f64>,
    /// Reply round trip minus the native build time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ipc_ms: Option<f64>,
    /// `shortcut.triggered.monoMs` when the ask came from a global shortcut.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shortcut_ms: Option<f64>,
    /// `SnapshotTrace::capture_done_ms`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_done_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_px: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask_started_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_ready_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retrieval_done_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_built_ms: Option<f64>,
    /// The moment `ai_stream` was invoked (the request's IPC starts here).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_invoked_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_paint_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done_ms: Option<f64>,
}

/// Mirrors `SnapshotTrace` — what the native snapshot builder observed, on the
/// Rust clock. Travels on `ContextSnapshot.trace` so the WebView can anchor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotTrace {
    pub started_ms: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_done_ms: Option<f64>,
    /// Stamped right before the reply leaves Rust.
    pub reply_ms: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_px: Option<u32>,
}

/// Mirrors `BenchRow` — one stage of a percentile table (cumulative ms since the trigger).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchRow {
    /// Stable id (`capture`, `snapshot_ready`, …).
    pub stage: String,
    pub label: String,
    pub samples: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p50_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p95_ms: Option<f64>,
}

/// Mirrors `BenchReport` — the `dev_bench_fast_path` result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchReport {
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Iterations that produced a trace (after the discarded warm-up runs).
    pub iterations: u32,
    pub discarded: u32,
    pub failures: u32,
    /// Whether a fixture screen stood in for the live capture.
    pub fixture: bool,
    pub rows: Vec<BenchRow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_bytes_p50: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_tokens_p50: Option<u32>,
    /// `request_sent` p50 / p95 — the local total the ADR gates on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_total_p50_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_total_p95_ms: Option<f64>,
    /// The table as the PR template wants it pasted.
    pub markdown: String,
    pub ran_at: String,
}

/// Mirrors `BenchOptions` — the `dev_bench_fast_path` arguments.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchOptions {
    /// Total runs; the first [`crate::latency::BENCH_WARMUP`] are discarded.
    pub iterations: u32,
    /// `mock` or a configured provider id.
    pub provider: String,
    /// A screen image (JPEG / PNG, or its base64 text) standing in for the live capture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixture: Option<String>,
}
