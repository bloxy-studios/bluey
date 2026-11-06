/**
 * Streaming layer over `bluey.ai.stream`.
 *
 * - `streamRequest` wires `AIChunk`s to handlers and returns {cancel, done}.
 * - `CodeFenceBuffer` exposes streamed text progressively, open code blocks
 *   included, holding back only fence markers that are still arriving.
 * - `extractPartialStringField` (from `lib/utils/partial-json`) pulls a
 *   growing `content` field out of a partially-streamed JSON structured output.
 */

import type { AIChunk, AIRequest, BlueyError, ModelSelection } from "@/lib/types";

// ── Code fence handling ─────────────────────────────────────────────────────

const FENCE_LINE = /^\s{0,3}```/;
const PARTIAL_FENCE = /^\s{0,3}`{1,2}$/;

/**
 * Compute the draft-safe prefix of `text`. An open ``` block streams through:
 * the HUD closes it temporarily (`splitStreamingMarkdown`), so a coding answer
 * whose `content` opens with its solution paints line by line instead of only
 * once the whole block is written. Held back are only fence markers still
 * arriving: a trailing partial fence ("`" / "``" at the very end) and an
 * opening fence line without its newline yet (it would render "```pyt" as the
 * language tag).
 */
export function draftSafeText(text: string): string {
  const lastNewline = text.lastIndexOf("\n");
  const head = text.slice(0, lastNewline + 1);
  const lastLine = text.slice(lastNewline + 1);
  if (PARTIAL_FENCE.test(lastLine)) return head;
  if (FENCE_LINE.test(lastLine)) {
    // An even count of fences before it means this line opens a block.
    const fencesBefore = head.split("\n").filter((line) => FENCE_LINE.test(line)).length;
    if (fencesBefore % 2 === 0) return head;
  }
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
    return draftSafeText(this.raw);
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

  // `ai_stream` returns once Rust has registered the request; a cancel sent
  // before that finds nothing to cancel (returns false) and would be lost, so
  // it is sent again once the command returns.
  let cancelRequested = false;
  const sendCancel = async (): Promise<void> => {
    try {
      await api.ai.cancel({ requestId: request.requestId });
    } catch {
      // Cancellation is best-effort; the gate protects against stale output.
    }
  };

  api.ai.stream(request, onChunk).then(
    () => {
      if (cancelRequested) void sendCancel();
    },
    (error: unknown) => {
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
    },
  );

  return {
    cancel: async () => {
      cancelRequested = true;
      await sendCancel();
      settle({ text, finishReason: "cancelled", selection, totalMs: clock() - startedAt });
    },
    done,
  };
}
