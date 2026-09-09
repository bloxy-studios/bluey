use serde::{Deserialize, Serialize};

use super::ai::{AiProviderConfig, ModelRoleAssignments};
use super::context::OcrLevel;
use super::mode::{ResponseLength, ResponseTone, DEFAULT_MODE_ID};
use super::transcript::{RawAudioRetention, TranscriptionProviderKind, VadSensitivity};

pub const SETTINGS_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    System,
    Dark,
    Light,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PanelDensity {
    Compact,
    #[default]
    Comfortable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum FontSize {
    Small,
    #[default]
    Medium,
    Large,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PanelPositionPreference {
    #[default]
    Remember,
    Center,
    Top,
    Bottom,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DisplayMode {
    #[default]
    Standard,
    Privacy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ObservationMode {
    #[default]
    Manual,
    Smart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CaptureTargetPreference {
    #[default]
    Display,
    ActiveWindow,
    Region,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Error,
    Warn,
    #[default]
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
            Self::Trace => "trace",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ReducedMotion {
    #[default]
    System,
    On,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AudioSourcePreference {
    Microphone,
    System,
    #[default]
    Both,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneralSettings {
    pub bluey_name: String,
    pub launch_at_login: bool,
    pub default_mode_id: String,
    pub onboarding_completed: bool,
    pub developer_mode: bool,
    pub output_language: String,
}

impl Default for GeneralSettings {
    fn default() -> Self {
        Self {
            bluey_name: "Bluey".into(),
            launch_at_login: false,
            default_mode_id: DEFAULT_MODE_ID.into(),
            onboarding_completed: false,
            developer_mode: false,
            output_language: "en".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppearanceSettings {
    pub theme: Theme,
    pub opacity: f32,
    pub width: u32,
    pub blur: bool,
    pub font_size: FontSize,
    pub always_on_top: bool,
    pub density: PanelDensity,
    pub position: PanelPositionPreference,
    pub follow_active_display: bool,
    pub reduced_motion: ReducedMotion,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            opacity: 0.92,
            width: 690,
            blur: true,
            font_size: FontSize::Medium,
            always_on_top: true,
            density: PanelDensity::Comfortable,
            position: PanelPositionPreference::Remember,
            follow_active_display: true,
            reduced_motion: ReducedMotion::System,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioSettings {
    pub source: AudioSourcePreference,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub microphone_device_id: Option<String>,
    pub transcription_language: String,
    pub speaker_identification: bool,
    pub transcription_provider: TranscriptionProviderKind,
    pub vad_sensitivity: VadSensitivity,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            source: AudioSourcePreference::Both,
            microphone_device_id: None,
            transcription_language: "auto".into(),
            speaker_identification: true,
            // Gemini Live by default (ADR 0007); the audio manager falls back to
            // Apple Speech — with an `stt_fallback` notice — when no Google key is stored.
            transcription_provider: TranscriptionProviderKind::GeminiLive,
            vad_sensitivity: VadSensitivity::Medium,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenSettings {
    pub capture_target: CaptureTargetPreference,
    pub observation: ObservationMode,
    pub observation_interval_ms: u32,
    pub preferred_display: String,
    pub ocr_level: OcrLevel,
    pub ocr_languages: Vec<String>,
    pub max_image_dimension: u32,
}

impl Default for ScreenSettings {
    fn default() -> Self {
        Self {
            capture_target: CaptureTargetPreference::Display,
            observation: ObservationMode::Manual,
            observation_interval_ms: 1500,
            preferred_display: "active".into(),
            ocr_level: OcrLevel::Fast,
            ocr_languages: vec!["en-US".into()],
            max_image_dimension: 1600,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSettings {
    pub providers: Vec<AiProviderConfig>,
    pub models: ModelRoleAssignments,
    pub response_length: ResponseLength,
    pub response_tone: ResponseTone,
    pub research_enabled: bool,
    pub deep_research_enabled: bool,
    pub embeddings_enabled: bool,
    pub proactive_preparation: bool,
    pub context_token_budget: u32,
    /// Provider id the `.env` import nominated at boot (`BLUEY_AI_PROVIDER`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootstrap_provider: Option<String>,
    /// MRL-truncated embedding size for `gemini-embedding-2` (768 · 1536 · 3072).
    #[serde(default = "default_embedding_dimensions")]
    pub embedding_dimensions: u32,
    /// Which backend the research sidecar runs (`RESEARCH_BACKEND`).
    #[serde(default)]
    pub research_backend: ResearchBackend,
}

/// Default `ai.embeddingDimensions`: the recommended MRL size for `gemini-embedding-2`.
pub const DEFAULT_EMBEDDING_DIMENSIONS: u32 = 768;

fn default_embedding_dimensions() -> u32 {
    DEFAULT_EMBEDDING_DIMENSIONS
}

/// Mirrors `ResearchBackend` (`"gemini" | "claude"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ResearchBackend {
    #[default]
    Gemini,
    Claude,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            providers: Vec::new(),
            models: ModelRoleAssignments::default(),
            response_length: ResponseLength::Concise,
            response_tone: ResponseTone::Natural,
            research_enabled: true,
            deep_research_enabled: true,
            embeddings_enabled: false,
            proactive_preparation: true,
            context_token_budget: 12_000,
            bootstrap_provider: None,
            embedding_dimensions: DEFAULT_EMBEDDING_DIMENSIONS,
            research_backend: ResearchBackend::Gemini,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrivacySettings {
    pub display_mode: DisplayMode,
    pub store_session_history: bool,
    pub store_screenshots: bool,
    pub store_transcripts: bool,
    pub store_raw_audio: RawAudioRetention,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_audio_retention_minutes: Option<u32>,
    pub cloud_ai_enabled: bool,
    pub debug_log_transcripts: bool,
}

impl Default for PrivacySettings {
    fn default() -> Self {
        Self {
            display_mode: DisplayMode::Standard,
            store_session_history: true,
            store_screenshots: false,
            store_transcripts: true,
            store_raw_audio: RawAudioRetention::Never,
            raw_audio_retention_minutes: None,
            cloud_ai_enabled: true,
            debug_log_transcripts: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShortcutId {
    TogglePanel,
    CaptureAnalyze,
    GenerateResponse,
    ToggleListening,
    NewChat,
    OpenSettings,
    MoveUp,
    MoveDown,
    MoveLeft,
    MoveRight,
    ScrollUp,
    ScrollDown,
}

impl ShortcutId {
    pub const ALL: [ShortcutId; 12] = [
        ShortcutId::TogglePanel,
        ShortcutId::CaptureAnalyze,
        ShortcutId::GenerateResponse,
        ShortcutId::ToggleListening,
        ShortcutId::NewChat,
        ShortcutId::OpenSettings,
        ShortcutId::MoveUp,
        ShortcutId::MoveDown,
        ShortcutId::MoveLeft,
        ShortcutId::MoveRight,
        ShortcutId::ScrollUp,
        ShortcutId::ScrollDown,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShortcutGroup {
    General,
    Window,
    Scroll,
}

/// Mirrors `ShortcutBinding`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutBinding {
    pub id: ShortcutId,
    pub label: String,
    pub group: ShortcutGroup,
    pub accelerator: String,
    pub default_accelerator: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdvancedSettings {
    pub log_level: LogLevel,
    pub show_dev_overlay: bool,
    pub helper_restart_on_crash: bool,
}

impl Default for AdvancedSettings {
    fn default() -> Self {
        Self {
            log_level: LogLevel::Info,
            show_dev_overlay: false,
            helper_restart_on_crash: true,
        }
    }
}

/// Mirrors `Settings`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub version: u32,
    pub general: GeneralSettings,
    pub appearance: AppearanceSettings,
    pub audio: AudioSettings,
    pub screen: ScreenSettings,
    pub ai: AiSettings,
    pub privacy: PrivacySettings,
    pub shortcuts: Vec<ShortcutBinding>,
    pub advanced: AdvancedSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            general: GeneralSettings::default(),
            appearance: AppearanceSettings::default(),
            audio: AudioSettings::default(),
            screen: ScreenSettings::default(),
            ai: AiSettings::default(),
            privacy: PrivacySettings::default(),
            shortcuts: crate::shortcuts::default_bindings(),
            advanced: AdvancedSettings::default(),
        }
    }
}

impl Settings {
    /// Apply a deep-partial JSON patch (`SettingsPatch` on the TS side).
    /// Objects are merged recursively; arrays and scalars are replaced.
    pub fn apply_patch(&self, patch: &serde_json::Value) -> Result<Settings, serde_json::Error> {
        let mut current = serde_json::to_value(self)?;
        merge_json(&mut current, patch);
        serde_json::from_value(current)
    }
}

/// Recursive JSON merge: objects merge key-by-key, everything else is replaced.
pub fn merge_json(base: &mut serde_json::Value, patch: &serde_json::Value) {
    match (base, patch) {
        (serde_json::Value::Object(base_map), serde_json::Value::Object(patch_map)) => {
            for (k, v) in patch_map {
                match base_map.get_mut(k) {
                    Some(existing) if existing.is_object() && v.is_object() => {
                        merge_json(existing, v)
                    }
                    _ => {
                        base_map.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (base, patch) => *base = patch.clone(),
    }
}

/// Mirrors `PanelState`: logical native-frame dimensions, including the
/// transparent shadow insets (not the appearance surface width).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PanelState {
    pub visible: bool,
    pub pinned: bool,
    pub expanded: bool,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_id: Option<String>,
    pub opacity: f32,
}

impl Default for PanelState {
    fn default() -> Self {
        Self {
            visible: true,
            pinned: false,
            expanded: false,
            x: 0.0,
            y: 0.0,
            // Keep launch defaults in sync with bluey-protocols::panel and
            // tauri.conf.json (the protocols crate tests this contract).
            width: 754.0,
            height: 175.0,
            display_id: None,
            opacity: 0.92,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShortcutConflictKind {
    System,
    Bluey,
    RegistrationFailed,
}

/// Mirrors `ShortcutConflict`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutConflict {
    pub accelerator: String,
    pub conflicts_with: ShortcutConflictKind,
    pub detail: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_serialize_camel_case() {
        let s = Settings::default();
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["general"]["blueyName"], "Bluey");
        assert_eq!(v["privacy"]["storeRawAudio"], "never");
        assert_eq!(v["screen"]["captureTarget"], "display");
        assert!(v["shortcuts"].as_array().unwrap().len() == 12);
    }

    #[test]
    fn deep_patch_merges_objects_and_replaces_arrays() {
        let s = Settings::default();
        let patched = s
            .apply_patch(&serde_json::json!({
                "general": { "blueyName": "Blue" },
                "privacy": { "displayMode": "privacy" },
                "screen": { "ocrLanguages": ["de-DE"] }
            }))
            .unwrap();
        assert_eq!(patched.general.bluey_name, "Blue");
        assert_eq!(patched.general.default_mode_id, DEFAULT_MODE_ID);
        assert_eq!(patched.privacy.display_mode, DisplayMode::Privacy);
        assert_eq!(patched.screen.ocr_languages, vec!["de-DE".to_string()]);
        assert_eq!(patched.screen.max_image_dimension, 1600);
    }
}
