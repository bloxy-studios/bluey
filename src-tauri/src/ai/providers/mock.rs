//! Deterministic mock provider for developer mode / the `dev-tools` feature:
//! streamed markdown (a code block for coding tasks, schema-valid JSON when a
//! structured output is requested), with latency/failure knobs from
//! `dev_simulate`.

use std::sync::Arc;
use std::time::Duration;

use bluey_core::types::{AiTask, FinishReason};
use bluey_core::{BlueyError, BlueyResult};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::{channel_stream, AiProvider, ChunkStream, ProviderRequest, StreamItem};
use crate::state::DevState;

pub struct MockProvider {
    dev: Arc<DevState>,
}

impl MockProvider {
    pub fn new(dev: Arc<DevState>) -> Self {
        Self { dev }
    }
}

#[async_trait::async_trait]
impl AiProvider for MockProvider {
    async fn stream(
        &self,
        request: &ProviderRequest,
        token: CancellationToken,
    ) -> BlueyResult<ChunkStream> {
        let knobs = self.dev.knobs();
        let delay = Duration::from_millis(knobs.ai_latency_ms.unwrap_or(35));
        let failure = knobs.ai_failure_code.clone();
        let text = mock_text(request);
        let (tx, stream) = channel_stream();
        tauri::async_runtime::spawn(async move {
            let words: Vec<String> = split_chunks(&text);
            let total = words.len() as u32;
            for (i, word) in words.into_iter().enumerate() {
                tokio::select! {
                    _ = token.cancelled() => return,
                    _ = tokio::time::sleep(delay) => {}
                }
                if let (1, Some(code)) = (i, failure.as_deref()) {
                    let _ = tx
                        .send(Err(BlueyError::ai(code, "simulated failure (dev tools)")))
                        .await;
                    return;
                }
                if tx.send(Ok(StreamItem::Delta(word))).await.is_err() {
                    return;
                }
            }
            let _ = tx
                .send(Ok(StreamItem::Usage {
                    input: Some(120),
                    output: Some(total),
                }))
                .await;
            let _ = tx.send(Ok(StreamItem::Finished(FinishReason::Stop))).await;
        });
        Ok(stream)
    }

    async fn embed(&self, _model: &str, texts: &[String]) -> BlueyResult<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|t| pseudo_embedding(t)).collect())
    }

    async fn list_models(&self) -> BlueyResult<Vec<String>> {
        Ok(vec![
            "mock-fast".into(),
            "mock-default".into(),
            "mock-reasoning".into(),
            "mock-vision".into(),
            "mock-embedding".into(),
        ])
    }
}

fn mock_text(request: &ProviderRequest) -> String {
    if let Some(spec) = &request.output_schema {
        return synthesize_json(&spec.schema).to_string();
    }
    match request.task {
        AiTask::Coding => "Here is a working approach.\n\n\
**Idea**: use a hash map for O(1) lookups while scanning once.\n\n\
```python\ndef two_sum(nums, target):\n    seen = {}\n    for i, n in enumerate(nums):\n        if target - n in seen:\n            return [seen[target - n], i]\n        seen[n] = i\n    return []\n```\n\n\
**Complexity**: O(n) time, O(n) space."
            .to_string(),
        AiTask::Summarization => {
            "- The discussion covered goals, blockers and next steps.\n\
- A decision was made to proceed with the current plan.\n\
- Follow-ups were assigned with owners and dates."
                .to_string()
        }
        _ => "Here's a concise answer based on your screen and conversation.\n\n\
- The key point is to acknowledge the question directly.\n\
- Support it with one concrete example from your experience.\n\
- Close with the outcome and what you learned.\n\n\
*(mock response — configure a provider in Settings → AI for real answers)*"
            .to_string(),
    }
}

/// Split text into small streaming chunks (words with their whitespace).
fn split_chunks(text: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        current.push(ch);
        if ch == ' ' || ch == '\n' {
            chunks.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

/// Build a minimal instance that satisfies a JSON schema (best effort).
fn synthesize_json(schema: &Value) -> Value {
    match schema.get("type").and_then(Value::as_str) {
        Some("object") => {
            let mut object = serde_json::Map::new();
            if let Some(props) = schema.get("properties").and_then(Value::as_object) {
                for (key, subschema) in props {
                    object.insert(key.clone(), synthesize_json(subschema));
                }
            }
            Value::Object(object)
        }
        Some("array") => {
            let item = schema
                .get("items")
                .map(synthesize_json)
                .unwrap_or(Value::Null);
            Value::Array(vec![item])
        }
        Some("string") => schema
            .get("enum")
            .and_then(Value::as_array)
            .and_then(|options| options.first().cloned())
            .unwrap_or_else(|| Value::String("example".into())),
        Some("number") => serde_json::json!(0.5),
        Some("integer") => serde_json::json!(1),
        Some("boolean") => Value::Bool(true),
        _ => {
            if let Some(options) = schema.get("enum").and_then(Value::as_array) {
                return options.first().cloned().unwrap_or(Value::Null);
            }
            if let Some(any_of) = schema.get("anyOf").and_then(Value::as_array) {
                return any_of.first().map(synthesize_json).unwrap_or(Value::Null);
            }
            if schema.get("properties").is_some() {
                return synthesize_json(&serde_json::json!({
                    "type": "object",
                    "properties": schema.get("properties").cloned().unwrap_or(Value::Null)
                }));
            }
            Value::Null
        }
    }
}

/// Deterministic 8-dim unit-ish vector from a text hash.
fn pseudo_embedding(text: &str) -> Vec<f32> {
    use std::hash::{Hash, Hasher};
    let mut vector = Vec::with_capacity(8);
    for salt in 0..8u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        salt.hash(&mut hasher);
        text.hash(&mut hasher);
        let value = hasher.finish();
        vector.push(((value % 2000) as f32 / 1000.0) - 1.0);
    }
    let norm = vector.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-6);
    vector.iter().map(|v| v / norm).collect()
}
