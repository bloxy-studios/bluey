/**
 * FakeTransport: an in-memory `Transport` for tests. Records every invocation,
 * lets tests script command results, and streams `AIChunk`s through the same
 * channel mechanism the real Tauri transport uses.
 */

import type { CommandArgs, CommandName, CommandResult } from "@/lib/tauri/commands";
import type { EventName, EventPayload } from "@/lib/tauri/events";
import type { StreamChannel, Transport, Unlisten } from "@/lib/tauri/transport";
import type { AIChunk, AIRequest } from "@/lib/types";

type AnyHandler = (args: never) => unknown;

export type AIScript = (
  request: AIRequest,
  emit: (chunk: AIChunk) => void,
) => void | Promise<void>;

interface FakeChannelRaw<T> {
  __fake: true;
  __emit(message: T): void;
}

/** Simple JSON structured-output happy path used when no script is set. */
export function defaultAIScript(request: AIRequest, emit: (chunk: AIChunk) => void): void {
  emit({
    type: "started",
    requestId: request.requestId,
    selection: {
      providerId: "mock",
      providerKind: "mock",
      model: "mock-1",
      role: "default",
      reason: "test",
    },
  });
  const payload = JSON.stringify({
    responseType: "answer",
    title: "Mock answer",
    content: "This is a mock answer.",
  });
  emit({ type: "delta", requestId: request.requestId, text: payload });
  emit({ type: "usage", requestId: request.requestId, inputTokens: 100, outputTokens: 20 });
  emit({
    type: "completed",
    requestId: request.requestId,
    finishReason: "stop",
    totalMs: 420,
    timeToFirstTokenMs: 120,
  });
}

export function deferred<T = void>(): { promise: Promise<T>; resolve: (value: T) => void } {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

export class FakeTransport implements Transport {
  readonly kind = "mock" as const;

  readonly calls: Array<{ command: CommandName; args: unknown }> = [];
  private readonly handlers = new Map<CommandName, AnyHandler>();
  private readonly listeners = new Map<EventName, Set<(payload: unknown) => void>>();
  private aiScript: AIScript = defaultAIScript;

  /** Script the result (or thrown error) of a command. */
  handle<K extends CommandName>(
    command: K,
    handler: (args: CommandArgs<K>) => CommandResult<K> | Promise<CommandResult<K>>,
  ): this {
    this.handlers.set(command, handler as AnyHandler);
    return this;
  }

  /** Script the `ai_stream` chunk sequence. */
  setAIScript(script: AIScript): this {
    this.aiScript = script;
    return this;
  }

  callsFor<K extends CommandName>(command: K): CommandArgs<K>[] {
    return this.calls.filter((c) => c.command === command).map((c) => c.args as CommandArgs<K>);
  }

  async invoke<K extends CommandName>(command: K, args: CommandArgs<K>): Promise<CommandResult<K>> {
    this.calls.push({ command, args });

    if (command === "ai_stream") {
      const { request, onChunk } = args as unknown as { request: AIRequest; onChunk: unknown };
      const channel = onChunk as FakeChannelRaw<AIChunk>;
      if (!channel || channel.__fake !== true) {
        throw new Error("FakeTransport: ai_stream called without a fake channel");
      }
      await this.aiScript(request, (chunk) => channel.__emit(chunk));
      return undefined as CommandResult<K>;
    }

    const handler = this.handlers.get(command);
    if (!handler) {
      throw new Error(`FakeTransport: no handler registered for command "${command}"`);
    }
    return (await handler(args as never)) as CommandResult<K>;
  }

  listen<K extends EventName>(
    event: K,
    handler: (payload: EventPayload<K>) => void,
  ): Promise<Unlisten> {
    let set = this.listeners.get(event);
    if (!set) {
      set = new Set();
      this.listeners.set(event, set);
    }
    const wrapped = handler as (payload: unknown) => void;
    set.add(wrapped);
    return Promise.resolve(() => {
      set?.delete(wrapped);
    });
  }

  /** Simulate a Rust-side event. */
  emitRemote<K extends EventName>(event: K, payload: EventPayload<K>): void {
    for (const handler of this.listeners.get(event) ?? []) handler(payload);
  }

  createChannel<T>(): StreamChannel<T> {
    const handlers: Array<(message: T) => void> = [];
    const raw: FakeChannelRaw<T> = {
      __fake: true,
      __emit: (message) => {
        for (const handler of handlers) handler(message);
      },
    };
    return {
      onMessage: (handler) => {
        handlers.push(handler);
      },
      raw,
    };
  }

  currentWindowLabel(): string {
    return "main";
  }
}
