import { useCallback, useEffect, useState } from "react";

import { bluey } from "@/lib/tauri/api";
import { toBlueyError, type AiReadiness } from "@/lib/types";
import { useSettingsStore } from "@/stores/settingsStore";

/**
 * Whether an answer routes right now (Rust `ai_readiness`: the router against the real provider
 * state). Re-asked whenever settings change (keys, roles, enabled flags, Cloud AI) and when the
 * window regains focus (an account signed in elsewhere). `null` until the first answer.
 */
export function useAiReadiness(): { readiness: AiReadiness | null; refresh: () => void } {
  const settings = useSettingsStore((s) => s.settings);
  const [readiness, setReadiness] = useState<AiReadiness | null>(null);
  const [tick, setTick] = useState(0);
  const refresh = useCallback(() => setTick((n) => n + 1), []);

  useEffect(() => {
    let live = true;
    bluey.ai
      .readiness()
      .then((next) => {
        if (live) setReadiness(next);
      })
      .catch((error: unknown) => {
        // Never claim ready on a failed check: the error is the cause.
        if (live) setReadiness({ ok: false, vision: false, error: toBlueyError(error, "ai") });
      });
    return () => {
      live = false;
    };
  }, [settings, tick]);

  useEffect(() => {
    window.addEventListener("focus", refresh);
    return () => window.removeEventListener("focus", refresh);
  }, [refresh]);

  return { readiness, refresh };
}
