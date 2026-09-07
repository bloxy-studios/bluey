//! JSON-Lines envelope shared by the native helper (`bluey-helper`) and the
//! research agent (`bluey-agent`) sidecars:
//!
//! ```jsonc
//! { "id": "r-42", "method": "capture.display", "params": { ... } }   // request
//! { "id": "r-42", "result": { ... } }                                 // success
//! { "id": "r-42", "error": { "code", "message", "kind", "details" } } // failure
//! { "event": "audio.chunk", "data": { ... } }                         // event
//! ```

use bluey_core::error::{BlueyError, BlueyErrorKind, RecoveryAction};
use bluey_core::types::PermissionKind;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Sidecar error payload (`error` field of a response).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireError {
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub details: Option<Value>,
}

impl WireError {
    /// Map onto [`BlueyError`]. `error.kind` picks the Bluey error kind
    /// (`sidecar` when missing/unknown); permission errors keep their
    /// `details.permission` pane for the recovery action.
    pub fn into_bluey(self) -> BlueyError {
        let kind = match self.kind.as_deref() {
            Some("permission") => BlueyErrorKind::Permission,
            Some("capture") => BlueyErrorKind::Capture,
            Some("audio") => BlueyErrorKind::Audio,
            Some("transcription") => BlueyErrorKind::Transcription,
            Some("not_supported") => BlueyErrorKind::NotSupported,
            Some("invalid_params") | Some("internal") => BlueyErrorKind::Internal,
            Some("research") => BlueyErrorKind::Research,
            Some("cancelled") => BlueyErrorKind::Cancelled,
            Some("configuration") => BlueyErrorKind::Configuration,
            Some("network") => BlueyErrorKind::Network,
            _ => BlueyErrorKind::Sidecar,
        };
        let prefix = kind.as_str();
        let code = if self.code.is_empty() {
            format!("{prefix}.unknown")
        } else if self.code.starts_with(&format!("{prefix}.")) {
            self.code.clone()
        } else {
            format!("{prefix}.{}", self.code)
        };
        let mut error = BlueyError::new(kind, code, self.message);
        if kind == BlueyErrorKind::Permission {
            if let Some(pane) = self
                .details
                .as_ref()
                .and_then(|d| d.get("permission"))
                .and_then(|p| serde_json::from_value::<PermissionKind>(p.clone()).ok())
            {
                error = error.recoverable(RecoveryAction::OpenSystemSettings { pane });
            }
        }
        if let Some(details) = self.details {
            error = error.with_details(details);
        }
        error
    }
}

/// One parsed stdout line from a sidecar.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    /// A response correlated by request id.
    Response {
        id: String,
        result: Result<Value, WireError>,
    },
    /// An unsolicited event.
    Event { event: String, data: Value },
}

/// Encode one request line (no trailing newline; the writer appends it).
pub fn encode_request(id: &str, method: &str, params: Value) -> String {
    serde_json::json!({ "id": id, "method": method, "params": params }).to_string()
}

