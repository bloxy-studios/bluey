//! Pure panel geometry: clamping into a work area, step moves, and per-display
//! position memory. All values are **logical** (scale-independent) pixels.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// A rectangle in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Centre point.
    pub fn center(&self) -> (f64, f64) {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
    }

    /// Whether a point lies inside (right/bottom edges exclusive).
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
}

/// Direction for step moves. Serialized lowercase (`PanelMoveDirection` in TS).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MoveDirection {
    Up,
    Down,
    Left,
    Right,
}

/// Clamp `rect`'s origin so it lies fully inside `work` (size unchanged).
/// Rects larger than the work area pin to the work area origin.
pub fn clamp_into(rect: Rect, work: Rect) -> Rect {
    let max_x = work.x + (work.width - rect.width).max(0.0);
    let max_y = work.y + (work.height - rect.height).max(0.0);
    Rect {
        x: rect.x.clamp(work.x, max_x),
        y: rect.y.clamp(work.y, max_y),
        width: rect.width,
        height: rect.height,
    }
}

/// Move `rect` one `step` in `direction`, clamped into `work`.
pub fn step_move(rect: Rect, direction: MoveDirection, step: f64, work: Rect) -> Rect {
    let step = step.max(0.0);
    let moved = match direction {
        MoveDirection::Up => Rect {
            y: rect.y - step,
            ..rect
        },
        MoveDirection::Down => Rect {
            y: rect.y + step,
            ..rect
        },
        MoveDirection::Left => Rect {
            x: rect.x - step,
            ..rect
        },
        MoveDirection::Right => Rect {
            x: rect.x + step,
            ..rect
        },
    };
    clamp_into(moved, work)
}

/// Pick the work area whose display contains `point`, falling back to the one
/// whose centre is nearest.
pub fn work_area_at(areas: &[(String, Rect)], x: f64, y: f64) -> Option<&(String, Rect)> {
    if areas.is_empty() {
        return None;
    }
    areas
        .iter()
        .find(|(_, area)| area.contains(x, y))
        .or_else(|| {
            areas.iter().min_by(|(_, a), (_, b)| {
                let da = distance2(a.center(), (x, y));
                let db = distance2(b.center(), (x, y));
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })
        })
}

fn distance2(a: (f64, f64), b: (f64, f64)) -> f64 {
    let dx = a.0 - b.0;
    let dy = a.1 - b.1;
    dx * dx + dy * dy
}

/// Default position for a panel on a display: horizontally centred, near the
/// top of the work area (below the menu bar).
pub fn default_position(panel_width: f64, work: Rect) -> (f64, f64) {
    let x = work.x + ((work.width - panel_width) / 2.0).max(0.0);
    let y = work.y + 24.0_f64.min(work.height / 10.0);
    (x, y)
}

/// Remembered panel position per display id. Serialized as a plain map so it
/// round-trips through the settings key/value store.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PanelPositions(pub BTreeMap<String, (f64, f64)>);

impl PanelPositions {
    /// Remember the panel position for a display.
    pub fn remember(&mut self, display_id: &str, x: f64, y: f64) {
        self.0.insert(display_id.to_string(), (x, y));
    }

    /// Recall the remembered position for a display.
    pub fn recall(&self, display_id: &str) -> Option<(f64, f64)> {
        self.0.get(display_id).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    const WORK: Rect = Rect {
        x: 0.0,
        y: 25.0,
        width: 1512.0,
        height: 950.0,
    };

    #[test]
    fn clamps_all_edges() {
        let panel = Rect::new(-50.0, 0.0, 690.0, 108.0);
        let c = clamp_into(panel, WORK);
        assert_eq!((c.x, c.y), (0.0, 25.0));

        let panel = Rect::new(2000.0, 2000.0, 690.0, 108.0);
        let c = clamp_into(panel, WORK);
        assert_eq!((c.x, c.y), (1512.0 - 690.0, 25.0 + 950.0 - 108.0));

        // Oversized rect pins to origin.
        let panel = Rect::new(100.0, 100.0, 3000.0, 2000.0);
        let c = clamp_into(panel, WORK);
        assert_eq!((c.x, c.y), (0.0, 25.0));
        assert_eq!((c.width, c.height), (3000.0, 2000.0));
    }

    #[test]
    fn step_moves_clamp_at_edges() {
        let panel = Rect::new(400.0, 30.0, 690.0, 108.0);
        let up = step_move(panel, MoveDirection::Up, 40.0, WORK);
        assert_eq!(up.y, 25.0, "clamped to the top of the work area");
        let down = step_move(panel, MoveDirection::Down, 40.0, WORK);
        assert_eq!(down.y, 70.0);
        let left = step_move(panel, MoveDirection::Left, 40.0, WORK);
        assert_eq!(left.x, 360.0);
        let right = step_move(panel, MoveDirection::Right, 40.0, WORK);
        assert_eq!(right.x, 440.0);
    }

    #[test]
    fn picks_the_work_area_under_a_point() {
        let areas = vec![
            ("main".to_string(), Rect::new(0.0, 0.0, 1512.0, 982.0)),
            ("ext".to_string(), Rect::new(1512.0, 0.0, 2560.0, 1440.0)),
        ];
        assert_eq!(work_area_at(&areas, 100.0, 100.0).unwrap().0, "main");
        assert_eq!(work_area_at(&areas, 1600.0, 100.0).unwrap().0, "ext");
        // Off every display → nearest centre.
        assert_eq!(work_area_at(&areas, -500.0, 5000.0).unwrap().0, "main");
        assert!(work_area_at(&[], 0.0, 0.0).is_none());
    }

    #[test]
    fn default_position_centres_near_top() {
        let (x, y) = default_position(690.0, WORK);
        assert_eq!(x, (1512.0 - 690.0) / 2.0);
        assert_eq!(y, 49.0);
    }

    #[test]
    fn per_display_memory_round_trips() {
        let mut memory = PanelPositions::default();
        memory.remember("69733382", 411.0, 49.0);
        memory.remember("ext", 1800.0, 60.0);
        assert_eq!(memory.recall("69733382"), Some((411.0, 49.0)));
        assert_eq!(memory.recall("missing"), None);
        let json = serde_json::to_string(&memory).unwrap();
        let back: PanelPositions = serde_json::from_str(&json).unwrap();
        assert_eq!(back, memory);
    }
}
