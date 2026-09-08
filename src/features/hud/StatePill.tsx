import { X } from "lucide-react";

import { Pill } from "@/components/ui/Pill";
import { Spinner } from "@/components/ui/Spinner";
import { showErrorToast } from "@/components/ui/toast-store";
import { presentError } from "@/lib/errors/present";
import { bluey } from "@/lib/tauri/api";
import { toBlueyError } from "@/lib/types";
import { useAppStore } from "@/stores/appStore";
import { useChatStore } from "@/stores/chatStore";
import { modeById, useModesStore } from "@/stores/modesStore";
import { useProactiveStore } from "@/stores/proactive";
import { useResearchStore } from "@/stores/researchStore";
import { derivePill } from "./state-pill";

export interface StatePillProps {
  /** Runs when the error's recovery is `retry` (re-asks the last question). */
  onRetry?: () => void;
}

/** Returns the state machine to ready/listening after the user handled the error. */
async function recover(): Promise<void> {
  try {
    await bluey.app.recover();
  } catch (error) {
    showErrorToast(toBlueyError(error));
  }
}

/** The HUD's left status pill: `Bluey · General` / `● Listening` / `◌ Thinking` / `! <error>` + recovery. */
export function StatePill({ onRetry }: StatePillProps = {}) {
  const status = useAppStore((s) => s.status);
  const phase = useChatStore((s) => s.phase);
  const prepared = useChatStore((s) => s.prepared);
  const preparing = useProactiveStore((s) => s.preparingEventId !== null);
  const researching = useResearchStore((s) => s.active?.message ?? null);
  const modes = useModesStore((s) => s.modes);
  const modeName = modeById(modes, status?.modeId)?.name ?? "General";

  const pill = derivePill(status, phase, prepared !== null, modeName, { preparing, researching });

  switch (pill.kind) {
    case "researching":
      return (
        <Pill variant="hud" title={pill.message}>
          <Spinner size={11} /> Researching
        </Pill>
      );
    case "error": {
      const presented = pill.error ? presentError(pill.error, { onRetry }) : null;
      return (
        <div className="flex min-w-0 items-center gap-1.5">
          <Pill variant="hud" className="min-w-0 text-danger" title={presented?.message}>
            <span aria-hidden>!</span>
            <span className="truncate">{presented?.title ?? "Something went wrong"}</span>
          </Pill>
          {presented?.action ? (
            <button
              type="button"
              className="shrink-0 rounded-full bg-hud-chip px-2.5 py-1 text-[12px] font-medium text-fg hover:bg-white/15"
              onClick={() => {
                void (async () => {
                  await presented.action?.();
                  await recover();
                })();
              }}
            >
              {presented.actionLabel}
            </button>
          ) : null}
          <button
            type="button"
            aria-label="Dismiss error"
            className="flex size-6 shrink-0 items-center justify-center rounded-full text-fg-muted hover:bg-white/10 hover:text-fg"
            onClick={() => void recover()}
          >
            <X className="size-3.5" aria-hidden />
          </button>
        </div>
      );
    }
    case "reading":
      return (
        <Pill variant="hud">
          <Spinner size={11} /> Reading screen
        </Pill>
      );
    case "thinking":
      return (
        <Pill variant="hud">
          <Spinner size={11} /> Thinking
        </Pill>
      );
    case "prepared":
      return (
        <Pill variant="accent" className="motion-safe:animate-fade-in">
          Bluey has a suggestion · ⌘⇧↵
        </Pill>
      );
    case "preparing":
      return (
        <Pill variant="hud">
          <Spinner size={11} /> Preparing a suggestion
        </Pill>
      );
    case "listening":
      return (
        <Pill variant="hud">
          <span className="size-[7px] rounded-full bg-success motion-safe:animate-pulse-dot" aria-hidden />
          Listening
        </Pill>
      );
    case "idle":
      return <Pill variant="hud">Bluey · {pill.modeName}</Pill>;
  }
}
