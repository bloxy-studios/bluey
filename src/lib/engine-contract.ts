/**
 * Contract between the UI layer (stores/HUD) and the intelligence layer (`src/ai`).
 *
 * The UI never builds prompts or talks to providers; it hands the engine an
 * `AskInput` and renders the streamed `BlueyResponse` drafts. The engine never
 * touches React or Zustand; it uses the typed API (`bluey.*`) and the event bus.
 */

import type {
  BlueyError,
  BlueyMode,
  BlueyResponse,
  ContextSnapshot,
  DetectedEvent,
  Session,
  SessionEvent,
  SessionNote,
  SessionSummary,
  Settings,
  TranscriptSegment,
} from "./types";

export type AskTrigger =
  | "shortcut_capture" // ⌘↵ — capture + analyze
  | "shortcut_generate" // ⌘⇧↵ — generate a suggested response from transcript
  | "typed" // user typed a question in the HUD
  | "follow_up" // follow-up in an existing response thread
  | "detected_event" // proactive preparation for a detected question
  | "regenerate"
  | "assist"; // "Assist" button — no instruction, infer from context

export interface AskInput {
  trigger: AskTrigger;
  /** Explicit user question (highest priority context). */
  instruction?: string;
  /** Pre-built native snapshot. When absent and `captureScreen` is true the engine builds one. */
  snapshot?: ContextSnapshot;
  captureScreen: boolean;
  mode: BlueyMode;
  session?: Session | null;
  settings: Settings;
  /** Earlier responses in this chat for continuity (most recent last). */
  previousResponses?: BlueyResponse[];
  detectedEvent?: DetectedEvent;
  /** Explicit transcript window override (seconds). */
  transcriptWindowSeconds?: number;
  /** Recent session timeline events (for `SessionContext.recentEvents`). */
  sessionEvents?: SessionEvent[];
  /** Session notes (for `SessionContext.notes`). */
  sessionNotes?: SessionNote[];
  /** Documents attached to the session (for `SessionContext.documentIds`). */
  sessionDocumentIds?: string[];
  /** The global shortcut's keydown on Bluey's monotonic clock — the fast-path trace's `tShortcut` (ADR 0010 §2). */
  triggeredAtMs?: number;
}

export type EnginePhase = "capturing" | "analyzing" | "thinking" | "streaming" | "done" | "error" | "cancelled";

export interface EngineCallbacks {
  onPhase?(phase: EnginePhase, requestId: string): void;
  /** Called repeatedly while streaming; `response.content` grows monotonically. */
  onDraft?(response: BlueyResponse): void;
  onComplete?(response: BlueyResponse): void;
  onError?(error: BlueyError, requestId: string): void;
}

export interface EngineHandle {
  requestId: string;
  /** Monotonic generation used for stale-response protection. */
  generation: number;
  cancel(): Promise<void>;
  /** Resolves with the final response, or null when cancelled/failed. */
  done: Promise<BlueyResponse | null>;
}

export interface SummarizeInput {
  session: Session;
  mode: BlueyMode;
  transcript: TranscriptSegment[];
  responses: BlueyResponse[];
  events: SessionEvent[];
  notes: SessionNote[];
  settings: Settings;
}

export interface ClassifyInput {
  segment: TranscriptSegment;
  recent: TranscriptSegment[];
  mode: BlueyMode;
  settings: Settings;
}

export interface ResponseEngine {
  /** Start a request. Any in-flight request for the same window is superseded (never overwritten by stale output). */
  ask(input: AskInput, callbacks?: EngineCallbacks): EngineHandle;
  /**
   * Proactive preparation for a detected question. Silent by default: the result is
   * cached for `takePrepared` and announced as `response.prepared`. With
   * `callbacks.onComplete` the answer is *live*: phases, drafts, completion and errors
   * go to the callbacks, the finished response is handed to `onComplete` and persisted
   * like any shown answer, and it is neither cached nor announced — the HUD shows it as
   * it is written.
   */
  prepare(input: AskInput, callbacks?: EngineCallbacks): Promise<BlueyResponse | null>;
  /** Pop a prepared response (optionally for a specific detected event id). */
  takePrepared(eventId?: string): BlueyResponse | null;
  /** Lightweight transcript classification (heuristics first, fast model when configured). */
  classify(input: ClassifyInput): Promise<DetectedEvent | null>;
  /** Post-session summary structured by mode. */
  summarizeSession(input: SummarizeInput): Promise<SessionSummary>;
  /** Cancel everything in flight. */
  cancelAll(): Promise<void>;
}
