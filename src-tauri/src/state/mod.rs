//! Managed application state: the state-machine hub, the [`AppCore`] bundle of
//! managers handed to every command, the latency metrics recorder and the
//! developer-mode simulation knobs.

use std::sync::Arc;

use bluey_core::events::BlueyEvent;
use bluey_core::state::AppStateMachine;
use bluey_core::types::{AppEvent, AppState, AppStatus, LatencyMetrics};
use bluey_core::{now_iso, BlueyError, BlueyResult};

use crate::events::EventBus;

/// The state machine plus its bus: every accepted transition publishes
/// `app.state`. All managers share one hub.
pub struct StateHub {
    machine: parking_lot::Mutex<AppStateMachine>,
    bus: Arc<EventBus>,
}

impl StateHub {
    pub fn new(mode_id: impl Into<String>, bus: Arc<EventBus>) -> Self {
        Self {
            machine: parking_lot::Mutex::new(AppStateMachine::new(mode_id)),
            bus,
        }
    }

    /// Current status snapshot.
    pub fn status(&self) -> AppStatus {
        self.machine.lock().status().clone()
    }

    /// Current pipeline state.
    pub fn state(&self) -> AppState {
        self.machine.lock().state()
    }

    /// Whether listening is active.
    pub fn audio_active(&self) -> bool {
        self.machine.lock().audio_active()
    }

    /// Apply a transition; on success the new status is published as
    /// `app.state` and returned. Rejected transitions become `BlueyError`s.
    pub fn transition(&self, event: AppEvent) -> BlueyResult<AppStatus> {
        let result = self.machine.lock().transition(event);
        match result {
            Ok(status) => {
                self.bus.publish(BlueyEvent::AppState(status.clone()));
                Ok(status)
            }
            Err(e) => Err(BlueyError::from(e)),
        }
    }

    /// Apply a transition, ignoring rejections (used on pipeline edges where a
    /// concurrent actor may already have moved the machine). Returns the new
    /// status when the transition was accepted.
    pub fn transition_soft(&self, event: AppEvent) -> Option<AppStatus> {
        let result = self.machine.lock().transition(event);
        match result {
            Ok(status) => {
                self.bus.publish(BlueyEvent::AppState(status.clone()));
                Some(status)
            }
            Err(e) => {
                tracing::debug!(event = %e.event, from = ?e.from, "state transition skipped");
                None
            }
        }
    }
}

/// Rolling latency metrics for the dev overlay. Every update publishes
/// `dev.metrics`.
pub struct MetricsRecorder {
    metrics: parking_lot::Mutex<LatencyMetrics>,
    bus: Arc<EventBus>,
}

impl MetricsRecorder {
    pub fn new(bus: Arc<EventBus>) -> Self {
        Self {
            metrics: parking_lot::Mutex::new(LatencyMetrics {
                updated_at: now_iso(),
                ..LatencyMetrics::default()
            }),
            bus,
        }
    }

    /// Current snapshot.
    pub fn snapshot(&self) -> LatencyMetrics {
        self.metrics.lock().clone()
    }

    /// Merge one partial update (only `Some` fields overwrite) and publish.
    pub fn update(&self, apply: impl FnOnce(&mut LatencyMetrics)) {
        let snapshot = {
            let mut m = self.metrics.lock();
            apply(&mut m);
            m.updated_at = now_iso();
            m.clone()
        };
        self.bus.publish(BlueyEvent::DevMetrics(snapshot));
    }

    /// Record context-assembly timings (`capture`/`ocr`/`accessibility`/
    /// `transcript`/`assembly` keys from the snapshot builder).
    pub fn record_context(&self, timings: &std::collections::BTreeMap<String, u64>) {
        self.update(|m| {
            if let Some(v) = timings.get("capture") {
                m.capture_ms = Some(*v);
            }
            if let Some(v) = timings.get("ocr") {
                m.ocr_ms = Some(*v);
            }
            if let Some(v) = timings.get("accessibility") {
                m.accessibility_ms = Some(*v);
            }
            if let Some(v) = timings.get("transcript") {
                m.transcript_ms = Some(*v);
            }
            if let Some(v) = timings.get("assembly") {
                m.context_assembly_ms = Some(*v);
            }
        });
    }
}

/// Developer-mode simulation knobs (`dev_simulate`). Read by the mock AI
/// provider; cleared with `DevSimulation::Clear`.
#[derive(Default)]
pub struct DevState {
    inner: parking_lot::Mutex<DevKnobs>,
}

#[derive(Default, Clone)]
pub struct DevKnobs {
    /// Extra latency between mock stream chunks.
    pub ai_latency_ms: Option<u64>,
    /// Force the next mock AI requests to fail with this code.
    pub ai_failure_code: Option<String>,
}

impl DevState {
    pub fn knobs(&self) -> DevKnobs {
        self.inner.lock().clone()
    }

    pub fn set_latency(&self, ms: u64) {
        self.inner.lock().ai_latency_ms = Some(ms);
    }

    pub fn set_failure(&self, code: Option<String>) {
        self.inner.lock().ai_failure_code = Some(code.unwrap_or_else(|| "simulated".to_string()));
    }

    pub fn clear(&self) {
        *self.inner.lock() = DevKnobs::default();
    }
}

/// Everything a command can reach, managed once via `app.manage(AppCore)`.
pub struct AppCore {
    pub paths: Arc<crate::storage::AppPaths>,
    pub bus: Arc<EventBus>,
    pub hub: Arc<StateHub>,
    pub storage: Arc<crate::storage::Storage>,
    pub settings: Arc<crate::settings::SettingsManager>,
    pub secrets: Arc<crate::secrets::SecretsStore>,
    pub metrics: Arc<MetricsRecorder>,
    pub dev: Arc<DevState>,
    pub helper: Arc<crate::sidecar::HelperClient>,
    pub permissions: Arc<crate::permissions::PermissionManager>,
    pub ax: Arc<crate::accessibility::AxManager>,
    pub modes: Arc<crate::modes::ModeManager>,
    pub sessions: Arc<crate::sessions::SessionManager>,
    pub capture: Arc<crate::capture::CaptureManager>,
    pub audio: Arc<crate::audio::AudioManager>,
    pub ai: Arc<crate::ai::AiManager>,
    pub agent: Arc<crate::agent::AgentManager>,
    pub research: Arc<crate::research::ResearchManager>,
    pub documents: Arc<crate::documents::DocumentsManager>,
    pub auth: Arc<crate::auth::AuthManager>,
    pub accounts: Arc<crate::accounts::AccountsManager>,
    pub panel: Arc<crate::overlay::PanelManager>,
    pub shortcuts: Arc<crate::shortcuts::ShortcutManager>,
}

impl AppCore {
    /// Convenience: the error used when a manager needs the app to be fully
    /// booted but it is not (should never surface in practice).
    pub fn not_ready() -> BlueyError {
        BlueyError::internal("application core is not initialised yet")
    }
}
