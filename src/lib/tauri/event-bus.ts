/**
 * Typed event bus over the transport. Also supports local (frontend-only)
 * emission so TS modules can publish derived events (e.g. `question.detected`
 * computed in the intelligence layer) to UI subscribers.
 */

import type { EventName, EventPayload } from "./events";
import { getTransport, type Unlisten } from "./transport";

type Handler<K extends EventName> = (payload: EventPayload<K>) => void;

class EventBus {
  private readonly local = new Map<EventName, Set<Handler<EventName>>>();
  private readonly remote = new Map<EventName, Promise<Unlisten>>();

  /** Subscribe to an event (from Rust or emitted locally). Returns an unsubscribe fn. */
  on<K extends EventName>(name: K, handler: Handler<K>): Unlisten {
    let set = this.local.get(name);
    if (!set) {
      set = new Set();
      this.local.set(name, set);
      this.ensureRemote(name);
    }
    set.add(handler as Handler<EventName>);
    return () => {
      set?.delete(handler as Handler<EventName>);
    };
  }

  /** Subscribe once. */
  once<K extends EventName>(name: K, handler: Handler<K>): Unlisten {
    const off = this.on(name, (payload) => {
      off();
      handler(payload);
    });
    return off;
  }

  /** Emit locally (does not cross into Rust). */
  emit<K extends EventName>(name: K, payload: EventPayload<K>): void {
    const set = this.local.get(name);
    if (!set) return;
    for (const handler of Array.from(set)) {
      try {
        handler(payload);
      } catch (error) {
        console.error(`[eventBus] handler for ${name} threw`, error);
      }
    }
  }

  private ensureRemote(name: EventName): void {
    if (this.remote.has(name)) return;
    const p = getTransport().listen(name, (payload) => this.emit(name, payload));
    this.remote.set(name, p);
    p.catch((error) => {
      console.error(`[eventBus] failed to listen for ${name}`, error);
      this.remote.delete(name);
    });
  }

  /** Test helper: tear down remote listeners. */
  async dispose(): Promise<void> {
    const unlistens = await Promise.all(Array.from(this.remote.values()));
    unlistens.forEach((fn) => fn());
    this.remote.clear();
    this.local.clear();
  }
}

export const eventBus = new EventBus();
export type { EventBus };
