//! HUD overlay: the `main` window converted into a non-activating, floating
//! `NSPanel` (tauri-nspanel) that joins every Space and shows over fullscreen
//! apps. Owns the persisted [`PanelState`] (position per display, size,
//! opacity, pinned/expanded flags) and publishes `panel.state` on every change.
//!
//! All AppKit calls run on the main thread (`run_on_main_thread`); Tauri's
//! window API (`set_position`/`set_size`/…) is thread-safe and used directly.

use std::sync::Arc;

use bluey_core::events::BlueyEvent;
use bluey_core::types::{PanelPositionPreference, PanelState};
use bluey_core::{BlueyError, BlueyResult};
use bluey_protocols::panel::{self as geometry, MoveDirection, PanelPositions, Rect};
use bluey_storage::SettingsRepository;
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, WebviewWindow};

use crate::events::EventBus;
use crate::settings::SettingsManager;
use crate::storage::Storage;

/// Window label of the HUD.
pub const HUD_LABEL: &str = "main";
/// Collapsed HUD height (logical px) — matches `tauri.conf.json`.
pub const COLLAPSED_HEIGHT: f64 = 108.0;
/// Default expanded height when the frontend does not pass one.
pub const DEFAULT_EXPANDED_HEIGHT: f64 = 480.0;
/// Default step for keyboard moves (⌘ arrows).
pub const DEFAULT_STEP_PX: f64 = 24.0;
/// Settings-table key for the per-display position memory.
const POSITIONS_KEY: &str = "panel_positions";

#[cfg(target_os = "macos")]
tauri_nspanel::tauri_panel! {
    BlueyHudPanel {
        config: {
            can_become_key_window: true,
            can_become_main_window: false,
            is_floating_panel: true,
            hides_on_deactivate: false
        }
    }
}

#[cfg(target_os = "macos")]
type PanelRef = tauri_nspanel::PanelHandle<tauri::Wry>;

pub struct PanelManager {
    app: AppHandle,
    storage: Arc<Storage>,
    settings: Arc<SettingsManager>,
    bus: Arc<EventBus>,
    state: parking_lot::Mutex<PanelState>,
    positions: parking_lot::Mutex<PanelPositions>,
    #[cfg(target_os = "macos")]
    panel: parking_lot::Mutex<Option<PanelRef>>,
}

impl PanelManager {
    /// Load the persisted panel state (bootstrap, synchronous).
    pub fn load(
        app: AppHandle,
        storage: Arc<Storage>,
        settings: Arc<SettingsManager>,
        bus: Arc<EventBus>,
    ) -> BlueyResult<Self> {
        let mut state = storage.run_sync(SettingsRepository::get_panel_state)?;
        let positions = storage
            .run_sync(|db| SettingsRepository::get_json(db, POSITIONS_KEY))?
            .and_then(|v| serde_json::from_value::<PanelPositions>(v).ok())
            .unwrap_or_default();
        let appearance = settings.get().appearance;
        state.opacity = appearance.opacity;
        state.width = f64::from(appearance.width);
        Ok(Self {
            app,
            storage,
            settings,
            bus,
            state: parking_lot::Mutex::new(state),
            positions: parking_lot::Mutex::new(positions),
            #[cfg(target_os = "macos")]
            panel: parking_lot::Mutex::new(None),
        })
    }

    /// The Tauri app handle (used by settings side effects).
    pub fn app_handle(&self) -> AppHandle {
        self.app.clone()
    }

    fn window(&self) -> BlueyResult<WebviewWindow> {
        self.app
            .get_webview_window(HUD_LABEL)
            .ok_or_else(|| BlueyError::internal("the HUD window does not exist"))
    }

    /// Current state snapshot.
    pub fn state(&self) -> PanelState {
        self.state.lock().clone()
    }

