//! OpenAI-compatible chat-completions / embeddings wire models.
//!
//! Used verbatim by the `openai_compatible` provider and (with a different URL
//! scheme + auth header) by the Azure Foundry v1 provider.

use bluey_core::types::{AiContentPart, AiMessage, AiRole, FinishReason, JsonSchemaSpec};
use serde::Deserialize;
use serde_json::{json, Value};

/// Everything needed to build a chat-completions request body.
#[derive(Debug, Clone)]
pub struct ChatBodyOptions<'a> {
    /// Model (or Azure deployment name) sent as `model`.
    pub model: &'a str,
    pub messages: &'a [AiMessage],
    pub stream: bool,
    /// Adds `stream_options: { include_usage: true }` (streaming only).
    pub include_usage: bool,
    /// Sent as `max_completion_tokens`.
    pub max_output_tokens: Option<u32>,
    pub temperature: Option<f32>,
    /// Structured output via `response_format: { type: "json_schema", ... }`.
    pub output_schema: Option<&'a JsonSchemaSpec>,
}

/// Chat-completions URL for a generic OpenAI-compatible base URL:
/// `{base}/v1/chat/completions`, or `{base}/chat/completions` when the base
/// already ends with `/v1`.
pub fn chat_url(base_url: &str) -> String {
    versioned_url(base_url, "chat/completions")
}

/// Embeddings URL, same `/v1` handling as [`chat_url`].
pub fn embeddings_url(base_url: &str) -> String {
    versioned_url(base_url, "embeddings")
}

/// Models listing URL, same `/v1` handling as [`chat_url`].
pub fn models_url(base_url: &str) -> String {
    versioned_url(base_url, "models")
}

fn versioned_url(base_url: &str, path: &str) -> String {
    let base = base_url.trim_end_matches('/');
    if base.ends_with("/v1") {
        format!("{base}/{path}")
    } else {
        format!("{base}/v1/{path}")
    }
}

/// Build the JSON body for a chat-completions request.
pub fn build_chat_body(opts: &ChatBodyOptions<'_>) -> Value {
    let messages: Vec<Value> = opts.messages.iter().map(message_to_json).collect();
    let mut body = json!({
        "model": opts.model,
        "messages": messages,
    });
    let obj = body.as_object_mut().expect("body is an object");
    if opts.stream {
        obj.insert("stream".into(), json!(true));
        if opts.include_usage {
            obj.insert("stream_options".into(), json!({ "include_usage": true }));
        }
    }
    if let Some(max) = opts.max_output_tokens {
        obj.insert("max_completion_tokens".into(), json!(max));
    }
    if let Some(t) = opts.temperature {
        obj.insert("temperature".into(), json!(t));
    }
    if let Some(spec) = opts.output_schema {
        obj.insert(
            "response_format".into(),
            json!({
                "type": "json_schema",
                "json_schema": {
                    "name": spec.name,
                    "strict": spec.strict.unwrap_or(true),
                    "schema": spec.schema,
                }
            }),
        );
    }
    body
}

/// One message in OpenAI wire shape. Text-only content collapses to a plain
/// string; content with images becomes an array of parts with `image_url`
/// data URLs.
fn message_to_json(message: &AiMessage) -> Value {
    let role = match message.role {
        AiRole::System => "system",
        AiRole::User => "user",
        AiRole::Assistant => "assistant",
    };
    if !message.has_images() {
        return json!({ "role": role, "content": message.text_content() });
    }
    let parts: Vec<Value> = message
        .content
        .iter()
        .map(|part| match part {
            AiContentPart::Text { text } => json!({ "type": "text", "text": text }),
            AiContentPart::Image { media_type, data } => json!({
                "type": "image_url",
                "image_url": { "url": format!("data:{};base64,{}", media_type.as_str(), data) }
            }),
        })
        .collect();
    json!({ "role": role, "content": parts })
}

