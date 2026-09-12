/**
 * The two ways a model reply fails to become an answer. Both are recoverable
 * by regenerating; the copy the user sees lives in `present.ts` (`CODE_COPY`).
 */

import type { BlueyError } from "@/lib/types";

/** The output budget ran out (twice — the engine retries once with double the room) before the answer finished. */
export function truncatedAnswerError(): BlueyError {
  return {
    kind: "ai",
    code: "ai.truncated",
    message: "The model ran out of room before finishing the answer.",
    recoverable: true,
    recovery: { type: "retry" },
  };
}

/** The reply was a JSON envelope Bluey could not read, or nothing at all — never shown raw. */
export function unreadableAnswerError(): BlueyError {
  return {
    kind: "ai",
    code: "ai.unreadable_output",
    message: "The model's reply was not an answer Bluey can show.",
    recoverable: true,
    recovery: { type: "retry" },
  };
}
