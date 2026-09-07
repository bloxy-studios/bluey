import { Check, FileText, Plus } from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import { LucideIcon } from "@/components/ui/LucideIcon";
import { bluey } from "@/lib/tauri/api";
import type { BlueyMode } from "@/lib/types";
import { cn } from "@/lib/utils/cn";
import { useAppStore } from "@/stores/appStore";
import { useModesStore } from "@/stores/modesStore";
import { ModeEditor } from "../ModeEditor";

function ModeRow({ mode, selected, active, onSelect }: { mode: BlueyMode; selected: boolean; active: boolean; onSelect: () => void }) {
  return (
    <button
      type="button"
      onClick={onSelect}
      aria-current={selected ? "true" : undefined}
      className={cn(
        "flex w-full items-center gap-3 rounded-card px-3 py-2.5 text-left transition-colors",
        selected ? "bg-bg-elevated" : "hover:bg-bg-hover",
      )}
    >
      <span className="flex size-9 shrink-0 items-center justify-center rounded-[10px] bg-bg-tile">
        {mode.id === "general" ? (
          <FileText className="size-4 text-fg-muted" strokeWidth={1.8} aria-hidden />
        ) : (
          <LucideIcon name={mode.icon} className="size-4 text-fg-muted" strokeWidth={1.8} />
        )}
      </span>
      <span className="min-w-0 flex-1 truncate text-[15px] text-fg">{mode.name}</span>
      {active ? (
        <span className="flex size-5 shrink-0 items-center justify-center rounded-full bg-accent" aria-label="Active mode">
          <Check className="size-3 text-white" strokeWidth={3} aria-hidden />
        </span>
      ) : null}
    </button>
  );
}

function GroupLabel({ label }: { label: string }) {
  return (
    <div className="mt-3 mb-1 flex items-center gap-2 px-3">
      <span className="text-[11px] font-medium text-fg-subtle">{label}</span>
      <span className="h-px flex-1 bg-border" aria-hidden />
    </div>
  );
}

export default function ModesTab() {
  const modes = useModesStore((s) => s.modes);
  const activeModeId = useAppStore((s) => s.status?.modeId);
  const [selectedId, setSelectedId] = useState<string | null>(null);

  useEffect(() => {
    if (!selectedId && modes.length > 0) setSelectedId(activeModeId ?? modes[0]?.id ?? null);
  }, [modes, activeModeId, selectedId]);

  const selected = modes.find((m) => m.id === selectedId) ?? null;

  const groups = useMemo(() => {
    const general = modes.filter((m) => m.id === "general");
    const grouped = new Map<string, BlueyMode[]>();
    const custom: BlueyMode[] = [];
    for (const mode of modes) {
      if (mode.id === "general") continue;
      if (mode.group) {
        const list = grouped.get(mode.group) ?? [];
        list.push(mode);
        grouped.set(mode.group, list);
      } else {
        custom.push(mode);
      }
    }
    return { general, grouped, custom };
  }, [modes]);

  const createMode = async () => {
    try {
      const created = await bluey.modes.create({ draft: { name: "Untitled Mode" } });
      setSelectedId(created.id);
    } catch (error) {
      console.warn("[modes] create failed", error);
    }
  };

  return (
    <div className="flex min-h-0 w-full flex-1">
      <aside className="flex w-[250px] shrink-0 flex-col gap-1 overflow-y-auto border-r border-border p-3">
        <button
          type="button"
          onClick={() => void createMode()}
          className="flex w-full items-center gap-3 rounded-card bg-accent-soft/45 px-3 py-2.5 text-left transition-colors hover:bg-accent-soft/70"
        >
          <span className="flex size-9 shrink-0 items-center justify-center rounded-[10px] bg-accent-soft">
            <Plus className="size-4 text-accent" strokeWidth={2.2} aria-hidden />
          </span>
          <span className="text-[15px] font-medium text-accent">New Mode</span>
        </button>

        {groups.general.map((mode) => (
          <ModeRow
            key={mode.id}
            mode={mode}
            selected={mode.id === selectedId}
            active={mode.id === activeModeId}
            onSelect={() => setSelectedId(mode.id)}
          />
        ))}

        {Array.from(groups.grouped.entries()).map(([group, groupModes]) => (
          <div key={group}>
            <GroupLabel label={group} />
            <div className="flex flex-col gap-1">
              {groupModes.map((mode) => (
                <ModeRow
                  key={mode.id}
                  mode={mode}
                  selected={mode.id === selectedId}
                  active={mode.id === activeModeId}
                  onSelect={() => setSelectedId(mode.id)}
                />
              ))}
            </div>
          </div>
        ))}

        {groups.custom.length > 0 ? (
          <div>
            <GroupLabel label="Your modes" />
            <div className="flex flex-col gap-1">
              {groups.custom.map((mode) => (
                <ModeRow
                  key={mode.id}
                  mode={mode}
                  selected={mode.id === selectedId}
                  active={mode.id === activeModeId}
                  onSelect={() => setSelectedId(mode.id)}
                />
              ))}
            </div>
          </div>
        ) : null}
      </aside>

      {selected ? (
        <ModeEditor
          key={selected.id}
          mode={selected}
          isActive={selected.id === activeModeId}
          onDeleted={() => setSelectedId("general")}
        />
      ) : (
        <div className="flex flex-1 items-center justify-center text-[13px] text-fg-muted">Select a mode</div>
      )}
    </div>
  );
}
