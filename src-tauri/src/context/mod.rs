//! The ⌘↵ fast path: assemble a [`ContextSnapshot`] from the helper (frontmost
//! app, screen capture + OCR, accessibility) **in parallel**, the transcript ring
//! buffer, the active session and mode, then trim it (`bluey_core::context`)
//! and tag it with an application adapter. Timings feed the dev overlay; the
//! monotonic stamps on `ContextSnapshot::trace` (capture done, reply) are the
//! Rust half of the fast-path trace the WebView anchors on (ADR 0010 §2).

use std::collections::BTreeMap;
use std::future::Future;
use std::time::{Duration, Instant};

use bluey_core::context::{apply_adapter, trim_snapshot, SnapshotLimits};
use bluey_core::events::BlueyEvent;
use bluey_core::types::{
    AppEvent, CaptureTarget, ContextSnapshot, DocumentScope, ImageMimeType, ModeContext,
    OcrContext, RecentResponseRef, ScreenFrame, ScreenSummary, SessionContext, SnapshotOptions,
    SnapshotTrace, SnapshotWarning, SnapshotWarningKind, TranscriptContext,
};
use bluey_core::{new_id, BlueyError, BlueyResult};
use bluey_storage::{
    DocumentRepository, ResponseRepository, SessionEventRepository, SessionNoteRepository,
};

use crate::state::AppCore;

/// Recent responses/events included in the session context.
const RECENT_RESPONSES: u32 = 5;
const RECENT_EVENTS: usize = 10;
const DEFAULT_TRANSCRIPT_WINDOW_S: u32 = 120;
/// How long the fast path waits for OCR once the frame is in (ADR 0010 §3).
/// A slower pass finishes in the background into the OCR cache (the next ask
/// on the same screen reuses it) and this snapshot carries the image instead.
const OCR_SOFT_DEADLINE: Duration = Duration::from_millis(150);

/// A screen image standing in for the live capture — the fast-path bench on a
/// machine without ScreenCaptureKit (`docs/LATENCY.md › The bench`). It never
/// reaches the helper, so OCR is skipped for it.
#[derive(Debug, Clone)]
pub struct FixtureFrame {
    pub base64: String,
    pub bytes: u64,
    pub mime_type: ImageMimeType,
    pub width: u32,
    pub height: u32,
}

/// Decoded byte count of a base64 string (padding excluded).
fn decoded_len(base64: &str) -> u64 {
    let trimmed = base64.trim_end_matches('=');
    (trimmed.len() as u64) * 3 / 4
}

fn frame_from_fixture(fixture: &FixtureFrame, options: &SnapshotOptions) -> ScreenFrame {
    ScreenFrame {
        id: new_id("frame"),
        image: Some(fixture.base64.clone()),
        mime_type: fixture.mime_type,
        path: None,
        width: fixture.width,
        height: fixture.height,
        display_id: None,
        scale_factor: 1.0,
        captured_at: bluey_core::now_iso(),
        hash: None,
        changed: true,
        target: options
            .capture
            .as_ref()
            .and_then(|c| c.target.clone())
            .unwrap_or(CaptureTarget::Display { display_id: None }),
        duration_ms: Some(0),
    }
}

/// The screen half of the snapshot: the frame, its OCR and why either is missing.
#[derive(Default)]
struct ScreenBranch {
    frame: Option<ScreenFrame>,
    ocr: Option<OcrContext>,
    capture_ms: u64,
    ocr_ms: u64,
    capture_done_ms: Option<f64>,
    warning: Option<SnapshotWarning>,
}

/// Run `ocr` on its own task and wait up to `deadline` for it (`None` waits
/// for it). A pass that misses the deadline keeps running — the capture
/// manager caches its result — and `None` is returned.
async fn ocr_within<F>(ocr: F, deadline: Option<Duration>) -> Option<BlueyResult<OcrContext>>
where
    F: Future<Output = BlueyResult<OcrContext>> + Send + 'static,
{
    let task = tauri::async_runtime::spawn(ocr);
    let joined = match deadline {
        Some(deadline) => tokio::time::timeout(deadline, task).await.ok()?,
        None => task.await,
    };
    Some(joined.unwrap_or_else(|_| Err(BlueyError::internal("OCR task failed"))))
}

