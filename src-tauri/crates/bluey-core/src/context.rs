//! Native context snapshot hygiene: bound the size of what OCR/accessibility/
//! transcript feed into prompts, dedupe overlap between OCR and accessibility
//! text, and tag the active window with an application adapter (browser, IDE,
//! meeting app, …) plus machine-readable hints. Adapters only *hint* — no
//! behaviour is hard-coded off them.

use serde_json::{Map, Value};

use crate::text::truncate_chars;
use crate::types::{ApplicationContext, ContextSnapshot, OcrContext, WindowContext};

/// Upper bounds applied by [`trim_snapshot`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotLimits {
    /// Max characters of OCR text (blocks are trimmed to the same budget).
    pub ocr_chars: usize,
    /// Max characters of accessibility `visible_text`.
    pub ax_visible_text_chars: usize,
    /// Max number of accessibility elements.
    pub ax_elements: usize,
    /// Max number of transcript segments (most recent kept).
    pub transcript_segments: usize,
    /// Max transcript window in seconds, ending at the newest segment.
    pub transcript_seconds: u32,
}

impl Default for SnapshotLimits {
    fn default() -> Self {
        Self {
            ocr_chars: 12_000,
            ax_visible_text_chars: 8_000,
            ax_elements: 150,
            transcript_segments: 200,
            transcript_seconds: 300,
        }
    }
}

/// Ellipsis marker used on trimmed text.
const ELLIPSIS: &str = "…";

/// Bound a snapshot's size: dedupe OCR lines that also appear in the
/// accessibility text, cap OCR/accessibility text and element counts, and keep
/// only the most recent transcript window. Never grows any field.
pub fn trim_snapshot(mut snapshot: ContextSnapshot, limits: &SnapshotLimits) -> ContextSnapshot {
    // 1. Dedupe OCR against accessibility (AX text is structured, keep it there).
    if let (Some(ocr), Some(ax)) = (snapshot.ocr.as_mut(), snapshot.accessibility.as_ref()) {
        dedupe_ocr_against(ocr, &ax.visible_text);
    }

    // 2. Bound OCR size.
    if let Some(ocr) = snapshot.ocr.as_mut() {
        ocr.text = truncate_chars(&ocr.text, limits.ocr_chars, ELLIPSIS);
        let mut chars = 0usize;
        ocr.blocks.retain(|block| {
            chars += block.text.chars().count();
            chars <= limits.ocr_chars
        });
    }

    // 3. Bound accessibility size.
    if let Some(ax) = snapshot.accessibility.as_mut() {
        if ax.visible_text.chars().count() > limits.ax_visible_text_chars {
            ax.visible_text =
                truncate_chars(&ax.visible_text, limits.ax_visible_text_chars, ELLIPSIS);
            ax.truncated = true;
        }
        if ax.elements.len() > limits.ax_elements {
            ax.elements.truncate(limits.ax_elements);
            ax.truncated = true;
        }
    }

    // 4. Keep the most recent transcript window.
    if let Some(transcript) = snapshot.transcript.as_mut() {
        let segments = &mut transcript.segments;
        if segments.len() > limits.transcript_segments {
            let cut = segments.len() - limits.transcript_segments;
            segments.drain(..cut);
        }
        if let Some(last_end) = segments.last().map(|s| s.end_time) {
            let window_ms = u64::from(limits.transcript_seconds) * 1_000;
            let cutoff = last_end.saturating_sub(window_ms);
            segments.retain(|s| s.end_time >= cutoff);
        }
        transcript.window_seconds = transcript.window_seconds.min(limits.transcript_seconds);
    }

    snapshot
}

/// Remove OCR text lines (and blocks) whose trimmed content also appears as a
/// line of the accessibility text — the AX version is more structured.
fn dedupe_ocr_against(ocr: &mut OcrContext, ax_text: &str) {
    let ax_lines: std::collections::HashSet<&str> = ax_text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if ax_lines.is_empty() {
        return;
    }
    let kept: Vec<&str> = ocr
        .text
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty() && !ax_lines.contains(trimmed)
        })
        .collect();
    ocr.text = kept.join("\n");
    ocr.blocks
        .retain(|block| !ax_lines.contains(block.text.trim()));
}

