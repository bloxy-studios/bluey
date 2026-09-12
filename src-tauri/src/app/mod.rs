//! Application bootstrap: plugin registration, the manager graph behind
//! [`AppCore`], window/panel/tray setup, shutdown, log-level switching, plus
//! the developer-mode simulations and setup checks behind the `app_*`/`dev_*`
//! commands.
//!
//! Bootstrap order: logging → `.env` → storage → secrets → settings → bus →
//! state hub → helper client → managers → `app.manage(AppCore)` → panel/windows
//! → tray → shortcuts (async) → initial side effects → `boot_completed`.

pub mod bench;
pub mod checks;
pub mod dev;
pub mod env_import;

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use bluey_core::events::BlueyEvent;
use bluey_core::types::{AppEvent, DisplayMode, ObservationMode};
use bluey_core::{BlueyError, BlueyResult};
use tauri::{AppHandle, Manager, RunEvent, WindowEvent, Wry};

use crate::accessibility::AxManager;
use crate::agent::AgentManager;
use crate::ai::AiManager;
use crate::audio::AudioManager;
use crate::auth::AuthManager;
use crate::capture::CaptureManager;
use crate::documents::DocumentsManager;
use crate::events::{self, EventBus};
use crate::logging::{self, Logging};
use crate::modes::ModeManager;
use crate::overlay::PanelManager;
use crate::permissions::PermissionManager;
use crate::research::ResearchManager;
use crate::secrets::SecretsStore;
use crate::sessions::SessionManager;
use crate::settings::SettingsManager;
use crate::shortcuts::ShortcutManager;
use crate::sidecar::HelperClient;
use crate::state::{AppCore, DevState, MetricsRecorder, StateHub};
use crate::storage::{AppPaths, Storage};
use crate::updates::UpdatesManager;

/// The initialised logging stack (level switching from Settings → Advanced).
static LOGGING: OnceLock<Logging> = OnceLock::new();

/// Change the log level at runtime (settings side effect).
pub fn set_log_level(level: &str) {
    if let Some(logging) = LOGGING.get() {
        logging.set_level(&logging::effective_level(level));
    }
}

