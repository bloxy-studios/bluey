//! Anthropic Messages API wire models: request bodies (system extraction,
//! image blocks, structured output via `output_config`) and SSE stream events
//! including `input_json_delta` accumulation.

use bluey_core::types::{AiContentPart, AiMessage, AiRole, FinishReason, JsonSchemaSpec};
use serde::Deserialize;
use serde_json::{json, Value};

/// Header value for `anthropic-version`.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Default `max_tokens` when a request does not specify one (the field is
/// mandatory on the Messages API).
pub const DEFAULT_MAX_TOKENS: u32 = 4096;

/// Messages URL for a base URL (`https://api.anthropic.com` by default).
pub fn messages_url(base_url: &str) -> String {
    format!("{}/v1/messages", base_url.trim_end_matches('/'))
}

/// Models listing URL.
pub fn models_url(base_url: &str) -> String {
    format!("{}/v1/models", base_url.trim_end_matches('/'))
}

/// Options for [`build_messages_body`].
#[derive(Debug, Clone)]
pub struct MessagesBodyOptions<'a> {
    pub model: &'a str,
    pub messages: &'a [AiMessage],
    pub stream: bool,
    pub max_output_tokens: Option<u32>,
    pub temperature: Option<f32>,
    /// Structured output via `output_config.format` (json_schema).
    pub output_schema: Option<&'a JsonSchemaSpec>,
    /// When the API rejected `output_config` (HTTP 400), rebuild the body with
    /// this set: the schema is instructed in the system prompt instead.
    pub schema_as_prompt_fallback: bool,
}

/// Build the JSON body for `POST /v1/messages`. System messages are extracted
/// into the top-level `system` string; remaining messages become user/assistant
/// turns with text/image content blocks.
pub fn build_messages_body(opts: &MessagesBodyOptions<'_>) -> Value {
    let mut system_parts: Vec<String> = Vec::new();
    let mut messages: Vec<Value> = Vec::new();
    for message in opts.messages {
        match message.role {
            AiRole::System => {
                let text = message.text_content();
                if !text.is_empty() {
                    system_parts.push(text);
                }
            }
            AiRole::User | AiRole::Assistant => messages.push(message_to_json(message)),
        }
    }

    if opts.schema_as_prompt_fallback {
        if let Some(spec) = opts.output_schema {
            system_parts.push(format!(
                "Respond with a single JSON object that validates against this JSON Schema \
                 (no prose, no markdown fences):\n{}",
                spec.schema
            ));
        }
    }

    let mut body = json!({
        "model": opts.model,
        "max_tokens": opts.max_output_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
        "messages": messages,
    });
    let obj = body.as_object_mut().expect("body is an object");
    if !system_parts.is_empty() {
        obj.insert("system".into(), json!(system_parts.join("\n\n")));
    }
    if opts.stream {
        obj.insert("stream".into(), json!(true));
    }
    if let Some(t) = opts.temperature {
        obj.insert("temperature".into(), json!(t));
    }
    if let Some(spec) = opts.output_schema {
        if !opts.schema_as_prompt_fallback {
            obj.insert(
                "output_config".into(),
                json!({ "format": { "type": "json_schema", "schema": spec.schema } }),
            );
        }
    }
    body
}

fn message_to_json(message: &AiMessage) -> Value {
    let role = match message.role {
        AiRole::Assistant => "assistant",
        _ => "user",
    };
    let blocks: Vec<Value> = message
        .content
        .iter()
        .filter_map(|part| match part {
            AiContentPart::Text { text } => {
                if text.is_empty() {
                    None
                } else {
                    Some(json!({ "type": "text", "text": text }))
                }
            }
            AiContentPart::Image { media_type, data } => Some(json!({
                "type": "image",
                "source": { "type": "base64", "media_type": media_type.as_str(), "data": data }
            })),
        })
        .collect();
    json!({ "role": role, "content": blocks })
}

// ── SSE stream events ────────────────────────────────────────────────────────

/// Decoded Anthropic stream event (only the fields Bluey consumes).
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    /// `message_start` — carries initial usage (input tokens).
    MessageStart { input_tokens: Option<u32> },
    /// `content_block_start` for any block type.
    ContentBlockStart { index: u32 },
    /// `content_block_delta` with `text_delta`.
    TextDelta { text: String },
    /// `content_block_delta` with `input_json_delta` (accumulate `partial_json`).
    InputJsonDelta { partial_json: String },
    /// `content_block_stop`.
    ContentBlockStop { index: u32 },
    /// `message_delta` — stop reason + cumulative output tokens.
    MessageDelta {
        stop_reason: Option<String>,
        output_tokens: Option<u32>,
    },
    /// `message_stop`.
    MessageStop,
    /// `ping` keep-alive.
    Ping,
    /// `error` event.
    Error { error_type: String, message: String },
    /// Anything Bluey does not consume (`thinking_delta`, `signature_delta`, …).
    Other,
}

