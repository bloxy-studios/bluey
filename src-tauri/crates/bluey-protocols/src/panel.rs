//! Pure panel geometry: surface/frame sizing, work-area fitting, step moves,
//! and per-display position memory. All values are **logical** pixels (macOS
//! points), never backing pixels. `PanelState` describes the native frame;
//! the appearance width describes the visible, bordered HUD surface.

use std::collections::BTreeMap;

use bluey_core::types::PanelPositionPreference;
use serde::{Deserialize, Serialize};

/// Transparent space for the existing CSS shadow (0 8px 32px). Keep in sync
/// with `src/features/hud/geometry.ts`; it is part of the measured frame.
pub const FRAME_INSET_TOP: f64 = 24.0;
pub const FRAME_INSET_RIGHT: f64 = 32.0;
pub const FRAME_INSET_BOTTOM: f64 = 40.0;
pub const FRAME_INSET_LEFT: f64 = 32.0;
pub const FRAME_INSET_Y: f64 = FRAME_INSET_TOP + FRAME_INSET_BOTTOM;
/// 56px input + 52px toolbar + their divider and the two surface borders.
/// Only a launch/no-measurement fallback: live transcript rows can be taller.
pub const COLLAPSED_FRAME_HEIGHT: f64 = 111.0 + FRAME_INSET_Y;
pub const DEFAULT_EXPANDED_FRAME_HEIGHT: f64 = 480.0 + FRAME_INSET_Y;
/// The visible surface must remain usable unless the display itself is smaller.
pub const MIN_SURFACE_WIDTH: f64 = 420.0;
pub const MIN_FRAME_WIDTH: f64 = MIN_SURFACE_WIDTH + FRAME_INSET_LEFT + FRAME_INSET_RIGHT;

/// Runtime native constraints, installed BEFORE sizing on each display. A
/// fixed 484px minimum would prevent fitting a genuinely narrower work area;
/// a permanent 1px minimum would permit an unusable window on a normal one.
pub fn minimum_frame_size(work: Rect) -> (f64, f64) {
    (
        MIN_FRAME_WIDTH.min(work.width.max(1.0)),
        COLLAPSED_FRAME_HEIGHT.min(work.height.max(1.0)),
    )
}

/// Appearance widths exclude the transparent frame insets.
pub fn frame_width(surface_width: f64) -> f64 {
    surface_width.max(1.0) + FRAME_INSET_LEFT + FRAME_INSET_RIGHT
}

/// The measured **frame** height wins in either mode. `expanded` describes
/// chat state, not whether listening/transcript rows need additional space.
/// The work-area-clamped minimum is applied by `fit_into`, not here.
pub fn frame_height(expanded: bool, measured: Option<f64>) -> f64 {
    measured
        .filter(|height| height.is_finite() && *height > 0.0)
        .map(f64::ceil)
        .unwrap_or(if expanded {
            DEFAULT_EXPANDED_FRAME_HEIGHT
        } else {
            COLLAPSED_FRAME_HEIGHT
        })
}

/// Tauri reads physical integer pixels back from the window. Compare at that
/// boundary's precision when deciding whether to issue a native operation;
/// exact float equality can loop on e.g. 175 logical points at a 1.5x scale.
/// This does NOT round or change the logical work-area geometry itself.
pub fn same_backing_pixel(a: f64, b: f64, scale: f64) -> bool {
    a.is_finite()
        && b.is_finite()
        && scale.is_finite()
        && scale > 0.0
        && (a * scale).round() == (b * scale).round()
}

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

/// Fit the native frame, not just its origin. Moving the origin of an
/// oversized frame alone still leaves its toolbar and right edge off-screen.
pub fn fit_into(rect: Rect, work: Rect) -> Rect {
    let (min_width, min_height) = minimum_frame_size(work);
    clamp_into(
        Rect {
            width: rect.width.max(min_width).min(work.width.max(1.0)),
            height: rect.height.max(min_height).min(work.height.max(1.0)),
            ..rect
        },
        work,
    )
}

