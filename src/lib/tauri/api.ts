/**
 * Typed API layer — the only way the app talks to the backend.
 *
 *   bluey.capture.screen()
 *   bluey.audio.start()
 *   bluey.session.start()
 *   bluey.ai.stream(request, onChunk)
 *
 * Each method is a thin wrapper over `getTransport().invoke(...)`.
 */

import type { AIChunk, AIRequest } from "../types";
import type { CommandArgs, CommandName, CommandResult } from "./commands";
import { getTransport } from "./transport";

function call<K extends CommandName>(command: K): (args: CommandArgs<K>) => Promise<CommandResult<K>>;
function call<K extends CommandName>(command: K) {
  return (args: CommandArgs<K>) => getTransport().invoke(command, args);
}

function callNoArgs<K extends CommandName>(command: K): () => Promise<CommandResult<K>> {
  return () => getTransport().invoke(command, undefined as CommandArgs<K>);
}

export const bluey = {
  app: {
    getStatus: callNoArgs("app_get_status"),
    pause: callNoArgs("app_pause"),
    resume: callNoArgs("app_resume"),
    recover: callNoArgs("app_recover"),
    dismissResponse: callNoArgs("app_dismiss_response"),
    getDevInfo: callNoArgs("app_get_dev_info"),
    runSetupChecks: callNoArgs("app_run_setup_checks"),
    quit: callNoArgs("app_quit"),
  },
  auth: {
    getStatus: callNoArgs("auth_get_status"),
    storeSession: call("auth_store_session"),
    storeToken: call("auth_store_token"),
    loadClientToken: callNoArgs("auth_load_client_token"),
    clearSession: callNoArgs("auth_clear_session"),
    fapiFetch: call("auth_fapi_fetch"),
  },
  permissions: {
    get: callNoArgs("permissions_get"),
    request: call("permissions_request"),
    openSettings: call("permissions_open_settings"),
  },
  capture: {
    listDisplays: callNoArgs("capture_list_displays"),
    listWindows: callNoArgs("capture_list_windows"),
    screen: (options?: CommandArgs<"capture_screen">["options"]) =>
      getTransport().invoke("capture_screen", { options }),
    readFrame: call("capture_read_frame"),
    discardFrame: call("capture_discard_frame"),
    observeStart: call("capture_observe_start"),
    observeStop: callNoArgs("capture_observe_stop"),
    getProtection: callNoArgs("capture_get_protection"),
    setProtection: call("capture_set_protection"),
  },
  ocr: {
    recognize: call("ocr_recognize"),
  },
  accessibility: {
    snapshot: call("accessibility_snapshot"),
    frontmostApp: callNoArgs("accessibility_frontmost_app"),
  },
  audio: {
    listDevices: callNoArgs("audio_list_devices"),
    start: (config?: CommandArgs<"audio_start">["config"]) =>
      getTransport().invoke("audio_start", { config }),
    stop: callNoArgs("audio_stop"),
    pause: callNoArgs("audio_pause"),
    resume: callNoArgs("audio_resume"),
    getStatus: callNoArgs("audio_get_status"),
    testMicrophone: call("audio_test_microphone"),
    pickRecording: callNoArgs("audio_pick_recording"),
  },
  transcript: {
    list: call("transcript_list"),
    recent: call("transcript_recent"),
    clear: call("transcript_clear"),
  },
  context: {
    buildSnapshot: call("context_build_snapshot"),
  },
  ai: {
    /**
     * Streams a request. Resolves when the command returns (the stream itself
     * completes via `completed`/`failed` chunks). Returns nothing; use `cancel`.
     */
    stream: async (request: AIRequest, onChunk: (chunk: AIChunk) => void): Promise<void> => {
      const transport = getTransport();
      const channel = transport.createChannel<AIChunk>();
      channel.onMessage(onChunk);
      await transport.invoke("ai_stream", { request, onChunk: channel.raw });
    },
    cancel: call("ai_cancel"),
    cancelAll: callNoArgs("ai_cancel_all"),
    embed: call("ai_embed"),
    testConnection: call("ai_test_connection"),
    listModels: call("ai_list_models"),
    applyProviderPresets: call("ai_apply_provider_presets"),
    transcribeFile: call("ai_transcribe_file"),
  },
  research: {
    search: call("research_search"),
    scrape: call("research_scrape"),
    deepStart: call("research_deep_start"),
    deepCancel: call("research_deep_cancel"),
    available: callNoArgs("research_available"),
  },
  modes: {
    list: callNoArgs("modes_list"),
    get: call("modes_get"),
    create: call("modes_create"),
    update: call("modes_update"),
    delete: call("modes_delete"),
    duplicate: call("modes_duplicate"),
    setDefault: call("modes_set_default"),
    setActive: call("modes_set_active"),
    resetBuiltIn: call("modes_reset_built_in"),
  },
  session: {
    start: (args: CommandArgs<"sessions_start"> = {}) => getTransport().invoke("sessions_start", args),
    pause: callNoArgs("sessions_pause"),
    resume: callNoArgs("sessions_resume"),
    end: callNoArgs("sessions_end"),
    getActive: callNoArgs("sessions_get_active"),
    list: (query?: CommandArgs<"sessions_list">["query"]) =>
      getTransport().invoke("sessions_list", { query }),
    get: call("sessions_get"),
    search: call("sessions_search"),
    delete: call("sessions_delete"),
    deleteAll: callNoArgs("sessions_delete_all"),
    rename: call("sessions_rename"),
    addEvent: call("sessions_add_event"),
    listEvents: call("sessions_list_events"),
    addNote: call("sessions_add_note"),
    deleteNote: call("sessions_delete_note"),
    saveSummary: call("sessions_save_summary"),
    getSummary: call("sessions_get_summary"),
  },
  responses: {
    save: call("responses_save"),
    list: call("responses_list"),
    get: call("responses_get"),
    feedback: call("responses_feedback"),
    delete: call("responses_delete"),
  },
  documents: {
    add: call("documents_add"),
    list: (args: CommandArgs<"documents_list"> = {}) => getTransport().invoke("documents_list", args),
    get: call("documents_get"),
    getText: call("documents_get_text"),
    delete: call("documents_delete"),
    deleteAll: (args: CommandArgs<"documents_delete_all"> = {}) =>
      getTransport().invoke("documents_delete_all", args),
    retrieve: call("documents_retrieve"),
    reindex: (args: CommandArgs<"documents_reindex"> = {}) =>
      getTransport().invoke("documents_reindex", args),
    pickFiles: callNoArgs("documents_pick_files"),
  },
  settings: {
    get: callNoArgs("settings_get"),
    update: call("settings_update"),
    reset: callNoArgs("settings_reset"),
  },
  secrets: {
    set: call("secrets_set"),
    has: call("secrets_has"),
    delete: call("secrets_delete"),
  },
  shortcuts: {
    list: callNoArgs("shortcuts_list"),
    update: call("shortcuts_update"),
    reset: callNoArgs("shortcuts_reset"),
    checkConflict: call("shortcuts_check_conflict"),
  },
  panel: {
    show: callNoArgs("panel_show"),
    hide: callNoArgs("panel_hide"),
    toggle: callNoArgs("panel_toggle"),
    move: call("panel_move"),
    setPosition: call("panel_set_position"),
    resize: call("panel_resize"),
    setExpanded: call("panel_set_expanded"),
    setOpacity: call("panel_set_opacity"),
    setPinned: call("panel_set_pinned"),
    getState: callNoArgs("panel_get_state"),
    startDrag: callNoArgs("panel_start_drag"),
  },
  window: {
    open: call("window_open"),
    close: call("window_close"),
  },
  data: {
    usageStats: callNoArgs("data_usage_stats"),
    deleteScreenshots: callNoArgs("data_delete_screenshots"),
    clearTranscripts: callNoArgs("data_clear_transcripts"),
    clearAiCache: callNoArgs("data_clear_ai_cache"),
    resetAll: callNoArgs("data_reset_all"),
    exportSession: call("data_export_session"),
  },
  dev: {
    simulate: call("dev_simulate"),
    getMetrics: callNoArgs("dev_get_metrics"),
    restartHelper: callNoArgs("dev_restart_helper"),
  },
} as const;

export type BlueyApi = typeof bluey;
