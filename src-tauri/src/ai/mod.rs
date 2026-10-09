//! AI manager: routing (bluey-core router), provider adapters, streaming to
//! the frontend `Channel<AiChunk>` + mirrored `ai.*` bus events, cancellation
//! and generation superseding (per session *and* scope), request records and
//! metrics. Privacy → Cloud AI off refuses every model call made here.

pub mod providers;

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use bluey_core::accounts as account_rules;
use bluey_core::error::RecoveryAction;
use bluey_core::events::BlueyEvent;
use bluey_core::latency::{self, RustStamps};
use bluey_core::presets;
use bluey_core::router::{self, RoutingInput};
use bluey_core::types::{
    AccountStatus, AiChunk, AiProviderConfig, AiProviderKind, AiReadiness, AiRequest, AiTask,
    AppEvent, AppState, ConnectionTestResult, FinishReason, LatencyBudget, LatencyTrace,
    ModelAssignment, ModelRole, ModelSelection, ProviderAuthMethod, ReasoningLevel, Settings,
    TraceStamps,
};
use bluey_core::{now_iso, BlueyError, BlueyErrorKind, BlueyResult};
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
    /// Supersede group: generations are only comparable within one scope.
    scope: Option<String>,
    generation: u64,
    /// Whether this request drives the app state machine ([`drives_state`]).
    drives_state: bool,
}

