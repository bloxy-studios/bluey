import { create } from "zustand";

import { bluey } from "@/lib/tauri/api";
import { toBlueyError, type AppStatus } from "@/lib/types";

interface AppStore {
  status: AppStatus | null;
  /** Set from `app.state` events (single source of truth: the backend). */
  setStatus(status: AppStatus): void;
  load(): Promise<void>;
}

export const useAppStore = create<AppStore>((set) => ({
  status: null,
  setStatus: (status) => set({ status }),
  load: async () => {
    try {
      set({ status: await bluey.app.getStatus() });
    } catch (error) {
      console.warn("[appStore] failed to load status", toBlueyError(error));
    }
  },
}));
