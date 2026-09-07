use serde::{Deserialize, Serialize};

use crate::error::BlueyError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioSource {
    Microphone,
    System,
}

/// Mirrors `TranscriptSegment`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptSegment {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker_confidence: Option<f32>,
    pub source: AudioSource,
    pub text: String,
    /// Milliseconds since audio session start.
    pub start_time: u64,
    pub end_time: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    pub finalized: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
    pub kind: AudioDeviceKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioDeviceKind {
    Input,
    Output,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptionProviderKind {
    #[default]
    Apple,
    CloudRealtime,
    Mock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum VadSensitivity {
    Low,
    #[default]
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RawAudioRetention {
    #[default]
    Never,
    UntilSessionEnd,
    Custom,
}

/// Mirrors `AudioSessionConfig`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioSessionConfig {
    pub microphone: MicrophoneConfig,
    pub system_audio: SystemAudioConfig,
    pub transcription: TranscriptionConfig,
    pub vad: VadConfig,
    pub retain_raw_audio: RawAudioRetention,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MicrophoneConfig {
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemAudioConfig {
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionConfig {
    pub provider: TranscriptionProviderKind,
    /// "auto" or a BCP-47 tag.
    pub language: String,
    pub speaker_identification: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VadConfig {
    pub enabled: bool,
    pub sensitivity: VadSensitivity,
}

impl Default for AudioSessionConfig {
    fn default() -> Self {
        Self {
            microphone: MicrophoneConfig {
                enabled: true,
                device_id: None,
            },
            system_audio: SystemAudioConfig { enabled: true },
            transcription: TranscriptionConfig {
                provider: TranscriptionProviderKind::Apple,
                language: "auto".into(),
                speaker_identification: true,
            },
            vad: VadConfig {
                enabled: true,
                sensitivity: VadSensitivity::Medium,
            },
            retain_raw_audio: RawAudioRetention::Never,
        }
    }
}

/// Partial config (`Partial<AudioSessionConfig>` on the TS side).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AudioSessionConfigPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub microphone: Option<MicrophoneConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_audio: Option<SystemAudioConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcription: Option<TranscriptionConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vad: Option<VadConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retain_raw_audio: Option<RawAudioRetention>,
}

impl AudioSessionConfig {
    pub fn apply(mut self, patch: AudioSessionConfigPatch) -> Self {
        if let Some(m) = patch.microphone {
            self.microphone = m;
        }
        if let Some(s) = patch.system_audio {
            self.system_audio = s;
        }
        if let Some(t) = patch.transcription {
            self.transcription = t;
        }
        if let Some(v) = patch.vad {
            self.vad = v;
        }
        if let Some(r) = patch.retain_raw_audio {
            self.retain_raw_audio = r;
        }
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AudioSessionState {
    #[default]
    Stopped,
    Starting,
    Running,
    Paused,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AudioLevels {
    pub microphone: f32,
    pub system: f32,
}

/// Mirrors `AudioStatus`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AudioStatus {
    pub state: AudioSessionState,
    pub microphone_active: bool,
    pub system_audio_active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<TranscriptionProviderKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_input_device: Option<AudioDevice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub levels: Option<AudioLevels>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<BlueyError>,
}

/// Mirrors `DetectedEventType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DetectedEventType {
    Question,
    BehavioralQuestion,
    TechnicalQuestion,
    CodingProblem,
    Objection,
    BuyingSignal,
    PricingConcern,
    CompetitorMention,
    Decision,
    ActionItem,
    TopicChange,
    ImportantStatement,
    FollowUp,
}

/// Mirrors `DetectedEvent`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedEvent {
    pub id: String,
    #[serde(rename = "type")]
    pub event_type: DetectedEventType,
    pub confidence: f32,
    pub requires_response: bool,
    pub text: String,
    pub segment_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
    pub detected_at: String,
}
