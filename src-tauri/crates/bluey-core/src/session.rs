//! Session lifecycle rules and pure helpers: status transitions, default
//! titles, human labels for timeline events, DetectedEvent → SessionEvent
//! mapping, and the Markdown export used by `data_export_session`.

use crate::text::{normalize_whitespace, truncate_chars};
use crate::types::{
    DetectedEvent, DetectedEventType, SessionDetail, SessionEventType, SessionStatus,
    TranscriptSegment,
};

/// Whether a session may move from `from` to `to`.
/// active ⇄ paused, active/paused → completed; completed is terminal.
pub fn can_transition(from: SessionStatus, to: SessionStatus) -> bool {
    use SessionStatus as S;
    matches!(
        (from, to),
        (S::Active, S::Paused)
            | (S::Active, S::Completed)
            | (S::Paused, S::Active)
            | (S::Paused, S::Completed)
    )
}

/// Default session title, e.g. `"Interview · Sep 7, 09:14"`. `started_at` is
/// parsed as RFC 3339 and rendered in its own UTC offset; on parse failure the
/// raw string is used.
pub fn default_title(mode_name: &str, started_at: &str) -> String {
    match chrono::DateTime::parse_from_rfc3339(started_at) {
        Ok(dt) => format!("{mode_name} · {}", dt.format("%b %-d, %H:%M")),
        Err(_) => format!("{mode_name} · {started_at}"),
    }
}

/// Human label for a session timeline event type.
pub fn event_title(event_type: SessionEventType) -> &'static str {
    use SessionEventType as T;
    match event_type {
        T::SessionStarted => "Session started",
        T::SessionPaused => "Session paused",
        T::SessionResumed => "Session resumed",
        T::SessionEnded => "Session ended",
        T::QuestionDetected => "Question detected",
        T::CodingProblemDetected => "Coding problem captured",
        T::ObjectionDetected => "Objection detected",
        T::DecisionDetected => "Decision detected",
        T::ActionItemDetected => "Action item detected",
        T::TopicChange => "Topic changed",
        T::ImportantStatement => "Important statement",
        T::FollowUp => "Follow-up question",
        T::ScreenCaptured => "Screen captured",
        T::ResponsePrepared => "Response prepared",
        T::ResponseGenerated => "Response generated",
        T::ResponseFailed => "Response failed",
        T::DocumentAttached => "Document attached",
        T::ModeChanged => "Mode changed",
        T::NoteAdded => "Note added",
        T::SummaryGenerated => "Summary generated",
        T::RecordingImported => "Recording imported",
    }
}

/// Map a live [`DetectedEvent`] to the session-event type it should be logged
/// as, with a display title (label plus a short snippet of the detected text).
pub fn detected_to_session_event(detected: &DetectedEvent) -> Option<(SessionEventType, String)> {
    use DetectedEventType as D;
    use SessionEventType as S;
    let (event_type, label) = match detected.event_type {
        D::Question => (S::QuestionDetected, "Question detected"),
        D::BehavioralQuestion => (S::QuestionDetected, "Behavioral question detected"),
        D::TechnicalQuestion => (S::QuestionDetected, "Technical question detected"),
        D::CodingProblem => (S::CodingProblemDetected, "Coding problem captured"),
        D::Objection => (S::ObjectionDetected, "Objection detected"),
        D::PricingConcern => (S::ObjectionDetected, "Pricing concern detected"),
        D::BuyingSignal => (S::ImportantStatement, "Buying signal detected"),
        D::CompetitorMention => (S::ImportantStatement, "Competitor mentioned"),
        D::Decision => (S::DecisionDetected, "Decision detected"),
        D::ActionItem => (S::ActionItemDetected, "Action item detected"),
        D::TopicChange => (S::TopicChange, "Topic changed"),
        D::ImportantStatement => (S::ImportantStatement, "Important statement"),
        D::FollowUp => (S::FollowUp, "Follow-up question"),
    };
    let snippet = truncate_chars(&normalize_whitespace(&detected.text), 64, "…");
    let title = if snippet.is_empty() {
        label.to_string()
    } else {
        format!("{label}: {snippet}")
    };
    Some((event_type, title))
}

