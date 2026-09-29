//! Research-agent sidecar client (`bluey-agent`, ADR 0004): one process per
//! job, JSON-Lines over stdio (`bluey_protocols::{jsonl, agent}`), credentials
//! injected from the Keychain into the child environment only, document reads
//! served from SQLite for the request's allow-list, cancellation and a hard
//! wall-clock cap (the job table in [`jobs`]). Events are mapped onto
//! `research.event`.

mod jobs;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use bluey_core::presets;
use bluey_core::types::{
    AiProviderConfig, AiProviderKind, DeepResearchRequest, ModelRole, ResearchBackend, Settings,
};
use bluey_core::{BlueyError, BlueyResult};
use bluey_protocols::agent::{self as proto, AgentInfo};
use bluey_protocols::jsonl::{self, Incoming};
use tauri::AppHandle;
use tauri_plugin_shell::process::CommandEvent;
use tauri_plugin_shell::ShellExt;
use tokio::sync::OnceCell;

use crate::events::EventBus;
use crate::secrets::{provider_key, SecretsStore, AGENT_ANTHROPIC_KEY, EXA_KEY, FIRECRAWL_KEY};
use crate::settings::SettingsManager;
use crate::storage::Storage;
use jobs::{AgentJobs, JobTiming};

/// Sidecar binary name (matches `bundle.externalBin`).
pub const AGENT_BIN: &str = "bluey-agent";
/// Hard cap per job; the TS caller enforces its own (shorter) timeout.
const JOB_WALL_CLOCK: Duration = Duration::from_secs(10 * 60);
/// Grace period between `research.cancel` and killing the process.
const CANCEL_GRACE: Duration = Duration::from_secs(2);
/// How long the one-off `agent.info` probe may take.
const INFO_TIMEOUT: Duration = Duration::from_secs(5);
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

pub struct AgentManager {
    app: AppHandle,
    secrets: Arc<SecretsStore>,
    settings: Arc<SettingsManager>,
    jobs: Arc<AgentJobs>,
    /// What the installed sidecar build supports (`agent.info`), asked once.
    info: OnceCell<AgentInfo>,
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
            secrets,
            settings,
            jobs: Arc::new(AgentJobs::new(
                bus,
                storage,
                JobTiming {
                    cancel_grace: CANCEL_GRACE,
                    wall_clock: JOB_WALL_CLOCK,
                },
            )),
            info: OnceCell::new(),
        }
    }

    /// Where Tauri places the sidecar next to the app executable.
    pub fn binary_path() -> Option<PathBuf> {
        let exe = std::env::current_exe().ok()?;
        let path = exe.parent()?.join(AGENT_BIN);
        path.is_file().then_some(path)
    }

    /// Whether the sidecar binary exists, the selected backend has a model
    /// credential, and the installed build can run that backend (the lite
    /// build has no Claude Code CLI).
    pub async fn available(&self) -> bool {
        if Self::binary_path().is_none() {
            return false;
        }
        let settings = self.settings.get();
        let backend = settings.ai.research_backend;
        let has_credential = match backend {
            ResearchBackend::Gemini => match Self::gemini_provider_id(&settings) {
                Some(id) => self.secrets.has(&provider_key(&id)).await.unwrap_or(false),
                None => false,
            },
            ResearchBackend::Claude => {
                if env_truthy("CLAUDE_CODE_USE_FOUNDRY") {
                    env_present("ANTHROPIC_FOUNDRY_API_KEY")
                        || env_present("ANTHROPIC_FOUNDRY_AUTH_TOKEN")
                        || env_present("AZURE_FOUNDRY_API_KEY")
                } else {
                    self.secrets.has(AGENT_ANTHROPIC_KEY).await.unwrap_or(false)
                        || env_present("ANTHROPIC_API_KEY")
                }
            }
        };
        has_credential && backend_supported(self.info().await.as_ref(), backend)
    }

    /// Research backends the installed sidecar can run (empty when it is not
    /// installed or did not answer).
    pub async fn supported_backends(&self) -> Vec<ResearchBackend> {
        if Self::binary_path().is_none() {
            return Vec::new();
        }
        self.info()
            .await
            .map(|info| info.backends)
            .unwrap_or_default()
    }

    /// The sidecar's `agent.info`, probed once per run; a failed probe is not
    /// cached (the next call asks again).
    async fn info(&self) -> Option<AgentInfo> {
        self.info
            .get_or_try_init(|| async { self.probe_info().await.ok_or(()) })
            .await
            .ok()
            .cloned()
    }

    /// Spawn the sidecar with no credentials, ask `agent.info`, and kill it.
    async fn probe_info(&self) -> Option<AgentInfo> {
        let mut env = crate::sidecar::child_base_env();
        // A dev `BLUEY_CLAUDE_CLI` makes Claude runnable even on the lite build.
        push_passthrough(&mut env, &["BLUEY_CLAUDE_CLI"]);
        let command = self
            .app
            .shell()
            .sidecar(AGENT_BIN)
            .ok()?
            .env_clear()
            .envs(env);
        let (mut rx, mut child) = command.spawn().ok()?;
        let line = jsonl::encode_request("info-1", "agent.info", serde_json::json!({}));
        if child.write(format!("{line}\n").as_bytes()).is_err() {
            let _ = child.kill();
            return None;
        }
        let answer = tokio::time::timeout(INFO_TIMEOUT, async {
            while let Some(event) = rx.recv().await {
                match event {
                    CommandEvent::Stdout(line) => {
                        match jsonl::parse_line(&String::from_utf8_lossy(&line)) {
                            Ok(Incoming::Response { result, .. }) => {
                                return result.ok().and_then(proto::parse_agent_info)
                            }
                            _ => continue,
                        }
                    }
                    CommandEvent::Terminated(_) => return None,
                    _ => {}
                }
            }
            None
        })
        .await
        .ok()
        .flatten();
        let _ = child.kill();
        if answer.is_none() {
            tracing::warn!("the research agent did not answer agent.info");
        }
        answer
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
                match self.secrets.get(AGENT_ANTHROPIC_KEY).await? {
                    Some(key) => env.push(("ANTHROPIC_API_KEY".into(), key)),
                    // The sidecar runs with a cleared environment, so a key that
                    // only lives in Bluey's `.env` must be forwarded explicitly.
                    None => push_passthrough(&mut env, &["ANTHROPIC_API_KEY"]),
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
        // The job sends the query and allow-listed documents to a model provider.
        crate::ai::ensure_cloud_ai(&self.settings.get())?;
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
        if self.jobs.is_running(&request.job_id) {
            return Err(BlueyError::research(
                "job_already_running",
                "this job is already running",
            ));
        }
        // Exactly one backend's credentials (ADR 0007): the sidecar gets a
        // cleared environment plus what `job_env` decided — never Bluey's own
        // environment with every `.env` key in it.
        let mut env = crate::sidecar::child_base_env();
        env.extend(self.job_env().await?);
        let model = self.research_model();

        let command = self
            .app
            .shell()
            .sidecar(AGENT_BIN)
            .map_err(|e| {
                BlueyError::sidecar("spawn", format!("cannot resolve the agent sidecar: {e}"))
            })?
            .env_clear()
            .envs(env);
        let (rx, child) = command.spawn().map_err(|e| {
            BlueyError::sidecar("spawn", format!("cannot spawn the agent sidecar: {e}"))
        })?;
        self.jobs.launch(
            request.job_id.clone(),
            proto::research_run_params(&request, model.as_deref()),
            request.allowed_document_ids.clone(),
            Box::new(child),
            rx,
        )
    }

    /// Ask the job to stop; the job table publishes `failed{cancelled}` if the
    /// sidecar does not finish within the grace period. Returns whether the
    /// job was known.
    pub async fn cancel(&self, job_id: &str) -> bool {
        self.jobs.cancel(job_id)
    }

    /// Kill every running job (app exit).
    pub fn shutdown(&self) {
        self.jobs.shutdown();
    }
}

