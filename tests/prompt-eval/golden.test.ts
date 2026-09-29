/**
 * Golden snapshots of the composed prompt (TEST-001) for the cases a prompt
 * change is most likely to move. A diff here is a review prompt, not a
 * failure to paper over: update with `bunx vitest run tests/prompt-eval -u`
 * only after reading what changed in the composed text.
 */

import { describe, expect, it } from "vitest";
import { EVAL_CASES } from "./cases";
import { composeAsk } from "./harness";

const GOLDEN_CASES = [
  "multiple-choice",
  "spoken-interview",
  "coding",
  "live-suggestion",
  "screen-injection",
  "custom-mode",
] as const;

/** The per-request nonce is random; everything else is deterministic. */
function stable(text: string): string {
  return text.replace(/(<\/?context\b[^>\n]*? id=")[0-9a-f]{8}"/g, '$1NONCE"');
}

describe("prompt-eval golden prompts", () => {
  it.each(GOLDEN_CASES)("%s", async (id) => {
    const evalCase = EVAL_CASES.find((candidate) => candidate.id === id);
    if (!evalCase) throw new Error(`prompt-eval: golden case "${id}" missing`);
    const { system, user } = await composeAsk(evalCase);
    const composed = `=== SYSTEM ===\n${system}\n\n=== USER ===\n${stable(user)}\n`;
    await expect(composed).toMatchFileSnapshot(`./__golden__/${id}.txt`);
  });
});
