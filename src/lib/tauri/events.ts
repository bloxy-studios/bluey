/**
 * SINGLE SOURCE OF TRUTH for the event bus.
 *
 * Rust emits every event on the Tauri event `tauriEventName(name)` — the
 * `bluey:` prefix plus the dotted name with `.` replaced by `/`, because Tauri
 * v2 only accepts `[A-Za-z0-9-/:_]` in event names (`transcript.final` →
 * `bluey:transcript/final`) — with the payload below (serde camelCase). The
 * frontend subscribes through `eventBus.on(name, handler)` (see
 * `./event-bus.ts`) and never calls `listen()` directly.
 */

import type {
  AccessibilityContext,
  AIChunk,
  AppStatus,
  AudioSource,
  AudioStatus,
  AuthStatus,
  BlueyError,
  BlueyMode,
  BlueyResponse,
  ContextSnapshot,
  DeepResearchEvent,
  DetectedEvent,
  LatencyMetrics,
  OCRContext,
  PanelState,
  PermissionState,
  ScreenFrame,
  Session,
  SessionEvent,
  Settings,
  ShortcutId,
  TranscriptSegment,
  AudioDevice,
} from "../types";

export interface EventMap {
  // app
  "app.state": AppStatus;
  "app.error": BlueyError;
  "settings.changed": Settings;
  "permissions.changed": PermissionState;
  /** Browser sign-in started / finished, sign-out (ADR 0008). */
  "auth.changed": AuthStatus;
  "helper.status": { running: boolean; version?: string; restarted?: boolean; error?: BlueyError };

  // screen
  "screen.changed": { hash: string; delta: number; displayId?: string; at: string };
  "screen.captured": ScreenFrame;
  "ocr.completed": OCRContext;
  "accessibility.updated": AccessibilityContext;
  "activeApp.changed": { name: string; bundleId?: string; pid?: number; windowTitle?: string };

  // audio / transcript
  "audio.started": AudioStatus;
  "audio.stopped": AudioStatus;
  "audio.paused": AudioStatus;
  "audio.resumed": AudioStatus;
  "audio.level": { microphone: number; system: number };
  "audio.chunk": { source: AudioSource; startMs: number; endMs: number; isSpeech: boolean; rms: number };
  "audio.deviceChanged": { devices: AudioDevice[]; currentInput?: AudioDevice };
  "audio.error": BlueyError;
  "transcript.partial": TranscriptSegment;
  "transcript.final": TranscriptSegment;
  "transcript.cleared": { sessionId?: string };

  // intelligence
  "question.detected": DetectedEvent;
  "context.updated": { snapshot: ContextSnapshot; reason: string };
  "response.prepared": BlueyResponse;

  // ai
  "ai.requested": { requestId: string; task: string; sessionId?: string };
  "ai.started": { requestId: string; provider: string; model: string };
  "ai.chunk": AIChunk;
  "ai.completed": { requestId: string; totalMs: number; timeToFirstTokenMs?: number };
  "ai.failed": { requestId: string; error: BlueyError };
  "ai.cancelled": { requestId: string };

  // research
  "research.event": DeepResearchEvent;

  // sessions & modes
  "session.started": Session;
  "session.paused": Session;
  "session.resumed": Session;
  "session.ended": Session;
  "session.event": SessionEvent;
  "mode.changed": { mode: BlueyMode; sessionId?: string };
  "modes.changed": BlueyMode[];

  // shortcuts / panel
  "shortcut.triggered": { id: ShortcutId; at: string };
  "panel.state": PanelState;
  "panel.scroll": { direction: "up" | "down" };
  "panel.focusInput": Record<string, never>;
  "panel.newChat": Record<string, never>;

  // dev
  "dev.metrics": LatencyMetrics;
  "dev.log": { level: string; target: string; message: string; at: string };
}

export type EventName = keyof EventMap;
export type EventPayload<K extends EventName> = EventMap[K];

export const EVENT_PREFIX = "bluey:";

/**
 * Wire name of an event. Tauri v2 rejects event names outside
 * `[A-Za-z0-9-/:_]` (no dots), so the dotted contract name travels as
 * `bluey:` + name with `.` → `/`: `"transcript.final"` → `"bluey:transcript/final"`.
 * Mirrors `BlueyEvent::tauri_event_name` in `bluey_core::events`.
 */
export function tauriEventName(name: EventName): string {
  return `${EVENT_PREFIX}${name.replace(/\./g, "/")}`;
}

export const EVENT_NAMES: readonly EventName[] = [
  "app.state",
  "app.error",
  "settings.changed",
  "permissions.changed",
  "auth.changed",
  "helper.status",
  "screen.changed",
  "screen.captured",
  "ocr.completed",
  "accessibility.updated",
  "activeApp.changed",
  "audio.started",
  "audio.stopped",
  "audio.paused",
  "audio.resumed",
  "audio.level",
  "audio.chunk",
  "audio.deviceChanged",
  "audio.error",
  "transcript.partial",
  "transcript.final",
  "transcript.cleared",
  "question.detected",
  "context.updated",
  "response.prepared",
  "ai.requested",
  "ai.started",
  "ai.chunk",
  "ai.completed",
  "ai.failed",
  "ai.cancelled",
  "research.event",
  "session.started",
  "session.paused",
  "session.resumed",
  "session.ended",
  "session.event",
  "mode.changed",
  "modes.changed",
  "shortcut.triggered",
  "panel.state",
  "panel.scroll",
  "panel.focusInput",
  "panel.newChat",
  "dev.metrics",
  "dev.log",
] as const;
