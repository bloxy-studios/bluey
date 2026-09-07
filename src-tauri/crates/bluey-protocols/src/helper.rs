//! Native-helper wire types (see `docs/HELPER_PROTOCOL.md`) and their mappers
//! onto `bluey_core` types, including speaker labelling and stable-id
//! assignment for streaming transcript segments.

use std::collections::HashMap;

use bluey_core::types::{
    AccessibilityContext, ApplicationContext, AudioDevice, AudioSource, BoundingBox, CaptureTarget,
    DisplayInfo, ImageMimeType, OcrBlock, OcrContext, OcrLevel, PermissionKind, PermissionStatus,
    ScreenFrame, TranscriptSegment, WindowContext,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── Frames ───────────────────────────────────────────────────────────────────

/// `Frame` as returned by every `capture.*` method.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WireFrame {
    pub id: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub mime_type: Option<String>,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub display_id: Option<String>,
    #[serde(default = "default_scale")]
    pub scale_factor: f64,
    pub captured_at: String,
    #[serde(default)]
    pub hash: Option<String>,
    #[serde(default = "default_true")]
    pub changed: bool,
    #[serde(default)]
    pub duration_ms: Option<u64>,
}

fn default_scale() -> f64 {
    1.0
}
fn default_true() -> bool {
    true
}

/// Map a helper frame onto [`ScreenFrame`], attaching the capture target the
/// app requested (the helper does not echo it back).
pub fn frame_to_screen_frame(frame: WireFrame, target: CaptureTarget) -> ScreenFrame {
    ScreenFrame {
        id: frame.id,
        image: frame.image,
        mime_type: mime_from_str(frame.mime_type.as_deref()),
        path: frame.path,
        width: frame.width,
        height: frame.height,
        display_id: frame.display_id,
        scale_factor: frame.scale_factor,
        captured_at: frame.captured_at,
        hash: frame.hash,
        changed: frame.changed,
        target,
        duration_ms: frame.duration_ms,
    }
}

fn mime_from_str(mime: Option<&str>) -> ImageMimeType {
    match mime {
        Some("image/png") => ImageMimeType::Png,
        Some("image/webp") => ImageMimeType::Webp,
        _ => ImageMimeType::Jpeg,
    }
}

// ── OCR ──────────────────────────────────────────────────────────────────────

/// `ocr.recognize` result.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WireOcrResult {
    #[serde(default)]
    pub blocks: Vec<WireOcrBlock>,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WireOcrBlock {
    pub text: String,
    #[serde(default)]
    pub confidence: f32,
    #[serde(default)]
    pub bounding_box: BoundingBox,
}

/// Map an OCR result onto [`OcrContext`], attaching the request parameters the
/// helper does not echo back.
pub fn ocr_to_context(
    result: WireOcrResult,
    level: OcrLevel,
    languages: Vec<String>,
    frame_id: Option<String>,
) -> OcrContext {
    OcrContext {
        blocks: result
            .blocks
            .into_iter()
            .map(|b| OcrBlock {
                text: b.text,
                confidence: b.confidence,
                bounding_box: b.bounding_box,
            })
            .collect(),
        text: result.text,
        level,
        languages,
        duration_ms: result.duration_ms,
        frame_id,
    }
}

// ── Accessibility / frontmost ────────────────────────────────────────────────

/// Parse an `accessibility.snapshot` result (wire shape matches
/// [`AccessibilityContext`] camelCase field-for-field).
pub fn parse_ax_snapshot(value: Value) -> Result<AccessibilityContext, serde_json::Error> {
    serde_json::from_value(value)
}

/// `app.frontmost` result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontmostApp {
    pub application: ApplicationContext,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<WindowContext>,
}

// ── Displays / windows ───────────────────────────────────────────────────────

/// `displays.list` result envelope; entries match [`DisplayInfo`] exactly.
#[derive(Debug, Clone, Deserialize)]
pub struct DisplaysResult {
    #[serde(default)]
    pub displays: Vec<DisplayInfo>,
}

/// One window from `windows.list`. Mirrors the `CapturableWindow` interface in
/// `src/lib/tauri/commands.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapturableWindow {
    pub window_id: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub owner_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_id: Option<String>,
    pub pid: i32,
    pub bounds: BoundingBox,
    pub on_screen: bool,
}

