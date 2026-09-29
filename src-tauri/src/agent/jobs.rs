//! The agent job table: one entry per running sidecar process, the stdout
//! reader that maps protocol lines onto `research.event`, `document.request`
//! answers, cancellation with a grace period and the wall-clock watchdog.
//!
//! It only needs a line-writable, killable process and its event stream, so
//! the app drives it with the Tauri `CommandChild` and tests drive it with a
//! scripted fake sidecar. Every job ends with exactly one terminal event
//! (`completed` / `failed`) on the bus, whoever ends it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use bluey_core::events::BlueyEvent;
use bluey_core::types::DeepResearchEvent;
use bluey_core::{BlueyError, BlueyResult};
use bluey_protocols::agent::{self as proto, AgentEvent};
use bluey_protocols::jsonl::{self, Incoming};
use bluey_storage::DocumentRepository;
use serde_json::Value;
use tauri::async_runtime::Receiver;
use tauri_plugin_shell::process::{CommandChild, CommandEvent};

use crate::events::EventBus;
use crate::storage::Storage;

/// A running sidecar process as the job table sees it.
pub(crate) trait AgentProcess: Send + 'static {
    /// Write one protocol line (the newline is appended here).
    fn write_line(&mut self, line: &str) -> Result<(), String>;
    /// Kill the process (best effort).
    fn kill(self: Box<Self>);
}

impl AgentProcess for CommandChild {
    fn write_line(&mut self, line: &str) -> Result<(), String> {
        self.write(format!("{line}\n").as_bytes())
            .map_err(|e| e.to_string())
    }

    fn kill(self: Box<Self>) {
        let _ = CommandChild::kill(*self);
    }
}

/// Where `document.request` text comes from (SQLite in the app).
#[async_trait]
pub(crate) trait DocumentSource: Send + Sync {
    async fn document_text(&self, document_id: &str) -> Result<String, String>;
}

#[async_trait]
impl DocumentSource for Storage {
    async fn document_text(&self, document_id: &str) -> Result<String, String> {
        let id = document_id.to_string();
        self.run(move |db| DocumentRepository::get_text(db, &id))
            .await
            .map_err(|e| e.message)
    }
}

/// Timing knobs (constants in the app, short in tests).
#[derive(Debug, Clone, Copy)]
pub(crate) struct JobTiming {
    /// Between `research.cancel` (or a terminal event) and killing the process.
    pub cancel_grace: Duration,
    /// Hard cap per job; the TS caller enforces its own (shorter) timeout.
    pub wall_clock: Duration,
}

struct Job {
    process: Option<Box<dyn AgentProcess>>,
    allowed_documents: Option<Vec<String>>,
    started: Instant,
}

pub(crate) struct AgentJobs {
    bus: Arc<EventBus>,
    documents: Arc<dyn DocumentSource>,
    timing: JobTiming,
    jobs: parking_lot::Mutex<HashMap<String, Job>>,
    next_request_id: AtomicU64,
}

impl AgentJobs {
    pub(crate) fn new(
        bus: Arc<EventBus>,
        documents: Arc<dyn DocumentSource>,
        timing: JobTiming,
    ) -> Self {
        Self {
            bus,
            documents,
            timing,
            jobs: parking_lot::Mutex::new(HashMap::new()),
            next_request_id: AtomicU64::new(1),
        }
    }

    pub(crate) fn is_running(&self, job_id: &str) -> bool {
        self.jobs.lock().contains_key(job_id)
    }

    fn next_id(&self) -> String {
        format!("r-{}", self.next_request_id.fetch_add(1, Ordering::SeqCst))
    }

