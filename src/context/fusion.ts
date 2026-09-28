/**
 * Context Fusion: turns a `ContextSnapshot` into scored `ContextItem`s.
 *
 * Every source gets a relevance score in 0..1 relative to the current ask:
 *   - user instruction / detected question: 1.0 (always the anchor)
 *   - conversation (earlier turns of this chat): recent turns high, older lower
 *   - transcript: recent segments decay with age; questions get a boost
 *   - OCR: keyword overlap with the instruction/question + code/question markers
 *   - active app/window: one compact identity line alongside screen context
 *   - accessibility: focused element / selected text are high-value; the
 *     window text drops lines already in the OCR or the focused value
 *   - retrieved chunks: use their retrieval score
 *   - session memory (recent responses, the session's notes): moderate
 *
 * A typed ask with no screen cue and no keyword in common with the screen
 * drops the screen text and all but the last two transcript turns.
 *   - personal instructions: 0.9
 */

import type {
  ContextItem,
  ContextSnapshot,
  ContextSource,
  ConversationTurn,
  DetectedEvent,
  RetrievedChunk,
  TranscriptSegment,
} from "@/lib/types";

/** Estimate tokens: ~4 chars/token for latin text, ~1 token per CJK codepoint. */
export function estimateTokens(text: string): number {
  if (text.length === 0) return 0;
  let cjk = 0;
  let other = 0;
  for (const ch of text) {
    const cp = ch.codePointAt(0) ?? 0;
    const isCjk =
      (cp >= 0x2e80 && cp <= 0x9fff) || // CJK radicals, Kana, unified ideographs
      (cp >= 0xac00 && cp <= 0xd7af) || // Hangul syllables
      (cp >= 0xf900 && cp <= 0xfaff) || // CJK compat ideographs
      (cp >= 0x20000 && cp <= 0x2ffff); // CJK extension B+
    if (isCjk) cjk += 1;
    else other += 1;
  }
  return cjk + Math.ceil(other / 4);
}

const QUESTION_LEADS =
  /\b(tell me about|walk me through|how would you|what is|what are|why did|why do|can you explain|describe|how do|how does|what would)\b/i;

/** True when a piece of transcript reads like a question. */
export function looksLikeQuestion(text: string): boolean {
  const trimmed = text.trim();
  if (trimmed.length === 0) return false;
  if (trimmed.includes("?")) return true;
  return QUESTION_LEADS.test(trimmed);
}

const STOP_WORDS = new Set([
  "the", "a", "an", "and", "or", "but", "of", "to", "in", "on", "for", "with", "is", "are",
  "was", "were", "be", "this", "that", "it", "as", "at", "by", "from", "you", "your", "my",
  "me", "we", "our", "i", "do", "does", "did", "how", "what", "when", "where", "who", "why",
  "would", "could", "should", "can", "about", "tell", "me", "us", "they", "them",
]);

/** Lowercased content words (stop words removed). */
export function keywordTokens(text: string): string[] {
  return text
    .toLowerCase()
    .split(/[^\p{L}\p{N}_]+/u)
    .filter((t) => t.length > 2 && !STOP_WORDS.has(t));
}

/** Fraction of query keywords present in `target` (0..1). */
export function keywordOverlap(query: string, target: string): number {
  const q = new Set(keywordTokens(query));
  if (q.size === 0) return 0;
  const t = new Set(keywordTokens(target));
  let hits = 0;
  for (const token of q) if (t.has(token)) hits += 1;
  return hits / q.size;
}

