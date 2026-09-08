/** Session domain types (mirrors `bluey_core::types::session`). */

import type { BlueyResponse } from "./response";

export type SessionStatus = "active" | "paused" | "completed";

export interface Session {
  id: string;
  modeId: string;
  startedAt: string;
  endedAt?: string;
  status: SessionStatus;
  title?: string;
  metadata?: Record<string, unknown>;
}

export type SessionEventType =
  | "session_started"
  | "session_paused"
  | "session_resumed"
  | "session_ended"
  | "question_detected"
  | "coding_problem_detected"
  | "objection_detected"
  | "decision_detected"
  | "action_item_detected"
  | "topic_change"
  | "important_statement"
  | "follow_up"
  | "screen_captured"
  | "response_prepared"
  | "response_generated"
  | "response_failed"
  | "document_attached"
  | "mode_changed"
  | "note_added"
  | "summary_generated"
  | "recording_imported";

export interface SessionEvent {
  id: string;
  sessionId: string;
  type: SessionEventType;
  /** Short human label shown in the timeline, e.g. "Question detected". */
  title: string;
  /** Optional detail (question text, decision text...). */
  detail?: string;
  /** Related entity ids for inspection (responseId, transcriptSegmentId, snapshotId...). */
  refs?: Record<string, string>;
  confidence?: number;
  createdAt: string;
}

export interface SessionNote {
  id: string;
  sessionId: string;
  content: string;
  createdAt: string;
  updatedAt: string;
}

export interface SessionSummary {
  id: string;
  sessionId: string;
  modeId: string;
  overview: string;
  topics: string[];
  questions: string[];
  answers: string[];
  decisions: string[];
  actionItems: string[];
  openItems: string[];
  improvements: string[];
  /** Mode-specific extra sections (e.g. lecture study guide). */
  sections?: Array<{ title: string; content: string }>;
  createdAt: string;
}

export interface SessionListItem {
  session: Session;
  modeName: string;
  eventCount: number;
  responseCount: number;
  transcriptSegmentCount: number;
  hasSummary: boolean;
  /** Search snippet when returned from search. */
  snippet?: string;
}

export interface SessionDetail {
  session: Session;
  events: SessionEvent[];
  notes: SessionNote[];
  summary?: SessionSummary;
  responses: BlueyResponse[];
  transcriptSegmentCount: number;
}

export interface SessionSearchQuery {
  text?: string;
  modeId?: string;
  from?: string;
  to?: string;
  limit?: number;
  offset?: number;
}
