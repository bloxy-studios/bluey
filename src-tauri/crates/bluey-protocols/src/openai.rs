//! OpenAI-compatible chat-completions / embeddings wire models.
//!
//! Used verbatim by the `openai_compatible` provider and (with a different URL
//! scheme + auth header) by the Azure Foundry v1 provider.

use bluey_core::types::{
    AiContentPart, AiMessage, AiRole, FinishReason, JsonSchemaSpec, LatencyBudget, ReasoningLevel,
};
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
    /// `reasoning_effort` for reasoning-model families (see
    /// [`reasoning_effort_for`]); when set, `temperature` is not sent.
    pub reasoning_effort: Option<&'a str>,
    /// Structured output via `response_format: { type: "json_schema", ... }`.
    pub output_schema: Option<&'a JsonSchemaSpec>,
    /// When the endpoint rejected `response_format` (HTTP 400), the schema is
    /// instructed in a leading system message instead.
    pub schema_as_prompt_fallback: bool,
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

/// OpenAI's reasoning families — GPT-5 and later, the o-series — reject
/// `temperature` (and `top_p`, the penalties) and take `reasoning_effort`
/// instead. Decided by the model family, whichever endpoint serves it
/// (Foundry, OpenAI, a gateway's `openai/gpt-5`); `*-chat` variants are chat
/// models and keep sampling.
pub fn is_reasoning_model(model: &str) -> bool {
    let lower = model.to_ascii_lowercase();
    let name = lower.rsplit('/').next().unwrap_or(&lower);
    if name.contains("-chat") {
        return false;
    }
    let gpt_major = name
        .strip_prefix("gpt-")
        .and_then(|rest| rest.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|major| major.parse::<u32>().ok());
    let o_series = ["o1", "o3", "o4"]
        .iter()
        .any(|series| name == *series || name.starts_with(&format!("{series}-")));
    gpt_major.is_some_and(|major| major >= 5) || o_series
}

/// `reasoning_effort` for a request to `model` — Bluey's reasoning level and
/// latency budget through the Codex policy (`low`/`medium`/`high`) — or `None`
/// when the model is not a reasoning model.
pub fn reasoning_effort_for(
    model: &str,
    level: ReasoningLevel,
    latency: LatencyBudget,
) -> Option<String> {
    is_reasoning_model(model).then(|| crate::codex::reasoning_effort(level, latency, &[], None))
}

/// Whether an HTTP-400 error body says the endpoint does not take native
/// structured output (→ resend with the schema in the prompt instead).
pub fn is_response_format_rejection(status: u16, body: &str) -> bool {
    status == 400 && (body.contains("response_format") || body.contains("json_schema"))
}

