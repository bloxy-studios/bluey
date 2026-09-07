import { create } from "zustand";

import { bluey } from "@/lib/tauri/api";
import { toBlueyError, type BlueyError, type Settings, type SettingsPatch } from "@/lib/types";

interface SettingsStore {
  settings: Settings | null;
  lastError: BlueyError | null;
  /** From `settings.changed` events. */
  applyRemote(settings: Settings): void;
  load(): Promise<void>;
  /** Persist a patch through the backend; state is updated from the result. */
  update(patch: SettingsPatch): Promise<Settings | null>;
  reset(): Promise<void>;
}

export const useSettingsStore = create<SettingsStore>((set) => ({
  settings: null,
  lastError: null,
  applyRemote: (settings) => set({ settings }),
  load: async () => {
    try {
      set({ settings: await bluey.settings.get(), lastError: null });
    } catch (error) {
      set({ lastError: toBlueyError(error) });
    }
  },
  update: async (patch) => {
    try {
      const settings = await bluey.settings.update({ patch });
      set({ settings, lastError: null });
      return settings;
    } catch (error) {
      set({ lastError: toBlueyError(error) });
      return null;
    }
  },
  reset: async () => {
    try {
      set({ settings: await bluey.settings.reset(), lastError: null });
    } catch (error) {
      set({ lastError: toBlueyError(error) });
    }
  },
}));
