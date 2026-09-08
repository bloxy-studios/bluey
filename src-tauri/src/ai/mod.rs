//! AI manager: routing (bluey-core router), provider adapters, streaming to
//! the frontend `Channel<AiChunk>` + mirrored `ai.*` bus events, cancellation
//! and generation superseding, request records and metrics.

pub mod providers;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bluey_core::events::BlueyEvent;
use bluey_core::router::{self, RoutingInput};
use bluey_core::types::{
    AiChunk, AiProviderConfig, AiProviderKind, AiRequest, AiTask, AppEvent, ConnectionTestResult,
    FinishReason, LatencyBudget, ModelRole, ModelSelection, ReasoningLevel, Settings,
};
use bluey_core::{now_iso, BlueyError, BlueyResult};
use bluey_storage::{AiRequestRecord, AiRequestRepository};
use tauri::ipc::Channel;
use tokio_util::sync::CancellationToken;

use crate::events::EventBus;
use crate::secrets::{provider_key, SecretsStore};
use crate::settings::SettingsManager;
use crate::state::{DevState, MetricsRecorder, StateHub};
use crate::storage::Storage;
pub use providers::EmbedPurpose;
use providers::{build_provider, AiProvider, ProviderRequest, StreamItem};

/// Implicit mock provider id (available in developer mode / `dev-tools`).
pub const MOCK_PROVIDER_ID: &str = "mock";

struct ActiveRequest {
    token: CancellationToken,
    session_id: Option<String>,
    generation: u64,
}

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
}

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
        }
    }

    /// Whether the mock provider may be used (dev-tools build or developer mode).
    fn mock_allowed(&self) -> bool {
        cfg!(feature = "dev-tools")
            || cfg!(debug_assertions)
            || self.settings.get().general.developer_mode
    }

    /// Providers visible to the router: the configured ones (has_api_key kept
    /// fresh by the settings manager) plus the implicit mock provider.
    pub fn providers(&self) -> Vec<AiProviderConfig> {
        let mut providers = self.settings.get().ai.providers;
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
        let key = if config.kind == AiProviderKind::Mock {
            None
        } else {
            self.secrets.get(&provider_key(&config.id)).await?
        };
        let dims = self.settings.get().ai.embedding_dimensions;
        build_provider(config, key, self.http.clone(), self.dev.clone(), dims)
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
        self.bus.publish(BlueyEvent::AiRequested {
            request_id: request.request_id.clone(),
            task: request.task,
            session_id: request.session_id.clone(),
        });
        let selection = match self.select(&request) {
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

        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            this.run_stream(request, selection, channel, token).await;
        });
        Ok(())
    }

    async fn run_stream(
        self: Arc<Self>,
        request: AiRequest,
        selection: ModelSelection,
        channel: Channel<AiChunk>,
        token: CancellationToken,
    ) {
        let request_id = request.request_id.clone();
        let primary = is_primary(request.task);
        let started = Instant::now();

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
            .drive_provider(&request, &selection, &channel, &token, started)
            .await;

        self.active.lock().remove(&request_id);

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

        let storage = self.storage.clone();
        let _ = storage
            .run(move |db| AiRequestRepository::record(db, &record))
            .await
            .map_err(|e| tracing::warn!(error = %e, "failed to record ai request"));
    }

    async fn drive_provider(
        &self,
        request: &AiRequest,
        selection: &ModelSelection,
        channel: &Channel<AiChunk>,
        token: &CancellationToken,
        started: Instant,
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
        };
        let mut stream = match adapter.stream(&provider_request, token.clone()).await {
            Ok(stream) => stream,
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
            max_output_tokens: Some(8),
            temperature: Some(0.0),
            output_schema: None,
            task: AiTask::Answer,
            latency: LatencyBudget::UltraFast,
            reasoning: ReasoningLevel::None,
        };
        let started = Instant::now();
        let token = CancellationToken::new();
        let result = tokio::time::timeout(Duration::from_secs(20), async {
            let stream = adapter.stream(&request, token.clone()).await?;
            providers::collect_text(stream).await
        })
        .await;
        let latency = started.elapsed().as_millis() as u64;
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

    /// The model assigned to this provider closest to the default role.
    fn default_model_for(&self, config: &AiProviderConfig) -> Option<String> {
        let models = self.settings.get().ai.models;
        for role in ModelRole::ALL {
            if let Some(assignment) = models.get(role) {
                if assignment.provider_id == config.id {
                    return Some(assignment.model.clone());
                }
            }
        }
        if config.kind == AiProviderKind::Mock {
            return Some("mock-default".into());
        }
        None
    }

    /// List models for one provider, optionally only those fit for `role`.
    pub async fn list_models(
        &self,
        provider_id: &str,
        role: Option<ModelRole>,
    ) -> BlueyResult<Vec<String>> {
        let config = self.find_provider(provider_id)?;
        let adapter = self.adapter_for(&config).await?;
        adapter.list_models(role).await
    }

    /// Point roles at the provider's recommended models (`bluey_core::presets`).
    /// `overwrite = false` fills only unassigned roles. Returns the new settings.
    pub async fn apply_provider_presets(
        &self,
        provider_id: &str,
        overwrite: bool,
    ) -> BlueyResult<Settings> {
        let config = self.find_provider(provider_id)?;
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
