//! Developer-mode simulations (`dev_simulate`): synthetic transcript and
//! detections, a fixture-free screen capture, permission errors and the mock AI
//! provider's latency/failure knobs. Isolated from production paths — the
//! frontend only offers these when developer mode is on.

use bluey_core::events::BlueyEvent;
use bluey_core::types::{
    AudioSource, DetectedEvent, DetectedEventType, DevSimulation, OcrContext, OcrLevel,
    PermissionState, PermissionStatus,
};
use bluey_core::{new_id, now_iso, BlueyError, BlueyResult};

use crate::state::AppCore;

/// Speaker used for simulated questions when none is given.
const DEFAULT_SPEAKER: &str = "Interviewer";

/// Run one simulation against the live managers.
pub async fn simulate(core: &AppCore, simulation: DevSimulation) -> BlueyResult<()> {
    match simulation {
        DevSimulation::Question { text, speaker } => {
            let speaker = speaker.unwrap_or_else(|| DEFAULT_SPEAKER.to_string());
            let segment = core
                .audio
                .push_simulated(text.clone(), Some(speaker), AudioSource::System)
                .await;
            core.bus
                .publish(BlueyEvent::QuestionDetected(DetectedEvent {
                    id: new_id("det"),
                    event_type: DetectedEventType::Question,
                    confidence: 0.9,
                    requires_response: true,
                    text,
                    segment_ids: vec![segment.id],
                    speaker: segment.speaker,
                    detected_at: now_iso(),
                }));
        }
        DevSimulation::CodingProblem { text } => {
            core.bus.publish(BlueyEvent::OcrCompleted(OcrContext {
                blocks: Vec::new(),
                text: text.clone(),
                level: OcrLevel::Accurate,
                languages: vec!["en-US".into()],
                duration_ms: 0,
                frame_id: None,
            }));
            core.bus
                .publish(BlueyEvent::QuestionDetected(DetectedEvent {
                    id: new_id("det"),
                    event_type: DetectedEventType::CodingProblem,
                    confidence: 0.95,
                    requires_response: true,
                    text,
                    segment_ids: Vec::new(),
                    speaker: None,
                    detected_at: now_iso(),
                }));
        }
        DevSimulation::Transcript { segments } => {
            for part in segments {
                core.audio
                    .push_simulated(
                        part.text,
                        part.speaker,
                        part.source.unwrap_or(AudioSource::Microphone),
                    )
                    .await;
            }
        }
        DevSimulation::ScreenCapture { fixture: _ } => {
            // A real capture through the helper; the frame is published as
            // `screen.captured` by the capture manager.
            if let Err(e) = core.capture.capture(None).await {
                core.bus.publish(BlueyEvent::DevLog {
                    level: "warn".into(),
                    target: "bluey::dev".into(),
                    message: format!("simulated capture failed: {}", e.message),
                    at: now_iso(),
                });
            }
        }
        DevSimulation::PermissionError { permission } => {
            let mut state = core
                .permissions
                .cached()
                .unwrap_or_else(|| PermissionState::unknown(now_iso()));
            state.set(permission, PermissionStatus::Denied);
            state.checked_at = now_iso();
            core.bus.publish(BlueyEvent::PermissionsChanged(state));
            core.bus
                .publish(BlueyEvent::AppError(BlueyError::permission(
                    permission,
                    format!(
                        "{} permission is denied (simulated)",
                        permission.code_suffix()
                    ),
                )));
        }
        DevSimulation::AiLatency { ms } => core.dev.set_latency(ms),
        DevSimulation::AiFailure { code } => core.dev.set_failure(code),
        DevSimulation::Clear => {
            core.dev.clear();
            core.audio.clear(None).await?;
        }
    }
    Ok(())
}
