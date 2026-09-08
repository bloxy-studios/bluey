//! Google Gemini Developer API (Google AI Studio, `generativelanguage.googleapis.com`)
//! wire codec: `generateContent` / `streamGenerateContent` bodies and chunks,
//! error bodies, embeddings (`gemini-embedding-2`), the models list and the
//! Live API WebSocket frames used for real-time transcription.
//!
//! Pure functions only — the app crate does the HTTP/WebSocket I/O. Facts
//! encoded here were verified against ai.google.dev on 2026-09-08 (see
//! `docs/reference/gemini-api-sept-2026.md`):
//!
//! * REST base `…/v1beta`, auth header `x-goog-api-key`; the Live WebSocket is
//!   the only place the key travels in the query string.
//! * Gemini 3.x: `thinkingLevel` (`low|medium|high`; `minimal` only on 3.5/3.6
//!   and the Flash-Lite line — it errors on 3.7/3.8), no `temperature`/`topP`/
//!   `topK`/`candidateCount`/`thinkingBudget`.
//! * Roles are exactly `user` and `model`; every system message folds into
//!   `systemInstruction`.
//! * `gemini-embedding-2` has no `taskType`: use prompt prefixes and one
//!   `requests[]` entry per chunk.

use std::time::Duration;

use bluey_core::error::{BlueyError, BlueyErrorKind, RecoveryAction};
use bluey_core::types::{
    AiContentPart, AiMessage, AiRole, AiTask, FinishReason, JsonSchemaSpec, LatencyBudget,
    ModelRole, ReasoningLevel,
};
use serde::Deserialize;
use serde_json::{json, Map, Value};

/// Default REST base URL (overridable per provider for proxies).
pub const API_BASE: &str = "https://generativelanguage.googleapis.com/v1beta";

/// Live API WebSocket endpoint (the API key is appended as `?key=`).
pub const LIVE_WS_URL_PREFIX: &str = "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent";

/// Page size used when listing models.
pub const MODELS_PAGE_SIZE: u32 = 200;

/// Max `requests[]` entries per `batchEmbedContents` call.
pub const EMBED_BATCH_SIZE: usize = 100;

/// PCM sample rate the Live transcription models expect.
pub const LIVE_SAMPLE_RATE: u32 = 16_000;

// ── URLs ─────────────────────────────────────────────────────────────────────

fn base(base_url: &str) -> &str {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        API_BASE
    } else {
        trimmed
    }
}

/// `…/models/{model}:generateContent` or `…:streamGenerateContent?alt=sse`.
pub fn generate_url(base_url: &str, model: &str, stream: bool) -> String {
    if stream {
        format!(
            "{}/models/{model}:streamGenerateContent?alt=sse",
            base(base_url)
        )
    } else {
        format!("{}/models/{model}:generateContent", base(base_url))
    }
}

/// `…/models/{model}:embedContent`.
pub fn embed_url(base_url: &str, model: &str) -> String {
    format!("{}/models/{model}:embedContent", base(base_url))
}

/// `…/models/{model}:batchEmbedContents`.
pub fn batch_embed_url(base_url: &str, model: &str) -> String {
    format!("{}/models/{model}:batchEmbedContents", base(base_url))
}

/// `…/models?pageSize=200[&pageToken=…]`.
pub fn models_url(base_url: &str, page_token: Option<&str>) -> String {
    match page_token {
        Some(token) if !token.is_empty() => format!(
            "{}/models?pageSize={MODELS_PAGE_SIZE}&pageToken={token}",
            base(base_url)
        ),
        _ => format!("{}/models?pageSize={MODELS_PAGE_SIZE}", base(base_url)),
    }
}

/// Live WebSocket URL. **Contains the API key** — never log it raw, use
/// [`redact_live_url`] for diagnostics.
pub fn live_url(api_key: &str) -> String {
    format!("{LIVE_WS_URL_PREFIX}?key={api_key}")
}

/// Replace the `key=` value of a Live URL with `[redacted]`.
pub fn redact_live_url(url: &str) -> String {
    match url.find("key=") {
        Some(idx) => {
            let end = url[idx..].find('&').map(|e| idx + e).unwrap_or(url.len());
            format!("{}key=[redacted]{}", &url[..idx], &url[end..])
        }
        None => url.to_string(),
    }
}

// ── Model families & thinking policy ─────────────────────────────────────────

/// Gemini 3.x model ids (`gemini-3-…`, `gemini-3.5-…`, `gemini-3.8-…`).
pub fn is_gemini_3(model: &str) -> bool {
    let m = model.trim().to_ascii_lowercase();
    let m = m.strip_prefix("models/").unwrap_or(&m);
    m.starts_with("gemini-3")
}

/// Whether `thinkingLevel: minimal` is accepted: the 3.5/3.6 Flash and the
/// Flash-Lite line yes; 3.7/3.8 Flash and the 3.1 Pro preview return 400.
pub fn supports_minimal(model: &str) -> bool {
    let m = model.trim().to_ascii_lowercase();
    let m = m.strip_prefix("models/").unwrap_or(&m);
    if !m.starts_with("gemini-3") || m.contains("pro") {
        return false;
    }
    !(m.starts_with("gemini-3.7") || m.starts_with("gemini-3.8"))
}

/// `generationConfig.thinkingConfig.thinkingLevel`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinkingLevel {
    Minimal,
    Low,
    Medium,
    High,
}

impl ThinkingLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

/// Bluey's thinking policy for Gemini 3.x (`None` for older models, which use
/// `thinkingBudget` semantics the adapter leaves at the server default):
///
/// | request | level |
/// |---|---|
/// | `reasoning=deep`, `task=deep_reasoning`, `system_design` with reasoning ≥ light, `latency=deep` | `high` |
/// | `task=classification` or `latency=ultra-fast` | `minimal` when the model accepts it, else `low` |
/// | `task=answer`/`vision` with `latency=fast` | `low` |
/// | everything else (balanced, coding, summarization, research) | `medium` (the default; omitted from the body) |
pub fn thinking_level_for(
    task: AiTask,
    latency: LatencyBudget,
    reasoning: ReasoningLevel,
    model: &str,
) -> Option<ThinkingLevel> {
    if !is_gemini_3(model) {
        return None;
    }
    let deep = reasoning == ReasoningLevel::Deep
        || task == AiTask::DeepReasoning
        || (task == AiTask::SystemDesign && reasoning != ReasoningLevel::None)
        || latency == LatencyBudget::Deep;
    if deep {
        return Some(ThinkingLevel::High);
    }
    if task == AiTask::Classification || latency == LatencyBudget::UltraFast {
        return Some(if supports_minimal(model) {
            ThinkingLevel::Minimal
        } else {
            ThinkingLevel::Low
        });
    }
    if matches!(task, AiTask::Answer | AiTask::Vision) && latency == LatencyBudget::Fast {
        return Some(ThinkingLevel::Low);
    }
    Some(ThinkingLevel::Medium)
}

// ── generateContent request ──────────────────────────────────────────────────

/// Everything needed to build a `generateContent` body.
#[derive(Debug, Clone)]
pub struct GenerateBodyOptions<'a> {
    pub model: &'a str,
    pub messages: &'a [AiMessage],
    pub max_output_tokens: Option<u32>,
    /// Forwarded only for pre-3 models (sampling params are deprecated on 3.x).
    pub temperature: Option<f32>,
    pub output_schema: Option<&'a JsonSchemaSpec>,
    pub thinking_level: Option<ThinkingLevel>,
}

/// Build the request body (see module docs for the rules). `generationConfig`
/// is omitted entirely when it would be empty.
pub fn build_generate_body(opts: &GenerateBodyOptions<'_>) -> Value {
    let mut system_texts: Vec<String> = Vec::new();
    let mut contents: Vec<Value> = Vec::new();
    for message in opts.messages {
        match message.role {
            AiRole::System => {
                let text = message.text_content();
                if !text.trim().is_empty() {
                    system_texts.push(text);
                }
            }
            AiRole::User | AiRole::Assistant => {
                let role = if message.role == AiRole::User {
                    "user"
                } else {
                    "model"
                };
                let parts: Vec<Value> = message.content.iter().map(part_to_json).collect();
                contents.push(json!({ "role": role, "parts": parts }));
            }
        }
    }

    let mut body = Map::new();
    if !system_texts.is_empty() {
        body.insert(
            "systemInstruction".into(),
            json!({ "parts": [ { "text": system_texts.join("\n\n") } ] }),
        );
    }
    body.insert("contents".into(), Value::Array(contents));

    let mut config = Map::new();
    if let Some(max) = opts.max_output_tokens {
        config.insert("maxOutputTokens".into(), json!(max));
    }
    if let Some(level) = opts.thinking_level {
        if level != ThinkingLevel::Medium {
            config.insert(
                "thinkingConfig".into(),
                json!({ "thinkingLevel": level.as_str() }),
            );
        }
    }
    if let Some(spec) = opts.output_schema {
        config.insert("responseMimeType".into(), json!("application/json"));
        config.insert("responseJsonSchema".into(), strip_schema_meta(&spec.schema));
    }
    if let Some(temperature) = opts.temperature {
        if !is_gemini_3(opts.model) {
            config.insert("temperature".into(), json!(temperature));
        }
    }
    if !config.is_empty() {
        body.insert("generationConfig".into(), Value::Object(config));
    }
    Value::Object(body)
}