/// Capture, then OCR under the soft deadline. A failed capture leaves the
/// frame empty with a `screen_unavailable` warning instead of failing the ask;
/// `ocr` is `None` when OCR is off.
async fn screen_branch<O, F>(
    options: &SnapshotOptions,
    frame: impl Future<Output = BlueyResult<ScreenFrame>>,
    ocr: Option<O>,
) -> ScreenBranch
where
    O: FnOnce(&ScreenFrame) -> F,
    F: Future<Output = BlueyResult<OcrContext>> + Send + 'static,
{
    let t = Instant::now();
    let frame = match frame.await {
        Ok(frame) => frame,
        Err(e) => {
            tracing::warn!(error = %e, "screen capture failed; continuing without it");
            return ScreenBranch {
                capture_ms: t.elapsed().as_millis() as u64,
                warning: Some(SnapshotWarning::screen_unavailable(&e)),
                ..ScreenBranch::default()
            };
        }
    };
    let mut branch = ScreenBranch {
        capture_ms: t.elapsed().as_millis() as u64,
        capture_done_ms: Some(crate::clock::mono_ms()),
        ..ScreenBranch::default()
    };
    if let Some(ocr) = ocr {
        let t = Instant::now();
        // Only race OCR when the image travels with the snapshot to stand in for it.
        let deadline = options
            .inline_image
            .unwrap_or(true)
            .then_some(OCR_SOFT_DEADLINE);
        match ocr_within(ocr(&frame), deadline).await {
            Some(Ok(context)) => branch.ocr = Some(context),
            Some(Err(e)) => tracing::warn!(error = %e, "OCR failed; continuing without it"),
            None => branch.warning = Some(ocr_pending_warning()),
        }
        branch.ocr_ms = t.elapsed().as_millis() as u64;
    }
    branch.frame = Some(frame);
    branch
}

fn ocr_pending_warning() -> SnapshotWarning {
    SnapshotWarning {
        kind: SnapshotWarningKind::OcrPending,
        code: "capture.ocr_pending".into(),
        message: "Screen text is still being recognised; the screenshot is attached instead."
            .into(),
        recovery: None,
    }
}

/// Build the snapshot for `options`. Failures of individual sources degrade
/// gracefully (the field stays `None`); a failed screen capture leaves
/// `screen` empty and records a `screen_unavailable` warning instead of
/// failing the ask.
pub async fn build_snapshot(
    core: &AppCore,
    options: SnapshotOptions,
) -> BlueyResult<ContextSnapshot> {
    build_snapshot_with(core, options, None).await
}