/// `windows.list` result envelope.
#[derive(Debug, Clone, Deserialize)]
pub struct WindowsResult {
    #[serde(default)]
    pub windows: Vec<CapturableWindow>,
}

// ── Permissions ──────────────────────────────────────────────────────────────

/// `permissions.status` result (helper-side kinds only; notifications are
/// handled in Rust).
#[derive(Debug, Clone, Copy, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WirePermissions {
    #[serde(default)]
    pub screen_recording: WireStatus,
    #[serde(default)]
    pub microphone: WireStatus,
    #[serde(default)]
    pub accessibility: WireStatus,
    #[serde(default)]
    pub speech_recognition: WireStatus,
}

/// Helper permission status strings.
#[derive(Debug, Clone, Copy, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WireStatus {
    Granted,
    Denied,
    NotDetermined,
    Restricted,
    #[default]
    #[serde(other)]
    Unknown,
}

impl From<WireStatus> for PermissionStatus {
    fn from(status: WireStatus) -> Self {
        match status {
            WireStatus::Granted => PermissionStatus::Granted,
            WireStatus::Denied => PermissionStatus::Denied,
            WireStatus::NotDetermined => PermissionStatus::NotDetermined,
            WireStatus::Restricted => PermissionStatus::Restricted,
            WireStatus::Unknown => PermissionStatus::Unknown,
        }
    }
}

/// The `kind` string a `permissions.request`/`permissions.status` call uses
/// for a [`PermissionKind`]. Notifications are not helper-managed → `None`.
pub fn permission_wire_kind(kind: PermissionKind) -> Option<&'static str> {
    match kind {
        PermissionKind::Microphone => Some("microphone"),
        PermissionKind::ScreenRecording => Some("screenRecording"),
        PermissionKind::Accessibility => Some("accessibility"),
        PermissionKind::SpeechRecognition => Some("speechRecognition"),
        PermissionKind::Notifications => None,
    }
}

// ── helper.version ───────────────────────────────────────────────────────────

/// `helper.version` result.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct HelperVersion {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub macos: Option<String>,
    #[serde(default)]
    pub arch: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Protocol major version; assumed 1 when the helper predates the field.
    #[serde(default = "default_protocol")]
    pub protocol: u64,
}

fn default_protocol() -> u64 {
    1
}

/// The protocol major version this client speaks.
pub const PROTOCOL_MAJOR: u64 = 1;

impl HelperVersion {
    /// Whether this helper can be driven by the current client.
    pub fn is_compatible(&self) -> bool {
        self.protocol == PROTOCOL_MAJOR
    }

    /// Whether the helper reports a capability (e.g. `"audio.system"`).
    pub fn has_capability(&self, name: &str) -> bool {
        self.capabilities.iter().any(|c| c == name)
    }
}

// ── Audio / transcript events ────────────────────────────────────────────────

/// `audio.chunk` event data.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WireAudioChunk {
    pub source: AudioSource,
    #[serde(default)]
    pub pcm16: Option<String>,
    #[serde(default)]
    pub sample_rate: Option<u32>,
    #[serde(default)]
    pub start_ms: u64,
    #[serde(default)]
    pub end_ms: u64,
    #[serde(default)]
    pub is_speech: bool,
    #[serde(default)]
    pub rms: f32,
}

/// `transcript.partial` / `transcript.final` event data.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WireTranscript {
    pub source: AudioSource,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub start_ms: u64,
    #[serde(default)]
    pub end_ms: u64,
    #[serde(default)]
    pub confidence: Option<f32>,
    #[serde(default)]
    pub locale: Option<String>,
}

/// `audio.deviceChanged` event data.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WireDeviceChange {
    #[serde(default)]
    pub devices: Vec<AudioDevice>,
    #[serde(default)]
    pub current_input: Option<AudioDevice>,
}

/// `audio.devices` result envelope.
#[derive(Debug, Clone, Deserialize)]
pub struct DevicesResult {
    #[serde(default)]
    pub devices: Vec<AudioDevice>,
}

