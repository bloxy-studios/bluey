import type { EnginePhase } from "@/lib/engine-contract";
import type { AppStatus, BlueyError, UpdateStatus } from "@/lib/types";
import { updatePillLabel } from "@/lib/updates/describe";

export type PillContent =
  | { kind: "error"; error?: BlueyError }
  | { kind: "researching"; message: string }
  | { kind: "reading" }
  | { kind: "thinking" }
  | { kind: "prepared" }
  | { kind: "preparing" }
  | { kind: "listening" }
  /** An update is available, downloading or installed (docs/UPDATES.md); `label` is the pill text. */
  | { kind: "update"; phase: "available" | "downloading" | "ready"; label: string }
  | { kind: "idle"; modeName: string };

export interface PillSignals {
  /** A proactive preparation is running. */
  preparing?: boolean;
  /** Progress line of the deep-research job attached to the current ask. */
  researching?: string | null;
  /** The in-app updater's status; only shown when the HUD is otherwise idle. */
  update?: UpdateStatus | null;
}

/**
 * Pure pill derivation: error > researching > busy phase > prepared hint >
 * preparing > listening > update > idle. An update never interrupts work in
 * progress or a live session — it waits for the idle pill.
 */
export function derivePill(
  status: AppStatus | null,
  phase: EnginePhase | null,
  hasPrepared: boolean,
  modeName: string,
  signals: PillSignals | boolean = {},
): PillContent {
  const {
    preparing = false,
    researching = null,
    update = null,
  } = typeof signals === "boolean" ? { preparing: signals } : signals;
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
  if (
    update &&
    (update.phase === "available" || update.phase === "downloading" || update.phase === "ready")
  ) {
    const label = updatePillLabel(update);
    if (label) return { kind: "update", phase: update.phase, label };
  }
  return { kind: "idle", modeName };
}
