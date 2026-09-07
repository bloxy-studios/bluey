/**
 * Token budget allocator (spec §77).
 *
 * Priorities: the current question is always kept; recent transcript, OCR and
 * the active UI are high; job description / resume are medium; old transcript
 * and session memory are low. We never blindly truncate from the end:
 * transcript is compressed by keeping the TAIL (most recent turns), OCR by
 * keeping the HEAD with an explicit truncation marker; everything else is
 * dropped lowest-relevance-first.
 */

import type { ContextItem, ContextSource, Settings } from "@/lib/types";
import { estimateTokens } from "./fusion";

export interface BudgetPolicy {
  /** Lower number = more important. */
  priorities: Record<ContextSource, number>;
  /** Below this an item is dropped instead of compressed. */
  minCompressedTokens: number;
}

export const DEFAULT_BUDGET_POLICY: BudgetPolicy = {
  priorities: {
    user_instruction: 0,
    personal_instructions: 1,
    transcript: 1,
    ocr: 1,
    accessibility: 1,
    screen: 2,
    job_description: 2,
    resume: 2,
    document: 2,
    session_memory: 3,
    transcript_old: 4,
  },
  minCompressedTokens: 24,
};

export interface BudgetAllocation {
  included: ContextItem[];
  dropped: ContextItem[];
  /** refs (or sources) of items whose content was compressed to fit. */
  compressed: string[];
  totalTokens: number;
  /** Human-readable note about what was omitted, for prompt transparency. */
  omittedNote?: string;
}

export const TRUNCATION_MARKER = "[… truncated]";

/** Keep the tail of a text within a token budget (used for transcript). */
export function compressKeepTail(content: string, maxTokens: number): string {
  if (estimateTokens(content) <= maxTokens) return content;
  const lines = content.split("\n");
  const kept: string[] = [];
  let tokens = estimateTokens(TRUNCATION_MARKER);
  for (let i = lines.length - 1; i >= 0; i -= 1) {
    const line = lines[i] ?? "";
    const cost = estimateTokens(line) + 1;
    if (tokens + cost > maxTokens) break;
    kept.unshift(line);
    tokens += cost;
  }
  if (kept.length === 0) {
    // Single long line: keep the last ~maxTokens*4 chars.
    const chars = Math.max(0, maxTokens - estimateTokens(TRUNCATION_MARKER)) * 4;
    return `${TRUNCATION_MARKER} ${content.slice(content.length - chars)}`;
  }
  return `${TRUNCATION_MARKER}\n${kept.join("\n")}`;
}

/** Keep the head of a text within a token budget with a marker (used for OCR). */
export function compressKeepHead(content: string, maxTokens: number): string {
  if (estimateTokens(content) <= maxTokens) return content;
  const lines = content.split("\n");
  const kept: string[] = [];
  let tokens = estimateTokens(TRUNCATION_MARKER);
  for (const line of lines) {
    const cost = estimateTokens(line) + 1;
    if (tokens + cost > maxTokens) break;
    kept.push(line);
    tokens += cost;
  }
  if (kept.length === 0) {
    const chars = Math.max(0, maxTokens - estimateTokens(TRUNCATION_MARKER)) * 4;
    return `${content.slice(0, chars)} ${TRUNCATION_MARKER}`;
  }
  return `${kept.join("\n")}\n${TRUNCATION_MARKER}`;
}

function withContent(item: ContextItem, content: string): ContextItem {
  return { ...item, content, tokens: estimateTokens(content) };
}

/**
 * Fit items into `budgetTokens`. The user instruction is always included
 * (compressed only if it alone exceeds the budget, keeping its head).
 */
export function allocateBudget(
  items: ContextItem[],
  budgetTokens: number,
  policy: BudgetPolicy = DEFAULT_BUDGET_POLICY,
): BudgetAllocation {
  const included: ContextItem[] = [];
  const dropped: ContextItem[] = [];
  const compressed: string[] = [];
  let remaining = Math.max(0, Math.floor(budgetTokens));

  const instructions = items.filter((i) => i.source === "user_instruction");
  const rest = items
    .filter((i) => i.source !== "user_instruction")
    .slice()
    .sort((a, b) => {
      const pa = policy.priorities[a.source];
      const pb = policy.priorities[b.source];
      if (pa !== pb) return pa - pb;
      return b.relevance - a.relevance;
    });

  for (const item of instructions) {
    if (item.tokens <= remaining) {
      included.push(item);
      remaining -= item.tokens;
    } else {
      const squeezed = withContent(item, compressKeepHead(item.content, Math.max(remaining, policy.minCompressedTokens)));
      included.push(squeezed);
      compressed.push(item.ref ?? item.source);
      remaining = Math.max(0, remaining - squeezed.tokens);
    }
  }

  for (const item of rest) {
    if (remaining <= 0) {
      dropped.push(item);
      continue;
    }
    if (item.tokens <= remaining) {
      included.push(item);
      remaining -= item.tokens;
      continue;
    }
    // Does not fit whole — compress transcript (tail) and OCR (head), drop others.
    const room = remaining;
    if (room >= policy.minCompressedTokens) {
      if (item.source === "transcript" || item.source === "transcript_old") {
        const squeezed = withContent(item, compressKeepTail(item.content, room));
        if (squeezed.tokens <= room && squeezed.content.length > TRUNCATION_MARKER.length) {
          included.push(squeezed);
          compressed.push(item.ref ?? item.source);
          remaining -= squeezed.tokens;
          continue;
        }
      } else if (item.source === "ocr") {
        const squeezed = withContent(item, compressKeepHead(item.content, room));
        if (squeezed.tokens <= room && squeezed.content.length > TRUNCATION_MARKER.length) {
          included.push(squeezed);
          compressed.push(item.ref ?? item.source);
          remaining -= squeezed.tokens;
          continue;
        }
      }
    }
    dropped.push(item);
  }

  const totalTokens = included.reduce((sum, item) => sum + item.tokens, 0);
  let omittedNote: string | undefined;
  if (dropped.length > 0) {
    const bySource = new Map<ContextSource, number>();
    for (const item of dropped) bySource.set(item.source, (bySource.get(item.source) ?? 0) + 1);
    const parts = Array.from(bySource.entries()).map(([source, count]) =>
      count > 1 ? `${count}× ${source.replace(/_/g, " ")}` : source.replace(/_/g, " "),
    );
    omittedNote = `Context omitted to fit the token budget: ${parts.join(", ")}.`;
  }

  return { included, dropped, compressed, totalTokens, omittedNote };
}

/** Response headroom reserved out of the configured context budget. */
export const DEFAULT_RESPONSE_HEADROOM_TOKENS = 1024;

/** Default input budget: configured context budget minus response headroom. */
export function defaultContextBudget(settings: Settings, responseHeadroomTokens?: number): number {
  const headroom = responseHeadroomTokens ?? DEFAULT_RESPONSE_HEADROOM_TOKENS;
  const configured = settings.ai.contextTokenBudget;
  return Math.max(512, configured - headroom);
}
