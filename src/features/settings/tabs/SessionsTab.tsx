import { FileAudio, History, Search, Trash2 } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";

import { Button } from "@/components/ui/Button";
import { ConfirmDialog } from "@/components/ui/Dialog";
import { EmptyState } from "@/components/ui/EmptyState";
import { ErrorBanner } from "@/components/ui/ErrorBanner";
import { IconButton } from "@/components/ui/IconButton";
import { Input } from "@/components/ui/Input";
import { Select } from "@/components/ui/Select";
import { showErrorToast, showToast } from "@/components/ui/toast-store";
import { Tooltip } from "@/components/ui/Tooltip";
import { useDebouncedCallback } from "@/hooks/useDebouncedCallback";
import { bluey } from "@/lib/tauri/api";
import { toBlueyError, type BlueyError, type SessionListItem } from "@/lib/types";
import { cn } from "@/lib/utils/cn";
import { formatDateTime } from "@/lib/utils/format";
import { useModesStore } from "@/stores/modesStore";
import { useSessionStore } from "@/stores/sessionStore";
import { useSettingsStore } from "@/stores/settingsStore";
import { SessionDetail } from "../SessionDetail";
import { canTranscribeFiles, IMPORT_DISABLED_HINT, importRecording } from "../session-import";

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

const STATUS_LABEL: Record<SessionListItem["session"]["status"], string | null> = {
  active: "Live",
  paused: "Paused",
  completed: null,
};

export default function SessionsTab() {
  const modes = useModesStore((s) => s.modes);
  const activeSessionId = useSessionStore((s) => s.active?.id);
  const canImport = useSettingsStore((s) => canTranscribeFiles(s.settings));
  const [items, setItems] = useState<SessionListItem[]>([]);
  const [text, setText] = useState("");
  const [modeId, setModeId] = useState("");
  const [dateFilter, setDateFilter] = useState<DateFilter>("all");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [error, setError] = useState<BlueyError | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<SessionListItem | null>(null);
  const [importing, setImporting] = useState(false);

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
      setError(null);
    } catch (err) {
      setError(toBlueyError(err, "storage"));
    }
  }, []);

  const debouncedSearch = useDebouncedCallback(search, 300);

  useEffect(() => {
    void search({ text: "", modeId: "", dateFilter: "all" });
  }, [search]);

  // Keep the list fresh when a session starts / ends from the HUD (filters are read at call time).
  const filtersRef = useRef({ text, modeId, dateFilter });
  filtersRef.current = { text, modeId, dateFilter };
  useEffect(() => {
    void search(filtersRef.current);
  }, [activeSessionId, search]);

  const onFilterChange = (next: { text?: string; modeId?: string; dateFilter?: DateFilter }) => {
    const merged = { text, modeId, dateFilter, ...next };
    if (next.text !== undefined) setText(next.text);
    if (next.modeId !== undefined) setModeId(next.modeId);
    if (next.dateFilter !== undefined) setDateFilter(next.dateFilter);
    debouncedSearch(merged);
  };

  const deleteSession = async (item: SessionListItem) => {
    try {
      await bluey.session.delete({ id: item.session.id });
      showToast("Session deleted");
      await search({ text, modeId, dateFilter });
    } catch (err) {
      showErrorToast(toBlueyError(err, "storage"));
    }
  };

  const importNewRecording = async () => {
    setImporting(true);
    try {
      const result = await importRecording();
      if (result) {
        await search({ text, modeId, dateFilter });
        setSelectedId(result.session.id);
      }
    } finally {
      setImporting(false);
    }
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
          <Search
            className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-fg-subtle"
            aria-hidden
          />
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
        <Button
          variant="secondary"
          size="sm"
          disabled={importing || !canImport}
          title={canImport ? undefined : IMPORT_DISABLED_HINT}
          onClick={() => void importNewRecording()}
        >
          <FileAudio className="size-3.5" aria-hidden /> {importing ? "Transcribing…" : "Import recording…"}
        </Button>
      </div>

      {error ? (
        <ErrorBanner
          error={error}
          onRetry={() => void search({ text, modeId, dateFilter })}
          compact
          className="mb-3"
        />
      ) : null}

      <div className="min-h-0 flex-1 overflow-y-auto pb-10">
        {items.length === 0 ? (
          !error ? (
            <EmptyState
              icon={History}
              title="No sessions yet"
              description="Start an audio session from the HUD and it will show up here with its timeline, responses and summary."
            />
          ) : null
        ) : (
          <ul className="flex flex-col gap-1.5">
            {items.map((item) => {
              const statusLabel = STATUS_LABEL[item.session.status];
              const title = item.session.title ?? "Untitled session";
              return (
                <li key={item.session.id} className="group relative">
                  <button
                    type="button"
                    onClick={() => setSelectedId(item.session.id)}
                    className={cn(
                      "flex w-full items-center gap-4 rounded-card border border-transparent px-3.5 py-3 pr-12 text-left transition-colors",
                      "hover:border-border hover:bg-bg-elevated",
                    )}
                  >
                    <div className="min-w-0 flex-1">
                      <div className="flex items-center gap-2">
                        <span className="truncate text-[14px] font-medium text-fg">{title}</span>
                        {statusLabel ? (
                          <span
                            className={cn(
                              "rounded-full px-2 py-0.5 text-[11px] font-medium",
                              item.session.status === "active"
                                ? "bg-success/15 text-success"
                                : "bg-bg-tile text-fg-muted",
                            )}
                          >
                            {statusLabel}
                          </span>
                        ) : null}
                      </div>
                      <div className="mt-0.5 text-[12.5px] text-fg-muted">
                        {item.modeName} · {formatDateTime(item.session.startedAt)}
                      </div>
                    </div>
                    <div className="flex shrink-0 items-center gap-3 text-[12px] text-fg-subtle">
                      <span>{item.eventCount} events</span>
                      <span>{item.responseCount} responses</span>
                      {item.hasSummary ? (
                        <span className="rounded-full bg-bg-tile px-2 py-0.5 text-fg-muted">Summary</span>
                      ) : null}
                    </div>
                  </button>
                  <div className="absolute right-2 top-1/2 -translate-y-1/2 opacity-0 transition-opacity focus-within:opacity-100 group-hover:opacity-100">
                    <Tooltip label="Delete session">
                      <IconButton
                        aria-label={`Delete session ${title}`}
                        variant="plain"
                        onClick={() => setConfirmDelete(item)}
                        disabled={item.session.id === activeSessionId}
                      >
                        <Trash2 className="size-4 text-danger" aria-hidden />
                      </IconButton>
                    </Tooltip>
                  </div>
                </li>
              );
            })}
          </ul>
        )}
      </div>

      <ConfirmDialog
        open={confirmDelete !== null}
        onOpenChange={(open) => {
          if (!open) setConfirmDelete(null);
        }}
        title="Delete this session?"
        description="The timeline, responses and notes for this session will be removed. This cannot be undone."
        confirmLabel="Delete session"
        onConfirm={async () => {
          if (confirmDelete) await deleteSession(confirmDelete);
        }}
      />
    </div>
  );
}
