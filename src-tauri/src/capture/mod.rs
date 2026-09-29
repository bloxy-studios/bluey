//! Capture manager: display/window/region/active-window capture via the
//! helper, frame cache, OCR result cache, observation, snapshot persistence
//! and window content protection.

use std::collections::{HashMap, VecDeque};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use base64::Engine;
use bluey_core::events::BlueyEvent;
use bluey_core::types::{
    CaptureOptions, CaptureProtection, CaptureTarget, CaptureTargetPreference, DisplayInfo,
    DisplayMode, ImageMimeType, OcrContext, OcrLevel, ScreenFrame,
};
use bluey_core::{BlueyError, BlueyResult};
use bluey_protocols::helper::{self as helper_proto, CapturableWindow};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use crate::events::EventBus;
use crate::settings::SettingsManager;
use crate::sidecar::HelperClient;
use crate::storage::Storage;

/// Honest ADR-0006 notes shown in the privacy centre (SEC-004). Protection is
/// `NSWindow.sharingType = .none`, which ScreenCaptureKit on macOS 15 and
/// later may not honour; NSMenu popups are separate windows and never covered.
const PROTECTED_NOTE: &str = "Bluey is hidden from screen sharing and recording that honour \
macOS window protection. Hardware capture devices, some virtual displays and Bluey's menus may \
still show it.";
const PARTIAL_NOTE: &str = "Bluey is hidden from apps that honour macOS window protection \
(legacy capture). Modern ScreenCaptureKit screen sharing and recording on macOS 15 and later may \
still show Bluey, and its menus are never hidden.";
const UNPROTECTED_NOTE: &str = "Bluey windows are visible in screen shares and recordings.";

/// First macOS whose ScreenCaptureKit may ignore `NSWindow.sharingType`.
const SHARING_TYPE_PARTIAL_FROM: u32 = 15;

/// The protection status to report: an unknown macOS version counts as
/// partial, never as fully hidden.
fn protection_status(
    supported: bool,
    enabled: bool,
    macos_major: Option<u32>,
) -> CaptureProtection {
    let partial =
        supported && enabled && macos_major.is_none_or(|major| major >= SHARING_TYPE_PARTIAL_FROM);
    let note = match (enabled, partial) {
        (false, _) => UNPROTECTED_NOTE,
        (true, true) => PARTIAL_NOTE,
        (true, false) => PROTECTED_NOTE,
    };
    CaptureProtection {
        supported,
        enabled,
        partial,
        note: note.to_string(),
    }
}

