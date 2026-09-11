//! Provider adapter trait + shared HTTP/error plumbing for the AI manager.

pub mod anthropic;
pub mod azure;
#[cfg(feature = "subscription-accounts")]
pub mod chatgpt;
pub mod gemini;
pub mod mock;
pub mod openai;

use std::collections::HashMap;
use std::sync::OnceLock;

use bluey_core::error::RecoveryAction;
use bluey_core::types::{
    AiMessage, AiProviderConfig, AiProviderKind, AiTask, FinishReason, JsonSchemaSpec,
    LatencyBudget, ModelRole, ProviderModelCatalog, ReasoningLevel,
};
use bluey_core::{BlueyError, BlueyErrorKind, BlueyResult};
use futures::stream::BoxStream;
use tokio_util::sync::CancellationToken;

/// What a text is embedded for (documents get `title:` prefixes, queries
/// `task:` prefixes on `gemini-embedding-2`; other providers ignore it).
pub use bluey_protocols::gemini::EmbedPurpose;
/// Batch-transcription results (`gemini-3.5-transcribe`), reused by the app.
pub use bluey_protocols::gemini::{TranscribedWord, TranscriptTurn, Transcription};

/// A whole recording handed to [`AiProvider::transcribe_audio`].
#[derive(Debug, Clone)]
pub struct AudioFile {
    pub bytes: Vec<u8>,
    /// One of the MIME types from `bluey_protocols::gemini::audio_mime_for_extension`.
    pub mime_type: &'static str,
    /// File name shown in the provider's file store (never a path).
    pub display_name: String,
}

/// Knobs for [`AiProvider::transcribe_audio`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TranscribeFileOptions {
    /// BCP-47 tag; `None` = auto-detect.
    pub language: Option<String>,
    /// Speaker labels (`spk_n`); limits the recording to 30 minutes.
    pub diarization: bool,
    /// Word-level timings; limits the recording to 30 minutes.
    pub word_timestamps: bool,
}

/// A provider-agnostic generation request (already routed to a model).
#[derive(Debug, Clone)]
pub struct ProviderRequest {
    pub model: String,
    pub messages: Vec<AiMessage>,
    pub max_output_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub output_schema: Option<JsonSchemaSpec>,
    pub task: AiTask,
    /// Drives thinking depth on providers that expose it (Gemini `thinkingLevel`).
    pub latency: LatencyBudget,
    pub reasoning: ReasoningLevel,
    /// The Bluey session the request belongs to — subscription providers key
    /// their conversation ids and prompt caches on it.
    pub session_id: Option<String>,
}

/// An OAuth subscription account's credential for one request (ADR 0009): the
/// access token travels here for the duration of the call; adapters never keep it.
#[derive(Clone)]
pub struct OAuthCredential {
    pub access_token: String,
    /// Provider-side account id (Codex `chatgpt_account_id`), when known.
    pub account_id: Option<String>,
    /// The account's cached model catalog (reasoning levels, `list_models`).
    pub catalog: Option<ProviderModelCatalog>,
    /// Stable per-install device id (64 hex) for `metadata.user_id`-style fields.
    pub device_id: String,
}

impl std::fmt::Debug for OAuthCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthCredential")
            .field("access_token", &"<redacted>")
            .field("account_id", &self.account_id)
            .field("catalog", &self.catalog.as_ref().map(|c| c.models.len()))
            .field("device_id", &self.device_id)
            .finish()
    }
}

/// One UUID per Bluey session (subscription providers' `session-id` / `thread-id`
/// headers and prompt-cache keys), a process-wide one for requests without a session.
pub(super) fn conversation_id(session_id: Option<&str>) -> String {
    static PER_SESSION: OnceLock<parking_lot::Mutex<HashMap<String, String>>> = OnceLock::new();
    static PROCESS: OnceLock<String> = OnceLock::new();
    match session_id {
        Some(session) => PER_SESSION
            .get_or_init(Default::default)
            .lock()
            .entry(session.to_string())
            .or_insert_with(|| uuid::Uuid::new_v4().to_string())
            .clone(),
        None => PROCESS
            .get_or_init(|| uuid::Uuid::new_v4().to_string())
            .clone(),
    }
}

/// Error bodies are read up to this size (provider words, never prompts).
pub(super) const ERROR_BODY_LIMIT: usize = 16 * 1024;

pub(super) fn header_pairs(headers: &reqwest::header::HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_string(),
                String::from_utf8_lossy(v.as_bytes()).into_owned(),
            )
        })
        .collect()
}

