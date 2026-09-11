//! The ⌘↵ fast-path trace, pure (ADR 0010 §2, `docs/LATENCY.md`): how the Rust
//! half and the WebView half of a request's timestamps merge into one
//! [`LatencyTrace`], the cumulative stage milestones, nearest-rank percentiles
//! and the bench table the fast-path PRs paste.
//!
//! **Clocks.** Rust stamps everything it observes on one monotonic clock (ms
//! since the process started). The WebView cannot read that clock, so it
//! reports *offsets* relative to an anchor it shares with Rust: the moment the
//! `context_build_snapshot` reply arrived — Rust stamped `reply_ms` right before
//! returning it, the WebView adds the reply's IPC cost it measured. Asks without
//! a native snapshot anchor on the request's arrival instead. No wall clock is
//! involved on either side.

use crate::types::{BenchReport, BenchRow, LatencyTrace, TraceStamps};

/// Warm-up runs a bench discards before counting.
pub const BENCH_WARMUP: u32 = 3;
/// Traces the dev overlay keeps for its percentiles.
pub const OVERLAY_WINDOW: usize = 50;

/// What Rust observed about one request, on the Rust clock.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RustStamps {
    /// `AiManager::stream` entry — the request reached Rust.
    pub request_received: Option<f64>,
    /// The adapter handed the request to the network.
    pub request_sent: Option<f64>,
    /// The provider's response headers arrived.
    pub response_headers: Option<f64>,
    /// The first text delta arrived.
    pub first_token: Option<f64>,
    /// The stream finished (Rust side).
    pub stream_done: Option<f64>,
}

/// Merge the WebView stamps (offsets) with the Rust stamps into one trace.
///
/// The anchor is `stamps.anchor_ms + stamps.ipc_ms` (the reply's arrival, on the
/// Rust clock); without a snapshot anchor the request's arrival stands in
/// (`request_received - stream_invoked_ms`, treating the request's own IPC as
/// zero). `t_shortcut` is the shortcut moment when there was one, else the
/// moment the ask started.
pub fn merge(
    request_id: &str,
    stamps: Option<&TraceStamps>,
    rust: &RustStamps,
    provider_id: Option<&str>,
    model: Option<&str>,
    prompt_tokens: Option<u32>,
) -> LatencyTrace {
    let anchor = stamps.and_then(|s| {
        s.anchor_ms
            .map(|a| a + s.ipc_ms.unwrap_or(0.0).max(0.0))
            .or_else(|| {
                rust.request_received
                    .zip(s.stream_invoked_ms)
                    .map(|(received, invoked)| received - invoked)
            })
    });
    let abs = |offset: Option<f64>| anchor.zip(offset).map(|(a, o)| a + o);
    let s = stamps;
    let t_shortcut = s
        .and_then(|s| s.shortcut_ms)
        .or_else(|| abs(s.and_then(|s| s.ask_started_ms)));
    LatencyTrace {
        request_id: request_id.to_string(),
        trigger: s
            .and_then(|s| s.trigger.clone())
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| "internal".to_string()),
        t_shortcut,
        t_capture_done: s.and_then(|s| s.capture_done_ms),
        t_snapshot_ready: abs(s.and_then(|s| s.snapshot_ready_ms)),
        t_retrieval_done: abs(s.and_then(|s| s.retrieval_done_ms)),
        t_prompt_built: abs(s.and_then(|s| s.prompt_built_ms)),
        t_request_sent: rust.request_sent,
        t_response_headers: rust.response_headers,
        t_first_token: rust.first_token,
        t_first_paint: abs(s.and_then(|s| s.first_paint_ms)),
        t_done: abs(s.and_then(|s| s.done_ms)).or(rust.stream_done),
        ipc_ms: s.and_then(|s| s.ipc_ms),
        image_bytes: s.and_then(|s| s.image_bytes),
        image_px: s.and_then(|s| s.image_px),
        prompt_tokens,
        provider_id: provider_id.map(str::to_string),
        model: model.map(str::to_string),
    }
}