/// Parse one SSE frame (`event:` name + `data:` JSON) into a [`StreamEvent`].
pub fn parse_event(event_name: &str, data: &str) -> Result<StreamEvent, serde_json::Error> {
    match event_name {
        "message_start" => {
            #[derive(Deserialize)]
            struct MessageStart {
                #[serde(default)]
                message: MessageHead,
            }
            #[derive(Deserialize, Default)]
            struct MessageHead {
                #[serde(default)]
                usage: Option<UsageIn>,
            }
            #[derive(Deserialize)]
            struct UsageIn {
                #[serde(default)]
                input_tokens: Option<u32>,
            }
            let parsed: MessageStart = serde_json::from_str(data)?;
            Ok(StreamEvent::MessageStart {
                input_tokens: parsed.message.usage.and_then(|u| u.input_tokens),
            })
        }
        "content_block_start" => {
            #[derive(Deserialize)]
            struct BlockStart {
                #[serde(default)]
                index: u32,
            }
            let parsed: BlockStart = serde_json::from_str(data)?;
            Ok(StreamEvent::ContentBlockStart {
                index: parsed.index,
            })
        }
        "content_block_delta" => {
            #[derive(Deserialize)]
            struct BlockDelta {
                delta: Delta,
            }
            #[derive(Deserialize)]
            #[serde(tag = "type")]
            enum Delta {
                #[serde(rename = "text_delta")]
                Text { text: String },
                #[serde(rename = "input_json_delta")]
                InputJson { partial_json: String },
                #[serde(other)]
                Other,
            }
            let parsed: BlockDelta = serde_json::from_str(data)?;
            Ok(match parsed.delta {
                Delta::Text { text } => StreamEvent::TextDelta { text },
                Delta::InputJson { partial_json } => StreamEvent::InputJsonDelta { partial_json },
                Delta::Other => StreamEvent::Other,
            })
        }
        "content_block_stop" => {
            #[derive(Deserialize)]
            struct BlockStop {
                #[serde(default)]
                index: u32,
            }
            let parsed: BlockStop = serde_json::from_str(data)?;
            Ok(StreamEvent::ContentBlockStop {
                index: parsed.index,
            })
        }
        "message_delta" => {
            #[derive(Deserialize)]
            struct MessageDelta {
                #[serde(default)]
                delta: DeltaBody,
                #[serde(default)]
                usage: Option<UsageOut>,
            }
            #[derive(Deserialize, Default)]
            struct DeltaBody {
                #[serde(default)]
                stop_reason: Option<String>,
            }
            #[derive(Deserialize)]
            struct UsageOut {
                #[serde(default)]
                output_tokens: Option<u32>,
            }
            let parsed: MessageDelta = serde_json::from_str(data)?;
            Ok(StreamEvent::MessageDelta {
                stop_reason: parsed.delta.stop_reason,
                output_tokens: parsed.usage.and_then(|u| u.output_tokens),
            })
        }
        "message_stop" => Ok(StreamEvent::MessageStop),
        "ping" => Ok(StreamEvent::Ping),
        "error" => {
            #[derive(Deserialize)]
            struct ErrorEvent {
                error: ErrorBody,
            }
            #[derive(Deserialize)]
            struct ErrorBody {
                #[serde(rename = "type", default)]
                error_type: String,
                #[serde(default)]
                message: String,
            }
            let parsed: ErrorEvent = serde_json::from_str(data)?;
            Ok(StreamEvent::Error {
                error_type: parsed.error.error_type,
                message: parsed.error.message,
            })
        }
        _ => Ok(StreamEvent::Other),
    }
}

/// Map an Anthropic `stop_reason` onto the Bluey [`FinishReason`].
pub fn map_stop_reason(reason: &str) -> FinishReason {
    match reason {
        "max_tokens" => FinishReason::Length,
        _ => FinishReason::Stop,
    }
}

/// Accumulates `input_json_delta` fragments into a parseable JSON value.
#[derive(Debug, Default)]
pub struct JsonAccumulator {
    buffer: String,
}

impl JsonAccumulator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append one `partial_json` fragment.
    pub fn push(&mut self, fragment: &str) {
        self.buffer.push_str(fragment);
    }

    /// Whether anything was accumulated.
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// The raw accumulated text so far.
    pub fn text(&self) -> &str {
        &self.buffer
    }

    /// Parse the accumulated fragments as JSON, if complete.
    pub fn finish(&self) -> Option<Value> {
        serde_json::from_str(&self.buffer).ok()
    }
}

