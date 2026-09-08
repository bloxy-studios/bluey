/**
 * SINGLE SOURCE OF TRUTH for the Tauri command surface.
 *
 * Every Rust `#[tauri::command]` is listed here with its argument and return
 * types. The frontend never calls `invoke("...")` directly — it goes through
 * `bluey.*` (see `./api.ts`) which is generated from this map. The Rust side
 * (`src-tauri/src/commands/*`) must keep names and shapes in sync; the
 * integration test `tests/integration/command-surface.test.ts` checks that the
 * Rust `generate_handler!` list matches `COMMAND_NAMES`.
 *
 * Naming: `<domain>_<verb>` in snake_case. Args are always a single object
 * (Tauri maps its keys to camelCase parameters).
 */

import type {
  AIRequest,
  AudioDevice,
  AudioSessionConfig,
  AudioStatus,
  AuthStatus,
  AuthUser,
  BlueyDocument,
  BlueyMode,
  BlueyResponse,
  CaptureOptions,
  CaptureProtection,
  ConnectionTestResult,
  ContextSnapshot,
  DeepResearchRequest,
  DevInfo,
  DevSimulation,
  DisplayInfo,
  DocumentScope,
  FeedbackCategory,
  FeedbackRating,
  LatencyMetrics,
  ModeDraft,
  ModePatch,
  OCRContext,
  AccessibilityContext,
  ApplicationContext,
  PermissionKind,
  PermissionState,
  RetrievedChunk,
  RetrievalQuery,
  AddDocumentInput,
  ScrapeResult,
  ScreenFrame,
  SearchResult,
  Session,
  SessionDetail,
  SessionEvent,
  SessionEventType,
  SessionListItem,
  SessionNote,
  SessionSearchQuery,
  SessionSummary,
  EmbedPurpose,
  ModelRole,
  Settings,
  SettingsPatch,
  SetupCheck,
  ShortcutBinding,
  ShortcutConflict,
  ShortcutId,
  SnapshotOptions,
  AppStatus,
  PanelState,
  TranscriptSegment,
  WindowContext,
} from "../types";

/** A window listed by the native helper (for window-capture selection). */
export interface CapturableWindow {
  windowId: number;
  title?: string;
  ownerName: string;
  bundleId?: string;
  pid: number;
  bounds: { x: number; y: number; width: number; height: number };
  onScreen: boolean;
}

export interface DataUsageStats {
  sessions: number;
  responses: number;
  transcriptSegments: number;
  screenshots: number;
  documents: number;
  dbSizeBytes: number;
  screenshotCacheBytes: number;
}

export type WindowLabel = "main" | "settings" | "onboarding";
export type PanelMoveDirection = "up" | "down" | "left" | "right";

/**
 * Command map: name -> { args, result }.
 * `void` args means the command takes no parameters.
 */
export interface CommandMap {
  // ── App ────────────────────────────────────────────────────────────────
  app_get_status: { args: void; result: AppStatus };
  app_pause: { args: void; result: AppStatus };
  app_resume: { args: void; result: AppStatus };
  app_recover: { args: void; result: AppStatus };
  app_dismiss_response: { args: void; result: AppStatus };
  app_get_dev_info: { args: void; result: DevInfo };
  app_run_setup_checks: { args: void; result: SetupCheck[] };
  app_quit: { args: void; result: void };

  // ── Auth (Clerk runs in the WebView; Rust only stores the client token) ─
  auth_get_status: { args: void; result: AuthStatus };
  auth_store_session: { args: { clientToken: string; user: AuthUser }; result: AuthStatus };
  /** Persist a rotated Clerk client JWT alone (user unchanged). */
  auth_store_token: { args: { clientToken: string }; result: void };
  auth_load_client_token: { args: void; result: string | null };
  auth_clear_session: { args: void; result: AuthStatus };
  /**
   * Fallback proxy for Clerk Frontend API calls when the WebView origin makes the
   * Origin+Authorization header combination fail. Only allows https URLs on the
   * configured Clerk domain; never logs bodies.
   */
  auth_fapi_fetch: {
    args: { url: string; method: string; headers: Record<string, string>; body?: string };
    result: { status: number; headers: Record<string, string>; body: string };
  };

  // ── Permissions ────────────────────────────────────────────────────────
  permissions_get: { args: void; result: PermissionState };
  permissions_request: { args: { kind: PermissionKind }; result: PermissionState };
  permissions_open_settings: { args: { kind: PermissionKind }; result: void };

