/**
 * Prompt-eval harness (TEST-001): runs the real engine over the fake
 * transport for one evaluation case and returns the composed request —
 * the exact system and user text a provider would receive.
 */

import { createResponseEngine } from "@/ai/engine";
import type { AskInput } from "@/lib/engine-contract";
import { setTransport } from "@/lib/tauri/transport";
import type { AIRequest, ContextSnapshot, RetrievedChunk } from "@/lib/types";
import { FakeTransport } from "../fixtures/helpers/fake-transport";
import { makeSession, makeSettings } from "../fixtures/helpers/builders";
import type { EvalCase } from "./cases";

export interface ComposedAsk {
  request: AIRequest;
  system: string;
  user: string;
}

function textOf(request: AIRequest, index: number): string {
  const part = request.messages[index]?.content[0];
  return part && "text" in part ? part.text : "";
}

/** Chunks for a retrieval query: personal instructions only on the leading pass. */
function chunksFor(evalCase: EvalCase, kinds: readonly string[] | undefined): RetrievedChunk[] {
  const wantsPersonal = kinds?.includes("personal_instructions") ?? false;
  return (evalCase.chunks ?? []).filter(
    (chunk) => (chunk.documentKind === "personal_instructions") === wantsPersonal,
  );
}

/** Compose one ask end to end and capture the provider-bound request. */
export async function composeAsk(evalCase: EvalCase): Promise<ComposedAsk> {
  const fake = new FakeTransport();
  const snapshot = evalCase.snapshot as ContextSnapshot;
  fake.handle("context_build_snapshot", () => snapshot);
  fake.handle("documents_retrieve", ({ query }) => chunksFor(evalCase, query.kinds));
  fake.handle("responses_save", ({ response }) => response);
  fake.handle("sessions_add_event", (args) => ({
    id: "evt_eval",
    sessionId: args.sessionId,
    type: args.type,
    title: args.title,
    createdAt: "2026-09-07T09:10:00.000Z",
  }));
  fake.handle("ai_cancel", () => true);
  setTransport(fake);

  const engine = createResponseEngine({ now: () => new Date("2026-09-07T09:09:00.000Z") });
  const input: AskInput = {
    trigger: evalCase.trigger,
    instruction: evalCase.instruction,
    captureScreen: evalCase.trigger === "shortcut_capture" || evalCase.trigger === "assist",
    mode: evalCase.mode,
    session: makeSession({ id: "ses_eval", modeId: evalCase.mode.id }),
    settings: makeSettings(),
    previousResponses: [...(evalCase.previousResponses ?? [])],
    detectedEvent: evalCase.detectedEvent,
  };
  // A live suggestion is a prepared answer shown as it is written (engine.prepare with onComplete).
  if (evalCase.live) await engine.prepare(input, { onComplete: () => {} });
  else await engine.ask(input).done;

  const request = fake.callsFor("ai_stream")[0]?.request;
  if (!request) throw new Error(`prompt-eval: case "${evalCase.id}" sent no request`);
  return { request, system: textOf(request, 0), user: textOf(request, 1) };
}