    /// Hand `research.run` to a freshly spawned process, then track it:
    /// publish `started`, read its events and arm the watchdog.
    pub(crate) fn launch(
        self: &Arc<Self>,
        job_id: String,
        run_params: Value,
        allowed_documents: Option<Vec<String>>,
        mut process: Box<dyn AgentProcess>,
        rx: Receiver<CommandEvent>,
    ) -> BlueyResult<()> {
        let line = jsonl::encode_request(&self.next_id(), "research.run", run_params);
        if let Err(e) = process.write_line(&line) {
            process.kill();
            return Err(BlueyError::sidecar(
                "write",
                format!("cannot talk to the agent sidecar: {e}"),
            ));
        }
        self.jobs.lock().insert(
            job_id.clone(),
            Job {
                process: Some(process),
                allowed_documents,
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

    fn spawn_reader(self: &Arc<Self>, job_id: String, mut rx: Receiver<CommandEvent>) {
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
                        if this.forget(&job_id) {
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
                    if !self.is_running(job_id) {
                        tracing::debug!(job = %job_id, "ignoring an event for a finished job");
                        return;
                    }
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
                    if self.forget(job_id) {
                        self.publish_failed(job_id, error);
                    }
                }
            }
            Err(reason) => {
                tracing::debug!(job = %job_id, %reason, "ignoring non-protocol agent line")
            }
        }
    }

    /// Serve `document.request` from the document source, enforcing the
    /// request allow-list.
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
            Some(true) => self.documents.document_text(document_id).await,
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
        let line = jsonl::encode_request(&self.next_id(), method, params);
        let mut jobs = self.jobs.lock();
        if let Some(process) = jobs.get_mut(job_id).and_then(|job| job.process.as_mut()) {
            if let Err(e) = process.write_line(&line) {
                tracing::warn!(job = %job_id, method, error = %e, "agent stdin write failed");
            }
        }
    }

    /// Drop the job from the table; true when it was still running. Whoever
    /// removes it owns the terminal event, so exactly one is ever published.
    fn forget(&self, job_id: &str) -> bool {
        self.jobs.lock().remove(job_id).is_some()
    }

    /// Remove the job and kill its process after the grace period.
    fn take_and_reap(&self, job_id: &str) -> bool {
        let Some(job) = self.jobs.lock().remove(job_id) else {
            return false;
        };
        if let Some(process) = job.process {
            let grace = self.timing.cancel_grace;
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(grace).await;
                process.kill();
            });
        }
        true
    }

    fn finish(&self, job_id: &str) {
        // The job is over: forget it now — so its `Terminated` is not reported
        // as a failure — and make sure the process really exits.
        self.take_and_reap(job_id);
    }

    fn spawn_watchdog(self: &Arc<Self>, job_id: String) {
        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(this.timing.wall_clock).await;
            let running = this
                .jobs
                .lock()
                .get(&job_id)
                .map(|job| job.started.elapsed() >= this.timing.wall_clock)
                .unwrap_or(false);
            if running {
                tracing::warn!(job = %job_id, "research job hit the wall-clock cap");
                // Ask it to stop, then forget it so the sidecar's own
                // `failed{cancelled}` is not reported on top of the timeout.
                this.write(
                    &job_id,
                    "research.cancel",
                    proto::research_cancel_params(&job_id),
                );
                if this.take_and_reap(&job_id) {
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

    /// Ask the job to stop. The sidecar answers with `failed{cancelled}`;
    /// if it has not finished when the grace period ends (a tool call that
    /// ignores the abort, a wedged process) the job is killed and the
    /// cancellation is published here instead, so the caller never waits on
    /// an event that will not come. Returns whether the job was known.
    pub(crate) fn cancel(self: &Arc<Self>, job_id: &str) -> bool {
        if !self.is_running(job_id) {
            return false;
        }
        self.write(
            job_id,
            "research.cancel",
            proto::research_cancel_params(job_id),
        );
        let this = self.clone();
        let id = job_id.to_string();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(this.timing.cancel_grace).await;
            let job = this.jobs.lock().remove(&id);
            if let Some(job) = job {
                if let Some(process) = job.process {
                    process.kill();
                }
                this.publish_failed(&id, BlueyError::cancelled());
            }
        });
        true
    }

    /// Kill every running job (app exit).
    pub(crate) fn shutdown(&self) {
        let jobs: Vec<Job> = self.jobs.lock().drain().map(|(_, job)| job).collect();
        for job in jobs {
            if let Some(process) = job.process {
                process.kill();
            }
        }
    }
}

#[cfg(test)]
#[path = "jobs_tests.rs"]
mod tests;
