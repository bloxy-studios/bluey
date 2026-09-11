//! The ⌘↵ fast-path bench (ADR 0010 §2, `docs/LATENCY.md › The bench`):
//! `dev_bench_fast_path { iterations, provider, fixture }` and the boot hook
//! behind `bun run bench:fastpath`.
//!
//! Each run walks the real native path — shortcut moment → capture (live, or a
//! fixture screen where ScreenCaptureKit is unavailable) → snapshot assembly →
//! a request with the screenshot → provider → first token — and the merged
//! `LatencyTrace` of every run feeds the percentile table. What the bench cannot
//! measure is the WebView's own work: the TypeScript prompt build and the first
//! paint live in the traces of real ⌘↵ presses (the dev overlay), not here. The
//! first [`BENCH_WARMUP`] runs are discarded; p50 / p95 are nearest-rank, never
//! the mean. Developer mode or a `dev-tools` build only.

use std::path::Path;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use bluey_core::latency::{self, BENCH_WARMUP};
use bluey_core::types::{
    AiContentPart, AiMessage, AiRequest, AiRole, AiTask, BenchOptions, BenchReport, CaptureOptions,
    CaptureTarget, ImageMimeType, LatencyBudget, LatencyTrace, ReasoningLevel, Settings,
    SnapshotOptions, TraceStamps,
};
use bluey_core::{new_id, now_iso, BlueyError, BlueyResult};
use tauri::{AppHandle, Manager};

use crate::context::{build_snapshot_with, FixtureFrame};
use crate::state::AppCore;

/// Set (non-empty) to run the bench at boot and exit: `bun run bench:fastpath`.
pub const ENV_ENABLE: &str = "BLUEY_BENCH_FASTPATH";
pub const ENV_ITERATIONS: &str = "BLUEY_BENCH_ITERATIONS";
pub const ENV_PROVIDER: &str = "BLUEY_BENCH_PROVIDER";
pub const ENV_FIXTURE: &str = "BLUEY_BENCH_FIXTURE";
/// Where the boot hook writes the report as JSON (the script reads it back).
pub const ENV_OUT: &str = "BLUEY_BENCH_OUT";

const DEFAULT_ITERATIONS: u32 = 30;
const MAX_ITERATIONS: u32 = 200;
const BENCH_SYSTEM: &str = "You are Bluey, a discreet copilot. Answer in one short sentence.";
const BENCH_QUESTION: &str = "What is on my screen?";

/// The bench options the environment describes, if the app was launched to bench.
pub fn options_from_env() -> Option<BenchOptions> {
    let enabled = std::env::var(ENV_ENABLE)
        .ok()
        .filter(|v| !v.trim().is_empty())?;
    if enabled.trim() == "0" || enabled.trim().eq_ignore_ascii_case("false") {
        return None;
    }
    Some(BenchOptions {
        iterations: std::env::var(ENV_ITERATIONS)
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(DEFAULT_ITERATIONS),
        provider: std::env::var(ENV_PROVIDER)
            .ok()
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| crate::ai::MOCK_PROVIDER_ID.to_string()),
        fixture: std::env::var(ENV_FIXTURE)
            .ok()
            .map(|f| f.trim().to_string())
            .filter(|f| !f.is_empty()),
    })
}

/// Boot hook: when the environment asks for a bench, run it, print the table,
/// write the JSON report and leave — exit 0 when every counted run produced a
/// trace, 2 when some failed, 3 when the bench could not run at all.
pub async fn run_from_env(app: &AppHandle) {
    let Some(options) = options_from_env() else {
        return;
    };
    let core = app.state::<AppCore>();
    let code = match run(&core, options).await {
        Ok(report) => {
            println!("{}", report.markdown);
            if let Ok(path) = std::env::var(ENV_OUT) {
                if let Ok(json) = serde_json::to_string_pretty(&report) {
                    if let Err(error) = std::fs::write(&path, json) {
                        eprintln!("bench: cannot write {path}: {error}");
                    }
                }
            }
            if report.failures == 0 {
                0
            } else {
                2
            }
        }
        Err(error) => {
            eprintln!("bench: {} ({})", error.message, error.code);
            3
        }
    };
    std::process::exit(code);
}

fn bench_allowed(settings: &Settings) -> bool {
    cfg!(feature = "dev-tools") || cfg!(debug_assertions) || settings.general.developer_mode
}

