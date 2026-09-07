import { createContext, useContext } from "react";

export const SETTINGS_TABS = [
  "general",
  "modes",
  "keybinds",
  "audio",
  "screen",
  "ai",
  "privacy",
  "permissions",
  "sessions",
  "profile",
  "advanced",
  "about",
] as const;

export type SettingsTab = (typeof SETTINGS_TABS)[number];

export function isSettingsTab(value: string | null): value is SettingsTab {
  return value !== null && (SETTINGS_TABS as readonly string[]).includes(value);
}

export interface SettingsNav {
  tab: SettingsTab;
  setTab: (tab: SettingsTab) => void;
}

export const SettingsNavContext = createContext<SettingsNav>({ tab: "general", setTab: () => {} });

export function useSettingsNav(): SettingsNav {
  return useContext(SettingsNavContext);
}