/// [`build_snapshot`] with an optional fixture standing in for the live capture.
pub async fn build_snapshot_with(
    core: &AppCore,
    options: SnapshotOptions,
    fixture: Option<&FixtureFrame>,
) -> BlueyResult<ContextSnapshot> {
    let started = Instant::now();
    let started_ms = crate::clock::mono_ms();
    let mut timings: BTreeMap<String, u64> = BTreeMap::new();
    core.hub.transition_soft(AppEvent::CaptureStarted);

    // Frontmost app + accessibility + capture + session reads run concurrently;
    // OCR follows the capture.
    let frontmost = async { core.ax.frontmost().await.ok() };
    let accessibility = async {
        if !options.include_accessibility {
            return (None, 0u64);
        }
        let t = Instant::now();
        let result = core.ax.snapshot(None, None).await;
        (result.ok(), t.elapsed().as_millis() as u64)
    };
    let screen = async {
        if !options.include_screen {
            return ScreenBranch::default();
        }
        let (capture, level) = (core.capture.clone(), options.ocr_level);
        let ocr = move |frame: &ScreenFrame| {
            let (id, hash, changed) = (frame.id.clone(), frame.hash.clone(), frame.changed);
            async move {
                capture
                    .ocr(&id, level, None, hash.as_deref(), changed)
                    .await
            }
        };
        // A fixture never reaches the helper, so it gets no OCR either.
        let ocr = (options.include_ocr && fixture.is_none()).then_some(ocr);
        let frame = async {
            match fixture {
                Some(fixture) => Ok(frame_from_fixture(fixture, &options)),
                None => core.capture.capture(options.capture.clone()).await,
            }
        };
        screen_branch(&options, frame, ocr).await
    };
    let (frontmost, (accessibility, ax_ms), screen, session) =
        tokio::join!(frontmost, accessibility, screen, session_context(core));
    let ScreenBranch {
        frame,
        ocr,
        capture_ms,
        ocr_ms,
        capture_done_ms,
        warning,
    } = screen;

    let mut snapshot = ContextSnapshot {
        timestamp: bluey_core::now_iso(),
        session,
        warnings: warning.into_iter().collect(),
        ..ContextSnapshot::default()
    };
    if let Some(frontmost) = frontmost {
        snapshot.active_application = Some(frontmost.application);
        snapshot.active_window = frontmost.window;
    } else if let Some(ax) = &accessibility {
        snapshot.active_application = Some(ax.application.clone());
        snapshot.active_window = ax.window.clone();
    }
    if options.include_screen {
        timings.insert("capture".into(), capture_ms);
    }
    if options.include_ocr && options.include_screen {
        timings.insert("ocr".into(), ocr_ms);
    }
    if options.include_accessibility {
        timings.insert("accessibility".into(), ax_ms);
    }

    if let Some(frame) = frame {
        let inline = options.inline_image.unwrap_or(true);
        let image = if inline {
            match frame.image.clone() {
                Some(image) => Some(image),
                None => match core.capture.read_frame(&frame.id).await {
                    Ok(image) => Some(image),
                    Err(e) => {
                        tracing::warn!(error = %e, "cannot inline the captured frame");
                        None
                    }
                },
            }
        } else {
            None
        };
        snapshot.screen = Some(ScreenSummary {
            image,
            mime_type: Some(mime_str(frame.mime_type)),
            width: frame.width,
            height: frame.height,
            display_id: frame.display_id.clone(),
            frame_id: Some(frame.id.clone()),
        });
    }
    snapshot.ocr = ocr;
    snapshot.accessibility = accessibility;

    if options.include_transcript {
        let t = Instant::now();
        let window_seconds = options
            .transcript_window_seconds
            .unwrap_or(DEFAULT_TRANSCRIPT_WINDOW_S);
        let segments = core.audio.recent(window_seconds);
        if !segments.is_empty() {
            snapshot.transcript = Some(TranscriptContext {
                segments,
                earlier_summary: None,
                window_seconds,
            });
        }
        timings.insert("transcript".into(), t.elapsed().as_millis() as u64);
    }

    let mode = core.modes.active_mode();
    let style = bluey_core::modes::effective_style(
        &mode,
        &bluey_core::types::ResponseStyle {
            length: core.settings.get().ai.response_length,
            tone: core.settings.get().ai.response_tone,
        },
    );
    snapshot.mode = Some(ModeContext {
        mode,
        response_style: style,
    });

    let mut snapshot = trim_snapshot(snapshot, &SnapshotLimits::default());
    apply_adapter(&mut snapshot);
    timings.insert("assembly".into(), started.elapsed().as_millis() as u64);
    core.metrics.record_context(&timings);
    snapshot.timings = Some(timings);
    let (image_bytes, image_px) = snapshot
        .screen
        .as_ref()
        .map(|screen| {
            (
                screen.image.as_deref().map(decoded_len),
                Some(screen.width.max(screen.height)),
            )
        })
        .unwrap_or((None, None));
    snapshot.trace = Some(SnapshotTrace {
        started_ms,
        capture_done_ms,
        reply_ms: crate::clock::mono_ms(),
        image_bytes,
        image_px,
    });

    core.hub.transition_soft(AppEvent::AnalysisStarted);

    // The bus copy never carries the inline image (kept small for the WebView).
    let mut event_snapshot = snapshot.clone();
    if let Some(screen) = event_snapshot.screen.as_mut() {
        screen.image = None;
    }
    core.bus.publish(BlueyEvent::ContextUpdated {
        snapshot: event_snapshot,
        reason: "manual".into(),
    });
    Ok(snapshot)
}

/// Session context for the active session (recent responses, events, notes,
/// attached document ids). `None` without a session.
async fn session_context(core: &AppCore) -> Option<SessionContext> {
    let session = core.sessions.active()?;
    let id = session.id.clone();
    let loaded = core
        .storage
        .run(move |db| {
            let responses = ResponseRepository::list(db, &id, Some(RECENT_RESPONSES))?;
            let events = SessionEventRepository::list_recent(db, &id, RECENT_EVENTS)?;
            let notes = SessionNoteRepository::list(db, &id)?;
            let documents = DocumentRepository::list(db, Some(DocumentScope::Session), Some(&id))?;
            Ok((responses, events, notes, documents))
        })
        .await;
    let (responses, events, notes, documents) = match loaded {
        Ok(loaded) => loaded,
        Err(e) => {
            tracing::warn!(error = %e, "cannot load session context");
            return None;
        }
    };
    Some(SessionContext {
        session_id: session.id,
        mode_id: session.mode_id,
        started_at: session.started_at,
        recent_responses: responses
            .into_iter()
            .map(|r| RecentResponseRef {
                id: r.id,
                title: r.title,
                content: r.content,
                created_at: r.created_at,
            })
            .collect(),
        recent_events: events,
        notes: notes.into_iter().map(|n| n.content).collect(),
        document_ids: documents.into_iter().map(|d| d.id).collect(),
    })
}

