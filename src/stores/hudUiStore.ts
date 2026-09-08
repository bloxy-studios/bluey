/**
 * HUD-local UI preferences that are not app settings but should survive a
 * relaunch: whether screen context is attached to asks and whether the live
 * transcript strip is collapsed. Persisted in localStorage (never sensitive).
 */

import { create } from "zustand";

export const HUD_UI_STORAGE_KEY = "bluey.hud.ui";

interface HudUiState {
  screenEnabled: boolean;
  transcriptCollapsed: boolean;
}

interface HudUiStore extends HudUiState {
  setScreenEnabled(enabled: boolean): void;
  toggleScreen(): void;
  toggleTranscript(): void;
}

const DEFAULTS: HudUiState = { screenEnabled: true, transcriptCollapsed: false };

function readPersisted(): HudUiState {
  try {
    const raw = globalThis.localStorage?.getItem(HUD_UI_STORAGE_KEY);
    if (!raw) return DEFAULTS;
    const parsed = JSON.parse(raw) as Partial<HudUiState>;
    return {
      screenEnabled:
        typeof parsed.screenEnabled === "boolean" ? parsed.screenEnabled : DEFAULTS.screenEnabled,
      transcriptCollapsed:
        typeof parsed.transcriptCollapsed === "boolean"
          ? parsed.transcriptCollapsed
          : DEFAULTS.transcriptCollapsed,
    };
  } catch {
    return DEFAULTS;
  }
}

function persist(state: HudUiState): void {
  try {
    globalThis.localStorage?.setItem(HUD_UI_STORAGE_KEY, JSON.stringify(state));
  } catch {
    // Private mode / quota — preferences simply don't survive the relaunch.
  }
}

export const useHudUiStore = create<HudUiStore>((set, get) => {
  const apply = (patch: Partial<HudUiState>) => {
    const next = {
      screenEnabled: get().screenEnabled,
      transcriptCollapsed: get().transcriptCollapsed,
      ...patch,
    };
    persist(next);
    set(next);
  };
  return {
    ...readPersisted(),
    setScreenEnabled: (enabled) => apply({ screenEnabled: enabled }),
    toggleScreen: () => apply({ screenEnabled: !get().screenEnabled }),
    toggleTranscript: () => apply({ transcriptCollapsed: !get().transcriptCollapsed }),
  };
});

/** Test helper. */
export function resetHudUiForTest(): void {
  useHudUiStore.setState({ ...DEFAULTS });
  try {
    globalThis.localStorage?.removeItem(HUD_UI_STORAGE_KEY);
  } catch {
    // ignore
  }
}
