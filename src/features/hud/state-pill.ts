import type { EnginePhase } from "@/lib/engine-contract";
import type { AppStatus, BlueyError } from "@/lib/types";

export type PillContent =
  | { kind: "error"; error?: BlueyError }
  | { kind: "researching"; message: string }
  | { kind: "reading" }
  | { kind: "thinking" }
  | { kind: "prepared" }
  | { kind: "preparing" }
  | { kind: "listening" }
  | { kind: "idle"; modeName: string };

export interface PillSignals {
  /** A proactive preparation is running. */
  preparing?: boolean;
  /** Progress line of the deep-research job attached to the current ask. */
  researching?: string | null;
}

/** Pure pill derivation: error > researching > busy phase > prepared hint > preparing > listening > idle. */
export function derivePill(
  status: AppStatus | null,
  phase: EnginePhase | null,
  hasPrepared: boolean,
  modeName: string,
  signals: PillSignals | boolean = {},
): PillContent {
  const { preparing = false, researching = null } =
    typeof signals === "boolean" ? { preparing: signals } : signals;
  if (status?.state === "error") return { kind: "error", error: status.error };
  const busy =
    phase === "capturing" || phase === "analyzing" || phase === "thinking" || phase === "streaming";
  if (researching && busy) return { kind: "researching", message: researching };
  if (
    phase === "capturing" ||
    phase === "analyzing" ||
    status?.state === "capturing" ||
    status?.state === "analyzing"
  )
    return { kind: "reading" };
  if (phase === "thinking" || phase === "streaming" || status?.state === "thinking")
    return { kind: "thinking" };
  if (hasPrepared) return { kind: "prepared" };
  if (preparing) return { kind: "preparing" };
  if (status?.audioActive || status?.state === "listening") return { kind: "listening" };
  return { kind: "idle", modeName };
}
