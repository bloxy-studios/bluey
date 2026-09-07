import { History, Search } from "lucide-react";
import { useCallback, useEffect, useState } from "react";

import { EmptyState } from "@/components/ui/EmptyState";
import { Input } from "@/components/ui/Input";
import { Select } from "@/components/ui/Select";
import { useDebouncedCallback } from "@/hooks/useDebouncedCallback";
import { bluey } from "@/lib/tauri/api";
import type { SessionListItem } from "@/lib/types";
import { cn } from "@/lib/utils/cn";
import { formatDateTime } from "@/lib/utils/format";
import { useModesStore } from "@/stores/modesStore";
import { SessionDetail } from "../SessionDetail";

type DateFilter = "all" | "today" | "week";

function dateFrom(filter: DateFilter): string | undefined {
  const now = new Date();
  if (filter === "today") {
    now.setHours(0, 0, 0, 0);
    return now.toISOString();
  }
  if (filter === "week") return new Date(now.getTime() - 7 * 86_400_000).toISOString();
  return undefined;
}

export default function SessionsTab() {
  const modes = useModesStore((s) => s.modes);
  const [items, setItems] = useState<SessionListItem[]>([]);
  const [text, setText] = useState("");
  const [modeId, setModeId] = useState("");
  const [dateFilter, setDateFilter] = useState<DateFilter>("all");
  const [selectedId, setSelectedId] = useState<string | null>(null);

  const search = useCallback(async (query: { text: string; modeId: string; dateFilter: DateFilter }) => {
    try {
      const result = await bluey.session.search({
        query: {
          text: query.text.trim() || undefined,
          modeId: query.modeId || undefined,
          from: dateFrom(query.dateFilter),
        },
      });
      setItems(result);
    } catch (error) {
      console.warn("[sessions] search failed", error);
    }
  }, []);

  const debouncedSearch = useDebouncedCallback(search, 300);

  useEffect(() => {
    void search({ text: "", modeId: "", dateFilter: "all" });
  }, [search]);

  const onFilterChange = (next: { text?: string; modeId?: string; dateFilter?: DateFilter }) => {
    const merged = { text, modeId, dateFilter, ...next };
    if (next.text !== undefined) setText(next.text);
    if (next.modeId !== undefined) setModeId(next.modeId);
    if (next.dateFilter !== undefined) setDateFilter(next.dateFilter);
    debouncedSearch(merged);
  };

  if (selectedId) {
    return (
      <SessionDetail
        sessionId={selectedId}
        onBack={() => {
          setSelectedId(null);
          void search({ text, modeId, dateFilter });
        }}
      />
    );
  }

  return (
    <div className="mx-auto flex min-h-0 w-full max-w-[800px] flex-1 flex-col px-6">
      <div className="flex shrink-0 items-center gap-2.5 pb-3 pt-1">
        <div className="relative flex-1">
          <Search className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-fg-subtle" aria-hidden />
          <Input
            value={text}
            onChange={(e) => onFilterChange({ text: e.target.value })}
            placeholder="Search sessions"
            aria-label="Search sessions"
            className="w-full pl-9"
          />
        </div>
        <Select
          aria-label="Filter by mode"
          value={modeId}
          onChange={(e) => onFilterChange({ modeId: e.target.value })}
          options={[{ value: "", label: "All modes" }, ...modes.map((m) => ({ value: m.id, label: m.name }))]}
        />
        <Select
          aria-label="Filter by date"
          value={dateFilter}
          onChange={(e) => onFilterChange({ dateFilter: e.target.value as DateFilter })}
          options={[
            { value: "all", label: "Any time" },
            { value: "today", label: "Today" },
            { value: "week", label: "Past week" },
          ]}
        />
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto pb-10">
        {items.length === 0 ? (
          <EmptyState
            icon={History}
            title="No sessions yet"
            description="Start an audio session from the HUD and it will show up here with its timeline, responses and summary."
          />
        ) : (
          <ul className="flex flex-col gap-1.5">
            {items.map((item) => (
              <li key={item.session.id}>
                <button
                  type="button"
                  onClick={() => setSelectedId(item.session.id)}
                  className={cn(
                    "flex w-full items-center gap-4 rounded-card border border-transparent px-3.5 py-3 text-left transition-colors",
                    "hover:border-border hover:bg-bg-elevated",
                  )}
                >
                  <div className="min-w-0 flex-1">
                    <div className="truncate text-[14px] font-medium text-fg">{item.session.title ?? "Untitled session"}</div>
                    <div className="mt-0.5 text-[12.5px] text-fg-muted">
                      {item.modeName} · {formatDateTime(item.session.startedAt)}
                    </div>
                  </div>
                  <div className="flex shrink-0 items-center gap-3 text-[12px] text-fg-subtle">
                    <span>{item.eventCount} events</span>
                    <span>{item.responseCount} responses</span>
                    {item.hasSummary ? <span className="rounded-full bg-bg-tile px-2 py-0.5 text-fg-muted">Summary</span> : null}
                  </div>
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}
