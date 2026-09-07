//! Accessibility manager: helper-backed AX snapshots and frontmost-app info
//! with a 500 ms cache, `accessibility.updated` + `activeApp.changed` events.

use std::sync::Arc;
use std::time::{Duration, Instant};

use bluey_core::events::BlueyEvent;
use bluey_core::types::AccessibilityContext;
use bluey_core::{BlueyError, BlueyResult};
use bluey_protocols::helper::FrontmostApp;
use serde_json::json;

use crate::events::EventBus;
use crate::sidecar::HelperClient;

const CACHE_TTL: Duration = Duration::from_millis(500);

pub struct AxManager {
    helper: Arc<HelperClient>,
    bus: Arc<EventBus>,
    snapshot_cache: parking_lot::Mutex<Option<(Instant, AccessibilityContext)>>,
    frontmost_cache: parking_lot::Mutex<Option<(Instant, FrontmostApp)>>,
    last_frontmost_key: parking_lot::Mutex<Option<String>>,
}

impl AxManager {
    pub fn new(helper: Arc<HelperClient>, bus: Arc<EventBus>) -> Self {
        Self {
            helper,
            bus,
            snapshot_cache: parking_lot::Mutex::new(None),
            frontmost_cache: parking_lot::Mutex::new(None),
            last_frontmost_key: parking_lot::Mutex::new(None),
        }
    }

    /// AX snapshot of the frontmost window (500 ms cache). Publishes
    /// `accessibility.updated` on fresh fetches.
    pub async fn snapshot(
        &self,
        max_depth: Option<u32>,
        max_elements: Option<u32>,
    ) -> BlueyResult<AccessibilityContext> {
        if let Some((at, cached)) = self.snapshot_cache.lock().clone() {
            if at.elapsed() < CACHE_TTL {
                return Ok(cached);
            }
        }
        let params = json!({
            "maxDepth": max_depth.unwrap_or(6),
            "maxElements": max_elements.unwrap_or(150),
            "includeSelectedText": true,
        });
        let value = self.helper.call("accessibility.snapshot", params).await?;
        let context = bluey_protocols::helper::parse_ax_snapshot(value)
            .map_err(|_| BlueyError::internal("malformed accessibility snapshot"))?;
        *self.snapshot_cache.lock() = Some((Instant::now(), context.clone()));
        self.bus
            .publish(BlueyEvent::AccessibilityUpdated(context.clone()));
        Ok(context)
    }

    /// Frontmost application + window (500 ms cache).
    pub async fn frontmost(&self) -> BlueyResult<FrontmostApp> {
        if let Some((at, cached)) = self.frontmost_cache.lock().clone() {
            if at.elapsed() < CACHE_TTL {
                return Ok(cached);
            }
        }
        let value = self.helper.call("app.frontmost", json!({})).await?;
        let frontmost: FrontmostApp = serde_json::from_value(value)
            .map_err(|_| BlueyError::internal("malformed frontmost-app response"))?;
        *self.frontmost_cache.lock() = Some((Instant::now(), frontmost.clone()));
        Ok(frontmost)
    }

    /// Cached frontmost app without hitting the helper (snapshot save paths).
    pub fn cached_frontmost(&self) -> Option<FrontmostApp> {
        self.frontmost_cache.lock().as_ref().map(|(_, f)| f.clone())
    }

    /// Poll step used while the HUD is visible: fetch frontmost, publish
    /// `activeApp.changed` when it differs from the previous poll.
    pub async fn poll_active_app(&self) {
        let Ok(frontmost) = self.frontmost().await else {
            return;
        };
        let key = format!(
            "{}|{}|{}",
            frontmost.application.name,
            frontmost.application.bundle_id.as_deref().unwrap_or(""),
            frontmost
                .window
                .as_ref()
                .and_then(|w| w.title.as_deref())
                .unwrap_or("")
        );
        let changed = {
            let mut last = self.last_frontmost_key.lock();
            if last.as_deref() == Some(key.as_str()) {
                false
            } else {
                *last = Some(key);
                true
            }
        };
        if changed {
            self.bus.publish(BlueyEvent::ActiveAppChanged {
                name: frontmost.application.name.clone(),
                bundle_id: frontmost.application.bundle_id.clone(),
                pid: frontmost.application.pid,
                window_title: frontmost.window.as_ref().and_then(|w| w.title.clone()),
            });
        }
    }
}
