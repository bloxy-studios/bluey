//! HAR import: the capture format from a MITM proxy's export (mitmproxy, Proxyman,
//! Charles), for clients without a base-URL knob — the Antigravity Electron app.

use anyhow::{anyhow, Context};
use base64::Engine;
use bluey_protocols::fingerprints::{
    client_from_user_agent, scrub_capture, Body, Capture, CaptureSource, CapturedRequest,
    CapturedResponse, FingerprintStamp, Header, Provider, SCHEMA_VERSION,
};
use serde_json::Value;

fn headers_of(value: &Value) -> Vec<Header> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|h| {
                    Some(Header::new(
                        h.get("name")?.as_str()?,
                        h.get("value")?.as_str()?.to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn header_value<'a>(headers: &'a [Header], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|h| h.name == name)
        .map(|h| h.value.as_str())
}

fn request_body(request: &Value, headers: &[Header]) -> Body {
    let Some(post) = request.get("postData") else {
        return Body::Empty;
    };
    let text = post.get("text").and_then(Value::as_str).unwrap_or("");
    let mime = post
        .get("mimeType")
        .and_then(Value::as_str)
        .or_else(|| header_value(headers, "content-type"));
    Body::from_bytes(mime, text.as_bytes())
}

fn response_body(response: &Value, headers: &[Header]) -> Body {
    let Some(content) = response.get("content") else {
        return Body::Empty;
    };
    let text = content.get("text").and_then(Value::as_str).unwrap_or("");
    let mime = content
        .get("mimeType")
        .and_then(Value::as_str)
        .or_else(|| header_value(headers, "content-type"));
    if content.get("encoding").and_then(Value::as_str) == Some("base64") {
        match base64::engine::general_purpose::STANDARD.decode(text) {
            Ok(bytes) => Body::from_bytes(mime, &bytes),
            Err(_) => Body::Binary { bytes: text.len() },
        }
    } else {
        Body::from_bytes(mime, text.as_bytes())
    }
}

/// Convert every entry of a HAR document that belongs to `provider` (or every entry
/// with `all_hosts`) into a scrubbed capture.
pub fn import(text: &str, provider: Provider, all_hosts: bool) -> anyhow::Result<Vec<Capture>> {
    let har: Value = serde_json::from_str(text).context("parsing the HAR file")?;
    let entries = har
        .pointer("/log/entries")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("not a HAR file: no log.entries"))?;
    let rules = provider.rules();
    let mut out = Vec::new();
    for entry in entries {
        let request = &entry["request"];
        let Some(url) = request.get("url").and_then(Value::as_str) else {
            continue;
        };
        let host = url::Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(|h| h.to_string()))
            .unwrap_or_default();
        if !all_hosts && !rules.is_provider_host(&host) {
            continue;
        }
        let request_headers = headers_of(&request["headers"]);
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("GET")
            .to_string();
        let response = entry
            .get("response")
            .filter(|r| r.get("status").is_some())
            .map(|r| {
                let headers = headers_of(&r["headers"]);
                CapturedResponse {
                    status: r["status"].as_u64().unwrap_or(0) as u16,
                    body: response_body(r, &headers),
                    headers,
                    duration_ms: entry
                        .get("time")
                        .and_then(Value::as_f64)
                        .map(|t| t.max(0.0) as u64),
                    truncated: false,
                }
            });
        let captured_at = entry
            .get("startedDateTime")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(crate::now_rfc3339);
        let mut capture = Capture {
            schema: SCHEMA_VERSION,
            provider: provider.id().to_string(),
            source: CaptureSource::Har,
            captured_at,
            fingerprint: FingerprintStamp {
                version: rules.info.version.to_string(),
                captured_on: rules.info.captured_on.to_string(),
            },
            client: header_value(&request_headers, "user-agent").and_then(client_from_user_agent),
            request: CapturedRequest {
                method,
                url: url.to_string(),
                body: request_body(request, &request_headers),
                headers: request_headers,
            },
            response,
            scrubbed: Vec::new(),
            notes: vec!["imported from a HAR export".to_string()],
        };
        scrub_capture(&mut capture, rules);
        out.push(capture);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn imports_provider_entries_and_scrubs_them() {
        let har = json!({ "log": { "entries": [
            {
                "startedDateTime": "2026-09-11T10:00:00.000Z",
                "time": 812.5,
                "request": {
                    "method": "POST",
                    "url": "https://daily-cloudcode-pa.googleapis.com/v1internal:streamGenerateContent?alt=sse",
                    "headers": [
                        { "name": "Authorization", "value": "Bearer ya29.a0AfH6SMBsecretsecret" },
                        { "name": "User-Agent", "value": "antigravity/hub/2.13.0 darwin/arm64" },
                        { "name": "Content-Type", "value": "application/json" }
                    ],
                    "postData": { "mimeType": "application/json", "text": "{\"model\":\"gemini-3.8-flash-high\",\"project\":\"bluey-owner-4f2a\",\"request\":{\"contents\":[{\"role\":\"user\",\"parts\":[{\"text\":\"hello there\"}]}],\"sessionId\":\"abc\"},\"userAgent\":\"antigravity\",\"requestType\":\"agent\",\"requestId\":\"agent-6f1d2c3b-4a5e-4f60-9a1b-2c3d4e5f6a7b\"}" }
                },
                "response": {
                    "status": 200,
                    "headers": [ { "name": "Content-Type", "value": "text/event-stream" } ],
                    "content": { "mimeType": "text/event-stream", "text": "data: {\"response\":{\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hi\"}]}}]},\"traceId\":\"abc\"}\n\n" }
                }
            },
            { "request": { "method": "GET", "url": "https://example.com/", "headers": [] }, "response": { "status": 200, "headers": [], "content": { "text": "" } } }
        ] } });
        let captures = import(&har.to_string(), Provider::Antigravity, false).unwrap();
        assert_eq!(captures.len(), 1, "only provider hosts are imported");
        let c = &captures[0];
        assert_eq!(c.source, CaptureSource::Har);
        assert_eq!(c.client.as_deref(), Some("antigravity/hub/2.13.0"));
        assert_eq!(
            c.request.header("authorization"),
            Some("Bearer <ACCESS_TOKEN>")
        );
        let Body::Json { value } = &c.request.body else {
            panic!("json body")
        };
        assert_eq!(value["project"], json!("<PROJECT_ID>"));
        assert_eq!(value["request"]["sessionId"], json!("<SESSION_ID>"));
        assert_eq!(
            value["request"]["contents"][0]["parts"][0]["text"],
            json!("<TEXT 11>")
        );
        assert_eq!(value["requestId"], json!("agent-<UUID>"));
        let response = c.response.as_ref().unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.duration_ms, Some(812));
        assert!(matches!(&response.body, Body::Sse { events } if events.len() == 1));
        assert_eq!(
            import(&har.to_string(), Provider::Antigravity, true)
                .unwrap()
                .len(),
            2
        );
        assert!(import("{}", Provider::Claude, false).is_err());
    }
}