impl ActiveRequest {
    /// A newer generation of the same session and scope replaces this one.
    /// Requests of different scopes never cancel each other: each scope
    /// counts its own generations (ADR 0005), so comparing across them would
    /// let a busy background counter cancel the user's own answer.
    fn superseded_by(&self, request: &AiRequest) -> bool {
        request.session_id.is_some()
            && self.session_id == request.session_id
            && self.scope == request.scope
            && self.generation < request.generation
    }
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
        Ok(self.adapter_and_token(config).await?.0)
    }

    /// [`Self::adapter_for`] plus the subscription access token it carries, so
    /// a 401 can force a refresh of exactly that token.
    async fn adapter_and_token(
        &self,
        config: &AiProviderConfig,
    ) -> BlueyResult<(Box<dyn AiProvider>, Option<String>)> {
        let mut oauth_token = None;
        let credential = if config.kind == AiProviderKind::Mock {
            ProviderCredential::None
        } else if config.auth_method == ProviderAuthMethod::OauthSubscription {
            // A fresh access token for this one request (single-flight refresh
            // inside); a dead refresh token surfaces as `account.needs_reauth`.
            let accounts = self.accounts()?;
            let tokens = accounts.credential_for(&config.id).await?;
            let identity = accounts.identity(&config.id);
            oauth_token = Some(tokens.access_token.clone());
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
        let adapter = build_provider(
            config,
            credential,
            self.http.clone(),
            self.dev.clone(),
            dims,
        )?;
        Ok((adapter, oauth_token))
    }

    /// A subscription provider rejected `token` (HTTP 401) before it expired:
    /// force one refresh (single-flight). `true` = retry with the new token.
    async fn refreshed_after_rejection(
        &self,
        config: &AiProviderConfig,
        token: Option<&str>,
        error: &BlueyError,
    ) -> bool {
        let (Some(token), Some(accounts)) = (token, self.accounts.get()) else {
            return false;
        };
        error.code == bluey_core::accounts::codes::NEEDS_REAUTH
            && accounts.refresh_rejected(&config.id, token).await
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
        let preferred_role =
            preferred_role_for(request, || self.modes.active_mode().preferred_model_role);
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
            .map_err(|error| self.name_account_state(error))
    }

    /// Whether an answer (and a screen question) routes right now: the same
    /// router, providers and Cloud AI switch a real ask meets. Onboarding and
    /// Settings show this instead of guessing from the settings.
    pub fn readiness(&self) -> AiReadiness {
        let preferred_role = self.modes.active_mode().preferred_model_role;
        let mut readiness = readiness_of(&self.settings.get(), &self.providers(), preferred_role);
        readiness.error = readiness.error.map(|error| self.name_account_state(error));
        readiness
    }

    /// An account stop signal says "your API key is used meanwhile" only when
    /// the router now reaches another provider for the request: that provider
    /// rides along as `details.fallbackProviderId`.
    fn with_fallback_hint(
        &self,
        request: &AiRequest,
        failed_provider: &str,
        mut error: BlueyError,
    ) -> BlueyError {
        if !error.code.starts_with("account.") {
            return error;
        }
        let Ok(fallback) = self.select(request) else {
            return error;
        };
        if fallback.provider_id == failed_provider {
            return error;
        }
        let hint = serde_json::Value::from(fallback.provider_id);
        match error.details.as_mut() {
            Some(serde_json::Value::Object(details)) => {
                details.insert("fallbackProviderId".into(), hint);
            }
            None => error.details = Some(serde_json::json!({ "fallbackProviderId": hint })),
            Some(_) => {}
        }
        error
    }

    /// The router only knows an account is unusable; name its state
    /// (`account_needs_reauth`, `account_rate_limited`, …) for the copy.
    fn name_account_state(&self, mut error: BlueyError) -> BlueyError {
        let Some(details) = error.details.as_mut().and_then(|d| d.as_object_mut()) else {
            return error;
        };
        if details.get("cause").and_then(|c| c.as_str()) != Some("account_unavailable") {
            return error;
        }
        let provider_id = details.get("providerId").and_then(|p| p.as_str());
        let account = self.accounts.get().and_then(|accounts| {
            accounts
                .list()
                .into_iter()
                .find(|account| Some(account.provider_id.as_str()) == provider_id)
        });
        if let Some(account) = account {
            let state = match account.status {
                AccountStatus::Disconnected => "disconnected",
                AccountStatus::Connecting { .. } => "connecting",
                AccountStatus::Connected => "connected",
                AccountStatus::NeedsReauth => "needs_reauth",
                AccountStatus::RateLimited { .. } => "rate_limited",
                AccountStatus::Unavailable { .. } => "unavailable",
            };
            details.insert("cause".into(), format!("account_{state}").into());
        }
        error
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
        let selection =
            match ensure_cloud_ai(&self.settings.get()).and_then(|()| self.select(request)) {
                Ok(selection) => selection,
                Err(error) => {
                    self.publish_failed(&request.request_id, error.clone(), drives_state(request));
                    return Err(error);
                }
            };

        // Supersede older generations of the same session and scope.
        {
            let active = self.active.lock();
            for (id, entry) in active.iter() {
                if entry.superseded_by(request) {
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
                scope: request.scope.clone(),
                generation: request.generation,
                drives_state: drives_state(request),
            },
        );

        if drives_state(request) {
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
        let primary = drives_state(&request);
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
                if primary {
                    self.leave_thinking_after_cancel();
                }
            }
            StreamOutcome::Failed { error } => {
                let error = self.with_fallback_hint(&request, &selection.provider_id, error);
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
        let (adapter, oauth_token) = match self.adapter_and_token(&config).await {
            Ok(built) => built,
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
        let opened = match adapter.stream(&provider_request, token.clone()).await {
            // A subscription token rejected before its expiry (revoked early,
            // clock skew): one forced refresh and one retry, before any byte.
            Err(error)
                if self
                    .refreshed_after_rejection(&config, oauth_token.as_deref(), &error)
                    .await =>
            {
                match self.adapter_for(&config).await {
                    Ok(adapter) => adapter.stream(&provider_request, token.clone()).await,
                    Err(error) => Err(error),
                }
            }
            opened => opened,
        };
        let mut stream = match opened {
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

    /// A cancelled (Escape, Stop, superseded) answer must not leave the pill on
    /// "Thinking": when no other state-driving request is still running, the
    /// machine goes back to idle.
    fn leave_thinking_after_cancel(&self) {
        let others_running = self.active.lock().values().any(|entry| entry.drives_state);
        if !others_running && self.hub.state() == AppState::Thinking {
            self.hub.transition_soft(AppEvent::ResponseDismissed);
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
        ensure_cloud_ai(&settings)?;
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
        ensure_cloud_ai(&settings)?;
        let (config, assigned_model) = batch_transcription_target(
            settings.ai.models.transcription.as_ref(),
            &self.providers(),
        )?;
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
        let model = gemini_proto::batch_transcribe_model(&assigned_model);
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

    /// Whether an embedding model is currently usable (for documents). With
    /// Cloud AI off nothing is embedded: documents fall back to keyword search.
    pub fn embeddings_ready(&self) -> bool {
        let settings = self.settings.get();
        if !settings.ai.embeddings_enabled || ensure_cloud_ai(&settings).is_err() {
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

/// The role the request's own mode prefers; the active mode only for callers
/// that did not say (a mode switch must not reroute an answer in flight).
fn preferred_role_for(
    request: &AiRequest,
    active_mode_role: impl FnOnce() -> Option<ModelRole>,
) -> Option<ModelRole> {
    request.preferred_model_role.or_else(active_mode_role)
}

/// Only answers the user asked for move the app state (Thinking, Error);
/// background work (proactive preparation, live suggestions) never does.
fn drives_state(request: &AiRequest) -> bool {
    is_primary(request.task) && !request.background
}

/// Provider + model that batch-transcribes an imported recording. Only Gemini
/// (and the dev mock) can; when the Transcription role points elsewhere (the
/// Foundry preset assigns MAI-Transcribe, a live-only model) the first enabled
/// Gemini provider with a key does it with its preset model, as live
/// transcription already does.
fn batch_transcription_target(
    assignment: Option<&ModelAssignment>,
    providers: &[AiProviderConfig],
) -> BlueyResult<(AiProviderConfig, String)> {
    let can_batch = |p: &AiProviderConfig| {
        matches!(p.kind, AiProviderKind::GoogleGemini | AiProviderKind::Mock)
    };
    if let Some(assignment) = assignment {
        if let Some(config) = providers
            .iter()
            .find(|p| p.id == assignment.provider_id && can_batch(p))
        {
            return Ok((config.clone(), assignment.model.clone()));
        }
    }
    let mut gemini: Vec<_> = providers
        .iter()
        .filter(|p| p.kind == AiProviderKind::GoogleGemini && p.enabled && p.has_api_key)
        .collect();
    gemini.sort_by_key(|p| p.id != presets::GEMINI_ID);
    let model = presets::by_kind(AiProviderKind::GoogleGemini)
        .and_then(|preset| preset.model_for(ModelRole::Transcription));
    match (gemini.first(), model) {
        (Some(config), Some(model)) => {
            tracing::info!(provider = %config.id, "importing a recording through Gemini");
            Ok(((*config).clone(), model.to_string()))
        }
        _ => Err(BlueyError::not_supported(
            "transcribe_file",
            "importing a recording needs a Google Gemini provider with a key; add one in Settings → AI",
        )),
    }
}

/// Privacy → Cloud AI is the master switch for sending anything to a model
/// provider; it is enforced here, where the network calls happen, and not
/// only in the WebView (research and the agent sidecar check it too).
pub(crate) fn ensure_cloud_ai(settings: &Settings) -> BlueyResult<()> {
    if settings.privacy.cloud_ai_enabled {
        return Ok(());
    }
    Err(BlueyError::new(
        BlueyErrorKind::Configuration,
        CLOUD_AI_DISABLED_CODE,
        "Cloud AI is turned off in Privacy settings, so Bluey cannot ask a model right now.",
    )
    .recoverable(RecoveryAction::OpenSettings {
        tab: "privacy".into(),
    }))
}

/// [`AiManager::readiness`] over explicit state: an Answer request (and one
/// with images) through the router, behind the Cloud AI switch.
fn readiness_of(
    settings: &Settings,
    providers: &[AiProviderConfig],
    preferred_role: Option<ModelRole>,
) -> AiReadiness {
    let route = |vision_required: bool| {
        ensure_cloud_ai(settings)?;
        let input = RoutingInput {
            task: AiTask::Answer,
            latency: LatencyBudget::Fast,
            reasoning: ReasoningLevel::None,
            context_tokens: 0,
            vision_required,
            preferred_role,
            model_override: None,
        };
        router::select(&input, &settings.ai.models, providers)
    };
    let vision = route(true).is_ok();
    match route(false) {
        Ok(selection) => AiReadiness {
            ok: true,
            provider_id: Some(selection.provider_id),
            model: Some(selection.model),
            vision,
            error: None,
        },
        Err(error) => AiReadiness {
            ok: false,
            provider_id: None,
            model: None,
            vision,
            error: Some(error),
        },
    }
}

/// Error code of a model call refused because Cloud AI is off (`present.ts` copy).
pub const CLOUD_AI_DISABLED_CODE: &str = "privacy.cloud_ai_disabled";

/// serde string tag of a unit enum value (e.g. `AiTask::Answer` → `answer`).
fn enum_tag<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bluey_core::types::{LatencyBudget, ReasoningLevel};

    fn request(session: Option<&str>, scope: Option<&str>, generation: u64) -> AiRequest {
        AiRequest {
            request_id: format!("req_{generation}"),
            session_id: session.map(str::to_string),
            generation,
            task: AiTask::Answer,
            latency_budget: LatencyBudget::Fast,
            reasoning: ReasoningLevel::None,
            vision_required: false,
            context_tokens: 10,
            messages: Vec::new(),
            output_schema: None,
            max_output_tokens: None,
            temperature: None,
            model_override: None,
            trace: None,
            scope: scope.map(str::to_string),
            background: false,
            preferred_model_role: None,
            created_at: now_iso(),
        }
    }

    fn active(request: &AiRequest) -> ActiveRequest {
        ActiveRequest {
            token: CancellationToken::new(),
            session_id: request.session_id.clone(),
            scope: request.scope.clone(),
            generation: request.generation,
            drives_state: drives_state(request),
        }
    }

    #[test]
    fn a_busy_prepare_scope_never_cancels_the_users_answer() {
        // The user's ⌘↵ answer (ask gen 1) is streaming while the fifth
        // detected question is prepared (prepare gen 5) in the same session.
        let ask = active(&request(Some("s1"), Some("ask"), 1));
        assert!(!ask.superseded_by(&request(Some("s1"), Some("prepare"), 5)));
        // …and the reverse: a manual ask does not kill a preparation.
        let prepare = active(&request(Some("s1"), Some("prepare"), 1));
        assert!(!prepare.superseded_by(&request(Some("s1"), Some("ask"), 3)));
    }

    #[test]
    fn a_newer_generation_supersedes_within_its_session_and_scope() {
        let first = active(&request(Some("s1"), Some("ask"), 1));
        assert!(first.superseded_by(&request(Some("s1"), Some("ask"), 2)));
        assert!(!first.superseded_by(&request(Some("s1"), Some("ask"), 1)));
        assert!(!first.superseded_by(&request(Some("s2"), Some("ask"), 2)));
        assert!(!first.superseded_by(&request(None, Some("ask"), 2)));
        // Callers that send no scope keep superseding each other per session.
        let legacy = active(&request(Some("s1"), None, 1));
        assert!(legacy.superseded_by(&request(Some("s1"), None, 2)));
    }

    #[test]
    fn background_requests_never_drive_the_app_state() {
        let mut prepare = request(Some("s1"), Some("prepare"), 1);
        prepare.background = true;
        assert!(!drives_state(&prepare));
        assert!(drives_state(&request(Some("s1"), Some("ask"), 1)));
        let mut classify = request(None, None, 1);
        classify.task = AiTask::Classification;
        assert!(!drives_state(&classify));
    }

    #[test]
    fn background_flag_is_optional_on_the_wire() {
        let json = serde_json::json!({
            "requestId": "r", "generation": 1, "task": "answer", "latencyBudget": "fast",
            "reasoning": "none", "visionRequired": false, "contextTokens": 1,
            "messages": [], "createdAt": "2026-09-28T00:00:00Z",
            "scope": "prepare", "background": true, "preferredModelRole": "reasoning"
        });
        let parsed: AiRequest = serde_json::from_value(json).expect("request");
        assert!(parsed.background);
        assert_eq!(parsed.scope.as_deref(), Some("prepare"));
        assert_eq!(parsed.preferred_model_role, Some(ModelRole::Reasoning));
        let bare: AiRequest = serde_json::from_value(serde_json::json!({
            "requestId": "r", "generation": 1, "task": "answer", "latencyBudget": "fast",
            "reasoning": "none", "visionRequired": false, "contextTokens": 1,
            "messages": [], "createdAt": "2026-09-28T00:00:00Z"
        }))
        .expect("bare request");
        assert!(!bare.background);
        assert_eq!(bare.scope, None);
    }

    #[test]
    fn routing_uses_the_requests_mode_role_over_the_active_mode() {
        let mut built_in_sales = request(None, Some("ask"), 1);
        built_in_sales.preferred_model_role = Some(ModelRole::Fast);
        assert_eq!(
            preferred_role_for(&built_in_sales, || Some(ModelRole::Reasoning)),
            Some(ModelRole::Fast)
        );
        let unspecified = request(None, Some("ask"), 1);
        assert_eq!(
            preferred_role_for(&unspecified, || Some(ModelRole::Reasoning)),
            Some(ModelRole::Reasoning)
        );
    }

    #[test]
    fn cloud_ai_off_refuses_model_calls_with_the_privacy_code() {
        let mut settings = Settings::default();
        assert!(ensure_cloud_ai(&settings).is_ok());
        settings.privacy.cloud_ai_enabled = false;
        let error = ensure_cloud_ai(&settings).expect_err("refused");
        assert_eq!(error.code, CLOUD_AI_DISABLED_CODE);
        assert_eq!(error.kind, BlueyErrorKind::Configuration);
        assert!(error.recoverable);
    }

    fn keyed(id: &str, kind: AiProviderKind) -> AiProviderConfig {
        let preset = presets::by_kind(kind).expect("preset kind");
        AiProviderConfig {
            id: id.into(),
            has_api_key: true,
            ..preset.config()
        }
    }

    fn assigned(provider_id: &str, model: &str) -> ModelAssignment {
        ModelAssignment {
            provider_id: provider_id.into(),
            model: model.into(),
        }
    }

    #[test]
    fn a_foundry_transcription_role_imports_recordings_through_gemini() {
        let foundry = assigned(presets::AZURE_FOUNDRY_ID, "MAI-Transcribe-1.5");
        let providers = [
            keyed(presets::AZURE_FOUNDRY_ID, AiProviderKind::AzureFoundry),
            keyed(presets::GEMINI_ID, AiProviderKind::GoogleGemini),
        ];
        let (config, model) =
            batch_transcription_target(Some(&foundry), &providers).expect("gemini fallback");
        assert_eq!(config.id, presets::GEMINI_ID);
        let preset = presets::by_kind(AiProviderKind::GoogleGemini).unwrap();
        assert_eq!(
            Some(model.as_str()),
            preset.model_for(ModelRole::Transcription)
        );

        let gemini = assigned(presets::GEMINI_ID, "gemini-custom");
        let (_, model) = batch_transcription_target(Some(&gemini), &providers).unwrap();
        assert_eq!(model, "gemini-custom", "a Gemini assignment is used as is");
    }

    #[test]
    fn without_a_keyed_gemini_provider_import_asks_for_one() {
        let foundry = assigned(presets::AZURE_FOUNDRY_ID, "MAI-Transcribe-1.5");
        let mut gemini = keyed(presets::GEMINI_ID, AiProviderKind::GoogleGemini);
        gemini.has_api_key = false;
        let providers = [
            keyed(presets::AZURE_FOUNDRY_ID, AiProviderKind::AzureFoundry),
            gemini,
        ];
        let error = batch_transcription_target(Some(&foundry), &providers).expect_err("none");
        assert_eq!(error.kind, BlueyErrorKind::NotSupported);
        assert!(error.message.contains("Google Gemini provider with a key"));
    }

    #[test]
    fn readiness_names_why_an_answer_cannot_be_routed() {
        let mut settings = Settings::default();
        settings.ai.models.default = Some(assigned(presets::GEMINI_ID, "gemini-flash"));
        let mut gemini = keyed(presets::GEMINI_ID, AiProviderKind::GoogleGemini);
        gemini.has_api_key = false;

        let keyless = readiness_of(&settings, &[gemini.clone()], None);
        assert!(!keyless.ok && !keyless.vision);
        let error = keyless.error.expect("cause");
        assert_eq!(error.code, "config.provider_unusable");
        assert_eq!(error.details.expect("details")["cause"], "missing_key");

        gemini.has_api_key = true;
        let ready = readiness_of(&settings, &[gemini.clone()], None);
        assert!(ready.ok && ready.error.is_none());
        assert_eq!(ready.provider_id.as_deref(), Some(presets::GEMINI_ID));
        assert_eq!(ready.model.as_deref(), Some("gemini-flash"));

        settings.privacy.cloud_ai_enabled = false;
        let off = readiness_of(&settings, &[gemini], None);
        assert!(!off.ok);
        assert_eq!(off.error.expect("cause").code, CLOUD_AI_DISABLED_CODE);
    }
}
