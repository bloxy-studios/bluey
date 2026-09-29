//! The job table against a scripted fake sidecar: each test scripts how the
//! "process" answers the lines Rust writes (research.run / research.cancel /
//! document.response) and asserts the `research.event`s on the bus.

use std::sync::atomic::AtomicBool;

use bluey_core::BlueyErrorKind;
use serde_json::json;
use tauri_plugin_shell::process::TerminatedPayload;
use tokio::sync::{broadcast, mpsc};

use super::*;

const GRACE: Duration = Duration::from_millis(80);
/// Longer than any scripted exchange, so "nothing else arrived" is meaningful.
const QUIET: Duration = Duration::from_millis(250);

type Script = Box<dyn FnMut(&Value, &Stdout) + Send>;

/// The fake's stdout: what it "prints" reaches the job table's reader.
#[derive(Clone)]
struct Stdout(mpsc::Sender<CommandEvent>);

impl Stdout {
    fn event(&self, name: &str, data: Value) {
        self.line(json!({ "event": name, "data": data }).to_string());
    }

    fn line(&self, text: String) {
        let _ = self.0.try_send(CommandEvent::Stdout(text.into_bytes()));
    }

    fn exit(&self, code: i32) {
        let _ = self.0.try_send(CommandEvent::Terminated(TerminatedPayload {
            code: Some(code),
            signal: None,
        }));
    }
}

struct FakeSidecar {
    stdout: Stdout,
    script: Script,
    written: Arc<parking_lot::Mutex<Vec<Value>>>,
    killed: Arc<AtomicBool>,
}

impl AgentProcess for FakeSidecar {
    fn write_line(&mut self, line: &str) -> Result<(), String> {
        let value: Value = serde_json::from_str(line.trim()).map_err(|e| e.to_string())?;
        self.written.lock().push(value.clone());
        (self.script)(&value, &self.stdout);
        Ok(())
    }

    fn kill(self: Box<Self>) {
        self.killed.store(true, Ordering::SeqCst);
        // A killed process still reports its exit.
        self.stdout.exit(-1);
    }
}

struct Harness {
    jobs: Arc<AgentJobs>,
    events: broadcast::Receiver<BlueyEvent>,
    written: Arc<parking_lot::Mutex<Vec<Value>>>,
    killed: Arc<AtomicBool>,
}

struct Documents;

#[async_trait]
impl DocumentSource for Documents {
    async fn document_text(&self, document_id: &str) -> Result<String, String> {
        Ok(format!("text of {document_id}"))
    }
}

fn start(wall_clock: Duration, script: impl FnMut(&Value, &Stdout) + Send + 'static) -> Harness {
    let bus = Arc::new(EventBus::new());
    let events = bus.subscribe();
    let jobs = Arc::new(AgentJobs::new(
        bus,
        Arc::new(Documents),
        JobTiming {
            cancel_grace: GRACE,
            wall_clock,
        },
    ));
    let (tx, rx) = mpsc::channel(64);
    let written = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let killed = Arc::new(AtomicBool::new(false));
    let process = FakeSidecar {
        stdout: Stdout(tx),
        script: Box::new(script),
        written: written.clone(),
        killed: killed.clone(),
    };
    jobs.launch(
        "job-1".into(),
        json!({ "jobId": "job-1", "query": "q" }),
        Some(vec!["doc-1".into()]),
        Box::new(process),
        rx,
    )
    .expect("launch");
    Harness {
        jobs,
        events,
        written,
        killed,
    }
}

fn method(line: &Value) -> &str {
    line["method"].as_str().unwrap_or_default()
}

impl Harness {
    async fn next(&mut self) -> DeepResearchEvent {
        loop {
            let event = tokio::time::timeout(Duration::from_secs(5), self.events.recv())
                .await
                .expect("a research event")
                .expect("bus open");
            if let BlueyEvent::ResearchEvent(event) = event {
                return event;
            }
        }
    }

    /// No further research event arrives for a while.
    async fn assert_quiet(&mut self) {
        let next = tokio::time::timeout(QUIET, self.events.recv()).await;
        assert!(next.is_err(), "unexpected event: {next:?}");
    }

    async fn next_failure(&mut self) -> BlueyError {
        loop {
            match self.next().await {
                DeepResearchEvent::Failed { error, .. } => return error,
                DeepResearchEvent::Completed { .. } => panic!("expected a failure"),
                _ => continue,
            }
        }
    }
}

#[tokio::test]
async fn a_completed_job_publishes_its_events_once_and_is_reaped() {
    let mut h = start(Duration::from_secs(30), |line, out| {
        if method(line) == "research.run" {
            out.event("research.started", json!({ "jobId": "job-1" }));
            out.event(
                "research.progress",
                json!({ "jobId": "job-1", "message": "Searching" }),
            );
            out.event(
                "research.completed",
                json!({ "jobId": "job-1", "report": "done", "citations": [], "totalMs": 5, "turns": 1 }),
            );
        }
    });
    assert!(matches!(h.next().await, DeepResearchEvent::Started { .. }));
    assert!(matches!(h.next().await, DeepResearchEvent::Progress { .. }));
    assert!(matches!(
        h.next().await,
        DeepResearchEvent::Completed { .. }
    ));
    assert!(!h.jobs.is_running("job-1"));
    // The process exit after `completed` is not reported as a crash.
    h.assert_quiet().await;
    assert!(h.killed.load(Ordering::SeqCst));
}