/// Fold the WebView's late stamps (first paint, done) into an existing trace.
pub fn apply_late_stamps(trace: &mut LatencyTrace, stamps: &TraceStamps) {
    let anchor = stamps
        .anchor_ms
        .map(|a| a + stamps.ipc_ms.unwrap_or(0.0).max(0.0))
        .or_else(|| {
            // Re-derive the anchor the same way `merge` did: the WebView's
            // request-invoke offset against the Rust-side send time is the best
            // stand-in when no snapshot anchor exists.
            trace
                .t_request_sent
                .zip(stamps.stream_invoked_ms)
                .map(|(sent, invoked)| sent - invoked)
        });
    let abs = |offset: Option<f64>| anchor.zip(offset).map(|(a, o)| a + o);
    if let Some(t) = abs(stamps.first_paint_ms) {
        trace.t_first_paint = Some(t);
    }
    if let Some(t) = abs(stamps.done_ms) {
        trace.t_done = Some(t);
    }
    if trace.ipc_ms.is_none() {
        trace.ipc_ms = stamps.ipc_ms;
    }
    if trace.t_shortcut.is_none() {
        trace.t_shortcut = stamps.shortcut_ms.or_else(|| abs(stamps.ask_started_ms));
    }
    if trace.t_capture_done.is_none() {
        trace.t_capture_done = stamps.capture_done_ms;
    }
    if trace.t_snapshot_ready.is_none() {
        trace.t_snapshot_ready = abs(stamps.snapshot_ready_ms);
    }
    if trace.t_retrieval_done.is_none() {
        trace.t_retrieval_done = abs(stamps.retrieval_done_ms);
    }
    if trace.t_prompt_built.is_none() {
        trace.t_prompt_built = abs(stamps.prompt_built_ms);
    }
    if trace.image_bytes.is_none() {
        trace.image_bytes = stamps.image_bytes;
    }
    if trace.image_px.is_none() {
        trace.image_px = stamps.image_px;
    }
    if trace.trigger == "internal" {
        if let Some(trigger) = stamps.trigger.as_deref().filter(|t| !t.trim().is_empty()) {
            trace.trigger = trigger.to_string();
        }
    }
}

/// The milestones of the `docs/LATENCY.md` table, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stage {
    Capture,
    SnapshotReady,
    Retrieval,
    PromptBuilt,
    RequestSent,
    ResponseHeaders,
    FirstToken,
    FirstPaint,
    Done,
}

impl Stage {
    pub const ALL: [Stage; 9] = [
        Stage::Capture,
        Stage::SnapshotReady,
        Stage::Retrieval,
        Stage::PromptBuilt,
        Stage::RequestSent,
        Stage::ResponseHeaders,
        Stage::FirstToken,
        Stage::FirstPaint,
        Stage::Done,
    ];

    /// Stable id (`BenchRow::stage`, the TS mirror uses the same strings).
    pub const fn id(self) -> &'static str {
        match self {
            Stage::Capture => "capture",
            Stage::SnapshotReady => "snapshot_ready",
            Stage::Retrieval => "retrieval",
            Stage::PromptBuilt => "prompt_built",
            Stage::RequestSent => "request_sent",
            Stage::ResponseHeaders => "response_headers",
            Stage::FirstToken => "first_token",
            Stage::FirstPaint => "first_paint",
            Stage::Done => "done",
        }
    }

    /// The row label of the PR-template table.
    pub const fn label(self) -> &'static str {
        match self {
            Stage::Capture => "capture",
            Stage::SnapshotReady => "snapshot ready (incl. OCR/AX where still awaited)",
            Stage::Retrieval => "retrieval",
            Stage::PromptBuilt => "prompt built",
            Stage::RequestSent => "request sent (local total)",
            Stage::ResponseHeaders => "response headers",
            Stage::FirstToken => "first token",
            Stage::FirstPaint => "first paint",
            Stage::Done => "done",
        }
    }

    fn timestamp(self, trace: &LatencyTrace) -> Option<f64> {
        match self {
            Stage::Capture => trace.t_capture_done,
            Stage::SnapshotReady => trace.t_snapshot_ready,
            Stage::Retrieval => trace.t_retrieval_done,
            Stage::PromptBuilt => trace.t_prompt_built,
            Stage::RequestSent => trace.t_request_sent,
            Stage::ResponseHeaders => trace.t_response_headers,
            Stage::FirstToken => trace.t_first_token,
            Stage::FirstPaint => trace.t_first_paint,
            Stage::Done => trace.t_done,
        }
    }
}

/// Where the trace starts: the trigger, else the earliest stamp it has.
pub fn origin(trace: &LatencyTrace) -> Option<f64> {
    trace.t_shortcut.or_else(|| {
        Stage::ALL
            .iter()
            .filter_map(|stage| stage.timestamp(trace))
            .fold(None, |min: Option<f64>, t| {
                Some(min.map_or(t, |m| m.min(t)))
            })
    })
}