pub(super) async fn read_limited(response: reqwest::Response) -> String {
    use futures::StreamExt;
    let mut collected: Vec<u8> = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(Ok(chunk)) = stream.next().await {
        let room = ERROR_BODY_LIMIT.saturating_sub(collected.len());
        collected.extend_from_slice(&chunk[..chunk.len().min(room)]);
        if room == 0 {
            break;
        }
    }
    String::from_utf8_lossy(&collected).into_owned()
}

/// What `build_provider` authenticates with.
#[derive(Debug, Clone)]
pub enum ProviderCredential {
    /// The mock, or an API-key provider without a key (→ `config.missing_key`).
    None,
    ApiKey(String),
    OAuth(OAuthCredential),
}

/// One item of a provider stream.
#[derive(Debug, Clone)]
pub enum StreamItem {
    /// A text delta.
    Delta(String),
    /// Token usage (may arrive multiple times; the last one wins).
    Usage {
        input: Option<u32>,
        output: Option<u32>,
    },
    /// The stream finished.
    Finished(FinishReason),
}

pub type ChunkStream = BoxStream<'static, BlueyResult<StreamItem>>;

/// A streaming AI provider adapter.
#[async_trait::async_trait]
pub trait AiProvider: Send + Sync {
    /// Start a streaming generation.
    async fn stream(
        &self,
        request: &ProviderRequest,
        token: CancellationToken,
    ) -> BlueyResult<ChunkStream>;

    /// Embed a batch of texts with `model` for `purpose`.
    async fn embed(
        &self,
        model: &str,
        texts: &[String],
        purpose: &EmbedPurpose,
    ) -> BlueyResult<Vec<Vec<f32>>>;

    /// List the models this provider can serve, optionally only those fit for `role`.
    async fn list_models(&self, role: Option<ModelRole>) -> BlueyResult<Vec<String>>;

    /// Transcribe a whole recording with `model` in one call (batch
    /// speech-to-text). Providers without such a model return `not_supported`.
    async fn transcribe_audio(
        &self,
        _model: &str,
        _audio: AudioFile,
        _options: &TranscribeFileOptions,
    ) -> BlueyResult<Transcription> {
        Err(BlueyError::not_supported(
            "transcribe_file",
            "this provider cannot transcribe recordings; assign the transcription role to Google Gemini",
        ))
    }
}

/// The ChatGPT adapter — real in builds with the `subscription-accounts` feature.
#[cfg(feature = "subscription-accounts")]
fn chatgpt_provider(
    credential: ProviderCredential,
    http: reqwest::Client,
) -> BlueyResult<Box<dyn AiProvider>> {
    match credential {
        ProviderCredential::OAuth(oauth) => {
            Ok(Box::new(chatgpt::ChatgptProvider::new(http, oauth)))
        }
        _ => Err(BlueyError::account(
            "not_connected",
            "connect the ChatGPT account in Settings → AI → Accounts first",
        )
        .recoverable(RecoveryAction::reconnect_account(
            bluey_core::types::CHATGPT_PROVIDER_ID,
            bluey_core::types::CHATGPT_PROVIDER_ID,
        ))),
    }
}

#[cfg(not(feature = "subscription-accounts"))]
fn chatgpt_provider(
    _credential: ProviderCredential,
    _http: reqwest::Client,
) -> BlueyResult<Box<dyn AiProvider>> {
    Err(BlueyError::account(
        "disabled",
        "this build of Bluey was made without subscription accounts",
    )
    .recoverable(RecoveryAction::UseApiKey))
}

/// The Claude Pro/Max adapter — the Anthropic adapter in OAuth mode (feature builds).
#[cfg(feature = "subscription-accounts")]
fn claude_provider(
    credential: ProviderCredential,
    http: reqwest::Client,
) -> BlueyResult<Box<dyn AiProvider>> {
    match credential {
        ProviderCredential::OAuth(oauth) => {
            Ok(Box::new(anthropic::AnthropicProvider::oauth(http, oauth)))
        }
        _ => Err(BlueyError::account(
            "not_connected",
            "connect the Claude account in Settings → AI → Accounts first",
        )
        .recoverable(RecoveryAction::reconnect_account(
            bluey_core::types::CLAUDE_PROVIDER_ID,
            bluey_core::types::CLAUDE_PROVIDER_ID,
        ))),
    }
}

#[cfg(not(feature = "subscription-accounts"))]
fn claude_provider(
    _credential: ProviderCredential,
    _http: reqwest::Client,
) -> BlueyResult<Box<dyn AiProvider>> {
    Err(BlueyError::account(
        "disabled",
        "this build of Bluey was made without subscription accounts",
    )
    .recoverable(RecoveryAction::UseApiKey))
}