fn part_to_json(part: &AiContentPart) -> Value {
    match part {
        AiContentPart::Text { text } => json!({ "text": text }),
        AiContentPart::Image { media_type, data } => json!({
            "inlineData": { "mimeType": media_type.as_str(), "data": data }
        }),
    }
}

/// Remove the top-level `$schema` key (zod v4's `toJSONSchema` adds it and the
/// API rejects unknown keywords); everything else is forwarded untouched.
pub fn strip_schema_meta(schema: &Value) -> Value {
    match schema {
        Value::Object(map) => {
            let mut cleaned = map.clone();
            cleaned.remove("$schema");
            Value::Object(cleaned)
        }
        other => other.clone(),
    }
}

// ── generateContent response / SSE chunk ─────────────────────────────────────

/// Token usage from `usageMetadata`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    pub prompt: Option<u32>,
    pub candidates: Option<u32>,
    pub thoughts: Option<u32>,
}

impl Usage {
    /// Billable output tokens (answer + thinking).
    pub fn output_tokens(&self) -> Option<u32> {
        match (self.candidates, self.thoughts) {
            (None, None) => None,
            (c, t) => Some(c.unwrap_or(0) + t.unwrap_or(0)),
        }
    }
}

/// A `functionCall` part.
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionCall {
    pub id: Option<String>,
    pub name: String,
    pub args: Value,
}

