/**
 * Streaming layer over `bluey.ai.stream`.
 *
 * - `streamRequest` wires `AIChunk`s to handlers and returns {cancel, done}.
 * - `CodeFenceBuffer` withholds an unterminated fenced code block from drafts
 *   until the closing fence arrives (spec §80) while exposing plain text
 *   progressively.
 * - `extractPartialStringField` (from `lib/utils/partial-json`) pulls a
 *   growing `content` field out of a partially-streamed JSON structured output.
 */

import type { AIChunk, AIRequest, BlueyError, ModelSelection } from "@/lib/types";

// ── Code fence handling ─────────────────────────────────────────────────────

/**
 * Compute the draft-safe prefix of `text`: everything before an unterminated
 * ``` fence (the open fence line itself is withheld), and without a trailing
 * partial fence ("`" / "``" at the very end of the text).
 */
export function visibleWithHeldFences(text: string): string {
  let offset = 0;
  let open = false;
  let openStart = -1;
  const lines = text.split("\n");
  for (let i = 0; i < lines.length; i += 1) {
    const line = lines[i] ?? "";
    if (/^\s{0,3}```/.test(line)) {
      // A fence line only counts once fully terminated by a newline, except
      // a closing fence which may legitimately end the text.
      const isLastLine = i === lines.length - 1;
      if (!open) {
        open = true;
        openStart = offset;
      } else if (!isLastLine || /^\s{0,3}```\s*$/.test(line)) {
        open = false;
        openStart = -1;
      }
    }
    offset += line.length + 1;
  }
  if (open && openStart >= 0) return text.slice(0, openStart);

  // Withhold a trailing partial fence marker (1-2 backticks on their own).
  const lastNewline = text.lastIndexOf("\n");
  const lastLine = text.slice(lastNewline + 1);
  if (/^\s{0,3}`{1,2}$/.test(lastLine)) return text.slice(0, lastNewline + 1);
  return text;
}

/** Accumulates streamed text; `push` returns the currently draft-safe text. */
export class CodeFenceBuffer {
  private raw = "";

  push(delta: string): string {
    this.raw += delta;
    return this.visible();
  }

  visible(): string {
    return visibleWithHeldFences(this.raw);
  }

  /** Full raw text (used once the stream completes). */
  flush(): string {
    return this.raw;
  }

  get rawText(): string {
    return this.raw;
  }
}

// ── Partial structured output ───────────────────────────────────────────────

// The scanner lives in `src/lib/utils/partial-json.ts` so the tolerant parser
// and the HUD can share it; re-exported here for the stream layer's callers.
export { extractPartialStringField } from "@/lib/utils/partial-json";

// ── Stream driver ───────────────────────────────────────────────────────────

export interface StreamHandlers {
  onStarted?(selection: ModelSelection): void;
  /** Called per delta with the delta and the full accumulated raw text. */
  onDelta?(delta: string, accumulated: string): void;
  onUsage?(usage: { inputTokens?: number; outputTokens?: number }): void;
}

export interface StreamOutcome {
  text: string;
  finishReason: "stop" | "length" | "cancelled" | "error";
  selection?: ModelSelection;
  error?: BlueyError;
  inputTokens?: number;
  outputTokens?: number;
  timeToFirstTokenMs?: number;
  totalMs?: number;
}

export interface StreamApi {
  ai: {
    stream(request: AIRequest, onChunk: (chunk: AIChunk) => void): Promise<void>;
    cancel(args: { requestId: string }): Promise<boolean>;
  };
}

export interface StreamHandle {
  cancel(): Promise<void>;
  done: Promise<StreamOutcome>;
}

/**
 * Drive one streaming request. `done` always resolves (never rejects): a
 * failed stream resolves with finishReason "error" and the `BlueyError`.
 * Timing (time-to-first-token, total) is measured locally as a fallback when
 * the backend omits it from the `completed` chunk.
 */
export function streamRequest(
  request: AIRequest,
  handlers: StreamHandlers = {},
  api: StreamApi,
  clock: () => number = () => Date.now(),
): StreamHandle {
  let text = "";
  let selection: ModelSelection | undefined;
  let inputTokens: number | undefined;
  let outputTokens: number | undefined;
  let firstTokenAt: number | undefined;
  const startedAt = clock();

  let settle: (outcome: StreamOutcome) => void;
  let settled = false;
  const done = new Promise<StreamOutcome>((resolve) => {
    settle = (outcome) => {
      if (settled) return;
      settled = true;
      resolve(outcome);
    };
  });

  const onChunk = (chunk: AIChunk): void => {
    if (chunk.requestId !== request.requestId) return;
    switch (chunk.type) {
      case "started":
        selection = chunk.selection;
        handlers.onStarted?.(chunk.selection);
        break;
      case "delta":
        if (firstTokenAt === undefined) firstTokenAt = clock();
        text += chunk.text;
        handlers.onDelta?.(chunk.text, text);
        break;
      case "usage":
        inputTokens = chunk.inputTokens ?? inputTokens;
        outputTokens = chunk.outputTokens ?? outputTokens;
        handlers.onUsage?.({ inputTokens: chunk.inputTokens, outputTokens: chunk.outputTokens });
        break;
      case "completed":
        settle({
          text,
          finishReason: chunk.finishReason,
          selection,
          inputTokens,
          outputTokens,
          timeToFirstTokenMs:
            chunk.timeToFirstTokenMs ??
            (firstTokenAt !== undefined ? firstTokenAt - startedAt : undefined),
          totalMs: chunk.totalMs,
        });
        break;
      case "failed":
        settle({
          text,
          finishReason: "error",
          selection,
          error: chunk.error,
          inputTokens,
          outputTokens,
          totalMs: clock() - startedAt,
        });
        break;
    }
  };

  api.ai.stream(request, onChunk).catch((error: unknown) => {
    settle({
      text,
      finishReason: "error",
      selection,
      error: {
        kind: "ai",
        code: "ai.stream_failed",
        message: error instanceof Error ? error.message : "AI stream failed",
        recoverable: true,
        recovery: { type: "retry" },
      },
      totalMs: clock() - startedAt,
    });
  });

  return {
    cancel: async () => {
      try {
        await api.ai.cancel({ requestId: request.requestId });
      } catch {
        // Cancellation is best-effort; the gate protects against stale output.
      }
      settle({ text, finishReason: "cancelled", selection, totalMs: clock() - startedAt });
    },
    done,
  };
}
