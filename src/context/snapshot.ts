/**
 * Native snapshot building + TS-side enrichment.
 *
 * `buildNativeSnapshot` asks Rust for the fast-path snapshot with
 * settings-driven options (screen only when the mode requires it or the
 * trigger is a capture). `enrichSnapshot` then fills the TS-owned parts:
 * the chat thread, session context, user context (retrieved chunks +
 * personal instructions), mode context and the explicit user instruction.
 */

import type { AskTrigger } from "@/lib/engine-contract";
import type {
  BlueyMode,
  BlueyResponse,
  CaptureTarget,
  ContextSnapshot,
  ConversationTurn,
  DetectedEvent,
  RetrievedChunk,
  Session,
  SessionContext,
  SessionEvent,
  SessionNote,
  Settings,
  SnapshotOptions,
  TranscriptSegment,
} from "@/lib/types";
import { effectiveStyle, requires } from "@/modes/registry";
import { isSpokenAsk } from "./relevance";

export interface SnapshotApi {
  context: {
    buildSnapshot(args: { options: SnapshotOptions }): Promise<ContextSnapshot>;
  };
}

export interface BuildNativeSnapshotArgs {
  mode: BlueyMode;
  settings: Settings;
  trigger: AskTrigger;
  /** Force screen inclusion regardless of mode requirements. */
  captureScreen?: boolean;
  /**
   * `false` when the user turned screen context off in the HUD: no capture,
   * OCR or accessibility tree, whatever the trigger or mode asks for.
   */
  screenAllowed?: boolean;
  transcriptWindowSeconds?: number;
  /** The heard question the ask answers: its conversation is part of the context. */
  detectedEvent?: DetectedEvent;
  api: SnapshotApi;
}

export const DEFAULT_TRANSCRIPT_WINDOW_SECONDS = 180;

function captureTargetFor(settings: Settings): CaptureTarget {
  switch (settings.screen.captureTarget) {
    case "active_window":
      return { type: "active_window" };
    case "display":
    default: {
      const preferred = settings.screen.preferredDisplay;
      return preferred === "active" ? { type: "display" } : { type: "display", displayId: preferred };
    }
  }
}

/** Settings-driven snapshot options for the Rust fast path. */
export function snapshotOptionsFor(args: Omit<BuildNativeSnapshotArgs, "api">): SnapshotOptions {
  const { mode, settings, trigger, captureScreen, transcriptWindowSeconds } = args;
  const screenAllowed = args.screenAllowed !== false;
  const includeScreen =
    screenAllowed && (captureScreen === true || trigger === "shortcut_capture" || requires(mode, "screen"));
  const includeTranscript = requires(mode, "transcript") || isSpokenAsk(trigger, args.detectedEvent);

  const options: SnapshotOptions = {
    includeScreen,
    includeOcr: includeScreen,
    includeAccessibility: includeScreen || (screenAllowed && requires(mode, "accessibility")),
    includeTranscript,
    ocrLevel: settings.screen.ocrLevel,
    inlineImage: includeScreen,
  };
  if (includeTranscript) {
    options.transcriptWindowSeconds = transcriptWindowSeconds ?? DEFAULT_TRANSCRIPT_WINDOW_SECONDS;
  }
  if (includeScreen) {
    options.capture = {
      target: captureTargetFor(settings),
      format: "jpeg",
      quality: 0.8,
      maxDimension: settings.screen.maxImageDimension,
      inline: true,
      changeDetection: false,
    };
  }
  return options;
}

/** Build the native part of the snapshot via `bluey.context.buildSnapshot`. */
export async function buildNativeSnapshot(args: BuildNativeSnapshotArgs): Promise<ContextSnapshot> {
  const options = snapshotOptionsFor(args);
  return args.api.context.buildSnapshot({ options });
}

export interface EnrichSnapshotArgs {
  mode: BlueyMode;
  settings: Settings;
  session?: Session | null;
  instruction?: string;
  previousResponses?: BlueyResponse[];
  /** Explicit transcript override (e.g. UI-held rolling window). */
  transcriptSegments?: TranscriptSegment[];
  retrieved?: RetrievedChunk[];
  /** Recent session timeline events supplied by the UI (most recent last). */
  sessionEvents?: SessionEvent[];
  /** Session notes supplied by the UI. */
  sessionNotes?: SessionNote[];
  /** Ids of documents attached to the session. */
  sessionDocumentIds?: string[];
}

const RECENT_RESPONSE_LIMIT = 5;
/** Chat turns carried into the prompt as conversation memory (fusion renders the last few). */
const CONVERSATION_TURN_LIMIT = 5;
const RECENT_RESPONSE_CHARS = 320;
const RECENT_EVENT_LIMIT = 12;
const NOTE_LIMIT = 10;
const NOTE_CHARS = 400;
/** Personal instructions apply to every answer; cap them so they stay a preamble. */
export const PERSONAL_INSTRUCTIONS_CHARS = 1500;

