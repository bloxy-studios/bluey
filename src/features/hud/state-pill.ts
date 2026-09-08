import type { EnginePhase } from "@/lib/engine-contract";
import type { AppStatus, BlueyError } from "@/lib/types";

export type PillContent =
  | { kind: "error"; error?: BlueyError }
  | { kind: "reading" }
  | { kind: "thinking" }
  | { kind: "prepared" }
  | { kind: "preparing" }
  | { kind: "listening" }
  | { kind: "idle"; modeName: string };

/** Pure pill derivation: error > busy phase > prepared hint > preparing > listening > idle. */
export function derivePill(
  status: AppStatus | null,
  phase: EnginePhase | null,
  hasPrepared: boolean,
  modeName: string,
  preparing = false,
): PillContent {
  if (status?.state === "error") return { kind: "error", error: status.error };
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