/// Milliseconds from the origin to `stage` (cumulative), when both exist and
/// the stage did not precede the origin (a stale stamp is not a milestone).
pub fn milestone(trace: &LatencyTrace, stage: Stage) -> Option<f64> {
    let origin = origin(trace)?;
    let t = stage.timestamp(trace)?;
    (t >= origin).then_some(t - origin)
}

/// Every available milestone of a trace, in table order.
pub fn milestones(trace: &LatencyTrace) -> Vec<(Stage, f64)> {
    Stage::ALL
        .iter()
        .filter_map(|stage| milestone(trace, *stage).map(|ms| (*stage, ms)))
        .collect()
}

/// Nearest-rank percentile of `values` (any order; `q` in 0..=1). Never the mean.
pub fn percentile(values: &[f64], q: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
    if sorted.is_empty() {
        return None;
    }
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let q = q.clamp(0.0, 1.0);
    let rank = ((q * sorted.len() as f64).ceil() as usize).max(1);
    Some(sorted[rank.min(sorted.len()) - 1])
}

/// One row per stage that at least one trace reached (cumulative ms since the trigger).
pub fn summarize(traces: &[LatencyTrace]) -> Vec<BenchRow> {
    Stage::ALL
        .iter()
        .filter_map(|stage| {
            let values: Vec<f64> = traces
                .iter()
                .filter_map(|trace| milestone(trace, *stage))
                .collect();
            if values.is_empty() {
                return None;
            }
            Some(BenchRow {
                stage: stage.id().to_string(),
                label: stage.label().to_string(),
                samples: values.len() as u32,
                p50_ms: percentile(&values, 0.5),
                p95_ms: percentile(&values, 0.95),
            })
        })
        .collect()
}

fn fmt_ms(value: Option<f64>) -> String {
    match value {
        Some(v) if v >= 100.0 => format!("{v:.0} ms"),
        Some(v) => format!("{v:.1} ms"),
        None => "—".to_string(),
    }
}