/// One parsed `GenerateContentResponse` (unary or a single SSE chunk).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GeminiResponse {
    /// Visible text of the first candidate (thought summaries excluded).
    pub text: String,
    /// `candidates[0].finishReason` when present (e.g. `STOP`, `MAX_TOKENS`).
    pub finish: Option<String>,
    pub usage: Option<Usage>,
    /// `promptFeedback.blockReason` — the prompt itself was refused.
    pub block_reason: Option<String>,
    pub function_calls: Vec<FunctionCall>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireResponse {
    #[serde(default)]
    candidates: Vec<WireCandidate>,
    #[serde(default)]
    usage_metadata: Option<WireUsage>,
    #[serde(default)]
    prompt_feedback: Option<WirePromptFeedback>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireCandidate {
    #[serde(default)]
    content: Option<WireContent>,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct WireContent {
    #[serde(default)]
    parts: Vec<WirePart>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WirePart {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    thought: Option<bool>,
    #[serde(default)]
    function_call: Option<WireFunctionCall>,
    /// `gemini-3.5-transcribe` with diarization / word timestamps.
    #[serde(default)]
    audio_transcription: Option<WireAudioTranscription>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireAudioTranscription {
    #[serde(default)]
    speaker_label: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    words: Vec<WireWord>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireWord {
    #[serde(default)]
    word: String,
    #[serde(default)]
    start_offset: Option<String>,
    #[serde(default)]
    end_offset: Option<String>,
}

#[derive(Deserialize)]
struct WireFunctionCall {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: String,
    #[serde(default)]
    args: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireUsage {
    #[serde(default)]
    prompt_token_count: Option<u32>,
    #[serde(default)]
    candidates_token_count: Option<u32>,
    #[serde(default)]
    thoughts_token_count: Option<u32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WirePromptFeedback {
    #[serde(default)]
    block_reason: Option<String>,
}

/// Parse one response / SSE `data:` payload.
pub fn parse_response(data: &str) -> Result<GeminiResponse, serde_json::Error> {
    let wire: WireResponse = serde_json::from_str(data)?;
    let mut out = GeminiResponse {
        usage: wire.usage_metadata.map(|u| Usage {
            prompt: u.prompt_token_count,
            candidates: u.candidates_token_count,
            thoughts: u.thoughts_token_count,
        }),
        block_reason: wire.prompt_feedback.and_then(|f| f.block_reason),
        ..GeminiResponse::default()
    };
    if let Some(candidate) = wire.candidates.into_iter().next() {
        out.finish = candidate.finish_reason;
        if let Some(content) = candidate.content {
            for part in content.parts {
                if let Some(call) = part.function_call {
                    out.function_calls.push(FunctionCall {
                        id: call.id,
                        name: call.name,
                        args: call.args,
                    });
                }
                if part.thought.unwrap_or(false) {
                    continue;
                }
                if let Some(text) = part.text {
                    out.text.push_str(&text);
                }
            }
        }
    }
    Ok(out)
}

/// `STOP` → `Stop`, `MAX_TOKENS` → `Length`, everything else (`SAFETY`,
/// `RECITATION`, `PROHIBITED_CONTENT`, `BLOCKLIST`, `SPII`,
/// `MALFORMED_FUNCTION_CALL`, `OTHER`, …) → `Error`.
pub fn map_finish_reason(reason: &str) -> FinishReason {
    match reason.trim().to_ascii_uppercase().as_str() {
        "STOP" | "FINISH_REASON_UNSPECIFIED" | "" => FinishReason::Stop,
        "MAX_TOKENS" => FinishReason::Length,
        _ => FinishReason::Error,
    }
}

/// Whether a finish reason denotes a refusal/failure (not a normal end).
pub fn is_error_finish(reason: &str) -> bool {
    map_finish_reason(reason) == FinishReason::Error
}

/// Error code suffix for a block/finish reason: `SAFETY` → `blocked_safety`.
pub fn blocked_code(reason: &str) -> String {
    let cleaned: String = reason
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches('_');
    if cleaned.is_empty() {
        "blocked_other".to_string()
    } else {
        format!("blocked_{cleaned}")
    }
}

/// `BlueyError` for a refused answer (`promptFeedback.blockReason` or an
/// error-class finish reason).
pub fn blocked_error(reason: &str) -> BlueyError {
    BlueyError::ai(
        &blocked_code(reason),
        format!(
            "the model refused to answer ({})",
            reason.trim().to_ascii_lowercase()
        ),
    )
}

// ── Error bodies ─────────────────────────────────────────────────────────────

/// Parsed `{ "error": { code, status, details[] } }` body. The `message` is
/// deliberately not kept — it can echo prompt text.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GeminiError {
    pub http_code: Option<u16>,
    /// gRPC status name, e.g. `RESOURCE_EXHAUSTED`, `INVALID_ARGUMENT`.
    pub status: Option<String>,
    /// `google.rpc.ErrorInfo.reason`, e.g. `API_KEY_INVALID`.
    pub reason: Option<String>,
    /// `google.rpc.RetryInfo.retryDelay`.
    pub retry_after: Option<Duration>,
    /// `google.rpc.QuotaFailure.violations[0].quotaId`.
    pub quota_id: Option<String>,
}

impl GeminiError {
    /// A daily quota (resets at midnight Pacific) rather than a per-minute one.
    pub fn is_daily_quota(&self) -> bool {
        self.quota_id
            .as_deref()
            .map(|q| q.contains("PerDay"))
            .unwrap_or(false)
    }
}

/// Parse an error body; `None` when it is not a Google error envelope.
pub fn parse_error_body(body: &str) -> Option<GeminiError> {
    let value: Value = serde_json::from_str(body).ok()?;
    let error = value.get("error")?.as_object()?;
    let mut out = GeminiError {
        http_code: error
            .get("code")
            .and_then(Value::as_u64)
            .and_then(|c| u16::try_from(c).ok()),
        status: error
            .get("status")
            .and_then(Value::as_str)
            .map(String::from),
        ..GeminiError::default()
    };
    if let Some(details) = error.get("details").and_then(Value::as_array) {
        for detail in details {
            let kind = detail.get("@type").and_then(Value::as_str).unwrap_or("");
            if kind.ends_with("google.rpc.ErrorInfo") {
                out.reason = detail
                    .get("reason")
                    .and_then(Value::as_str)
                    .map(String::from);
            } else if kind.ends_with("google.rpc.RetryInfo") {
                out.retry_after = detail
                    .get("retryDelay")
                    .and_then(Value::as_str)
                    .and_then(parse_proto_duration);
            } else if kind.ends_with("google.rpc.QuotaFailure") {
                out.quota_id = detail
                    .get("violations")
                    .and_then(Value::as_array)
                    .and_then(|v| v.first())
                    .and_then(|v| v.get("quotaId"))
                    .and_then(Value::as_str)
                    .map(String::from);
            }
        }
    }
    Some(out)
}

/// Parse a protobuf JSON duration (`"23s"`, `"0.500s"`, `"1.5s"`).
pub fn parse_proto_duration(text: &str) -> Option<Duration> {
    let seconds = text.trim().strip_suffix('s')?.parse::<f64>().ok()?;
    if seconds.is_sign_negative() || !seconds.is_finite() {
        return None;
    }
    Some(Duration::from_secs_f64(seconds))
}

/// Map an HTTP failure onto the contract error (see the table in
/// `docs/AI_ARCHITECTURE.md`). Never includes response bodies.
pub fn map_gemini_error(http_status: u16, error: Option<&GeminiError>) -> BlueyError {
    let reason = error.and_then(|e| e.reason.as_deref()).unwrap_or("");
    match http_status {
        400 if reason == "API_KEY_INVALID" => BlueyError::new(
            BlueyErrorKind::Configuration,
            "config.api_key_invalid",
            "Google AI Studio rejected the API key — create a new key at aistudio.google.com/apikey",
        )
        .recoverable(RecoveryAction::ConfigureProvider),
        400 => BlueyError::ai(
            "invalid_request",
            "the Gemini API rejected the request (HTTP 400) — check the model id and request options",
        ),
        401 | 403 => BlueyError::new(
            BlueyErrorKind::Configuration,
            format!("config.http_{http_status}"),
            format!("the Gemini API rejected the credentials (HTTP {http_status})"),
        )
        .recoverable(RecoveryAction::ConfigureProvider),
        404 => BlueyError::new(
            BlueyErrorKind::Configuration,
            "config.model_not_found",
            "this model is not available for the key — pick another model in Settings → AI",
        )
        .recoverable(RecoveryAction::ConfigureProvider),
        429 => {
            let daily = error.map(GeminiError::is_daily_quota).unwrap_or(false);
            let retry_ms = error
                .and_then(|e| e.retry_after)
                .map(|d| d.as_millis() as u64);
            let message = if daily {
                "daily free-tier quota reached; wait until midnight Pacific or enable billing in AI Studio"
            } else {
                "the Gemini API rate-limited the request (HTTP 429)"
            };
            let mut details = Map::new();
            if let Some(ms) = retry_ms {
                details.insert("retryAfterMs".into(), json!(ms));
            }
            if let Some(quota) = error.and_then(|e| e.quota_id.clone()) {
                details.insert("quotaId".into(), json!(quota));
            }
            details.insert("dailyQuota".into(), json!(daily));
            BlueyError::network("http_429", message).with_details(Value::Object(details))
        }
        500 | 502 | 503 | 504 => BlueyError::network(
            "http_5xx",
            format!("the Gemini API is unavailable (HTTP {http_status}) — try again"),
        ),
        other => BlueyError::ai(
            &format!("http_{other}"),
            format!("the Gemini API returned HTTP {other}"),
        ),
    }
}

/// Whether a failed non-streaming call may be retried (429 / 5xx only).
pub fn is_retryable_status(http_status: u16) -> bool {
    matches!(http_status, 408 | 429 | 500 | 502 | 503 | 504)
}

// ── Embeddings ───────────────────────────────────────────────────────────────

/// What a text is embedded for; drives the `gemini-embedding-2` prompt prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmbedPurpose {
    /// A stored chunk: `title: {title|none} | text: {content}`.
    Document { title: Option<String> },
    /// A retrieval query: `task: search result | query: {content}`.
    Query,
}

/// Whether `model` is the prefix-driven `gemini-embedding-2` (not `-001`).
pub fn uses_prompt_prefixes(model: &str) -> bool {
    let m = model.trim().to_ascii_lowercase();
    let m = m.strip_prefix("models/").unwrap_or(&m);
    m.starts_with("gemini-embedding-2")
}

/// Text to embed for `purpose` (prefixes only for `gemini-embedding-2`).
pub fn embedding_text(purpose: &EmbedPurpose, text: &str, model: &str) -> String {
    if !uses_prompt_prefixes(model) {
        return text.to_string();
    }
    match purpose {
        EmbedPurpose::Document { title } => {
            let title = title
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .unwrap_or("none");
            format!("title: {title} | text: {text}")
        }
        EmbedPurpose::Query => format!("task: search result | query: {text}"),
    }
}

/// `batchEmbedContents` body: one request per text (several parts in one
/// `content` would collapse into a single vector).
pub fn build_batch_embed_body(model: &str, texts: &[String], dims: Option<u32>) -> Value {
    let model_name = if model.starts_with("models/") {
        model.to_string()
    } else {
        format!("models/{model}")
    };
    let requests: Vec<Value> = texts
        .iter()
        .map(|text| {
            let mut request = json!({
                "model": model_name,
                "content": { "parts": [ { "text": text } ] },
            });
            if let Some(dims) = dims {
                request["outputDimensionality"] = json!(dims);
            }
            request
        })
        .collect();
    json!({ "requests": requests })
}

/// `{ "embeddings": [ { "values": [...] }, … ] }` → vectors in request order.
pub fn parse_batch_embeddings(json: &str) -> Result<Vec<Vec<f32>>, serde_json::Error> {
    #[derive(Deserialize)]
    struct Batch {
        #[serde(default)]
        embeddings: Vec<Row>,
    }
    #[derive(Deserialize)]
    struct Row {
        #[serde(default)]
        values: Vec<f32>,
    }
    let batch: Batch = serde_json::from_str(json)?;
    Ok(batch.embeddings.into_iter().map(|r| r.values).collect())
}

/// `{ "embedding": { "values": [...] } }` → vector.
pub fn parse_single_embedding(json: &str) -> Result<Vec<f32>, serde_json::Error> {
    #[derive(Deserialize)]
    struct Single {
        #[serde(default)]
        embedding: Row,
    }
    #[derive(Deserialize, Default)]
    struct Row {
        #[serde(default)]
        values: Vec<f32>,
    }
    let single: Single = serde_json::from_str(json)?;
    Ok(single.embedding.values)
}

// ── Models list ──────────────────────────────────────────────────────────────

/// One entry of `GET /models`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelInfo {
    /// Id without the `models/` prefix.
    pub id: String,
    pub display_name: Option<String>,
    /// `supportedGenerationMethods`, e.g. `generateContent`, `embedContent`,
    /// `bidiGenerateContent`.
    pub supported_methods: Vec<String>,
    pub input_token_limit: Option<u64>,
}

/// Parse one page → `(models, next_page_token)`.
pub fn parse_models_page(
    json: &str,
) -> Result<(Vec<ModelInfo>, Option<String>), serde_json::Error> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Page {
        #[serde(default)]
        models: Vec<WireModel>,
        #[serde(default)]
        next_page_token: Option<String>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct WireModel {
        #[serde(default)]
        name: String,
        #[serde(default)]
        display_name: Option<String>,
        #[serde(default)]
        supported_generation_methods: Vec<String>,
        #[serde(default)]
        input_token_limit: Option<u64>,
    }
    let page: Page = serde_json::from_str(json)?;
    let models = page
        .models
        .into_iter()
        .filter(|m| !m.name.is_empty())
        .map(|m| ModelInfo {
            id: m
                .name
                .strip_prefix("models/")
                .unwrap_or(&m.name)
                .to_string(),
            display_name: m.display_name,
            supported_methods: m.supported_generation_methods,
            input_token_limit: m.input_token_limit,
        })
        .collect();
    let next = page.next_page_token.filter(|t| !t.is_empty());
    Ok((models, next))
}

/// Whether a model is a sensible candidate for a Bluey role: embeddings need
/// `embedContent`; transcription needs `bidiGenerateContent` on a `transcribe`
/// model; text roles need `generateContent` on a non-TTS/image/live/embedding/
/// transcribe model.
pub fn role_filter(role: ModelRole, info: &ModelInfo) -> bool {
    let id = info.id.to_ascii_lowercase();
    let supports = |method: &str| info.supported_methods.iter().any(|m| m == method);
    match role {
        ModelRole::Embedding => supports("embedContent"),
        ModelRole::Transcription => supports("bidiGenerateContent") && id.contains("transcribe"),
        _ => {
            supports("generateContent")
                && !id.contains("tts")
                && !id.contains("image")
                && !id.contains("live")
                && !id.contains("embedding")
                && !id.contains("transcribe")
        }
    }
}

// ── Live API (transcription) ─────────────────────────────────────────────────

/// Options for the first (`setup`) frame of a transcription session.
#[derive(Debug, Clone)]
pub struct LiveSetupOptions<'a> {
    /// e.g. `gemini-3.5-transcribe-live` (with or without `models/`).
    pub model: &'a str,
    /// BCP-47 tag; `None`/`"auto"` → `languageCodes: []` (auto-detect).
    pub language: Option<&'a str>,
    /// ≤ 1,000 terms (best ≤ 100). Empty → omitted.
    pub custom_vocabulary: &'a [String],
    /// `mode: "SMART"` (light clean-up) instead of verbatim.
    pub smart_mode: bool,
    /// Server VAD end-of-speech silence (500–800 ms recommended).
    pub silence_ms: u32,
}

/// The `setup` message (must be the first frame; wait for `setupComplete`).
pub fn live_setup_message(opts: &LiveSetupOptions<'_>) -> Value {
    let model = if opts.model.starts_with("models/") {
        opts.model.to_string()
    } else {
        format!("models/{}", opts.model)
    };
    let language_codes: Vec<&str> = match opts.language {
        Some(tag) if !tag.trim().is_empty() && tag != "auto" => vec![tag.trim()],
        _ => Vec::new(),
    };
    let mut transcription = json!({ "languageCodes": language_codes });
    if !opts.custom_vocabulary.is_empty() {
        transcription["customVocabulary"] = json!(opts.custom_vocabulary);
    }
    if opts.smart_mode {
        transcription["mode"] = json!("SMART");
    }
    json!({
        "setup": {
            "model": model,
            "generationConfig": { "responseModalities": ["TEXT"] },
            "inputAudioTranscription": transcription,
            "realtimeInputConfig": {
                "automaticActivityDetection": {
                    "disabled": false,
                    "silenceDurationMs": opts.silence_ms,
                }
            }
        }
    })
}

/// One audio chunk (base64 PCM16 LE mono at `sample_rate`).
pub fn live_audio_message(base64_pcm: &str, sample_rate: u32) -> Value {
    json!({
        "realtimeInput": {
            "audio": { "data": base64_pcm, "mimeType": format!("audio/pcm;rate={sample_rate}") }
        }
    })
}

/// Force finalization of the current utterance (hybrid VAD) / end of stream.
pub fn live_audio_stream_end() -> Value {
    json!({ "realtimeInput": { "audioStreamEnd": true } })
}

/// Server → client Live messages relevant to transcription.
#[derive(Debug, Clone, PartialEq)]
pub enum LiveEvent {
    SetupComplete,
    /// Speculative text replacing the current partial utterance.
    Interim(String),
    /// Committed transcript text.
    Final {
        text: String,
        language: Option<String>,
    },
    /// The server will close soon; open the replacement session now.
    GoAway {
        time_left_ms: Option<u64>,
    },
    ResumptionUpdate {
        handle: Option<String>,
        resumable: bool,
    },
    /// Server-reported error (`status`/`code` only, never the message body).
    Error(String),
    Other,
}

/// Decode one text frame from the Live socket.
pub fn parse_live_message(text: &str) -> LiveEvent {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return LiveEvent::Other;
    };
    if value.get("setupComplete").is_some() {
        return LiveEvent::SetupComplete;
    }
    if let Some(content) = value.get("serverContent") {
        if let Some(final_text) = content
            .get("inputTranscription")
            .and_then(|t| t.get("text"))
            .and_then(Value::as_str)
        {
            let language = content
                .get("inputTranscription")
                .and_then(|t| t.get("languageCode"))
                .and_then(Value::as_str)
                .map(String::from);
            return LiveEvent::Final {
                text: final_text.to_string(),
                language,
            };
        }
        if let Some(interim) = content
            .get("interimInputTranscription")
            .and_then(|t| t.get("text"))
            .and_then(Value::as_str)
        {
            return LiveEvent::Interim(interim.to_string());
        }
        return LiveEvent::Other;
    }
    if let Some(go_away) = value.get("goAway") {
        let time_left_ms = go_away
            .get("timeLeft")
            .and_then(Value::as_str)
            .and_then(parse_proto_duration)
            .map(|d| d.as_millis() as u64);
        return LiveEvent::GoAway { time_left_ms };
    }
    if let Some(update) = value.get("sessionResumptionUpdate") {
        return LiveEvent::ResumptionUpdate {
            handle: update
                .get("newHandle")
                .and_then(Value::as_str)
                .filter(|h| !h.is_empty())
                .map(String::from),
            resumable: update
                .get("resumable")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        };
    }
    if let Some(error) = value.get("error") {
        let status = error
            .get("status")
            .and_then(Value::as_str)
            .map(String::from)
            .or_else(|| error.get("code").map(|c| c.to_string()))
            .unwrap_or_else(|| "unknown".to_string());
        return LiveEvent::Error(status);
    }
    LiveEvent::Other
}

// ── Batch transcription (`gemini-3.5-transcribe`) & Files API ────────────────

/// Largest recording sent inline as `inlineData`: base64 inflates by 4/3 and
/// the whole request must stay under the 20 MB cap.
pub const INLINE_AUDIO_MAX_BYTES: u64 = 14 * 1024 * 1024;
/// Largest file the Files API accepts (2 GB per file, 48 h retention).
pub const FILES_API_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Extensions of the audio formats the transcribe model accepts.
pub const SUPPORTED_AUDIO_EXTENSIONS: &[&str] =
    &["wav", "mp3", "aiff", "aif", "aac", "ogg", "flac"];
/// Response header of the resumable-upload `start` call carrying the upload URL.
pub const UPLOAD_URL_HEADER: &str = "x-goog-upload-url";
/// The batch speech-to-text model (`generateContent`, not Live).
pub const BATCH_TRANSCRIBE_MODEL: &str = "gemini-3.5-transcribe";

/// The batch model to call for a transcription-role assignment: Live-only
/// models (`…-transcribe-live`) map to their batch sibling, empty → default.
pub fn batch_transcribe_model(assigned: &str) -> String {
    let model = assigned.trim();
    if model.is_empty() {
        return BATCH_TRANSCRIBE_MODEL.to_string();
    }
    match model.strip_suffix("-live") {
        Some(batch) if !batch.is_empty() => batch.to_string(),
        _ => model.to_string(),
    }
}

/// MIME type for an audio file extension the transcribe model accepts
/// (WAV, MP3, AIFF, AAC, OGG, FLAC); `None` for anything else (M4A/MP4
/// containers are not on the documented list).
pub fn audio_mime_for_extension(ext: &str) -> Option<&'static str> {
    match ext
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase()
        .as_str()
    {
        "wav" | "wave" => Some("audio/wav"),
        "mp3" => Some("audio/mp3"),
        "aiff" | "aif" => Some("audio/aiff"),
        "aac" => Some("audio/aac"),
        "ogg" | "oga" => Some("audio/ogg"),
        "flac" => Some("audio/flac"),
        _ => None,
    }
}

/// `…/upload/v1beta/files` — the resumable-upload entry point (the `upload`
/// segment sits *before* the API version).
pub fn files_upload_url(base_url: &str) -> String {
    let api = base(base_url);
    match api.rfind("/v1") {
        Some(idx) => format!("{}/upload{}/files", &api[..idx], &api[idx..]),
        None => format!("{api}/upload/files"),
    }
}

/// `…/files/{id}` for a `File.name` such as `files/abc123`.
pub fn file_url(base_url: &str, name: &str) -> String {
    format!("{}/{}", base(base_url), name.trim_start_matches('/'))
}

/// Body of the resumable-upload `start` request.
pub fn upload_start_body(display_name: &str) -> Value {
    json!({ "file": { "display_name": display_name } })
}

/// A `File` resource (finalize response or `GET …/files/{id}`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UploadedFile {
    pub name: String,
    pub uri: String,
    pub mime_type: Option<String>,
    pub state: Option<String>,
}