/// Build the JSON body for a chat-completions request.
pub fn build_chat_body(opts: &ChatBodyOptions<'_>) -> Value {
    let mut messages: Vec<Value> = opts.messages.iter().map(message_to_json).collect();
    let schema_in_prompt = opts
        .output_schema
        .filter(|_| opts.schema_as_prompt_fallback);
    if let Some(spec) = schema_in_prompt {
        let instruction = crate::anthropic::schema_prompt(spec);
        messages.insert(0, json!({ "role": "system", "content": instruction }));
    }
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
    match opts.reasoning_effort {
        // Reasoning models reject sampling knobs (HTTP 400 "Unsupported parameter").
        Some(effort) => {
            obj.insert("reasoning_effort".into(), json!(effort));
        }
        None => {
            if let Some(t) = opts.temperature {
                obj.insert("temperature".into(), json!(t));
            }
        }
    }
    if let Some(spec) = opts.output_schema.filter(|_| schema_in_prompt.is_none()) {
        // Strict structured outputs reject a schema whose objects leave a property out of
        // `required` or lack `additionalProperties: false` (HTTP 400) — zod's output does
        // both, so send the strict-mode variant (see `crate::json_schema`).
        let strict = spec.strict.unwrap_or(true);
        obj.insert(
            "response_format".into(),
            json!({
                "type": "json_schema",
                "json_schema": {
                    "name": spec.name,
                    "strict": strict,
                    "schema": if strict {
                        crate::json_schema::strict_variant(&spec.schema)
                    } else {
                        crate::json_schema::strip_meta(&spec.schema)
                    },
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
            schema: serde_json::json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "properties": { "content": { "type": "string" }, "title": { "type": "string" } },
                "required": ["content"]
            }),
            strict: None,
        };
        let body = build_chat_body(&ChatBodyOptions {
            model: "gpt-test",
            messages: &messages,
            stream: true,
            include_usage: true,
            max_output_tokens: Some(256),
            temperature: Some(0.2),
            reasoning_effort: None,
            schema_as_prompt_fallback: false,
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
        // The strict-mode variant of the schema goes on the wire (`crate::json_schema`).
        let schema = &body["response_format"]["json_schema"]["schema"];
        assert!(schema.get("$schema").is_none());
        assert_eq!(schema["required"], serde_json::json!(["content", "title"]));
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(
            schema["properties"]["title"]["type"],
            serde_json::json!(["string", "null"])
        );
    }

    #[test]
    fn reasoning_families_get_an_effort_and_never_a_temperature() {
        for model in [
            "gpt-5.6-terra",
            "gpt-6-astra",
            "GPT-5",
            "openai/gpt-5.5",
            "o3",
            "o4-mini",
        ] {
            assert!(is_reasoning_model(model), "{model}");
        }
        for model in [
            "gpt-4.1",
            "gpt-4.1-mini",
            "gpt-5-chat-latest",
            "gpt-oss-120b",
            "llama-3",
        ] {
            assert!(!is_reasoning_model(model), "{model}");
        }
        let effort =
            reasoning_effort_for("gpt-6-astra", ReasoningLevel::Deep, LatencyBudget::Balanced);
        assert_eq!(effort.as_deref(), Some("high"));
        assert_eq!(
            reasoning_effort_for("gpt-4.1", ReasoningLevel::Deep, LatencyBudget::Balanced),
            None
        );

        let messages = vec![text_message(AiRole::User, "hi")];
        let body = build_chat_body(&ChatBodyOptions {
            model: "gpt-5.6-terra",
            messages: &messages,
            stream: true,
            include_usage: false,
            max_output_tokens: None,
            temperature: Some(0.6),
            reasoning_effort: effort.as_deref(),
            schema_as_prompt_fallback: false,
            output_schema: None,
        });
        assert_eq!(body["reasoning_effort"], "high");
        assert!(body.get("temperature").is_none(), "{body}");
    }

    #[test]
    fn the_schema_moves_into_the_prompt_when_response_format_was_rejected() {
        let spec = JsonSchemaSpec {
            name: "answer".into(),
            schema: json!({ "type": "object" }),
            strict: None,
        };
        let messages = vec![text_message(AiRole::User, "hi")];
        let body = build_chat_body(&ChatBodyOptions {
            model: "llama-3",
            messages: &messages,
            stream: true,
            include_usage: false,
            max_output_tokens: None,
            temperature: None,
            reasoning_effort: None,
            output_schema: Some(&spec),
            schema_as_prompt_fallback: true,
        });
        assert!(body.get("response_format").is_none(), "{body}");
        assert_eq!(body["messages"][0]["role"], "system");
        assert!(body["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("JSON Schema"));
        assert_eq!(body["messages"][1]["content"], "hi");

        let rejected = r#"{"error":{"message":"Invalid parameter: 'response_format' of type 'json_schema' is not supported with this model."}}"#;
        assert!(is_response_format_rejection(400, rejected));
        assert!(!is_response_format_rejection(
            400,
            r#"{"error":{"message":"bad temperature"}}"#
        ));
        assert!(!is_response_format_rejection(500, rejected));
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
            reasoning_effort: None,
            schema_as_prompt_fallback: false,
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