#[tokio::test]
async fn cancel_answered_by_the_sidecar_publishes_one_cancellation() {
    let mut h = start(Duration::from_secs(30), |line, out| {
        if method(line) == "research.cancel" {
            out.event(
                "research.failed",
                json!({ "jobId": "job-1", "error": { "kind": "cancelled", "code": "cancelled", "message": "stopped" } }),
            );
        }
    });
    assert!(matches!(h.next().await, DeepResearchEvent::Started { .. }));
    assert!(h.jobs.cancel("job-1"));
    let error = h.next_failure().await;
    assert_eq!(error.code, "cancelled.cancelled");
    h.assert_quiet().await;
    assert!(h.killed.load(Ordering::SeqCst));
}

/// LIVE-005: a sidecar stuck in a tool call never answers `research.cancel`;
/// the caller still gets a terminal event when the grace period ends.
#[tokio::test]
async fn cancel_ignored_by_the_sidecar_still_ends_the_job() {
    let mut h = start(Duration::from_secs(30), |_, _| {});
    assert!(matches!(h.next().await, DeepResearchEvent::Started { .. }));
    assert!(h.jobs.cancel("job-1"));
    let error = h.next_failure().await;
    assert_eq!(error.kind, BlueyErrorKind::Cancelled);
    assert!(h.killed.load(Ordering::SeqCst));
    assert!(!h.jobs.is_running("job-1"));
    // The kill's own exit is not reported as `agent_exited` on top.
    h.assert_quiet().await;
    assert!(!h.jobs.cancel("job-1"), "a finished job is unknown");
}

#[tokio::test]
async fn a_crash_before_the_terminal_event_fails_the_job() {
    let mut h = start(Duration::from_secs(30), |line, out| {
        if method(line) == "research.run" {
            out.event(
                "research.progress",
                json!({ "jobId": "job-1", "message": "Searching" }),
            );
            out.exit(1);
        }
    });
    let error = h.next_failure().await;
    assert_eq!(error.code, "research.agent_exited");
    h.assert_quiet().await;
}

#[tokio::test]
async fn a_rejected_run_request_fails_the_job_with_the_sidecar_error() {
    let mut h = start(Duration::from_secs(30), |line, out| {
        if method(line) == "research.run" {
            let id = line["id"].as_str().unwrap_or_default();
            out.line(
                json!({ "id": id, "error": { "kind": "configuration", "code": "missing_credentials", "message": "no key" } })
                    .to_string(),
            );
        }
    });
    let error = h.next_failure().await;
    assert_eq!(error.code, "configuration.missing_credentials");
    h.assert_quiet().await;
}

/// AI-014 boundary: a multi-megabyte final frame is one line and arrives whole.
#[tokio::test]
async fn a_huge_final_frame_is_delivered_intact() {
    let report = "x".repeat(3 * 1024 * 1024);
    let citations: Vec<Value> = (0..400)
        .map(|i| json!({ "title": format!("t{i}"), "url": format!("https://example.com/{i}") }))
        .collect();
    let expected = report.len();
    let mut h = start(Duration::from_secs(30), move |line, out| {
        if method(line) == "research.run" {
            out.event(
                "research.completed",
                json!({ "jobId": "job-1", "report": report, "citations": citations, "totalMs": 1, "turns": 3 }),
            );
            out.exit(0);
        }
    });
    loop {
        match h.next().await {
            DeepResearchEvent::Completed {
                report, citations, ..
            } => {
                assert_eq!(report.len(), expected);
                assert_eq!(citations.len(), 400);
                break;
            }
            DeepResearchEvent::Failed { error, .. } => panic!("failed: {}", error.code),
            _ => continue,
        }
    }
    h.assert_quiet().await;
}

#[tokio::test]
async fn the_wall_clock_cap_cancels_and_fails_the_job() {
    let mut h = start(Duration::from_millis(100), |_, _| {});
    let error = h.next_failure().await;
    assert_eq!(error.code, "research.timeout");
    assert!(h
        .written
        .lock()
        .iter()
        .any(|line| method(line) == "research.cancel"));
    h.assert_quiet().await;
    assert!(h.killed.load(Ordering::SeqCst));
}

#[tokio::test]
async fn document_requests_are_served_only_from_the_allow_list() {
    let h = start(Duration::from_secs(30), |line, out| {
        if method(line) == "research.run" {
            for (request, document) in [("rq1", "doc-1"), ("rq2", "doc-secret")] {
                out.event(
                    "document.request",
                    json!({ "jobId": "job-1", "requestId": request, "documentId": document }),
                );
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    let responses = loop {
        let responses: Vec<Value> = h
            .written
            .lock()
            .iter()
            .filter(|line| method(line) == "document.response")
            .map(|line| line["params"].clone())
            .collect();
        if responses.len() == 2 || Instant::now() > deadline {
            break responses;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert_eq!(responses.len(), 2);
    let allowed = responses.iter().find(|r| r["requestId"] == "rq1").unwrap();
    assert_eq!(allowed["text"], "text of doc-1");
    let denied = responses.iter().find(|r| r["requestId"] == "rq2").unwrap();
    assert!(denied.get("text").is_none());
    assert!(denied["error"].is_string());
}
