import type { EnginePhase } from "@/lib/engine-contract";
import type { AppStatus } from "@/lib/types";

export type PillContent =
  | { kind: "error" }
  | { kind: "reading" }
  | { kind: "thinking" }
  | { kind: "prepared" }
  | { kind: "listening" }
  | { kind: "idle"; modeName: string };

/** Pure pill derivation: error > busy phase > prepared hint > listening > idle. */
export function derivePill(
  status: AppStatus | null,
  phase: EnginePhase | null,
  hasPrepared: boolean,
  modeName: string,
): PillContent {
  if (status?.state === "error") return { kind: "error" };
  if (phase === "capturing" || phase === "analyzing" || status?.state === "capturing" || status?.state === "analyzing")
    return { kind: "reading" };
  if (phase === "thinking" || phase === "streaming" || status?.state === "thinking") return { kind: "thinking" };
  if (hasPrepared) return { kind: "prepared" };
  if (status?.audioActive || status?.state === "listening") return { kind: "listening" };
  return { kind: "idle", modeName };
}