  // ── Screen capture ─────────────────────────────────────────────────────
  capture_list_displays: { args: void; result: DisplayInfo[] };
  capture_list_windows: { args: void; result: CapturableWindow[] };
  capture_screen: { args: { options?: CaptureOptions }; result: ScreenFrame };
  capture_read_frame: { args: { frameId: string }; result: string /* base64 */ };
  capture_discard_frame: { args: { frameId: string }; result: void };
  capture_observe_start: { args: { intervalMs?: number; displayId?: string }; result: void };
  capture_observe_stop: { args: void; result: void };
  capture_get_protection: { args: void; result: CaptureProtection };
  capture_set_protection: { args: { enabled: boolean }; result: CaptureProtection };

  // ── OCR / Accessibility ────────────────────────────────────────────────
  ocr_recognize: {
    args: { frameId: string; level?: "fast" | "accurate"; languages?: string[] };
    result: OCRContext;
  };
  accessibility_snapshot: { args: { maxDepth?: number; maxElements?: number }; result: AccessibilityContext };
  accessibility_frontmost_app: {
    args: void;
    result: { application: ApplicationContext; window?: WindowContext };
  };

  // ── Audio / Transcript ─────────────────────────────────────────────────
  audio_list_devices: { args: void; result: AudioDevice[] };
  audio_start: { args: { config?: Partial<AudioSessionConfig> }; result: AudioStatus };
  audio_stop: { args: void; result: AudioStatus };
  audio_pause: { args: void; result: AudioStatus };
  audio_resume: { args: void; result: AudioStatus };
  audio_get_status: { args: void; result: AudioStatus };
  audio_test_microphone: {
    args: { deviceId?: string; durationMs?: number };
    result: { peakLevel: number; ok: boolean };
  };
  transcript_list: {
    args: { sessionId?: string; sinceMs?: number; limit?: number };
    result: TranscriptSegment[];
  };
  transcript_recent: { args: { windowSeconds: number }; result: TranscriptSegment[] };
  transcript_clear: { args: { sessionId?: string }; result: void };

  // ── Context (fast path) ────────────────────────────────────────────────
  context_build_snapshot: { args: { options: SnapshotOptions }; result: ContextSnapshot };

  // ── AI (Rust owns credentials; streams via Channel) ────────────────────
  /** `onChunk` is a `Channel<AIChunk>` — see api.ts. */
  ai_stream: { args: { request: AIRequest; onChunk: unknown }; result: void };
  ai_cancel: { args: { requestId: string }; result: boolean };
  ai_cancel_all: { args: void; result: number };
  /** `purpose` defaults to `document`; queries get the retrieval prompt prefix on gemini-embedding-2. */
  ai_embed: { args: { texts: string[]; purpose?: EmbedPurpose }; result: number[][] };
  ai_test_connection: { args: { providerId: string; model?: string }; result: ConnectionTestResult };
  /** `role` narrows the catalogue to models fit for that role (embedding, transcription, text). */
  ai_list_models: { args: { providerId: string; role?: ModelRole }; result: string[] };
  /** Point roles at the provider's recommended models; `overwrite: false` fills only unassigned roles. */
  ai_apply_provider_presets: { args: { providerId: string; overwrite: boolean }; result: Settings };

  // ── Research ───────────────────────────────────────────────────────────
  research_search: { args: { query: string; numResults?: number }; result: SearchResult[] };
  research_scrape: { args: { url: string }; result: ScrapeResult };
  research_deep_start: { args: { request: DeepResearchRequest }; result: void };
  research_deep_cancel: { args: { jobId: string }; result: boolean };
  research_available: { args: void; result: { search: boolean; scrape: boolean; deepAgent: boolean } };

  // ── Modes ──────────────────────────────────────────────────────────────
  modes_list: { args: void; result: BlueyMode[] };
  modes_get: { args: { id: string }; result: BlueyMode };
  modes_create: { args: { draft: ModeDraft }; result: BlueyMode };
  modes_update: { args: { id: string; patch: ModePatch }; result: BlueyMode };
  modes_delete: { args: { id: string }; result: void };
  modes_duplicate: { args: { id: string }; result: BlueyMode };
  modes_set_default: { args: { id: string }; result: Settings };
  modes_set_active: { args: { id: string }; result: AppStatus };
  modes_reset_built_in: { args: { id: string }; result: BlueyMode };