fn mime_str(mime: bluey_core::types::ImageMimeType) -> String {
    match mime {
        bluey_core::types::ImageMimeType::Jpeg => "image/jpeg".into(),
        bluey_core::types::ImageMimeType::Png => "image/png".into(),
        bluey_core::types::ImageMimeType::Webp => "image/webp".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bluey_core::error::RecoveryAction;
    use bluey_core::types::{OcrLevel, PermissionKind};

    fn screen_options() -> SnapshotOptions {
        SnapshotOptions {
            include_screen: true,
            include_ocr: true,
            ..SnapshotOptions::default()
        }
    }

    fn frame() -> BlueyResult<ScreenFrame> {
        let fixture = FixtureFrame {
            base64: "QUJD".into(),
            bytes: 3,
            mime_type: ImageMimeType::Jpeg,
            width: 1440,
            height: 900,
        };
        Ok(frame_from_fixture(&fixture, &screen_options()))
    }

    fn ocr_text(text: &str) -> BlueyResult<OcrContext> {
        Ok(OcrContext {
            blocks: vec![],
            text: text.into(),
            level: OcrLevel::Fast,
            languages: vec![],
            duration_ms: 0,
            frame_id: None,
        })
    }

    #[tokio::test]
    async fn a_failed_capture_degrades_to_a_screen_unavailable_warning() {
        let denied = BlueyError::permission(PermissionKind::ScreenRecording, "not granted");
        let ocr = |_: &ScreenFrame| async { ocr_text("never") };
        let branch = screen_branch(&screen_options(), async { Err(denied) }, Some(ocr)).await;
        assert!(branch.frame.is_none() && branch.ocr.is_none());
        let warning = branch.warning.expect("a screen_unavailable warning");
        assert_eq!(warning.kind, SnapshotWarningKind::ScreenUnavailable);
        assert_eq!(warning.code, "permission.screen_recording");
        assert!(matches!(
            warning.recovery,
            Some(RecoveryAction::OpenSystemSettings { .. })
        ));
    }

    #[tokio::test]
    async fn ocr_inside_the_soft_deadline_rides_with_the_snapshot() {
        let ocr = |_: &ScreenFrame| async { ocr_text("fn main()") };
        let branch = screen_branch(&screen_options(), async { frame() }, Some(ocr)).await;
        assert_eq!(branch.ocr.map(|o| o.text).as_deref(), Some("fn main()"));
        assert!(branch.frame.is_some() && branch.warning.is_none());
    }

    #[tokio::test]
    async fn slow_ocr_misses_the_deadline_and_finishes_in_the_background() {
        let (done_tx, done_rx) = tokio::sync::oneshot::channel();
        let slow = move |_: &ScreenFrame| async move {
            tokio::time::sleep(Duration::from_millis(1500)).await;
            let _ = done_tx.send(());
            ocr_text("late")
        };
        let started = Instant::now();
        let branch = screen_branch(&screen_options(), async { frame() }, Some(slow)).await;
        assert!(started.elapsed() < Duration::from_millis(1000));
        assert!(branch.ocr.is_none());
        assert!(branch.frame.is_some(), "the image stands in for the text");
        let kind = branch.warning.map(|w| w.kind);
        assert_eq!(kind, Some(SnapshotWarningKind::OcrPending));
        tokio::time::timeout(Duration::from_secs(10), done_rx)
            .await
            .expect("the OCR pass keeps running after the snapshot")
            .unwrap();
    }

    #[tokio::test]
    async fn without_an_inline_image_ocr_is_awaited() {
        let options = SnapshotOptions {
            inline_image: Some(false),
            ..screen_options()
        };
        let slow = |_: &ScreenFrame| async {
            tokio::time::sleep(OCR_SOFT_DEADLINE * 2).await;
            ocr_text("late")
        };
        let branch = screen_branch(&options, async { frame() }, Some(slow)).await;
        assert_eq!(branch.ocr.map(|o| o.text).as_deref(), Some("late"));
        assert!(branch.warning.is_none());
    }

    #[test]
    fn mime_strings_match_the_contract() {
        assert_eq!(mime_str(bluey_core::types::ImageMimeType::Png), "image/png");
        assert_eq!(
            mime_str(bluey_core::types::ImageMimeType::Jpeg),
            "image/jpeg"
        );
    }

    #[test]
    fn fixture_frames_carry_their_size_and_skip_the_helper() {
        assert_eq!(decoded_len("QUJD"), 3);
        assert_eq!(decoded_len("QUI="), 2);
        assert_eq!(decoded_len(""), 0);
        let fixture = FixtureFrame {
            base64: "QUJD".into(),
            bytes: 3,
            mime_type: ImageMimeType::Jpeg,
            width: 1440,
            height: 900,
        };
        let frame = frame_from_fixture(&fixture, &SnapshotOptions::default());
        assert_eq!(frame.image.as_deref(), Some("QUJD"));
        assert_eq!((frame.width, frame.height), (1440, 900));
        assert!(frame.id.starts_with("frame"));
        assert_eq!(frame.duration_ms, Some(0));
    }
}
