//! Global shortcut definitions, accelerator normalisation and conflict detection.
//!
//! Accelerators use the Tauri/global-hotkey string format: modifiers joined with `+`
//! (`CmdOrCtrl`, `Cmd`, `Ctrl`, `Alt`, `Shift`, `Super`) followed by a
//! `keyboard_types::Code` name such as `Backslash`, `Enter`, `ArrowUp`, `KeyR`, `Comma`.

use crate::types::settings::{
    ShortcutBinding, ShortcutConflict, ShortcutConflictKind, ShortcutGroup, ShortcutId,
};

/// Default bindings (spec §46 + the screenshot reference for New Chat / Settings / Scroll).
pub fn default_bindings() -> Vec<ShortcutBinding> {
    let mk = |id: ShortcutId, label: &str, group: ShortcutGroup, acc: &str| ShortcutBinding {
        id,
        label: label.to_string(),
        group,
        accelerator: acc.to_string(),
        default_accelerator: acc.to_string(),
        enabled: true,
    };
    vec![
        mk(
            ShortcutId::TogglePanel,
            "Toggle visibility of Bluey",
            ShortcutGroup::General,
            "CmdOrCtrl+Backslash",
        ),
        mk(
            ShortcutId::CaptureAnalyze,
            "Ask Bluey about your screen or audio",
            ShortcutGroup::General,
            "CmdOrCtrl+Enter",
        ),
        mk(
            ShortcutId::GenerateResponse,
            "Generate a suggested response",
            ShortcutGroup::General,
            "CmdOrCtrl+Shift+Enter",
        ),
        mk(
            ShortcutId::ToggleListening,
            "Start or stop listening",
            ShortcutGroup::General,
            "CmdOrCtrl+Shift+KeyL",
        ),
        mk(
            ShortcutId::NewChat,
            "Start a new chat",
            ShortcutGroup::General,
            "CmdOrCtrl+KeyR",
        ),
        mk(
            ShortcutId::OpenSettings,
            "Open Bluey settings",
            ShortcutGroup::General,
            "CmdOrCtrl+Comma",
        ),
        mk(
            ShortcutId::MoveUp,
            "Move the window position up",
            ShortcutGroup::Window,
            "CmdOrCtrl+ArrowUp",
        ),
        mk(
            ShortcutId::MoveDown,
            "Move the window position down",
            ShortcutGroup::Window,
            "CmdOrCtrl+ArrowDown",
        ),
        mk(
            ShortcutId::MoveLeft,
            "Move the window position left",
            ShortcutGroup::Window,
            "CmdOrCtrl+ArrowLeft",
        ),
        mk(
            ShortcutId::MoveRight,
            "Move the window position right",
            ShortcutGroup::Window,
            "CmdOrCtrl+ArrowRight",
        ),
        mk(
            ShortcutId::ScrollUp,
            "Scroll the response window up",
            ShortcutGroup::Scroll,
            "CmdOrCtrl+Shift+ArrowUp",
        ),
        mk(
            ShortcutId::ScrollDown,
            "Scroll the response window down",
            ShortcutGroup::Scroll,
            "CmdOrCtrl+Shift+ArrowDown",
        ),
    ]
}

/// Well-known macOS system shortcuts we should warn about (not exhaustive, best effort).
pub const KNOWN_SYSTEM_SHORTCUTS: &[(&str, &str)] = &[
    ("Cmd+Space", "Spotlight"),
    ("Cmd+Tab", "Application switcher"),
    ("Cmd+KeyQ", "Quit application"),
    ("Cmd+KeyH", "Hide application"),
    ("Cmd+KeyM", "Minimise window"),
    ("Cmd+KeyW", "Close window"),
    ("Cmd+Shift+Digit3", "Screenshot"),
    ("Cmd+Shift+Digit4", "Screenshot selection"),
    ("Cmd+Shift+Digit5", "Screenshot and recording"),
    ("Cmd+Ctrl+Space", "Emoji & Symbols"),
    ("Cmd+Ctrl+KeyQ", "Lock screen"),
    ("Cmd+Alt+Escape", "Force quit"),
    ("Cmd+Alt+KeyD", "Toggle Dock"),
    ("Ctrl+ArrowUp", "Mission Control"),
    ("Ctrl+ArrowDown", "Application windows"),
    ("Ctrl+ArrowLeft", "Previous Space"),
    ("Ctrl+ArrowRight", "Next Space"),
    ("Cmd+Shift+KeyA", "Applications folder (Finder)"),
];

