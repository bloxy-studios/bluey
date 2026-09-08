//! Microsoft Foundry Voice Live — live MAI-Transcribe over WebSocket.
//!
//! `gpt-4o-mini-transcribe` and friends are Azure OpenAI realtime models and
//! are often not deployed. MAI-Transcribe is the Foundry speech model. Live
//! STT goes through Voice Live, not `/openai/v1/realtime`.
//!
//! Wire:
//! 1. Connect [`crate::azure::voice_live_url`] with a Voice Live *companion*
//!    chat model (query `model=`). The companion is fully managed and does
//!    not need a Foundry deployment. It is **not** the STT model.
//! 2. Send [`session_update_transcription_only`] with
//!    `input_audio_transcription.model = mai-transcribe` and
//!    `turn_detection.create_response = false` so the companion never replies.
//! 3. Stream [`crate::realtime::append_audio`] (PCM16 base64).
//! 4. Parse [`crate::realtime::parse_event`] (same event names as OpenAI
//!    Realtime).
//!
//! Auth is the Foundry resource key in the `api-key` header (or Entra Bearer).
//! The host is `{resource}.services.ai.azure.com`, not `.openai.azure.com`.

use serde_json::{json, Value};

use crate::realtime;

/// Voice Live alias. The service maps this to MAI-Transcribe-1.5.
pub const MAI_TRANSCRIBE: &str = "mai-transcribe";

/// Foundry catalog / Fast Transcription REST id for 1.5 (`enhancedMode.model`).
pub const MAI_TRANSCRIBE_1_5: &str = "MAI-Transcribe-1.5";

/// Foundry catalog / Fast Transcription REST id for 2 (`enhancedMode.model`).
pub const MAI_TRANSCRIBE_2: &str = "MAI-Transcribe-2";

/// Voice Live kebab id for generation 2. Do not send the `azureml://…` catalog
/// URI or the `2026-09-03` version on this socket.
pub const MAI_TRANSCRIBE_2_VOICE_LIVE: &str = "mai-transcribe-2";

/// Default companion chat model on the Voice Live `model=` query param.
///
/// Voice Live bills the session by companion tier even with
/// `create_response: false`. `gpt-4.1-mini` is Basic, fully managed, and
/// listed. `gpt-5-nano` is Lite (cheaper). `gpt-5.6-luna` and `gpt-6-astra`
/// are **not** Voice Live companions — do not use them here.
pub const DEFAULT_COMPANION_MODEL: &str = "gpt-4.1-mini";

/// PCM16 sample rate that matches helper capture (16 kHz mono). Voice Live
/// also accepts 24 kHz; 16 kHz avoids a resample on the cloud path.
pub const SAMPLE_RATE: u32 = 16_000;

/// Which cloud STT WebSocket a configured transcription model should open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptionTransport {
    /// Azure/OpenAI `/openai/v1/realtime?intent=transcription`.
    OpenaiRealtime,
    /// Foundry Voice Live `/voice-live/realtime`.
    VoiceLive,
}

/// True when `model` is an MAI-Transcribe alias (`mai-transcribe`,
/// `MAI-Transcribe-1.5`, `MAI-Transcribe-2`, Foundry `azureml://…` URI, …).
pub fn is_mai_transcribe(model: &str) -> bool {
    let n = normalize(catalog_model_name(model));
    n.contains("mai-transcribe") || n.contains("mai transcribe")
}

/// Route a transcription-role model id onto the matching WebSocket surface.
pub fn transport_for_model(model: &str) -> TranscriptionTransport {
    if is_mai_transcribe(model) {
        TranscriptionTransport::VoiceLive
    } else {
        TranscriptionTransport::OpenaiRealtime
    }
}

