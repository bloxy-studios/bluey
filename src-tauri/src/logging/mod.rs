//! Logging: JSON lines into `~/Library/Logs/Bluey/bluey-YYYY-MM-DD.log`
//! (+ stderr in debug builds), a reloadable level filter, a redaction pass
//! that masks API keys/tokens, and a layer that mirrors warnings/errors onto
//! the bus as `dev.log`.
//!
//! Transcript text, OCR output and screenshots are never logged by policy —
//! callers must not put them in log fields; this module additionally masks
//! secret-shaped strings as a second line of defence.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use bluey_core::events::BlueyEvent;
use bluey_core::now_iso;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{reload, EnvFilter, Layer, Registry};

use crate::events::EventBus;

type ReloadHandle = reload::Handle<EnvFilter, Registry>;

/// Handle to the initialised logging stack.
pub struct Logging {
    reload: ReloadHandle,
}

/// Bus used by the `dev.log` mirror layer; set once at bootstrap.
static LOG_BUS: OnceLock<Arc<EventBus>> = OnceLock::new();

impl Logging {
    /// Initialise tracing. `level` is the initial level (the settings value or
    /// the `BLUEY_LOG_LEVEL`/`RUST_LOG` overrides, resolved by the caller).
    pub fn init(logs_dir: PathBuf, level: &str) -> Self {
        let (filter, reload) = reload::Layer::new(build_filter(level));
        let sink = Arc::new(FileSink::new(logs_dir));
        let file_layer = tracing_subscriber::fmt::layer()
            .json()
            .with_ansi(false)
            .with_writer(MakeFileWriter { sink });

        let stderr_layer = if cfg!(debug_assertions) {
            Some(
                tracing_subscriber::fmt::layer()
                    .compact()
                    .with_writer(std::io::stderr),
            )
        } else {
            None
        };

        let registry = tracing_subscriber::registry()
            .with(filter)
            .with(file_layer)
            .with(stderr_layer)
            .with(BusLayer);
        // Ignore double-init in tests.
        let _ = registry.try_init();
        Self { reload }
    }

    /// Change the level filter at runtime (settings → advanced.logLevel).
    pub fn set_level(&self, level: &str) {
        if let Err(e) = self.reload.reload(build_filter(level)) {
            tracing::warn!(error = %e, "failed to reload log filter");
        }
    }

    /// Connect the bus so WARN/ERROR records mirror to `dev.log`.
    pub fn connect_bus(bus: Arc<EventBus>) {
        let _ = LOG_BUS.set(bus);
    }
}

/// Resolve the effective initial level: `RUST_LOG` > `BLUEY_LOG_LEVEL` >
/// the passed settings level.
pub fn effective_level(settings_level: &str) -> String {
    if let Ok(v) = std::env::var("RUST_LOG") {
        if !v.is_empty() {
            return v;
        }
    }
    if let Ok(v) = std::env::var("BLUEY_LOG_LEVEL") {
        if !v.is_empty() {
            return v;
        }
    }
    settings_level.to_string()
}

fn build_filter(level: &str) -> EnvFilter {
    // Quieten chatty dependencies regardless of the app level.
    let directives = format!(
        "{level},hyper=warn,hyper_util=warn,reqwest=warn,tungstenite=warn,tokio_tungstenite=warn,tao=warn,wry=warn,rustls=warn"
    );
    EnvFilter::try_new(directives).unwrap_or_else(|_| EnvFilter::new("info"))
}

// ── Secret redaction ─────────────────────────────────────────────────────────

