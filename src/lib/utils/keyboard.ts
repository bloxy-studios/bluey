/**
 * Accelerator utilities: Tauri accelerator strings ("CmdOrCtrl+Shift+Enter")
 * ↔ macOS keycap glyphs (⌘ ⇧ ↵) ↔ KeyboardEvents (for click-to-record).
 */

const MODIFIER_ORDER = ["CmdOrCtrl", "Cmd", "Ctrl", "Alt", "Shift"] as const;

const MODIFIER_GLYPHS: Record<string, string> = {
  CmdOrCtrl: "⌘",
  Cmd: "⌘",
  Super: "⌘",
  Meta: "⌘",
  Ctrl: "⌃",
  Control: "⌃",
  Alt: "⌥",
  Option: "⌥",
  Shift: "⇧",
};

const KEY_GLYPHS: Record<string, string> = {
  Enter: "↵",
  Return: "↵",
  Up: "↑",
  Down: "↓",
  Left: "←",
  Right: "→",
  Backslash: "\\",
  Comma: ",",
  Period: ".",
  Slash: "/",
  Space: "␣",
  Tab: "⇥",
  Backspace: "⌫",
  Delete: "⌦",
  Escape: "⎋",
  Plus: "+",
  Minus: "-",
  Equal: "=",
  Semicolon: ";",
  Quote: "'",
  BracketLeft: "[",
  BracketRight: "]",
  Backquote: "`",
};

function isModifier(part: string): boolean {
  return part in MODIFIER_GLYPHS;
}

/** "CmdOrCtrl+Shift+Enter" → ["⌘", "⇧", "↵"] (modifiers first, canonical order). */
export function acceleratorToGlyphs(accelerator: string): string[] {
  const parts = accelerator.split("+").filter(Boolean);
  const modifiers = parts.filter(isModifier);
  const keys = parts.filter((p) => !isModifier(p));
  modifiers.sort((a, b) => MODIFIER_ORDER.indexOf(a as (typeof MODIFIER_ORDER)[number]) - MODIFIER_ORDER.indexOf(b as (typeof MODIFIER_ORDER)[number]));
  return [
    ...modifiers.map((m) => MODIFIER_GLYPHS[m] ?? m),
    ...keys.map((k) => KEY_GLYPHS[k] ?? (k.length === 1 ? k.toUpperCase() : k)),
  ];
}

/** Human string for tooltips: "⌘ ⇧ ↵". */
export function acceleratorLabel(accelerator: string): string {
  return acceleratorToGlyphs(accelerator).join(" ");
}

const EVENT_KEY_TO_ACCEL: Record<string, string> = {
  Enter: "Enter",
  ArrowUp: "Up",
  ArrowDown: "Down",
  ArrowLeft: "Left",
  ArrowRight: "Right",
  "\\": "Backslash",
  ",": "Comma",
  ".": "Period",
  "/": "Slash",
  " ": "Space",
  Tab: "Tab",
  Backspace: "Backspace",
  Delete: "Delete",
  "+": "Plus",
  "-": "Minus",
  "=": "Equal",
  ";": "Semicolon",
  "'": "Quote",
  "[": "BracketLeft",
  "]": "BracketRight",
  "`": "Backquote",
};

const IGNORED_KEYS = new Set(["Meta", "Control", "Alt", "Shift", "CapsLock", "Fn", "Escape", "Dead"]);

/**
 * Build a Tauri accelerator from a KeyboardEvent while recording.
 * Returns null while only modifiers are held (recording continues).
 */
export function eventToAccelerator(event: Pick<KeyboardEvent, "key" | "metaKey" | "ctrlKey" | "altKey" | "shiftKey">): string | null {
  if (IGNORED_KEYS.has(event.key)) return null;

  const parts: string[] = [];
  if (event.metaKey || event.ctrlKey) parts.push("CmdOrCtrl");
  if (event.altKey) parts.push("Alt");
  if (event.shiftKey) parts.push("Shift");

  let key = EVENT_KEY_TO_ACCEL[event.key];
  if (!key) {
    if (event.key.length === 1) {
      key = event.key.toUpperCase();
      // Single printable char without any modifier is not a valid global shortcut.
      if (parts.length === 0) return null;
    } else if (/^F\d{1,2}$/.test(event.key)) {
      key = event.key;
    } else {
      return null;
    }
  } else if (parts.length === 0) {
    return null;
  }

  return [...parts, key].join("+");
}
