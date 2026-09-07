//! Research-agent sidecar protocol (see `docs/AGENT_SIDECAR_PROTOCOL.md`):
//! request encoding and event → [`DeepResearchEvent`] mapping.

use bluey_core::types::{Citation, DeepResearchEvent, DeepResearchRequest};
use serde::Deserialize;
use serde_json::{json, Value};

/// Something the agent sidecar sent us.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentEvent {
    /// Mapped onto the frontend `research.event` payload.
    Research(DeepResearchEvent),
    /// The agent wants a document's text (`document.request`); Rust answers
    /// with `document.response`.
    DocumentRequest {
        request_id: String,
        document_id: String,
    },
}

/// Build the `research.run` params: the request itself plus the model the
/// router chose (the sidecar also honours `BLUEY_RESEARCH_MODEL`).
pub fn research_run_params(request: &DeepResearchRequest, model: Option<&str>) -> Value {
    let mut params = serde_json::to_value(request).unwrap_or(Value::Null);
    if let (Some(model), Some(obj)) = (model, params.as_object_mut()) {
        obj.insert("model".into(), json!(model));
    }
    params
}

/// Build the `research.cancel` params.
pub fn research_cancel_params(job_id: &str) -> Value {
    json!({ "jobId": job_id })
}

/// Build the `document.response` params answering a `document.request`.
pub fn document_response_params(
    request_id: &str,
    document_id: &str,
    result: Result<&str, &str>,
) -> Value {
    match result {
        Ok(text) => json!({ "requestId": request_id, "documentId": document_id, "text": text }),
        Err(error) => {
            json!({ "requestId": request_id, "documentId": document_id, "error": error })
        }
    }
}