/// Canonical form: modifiers sorted in a fixed order, key capitalised as a `Code` name.
/// Returns `None` when the string is not a valid accelerator.
pub fn normalize_accelerator(input: &str) -> Option<String> {
    let parts: Vec<&str> = input
        .split('+')
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() {
        return None;
    }
    let mut cmd_or_ctrl = false;
    let mut cmd = false;
    let mut ctrl = false;
    let mut alt = false;
    let mut shift = false;
    let mut key: Option<String> = None;
    for part in parts {
        match part.to_ascii_lowercase().as_str() {
            "cmdorctrl" | "commandorcontrol" => cmd_or_ctrl = true,
            "cmd" | "command" | "super" | "meta" => cmd = true,
            "ctrl" | "control" => ctrl = true,
            "alt" | "option" => alt = true,
            "shift" => shift = true,
            _ => {
                if key.is_some() {
                    return None;
                }
                key = Some(normalize_key(part)?);
            }
        }
    }
    let key = key?;
    let mut out: Vec<&str> = Vec::new();
    if cmd_or_ctrl {
        out.push("CmdOrCtrl");
    }
    if cmd {
        out.push("Cmd");
    }
    if ctrl {
        out.push("Ctrl");
    }
    if alt {
        out.push("Alt");
    }
    if shift {
        out.push("Shift");
    }
    if out.is_empty() {
        // Global shortcuts without a modifier would hijack normal typing.
        return None;
    }
    let mut s = out.join("+");
    s.push('+');
    s.push_str(&key);
    Some(s)
}

fn normalize_key(part: &str) -> Option<String> {
    let lower = part.to_ascii_lowercase();
    let mapped = match lower.as_str() {
        "\\" | "backslash" => "Backslash",
        "enter" | "return" => "Enter",
        "space" => "Space",
        "," | "comma" => "Comma",
        "." | "period" => "Period",
        "/" | "slash" => "Slash",
        ";" | "semicolon" => "Semicolon",
        "'" | "quote" => "Quote",
        "[" | "bracketleft" => "BracketLeft",
        "]" | "bracketright" => "BracketRight",
        "-" | "minus" => "Minus",
        "=" | "equal" => "Equal",
        "`" | "backquote" => "Backquote",
        "tab" => "Tab",
        "escape" | "esc" => "Escape",
        "backspace" => "Backspace",
        "delete" => "Delete",
        "up" | "arrowup" => "ArrowUp",
        "down" | "arrowdown" => "ArrowDown",
        "left" | "arrowleft" => "ArrowLeft",
        "right" | "arrowright" => "ArrowRight",
        "home" => "Home",
        "end" => "End",
        "pageup" => "PageUp",
        "pagedown" => "PageDown",
        other => {
            // Single letter → KeyX, single digit → DigitN, F-keys, or already a Code name.
            if other.len() == 1 && other.chars().all(|c| c.is_ascii_alphabetic()) {
                return Some(format!("Key{}", other.to_ascii_uppercase()));
            }
            if other.len() == 1 && other.chars().all(|c| c.is_ascii_digit()) {
                return Some(format!("Digit{other}"));
            }
            if let Some(rest) = other.strip_prefix("key") {
                if rest.len() == 1 && rest.chars().all(|c| c.is_ascii_alphabetic()) {
                    return Some(format!("Key{}", rest.to_ascii_uppercase()));
                }
            }
            if let Some(rest) = other.strip_prefix("digit") {
                if rest.len() == 1 && rest.chars().all(|c| c.is_ascii_digit()) {
                    return Some(format!("Digit{rest}"));
                }
            }
            if let Some(rest) = other.strip_prefix('f') {
                if let Ok(n) = rest.parse::<u8>() {
                    if (1..=24).contains(&n) {
                        return Some(format!("F{n}"));
                    }
                }
            }
            return None;
        }
    };
    Some(mapped.to_string())
}

/// Human-readable key caps for the UI, e.g. `["⌘", "\\"]` — macOS glyphs.
pub fn display_keys(accelerator: &str) -> Vec<String> {
    accelerator
        .split('+')
        .map(|p| match p {
            "CmdOrCtrl" | "Cmd" | "Super" | "Meta" | "Command" => "⌘".to_string(),
            "Ctrl" | "Control" => "⌃".to_string(),
            "Alt" | "Option" => "⌥".to_string(),
            "Shift" => "⇧".to_string(),
            "Backslash" => "\\".to_string(),
            "Enter" => "↵".to_string(),
            "Comma" => ",".to_string(),
            "Period" => ".".to_string(),
            "Slash" => "/".to_string(),
            "Space" => "␣".to_string(),
            "ArrowUp" => "↑".to_string(),
            "ArrowDown" => "↓".to_string(),
            "ArrowLeft" => "←".to_string(),
            "ArrowRight" => "→".to_string(),
            "Escape" => "⎋".to_string(),
            "Tab" => "⇥".to_string(),
            "Backspace" => "⌫".to_string(),
            "Delete" => "⌦".to_string(),
            other => other
                .strip_prefix("Key")
                .or_else(|| other.strip_prefix("Digit"))
                .unwrap_or(other)
                .to_string(),
        })
        .collect()
}