/// Build the adapter for a provider config. `embedding_dimensions` is the
/// configured MRL size for Gemini embeddings.
pub fn build_provider(
    config: &AiProviderConfig,
    credential: ProviderCredential,
    http: reqwest::Client,
    dev: std::sync::Arc<crate::state::DevState>,
    embedding_dimensions: u32,
) -> BlueyResult<Box<dyn AiProvider>> {
    match config.kind {
        AiProviderKind::Mock => Ok(Box::new(mock::MockProvider::new(dev))),
        // Served by a subscription account (ADR 0009).
        AiProviderKind::ChatgptCodex => chatgpt_provider(credential, http),
        AiProviderKind::ClaudeSubscription => claude_provider(credential, http),
        // The Google adapter lands in PR 3c. Until then the router's fallback
        // chain reaches the API-key providers.
        AiProviderKind::AntigravityGoogle => Err(BlueyError::account(
            "provider_pending",
            "this provider is served by a subscription account, which this version cannot route yet — use an API-key provider for now",
        )
        .recoverable(RecoveryAction::UseApiKey)),
        kind => {
            let api_key = match credential {
                ProviderCredential::ApiKey(key) => key,
                _ => {
                    return Err(BlueyError::configuration(
                        "missing_key",
                        "the provider has no API key configured",
                    )
                    .recoverable(RecoveryAction::ConfigureProvider))
                }
            };
            match kind {
                AiProviderKind::GoogleGemini => Ok(Box::new(gemini::GeminiProvider::new(
                    http,
                    config.base_url.clone(),
                    api_key,
                    embedding_dimensions,
                ))),
                AiProviderKind::AzureFoundry => Ok(Box::new(azure::AzureProvider::new(
                    http,
                    config.base_url.clone(),
                    config.api_version.clone(),
                    config.deployments.clone(),
                    api_key,
                ))),
                AiProviderKind::Anthropic => Ok(Box::new(anthropic::AnthropicProvider::new(
                    http,
                    config.base_url.clone(),
                    api_key,
                ))),
                AiProviderKind::OpenaiCompatible => Ok(Box::new(openai::OpenAiProvider::new(
                    http,
                    config.base_url.clone(),
                    api_key,
                ))),
                AiProviderKind::Mock
                | AiProviderKind::ChatgptCodex
                | AiProviderKind::ClaudeSubscription
                | AiProviderKind::AntigravityGoogle => {
                    unreachable!("handled above")
                }
            }
        }
    }
}

/// Map an HTTP status onto the contract errors: 401/403 → configuration with
/// `ConfigureProvider` recovery; 429 → network with `Retry`; everything else →
/// `ai.http_<status>`. Bodies are never included (they can echo prompts).
pub fn map_http_status(status: u16, provider_hint: &str) -> BlueyError {
    match status {
        401 | 403 => BlueyError::new(
            BlueyErrorKind::Configuration,
            format!("config.http_{status}"),
            format!("the {provider_hint} API rejected the credentials (HTTP {status})"),
        )
        .recoverable(RecoveryAction::ConfigureProvider),
        429 => BlueyError::network(
            "http_429",
            format!("the {provider_hint} API rate-limited the request (HTTP 429)"),
        ),
        _ => BlueyError::ai(
            &format!("http_{status}"),
            format!("the {provider_hint} API returned HTTP {status}"),
        ),
    }
}

/// Map a transport error (timeouts/connects → network+Retry).
pub fn map_transport_error(error: &reqwest::Error, provider_hint: &str) -> BlueyError {
    if error.is_timeout() {
        BlueyError::network("timeout", format!("the {provider_hint} request timed out"))
    } else if error.is_connect() {
        BlueyError::network(
            "connect",
            format!("could not connect to the {provider_hint} API"),
        )
    } else {
        BlueyError::network(
            "request",
            format!("the {provider_hint} request failed to complete"),
        )
    }
}

/// Bridge an inner producer task to a [`ChunkStream`]: the producer writes into
/// the sender; dropping the stream stops the producer on its next send.
pub fn channel_stream() -> (
    tokio::sync::mpsc::Sender<BlueyResult<StreamItem>>,
    ChunkStream,
) {
    let (tx, rx) = tokio::sync::mpsc::channel::<BlueyResult<StreamItem>>(64);
    let stream = futures::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|item| (item, rx))
    });
    (tx, Box::pin(stream))
}

/// Collect a stream into plain text (test connections, embeddings fallbacks).
pub async fn collect_text(mut stream: ChunkStream) -> BlueyResult<String> {
    use futures::StreamExt;
    let mut text = String::new();
    while let Some(item) = stream.next().await {
        match item? {
            StreamItem::Delta(delta) => text.push_str(&delta),
            StreamItem::Finished(_) => break,
            StreamItem::Usage { .. } => {}
        }
    }
    Ok(text)
}
