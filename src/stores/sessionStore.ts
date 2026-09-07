import { create } from "zustand";

import { bluey } from "@/lib/tauri/api";
import { toBlueyError, type Session, type SessionEvent } from "@/lib/types";

interface SessionStore {
  active: Session | null;
  /** Timeline events for the active session (append-only during the session). */
  events: SessionEvent[];
  setActive(session: Session | null): void;
  pushEvent(event: SessionEvent): void;
  load(): Promise<void>;
}

export const useSessionStore = create<SessionStore>((set, get) => ({
  active: null,
  events: [],
  setActive: (session) =>
    set((state) => ({
      active: session,
      events: session && state.active?.id === session.id ? state.events : [],
    })),
  pushEvent: (event) => {
    const active = get().active;
    if (!active || event.sessionId !== active.id) return;
    set((state) => ({ events: [...state.events, event].slice(-200) }));
  },
  load: async () => {
    try {
      const active = await bluey.session.getActive();
      set({ active });
      if (active) {
        set({ events: await bluey.session.listEvents({ sessionId: active.id }) });
      }
    } catch (error) {
      console.warn("[sessionStore] failed to load", toBlueyError(error));
    }
  },
}));
