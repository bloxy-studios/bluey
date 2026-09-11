//! Capture manager: display/window/region/active-window capture via the
//! helper, frame cache, OCR result cache, observation, snapshot persistence
//! and window content protection.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use base64::Engine;
use bluey_core::events::BlueyEvent;
use bluey_core::types::{
    CaptureOptions, CaptureProtection, CaptureTarget, CaptureTargetPreference, DisplayInfo,
    DisplayMode, OcrContext, OcrLevel, ScreenFrame,
};
use bluey_core::{BlueyError, BlueyResult};
use bluey_protocols::helper::{self as helper_proto, CapturableWindow};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use crate::events::EventBus;
use crate::settings::SettingsManager;
use crate::sidecar::HelperClient;
use crate::storage::Storage;

/// Honest ADR-0006 notes shown in the privacy centre.
const PROTECTED_NOTE: &str = "Bluey is excluded from most screen sharing and recording \
(ScreenCaptureKit and window-list capture). Hardware capture devices and some virtual \
displays may still see it.";
const UNPROTECTED_NOTE: &str = "Bluey windows are visible in screen shares and recordings.";

/// Frames remembered per id. Inline captures carry their image here, so the
/// bound keeps memory flat over a long session; the helper's temp files are
/// its own concern (stale ones go at helper startup).
const FRAME_CACHE_CAPACITY: usize = 8;

/// A captured frame the app still holds: the helper's temp file when it wrote
/// one, the base64 image when the capture asked for it inline. Either one
/// serves `read_frame` and OCR (`ocr.recognize` accepts `path` or `image`).
#[derive(Debug, Clone, Default, PartialEq)]
struct CachedFrame {
    path: Option<PathBuf>,
    image: Option<String>,
}

/// Insertion-ordered frame cache bounded at [`FRAME_CACHE_CAPACITY`].
#[derive(Default)]
struct FrameCache {
    order: VecDeque<String>,
    entries: HashMap<String, CachedFrame>,
}

impl FrameCache {
    fn insert(&mut self, id: String, frame: CachedFrame) {
        if self.entries.insert(id.clone(), frame).is_none() {
            self.order.push_back(id);
        }
        while self.order.len() > FRAME_CACHE_CAPACITY {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
    }

    fn get(&self, id: &str) -> Option<CachedFrame> {
        self.entries.get(id).cloned()
    }

    fn remove(&mut self, id: &str) -> Option<CachedFrame> {
        self.order.retain(|known| known != id);
        self.entries.remove(id)
    }
}

pub struct CaptureManager {
    app: AppHandle,
    helper: Arc<HelperClient>,
    storage: Arc<Storage>,
    settings: Arc<SettingsManager>,
    bus: Arc<EventBus>,
    sessions: Arc<crate::sessions::SessionManager>,
    ax: Arc<crate::accessibility::AxManager>,
    frames: parking_lot::Mutex<FrameCache>,
    last_ocr: parking_lot::Mutex<Option<(String, OcrContext)>>,
    observing: AtomicBool,
    protection: AtomicBool,
}

impl CaptureManager {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        app: AppHandle,
        helper: Arc<HelperClient>,
        storage: Arc<Storage>,
        settings: Arc<SettingsManager>,
        bus: Arc<EventBus>,
        sessions: Arc<crate::sessions::SessionManager>,
        ax: Arc<crate::accessibility::AxManager>,
    ) -> Self {
        let protection = settings.get().privacy.display_mode == DisplayMode::Privacy;
        Self {
            app,
            helper,
            storage,
            settings,
            bus,
            sessions,
            ax,
            frames: parking_lot::Mutex::new(FrameCache::default()),
            last_ocr: parking_lot::Mutex::new(None),
            observing: AtomicBool::new(false),
            protection: AtomicBool::new(protection),
        }
    }

    pub async fn list_displays(&self) -> BlueyResult<Vec<DisplayInfo>> {
        let value = self.helper.call("displays.list", json!({})).await?;
        let result: helper_proto::DisplaysResult = serde_json::from_value(value)
            .map_err(|_| BlueyError::internal("malformed displays response"))?;
        Ok(result.displays)
    }

