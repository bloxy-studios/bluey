//! Research-agent sidecar client (`bluey-agent`, ADR 0004): one process per
//! job, JSON-Lines over stdio (`bluey_protocols::{jsonl, agent}`), credentials
//! injected from the Keychain into the child environment only, document reads
//! served from SQLite for the request's allow-list, cancellation and a hard
//! wall-clock cap. Events are mapped onto `research.event`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bluey_core::events::BlueyEvent;
use bluey_core::presets;
use bluey_core::types::{
    AiProviderConfig, AiProviderKind, DeepResearchEvent, DeepResearchRequest, ModelRole,
    ResearchBackend, Settings,
};
use bluey_core::{BlueyError, BlueyResult};
use bluey_protocols::agent::{self as proto, AgentEvent};
use bluey_protocols::jsonl::{self, Incoming};
use bluey_storage::DocumentRepository;
use serde_json::Value;
use tauri::AppHandle;
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
use tauri_plugin_shell::ShellExt;

use crate::events::EventBus;
use crate::secrets::{provider_key, SecretsStore, AGENT_ANTHROPIC_KEY, EXA_KEY, FIRECRAWL_KEY};
use crate::settings::SettingsManager;
use crate::storage::Storage;

/// Sidecar binary name (matches `bundle.externalBin`).
pub const AGENT_BIN: &str = "bluey-agent";
/// Hard cap per job; the TS caller enforces its own (shorter) timeout.
const JOB_WALL_CLOCK: Duration = Duration::from_secs(10 * 60);
/// Grace period between `research.cancel` and killing the process.
const CANCEL_GRACE: Duration = Duration::from_secs(2);
/// Claude-backend variables forwarded from Bluey's own environment (`.env`) when set.
const CLAUDE_PASSTHROUGH_ENV: &[&str] = &[
    "CLAUDE_CODE_USE_FOUNDRY",
    "ANTHROPIC_FOUNDRY_RESOURCE",
    "ANTHROPIC_FOUNDRY_BASE_URL",
    "ANTHROPIC_FOUNDRY_API_KEY",
    "ANTHROPIC_FOUNDRY_AUTH_TOKEN",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "AZURE_FOUNDRY_ENDPOINT",
    "AZURE_FOUNDRY_API_KEY",
    "BLUEY_CLAUDE_CLI",
];
/// Backend-independent knobs forwarded when set.
const COMMON_PASSTHROUGH_ENV: &[&str] = &["BLUEY_AGENT_MAX_TURNS", "BLUEY_AGENT_MOCK"];

struct Job {
    child: Option<CommandChild>,
    allowed_documents: Option<Vec<String>>,
    started: Instant,
}

pub struct AgentManager {
    app: AppHandle,
    bus: Arc<EventBus>,
    secrets: Arc<SecretsStore>,
    settings: Arc<SettingsManager>,
    storage: Arc<Storage>,
    jobs: Arc<parking_lot::Mutex<HashMap<String, Job>>>,
    next_request_id: AtomicU64,
}

impl AgentManager {
    pub fn new(
        app: AppHandle,
        bus: Arc<EventBus>,
        secrets: Arc<SecretsStore>,
        settings: Arc<SettingsManager>,
        storage: Arc<Storage>,
    ) -> Self {
        Self {
            app,
            bus,
            secrets,
            settings,
            storage,
            jobs: Arc::new(parking_lot::Mutex::new(HashMap::new())),
            next_request_id: AtomicU64::new(1),
        }
    }

    /// Where Tauri places the sidecar next to the app executable.
    pub fn binary_path() -> Option<PathBuf> {
        let exe = std::env::current_exe().ok()?;
        let path = exe.parent()?.join(AGENT_BIN);
        path.is_file().then_some(path)
    }

    /// Whether the sidecar binary exists and a Claude credential is available.
    pub async fn available(&self) -> bool {
        if Self::binary_path().is_none() {
            return false;
        }
        let settings = self.settings.get();
        match settings.ai.research_backend {
            ResearchBackend::Gemini => match Self::gemini_provider_id(&settings) {
                Some(id) => self.secrets.has(&provider_key(&id)).await.unwrap_or(false),
                None => false,
            },
            ResearchBackend::Claude => {
                if env_truthy("CLAUDE_CODE_USE_FOUNDRY") {
                    return env_present("ANTHROPIC_FOUNDRY_API_KEY")
                        || env_present("ANTHROPIC_FOUNDRY_AUTH_TOKEN")
                        || env_present("AZURE_FOUNDRY_API_KEY");
                }
                self.secrets.has(AGENT_ANTHROPIC_KEY).await.unwrap_or(false)
                    || env_present("ANTHROPIC_API_KEY")
            }
        }
    }

