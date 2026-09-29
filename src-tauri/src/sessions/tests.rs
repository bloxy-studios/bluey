//! `SessionManager` lifecycle over a real (in-memory) database.

use std::sync::Arc;

use bluey_core::events::BlueyEvent;
use bluey_core::types::{SessionEventType, SessionStatus, DEFAULT_MODE_ID};
use bluey_storage::{SessionEventRepository, SessionRepository};
use tokio::sync::broadcast::Receiver;

use super::SessionManager;
use crate::events::EventBus;
use crate::modes::ModeManager;
use crate::secrets::backend::fake::CountingFake;
use crate::secrets::SecretsStore;
use crate::settings::SettingsManager;
use crate::state::StateHub;
use crate::storage::Storage;

struct Harness {
    storage: Arc<Storage>,
    settings: Arc<SettingsManager>,
    bus: Arc<EventBus>,
    hub: Arc<StateHub>,
    modes: Arc<ModeManager>,
}

impl Harness {
    fn new() -> Self {
        let storage = Arc::new(Storage::in_memory());
        let bus = Arc::new(EventBus::new());
        let settings = Arc::new(
            SettingsManager::load(
                storage.clone(),
                Arc::new(SecretsStore::with_backend(
                    Arc::new(CountingFake::default()),
                )),
                bus.clone(),
            )
            .unwrap(),
        );
        let hub = Arc::new(StateHub::new(DEFAULT_MODE_ID, bus.clone()));
        let modes = Arc::new(
            ModeManager::load(storage.clone(), settings.clone(), bus.clone(), hub.clone()).unwrap(),
        );
        Self {
            storage,
            settings,
            bus,
            hub,
            modes,
        }
    }

    /// A freshly launched manager over the same database.
    fn launch(&self) -> SessionManager {
        SessionManager::load(
            self.storage.clone(),
            self.settings.clone(),
            self.bus.clone(),
            self.hub.clone(),
            self.modes.clone(),
        )
        .unwrap()
    }

    /// A session row left `active` by a run that never shut down.
    fn leave_live_session(&self) -> String {
        self.storage
            .run_sync(|db| SessionRepository::create(db, DEFAULT_MODE_ID, None))
            .unwrap()
            .id
    }
}

/// The next `session.ended` on the bus, skipping every other event.
fn next_ended(rx: &mut Receiver<BlueyEvent>) -> Option<bluey_core::types::Session> {
    while let Ok(event) = rx.try_recv() {
        if let BlueyEvent::SessionEnded(session) = event {
            return Some(session);
        }
    }
    None
}

#[test]
fn launch_ends_a_session_left_live_by_an_unexpected_quit() {
    let h = Harness::new();
    let zombie = h.leave_live_session();

    let manager = h.launch();

    assert!(manager.active().is_none(), "nothing is restored as live");
    let session = h
        .storage
        .run_sync(|db| SessionRepository::get(db, &zombie))
        .unwrap();
    assert_eq!(session.status, SessionStatus::Completed);
    assert_eq!(
        session.ended_at.as_deref(),
        Some(session.started_at.as_str())
    );
    let events = h
        .storage
        .run_sync(|db| SessionEventRepository::list(db, &zombie))
        .unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, SessionEventType::SessionRecovered);
    assert_eq!(events[0].created_at, session.started_at);
}

#[test]
fn launch_deletes_the_left_live_session_when_history_is_off() {
    let h = Harness::new();
    h.settings
        .update_sync(serde_json::json!({ "privacy": { "storeSessionHistory": false } }))
        .unwrap();
    let zombie = h.leave_live_session();

    let manager = h.launch();

    assert!(manager.active().is_none());
    assert!(h
        .storage
        .run_sync(|db| SessionRepository::get(db, &zombie))
        .is_err());
}

#[tokio::test]
async fn listening_after_a_relaunch_starts_a_new_session() {
    let h = Harness::new();
    let zombie = h.leave_live_session();
    let manager = h.launch();

    let fresh = manager.start(None, None).await.unwrap();

    assert_ne!(fresh.id, zombie);
    assert_eq!(manager.active_id(), Some(fresh.id));
}

#[tokio::test]
async fn deleting_the_live_session_publishes_session_ended() {
    let h = Harness::new();
    let manager = h.launch();
    let live = manager.start(None, None).await.unwrap();
    let mut rx = h.bus.subscribe();

    manager.delete(live.id.clone()).await.unwrap();

    let ended = next_ended(&mut rx).expect("session.ended published");
    assert_eq!(ended.id, live.id);
    assert_eq!(ended.status, SessionStatus::Completed);
    assert!(manager.active().is_none());
}

#[tokio::test]
async fn deleting_another_session_leaves_the_live_one_alone() {
    let h = Harness::new();
    let manager = h.launch();
    let old = manager.start(None, None).await.unwrap();
    let live = manager.start(None, None).await.unwrap();
    let mut rx = h.bus.subscribe();

    manager.delete(old.id).await.unwrap();

    assert!(next_ended(&mut rx).is_none());
    assert_eq!(manager.active_id(), Some(live.id));
}

#[tokio::test]
async fn deleting_all_sessions_ends_the_live_one() {
    let h = Harness::new();
    let manager = h.launch();
    let live = manager.start(None, None).await.unwrap();
    let mut rx = h.bus.subscribe();

    assert_eq!(manager.delete_all().await.unwrap(), 1);

    assert_eq!(next_ended(&mut rx).map(|s| s.id), Some(live.id));
    assert!(manager.active().is_none());
}
