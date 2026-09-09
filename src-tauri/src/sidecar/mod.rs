//! Native helper sidecar client (`bluey-helper`): JSON-Lines request/response
//! over stdio with per-method timeouts, typed event fan-out and crash restart
//! with exponential backoff (max 5 attempts, reset after 60 s healthy).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bluey_core::events::BlueyEvent;
use bluey_core::{BlueyError, BlueyResult};
use bluey_protocols::helper::{parse_helper_event, HelperEvent, HelperVersion};
use bluey_protocols::jsonl::{self, Incoming};
use serde_json::Value;
use tauri::AppHandle;
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
use tauri_plugin_shell::ShellExt;
use tokio::sync::{broadcast, oneshot};

use crate::events::EventBus;

/// Sidecar binary name (matches `bundle.externalBin`).
const HELPER_BIN: &str = "bluey-helper";
const MAX_RESTARTS: u32 = 5;
const HEALTHY_AFTER: Duration = Duration::from_secs(60);

type Pending = Arc<parking_lot::Mutex<HashMap<String, oneshot::Sender<BlueyResult<Value>>>>>;

/// Client for the native Swift helper.
pub struct HelperClient {
    app: AppHandle,
    bus: Arc<EventBus>,
    child: parking_lot::Mutex<Option<CommandChild>>,
    pending: Pending,
    events: broadcast::Sender<HelperEvent>,
    running: AtomicBool,
    desired: AtomicBool,
    restart_on_crash: AtomicBool,
    version: parking_lot::Mutex<Option<HelperVersion>>,
    next_id: AtomicU64,
    generation: AtomicU64,
    restart_attempts: AtomicU32,
    spawn_lock: tokio::sync::Mutex<()>,
}

impl HelperClient {
    pub fn new(app: AppHandle, bus: Arc<EventBus>, restart_on_crash: bool) -> Self {
        let (events, _) = broadcast::channel(1024);
        Self {
            app,
            bus,
            child: parking_lot::Mutex::new(None),
            pending: Arc::new(parking_lot::Mutex::new(HashMap::new())),
            events,
            running: AtomicBool::new(false),
            desired: AtomicBool::new(false),
            restart_on_crash: AtomicBool::new(restart_on_crash),
            version: parking_lot::Mutex::new(None),
            next_id: AtomicU64::new(1),
            generation: AtomicU64::new(0),
            restart_attempts: AtomicU32::new(0),
            spawn_lock: tokio::sync::Mutex::new(()),
        }
    }

    /// Subscribe to typed helper events (audio, transcript, screen…).
    pub fn subscribe(&self) -> broadcast::Receiver<HelperEvent> {
        self.events.subscribe()
    }

    /// Whether the helper process is running.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// The helper version from the handshake, if running.
    pub fn version(&self) -> Option<HelperVersion> {
        self.version.lock().clone()
    }

    pub fn set_restart_on_crash(&self, enabled: bool) {
        self.restart_on_crash.store(enabled, Ordering::SeqCst);
    }

    /// Spawn the helper if it is not running (idempotent, serialised).
    pub async fn ensure_running(self: &Arc<Self>) -> BlueyResult<()> {
        if self.is_running() {
            return Ok(());
        }
        let _guard = self.spawn_lock.lock().await;
        if self.is_running() {
            return Ok(());
        }
        self.desired.store(true, Ordering::SeqCst);
        self.spawn_once().await
    }

    async fn spawn_once(self: &Arc<Self>) -> BlueyResult<()> {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        // The helper needs no configuration from the environment; a cleared
        // environment keeps `.env` keys loaded into Bluey's process away from it.
        let command = self
            .app
            .shell()
            .sidecar(HELPER_BIN)
            .map_err(|e| BlueyError::sidecar("spawn", format!("cannot resolve helper: {e}")))?
            .env_clear()
            .envs(child_base_env());
        let (rx, child) = command
            .spawn()
            .map_err(|e| BlueyError::sidecar("spawn", format!("cannot spawn helper: {e}")))?;
        *self.child.lock() = Some(child);
        self.running.store(true, Ordering::SeqCst);

        let (ready_tx, ready_rx) = oneshot::channel::<()>();
        self.spawn_reader(rx, generation, ready_tx);

        // Wait for `helper.ready`, then do the version handshake.
        let ready = tokio::time::timeout(Duration::from_secs(5), ready_rx).await;
        if ready.is_err() {
            tracing::warn!("helper did not report ready in time; probing with helper.version");
        }
        match self.request("helper.version", Value::Null).await {
            Ok(value) => {
                let version: HelperVersion = serde_json::from_value(value).unwrap_or_default();
                if !version.is_compatible() {
                    let error = BlueyError::sidecar(
                        "protocol",
                        format!(
                            "helper protocol {} is incompatible (need {})",
                            version.protocol,
                            bluey_protocols::helper::PROTOCOL_MAJOR
                        ),
                    );
                    self.publish_status(false, None, Some(error.clone()), false);
                    self.desired.store(false, Ordering::SeqCst);
                    self.kill();
                    return Err(error);
                }
                let version_string = version.version.clone();
                *self.version.lock() = Some(version);
                self.publish_status(true, Some(version_string), None, false);
                Ok(())
            }
            Err(e) => {
                self.kill();
                Err(e)
            }
        }
    }

