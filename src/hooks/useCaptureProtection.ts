import { useEffect, useState } from "react";

import { bluey } from "@/lib/tauri/api";
import type { CaptureProtection } from "@/lib/types";
import { useSettingsStore } from "@/stores/settingsStore";

/**
 * What content protection really guarantees (Rust `capture_get_protection`: `partial` on
 * macOS 15+, where ScreenCaptureKit may ignore it), re-read whenever the saved display mode
 * changes, from anywhere (HUD eye, tray, Settings). `null` while unknown: before the first
 * answer, when the read fails, or when the answer does not match the saved mode yet (the tray
 * publishes the new settings just before the side effect applies them). Callers must not claim
 * full protection on `null` (SEC-004, UX-004).
 */
export function useCaptureProtection(): CaptureProtection | null {
  const displayMode = useSettingsStore((s) => s.settings?.privacy.displayMode);
  const [protection, setProtection] = useState<CaptureProtection | null>(null);

  useEffect(() => {
    if (!displayMode) return;
    let live = true;
    bluey.capture
      .getProtection()
      .then((next) => {
        if (live) setProtection(next);
      })
      .catch(() => {
        if (live) setProtection(null);
      });
    return () => {
      live = false;
    };
  }, [displayMode]);

  return protection && displayMode && protection.enabled === (displayMode === "privacy") ? protection : null;
}
