/**
 * Untrusted context (OCR, accessibility text, transcript, heard questions,
 * documents, web pages) is fenced in per-request nonce blocks and
 * neutralised so it can't forge the prompt's own structure (SEC-009): a
 * `### heading`, a `Task:` / `Shape:` / `Voice:` / `My question:` line, a
 * closing `</context …>` tag or a `<system-reminder>` look-alike.
 */

/** Prefix for an untrusted line that would otherwise read as prompt structure. */
const QUOTE_MARK = "│ ";

/** Lines that open with prompt structure: a heading or one of the task-area labels. */
const STRUCTURE_LINE = /^(\s*)(#|task:|shape:|voice:|my question:)/i;

/** `<context`, `</context`, `<system-reminder`, `<\system…` and the like. */
const TAG_LOOKALIKE = /<(?=\s*[/\\]?\s*(?:context|system)\b)/gi;

/** Fullwidth less-than: reads the same to the model, never parses as a tag. */
const DEFANGED_LT = "＜";

export function neutralizeUntrusted(text: string): string {
  return text
    .replace(TAG_LOOKALIKE, DEFANGED_LT)
    .split("\n")
    .map((line) => (STRUCTURE_LINE.test(line) ? `${QUOTE_MARK}${line}` : line))
    .join("\n");
}

/** Eight hex characters from the platform CSPRNG, fresh for every request. */
export function newContextNonce(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(4));
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
}

/** One untrusted section, fenced so only the closing tag with this request's nonce ends it. */
export function contextBlock(label: string, nonce: string, body: string): string {
  return `<context source="${label}" id="${nonce}">\n${neutralizeUntrusted(body)}\n</context id="${nonce}">`;
}