impl UploadedFile {
    /// Usable from `fileData` (`ACTIVE`, or no state reported at all).
    pub fn is_active(&self) -> bool {
        self.state
            .as_deref()
            .map(|s| s.eq_ignore_ascii_case("ACTIVE"))
            .unwrap_or(true)
    }

    pub fn is_processing(&self) -> bool {
        self.state
            .as_deref()
            .map(|s| s.eq_ignore_ascii_case("PROCESSING"))
            .unwrap_or(false)
    }

    pub fn is_failed(&self) -> bool {
        self.state
            .as_deref()
            .map(|s| s.eq_ignore_ascii_case("FAILED"))
            .unwrap_or(false)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireFile {
    #[serde(default)]
    name: String,
    #[serde(default)]
    uri: String,
    #[serde(default)]
    mime_type: Option<String>,
    #[serde(default)]
    state: Option<String>,
}

/// Parse `{"file": {…}}` (upload finalize) or a bare `File` (`GET`).
pub fn parse_uploaded_file(json: &str) -> Result<UploadedFile, serde_json::Error> {
    let value: Value = serde_json::from_str(json)?;
    let file = value.get("file").cloned().unwrap_or(value);
    let wire: WireFile = serde_json::from_value(file)?;
    Ok(UploadedFile {
        name: wire.name,
        uri: wire.uri,
        mime_type: wire.mime_type,
        state: wire.state,
    })
}

/// The recording: inline base64 or a previously uploaded file.
#[derive(Debug, Clone, Copy)]
pub enum AudioInput<'a> {
    Inline { mime_type: &'a str, base64: &'a str },
    File { uri: &'a str, mime_type: &'a str },
}

/// `generationConfig.audioTranscriptionConfig` knobs.
#[derive(Debug, Clone, Copy, Default)]
pub struct TranscribeOptions<'a> {
    /// BCP-47 tag; `None` = auto-detect (`languageCodes: []`).
    pub language: Option<&'a str>,
    /// Speaker labels `spk_1`, `spk_2`, … (≤ 8 speakers, ≤ 30 min of audio).
    pub diarization: bool,
    /// Word-level `startOffset` / `endOffset` (≤ 30 min of audio).
    pub word_timestamps: bool,
}

/// Body for `models/gemini-3.5-transcribe:generateContent`: one `user`
/// content holding only the audio part, and `audioTranscriptionConfig` under
/// `generationConfig`. No prompt text, no thinking or sampling parameters —
/// the transcribe model has neither.
pub fn build_transcribe_body(audio: &AudioInput<'_>, opts: &TranscribeOptions<'_>) -> Value {
    let part = match audio {
        AudioInput::Inline { mime_type, base64 } => {
            json!({ "inlineData": { "mimeType": mime_type, "data": base64 } })
        }
        AudioInput::File { uri, mime_type } => {
            json!({ "fileData": { "fileUri": uri, "mimeType": mime_type } })
        }
    };
    let mut config = Map::new();
    let languages: Vec<&str> = opts.language.map(|l| vec![l]).unwrap_or_default();
    config.insert("languageCodes".into(), json!(languages));
    if opts.diarization {
        config.insert("diarization".into(), json!(true));
    }
    if opts.word_timestamps {
        config.insert("wordTimestamp".into(), json!(true));
    }
    json!({
        "contents": [ { "role": "user", "parts": [ part ] } ],
        "generationConfig": { "audioTranscriptionConfig": Value::Object(config) }
    })
}

/// One word with optional timing (`"0.450s"` offsets → milliseconds).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscribedWord {
    pub word: String,
    pub start_ms: Option<u64>,
    pub end_ms: Option<u64>,
}

/// One `audioTranscription` part (a speaker turn) or a plain `text` part.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TranscriptTurn {
    /// `spk_1`, `spk_2`, … with diarization; `None` otherwise.
    pub speaker: Option<String>,
    pub text: String,
    /// Empty unless word timestamps were requested.
    pub words: Vec<TranscribedWord>,
}

