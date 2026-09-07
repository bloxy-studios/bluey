import { MoreHorizontal, X } from "lucide-react";
import { useCallback, useEffect, useState } from "react";

import { Button } from "@/components/ui/Button";
import { ConfirmDialog } from "@/components/ui/Dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/DropdownMenu";
import { IconButton } from "@/components/ui/IconButton";
import { Select } from "@/components/ui/Select";
import { Textarea } from "@/components/ui/Textarea";
import { showToast } from "@/components/ui/toast-store";
import { useDebouncedCallback } from "@/hooks/useDebouncedCallback";
import { bluey } from "@/lib/tauri/api";
import type { BlueyDocument, BlueyMode } from "@/lib/types";
import { formatBytes } from "@/lib/utils/format";
import { ModeFilesDropzone } from "./ModeFilesDropzone";

const LATENCY_OPTIONS = [
  { value: "ultra-fast", label: "Ultra fast" },
  { value: "fast", label: "Fast" },
  { value: "balanced", label: "Balanced" },
  { value: "deep", label: "Deep" },
];

const LENGTH_OPTIONS = [
  { value: "", label: "Default length" },
  { value: "concise", label: "Concise" },
  { value: "balanced", label: "Balanced" },
  { value: "detailed", label: "Detailed" },
];

const TONE_OPTIONS = [
  { value: "", label: "Default tone" },
  { value: "natural", label: "Natural" },
  { value: "professional", label: "Professional" },
  { value: "technical", label: "Technical" },
  { value: "conversational", label: "Conversational" },
  { value: "direct", label: "Direct" },
];

const MODEL_ROLE_OPTIONS = [
  { value: "", label: "Auto model" },
  { value: "default", label: "Default model" },
  { value: "fast", label: "Fast model" },
  { value: "reasoning", label: "Reasoning model" },
  { value: "vision", label: "Vision model" },
  { value: "research", label: "Research model" },
];

export interface ModeEditorProps {
  mode: BlueyMode;
  isActive: boolean;
  onDeleted: () => void;
}