/// Typed helper event, decoded from the `event`/`data` envelope.
#[derive(Debug, Clone)]
pub enum HelperEvent {
    Ready {
        version: Option<String>,
    },
    ScreenChanged {
        hash: String,
        delta: f64,
        display_id: Option<String>,
        at: String,
    },
    AudioStarted {
        microphone: bool,
        system_audio: bool,
        device: Option<AudioDevice>,
    },
    AudioStopped {
        reason: String,
    },
    AudioLevel {
        microphone: f32,
        system: f32,
    },
    AudioChunk(WireAudioChunk),
    AudioDeviceChanged(WireDeviceChange),
    AudioError(crate::jsonl::WireError),
    TranscriptPartial(WireTranscript),
    TranscriptFinal(WireTranscript),
    Unknown {
        event: String,
    },
}

/// Decode one helper event by name. Undecodable payloads degrade to `Unknown`
/// (the caller logs the event name only — data may contain transcript text).
pub fn parse_helper_event(event: &str, data: Value) -> HelperEvent {
    fn de<T: serde::de::DeserializeOwned>(data: Value) -> Option<T> {
        serde_json::from_value(data).ok()
    }
    let unknown = || HelperEvent::Unknown {
        event: event.to_string(),
    };
    match event {
        "helper.ready" => HelperEvent::Ready {
            version: data
                .get("version")
                .and_then(Value::as_str)
                .map(String::from),
        },
        "screen.changed" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Changed {
                #[serde(default)]
                hash: String,
                #[serde(default)]
                delta: f64,
                #[serde(default)]
                display_id: Option<String>,
                #[serde(default)]
                at: String,
            }
            match de::<Changed>(data) {
                Some(c) => HelperEvent::ScreenChanged {
                    hash: c.hash,
                    delta: c.delta,
                    display_id: c.display_id,
                    at: c.at,
                },
                None => unknown(),
            }
        }
        "audio.started" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Started {
                #[serde(default)]
                microphone: bool,
                #[serde(default)]
                system_audio: bool,
                #[serde(default)]
                device: Option<AudioDevice>,
            }
            match de::<Started>(data) {
                Some(s) => HelperEvent::AudioStarted {
                    microphone: s.microphone,
                    system_audio: s.system_audio,
                    device: s.device,
                },
                None => unknown(),
            }
        }
        "audio.stopped" => HelperEvent::AudioStopped {
            reason: data
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("requested")
                .to_string(),
        },
        "audio.level" => {
            #[derive(Deserialize)]
            struct Level {
                #[serde(default)]
                microphone: f32,
                #[serde(default)]
                system: f32,
            }
            match de::<Level>(data) {
                Some(l) => HelperEvent::AudioLevel {
                    microphone: l.microphone,
                    system: l.system,
                },
                None => unknown(),
            }
        }
        "audio.chunk" => de::<WireAudioChunk>(data)
            .map(HelperEvent::AudioChunk)
            .unwrap_or_else(unknown),
        "audio.deviceChanged" => de::<WireDeviceChange>(data)
            .map(HelperEvent::AudioDeviceChanged)
            .unwrap_or_else(unknown),
        "audio.error" => de::<crate::jsonl::WireError>(data)
            .map(HelperEvent::AudioError)
            .unwrap_or_else(unknown),
        "transcript.partial" => de::<WireTranscript>(data)
            .map(HelperEvent::TranscriptPartial)
            .unwrap_or_else(unknown),
        "transcript.final" => de::<WireTranscript>(data)
            .map(HelperEvent::TranscriptFinal)
            .unwrap_or_else(unknown),
        _ => unknown(),
    }
}

// ── Speaker labelling & transcript assembly ──────────────────────────────────

/// Speaker label + confidence for a transcript source given the active mode:
/// microphone is always the user ("You", 0.95); system audio depends on the
/// mode family (0.6).
pub fn speaker_label(source: AudioSource, mode_id: &str) -> (&'static str, f32) {
    match source {
        AudioSource::Microphone => ("You", 0.95),
        AudioSource::System => {
            let label = match mode_id {
                "interview"
                | "behavioral-interview"
                | "coding-interview"
                | "system-design"
                | "case-interview" => "Interviewer",
                "sales" => "Customer",
                "recruiting" => "Candidate",
                _ => "Speaker",
            };
            (label, 0.6)
        }
    }
}

/// Assigns stable segment ids across partial → final updates of the same
/// utterance (keyed by source + utterance key) and applies speaker labels.
///
/// The utterance key is the helper's `startMs` for Apple Speech (stable across
/// partials of one utterance) or the realtime API's `item_id` for the cloud
/// path (callers pass it via [`TranscriptAssembler::ingest_keyed`]).
#[derive(Debug, Default)]
pub struct TranscriptAssembler {
    pending: HashMap<(AudioSource, String), String>,
    counter: u64,
}

