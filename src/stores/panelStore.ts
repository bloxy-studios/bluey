import { create } from "zustand";

import { bluey } from "@/lib/tauri/api";
import { toBlueyError, type PanelState } from "@/lib/types";

interface PanelStore {
  state: PanelState | null;
  applyRemote(state: PanelState): void;
  load(): Promise<void>;
  /** Whether the last queued measurement succeeded; failures remain retryable. */
  setExpanded(expanded: boolean, height?: number): Promise<boolean>;
}

export const usePanelStore = create<PanelStore>((set) => {
  type Measurement = { expanded: boolean; height?: number };
  let pending: Measurement | null = null;
  let active: Measurement | null = null;
  let resizing: Promise<boolean> | null = null;
  let remoteRevision = 0;
  let stateRevision = 0;
  let measurementRevision = 0;
  let loadRevision = 0;

  const applyState = (state: PanelState) => {
    stateRevision += 1;
    set({ state });
  };

  const same = (a: Measurement | null, b: Measurement) =>
    a !== null && a.expanded === b.expanded && a.height === b.height;

  // Only one native resize may be in flight. Keep the latest pending intent,
  // so a slow expanded response can never resize after a newer collapse.
  const drain = async () => {
    let succeeded = false;
    while (pending) {
      const request = pending;
      pending = null;
      active = request;
      const revision = remoteRevision;
      try {
        const state = await bluey.panel.setExpanded(request);
        // An event is newer than the command's snapshot (e.g. a keyboard move
        // or opacity change during persistence). Do not overwrite it on reply.
        if (!pending && revision === remoteRevision) applyState(state);
        succeeded = true;
      } catch (error) {
        succeeded = false;
        console.warn("[panelStore] setExpanded failed", toBlueyError(error));
      }
    }
    active = null;
    resizing = null;
    return succeeded;
  };

  return {
    state: null,
    applyRemote: (state) => {
      remoteRevision += 1;
      applyState(state);
    },
    load: async () => {
      const revision = stateRevision;
      const measurement = measurementRevision;
      const request = ++loadRevision;
      try {
        const state = await bluey.panel.getState();
        // Loads can race another load or a command reply even if no panel
        // event arrives. Neither may restore a pre-resize snapshot.
        if (
          request === loadRevision &&
          revision === stateRevision &&
          measurement === measurementRevision &&
          !resizing
        )
          applyState(state);
      } catch (error) {
        console.warn("[panelStore] failed to load", toBlueyError(error));
      }
    },
    setExpanded: (expanded, height) => {
      measurementRevision += 1;
      const request = { expanded, height };
      if (resizing) {
        // Returning to the in-flight intent also cancels an obsolete pending one.
        pending = same(active, request) ? null : request;
        return resizing;
      }
      pending = request;
      // Install the in-flight marker before invoking transport: even a
      // synchronous transport throw must not leave a resolved promise locked
      // in `resizing`, or every subsequent height report would be stranded.
      let finish!: (succeeded: boolean) => void;
      const completion = new Promise<boolean>((resolve) => {
        finish = resolve;
      });
      resizing = completion;
      void drain().then(finish);
      return completion;
    },
  };
});