/// Image dimensions from a JPEG (first SOF marker) or PNG (IHDR) header.
pub fn image_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) && bytes.len() >= 24 {
        let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        return Some((width, height));
    }
    if bytes.starts_with(&[0xFF, 0xD8]) {
        let mut i = 2;
        while i + 9 < bytes.len() {
            if bytes[i] != 0xFF {
                i += 1;
                continue;
            }
            let marker = bytes[i + 1];
            if marker == 0xFF {
                i += 1;
                continue;
            }
            let length = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
            let is_sof = (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC);
            if is_sof {
                let height = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]);
                let width = u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]);
                return Some((u32::from(width), u32::from(height)));
            }
            if marker == 0xD9 || marker == 0xDA {
                break;
            }
            i += 2 + length.max(2);
        }
    }
    None
}

fn mime_of(bytes: &[u8]) -> Option<ImageMimeType> {
    if bytes.starts_with(&[0xFF, 0xD8]) {
        Some(ImageMimeType::Jpeg)
    } else if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some(ImageMimeType::Png)
    } else if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(ImageMimeType::Webp)
    } else {
        None
    }
}

/// Decode a fixture file: raw JPEG / PNG bytes, or their base64 text (`.b64`).
pub fn fixture_from_bytes(raw: &[u8]) -> BlueyResult<FixtureFrame> {
    let looks_text = raw
        .iter()
        .take(64)
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=' | b'\n' | b'\r'));
    let (bytes, base64) = if mime_of(raw).is_some() {
        (raw.to_vec(), BASE64.encode(raw))
    } else if looks_text {
        let text: String = String::from_utf8_lossy(raw)
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        let bytes = BASE64.decode(text.as_bytes()).map_err(|_| {
            BlueyError::invalid_params("the fixture screen is neither an image nor base64 text")
        })?;
        (bytes, text)
    } else {
        return Err(BlueyError::invalid_params(
            "the fixture screen is neither a JPEG / PNG nor base64 text",
        ));
    };
    let mime_type = mime_of(&bytes)
        .ok_or_else(|| BlueyError::invalid_params("the fixture screen is not a JPEG / PNG"))?;
    let (width, height) = image_dimensions(&bytes).unwrap_or((0, 0));
    Ok(FixtureFrame {
        bytes: bytes.len() as u64,
        base64,
        mime_type,
        width,
        height,
    })
}

fn load_fixture(path: &str) -> BlueyResult<FixtureFrame> {
    let raw = std::fs::read(Path::new(path)).map_err(|error| {
        BlueyError::invalid_params(format!("cannot read the fixture screen `{path}`: {error}"))
    })?;
    fixture_from_bytes(&raw)
}

/// The snapshot options the ⌘↵ path uses (`snapshotOptionsFor` in
/// `src/context/snapshot.ts` for a screen-requiring trigger), from the settings.
pub fn fast_path_options(settings: &Settings) -> SnapshotOptions {
    let preference = serde_json::to_value(settings.screen.capture_target)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();
    let target = if preference == "active_window" {
        serde_json::from_value::<CaptureTarget>(serde_json::json!({ "type": "active_window" }))
            .unwrap_or(CaptureTarget::Display { display_id: None })
    } else {
        let preferred = settings.screen.preferred_display.trim();
        CaptureTarget::Display {
            display_id: (preferred != "active" && !preferred.is_empty())
                .then(|| preferred.to_string()),
        }
    };
    SnapshotOptions {
        include_screen: true,
        include_ocr: true,
        include_accessibility: true,
        include_transcript: false,
        transcript_window_seconds: None,
        capture: Some(CaptureOptions {
            target: Some(target),
            format: serde_json::from_value(serde_json::json!("jpeg")).ok(),
            quality: Some(0.8),
            max_dimension: Some(settings.screen.max_image_dimension),
            inline: Some(true),
            change_detection: Some(false),
        }),
        ocr_level: Some(settings.screen.ocr_level),
        inline_image: Some(true),
    }
}

