/**
 * Opt-in model-graded tier (TEST-001): sends each composed prompt to a real
 * model and grades the answer against the case's rubric. Skipped unless
 * BLUEY_PROMPT_EVAL_LIVE=1 and ANTHROPIC_API_KEY are set; the model is
 * BLUEY_PROMPT_EVAL_MODEL (default claude-sonnet-4-6). Costs real tokens.
 *
 *   BLUEY_PROMPT_EVAL_LIVE=1 ANTHROPIC_API_KEY=… bunx vitest run tests/prompt-eval/live.test.ts
 */

import { describe, expect, it } from "vitest";
import { parseStructuredOutput } from "@/modes/schemas";
import { EVAL_CASES } from "./cases";
import { composeAsk } from "./harness";

const API_KEY = process.env.ANTHROPIC_API_KEY ?? "";
const LIVE = process.env.BLUEY_PROMPT_EVAL_LIVE === "1" && API_KEY.length > 0;
const MODEL = process.env.BLUEY_PROMPT_EVAL_MODEL ?? "claude-sonnet-4-6";
const MAX_OUTPUT_TOKENS = 1500;
const LIVE_TIMEOUT_MS = 60_000;

async function complete(system: string, user: string): Promise<string> {
  const response = await fetch("https://api.anthropic.com/v1/messages", {
    method: "POST",
    headers: { "content-type": "application/json", "x-api-key": API_KEY, "anthropic-version": "2023-06-01" },
    body: JSON.stringify({
      model: MODEL,
      max_tokens: MAX_OUTPUT_TOKENS,
      system,
      messages: [{ role: "user", content: user }],
    }),
  });
  if (!response.ok) throw new Error(`prompt-eval live: HTTP ${response.status}`);
  const body = (await response.json()) as { content?: Array<{ type: string; text?: string }> };
  return (body.content ?? []).map((part) => (part.type === "text" ? (part.text ?? "") : "")).join("");
}

describe.skipIf(!LIVE)("prompt-eval live (model-graded)", () => {
  it.each(EVAL_CASES.map((evalCase) => [evalCase.id, evalCase] as const))(
    "%s meets its rubric",
    async (_id, evalCase) => {
      const { system, user } = await composeAsk(evalCase);
      const raw = await complete(system, user);
      const answer = parseStructuredOutput(evalCase.expect.schema, raw)?.content ?? raw;
      for (const pattern of evalCase.rubric.must ?? []) expect(answer).toMatch(pattern);
      for (const pattern of evalCase.rubric.mustNot ?? []) expect(answer).not.toMatch(pattern);
    },
    LIVE_TIMEOUT_MS,
  );
});
