import { X } from "lucide-react";

import { Pill } from "@/components/ui/Pill";
import { Spinner } from "@/components/ui/Spinner";
import { showErrorToast } from "@/components/ui/toast-store";
import { presentError, runRecovery } from "@/lib/errors/present";
import { bluey } from "@/lib/tauri/api";
import { toBlueyError } from "@/lib/types";
import { acceleratorToGlyphs } from "@/lib/utils/keyboard";
import { useUpdatesStore } from "@/stores/updatesStore";
import { useShortcutAccelerator } from "./useShortcutAccelerator";
import { useStatePill } from "./useStatePill";

export interface StatePillProps {
  /** Runs when the error's recovery is `retry` (re-asks the last question). */
  onRetry?: () => void;
  /** Shows the prepared answer (the "Bluey has a suggestion" pill is a button, UX-026). */
  onTakePrepared?: () => void;
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
export function StatePill({ onRetry, onTakePrepared }: StatePillProps = {}) {
  const pill = useStatePill();
  const generateKeys = useShortcutAccelerator("generate_response");

  switch (pill.kind) {
    case "update": {
      // Available → Install; ready → relaunch; downloading is informational.
      const act =
        pill.phase === "ready"
          ? () => void useUpdatesStore.getState().relaunch()
          : pill.phase === "available"
            ? () => void useUpdatesStore.getState().install()
            : null;
      return (
        <button
          type="button"
          className="flex min-w-0 shrink items-center disabled:cursor-default"
          onClick={act ?? undefined}
          disabled={!act}
          aria-label={pill.label}
          title={pill.label}
        >
          <Pill
            variant="accent"
            interactive={act !== null}
            className="min-w-0 motion-safe:animate-fade-in"
          >
            {pill.phase === "downloading" ? <Spinner size={11} decorative /> : null}
            <span className="truncate">{pill.label}</span>
          </Pill>
        </button>
      );
    }
    case "researching":
      return (
        <Pill variant="hud" className="min-w-0" title={pill.message}>
          <Spinner size={11} decorative /> <span className="truncate">Researching</span>
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
              className="shrink-0 rounded-full bg-hud-chip px-2.5 py-1 text-[12px] font-medium text-fg hover:bg-fg/15"
              onClick={() => {
                void (async () => {
                  // The pill clears even when the recovery fails; the runner shows that failure.
                  if (presented.action) await runRecovery(presented.action);
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
            className="flex size-6 shrink-0 items-center justify-center rounded-full text-fg-muted hover:bg-fg/10 hover:text-fg"
            onClick={() => void recover()}
          >
            <X className="size-3.5" aria-hidden />
          </button>
        </div>
      );
    }
    case "reading":
      return (
        <Pill variant="hud" className="min-w-0" title="Reading screen">
          <Spinner size={11} decorative /> <span className="truncate">Reading screen</span>
        </Pill>
      );
    case "thinking":
      return (
        <Pill variant="hud" className="min-w-0" title="Thinking">
          <Spinner size={11} decorative /> <span className="truncate">Thinking</span>
        </Pill>
      );
    case "prepared": {
      const hint = generateKeys ? acceleratorToGlyphs(generateKeys).join("") : null;
      const label = hint ? `Bluey has a suggestion · ${hint}` : "Bluey has a suggestion";
      return (
        <button
          type="button"
          className="flex min-w-0 shrink items-center disabled:cursor-default"
          onClick={() => onTakePrepared?.()}
          disabled={!onTakePrepared}
          aria-label="Show Bluey's suggestion"
          title={label}
        >
          <Pill
            variant="accent"
            interactive={Boolean(onTakePrepared)}
            className="min-w-0 motion-safe:animate-fade-in"
          >
            <span className="truncate">{label}</span>
          </Pill>
        </button>
      );
    }
    case "preparing":
      return (
        <Pill variant="hud" className="min-w-0" title="Preparing a suggestion">
          <Spinner size={11} decorative /> <span className="truncate">Preparing a suggestion</span>
        </Pill>
      );
    case "listening":
      return (
        <Pill variant="hud" className="min-w-0" title="Listening">
          <span
            className="size-[7px] shrink-0 rounded-full bg-success motion-safe:animate-pulse-dot"
            aria-hidden
          />
          <span className="truncate">Listening</span>
        </Pill>
      );
    case "idle":
      return (
        <Pill variant="hud" className="min-w-0" title={`Bluey · ${pill.modeName}`}>
          <span className="truncate">Bluey · {pill.modeName}</span>
        </Pill>
      );
  }
}