    fn spawn_reader(
        self: &Arc<Self>,
        mut rx: tauri::async_runtime::Receiver<CommandEvent>,
        generation: u64,
        ready_tx: oneshot::Sender<()>,
    ) {
        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            let mut ready_tx = Some(ready_tx);
            let started = Instant::now();
            while let Some(event) = rx.recv().await {
                match event {
                    CommandEvent::Stdout(line) => {
                        let line = String::from_utf8_lossy(&line);
                        match jsonl::parse_line(&line) {
                            Ok(Incoming::Response { id, result }) => {
                                let sender = this.pending.lock().remove(&id);
                                if let Some(sender) = sender {
                                    let _ =
                                        sender.send(result.map_err(jsonl::WireError::into_bluey));
                                }
                            }
                            Ok(Incoming::Event { event, data }) => {
                                let typed = parse_helper_event(&event, data);
                                if let HelperEvent::Ready { version } = &typed {
                                    tracing::info!(version = ?version, "helper ready");
                                    if let Some(tx) = ready_tx.take() {
                                        let _ = tx.send(());
                                    }
                                }
                                this.handle_event(typed);
                            }
                            Err(reason) => {
                                tracing::debug!(%reason, "ignoring non-protocol helper line");
                            }
                        }
                    }
                    CommandEvent::Stderr(line) => {
                        let line = String::from_utf8_lossy(&line);
                        tracing::debug!(target: "bluey_helper", "{}", line.trim_end());
                    }
                    CommandEvent::Error(error) => {
                        tracing::warn!(%error, "helper process error");
                    }
                    CommandEvent::Terminated(payload) => {
                        tracing::warn!(code = ?payload.code, "helper exited");
                        this.on_terminated(generation, started).await;
                        break;
                    }
                    _ => {}
                }
            }
        });
    }

    /// Typed event dispatch: update internal state, publish contract events,
    /// fan out to subscribers (audio/capture managers).
    fn handle_event(&self, event: HelperEvent) {
        match &event {
            HelperEvent::ScreenChanged {
                hash,
                delta,
                display_id,
                at,
            } => {
                self.bus.publish(BlueyEvent::ScreenChanged {
                    hash: hash.clone(),
                    delta: *delta,
                    display_id: display_id.clone(),
                    at: if at.is_empty() {
                        bluey_core::now_iso()
                    } else {
                        at.clone()
                    },
                });
            }
            HelperEvent::Unknown { event } => {
                tracing::debug!(%event, "unhandled helper event");
            }
            _ => {}
        }
        let _ = self.events.send(event);
    }

    async fn on_terminated(self: &Arc<Self>, generation: u64, started: Instant) {
        // Ignore stale readers of an already-replaced process.
        if self.generation.load(Ordering::SeqCst) != generation {
            return;
        }
        self.running.store(false, Ordering::SeqCst);
        *self.child.lock() = None;
        *self.version.lock() = None;
        self.fail_pending(BlueyError::sidecar("crashed", "the native helper exited"));

        let should_restart =
            self.desired.load(Ordering::SeqCst) && self.restart_on_crash.load(Ordering::SeqCst);
        if !should_restart {
            self.publish_status(false, None, None, false);
            return;
        }
        if started.elapsed() >= HEALTHY_AFTER {
            self.restart_attempts.store(0, Ordering::SeqCst);
        }
        let attempts = self.restart_attempts.fetch_add(1, Ordering::SeqCst) + 1;
        if attempts > MAX_RESTARTS {
            let error = BlueyError::sidecar(
                "crash_loop",
                "the native helper keeps crashing; restart it from Settings",
            );
            self.publish_status(false, None, Some(error.clone()), false);
            self.bus.publish(BlueyEvent::AppError(error));
            self.desired.store(false, Ordering::SeqCst);
            return;
        }
        self.publish_status(false, None, None, true);
        let delay = Duration::from_millis(500u64.saturating_mul(1 << (attempts - 1).min(6)));
        tracing::info!(attempt = attempts, ?delay, "restarting helper");
        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(delay).await;
            if !this.desired.load(Ordering::SeqCst) {
                return;
            }
            let _guard = this.spawn_lock.lock().await;
            if this.is_running() {
                return;
            }
            if let Err(e) = this.spawn_once().await {
                tracing::warn!(error = %e, "helper restart failed");
            } else {
                this.publish_status(true, this.version().map(|v| v.version), None, true);
            }
        });
    }

    fn fail_pending(&self, error: BlueyError) {
        let pending: Vec<_> = self.pending.lock().drain().collect();
        for (_, sender) in pending {
            let _ = sender.send(Err(error.clone()));
        }
    }

    fn publish_status(
        &self,
        running: bool,
        version: Option<String>,
        error: Option<BlueyError>,
        restarted: bool,
    ) {
        self.bus.publish(BlueyEvent::HelperStatus {
            running,
            version,
            restarted: restarted.then_some(true),
            error,
        });
    }

    /// Send one request and await its response (per-method timeout).
    pub async fn request(&self, method: &str, params: Value) -> BlueyResult<Value> {
        if !self.is_running() {
            return Err(BlueyError::sidecar(
                "not_running",
                "the native helper is not running",
            ));
        }
        let id = format!("r-{}", self.next_id.fetch_add(1, Ordering::SeqCst));
        let (tx, rx) = oneshot::channel();
        self.pending.lock().insert(id.clone(), tx);

        let line = jsonl::encode_request(&id, method, params);
        let write_result = {
            let mut guard = self.child.lock();
            match guard.as_mut() {
                Some(child) => child
                    .write(format!("{line}\n").as_bytes())
                    .map_err(|e| BlueyError::sidecar("write", format!("stdin write failed: {e}"))),
                None => Err(BlueyError::sidecar(
                    "not_running",
                    "the native helper is not running",
                )),
            }
        };
        if let Err(e) = write_result {
            self.pending.lock().remove(&id);
            return Err(e);
        }

        match tokio::time::timeout(timeout_for(method), rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(BlueyError::sidecar(
                "closed",
                "the helper closed unexpectedly",
            )),
            Err(_) => {
                self.pending.lock().remove(&id);
                Err(BlueyError::sidecar(
                    "timeout",
                    format!("helper call `{method}` timed out"),
                ))
            }
        }
    }

    /// Spawn-if-needed convenience wrapper around [`Self::request`].
    pub async fn call(self: &Arc<Self>, method: &str, params: Value) -> BlueyResult<Value> {
        self.ensure_running().await?;
        self.request(method, params).await
    }

    /// Graceful stop: `helper.shutdown` then kill.
    pub async fn shutdown(&self) {
        self.desired.store(false, Ordering::SeqCst);
        if self.is_running() {
            let _ = tokio::time::timeout(
                Duration::from_millis(750),
                self.request("helper.shutdown", Value::Null),
            )
            .await;
        }
        self.kill();
        self.publish_status(false, None, None, false);
    }

    /// Kill + respawn (used by `dev_restart_helper`).
    pub async fn restart(self: &Arc<Self>) -> BlueyResult<()> {
        {
            let _guard = self.spawn_lock.lock().await;
            self.desired.store(true, Ordering::SeqCst);
            self.restart_attempts.store(0, Ordering::SeqCst);
            self.kill();
        }
        self.ensure_running().await
    }

    fn kill(&self) {
        // Bump the generation so the old reader's Terminated event is ignored.
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.running.store(false, Ordering::SeqCst);
        if let Some(child) = self.child.lock().take() {
            let _ = child.kill();
        }
        self.fail_pending(BlueyError::sidecar("stopped", "the native helper stopped"));
        *self.version.lock() = None;
    }
}