/// Body for an embeddings request.
pub fn build_embeddings_body(model: &str, texts: &[String]) -> Value {
    json!({ "model": model, "input": texts })
}

// ── Response models ──────────────────────────────────────────────────────────

/// One streamed chat-completion chunk (`object: "chat.completion.chunk"`).
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ChatChunk {
    #[serde(default)]
    pub choices: Vec<ChunkChoice>,
    #[serde(default)]
    pub usage: Option<ChatUsage>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ChunkChoice {
    #[serde(default)]
    pub delta: ChunkDelta,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ChunkDelta {
    #[serde(default)]
    pub content: Option<String>,
    /// Reasoning-model extension; surfaced so callers can ignore it knowingly.
    #[serde(default)]
    pub reasoning_content: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Default)]
pub struct ChatUsage {
    #[serde(default)]
    pub prompt_tokens: Option<u32>,
    #[serde(default)]
    pub completion_tokens: Option<u32>,
}

/// Parse one SSE `data:` payload into a [`ChatChunk`].
pub fn parse_chunk(data: &str) -> Result<ChatChunk, serde_json::Error> {
    serde_json::from_str(data)
}

/// Map an OpenAI `finish_reason` string onto the Bluey [`FinishReason`].
pub fn map_finish_reason(reason: &str) -> FinishReason {
    match reason {
        "length" => FinishReason::Length,
        _ => FinishReason::Stop,
    }
}

/// Non-streaming chat completion (used by `ai_test_connection`).
#[derive(Debug, Clone, Deserialize)]
pub struct ChatCompletion {
    #[serde(default)]
    pub choices: Vec<CompletionChoice>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CompletionChoice {
    #[serde(default)]
    pub message: CompletionMessage,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct CompletionMessage {
    #[serde(default)]
    pub content: Option<String>,
}

/// Embeddings response → vectors in input order.
#[derive(Debug, Clone, Deserialize)]
pub struct EmbeddingsResponse {
    #[serde(default)]
    pub data: Vec<EmbeddingRow>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EmbeddingRow {
    #[serde(default)]
    pub index: Option<u32>,
    pub embedding: Vec<f32>,
}

impl EmbeddingsResponse {
    /// Vectors sorted by their `index` field (defensive: some servers reorder).
    pub fn into_vectors(mut self) -> Vec<Vec<f32>> {
        self.data.sort_by_key(|row| row.index.unwrap_or(u32::MAX));
        self.data.into_iter().map(|row| row.embedding).collect()
    }
}

/// `GET /v1/models` response → model ids.
#[derive(Debug, Clone, Deserialize)]
pub struct ModelsResponse {
    #[serde(default)]
    pub data: Vec<ModelRow>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelRow {
    pub id: String,
}

impl ModelsResponse {
    pub fn into_ids(self) -> Vec<String> {
        self.data.into_iter().map(|m| m.id).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bluey_core::types::ImageMediaType;
    use pretty_assertions::assert_eq;

    fn text_message(role: AiRole, text: &str) -> AiMessage {
        AiMessage::text(role, text)
    }

    #[test]
    fn urls_handle_v1_suffix() {
        assert_eq!(
            chat_url("https://api.openai.com"),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            chat_url("https://openrouter.ai/api/v1"),
            "https://openrouter.ai/api/v1/chat/completions"
        );
        assert_eq!(
            chat_url("https://openrouter.ai/api/v1/"),
            "https://openrouter.ai/api/v1/chat/completions"
        );
        assert_eq!(
            embeddings_url("https://api.openai.com/v1"),
            "https://api.openai.com/v1/embeddings"
        );
        assert_eq!(
            models_url("https://api.openai.com"),
            "https://api.openai.com/v1/models"
        );
    }

    #[test]
    fn builds_streaming_body_with_usage_and_schema() {
        let messages = vec![
            text_message(AiRole::System, "be brief"),
            text_message(AiRole::User, "hi"),
        ];
        let spec = JsonSchemaSpec {
            name: "answer".into(),
            schema: serde_json::json!({"type":"object"}),
            strict: None,
        };
        let body = build_chat_body(&ChatBodyOptions {
            model: "gpt-test",
            messages: &messages,
            stream: true,
            include_usage: true,
            max_output_tokens: Some(256),
            temperature: Some(0.2),
            output_schema: Some(&spec),
        });
        assert_eq!(body["model"], "gpt-test");
        assert_eq!(body["stream"], true);
        assert_eq!(body["stream_options"]["include_usage"], true);
        assert_eq!(body["max_completion_tokens"], 256);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][0]["content"], "be brief");
        assert_eq!(body["response_format"]["type"], "json_schema");
        assert_eq!(body["response_format"]["json_schema"]["name"], "answer");
        assert_eq!(body["response_format"]["json_schema"]["strict"], true);
    }

    #[test]
    fn images_become_data_url_parts() {
        let message = AiMessage {
            role: AiRole::User,
            content: vec![
                AiContentPart::Text {
                    text: "what is this".into(),
                },
                AiContentPart::Image {
                    media_type: ImageMediaType::Png,
                    data: "QUJD".into(),
                },
            ],
        };
        let body = build_chat_body(&ChatBodyOptions {
            model: "m",
            messages: std::slice::from_ref(&message),
            stream: false,
            include_usage: false,
            max_output_tokens: None,
            temperature: None,
            output_schema: None,
        });
        let parts = body["messages"][0]["content"].as_array().unwrap();
        assert_eq!(parts[0]["type"], "text");
        assert_eq!(parts[1]["type"], "image_url");
        assert_eq!(parts[1]["image_url"]["url"], "data:image/png;base64,QUJD");
        assert!(body.get("stream").is_none());
    }

    #[test]
    fn parses_delta_finish_and_usage_chunks() {
        let c = parse_chunk(
            r#"{"id":"x","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"content":"Hel"},"finish_reason":null}]}"#,
        )
        .unwrap();
        assert_eq!(c.choices[0].delta.content.as_deref(), Some("Hel"));
        assert_eq!(c.choices[0].finish_reason, None);

        let c = parse_chunk(
            r#"{"choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":null}"#,
        )
        .unwrap();
        assert_eq!(c.choices[0].finish_reason.as_deref(), Some("stop"));

        // Final usage chunk: empty choices + usage.
        let c = parse_chunk(
            r#"{"choices":[],"usage":{"prompt_tokens":12,"completion_tokens":34,"total_tokens":46}}"#,
        )
        .unwrap();
        assert!(c.choices.is_empty());
        let usage = c.usage.unwrap();
        assert_eq!(usage.prompt_tokens, Some(12));
        assert_eq!(usage.completion_tokens, Some(34));
    }

    #[test]
    fn maps_finish_reasons() {
        assert_eq!(map_finish_reason("stop"), FinishReason::Stop);
        assert_eq!(map_finish_reason("length"), FinishReason::Length);
        assert_eq!(map_finish_reason("tool_calls"), FinishReason::Stop);
    }

    #[test]
    fn embeddings_round_trip() {
        let body = build_embeddings_body("embed-1", &["a".into(), "b".into()]);
        assert_eq!(body["input"][1], "b");
        let resp: EmbeddingsResponse = serde_json::from_str(
            r#"{"data":[{"index":1,"embedding":[3.0]},{"index":0,"embedding":[1.0,2.0]}]}"#,
        )
        .unwrap();
        assert_eq!(resp.into_vectors(), vec![vec![1.0, 2.0], vec![3.0]]);
    }

    #[test]
    fn parses_models_list() {
        let resp: ModelsResponse =
            serde_json::from_str(r#"{"object":"list","data":[{"id":"m1"},{"id":"m2"}]}"#).unwrap();
        assert_eq!(resp.into_ids(), vec!["m1", "m2"]);
    }
}
