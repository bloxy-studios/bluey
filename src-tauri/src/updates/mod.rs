//! In-app updates on top of `tauri-plugin-updater` (docs/UPDATES.md).
//!
//! One manager owns the cycle — check the feed of the channel chosen in
//! Settings → General → Updates → *available* → download + install → *ready*
//! → relaunch — and publishes every transition as `update.status`, which the
//! WebView's `updatesStore` mirrors for the HUD pill and the Settings section.
//! With automatic updates on (the default) the manager downloads and installs
//! as soon as a check finds something; the user only decides when to relaunch.
//!
//! Nothing here trusts the feed: the plugin verifies each bundle's minisign
//! signature against the public key compiled into `tauri.conf.json` before it
//! touches the installed app. Debug builds (`tauri dev`) report
//! `supported: false` — a manual check still answers, nothing is installed and
//! no background check runs.

use std::sync::Arc;
use std::time::{Duration, Instant};

use bluey_core::error::RecoveryAction;
use bluey_core::events::BlueyEvent;
use bluey_core::types::{
    AvailableUpdate, UpdatePhase, UpdateProgress, UpdateStatus, UpdatesSettings,
};
use bluey_core::{now_iso, BlueyError, BlueyErrorKind, BlueyResult};
use parking_lot::{Mutex, RwLock};
use tauri::AppHandle;
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::events::EventBus;
use crate::settings::SettingsManager;

/// First background check after launch — lets the HUD settle and the network come up.
pub const FIRST_CHECK_DELAY: Duration = Duration::from_secs(30);
/// Interval between background checks.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
/// Progress is published at most this often while downloading.
const PROGRESS_PUBLISH_EVERY: Duration = Duration::from_millis(400);

/// What started a check. Kept for the log line; every trigger runs the same cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckTrigger {
    Launch,
    Scheduled,
    Manual,
    ChannelChanged,
}

impl CheckTrigger {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Launch => "launch",
            Self::Scheduled => "scheduled",
            Self::Manual => "manual",
            Self::ChannelChanged => "channel_changed",
        }
    }
}

pub struct UpdatesManager {
    app: AppHandle,
    bus: Arc<EventBus>,
    settings: Arc<SettingsManager>,
    status: RwLock<UpdateStatus>,
    /// The update the last check found (kept through `Ready` so a relaunch can be requested later).
    pending: Mutex<Option<Update>>,
    /// A check or install is in flight; concurrent requests get the current status back.
    busy: Mutex<bool>,
    last_progress_publish: Mutex<Instant>,
}

fn update_error(code: &str, message: impl Into<String>) -> BlueyError {
    BlueyError::new(BlueyErrorKind::Network, format!("update.{code}"), message)
        .recoverable(RecoveryAction::Retry)
}

impl UpdatesManager {
    pub fn new(app: AppHandle, bus: Arc<EventBus>, settings: Arc<SettingsManager>) -> Self {
        let prefs = settings.get().updates;
        let status = UpdateStatus::idle(
            app.package_info().version.to_string(),
            prefs.channel,
            prefs.automatic,
            Self::supported(),
        );
        Self {
            app,
            bus,
            settings,
            status: RwLock::new(status),
            pending: Mutex::new(None),
            busy: Mutex::new(false),
            last_progress_publish: Mutex::new(Instant::now()),
        }
    }

    /// Release builds replace themselves; `tauri dev` builds only report.
    fn supported() -> bool {
        !cfg!(debug_assertions)
    }

    pub fn status(&self) -> UpdateStatus {
        self.status.read().clone()
    }

    fn set(&self, change: impl FnOnce(&mut UpdateStatus)) -> UpdateStatus {
        let mut status = self.status.write();
        change(&mut status);
        status.clone()
    }

    fn publish(&self, status: &UpdateStatus) {
        self.bus.publish(BlueyEvent::UpdateStatus(status.clone()));
    }

