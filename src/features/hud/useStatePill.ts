import { useAppStore } from "@/stores/appStore";
import { useChatStore } from "@/stores/chatStore";
import { modeById, useModesStore } from "@/stores/modesStore";
import { useProactiveStore } from "@/stores/proactive";
import { useResearchStore } from "@/stores/researchStore";
import { useUpdatesStore } from "@/stores/updatesStore";
import { derivePill, type PillContent } from "./state-pill";

/** What the status pill shows now (shared with the screen-reader announcer, UX-025). */
export function useStatePill(): PillContent {
  const status = useAppStore((s) => s.status);
  const phase = useChatStore((s) => s.phase);
  const prepared = useChatStore((s) => s.prepared);
  const preparing = useProactiveStore((s) => s.preparingEventId !== null);
  const researching = useResearchStore((s) => s.active?.message ?? null);
  const update = useUpdatesStore((s) => s.status);
  const modes = useModesStore((s) => s.modes);
  const modeName = modeById(modes, status?.modeId)?.name ?? "General";

  return derivePill(status, phase, prepared !== null, modeName, { preparing, researching, update });
}
