use super::*;
use crate::types::*;
use pretty_assertions::assert_eq;

fn segment() -> TranscriptSegment {
    TranscriptSegment {
        id: "seg_1".into(),
        session_id: None,
        speaker: Some("Speaker 1".into()),
        speaker_confidence: None,
        source: AudioSource::Microphone,
        text: "hello".into(),
        start_time: 0,
        end_time: 800,
        confidence: Some(0.9),
        finalized: true,
        language: None,
        created_at: crate::now_iso(),
    }
}

fn session() -> Session {
    Session {
        id: "ses_1".into(),
        mode_id: "general".into(),
        started_at: crate::now_iso(),
        ended_at: None,
        status: SessionStatus::Active,
        title: None,
        metadata: None,
    }
}

fn frame() -> ScreenFrame {
    ScreenFrame {
        id: "frm_1".into(),
        image: None,
        mime_type: ImageMimeType::Jpeg,
        path: None,
        width: 100,
        height: 100,
        display_id: None,
        scale_factor: 2.0,
        captured_at: crate::now_iso(),
        hash: None,
        changed: true,
        target: CaptureTarget::default(),
        duration_ms: None,
    }
}

fn response() -> BlueyResponse {
    BlueyResponse {
        id: "res_1".into(),
        request_id: "req_1".into(),
        session_id: None,
        mode_id: "general".into(),
        response_type: ResponseType::Answer,
        title: None,
        content: "42".into(),
        code: None,
        sections: None,
        citations: None,
        confidence: None,
        prompt: None,
        diagram: None,
        metrics: None,
        feedback: None,
        prepared: None,
        created_at: crate::now_iso(),
    }
}

fn detected() -> DetectedEvent {
    DetectedEvent {
        id: "det_1".into(),
        event_type: DetectedEventType::Question,
        confidence: 0.8,
        requires_response: true,
        text: "why?".into(),
        segment_ids: vec![],
        speaker: None,
        detected_at: crate::now_iso(),
    }
}

fn mode() -> BlueyMode {
    crate::modes::built_in_modes(&crate::now_iso()).remove(0)
}

