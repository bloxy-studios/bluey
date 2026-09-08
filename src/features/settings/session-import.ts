/**
 * Importing recordings: native picker → `ai_transcribe_file` (batch
 * `gemini-3.5-transcribe`) → segments filed under a session. Pure helpers live
 * here so the Sessions tab and the session detail share one flow (and tests
 * can exercise it without driving menus).
 */

import { showErrorToast, showToast } from "@/components/ui/toast-store";
import { bluey } from "@/lib/tauri/api";
import {
  toBlueyError,
  type AIProviderKind,
  type Settings,
  type TranscribeFileResult,
  type TranscriptSegment,
} from "@/lib/types";

/** Import summaries carry counts and a title — give them longer than a "Copied" pill. */
const IMPORT_TOAST_MS = 6000;

/** Provider kinds whose adapter implements batch file transcription (mirror of the Rust gate). */
const FILE_TRANSCRIBING_KINDS: ReadonlySet<AIProviderKind> = new Set<AIProviderKind>(["google_gemini", "mock"]);

/** Title of the disabled import buttons. */
export const IMPORT_DISABLED_HINT =
  "Assign the transcription role to Google Gemini in Settings → AI to import recordings";

/**
 * True when the transcription role points at a provider that can transcribe whole
 * files (`ai_transcribe_file` returns `not_supported.transcribe_file` otherwise).
 */
export function canTranscribeFiles(settings: Pick<Settings, "ai"> | null | undefined): boolean {
  if (!settings) return false;
  const providerId = settings.ai.models.transcription?.providerId;
  const provider = settings.ai.providers.find((p) => p.id === providerId);
  return provider !== undefined && FILE_TRANSCRIBING_KINDS.has(provider.kind);
}

export interface ImportRecordingOptions {
  /** Append to this session; omitted → a new completed "Imported · <file>" session. */
  sessionId?: string;
  /** Speaker labels (`spk_n`). Default on. */
  diarization?: boolean;
  /** Word-level timings for accurate segment times. Default on. */
  wordTimestamps?: boolean;
  /** BCP-47 tag; omitted = auto-detect. */
  language?: string;
}

/**
 * Pick a recording and transcribe it. Resolves `null` when the picker is
 * cancelled or the import fails (the failure is already toasted).
 */
export async function importRecording(options: ImportRecordingOptions = {}): Promise<TranscribeFileResult | null> {
  let path: string | null;
  try {
    path = await bluey.audio.pickRecording();
  } catch (error) {
    showErrorToast(toBlueyError(error, "audio"));
    return null;
  }
  if (!path) return null;
  try {
    const result = await bluey.ai.transcribeFile({
      path,
      diarization: options.diarization ?? true,
      wordTimestamps: options.wordTimestamps ?? true,
      language: options.language,
      sessionId: options.sessionId,
    });
    showToast(describeImport(result), IMPORT_TOAST_MS);
    return result;
  } catch (error) {
    showErrorToast(toBlueyError(error, "ai"));
    return null;
  }
}

/** Toast copy for a finished import. */
export function describeImport(result: TranscribeFileResult): string {
  const count = result.segments.length;
  const title = result.session.title ?? "Untitled session";
  const speakers =
    result.speakers > 0 ? ` from ${result.speakers} speaker${result.speakers === 1 ? "" : "s"}` : "";
  const stored = result.stored ? "" : " — not stored (transcript storage is off)";
  return `Imported ${count} segment${count === 1 ? "" : "s"}${speakers} into “${title}”${stored}`;
}

/** `spk_1` → "Speaker 1"; other labels verbatim; unlabelled → You / Them by source. */
export function speakerLabel(segment: Pick<TranscriptSegment, "speaker" | "source">): string {
  const match = segment.speaker?.match(/^spk_(\d+)$/);
  if (match) return `Speaker ${match[1]}`;
  if (segment.speaker) return segment.speaker;
  return segment.source === "microphone" ? "You" : "Them";
}

/** Milliseconds from the start of a recording → `mm:ss` (or `h:mm:ss`). */
export function formatOffset(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const seconds = total % 60;
  const mm = String(minutes).padStart(2, "0");
  const ss = String(seconds).padStart(2, "0");
  return hours > 0 ? `${hours}:${mm}:${ss}` : `${mm}:${ss}`;
}