    /// Convert the HUD window into an NSPanel and apply the persisted geometry.
    /// Must run on the main thread (Tauri `setup`). `show` controls whether the
    /// panel is ordered front immediately (false while onboarding runs).
    pub fn attach(&self, show: bool) -> BlueyResult<()> {
        let window = self.window()?;
        #[cfg(target_os = "macos")]
        {
            use tauri_nspanel::{CollectionBehavior, PanelLevel, StyleMask, WebviewWindowExt};
            let panel = window
                .to_panel::<BlueyHudPanel>()
                .map_err(|e| BlueyError::internal(format!("cannot create the HUD panel: {e}")))?;
            panel.set_level(PanelLevel::Floating.value());
            panel.set_style_mask(
                StyleMask::empty()
                    .borderless()
                    .nonactivating_panel()
                    .value(),
            );
            panel.set_collection_behavior(
                CollectionBehavior::new()
                    .can_join_all_spaces()
                    .full_screen_auxiliary()
                    .ignores_cycle()
                    .value(),
            );
            panel.set_becomes_key_only_if_needed(true);
            panel.set_has_shadow(false);
            panel.set_alpha_value(f64::from(self.state().opacity));
            *self.panel.lock() = Some(panel);
        }

        let initial = self.initial_rect(&window)?;
        window
            .set_size(LogicalSize::new(initial.width, initial.height))
            .map_err(window_err)?;
        window
            .set_position(LogicalPosition::new(initial.x, initial.y))
            .map_err(window_err)?;
        {
            let mut state = self.state.lock();
            state.x = initial.x;
            state.y = initial.y;
            state.width = initial.width;
            state.height = initial.height;
            state.visible = show && state.visible;
        }
        let visible = self.state().visible;
        self.set_native_visible(visible);
        self.publish();
        Ok(())
    }

    /// Where the panel should appear at launch: the remembered position for the
    /// current display when the preference is `Remember`, otherwise the
    /// preference-derived spot on the display under the cursor/last position.
    fn initial_rect(&self, window: &WebviewWindow) -> BlueyResult<Rect> {
        let state = self.state();
        let width = f64::from(self.settings.get().appearance.width);
        let height = if state.expanded {
            state.height.max(COLLAPSED_HEIGHT)
        } else {
            COLLAPSED_HEIGHT
        };
        let areas = work_areas(window);
        let Some((display_id, work)) = geometry::work_area_at(&areas, state.x, state.y).cloned()
        else {
            return Ok(Rect::new(state.x, state.y, width, height));
        };
        let preference = self.settings.get().appearance.position;
        let (x, y) = match preference {
            PanelPositionPreference::Remember => self
                .positions
                .lock()
                .recall(&display_id)
                .filter(|_| state.x != 0.0 || state.y != 0.0)
                .unwrap_or_else(|| geometry::default_position(width, work)),
            other => preferred_position(other, width, height, work),
        };
        Ok(geometry::clamp_into(Rect::new(x, y, width, height), work))
    }

    // ── Visibility ─────────────────────────────────────────────────────────

    pub async fn show(&self) -> BlueyResult<PanelState> {
        self.state.lock().visible = true;
        self.set_native_visible(true);
        Ok(self.commit().await)
    }

    pub async fn hide(&self) -> BlueyResult<PanelState> {
        self.state.lock().visible = false;
        self.set_native_visible(false);
        Ok(self.commit().await)
    }

    pub async fn toggle(&self) -> BlueyResult<PanelState> {
        let visible = self.state().visible;
        if visible {
            self.hide().await
        } else {
            self.show().await
        }
    }

    fn set_native_visible(&self, visible: bool) {
        #[cfg(target_os = "macos")]
        {
            if let Some(panel) = self.panel.lock().clone() {
                let _ = self.app.run_on_main_thread(move || {
                    if visible {
                        panel.show();
                    } else {
                        panel.hide();
                    }
                });
                return;
            }
        }
        if let Ok(window) = self.window() {
            let result = if visible {
                window.show()
            } else {
                window.hide()
            };
            if let Err(e) = result {
                tracing::warn!(error = %e, "cannot change HUD visibility");
            }
        }
    }

    // ── Geometry ───────────────────────────────────────────────────────────

    /// Move one keyboard step in `direction`, clamped to the current display.
    pub async fn move_step(
        &self,
        direction: MoveDirection,
        step_px: Option<f64>,
    ) -> BlueyResult<PanelState> {
        let window = self.window()?;
        let current = self.current_rect(&window);
        let areas = work_areas(&window);
        let work = area_for(&areas, &current);
        let moved =
            geometry::step_move(current, direction, step_px.unwrap_or(DEFAULT_STEP_PX), work);
        self.apply_rect(&window, moved, true).await
    }

    /// Absolute position (logical px), clamped to the display it lands on.
    pub async fn set_position(&self, x: f64, y: f64) -> BlueyResult<PanelState> {
        let window = self.window()?;
        let current = self.current_rect(&window);
        let target = Rect::new(x, y, current.width, current.height);
        let areas = work_areas(&window);
        let work = area_for(&areas, &target);
        self.apply_rect(&window, geometry::clamp_into(target, work), true)
            .await
    }

    /// Resize (logical px) keeping the origin, clamped into the display.
    pub async fn resize(&self, width: f64, height: f64) -> BlueyResult<PanelState> {
        let window = self.window()?;
        let current = self.current_rect(&window);
        let target = Rect::new(current.x, current.y, width.max(1.0), height.max(1.0));
        let areas = work_areas(&window);
        let work = area_for(&areas, &target);
        self.apply_rect(&window, geometry::clamp_into(target, work), false)
            .await
    }

