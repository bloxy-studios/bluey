use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::documents::{DocumentKind, DocumentScope};
use super::mode::{BlueyMode, ResponseStyle};
use super::session::SessionEvent;
use super::transcript::TranscriptSegment;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct BoundingBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationContext {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WindowContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_id: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds: Option<BoundingBox>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hints: Option<serde_json::Map<String, serde_json::Value>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayInfo {
    pub id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub x: i32,
    pub y: i32,
    pub scale_factor: f64,
    pub is_main: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ImageMimeType {
    #[default]
    #[serde(rename = "image/jpeg")]
    Jpeg,
    #[serde(rename = "image/png")]
    Png,
    #[serde(rename = "image/webp")]
    Webp,
}

/// Mirrors `CaptureTarget` (tag = "type").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CaptureTarget {
    #[serde(rename_all = "camelCase")]
    Display {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        display_id: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Window {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        window_id: Option<u32>,
    },
    #[serde(rename_all = "camelCase")]
    Region {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        display_id: Option<String>,
        rect: BoundingBox,
    },
    ActiveWindow,
}

impl Default for CaptureTarget {
    fn default() -> Self {
        CaptureTarget::Display { display_id: None }
    }
}

/// Mirrors `ScreenFrame`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenFrame {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    pub mime_type: ImageMimeType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub width: u32,
    pub height: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_id: Option<String>,
    pub scale_factor: f64,
    pub captured_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    pub changed: bool,
    pub target: CaptureTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ImageFormat {
    #[default]
    Jpeg,
    Png,
}

/// Mirrors `CaptureOptions`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CaptureOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<CaptureTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<ImageFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_dimension: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inline: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_detection: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrBlock {
    pub text: String,
    pub confidence: f32,
    pub bounding_box: BoundingBox,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum OcrLevel {
    #[default]
    Fast,
    Accurate,
}

/// Mirrors `OCRContext`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrContext {
    pub blocks: Vec<OcrBlock>,
    pub text: String,
    pub level: OcrLevel,
    pub languages: Vec<String>,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct Size {
    pub width: f64,
    pub height: f64,
}

/// Mirrors `AccessibilityElement`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AccessibilityElement {
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<Point>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<Size>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actions: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focused: Option<bool>,
    pub depth: u32,
}

/// Mirrors `AccessibilityContext`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessibilityContext {
    pub application: ApplicationContext,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<WindowContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focused_element: Option<AccessibilityElement>,
    pub elements: Vec<AccessibilityElement>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_text: Option<String>,
    pub visible_text: String,
    pub truncated: bool,
    pub captured_at: String,
}

/// Mirrors `TranscriptContext`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptContext {
    pub segments: Vec<TranscriptSegment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub earlier_summary: Option<String>,
    pub window_seconds: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentResponseRef {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub content: String,
    pub created_at: String,
}

/// Mirrors `SessionContext`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionContext {
    pub session_id: String,
    pub mode_id: String,
    pub started_at: String,
    pub recent_responses: Vec<RecentResponseRef>,
    pub recent_events: Vec<SessionEvent>,
    pub notes: Vec<String>,
    pub document_ids: Vec<String>,
}

/// Mirrors `RetrievedChunk`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetrievedChunk {
    pub chunk_id: String,
    pub document_id: String,
    pub document_title: String,
    pub document_kind: DocumentKind,
    pub content: String,
    pub score: f32,
    pub scope: DocumentScope,
}

/// Mirrors `UserContext`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UserContext {
    pub chunks: Vec<RetrievedChunk>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub personal_instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

/// Mirrors `ModeContext`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModeContext {
    pub mode: BlueyMode,
    pub response_style: ResponseStyle,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ScreenSummary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    pub width: u32,
    pub height: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_id: Option<String>,
}

/// Mirrors `ContextSnapshot` — the primary input to the AI system.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ContextSnapshot {
    pub timestamp: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_application: Option<ApplicationContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_window: Option<WindowContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen: Option<ScreenSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ocr: Option<OcrContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accessibility: Option<AccessibilityContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript: Option<TranscriptContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_context: Option<UserContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<ModeContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_instruction: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timings: Option<BTreeMap<String, u64>>,
}

/// Mirrors `SnapshotOptions`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotOptions {
    pub include_screen: bool,
    pub include_ocr: bool,
    pub include_accessibility: bool,
    pub include_transcript: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript_window_seconds: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture: Option<CaptureOptions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ocr_level: Option<OcrLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inline_image: Option<bool>,
}

impl Default for SnapshotOptions {
    fn default() -> Self {
        Self {
            include_screen: true,
            include_ocr: true,
            include_accessibility: true,
            include_transcript: true,
            transcript_window_seconds: Some(120),
            capture: None,
            ocr_level: None,
            inline_image: Some(true),
        }
    }
}

/// Mirrors `ContextSource`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextSource {
    UserInstruction,
    Screen,
    Ocr,
    Accessibility,
    Transcript,
    TranscriptOld,
    Resume,
    JobDescription,
    Document,
    SessionMemory,
    PersonalInstructions,
}

/// Mirrors `ContextItem` — a scored unit of context competing for the token budget.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextItem {
    pub source: ContextSource,
    pub content: String,
    pub relevance: f32,
    pub tokens: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#ref: Option<String>,
}