impl TranscriptAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drop all pending partial-id state (audio stopped).
    pub fn reset(&mut self) {
        self.pending.clear();
    }

    /// Ingest a helper transcript event keyed by its `startMs`.
    #[allow(clippy::too_many_arguments)]
    pub fn ingest(
        &mut self,
        event: &WireTranscript,
        finalized: bool,
        session_id: Option<&str>,
        mode_id: &str,
        speaker_identification: bool,
        now_iso: &str,
        new_id: impl FnMut() -> String,
    ) -> TranscriptSegment {
        let key = event.start_ms.to_string();
        self.assemble(
            event,
            &key,
            finalized,
            session_id,
            mode_id,
            speaker_identification,
            now_iso,
            new_id,
        )
    }

    /// Ingest with an explicit utterance key (cloud realtime `item_id`).
    #[allow(clippy::too_many_arguments)]
    pub fn ingest_keyed(
        &mut self,
        event: &WireTranscript,
        key: &str,
        finalized: bool,
        session_id: Option<&str>,
        mode_id: &str,
        speaker_identification: bool,
        now_iso: &str,
        new_id: impl FnMut() -> String,
    ) -> TranscriptSegment {
        self.assemble(
            event,
            key,
            finalized,
            session_id,
            mode_id,
            speaker_identification,
            now_iso,
            new_id,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn assemble(
        &mut self,
        event: &WireTranscript,
        key: &str,
        finalized: bool,
        session_id: Option<&str>,
        mode_id: &str,
        speaker_identification: bool,
        now_iso: &str,
        mut new_id: impl FnMut() -> String,
    ) -> TranscriptSegment {
        self.counter += 1;
        let map_key = (event.source, key.to_string());
        let id = if finalized {
            self.pending.remove(&map_key).unwrap_or_else(&mut new_id)
        } else {
            self.pending
                .entry(map_key)
                .or_insert_with(&mut new_id)
                .clone()
        };
        let (speaker, speaker_confidence) = if speaker_identification {
            let (label, confidence) = speaker_label(event.source, mode_id);
            (Some(label.to_string()), Some(confidence))
        } else {
            (None, None)
        };
        TranscriptSegment {
            id,
            session_id: session_id.map(String::from),
            speaker,
            speaker_confidence,
            source: event.source,
            text: event.text.clone(),
            start_time: event.start_ms,
            end_time: event.end_ms.max(event.start_ms),
            confidence: event.confidence,
            finalized,
            language: event.locale.clone(),
            created_at: now_iso.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn frame_mapping_defaults() {
        let raw = r#"{
          "id": "f-1", "path": "/tmp/f-1.jpg", "image": null, "mimeType": "image/jpeg",
          "width": 1600, "height": 1035, "displayId": "69733382", "scaleFactor": 2,
          "capturedAt": "2026-09-07T12:00:00.000Z", "hash": "a3f0", "changed": true,
          "durationMs": 87
        }"#;
        let frame: WireFrame = serde_json::from_str(raw).unwrap();
        let screen = frame_to_screen_frame(frame, CaptureTarget::ActiveWindow);
        assert_eq!(screen.id, "f-1");
        assert_eq!(screen.mime_type, ImageMimeType::Jpeg);
        assert_eq!(screen.path.as_deref(), Some("/tmp/f-1.jpg"));
        assert_eq!(screen.target, CaptureTarget::ActiveWindow);
        assert_eq!(screen.duration_ms, Some(87));
        assert!(screen.changed);

        // Minimal frame: unknown mime → jpeg, changed defaults true.
        let frame: WireFrame =
            serde_json::from_str(r#"{"id":"f-2","width":10,"height":10,"capturedAt":"t"}"#)
                .unwrap();
        let screen = frame_to_screen_frame(frame, CaptureTarget::default());
        assert_eq!(screen.mime_type, ImageMimeType::Jpeg);
        assert_eq!(screen.scale_factor, 1.0);
        assert!(screen.changed);
    }

    #[test]
    fn ocr_mapping_attaches_request_params() {
        let raw = r#"{
          "blocks": [ { "text": "Hello", "confidence": 0.98,
                        "boundingBox": { "x": 0.1, "y": 0.2, "width": 0.3, "height": 0.05 } } ],
          "text": "Hello", "width": 1600, "height": 1000, "durationMs": 42
        }"#;
        let result: WireOcrResult = serde_json::from_str(raw).unwrap();
        let ocr = ocr_to_context(
            result,
            OcrLevel::Fast,
            vec!["en-US".into()],
            Some("f-1".into()),
        );
        assert_eq!(ocr.text, "Hello");
        assert_eq!(ocr.blocks[0].confidence, 0.98);
        assert_eq!(ocr.blocks[0].bounding_box.width, 0.3);
        assert_eq!(ocr.level, OcrLevel::Fast);
        assert_eq!(ocr.frame_id.as_deref(), Some("f-1"));
        assert_eq!(ocr.duration_ms, 42);
    }

    #[test]
    fn ax_snapshot_parses_into_core_type() {
        let raw = serde_json::json!({
            "application": { "name": "Code", "bundleId": "com.microsoft.VSCode", "pid": 913 },
            "window": { "title": "main.rs — bluey", "windowId": 88 },
            "focusedElement": { "role": "AXTextArea", "label": "Editor", "focused": true, "depth": 0 },
            "elements": [ { "role": "AXButton", "title": "Run", "depth": 1 } ],
            "selectedText": "let x",
            "visibleText": "fn main() {}",
            "truncated": false,
            "capturedAt": "2026-09-07T12:00:00.000Z"
        });
        let ax = parse_ax_snapshot(raw).unwrap();
        assert_eq!(
            ax.application.bundle_id.as_deref(),
            Some("com.microsoft.VSCode")
        );
        assert_eq!(ax.window.unwrap().window_id, Some(88));
        assert_eq!(ax.focused_element.unwrap().role, "AXTextArea");
        assert_eq!(ax.elements.len(), 1);
        assert_eq!(ax.selected_text.as_deref(), Some("let x"));
    }

    #[test]
    fn permission_status_and_kind_mapping() {
        let raw = r#"{ "screenRecording": "granted", "microphone": "denied",
                       "accessibility": "not_determined", "speechRecognition": "restricted" }"#;
        let p: WirePermissions = serde_json::from_str(raw).unwrap();
        assert_eq!(
            PermissionStatus::from(p.screen_recording),
            PermissionStatus::Granted
        );
        assert_eq!(
            PermissionStatus::from(p.microphone),
            PermissionStatus::Denied
        );
        assert_eq!(
            PermissionStatus::from(p.accessibility),
            PermissionStatus::NotDetermined
        );
        assert_eq!(
            PermissionStatus::from(p.speech_recognition),
            PermissionStatus::Restricted
        );
        assert_eq!(
            permission_wire_kind(PermissionKind::ScreenRecording),
            Some("screenRecording")
        );
        assert_eq!(permission_wire_kind(PermissionKind::Notifications), None);
    }

    #[test]
    fn version_compatibility() {
        let v: HelperVersion = serde_json::from_str(
            r#"{ "version": "0.3.0", "macos": "15.1", "arch": "arm64",
                 "capabilities": ["capture", "audio.system"], "protocol": 1 }"#,
        )
        .unwrap();
        assert!(v.is_compatible());
        assert!(v.has_capability("audio.system"));
        assert!(!v.has_capability("speech.onDevice"));

        let incompatible: HelperVersion =
            serde_json::from_str(r#"{ "version": "9", "protocol": 2 }"#).unwrap();
        assert!(!incompatible.is_compatible());

        // Missing protocol field → assumed 1.
        let legacy: HelperVersion = serde_json::from_str(r#"{ "version": "0.1" }"#).unwrap();
        assert!(legacy.is_compatible());
    }

    #[test]
    fn helper_event_decoding() {
        let e = parse_helper_event(
            "screen.changed",
            serde_json::json!({ "hash": "ff", "delta": 0.2, "displayId": "1", "at": "t" }),
        );
        match e {
            HelperEvent::ScreenChanged {
                hash,
                delta,
                display_id,
                at,
            } => {
                assert_eq!(hash, "ff");
                assert_eq!(delta, 0.2);
                assert_eq!(display_id.as_deref(), Some("1"));
                assert_eq!(at, "t");
            }
            other => panic!("unexpected: {other:?}"),
        }

        let e = parse_helper_event(
            "transcript.final",
            serde_json::json!({ "source": "system", "text": "Tell me about yourself.",
                                 "startMs": 1000, "endMs": 2500, "confidence": 0.9, "locale": "en-US" }),
        );
        match e {
            HelperEvent::TranscriptFinal(t) => {
                assert_eq!(t.source, AudioSource::System);
                assert_eq!(t.start_ms, 1000);
            }
            other => panic!("unexpected: {other:?}"),
        }

        match parse_helper_event("something.new", serde_json::json!({})) {
            HelperEvent::Unknown { event } => assert_eq!(event, "something.new"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn speaker_labels_by_mode() {
        assert_eq!(
            speaker_label(AudioSource::Microphone, "sales"),
            ("You", 0.95)
        );
        assert_eq!(
            speaker_label(AudioSource::System, "interview"),
            ("Interviewer", 0.6)
        );
        assert_eq!(
            speaker_label(AudioSource::System, "coding-interview"),
            ("Interviewer", 0.6)
        );
        assert_eq!(
            speaker_label(AudioSource::System, "sales"),
            ("Customer", 0.6)
        );
        assert_eq!(
            speaker_label(AudioSource::System, "recruiting"),
            ("Candidate", 0.6)
        );
        assert_eq!(
            speaker_label(AudioSource::System, "general"),
            ("Speaker", 0.6)
        );
        assert_eq!(
            speaker_label(AudioSource::System, "team-meeting"),
            ("Speaker", 0.6)
        );
    }

    #[test]
    fn assembler_keeps_ids_stable_across_partials() {
        let mut assembler = TranscriptAssembler::new();
        let mut n = 0u32;
        let mut next = || {
            n += 1;
            format!("seg_{n}")
        };
        let event = WireTranscript {
            source: AudioSource::System,
            text: "Tell me".into(),
            start_ms: 1000,
            end_ms: 1500,
            confidence: None,
            locale: None,
        };
        let p1 = assembler.ingest(
            &event,
            false,
            Some("ses_1"),
            "interview",
            true,
            "t1",
            &mut next,
        );
        assert_eq!(p1.id, "seg_1");
        assert!(!p1.finalized);
        assert_eq!(p1.speaker.as_deref(), Some("Interviewer"));
        assert_eq!(p1.speaker_confidence, Some(0.6));
        assert_eq!(p1.session_id.as_deref(), Some("ses_1"));

        let mut longer = event.clone();
        longer.text = "Tell me about".into();
        longer.end_ms = 1900;
        let p2 = assembler.ingest(
            &longer,
            false,
            Some("ses_1"),
            "interview",
            true,
            "t2",
            &mut next,
        );
        assert_eq!(p2.id, "seg_1", "same utterance keeps its id");

        let mut done = longer.clone();
        done.text = "Tell me about yourself.".into();
        done.end_ms = 2500;
        let f = assembler.ingest(
            &done,
            true,
            Some("ses_1"),
            "interview",
            true,
            "t3",
            &mut next,
        );
        assert_eq!(f.id, "seg_1");
        assert!(f.finalized);

        // The next utterance gets a fresh id even with the same startMs after reset.
        let f2 = assembler.ingest(&done, true, None, "general", false, "t4", &mut next);
        assert_eq!(f2.id, "seg_2", "final without partial gets a fresh id");
        assert_eq!(f2.speaker, None, "speaker identification off");
        assert_eq!(f2.session_id, None);
    }

    #[test]
    fn assembler_keyed_by_item_id() {
        let mut assembler = TranscriptAssembler::new();
        let mut n = 0u32;
        let mut next = || {
            n += 1;
            format!("seg_{n}")
        };
        let event = WireTranscript {
            source: AudioSource::Microphone,
            text: "hi".into(),
            start_ms: 0,
            end_ms: 400,
            confidence: None,
            locale: None,
        };
        let a = assembler.ingest_keyed(
            &event, "item_1", false, None, "general", true, "t", &mut next,
        );
        let b = assembler.ingest_keyed(
            &event, "item_1", true, None, "general", true, "t", &mut next,
        );
        assert_eq!(a.id, b.id);
        assert_eq!(a.speaker.as_deref(), Some("You"));
        // end < start is clamped
        let bad = WireTranscript {
            start_ms: 500,
            end_ms: 100,
            ..event
        };
        let c = assembler.ingest_keyed(&bad, "item_2", true, None, "general", true, "t", &mut next);
        assert_eq!(c.end_time, 500);
    }
}