/// Detect conflicts of `accelerator` against other Bluey bindings and known system shortcuts.
pub fn detect_conflict(
    accelerator: &str,
    bindings: &[ShortcutBinding],
    ignore: Option<ShortcutId>,
) -> Option<ShortcutConflict> {
    let normalized = normalize_accelerator(accelerator)?;
    // macOS-first: `CmdOrCtrl` resolves to `Cmd`, so compare with it folded.
    let as_cmd = normalized.replace("CmdOrCtrl", "Cmd");
    for b in bindings {
        if Some(b.id) == ignore || !b.enabled {
            continue;
        }
        let existing = normalize_accelerator(&b.accelerator).map(|n| n.replace("CmdOrCtrl", "Cmd"));
        if existing.as_deref() == Some(as_cmd.as_str()) {
            return Some(ShortcutConflict {
                accelerator: normalized,
                conflicts_with: ShortcutConflictKind::Bluey,
                detail: format!("Already used by “{}”", b.label),
            });
        }
    }
    // Compare with system shortcuts on macOS, treating CmdOrCtrl as Cmd.
    for (sys, name) in KNOWN_SYSTEM_SHORTCUTS {
        if normalize_accelerator(sys).as_deref() == Some(as_cmd.as_str()) {
            return Some(ShortcutConflict {
                accelerator: normalized,
                conflicts_with: ShortcutConflictKind::System,
                detail: format!("Reserved by macOS: {name}"),
            });
        }
    }
    None
}

/// Merge stored bindings with defaults (adds new ids, drops unknown ones, keeps user changes).
pub fn reconcile(stored: &[ShortcutBinding]) -> Vec<ShortcutBinding> {
    default_bindings()
        .into_iter()
        .map(|def| match stored.iter().find(|s| s.id == def.id) {
            Some(s) => ShortcutBinding {
                accelerator: normalize_accelerator(&s.accelerator)
                    .unwrap_or(def.default_accelerator.clone()),
                enabled: s.enabled,
                ..def
            },
            None => def,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid_and_unique() {
        let b = default_bindings();
        assert_eq!(b.len(), 12);
        let mut seen = std::collections::HashSet::new();
        for x in &b {
            let n = normalize_accelerator(&x.accelerator).expect("valid accelerator");
            assert!(
                seen.insert(n),
                "duplicate default accelerator {}",
                x.accelerator
            );
        }
    }

    #[test]
    fn normalizes_aliases() {
        assert_eq!(
            normalize_accelerator("command+\\").unwrap(),
            "Cmd+Backslash"
        );
        assert_eq!(
            normalize_accelerator("cmdorctrl + shift + l").unwrap(),
            "CmdOrCtrl+Shift+KeyL"
        );
        assert_eq!(normalize_accelerator("Cmd+Up").unwrap(), "Cmd+ArrowUp");
        assert_eq!(normalize_accelerator("Ctrl+Alt+f5").unwrap(), "Ctrl+Alt+F5");
        assert!(
            normalize_accelerator("Enter").is_none(),
            "modifier required"
        );
        assert!(
            normalize_accelerator("Cmd+Enter+KeyA").is_none(),
            "two keys invalid"
        );
        assert!(normalize_accelerator("Cmd+Foo").is_none());
    }

    #[test]
    fn detects_conflicts() {
        let b = default_bindings();
        let c = detect_conflict("Cmd+Enter", &b, None).unwrap();
        assert_eq!(c.conflicts_with, ShortcutConflictKind::Bluey);
        let c = detect_conflict("Cmd+Space", &b, None).unwrap();
        assert_eq!(c.conflicts_with, ShortcutConflictKind::System);
        assert!(detect_conflict("Cmd+Enter", &b, Some(ShortcutId::CaptureAnalyze)).is_none());
        assert!(detect_conflict("Cmd+Alt+KeyB", &b, None).is_none());
    }

    #[test]
    fn display_keys_use_mac_glyphs() {
        assert_eq!(display_keys("CmdOrCtrl+Backslash"), vec!["⌘", "\\"]);
        assert_eq!(display_keys("CmdOrCtrl+Shift+ArrowUp"), vec!["⌘", "⇧", "↑"]);
        assert_eq!(display_keys("CmdOrCtrl+KeyR"), vec!["⌘", "R"]);
    }

    #[test]
    fn reconcile_keeps_user_changes_and_adds_missing() {
        let mut stored = default_bindings();
        stored[0].accelerator = "Cmd+Alt+KeyB".into();
        stored.pop();
        let merged = reconcile(&stored);
        assert_eq!(merged.len(), 12);
        assert_eq!(merged[0].accelerator, "Cmd+Alt+KeyB");
        assert_eq!(merged[0].default_accelerator, "CmdOrCtrl+Backslash");
    }
}