/// Map user/env/catalog aliases onto the id Voice Live expects in `session.update`.
///
/// * 2-family (including `azureml://…/models/MAI-Transcribe-2/versions/2026-09-03`)
///   → [`MAI_TRANSCRIBE_2_VOICE_LIVE`].
/// * 1.5-family / bare `mai-transcribe` → [`MAI_TRANSCRIBE`] (service default = 1.5).
///
/// Fast Transcription REST uses [`canonical_fast_transcription_model`] instead
/// (`MAI-Transcribe-1.5` / `MAI-Transcribe-2` in `enhancedMode.model`).
/// Never send the catalog URI.
pub fn canonical_stt_model(model: &str) -> String {
    let name = catalog_model_name(model);
    let n = normalize(name);
    if n.contains("mai-transcribe-2") || n.contains("mai transcribe 2") {
        MAI_TRANSCRIBE_2_VOICE_LIVE.to_string()
    } else if is_mai_transcribe(model) {
        MAI_TRANSCRIBE.to_string()
    } else {
        name.trim().to_string()
    }
}

/// Fast Transcription REST `enhancedMode.model` (`MAI-Transcribe-2` / `MAI-Transcribe-1.5`).
pub fn canonical_fast_transcription_model(model: &str) -> String {
    let name = catalog_model_name(model);
    let n = normalize(name);
    if n.contains("mai-transcribe-2") || n.contains("mai transcribe 2") {
        MAI_TRANSCRIBE_2.to_string()
    } else if n.contains("mai-transcribe-1.5") || n.contains("mai transcribe 1.5") {
        MAI_TRANSCRIBE_1_5.to_string()
    } else if is_mai_transcribe(model) {
        MAI_TRANSCRIBE_1_5.to_string()
    } else {
        name.trim().to_string()
    }
}

/// Strip a Foundry catalog URI down to the model name.
/// `azureml://registries/azureml-cogsvc/models/MAI-Transcribe-2/versions/2026-09-03`
/// → `MAI-Transcribe-2`.
fn catalog_model_name(model: &str) -> &str {
    let trimmed = model.trim();
    let Some(rest) = trimmed
        .strip_prefix("azureml://")
        .or_else(|| trimmed.strip_prefix("AzureML://"))
    else {
        return trimmed;
    };
    let Some(idx) = rest.find("/models/") else {
        return trimmed;
    };
    let after = &rest[idx + "/models/".len()..];
    after
        .split(['/', '?', '#'])
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(trimmed)
}

fn normalize(model: &str) -> String {
    model.trim().to_ascii_lowercase().replace('_', "-")
}

