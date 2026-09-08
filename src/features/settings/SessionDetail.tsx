import { ArrowLeft, ChevronRight, Download, Pencil, Plus, Trash2 } from "lucide-react";
import { useCallback, useEffect, useState, type KeyboardEvent } from "react";

import { Button } from "@/components/ui/Button";
import { ConfirmDialog } from "@/components/ui/Dialog";
import { ErrorBanner } from "@/components/ui/ErrorBanner";
import { IconButton } from "@/components/ui/IconButton";
import { Input } from "@/components/ui/Input";
import { Spinner } from "@/components/ui/Spinner";
import { showErrorToast, showToast } from "@/components/ui/toast-store";
import { ResponseView } from "@/features/hud/ResponseView";
import { bluey } from "@/lib/tauri/api";
import {
  toBlueyError,
  type BlueyError,
  type SessionDetail as SessionDetailData,
  type SessionEvent,
} from "@/lib/types";
import { cn } from "@/lib/utils/cn";
import { copyText } from "@/lib/utils/clipboard";
import { formatDateTime, formatDuration, formatTime } from "@/lib/utils/format";
import { getEngine } from "@/stores/engine";
import { modeById, useModesStore } from "@/stores/modesStore";
import { useSettingsStore } from "@/stores/settingsStore";

function TimelineEvent({ event }: { event: SessionEvent }) {
  const [open, setOpen] = useState(false);
  const expandable = Boolean(event.detail || (event.refs && Object.keys(event.refs).length > 0));
  return (
    <li className="relative pl-6">
      <span className="absolute left-[7px] top-[13px] size-[7px] rounded-full bg-border-strong" aria-hidden />
      <button
        type="button"
        disabled={!expandable}
        onClick={() => setOpen((v) => !v)}
        className={cn(
          "flex w-full items-center gap-2 rounded-[8px] px-2 py-1.5 text-left",
          expandable && "transition-colors hover:bg-bg-hover",
        )}
        aria-expanded={expandable ? open : undefined}
      >
        <span className="w-[52px] shrink-0 font-mono text-[11px] text-fg-subtle">
          {formatTime(event.createdAt)}
        </span>
        <span className="flex-1 text-[13px] text-fg">{event.title}</span>
        {expandable ? (
          <ChevronRight
            className={cn("size-3.5 text-fg-subtle transition-transform", open && "rotate-90")}
            aria-hidden
          />
        ) : null}
      </button>
      {open ? (
        <div className="mb-1 ml-[60px] rounded-[8px] border border-border bg-bg-elevated px-3 py-2 text-[12.5px] leading-relaxed text-fg-muted">
          {event.detail ? <p className="m-0">{event.detail}</p> : null}
          {event.refs ? (
            <div className="mt-1 flex flex-wrap gap-1.5">
              {Object.entries(event.refs).map(([key, value]) => (
                <span
                  key={key}
                  className="rounded bg-bg-tile px-1.5 py-0.5 font-mono text-[10.5px] text-fg-subtle"
                >
                  {key}: {value}
                </span>
              ))}
            </div>
          ) : null}
          {event.confidence !== undefined ? (
            <div className="mt-1 text-[11px] text-fg-subtle">
              confidence {(event.confidence * 100).toFixed(0)}%
            </div>
          ) : null}
        </div>
      ) : null}
    </li>
  );
}

