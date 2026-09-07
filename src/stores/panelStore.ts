import { create } from "zustand";

import { bluey } from "@/lib/tauri/api";
import { toBlueyError, type PanelState } from "@/lib/types";

interface PanelStore {
  state: PanelState | null;
  applyRemote(state: PanelState): void;
  load(): Promise<void>;
  setExpanded(expanded: boolean, height?: number): Promise<void>;
}

export const usePanelStore = create<PanelStore>((set) => ({
  state: null,
  applyRemote: (state) => set({ state }),
  load: async () => {
    try {
      set({ state: await bluey.panel.getState() });
    } catch (error) {
      console.warn("[panelStore] failed to load", toBlueyError(error));
    }
  },
  setExpanded: async (expanded, height) => {
    try {
      set({ state: await bluey.panel.setExpanded({ expanded, height }) });
    } catch (error) {
      console.warn("[panelStore] setExpanded failed", toBlueyError(error));
    }
  },
}));
