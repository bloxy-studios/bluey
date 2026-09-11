//! The ⌘↵ fast path: assemble a [`ContextSnapshot`] from the helper (frontmost
//! app, screen capture + OCR, accessibility) **in parallel**, the transcript ring
//! buffer, the active session and mode, then trim it (`bluey_core::context`)
//! and tag it with an application adapter. Timings feed the dev overlay; the
//! monotonic stamps on `ContextSnapshot::trace` (capture done, reply) are the
//! Rust half of the fast-path trace the WebView anchors on (ADR 0010 §2).

use std::collections::BTreeMap;
use std::time::Instant;

use bluey_core::context::{apply_adapter, trim_snapshot, SnapshotLimits};
use bluey_core::events::BlueyEvent;
use bluey_core::types::{
    AppEvent, CaptureTarget, ContextSnapshot, DocumentScope, ImageMimeType, ModeContext,
    RecentResponseRef, ScreenFrame, ScreenSummary, SessionContext, SnapshotOptions, SnapshotTrace,
    TranscriptContext,
};
use bluey_core::{new_id, BlueyResult};
use bluey_storage::{
    DocumentRepository, ResponseRepository, SessionEventRepository, SessionNoteRepository,
};

use crate::state::AppCore;

/// Recent responses/events included in the session context.
const RECENT_RESPONSES: u32 = 5;
const RECENT_EVENTS: usize = 10;
const DEFAULT_TRANSCRIPT_WINDOW_S: u32 = 120;

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

/// Build the snapshot for `options`. Failures of individual sources degrade
/// gracefully (the field stays `None`); only a missing helper for a requested
/// screen capture is an error.
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

    // Frontmost app + accessibility + capture run concurrently; OCR follows the capture.
    let frontmost = async { core.ax.frontmost().await.ok() };
    let accessibility = async {
        if !options.include_accessibility {
            return (None, 0u64);
        }
        let t = Instant::now();
        let result = core.ax.snapshot(None, None).await;
        (result.ok(), t.elapsed().as_millis() as u64)
    };
    let capture_and_ocr = async {
        if !options.include_screen {
            return Ok::<_, bluey_core::BlueyError>((None, None, 0u64, 0u64, None));
        }
        let t = Instant::now();
        let frame = match fixture {
            Some(fixture) => frame_from_fixture(fixture, &options),
            None => core.capture.capture(options.capture.clone()).await?,
        };
        let capture_done_ms = crate::clock::mono_ms();
        let capture_ms = t.elapsed().as_millis() as u64;
        let mut ocr = None;
        let mut ocr_ms = 0u64;
        if options.include_ocr && fixture.is_none() {
            let t = Instant::now();
            match core
                .capture
                .ocr(
                    &frame.id,
                    options.ocr_level,
                    None,
                    frame.hash.as_deref(),
                    frame.changed,
                )
                .await
            {
                Ok(context) => ocr = Some(context),
                Err(e) => tracing::warn!(error = %e, "OCR failed; continuing without it"),
            }
            ocr_ms = t.elapsed().as_millis() as u64;
        }
        Ok((Some(frame), ocr, capture_ms, ocr_ms, Some(capture_done_ms)))
    };
    let (frontmost, (accessibility, ax_ms), captured) =
        tokio::join!(frontmost, accessibility, capture_and_ocr);
    let (frame, ocr, capture_ms, ocr_ms, capture_done_ms) = captured?;

    let mut snapshot = ContextSnapshot {
        timestamp: bluey_core::now_iso(),
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

    snapshot.session = session_context(core).await;
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
            let mut events = SessionEventRepository::list(db, &id)?;
            if events.len() > RECENT_EVENTS {
                events.drain(..events.len() - RECENT_EVENTS);
            }
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