/// One instance of every variant, in `events.ts` order.
fn all_events() -> Vec<BlueyEvent> {
    let err = BlueyError::internal("x");
    let device = AudioDevice {
        id: "dev_1".into(),
        name: "Mic".into(),
        is_default: true,
        kind: AudioDeviceKind::Input,
    };
    let session_event = SessionEvent {
        id: "sev_1".into(),
        session_id: "ses_1".into(),
        event_type: SessionEventType::QuestionDetected,
        title: "Question detected".into(),
        detail: None,
        refs: None,
        confidence: None,
        created_at: crate::now_iso(),
    };
    let ocr = OcrContext {
        blocks: vec![],
        text: String::new(),
        level: OcrLevel::Fast,
        languages: vec![],
        duration_ms: 1,
        frame_id: None,
    };
    let ax = AccessibilityContext {
        application: ApplicationContext {
            name: "App".into(),
            bundle_id: None,
            pid: None,
        },
        window: None,
        focused_element: None,
        elements: vec![],
        selected_text: None,
        visible_text: String::new(),
        truncated: false,
        captured_at: crate::now_iso(),
    };
    vec![
        BlueyEvent::AppState(AppStatus {
            state: AppState::Ready,
            audio_active: false,
            session_id: None,
            mode_id: "general".into(),
            error: None,
            resume_state: None,
            updated_at: crate::now_iso(),
        }),
        BlueyEvent::AppError(err.clone()),
        BlueyEvent::SettingsChanged(Settings::default()),
        BlueyEvent::PermissionsChanged(PermissionState::unknown(crate::now_iso())),
        BlueyEvent::AuthChanged(AuthStatus {
            state: AuthState::SignedOut,
            user: None,
            has_stored_session: false,
            configured: true,
            sign_in_pending: false,
            checked_at: crate::now_iso(),
        }),
        BlueyEvent::HelperStatus {
            running: true,
            version: None,
            restarted: None,
            error: None,
        },
        BlueyEvent::ScreenChanged {
            hash: "h".into(),
            delta: 0.2,
            display_id: None,
            at: crate::now_iso(),
        },
        BlueyEvent::ScreenCaptured(frame()),
        BlueyEvent::OcrCompleted(ocr),
        BlueyEvent::AccessibilityUpdated(ax),
        BlueyEvent::ActiveAppChanged {
            name: "Safari".into(),
            bundle_id: Some("com.apple.Safari".into()),
            pid: Some(42),
            window_title: None,
        },
        BlueyEvent::AudioStarted(AudioStatus::default()),
        BlueyEvent::AudioStopped(AudioStatus::default()),
        BlueyEvent::AudioPaused(AudioStatus::default()),
        BlueyEvent::AudioResumed(AudioStatus::default()),
        BlueyEvent::AudioLevel {
            microphone: 0.5,
            system: 0.1,
        },
        BlueyEvent::AudioChunk {
            source: AudioSource::System,
            start_ms: 0,
            end_ms: 320,
            is_speech: true,
            rms: 0.3,
        },
        BlueyEvent::AudioDeviceChanged {
            devices: vec![device.clone()],
            current_input: Some(device),
        },
        BlueyEvent::AudioError(err.clone()),
        BlueyEvent::TranscriptPartial(segment()),
        BlueyEvent::TranscriptFinal(segment()),
        BlueyEvent::TranscriptCleared {
            session_id: Some("ses_1".into()),
        },
        BlueyEvent::QuestionDetected(detected()),
        BlueyEvent::ContextUpdated {
            snapshot: ContextSnapshot::default(),
            reason: "capture".into(),
        },
        BlueyEvent::ResponsePrepared(response()),
        BlueyEvent::AiRequested {
            request_id: "req_1".into(),
            task: AiTask::Answer,
            session_id: None,
        },
        BlueyEvent::AiStarted {
            request_id: "req_1".into(),
            provider: "mock".into(),
            model: "m".into(),
        },
        BlueyEvent::AiChunk(AiChunk::Delta {
            request_id: "req_1".into(),
            text: "hi".into(),
        }),
        BlueyEvent::AiCompleted {
            request_id: "req_1".into(),
            total_ms: 10,
            time_to_first_token_ms: Some(2),
        },
        BlueyEvent::AiFailed {
            request_id: "req_1".into(),
            error: err.clone(),
        },
        BlueyEvent::AiCancelled {
            request_id: "req_1".into(),
        },
        BlueyEvent::ResearchEvent(DeepResearchEvent::Started {
            job_id: "job_1".into(),
        }),
        BlueyEvent::SessionStarted(session()),
        BlueyEvent::SessionPaused(session()),
        BlueyEvent::SessionResumed(session()),
        BlueyEvent::SessionEnded(session()),
        BlueyEvent::SessionEvent(session_event),
        BlueyEvent::ModeChanged {
            mode: mode(),
            session_id: None,
        },
        BlueyEvent::ModesChanged(vec![mode()]),
        BlueyEvent::ShortcutTriggered {
            id: ShortcutId::TogglePanel,
            at: crate::now_iso(),
        },
        BlueyEvent::PanelState(PanelState::default()),
        BlueyEvent::PanelScroll {
            direction: ScrollDirection::Down,
        },
        BlueyEvent::PanelFocusInput,
        BlueyEvent::PanelNewChat,
        BlueyEvent::DevMetrics(LatencyMetrics::default()),
        BlueyEvent::DevLog {
            level: "info".into(),
            target: "bluey".into(),
            message: "started".into(),
            at: crate::now_iso(),
        },
    ]
}

/// Hard-coded copy of EVENT_NAMES from `src/lib/tauri/events.ts`.
/// Keep in sync with the frontend.
const EVENT_NAMES: [&str; 46] = [
    "app.state",
    "app.error",
    "settings.changed",
    "permissions.changed",
    "auth.changed",
    "helper.status",
    "screen.changed",
    "screen.captured",
    "ocr.completed",
    "accessibility.updated",
    "activeApp.changed",
    "audio.started",
    "audio.stopped",
    "audio.paused",
    "audio.resumed",
    "audio.level",
    "audio.chunk",
    "audio.deviceChanged",
    "audio.error",
    "transcript.partial",
    "transcript.final",
    "transcript.cleared",
    "question.detected",
    "context.updated",
    "response.prepared",
    "ai.requested",
    "ai.started",
    "ai.chunk",
    "ai.completed",
    "ai.failed",
    "ai.cancelled",
    "research.event",
    "session.started",
    "session.paused",
    "session.resumed",
    "session.ended",
    "session.event",
    "mode.changed",
    "modes.changed",
    "shortcut.triggered",
    "panel.state",
    "panel.scroll",
    "panel.focusInput",
    "panel.newChat",
    "dev.metrics",
    "dev.log",
];

