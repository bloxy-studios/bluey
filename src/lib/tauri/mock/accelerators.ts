/**
 * Mirror of `bluey_core::shortcuts` for the mock transport: accelerator
 * normalisation and conflict detection, kept in step with the Rust rules so
 * UI tests see what the real app would store and warn about (UX-001).
 */
import type { ShortcutBinding, ShortcutConflict, ShortcutId } from "@/lib/types";

const MODIFIERS: Record<string, string> = {
  cmdorctrl: "CmdOrCtrl",
  commandorcontrol: "CmdOrCtrl",
  cmd: "Cmd",
  command: "Cmd",
  super: "Cmd",
  meta: "Cmd",
  ctrl: "Ctrl",
  control: "Ctrl",
  alt: "Alt",
  option: "Alt",
  shift: "Shift",
};
const MODIFIER_ORDER = ["CmdOrCtrl", "Cmd", "Ctrl", "Alt", "Shift"];

const NAMED_KEYS = [
  "Backslash", "Enter", "Space", "Comma", "Period", "Slash", "Semicolon", "Quote", "BracketLeft",
  "BracketRight", "Minus", "Equal", "Backquote", "Tab", "Escape", "Backspace", "Delete", "ArrowUp",
  "ArrowDown", "ArrowLeft", "ArrowRight", "Home", "End", "PageUp", "PageDown",
];
const KEYS: Record<string, string> = {
  ...Object.fromEntries(NAMED_KEYS.map((k) => [k.toLowerCase(), k])),
  "\\": "Backslash",
  return: "Enter",
  ",": "Comma",
  ".": "Period",
  "/": "Slash",
  ";": "Semicolon",
  "'": "Quote",
  "[": "BracketLeft",
  "]": "BracketRight",
  "-": "Minus",
  "=": "Equal",
  "`": "Backquote",
  esc: "Escape",
  up: "ArrowUp",
  down: "ArrowDown",
  left: "ArrowLeft",
  right: "ArrowRight",
};

function normalizeKey(part: string): string | null {
  const lower = part.toLowerCase();
  if (KEYS[lower]) return KEYS[lower];
  if (/^[a-z]$/.test(lower)) return `Key${lower.toUpperCase()}`;
  if (/^[0-9]$/.test(lower)) return `Digit${lower}`;
  if (/^key[a-z]$/.test(lower)) return `Key${lower.slice(3).toUpperCase()}`;
  if (/^digit[0-9]$/.test(lower)) return `Digit${lower.slice(5)}`;
  if (/^f([1-9]|1[0-9]|2[0-4])$/.test(lower)) return lower.toUpperCase();
  return null;
}

/** Canonical accelerator (`normalize_accelerator`), or null when invalid. */
export function normalizeAccelerator(input: string): string | null {
  const parts = input
    .split("+")
    .map((p) => p.trim())
    .filter(Boolean);
  const modifiers = new Set<string>();
  let key: string | null = null;
  for (const part of parts) {
    const modifier = MODIFIERS[part.toLowerCase()];
    if (modifier) {
      modifiers.add(modifier);
      continue;
    }
    if (key) return null;
    key = normalizeKey(part);
    if (!key) return null;
  }
  if (!key || modifiers.size === 0) return null;
  return [...MODIFIER_ORDER.filter((m) => modifiers.has(m)), key].join("+");
}

/** `KNOWN_SYSTEM_SHORTCUTS` (same entries, same order). */
export const KNOWN_SYSTEM_SHORTCUTS: ReadonlyArray<readonly [string, string]> = [
  ["Cmd+Space", "Spotlight"],
  ["Cmd+Tab", "Application switcher"],
  ["Cmd+KeyQ", "Quit application"],
  ["Cmd+KeyH", "Hide application"],
  ["Cmd+KeyM", "Minimise window"],
  ["Cmd+KeyW", "Close window"],
  ["Cmd+Shift+Digit3", "Screenshot"],
  ["Cmd+Shift+Digit4", "Screenshot selection"],
  ["Cmd+Shift+Digit5", "Screenshot and recording"],
  ["Cmd+Ctrl+Space", "Emoji & Symbols"],
  ["Cmd+Ctrl+KeyQ", "Lock screen"],
  ["Cmd+Alt+Escape", "Force quit"],
  ["Cmd+Alt+KeyD", "Toggle Dock"],
  ["Ctrl+ArrowUp", "Mission Control"],
  ["Ctrl+ArrowDown", "Application windows"],
  ["Ctrl+ArrowLeft", "Previous Space"],
  ["Ctrl+ArrowRight", "Next Space"],
  ["Cmd+Shift+KeyA", "Applications folder (Finder)"],
  ["Cmd+ArrowLeft", "Move to the start of the line"],
  ["Cmd+ArrowRight", "Move to the end of the line"],
  ["Cmd+ArrowUp", "Move to the start of the document"],
  ["Cmd+ArrowDown", "Move to the end of the document"],
  ["Cmd+Shift+ArrowLeft", "Select to the start of the line"],
  ["Cmd+Shift+ArrowRight", "Select to the end of the line"],
  ["Cmd+Shift+ArrowUp", "Select to the start of the document"],
  ["Cmd+Shift+ArrowDown", "Select to the end of the document"],
  ["Alt+ArrowLeft", "Move to the previous word"],
  ["Alt+ArrowRight", "Move to the next word"],
  ["Alt+Shift+ArrowLeft", "Select the previous word"],
  ["Alt+Shift+ArrowRight", "Select the next word"],
  ["Cmd+KeyA", "Select all"],
  ["Cmd+KeyC", "Copy"],
  ["Cmd+KeyV", "Paste"],
  ["Cmd+KeyX", "Cut"],
  ["Cmd+KeyZ", "Undo"],
  ["Cmd+Shift+KeyZ", "Redo"],
  ["Cmd+KeyF", "Find"],
  ["Cmd+KeyS", "Save"],
  ["Cmd+KeyN", "New window"],
  ["Cmd+KeyT", "New tab"],
  ["Cmd+KeyR", "Reload"],
  ["Cmd+Comma", "App settings"],
];

const asCmd = (accelerator: string | null) => accelerator?.replace("CmdOrCtrl", "Cmd") ?? null;

/** `detect_conflict`: other enabled Bluey bindings first, then macOS shortcuts. */
export function detectConflict(
  accelerator: string,
  bindings: readonly ShortcutBinding[],
  ignore?: ShortcutId | null,
): ShortcutConflict | null {
  const normalized = normalizeAccelerator(accelerator);
  if (!normalized) return null;
  const target = asCmd(normalized);
  const clash = bindings.find(
    (b) => b.id !== ignore && b.enabled && asCmd(normalizeAccelerator(b.accelerator)) === target,
  );
  if (clash) {
    return { accelerator: normalized, conflictsWith: "bluey", detail: `Already used by “${clash.label}”` };
  }
  const system = KNOWN_SYSTEM_SHORTCUTS.find(([sys]) => asCmd(normalizeAccelerator(sys)) === target);
  if (system) {
    return { accelerator: normalized, conflictsWith: "system", detail: `Reserved by macOS: ${system[1]}` };
  }
  return null;
}
