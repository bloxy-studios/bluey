//! The in-process event bus: a tokio broadcast channel of
//! [`bluey_core::events::BlueyEvent`] plus the forwarder task that mirrors
//! every event onto the Tauri event system (`bluey:<name>`).

use std::sync::Arc;

use bluey_core::events::{BlueyEvent, EventSink};
use tauri::{AppHandle, Emitter};
use tokio::sync::broadcast;

/// Broadcast capacity. Slow internal subscribers may lag and skip events; the
/// WebView forwarder keeps up by design (fire-and-forget emit).
const CAPACITY: usize = 1024;

/// Cheap-to-clone publish/subscribe hub for [`BlueyEvent`]s.
pub struct EventBus {
    tx: broadcast::Sender<BlueyEvent>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBus {
    /// New bus with the default capacity.
    pub fn new() -> Self {
        let (tx, _rx) = broadcast::channel(CAPACITY);
        Self { tx }
    }

    /// Publish one event to every subscriber (dropped when nobody listens).
    pub fn publish(&self, event: BlueyEvent) {
        let _ = self.tx.send(event);
    }

    /// Subscribe to every event published from now on.
    pub fn subscribe(&self) -> broadcast::Receiver<BlueyEvent> {
        self.tx.subscribe()
    }
}

impl EventSink for EventBus {
    fn publish(&self, event: BlueyEvent) {
        EventBus::publish(self, event);
    }
}

/// Spawn the forwarder task that emits every bus event to the WebView as
/// `bluey:<name>` with the exact contract payload.
pub fn spawn_forwarder(app: AppHandle, bus: Arc<EventBus>) {
    let mut rx = bus.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let name = event.tauri_event_name();
                    if let Err(e) = app.emit(&name, event.payload()) {
                        tracing::warn!(event = %name, error = %e, "failed to emit event");
                    }
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "event forwarder lagged; events dropped");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}