const CODE_MARKERS =
  /```|\bfunction\b|\bdef\b|\bclass\b|\breturn\b|=>|\bconst\b|\bimport\b|\bpublic\b|[{};]\s*$|\(\)|\bO\((?:n|1|log)/m;

function clamp01(value: number): number {
  return Math.min(1, Math.max(0, value));
}

export interface FuseOptions {
  instruction?: string;
  detectedEvent?: DetectedEvent;
  /** Reference "now" in ms since audio-session start for transcript decay; defaults to the newest segment end. */
  transcriptNowMs?: number;
  /** Segments older than this (seconds) become `transcript_old`. Default 120. */
  recentWindowSeconds?: number;
}

/** The question text the current ask centres on (instruction > detected event > last question heard). */
export function currentQuestionText(snapshot: ContextSnapshot, opts: FuseOptions): string {
  if (opts.instruction && opts.instruction.trim().length > 0) return opts.instruction.trim();
  if (opts.detectedEvent) return opts.detectedEvent.text;
  const segments = snapshot.transcript?.segments ?? [];
  for (let i = segments.length - 1; i >= 0; i -= 1) {
    const segment = segments[i];
    if (segment && looksLikeQuestion(segment.text)) return segment.text;
  }
  return "";
}

function transcriptItem(
  segment: TranscriptSegment,
  nowMs: number,
  recentWindowSeconds: number,
): ContextItem {
  const ageSeconds = Math.max(0, (nowMs - segment.endTime) / 1000);
  const old = ageSeconds > recentWindowSeconds;
  // Exponential decay with ~90s half-relevance; questions boosted.
  let relevance = (old ? 0.45 : 0.75) * Math.exp(-ageSeconds / 180);
  if (looksLikeQuestion(segment.text)) relevance = Math.min(0.95, relevance * 1.35 + 0.1);
  const speaker = segment.speaker ?? (segment.source === "microphone" ? "You" : "Speaker");
  return {
    source: old ? "transcript_old" : "transcript",
    content: `${speaker}: ${segment.text}`,
    relevance: clamp01(relevance),
    tokens: estimateTokens(`${speaker}: ${segment.text}`),
    ref: `segment:${segment.id}`,
    at: segment.startTime,
  };
}

/** Turns rendered in full (question, answer and the last turn's code). */
const FULL_TURNS = 2;
/** Older turns kept as one-line summaries. */
const SUMMARY_TURNS = 3;
const TURN_ANSWER_CHARS = 800;
const TURN_CODE_CHARS = 1500;
const TURN_SUMMARY_CHARS = 200;

function clip(text: string, max: number): string {
  const trimmed = text.trim();
  return trimmed.length > max ? `${trimmed.slice(0, max).trimEnd()}…` : trimmed;
}

const FENCED_CODE = /```([\w+#.-]*)\n([\s\S]*?)```/g;

/** The answer's prose and its code (the `code` field, else the last fenced block in the body). */
function splitAnswer(turn: ConversationTurn): { prose: string; code?: { language: string; code: string } } {
  if (turn.code && turn.code.code.trim().length > 0) {
    return { prose: turn.content, code: { language: turn.code.language, code: turn.code.code } };
  }
  const blocks = Array.from(turn.content.matchAll(FENCED_CODE));
  const last = blocks.at(-1);
  if (!last) return { prose: turn.content };
  const prose = turn.content.replace(FENCED_CODE, "").replace(/\n{3,}/g, "\n\n");
  return { prose, code: { language: last[1] ?? "", code: last[2] ?? "" } };
}

/**
 * Earlier turns of this chat, oldest first: the last two as `Q:`/`A:` (the
 * newest with its code block, capped), older ones as one-line summaries.
 */
function conversationItems(turns: ConversationTurn[]): ContextItem[] {
  const kept = turns.slice(-(FULL_TURNS + SUMMARY_TURNS));
  return kept.map((turn, index) => {
    const fromEnd = kept.length - index; // 1 = newest
    const question = turn.prompt?.trim() ? `Q: ${clip(turn.prompt, TURN_SUMMARY_CHARS)}\n` : "";
    let content: string;
    if (fromEnd <= FULL_TURNS) {
      const { prose, code } = splitAnswer(turn);
      content = `${question}A: ${clip(prose, TURN_ANSWER_CHARS)}`;
      if (fromEnd === 1 && code) {
        content += `\n\`\`\`${code.language}\n${clip(code.code, TURN_CODE_CHARS)}\n\`\`\``;
      }
    } else {
      const summary = turn.title?.trim() || splitAnswer(turn).prose;
      content = `${question}A (summary): ${clip(summary, TURN_SUMMARY_CHARS)}`;
    }
    const createdAt = Date.parse(turn.createdAt);
    return {
      source: "conversation",
      content,
      relevance: clamp01(fromEnd <= FULL_TURNS ? 0.9 - 0.05 * (fromEnd - 1) : 0.5),
      tokens: estimateTokens(content),
      ref: `response:${turn.id}`,
      ...(Number.isFinite(createdAt) ? { at: createdAt } : {}),
    };
  });
}

function chunkSource(chunk: RetrievedChunk): ContextSource {
  switch (chunk.documentKind) {
    case "resume":
    case "cv":
    case "bio":
    case "portfolio":
    case "skills":
    case "experience":
      return "resume";
    case "job_description":
    case "role_description":
    case "company_notes":
      return "job_description";
    default:
      return "document";
  }
}

function isQuestionSource(source: ContextSource): boolean {
  return source === "user_instruction" || source === "detected_question";
}

const WINDOW_TITLE_CHARS = 120;
const FOCUSED_VALUE_CHARS = 1500;
/** Window text that is less than this share new (vs OCR / focused value) is skipped. */
const MIN_NEW_WINDOW_TEXT = 0.2;

/** Compact "which app, which window" line (~20 tokens), only alongside screen context. */
function appIdentityItem(snapshot: ContextSnapshot): ContextItem | undefined {
  if (!snapshot.screen && !snapshot.ocr && !snapshot.accessibility) return undefined;
  const app = snapshot.activeApplication?.name.trim() ?? "";
  const title = snapshot.activeWindow?.title?.trim() ?? "";
  if (app.length === 0 && title.length === 0) return undefined;
  const adapter = snapshot.activeWindow?.adapter;
  const kind = adapter && adapter !== "generic" ? ` (${adapter})` : "";
  const content = [app && `App: ${app}${kind}`, title && `Window: ${clip(title, WINDOW_TITLE_CHARS)}`]
    .filter((part) => part.length > 0)
    .join(" — ");
  return { source: "active_app", content, relevance: 0.7, tokens: estimateTokens(content), ref: "app:active" };
}

function normalizeForMatch(text: string): string {
  return text.toLowerCase().replace(/\s+/g, " ").trim();
}

/**
 * The accessibility window text minus lines already present (whitespace- and
 * case-insensitively) in the OCR text or the focused value; `undefined` when
 * what is left is mostly a duplicate.
 */
export function novelWindowText(visible: string, seen: readonly string[]): string | undefined {
  const haystacks = seen.map(normalizeForMatch).filter((text) => text.length > 0);
  const kept = visible.split("\n").filter((line) => {
    const needle = normalizeForMatch(line);
    return needle.length > 0 && !haystacks.some((hay) => hay.includes(needle));
  });
  const text = kept.join("\n").trim();
  if (text.length === 0) return undefined;
  return text.length / visible.trim().length < MIN_NEW_WINDOW_TEXT ? undefined : text;
}

/** Words that point at the screen (or at what is being worked on): screen context stays. */
const SCREEN_CUES =
  /\b(this|that|these|those|it|here|above|below|screen|page|window|tab|code|error|bug|question|problem|task|solve|answer|fix|debug|solution|approach|complexity|optimi[sz]e|output|chart|graph|diagram|image|picture|slide|table)\b/i;
/** Most recent transcript turns kept when the floor applies. */
const FLOOR_TRANSCRIPT_TURNS = 2;
const FLOORED_SOURCES: ReadonlySet<ContextSource> = new Set(["ocr", "screen", "window_text", "transcript_old"]);

/**
 * Relevance floor for a typed ask that neither points at the screen nor
 * shares a keyword with it: the screen text and the older transcript cannot
 * help, so keep only the focused/selected UI and the last two transcript turns.
 */
function applyRelevanceFloor(items: ContextItem[], snapshot: ContextSnapshot, instruction: string): ContextItem[] {
  if (instruction.length === 0 || SCREEN_CUES.test(instruction) || keywordTokens(instruction).length === 0) {
    return items;
  }
  const ax = snapshot.accessibility;
  const screenText = [snapshot.ocr?.text, ax?.visibleText, ax?.selectedText, ax?.focusedElement?.value]
    .filter((text): text is string => typeof text === "string")
    .join("\n");
  if (screenText.trim().length === 0 || keywordOverlap(instruction, screenText) > 0) return items;
  const recentTurns = new Set(
    items
      .filter((item) => item.source === "transcript")
      .sort((a, b) => (b.at ?? 0) - (a.at ?? 0))
      .slice(0, FLOOR_TRANSCRIPT_TURNS),
  );
  return items.filter((item) =>
    item.source === "transcript" ? recentTurns.has(item) : !FLOORED_SOURCES.has(item.source),
  );
}

/**
 * Accessibility items: selected text and the focused element are high-signal
 * ("Focused UI"); the rest of the window's text is its own, de-duplicated
 * `window_text` item.
 */
function accessibilityItems(snapshot: ContextSnapshot, alreadyShown: readonly string[]): ContextItem[] {
  const ax = snapshot.accessibility;
  if (!ax) return [];
  const items: ContextItem[] = [];
  if (ax.selectedText && ax.selectedText.trim().length > 0) {
    items.push({
      source: "accessibility",
      content: `Selected text: ${ax.selectedText}`,
      relevance: 0.9,
      tokens: estimateTokens(ax.selectedText),
      ref: "ax:selected",
    });
  }
  const focused = ax.focusedElement;
  const value = focused?.value ? clip(focused.value, FOCUSED_VALUE_CHARS) : undefined;
  const label = [focused?.role, focused?.label ?? focused?.title, value]
    .filter((p): p is string => typeof p === "string" && p.length > 0)
    .join(" — ");
  if (label.length > 0) {
    const content = `Focused element: ${label}`;
    items.push({ source: "accessibility", content, relevance: 0.85, tokens: estimateTokens(content), ref: "ax:focused" });
  }
  const visible = ax.visibleText?.trim()
    ? novelWindowText(ax.visibleText, [...alreadyShown, focused?.value ?? ""])
    : undefined;
  if (visible) {
    items.push({ source: "window_text", content: visible, relevance: 0.55, tokens: estimateTokens(visible), ref: "ax:visible" });
  }
  return items;
}

/**
 * Fuse a snapshot into scored context items. Pure; ordering is highest
 * relevance first with the question (typed or heard) always at the front.
 */
export function fuseContext(snapshot: ContextSnapshot, opts: FuseOptions = {}): ContextItem[] {
  const items: ContextItem[] = [];
  const question = currentQuestionText(snapshot, opts);

  const instruction = opts.instruction?.trim() || snapshot.userInstruction?.trim() || "";
  const event = instruction.length === 0 ? opts.detectedEvent : undefined;
  if (instruction.length > 0) {
    items.push({
      source: "user_instruction",
      content: instruction,
      relevance: 1,
      tokens: estimateTokens(instruction),
      ref: "instruction",
    });
  } else if (event && event.text.trim().length > 0) {
    // A live-detected question: rendered on its own (as heard) instead of
    // being one transcript line among several recent questions.
    const content = `${event.speaker ?? "Speaker"}: ${event.text.trim()}`;
    items.push({
      source: "detected_question",
      content,
      relevance: 1,
      tokens: estimateTokens(content),
      ref: `event:${event.id}`,
    });
  }

  items.push(...conversationItems(snapshot.conversation ?? []));

  // Transcript — per segment so budget can drop the oldest first (keep the tail).
  const segments = snapshot.transcript?.segments ?? [];
  if (segments.length > 0) {
    const nowMs =
      opts.transcriptNowMs ?? segments.reduce((max, s) => Math.max(max, s.endTime), 0);
    const windowSeconds = opts.recentWindowSeconds ?? 120;
    const asked = new Set(event?.segmentIds ?? []);
    for (const segment of segments) {
      if (segment.text.trim().length === 0) continue;
      if (asked.has(segment.id)) continue; // already rendered as the detected question
      items.push(transcriptItem(segment, nowMs, windowSeconds));
    }
  }
  const earlier = snapshot.transcript?.earlierSummary;
  if (earlier && earlier.trim().length > 0) {
    items.push({
      source: "transcript_old",
      content: `Earlier (summary): ${earlier}`,
      relevance: 0.35,
      tokens: estimateTokens(earlier),
      ref: "transcript:earlier-summary",
    });
  }

  const identity = appIdentityItem(snapshot);
  if (identity) items.push(identity);

  // OCR — keyword overlap with the ask, plus code/question markers.
  const ocrText = snapshot.ocr?.text ?? "";
  if (ocrText.trim().length > 0) {
    const overlap = question.length > 0 ? keywordOverlap(question, ocrText) : 0;
    let relevance = 0.35 + 0.4 * overlap;
    if (CODE_MARKERS.test(ocrText)) relevance += 0.15;
    if (ocrText.includes("?")) relevance += 0.05;
    items.push({
      source: "ocr",
      content: ocrText,
      relevance: clamp01(Math.min(0.92, relevance)),
      tokens: estimateTokens(ocrText),
      ref: snapshot.ocr?.frameId ? `frame:${snapshot.ocr.frameId}` : "ocr",
    });
  }

  items.push(...accessibilityItems(snapshot, [ocrText, identity?.content ?? ""]));

  // Retrieved chunks — trust the retrieval score.
  for (const chunk of snapshot.userContext?.chunks ?? []) {
    items.push({
      source: chunkSource(chunk),
      content: `${chunk.documentTitle}: ${chunk.content}`,
      relevance: clamp01(chunk.score),
      tokens: estimateTokens(chunk.content),
      ref: `chunk:${chunk.chunkId}`,
    });
  }

  // Session memory — recent responses not already in the chat thread, newest slightly higher.
  const inThread = new Set((snapshot.conversation ?? []).map((turn) => turn.id));
  const recent = (snapshot.session?.recentResponses ?? []).filter((r) => !inThread.has(r.id));
  recent.forEach((response, index) => {
    const recency = (index + 1) / recent.length; // most recent last per contract
    const title = response.title ? `${response.title}: ` : "";
    items.push({
      source: "session_memory",
      content: `${title}${response.content}`,
      relevance: clamp01(0.4 + 0.2 * recency),
      tokens: estimateTokens(response.content),
      ref: `response:${response.id}`,
    });
  });

  const personal = snapshot.userContext?.personalInstructions;
  if (personal && personal.trim().length > 0) {
    items.push({
      source: "personal_instructions",
      content: personal,
      relevance: 0.9,
      tokens: estimateTokens(personal),
      ref: "personal-instructions",
    });
  }

  const notes = (snapshot.session?.notes ?? []).filter((note) => note.trim().length > 0);
  if (notes.length > 0) {
    const content = `Your notes for this session:\n${notes.map((note) => `- ${note}`).join("\n")}`;
    items.push({ source: "session_memory", content, relevance: 0.6, tokens: estimateTokens(content), ref: "session:notes" });
  }

  return applyRelevanceFloor(items, snapshot, instruction).sort((a, b) => {
    const qa = isQuestionSource(a.source);
    const qb = isQuestionSource(b.source);
    if (qa !== qb) return qa ? -1 : 1;
    return b.relevance - a.relevance;
  });
}