/** Right pane of Settings → Modes: title, meeting context, files, actions. */
export function ModeEditor({ mode, isActive, onDeleted }: ModeEditorProps) {
  const [name, setName] = useState(mode.name);
  const [instructions, setInstructions] = useState(mode.systemInstructions);
  const [documents, setDocuments] = useState<BlueyDocument[]>([]);
  const [confirmDelete, setConfirmDelete] = useState(false);

  const saveName = useDebouncedCallback((value: string) => {
    if (value.trim().length > 0) void bluey.modes.update({ id: mode.id, patch: { name: value.trim() } });
  }, 500);
  const saveInstructions = useDebouncedCallback((value: string) => {
    void bluey.modes.update({ id: mode.id, patch: { systemInstructions: value } });
  }, 600);

  const refreshDocuments = useCallback(async () => {
    try {
      setDocuments(await bluey.documents.list({ scope: "mode", scopeId: mode.id }));
    } catch (error) {
      console.warn("[modes] documents.list failed", error);
    }
  }, [mode.id]);

  useEffect(() => {
    void refreshDocuments();
  }, [refreshDocuments]);

  const patchStyle = (key: "length" | "tone", value: string) => {
    const responseStyle = { ...mode.responseStyle };
    if (value === "") delete responseStyle[key];
    else responseStyle[key] = value as never;
    void bluey.modes.update({ id: mode.id, patch: { responseStyle } });
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="min-h-0 flex-1 overflow-y-auto px-8 py-6">
        <div className="flex items-start justify-between gap-4">
          {mode.builtIn ? (
            <h1 className="text-[28px] font-semibold leading-tight text-fg">{mode.name}</h1>
          ) : (
            <input
              value={name}
              aria-label="Mode name"
              onChange={(e) => {
                setName(e.target.value);
                saveName(e.target.value);
              }}
              className="w-full bg-transparent text-[28px] font-semibold leading-tight text-fg outline-none placeholder:text-fg-subtle"
              placeholder="Untitled Mode"
            />
          )}
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <IconButton aria-label="Mode actions" variant="circle" size="lg">
                <MoreHorizontal className="size-4" aria-hidden />
              </IconButton>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              <DropdownMenuItem onSelect={() => void bluey.modes.duplicate({ id: mode.id })}>Duplicate</DropdownMenuItem>
              <DropdownMenuItem
                onSelect={() => {
                  void bluey.modes.setDefault({ id: mode.id }).then(() => showToast("Default mode set"));
                }}
              >
                Set as default
              </DropdownMenuItem>
              {mode.builtIn ? (
                <DropdownMenuItem onSelect={() => void bluey.modes.resetBuiltIn({ id: mode.id })}>
                  Reset to default
                </DropdownMenuItem>
              ) : (
                <>
                  <DropdownMenuSeparator />
                  <DropdownMenuItem destructive onSelect={() => setConfirmDelete(true)}>
                    Delete
                  </DropdownMenuItem>
                </>
              )}
            </DropdownMenuContent>
          </DropdownMenu>
        </div>

        {mode.id === "general" ? (
          <p className="mt-4 max-w-[560px] text-[15px] leading-relaxed text-fg-muted">{mode.description}</p>
        ) : (
          <>
            <label htmlFor="meeting-context" className="mt-6 mb-2 block text-[14px] font-semibold text-fg">
              Meeting context
            </label>
            <Textarea
              id="meeting-context"
              value={instructions}
              onChange={(e) => {
                setInstructions(e.target.value);
                saveInstructions(e.target.value);
              }}
              placeholder="Tell Bluey what this meeting is about, or leave it blank to use the default prompt."
              className="min-h-[180px]"
            />

            {!mode.builtIn ? (
              <div className="mt-5 flex flex-wrap items-center gap-2.5">
                <Select
                  aria-label="Response length"
                  value={mode.responseStyle?.length ?? ""}
                  onChange={(e) => patchStyle("length", e.target.value)}
                  options={LENGTH_OPTIONS}
                />
                <Select
                  aria-label="Response tone"
                  value={mode.responseStyle?.tone ?? ""}
                  onChange={(e) => patchStyle("tone", e.target.value)}
                  options={TONE_OPTIONS}
                />
                <Select
                  aria-label="Latency preference"
                  value={mode.preferredLatency}
                  onChange={(e) => void bluey.modes.update({ id: mode.id, patch: { preferredLatency: e.target.value as BlueyMode["preferredLatency"] } })}
                  options={LATENCY_OPTIONS}
                />
                <Select
                  aria-label="Preferred model"
                  value={mode.preferredModelRole ?? ""}
                  onChange={(e) =>
                    void bluey.modes.update({
                      id: mode.id,
                      patch: { preferredModelRole: (e.target.value || undefined) as BlueyMode["preferredModelRole"] },
                    })
                  }
                  options={MODEL_ROLE_OPTIONS}
                />
              </div>
            ) : null}

            <div className="mt-6 mb-2 text-[14px] font-semibold text-fg">Files</div>
            <ModeFilesDropzone modeId={mode.id} onAdded={() => void refreshDocuments()} />
            {documents.length > 0 ? (
              <ul className="mt-3 flex flex-col gap-1.5">
                {documents.map((doc) => (
                  <li key={doc.id} className="flex items-center gap-3 rounded-[10px] border border-border bg-bg-elevated px-3 py-2">
                    <span className="min-w-0 flex-1 truncate text-[13.5px] text-fg">{doc.title}</span>
                    <span className="shrink-0 text-[12px] text-fg-subtle">{formatBytes(doc.sizeBytes)}</span>
                    <IconButton
                      aria-label={`Remove ${doc.title}`}
                      variant="plain"
                      size="sm"
                      onClick={() => void bluey.documents.delete({ id: doc.id }).then(() => refreshDocuments())}
                    >
                      <X className="size-3.5" aria-hidden />
                    </IconButton>
                  </li>
                ))}
              </ul>
            ) : null}
          </>
        )}
      </div>

      <div className="flex shrink-0 items-center justify-end border-t border-border px-6 py-3">
        <Button
          variant="primary"
          disabled={isActive}
          className="disabled:opacity-75"
          onClick={() => void bluey.modes.setActive({ id: mode.id })}
        >
          {isActive ? "Active" : "Set Active"}
        </Button>
      </div>

      <ConfirmDialog
        open={confirmDelete}
        onOpenChange={setConfirmDelete}
        title={`Delete “${mode.name}”?`}
        description="This removes the mode and its attached files. This cannot be undone."
        confirmLabel="Delete mode"
        onConfirm={async () => {
          await bluey.modes.delete({ id: mode.id });
          onDeleted();
        }}
      />
    </div>
  );
}
