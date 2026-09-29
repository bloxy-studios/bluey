import { useEffect, useState } from "react";

import { bluey } from "@/lib/tauri/api";
import type { CaptureProtection } from "@/lib/types";
import { useSettingsStore } from "@/stores/settingsStore";

/** A status read before the settings side effect applied the mode is re-read, a few times. */
const MAX_READS = 3;
const REREAD_DELAY_MS = 250;

/**
 * What content protection really guarantees (Rust `capture_get_protection`: `partial` on
 * macOS 15+, where ScreenCaptureKit may ignore it), re-read whenever the saved display mode
 * changes, from anywhere (HUD eye, tray, Settings). `null` while unknown: before the first
 * answer, when the read fails, or when the answer does not match the saved mode yet (Rust
 * publishes the new settings just before the side effect applies them, so it is re-read).
 * Callers must not claim full protection on `null` (SEC-004, UX-004).
 */
export function useCaptureProtection(): CaptureProtection | null {
  const displayMode = useSettingsStore((s) => s.settings?.privacy.displayMode);
  const [protection, setProtection] = useState<CaptureProtection | null>(null);

  useEffect(() => {
    if (!displayMode) return;
    let live = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const read = (attempt: number) => {
      bluey.capture
        .getProtection()
        .then((next) => {
          if (!live) return;
          setProtection(next);
          // Read before the side effect applied the new mode: ask again shortly.
          if (next.enabled !== (displayMode === "privacy") && attempt < MAX_READS) {
            timer = setTimeout(() => read(attempt + 1), REREAD_DELAY_MS);
          }
        })
        .catch(() => {
          if (live) setProtection(null);
        });
    };
    read(1);
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [displayMode]);

  return protection && displayMode && protection.enabled === (displayMode === "privacy") ? protection : null;
}
