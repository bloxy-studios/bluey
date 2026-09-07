/**
 * Real Tauri transport: wraps `@tauri-apps/api` and normalises errors into
 * `BlueyError`. This is the ONLY module (besides the mock) that talks to the
 * Tauri runtime directly.
 */

import { Channel, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { toBlueyError } from "../types";
import type { CommandArgs, CommandName, CommandResult } from "./commands";
import { tauriEventName, type EventName, type EventPayload } from "./events";
import type { StreamChannel, Transport, Unlisten } from "./transport";

class TauriStreamChannel<T> implements StreamChannel<T> {
  private readonly channel = new Channel<T>();

  get raw(): unknown {
    return this.channel;
  }

  onMessage(handler: (message: T) => void): void {
    this.channel.onmessage = handler;
  }
}

export class TauriTransport implements Transport {
  readonly kind = "tauri" as const;

  async invoke<K extends CommandName>(command: K, args: CommandArgs<K>): Promise<CommandResult<K>> {
    try {
      return await invoke<CommandResult<K>>(command, (args ?? undefined) as Record<string, unknown> | undefined);
    } catch (error) {
      throw toBlueyError(error);
    }
  }

  async listen<K extends EventName>(event: K, handler: (payload: EventPayload<K>) => void): Promise<Unlisten> {
    return listen<EventPayload<K>>(tauriEventName(event), (e) => handler(e.payload));
  }

  createChannel<T>(): StreamChannel<T> {
    return new TauriStreamChannel<T>();
  }

  currentWindowLabel(): string {
    return getCurrentWindow().label;
  }
}
