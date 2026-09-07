/**
 * Transport abstraction. The real implementation wraps `@tauri-apps/api`;
 * `MockTransport` (src/lib/tauri/mock/) implements the same command surface
 * in-memory for browser development and tests (Developer Mode).
 *
 * The UI, stores and intelligence layer only ever see `Transport`.
 */

import type { CommandArgs, CommandName, CommandResult } from "./commands";
import type { EventName, EventPayload } from "./events";

export type Unlisten = () => void;

/** A streaming channel handed to commands like `ai_stream`. */
export interface StreamChannel<T> {
  /** Called for every message from Rust in order. */
  onMessage(handler: (message: T) => void): void;
  /** Opaque object passed inside command args (Tauri `Channel`). */
  readonly raw: unknown;
}

export interface Transport {
  readonly kind: "tauri" | "mock";
  invoke<K extends CommandName>(command: K, args: CommandArgs<K>): Promise<CommandResult<K>>;
  listen<K extends EventName>(event: K, handler: (payload: EventPayload<K>) => void): Promise<Unlisten>;
  createChannel<T>(): StreamChannel<T>;
  /** Current window label ("main" | "settings" | "onboarding"). */
  currentWindowLabel(): string;
}

let current: Transport | null = null;

export function setTransport(transport: Transport): void {
  current = transport;
}

export function getTransport(): Transport {
  if (!current) {
    throw new Error("Transport not initialised. Call setTransport() during bootstrap.");
  }
  return current;
}

export function hasTauriRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}