/// Render a full session as Markdown: header, timeline, responses, summary,
/// notes, transcript. Used by the `data_export_session` command.
pub fn export_markdown(detail: &SessionDetail, transcript: &[TranscriptSegment]) -> String {
    let session = &detail.session;
    let title = session
        .title
        .clone()
        .unwrap_or_else(|| default_title(&session.mode_id, &session.started_at));

    let mut md = String::new();
    md.push_str(&format!("# {title}\n\n"));
    md.push_str(&format!("- Mode: {}\n", session.mode_id));
    md.push_str(&format!("- Status: {}\n", status_str(session.status)));
    md.push_str(&format!("- Started: {}\n", session.started_at));
    if let Some(ended) = &session.ended_at {
        md.push_str(&format!("- Ended: {ended}\n"));
    }

    if !detail.events.is_empty() {
        md.push_str("\n## Timeline\n\n");
        for event in &detail.events {
            md.push_str(&format!(
                "- **{}** — {}",
                clock(&event.created_at),
                event.title
            ));
            if let Some(event_detail) = &event.detail {
                md.push_str(&format!(" — {event_detail}"));
            }
            md.push('\n');
        }
    }

    if !detail.responses.is_empty() {
        md.push_str("\n## Responses\n\n");
        for (i, response) in detail.responses.iter().enumerate() {
            let heading = response
                .title
                .clone()
                .unwrap_or_else(|| "Response".to_string());
            md.push_str(&format!(
                "### {}. {heading} · {}\n\n{}\n",
                i + 1,
                clock(&response.created_at),
                response.content
            ));
            if let Some(code) = &response.code {
                md.push_str(&format!("\n```{}\n{}\n```\n", code.language, code.code));
            }
            if let Some(sections) = &response.sections {
                for section in sections {
                    md.push_str(&format!(
                        "\n#### {}\n\n{}\n",
                        section.title, section.content
                    ));
                }
            }
            md.push('\n');
        }
    }

    if let Some(summary) = &detail.summary {
        md.push_str("\n## Summary\n\n");
        md.push_str(&summary.overview);
        md.push('\n');
        push_list(&mut md, "Topics", &summary.topics);
        push_list(&mut md, "Questions", &summary.questions);
        push_list(&mut md, "Decisions", &summary.decisions);
        push_list(&mut md, "Action items", &summary.action_items);
        push_list(&mut md, "Open items", &summary.open_items);
    }

    if !detail.notes.is_empty() {
        md.push_str("\n## Notes\n\n");
        for note in &detail.notes {
            md.push_str(&format!("- {}\n", note.content));
        }
    }

    if !transcript.is_empty() {
        md.push_str("\n## Transcript\n\n");
        for segment in transcript {
            let speaker = segment.speaker.as_deref().unwrap_or(match segment.source {
                crate::types::AudioSource::Microphone => "Me",
                crate::types::AudioSource::System => "Them",
            });
            md.push_str(&format!(
                "- **[{}] {speaker}**: {}\n",
                mmss(segment.start_time),
                segment.text
            ));
        }
    }

    md
}

fn push_list(md: &mut String, heading: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    md.push_str(&format!("\n**{heading}**\n\n"));
    for item in items {
        md.push_str(&format!("- {item}\n"));
    }
}

fn status_str(status: SessionStatus) -> &'static str {
    match status {
        SessionStatus::Active => "active",
        SessionStatus::Paused => "paused",
        SessionStatus::Completed => "completed",
    }
}

/// `HH:MM:SS` from an RFC 3339 timestamp (in its own offset); raw on failure.
fn clock(rfc3339: &str) -> String {
    match chrono::DateTime::parse_from_rfc3339(rfc3339) {
        Ok(dt) => dt.format("%H:%M:%S").to_string(),
        Err(_) => rfc3339.to_string(),
    }
}