    /// Claim the cycle; `false` when a check or install is already running.
    fn begin(&self) -> bool {
        let mut busy = self.busy.lock();
        if *busy {
            return false;
        }
        *busy = true;
        true
    }

    fn end(&self) {
        *self.busy.lock() = false;
    }

    /// Check the current channel's feed. Finds → `Available` (and, with
    /// automatic updates on, straight on to download + install); nothing →
    /// `UpToDate`; failure → `Error` with `update.check_failed`.
    pub async fn check(&self, trigger: CheckTrigger) -> BlueyResult<UpdateStatus> {
        if !self.begin() {
            return Ok(self.status());
        }
        let prefs = self.settings.get().updates;
        let status = self.set(|s| {
            s.phase = UpdatePhase::Checking;
            s.channel = prefs.channel;
            s.automatic = prefs.automatic;
            s.error = None;
            s.progress = None;
        });
        self.publish(&status);
        tracing::info!(
            trigger = trigger.as_str(),
            channel = prefs.channel.as_str(),
            "checking for updates"
        );

        let found = self.query_feed(prefs.channel.feed_url()).await;
        let checked_at = now_iso();
        let status = match found {
            Ok(Some(update)) => {
                let available = AvailableUpdate {
                    version: update.version.clone(),
                    channel: prefs.channel,
                    notes: update.body.clone().filter(|b| !b.trim().is_empty()),
                    published_at: update
                        .raw_json
                        .get("pub_date")
                        .and_then(|d| d.as_str())
                        .map(str::to_string),
                };
                tracing::info!(version = %available.version, "update available");
                *self.pending.lock() = Some(update);
                self.set(|s| {
                    s.phase = UpdatePhase::Available;
                    s.available = Some(available);
                    s.last_checked_at = Some(checked_at.clone());
                })
            }
            Ok(None) => {
                *self.pending.lock() = None;
                self.set(|s| {
                    s.phase = UpdatePhase::UpToDate;
                    s.available = None;
                    s.last_checked_at = Some(checked_at.clone());
                })
            }
            Err(error) => {
                tracing::warn!(error = %error, "update check failed");
                self.set(|s| {
                    s.phase = UpdatePhase::Error;
                    s.error = Some(update_error("check_failed", error.to_string()));
                    s.last_checked_at = Some(checked_at.clone());
                })
            }
        };
        self.publish(&status);

        let install_now =
            status.phase == UpdatePhase::Available && prefs.automatic && status.supported;
        let status = if install_now {
            self.install_pending().await
        } else {
            status
        };
        self.end();
        Ok(status)
    }

