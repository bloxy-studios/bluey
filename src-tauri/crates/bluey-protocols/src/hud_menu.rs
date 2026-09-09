//! Bounded, display-only HUD menu wire contract. No domain actions or native handles.
//! Mirrors `src/lib/tauri/hud-menu-types.ts`; validation precedes any AppKit allocation.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};

use bluey_core::{BlueyError, BlueyResult};
use serde::{Deserialize, Serialize};

pub const MAX_ITEMS: usize = 128;
pub const MAX_ID_BYTES: usize = 64;
pub const MAX_LABEL_CHARS: usize = 80;
pub const MAX_CLIENT_POSITION: f64 = 16_384.0;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum HudMenuAlign {
    Start,
    End,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum HudMenuIcon {
    Manage,
    Play,
    Pause,
    Stop,
    History,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum HudMenuItem {
    Item {
        id: String,
        label: String,
        enabled: bool,
        checked: Option<bool>,
        icon: Option<HudMenuIcon>,
        destructive: bool,
    },
    Label {
        id: String,
        label: String,
    },
    Separator {
        id: String,
    },
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HudMenuPosition {
    /// Client logical pixels from the top-left of the undecorated HUD content view.
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HudMenuRequest {
    pub items: Vec<HudMenuItem>,
    pub position: HudMenuPosition,
    pub align: HudMenuAlign,
}

impl HudMenuRequest {
    pub fn validate(&self, window_label: &str) -> BlueyResult<()> {
        if window_label != "main" {
            return Err(BlueyError::invalid_params(
                "HUD menus belong to the main window",
            ));
        }
        if self.items.is_empty() || self.items.len() > MAX_ITEMS {
            return Err(BlueyError::invalid_params(
                "HUD menu item count is out of bounds",
            ));
        }
        for value in [self.position.x, self.position.y] {
            if !value.is_finite() || !(0.0..=MAX_CLIENT_POSITION).contains(&value) {
                return Err(BlueyError::invalid_params(
                    "Invalid HUD menu client position",
                ));
            }
        }
        let mut ids = HashSet::new();
        for item in &self.items {
            let (id, label) = match item {
                HudMenuItem::Item { id, label, .. } | HudMenuItem::Label { id, label } => {
                    (id, Some(label))
                }
                HudMenuItem::Separator { id } => (id, None),
            };
            if id.is_empty()
                || id.len() > MAX_ID_BYTES
                || !id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
                || !ids.insert(id)
            {
                return Err(BlueyError::invalid_params(
                    "Invalid or duplicate HUD menu id",
                ));
            }
            if let Some(label) = label {
                if label.trim().is_empty()
                    || label.chars().count() > MAX_LABEL_CHARS
                    || label
                        .chars()
                        .any(|c| c.is_control() || c == '\u{2028}' || c == '\u{2029}')
                {
                    return Err(BlueyError::invalid_params("Invalid HUD menu display label"));
                }
            }
        }
        Ok(())
    }

    /// Recheck against the current native content view, not a caller-supplied screen size.
    pub fn validate_view_size(&self, width: f64, height: f64) -> BlueyResult<()> {
        if !width.is_finite()
            || !height.is_finite()
            || width <= 0.0
            || height <= 0.0
            || self.position.x > width
            || self.position.y > height
        {
            return Err(BlueyError::invalid_params(
                "HUD menu anchor is outside its content view",
            ));
        }
        Ok(())
    }

    /// NSMenuItem tags are local indices, never domain IDs or commands. Revalidate selection.
    pub fn selected_id(&self, index: Option<usize>) -> Option<String> {
        match self.items.get(index?)? {
            HudMenuItem::Item {
                id, enabled: true, ..
            } => Some(id.clone()),
            _ => None,
        }
    }
}

/// Non-queuing popup lease. Kept by the main-thread task, NOT the IPC receiver;
/// a dropped caller must not unlock a menu that AppKit is still tracking.
#[derive(Debug, Default)]
pub struct PopupGate(AtomicBool);

impl PopupGate {
    pub const fn new() -> Self {
        Self(AtomicBool::new(false))
    }

    pub fn try_acquire(&self) -> Option<PopupPermit<'_>> {
        self.0
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| PopupPermit { gate: self })
    }
}

#[derive(Debug)]
#[must_use = "Keep the permit until native tracking and resource cleanup have finished"]
pub struct PopupPermit<'a> {
    gate: &'a PopupGate,
}

impl Drop for PopupPermit<'_> {
    fn drop(&mut self) {
        self.gate.0.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests;
