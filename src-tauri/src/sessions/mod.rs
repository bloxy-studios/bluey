//! Session manager: lifecycle (start/pause/resume/end with retention),
//! timeline events, notes, summaries, list/search/detail assembly and export.

use std::sync::Arc;

use bluey_core::events::BlueyEvent;
use bluey_core::session::{can_transition, default_title, event_title};
use bluey_core::types::{
    AppEvent, Session, SessionDetail, SessionEvent, SessionEventType, SessionListItem, SessionNote,
    SessionSearchQuery, SessionStatus, SessionSummary, SessionSummaryInput,
};
use bluey_core::{new_id, now_iso, BlueyError, BlueyResult};
use bluey_storage::{
    ModeRepository, ResponseRepository, SessionEventRepository, SessionNoteRepository,
    SessionRepository, SummaryRepository, TranscriptRepository,
};

use crate::events::EventBus;
use crate::settings::SettingsManager;
use crate::state::StateHub;
use crate::storage::Storage;

pub struct SessionManager {
    storage: Arc<Storage>,
    settings: Arc<SettingsManager>,
    bus: Arc<EventBus>,
    hub: Arc<StateHub>,
    modes: Arc<crate::modes::ModeManager>,
    active: parking_lot::Mutex<Option<Session>>,
}

impl SessionManager {
    /// Restore the active session from the database (bootstrap, synchronous).
    pub fn load(
        storage: Arc<Storage>,
        settings: Arc<SettingsManager>,
        bus: Arc<EventBus>,
        hub: Arc<StateHub>,
        modes: Arc<crate::modes::ModeManager>,
    ) -> BlueyResult<Self> {
        let active = storage.run_sync(SessionRepository::get_active)?;
        Ok(Self {
            storage,
            settings,
            bus,
            hub,
            modes,
            active: parking_lot::Mutex::new(active),
        })
    }

    /// Id of the active (or paused) session, if any.
    pub fn active_id(&self) -> Option<String> {
        self.active.lock().as_ref().map(|s| s.id.clone())
    }

    /// The active session, if any.
    pub fn active(&self) -> Option<Session> {
        self.active.lock().clone()
    }

    /// Any session by id (`storage.not_found` when missing).
    pub async fn get(&self, id: String) -> BlueyResult<Session> {
        self.storage
            .run(move |db| SessionRepository::get(db, &id))
            .await
    }

    /// A completed session for an imported recording, created without touching
    /// the live session (no state transition, no `session.*` events).
    pub async fn create_imported(&self, title: String) -> BlueyResult<Session> {
        let mode_id = self.modes.active_id();
        self.storage
            .run(move |db| {
                let session = SessionRepository::create(db, &mode_id, Some(title))?;
                SessionRepository::set_status(
                    db,
                    &session.id,
                    SessionStatus::Completed,
                    Some(now_iso()),
                )
            })
            .await
    }

    /// Start a new session (ending any active one first).
    pub async fn start(
        &self,
        mode_id: Option<String>,
        title: Option<String>,
    ) -> BlueyResult<Session> {
        if self.active().is_some() {
            let _ = self.end().await;
        }
        let mode_id = mode_id.unwrap_or_else(|| self.modes.active_id());
        let mode_name = {
            let fetch = mode_id.clone();
            self.storage
                .run(move |db| ModeRepository::get(db, &fetch))
                .await
                .map(|m| m.name)
                .unwrap_or_else(|_| mode_id.clone())
        };
        let create_mode = mode_id.clone();
        let session = self
            .storage
            .run(move |db| SessionRepository::create(db, &create_mode, title))
            .await?;
        let session = if session.title.is_none() {
            let title = default_title(&mode_name, &session.started_at);
            let id = session.id.clone();
            self.storage
                .run(move |db| SessionRepository::rename(db, &id, &title))
                .await?
        } else {
            session
        };
        *self.active.lock() = Some(session.clone());
        self.hub.transition_soft(AppEvent::SessionChanged {
            session_id: Some(session.id.clone()),
        });
        self.bus
            .publish(BlueyEvent::SessionStarted(session.clone()));
        let _ = self
            .add_event_internal(
                &session.id,
                SessionEventType::SessionStarted,
                event_title(SessionEventType::SessionStarted).to_string(),
                None,
                None,
                None,
            )
            .await;
        Ok(session)
    }