// ── Application adapters ─────────────────────────────────────────────────────

/// Adapter identifier for a generic, unrecognised application.
pub const ADAPTER_GENERIC: &str = "generic";

const BROWSER_BUNDLES: &[&str] = &[
    "com.google.Chrome",
    "com.apple.Safari",
    "org.mozilla.firefox",
    "company.thebrowser.Browser", // Arc
    "com.microsoft.edgemac",
    "com.brave.Browser",
];

const VSCODE_BUNDLES: &[&str] = &[
    "com.microsoft.VSCode",
    "com.todesktop.230313mzl4w4u92", // Cursor
];

const TERMINAL_BUNDLES: &[&str] = &[
    "com.apple.Terminal",
    "com.googlecode.iterm2",
    "dev.warp.Warp",
];

const TEAMS_BUNDLES: &[&str] = &["com.microsoft.teams2", "com.microsoft.teams"];

/// Classify the frontmost application into an adapter name plus hints.
///
/// Adapters: `generic`, `browser`, `vscode` (VS Code / Cursor / JetBrains
/// IDEs), `terminal`, `zoom`, `google-meet` (a browser whose window title
/// contains "Meet"), `teams`, `slack`. Hints (camelCase keys):
/// `meetingDetected: true` for zoom/google-meet/teams, `codingContext: true`
/// for vscode/terminal, `browserTitle` for browsers. Purely informational —
/// downstream code must treat them as hints, never as behaviour switches.
pub fn detect_adapter(
    app: Option<&ApplicationContext>,
    window: Option<&WindowContext>,
) -> (String, Map<String, Value>) {
    let bundle = app.and_then(|a| a.bundle_id.as_deref()).unwrap_or("");
    let title = window.and_then(|w| w.title.as_deref()).unwrap_or("");

    let adapter = if BROWSER_BUNDLES.contains(&bundle) {
        if title.contains("Meet") {
            "google-meet"
        } else {
            "browser"
        }
    } else if VSCODE_BUNDLES.contains(&bundle) || bundle.starts_with("com.jetbrains.") {
        "vscode"
    } else if TERMINAL_BUNDLES.contains(&bundle) {
        "terminal"
    } else if bundle == "us.zoom.xos" {
        "zoom"
    } else if TEAMS_BUNDLES.contains(&bundle) {
        "teams"
    } else if bundle == "com.tinyspeck.slackmacgap" {
        "slack"
    } else {
        ADAPTER_GENERIC
    };

    let mut hints = Map::new();
    if matches!(adapter, "zoom" | "google-meet" | "teams") {
        hints.insert("meetingDetected".into(), Value::Bool(true));
    }
    if matches!(adapter, "vscode" | "terminal") {
        hints.insert("codingContext".into(), Value::Bool(true));
    }
    if matches!(adapter, "browser" | "google-meet") && !title.is_empty() {
        hints.insert("browserTitle".into(), Value::String(title.to_string()));
    }
    (adapter.to_string(), hints)
}