impl TranscriptTurn {
    pub fn start_ms(&self) -> Option<u64> {
        self.words.iter().find_map(|w| w.start_ms)
    }

    pub fn end_ms(&self) -> Option<u64> {
        self.words.iter().rev().find_map(|w| w.end_ms)
    }
}

/// A parsed `gemini-3.5-transcribe` response.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Transcription {
    pub turns: Vec<TranscriptTurn>,
    pub finish: Option<String>,
    /// `promptFeedback.blockReason` — the audio itself was refused.
    pub block_reason: Option<String>,
    pub usage: Option<Usage>,
}

impl Transcription {
    /// Turns joined line by line, speaker labels prefixed when present.
    pub fn plain_text(&self) -> String {
        self.turns
            .iter()
            .map(|turn| match &turn.speaker {
                Some(speaker) => format!("{speaker}: {}", turn.text),
                None => turn.text.clone(),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// `"0.450s"` → `450`.
pub fn parse_offset_ms(text: &str) -> Option<u64> {
    parse_proto_duration(text).map(|d| d.as_millis() as u64)
}

/// Parse a transcription response: `audioTranscription` parts become speaker
/// turns (words joined with single spaces when the part carries only words),
/// plain `text` parts become unlabelled turns; thought parts are skipped.
pub fn parse_transcription(json: &str) -> Result<Transcription, serde_json::Error> {
    let wire: WireResponse = serde_json::from_str(json)?;
    let mut out = Transcription {
        usage: wire.usage_metadata.map(|u| Usage {
            prompt: u.prompt_token_count,
            candidates: u.candidates_token_count,
            thoughts: u.thoughts_token_count,
        }),
        block_reason: wire.prompt_feedback.and_then(|f| f.block_reason),
        ..Transcription::default()
    };
    let Some(candidate) = wire.candidates.into_iter().next() else {
        return Ok(out);
    };
    out.finish = candidate.finish_reason;
    let Some(content) = candidate.content else {
        return Ok(out);
    };
    for part in content.parts {
        if part.thought.unwrap_or(false) {
            continue;
        }
        if let Some(transcription) = part.audio_transcription {
            let words: Vec<TranscribedWord> = transcription
                .words
                .into_iter()
                .filter(|w| !w.word.trim().is_empty())
                .map(|w| TranscribedWord {
                    word: w.word.trim().to_string(),
                    start_ms: w.start_offset.as_deref().and_then(parse_offset_ms),
                    end_ms: w.end_offset.as_deref().and_then(parse_offset_ms),
                })
                .collect();
            let text = match transcription.text.filter(|t| !t.trim().is_empty()) {
                Some(text) => text.trim().to_string(),
                None => words
                    .iter()
                    .map(|w| w.word.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
            };
            if text.is_empty() {
                continue;
            }
            out.turns.push(TranscriptTurn {
                speaker: transcription.speaker_label.filter(|s| !s.is_empty()),
                text,
                words,
            });
        } else if let Some(text) = part.text {
            let text = text.trim();
            if !text.is_empty() {
                out.turns.push(TranscriptTurn {
                    speaker: None,
                    text: text.to_string(),
                    words: Vec::new(),
                });
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bluey_core::types::ImageMediaType;
    use pretty_assertions::assert_eq;

    fn text(role: AiRole, text: &str) -> AiMessage {
        AiMessage::text(role, text)
    }

    fn options<'a>(model: &'a str, messages: &'a [AiMessage]) -> GenerateBodyOptions<'a> {
        GenerateBodyOptions {
            model,
            messages,
            max_output_tokens: None,
            temperature: None,
            output_schema: None,
            thinking_level: None,
        }
    }

    #[test]
    fn urls() {
        assert_eq!(
            generate_url(API_BASE, "gemini-3.8-flash", false),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.8-flash:generateContent"
        );
        assert_eq!(
            generate_url("https://proxy.example/v1beta/", "gemini-3.8-flash", true),
            "https://proxy.example/v1beta/models/gemini-3.8-flash:streamGenerateContent?alt=sse"
        );
        assert_eq!(
            generate_url("", "m", false),
            format!("{API_BASE}/models/m:generateContent")
        );
        assert_eq!(
            embed_url(API_BASE, "gemini-embedding-2"),
            format!("{API_BASE}/models/gemini-embedding-2:embedContent")
        );
        assert_eq!(
            batch_embed_url(API_BASE, "gemini-embedding-2"),
            format!("{API_BASE}/models/gemini-embedding-2:batchEmbedContents")
        );
        assert_eq!(
            models_url(API_BASE, None),
            format!("{API_BASE}/models?pageSize=200")
        );
        assert_eq!(
            models_url(API_BASE, Some("tok")),
            format!("{API_BASE}/models?pageSize=200&pageToken=tok")
        );
        let live = live_url("AIzaSECRET");
        assert!(live.starts_with(LIVE_WS_URL_PREFIX));
        assert!(live.ends_with("?key=AIzaSECRET"));
        assert_eq!(
            redact_live_url(&live),
            format!("{LIVE_WS_URL_PREFIX}?key=[redacted]")
        );
        assert_eq!(
            redact_live_url("wss://x/?key=abc&foo=1"),
            "wss://x/?key=[redacted]&foo=1"
        );
    }

    #[test]
    fn model_families() {
        assert!(is_gemini_3("gemini-3.8-flash"));
        assert!(is_gemini_3("models/gemini-3.5-flash-lite"));
        assert!(is_gemini_3("gemini-3-flash-preview"));
        assert!(!is_gemini_3("gemini-2.5-flash"));
        assert!(supports_minimal("gemini-3.5-flash-lite"));
        assert!(supports_minimal("gemini-3.6-flash"));
        assert!(supports_minimal("gemini-3.1-flash-lite"));
        assert!(!supports_minimal("gemini-3.7-flash"));
        assert!(!supports_minimal("gemini-3.8-flash"));
        assert!(!supports_minimal("gemini-3.1-pro-preview"));
        assert!(!supports_minimal("gemini-2.5-flash"));
    }

    #[test]
    fn thinking_policy_table() {
        use AiTask::*;
        use LatencyBudget::*;
        use ReasoningLevel as R;
        let m38 = "gemini-3.8-flash";
        let lite = "gemini-3.5-flash-lite";
        assert_eq!(
            thinking_level_for(Classification, UltraFast, R::None, lite),
            Some(ThinkingLevel::Minimal)
        );
        assert_eq!(
            thinking_level_for(Classification, UltraFast, R::None, m38),
            Some(ThinkingLevel::Low)
        );
        assert_eq!(
            thinking_level_for(Answer, UltraFast, R::None, lite),
            Some(ThinkingLevel::Minimal)
        );
        assert_eq!(
            thinking_level_for(Answer, Fast, R::None, m38),
            Some(ThinkingLevel::Low)
        );
        assert_eq!(
            thinking_level_for(Vision, Fast, R::None, m38),
            Some(ThinkingLevel::Low)
        );
        assert_eq!(
            thinking_level_for(Answer, Balanced, R::None, m38),
            Some(ThinkingLevel::Medium)
        );
        assert_eq!(
            thinking_level_for(Coding, Balanced, R::Light, m38),
            Some(ThinkingLevel::Medium)
        );
        assert_eq!(
            thinking_level_for(Summarization, Fast, R::None, m38),
            Some(ThinkingLevel::Medium)
        );
        assert_eq!(
            thinking_level_for(Research, Balanced, R::None, m38),
            Some(ThinkingLevel::Medium)
        );
        assert_eq!(
            thinking_level_for(DeepReasoning, Fast, R::None, m38),
            Some(ThinkingLevel::High)
        );
        assert_eq!(
            thinking_level_for(SystemDesign, Balanced, R::Light, m38),
            Some(ThinkingLevel::High)
        );
        assert_eq!(
            thinking_level_for(SystemDesign, Balanced, R::None, m38),
            Some(ThinkingLevel::Medium)
        );
        assert_eq!(
            thinking_level_for(Answer, Deep, R::None, m38),
            Some(ThinkingLevel::High)
        );
        assert_eq!(
            thinking_level_for(Answer, Fast, R::Deep, m38),
            Some(ThinkingLevel::High)
        );
        assert_eq!(
            thinking_level_for(Answer, Fast, R::None, "gemini-2.5-flash"),
            None
        );
    }

    #[test]
    fn body_folds_system_messages_and_maps_roles() {
        let messages = vec![
            text(AiRole::System, "You are Bluey."),
            text(AiRole::System, "Be brief."),
            text(AiRole::User, "hi"),
            text(AiRole::Assistant, "hello"),
            text(AiRole::User, "again"),
        ];
        let body = build_generate_body(&options("gemini-3.8-flash", &messages));
        assert_eq!(
            body["systemInstruction"]["parts"][0]["text"],
            "You are Bluey.\n\nBe brief."
        );
        let contents = body["contents"].as_array().unwrap();
        assert_eq!(contents.len(), 3);
        assert_eq!(contents[0]["role"], "user");
        assert_eq!(contents[1]["role"], "model");
        assert_eq!(contents[1]["parts"][0]["text"], "hello");
        assert_eq!(contents[2]["role"], "user");
        assert!(
            body.get("generationConfig").is_none(),
            "empty config is omitted"
        );
    }

    #[test]
    fn body_keeps_consecutive_same_role_turns() {
        let messages = vec![text(AiRole::User, "one"), text(AiRole::User, "two")];
        let body = build_generate_body(&options("gemini-3.8-flash", &messages));
        let contents = body["contents"].as_array().unwrap();
        assert_eq!(
            contents.len(),
            2,
            "Gemini accepts consecutive user turns; nothing is merged"
        );
        assert_eq!(contents[0]["role"], "user");
        assert_eq!(contents[1]["role"], "user");
    }

    #[test]
    fn body_encodes_images_inline_and_preserves_part_order() {
        let messages = vec![AiMessage {
            role: AiRole::User,
            content: vec![
                AiContentPart::Image {
                    media_type: ImageMediaType::Jpeg,
                    data: "QUJD".into(),
                },
                AiContentPart::Text {
                    text: "What is on this screen?".into(),
                },
            ],
        }];
        let body = build_generate_body(&options("gemini-3.8-flash", &messages));
        let parts = body["contents"][0]["parts"].as_array().unwrap();
        assert_eq!(parts[0]["inlineData"]["mimeType"], "image/jpeg");
        assert_eq!(parts[0]["inlineData"]["data"], "QUJD");
        assert_eq!(parts[1]["text"], "What is on this screen?");
        assert!(body.get("systemInstruction").is_none());
    }

    #[test]
    fn body_config_thinking_schema_and_tokens() {
        let messages = vec![text(AiRole::User, "hi")];
        let spec = JsonSchemaSpec {
            name: "answer".into(),
            schema: json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "properties": { "content": { "type": "string" }, "nested": { "$schema": "keep-me" } },
                "required": ["content"],
                "additionalProperties": false
            }),
            strict: Some(true),
        };
        let body = build_generate_body(&GenerateBodyOptions {
            model: "gemini-3.8-flash",
            messages: &messages,
            max_output_tokens: Some(1200),
            temperature: Some(0.2),
            output_schema: Some(&spec),
            thinking_level: Some(ThinkingLevel::Low),
        });
        let config = &body["generationConfig"];
        assert_eq!(config["maxOutputTokens"], 1200);
        assert_eq!(config["thinkingConfig"]["thinkingLevel"], "low");
        assert_eq!(config["responseMimeType"], "application/json");
        assert!(
            config["responseJsonSchema"].get("$schema").is_none(),
            "top-level $schema stripped"
        );
        assert_eq!(
            config["responseJsonSchema"]["properties"]["nested"]["$schema"], "keep-me",
            "nested keys are left alone"
        );
        assert_eq!(config["responseJsonSchema"]["additionalProperties"], false);
        assert!(
            config.get("temperature").is_none(),
            "no sampling params on 3.x"
        );
        assert!(config.get("responseSchema").is_none());
        assert!(config.get("topP").is_none() && config.get("topK").is_none());
        assert!(config.get("candidateCount").is_none());
        assert!(config.get("thinkingBudget").is_none());
    }

    #[test]
    fn medium_thinking_is_the_default_and_omitted() {
        let messages = vec![text(AiRole::User, "hi")];
        let body = build_generate_body(&GenerateBodyOptions {
            thinking_level: Some(ThinkingLevel::Medium),
            ..options("gemini-3.8-flash", &messages)
        });
        assert!(body.get("generationConfig").is_none());
        let body = build_generate_body(&GenerateBodyOptions {
            thinking_level: Some(ThinkingLevel::High),
            ..options("gemini-3.8-flash", &messages)
        });
        assert_eq!(
            body["generationConfig"]["thinkingConfig"]["thinkingLevel"],
            "high"
        );
    }

    #[test]
    fn temperature_is_forwarded_for_pre_3_models_only() {
        let messages = vec![text(AiRole::User, "hi")];
        let body = build_generate_body(&GenerateBodyOptions {
            temperature: Some(0.0),
            ..options("gemini-2.5-flash", &messages)
        });
        assert_eq!(body["generationConfig"]["temperature"], 0.0);
        let body = build_generate_body(&GenerateBodyOptions {
            temperature: Some(0.0),
            ..options("gemini-3.5-flash-lite", &messages)
        });
        assert!(body.get("generationConfig").is_none());
    }

    #[test]
    fn parses_text_chunks_skipping_thoughts() {
        let chunk = parse_response(
            r#"{"candidates":[{"content":{"parts":[{"text":"plan…","thought":true},{"text":"Hel","thoughtSignature":"c2ln"},{"text":"lo"}],"role":"model"},"index":0}],"modelVersion":"gemini-3.8-flash","responseId":"r1"}"#,
        )
        .unwrap();
        assert_eq!(chunk.text, "Hello");
        assert_eq!(chunk.finish, None);
        assert!(chunk.usage.is_none());
        assert!(chunk.block_reason.is_none());
    }

    #[test]
    fn parses_finish_usage_block_and_function_calls() {
        let done = parse_response(
            r#"{"candidates":[{"content":{"parts":[{"text":"."}],"role":"model"},"finishReason":"STOP","index":0}],
                "usageMetadata":{"promptTokenCount":4,"candidatesTokenCount":12,"thoughtsTokenCount":30,"totalTokenCount":46}}"#,
        )
        .unwrap();
        assert_eq!(done.finish.as_deref(), Some("STOP"));
        let usage = done.usage.unwrap();
        assert_eq!(
            (usage.prompt, usage.candidates, usage.thoughts),
            (Some(4), Some(12), Some(30))
        );
        assert_eq!(usage.output_tokens(), Some(42));
        assert_eq!(Usage::default().output_tokens(), None);

        let blocked = parse_response(r#"{"promptFeedback":{"blockReason":"SAFETY"}}"#).unwrap();
        assert_eq!(blocked.block_reason.as_deref(), Some("SAFETY"));
        assert!(blocked.text.is_empty());

        let calls = parse_response(
            r#"{"candidates":[{"content":{"parts":[{"functionCall":{"id":"call_1","name":"exa_search","args":{"query":"q"}}}],"role":"model"},"finishReason":"STOP"}]}"#,
        )
        .unwrap();
        assert_eq!(
            calls.function_calls,
            vec![FunctionCall {
                id: Some("call_1".into()),
                name: "exa_search".into(),
                args: json!({ "query": "q" })
            }]
        );
        assert!(parse_response("not json").is_err());
    }

    #[test]
    fn finish_reasons_and_block_codes() {
        assert_eq!(map_finish_reason("STOP"), FinishReason::Stop);
        assert_eq!(map_finish_reason("MAX_TOKENS"), FinishReason::Length);
        for reason in [
            "SAFETY",
            "RECITATION",
            "PROHIBITED_CONTENT",
            "BLOCKLIST",
            "SPII",
            "MALFORMED_FUNCTION_CALL",
            "OTHER",
        ] {
            assert_eq!(map_finish_reason(reason), FinishReason::Error, "{reason}");
            assert!(is_error_finish(reason));
        }
        assert!(!is_error_finish("STOP"));
        assert_eq!(blocked_code("SAFETY"), "blocked_safety");
        assert_eq!(
            blocked_code("PROHIBITED_CONTENT"),
            "blocked_prohibited_content"
        );
        assert_eq!(blocked_code(""), "blocked_other");
        let error = blocked_error("SAFETY");
        assert_eq!(error.code, "ai.blocked_safety");
        assert_eq!(error.kind, BlueyErrorKind::Ai);
    }

    #[test]
    fn parses_error_bodies_with_retry_and_quota_details() {
        let body = r#"{ "error": { "code": 429, "message": "You exceeded your current quota", "status": "RESOURCE_EXHAUSTED",
          "details": [
            { "@type": "type.googleapis.com/google.rpc.RetryInfo", "retryDelay": "23s" },
            { "@type": "type.googleapis.com/google.rpc.QuotaFailure", "violations": [ { "quotaId": "GenerateRequestsPerDayPerProjectPerModel-FreeTier", "quotaValue": "20" } ] },
            { "@type": "type.googleapis.com/google.rpc.ErrorInfo", "reason": "RATE_LIMIT_EXCEEDED" } ] } }"#;
        let error = parse_error_body(body).unwrap();
        assert_eq!(error.http_code, Some(429));
        assert_eq!(error.status.as_deref(), Some("RESOURCE_EXHAUSTED"));
        assert_eq!(error.reason.as_deref(), Some("RATE_LIMIT_EXCEEDED"));
        assert_eq!(error.retry_after, Some(Duration::from_secs(23)));
        assert!(error.is_daily_quota());

        let mapped = map_gemini_error(429, Some(&error));
        assert_eq!(mapped.kind, BlueyErrorKind::Network);
        assert_eq!(mapped.code, "network.http_429");
        assert!(mapped.message.contains("daily"));
        let details = mapped.details.unwrap();
        assert_eq!(details["retryAfterMs"], 23_000);
        assert_eq!(details["dailyQuota"], true);
        assert_eq!(
            details["quotaId"],
            "GenerateRequestsPerDayPerProjectPerModel-FreeTier"
        );

        assert!(parse_error_body("<html>").is_none());
        assert!(parse_error_body(r#"{"ok":true}"#).is_none());
        assert_eq!(
            parse_proto_duration("0.5s"),
            Some(Duration::from_millis(500))
        );
        assert_eq!(parse_proto_duration("nope"), None);
    }

    #[test]
    fn maps_http_statuses_onto_contract_errors() {
        let invalid_key = GeminiError {
            status: Some("INVALID_ARGUMENT".into()),
            reason: Some("API_KEY_INVALID".into()),
            ..GeminiError::default()
        };
        let e = map_gemini_error(400, Some(&invalid_key));
        assert_eq!(e.code, "config.api_key_invalid");
        assert_eq!(e.recovery, Some(RecoveryAction::ConfigureProvider));

        let e = map_gemini_error(400, None);
        assert_eq!(e.code, "ai.invalid_request");

        let e = map_gemini_error(403, None);
        assert_eq!(
            (e.kind, e.code.as_str()),
            (BlueyErrorKind::Configuration, "config.http_403")
        );

        let e = map_gemini_error(404, None);
        assert_eq!(e.code, "config.model_not_found");
        assert!(e.recoverable);

        let e = map_gemini_error(429, None);
        assert_eq!(e.code, "network.http_429");
        assert!(!e.message.contains("daily"));
        assert_eq!(e.details.unwrap()["dailyQuota"], false);

        for status in [500, 503, 504] {
            let e = map_gemini_error(status, None);
            assert_eq!(e.code, "network.http_5xx", "{status}");
            assert_eq!(e.recovery, Some(RecoveryAction::Retry));
        }
        assert_eq!(map_gemini_error(418, None).code, "ai.http_418");
        assert!(is_retryable_status(429) && is_retryable_status(503));
        assert!(
            !is_retryable_status(400) && !is_retryable_status(403) && !is_retryable_status(404)
        );
    }

    #[test]
    fn embedding_prefixes_only_for_embedding_2() {
        let doc = EmbedPurpose::Document {
            title: Some("Resume".into()),
        };
        assert_eq!(
            embedding_text(&doc, "Rust engineer", "gemini-embedding-2"),
            "title: Resume | text: Rust engineer"
        );
        assert_eq!(
            embedding_text(
                &EmbedPurpose::Document { title: None },
                "chunk",
                "models/gemini-embedding-2"
            ),
            "title: none | text: chunk"
        );
        assert_eq!(
            embedding_text(&EmbedPurpose::Query, "tell me", "gemini-embedding-2"),
            "task: search result | query: tell me"
        );
        assert_eq!(
            embedding_text(&EmbedPurpose::Query, "tell me", "gemini-embedding-001"),
            "tell me"
        );
        assert!(!uses_prompt_prefixes("gemini-embedding-001"));
    }

    #[test]
    fn embedding_bodies_and_responses() {
        let body =
            build_batch_embed_body("gemini-embedding-2", &["a".into(), "b".into()], Some(768));
        let requests = body["requests"].as_array().unwrap();
        assert_eq!(requests.len(), 2, "one request per chunk");
        assert_eq!(requests[0]["model"], "models/gemini-embedding-2");
        assert_eq!(requests[0]["content"]["parts"][0]["text"], "a");
        assert_eq!(requests[1]["outputDimensionality"], 768);
        let no_dims = build_batch_embed_body("models/gemini-embedding-2", &["a".into()], None);
        assert!(no_dims["requests"][0].get("outputDimensionality").is_none());
        assert_eq!(no_dims["requests"][0]["model"], "models/gemini-embedding-2");

        let vectors =
            parse_batch_embeddings(r#"{"embeddings":[{"values":[0.1,0.2]},{"values":[0.3]}]}"#)
                .unwrap();
        assert_eq!(vectors, vec![vec![0.1, 0.2], vec![0.3]]);
        assert_eq!(
            parse_single_embedding(r#"{"embedding":{"values":[1.0,2.0]}}"#).unwrap(),
            vec![1.0, 2.0]
        );
        assert!(parse_batch_embeddings("not json").is_err());
    }

    #[test]
    fn models_page_and_role_filters() {
        let page = r#"{ "models": [
            { "name": "models/gemini-3.8-flash", "displayName": "Gemini 3.8 Flash", "inputTokenLimit": 1048576,
              "supportedGenerationMethods": ["generateContent", "countTokens"] },
            { "name": "models/gemini-3.5-transcribe-live", "supportedGenerationMethods": ["bidiGenerateContent"] },
            { "name": "models/gemini-3.5-transcribe", "supportedGenerationMethods": ["generateContent"] },
            { "name": "models/gemini-embedding-2", "supportedGenerationMethods": ["embedContent", "batchEmbedContents"] },
            { "name": "models/gemini-2.5-flash-preview-tts", "supportedGenerationMethods": ["generateContent"] },
            { "name": "models/gemini-3.1-flash-live-preview", "supportedGenerationMethods": ["bidiGenerateContent"] },
            { "name": "models/nano-banana-2-image", "supportedGenerationMethods": ["generateContent"] }
          ], "nextPageToken": "page-2" }"#;
        let (models, next) = parse_models_page(page).unwrap();
        assert_eq!(next.as_deref(), Some("page-2"));
        assert_eq!(models[0].id, "gemini-3.8-flash");
        assert_eq!(models[0].display_name.as_deref(), Some("Gemini 3.8 Flash"));
        assert_eq!(models[0].input_token_limit, Some(1_048_576));

        let ids = |role: ModelRole| -> Vec<&str> {
            models
                .iter()
                .filter(|m| role_filter(role, m))
                .map(|m| m.id.as_str())
                .collect()
        };
        assert_eq!(ids(ModelRole::Default), vec!["gemini-3.8-flash"]);
        assert_eq!(ids(ModelRole::Vision), vec!["gemini-3.8-flash"]);
        assert_eq!(
            ids(ModelRole::Transcription),
            vec!["gemini-3.5-transcribe-live"]
        );
        assert_eq!(ids(ModelRole::Embedding), vec!["gemini-embedding-2"]);

        let (empty, next) = parse_models_page(r#"{"models":[]}"#).unwrap();
        assert!(empty.is_empty() && next.is_none());
    }

    #[test]
    fn live_frames_round_trip() {
        let vocabulary = vec!["Kubernetes".to_string()];
        let setup = live_setup_message(&LiveSetupOptions {
            model: "gemini-3.5-transcribe-live",
            language: Some("auto"),
            custom_vocabulary: &vocabulary,
            smart_mode: true,
            silence_ms: 800,
        });
        assert_eq!(setup["setup"]["model"], "models/gemini-3.5-transcribe-live");
        assert_eq!(
            setup["setup"]["generationConfig"]["responseModalities"],
            json!(["TEXT"])
        );
        assert_eq!(
            setup["setup"]["inputAudioTranscription"]["languageCodes"],
            json!([])
        );
        assert_eq!(
            setup["setup"]["inputAudioTranscription"]["customVocabulary"],
            json!(["Kubernetes"])
        );
        assert_eq!(setup["setup"]["inputAudioTranscription"]["mode"], "SMART");
        assert_eq!(
            setup["setup"]["realtimeInputConfig"]["automaticActivityDetection"]
                ["silenceDurationMs"],
            800
        );
        assert_eq!(
            setup.as_object().unwrap().len(),
            1,
            "exactly one top-level key"
        );

        let english = live_setup_message(&LiveSetupOptions {
            model: "models/gemini-3.5-transcribe-live",
            language: Some("en-US"),
            custom_vocabulary: &[],
            smart_mode: false,
            silence_ms: 500,
        });
        assert_eq!(
            english["setup"]["model"],
            "models/gemini-3.5-transcribe-live"
        );
        assert_eq!(
            english["setup"]["inputAudioTranscription"]["languageCodes"],
            json!(["en-US"])
        );
        assert!(english["setup"]["inputAudioTranscription"]
            .get("customVocabulary")
            .is_none());
        assert!(english["setup"]["inputAudioTranscription"]
            .get("mode")
            .is_none());

        let audio = live_audio_message("AAAA", LIVE_SAMPLE_RATE);
        assert_eq!(audio["realtimeInput"]["audio"]["data"], "AAAA");
        assert_eq!(
            audio["realtimeInput"]["audio"]["mimeType"],
            "audio/pcm;rate=16000"
        );
        assert_eq!(
            live_audio_stream_end()["realtimeInput"]["audioStreamEnd"],
            true
        );

        assert_eq!(
            parse_live_message(r#"{"setupComplete":{}}"#),
            LiveEvent::SetupComplete
        );
        assert_eq!(
            parse_live_message(
                r#"{"serverContent":{"interimInputTranscription":{"text":"tell me ab"}}}"#
            ),
            LiveEvent::Interim("tell me ab".into())
        );
        assert_eq!(
            parse_live_message(
                r#"{"serverContent":{"inputTranscription":{"text":"Tell me about yourself.","languageCode":"en-US"}}}"#
            ),
            LiveEvent::Final {
                text: "Tell me about yourself.".into(),
                language: Some("en-US".into())
            }
        );
        assert_eq!(
            parse_live_message(r#"{"goAway":{"timeLeft":"30s"}}"#),
            LiveEvent::GoAway {
                time_left_ms: Some(30_000)
            }
        );
        assert_eq!(
            parse_live_message(
                r#"{"sessionResumptionUpdate":{"newHandle":"h-1","resumable":true}}"#
            ),
            LiveEvent::ResumptionUpdate {
                handle: Some("h-1".into()),
                resumable: true
            }
        );
        assert_eq!(
            parse_live_message(
                r#"{"error":{"code":429,"message":"secret prompt echo","status":"RESOURCE_EXHAUSTED"}}"#
            ),
            LiveEvent::Error("RESOURCE_EXHAUSTED".into())
        );
        assert_eq!(
            parse_live_message(r#"{"serverContent":{"turnComplete":true}}"#),
            LiveEvent::Other
        );
        assert_eq!(parse_live_message("garbage"), LiveEvent::Other);
    }
}

#[cfg(test)]
mod transcribe_tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn batch_model_maps_live_models_and_fills_the_default() {
        assert_eq!(
            batch_transcribe_model("gemini-3.5-transcribe-live"),
            "gemini-3.5-transcribe"
        );
        assert_eq!(
            batch_transcribe_model("gemini-3.5-transcribe"),
            "gemini-3.5-transcribe"
        );
        assert_eq!(batch_transcribe_model(""), BATCH_TRANSCRIBE_MODEL);
        assert_eq!(batch_transcribe_model("  "), BATCH_TRANSCRIBE_MODEL);
        assert_eq!(batch_transcribe_model("custom-stt"), "custom-stt");
        assert_eq!(
            batch_transcribe_model("-live"),
            "-live",
            "nothing left to call"
        );
    }

    #[test]
    fn audio_mime_covers_the_documented_formats_only() {
        assert_eq!(audio_mime_for_extension("wav"), Some("audio/wav"));
        assert_eq!(audio_mime_for_extension(".MP3"), Some("audio/mp3"));
        assert_eq!(audio_mime_for_extension("aif"), Some("audio/aiff"));
        assert_eq!(audio_mime_for_extension("FLAC"), Some("audio/flac"));
        assert_eq!(audio_mime_for_extension("ogg"), Some("audio/ogg"));
        assert_eq!(audio_mime_for_extension("aac"), Some("audio/aac"));
        assert_eq!(audio_mime_for_extension("m4a"), None);
        assert_eq!(audio_mime_for_extension("mp4"), None);
        assert_eq!(audio_mime_for_extension(""), None);
        for ext in SUPPORTED_AUDIO_EXTENSIONS {
            assert!(audio_mime_for_extension(ext).is_some(), "{ext}");
        }
    }

    #[test]
    fn files_api_urls_put_upload_before_the_version() {
        assert_eq!(
            files_upload_url(""),
            "https://generativelanguage.googleapis.com/upload/v1beta/files"
        );
        assert_eq!(
            files_upload_url("https://proxy.example.com/gemini/v1beta/"),
            "https://proxy.example.com/gemini/upload/v1beta/files"
        );
        assert_eq!(
            file_url("", "files/abc123"),
            "https://generativelanguage.googleapis.com/v1beta/files/abc123"
        );
        assert_eq!(
            upload_start_body("standup.wav"),
            json!({ "file": { "display_name": "standup.wav" } })
        );
    }

    #[test]
    fn inline_body_carries_only_the_audio_and_auto_detects_language() {
        let body = build_transcribe_body(
            &AudioInput::Inline {
                mime_type: "audio/wav",
                base64: "QUJD",
            },
            &TranscribeOptions::default(),
        );
        assert_eq!(body["contents"].as_array().map(Vec::len), Some(1));
        assert_eq!(body["contents"][0]["role"], "user");
        let parts = body["contents"][0]["parts"].as_array().unwrap();
        assert_eq!(parts.len(), 1, "no prompt text part");
        assert_eq!(parts[0]["inlineData"]["mimeType"], "audio/wav");
        assert_eq!(parts[0]["inlineData"]["data"], "QUJD");
        let config = &body["generationConfig"]["audioTranscriptionConfig"];
        assert_eq!(config["languageCodes"], json!([]));
        assert!(config.get("diarization").is_none());
        assert!(config.get("wordTimestamp").is_none());
        assert!(config.get("mode").is_none(), "VERBATIM is the default");
        let generation = body["generationConfig"].as_object().unwrap();
        for forbidden in [
            "thinkingConfig",
            "temperature",
            "topP",
            "topK",
            "candidateCount",
        ] {
            assert!(!generation.contains_key(forbidden), "{forbidden}");
        }
        assert!(body.get("systemInstruction").is_none());
    }

    #[test]
    fn file_body_sets_language_diarization_and_word_timestamps() {
        let body = build_transcribe_body(
            &AudioInput::File {
                uri: "https://generativelanguage.googleapis.com/v1beta/files/abc",
                mime_type: "audio/mp3",
            },
            &TranscribeOptions {
                language: Some("en-US"),
                diarization: true,
                word_timestamps: true,
            },
        );
        let part = &body["contents"][0]["parts"][0];
        assert_eq!(
            part["fileData"]["fileUri"],
            "https://generativelanguage.googleapis.com/v1beta/files/abc"
        );
        assert_eq!(part["fileData"]["mimeType"], "audio/mp3");
        let config = &body["generationConfig"]["audioTranscriptionConfig"];
        assert_eq!(config["languageCodes"], json!(["en-US"]));
        assert_eq!(config["diarization"], true);
        assert_eq!(config["wordTimestamp"], true);
    }

    #[test]
    fn uploaded_file_parses_wrapped_and_bare_shapes() {
        let wrapped = parse_uploaded_file(
            r#"{"file":{"name":"files/abc","uri":"https://generativelanguage.googleapis.com/v1beta/files/abc","mimeType":"audio/wav","state":"PROCESSING"}}"#,
        )
        .unwrap();
        assert_eq!(wrapped.name, "files/abc");
        assert!(wrapped.is_processing());
        assert!(!wrapped.is_active());
        let bare = parse_uploaded_file(
            r#"{"name":"files/abc","uri":"https://generativelanguage.googleapis.com/v1beta/files/abc","state":"ACTIVE"}"#,
        )
        .unwrap();
        assert!(bare.is_active());
        assert!(!bare.is_failed());
        let stateless = parse_uploaded_file(r#"{"file":{"name":"files/x","uri":"u"}}"#).unwrap();
        assert!(stateless.is_active(), "no state reported → usable");
        assert!(parse_uploaded_file(r#"{"file":{"state":"FAILED"}}"#)
            .unwrap()
            .is_failed());
        assert!(parse_uploaded_file("not json").is_err());
    }

    #[test]
    fn offsets_parse_to_milliseconds() {
        assert_eq!(parse_offset_ms("0.450s"), Some(450));
        assert_eq!(parse_offset_ms("12s"), Some(12_000));
        assert_eq!(parse_offset_ms("1.5"), None);
        assert_eq!(parse_offset_ms("-1s"), None);
    }

    #[test]
    fn diarized_response_becomes_speaker_turns_with_timings() {
        let json = r#"{
          "candidates": [{
            "content": {
              "parts": [
                { "audioTranscription": { "speakerLabel": "spk_1", "words": [
                    { "word": "Hello", "startOffset": "0.100s", "endOffset": "0.450s" },
                    { "word": "world", "startOffset": "0.500s", "endOffset": "0.850s" } ] } },
                { "text": "ignored summary", "thought": true },
                { "audioTranscription": { "speakerLabel": "spk_2", "words": [
                    { "word": "Hi", "startOffset": "1.200s", "endOffset": "1.400s" } ] } }
              ],
              "role": "model"
            },
            "finishReason": "STOP"
          }],
          "usageMetadata": { "promptTokenCount": 40, "candidatesTokenCount": 3 }
        }"#;
        let parsed = parse_transcription(json).unwrap();
        assert_eq!(parsed.finish.as_deref(), Some("STOP"));
        assert_eq!(parsed.turns.len(), 2);
        assert_eq!(parsed.turns[0].speaker.as_deref(), Some("spk_1"));
        assert_eq!(parsed.turns[0].text, "Hello world");
        assert_eq!(parsed.turns[0].start_ms(), Some(100));
        assert_eq!(parsed.turns[0].end_ms(), Some(850));
        assert_eq!(parsed.turns[0].words.len(), 2);
        assert_eq!(parsed.turns[1].speaker.as_deref(), Some("spk_2"));
        assert_eq!(parsed.turns[1].start_ms(), Some(1200));
        assert_eq!(parsed.usage.unwrap().prompt, Some(40));
        assert_eq!(parsed.plain_text(), "spk_1: Hello world\nspk_2: Hi");
    }

    #[test]
    fn plain_text_response_becomes_an_unlabelled_turn() {
        let json = r#"{"candidates":[{"content":{"parts":[{"text":"  Thanks for joining, let's get started.  "}],"role":"model"},"finishReason":"STOP"}]}"#;
        let parsed = parse_transcription(json).unwrap();
        assert_eq!(parsed.turns.len(), 1);
        assert_eq!(parsed.turns[0].speaker, None);
        assert_eq!(
            parsed.turns[0].text,
            "Thanks for joining, let's get started."
        );
        assert!(parsed.turns[0].words.is_empty());
        assert_eq!(parsed.turns[0].start_ms(), None);
    }

    #[test]
    fn blocked_and_empty_responses_are_reported_not_invented() {
        let blocked = parse_transcription(
            r#"{"promptFeedback":{"blockReason":"PROHIBITED_CONTENT"},"candidates":[]}"#,
        )
        .unwrap();
        assert_eq!(blocked.block_reason.as_deref(), Some("PROHIBITED_CONTENT"));
        assert!(blocked.turns.is_empty());
        let empty = parse_transcription(r#"{"candidates":[{"content":{"parts":[]}}]}"#).unwrap();
        assert!(empty.turns.is_empty());
        assert_eq!(empty.plain_text(), "");
        assert!(parse_transcription("not json").is_err());
    }
}