  // ── Sessions ───────────────────────────────────────────────────────────
  sessions_start: { args: { modeId?: string; title?: string }; result: Session };
  sessions_pause: { args: void; result: Session };
  sessions_resume: { args: void; result: Session };
  sessions_end: { args: void; result: Session };
  sessions_get_active: { args: void; result: Session | null };
  sessions_list: { args: { query?: SessionSearchQuery }; result: SessionListItem[] };
  sessions_get: { args: { id: string }; result: SessionDetail };
  sessions_search: { args: { query: SessionSearchQuery }; result: SessionListItem[] };
  sessions_delete: { args: { id: string }; result: void };
  sessions_delete_all: { args: void; result: number };
  sessions_rename: { args: { id: string; title: string }; result: Session };
  sessions_add_event: {
    args: {
      sessionId: string;
      type: SessionEventType;
      title: string;
      detail?: string;
      refs?: Record<string, string>;
      confidence?: number;
    };
    result: SessionEvent;
  };
  sessions_list_events: { args: { sessionId: string }; result: SessionEvent[] };
  sessions_add_note: { args: { sessionId: string; content: string }; result: SessionNote };
  sessions_delete_note: { args: { noteId: string }; result: void };
  sessions_save_summary: {
    args: { summary: Omit<SessionSummary, "id" | "createdAt"> };
    result: SessionSummary;
  };
  sessions_get_summary: { args: { sessionId: string }; result: SessionSummary | null };

  // ── Responses ──────────────────────────────────────────────────────────
  responses_save: { args: { response: BlueyResponse }; result: BlueyResponse };
  responses_list: { args: { sessionId: string; limit?: number }; result: BlueyResponse[] };
  responses_get: { args: { id: string }; result: BlueyResponse };
  responses_feedback: {
    args: { responseId: string; rating: FeedbackRating; categories?: FeedbackCategory[]; comment?: string };
    result: BlueyResponse;
  };
  responses_delete: { args: { id: string }; result: void };

  // ── Documents (My Context / session / mode) ────────────────────────────
  documents_add: { args: { input: AddDocumentInput }; result: BlueyDocument };
  documents_list: { args: { scope?: DocumentScope; scopeId?: string }; result: BlueyDocument[] };
  documents_get: { args: { id: string }; result: BlueyDocument };
  documents_get_text: { args: { id: string }; result: string };
  documents_delete: { args: { id: string }; result: void };
  documents_delete_all: { args: { scope?: DocumentScope }; result: number };
  documents_retrieve: { args: { query: RetrievalQuery }; result: RetrievedChunk[] };
  documents_reindex: { args: { id?: string }; result: number };
  documents_pick_files: { args: void; result: string[] /* absolute paths chosen in a native dialog */ };

  // ── Settings & secrets ─────────────────────────────────────────────────
  settings_get: { args: void; result: Settings };
  settings_update: { args: { patch: SettingsPatch }; result: Settings };
  settings_reset: { args: void; result: Settings };
  secrets_set: { args: { key: string; value: string }; result: void };
  secrets_has: { args: { key: string }; result: boolean };
  secrets_delete: { args: { key: string }; result: void };

  // ── Shortcuts ──────────────────────────────────────────────────────────
  shortcuts_list: { args: void; result: ShortcutBinding[] };
  shortcuts_update: {
    args: { id: ShortcutId; accelerator: string; enabled?: boolean };
    result: ShortcutBinding[];
  };
  shortcuts_reset: { args: void; result: ShortcutBinding[] };
  shortcuts_check_conflict: {
    args: { accelerator: string; ignoreId?: ShortcutId };
    result: ShortcutConflict | null;
  };

  // ── Panel / windows ────────────────────────────────────────────────────
  panel_show: { args: void; result: PanelState };
  panel_hide: { args: void; result: PanelState };
  panel_toggle: { args: void; result: PanelState };
  panel_move: { args: { direction: PanelMoveDirection; stepPx?: number }; result: PanelState };
  panel_set_position: { args: { x: number; y: number }; result: PanelState };
  panel_resize: { args: { width: number; height: number }; result: PanelState };
  panel_set_expanded: { args: { expanded: boolean; height?: number }; result: PanelState };
  panel_set_opacity: { args: { opacity: number }; result: PanelState };
  panel_set_pinned: { args: { pinned: boolean }; result: PanelState };
  panel_get_state: { args: void; result: PanelState };
  panel_start_drag: { args: void; result: void };
  /**
   * Show + focus a window. `route` is a settings tab id (see
   * `src/features/settings/settings-nav.ts`); Rust delivers it to the window as the
   * `?tab=<route>` query parameter (or navigates the already-open window to it).
   */
  window_open: { args: { label: WindowLabel; route?: string }; result: void };
  window_close: { args: { label: WindowLabel }; result: void };