/// Grow/shrink at the same top-left origin, moving it only as needed to fit.
/// Select `work` from the current frame, not the requested (possibly huge)
/// content height, which could otherwise select an adjacent display.
pub fn resize_in_work_area(current: Rect, width: f64, height: f64, work: Rect) -> Rect {
    fit_into(Rect::new(current.x, current.y, width, height), work)
}

/// Apply an appearance width without moving the visible surface's centre.
pub fn apply_surface_width(current: Rect, width: f64, work: Rect) -> Rect {
    let (min_width, _) = minimum_frame_size(work);
    let width = frame_width(width).max(min_width).min(work.width.max(1.0));
    fit_into(
        Rect::new(
            current.x + (current.width - width) / 2.0,
            current.y,
            width,
            current.height,
        ),
        work,
    )
}

/// Move `rect` one logical-point `step`, fitting the entire native frame.
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
    fit_into(moved, work)
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

/// The work area containing the current frame's centre. When monitor lookup
/// temporarily fails, preserve the origin but do not treat the old auto-height
/// as a display limit: a collapsed frame must still be able to grow.
pub fn area_for(areas: &[(String, Rect)], rect: &Rect) -> Rect {
    let (cx, cy) = rect.center();
    work_area_at(areas, cx, cy)
        .map(|(_, area)| *area)
        .unwrap_or_else(|| Rect::new(rect.x, rect.y, f64::MAX, f64::MAX))
}

