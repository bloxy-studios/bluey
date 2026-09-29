/**
 * Trigger-specific task instructions (what to do with the assembled context)
 * and the answer-shape line that follows them. Both are answer-first: they
 * ask for the answer itself, never for a description of the screen or advice
 * about what the answer should contain.
 */

import type { AskTrigger } from "@/lib/engine-contract";
import type { AnswerShape, AnswerVoice } from "@/lib/types";

const TASK_LINES: Record<AskTrigger, string> = {
  shortcut_capture:
    "Task: Solve or answer what is on the screen. If it is a question or problem, give the answer; if it is content with no question, give the single most useful fact or fix about it. Do not describe the screen.",
  shortcut_generate:
    "Task: Write exactly what I say next in this conversation — the reply itself, first person, ready to speak aloud. Not advice about what to say.",
  typed:
    "Task: Answer my question above. Lead with the answer; use the context only where it changes the answer.",
  follow_up:
    "Task: Answer this follow-up. Build on the earlier responses in this session without repeating them.",
  detected_event:
    'Task: Answer the question just asked in the live conversation (see "Question just asked") as me — first person, ready to speak. The answer itself, not coaching about it.',
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
  boolean:
    'Shape: yes/no. First word: yes or no (or true/false) — or "Neither"/"It depends" only when the premise is wrong. Then the one fact that decides it.',
  fill_in:
    "Shape: fill in the blank. First line: the missing word(s) or value exactly as they should be entered. At most one sentence after it, only if a reason is needed.",
  calculation:
    "Shape: calculation. First line: the final result with its unit. Then the working, one step per line.",
  compare:
    "Shape: comparison. First sentence: which one is better, named the way the source names it (Response A, option 2…). Then the concrete reasons it wins, most decisive first, and what the weaker one gets wrong. No scores unless asked.",
  short_answer:
    "Shape: short answer. One to three sentences, answer first; for a prediction, the best estimate and what it hinges on.",
  explain:
    "Shape: explanation. First sentence: the direct answer. Then the reasons or steps that make it usable, in order — no recap at the end.",
  spoken: "Shape: spoken. Natural spoken rhythm. No headings, no bullets, no stage directions.",
  written: "Shape: written. The text itself, ready to paste as-is — no framing around it.",
  code: "Shape: code. The working solution is the deliverable; keep the prose to the approach, complexity and edge cases the mode asks for.",
  debug:
    "Shape: debug. First line: the exact fix. Then the cause in one sentence. Then only the changed lines in a fenced block — not the whole file.",
  design:
    "Shape: design. The design itself with the trade-offs I would state — quantified wherever numbers exist.",
  summary:
    "Shape: summary. The points themselves, grouped the way the mode asks — no introduction, no commentary about the summary.",
};

/**
 * Whose words the answer is, one line per request (MODE-002): an explanation
 * is addressed to me, a pick or a text is mine to submit, speech is mine to say.
 */
const VOICE_LINES: Record<AnswerVoice, string> = {
  "speak-as-user":
    "Voice: my words to say aloud — first person, as I would speak them. Never about me in the third person, never advice about what to say.",
  "write-as-user":
    'Voice: my words to submit or send — first person ("My pick is B…", "I would…"). Never about me in the third person unless the text itself calls for it.',
  "explain-to-user":
    'Voice: explain it to me — directly and plainly, addressing me as "you". Never a script for me to recite unless I ask for one.',
};

export function voiceLine(voice: AnswerVoice): string {
  return VOICE_LINES[voice];
}

export function taskLineFor(trigger: AskTrigger): string {
  return TASK_LINES[trigger];
}

export function answerShapeLine(shape: AnswerShape): string {
  return SHAPE_LINES[shape];
}