/**
 * Every `personal_instructions` chunk (global, mode and session scope, in the
 * order retrieval returned them), joined and capped. Undefined when none.
 */
function joinPersonalInstructions(chunks: RetrievedChunk[]): string | undefined {
  const text = chunks
    .filter((chunk) => chunk.documentKind === "personal_instructions")
    .map((chunk) => chunk.content.trim())
    .filter((content) => content.length > 0)
    .join("\n\n");
  if (text.length === 0) return undefined;
  return text.length > PERSONAL_INSTRUCTIONS_CHARS ? `${text.slice(0, PERSONAL_INSTRUCTIONS_CHARS).trimEnd()}…` : text;
}

interface SessionExtras {
  events?: SessionEvent[];
  notes?: SessionNote[];
  documentIds?: string[];
}

function toConversation(previousResponses: BlueyResponse[]): ConversationTurn[] {
  return previousResponses.slice(-CONVERSATION_TURN_LIMIT).map((response) => ({
    id: response.id,
    prompt: response.prompt,
    title: response.title,
    content: response.content,
    code: response.code,
    createdAt: response.createdAt,
  }));
}

function toSessionContext(
  session: Session,
  previousResponses: BlueyResponse[] | undefined,
  existing: SessionContext | undefined,
  extras: SessionExtras = {},
): SessionContext {
  // Without chat turns from the UI, keep what the native builder loaded from the DB.
  const recentResponses =
    previousResponses === undefined
      ? (existing?.recentResponses ?? [])
      : previousResponses.slice(-RECENT_RESPONSE_LIMIT).map((response) => ({
          id: response.id,
          title: response.title,
          content:
            response.content.length > RECENT_RESPONSE_CHARS
              ? `${response.content.slice(0, RECENT_RESPONSE_CHARS)}…`
              : response.content,
          createdAt: response.createdAt,
        }));
  return {
    sessionId: session.id,
    modeId: session.modeId,
    startedAt: session.startedAt,
    recentResponses,
    recentEvents: extras.events?.length
      ? extras.events.slice(-RECENT_EVENT_LIMIT)
      : (existing?.recentEvents ?? []),
    notes: extras.notes?.length
      ? extras.notes.slice(-NOTE_LIMIT).map((note) =>
          note.content.length > NOTE_CHARS ? `${note.content.slice(0, NOTE_CHARS)}…` : note.content,
        )
      : (existing?.notes ?? []),
    documentIds: extras.documentIds?.length ? [...extras.documentIds] : (existing?.documentIds ?? []),
  };
}

/**
 * Enrich a native snapshot with the TS-owned context. Pure: returns a new
 * snapshot, never mutates the input.
 */
export function enrichSnapshot(snapshot: ContextSnapshot, args: EnrichSnapshotArgs): ContextSnapshot {
  const {
    mode,
    settings,
    session,
    instruction,
    previousResponses,
    transcriptSegments,
    retrieved,
    sessionEvents,
    sessionNotes,
    sessionDocumentIds,
  } = args;

  const chunks = retrieved ?? snapshot.userContext?.chunks ?? [];
  const contentChunks = chunks.filter((chunk) => chunk.documentKind !== "personal_instructions");

  const enriched: ContextSnapshot = {
    ...snapshot,
    mode: { mode, responseStyle: effectiveStyle(mode, settings) },
    userContext: {
      chunks: contentChunks,
      personalInstructions: joinPersonalInstructions(chunks) ?? snapshot.userContext?.personalInstructions,
      displayName: snapshot.userContext?.displayName,
    },
  };

  if (instruction !== undefined && instruction.trim().length > 0) {
    enriched.userInstruction = instruction.trim();
  }
  // The chat thread is conversation memory whether or not a session is active.
  if (previousResponses && previousResponses.length > 0) {
    enriched.conversation = toConversation(previousResponses);
  }
  if (session) {
    enriched.session = toSessionContext(session, previousResponses, snapshot.session, {
      events: sessionEvents,
      notes: sessionNotes,
      documentIds: sessionDocumentIds,
    });
  }
  if (transcriptSegments && transcriptSegments.length > 0) {
    enriched.transcript = {
      segments: transcriptSegments,
      earlierSummary: snapshot.transcript?.earlierSummary,
      windowSeconds: snapshot.transcript?.windowSeconds ?? DEFAULT_TRANSCRIPT_WINDOW_SECONDS,
    };
  }
  return enriched;
}