/// `mm:ss` from milliseconds since audio start.
fn mmss(ms: u64) -> String {
    let total_seconds = ms / 1_000;
    format!("{:02}:{:02}", total_seconds / 60, total_seconds % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        AudioSource, BlueyResponse, CodeBlock, ResponseType, Session, SessionEvent, SessionNote,
        SessionSummary,
    };
    use pretty_assertions::assert_eq;

    #[test]
    fn status_transitions() {
        use SessionStatus as S;
        assert!(can_transition(S::Active, S::Paused));
        assert!(can_transition(S::Active, S::Completed));
        assert!(can_transition(S::Paused, S::Active));
        assert!(can_transition(S::Paused, S::Completed));
        assert!(!can_transition(S::Completed, S::Active));
        assert!(!can_transition(S::Completed, S::Paused));
        assert!(!can_transition(S::Completed, S::Completed));
        assert!(!can_transition(S::Active, S::Active));
        assert!(!can_transition(S::Paused, S::Paused));
    }

    #[test]
    fn default_titles() {
        assert_eq!(
            default_title("Interview", "2026-09-07T09:14:03.000+00:00"),
            "Interview · Sep 7, 09:14"
        );
        assert_eq!(
            default_title("General", "2026-12-25T23:05:00Z"),
            "General · Dec 25, 23:05"
        );
        // Fallback to the raw string on unparseable input.
        assert_eq!(
            default_title("General", "not-a-date"),
            "General · not-a-date"
        );
    }

    #[test]
    fn event_titles_are_human() {
        assert_eq!(
            event_title(SessionEventType::QuestionDetected),
            "Question detected"
        );
        assert_eq!(
            event_title(SessionEventType::ResponsePrepared),
            "Response prepared"
        );
        assert_eq!(
            event_title(SessionEventType::CodingProblemDetected),
            "Coding problem captured"
        );
        assert_eq!(
            event_title(SessionEventType::FollowUp),
            "Follow-up question"
        );
        assert_eq!(
            event_title(SessionEventType::DecisionDetected),
            "Decision detected"
        );
        assert_eq!(
            event_title(SessionEventType::ActionItemDetected),
            "Action item detected"
        );
    }

    fn detected(event_type: DetectedEventType, text: &str) -> DetectedEvent {
        DetectedEvent {
            id: "det_1".into(),
            event_type,
            confidence: 0.9,
            requires_response: true,
            text: text.into(),
            segment_ids: vec!["seg_1".into()],
            speaker: Some("Speaker 1".into()),
            detected_at: crate::now_iso(),
        }
    }

    #[test]
    fn maps_detected_events_to_session_events() {
        use DetectedEventType as D;
        use SessionEventType as S;
        let cases: &[(D, S)] = &[
            (D::Question, S::QuestionDetected),
            (D::BehavioralQuestion, S::QuestionDetected),
            (D::TechnicalQuestion, S::QuestionDetected),
            (D::CodingProblem, S::CodingProblemDetected),
            (D::Objection, S::ObjectionDetected),
            (D::PricingConcern, S::ObjectionDetected),
            (D::BuyingSignal, S::ImportantStatement),
            (D::CompetitorMention, S::ImportantStatement),
            (D::Decision, S::DecisionDetected),
            (D::ActionItem, S::ActionItemDetected),
            (D::TopicChange, S::TopicChange),
            (D::ImportantStatement, S::ImportantStatement),
            (D::FollowUp, S::FollowUp),
        ];
        for (from, want) in cases {
            let (got, title) =
                detected_to_session_event(&detected(*from, "some text")).expect("mapped");
            assert_eq!(got, *want, "{from:?}");
            assert!(!title.is_empty());
        }

        let (_, title) = detected_to_session_event(&detected(
            DetectedEventType::Question,
            "How   would you scale this system to a billion users across many regions?",
        ))
        .expect("mapped");
        assert!(title.starts_with("Question detected: How would you scale"));
        assert!(title.chars().count() <= "Question detected: ".chars().count() + 64);
    }

    fn sample_detail() -> SessionDetail {
        let session = Session {
            id: "ses_1".into(),
            mode_id: "interview".into(),
            started_at: "2026-09-07T09:14:03.000Z".into(),
            ended_at: Some("2026-09-07T10:02:41.000Z".into()),
            status: SessionStatus::Completed,
            title: Some("Interview · Sep 7, 09:14".into()),
            metadata: None,
        };
        let events = vec![SessionEvent {
            id: "sev_1".into(),
            session_id: "ses_1".into(),
            event_type: SessionEventType::QuestionDetected,
            title: "Question detected: Tell me about yourself".into(),
            detail: None,
            refs: None,
            confidence: Some(0.9),
            created_at: "2026-09-07T09:15:00.000Z".into(),
        }];
        let responses = vec![BlueyResponse {
            id: "res_1".into(),
            request_id: "req_1".into(),
            session_id: Some("ses_1".into()),
            mode_id: "interview".into(),
            response_type: ResponseType::Answer,
            title: Some("Suggested answer".into()),
            content: "I'm a systems engineer with…".into(),
            code: Some(CodeBlock {
                language: "python".into(),
                code: "print('hi')".into(),
                filename: None,
            }),
            sections: None,
            citations: None,
            confidence: None,
            prompt: None,
            diagram: None,
            metrics: None,
            feedback: None,
            prepared: None,
            truncated: None,
            created_at: "2026-09-07T09:15:04.000Z".into(),
        }];
        let notes = vec![SessionNote {
            id: "note_1".into(),
            session_id: "ses_1".into(),
            content: "Follow up about the on-site".into(),
            created_at: crate::now_iso(),
            updated_at: crate::now_iso(),
        }];
        let summary = SessionSummary {
            id: "sum_1".into(),
            session_id: "ses_1".into(),
            mode_id: "interview".into(),
            overview: "45-minute screen with the hiring manager.".into(),
            decisions: vec!["Proceed to on-site".into()],
            action_items: vec!["Send availability — me — Friday".into()],
            ..SessionSummary::default()
        };
        SessionDetail {
            session,
            events,
            notes,
            summary: Some(summary),
            responses,
            transcript_segment_count: 2,
        }
    }

    #[test]
    fn exports_markdown_with_all_sections() {
        let transcript = vec![
            TranscriptSegment {
                id: "seg_1".into(),
                session_id: Some("ses_1".into()),
                speaker: Some("Interviewer".into()),
                speaker_confidence: None,
                source: AudioSource::System,
                text: "Tell me about yourself.".into(),
                start_time: 61_000,
                end_time: 63_000,
                confidence: None,
                finalized: true,
                language: None,
                created_at: crate::now_iso(),
            },
            TranscriptSegment {
                id: "seg_2".into(),
                session_id: Some("ses_1".into()),
                speaker: None,
                speaker_confidence: None,
                source: AudioSource::Microphone,
                text: "Sure — I'm a systems engineer.".into(),
                start_time: 64_000,
                end_time: 66_500,
                confidence: None,
                finalized: true,
                language: None,
                created_at: crate::now_iso(),
            },
        ];
        let md = export_markdown(&sample_detail(), &transcript);

        assert!(md.starts_with("# Interview · Sep 7, 09:14\n"));
        assert!(md.contains("- Mode: interview"));
        assert!(md.contains("- Status: completed"));
        assert!(md.contains("## Timeline"));
        assert!(md.contains("**09:15:00** — Question detected: Tell me about yourself"));
        assert!(md.contains("## Responses"));
        assert!(md.contains("### 1. Suggested answer · 09:15:04"));
        assert!(md.contains("```python\nprint('hi')\n```"));
        assert!(md.contains("## Summary"));
        assert!(md.contains("**Decisions**"));
        assert!(md.contains("- Proceed to on-site"));
        assert!(md.contains("## Notes"));
        assert!(md.contains("- Follow up about the on-site"));
        assert!(md.contains("## Transcript"));
        assert!(md.contains("- **[01:01] Interviewer**: Tell me about yourself."));
        assert!(md.contains("- **[01:04] Me**: Sure — I'm a systems engineer."));
    }

    #[test]
    fn export_omits_empty_sections() {
        let mut detail = sample_detail();
        detail.events.clear();
        detail.notes.clear();
        detail.summary = None;
        detail.responses.clear();
        let md = export_markdown(&detail, &[]);
        assert!(!md.contains("## Timeline"));
        assert!(!md.contains("## Responses"));
        assert!(!md.contains("## Summary"));
        assert!(!md.contains("## Notes"));
        assert!(!md.contains("## Transcript"));
        assert!(md.contains("- Ended: 2026-09-07T10:02:41.000Z"));
    }
}