    /// Environment for one job: every credential the sidecar may need, read
    /// from the Keychain (values never logged), plus documented pass-throughs.
    async fn job_env(&self) -> BlueyResult<Vec<(String, String)>> {
        let settings = self.settings.get();
        let mut env: Vec<(String, String)> = Vec::new();
        // Exactly one backend's credentials travel to the sidecar (ADR 0007).
        match settings.ai.research_backend {
            ResearchBackend::Gemini => {
                env.push(("RESEARCH_BACKEND".into(), "gemini".into()));
                if let Some(id) = Self::gemini_provider_id(&settings) {
                    if let Some(key) = self.secrets.get(&provider_key(&id)).await? {
                        env.push(("GEMINI_API_KEY".into(), key));
                    }
                }
            }
            ResearchBackend::Claude => {
                env.push(("RESEARCH_BACKEND".into(), "claude".into()));
                if let Some(key) = self.secrets.get(AGENT_ANTHROPIC_KEY).await? {
                    env.push(("ANTHROPIC_API_KEY".into(), key));
                }
                push_passthrough(&mut env, CLAUDE_PASSTHROUGH_ENV);
            }
        }
        // Tool credentials and generic knobs serve both backends.
        if let Some(key) = self.secrets.get(EXA_KEY).await? {
            env.push(("EXA_API_KEY".into(), key));
        }
        if let Some(key) = self.secrets.get(FIRECRAWL_KEY).await? {
            env.push(("FIRECRAWL_API_KEY".into(), key));
        }
        push_passthrough(&mut env, COMMON_PASSTHROUGH_ENV);
        if let Some(model) = self.research_model() {
            env.push(("BLUEY_RESEARCH_MODEL".into(), model));
        }
        Ok(env)
    }

    /// The research-role model for the active backend: the assigned model when
    /// it lives on a provider of the backend's kind, else the Gemini preset
    /// default (the Claude backend only understands Claude ids / Foundry
    /// deployments, so it gets `None` and the sidecar default).
    fn research_model(&self) -> Option<String> {
        let settings = self.settings.get();
        let assignment = settings.ai.models.research.as_ref();
        let provider_kind = assignment
            .and_then(|a| settings.ai.providers.iter().find(|p| p.id == a.provider_id))
            .map(|p| p.kind);
        match settings.ai.research_backend {
            ResearchBackend::Gemini => match (assignment, provider_kind) {
                (Some(a), Some(AiProviderKind::GoogleGemini)) => Some(a.model.clone()),
                _ => presets::GEMINI
                    .model_for(ModelRole::Research)
                    .map(str::to_string),
            },
            ResearchBackend::Claude => match (assignment, provider_kind) {
                (Some(a), Some(AiProviderKind::Anthropic)) => Some(a.model.clone()),
                _ => None,
            },
        }
    }

    /// The enabled Gemini provider whose key feeds the sidecar (the reserved
    /// `gemini` id wins over user-added Gemini providers).
    fn gemini_provider_id(settings: &Settings) -> Option<String> {
        let mut candidates: Vec<&AiProviderConfig> = settings
            .ai
            .providers
            .iter()
            .filter(|p| p.kind == AiProviderKind::GoogleGemini && p.enabled)
            .collect();
        candidates.sort_by_key(|p| p.id != presets::GEMINI_ID);
        candidates.first().map(|p| p.id.clone())
    }