    pub async fn list_windows(&self) -> BlueyResult<Vec<CapturableWindow>> {
        let value = self
            .helper
            .call("windows.list", json!({ "onScreenOnly": true }))
            .await?;
        let result: helper_proto::WindowsResult = serde_json::from_value(value)
            .map_err(|_| BlueyError::internal("malformed windows response"))?;
        Ok(result.windows)
    }

    /// Default capture target from settings.
    fn default_target(&self) -> CaptureTarget {
        let screen = self.settings.get().screen;
        match screen.capture_target {
            CaptureTargetPreference::ActiveWindow => CaptureTarget::ActiveWindow,
            CaptureTargetPreference::Region | CaptureTargetPreference::Display => {
                // A region preference without a stored rect degrades to display.
                let display_id = match screen.preferred_display.as_str() {
                    "" | "active" | "main" => None,
                    id => Some(id.to_string()),
                };
                CaptureTarget::Display { display_id }
            }
        }
    }

    /// Capture a frame. Merges option defaults from settings, maps the helper
    /// frame, caches the frame path, publishes `screen.captured` and persists a
    /// snapshot row when a session is active.
    pub async fn capture(&self, options: Option<CaptureOptions>) -> BlueyResult<ScreenFrame> {
        let screen_settings = self.settings.get().screen;
        let options = options.unwrap_or_default();
        let target = options
            .target
            .clone()
            .unwrap_or_else(|| self.default_target());

        let mut params = serde_json::Map::new();
        params.insert(
            "format".into(),
            json!(match options.format {
                Some(bluey_core::types::ImageFormat::Png) => "png",
                _ => "jpeg",
            }),
        );
        params.insert("quality".into(), json!(options.quality.unwrap_or(0.8)));
        params.insert(
            "maxDimension".into(),
            json!(options
                .max_dimension
                .unwrap_or(screen_settings.max_image_dimension)),
        );
        params.insert("inline".into(), json!(options.inline.unwrap_or(false)));
        params.insert(
            "changeDetection".into(),
            json!(options.change_detection.unwrap_or(true)),
        );
        params.insert("excludeSelf".into(), json!(true));

        let method = match &target {
            CaptureTarget::Display { display_id } => {
                if let Some(id) = display_id {
                    params.insert("displayId".into(), json!(id));
                }
                "capture.display"
            }
            CaptureTarget::Window { window_id } => {
                let id = window_id.ok_or_else(|| {
                    BlueyError::invalid_params("window capture requires a windowId")
                })?;
                params.insert("windowId".into(), json!(id));
                "capture.window"
            }
            CaptureTarget::Region { display_id, rect } => {
                if let Some(id) = display_id {
                    params.insert("displayId".into(), json!(id));
                }
                params.insert("rect".into(), serde_json::to_value(rect)?);
                "capture.region"
            }
            CaptureTarget::ActiveWindow => "capture.activeWindow",
        };

        let value = self.helper.call(method, Value::Object(params)).await?;
        let wire: helper_proto::WireFrame = serde_json::from_value(value)
            .map_err(|_| BlueyError::capture("malformed", "malformed capture response"))?;
        let frame = helper_proto::frame_to_screen_frame(wire, target);

        // An inline capture used to arrive without a temp file, so the frame id
        // was never cached and the OCR that follows every ⌘↵ failed with
        // `unknown_frame`. Remember whatever the helper returned — path, image
        // or both — so OCR and `read_frame` can always find the frame.
        if frame.path.is_some() || frame.image.is_some() {
            self.frames.lock().insert(
                frame.id.clone(),
                CachedFrame {
                    path: frame.path.as_deref().map(PathBuf::from),
                    image: frame.image.clone(),
                },
            );
        }

        // The event mirrors the frame without the inline image (kept small).
        let mut event_frame = frame.clone();
        event_frame.image = None;
        self.bus.publish(BlueyEvent::ScreenCaptured(event_frame));

        self.persist_snapshot(&frame);
        Ok(frame)
    }