/// Position preferences use native-frame dimensions, including shadow room.
pub fn preferred_position(
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
            default_position(width, work)
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
    fn surface_dimensions_include_borders_and_shadow_insets_once() {
        assert_eq!(frame_width(690.0), 754.0);
        assert_eq!(COLLAPSED_FRAME_HEIGHT, 56.0 + 52.0 + 3.0 + 24.0 + 40.0);
        // The frontend sends the already-measured frame, not a bare surface.
        assert_eq!(frame_height(false, Some(175.25)), 176.0);
        assert_eq!(frame_height(true, Some(400.0)), 400.0);
    }

    #[test]
    fn idle_transcript_height_is_honored_and_can_shrink_again() {
        assert_eq!(frame_height(false, Some(286.0)), 286.0);
        assert_eq!(frame_height(false, Some(232.0)), 232.0);
        assert_eq!(frame_height(false, Some(175.0)), 175.0);
        assert_eq!(frame_height(true, Some(160.0)), 160.0);
        for invalid in [
            None,
            Some(0.0),
            Some(-10.0),
            Some(f64::NAN),
            Some(f64::INFINITY),
        ] {
            assert_eq!(frame_height(false, invalid), COLLAPSED_FRAME_HEIGHT);
            assert_eq!(frame_height(true, invalid), DEFAULT_EXPANDED_FRAME_HEIGHT);
        }
    }

    #[test]
    fn native_no_ops_compare_backing_pixels_without_rounding_logical_work_areas() {
        assert!(same_backing_pixel(175.0, 175.0, 1.0));
        assert!(same_backing_pixel(175.25, 175.5, 2.0));
        assert!(same_backing_pixel(175.0, 263.0 / 1.5, 1.5));
        assert!(same_backing_pixel(-101.2, -101.0, 2.0));
        assert!(!same_backing_pixel(175.0, 175.5, 2.0));
        assert!(!same_backing_pixel(175.0, 175.0, 0.0));
        assert!(!same_backing_pixel(f64::NAN, f64::NAN, 2.0));
    }

    #[test]
    fn runtime_minimum_is_usable_on_normal_displays_and_relaxes_only_to_the_work_area() {
        assert_eq!(MIN_FRAME_WIDTH, 420.0 + 32.0 + 32.0);
        assert_eq!(minimum_frame_size(WORK), (484.0, 175.0));
        let narrow = Rect::new(-800.0, -100.0, 480.5, 360.25);
        assert_eq!(minimum_frame_size(narrow), (480.5, 175.0));
        let short = Rect::new(50.0, 25.0, 1200.0, 160.25);
        assert_eq!(minimum_frame_size(short), (484.0, 160.25));
        // Keep fractional logical work dimensions exact; do not round up past
        // an edge or multiply by either the old or the destination DPI scale.
        let tiny = Rect::new(-320.25, 30.5, 320.25, 150.5);
        assert_eq!(minimum_frame_size(tiny), (320.25, 150.5));
        assert_eq!(fit_into(Rect::new(0.0, 0.0, 1.0, 1.0), tiny), tiny);
        assert_eq!(minimum_frame_size(WORK), (484.0, 175.0));
    }

    #[test]
    fn fitting_enforces_the_same_work_area_clamped_minimum_as_the_native_window() {
        let undersized = Rect::new(300.0, 100.0, 1.0, 1.0);
        let fit = fit_into(undersized, WORK);
        assert_eq!(fit, Rect::new(300.0, 100.0, 484.0, 175.0));
        assert_eq!(fit_into(fit, WORK), fit);
        for dimension in [0.0, -10.0, f64::NAN] {
            assert_eq!(
                fit_into(Rect::new(300.0, 100.0, dimension, dimension), WORK),
                fit
            );
        }
        let requested = resize_in_work_area(fit, fit.width, frame_height(true, Some(160.0)), WORK);
        assert_eq!(
            requested, fit,
            "a measurement cannot cut off idle chrome on a normal display"
        );
        let short = Rect::new(0.0, 25.0, 600.0, 160.25);
        assert_eq!(
            resize_in_work_area(fit, fit.width, 684.0, short).height,
            160.25
        );
    }

    #[test]
    fn fitting_shrinks_oversized_frames_not_just_their_origin() {
        let small = Rect::new(-800.0, 30.0, 480.0, 360.0);
        let fit = fit_into(Rect::new(-700.0, 100.0, frame_width(960.0), 684.0), small);
        assert_eq!(fit, small);
        assert_eq!(fit_into(fit, small), fit, "a duplicate fit is a no-op");
    }

    #[test]
    fn content_resize_preserves_origin_until_an_edge_requires_a_move() {
        let current = Rect::new(300.0, 100.0, frame_width(690.0), COLLAPSED_FRAME_HEIGHT);
        let grown = resize_in_work_area(current, current.width, 500.0, WORK);
        assert_eq!((grown.x, grown.y), (current.x, current.y));
        let shrunk = resize_in_work_area(grown, grown.width, 175.0, WORK);
        assert_eq!(shrunk, current);
        let bottom = Rect::new(300.0, 750.0, current.width, 175.0);
        let grown = resize_in_work_area(bottom, bottom.width, 500.0, WORK);
        assert_eq!(grown.y, WORK.y + WORK.height - grown.height);
        assert_eq!(grown.x, bottom.x);
        assert_eq!(
            resize_in_work_area(grown, grown.width, grown.height, WORK),
            grown
        );
    }

    #[test]
    fn width_setting_keeps_surface_centred_and_fits_small_displays() {
        let current = Rect::new(300.0, 100.0, frame_width(690.0), 175.0);
        let wider = apply_surface_width(current, 960.0, WORK);
        assert_eq!(wider.center().0, current.center().0);
        assert_eq!(wider.width, frame_width(960.0));
        let minimum = apply_surface_width(current, 1.0, WORK);
        assert_eq!(minimum.width, MIN_FRAME_WIDTH);
        assert_eq!(minimum.center().0, current.center().0);
        let small = Rect::new(0.0, 25.0, 480.0, 400.0);
        let fitted = apply_surface_width(current, 960.0, small);
        assert_eq!((fitted.x, fitted.width), (0.0, 480.0));
    }

    #[test]
    fn keyboard_moves_keep_the_entire_frame_inside_offset_work_areas() {
        let work = Rect::new(-1280.0, -200.0, 1280.0, 720.0);
        let current = Rect::new(-950.0, -100.0, frame_width(690.0), 400.0);
        for direction in [
            MoveDirection::Up,
            MoveDirection::Down,
            MoveDirection::Left,
            MoveDirection::Right,
        ] {
            let moved = step_move(current, direction, 10_000.0, work);
            assert_eq!(fit_into(moved, work), moved);
        }
        // 24 logical points, not 48 on Retina; conversion belongs at the API boundary.
        assert_eq!(
            step_move(current, MoveDirection::Right, 24.0, work).x,
            current.x + 24.0
        );
        assert_eq!(
            step_move(current, MoveDirection::Right, -24.0, work),
            current
        );
        let tiny = Rect::new(-900.0, 30.0, 480.0, 300.0);
        assert_eq!(step_move(current, MoveDirection::Down, 24.0, tiny), tiny);
    }

    #[test]
    fn current_display_is_not_changed_by_a_tall_resize_request() {
        let upper = Rect::new(0.0, -720.0, 1280.0, 720.0);
        let lower = Rect::new(0.0, 0.0, 1280.0, 720.0);
        let areas = vec![("upper".to_string(), upper), ("lower".to_string(), lower)];
        let current = Rect::new(200.0, -700.0, 754.0, 175.0);
        let work = area_for(&areas, &current);
        assert_eq!(work, upper);
        let resized = resize_in_work_area(current, current.width, 2000.0, work);
        assert_eq!(resized.y, -720.0);
        assert_eq!(resized.height, 720.0);
        let unknown = area_for(&[], &current);
        let grown = resize_in_work_area(current, current.width, 684.0, unknown);
        assert_eq!(grown, Rect::new(current.x, current.y, current.width, 684.0));
        assert_eq!(
            minimum_frame_size(unknown),
            (MIN_FRAME_WIDTH, COLLAPSED_FRAME_HEIGHT)
        );
    }

    #[test]
    fn preferred_positions_stay_inside_the_work_area() {
        let width = frame_width(690.0);
        let height = COLLAPSED_FRAME_HEIGHT;
        for preference in [
            PanelPositionPreference::Remember,
            PanelPositionPreference::Center,
            PanelPositionPreference::Top,
            PanelPositionPreference::Bottom,
            PanelPositionPreference::Left,
            PanelPositionPreference::Right,
        ] {
            let (x, y) = preferred_position(preference, width, height, WORK);
            let clamped = fit_into(Rect::new(x, y, width, height), WORK);
            assert_eq!((clamped.x, clamped.y), (x, y), "{preference:?} was clamped");
        }
        let (x, y) = preferred_position(PanelPositionPreference::Bottom, width, height, WORK);
        assert_eq!(x, (WORK.width - width) / 2.0);
        assert_eq!(y, WORK.y + WORK.height - height - 24.0);
    }

    #[test]
    fn launch_frame_and_constraints_match_the_native_config() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../../../tauri.conf.json")).unwrap();
        let main = &config["app"]["windows"][0];
        assert_eq!(main["label"], "main");
        assert_eq!(main["width"].as_f64(), Some(frame_width(690.0)));
        assert_eq!(main["height"].as_f64(), Some(COLLAPSED_FRAME_HEIGHT));
        assert_eq!(main["minWidth"].as_f64(), Some(MIN_FRAME_WIDTH));
        assert_eq!(main["minHeight"].as_f64(), Some(COLLAPSED_FRAME_HEIGHT));
        assert_eq!(main["resizable"].as_bool(), Some(false));
        let defaults = bluey_core::types::PanelState::default();
        assert_eq!(defaults.width, frame_width(690.0));
        assert_eq!(defaults.height, COLLAPSED_FRAME_HEIGHT);
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
