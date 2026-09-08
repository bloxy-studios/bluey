//! Provider adapter trait + shared HTTP/error plumbing for the AI manager.

pub mod anthropic;
pub mod azure;
pub mod gemini;
pub mod mock;
pub mod openai;

use bluey_core::error::RecoveryAction;
use bluey_core::types::{
    AiMessage, AiProviderConfig, AiProviderKind, AiTask, FinishReason, JsonSchemaSpec,
    LatencyBudget, ModelRole, ReasoningLevel,
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

/// Build the adapter for a provider config. `api_key` is `None` only for mock.
/// `embedding_dimensions` is the configured MRL size for Gemini embeddings.
pub fn build_provider(
    config: &AiProviderConfig,
    api_key: Option<String>,
    http: reqwest::Client,
    dev: std::sync::Arc<crate::state::DevState>,
    embedding_dimensions: u32,
) -> BlueyResult<Box<dyn AiProvider>> {
    match config.kind {
        AiProviderKind::Mock => Ok(Box::new(mock::MockProvider::new(dev))),
        kind => {
            let api_key = api_key.ok_or_else(|| {
                BlueyError::configuration("missing_key", "the provider has no API key configured")
                    .recoverable(RecoveryAction::ConfigureProvider)
            })?;
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
                AiProviderKind::Mock => unreachable!(),
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
