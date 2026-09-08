/**
 * App bootstrap: pick the window from `?window=`, select the transport
 * (Tauri when the runtime is present, otherwise the in-memory mock),
 * initialise stores, and keep appearance settings mirrored onto `<html>`
 * as `data-*` attributes.
 */

import { initStores } from "@/stores/initStores";
import { useSettingsStore } from "@/stores/settingsStore";
import type { Settings } from "../types";
import type { WindowLabel } from "./commands";
import { hasTauriRuntime, setTransport, type Transport } from "./transport";

export interface BootstrapResult {
  windowLabel: WindowLabel;
  transport: Transport;
}

export function resolveWindowLabel(search: string): WindowLabel {
  const label = new URLSearchParams(search).get("window");
  return label === "settings" || label === "onboarding" ? label : "main";
}

function applyAppearance(settings: Settings | null): void {
  const root = document.documentElement;
  const appearance = settings?.appearance;

  const theme = appearance?.theme ?? "system";
  const prefersDark = window.matchMedia?.("(prefers-color-scheme: dark)").matches ?? true;
  root.dataset.theme = theme === "system" ? (prefersDark ? "dark" : "light") : theme;

  root.dataset.fontSize = appearance?.fontSize ?? "medium";
  root.dataset.density = appearance?.density ?? "comfortable";

  const reduced = appearance?.reducedMotion ?? "system";
  const prefersReduced = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;
  root.dataset.reducedMotion = reduced === "system" ? String(prefersReduced) : String(reduced === "on");
}

function watchAppearance(): void {
  const reapply = () => applyAppearance(useSettingsStore.getState().settings);

  useSettingsStore.subscribe((state, previous) => {
    if (state.settings?.appearance !== previous.settings?.appearance) reapply();
  });

  const colorScheme = window.matchMedia?.("(prefers-color-scheme: dark)");
  const motion = window.matchMedia?.("(prefers-reduced-motion: reduce)");
  colorScheme?.addEventListener?.("change", reapply);
  motion?.addEventListener?.("change", reapply);
}

export async function bootstrap(): Promise<BootstrapResult> {
  const windowLabel = resolveWindowLabel(window.location.search);

  let transport: Transport;
  if (hasTauriRuntime()) {
    const { TauriTransport } = await import("./tauri-transport");
    transport = new TauriTransport();
  } else {
    // Developer Mode backend — never reached inside the packaged app.
    const { MockTransport } = await import("./mock");
    transport = new MockTransport();
  }
  setTransport(transport);

  const root = document.documentElement;
  root.dataset.window = windowLabel;
  root.dataset.mock = String(transport.kind === "mock");
  applyAppearance(null);

  await initStores();
  applyAppearance(useSettingsStore.getState().settings);
  watchAppearance();

  return { windowLabel, transport };
}
