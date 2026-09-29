//! Error answers of the API-key HTTP APIs that share one error envelope —
//! OpenAI-compatible and Azure Foundry (`{"error":{"code","message",…}}`) and
//! Anthropic (`{"type":"error","error":{"type","message"}}`) — mapped onto the
//! contract error codes the WebView words its copy from.
//!
//! Bodies can echo the request (prompts, screen text), so only the error's
//! code and a capped first line of its message are kept, and neither is logged.

use std::time::Duration;

use bluey_core::error::{BlueyError, BlueyErrorKind, RecoveryAction};
use serde_json::{json, Map, Value};

/// The provider's reason is quoted to the user up to this many characters.
const MESSAGE_LIMIT: usize = 300;

/// Error codes / types that mean "this model or deployment does not exist here".
const NOT_FOUND_CODES: [&str; 4] = [
    "DeploymentNotFound",
    "model_not_found",
    "not_found_error",
    "NotFound",
];

/// The parts of an error body Bluey keeps.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApiErrorBody {
    /// `error.code` (OpenAI, Azure) or `error.type` (Anthropic).
    pub code: Option<String>,
    /// The first line of `error.message`, capped at [`MESSAGE_LIMIT`].
    pub message: Option<String>,
}

impl ApiErrorBody {
    fn is_not_found(&self) -> bool {
        self.code
            .as_deref()
            .is_some_and(|code| NOT_FOUND_CODES.contains(&code))
    }
}

/// Parse an error body; `None` when it is not the shared envelope.
pub fn parse_error_body(body: &str) -> Option<ApiErrorBody> {
    let value: Value = serde_json::from_str(body).ok()?;
    let error = value.get("error")?.as_object()?;
    let text = |key: &str| {
        error
            .get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
    };
    let code = text("code").or_else(|| text("type")).map(str::to_string);
    let message = text("message").map(|message| {
        let line = message.lines().next().unwrap_or_default().trim();
        match line.char_indices().nth(MESSAGE_LIMIT) {
            Some((cut, _)) => format!("{}…", &line[..cut]),
            None => line.to_string(),
        }
    });
    Some(ApiErrorBody { code, message })
}

/// The server's retry hint: `retry-after-ms`, `retry-after` (seconds) or
/// OpenAI's `x-ratelimit-reset-requests` (`"1s"`, `"6m0s"`, `"20ms"`).
pub fn retry_after(headers: &[(String, String)]) -> Option<Duration> {
    let header = |name: &str| {
        headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.trim())
    };
    let millis = |ms: f64| (ms.is_finite() && ms >= 0.0).then(|| Duration::from_millis(ms as u64));
    header("retry-after-ms")
        .and_then(|v| v.parse::<f64>().ok())
        .and_then(millis)
        .or_else(|| {
            header("retry-after")
                .and_then(|v| v.parse::<f64>().ok())
                .and_then(|secs| millis(secs * 1000.0))
        })
        .or_else(|| header("x-ratelimit-reset-requests").and_then(parse_reset))
}

/// `"6m0s"` / `"1.5s"` / `"20ms"` → a duration.
fn parse_reset(value: &str) -> Option<Duration> {
    let mut total_ms = 0.0_f64;
    let mut rest = value;
    while !rest.is_empty() {
        let digits = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(rest.len());
        let number: f64 = rest[..digits].parse().ok()?;
        rest = &rest[digits..];
        let unit_len = rest
            .find(|c: char| c.is_ascii_digit())
            .unwrap_or(rest.len());
        let factor = match &rest[..unit_len] {
            "ms" => 1.0,
            "s" => 1000.0,
            "m" => 60_000.0,
            "h" => 3_600_000.0,
            _ => return None,
        };
        total_ms += number * factor;
        rest = &rest[unit_len..];
    }
    (total_ms > 0.0).then(|| Duration::from_millis(total_ms as u64))
}

/// Map a non-2xx answer onto the contract error. `provider` names the API in
/// the message ("Azure Foundry"); `model` is the model or deployment asked for.
pub fn map_api_error(
    status: u16,
    body: Option<&ApiErrorBody>,
    retry_after: Option<Duration>,
    provider: &str,
    model: &str,
) -> BlueyError {
    let reason = body.and_then(|b| b.message.as_deref());
    let quoted = |fallback: String| match reason {
        Some(reason) => format!("{provider} rejected the request: {reason}"),
        None => fallback,
    };
    match status {
        404 => not_found(provider, model),
        _ if body.is_some_and(ApiErrorBody::is_not_found) => not_found(provider, model),
        400 | 422 => BlueyError::ai(
            "invalid_request",
            quoted(format!(
                "the {provider} API rejected the request (HTTP {status}) — check the model and its options"
            )),
        ),
        401 | 403 => BlueyError::new(
            BlueyErrorKind::Configuration,
            format!("config.http_{status}"),
            format!("the {provider} API rejected the credentials (HTTP {status})"),
        )
        .recoverable(RecoveryAction::ConfigureProvider),
        429 => {
            let mut details = Map::new();
            if let Some(delay) = retry_after {
                details.insert("retryAfterMs".into(), json!(delay.as_millis() as u64));
            }
            BlueyError::network(
                "http_429",
                format!("the {provider} API rate-limited the request (HTTP 429)"),
            )
            .with_details(Value::Object(details))
        }
        500..=599 => BlueyError::network(
            "http_5xx",
            format!("the {provider} API is unavailable (HTTP {status}) — try again"),
        ),
        other => BlueyError::ai(
            &format!("http_{other}"),
            quoted(format!("the {provider} API returned HTTP {other}")),
        ),
    }
}

