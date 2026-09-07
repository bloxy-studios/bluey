/** Trigger-specific task instructions (what to do with the assembled context). */

import type { AskTrigger } from "@/lib/engine-contract";

const TASK_LINES: Record<AskTrigger, string> = {
  shortcut_capture:
    "Task: Explain or solve what is on the screen right now. If it is a problem (code, question, error), solve it; if it is content, explain the part that matters.",
  shortcut_generate:
    "Task: Draft what I should say next in this conversation, based on the recent transcript. First person, ready to speak aloud.",
  typed: "Task: Answer my question below directly, using the context only where it helps.",
  follow_up:
    "Task: Continue the thread. This is a follow-up to the earlier responses in this session — do not repeat what was already said; build on it.",
  detected_event:
    "Task: A question was just asked in the live conversation (see \"Current question\"). Prepare the answer I should give, first person, ready to speak.",
  regenerate:
    "Task: Regenerate the previous answer. Take a different or sharper angle rather than rephrasing the same content.",
  assist:
    "Task: No explicit question was asked. Infer the single most useful thing to do from the context (answer the visible question, fix the visible error, or suggest what to say next) and do it.",
};

export function taskLineFor(trigger: AskTrigger): string {
  return TASK_LINES[trigger];
}