    /// Spawn the sidecar for `request` and stream its events onto the bus.
    pub async fn start(self: &Arc<Self>, request: DeepResearchRequest) -> BlueyResult<()> {
        if !self.settings.get().ai.deep_research_enabled {
            return Err(BlueyError::configuration(
                "deep_research_disabled",
                "deep research is turned off in Settings → AI",
            ));
        }
        if Self::binary_path().is_none() {
            return Err(BlueyError::research(
                "agent_unavailable",
                "the research agent sidecar is not installed with this build",
            ));
        }
        if self.jobs.lock().contains_key(&request.job_id) {
            return Err(BlueyError::research(
                "job_already_running",
                "this job is already running",
            ));
        }
        let env = self.job_env().await?;
        let job_id = request.job_id.clone();
        let model = self.research_model();

        let command = self
            .app
            .shell()
            .sidecar(AGENT_BIN)
            .map_err(|e| {
                BlueyError::sidecar("spawn", format!("cannot resolve the agent sidecar: {e}"))
            })?
            .envs(env);
        let (rx, mut child) = command.spawn().map_err(|e| {
            BlueyError::sidecar("spawn", format!("cannot spawn the agent sidecar: {e}"))
        })?;

        let request_id = format!("r-{}", self.next_request_id.fetch_add(1, Ordering::SeqCst));
        let line = jsonl::encode_request(
            &request_id,
            "research.run",
            proto::research_run_params(&request, model.as_deref()),
        );
        if let Err(e) = child.write(format!("{line}\n").as_bytes()) {
            let _ = child.kill();
            return Err(BlueyError::sidecar(
                "write",
                format!("cannot talk to the agent sidecar: {e}"),
            ));
        }

        self.jobs.lock().insert(
            job_id.clone(),
            Job {
                child: Some(child),
                allowed_documents: request.allowed_document_ids.clone(),
                started: Instant::now(),
            },
        );
        self.bus
            .publish(BlueyEvent::ResearchEvent(DeepResearchEvent::Started {
                job_id: job_id.clone(),
            }));
        self.spawn_reader(job_id.clone(), rx);
        self.spawn_watchdog(job_id);
        Ok(())
    }