fn not_found(provider: &str, model: &str) -> BlueyError {
    BlueyError::new(
        BlueyErrorKind::Configuration,
        "config.model_not_found",
        format!("{provider} has no model or deployment named \"{model}\""),
    )
    .recoverable(RecoveryAction::ConfigureProvider)
    .with_details(json!({ "model": model, "provider": provider }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn keeps_only_the_code_and_a_capped_first_line_of_the_message() {
        let azure = r#"{"error":{"code":"DeploymentNotFound","message":"The API deployment for this resource does not exist.\nsecret prompt echo"}}"#;
        let parsed = parse_error_body(azure).unwrap();
        assert_eq!(parsed.code.as_deref(), Some("DeploymentNotFound"));
        assert_eq!(
            parsed.message.as_deref(),
            Some("The API deployment for this resource does not exist.")
        );
        let anthropic =
            r#"{"type":"error","error":{"type":"not_found_error","message":"model: claude-x"}}"#;
        assert_eq!(
            parse_error_body(anthropic).unwrap().code.as_deref(),
            Some("not_found_error")
        );
        let long = format!(r#"{{"error":{{"message":"{}"}}}}"#, "é".repeat(400));
        let message = parse_error_body(&long).unwrap().message.unwrap();
        assert_eq!(
            message.chars().count(),
            MESSAGE_LIMIT + 1,
            "capped + ellipsis"
        );
        assert!(parse_error_body("<html>502</html>").is_none());
    }

    #[test]
    fn a_missing_deployment_or_model_is_a_configuration_error_naming_it() {
        let body = parse_error_body(r#"{"error":{"code":"DeploymentNotFound","message":"x"}}"#);
        let error = map_api_error(404, body.as_ref(), None, "Azure Foundry", "gpt-5.5");
        assert_eq!(error.code, "config.model_not_found");
        assert_eq!(error.recovery, Some(RecoveryAction::ConfigureProvider));
        assert_eq!(error.details.unwrap()["model"], "gpt-5.5");
        // OpenAI answers an unknown model with a 400 + model_not_found on some gateways.
        let body = parse_error_body(r#"{"error":{"code":"model_not_found","message":"x"}}"#);
        let error = map_api_error(400, body.as_ref(), None, "provider", "m");
        assert_eq!(error.code, "config.model_not_found");
    }

    #[test]
    fn a_400_quotes_the_provider_reason() {
        let body = parse_error_body(
            r#"{"error":{"code":"unsupported_parameter","message":"Unsupported parameter: 'temperature'."}}"#,
        );
        let error = map_api_error(400, body.as_ref(), None, "Azure Foundry", "gpt-5.5");
        assert_eq!(error.code, "ai.invalid_request");
        assert_eq!(
            error.message,
            "Azure Foundry rejected the request: Unsupported parameter: 'temperature'."
        );
    }

    #[test]
    fn a_429_carries_the_server_delay_and_5xx_is_retryable() {
        let delay = retry_after(&headers(&[("Retry-After", "7")]));
        let error = map_api_error(429, None, delay, "Anthropic", "m");
        assert_eq!(error.code, "network.http_429");
        assert_eq!(error.details.unwrap()["retryAfterMs"], 7000);
        assert_eq!(error.recovery, Some(RecoveryAction::Retry));
        let error = map_api_error(529, None, None, "Anthropic", "m");
        assert_eq!(error.code, "network.http_5xx");
        assert_eq!(
            map_api_error(401, None, None, "p", "m").code,
            "config.http_401"
        );
    }

    #[test]
    fn reads_every_retry_hint_format() {
        let ms = retry_after(&headers(&[
            ("retry-after-ms", "1500"),
            ("retry-after", "9"),
        ]));
        assert_eq!(ms, Some(Duration::from_millis(1500)), "ms wins");
        let reset = headers(&[("x-ratelimit-reset-requests", "6m0s")]);
        assert_eq!(retry_after(&reset), Some(Duration::from_secs(360)));
        let reset = headers(&[("x-ratelimit-reset-requests", "20ms")]);
        assert_eq!(retry_after(&reset), Some(Duration::from_millis(20)));
        let date = headers(&[("retry-after", "Wed, 21 Oct 2026 07:28:00 GMT")]);
        assert_eq!(retry_after(&date), None);
    }
}