/// Parse one agent event by name. Returns `None` for unknown events or
/// undecodable payloads.
pub fn parse_agent_event(event: &str, data: Value) -> Option<AgentEvent> {
    match event {
        "research.started" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Started {
                job_id: String,
            }
            let s: Started = serde_json::from_value(data).ok()?;
            Some(AgentEvent::Research(DeepResearchEvent::Started {
                job_id: s.job_id,
            }))
        }
        "research.progress" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Progress {
                job_id: String,
                #[serde(default)]
                message: String,
            }
            let p: Progress = serde_json::from_value(data).ok()?;
            Some(AgentEvent::Research(DeepResearchEvent::Progress {
                job_id: p.job_id,
                message: p.message,
            }))
        }
        "research.toolCall" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct ToolCall {
                job_id: String,
                #[serde(default)]
                tool: String,
                #[serde(default)]
                input: Value,
            }
            let t: ToolCall = serde_json::from_value(data).ok()?;
            Some(AgentEvent::Research(DeepResearchEvent::ToolCall {
                job_id: t.job_id,
                tool: t.tool,
                input: t.input,
            }))
        }
        "research.textDelta" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct TextDelta {
                job_id: String,
                #[serde(default)]
                text: String,
            }
            let t: TextDelta = serde_json::from_value(data).ok()?;
            Some(AgentEvent::Research(DeepResearchEvent::TextDelta {
                job_id: t.job_id,
                text: t.text,
            }))
        }
        "research.completed" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Completed {
                job_id: String,
                #[serde(default)]
                report: String,
                #[serde(default)]
                citations: Vec<WireCitation>,
                #[serde(default)]
                total_ms: u64,
                #[serde(default)]
                turns: u32,
            }
            #[derive(Deserialize)]
            struct WireCitation {
                #[serde(default)]
                title: String,
                url: String,
                #[serde(default)]
                snippet: Option<String>,
            }
            let c: Completed = serde_json::from_value(data).ok()?;
            let citations = c
                .citations
                .into_iter()
                .enumerate()
                .map(|(i, w)| Citation {
                    id: format!("cit_{}", i + 1),
                    title: if w.title.is_empty() {
                        w.url.clone()
                    } else {
                        w.title
                    },
                    url: w.url,
                    snippet: w.snippet,
                })
                .collect();
            Some(AgentEvent::Research(DeepResearchEvent::Completed {
                job_id: c.job_id,
                report: c.report,
                citations,
                total_ms: c.total_ms,
                turns: c.turns,
            }))
        }
        "research.failed" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Failed {
                job_id: String,
                error: crate::jsonl::WireError,
            }
            let f: Failed = serde_json::from_value(data).ok()?;
            Some(AgentEvent::Research(DeepResearchEvent::Failed {
                job_id: f.job_id,
                error: f.error.into_bluey(),
            }))
        }
        "document.request" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct DocRequest {
                request_id: String,
                document_id: String,
            }
            let d: DocRequest = serde_json::from_value(data).ok()?;
            Some(AgentEvent::DocumentRequest {
                request_id: d.request_id,
                document_id: d.document_id,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bluey_core::error::BlueyErrorKind;
    use bluey_core::types::ResearchTool;
    use pretty_assertions::assert_eq;

    fn request() -> DeepResearchRequest {
        DeepResearchRequest {
            job_id: "job-1".into(),
            session_id: None,
            query: "public query".into(),
            goal: "answer the question".into(),
            max_turns: Some(12),
            tools: vec![ResearchTool::ExaSearch, ResearchTool::DocumentRead],
            allowed_document_ids: Some(vec!["doc-1".into()]),
        }
    }

    #[test]
    fn run_params_carry_the_model() {
        let params = research_run_params(&request(), Some("claude-sonnet-5"));
        assert_eq!(params["jobId"], "job-1");
        assert_eq!(params["tools"][0], "exa_search");
        assert_eq!(params["allowedDocumentIds"][0], "doc-1");
        assert_eq!(params["model"], "claude-sonnet-5");
        let params = research_run_params(&request(), None);
        assert!(params.get("model").is_none());
    }

    #[test]
    fn document_response_shapes() {
        let ok = document_response_params("rq1", "doc-1", Ok("the text"));
        assert_eq!(ok["text"], "the text");
        assert!(ok.get("error").is_none());
        let err = document_response_params("rq1", "doc-2", Err("not allowed"));
        assert_eq!(err["error"], "not allowed");
        assert!(err.get("text").is_none());
    }

    #[test]
    fn maps_progress_and_completion() {
        let e = parse_agent_event(
            "research.progress",
            serde_json::json!({ "jobId": "job-1", "message": "searching…" }),
        )
        .unwrap();
        assert_eq!(
            e,
            AgentEvent::Research(DeepResearchEvent::Progress {
                job_id: "job-1".into(),
                message: "searching…".into()
            })
        );

        let e = parse_agent_event(
            "research.completed",
            serde_json::json!({
                "jobId": "job-1", "report": "# Report", "turns": 4, "totalMs": 90000,
                "citations": [ { "title": "One", "url": "https://one.dev", "snippet": "s" },
                                { "title": "", "url": "https://two.dev" } ],
                "usage": { "inputTokens": 1, "outputTokens": 2 }
            }),
        )
        .unwrap();
        let AgentEvent::Research(DeepResearchEvent::Completed {
            citations, turns, ..
        }) = e
        else {
            panic!("expected completed");
        };
        assert_eq!(turns, 4);
        assert_eq!(citations.len(), 2);
        assert_eq!(citations[0].id, "cit_1");
        assert_eq!(citations[1].title, "https://two.dev");
    }

    #[test]
    fn maps_failures_and_document_requests() {
        let e = parse_agent_event(
            "research.failed",
            serde_json::json!({ "jobId": "job-1",
                "error": { "code": "missing_key", "message": "set ANTHROPIC_API_KEY", "kind": "configuration" } }),
        )
        .unwrap();
        let AgentEvent::Research(DeepResearchEvent::Failed { error, .. }) = e else {
            panic!("expected failed");
        };
        assert_eq!(error.kind, BlueyErrorKind::Configuration);

        let e = parse_agent_event(
            "document.request",
            serde_json::json!({ "requestId": "rq1", "documentId": "doc-1" }),
        )
        .unwrap();
        assert_eq!(
            e,
            AgentEvent::DocumentRequest {
                request_id: "rq1".into(),
                document_id: "doc-1".into()
            }
        );

        assert!(parse_agent_event("weird.event", serde_json::json!({})).is_none());
    }
}