    /// Save a `screen_snapshots` row when a session is active (image path only
    /// when the privacy setting allows).
    fn persist_snapshot(&self, frame: &ScreenFrame) {
        let Some(session_id) = self.sessions.active_id() else {
            return;
        };
        let store_image = self.settings.get().privacy.store_screenshots;
        let frontmost = self.ax.cached_frontmost();
        let storage = self.storage.clone();
        let frame = frame.clone();
        tauri::async_runtime::spawn(async move {
            let result = storage
                .run(move |db| {
                    bluey_storage::SnapshotRepository::save_screen(
                        db,
                        Some(&session_id),
                        &frame,
                        None,
                        frontmost.as_ref().map(|f| &f.application),
                        frontmost.as_ref().and_then(|f| f.window.as_ref()),
                        store_image,
                    )
                })
                .await;
            if let Err(e) = result {
                tracing::warn!(error = %e, "failed to persist screen snapshot");
            }
        });
    }

    fn cached_frame(&self, frame_id: &str) -> BlueyResult<CachedFrame> {
        self.frames
            .lock()
            .get(frame_id)
            .ok_or_else(|| BlueyError::capture("unknown_frame", "unknown frame id"))
    }

    /// Read a cached frame back as base64 (the temp file, else the inline image).
    pub async fn read_frame(&self, frame_id: &str) -> BlueyResult<String> {
        let cached = self.cached_frame(frame_id)?;
        if let Some(path) = &cached.path {
            if let Ok(bytes) = tokio::fs::read(path).await {
                return Ok(base64::engine::general_purpose::STANDARD.encode(bytes));
            }
            if cached.image.is_none() {
                return Err(BlueyError::capture(
                    "read_failed",
                    "cannot read the cached frame",
                ));
            }
        }
        cached
            .image
            .ok_or_else(|| BlueyError::capture("unknown_frame", "unknown frame id"))
    }

    /// Discard a cached frame (helper deletes the temp file when there is one).
    pub async fn discard_frame(&self, frame_id: &str) -> BlueyResult<()> {
        let cached = self.frames.lock().remove(frame_id);
        if let Some(path) = cached.and_then(|frame| frame.path) {
            let _ = self
                .helper
                .call("capture.discard", json!({ "path": path.to_string_lossy() }))
                .await;
            // Belt and braces if the helper already exited.
            let _ = tokio::fs::remove_file(&path).await;
        }
        Ok(())
    }

    /// OCR a cached frame. When `frame.changed == false` and a cached OCR for
    /// the same hash exists, the cache is reused (fast path).
    pub async fn ocr(
        &self,
        frame_id: &str,
        level: Option<OcrLevel>,
        languages: Option<Vec<String>>,
        frame_hash: Option<&str>,
        frame_changed: bool,
    ) -> BlueyResult<OcrContext> {
        if !frame_changed {
            if let (Some(hash), Some((cached_hash, cached))) =
                (frame_hash, self.last_ocr.lock().clone())
            {
                if cached_hash == hash {
                    let mut reused = cached;
                    reused.frame_id = Some(frame_id.to_string());
                    return Ok(reused);
                }
            }
        }
        let screen = self.settings.get().screen;
        let level = level.unwrap_or(screen.ocr_level);
        let languages = languages.unwrap_or(screen.ocr_languages);
        let cached = self.cached_frame(frame_id)?;
        let mut params = json!({
            "level": match level { OcrLevel::Fast => "fast", OcrLevel::Accurate => "accurate" },
            "languages": languages,
            "minConfidence": 0.3,
        });
        // `ocr.recognize` takes the temp file by path, or the image itself when
        // an inline capture has no file (docs/HELPER_PROTOCOL.md › ocr.recognize).
        match (&cached.path, &cached.image) {
            (Some(path), _) => params["path"] = json!(path.to_string_lossy()),
            (None, Some(image)) => params["image"] = json!(image),
            (None, None) => return Err(BlueyError::capture("unknown_frame", "unknown frame id")),
        }
        let value = self.helper.call("ocr.recognize", params).await?;
        let wire: helper_proto::WireOcrResult = serde_json::from_value(value)
            .map_err(|_| BlueyError::internal("malformed OCR response"))?;
        let context =
            helper_proto::ocr_to_context(wire, level, languages, Some(frame_id.to_string()));
        if let Some(hash) = frame_hash {
            *self.last_ocr.lock() = Some((hash.to_string(), context.clone()));
        }
        self.bus.publish(BlueyEvent::OcrCompleted(context.clone()));
        Ok(context)
    }