/// Whether an HTTP-400 error body indicates the API rejected `output_config`
/// (→ retry with the schema instructed in the prompt instead).
pub fn is_output_config_rejection(status: u16, body: &str) -> bool {
    status == 400 && (body.contains("output_config") || body.contains("output_format"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bluey_core::types::ImageMediaType;
    use pretty_assertions::assert_eq;

    #[test]
    fn extracts_system_and_builds_blocks() {
        let messages = vec![
            AiMessage::text(AiRole::System, "You are terse."),
            AiMessage {
                role: AiRole::User,
                content: vec![
                    AiContentPart::Text {
                        text: "what is on screen".into(),
                    },
                    AiContentPart::Image {
                        media_type: ImageMediaType::Jpeg,
                        data: "QUJD".into(),
                    },
                ],
            },
            AiMessage::text(AiRole::Assistant, "A code editor."),
        ];
        let body = build_messages_body(&MessagesBodyOptions {
            model: "claude-sonnet-5",
            messages: &messages,
            stream: true,
            max_output_tokens: Some(1000),
            temperature: Some(0.1),
            output_schema: None,
            schema_as_prompt_fallback: false,
        });
        assert_eq!(body["system"], "You are terse.");
        assert_eq!(body["stream"], true);
        assert_eq!(body["max_tokens"], 1000);
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[0]["content"][1]["type"], "image");
        assert_eq!(msgs[0]["content"][1]["source"]["media_type"], "image/jpeg");
        assert_eq!(msgs[0]["content"][1]["source"]["type"], "base64");
        assert_eq!(msgs[1]["role"], "assistant");
    }

    #[test]
    fn output_config_and_prompt_fallback() {
        let messages = vec![AiMessage::text(AiRole::User, "classify")];
        let spec = JsonSchemaSpec {
            name: "classification".into(),
            schema: serde_json::json!({"type":"object","properties":{"label":{"type":"string"}}}),
            strict: Some(true),
        };
        let body = build_messages_body(&MessagesBodyOptions {
            model: "m",
            messages: &messages,
            stream: false,
            max_output_tokens: None,
            temperature: None,
            output_schema: Some(&spec),
            schema_as_prompt_fallback: false,
        });
        assert_eq!(body["output_config"]["format"]["type"], "json_schema");
        assert_eq!(body["max_tokens"], DEFAULT_MAX_TOKENS);
        assert!(body.get("system").is_none());

        let fallback = build_messages_body(&MessagesBodyOptions {
            model: "m",
            messages: &messages,
            stream: false,
            max_output_tokens: None,
            temperature: None,
            output_schema: Some(&spec),
            schema_as_prompt_fallback: true,
        });
        assert!(fallback.get("output_config").is_none());
        let system = fallback["system"].as_str().unwrap();
        assert!(system.contains("JSON Schema"));
        assert!(system.contains("\"label\""));
    }

    #[test]
    fn parses_the_event_sequence() {
        let e = parse_event(
            "message_start",
            r#"{"type":"message_start","message":{"id":"m","usage":{"input_tokens":25,"output_tokens":1}}}"#,
        )
        .unwrap();
        assert_eq!(
            e,
            StreamEvent::MessageStart {
                input_tokens: Some(25)
            }
        );

        let e = parse_event(
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"ello frien"}}"#,
        )
        .unwrap();
        assert_eq!(
            e,
            StreamEvent::TextDelta {
                text: "ello frien".into()
            }
        );

        let e = parse_event(
            "content_block_delta",
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"location\": \"San Fra"}}"#,
        )
        .unwrap();
        assert_eq!(
            e,
            StreamEvent::InputJsonDelta {
                partial_json: "{\"location\": \"San Fra".into()
            }
        );

        let e = parse_event(
            "message_delta",
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":103}}"#,
        )
        .unwrap();
        assert_eq!(
            e,
            StreamEvent::MessageDelta {
                stop_reason: Some("end_turn".into()),
                output_tokens: Some(103)
            }
        );

        assert_eq!(
            parse_event("message_stop", r#"{"type":"message_stop"}"#).unwrap(),
            StreamEvent::MessageStop
        );
        assert_eq!(
            parse_event("ping", r#"{"type":"ping"}"#).unwrap(),
            StreamEvent::Ping
        );

        let e = parse_event(
            "error",
            r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
        )
        .unwrap();
        assert_eq!(
            e,
            StreamEvent::Error {
                error_type: "overloaded_error".into(),
                message: "Overloaded".into()
            }
        );

        // thinking deltas and unknown events are Other
        let e = parse_event(
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"..."}}"#,
        )
        .unwrap();
        assert_eq!(e, StreamEvent::Other);
    }

    #[test]
    fn accumulates_input_json() {
        let mut acc = JsonAccumulator::new();
        assert!(acc.is_empty());
        acc.push("{\"location\": \"San Fra");
        assert!(acc.finish().is_none());
        acc.push("ncisco\"}");
        let value = acc.finish().unwrap();
        assert_eq!(value["location"], "San Francisco");
    }

    #[test]
    fn stop_reasons_and_rejection_detection() {
        assert_eq!(map_stop_reason("end_turn"), FinishReason::Stop);
        assert_eq!(map_stop_reason("max_tokens"), FinishReason::Length);
        assert!(is_output_config_rejection(
            400,
            r#"{"error":{"message":"output_config: Extra inputs are not permitted"}}"#
        ));
        assert!(!is_output_config_rejection(400, r#"{"error":"bad model"}"#));
        assert!(!is_output_config_rejection(500, "output_config"));
    }

    #[test]
    fn urls() {
        assert_eq!(
            messages_url("https://api.anthropic.com"),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            models_url("https://api.anthropic.com/"),
            "https://api.anthropic.com/v1/models"
        );
    }
}
