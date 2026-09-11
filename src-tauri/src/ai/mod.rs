//! AI manager: routing (bluey-core router), provider adapters, streaming to
//! the frontend `Channel<AiChunk>` + mirrored `ai.*` bus events, cancellation
//! and generation superseding, request records and metrics.

pub mod providers;

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use bluey_core::accounts as account_rules;
use bluey_core::events::BlueyEvent;
use bluey_core::latency::{self, RustStamps};
use bluey_core::presets;
use bluey_core::router::{self, RoutingInput};
use bluey_core::types::{
    AiChunk, AiProviderConfig, AiProviderKind, AiRequest, AiTask, AppEvent, ConnectionTestResult,
    FinishReason, LatencyBudget, LatencyTrace, ModelAssignment, ModelRole, ModelSelection,
    ProviderAuthMethod, ReasoningLevel, Settings, TraceStamps,
};
use bluey_core::{now_iso, BlueyError, BlueyResult};
use bluey_protocols::gemini as gemini_proto;
use bluey_storage::{AiRequestRecord, AiRequestRepository};
use tauri::ipc::Channel;
use tokio_util::sync::CancellationToken;

use crate::accounts::AccountsManager;
use crate::events::EventBus;
use crate::secrets::{provider_key, SecretsStore};
use crate::settings::SettingsManager;
use crate::state::{DevState, MetricsRecorder, StateHub};
use crate::storage::Storage;
pub use providers::EmbedPurpose;
use providers::{
    build_provider, AiProvider, AudioFile, OAuthCredential, ProviderCredential, ProviderRequest,
    StreamItem, TranscribeFileOptions, Transcription,
};

/// Implicit mock provider id (available in developer mode / `dev-tools`).
pub const MOCK_PROVIDER_ID: &str = "mock";

struct ActiveRequest {
    token: CancellationToken,
    session_id: Option<String>,
    generation: u64,
}

/// A request's fast-path trace while its two halves are still arriving
/// (ADR 0010 §2): Rust writes its stamps at stream end, the WebView reports
/// first paint / done afterwards; whichever comes second updates the row.
struct TraceEntry {
    trace: LatencyTrace,
    /// The `ai_requests` row has been written.
    persisted: bool,
    /// Changed since the row was written (or before it was).
    dirty: bool,
}

/// Traces kept in memory for late stamps (and the bench).
const TRACE_BOOK_CAPACITY: usize = 64;

/// The AI orchestrator.
pub struct AiManager {
    http: reqwest::Client,
    secrets: Arc<SecretsStore>,
    settings: Arc<SettingsManager>,
    storage: Arc<Storage>,
    bus: Arc<EventBus>,
    hub: Arc<StateHub>,
    metrics: Arc<MetricsRecorder>,
    dev: Arc<DevState>,
    modes: Arc<crate::modes::ModeManager>,
    active: parking_lot::Mutex<HashMap<String, ActiveRequest>>,
    /// `list_models` results per (provider, role) — the Settings → AI tab asks
    /// once per role row, which would otherwise be seven catalogue fetches.
    model_cache: parking_lot::Mutex<ModelCache>,
    /// The subscription accounts (ADR 0009), attached once both managers exist:
    /// connected accounts are providers to the router and lend their tokens
    /// to the adapters per request.
    accounts: OnceLock<Arc<AccountsManager>>,
    /// Recent requests' fast-path traces (newest last).
    traces: parking_lot::Mutex<Vec<TraceEntry>>,
}

/// `(fetched at, model ids)` per `(provider id, role)`.
type ModelCache = HashMap<(String, Option<ModelRole>), (Instant, Vec<String>)>;

/// How long a provider's model catalogue is served from memory.
const MODEL_CACHE_TTL: Duration = Duration::from_secs(120);
/// Largest recording read into memory for batch transcription.
const MAX_RECORDING_BYTES: u64 = 512 * 1024 * 1024;