/// Fill `snapshot.active_window.adapter`/`hints` from the active application
/// and window. Does nothing when the snapshot has neither.
pub fn apply_adapter(snapshot: &mut ContextSnapshot) {
    if snapshot.active_application.is_none() && snapshot.active_window.is_none() {
        return;
    }
    let (adapter, hints) = detect_adapter(
        snapshot.active_application.as_ref(),
        snapshot.active_window.as_ref(),
    );
    let window = snapshot
        .active_window
        .get_or_insert_with(WindowContext::default);
    window.adapter = Some(adapter);
    window.hints = if hints.is_empty() { None } else { Some(hints) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        AccessibilityContext, AudioSource, BoundingBox, OcrBlock, OcrLevel, TranscriptContext,
        TranscriptSegment,
    };
    use pretty_assertions::assert_eq;

    fn app(bundle: Option<&str>) -> ApplicationContext {
        ApplicationContext {
            name: "App".into(),
            bundle_id: bundle.map(String::from),
            pid: Some(1),
        }
    }

    fn window(title: &str) -> WindowContext {
        WindowContext {
            title: Some(title.into()),
            ..WindowContext::default()
        }
    }

    fn ocr(text: &str) -> OcrContext {
        OcrContext {
            blocks: text
                .lines()
                .map(|l| OcrBlock {
                    text: l.to_string(),
                    confidence: 0.9,
                    bounding_box: BoundingBox::default(),
                })
                .collect(),
            text: text.to_string(),
            level: OcrLevel::Fast,
            languages: vec!["en-US".into()],
            duration_ms: 3,
            frame_id: None,
        }
    }

    fn ax(visible_text: &str) -> AccessibilityContext {
        AccessibilityContext {
            application: app(None),
            window: None,
            focused_element: None,
            elements: Vec::new(),
            selected_text: None,
            visible_text: visible_text.to_string(),
            truncated: false,
            captured_at: crate::now_iso(),
        }
    }

    fn segment(id: u32, start_s: u64, end_s: u64) -> TranscriptSegment {
        TranscriptSegment {
            id: format!("seg_{id}"),
            session_id: None,
            speaker: None,
            speaker_confidence: None,
            source: AudioSource::Microphone,
            text: format!("segment {id}"),
            start_time: start_s * 1_000,
            end_time: end_s * 1_000,
            confidence: None,
            finalized: true,
            language: None,
            created_at: crate::now_iso(),
        }
    }

    #[test]
    fn trims_ocr_text_and_blocks() {
        let long = "x".repeat(20_000);
        let snapshot = ContextSnapshot {
            ocr: Some(ocr(&long)),
            ..ContextSnapshot::default()
        };
        let out = trim_snapshot(snapshot, &SnapshotLimits::default());
        let ocr = out.ocr.expect("ocr kept");
        assert_eq!(ocr.text.chars().count(), 12_000);
        assert!(ocr.text.ends_with('…'));
    }

    #[test]
    fn dedupes_ocr_lines_that_appear_in_ax_text() {
        let snapshot = ContextSnapshot {
            ocr: Some(ocr("shared line\nocr only line\nshared line 2")),
            accessibility: Some(ax("shared line\nshared line 2\nax only line")),
            ..ContextSnapshot::default()
        };
        let out = trim_snapshot(snapshot, &SnapshotLimits::default());
        let ocr = out.ocr.expect("ocr kept");
        assert_eq!(ocr.text, "ocr only line");
        assert_eq!(ocr.blocks.len(), 1);
        assert_eq!(ocr.blocks[0].text, "ocr only line");
    }

    #[test]
    fn caps_accessibility_text_and_elements() {
        let mut a = ax(&"y".repeat(10_000));
        a.elements = (0..300).map(|_| Default::default()).collect();
        let snapshot = ContextSnapshot {
            accessibility: Some(a),
            ..ContextSnapshot::default()
        };
        let out = trim_snapshot(snapshot, &SnapshotLimits::default());
        let a = out.accessibility.expect("ax kept");
        assert_eq!(a.visible_text.chars().count(), 8_000);
        assert_eq!(a.elements.len(), 150);
        assert!(a.truncated);
    }

    #[test]
    fn keeps_only_the_most_recent_transcript() {
        // 400 one-second segments: 0..400s.
        let segments: Vec<_> = (0..400)
            .map(|i| segment(i, u64::from(i), u64::from(i) + 1))
            .collect();
        let snapshot = ContextSnapshot {
            transcript: Some(TranscriptContext {
                segments,
                earlier_summary: None,
                window_seconds: 600,
            }),
            ..ContextSnapshot::default()
        };
        let out = trim_snapshot(snapshot, &SnapshotLimits::default());
        let t = out.transcript.expect("transcript kept");
        assert_eq!(t.segments.len(), 200, "segment cap applies");
        assert_eq!(
            t.segments.last().map(|s| s.id.clone()),
            Some("seg_399".into())
        );
        assert_eq!(
            t.segments.first().map(|s| s.id.clone()),
            Some("seg_200".into())
        );
        assert_eq!(t.window_seconds, 300);

        // Time window tighter than the count cap.
        let sparse: Vec<_> = vec![
            segment(1, 0, 10),
            segment(2, 500, 510),
            segment(3, 520, 530),
        ];
        let snapshot = ContextSnapshot {
            transcript: Some(TranscriptContext {
                segments: sparse,
                earlier_summary: None,
                window_seconds: 600,
            }),
            ..ContextSnapshot::default()
        };
        let out = trim_snapshot(snapshot, &SnapshotLimits::default());
        let t = out.transcript.expect("transcript kept");
        assert_eq!(
            t.segments.len(),
            2,
            "segment older than 300s window dropped"
        );
        assert_eq!(t.segments[0].id, "seg_2");
    }

    #[test]
    fn detects_adapters_by_bundle_id() {
        let cases: &[(&str, &str)] = &[
            ("com.google.Chrome", "browser"),
            ("com.apple.Safari", "browser"),
            ("org.mozilla.firefox", "browser"),
            ("company.thebrowser.Browser", "browser"),
            ("com.microsoft.edgemac", "browser"),
            ("com.brave.Browser", "browser"),
            ("com.microsoft.VSCode", "vscode"),
            ("com.todesktop.230313mzl4w4u92", "vscode"),
            ("com.jetbrains.intellij", "vscode"),
            ("com.apple.Terminal", "terminal"),
            ("com.googlecode.iterm2", "terminal"),
            ("dev.warp.Warp", "terminal"),
            ("us.zoom.xos", "zoom"),
            ("com.microsoft.teams2", "teams"),
            ("com.microsoft.teams", "teams"),
            ("com.tinyspeck.slackmacgap", "slack"),
            ("com.something.else", "generic"),
        ];
        for (bundle, expected) in cases {
            let (adapter, _) = detect_adapter(Some(&app(Some(bundle))), None);
            assert_eq!(&adapter, expected, "bundle {bundle}");
        }
        let (adapter, hints) = detect_adapter(None, None);
        assert_eq!(adapter, "generic");
        assert!(hints.is_empty());
    }

    #[test]
    fn browser_with_meet_title_is_google_meet() {
        let a = app(Some("com.google.Chrome"));
        let w = window("Weekly sync – Google Meet");
        let (adapter, hints) = detect_adapter(Some(&a), Some(&w));
        assert_eq!(adapter, "google-meet");
        assert_eq!(hints.get("meetingDetected"), Some(&Value::Bool(true)));
        assert_eq!(
            hints.get("browserTitle"),
            Some(&Value::String("Weekly sync – Google Meet".into()))
        );

        let w = window("Hacker News");
        let (adapter, hints) = detect_adapter(Some(&a), Some(&w));
        assert_eq!(adapter, "browser");
        assert!(hints.get("meetingDetected").is_none());
        assert_eq!(
            hints.get("browserTitle"),
            Some(&Value::String("Hacker News".into()))
        );
    }

    #[test]
    fn coding_and_meeting_hints() {
        let (_, hints) = detect_adapter(Some(&app(Some("com.microsoft.VSCode"))), None);
        assert_eq!(hints.get("codingContext"), Some(&Value::Bool(true)));
        let (_, hints) = detect_adapter(Some(&app(Some("dev.warp.Warp"))), None);
        assert_eq!(hints.get("codingContext"), Some(&Value::Bool(true)));
        let (_, hints) = detect_adapter(Some(&app(Some("us.zoom.xos"))), None);
        assert_eq!(hints.get("meetingDetected"), Some(&Value::Bool(true)));
    }

    #[test]
    fn apply_adapter_fills_the_active_window() {
        let mut snapshot = ContextSnapshot {
            active_application: Some(app(Some("us.zoom.xos"))),
            ..ContextSnapshot::default()
        };
        apply_adapter(&mut snapshot);
        let w = snapshot.active_window.expect("window created");
        assert_eq!(w.adapter.as_deref(), Some("zoom"));
        assert_eq!(
            w.hints.expect("hints").get("meetingDetected"),
            Some(&Value::Bool(true))
        );

        let mut empty = ContextSnapshot::default();
        apply_adapter(&mut empty);
        assert!(
            empty.active_window.is_none(),
            "nothing to classify → untouched"
        );
    }
}