/// Per-method timeouts (spec: capture 3 s, ocr 5 s, ax 1 s, audio.start 5 s,
/// default 2 s; permission prompts and mic tests need longer).
fn timeout_for(method: &str) -> Duration {
    match method {
        m if m.starts_with("capture.") => Duration::from_secs(3),
        "ocr.recognize" => Duration::from_secs(5),
        "accessibility.snapshot" => Duration::from_secs(1),
        "audio.start" => Duration::from_secs(5),
        "audio.testMicrophone" => Duration::from_secs(8),
        "permissions.request" => Duration::from_secs(120),
        _ => Duration::from_secs(2),
    }
}

/// Variables a child process needs to run at all. Everything else — in
/// particular the API keys `load_dotenv` puts into Bluey's own environment — is
/// withheld from sidecars; whatever a child must know is passed explicitly.
pub const CHILD_BASE_ENV: &[&str] = &[
    "PATH", "HOME", "TMPDIR", "USER", "LOGNAME", "LANG", "LC_ALL",
];

/// The minimal environment for a spawned sidecar: [`CHILD_BASE_ENV`] copied
/// from Bluey's environment when set.
pub fn child_base_env() -> Vec<(String, String)> {
    CHILD_BASE_ENV
        .iter()
        .filter_map(|name| {
            std::env::var(name)
                .ok()
                .map(|value| (name.to_string(), value))
        })
        .collect()
}