export function SessionDetail({ sessionId, onBack }: { sessionId: string; onBack: () => void }) {
  const modes = useModesStore((s) => s.modes);
  const settings = useSettingsStore((s) => s.settings);
  const [detail, setDetail] = useState<SessionDetailData | null>(null);
  const [loadError, setLoadError] = useState<BlueyError | null>(null);
  const [note, setNote] = useState("");
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [summarizing, setSummarizing] = useState(false);
  const [summaryError, setSummaryError] = useState<BlueyError | null>(null);
  const [editingTitle, setEditingTitle] = useState(false);
  const [titleDraft, setTitleDraft] = useState("");

  const load = useCallback(async () => {
    try {
      setDetail(await bluey.session.get({ id: sessionId }));
      setLoadError(null);
    } catch (error) {
      setLoadError(toBlueyError(error, "storage"));
    }
  }, [sessionId]);

  useEffect(() => {
    void load();
  }, [load]);

  if (!detail) {
    return (
      <div className="mx-auto flex w-full max-w-[800px] flex-col px-6 pt-1">
        <div className="flex items-center gap-3 pb-3">
          <IconButton aria-label="Back to sessions" variant="circle" onClick={onBack}>
            <ArrowLeft className="size-4" aria-hidden />
          </IconButton>
        </div>
        {loadError ? (
          <ErrorBanner error={loadError} onRetry={() => void load()} />
        ) : (
          <div className="flex w-full items-center justify-center py-20">
            <Spinner size={16} />
          </div>
        )}
      </div>
    );
  }

  const { session, events, notes, responses, summary } = detail;
  const mode = modeById(modes, session.modeId);
  const title = session.title ?? "Untitled session";

  const startRename = () => {
    setTitleDraft(session.title ?? "");
    setEditingTitle(true);
  };

  const commitRename = async () => {
    const next = titleDraft.trim();
    setEditingTitle(false);
    if (!next || next === session.title) return;
    try {
      await bluey.session.rename({ id: sessionId, title: next });
      await load();
    } catch (error) {
      showErrorToast(toBlueyError(error, "storage"));
    }
  };

  const onTitleKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Enter") void commitRename();
    if (event.key === "Escape") setEditingTitle(false);
  };

  const addNote = async () => {
    const content = note.trim();
    if (!content) return;
    try {
      await bluey.session.addNote({ sessionId, content });
      setNote("");
      await load();
    } catch (error) {
      showErrorToast(toBlueyError(error, "storage"));
    }
  };

  const exportMarkdown = async () => {
    try {
      const markdown = await bluey.data.exportSession({ sessionId, format: "markdown" });
      if (await copyText(markdown)) showToast("Markdown copied");
    } catch (error) {
      showErrorToast(toBlueyError(error, "storage"));
    }
  };

  const generateSummary = async () => {
    if (!settings || !mode) return;
    setSummarizing(true);
    setSummaryError(null);
    try {
      const transcript = await bluey.transcript.list({ sessionId });
      const generated = await getEngine().summarizeSession({
        session,
        mode,
        transcript,
        responses,
        events,
        notes,
        settings,
      });
      const { id: _id, createdAt: _createdAt, ...rest } = generated;
      await bluey.session.saveSummary({ summary: { ...rest, sessionId, modeId: session.modeId } });
      await load();
    } catch (error) {
      setSummaryError(toBlueyError(error, "ai"));
    } finally {
      setSummarizing(false);
    }
  };

  return (
    <div className="mx-auto flex min-h-0 w-full max-w-[800px] flex-1 flex-col px-6">
      <div className="flex shrink-0 items-center gap-3 pb-3 pt-1">
        <IconButton aria-label="Back to sessions" variant="circle" onClick={onBack}>
          <ArrowLeft className="size-4" aria-hidden />
        </IconButton>
        <div className="min-w-0 flex-1">
          {editingTitle ? (
            <Input
              autoFocus
              value={titleDraft}
              onChange={(e) => setTitleDraft(e.target.value)}
              onKeyDown={onTitleKeyDown}
              onBlur={() => void commitRename()}
              aria-label="Session title"
              placeholder="Session title"
              className="w-full max-w-[420px]"
            />
          ) : (
            <div className="flex min-w-0 items-center gap-1.5">
              <h2 className="truncate text-[16px] font-semibold text-fg">{title}</h2>
              <IconButton aria-label="Rename session" variant="plain" size="sm" onClick={startRename}>
                <Pencil className="size-3.5" aria-hidden />
              </IconButton>
            </div>
          )}
          <div className="text-[12.5px] text-fg-muted">
            {mode?.name ?? session.modeId} · {formatDateTime(session.startedAt)} ·{" "}
            {formatDuration(session.startedAt, session.endedAt)}
            {session.status !== "completed" ? ` · ${session.status === "paused" ? "paused" : "live"}` : ""}
          </div>
        </div>
        <Button variant="secondary" size="sm" onClick={() => void exportMarkdown()}>
          <Download className="size-3.5" aria-hidden /> Export markdown
        </Button>
        <Button variant="danger" size="sm" onClick={() => setConfirmDelete(true)}>
          <Trash2 className="size-3.5" aria-hidden /> Delete
        </Button>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto pb-12">
        <section className="mt-2">
          <h3 className="mb-1.5 text-[13px] font-semibold uppercase tracking-wide text-fg-subtle">
            Timeline
          </h3>
          <ul className="relative flex flex-col before:absolute before:bottom-2 before:left-[10px] before:top-2 before:w-px before:bg-border">
            {events.map((event) => (
              <TimelineEvent key={event.id} event={event} />
            ))}
          </ul>
        </section>

        {responses.length > 0 ? (
          <section className="mt-6">
            <h3 className="mb-2 text-[13px] font-semibold uppercase tracking-wide text-fg-subtle">
              Responses
            </h3>
            <div className="flex flex-col gap-3">
              {responses.map((response) => (
                <div key={response.id} className="rounded-card border border-border bg-bg-elevated p-4">
                  {response.prompt ? (
                    <div className="mb-2 text-[12.5px] text-fg-subtle">“{response.prompt}”</div>
                  ) : null}
                  <ResponseView response={response} />
                </div>
              ))}
            </div>
          </section>
        ) : null}

        <section className="mt-6">
          <h3 className="mb-2 text-[13px] font-semibold uppercase tracking-wide text-fg-subtle">Notes</h3>
          <div className="flex flex-col gap-1.5">
            {notes.map((n) => (
              <div
                key={n.id}
                className="rounded-[10px] border border-border bg-bg-elevated px-3.5 py-2.5 text-[13.5px] text-fg"
              >
                {n.content}
              </div>
            ))}
            <div className="flex gap-2">
              <Input
                value={note}
                onChange={(e) => setNote(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") void addNote();
                }}
                placeholder="Add a note"
                aria-label="Add a note"
                className="flex-1"
              />
              <Button variant="secondary" onClick={() => void addNote()} disabled={!note.trim()}>
                <Plus className="size-3.5" aria-hidden /> Add
              </Button>
            </div>
          </div>
        </section>

        <section className="mt-6">
          <div className="mb-2 flex items-center justify-between">
            <h3 className="text-[13px] font-semibold uppercase tracking-wide text-fg-subtle">Summary</h3>
            <Button
              variant="secondary"
              size="sm"
              onClick={() => void generateSummary()}
              disabled={summarizing}
            >
              {summarizing ? <Spinner size={12} /> : null}{" "}
              {summary ? "Regenerate summary" : "Generate summary"}
            </Button>
          </div>
          {summaryError ? (
            <ErrorBanner
              error={summaryError}
              onRetry={() => void generateSummary()}
              compact
              className="mb-2"
            />
          ) : null}
          {summary ? (
            <div className="rounded-card border border-border bg-bg-elevated p-4">
              <p className="m-0 text-[14px] leading-relaxed text-fg">{summary.overview}</p>
              {(
                [
                  ["Topics", summary.topics],
                  ["Questions", summary.questions],
                  ["Decisions", summary.decisions],
                  ["Action items", summary.actionItems],
                  ["Open items", summary.openItems],
                  ["Improvements", summary.improvements],
                ] as Array<[string, string[]]>
              )
                .filter(([, list]) => list.length > 0)
                .map(([label, list]) => (
                  <div key={label} className="mt-3">
                    <div className="mb-1 text-[11.5px] font-medium uppercase tracking-wide text-fg-subtle">
                      {label}
                    </div>
                    <ul className="m-0 flex list-disc flex-col gap-0.5 pl-5 text-[13.5px] text-fg-muted">
                      {list.map((item, i) => (
                        <li key={i}>{item}</li>
                      ))}
                    </ul>
                  </div>
                ))}
            </div>
          ) : !summaryError ? (
            <p className="text-[13px] text-fg-muted">No summary yet.</p>
          ) : null}
        </section>
      </div>

      <ConfirmDialog
        open={confirmDelete}
        onOpenChange={setConfirmDelete}
        title="Delete this session?"
        description="The timeline, responses and notes for this session will be removed. This cannot be undone."
        confirmLabel="Delete session"
        onConfirm={async () => {
          try {
            await bluey.session.delete({ id: sessionId });
            onBack();
          } catch (error) {
            showErrorToast(toBlueyError(error, "storage"));
          }
        }}
      />
    </div>
  );
}