/// Run the bench: `options.iterations` runs (the first [`BENCH_WARMUP`] discarded).
pub async fn run(core: &AppCore, options: BenchOptions) -> BlueyResult<BenchReport> {
    let settings = core.settings.get();
    if !bench_allowed(&settings) {
        return Err(BlueyError::not_supported(
            "bench",
            "the fast-path bench needs Developer Mode or a dev-tools build",
        ));
    }
    if options.iterations == 0 || options.iterations > MAX_ITERATIONS {
        return Err(BlueyError::invalid_params(format!(
            "iterations must be between 1 and {MAX_ITERATIONS}"
        )));
    }
    let assignment = core.ai.bench_assignment(&options.provider)?;
    let fixture = match options.fixture.as_deref() {
        Some(path) => Some(load_fixture(path)?),
        None => None,
    };
    let snapshot_options = fast_path_options(&settings);
    let discard = if options.iterations > BENCH_WARMUP {
        BENCH_WARMUP
    } else {
        0
    };
    let mut traces: Vec<LatencyTrace> = Vec::new();
    let mut failures = 0u32;
    tracing::info!(
        provider = %options.provider,
        model = %assignment.model,
        iterations = options.iterations,
        fixture = fixture.is_some(),
        "fast-path bench starting"
    );
    for i in 0..options.iterations {
        let t_shortcut = crate::clock::mono_ms();
        let snapshot =
            match build_snapshot_with(core, snapshot_options.clone(), fixture.as_ref()).await {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    tracing::warn!(run = i, code = %error.code, "bench capture failed");
                    failures += 1;
                    continue;
                }
            };
        let t_ready = crate::clock::mono_ms();
        let reply_ms = snapshot
            .trace
            .as_ref()
            .map(|t| t.reply_ms)
            .unwrap_or(t_ready);
        let image = snapshot.screen.as_ref().and_then(|screen| {
            screen.image.clone().and_then(|data| {
                let media_type = screen.mime_type.as_deref().and_then(|m| {
                    serde_json::from_value(serde_json::Value::String(m.to_string())).ok()
                })?;
                Some(AiContentPart::Image { media_type, data })
            })
        });
        let mut content = vec![AiContentPart::Text {
            text: BENCH_QUESTION.to_string(),
        }];
        let has_image = image.is_some();
        content.extend(image);
        let messages = vec![
            AiMessage::text(AiRole::System, BENCH_SYSTEM),
            AiMessage {
                role: AiRole::User,
                content,
            },
        ];
        let context_tokens = ((BENCH_SYSTEM.len() + BENCH_QUESTION.len()) / 4) as u32
            + if has_image { 560 } else { 0 };
        let t_prompt = crate::clock::mono_ms();
        let snapshot_trace = snapshot.trace.as_ref();
        let request = AiRequest {
            request_id: new_id("bench"),
            session_id: None,
            generation: u64::from(i) + 1,
            task: AiTask::Answer,
            latency_budget: LatencyBudget::Fast,
            reasoning: ReasoningLevel::None,
            vision_required: has_image,
            context_tokens,
            messages,
            output_schema: None,
            max_output_tokens: Some(64),
            temperature: Some(0.0),
            model_override: Some(assignment.clone()),
            trace: Some(TraceStamps {
                trigger: Some("bench".into()),
                anchor_ms: Some(reply_ms),
                ipc_ms: Some(0.0),
                shortcut_ms: Some(t_shortcut),
                capture_done_ms: snapshot_trace.and_then(|t| t.capture_done_ms),
                image_bytes: snapshot_trace.and_then(|t| t.image_bytes),
                image_px: snapshot_trace.and_then(|t| t.image_px),
                ask_started_ms: Some(t_shortcut - reply_ms),
                snapshot_ready_ms: Some(t_ready - reply_ms),
                retrieval_done_ms: None,
                prompt_built_ms: Some(t_prompt - reply_ms),
                stream_invoked_ms: Some(crate::clock::mono_ms() - reply_ms),
                first_paint_ms: None,
                done_ms: None,
            }),
            created_at: now_iso(),
        };
        match core.ai.run_traced(request).await {
            Ok(trace) => {
                if i >= discard {
                    traces.push(trace);
                }
            }
            Err(error) => {
                tracing::warn!(run = i, code = %error.code, "bench request failed");
                if i >= discard {
                    failures += 1;
                }
            }
        }
    }
    let report = latency::bench_report(
        &traces,
        &options.provider,
        Some(&assignment.model),
        discard,
        failures,
        fixture.is_some(),
        &now_iso(),
    );
    tracing::info!(
        counted = report.iterations,
        failures,
        local_total_p50 = ?report.local_total_p50_ms,
        "fast-path bench finished"
    );
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jpeg_header(width: u16, height: u16) -> Vec<u8> {
        // SOI, an APP0 segment, then SOF0 with the dimensions.
        let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x4A, 0x46];
        bytes.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x11, 0x08]);
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&[0x03, 0x01, 0x22, 0x00, 0x02, 0x11, 0x01, 0x03, 0x11, 0x01]);
        bytes
    }

    fn png_header(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        bytes.extend_from_slice(&13u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 2, 0, 0, 0]);
        bytes
    }

    #[test]
    fn image_headers_yield_dimensions_and_mime() {
        assert_eq!(image_dimensions(&jpeg_header(1440, 900)), Some((1440, 900)));
        assert_eq!(image_dimensions(&png_header(1512, 982)), Some((1512, 982)));
        assert_eq!(image_dimensions(b"not an image"), None);
        assert_eq!(mime_of(&jpeg_header(1, 1)), Some(ImageMimeType::Jpeg));
        assert_eq!(mime_of(&png_header(1, 1)), Some(ImageMimeType::Png));
        assert_eq!(mime_of(b"RIFF\0\0\0\0WEBPVP8 "), Some(ImageMimeType::Webp));
        assert_eq!(mime_of(b"hello"), None);
    }

    #[test]
    fn fixtures_load_from_raw_bytes_or_base64_text() {
        let raw = jpeg_header(1440, 900);
        let from_raw = fixture_from_bytes(&raw).unwrap();
        assert_eq!(from_raw.mime_type, ImageMimeType::Jpeg);
        assert_eq!((from_raw.width, from_raw.height), (1440, 900));
        assert_eq!(from_raw.bytes, raw.len() as u64);
        assert_eq!(from_raw.base64, BASE64.encode(&raw));

        let text = format!("{}\n", BASE64.encode(&raw));
        let from_text = fixture_from_bytes(text.as_bytes()).unwrap();
        assert_eq!(from_text.bytes, raw.len() as u64);
        assert_eq!(from_text.base64, BASE64.encode(&raw));
        assert_eq!((from_text.width, from_text.height), (1440, 900));

        assert!(fixture_from_bytes(b"\x00\x01\x02 garbage").is_err());
        assert!(
            fixture_from_bytes(b"aGVsbG8=").is_err(),
            "base64 of a non-image is refused"
        );
    }

    #[test]
    fn env_options_default_to_thirty_mock_runs() {
        // The environment is process-global; only exercise the parsing helpers'
        // defaults through a fresh process view when the flag is absent.
        std::env::remove_var(ENV_ENABLE);
        assert!(options_from_env().is_none());
        std::env::set_var(ENV_ENABLE, "0");
        assert!(options_from_env().is_none());
        std::env::set_var(ENV_ENABLE, "1");
        std::env::remove_var(ENV_ITERATIONS);
        std::env::remove_var(ENV_PROVIDER);
        std::env::remove_var(ENV_FIXTURE);
        let options = options_from_env().unwrap();
        assert_eq!(options.iterations, DEFAULT_ITERATIONS);
        assert_eq!(options.provider, "mock");
        assert_eq!(options.fixture, None);
        std::env::set_var(ENV_ITERATIONS, "12");
        std::env::set_var(ENV_PROVIDER, "gemini");
        std::env::set_var(ENV_FIXTURE, "tests/fixtures/screens/general-1440.jpg.b64");
        let options = options_from_env().unwrap();
        assert_eq!(options.iterations, 12);
        assert_eq!(options.provider, "gemini");
        assert_eq!(
            options.fixture.as_deref(),
            Some("tests/fixtures/screens/general-1440.jpg.b64")
        );
        std::env::remove_var(ENV_ENABLE);
        std::env::remove_var(ENV_ITERATIONS);
        std::env::remove_var(ENV_PROVIDER);
        std::env::remove_var(ENV_FIXTURE);
    }

    #[test]
    fn the_fast_path_options_mirror_the_engine() {
        let settings = Settings::default();
        let options = fast_path_options(&settings);
        assert!(options.include_screen && options.include_ocr && options.include_accessibility);
        assert!(!options.include_transcript);
        let capture = options.capture.unwrap();
        assert_eq!(capture.quality, Some(0.8));
        assert_eq!(capture.inline, Some(true));
        assert_eq!(capture.change_detection, Some(false));
        assert_eq!(
            capture.max_dimension,
            Some(settings.screen.max_image_dimension)
        );
        assert!(capture.target.is_some());
    }
}