impl AiManager {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        http: reqwest::Client,
        secrets: Arc<SecretsStore>,
        settings: Arc<SettingsManager>,
        storage: Arc<Storage>,
        bus: Arc<EventBus>,
        hub: Arc<StateHub>,
        metrics: Arc<MetricsRecorder>,
        dev: Arc<DevState>,
        modes: Arc<crate::modes::ModeManager>,
    ) -> Self {
        Self {
            http,
            secrets,
            settings,
            storage,
            bus,
            hub,
            metrics,
            dev,
            modes,
            active: parking_lot::Mutex::new(HashMap::new()),
            model_cache: parking_lot::Mutex::new(HashMap::new()),
            accounts: OnceLock::new(),
            traces: parking_lot::Mutex::new(Vec::new()),
        }
    }

    /// Wire the accounts layer in (once, at boot).
    pub fn attach_accounts(&self, accounts: Arc<AccountsManager>) {
        let _ = self.accounts.set(accounts);
    }

    fn accounts(&self) -> BlueyResult<&Arc<AccountsManager>> {
        self.accounts
            .get()
            .ok_or_else(|| BlueyError::internal("the accounts layer is not attached"))
    }

    /// Whether the mock provider may be used (dev-tools build or developer mode).
    fn mock_allowed(&self) -> bool {
        cfg!(feature = "dev-tools")
            || cfg!(debug_assertions)
            || self.settings.get().general.developer_mode
    }

    /// Providers visible to the router: the configured ones (has_api_key kept
    /// fresh by the settings manager), the subscription accounts (usable ones
    /// count as keyed), plus the implicit mock provider.
    pub fn providers(&self) -> Vec<AiProviderConfig> {
        let mut providers = self.settings.get().ai.providers;
        if let Some(accounts) = self.accounts.get() {
            for account in accounts.provider_configs() {
                if !providers.iter().any(|p| p.id == account.id) {
                    providers.push(account);
                }
            }
        }
        if self.mock_allowed() && !providers.iter().any(|p| p.id == MOCK_PROVIDER_ID) {
            providers.push(AiProviderConfig {
                id: MOCK_PROVIDER_ID.into(),
                kind: AiProviderKind::Mock,
                name: "Mock (dev)".into(),
                base_url: String::new(),
                api_version: None,
                deployments: None,
                enabled: true,
                has_api_key: false,
                auth_method: Default::default(),
            });
        }
        providers
    }

    fn find_provider(&self, provider_id: &str) -> BlueyResult<AiProviderConfig> {
        self.providers()
            .into_iter()
            .find(|p| p.id == provider_id)
            .ok_or_else(|| {
                BlueyError::configuration("unknown_provider", "the provider is not configured")
            })
    }

    async fn adapter_for(&self, config: &AiProviderConfig) -> BlueyResult<Box<dyn AiProvider>> {
        let credential = if config.kind == AiProviderKind::Mock {
            ProviderCredential::None
        } else if config.auth_method == ProviderAuthMethod::OauthSubscription {
            // A fresh access token for this one request (single-flight refresh
            // inside); a dead refresh token surfaces as `account.needs_reauth`.
            let accounts = self.accounts()?;
            let tokens = accounts.credential_for(&config.id).await?;
            let identity = accounts.identity(&config.id);
            ProviderCredential::OAuth(OAuthCredential {
                access_token: tokens.access_token,
                account_id: identity.as_ref().and_then(|i| i.account_id.clone()),
                catalog: accounts.catalog(&config.id),
                device_id: accounts.device_id(),
                project_id: identity.and_then(|i| i.project_id),
            })
        } else {
            match self.secrets.get(&provider_key(&config.id)).await? {
                Some(key) => ProviderCredential::ApiKey(key),
                None => ProviderCredential::None,
            }
        };
        let dims = self.settings.get().ai.embedding_dimensions;
        build_provider(
            config,
            credential,
            self.http.clone(),
            self.dev.clone(),
            dims,
        )
    }

    /// A subscription provider's request outcome moves its account: a 401 to
    /// `NeedsReauth`, a plan limit to `RateLimited`, a block or drifted
    /// fingerprint to `Unavailable` — and a success back to `Connected`.
    async fn note_provider_outcome(&self, provider_id: &str, error: Option<&BlueyError>) {
        let Some(accounts) = self.accounts.get() else {
            return;
        };
        let is_account = self
            .providers()
            .iter()
            .any(|p| p.id == provider_id && p.auth_method == ProviderAuthMethod::OauthSubscription);
        if !is_account {
            return;
        }
        match error {
            Some(error) => accounts.note_request_error(provider_id, error).await,
            None => accounts.note_request_success(provider_id).await,
        }
    }

    /// Route a request to a provider+model.
    pub fn select(&self, request: &AiRequest) -> BlueyResult<ModelSelection> {
        let settings = self.settings.get();
        let preferred_role = self.modes.active_mode().preferred_model_role;
        let input = RoutingInput {
            task: request.task,
            latency: request.latency_budget,
            reasoning: request.reasoning,
            context_tokens: request.context_tokens,
            vision_required: request.vision_required,
            preferred_role,
            model_override: request.model_override.as_ref(),
        };
        router::select(&input, &settings.ai.models, &self.providers())
    }

    /// Start a streaming generation. Validation and routing happen before this
    /// returns; the stream itself runs on a task and reports through the
    /// channel and the `ai.*` events.
    pub fn stream(
        self: &Arc<Self>,
        request: AiRequest,
        channel: Channel<AiChunk>,
    ) -> BlueyResult<()> {
        let (selection, token, received) = self.start(&request)?;
        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            this.run_stream(request, selection, channel, token, received)
                .await;
        });
        Ok(())
    }

    /// Route, supersede older generations of the session, register the request
    /// as active. Returns the selection, the cancellation token and the arrival
    /// stamp (the request's `t_request_received` on the monotonic clock).
    fn start(&self, request: &AiRequest) -> BlueyResult<(ModelSelection, CancellationToken, f64)> {
        let received = crate::clock::mono_ms();
        self.bus.publish(BlueyEvent::AiRequested {
            request_id: request.request_id.clone(),
            task: request.task,
            session_id: request.session_id.clone(),
        });
        let selection = match self.select(request) {
            Ok(selection) => selection,
            Err(error) => {
                self.publish_failed(&request.request_id, error.clone(), is_primary(request.task));
                return Err(error);
            }
        };

        // Supersede older generations of the same session.
        if let Some(session_id) = &request.session_id {
            let active = self.active.lock();
            for (id, entry) in active.iter() {
                if entry.session_id.as_deref() == Some(session_id)
                    && entry.generation < request.generation
                {
                    tracing::debug!(request_id = %id, "superseding older generation");
                    entry.token.cancel();
                }
            }
        }

        let token = CancellationToken::new();
        self.active.lock().insert(
            request.request_id.clone(),
            ActiveRequest {
                token: token.clone(),
                session_id: request.session_id.clone(),
                generation: request.generation,
            },
        );

        if is_primary(request.task) {
            self.hub.transition_soft(AppEvent::ThinkingStarted);
        }
        Ok((selection, token, received))
    }

    /// Run one request to completion on the caller's task with no WebView
    /// listening, and return its merged trace (the bench, ADR 0010 §2).
    pub async fn run_traced(self: &Arc<Self>, request: AiRequest) -> BlueyResult<LatencyTrace> {
        let request_id = request.request_id.clone();
        let (selection, token, received) = self.start(&request)?;
        let sink: Channel<AiChunk> = Channel::new(|_| Ok(()));
        self.clone()
            .run_stream(request, selection, sink, token, received)
            .await;
        self.trace_of(&request_id)
            .ok_or_else(|| BlueyError::internal("the traced request left no trace"))
    }

    /// The trace of a recent request, if still in memory.
    pub fn trace_of(&self, request_id: &str) -> Option<LatencyTrace> {
        self.traces
            .lock()
            .iter()
            .find(|entry| entry.trace.request_id == request_id)
            .map(|entry| entry.trace.clone())
    }

    /// The model a bench runs on for `provider_id`: the role assignment closest
    /// to the default role, else the kind's preset (`mock-default` for the mock).
    pub fn bench_assignment(&self, provider_id: &str) -> BlueyResult<ModelAssignment> {
        let config = self.find_provider(provider_id)?;
        let model = self.default_model_for(&config).ok_or_else(|| {
            BlueyError::configuration(
                "no_model",
                format!("assign a model to `{provider_id}` first (Settings → AI)"),
            )
        })?;
        Ok(ModelAssignment {
            provider_id: provider_id.to_string(),
            model,
        })
    }

    /// Whether `ai.trace` events leave the process: `dev-tools` / debug builds,
    /// or developer mode.
    fn traces_enabled(&self) -> bool {
        cfg!(feature = "dev-tools")
            || cfg!(debug_assertions)
            || self.settings.get().general.developer_mode
    }

    fn publish_trace(&self, trace: &LatencyTrace) {
        if self.traces_enabled() {
            self.bus.publish(BlueyEvent::AiTrace(trace.clone()));
        }
    }

    /// Remember a freshly merged trace (not yet persisted).
    fn book_trace(&self, trace: LatencyTrace) {
        let mut book = self.traces.lock();
        book.retain(|entry| entry.trace.request_id != trace.request_id);
        if book.len() >= TRACE_BOOK_CAPACITY {
            book.remove(0);
        }
        book.push(TraceEntry {
            trace,
            persisted: false,
            dirty: false,
        });
    }

    /// The `ai_requests` row for `request_id` was written: flush a late report
    /// that arrived in between.
    async fn mark_trace_persisted(&self, request_id: &str) {
        let pending = {
            let mut book = self.traces.lock();
            let Some(entry) = book
                .iter_mut()
                .find(|entry| entry.trace.request_id == request_id)
            else {
                return;
            };
            entry.persisted = true;
            std::mem::take(&mut entry.dirty).then(|| entry.trace.clone())
        };
        if let Some(trace) = pending {
            self.persist_trace(trace).await;
        }
    }

    async fn persist_trace(&self, trace: LatencyTrace) {
        let storage = self.storage.clone();
        if let Err(error) = storage
            .run(move |db| AiRequestRepository::update_trace(db, &trace.request_id, &trace))
            .await
        {
            tracing::debug!(error = %error, "cannot update the request trace");
        }
    }

    /// The WebView's late stamps (first paint, done) for a request: merge, persist
    /// once the row exists, publish `ai.trace`. `None` for a request not in memory.
    pub async fn report_trace(
        &self,
        request_id: &str,
        stamps: TraceStamps,
    ) -> Option<LatencyTrace> {
        let (trace, persisted) = {
            let mut book = self.traces.lock();
            let entry = book
                .iter_mut()
                .find(|entry| entry.trace.request_id == request_id)?;
            latency::apply_late_stamps(&mut entry.trace, &stamps);
            if !entry.persisted {
                entry.dirty = true;
            }
            (entry.trace.clone(), entry.persisted)
        };
        if persisted {
            self.persist_trace(trace.clone()).await;
        }
        self.publish_trace(&trace);
        Some(trace)
    }

    async fn run_stream(
        self: Arc<Self>,
        request: AiRequest,
        selection: ModelSelection,
        channel: Channel<AiChunk>,
        token: CancellationToken,
        received: f64,
    ) {
        let request_id = request.request_id.clone();
        let primary = is_primary(request.task);
        let started = Instant::now();
        let mut rust = RustStamps {
            request_received: Some(received),
            ..RustStamps::default()
        };

        let started_chunk = AiChunk::Started {
            request_id: request_id.clone(),
            selection: selection.clone(),
        };
        let _ = channel.send(started_chunk.clone());
        self.bus.publish(BlueyEvent::AiChunk(started_chunk));
        self.bus.publish(BlueyEvent::AiStarted {
            request_id: request_id.clone(),
            provider: selection.provider_id.clone(),
            model: selection.model.clone(),
        });

        let outcome = self
            .drive_provider(&request, &selection, &channel, &token, started, &mut rust)
            .await;
        rust.stream_done = Some(crate::clock::mono_ms());

        self.active.lock().remove(&request_id);
        match &outcome {
            StreamOutcome::Failed { error } if !error.is_cancelled() => {
                self.note_provider_outcome(&selection.provider_id, Some(error))
                    .await
            }
            StreamOutcome::Completed { .. } => {
                self.note_provider_outcome(&selection.provider_id, None)
                    .await
            }
            _ => {}
        }

        let mut record = AiRequestRecord {
            id: request_id.clone(),
            session_id: request.session_id.clone(),
            task: enum_tag(&request.task),
            provider_id: Some(selection.provider_id.clone()),
            model: Some(selection.model.clone()),
            latency_budget: Some(enum_tag(&request.latency_budget)),
            context_tokens: Some(request.context_tokens),
            created_at: now_iso(),
            ..AiRequestRecord::default()
        };

        match outcome {
            StreamOutcome::Completed {
                finish,
                ttft_ms,
                input_tokens,
                output_tokens,
            } => {
                let total_ms = started.elapsed().as_millis() as u64;
                let completed = AiChunk::Completed {
                    request_id: request_id.clone(),
                    finish_reason: finish,
                    total_ms,
                    time_to_first_token_ms: ttft_ms,
                };
                let _ = channel.send(completed.clone());
                self.bus.publish(BlueyEvent::AiChunk(completed));
                self.bus.publish(BlueyEvent::AiCompleted {
                    request_id: request_id.clone(),
                    total_ms,
                    time_to_first_token_ms: ttft_ms,
                });
                record.input_tokens = input_tokens;
                record.output_tokens = output_tokens;
                record.ttft_ms = ttft_ms;
                record.total_ms = Some(total_ms);
                record.finish_reason = Some(enum_tag(&finish));
                self.metrics.update(|m| {
                    m.model_ms = Some(total_ms);
                    m.time_to_first_token_ms = ttft_ms;
                    m.total_response_ms = Some(total_ms);
                    m.input_tokens = input_tokens;
                    m.output_tokens = output_tokens;
                });
                if primary && matches!(finish, FinishReason::Stop | FinishReason::Length) {
                    self.hub.transition_soft(AppEvent::ResponseReady);
                }
            }
            StreamOutcome::Cancelled { ttft_ms } => {
                let total_ms = started.elapsed().as_millis() as u64;
                let completed = AiChunk::Completed {
                    request_id: request_id.clone(),
                    finish_reason: FinishReason::Cancelled,
                    total_ms,
                    time_to_first_token_ms: ttft_ms,
                };
                let _ = channel.send(completed.clone());
                self.bus.publish(BlueyEvent::AiChunk(completed));
                self.bus.publish(BlueyEvent::AiCancelled {
                    request_id: request_id.clone(),
                });
                record.finish_reason = Some("cancelled".into());
                record.total_ms = Some(total_ms);
            }
            StreamOutcome::Failed { error } => {
                record.finish_reason = Some("error".into());
                record.error_code = Some(error.code.clone());
                record.total_ms = Some(started.elapsed().as_millis() as u64);
                let failed = AiChunk::Failed {
                    request_id: request_id.clone(),
                    error: error.clone(),
                };
                let _ = channel.send(failed.clone());
                self.bus.publish(BlueyEvent::AiChunk(failed));
                self.publish_failed(&request_id, error, primary);
            }
        }

        // The merged fast-path trace (ADR 0010 §2): Rust's stamps plus the
        // WebView's pre-request offsets; first paint / done arrive by report.
        let trace = latency::merge(
            &request_id,
            request.trace.as_ref(),
            &rust,
            Some(&selection.provider_id),
            Some(&selection.model),
            record.input_tokens.or(Some(request.context_tokens)),
        );
        record.trace = Some(trace.clone());
        self.book_trace(trace.clone());
        self.publish_trace(&trace);

        let storage = self.storage.clone();
        let _ = storage
            .run(move |db| AiRequestRepository::record(db, &record))
            .await
            .map_err(|e| tracing::warn!(error = %e, "failed to record ai request"));
        self.mark_trace_persisted(&request_id).await;
    }

    async fn drive_provider(
        &self,
        request: &AiRequest,
        selection: &ModelSelection,
        channel: &Channel<AiChunk>,
        token: &CancellationToken,
        started: Instant,
        rust: &mut RustStamps,
    ) -> StreamOutcome {
        use futures::StreamExt;
        let config = match self.find_provider(&selection.provider_id) {
            Ok(config) => config,
            Err(error) => return StreamOutcome::Failed { error },
        };
        let adapter = match self.adapter_for(&config).await {
            Ok(adapter) => adapter,
            Err(error) => return StreamOutcome::Failed { error },
        };
        let provider_request = ProviderRequest {
            model: selection.model.clone(),
            messages: request.messages.clone(),
            max_output_tokens: request.max_output_tokens,
            temperature: request.temperature,
            output_schema: request.output_schema.clone(),
            task: request.task,
            latency: request.latency_budget,
            reasoning: request.reasoning,
            session_id: request.session_id.clone(),
        };
        rust.request_sent = Some(crate::clock::mono_ms());
        let mut stream = match adapter.stream(&provider_request, token.clone()).await {
            Ok(stream) => {
                rust.response_headers = Some(crate::clock::mono_ms());
                stream
            }
            Err(error) if error.is_cancelled() => {
                return StreamOutcome::Cancelled { ttft_ms: None }
            }
            Err(error) => return StreamOutcome::Failed { error },
        };

        let mut ttft_ms: Option<u64> = None;
        let mut input_tokens = None;
        let mut output_tokens = None;
        loop {
            let item = tokio::select! {
                _ = token.cancelled() => return StreamOutcome::Cancelled { ttft_ms },
                item = stream.next() => item,
            };
            let Some(item) = item else {
                // The adapter closed the stream without a terminal item: only a
                // cancellation does that legitimately (adapters report a cut-off
                // stream as an error).
                if token.is_cancelled() {
                    return StreamOutcome::Cancelled { ttft_ms };
                }
                return StreamOutcome::Completed {
                    finish: FinishReason::Stop,
                    ttft_ms,
                    input_tokens,
                    output_tokens,
                };
            };
            match item {
                Ok(StreamItem::Delta(text)) => {
                    if ttft_ms.is_none() {
                        ttft_ms = Some(started.elapsed().as_millis() as u64);
                        rust.first_token = Some(crate::clock::mono_ms());
                    }
                    let chunk = AiChunk::Delta {
                        request_id: request.request_id.clone(),
                        text,
                    };
                    let _ = channel.send(chunk.clone());
                    self.bus.publish(BlueyEvent::AiChunk(chunk));
                }
                Ok(StreamItem::Usage { input, output }) => {
                    input_tokens = input.or(input_tokens);
                    output_tokens = output.or(output_tokens);
                    let chunk = AiChunk::Usage {
                        request_id: request.request_id.clone(),
                        input_tokens: input,
                        output_tokens: output,
                    };
                    let _ = channel.send(chunk.clone());
                    self.bus.publish(BlueyEvent::AiChunk(chunk));
                }
                Ok(StreamItem::Finished(finish)) => {
                    return StreamOutcome::Completed {
                        finish,
                        ttft_ms,
                        input_tokens,
                        output_tokens,
                    };
                }
                Err(error) if error.is_cancelled() => return StreamOutcome::Cancelled { ttft_ms },
                Err(error) => return StreamOutcome::Failed { error },
            }
        }
    }

    fn publish_failed(&self, request_id: &str, error: BlueyError, primary: bool) {
        self.bus.publish(BlueyEvent::AiFailed {
            request_id: request_id.to_string(),
            error: error.clone(),
        });
        if primary && !error.is_cancelled() {
            self.hub.transition_soft(AppEvent::Failed { error });
        }
    }

    /// Cancel one request. Returns whether it was active.
    pub fn cancel(&self, request_id: &str) -> bool {
        let active = self.active.lock();
        match active.get(request_id) {
            Some(entry) => {
                entry.token.cancel();
                true
            }
            None => false,
        }
    }

    /// Cancel every active request; returns how many were cancelled.
    pub fn cancel_all(&self) -> u32 {
        let active = self.active.lock();
        for entry in active.values() {
            entry.token.cancel();
        }
        active.len() as u32
    }

    /// Embed texts via the embedding role, for `purpose` (documents vs queries).
    pub async fn embed(
        &self,
        texts: &[String],
        purpose: &EmbedPurpose,
    ) -> BlueyResult<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let settings = self.settings.get();
        let assignment = settings.ai.models.embedding.clone().ok_or_else(|| {
            BlueyError::configuration("no_model", "no embedding model is assigned")
        })?;
        let config = self.find_provider(&assignment.provider_id)?;
        let adapter = self.adapter_for(&config).await?;
        adapter.embed(&assignment.model, texts, purpose).await
    }

    /// Transcribe a whole recording with the transcription-role model (batch
    /// `gemini-3.5-transcribe`; a `*-live` assignment is mapped to its batch
    /// sibling). The file is read here so adapters only ever see bytes; the
    /// format is decided by extension (WAV, MP3, AIFF, AAC, OGG, FLAC).
    pub async fn transcribe_file(
        &self,
        path: &std::path::Path,
        options: &TranscribeFileOptions,
    ) -> BlueyResult<Transcription> {
        let settings = self.settings.get();
        let assignment = settings.ai.models.transcription.clone().ok_or_else(|| {
            BlueyError::configuration("no_model", "no transcription model is assigned")
        })?;
        let config = self.find_provider(&assignment.provider_id)?;
        if !matches!(
            config.kind,
            AiProviderKind::GoogleGemini | AiProviderKind::Mock
        ) {
            return Err(BlueyError::not_supported(
                "transcribe_file",
                "this provider cannot transcribe recordings; assign the transcription role to Google Gemini",
            ));
        }
        let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let mime_type = gemini_proto::audio_mime_for_extension(extension).ok_or_else(|| {
            BlueyError::invalid_params(format!(
                "unsupported recording format \"{extension}\" — use WAV, MP3, AIFF, AAC, OGG or FLAC"
            ))
        })?;
        let metadata = tokio::fs::metadata(path)
            .await
            .map_err(|e| BlueyError::invalid_params(format!("cannot read the recording: {e}")))?;
        if !metadata.is_file() || metadata.len() == 0 {
            return Err(BlueyError::invalid_params("the recording is empty"));
        }
        if metadata.len() > gemini_proto::FILES_API_MAX_BYTES.min(MAX_RECORDING_BYTES) {
            return Err(BlueyError::invalid_params(
                "the recording is larger than 512 MB — convert it to MP3 or FLAC first",
            ));
        }
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|e| BlueyError::invalid_params(format!("cannot read the recording: {e}")))?;
        // The name is only shown in Google's file store and file names can be
        // personal ("Interview with J. Doe.wav"); the real name stays local.
        let display_name = "recording".to_string();
        let model = gemini_proto::batch_transcribe_model(&assignment.model);
        let adapter = self.adapter_for(&config).await?;
        adapter
            .transcribe_audio(
                &model,
                AudioFile {
                    bytes,
                    mime_type,
                    display_name,
                },
                options,
            )
            .await
    }

    /// Whether an embedding model is currently usable (for documents).
    pub fn embeddings_ready(&self) -> bool {
        let settings = self.settings.get();
        if !settings.ai.embeddings_enabled {
            return false;
        }
        let Some(assignment) = settings.ai.models.embedding else {
            return false;
        };
        self.providers()
            .iter()
            .any(|p| p.id == assignment.provider_id && p.enabled)
    }

    /// Tiny prompt against one provider to measure reachability + latency.
    pub async fn test_connection(
        &self,
        provider_id: &str,
        model: Option<String>,
    ) -> BlueyResult<ConnectionTestResult> {
        let config = self.find_provider(provider_id)?;
        let model = model
            .or_else(|| self.default_model_for(&config))
            .ok_or_else(|| {
                BlueyError::configuration(
                    "no_model",
                    "pass a model or assign one to this provider first",
                )
            })?;
        let adapter = match self.adapter_for(&config).await {
            Ok(adapter) => adapter,
            Err(error) => {
                return Ok(ConnectionTestResult {
                    ok: false,
                    provider_id: provider_id.to_string(),
                    model: Some(model),
                    latency_ms: None,
                    error: Some(error),
                })
            }
        };
        let request = ProviderRequest {
            model: model.clone(),
            messages: vec![bluey_core::types::AiMessage::text(
                bluey_core::types::AiRole::User,
                "Reply with the single word: ok",
            )],
            // Thinking tokens count as output on Gemini 3.x; leave room for them.
            max_output_tokens: Some(64),
            temperature: Some(0.0),
            output_schema: None,
            task: AiTask::Answer,
            latency: LatencyBudget::UltraFast,
            reasoning: ReasoningLevel::None,
            session_id: None,
        };
        let started = Instant::now();
        let token = CancellationToken::new();
        let result = tokio::time::timeout(Duration::from_secs(20), async {
            let stream = adapter.stream(&request, token.clone()).await?;
            providers::collect_text(stream).await
        })
        .await;
        let latency = started.elapsed().as_millis() as u64;
        match &result {
            Ok(Ok(_)) => self.note_provider_outcome(provider_id, None).await,
            Ok(Err(error)) => self.note_provider_outcome(provider_id, Some(error)).await,
            Err(_) => {}
        }
        let outcome = match result {
            Ok(Ok(_text)) => ConnectionTestResult {
                ok: true,
                provider_id: provider_id.to_string(),
                model: Some(model),
                latency_ms: Some(latency),
                error: None,
            },
            Ok(Err(error)) => ConnectionTestResult {
                ok: false,
                provider_id: provider_id.to_string(),
                model: Some(model),
                latency_ms: Some(latency),
                error: Some(error),
            },
            Err(_) => ConnectionTestResult {
                ok: false,
                provider_id: provider_id.to_string(),
                model: Some(model),
                latency_ms: Some(latency),
                error: Some(BlueyError::network("timeout", "the test request timed out")),
            },
        };
        Ok(outcome)
    }

    /// The text model assigned to this provider closest to the default role
    /// (transcription/embedding models cannot answer a prompt), else the
    /// kind's preset default.
    fn default_model_for(&self, config: &AiProviderConfig) -> Option<String> {
        const TEXT_ROLES: [ModelRole; 5] = [
            ModelRole::Default,
            ModelRole::Fast,
            ModelRole::Reasoning,
            ModelRole::Vision,
            ModelRole::Research,
        ];
        let models = self.settings.get().ai.models;
        for role in TEXT_ROLES {
            if let Some(assignment) = models.get(role) {
                if assignment.provider_id == config.id {
                    return Some(assignment.model.clone());
                }
            }
        }
        if config.kind == AiProviderKind::Mock {
            return Some("mock-default".into());
        }
        if account_rules::is_subscription_kind(config.kind) {
            let catalog = self.accounts.get()?.catalog(&config.id)?;
            let suggested = account_rules::preset_assignments(&catalog)
                .into_iter()
                .find(|(role, _)| *role == ModelRole::Default)
                .map(|(_, model)| model.id.clone());
            return suggested.or_else(|| catalog.models.first().map(|m| m.id.clone()));
        }
        presets::by_kind(config.kind)
            .and_then(|preset| preset.model_for(ModelRole::Default))
            .map(str::to_string)
    }

    /// List models for one provider, optionally only those fit for `role`.
    pub async fn list_models(
        &self,
        provider_id: &str,
        role: Option<ModelRole>,
    ) -> BlueyResult<Vec<String>> {
        let config = self.find_provider(provider_id)?;
        let key = (provider_id.to_string(), role);
        if let Some((fetched, models)) = self.model_cache.lock().get(&key) {
            if fetched.elapsed() < MODEL_CACHE_TTL {
                return Ok(models.clone());
            }
        }
        let adapter = self.adapter_for(&config).await?;
        let models = adapter.list_models(role).await?;
        self.model_cache
            .lock()
            .insert(key, (Instant::now(), models.clone()));
        Ok(models)
    }

    /// Point roles at the provider's recommended models (`bluey_core::presets`).
    /// `overwrite = false` fills only unassigned roles. Returns the new settings.
    pub async fn apply_provider_presets(
        &self,
        provider_id: &str,
        overwrite: bool,
    ) -> BlueyResult<Settings> {
        let config = self.find_provider(provider_id)?;
        if account_rules::is_subscription_kind(config.kind) {
            // Subscription providers have no static preset: their recommended
            // models come from the account's catalog (§3.7).
            return self.accounts()?.apply_presets(provider_id, overwrite).await;
        }
        let mut models = self.settings.get().ai.models;
        let changed = bluey_core::presets::apply_presets(
            &mut models,
            &config,
            overwrite,
            &Default::default(),
        )?;
        tracing::info!(provider = %provider_id, roles = changed.len(), overwrite, "applied provider presets");
        let (_, new) = self
            .settings
            .update(serde_json::json!({ "ai": { "models": models } }))
            .await?;
        Ok(new)
    }

    /// Test the provider serving the default role (setup checks).
    pub async fn test_default_role(&self) -> BlueyResult<ConnectionTestResult> {
        let models = self.settings.get().ai.models;
        let assignment = models
            .default
            .or(models.fast)
            .ok_or_else(|| BlueyError::configuration("no_model", "no default model assigned"))?;
        self.test_connection(&assignment.provider_id, Some(assignment.model))
            .await
    }
}

enum StreamOutcome {
    Completed {
        finish: FinishReason,
        ttft_ms: Option<u64>,
        input_tokens: Option<u32>,
        output_tokens: Option<u32>,
    },
    Cancelled {
        ttft_ms: Option<u64>,
    },
    Failed {
        error: BlueyError,
    },
}

/// User-facing generations drive the HUD state machine; auxiliary tasks
/// (classification, summarisation, embeddings, vision-only analysis) do not.
fn is_primary(task: AiTask) -> bool {
    matches!(
        task,
        AiTask::Answer | AiTask::Coding | AiTask::SystemDesign | AiTask::DeepReasoning
    )
}

/// serde string tag of a unit enum value (e.g. `AiTask::Answer` → `answer`).
fn enum_tag<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_default()
}