    async fn set_status(
        &self,
        status: SessionStatus,
        ended_at: Option<String>,
    ) -> BlueyResult<Session> {
        let current = self
            .active()
            .ok_or_else(|| BlueyError::invalid_params("no active session"))?;
        if !can_transition(current.status, status) {
            return Err(BlueyError::invalid_params(format!(
                "session cannot move from {:?} to {status:?}",
                current.status
            )));
        }
        let id = current.id.clone();
        let session = self
            .storage
            .run(move |db| SessionRepository::set_status(db, &id, status, ended_at))
            .await?;
        Ok(session)
    }

    pub async fn pause(&self) -> BlueyResult<Session> {
        let session = self.set_status(SessionStatus::Paused, None).await?;
        *self.active.lock() = Some(session.clone());
        self.bus.publish(BlueyEvent::SessionPaused(session.clone()));
        let _ = self
            .add_event_internal(
                &session.id,
                SessionEventType::SessionPaused,
                event_title(SessionEventType::SessionPaused).to_string(),
                None,
                None,
                None,
            )
            .await;
        Ok(session)
    }

    pub async fn resume(&self) -> BlueyResult<Session> {
        let session = self.set_status(SessionStatus::Active, None).await?;
        *self.active.lock() = Some(session.clone());
        self.bus
            .publish(BlueyEvent::SessionResumed(session.clone()));
        let _ = self
            .add_event_internal(
                &session.id,
                SessionEventType::SessionResumed,
                event_title(SessionEventType::SessionResumed).to_string(),
                None,
                None,
                None,
            )
            .await;
        Ok(session)
    }

    /// End the active session. When session history is disabled the session is
    /// deleted immediately after ending (retention).
    pub async fn end(&self) -> BlueyResult<Session> {
        let current = self
            .active()
            .ok_or_else(|| BlueyError::invalid_params("no active session"))?;
        let _ = self
            .add_event_internal(
                &current.id,
                SessionEventType::SessionEnded,
                event_title(SessionEventType::SessionEnded).to_string(),
                None,
                None,
                None,
            )
            .await;
        let session = self
            .set_status(SessionStatus::Completed, Some(now_iso()))
            .await?;
        *self.active.lock() = None;
        self.hub
            .transition_soft(AppEvent::SessionChanged { session_id: None });
        self.bus.publish(BlueyEvent::SessionEnded(session.clone()));

        if !self.settings.get().privacy.store_session_history {
            let id = session.id.clone();
            let paths = self
                .storage
                .run(move |db| bluey_storage::delete_session(db, &id))
                .await?;
            Storage::remove_files(&paths);
        }
        Ok(session)
    }

    pub async fn list(&self, query: SessionSearchQuery) -> BlueyResult<Vec<SessionListItem>> {
        self.storage
            .run(move |db| SessionRepository::list(db, &query))
            .await
    }

    pub async fn search(&self, query: SessionSearchQuery) -> BlueyResult<Vec<SessionListItem>> {
        self.storage
            .run(move |db| bluey_storage::search_sessions(db, &query))
            .await
    }

    /// Full session detail (session, events, notes, summary, responses, count).
    pub async fn detail(&self, id: String) -> BlueyResult<SessionDetail> {
        self.storage
            .run(move |db| {
                let session = SessionRepository::get(db, &id)?;
                let events = SessionEventRepository::list(db, &id)?;
                let notes = SessionNoteRepository::list(db, &id)?;
                let summary = SummaryRepository::get(db, &id)?;
                let responses = ResponseRepository::list(db, &id, None)?;
                let transcript_segment_count = TranscriptRepository::count(db, Some(&id))? as u32;
                Ok(SessionDetail {
                    session,
                    events,
                    notes,
                    summary,
                    responses,
                    transcript_segment_count,
                })
            })
            .await
    }

    pub async fn delete(&self, id: String) -> BlueyResult<()> {
        let was_active = self.active_id().as_deref() == Some(id.as_str());
        let delete_id = id.clone();
        let paths = self
            .storage
            .run(move |db| bluey_storage::delete_session(db, &delete_id))
            .await?;
        Storage::remove_files(&paths);
        if was_active {
            *self.active.lock() = None;
            self.hub
                .transition_soft(AppEvent::SessionChanged { session_id: None });
        }
        Ok(())
    }

