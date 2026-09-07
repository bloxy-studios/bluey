import { create } from "zustand";

import { bluey } from "@/lib/tauri/api";
import { toBlueyError, type BlueyMode } from "@/lib/types";

interface ModesStore {
  modes: BlueyMode[];
  loaded: boolean;
  applyRemote(modes: BlueyMode[]): void;
  load(): Promise<void>;
}

export const useModesStore = create<ModesStore>((set) => ({
  modes: [],
  loaded: false,
  applyRemote: (modes) => set({ modes, loaded: true }),
  load: async () => {
    try {
      set({ modes: await bluey.modes.list(), loaded: true });
    } catch (error) {
      console.warn("[modesStore] failed to load", toBlueyError(error));
    }
  },
}));

export function modeById(modes: BlueyMode[], id: string | undefined): BlueyMode | undefined {
  return modes.find((mode) => mode.id === id);
}