    /// Expand/collapse the response area (the frontend measures its content
    /// and passes the wanted height).
    pub async fn set_expanded(
        &self,
        expanded: bool,
        height: Option<f64>,
    ) -> BlueyResult<PanelState> {
        let target_height = if expanded {
            height
                .unwrap_or(DEFAULT_EXPANDED_HEIGHT)
                .max(COLLAPSED_HEIGHT)
        } else {
            COLLAPSED_HEIGHT
        };
        self.state.lock().expanded = expanded;
        let current = self.state();
        self.resize(current.width, target_height).await
    }

    /// Apply the appearance width setting (keeps the panel centred on its old centre).
    pub async fn apply_width(&self, width: f64) -> BlueyResult<PanelState> {
        let window = self.window()?;
        let current = self.current_rect(&window);
        let dx = (current.width - width) / 2.0;
        let target = Rect::new(current.x + dx, current.y, width, current.height);
        let areas = work_areas(&window);
        let work = area_for(&areas, &target);
        self.apply_rect(&window, geometry::clamp_into(target, work), true)
            .await
    }

    async fn apply_rect(
        &self,
        window: &WebviewWindow,
        rect: Rect,
        remember: bool,
    ) -> BlueyResult<PanelState> {
        window
            .set_size(LogicalSize::new(rect.width, rect.height))
            .map_err(window_err)?;
        window
            .set_position(LogicalPosition::new(rect.x, rect.y))
            .map_err(window_err)?;
        let display_id =
            geometry::work_area_at(&work_areas(window), rect.x, rect.y).map(|(id, _)| id.clone());
        {
            let mut state = self.state.lock();
            state.x = rect.x;
            state.y = rect.y;
            state.width = rect.width;
            state.height = rect.height;
            state.display_id = display_id.clone();
        }
        if remember {
            if let Some(id) = display_id {
                self.positions.lock().remember(&id, rect.x, rect.y);
                self.persist_positions().await;
            }
        }
        Ok(self.commit().await)
    }

    /// Read the live window rect (logical px), falling back to the stored state.
    fn current_rect(&self, window: &WebviewWindow) -> Rect {
        let state = self.state();
        let scale = window.scale_factor().unwrap_or(1.0);
        let position = window
            .outer_position()
            .map(|p| p.to_logical::<f64>(scale))
            .map(|p| (p.x, p.y))
            .unwrap_or((state.x, state.y));
        let size = window
            .outer_size()
            .map(|s| s.to_logical::<f64>(scale))
            .map(|s| (s.width, s.height))
            .unwrap_or((state.width, state.height));
        Rect::new(position.0, position.1, size.0, size.1)
    }

    /// Called after a user drag ends: read the window position back into state.
    pub async fn sync_from_window(&self) -> BlueyResult<PanelState> {
        let window = self.window()?;
        let rect = self.current_rect(&window);
        self.apply_rect(&window, rect, true).await
    }

    // ── Appearance ─────────────────────────────────────────────────────────

    pub async fn set_opacity(&self, opacity: f32) -> BlueyResult<PanelState> {
        let opacity = opacity.clamp(0.2, 1.0);
        self.state.lock().opacity = opacity;
        #[cfg(target_os = "macos")]
        {
            if let Some(panel) = self.panel.lock().clone() {
                let _ = self.app.run_on_main_thread(move || {
                    panel.set_alpha_value(f64::from(opacity));
                });
            }
        }
        Ok(self.commit().await)
    }

    /// Pinned panels float above everything (status level) and never hide with
    /// the app; unpinned panels use the normal floating level.
    pub async fn set_pinned(&self, pinned: bool) -> BlueyResult<PanelState> {
        self.state.lock().pinned = pinned;
        self.apply_level();
        Ok(self.commit().await)
    }

    /// Appearance → always-on-top toggle.
    pub fn set_always_on_top(&self, _always_on_top: bool) {
        self.apply_level();
    }

    fn apply_level(&self) {
        let pinned = self.state().pinned;
        let always_on_top = self.settings.get().appearance.always_on_top;
        #[cfg(target_os = "macos")]
        {
            use tauri_nspanel::PanelLevel;
            if let Some(panel) = self.panel.lock().clone() {
                let level = if pinned {
                    PanelLevel::Status
                } else if always_on_top {
                    PanelLevel::Floating
                } else {
                    PanelLevel::Normal
                };
                let _ = self.app.run_on_main_thread(move || {
                    panel.set_level(level.value());
                });
                return;
            }
        }
        if let Ok(window) = self.window() {
            let _ = window.set_always_on_top(pinned || always_on_top);
        }
    }

