/**
 * Reading string fields out of JSON that may be incomplete — a structured
 * output still streaming, or one cut off by the output budget. Shared by the
 * stream layer (drafts), the tolerant parser (salvage) and the HUD (the
 * never-render-JSON guard); it imports nothing.
 */

const UNESCAPES: Record<string, string> = {
  n: "\n",
  t: "\t",
  r: "\r",
  b: "\b",
  f: "\f",
  '"': '"',
  "\\": "\\",
  "/": "/",
};

export interface ScannedStringField {
  /** The unescaped value read so far. */
  value: string;
  /** True when the closing quote was reached. */
  complete: boolean;
}

/**
 * Scan a string field's value out of (possibly partial) JSON text. Returns
 * null when the field has not started yet.
 */
export function scanStringField(text: string, field: string): ScannedStringField | null {
  const marker = text.match(new RegExp(`"${field}"\\s*:\\s*"`));
  if (!marker || marker.index === undefined) return null;
  let out = "";
  let i = marker.index + marker[0].length;
  while (i < text.length) {
    const ch = text[i];
    if (ch === undefined) break;
    if (ch === "\\") {
      const next = text[i + 1];
      if (next === undefined) break; // escape split across deltas — wait
      if (next === "u") {
        const hex = text.slice(i + 2, i + 6);
        if (hex.length < 4) break;
        const code = Number.parseInt(hex, 16);
        if (!Number.isNaN(code)) out += String.fromCharCode(code);
        i += 6;
        continue;
      }
      out += UNESCAPES[next] ?? next;
      i += 2;
      continue;
    }
    if (ch === '"') return { value: out, complete: true };
    out += ch;
    i += 1;
  }
  return { value: out, complete: false };
}

/**
 * Extract a string field's (possibly incomplete) value from partial JSON.
 * Returns null when the field has not started streaming yet.
 */
export function extractPartialStringField(text: string, field = "content"): string | null {
  return scanStringField(text, field)?.value ?? null;
}

/** A string field's value only when its closing quote was reached; null otherwise. */
export function extractCompleteStringField(text: string, field: string): string | null {
  const scanned = scanStringField(text, field);
  return scanned?.complete ? scanned.value : null;
}

const STRUCTURED_FIELD = /"(responseType|content|sections|title)"\s*:/;
const WRAPPING_FENCE = /^```[a-zA-Z0-9_-]*\s*\n/;

/**
 * Whether the text is — or was meant to be — one of Bluey's JSON envelopes:
 * it opens with `{` (a wrapping fence allowed) and names one of the envelope
 * fields. Such text is never shown to the user as-is.
 */
export function looksLikeStructuredJson(text: string): boolean {
  const trimmed = text.trim().replace(WRAPPING_FENCE, "").trimStart();
  return trimmed.startsWith("{") && STRUCTURED_FIELD.test(trimmed);
}