/// Register plugins, run bootstrap in `setup`, keep the process alive as a
/// menu-bar app and clean up sidecars on exit. The command handler is attached
/// by the caller (`lib.rs`) so the parity test can read it there.
pub fn run(builder: tauri::Builder<Wry>) {
    let builder = builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ));
    #[cfg(target_os = "macos")]
    let builder = builder.plugin(tauri_nspanel::init());

    let app = builder
        .setup(|app| {
            bootstrap(app).map_err(|e| {
                tracing::error!(error = %e, "bootstrap failed");
                Box::new(e) as Box<dyn std::error::Error>
            })
        })
        .on_window_event(|window, event| {
            // Secondary windows hide instead of closing; the HUD is panel-managed.
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() != crate::overlay::HUD_LABEL {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building the Bluey application");

    app.run(|app, event| match event {
        RunEvent::ExitRequested { api, code, .. } => {
            // Closing the last window must not quit a menu-bar app.
            if code.is_none() {
                api.prevent_exit();
            }
        }
        RunEvent::Exit => {
            tauri::async_runtime::block_on(shutdown(app));
        }
        _ => {}
    });
}

/// Build every manager and hand them to Tauri as managed state.
fn bootstrap(app: &mut tauri::App) -> BlueyResult<()> {
    crate::clock::init();
    // `.env.local` / `.env` come first: `BLUEY_LOG_LEVEL` may live there.
    let env_files = crate::secrets::load_dotenv();
    let paths = Arc::new(AppPaths::resolve()?);
    let _ = LOGGING.set(Logging::init(
        paths.logs_dir.clone(),
        &logging::effective_level("info"),
    ));
    let bus = Arc::new(EventBus::new());
    Logging::connect_bus(bus.clone());
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "bluey starting");
    for path in &env_files {
        tracing::info!(path = %path.display(), "loaded env file");
    }

    let storage = Arc::new(Storage::open(paths.clone())?);
    let secrets = Arc::new(SecretsStore::new());
    let settings = Arc::new(SettingsManager::load(
        storage.clone(),
        secrets.clone(),
        bus.clone(),
    )?);
    set_log_level(settings.get().advanced.log_level.as_str());
    // `.env` → Keychain/settings (ADR 0007). Never fatal.
    if let Err(error) = env_import::import_env(&secrets, &settings, &storage) {
        tracing::warn!(error = %error, "env import failed");
    }

    let hub = Arc::new(StateHub::new(
        settings.get().general.default_mode_id.clone(),
        bus.clone(),
    ));
    let metrics = Arc::new(MetricsRecorder::new(bus.clone()));
    let dev = Arc::new(DevState::default());
    let handle: AppHandle = app.handle().clone();

    let helper = Arc::new(HelperClient::new(
        handle.clone(),
        bus.clone(),
        settings.get().advanced.helper_restart_on_crash,
    ));
    let permissions = Arc::new(PermissionManager::new(
        handle.clone(),
        helper.clone(),
        bus.clone(),
        hub.clone(),
    ));
    let ax = Arc::new(AxManager::new(helper.clone(), bus.clone()));
    let modes = Arc::new(ModeManager::load(
        storage.clone(),
        settings.clone(),
        bus.clone(),
        hub.clone(),
    )?);
    hub.transition_soft(AppEvent::ModeChanged {
        mode_id: modes.active_id(),
    });
    let sessions = Arc::new(SessionManager::load(
        storage.clone(),
        settings.clone(),
        bus.clone(),
        hub.clone(),
        modes.clone(),
    )?);
    if let Some(session) = sessions.active() {
        hub.transition_soft(AppEvent::SessionChanged {
            session_id: Some(session.id),
        });
    }
    let capture = Arc::new(CaptureManager::new(
        handle.clone(),
        helper.clone(),
        storage.clone(),
        settings.clone(),
        bus.clone(),
        sessions.clone(),
        ax.clone(),
    ));
    let audio = Arc::new(AudioManager::new(
        helper.clone(),
        bus.clone(),
        hub.clone(),
        settings.clone(),
        storage.clone(),
        sessions.clone(),
        modes.clone(),
        secrets.clone(),
    ));
    audio.start_listener();

    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .user_agent(concat!("Bluey/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| BlueyError::internal(format!("cannot build the HTTP client: {e}")))?;
    let ai = Arc::new(AiManager::new(
        http.clone(),
        secrets.clone(),
        settings.clone(),
        storage.clone(),
        bus.clone(),
        hub.clone(),
        metrics.clone(),
        dev.clone(),
        modes.clone(),
    ));
    let documents = Arc::new(DocumentsManager::new(
        handle.clone(),
        storage.clone(),
        settings.clone(),
        ai.clone(),
        bus.clone(),
    ));
    let agent = Arc::new(AgentManager::new(
        handle.clone(),
        bus.clone(),
        secrets.clone(),
        settings.clone(),
        storage.clone(),
    ));
    let research = Arc::new(ResearchManager::new(
        http.clone(),
        secrets.clone(),
        agent.clone(),
    ));
    let accounts = Arc::new(crate::accounts::AccountsManager::load(
        secrets.clone(),
        storage.clone(),
        bus.clone(),
        settings.clone(),
        http.clone(),
    )?);
    // Connected accounts are providers to the router and lend their tokens per request.
    ai.attach_accounts(accounts.clone());
    let auth = Arc::new(AuthManager::load(
        secrets.clone(),
        storage.clone(),
        hub.clone(),
        bus.clone(),
        http,
    )?);
    let panel = Arc::new(PanelManager::load(
        handle.clone(),
        storage.clone(),
        settings.clone(),
        bus.clone(),
    )?);
    let shortcuts = Arc::new(ShortcutManager::new(
        handle.clone(),
        bus.clone(),
        storage.clone(),
        settings.clone(),
        panel.clone(),
        audio.clone(),
    ));
    let updates = Arc::new(UpdatesManager::new(
        handle.clone(),
        bus.clone(),
        settings.clone(),
    ));

    let authenticated = !auth.auth_required() || auth.has_stored_session();
    let onboarding_completed = settings.get().general.onboarding_completed;

    app.manage(AppCore {
        paths,
        bus: bus.clone(),
        hub: hub.clone(),
        storage,
        settings,
        secrets,
        metrics,
        dev,
        helper,
        permissions: permissions.clone(),
        ax,
        modes,
        sessions,
        capture,
        audio,
        ai,
        agent,
        research,
        documents,
        auth,
        accounts,
        panel: panel.clone(),
        shortcuts,
        updates: updates.clone(),
    });
    events::spawn_forwarder(handle.clone(), bus.clone());
    // In-app updates: first check 30 s after launch, then every 6 h (release builds only).
    updates.start_background();

    // `bluey://auth/callback` — the browser hands the sign-in back to us
    // (ADR 0008). Also picks up a link the app was *launched* with.
    {
        use tauri_plugin_deep_link::DeepLinkExt;
        let link_handle = handle.clone();
        app.deep_link().on_open_url(move |event| {
            let urls: Vec<String> = event.urls().into_iter().map(|u| u.to_string()).collect();
            let handle = link_handle.clone();
            tauri::async_runtime::spawn(async move { handle_deep_links(handle, urls).await });
        });
        if let Ok(Some(urls)) = app.deep_link().get_current() {
            let urls: Vec<String> = urls.into_iter().map(|u| u.to_string()).collect();
            let handle = handle.clone();
            tauri::async_runtime::spawn(async move { handle_deep_links(handle, urls).await });
        }
    }

    // Windows: HUD becomes an NSPanel; first run shows the onboarding wizard.
    panel.attach(onboarding_completed)?;
    if !onboarding_completed {
        crate::platform::open_window(&handle, "onboarding", None)?;
    }
    crate::platform::build_tray(&handle)?;
    permissions.spawn_refresh_loop();

    let boot_handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        finish_boot(&boot_handle).await;
        // `bun run bench:fastpath`: the app was launched to measure and leave.
        bench::run_from_env(&boot_handle).await;
    });

    hub.transition(AppEvent::BootCompleted { authenticated })?;
    Ok(())
}

/// Deep links: only the sign-in callback is handled; everything else is
/// logged and ignored (the URL itself is never logged — it carries the code).
async fn handle_deep_links(app: AppHandle, urls: Vec<String>) {
    let core = app.state::<AppCore>();
    for url in urls {
        if let Err(error) = core.auth.handle_callback_url(&app, &url).await {
            tracing::debug!(code = %error.code, "deep link not handled");
        }
    }
}

/// Async part of bootstrap: shortcuts, helper spawn, permission snapshot and
/// the side effects of the persisted settings.
async fn finish_boot(app: &AppHandle) {
    let core = app.state::<AppCore>();
    let settings = core.settings.get();
    // Validate / refresh the stored sign-in in the background (network).
    let auth_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        auth_handle.state::<AppCore>().auth.restore().await;
    });
    let accounts_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        accounts_handle.state::<AppCore>().accounts.restore().await;
    });
    if let Err(e) = core
        .shortcuts
        .apply_bindings(settings.shortcuts.clone())
        .await
    {
        tracing::warn!(error = %e, "shortcut registration failed at boot");
    }
    if let Err(e) = core.helper.ensure_running().await {
        tracing::warn!(error = %e, "native helper did not start");
        core.bus.publish(BlueyEvent::HelperStatus {
            running: false,
            version: None,
            restarted: None,
            error: Some(e),
        });
    }
    if let Err(e) = core.permissions.refresh().await {
        tracing::debug!(error = %e, "initial permission refresh failed");
    }
    crate::platform::set_autostart(app, settings.general.launch_at_login);
    if settings.privacy.display_mode == DisplayMode::Privacy {
        if let Err(e) = core.capture.set_protection(true) {
            tracing::warn!(error = %e, "cannot enable content protection");
        }
    }
    if settings.screen.observation == ObservationMode::Smart {
        if let Err(e) = core
            .capture
            .observe_start(Some(settings.screen.observation_interval_ms), None)
            .await
        {
            tracing::warn!(error = %e, "cannot start screen observation");
        }
    }
    // Documents embedded with another model / size (or never embedded) catch up.
    let documents = core.documents.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = documents.reembed_stale().await {
            tracing::debug!(error = %e, "stale embedding sweep failed");
        }
    });
}

/// Stop sidecars and audio before the process exits (idempotent).
pub async fn shutdown(app: &AppHandle) {
    let Some(core) = app.try_state::<AppCore>() else {
        return;
    };
    core.agent.shutdown();
    if core.audio.is_running() {
        let _ = core.audio.stop().await;
    }
    core.helper.shutdown().await;
}