    /// Start low-FPS change observation.
    pub async fn observe_start(
        &self,
        interval_ms: Option<u32>,
        display_id: Option<String>,
    ) -> BlueyResult<()> {
        let interval = interval_ms.unwrap_or(self.settings.get().screen.observation_interval_ms);
        let mut params = json!({ "intervalMs": interval, "minDelta": 0.04 });
        if let Some(id) = display_id {
            params["displayId"] = json!(id);
        }
        self.helper.call("observe.start", params).await?;
        self.observing.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// Stop observation.
    pub async fn observe_stop(&self) -> BlueyResult<()> {
        if self.helper.is_running() {
            let _ = self.helper.request("observe.stop", json!({})).await;
        }
        self.observing.store(false, Ordering::SeqCst);
        Ok(())
    }

    pub fn is_observing(&self) -> bool {
        self.observing.load(Ordering::SeqCst)
    }

    /// Current content-protection status.
    pub fn protection(&self) -> CaptureProtection {
        let enabled = self.protection.load(Ordering::SeqCst);
        CaptureProtection {
            supported: cfg!(target_os = "macos"),
            enabled,
            note: if enabled {
                PROTECTED_NOTE
            } else {
                UNPROTECTED_NOTE
            }
            .to_string(),
        }
    }

    /// Toggle `NSWindow.sharingType`-based protection on every Bluey window.
    pub fn set_protection(&self, enabled: bool) -> BlueyResult<CaptureProtection> {
        for (_, window) in self.app.webview_windows() {
            window.set_content_protected(enabled).map_err(|e| {
                BlueyError::capture("protection", format!("cannot set content protection: {e}"))
            })?;
        }
        self.protection.store(enabled, Ordering::SeqCst);
        Ok(self.protection())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(path: Option<&str>, image: Option<&str>) -> CachedFrame {
        CachedFrame {
            path: path.map(PathBuf::from),
            image: image.map(str::to_string),
        }
    }

    #[test]
    fn the_cache_keeps_inline_images_as_well_as_paths() {
        let mut cache = FrameCache::default();
        cache.insert("f-1".into(), frame(Some("/tmp/f-1.jpg"), None));
        cache.insert("f-2".into(), frame(None, Some("QUJD")));
        assert_eq!(cache.get("f-1"), Some(frame(Some("/tmp/f-1.jpg"), None)));
        assert_eq!(
            cache.get("f-2"),
            Some(frame(None, Some("QUJD"))),
            "an inline capture without a temp file is still a known frame"
        );
        assert_eq!(cache.remove("f-2"), Some(frame(None, Some("QUJD"))));
        assert_eq!(cache.get("f-2"), None);
        assert_eq!(cache.order.len(), 1);
    }

    #[test]
    fn the_cache_forgets_the_oldest_frames_past_its_capacity() {
        let mut cache = FrameCache::default();
        for i in 0..(FRAME_CACHE_CAPACITY + 3) {
            cache.insert(format!("f-{i}"), frame(None, Some("QUJD")));
        }
        assert_eq!(cache.entries.len(), FRAME_CACHE_CAPACITY);
        assert_eq!(cache.order.len(), FRAME_CACHE_CAPACITY);
        assert_eq!(cache.get("f-0"), None, "the oldest went first");
        assert!(cache
            .get(&format!("f-{}", FRAME_CACHE_CAPACITY + 2))
            .is_some());
        // Re-inserting a known id keeps one slot for it.
        cache.insert("f-5".into(), frame(Some("/tmp/f-5.jpg"), None));
        assert_eq!(cache.order.iter().filter(|id| *id == "f-5").count(), 1);
    }
}