/// The percentile table (`docs/LATENCY.md`), plus the image / token line.
pub fn bench_report(
    traces: &[LatencyTrace],
    provider: &str,
    model: Option<&str>,
    discarded: u32,
    failures: u32,
    fixture: bool,
    ran_at: &str,
) -> BenchReport {
    let rows = summarize(traces);
    let local = rows.iter().find(|r| r.stage == Stage::RequestSent.id());
    let image_bytes: Vec<f64> = traces
        .iter()
        .filter_map(|t| t.image_bytes.map(|b| b as f64))
        .collect();
    let prompt_tokens: Vec<f64> = traces
        .iter()
        .filter_map(|t| t.prompt_tokens.map(|n| n as f64))
        .collect();
    let image_bytes_p50 = percentile(&image_bytes, 0.5).map(|v| v.round() as u64);
    let prompt_tokens_p50 = percentile(&prompt_tokens, 0.5).map(|v| v.round() as u32);

    let mut markdown = String::new();
    markdown.push_str(&format!(
        "Fast path bench — provider `{provider}`{}, {} runs counted ({discarded} warm-up discarded, {failures} failed), {} screen, {ran_at}\n\n",
        model.map(|m| format!(" / `{m}`")).unwrap_or_default(),
        traces.len(),
        if fixture { "fixture" } else { "live" }
    ));
    markdown.push_str("| Stage | p50 / p95 | n |\n|---|---|---|\n");
    for stage in Stage::ALL {
        let row = rows.iter().find(|r| r.stage == stage.id());
        markdown.push_str(&format!(
            "| {} | {} / {} | {} |\n",
            stage.label(),
            fmt_ms(row.and_then(|r| r.p50_ms)),
            fmt_ms(row.and_then(|r| r.p95_ms)),
            row.map(|r| r.samples).unwrap_or(0)
        ));
    }
    markdown.push_str(&format!(
        "| image bytes / prompt tokens | {} / {} | |\n",
        image_bytes_p50
            .map(|b| format!("{:.0} KB", b as f64 / 1024.0))
            .unwrap_or_else(|| "—".into()),
        prompt_tokens_p50
            .map(|n| n.to_string())
            .unwrap_or_else(|| "—".into())
    ));

    BenchReport {
        provider: provider.to_string(),
        model: model.map(str::to_string),
        iterations: traces.len() as u32,
        discarded,
        failures,
        fixture,
        local_total_p50_ms: local.and_then(|r| r.p50_ms),
        local_total_p95_ms: local.and_then(|r| r.p95_ms),
        rows,
        image_bytes_p50,
        prompt_tokens_p50,
        markdown,
        ran_at: ran_at.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn stamps() -> TraceStamps {
        TraceStamps {
            trigger: Some("shortcut_capture".into()),
            anchor_ms: Some(1_000.0),
            ipc_ms: Some(12.0),
            shortcut_ms: Some(900.0),
            capture_done_ms: Some(980.0),
            image_bytes: Some(180_000),
            image_px: Some(1440),
            ask_started_ms: Some(-95.0),
            snapshot_ready_ms: Some(0.0),
            retrieval_done_ms: Some(8.0),
            prompt_built_ms: Some(20.0),
            stream_invoked_ms: Some(22.0),
            first_paint_ms: None,
            done_ms: None,
        }
    }

    fn rust() -> RustStamps {
        RustStamps {
            request_received: Some(1_040.0),
            request_sent: Some(1_045.0),
            response_headers: Some(1_100.0),
            first_token: Some(1_400.0),
            stream_done: Some(2_000.0),
        }
    }

    #[test]
    fn the_halves_merge_on_the_snapshot_anchor() {
        let trace = merge(
            "req_1",
            Some(&stamps()),
            &rust(),
            Some("gemini"),
            Some("gemini-3.5-flash-lite"),
            Some(2_100),
        );
        assert_eq!(trace.trigger, "shortcut_capture");
        assert_eq!(
            trace.t_shortcut,
            Some(900.0),
            "the shortcut moment wins over the ask start"
        );
        assert_eq!(trace.t_capture_done, Some(980.0));
        // anchor = 1000 + 12 ipc; offsets add to it
        assert_eq!(trace.t_snapshot_ready, Some(1_012.0));
        assert_eq!(trace.t_retrieval_done, Some(1_020.0));
        assert_eq!(trace.t_prompt_built, Some(1_032.0));
        assert_eq!(trace.t_request_sent, Some(1_045.0));
        assert_eq!(trace.t_response_headers, Some(1_100.0));
        assert_eq!(trace.t_first_token, Some(1_400.0));
        assert_eq!(trace.t_first_paint, None);
        assert_eq!(
            trace.t_done,
            Some(2_000.0),
            "rust's stream end until the WebView reports"
        );
        assert_eq!(trace.image_bytes, Some(180_000));
        assert_eq!(trace.image_px, Some(1440));
        assert_eq!(trace.prompt_tokens, Some(2_100));
        assert_eq!(trace.provider_id.as_deref(), Some("gemini"));

        let ms = milestones(&trace);
        assert_eq!(ms[0], (Stage::Capture, 80.0));
        assert_eq!(ms[1], (Stage::SnapshotReady, 112.0));
        assert_eq!(milestone(&trace, Stage::RequestSent), Some(145.0));
        assert_eq!(milestone(&trace, Stage::FirstToken), Some(500.0));
        assert_eq!(milestone(&trace, Stage::FirstPaint), None);
    }

    #[test]
    fn late_stamps_and_the_request_anchor_fallback() {
        let mut trace = merge("req_2", Some(&stamps()), &rust(), None, None, None);
        let late = TraceStamps {
            anchor_ms: Some(1_000.0),
            ipc_ms: Some(12.0),
            first_paint_ms: Some(405.0),
            done_ms: Some(1_010.0),
            ..TraceStamps::default()
        };
        apply_late_stamps(&mut trace, &late);
        assert_eq!(trace.t_first_paint, Some(1_417.0));
        assert_eq!(
            trace.t_done,
            Some(2_022.0),
            "the WebView's done replaces rust's stream end"
        );
        assert_eq!(milestone(&trace, Stage::FirstPaint), Some(517.0));

        // No native snapshot: the request's arrival anchors the offsets.
        let typed = TraceStamps {
            trigger: Some("typed".into()),
            ask_started_ms: Some(-30.0),
            prompt_built_ms: Some(-2.0),
            stream_invoked_ms: Some(0.0),
            ..TraceStamps::default()
        };
        let trace = merge("req_3", Some(&typed), &rust(), None, None, None);
        assert_eq!(trace.trigger, "typed");
        assert_eq!(
            trace.t_shortcut,
            Some(1_010.0),
            "ask start on the request-arrival anchor"
        );
        assert_eq!(trace.t_prompt_built, Some(1_038.0));
        assert_eq!(trace.t_snapshot_ready, None);
        assert_eq!(milestone(&trace, Stage::RequestSent), Some(35.0));

        // Nothing from the WebView at all (a classification request).
        let bare = merge(
            "req_4",
            None,
            &rust(),
            Some("mock"),
            Some("mock-default"),
            None,
        );
        assert_eq!(bare.trigger, "internal");
        assert_eq!(bare.t_shortcut, None);
        assert_eq!(
            origin(&bare),
            Some(1_045.0),
            "earliest stamp stands in for the origin"
        );
        assert_eq!(milestone(&bare, Stage::FirstToken), Some(355.0));
        assert_eq!(milestone(&bare, Stage::RequestSent), Some(0.0));
    }

    #[test]
    fn percentiles_are_nearest_rank_and_never_the_mean() {
        let values = [10.0, 20.0, 30.0, 40.0, 100.0];
        assert_eq!(percentile(&values, 0.5), Some(30.0));
        assert_eq!(percentile(&values, 0.95), Some(100.0));
        assert_eq!(percentile(&values, 0.0), Some(10.0));
        assert_eq!(percentile(&values, 1.0), Some(100.0));
        assert_eq!(percentile(&[7.0], 0.95), Some(7.0));
        assert_eq!(percentile(&[], 0.5), None);
        assert_eq!(percentile(&[f64::NAN, 3.0], 0.5), Some(3.0));
        let unsorted = [5.0, 1.0, 3.0, 2.0, 4.0, 6.0, 8.0, 7.0, 9.0, 10.0];
        assert_eq!(percentile(&unsorted, 0.5), Some(5.0));
        assert_eq!(percentile(&unsorted, 0.9), Some(9.0));
    }

    #[test]
    fn the_bench_table_reads_like_the_pr_template() {
        let traces: Vec<LatencyTrace> = (0..5)
            .map(|i| {
                let mut trace = merge(
                    &format!("req_{i}"),
                    Some(&stamps()),
                    &rust(),
                    Some("mock"),
                    Some("mock-default"),
                    Some(2_000 + i),
                );
                trace.t_request_sent = Some(1_045.0 + i as f64 * 10.0);
                trace
            })
            .collect();
        let report = bench_report(
            &traces,
            "mock",
            Some("mock-default"),
            3,
            0,
            true,
            "2026-09-11T12:00:00Z",
        );
        assert_eq!(report.iterations, 5);
        assert_eq!(report.discarded, 3);
        assert!(report.fixture);
        assert_eq!(report.local_total_p50_ms, Some(165.0));
        assert_eq!(report.local_total_p95_ms, Some(185.0));
        assert_eq!(report.image_bytes_p50, Some(180_000));
        assert_eq!(report.prompt_tokens_p50, Some(2_002));
        let capture = report.rows.iter().find(|r| r.stage == "capture").unwrap();
        assert_eq!(capture.samples, 5);
        assert_eq!(capture.p50_ms, Some(80.0));
        assert!(
            report.rows.iter().all(|r| r.stage != "first_paint"),
            "no paint in a bench"
        );
        assert!(
            report.markdown.contains("| Stage | p50 / p95 | n |"),
            "{}",
            report.markdown
        );
        assert!(
            report
                .markdown
                .contains("| request sent (local total) | 165 ms / 185 ms | 5 |"),
            "{}",
            report.markdown
        );
        assert!(report.markdown.contains("| first paint | — / — | 0 |"));
        assert!(
            report
                .markdown
                .contains("| image bytes / prompt tokens | 176 KB / 2002 | |"),
            "{}",
            report.markdown
        );
        assert!(report.markdown.starts_with("Fast path bench — provider `mock` / `mock-default`, 5 runs counted (3 warm-up discarded, 0 failed), fixture screen"));
        assert_eq!(fmt_ms(Some(12.345)), "12.3 ms");
        assert_eq!(fmt_ms(Some(250.4)), "250 ms");
    }

    #[test]
    fn the_wire_shapes_are_camel_case() {
        let trace = merge("req_9", Some(&stamps()), &rust(), Some("mock"), None, None);
        let json = serde_json::to_value(&trace).unwrap();
        assert_eq!(json["requestId"], "req_9");
        assert_eq!(json["tShortcut"], 900.0);
        assert_eq!(json["tRequestSent"], 1_045.0);
        assert!(
            json.get("tFirstPaint").is_none(),
            "absent stamps are omitted"
        );
        assert_eq!(json["imagePx"], 1440);
        let back: LatencyTrace = serde_json::from_value(json).unwrap();
        assert_eq!(back, trace);
        let stamps: TraceStamps =
            serde_json::from_str(r#"{"trigger":"typed","askStartedMs":-30,"streamInvokedMs":0}"#)
                .unwrap();
        assert_eq!(stamps.ask_started_ms, Some(-30.0));
        assert_eq!(stamps.anchor_ms, None);
    }
}
