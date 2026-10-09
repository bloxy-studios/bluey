/**
 * Deterministic prompt-eval tier (TEST-001): every row of the evaluation
 * matrix is composed by the real engine over the fake transport, then
 * checked for the invariants the prompt architecture promises — one task,
 * one question, the voice and shape the intent chose, untrusted text only
 * inside this request's nonce blocks, spoken order, and layer budgets.
 */

import { beforeAll, describe, expect, it } from "vitest";
import { answerShapeLine, voiceLine } from "@/ai/prompts/task";
import { defaultContextBudget } from "@/context/budget";
import { estimateTokens } from "@/context/fusion";
import { makeSettings } from "../fixtures/helpers/builders";
import { EVAL_CASES, type EvalCase } from "./cases";
import { composeAsk, type ComposedAsk } from "./harness";

/** Every `<context … id="n">…</context id="n">` block of this request, keyed by its nonce. */
const BLOCK = /<context source="[^"]*" id="([0-9a-f]{8})">\n([\s\S]*?)\n<\/context id="\1">/g;

function outsideBlocks(user: string): string {
  return user.replace(BLOCK, "");
}

function linesStarting(text: string, prefix: string): string[] {
  return text.split("\n").filter((line) => line.startsWith(prefix));
}

function occurrences(haystack: string, needle: string): number {
  return haystack.split(needle).length - 1;
}

const composed = new Map<string, ComposedAsk>();

beforeAll(async () => {
  for (const evalCase of EVAL_CASES) composed.set(evalCase.id, await composeAsk(evalCase));
});

function ask(evalCase: EvalCase): ComposedAsk {
  const result = composed.get(evalCase.id);
  if (!result) throw new Error(`prompt-eval: case "${evalCase.id}" was not composed`);
  return result;
}

describe.each(EVAL_CASES.map((evalCase) => [evalCase.id, evalCase] as const))(
  "prompt-eval: %s",
  (_id, evalCase) => {
    it("asks exactly one task, in one shape and one voice, at the end of the user message", () => {
      const { user } = ask(evalCase);
      const outside = outsideBlocks(user);
      expect(linesStarting(outside, "Task:")).toHaveLength(1);
      expect(linesStarting(outside, "Shape:")).toEqual([answerShapeLine(evalCase.expect.shape)]);
      expect(linesStarting(outside, "Voice:")).toEqual([voiceLine(evalCase.expect.voice)]);
      expect(user.trimEnd().endsWith(voiceLine(evalCase.expect.voice))).toBe(true);
    });

    it("requests the schema the intent routes to", () => {
      const { system } = ask(evalCase);
      expect(system).toContain(`"bluey_${evalCase.expect.schema.replace(/-/g, "_")}"`);
    });

    it("states the question once: a heard one as untrusted context, a typed one right before the task", () => {
      const question = evalCase.expect.question;
      if (!question) return;
      const { user } = ask(evalCase);
      expect(occurrences(user, question)).toBe(1);
      if (evalCase.detectedEvent) {
        expect(outsideBlocks(user)).not.toContain(question);
        return;
      }
      const outside = outsideBlocks(user)
        .split("\n")
        .filter((line) => line.trim().length > 0);
      const taskIndex = outside.findIndex((line) => line.startsWith("Task:"));
      expect(outside[taskIndex - 1]).toBe(`My question: ${question}`);
    });

    it("keeps untrusted text inside this request's nonce blocks, defanged", () => {
      const { user } = ask(evalCase);
      const outside = outsideBlocks(user);
      expect(outside).not.toMatch(/<\/?context\s|<system-reminder/);
      expect(linesStarting(outside, "#")).toEqual([]);
      for (const hostile of evalCase.expect.hostile ?? []) {
        expect(outside).not.toContain(hostile);
        expect(user).not.toContain(`\n${hostile}\n`);
      }
    });
  },
);

/**
 * abc0693 shipped ~653 estimated tokens of identity + safety rules + response
 * contract; the refinement replaced lines rather than adding them, so the
 * static head of every system prompt must stay under that.
 */
const STATIC_LAYER_BUDGET = 653;
/** The task area (My question / Task / Shape / Voice) and block framing on top of the fused context. */
const TASK_AREA_ALLOWANCE = 400;

describe("prompt-eval: layer budgets and ordering", () => {
  it.each(EVAL_CASES.map((evalCase) => [evalCase.id, evalCase] as const))(
    "%s keeps the static layers within budget",
    (_id, evalCase) => {
      const { system } = ask(evalCase);
      const staticHead = system.slice(0, system.indexOf("\n\nMode: "));
      expect(staticHead.length).toBeGreaterThan(0);
      expect(estimateTokens(staticHead)).toBeLessThanOrEqual(STATIC_LAYER_BUDGET);
    },
  );

  it.each(EVAL_CASES.map((evalCase) => [evalCase.id, evalCase] as const))(
    "%s keeps the context within the settings budget",
    (_id, evalCase) => {
      const { request, user } = ask(evalCase);
      const contextBudget = defaultContextBudget(makeSettings());
      expect(request.contextTokens).toBeLessThanOrEqual(contextBudget);
      expect(estimateTokens(user)).toBeLessThanOrEqual(contextBudget + TASK_AREA_ALLOWANCE);
    },
  );

  it("renders a long transcript in spoken order and keeps its latest turn", () => {
    const evalCase = EVAL_CASES.find((candidate) => candidate.id === "long-transcript");
    if (!evalCase) throw new Error("prompt-eval: long-transcript case missing");
    const turns = [...ask(evalCase).user.matchAll(/Turn (\d{2}):/g)].map((match) => Number(match[1]));
    expect(turns.length).toBeGreaterThan(1);
    expect(turns).toEqual([...turns].sort((a, b) => a - b));
    expect(turns.at(-1)).toBe(40);
  });
});
