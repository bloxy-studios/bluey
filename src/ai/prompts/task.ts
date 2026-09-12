/**
 * Trigger-specific task instructions (what to do with the assembled context)
 * and the answer-shape line that follows them. Both are answer-first: they
 * ask for the answer itself, never for a description of the screen or advice
 * about what the answer should contain.
 */

import type { AskTrigger } from "@/lib/engine-contract";
import type { AnswerShape } from "@/lib/types";

const TASK_LINES: Record<AskTrigger, string> = {
  shortcut_capture:
    "Task: Solve or answer what is on the screen. If it is a question or problem, give the answer; if it is content with no question, give the single most useful fact or fix about it. Do not describe the screen.",
  shortcut_generate:
    "Task: Write exactly what I say next in this conversation — the reply itself, first person, ready to speak aloud. Not advice about what to say.",
  typed: "Task: Answer my question below. Lead with the answer; use the context only where it changes the answer.",
  follow_up:
    "Task: Answer this follow-up. Build on the earlier responses in this session without repeating them.",
  detected_event:
    "Task: Answer the question just asked in the live conversation (see \"Current question\") as me — first person, ready to speak. The answer itself, not coaching about it.",
  regenerate:
    "Task: Answer again, better: a sharper or differently angled answer to the same question — not a rephrasing, and not a critique of the previous one.",
  assist:
    "Task: No question was typed. Do the single most useful thing the context calls for — answer the visible question, fix the visible error, or write the next thing to say — and only that.",
};

/**
 * One line per shape, rendered directly under the task line. The first line
 * of every shape is the answer; what may follow it is bounded here.
 */
const SHAPE_LINES: Record<AnswerShape, string> = {
  choice:
    "Shape: multiple choice. First line: the option to pick — its letter or number and its text. Then at most one sentence on why. Nothing else.",
  boolean: "Shape: yes/no. First word: yes or no (or true/false). Then one sentence of reason.",
  fill_in:
    "Shape: fill in the blank. First line: the missing word(s) or value exactly as they should be entered. At most one sentence after it, only if a reason is needed.",
  calculation: "Shape: calculation. First line: the final result with its unit. Then the working, one step per line.",
  compare:
    "Shape: comparison. First sentence: which one is better, named the way the source names it (Response A, option 2…). Then the concrete reasons it wins, most decisive first, and what the weaker one gets wrong. No scores unless asked.",
  short_answer: "Shape: short answer. One to three sentences, answer first.",
  explain:
    "Shape: explanation. First sentence: the direct answer. Then the reasons or steps that make it usable, in order — no recap at the end.",
  spoken:
    "Shape: spoken. Exactly what I say, first person, natural spoken rhythm. No headings, no bullets, no stage directions.",
  written: "Shape: written. The text I would send or submit, first person, ready to paste as-is — no framing around it.",
  code: "Shape: code. The working solution is the deliverable; keep the prose to the approach, complexity and edge cases the mode asks for.",
  design: "Shape: design. The design itself with the trade-offs I would state — quantified wherever numbers exist.",
  summary: "Shape: summary. The points themselves, grouped the way the mode asks — no introduction, no commentary about the summary.",
};

export function taskLineFor(trigger: AskTrigger): string {
  return TASK_LINES[trigger];
}

export function answerShapeLine(shape: AnswerShape): string {
  return SHAPE_LINES[shape];
}
