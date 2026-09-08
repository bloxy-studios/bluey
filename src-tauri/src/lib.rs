//! Bluey — Tauri v2 application crate (macOS). Managers live in their modules,
//! the typed command surface in [`commands`], and [`app`] wires everything
//! together at startup. The handler list in [`run`] is the single command
//! registration; `tests/integration/command-surface.test.ts` keeps it in sync
//! with `src/lib/tauri/commands.ts` (it parses this file, so the macro name
//! must not appear anywhere above the real invocation).

pub mod accessibility;
pub mod agent;
pub mod ai;
pub mod app;
pub mod audio;
pub mod auth;
pub mod capture;
pub mod commands;
pub mod context;
pub mod documents;
pub mod events;
pub mod logging;
pub mod modes;
pub mod overlay;
pub mod permissions;
pub mod platform;
pub mod research;
pub mod secrets;
pub mod sessions;
pub mod settings;
pub mod shortcuts;
pub mod sidecar;
pub mod state;
pub mod storage;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default().invoke_handler(tauri::generate_handler![
        // App
        commands::app::app_get_status,
        commands::app::app_pause,
        commands::app::app_resume,
        commands::app::app_recover,
        commands::app::app_dismiss_response,
        commands::app::app_get_dev_info,
        commands::app::app_run_setup_checks,
        commands::app::app_quit,
        // Auth
        commands::auth::auth_get_status,
        commands::auth::auth_store_session,
        commands::auth::auth_store_token,
        commands::auth::auth_load_client_token,
        commands::auth::auth_clear_session,
        commands::auth::auth_fapi_fetch,
        // Permissions
        commands::permissions::permissions_get,
        commands::permissions::permissions_request,
        commands::permissions::permissions_open_settings,
        // Screen capture / OCR / accessibility
        commands::capture::capture_list_displays,
        commands::capture::capture_list_windows,
        commands::capture::capture_screen,
        commands::capture::capture_read_frame,
        commands::capture::capture_discard_frame,
        commands::capture::capture_observe_start,
        commands::capture::capture_observe_stop,
        commands::capture::capture_get_protection,
        commands::capture::capture_set_protection,
        commands::capture::ocr_recognize,
        commands::capture::accessibility_snapshot,
        commands::capture::accessibility_frontmost_app,
        // Audio / transcript
        commands::audio::audio_list_devices,
        commands::audio::audio_start,
        commands::audio::audio_stop,
        commands::audio::audio_pause,
        commands::audio::audio_resume,
        commands::audio::audio_get_status,
        commands::audio::audio_test_microphone,
        commands::audio::transcript_list,
        commands::audio::transcript_recent,
        commands::audio::transcript_clear,
        // Context
        commands::context::context_build_snapshot,
        // AI
        commands::ai::ai_stream,
        commands::ai::ai_cancel,
        commands::ai::ai_cancel_all,
        commands::ai::ai_embed,
        commands::ai::ai_test_connection,
        commands::ai::ai_list_models,
        commands::ai::ai_apply_provider_presets,
        // Research
        commands::research::research_search,
        commands::research::research_scrape,
        commands::research::research_deep_start,
        commands::research::research_deep_cancel,
        commands::research::research_available,
        // Modes
        commands::modes::modes_list,
        commands::modes::modes_get,
        commands::modes::modes_create,
        commands::modes::modes_update,
        commands::modes::modes_delete,
        commands::modes::modes_duplicate,
        commands::modes::modes_set_default,
        commands::modes::modes_set_active,
        commands::modes::modes_reset_built_in,
        // Sessions
        commands::sessions::sessions_start,
        commands::sessions::sessions_pause,
        commands::sessions::sessions_resume,
        commands::sessions::sessions_end,
        commands::sessions::sessions_get_active,
        commands::sessions::sessions_list,
        commands::sessions::sessions_get,
        commands::sessions::sessions_search,
        commands::sessions::sessions_delete,
        commands::sessions::sessions_delete_all,
        commands::sessions::sessions_rename,
        commands::sessions::sessions_add_event,
        commands::sessions::sessions_list_events,
        commands::sessions::sessions_add_note,
        commands::sessions::sessions_delete_note,
        commands::sessions::sessions_save_summary,
        commands::sessions::sessions_get_summary,
        // Responses
        commands::responses::responses_save,
        commands::responses::responses_list,
        commands::responses::responses_get,
        commands::responses::responses_feedback,
        commands::responses::responses_delete,
        // Documents
        commands::documents::documents_add,
        commands::documents::documents_list,
        commands::documents::documents_get,
        commands::documents::documents_get_text,
        commands::documents::documents_delete,
        commands::documents::documents_delete_all,
        commands::documents::documents_retrieve,
        commands::documents::documents_reindex,
        commands::documents::documents_pick_files,
        // Settings & secrets
        commands::settings::settings_get,
        commands::settings::settings_update,
        commands::settings::settings_reset,
        commands::settings::secrets_set,
        commands::settings::secrets_has,
        commands::settings::secrets_delete,
        // Shortcuts
        commands::shortcuts::shortcuts_list,
        commands::shortcuts::shortcuts_update,
        commands::shortcuts::shortcuts_reset,
        commands::shortcuts::shortcuts_check_conflict,
        // Panel / windows
        commands::panel::panel_show,
        commands::panel::panel_hide,
        commands::panel::panel_toggle,
        commands::panel::panel_move,
        commands::panel::panel_set_position,
        commands::panel::panel_resize,
        commands::panel::panel_set_expanded,
        commands::panel::panel_set_opacity,
        commands::panel::panel_set_pinned,
        commands::panel::panel_get_state,
        commands::panel::panel_start_drag,
        commands::panel::window_open,
        commands::panel::window_close,
        // Data management
        commands::data::data_usage_stats,
        commands::data::data_delete_screenshots,
        commands::data::data_clear_transcripts,
        commands::data::data_clear_ai_cache,
        commands::data::data_reset_all,
        commands::data::data_export_session,
        // Developer mode
        commands::dev::dev_simulate,
        commands::dev::dev_get_metrics,
        commands::dev::dev_restart_helper,
    ]);
    app::run(builder);
}