    pub async fn delete_all(&self) -> BlueyResult<u64> {
        let (count, paths) = self.storage.run(bluey_storage::delete_all_sessions).await?;
        Storage::remove_files(&paths);
        *self.active.lock() = None;
        self.hub
            .transition_soft(AppEvent::SessionChanged { session_id: None });
        Ok(count)
    }

    pub async fn rename(&self, id: String, title: String) -> BlueyResult<Session> {
        let rename_id = id.clone();
        let session = self
            .storage
            .run(move |db| SessionRepository::rename(db, &rename_id, &title))
            .await?;
        let mut active = self.active.lock();
        if active.as_ref().map(|s| s.id.as_str()) == Some(id.as_str()) {
            *active = Some(session.clone());
        }
        Ok(session)
    }

    /// Append a timeline event and publish `session.event`.
    pub async fn add_event(
        &self,
        session_id: String,
        event_type: SessionEventType,
        title: String,
        detail: Option<String>,
        refs: Option<std::collections::BTreeMap<String, String>>,
        confidence: Option<f32>,
    ) -> BlueyResult<SessionEvent> {
        self.add_event_internal(&session_id, event_type, title, detail, refs, confidence)
            .await
    }

    async fn add_event_internal(
        &self,
        session_id: &str,
        event_type: SessionEventType,
        title: String,
        detail: Option<String>,
        refs: Option<std::collections::BTreeMap<String, String>>,
        confidence: Option<f32>,
    ) -> BlueyResult<SessionEvent> {
        let event = SessionEvent {
            id: new_id("sev"),
            session_id: session_id.to_string(),
            event_type,
            title,
            detail,
            refs,
            confidence,
            created_at: now_iso(),
        };
        let stored = {
            let event = event.clone();
            self.storage
                .run(move |db| SessionEventRepository::add(db, &event))
                .await?
        };
        self.bus.publish(BlueyEvent::SessionEvent(stored.clone()));
        Ok(stored)
    }

    pub async fn list_events(&self, session_id: String) -> BlueyResult<Vec<SessionEvent>> {
        self.storage
            .run(move |db| SessionEventRepository::list(db, &session_id))
            .await
    }

    pub async fn add_note(&self, session_id: String, content: String) -> BlueyResult<SessionNote> {
        let note = self
            .storage
            .run(move |db| SessionNoteRepository::add(db, &session_id, &content))
            .await?;
        let _ = self
            .add_event_internal(
                &note.session_id,
                SessionEventType::NoteAdded,
                event_title(SessionEventType::NoteAdded).to_string(),
                None,
                None,
                None,
            )
            .await;
        Ok(note)
    }

    pub async fn delete_note(&self, note_id: String) -> BlueyResult<()> {
        self.storage
            .run(move |db| SessionNoteRepository::delete(db, &note_id))
            .await
    }

    pub async fn save_summary(&self, input: SessionSummaryInput) -> BlueyResult<SessionSummary> {
        let session_id = input.session_id.clone();
        let summary = self
            .storage
            .run(move |db| SummaryRepository::save(db, &input))
            .await?;
        let _ = self
            .add_event_internal(
                &session_id,
                SessionEventType::SummaryGenerated,
                event_title(SessionEventType::SummaryGenerated).to_string(),
                None,
                None,
                None,
            )
            .await;
        Ok(summary)
    }

    pub async fn get_summary(&self, session_id: String) -> BlueyResult<Option<SessionSummary>> {
        self.storage
            .run(move |db| SummaryRepository::get(db, &session_id))
            .await
    }

    /// Export one session as Markdown or JSON.
    pub async fn export(&self, session_id: String, format: &str) -> BlueyResult<String> {
        let detail = self.detail(session_id.clone()).await?;
        let transcript = self
            .storage
            .run(move |db| TranscriptRepository::list(db, Some(&session_id), None, None))
            .await?;
        match format {
            "markdown" => Ok(bluey_core::session::export_markdown(&detail, &transcript)),
            "json" => serde_json::to_string_pretty(&serde_json::json!({
                "detail": detail,
                "transcript": transcript,
            }))
            .map_err(|e| BlueyError::internal(format!("export serialization failed: {e}"))),
            other => Err(BlueyError::invalid_params(format!(
                "unknown export format `{other}`"
            ))),
        }
    }
}