    /// Begin a native window drag (mouse-down on the HUD chrome).
    pub fn start_drag(&self) -> BlueyResult<()> {
        self.window()?.start_dragging().map_err(window_err)
    }

    // ── Persistence & events ───────────────────────────────────────────────

    async fn commit(&self) -> PanelState {
        let state = self.state();
        let persist = state.clone();
        let result = self
            .storage
            .run(move |db| SettingsRepository::save_panel_state(db, &persist))
            .await;
        if let Err(e) = result {
            tracing::warn!(error = %e, "failed to persist panel state");
        }
        self.publish();
        state
    }

    async fn persist_positions(&self) {
        let value = serde_json::to_value(&*self.positions.lock()).unwrap_or_default();
        let result = self
            .storage
            .run(move |db| SettingsRepository::set_json(db, POSITIONS_KEY, &value))
            .await;
        if let Err(e) = result {
            tracing::warn!(error = %e, "failed to persist panel positions");
        }
    }

    fn publish(&self) {
        self.bus.publish(BlueyEvent::PanelState(self.state()));
    }
}

fn window_err(e: tauri::Error) -> BlueyError {
    BlueyError::internal(format!("HUD window operation failed: {e}"))
}

/// Work areas of every display in logical pixels, keyed by a stable id (the
/// monitor name, else its origin).
fn work_areas(window: &WebviewWindow) -> Vec<(String, Rect)> {
    let monitors = window.available_monitors().unwrap_or_default();
    monitors
        .iter()
        .map(|monitor| {
            let scale = monitor.scale_factor();
            let area = monitor.work_area();
            let position = area.position.to_logical::<f64>(scale);
            let size = area.size.to_logical::<f64>(scale);
            let id = monitor
                .name()
                .cloned()
                .unwrap_or_else(|| format!("{}x{}", position.x, position.y));
            (
                id,
                Rect::new(position.x, position.y, size.width, size.height),
            )
        })
        .collect()
}

/// The work area a rect belongs to (by its centre), with a generous fallback
/// when no display is known (headless tests / detached displays).
fn area_for(areas: &[(String, Rect)], rect: &Rect) -> Rect {
    let (cx, cy) = rect.center();
    geometry::work_area_at(areas, cx, cy)
        .map(|(_, area)| *area)
        .unwrap_or_else(|| Rect::new(rect.x, rect.y, rect.width, rect.height))
}

/// Position for the non-`Remember` appearance preferences.
fn preferred_position(
    preference: PanelPositionPreference,
    width: f64,
    height: f64,
    work: Rect,
) -> (f64, f64) {
    let centre_x = work.x + ((work.width - width) / 2.0).max(0.0);
    let centre_y = work.y + ((work.height - height) / 2.0).max(0.0);
    let margin = 24.0;
    match preference {
        PanelPositionPreference::Remember | PanelPositionPreference::Top => {
            geometry::default_position(width, work)
        }
        PanelPositionPreference::Center => (centre_x, centre_y),
        PanelPositionPreference::Bottom => {
            (centre_x, work.y + (work.height - height - margin).max(0.0))
        }
        PanelPositionPreference::Left => (work.x + margin, centre_y),
        PanelPositionPreference::Right => {
            (work.x + (work.width - width - margin).max(0.0), centre_y)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: Rect = Rect {
        x: 0.0,
        y: 25.0,
        width: 1512.0,
        height: 957.0,
    };

    #[test]
    fn preferred_positions_stay_inside_the_work_area() {
        for preference in [
            PanelPositionPreference::Remember,
            PanelPositionPreference::Center,
            PanelPositionPreference::Top,
            PanelPositionPreference::Bottom,
            PanelPositionPreference::Left,
            PanelPositionPreference::Right,
        ] {
            let (x, y) = preferred_position(preference, 690.0, 108.0, WORK);
            let clamped = geometry::clamp_into(Rect::new(x, y, 690.0, 108.0), WORK);
            assert_eq!((clamped.x, clamped.y), (x, y), "{preference:?} was clamped");
        }
        let (x, y) = preferred_position(PanelPositionPreference::Bottom, 690.0, 108.0, WORK);
        assert_eq!(x, (1512.0 - 690.0) / 2.0);
        assert_eq!(y, 25.0 + 957.0 - 108.0 - 24.0);
    }

    #[test]
    fn area_for_falls_back_to_the_rect_itself() {
        let rect = Rect::new(10.0, 20.0, 690.0, 108.0);
        assert_eq!(area_for(&[], &rect), rect);
        let areas = vec![("main".to_string(), WORK)];
        assert_eq!(area_for(&areas, &rect), WORK);
    }
}
