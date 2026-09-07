import {
  AppWindow,
  ArrowDownToLine,
  ArrowUpToLine,
  MessageCircle,
  Mic,
  Settings,
  SquareChevronDown,
  SquareChevronLeft,
  SquareChevronRight,
  SquareChevronUp,
  SquarePlus,
  type LucideIcon,
} from "lucide-react";
import { useEffect, useState } from "react";

import { Button } from "@/components/ui/Button";
import { Keycaps } from "@/components/ui/Keycap";
import { SectionHeader } from "@/components/ui/SectionHeader";
import { bluey } from "@/lib/tauri/api";
import type { ShortcutBinding, ShortcutConflict, ShortcutId } from "@/lib/types";
import { cn } from "@/lib/utils/cn";
import { eventToAccelerator } from "@/lib/utils/keyboard";
import { useSettingsStore } from "@/stores/settingsStore";

const ICONS: Record<ShortcutId, LucideIcon> = {
  toggle_panel: AppWindow,
  capture_analyze: MessageCircle,
  generate_response: MessageCircle,
  new_chat: SquarePlus,
  open_settings: Settings,
  toggle_listening: Mic,
  move_up: SquareChevronUp,
  move_down: SquareChevronDown,
  move_left: SquareChevronLeft,
  move_right: SquareChevronRight,
  scroll_up: ArrowUpToLine,
  scroll_down: ArrowDownToLine,
};

const GROUPS: Array<{ id: ShortcutBinding["group"]; label: string }> = [
  { id: "general", label: "General" },
  { id: "window", label: "Window" },
  { id: "scroll", label: "Scroll" },
];

interface RowProps {
  binding: ShortcutBinding;
  recording: boolean;
  conflict: ShortcutConflict | null;
  onStartRecording: () => void;
}

function KeybindRow({ binding, recording, conflict, onStartRecording }: RowProps) {
  const Icon = ICONS[binding.id];
  return (
    <div className="flex flex-col">
      <button
        type="button"
        onClick={onStartRecording}
        aria-label={`Edit shortcut: ${binding.label}`}
        className={cn(
          "flex h-11 w-full items-center gap-3 rounded-[10px] px-2 text-left transition-colors",
          recording ? "bg-accent-soft/50" : "hover:bg-bg-hover",
        )}
      >
        <Icon className="size-5 shrink-0 text-fg-muted" strokeWidth={1.7} aria-hidden />
        <span className="min-w-0 flex-1 truncate text-[14px] text-fg">{binding.label}</span>
        {recording ? (
          <span className="text-[12.5px] font-medium text-accent">Press shortcut… (Esc cancels)</span>
        ) : (
          <Keycaps accelerator={binding.accelerator} />
        )}
      </button>
      {conflict ? (
        <div role="alert" className="mb-1 ml-10 mt-0.5 text-[12.5px] text-danger">
          {conflict.conflictsWith === "system" ? "Reserved by macOS — " : "Conflicts with another Bluey shortcut — "}
          {conflict.detail}
        </div>
      ) : null}
    </div>
  );
}

export default function KeybindsTab() {
  const shortcuts = useSettingsStore((s) => s.settings?.shortcuts ?? []);
  const [recordingId, setRecordingId] = useState<ShortcutId | null>(null);
  const [conflict, setConflict] = useState<{ id: ShortcutId; conflict: ShortcutConflict } | null>(null);

  useEffect(() => {
    if (!recordingId) return;
    const onKeyDown = (event: KeyboardEvent) => {
      event.preventDefault();
      event.stopPropagation();
      if (event.key === "Escape") {
        setRecordingId(null);
        return;
      }
      const accelerator = eventToAccelerator(event);
      if (!accelerator) return;
      void (async () => {
        try {
          const found = await bluey.shortcuts.checkConflict({ accelerator, ignoreId: recordingId });
          if (found) {
            setConflict({ id: recordingId, conflict: found });
            setRecordingId(null);
            return;
          }
          setConflict(null);
          await bluey.shortcuts.update({ id: recordingId, accelerator });
        } catch (error) {
          console.warn("[keybinds] update failed", error);
        } finally {
          setRecordingId(null);
        }
      })();
    };
    window.addEventListener("keydown", onKeyDown, { capture: true });
    return () => window.removeEventListener("keydown", onKeyDown, { capture: true });
  }, [recordingId]);

  return (
    <>
      <div className="flex items-end justify-between">
        <SectionHeader
          title="Keyboard shortcuts"
          description="Bluey works with these easy to remember commands. Click any of the keybinds to edit."
          className="mb-0"
        />
        <Button variant="ghost" size="sm" onClick={() => void bluey.shortcuts.reset()}>
          Reset all
        </Button>
      </div>

      {GROUPS.map((group) => {
        const bindings = shortcuts.filter((s) => s.group === group.id);
        if (bindings.length === 0) return null;
        return (
          <section key={group.id} className="mt-6">
            <h3 className="mb-2 text-[15px] font-semibold text-fg">{group.label}</h3>
            <div className="flex flex-col gap-0.5">
              {bindings.map((binding) => (
                <KeybindRow
                  key={binding.id}
                  binding={binding}
                  recording={recordingId === binding.id}
                  conflict={conflict?.id === binding.id ? conflict.conflict : null}
                  onStartRecording={() => {
                    setConflict(null);
                    setRecordingId(binding.id);
                  }}
                />
              ))}
            </div>
          </section>
        );
      })}
    </>
  );
}
