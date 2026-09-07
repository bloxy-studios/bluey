/**
 * Native snapshot building + TS-side enrichment.
 *
 * `buildNativeSnapshot` asks Rust for the fast-path snapshot with
 * settings-driven options (screen only when the mode requires it or the
 * trigger is a capture). `enrichSnapshot` then fills the TS-owned parts:
 * session context, user context (retrieved chunks + personal instructions),
 * mode context and the explicit user instruction.
 */

import type { AskTrigger } from "@/lib/engine-contract";
import type {
  BlueyMode,
  BlueyResponse,
  CaptureTarget,
  ContextSnapshot,
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
  transcriptWindowSeconds?: number;
  api: SnapshotApi;
}

export const DEFAULT_TRANSCRIPT_WINDOW_SECONDS = 180;

function captureTargetFor(settings: Settings): CaptureTarget {
  switch (settings.screen.captureTarget) {
    case "active_window":
      return { type: "active_window" };
    case "display":
    case "region":
    default: {
      const preferred = settings.screen.preferredDisplay;
      return preferred === "active" ? { type: "display" } : { type: "display", displayId: preferred };
    }
  }
}

/** Settings-driven snapshot options for the Rust fast path. */
export function snapshotOptionsFor(args: Omit<BuildNativeSnapshotArgs, "api">): SnapshotOptions {
  const { mode, settings, trigger, captureScreen, transcriptWindowSeconds } = args;
  const includeScreen =
    captureScreen === true || trigger === "shortcut_capture" || requires(mode, "screen");
  const includeTranscript = requires(mode, "transcript") || trigger === "shortcut_generate" || trigger === "detected_event";

  const options: SnapshotOptions = {
    includeScreen,
    includeOcr: includeScreen,
    includeAccessibility: includeScreen || requires(mode, "accessibility"),
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
const RECENT_RESPONSE_CHARS = 320;
const RECENT_EVENT_LIMIT = 12;
const NOTE_LIMIT = 10;
const NOTE_CHARS = 400;

interface SessionExtras {
  events?: SessionEvent[];
  notes?: SessionNote[];
  documentIds?: string[];
}

function toSessionContext(
  session: Session,
  previousResponses: BlueyResponse[] | undefined,
  existing: SessionContext | undefined,
  extras: SessionExtras = {},
): SessionContext {
  const recentResponses = (previousResponses ?? [])
    .slice(-RECENT_RESPONSE_LIMIT)
    .map((response) => ({
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
  const personalChunk = chunks.find((chunk) => chunk.documentKind === "personal_instructions");
  const contentChunks = chunks.filter((chunk) => chunk.documentKind !== "personal_instructions");

  const enriched: ContextSnapshot = {
    ...snapshot,
    mode: { mode, responseStyle: effectiveStyle(mode, settings) },
    userContext: {
      chunks: contentChunks,
      personalInstructions: personalChunk?.content ?? snapshot.userContext?.personalInstructions,
      displayName: snapshot.userContext?.displayName,
    },
  };

  if (instruction !== undefined && instruction.trim().length > 0) {
    enriched.userInstruction = instruction.trim();
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