/// Mask secret-shaped substrings: `sk-…`, `fc-…`, `AIza…` (Google API keys),
/// `Bearer …`, `api-key: …` / `"api-key":"…"` values and `key=` URL queries
/// (the Gemini Live WebSocket URL).
pub fn redact(line: &str) -> String {
    static PATTERNS: OnceLock<Vec<(regex::Regex, &'static str)>> = OnceLock::new();
    let patterns = PATTERNS.get_or_init(|| {
        vec![
            (
                regex::Regex::new(r"sk-[A-Za-z0-9_\-]{8,}").expect("valid regex"),
                "[redacted]",
            ),
            (
                regex::Regex::new(r"fc-[A-Za-z0-9_\-]{8,}").expect("valid regex"),
                "[redacted]",
            ),
            (
                regex::Regex::new(r"AIza[0-9A-Za-z_\-]{30,}").expect("valid regex"),
                "[redacted]",
            ),
            (
                regex::Regex::new(r"(?i)bearer\s+[A-Za-z0-9._\-]{8,}").expect("valid regex"),
                "[redacted]",
            ),
            (
                regex::Regex::new(r#"(?i)(api-key["':\s=]+)[A-Za-z0-9._\-]{8,}"#)
                    .expect("valid regex"),
                "$1[redacted]",
            ),
            (
                regex::Regex::new(r#"(?i)([?&]key=)[^&\s"']+"#).expect("valid regex"),
                "$1[redacted]",
            ),
            // Subscription accounts (ADR 0009): Google access / refresh tokens and the
            // ChatGPT account id header. `sk-ant-oat…` / `sk-ant-ort…` fall under `sk-` above.
            (
                regex::Regex::new(r"ya29\.[A-Za-z0-9._\-]{8,}").expect("valid regex"),
                "[redacted]",
            ),
            (
                regex::Regex::new(r"1//[A-Za-z0-9._\-]{8,}").expect("valid regex"),
                "[redacted]",
            ),
            (
                regex::Regex::new(r#"(?i)(chatgpt-account-id["':\s=]+)[A-Za-z0-9._\-]{8,}"#)
                    .expect("valid regex"),
                "$1[redacted]",
            ),
        ]
    });
    let mut out = line.to_string();
    for (pattern, replacement) in patterns {
        out = pattern.replace_all(&out, *replacement).into_owned();
    }
    out
}

// ── Daily JSON file sink ─────────────────────────────────────────────────────

struct FileSink {
    dir: PathBuf,
    state: parking_lot::Mutex<Option<(String, File)>>,
}

impl FileSink {
    fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            state: parking_lot::Mutex::new(None),
        }
    }

    fn write_line(&self, bytes: &[u8]) {
        let day = now_iso().chars().take(10).collect::<String>();
        let mut state = self.state.lock();
        let reopen = match &*state {
            Some((current, _)) => current != &day,
            None => true,
        };
        if reopen {
            let path = self.dir.join(format!("bluey-{day}.log"));
            match OpenOptions::new().create(true).append(true).open(&path) {
                Ok(file) => *state = Some((day, file)),
                Err(_) => return, // never panic from the logger
            }
        }
        if let Some((_, file)) = state.as_mut() {
            let text = String::from_utf8_lossy(bytes);
            let redacted = redact(text.trim_end());
            let _ = writeln!(file, "{redacted}");
        }
    }
}

struct MakeFileWriter {
    sink: Arc<FileSink>,
}

struct EventWriter {
    buf: Vec<u8>,
    sink: Arc<FileSink>,
}

impl Write for EventWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for EventWriter {
    fn drop(&mut self) {
        if !self.buf.is_empty() {
            self.sink.write_line(&self.buf);
        }
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for MakeFileWriter {
    type Writer = EventWriter;
    fn make_writer(&'a self) -> Self::Writer {
        EventWriter {
            buf: Vec::with_capacity(256),
            sink: self.sink.clone(),
        }
    }
}

// ── dev.log mirror layer ─────────────────────────────────────────────────────

struct BusLayer;

impl<S: tracing::Subscriber> Layer<S> for BusLayer {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let level = *event.metadata().level();
        if level > tracing::Level::WARN {
            return;
        }
        let target = event.metadata().target();
        // Never mirror the forwarder itself (recursion guard).
        if target.contains("::events") {
            return;
        }
        let Some(bus) = LOG_BUS.get() else { return };
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        bus.publish(BlueyEvent::DevLog {
            level: level.to_string().to_lowercase(),
            target: target.to_string(),
            message: redact(&visitor.message),
            at: now_iso(),
        });
    }
}

#[derive(Default)]
struct MessageVisitor {
    message: String,
}

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        } else {
            use std::fmt::Write as _;
            let _ = write!(self.message, " {}={value:?}", field.name());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_key_shapes() {
        let line = r#"auth sk-abcdef123456789 and fc-ZZZZ99999999 Bearer eyJhbGciOi.abc-def "api-key":"abc123456789""#;
        let out = redact(line);
        assert!(!out.contains("sk-abcdef123456789"));
        assert!(!out.contains("fc-ZZZZ99999999"));
        assert!(!out.contains("eyJhbGciOi.abc-def"));
        assert!(!out.contains("abc123456789"));
        assert!(out.contains("[redacted]"));
    }

    #[test]
    fn redacts_subscription_account_tokens() {
        let line = r#"tokens sk-ant-oat01-AbCdEfGh12345 sk-ant-ort01-ZyXwVu98765 ya29.a0AfH6SMBxyz-123 1//0gabcdefGHIJKL "chatgpt-account-id":"1f2e3d4c-5b6a-4789-9abc-def012345678""#;
        let out = redact(line);
        for secret in [
            "sk-ant-oat01-AbCdEfGh12345",
            "sk-ant-ort01-ZyXwVu98765",
            "ya29.a0AfH6SMBxyz-123",
            "1//0gabcdefGHIJKL",
            "1f2e3d4c-5b6a-4789-9abc-def012345678",
        ] {
            assert!(!out.contains(secret), "{secret} leaked: {out}");
        }
        assert!(out.contains(r#""chatgpt-account-id":"[redacted]"#));
    }
}