/// Frames remembered per id. Inline captures carry their image here, so the
/// bound keeps memory flat over a long session. A frame's helper temp file is
/// deleted when the frame leaves the cache or once its image is held inline
/// (DATA-001); the helper sweeps whatever is left.
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
    /// Remember `frame`; returns the frames pushed out past capacity so their
    /// temp files can be deleted.
    fn insert(&mut self, id: String, frame: CachedFrame) -> Vec<CachedFrame> {
        if self.entries.insert(id.clone(), frame).is_none() {
            self.order.push_back(id);
        }
        let mut evicted = Vec::new();
        while self.order.len() > FRAME_CACHE_CAPACITY {
            if let Some(oldest) = self.order.pop_front() {
                evicted.extend(self.entries.remove(&oldest));
            }
        }
        evicted
    }

    /// Once the frame's image is held in memory (`image`, or the one it came
    /// with) its temp file is redundant: returns the path to delete. A frame
    /// without an image keeps its file, since it is the only copy.
    fn release_file(&mut self, id: &str, image: Option<&str>) -> Option<PathBuf> {
        let entry = self.entries.get_mut(id)?;
        if entry.image.is_none() {
            entry.image = image.map(str::to_string);
        }
        entry.image.as_ref()?;
        entry.path.take()
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
            CaptureTargetPreference::Display => {
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
            let evicted = self.frames.lock().insert(
                frame.id.clone(),
                CachedFrame {
                    path: frame.path.as_deref().map(PathBuf::from),
                    image: frame.image.clone(),
                },
            );
            self.delete_temp_frames(evicted.into_iter().filter_map(|f| f.path).collect());
        }

        // The event mirrors the frame without the inline image (kept small).
        let mut event_frame = frame.clone();
        event_frame.image = None;
        self.bus.publish(BlueyEvent::ScreenCaptured(event_frame));

        self.persist_snapshot(&frame).await;
        Ok(frame)
    }

    /// Save a `screen_snapshots` row when a session is active. With
    /// `storeScreenshots` on, the image is first copied out of the helper's
    /// temp dir into `screenshots_dir`, so the row never points at a temp
    /// file that is deleted after use or swept (DATA-001).
    async fn persist_snapshot(&self, frame: &ScreenFrame) {
        let Some(session_id) = self.sessions.active_id() else {
            return;
        };
        let mut frame = frame.clone();
        frame.path = if self.settings.get().privacy.store_screenshots {
            keep_screenshot(&self.storage.paths.screenshots_dir, &frame).await
        } else {
            None
        };
        frame.image = None;
        let store_image = frame.path.is_some();
        let frontmost = self.ax.cached_frontmost();
        let storage = self.storage.clone();
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

    /// Delete helper temp frames in the background (DATA-001).
    fn delete_temp_frames(&self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        let frames_dir = self.storage.paths.frames_dir.clone();
        tauri::async_runtime::spawn(async move { remove_temp_frames(&frames_dir, paths).await });
    }

    /// The caller now holds the frame's image (a context snapshot inlined it),
    /// so the helper's temp file can go (DATA-001). OCR and `read_frame` keep
    /// working from the image.
    pub fn release_frame_file(&self, frame_id: &str, image: Option<&str>) {
        let path = self.frames.lock().release_file(frame_id, image);
        self.delete_temp_frames(path.into_iter().collect());
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
        protection_status(
            cfg!(target_os = "macos"),
            self.protection.load(Ordering::SeqCst),
            crate::platform::macos_major_version(),
        )
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

/// Copy a frame the user chose to keep into `dir` — from the temp file, else
/// from the inline image. Returns the stored path.
async fn keep_screenshot(dir: &Path, frame: &ScreenFrame) -> Option<String> {
    let safe_id = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
    if frame.id.is_empty() || !frame.id.chars().all(safe_id) {
        return None;
    }
    let ext = match frame.mime_type {
        ImageMimeType::Png => "png",
        ImageMimeType::Jpeg => "jpg",
        ImageMimeType::Webp => "webp",
    };
    let target = dir.join(format!("{}.{ext}", frame.id));
    let result = match (&frame.path, &frame.image) {
        (Some(path), _) => tokio::fs::copy(path, &target).await.map(|_| ()),
        (None, Some(image)) => match base64::engine::general_purpose::STANDARD.decode(image) {
            Ok(bytes) => tokio::fs::write(&target, bytes).await,
            Err(e) => Err(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
        },
        (None, None) => return None,
    };
    match result {
        Ok(()) => Some(target.to_string_lossy().into_owned()),
        Err(e) => {
            tracing::warn!(error = %e, "cannot keep the screenshot");
            None
        }
    }
}

/// Delete temp frames the app no longer needs. Only files inside
/// `frames_dir` are touched, whatever path the helper reported.
async fn remove_temp_frames(frames_dir: &Path, paths: Vec<PathBuf>) {
    for path in paths {
        let inside = path.starts_with(frames_dir)
            && !path.components().any(|c| matches!(c, Component::ParentDir));
        if !inside {
            tracing::warn!("not deleting a frame outside the frames directory");
            continue;
        }
        if let Err(e) = tokio::fs::remove_file(&path).await {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::debug!(error = %e, "cannot delete a temp frame");
            }
        }
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
    fn protection_is_reported_as_partial_where_screencapturekit_may_ignore_it() {
        let sequoia = protection_status(true, true, Some(15));
        assert!(sequoia.partial);
        assert!(sequoia
            .note
            .contains("macOS 15 and later may still show Bluey"));
        assert!(protection_status(true, true, Some(26)).partial);
        assert!(
            protection_status(true, true, None).partial,
            "unknown is never full"
        );
        let sonoma = protection_status(true, true, Some(14));
        assert!(!sonoma.partial);
        assert!(!sonoma.note.contains("ScreenCaptureKit"));
        assert!(sonoma.note.contains("menus"));
        let off = protection_status(true, false, Some(26));
        assert!(!off.partial);
        assert_eq!(off.note, UNPROTECTED_NOTE);
        assert!(!protection_status(false, true, None).partial);
    }

    #[test]
    fn eviction_hands_back_the_frames_whose_files_must_go() {
        let mut cache = FrameCache::default();
        let mut evicted = Vec::new();
        for i in 0..(FRAME_CACHE_CAPACITY + 2) {
            evicted
                .extend(cache.insert(format!("f-{i}"), frame(Some(&format!("/f/{i}.jpg")), None)));
        }
        assert_eq!(
            evicted,
            vec![frame(Some("/f/0.jpg"), None), frame(Some("/f/1.jpg"), None)]
        );
    }

    #[test]
    fn a_frame_releases_its_file_only_once_its_image_is_held() {
        let mut cache = FrameCache::default();
        cache.insert("f-1".into(), frame(Some("/f/1.jpg"), None));
        cache.insert("f-2".into(), frame(Some("/f/2.jpg"), Some("QUJD")));
        cache.insert("f-3".into(), frame(Some("/f/3.jpg"), None));
        // Inlined by the caller: the image moves into the cache, the file goes.
        assert_eq!(
            cache.release_file("f-1", Some("QUJD")),
            Some("/f/1.jpg".into())
        );
        assert_eq!(cache.get("f-1"), Some(frame(None, Some("QUJD"))));
        // Captured inline: the file is redundant already.
        assert_eq!(cache.release_file("f-2", None), Some("/f/2.jpg".into()));
        // No image anywhere: the file is the only copy and stays.
        assert_eq!(cache.release_file("f-3", None), None);
        assert_eq!(cache.get("f-3"), Some(frame(Some("/f/3.jpg"), None)));
        assert_eq!(cache.release_file("f-unknown", Some("QUJD")), None);
    }

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bluey-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn temp_frames_are_deleted_inside_the_frames_dir_only() {
        let root = scratch_dir("frames");
        let frames = root.join("frames");
        std::fs::create_dir_all(&frames).unwrap();
        let inside = frames.join("f-1.jpg");
        let outside = root.join("keep.jpg");
        for path in [&inside, &outside] {
            std::fs::write(path, b"jpeg").unwrap();
        }
        let sneaky = frames.join("..").join("keep.jpg");
        remove_temp_frames(&frames, vec![inside.clone(), outside.clone(), sneaky]).await;
        assert!(!inside.exists());
        assert!(
            outside.exists(),
            "a path outside the frames dir is never deleted"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn a_kept_screenshot_is_copied_out_of_the_temp_dir() {
        let dir = scratch_dir("screenshots");
        let temp = dir.join("f-1.tmp");
        std::fs::write(&temp, b"jpeg").unwrap();
        let mut shot = ScreenFrame {
            id: "f-1".into(),
            image: None,
            mime_type: ImageMimeType::Jpeg,
            path: Some(temp.to_string_lossy().into_owned()),
            width: 1,
            height: 1,
            display_id: None,
            scale_factor: 1.0,
            captured_at: String::new(),
            hash: None,
            changed: true,
            target: CaptureTarget::ActiveWindow,
            duration_ms: None,
        };
        let kept = keep_screenshot(&dir, &shot).await.expect("copied");
        std::fs::remove_file(&temp).unwrap();
        assert_eq!(
            std::fs::read(&kept).unwrap(),
            b"jpeg",
            "survives the temp file"
        );

        // An inline-only frame is written from its image.
        shot.id = "f-2".into();
        shot.path = None;
        shot.image = Some("QUJD".into());
        let kept = keep_screenshot(&dir, &shot).await.expect("written");
        assert_eq!(std::fs::read(&kept).unwrap(), b"ABC");

        shot.id = "../f-3".into();
        assert_eq!(keep_screenshot(&dir, &shot).await, None);
        let _ = std::fs::remove_dir_all(&dir);
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
