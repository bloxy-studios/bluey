//! HUD overlay: the `main` window converted into a non-activating, floating
//! `NSPanel` (tauri-nspanel) that joins every Space and shows over fullscreen
//! apps. Owns the persisted [`PanelState`] (position per display, size,
//! opacity, pinned/expanded flags) and publishes `panel.state` on every change.
//!
//! All AppKit calls run on the main thread (`run_on_main_thread`); Tauri's
//! window API (`set_position`/`set_size`/…) is thread-safe and used directly.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bluey_core::events::BlueyEvent;
use bluey_core::types::{PanelPositionPreference, PanelState};
use bluey_core::{BlueyError, BlueyResult};
use bluey_protocols::panel::{self as geometry, MoveDirection, PanelPositions, Rect};
use bluey_storage::SettingsRepository;
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, WebviewWindow, WindowEvent};

use crate::events::EventBus;
use crate::settings::SettingsManager;
use crate::storage::Storage;

/// Window label of the HUD.
pub const HUD_LABEL: &str = "main";
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
    // Serialize panel mutations and deferred OS events, including persistence.
    // An old resize/opacity/drag snapshot must not overwrite newer geometry.
    mutation_lock: tokio::sync::Mutex<()>,
    minimum_size: parking_lot::Mutex<Option<(f64, f64, f64)>>,
    sync_revision: AtomicU64,
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
        state.width = geometry::frame_width(f64::from(appearance.width));
        Ok(Self {
            app,
            storage,
            settings,
            bus,
            state: parking_lot::Mutex::new(state),
            positions: parking_lot::Mutex::new(positions),
            mutation_lock: tokio::sync::Mutex::new(()),
            minimum_size: parking_lot::Mutex::new(None),
            sync_revision: AtomicU64::new(0),
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
    pub fn attach(self: &Arc<Self>, show: bool) -> BlueyResult<()> {
        let window = self.window()?;
        // Height belongs to measured content; width belongs to Appearance.
        // Do not allow a manual edge resize to cut the footer off beneath the
        // content-sized DOM. This matches the existing non-resizable NSPanel
        // style mask and, unlike a viewport cap, permits normal auto growth.
        window.set_resizable(false).map_err(window_err)?;
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
            // Opacity is applied once, on the CSS surface (also in browser
            // preview). A second NSPanel alpha would square the preference.
            panel.set_alpha_value(1.0);
            *self.panel.lock() = Some(panel);
        }

        let initial = self.initial_rect(&window)?;
        let areas = work_areas(&window);
        self.apply_minimum_size(&window, geometry::area_for(&areas, &initial))?;
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
            let (cx, cy) = initial.center();
            state.display_id = geometry::work_area_at(&areas, cx, cy).map(|(id, _)| id.clone());
            state.visible = show && state.visible;
        }
        let visible = self.state().visible;
        self.set_native_visible(visible);
        self.watch_geometry(&window);
        self.publish();
        Ok(())
    }

    /// Tauri emits moves/resizes for our own size+position pair as well as OS
    /// changes. Read the LIVE rect after the burst settles, never an event's
    /// stale physical payload; no-op suppression then breaks the feedback loop.
    fn watch_geometry(self: &Arc<Self>, window: &WebviewWindow) {
        let weak = Arc::downgrade(self);
        window.on_window_event(move |event| {
            if !matches!(
                event,
                WindowEvent::Moved(_)
                    | WindowEvent::Resized(_)
                    | WindowEvent::ScaleFactorChanged { .. }
                    | WindowEvent::Focused(true)
            ) {
                return;
            }
            let Some(panel) = weak.upgrade() else { return };
            let revision = panel.sync_revision.fetch_add(1, Ordering::Relaxed) + 1;
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_millis(120)).await;
                if panel.sync_revision.load(Ordering::Relaxed) == revision {
                    if let Err(error) = panel.sync_from_window().await {
                        tracing::warn!(error = %error, "cannot synchronize HUD geometry");
                    }
                }
            });
        });
    }

    /// Where the panel should appear at launch: the remembered position for the
    /// current display when the preference is `Remember`, otherwise the
    /// preference-derived spot on the display under the cursor/last position.
    fn initial_rect(&self, window: &WebviewWindow) -> BlueyResult<Rect> {
        let state = self.state();
        let width = geometry::frame_width(f64::from(self.settings.get().appearance.width));
        let height = geometry::frame_height(state.expanded, state.expanded.then_some(state.height));
        let areas = work_areas(window);
        let Some((display_id, work)) = geometry::work_area_at(&areas, state.x, state.y).cloned()
        else {
            let rect = Rect::new(state.x, state.y, width, height);
            return Ok(geometry::fit_into(rect, geometry::area_for(&areas, &rect)));
        };
        let preference = self.settings.get().appearance.position;
        let (x, y) = match preference {
            PanelPositionPreference::Remember => self
                .positions
                .lock()
                .recall(&display_id)
                .filter(|_| state.x != 0.0 || state.y != 0.0)
                .unwrap_or_else(|| geometry::default_position(width, work)),
            other => geometry::preferred_position(other, width, height, work),
        };
        Ok(geometry::fit_into(Rect::new(x, y, width, height), work))
    }

    // ── Visibility ─────────────────────────────────────────────────────────

    pub async fn show(&self) -> BlueyResult<PanelState> {
        let _mutation = self.mutation_lock.lock().await;
        self.state.lock().visible = true;
        self.set_native_visible(true);
        Ok(self.commit().await)
    }

    pub async fn hide(&self) -> BlueyResult<PanelState> {
        let _mutation = self.mutation_lock.lock().await;
        self.state.lock().visible = false;
        self.set_native_visible(false);
        Ok(self.commit().await)
    }

    pub async fn toggle(&self) -> BlueyResult<PanelState> {
        let _mutation = self.mutation_lock.lock().await;
        let visible = !self.state().visible;
        self.state.lock().visible = visible;
        self.set_native_visible(visible);
        Ok(self.commit().await)
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
        let _geometry = self.mutation_lock.lock().await;
        let window = self.window()?;
        let current = self.current_rect(&window);
        let areas = work_areas(&window);
        let work = geometry::area_for(&areas, &current);
        let moved =
            geometry::step_move(current, direction, step_px.unwrap_or(DEFAULT_STEP_PX), work);
        self.apply_rect(&window, moved, true, None).await
    }

    /// Absolute position (logical px), clamped to the display it lands on.
    pub async fn set_position(&self, x: f64, y: f64) -> BlueyResult<PanelState> {
        let _geometry = self.mutation_lock.lock().await;
        let window = self.window()?;
        let current = self.current_rect(&window);
        let target = Rect::new(x, y, current.width, current.height);
        let areas = work_areas(&window);
        let work = geometry::area_for(&areas, &target);
        self.apply_rect(&window, geometry::fit_into(target, work), true, None)
            .await
    }

    /// Resize the native frame (logical px), preserving its origin if it fits.
    pub async fn resize(&self, width: f64, height: f64) -> BlueyResult<PanelState> {
        let _geometry = self.mutation_lock.lock().await;
        let window = self.window()?;
        let current = self.current_rect(&window);
        let areas = work_areas(&window);
        let work = geometry::area_for(&areas, &current);
        let target = geometry::resize_in_work_area(current, width, height, work);
        self.apply_rect(&window, target, false, None).await
    }

    /// `height` is the measured frame border-box, including transparent insets.
    /// Honor it even without a chat: a live transcript also grows/shrinks the HUD.
    pub async fn set_expanded(
        &self,
        expanded: bool,
        height: Option<f64>,
    ) -> BlueyResult<PanelState> {
        let _geometry = self.mutation_lock.lock().await;
        let window = self.window()?;
        let current = self.current_rect(&window);
        let areas = work_areas(&window);
        let work = geometry::area_for(&areas, &current);
        let target = geometry::resize_in_work_area(
            current,
            current.width,
            geometry::frame_height(expanded, height),
            work,
        );
        self.apply_rect(&window, target, false, Some(expanded))
            .await
    }

    /// The appearance setting is a surface width, not a native-frame width.
    pub async fn apply_width(&self, width: f64) -> BlueyResult<PanelState> {
        let _geometry = self.mutation_lock.lock().await;
        let window = self.window()?;
        let current = self.current_rect(&window);
        let areas = work_areas(&window);
        let work = geometry::area_for(&areas, &current);
        let target = geometry::apply_surface_width(current, width, work);
        self.apply_rect(&window, target, true, None).await
    }

    /// Update constraints before set_size: the previous display's larger
    /// minimum must not force a narrow/short destination frame back off-screen.
    /// Include the window scale in the cache so the native backing constraint
    /// is refreshed on a DPI change even when logical work dimensions match.
    fn apply_minimum_size(&self, window: &WebviewWindow, work: Rect) -> BlueyResult<()> {
        let (width, height) = geometry::minimum_frame_size(work);
        let key = (width, height, window.scale_factor().unwrap_or(1.0));
        if *self.minimum_size.lock() != Some(key) {
            window
                .set_min_size(Some(LogicalSize::new(width, height)))
                .map_err(window_err)?;
            *self.minimum_size.lock() = Some(key);
        }
        Ok(())
    }

    async fn apply_rect(
        &self,
        window: &WebviewWindow,
        rect: Rect,
        remember: bool,
        expanded: Option<bool>,
    ) -> BlueyResult<PanelState> {
        let areas = work_areas(window);
        let work = geometry::area_for(&areas, &rect);
        self.apply_minimum_size(window, work)?;
        let mut rect = geometry::fit_into(rect, work);
        let current = self.current_rect(window);
        let scale = window.scale_factor().unwrap_or(1.0);
        let size_changed = !geometry::same_backing_pixel(current.width, rect.width, scale)
            || !geometry::same_backing_pixel(current.height, rect.height, scale);
        let position_changed = !geometry::same_backing_pixel(current.x, rect.x, scale)
            || !geometry::same_backing_pixel(current.y, rect.y, scale);
        if size_changed {
            window
                .set_size(LogicalSize::new(rect.width, rect.height))
                .map_err(window_err)?;
        }
        // Reassert the logical top-left after a size change; do not assume
        // AppKit's bottom-left frame anchor matches our coordinate system.
        if size_changed || position_changed {
            window
                .set_position(LogicalPosition::new(rect.x, rect.y))
                .map_err(window_err)?;
        } else {
            rect.x = current.x;
            rect.y = current.y;
        }
        if !size_changed {
            rect.width = current.width;
            rect.height = current.height;
        }
        let (cx, cy) = rect.center();
        let display_id = geometry::work_area_at(&areas, cx, cy).map(|(id, _)| id.clone());
        let changed = {
            let mut state = self.state.lock();
            let previous = state.clone();
            state.x = rect.x;
            state.y = rect.y;
            state.width = rect.width;
            state.height = rect.height;
            state.display_id = display_id.clone();
            if let Some(expanded) = expanded {
                state.expanded = expanded;
            }
            *state != previous
        };
        // ResizeObserver can report a duplicate (or a request already capped
        // by the work area). Neither should emit or persist the same state.
        if !changed {
            return Ok(self.state());
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

    /// Called after native geometry events settle (including a user drag).
    pub async fn sync_from_window(&self) -> BlueyResult<PanelState> {
        let _geometry = self.mutation_lock.lock().await;
        let window = self.window()?;
        let rect = self.current_rect(&window);
        let work = geometry::area_for(&work_areas(&window), &rect);
        // Fitting a narrow display must not permanently replace the user's
        // appearance width. Restore it (still work-area clamped) when room
        // returns. Height remains content-driven, never a viewport preference.
        let target = geometry::apply_surface_width(
            rect,
            f64::from(self.settings.get().appearance.width),
            work,
        );
        self.apply_rect(&window, target, true, None).await
    }

    // ── Appearance ─────────────────────────────────────────────────────────

    pub async fn set_opacity(&self, opacity: f32) -> BlueyResult<PanelState> {
        let _mutation = self.mutation_lock.lock().await;
        let opacity = opacity.clamp(0.2, 1.0);
        self.state.lock().opacity = opacity;
        // The state event updates CSS opacity; native alpha stays at 1.0.
        Ok(self.commit().await)
    }

    /// Pinned panels float above everything (status level) and never hide with
    /// the app; unpinned panels use the normal floating level.
    pub async fn set_pinned(&self, pinned: bool) -> BlueyResult<PanelState> {
        let _mutation = self.mutation_lock.lock().await;
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
