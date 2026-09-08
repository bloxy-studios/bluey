import { FileText } from "lucide-react";
import { useState, type DragEvent } from "react";

import { showErrorToast, showToast } from "@/components/ui/toast-store";
import { bluey } from "@/lib/tauri/api";
import { toBlueyError, type DocumentKind, type DocumentScope } from "@/lib/types";
import { cn } from "@/lib/utils/cn";

const TEXT_EXTENSIONS = [".txt", ".md", ".markdown", ".text"];

export interface FilesDropzoneProps {
  /** Document kind given to every file added through this dropzone. */
  kind: DocumentKind;
  scope: DocumentScope;
  /** modeId / sessionId when `scope` is not global. */
  scopeId?: string;
  onAdded: () => void;
  headline?: string;
  className?: string;
}

/**
 * Dashed dropzone with the stacked-documents illustration. Dropped text files are
 * read in the WebView and sent inline; PDF/DOCX go through the native picker
 * (`documents_pick_files`) so Rust reads them from disk. Failures surface as toasts.
 */
export function FilesDropzone({
  kind,
  scope,
  scopeId,
  onAdded,
  headline = "Adding files gives more context to Bluey",
  className,
}: FilesDropzoneProps) {
  const [dragging, setDragging] = useState(false);
  const [busy, setBusy] = useState(false);

  const browse = async () => {
    if (busy) return;
    setBusy(true);
    try {
      const paths = await bluey.documents.pickFiles();
      let added = 0;
      for (const path of paths) {
        try {
          await bluey.documents.add({ input: { kind, scope, scopeId, path } });
          added += 1;
        } catch (error) {
          showErrorToast(toBlueyError(error, "storage"));
        }
      }
      if (added > 0) {
        showToast(added === 1 ? "File added" : `${added} files added`);
        onAdded();
      }
    } catch (error) {
      showErrorToast(toBlueyError(error, "storage"));
    } finally {
      setBusy(false);
    }
  };

  const onDrop = async (event: DragEvent<HTMLDivElement>) => {
    event.preventDefault();
    setDragging(false);
    const files = Array.from(event.dataTransfer?.files ?? []);
    if (files.length === 0) return;
    let added = 0;
    let skipped = 0;
    for (const file of files) {
      const lower = file.name.toLowerCase();
      const isText = file.type.startsWith("text/") || TEXT_EXTENSIONS.some((ext) => lower.endsWith(ext));
      if (!isText) {
        skipped += 1;
        continue;
      }
      try {
        const content = await file.text();
        await bluey.documents.add({
          input: {
            title: file.name,
            kind,
            scope,
            scopeId,
            content,
            format: lower.endsWith(".md") ? "md" : "txt",
          },
        });
        added += 1;
      } catch (error) {
        showErrorToast(toBlueyError(error, "storage"));
      }
    }
    if (added > 0) {
      showToast(added === 1 ? "File added" : `${added} files added`);
      onAdded();
    }
    if (skipped > 0) showToast("Use browse for PDF and DOCX files", 2500);
  };

  return (
    <div
      role="button"
      tabIndex={0}
      aria-label="Add files"
      aria-busy={busy}
      onClick={() => void browse()}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") void browse();
      }}
      onDragOver={(e) => {
        e.preventDefault();
        setDragging(true);
      }}
      onDragLeave={() => setDragging(false)}
      onDrop={(e) => void onDrop(e)}
      className={cn(
        "flex h-[220px] cursor-default flex-col items-center justify-center gap-1 rounded-card border border-dashed border-[#3a3a3a]",
        "outline-none transition-colors focus-visible:border-accent",
        dragging ? "border-accent bg-accent-soft/40" : "hover:border-border-strong",
        className,
      )}
    >
      {/* stacked-documents illustration */}
      <div className="relative mb-3 h-[72px] w-[80px]" aria-hidden>
        <div className="absolute left-1 top-3 h-[62px] w-[52px] -rotate-6 rounded-[8px] bg-white/85 shadow-md shadow-black/25" />
        <div className="absolute left-6 top-0 flex h-[66px] w-[56px] rotate-3 items-center justify-center rounded-[8px] bg-white shadow-lg shadow-black/30">
          <FileText className="size-6 text-[#1d1d1f]" strokeWidth={1.6} />
        </div>
      </div>
      <div className="text-[15px] font-medium text-fg">{headline}</div>
      <div className="text-[13px] text-fg-muted">
        Drag &amp; drop files here to add them, or{" "}
        <span className="font-medium text-accent">browse files</span>
      </div>
    </div>
  );
}