    fn spawn_reader(
        self: &Arc<Self>,
        job_id: String,
        mut rx: tauri::async_runtime::Receiver<CommandEvent>,
    ) {
        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            while let Some(event) = rx.recv().await {
                match event {
                    CommandEvent::Stdout(line) => {
                        let text = String::from_utf8_lossy(&line);
                        this.handle_line(&job_id, &text).await;
                    }
                    CommandEvent::Stderr(line) => {
                        let text = String::from_utf8_lossy(&line);
                        tracing::debug!(target: "bluey_agent", job = %job_id, "{}", text.trim_end());
                    }
                    CommandEvent::Error(error) => {
                        tracing::warn!(job = %job_id, %error, "agent process error");
                    }
                    CommandEvent::Terminated(payload) => {
                        tracing::debug!(job = %job_id, code = ?payload.code, "agent exited");
                        // A job that exits without a terminal event failed.
                        if this.jobs.lock().remove(&job_id).is_some() {
                            this.publish_failed(
                                &job_id,
                                BlueyError::research(
                                    "agent_exited",
                                    "the research agent exited before finishing",
                                ),
                            );
                        }
                        break;
                    }
                    _ => {}
                }
            }
        });
    }

    async fn handle_line(&self, job_id: &str, text: &str) {
        match jsonl::parse_line(text) {
            Ok(Incoming::Event { event, data }) => match proto::parse_agent_event(&event, data) {
                Some(AgentEvent::Research(research_event)) => {
                    let terminal = matches!(
                        research_event,
                        DeepResearchEvent::Completed { .. } | DeepResearchEvent::Failed { .. }
                    );
                    // `started` was already published when the process was spawned.
                    if !matches!(research_event, DeepResearchEvent::Started { .. }) {
                        self.bus.publish(BlueyEvent::ResearchEvent(research_event));
                    }
                    if terminal {
                        self.finish(job_id);
                    }
                }
                Some(AgentEvent::DocumentRequest {
                    request_id,
                    document_id,
                }) => {
                    self.answer_document_request(job_id, &request_id, &document_id)
                        .await
                }
                None => tracing::debug!(job = %job_id, %event, "unhandled agent event"),
            },
            Ok(Incoming::Response { id, result }) => {
                if let Err(wire) = result {
                    let error = wire.into_bluey();
                    tracing::warn!(job = %job_id, request = %id, code = %error.code, "agent rejected a request");
                    if self.jobs.lock().remove(job_id).is_some() {
                        self.publish_failed(job_id, error);
                    }
                }
            }
            Err(reason) => {
                tracing::debug!(job = %job_id, %reason, "ignoring non-protocol agent line")
            }
        }
    }

    /// Serve `document.request` from SQLite, enforcing the request allow-list.
    async fn answer_document_request(&self, job_id: &str, request_id: &str, document_id: &str) {
        let allowed = {
            let jobs = self.jobs.lock();
            jobs.get(job_id).map(|job| {
                job.allowed_documents
                    .as_ref()
                    .map(|ids| ids.iter().any(|id| id == document_id))
                    .unwrap_or(false)
            })
        };
        let text = match allowed {
            Some(true) => {
                let id = document_id.to_string();
                self.storage
                    .run(move |db| DocumentRepository::get_text(db, &id))
                    .await
                    .map_err(|e| e.message)
            }
            Some(false) => Err("document is not in the allow-list for this job".to_string()),
            None => Err("job is no longer running".to_string()),
        };
        let params = proto::document_response_params(
            request_id,
            document_id,
            text.as_deref().map_err(String::as_str),
        );
        self.write(job_id, "document.response", params);
    }

    fn write(&self, job_id: &str, method: &str, params: Value) {
        let id = format!("r-{}", self.next_request_id.fetch_add(1, Ordering::SeqCst));
        let line = jsonl::encode_request(&id, method, params);
        let mut jobs = self.jobs.lock();
        if let Some(child) = jobs.get_mut(job_id).and_then(|job| job.child.as_mut()) {
            if let Err(e) = child.write(format!("{line}\n").as_bytes()) {
                tracing::warn!(job = %job_id, method, error = %e, "agent stdin write failed");
            }
        }
    }

    fn finish(&self, job_id: &str) {
        // The process exits on its own after a terminal event; make sure it does.
        let jobs = self.jobs.clone();
        let job_id = job_id.to_string();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(CANCEL_GRACE).await;
            if let Some(job) = jobs.lock().remove(&job_id) {
                if let Some(child) = job.child {
                    let _ = child.kill();
                }
            }
        });
    }

    fn spawn_watchdog(self: &Arc<Self>, job_id: String) {
        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(JOB_WALL_CLOCK).await;
            let running = this
                .jobs
                .lock()
                .get(&job_id)
                .map(|job| job.started.elapsed() >= JOB_WALL_CLOCK)
                .unwrap_or(false);
            if running {
                tracing::warn!(job = %job_id, "research job hit the wall-clock cap");
                if this.cancel(&job_id).await {
                    this.publish_failed(
                        &job_id,
                        BlueyError::research(
                            "timeout",
                            "the research job took too long and was stopped",
                        ),
                    );
                }
            }
        });
    }

    fn publish_failed(&self, job_id: &str, error: BlueyError) {
        self.bus
            .publish(BlueyEvent::ResearchEvent(DeepResearchEvent::Failed {
                job_id: job_id.to_string(),
                error,
            }));
    }

    /// Ask the job to stop, then kill it after a grace period. Returns whether
    /// the job was known.
    pub async fn cancel(&self, job_id: &str) -> bool {
        if !self.jobs.lock().contains_key(job_id) {
            return false;
        }
        self.write(
            job_id,
            "research.cancel",
            proto::research_cancel_params(job_id),
        );
        let jobs = self.jobs.clone();
        let id = job_id.to_string();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(CANCEL_GRACE).await;
            if let Some(job) = jobs.lock().remove(&id) {
                if let Some(child) = job.child {
                    let _ = child.kill();
                }
            }
        });
        true
    }

    /// Kill every running job (app exit).
    pub fn shutdown(&self) {
        let jobs: Vec<Job> = self.jobs.lock().drain().map(|(_, job)| job).collect();
        for job in jobs {
            if let Some(child) = job.child {
                let _ = child.kill();
            }
        }
    }
}

fn push_passthrough(env: &mut Vec<(String, String)>, names: &[&str]) {
    for name in names {
        if let Ok(value) = std::env::var(name) {
            if !value.is_empty() {
                env.push(((*name).to_string(), value));
            }
        }
    }
}

fn env_present(name: &str) -> bool {
    std::env::var(name)
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false)
}

fn env_truthy(name: &str) -> bool {
    std::env::var(name)
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(false)
}
