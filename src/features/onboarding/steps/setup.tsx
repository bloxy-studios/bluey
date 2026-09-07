import { Check } from "lucide-react";
import { useEffect, useState } from "react";

import { Keycaps } from "@/components/ui/Keycap";
import { LucideIcon } from "@/components/ui/LucideIcon";
import { bluey } from "@/lib/tauri/api";
import type { ShortcutId } from "@/lib/types";
import { cn } from "@/lib/utils/cn";
import { eventToAccelerator } from "@/lib/utils/keyboard";
import { useModesStore } from "@/stores/modesStore";
import { useSettingsStore } from "@/stores/settingsStore";
import type { StepProps } from "../OnboardingFlow";
import { StepShell } from "./basics";

export function DefaultModeStep(_props: StepProps) {
  const modes = useModesStore((s) => s.modes);
  const defaultModeId = useSettingsStore((s) => s.settings?.general.defaultModeId);

  return (
    <StepShell title="Choose your default mode" body="Modes shape how Bluey answers. Pick the one you'll use most — you can switch anytime from the HUD.">
      <div className="grid max-h-[260px] grid-cols-2 gap-2 overflow-y-auto pr-1">
        {modes.map((mode) => {
          const selected = mode.id === defaultModeId;
          return (
            <button
              key={mode.id}
              type="button"
              onClick={() => void bluey.modes.setDefault({ id: mode.id })}
              className={cn(
                "flex items-center gap-3 rounded-card border px-3.5 py-3 text-left transition-colors",
                selected ? "border-accent bg-accent-soft/50" : "border-border bg-bg-elevated hover:bg-bg-hover",
              )}
            >
              <span className="flex size-9 shrink-0 items-center justify-center rounded-[10px] bg-bg-tile">
                <LucideIcon name={mode.icon} className="size-4 text-fg-muted" strokeWidth={1.8} />
              </span>
              <span className="min-w-0 flex-1 truncate text-[13.5px] font-medium text-fg">{mode.name}</span>
              {selected ? <Check className="size-4 shrink-0 text-accent" aria-hidden /> : null}
            </button>
          );
        })}
      </div>
    </StepShell>
  );
}

const ONBOARDING_SHORTCUTS: ShortcutId[] = ["toggle_panel", "capture_analyze", "generate_response", "new_chat", "toggle_listening"];

export function ShortcutsStep(_props: StepProps) {
  const shortcuts = useSettingsStore((s) => s.settings?.shortcuts ?? []);
  const [recordingId, setRecordingId] = useState<ShortcutId | null>(null);
  const [warning, setWarning] = useState<string | null>(null);

  useEffect(() => {
    if (!recordingId) return;
    const onKeyDown = (event: KeyboardEvent) => {
      event.preventDefault();
      if (event.key === "Escape") {
        setRecordingId(null);
        return;
      }
      const accelerator = eventToAccelerator(event);
      if (!accelerator) return;
      void (async () => {
        const conflict = await bluey.shortcuts.checkConflict({ accelerator, ignoreId: recordingId });
        if (conflict) {
          setWarning(conflict.detail);
        } else {
          setWarning(null);
          await bluey.shortcuts.update({ id: recordingId, accelerator });
        }
        setRecordingId(null);
      })();
    };
    window.addEventListener("keydown", onKeyDown, { capture: true });
    return () => window.removeEventListener("keydown", onKeyDown, { capture: true });
  }, [recordingId]);

  return (
    <StepShell title="Your shortcuts" body="These work anywhere on your Mac. Click one to remap it.">
      <div className="flex flex-col gap-1">
        {shortcuts
          .filter((s) => ONBOARDING_SHORTCUTS.includes(s.id))
          .map((binding) => (
            <button
              key={binding.id}
              type="button"
              onClick={() => setRecordingId(binding.id)}
              className={cn(
                "flex h-11 items-center justify-between rounded-[10px] px-3.5 transition-colors",
                recordingId === binding.id ? "bg-accent-soft/50" : "bg-bg-elevated hover:bg-bg-hover",
              )}
            >
              <span className="text-[13.5px] text-fg">{binding.label}</span>
              {recordingId === binding.id ? (
                <span className="text-[12.5px] font-medium text-accent">Press shortcut…</span>
              ) : (
                <Keycaps accelerator={binding.accelerator} />
              )}
            </button>
          ))}
      </div>
      {warning ? (
        <p role="alert" className="mt-2 text-[12.5px] text-danger">
          {warning}
        </p>
      ) : null}
    </StepShell>
  );
}