/// Parse one stdout line. Returns `Err` with a short description for
/// non-protocol lines (never echoes the full line, which could contain
/// transcript text).
pub fn parse_line(line: &str) -> Result<Incoming, String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Err("empty line".into());
    }
    let value: Value =
        serde_json::from_str(trimmed).map_err(|e| format!("invalid JSON line: {e}"))?;
    let object = value.as_object().ok_or("line is not a JSON object")?;

    if let Some(event) = object.get("event").and_then(Value::as_str) {
        return Ok(Incoming::Event {
            event: event.to_string(),
            data: object.get("data").cloned().unwrap_or(Value::Null),
        });
    }
    if let Some(id) = object.get("id").and_then(Value::as_str) {
        if let Some(error) = object.get("error") {
            let wire: WireError = serde_json::from_value(error.clone())
                .map_err(|e| format!("malformed error object: {e}"))?;
            return Ok(Incoming::Response {
                id: id.to_string(),
                result: Err(wire),
            });
        }
        return Ok(Incoming::Response {
            id: id.to_string(),
            result: Ok(object.get("result").cloned().unwrap_or(Value::Null)),
        });
    }
    Err("line is neither a response nor an event".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn encodes_requests() {
        let line = encode_request("r-1", "helper.ping", Value::Null);
        let parsed: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed["id"], "r-1");
        assert_eq!(parsed["method"], "helper.ping");
        assert!(parsed["params"].is_null());
        assert!(!line.contains('\n'));
    }

    #[test]
    fn parses_success_and_error_responses() {
        let ok = parse_line(r#"{ "id": "r-42", "result": { "pong": true } }"#).unwrap();
        assert_eq!(
            ok,
            Incoming::Response {
                id: "r-42".into(),
                result: Ok(serde_json::json!({ "pong": true }))
            }
        );

        let err = parse_line(
            r#"{ "id": "r-9", "error": { "code": "permission_denied", "message": "Screen Recording not granted", "kind": "permission", "details": { "permission": "screenRecording" } } }"#,
        )
        .unwrap();
        let Incoming::Response { id, result } = err else {
            panic!("expected response")
        };
        assert_eq!(id, "r-9");
        let bluey = result.unwrap_err().into_bluey();
        assert_eq!(bluey.kind, BlueyErrorKind::Permission);
        assert_eq!(bluey.code, "permission.permission_denied");
        assert!(bluey.recoverable);
        assert_eq!(
            bluey.recovery,
            Some(RecoveryAction::OpenSystemSettings {
                pane: PermissionKind::ScreenRecording
            })
        );
    }

    #[test]
    fn parses_events_and_rejects_garbage() {
        let event = parse_line(
            r#"{ "event": "audio.level", "data": { "microphone": 0.4, "system": 0.1 } }"#,
        )
        .unwrap();
        assert_eq!(
            event,
            Incoming::Event {
                event: "audio.level".into(),
                data: serde_json::json!({ "microphone": 0.4, "system": 0.1 })
            }
        );

        assert!(parse_line("").is_err());
        assert!(parse_line("plain log text").is_err());
        assert!(parse_line("[1,2,3]").is_err());
        assert!(parse_line(r#"{ "neither": true }"#).is_err());
        // A response with a missing result field is Null.
        assert_eq!(
            parse_line(r#"{ "id": "x" }"#).unwrap(),
            Incoming::Response {
                id: "x".into(),
                result: Ok(Value::Null)
            }
        );
    }

    #[test]
    fn wire_error_kind_mapping() {
        let cases = [
            (Some("capture"), BlueyErrorKind::Capture, "capture.failed"),
            (Some("audio"), BlueyErrorKind::Audio, "audio.failed"),
            (
                Some("transcription"),
                BlueyErrorKind::Transcription,
                "transcription.failed",
            ),
            (
                Some("not_supported"),
                BlueyErrorKind::NotSupported,
                "not_supported.failed",
            ),
            (
                Some("internal"),
                BlueyErrorKind::Internal,
                "internal.failed",
            ),
            (None, BlueyErrorKind::Sidecar, "sidecar.failed"),
            (Some("weird"), BlueyErrorKind::Sidecar, "sidecar.failed"),
        ];
        for (kind, expected_kind, expected_code) in cases {
            let e = WireError {
                code: "failed".into(),
                message: "m".into(),
                kind: kind.map(String::from),
                details: None,
            };
            let b = e.into_bluey();
            assert_eq!(b.kind, expected_kind);
            assert_eq!(b.code, expected_code);
        }
        // Codes that already carry the prefix are not double-prefixed.
        let e = WireError {
            code: "capture.timeout".into(),
            message: "m".into(),
            kind: Some("capture".into()),
            details: None,
        };
        assert_eq!(e.into_bluey().code, "capture.timeout");
    }
}
