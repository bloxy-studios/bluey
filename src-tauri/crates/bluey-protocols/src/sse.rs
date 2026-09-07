//! Incremental Server-Sent-Events frame parser.
//!
//! Feed arbitrary UTF-8 chunks with [`SseParser::push`]; complete frames come
//! back in order. Handles CRLF/CR line endings, multi-line `data:` fields
//! (joined with `\n`), `event:` names, `id:`/`retry:` (ignored) and `:` comment
//! lines. A frame is dispatched on the first blank line, per the SSE spec.

/// One dispatched SSE frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseFrame {
    /// The `event:` field, if any (`None` = default `message` event).
    pub event: Option<String>,
    /// All `data:` lines joined with `\n`.
    pub data: String,
}

impl SseFrame {
    /// Whether the frame is an OpenAI-style stream terminator (`data: [DONE]`).
    pub fn is_done(&self) -> bool {
        is_done(&self.data)
    }
}

/// Whether an SSE `data` payload is the OpenAI `[DONE]` terminator.
pub fn is_done(data: &str) -> bool {
    data.trim() == "[DONE]"
}

/// Incremental SSE parser. Owns a partial-line buffer between pushes.
#[derive(Debug, Default)]
pub struct SseParser {
    partial_line: String,
    event: Option<String>,
    data_lines: Vec<String>,
    has_data: bool,
}

impl SseParser {
    /// New empty parser.
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed the next chunk of the byte stream (already UTF-8 decoded). Returns
    /// every frame completed by this chunk, in order.
    pub fn push(&mut self, chunk: &str) -> Vec<SseFrame> {
        let mut frames = Vec::new();
        for ch in chunk.chars() {
            match ch {
                '\n' => {
                    let line = std::mem::take(&mut self.partial_line);
                    if let Some(frame) = self.push_line(&line) {
                        frames.push(frame);
                    }
                }
                '\r' => { /* swallowed; a following \n produces the newline */ }
                _ => self.partial_line.push(ch),
            }
        }
        frames
    }

    /// Handle one complete line. A blank line dispatches the pending frame.
    fn push_line(&mut self, line: &str) -> Option<SseFrame> {
        if line.is_empty() {
            return self.dispatch();
        }
        if line.starts_with(':') {
            return None; // comment
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        match field {
            "data" => {
                self.data_lines.push(value.to_string());
                self.has_data = true;
            }
            "event" => self.event = Some(value.to_string()),
            // `id` and `retry` are irrelevant for our uses.
            _ => {}
        }
        None
    }

    fn dispatch(&mut self) -> Option<SseFrame> {
        if !self.has_data && self.event.is_none() {
            return None;
        }
        let frame = SseFrame {
            event: self.event.take(),
            data: std::mem::take(&mut self.data_lines).join("\n"),
        };
        self.has_data = false;
        Some(frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn frame(event: Option<&str>, data: &str) -> SseFrame {
        SseFrame {
            event: event.map(String::from),
            data: data.to_string(),
        }
    }

    #[test]
    fn parses_simple_data_frames() {
        let mut p = SseParser::new();
        let frames = p.push("data: {\"a\":1}\n\ndata: {\"b\":2}\n\n");
        assert_eq!(
            frames,
            vec![frame(None, "{\"a\":1}"), frame(None, "{\"b\":2}")]
        );
    }

    #[test]
    fn handles_split_chunks_and_crlf() {
        let mut p = SseParser::new();
        assert!(p.push("da").is_empty());
        assert!(p.push("ta: hel").is_empty());
        let frames = p.push("lo\r\n\r\n");
        assert_eq!(frames, vec![frame(None, "hello")]);
    }

    #[test]
    fn joins_multi_line_data_and_reads_event_names() {
        let mut p = SseParser::new();
        let frames = p.push("event: message_start\ndata: line1\ndata: line2\n\n");
        assert_eq!(frames, vec![frame(Some("message_start"), "line1\nline2")]);
    }

    #[test]
    fn ignores_comments_ids_and_retry() {
        let mut p = SseParser::new();
        let frames = p.push(": keepalive\nid: 42\nretry: 100\ndata: x\n\n");
        assert_eq!(frames, vec![frame(None, "x")]);
        // A comment-only block dispatches nothing.
        assert!(p.push(": ping\n\n").is_empty());
    }

    #[test]
    fn detects_done() {
        let mut p = SseParser::new();
        let frames = p.push("data: [DONE]\n\n");
        assert_eq!(frames.len(), 1);
        assert!(frames[0].is_done());
        assert!(is_done(" [DONE] "));
        assert!(!is_done("{\"x\":1}"));
    }

    #[test]
    fn data_without_space_after_colon() {
        let mut p = SseParser::new();
        let frames = p.push("data:tight\n\n");
        assert_eq!(frames, vec![frame(None, "tight")]);
    }
}