#[test]
fn names_match_frontend_event_list_exactly() {
    let names: Vec<&'static str> = all_events().iter().map(BlueyEvent::name).collect();
    assert_eq!(
        names,
        EVENT_NAMES.to_vec(),
        "one variant per events.ts entry, same order"
    );
}

#[test]
fn tauri_event_names_are_prefixed_and_slash_separated() {
    assert_eq!(
        BlueyEvent::PanelNewChat.tauri_event_name(),
        "bluey:panel/newChat"
    );
    assert_eq!(
        tauri_event_name_for("audio.deviceChanged"),
        "bluey:audio/deviceChanged"
    );
}

/// Tauri v2 rejects event names outside `[A-Za-z0-9-/:_]`; a dot in a wire
/// name means `emit` fails and the WebView never hears the event.
#[test]
fn every_wire_name_is_accepted_by_tauri() {
    assert!(is_valid_tauri_event_name("bluey:app/state"));
    assert!(!is_valid_tauri_event_name("bluey:app.state"));
    assert!(!is_valid_tauri_event_name(""));
    let rejected: Vec<String> = all_events()
        .iter()
        .map(BlueyEvent::tauri_event_name)
        .filter(|wire| !is_valid_tauri_event_name(wire))
        .collect();
    assert_eq!(
        rejected,
        Vec::<String>::new(),
        "wire names Tauri would reject"
    );
}

#[test]
fn payloads_are_camel_case_and_match_ts_shapes() {
    let p = BlueyEvent::AudioChunk {
        source: AudioSource::Microphone,
        start_ms: 5,
        end_ms: 25,
        is_speech: false,
        rms: 0.1,
    }
    .payload();
    assert_eq!(p["source"], "microphone");
    assert_eq!(p["startMs"], 5);
    assert_eq!(p["endMs"], 25);
    assert_eq!(p["isSpeech"], false);

    let p = BlueyEvent::HelperStatus {
        running: true,
        version: None,
        restarted: None,
        error: None,
    }
    .payload();
    assert_eq!(p["running"], true);
    assert!(
        p.get("version").is_none(),
        "optional fields are omitted when None"
    );

    let p = BlueyEvent::ShortcutTriggered {
        id: ShortcutId::CaptureAnalyze,
        at: "t".into(),
    }
    .payload();
    assert_eq!(p["id"], "capture_analyze");

    let p = BlueyEvent::PanelScroll {
        direction: ScrollDirection::Up,
    }
    .payload();
    assert_eq!(p["direction"], "up");

    let p = BlueyEvent::AiRequested {
        request_id: "req_9".into(),
        task: AiTask::SystemDesign,
        session_id: Some("ses_2".into()),
    }
    .payload();
    assert_eq!(p["requestId"], "req_9");
    assert_eq!(p["task"], "system_design");
    assert_eq!(p["sessionId"], "ses_2");

    assert_eq!(BlueyEvent::PanelFocusInput.payload(), serde_json::json!({}));
    assert_eq!(BlueyEvent::PanelNewChat.payload(), serde_json::json!({}));

    // Payload-carrying wrappers serialize the inner type directly.
    let p = BlueyEvent::TranscriptFinal(segment()).payload();
    assert_eq!(p["startTime"], 0);
    assert_eq!(p["source"], "microphone");
}

#[test]
fn every_payload_serializes() {
    for e in all_events() {
        let p = e.payload();
        assert!(!p.is_null(), "payload of {} must serialize", e.name());
    }
}

#[test]
fn recording_sink_records_and_drains() {
    let sink = RecordingSink::new();
    sink.publish(BlueyEvent::PanelNewChat);
    sink.publish(BlueyEvent::AudioLevel {
        microphone: 0.2,
        system: 0.0,
    });
    assert_eq!(sink.names(), vec!["panel.newChat", "audio.level"]);
    assert_eq!(sink.events().len(), 2);
    assert_eq!(sink.take().len(), 2);
    assert!(sink.events().is_empty());

    // NullSink accepts anything silently.
    NullSink.publish(BlueyEvent::PanelFocusInput);
}
