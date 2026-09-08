//! Importing recordings (`ai_transcribe_file`): the whole file goes through
//! the transcription-role provider in one batch call (`gemini-3.5-transcribe`
//! with diarization / word timestamps), the speaker turns become finalized
//! `TranscriptSegment`s (`bluey_protocols::transcript_import`), and the session
//! — the one the caller named or a new completed "Imported" one — gets a
//! `recording_imported` timeline event.
//!
//! The recording itself is never copied or kept: bytes are read, sent, and
//! dropped (the Files API upload for large files is deleted right after).

use std::collections::BTreeMap;
use std::path::Path;

use bluey_core::session::event_title;
use bluey_core::types::{SessionEventType, TranscribeFileResult};
use bluey_core::{new_id, now_iso, BlueyError, BlueyResult};
use bluey_protocols::transcript_import::{import_detail, segments_from_turns, speaker_count};
use bluey_storage::TranscriptRepository;

use crate::ai::providers::TranscribeFileOptions;
use crate::state::AppCore;

/// Transcribe `path` and file the result under `session_id` (or a new
/// completed session named after the file). Segments are persisted only when
/// privacy → store transcripts is on; the timeline event is always added.
pub async fn import_recording(
    core: &AppCore,
    path: String,
    options: TranscribeFileOptions,
    session_id: Option<String>,
) -> BlueyResult<TranscribeFileResult> {
    let file_path = Path::new(&path);
    let file_name = file_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "recording".to_string());
    let transcription = core.ai.transcribe_file(file_path, &options).await?;
    let session = match session_id {
        Some(id) => core.sessions.get(id).await?,
        None => {
            core.sessions
                .create_imported(format!("Imported · {file_name}"))
                .await?
        }
    };
    let created_at = now_iso();
    let segments = segments_from_turns(
        &transcription.turns,
        Some(&session.id),
        options.language.as_deref(),
        &created_at,
        || new_id("seg"),
    );
    if segments.is_empty() {
        return Err(BlueyError::transcription(
            "no_speech",
            "no speech was recognized in the recording",
        ));
    }
    let stored = core.settings.get().privacy.store_transcripts;
    if stored {
        let batch = segments.clone();
        core.storage
            .run(move |db| -> BlueyResult<()> {
                for segment in &batch {
                    TranscriptRepository::insert(db, segment)?;
                }
                Ok(())
            })
            .await?;
    }
    let speakers = speaker_count(&segments);
    let duration_ms = segments.iter().map(|s| s.end_time).max().unwrap_or(0);
    let mut refs = BTreeMap::new();
    refs.insert("file".to_string(), file_name.clone());
    refs.insert("segments".to_string(), segments.len().to_string());
    if speakers > 0 {
        refs.insert("speakers".to_string(), speakers.to_string());
    }
    core.sessions
        .add_event(
            session.id.clone(),
            SessionEventType::RecordingImported,
            event_title(SessionEventType::RecordingImported).to_string(),
            Some(import_detail(
                &file_name,
                segments.len(),
                speakers,
                duration_ms,
                stored,
            )),
            Some(refs),
            None,
        )
        .await?;
    tracing::info!(
        session = %session.id,
        segments = segments.len(),
        speakers,
        duration_ms,
        stored,
        "imported recording"
    );
    Ok(TranscribeFileResult {
        session,
        segments,
        speakers,
        duration_ms,
        language: options.language,
        stored,
    })
}