  // ── Data management ────────────────────────────────────────────────────
  data_usage_stats: { args: void; result: DataUsageStats };
  data_delete_screenshots: { args: void; result: number };
  data_clear_transcripts: { args: void; result: number };
  data_clear_ai_cache: { args: void; result: number };
  data_reset_all: { args: void; result: void };
  data_export_session: { args: { sessionId: string; format: "markdown" | "json" }; result: string };

  // ── Developer mode ─────────────────────────────────────────────────────
  dev_simulate: { args: { simulation: DevSimulation }; result: void };
  dev_get_metrics: { args: void; result: LatencyMetrics };
  dev_restart_helper: { args: void; result: void };
}

export type CommandName = keyof CommandMap;
export type CommandArgs<K extends CommandName> = CommandMap[K]["args"];
export type CommandResult<K extends CommandName> = CommandMap[K]["result"];

/** Exhaustive list (kept in sync with Rust `generate_handler!`). */
export const COMMAND_NAMES: readonly CommandName[] = [
  "app_get_status",
  "app_pause",
  "app_resume",
  "app_recover",
  "app_dismiss_response",
  "app_get_dev_info",
  "app_run_setup_checks",
  "app_quit",
  "auth_get_status",
  "auth_store_session",
  "auth_store_token",
  "auth_load_client_token",
  "auth_clear_session",
  "auth_fapi_fetch",
  "permissions_get",
  "permissions_request",
  "permissions_open_settings",
  "capture_list_displays",
  "capture_list_windows",
  "capture_screen",
  "capture_read_frame",
  "capture_discard_frame",
  "capture_observe_start",
  "capture_observe_stop",
  "capture_get_protection",
  "capture_set_protection",
  "ocr_recognize",
  "accessibility_snapshot",
  "accessibility_frontmost_app",
  "audio_list_devices",
  "audio_start",
  "audio_stop",
  "audio_pause",
  "audio_resume",
  "audio_get_status",
  "audio_test_microphone",
  "transcript_list",
  "transcript_recent",
  "transcript_clear",
  "context_build_snapshot",
  "ai_stream",
  "ai_cancel",
  "ai_cancel_all",
  "ai_embed",
  "ai_test_connection",
  "ai_list_models",
  "ai_apply_provider_presets",
  "research_search",
  "research_scrape",
  "research_deep_start",
  "research_deep_cancel",
  "research_available",
  "modes_list",
  "modes_get",
  "modes_create",
  "modes_update",
  "modes_delete",
  "modes_duplicate",
  "modes_set_default",
  "modes_set_active",
  "modes_reset_built_in",
  "sessions_start",
  "sessions_pause",
  "sessions_resume",
  "sessions_end",
  "sessions_get_active",
  "sessions_list",
  "sessions_get",
  "sessions_search",
  "sessions_delete",
  "sessions_delete_all",
  "sessions_rename",
  "sessions_add_event",
  "sessions_list_events",
  "sessions_add_note",
  "sessions_delete_note",
  "sessions_save_summary",
  "sessions_get_summary",
  "responses_save",
  "responses_list",
  "responses_get",
  "responses_feedback",
  "responses_delete",
  "documents_add",
  "documents_list",
  "documents_get",
  "documents_get_text",
  "documents_delete",
  "documents_delete_all",
  "documents_retrieve",
  "documents_reindex",
  "documents_pick_files",
  "settings_get",
  "settings_update",
  "settings_reset",
  "secrets_set",
  "secrets_has",
  "secrets_delete",
  "shortcuts_list",
  "shortcuts_update",
  "shortcuts_reset",
  "shortcuts_check_conflict",
  "panel_show",
  "panel_hide",
  "panel_toggle",
  "panel_move",
  "panel_set_position",
  "panel_resize",
  "panel_set_expanded",
  "panel_set_opacity",
  "panel_set_pinned",
  "panel_get_state",
  "panel_start_drag",
  "window_open",
  "window_close",
  "data_usage_stats",
  "data_delete_screenshots",
  "data_clear_transcripts",
  "data_clear_ai_cache",
  "data_reset_all",
  "data_export_session",
  "dev_simulate",
  "dev_get_metrics",
  "dev_restart_helper",
] as const;

/** Secret keys used with `secrets_set` / `secrets_has` (values live in the OS keychain). */
export const SECRET_KEYS = {
  providerApiKey: (providerId: string) => `provider:${providerId}:api_key`,
  exaApiKey: "research:exa:api_key",
  firecrawlApiKey: "research:firecrawl:api_key",
  anthropicAgentApiKey: "agent:anthropic:api_key",
  clerkClientToken: "auth:clerk:client_token",
} as const;
