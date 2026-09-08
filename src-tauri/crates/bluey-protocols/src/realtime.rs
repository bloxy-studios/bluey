//! OpenAI Realtime transcription WebSocket messages, also served by Azure at
//! `/openai/v1/realtime?intent=transcription`.
//!
//! The `model` in `session.update` must be a *realtime-capable OpenAI STT
//! model* (`gpt-realtime-whisper`, `gpt-4o-transcribe`,
//! `gpt-4o-mini-transcribe`, `gpt-4o-transcribe-diarize`). Those deployments
//! are often missing on Foundry. Microsoft's MAI-Transcribe models stream
//! through Voice Live instead — see [`crate::voice_live`]. Incoming event
//! names are shared, so [`parse_event`] and [`append_audio`] work on both.

use serde::Deserialize;
use serde_json::{json, Value};

/// OpenAI-hosted realtime transcription endpoint.
pub const OPENAI_REALTIME_URL: &str = "wss://api.openai.com/v1/realtime?intent=transcription";

/// PCM sample rate the app requests from the helper for the cloud path.
pub const CLOUD_SAMPLE_RATE: u32 = 24_000;

/// `session.update` configuring a transcription session with server VAD.
/// `language: None` (or "auto") omits the language field for auto-detection.
pub fn session_update(model: &str, language: Option<&str>) -> Value {
    let mut transcription = json!({ "model": model });
    if let Some(lang) = language {
        if !lang.is_empty() && lang != "auto" {
            transcription["language"] = json!(primary_language_tag(lang));
        }
    }
    json!({
        "type": "session.update",
        "session": {
            "type": "transcription",
            "audio": {
                "input": {
                    "format": { "type": "audio/pcm", "rate": CLOUD_SAMPLE_RATE },
                    "turn_detection": { "type": "server_vad" },
                    "transcription": transcription,
                }
            }
        }
    })
}

/// Realtime `language` wants a primary tag (`en`), not a full BCP-47 (`en-US`).
pub(crate) fn primary_language_tag(tag: &str) -> String {
    tag.split(['-', '_']).next().unwrap_or(tag).to_lowercase()
}

/// `input_audio_buffer.append` with base64 PCM16 audio.
pub fn append_audio(base64_pcm: &str) -> Value {
    json!({ "type": "input_audio_buffer.append", "audio": base64_pcm })
}

/// Incoming realtime events Bluey consumes.
#[derive(Debug, Clone, PartialEq)]
pub enum RealtimeEvent {
    /// Incremental transcript text for the current utterance.
    Delta {
        item_id: Option<String>,
        text: String,
    },
    /// Final transcript for one utterance.
    Completed {
        item_id: Option<String>,
        transcript: String,
    },
    /// Transcription failed for one utterance.
    Failed { message: String },
    /// Fatal server error.
    Error { message: String },
    /// Session lifecycle acks and everything else Bluey ignores.
    Other,
}

/// Parse one incoming WebSocket text message.
pub fn parse_event(raw: &str) -> RealtimeEvent {
    #[derive(Deserialize)]
    struct Envelope {
        #[serde(rename = "type", default)]
        kind: String,
        #[serde(default)]
        item_id: Option<String>,
        #[serde(default)]
        delta: Option<String>,
        #[serde(default)]
        transcript: Option<String>,
        #[serde(default)]
        error: Option<ErrorBody>,
    }
    #[derive(Deserialize)]
    struct ErrorBody {
        #[serde(default)]
        message: Option<String>,
        #[serde(rename = "type", default)]
        error_type: Option<String>,
    }

    let Ok(env) = serde_json::from_str::<Envelope>(raw) else {
        return RealtimeEvent::Other;
    };
    match env.kind.as_str() {
        "conversation.item.input_audio_transcription.delta" => RealtimeEvent::Delta {
            item_id: env.item_id,
            text: env.delta.unwrap_or_default(),
        },
        "conversation.item.input_audio_transcription.completed" => RealtimeEvent::Completed {
            item_id: env.item_id,
            transcript: env.transcript.unwrap_or_default(),
        },
        "conversation.item.input_audio_transcription.failed" => RealtimeEvent::Failed {
            message: env
                .error
                .and_then(|e| e.message.or(e.error_type))
                .unwrap_or_else(|| "transcription failed".to_string()),
        },
        "error" => RealtimeEvent::Error {
            message: env
                .error
                .and_then(|e| e.message.or(e.error_type))
                .unwrap_or_else(|| "realtime error".to_string()),
        },
        _ => RealtimeEvent::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn session_update_shape() {
        let msg = session_update("my-transcribe-deployment", Some("en-US"));
        assert_eq!(msg["type"], "session.update");
        assert_eq!(msg["session"]["type"], "transcription");
        let input = &msg["session"]["audio"]["input"];
        assert_eq!(input["format"]["type"], "audio/pcm");
        assert_eq!(input["format"]["rate"], 24000);
        assert_eq!(input["turn_detection"]["type"], "server_vad");
        assert_eq!(input["transcription"]["model"], "my-transcribe-deployment");
        assert_eq!(input["transcription"]["language"], "en");
    }

    #[test]
    fn auto_language_is_omitted() {
        let msg = session_update("m", Some("auto"));
        assert!(msg["session"]["audio"]["input"]["transcription"]
            .get("language")
            .is_none());
        let msg = session_update("m", None);
        assert!(msg["session"]["audio"]["input"]["transcription"]
            .get("language")
            .is_none());
    }

    #[test]
    fn append_audio_shape() {
        let msg = append_audio("AAECAw==");
        assert_eq!(msg["type"], "input_audio_buffer.append");
        assert_eq!(msg["audio"], "AAECAw==");
    }

    #[test]
    fn parses_delta_completed_failed() {
        let e = parse_event(
            r#"{"type":"conversation.item.input_audio_transcription.delta","item_id":"item_1","content_index":0,"delta":"Hel"}"#,
        );
        assert_eq!(
            e,
            RealtimeEvent::Delta {
                item_id: Some("item_1".into()),
                text: "Hel".into()
            }
        );

        let e = parse_event(
            r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"item_1","transcript":"Hello there."}"#,
        );
        assert_eq!(
            e,
            RealtimeEvent::Completed {
                item_id: Some("item_1".into()),
                transcript: "Hello there.".into()
            }
        );

        let e = parse_event(
            r#"{"type":"conversation.item.input_audio_transcription.failed","item_id":"i","error":{"type":"server_error","message":"boom"}}"#,
        );
        assert_eq!(
            e,
            RealtimeEvent::Failed {
                message: "boom".into()
            }
        );

        let e = parse_event(r#"{"type":"error","error":{"message":"bad session"}}"#);
        assert_eq!(
            e,
            RealtimeEvent::Error {
                message: "bad session".into()
            }
        );

        assert_eq!(
            parse_event(r#"{"type":"session.created"}"#),
            RealtimeEvent::Other
        );
        assert_eq!(parse_event("not json"), RealtimeEvent::Other);
    }
}