/// Transcription-only Voice Live `session.update`.
///
/// `modalities: ["text"]` plus `create_response: false` keeps the companion
/// LLM silent. `language: None` (or `"auto"`) omits the language field.
pub fn session_update_transcription_only(stt_model: &str, language: Option<&str>) -> Value {
    let model = canonical_stt_model(stt_model);
    let mut transcription = json!({ "model": model });
    if let Some(lang) = language {
        if !lang.is_empty() && lang != "auto" {
            transcription["language"] = json!(realtime::primary_language_tag(lang));
        }
    }
    json!({
        "type": "session.update",
        "session": {
            "modalities": ["text"],
            "input_audio_format": "pcm16",
            "input_audio_sampling_rate": SAMPLE_RATE,
            "input_audio_transcription": transcription,
            "turn_detection": {
                "type": "azure_semantic_vad_multilingual",
                "create_response": false,
                "interrupt_response": false,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn mai_aliases_route_to_voice_live() {
        for id in [
            "mai-transcribe",
            "MAI-Transcribe-1.5",
            "mai-transcribe-1.5",
            "MAI-Transcribe-2",
            "azureml://registries/azureml-cogsvc/models/MAI-Transcribe-2/versions/2026-09-03",
            "azureml://registries/azureml-cogsvc/models/MAI-Transcribe-1.5/versions/2026-06-02",
            "mai_transcribe",
        ] {
            assert!(is_mai_transcribe(id), "{id}");
            assert_eq!(transport_for_model(id), TranscriptionTransport::VoiceLive);
        }
        assert_eq!(
            transport_for_model("gpt-4o-mini-transcribe"),
            TranscriptionTransport::OpenaiRealtime
        );
        assert!(!is_mai_transcribe("gpt-4o-mini-transcribe"));
    }

    #[test]
    fn canonical_stt_collapses_1_5_family() {
        assert_eq!(canonical_stt_model("MAI-Transcribe-1.5"), MAI_TRANSCRIBE);
        assert_eq!(canonical_stt_model("mai-transcribe-1.5"), MAI_TRANSCRIBE);
        assert_eq!(canonical_stt_model("mai-transcribe"), MAI_TRANSCRIBE);
        assert_eq!(
            canonical_stt_model(
                "azureml://registries/azureml-cogsvc/models/MAI-Transcribe-1.5/versions/2026-06-02"
            ),
            MAI_TRANSCRIBE
        );
        assert_eq!(
            canonical_fast_transcription_model(
                "azureml://registries/azureml-cogsvc/models/MAI-Transcribe-1.5/versions/2026-06-02"
            ),
            MAI_TRANSCRIBE_1_5
        );
        assert_eq!(
            canonical_stt_model("MAI-Transcribe-2"),
            MAI_TRANSCRIBE_2_VOICE_LIVE
        );
        assert_eq!(
            canonical_stt_model("mai-transcribe-2"),
            MAI_TRANSCRIBE_2_VOICE_LIVE
        );
        assert_eq!(
            canonical_stt_model(
                "azureml://registries/azureml-cogsvc/models/MAI-Transcribe-2/versions/2026-09-03"
            ),
            MAI_TRANSCRIBE_2_VOICE_LIVE
        );
        assert_eq!(
            canonical_fast_transcription_model(
                "azureml://registries/azureml-cogsvc/models/MAI-Transcribe-2/versions/2026-09-03"
            ),
            MAI_TRANSCRIBE_2
        );
        assert_eq!(
            canonical_stt_model("gpt-4o-mini-transcribe"),
            "gpt-4o-mini-transcribe"
        );
    }

    #[test]
    fn transcription_only_session_shape() {
        let msg = session_update_transcription_only("MAI-Transcribe-1.5", Some("en-US"));
        assert_eq!(msg["type"], "session.update");
        assert!(msg["session"].get("type").is_none());
        assert_eq!(msg["session"]["modalities"], json!(["text"]));
        assert_eq!(msg["session"]["input_audio_format"], "pcm16");
        assert_eq!(msg["session"]["input_audio_sampling_rate"], 16_000);
        assert_eq!(
            msg["session"]["input_audio_transcription"]["model"],
            MAI_TRANSCRIBE
        );
        assert_eq!(
            msg["session"]["input_audio_transcription"]["language"],
            "en"
        );
        assert_eq!(
            msg["session"]["turn_detection"]["type"],
            "azure_semantic_vad_multilingual"
        );
        assert_eq!(msg["session"]["turn_detection"]["create_response"], false);
        assert_eq!(
            msg["session"]["turn_detection"]["interrupt_response"],
            false
        );

        let catalog = session_update_transcription_only(
            "azureml://registries/azureml-cogsvc/models/MAI-Transcribe-1.5/versions/2026-06-02",
            None,
        );
        assert_eq!(
            catalog["session"]["input_audio_transcription"]["model"],
            MAI_TRANSCRIBE
        );
        let v2 = session_update_transcription_only("MAI-Transcribe-2", None);
        assert_eq!(
            v2["session"]["input_audio_transcription"]["model"],
            MAI_TRANSCRIBE_2_VOICE_LIVE
        );
    }

    #[test]
    fn auto_language_is_omitted() {
        let msg = session_update_transcription_only("mai-transcribe", Some("auto"));
        assert!(msg["session"]["input_audio_transcription"]
            .get("language")
            .is_none());
        let msg = session_update_transcription_only("mai-transcribe", None);
        assert!(msg["session"]["input_audio_transcription"]
            .get("language")
            .is_none());
    }
}