/// Whether the installed build can run `backend`. Both builds run Gemini
/// (pure JS); Claude needs the CLI, which only the full build (or a dev
/// `BLUEY_CLAUDE_CLI`) has — an unknown build is not trusted with it.
fn backend_supported(info: Option<&AgentInfo>, backend: ResearchBackend) -> bool {
    match backend {
        ResearchBackend::Gemini => true,
        ResearchBackend::Claude => info.is_some_and(|info| info.backends.contains(&backend)),
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

#[cfg(test)]
mod env_boundary_tests {
    use super::*;

    /// ADR 0009: subscription-account tokens never reach a child process. The
    /// sidecar environment is built from these fixed names plus provider API
    /// keys; none of them may name an OAuth or account credential.
    #[test]
    fn sidecar_env_never_names_account_tokens() {
        for name in CLAUDE_PASSTHROUGH_ENV
            .iter()
            .chain(COMMON_PASSTHROUGH_ENV.iter())
        {
            let lower = name.to_ascii_lowercase();
            assert!(
                !lower.contains("oauth") && !lower.contains("account"),
                "{name} would forward account material to the sidecar"
            );
        }
    }

    #[test]
    fn claude_needs_a_build_that_reports_it() {
        let lite = AgentInfo {
            variant: "lite".into(),
            backends: vec![ResearchBackend::Gemini],
        };
        let full = AgentInfo {
            variant: "full".into(),
            backends: vec![ResearchBackend::Gemini, ResearchBackend::Claude],
        };
        assert!(!backend_supported(Some(&lite), ResearchBackend::Claude));
        assert!(backend_supported(Some(&full), ResearchBackend::Claude));
        assert!(!backend_supported(None, ResearchBackend::Claude));
        assert!(backend_supported(Some(&lite), ResearchBackend::Gemini));
        assert!(backend_supported(None, ResearchBackend::Gemini));
    }
}