    async fn query_feed(&self, url: &str) -> tauri_plugin_updater::Result<Option<Update>> {
        let endpoint = tauri::Url::parse(url).map_err(|e| {
            tauri_plugin_updater::Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("invalid update feed url: {e}"),
            ))
        })?;
        let updater = self
            .app
            .updater_builder()
            .endpoints(vec![endpoint])?
            .build()?;
        updater.check().await
    }

    /// Download, verify and install the update the last check found (the
    /// user pressed *Install*; automatic mode calls this itself).
    pub async fn install(&self) -> BlueyResult<UpdateStatus> {
        if !self.status().supported {
            return Err(BlueyError::new(
                BlueyErrorKind::NotSupported,
                "update.unsupported",
                "this build cannot update itself — install a release build from the download page",
            ));
        }
        if self.pending.lock().is_none() {
            return Err(BlueyError::new(
                BlueyErrorKind::Internal,
                "update.nothing_pending",
                "no update has been found yet — check for updates first",
            ));
        }
        if !self.begin() {
            return Ok(self.status());
        }
        let status = self.install_pending().await;
        self.end();
        Ok(status)
    }

    /// The download + install half of the cycle; the caller holds `busy`.
    async fn install_pending(&self) -> UpdateStatus {
        let Some(update) = self.pending.lock().clone() else {
            return self.status();
        };
        if self.status().phase == UpdatePhase::Ready {
            return self.status();
        }
        let status = self.set(|s| {
            s.phase = UpdatePhase::Downloading;
            s.error = None;
            s.progress = Some(UpdateProgress {
                downloaded: 0,
                total: None,
            });
        });
        self.publish(&status);
        tracing::info!(version = %update.version, "downloading update");

        let mut downloaded: u64 = 0;
        let result = update
            .download_and_install(
                |chunk, total| {
                    downloaded += chunk as u64;
                    self.report_progress(downloaded, total);
                },
                || tracing::info!("update downloaded; installing"),
            )
            .await;

        let status = match result {
            Ok(()) => {
                tracing::info!(version = %update.version, "update installed; relaunch to finish");
                self.set(|s| {
                    s.phase = UpdatePhase::Ready;
                    s.progress = None;
                })
            }
            Err(error) => {
                tracing::warn!(error = %error, "update install failed");
                self.set(|s| {
                    s.phase = UpdatePhase::Error;
                    s.progress = None;
                    s.error = Some(update_error("install_failed", error.to_string()));
                })
            }
        };
        self.publish(&status);
        status
    }

    /// Progress goes out at most every `PROGRESS_PUBLISH_EVERY`, plus the last byte.
    fn report_progress(&self, downloaded: u64, total: Option<u64>) {
        let finished = total.is_some_and(|t| downloaded >= t);
        {
            let mut last = self.last_progress_publish.lock();
            if !finished && last.elapsed() < PROGRESS_PUBLISH_EVERY {
                return;
            }
            *last = Instant::now();
        }
        let status = self.set(|s| {
            s.progress = Some(UpdateProgress { downloaded, total });
        });
        self.publish(&status);
    }

    /// Restart into the installed update (only meaningful in `Ready`).
    pub fn relaunch(&self) -> BlueyResult<()> {
        if self.status().phase != UpdatePhase::Ready {
            return Err(BlueyError::new(
                BlueyErrorKind::Internal,
                "update.nothing_pending",
                "no installed update is waiting for a relaunch",
            ));
        }
        tracing::info!("relaunching into the installed update");
        self.app.restart();
    }

    /// Settings → General → Updates changed. A new channel drops what the old
    /// channel found and checks again; turning automatic updates on installs
    /// an update that is already waiting.
    pub fn on_settings_changed(self: &Arc<Self>, old: &UpdatesSettings, new: &UpdatesSettings) {
        let channel_changed = old.channel != new.channel;
        let status = self.set(|s| {
            s.channel = new.channel;
            s.automatic = new.automatic;
            if channel_changed && s.phase != UpdatePhase::Ready {
                s.phase = UpdatePhase::Idle;
                s.available = None;
                s.progress = None;
                s.error = None;
            }
        });
        if channel_changed && status.phase != UpdatePhase::Ready {
            *self.pending.lock() = None;
        }
        self.publish(&status);

        let manager = Arc::clone(self);
        if channel_changed && status.phase != UpdatePhase::Ready {
            tauri::async_runtime::spawn(async move {
                let _ = manager.check(CheckTrigger::ChannelChanged).await;
            });
        } else if new.automatic && !old.automatic && status.phase == UpdatePhase::Available {
            tauri::async_runtime::spawn(async move {
                let _ = manager.install().await;
            });
        }
    }

    /// Background checks: once shortly after launch, then every `CHECK_INTERVAL`.
    pub fn start_background(self: &Arc<Self>) {
        if !self.status().supported {
            tracing::debug!("in-app updates: debug build, background checks off");
            return;
        }
        let manager = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(FIRST_CHECK_DELAY).await;
            let _ = manager.check(CheckTrigger::Launch).await;
            loop {
                tokio::time::sleep(CHECK_INTERVAL).await;
                if manager.status().phase == UpdatePhase::Ready {
                    continue; // installed and waiting for a relaunch — nothing to re-check
                }
                let _ = manager.check(CheckTrigger::Scheduled).await;
            }
        });
    }
}
