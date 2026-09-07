use serde::{Deserialize, Serialize};

use super::permissions::PermissionKind;
use super::transcript::AudioSource;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulatedSegment {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<AudioSource>,
}

/// Mirrors `DevSimulation` (tag = "type").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DevSimulation {
    Question {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        speaker: Option<String>,
    },
    CodingProblem {
        text: String,
    },
    Transcript {
        segments: Vec<SimulatedSegment>,
    },
    ScreenCapture {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fixture: Option<String>,
    },
    PermissionError {
        permission: PermissionKind,
    },
    AiLatency {
        ms: u64,
    },
    AiFailure {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code: Option<String>,
    },
    Clear,
}

/// Mirrors `LatencyMetrics`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LatencyMetrics {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ocr_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accessibility_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_assembly_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_to_first_token_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_response_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u32>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BuildProfile {
    Debug,
    Release,
}

/// Mirrors `DevInfo`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DevInfo {
    pub version: String,
    pub build_profile: BuildProfile,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub helper_version: Option<String>,
    pub helper_running: bool,
    pub agent_sidecar_available: bool,
    pub db_path: String,
    pub log_path: String,
    pub mock_transport: bool,
}
