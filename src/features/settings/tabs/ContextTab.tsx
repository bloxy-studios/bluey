import { FolderOpen, RefreshCw, Trash2 } from "lucide-react";
import { useCallback, useEffect, useState } from "react";

import { Button } from "@/components/ui/Button";
import { ConfirmDialog } from "@/components/ui/Dialog";
import { EmptyState } from "@/components/ui/EmptyState";
import { ErrorBanner } from "@/components/ui/ErrorBanner";
import { IconButton } from "@/components/ui/IconButton";
import { SectionHeader } from "@/components/ui/SectionHeader";
import { Select } from "@/components/ui/Select";
import { Spinner } from "@/components/ui/Spinner";
import { showErrorToast, showToast } from "@/components/ui/toast-store";
import { Tooltip } from "@/components/ui/Tooltip";
import { bluey } from "@/lib/tauri/api";
import { toBlueyError, type BlueyDocument, type BlueyError, type DocumentKind } from "@/lib/types";
import { cn } from "@/lib/utils/cn";
import { formatBytes, formatDate } from "@/lib/utils/format";
import { useSettingsStore } from "@/stores/settingsStore";
import { DOCUMENT_KIND_LABELS, DOCUMENT_KIND_OPTIONS } from "../document-kinds";
import { FilesDropzone } from "../FilesDropzone";

const INDEX_STATUS: Record<BlueyDocument["indexStatus"], { label: string; className: string }> = {
  indexed: { label: "Indexed", className: "text-fg-muted" },
  pending: { label: "Indexing…", className: "text-fg-subtle" },
  failed: { label: "Index failed", className: "text-danger" },
};

function DocumentRow({
  doc,
  onDelete,
  onReindex,
  reindexing,
}: {
  doc: BlueyDocument;
  onDelete: () => void;
  onReindex: () => void;
  reindexing: boolean;
}) {
  const status = INDEX_STATUS[doc.indexStatus];
  return (
    <li className="flex items-center gap-3 rounded-card border border-border bg-bg-elevated px-3.5 py-2.5">
      <div className="min-w-0 flex-1">
        <div className="truncate text-[14px] font-medium text-fg">{doc.title}</div>
        <div className="mt-0.5 flex flex-wrap items-center gap-x-2 text-[12.5px] text-fg-muted">
          <span className="rounded-full bg-bg-tile px-2 py-0.5 text-[11.5px] text-fg-muted">
            {DOCUMENT_KIND_LABELS[doc.kind]}
          </span>
          <span>{doc.format.toUpperCase()}</span>
          <span>·</span>
          <span>{formatBytes(doc.sizeBytes)}</span>
          <span>·</span>
          <span>{doc.chunkCount} chunks</span>
          <span>·</span>
          <span className={status.className}>{status.label}</span>
          {doc.hasEmbeddings ? (
            <>
              <span>·</span>
              <span>Embedded</span>
            </>
          ) : null}
          <span>·</span>
          <span>{formatDate(doc.createdAt)}</span>
        </div>
      </div>
      <Tooltip label="Reindex">
        <IconButton
          aria-label={`Reindex ${doc.title}`}
          variant="plain"
          onClick={onReindex}
          disabled={reindexing}
        >
          <RefreshCw className={cn("size-4", reindexing && "motion-safe:animate-spin")} aria-hidden />
        </IconButton>
      </Tooltip>
      <Tooltip label="Remove">
        <IconButton aria-label={`Remove ${doc.title}`} variant="plain" onClick={onDelete}>
          <Trash2 className="size-4 text-danger" aria-hidden />
        </IconButton>
      </Tooltip>
    </li>
  );
}

/**
 * Settings → Context ("My Context"): documents Bluey can draw on in every mode —
 * résumé, job description, company notes… Stored with `scope: "global"`; mode-
 * and session-scoped files live in Modes → Files and the session itself.
 */
export default function ContextTab() {
  const embeddingsEnabled = useSettingsStore((s) => s.settings?.ai.embeddingsEnabled ?? false);
  const [documents, setDocuments] = useState<BlueyDocument[] | null>(null);
  const [kind, setKind] = useState<DocumentKind>("resume");
  const [error, setError] = useState<BlueyError | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<BlueyDocument | null>(null);
  const [reindexing, setReindexing] = useState<string | "all" | null>(null);

  const load = useCallback(async () => {
    try {
      const list = await bluey.documents.list({ scope: "global" });
      setDocuments([...list].sort((a, b) => b.createdAt.localeCompare(a.createdAt)));
      setError(null);
    } catch (err) {
      setError(toBlueyError(err, "storage"));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const reindex = async (id?: string) => {
    setReindexing(id ?? "all");
    try {
      const count = await bluey.documents.reindex(id ? { id } : {});
      showToast(count === 1 ? "Reindexed 1 document" : `Reindexed ${count} documents`, 2000);
      await load();
    } catch (err) {
      showErrorToast(toBlueyError(err, "storage"));
    } finally {
      setReindexing(null);
    }
  };

  const remove = async (doc: BlueyDocument) => {
    try {
      await bluey.documents.delete({ id: doc.id });
      showToast("Document removed");
      await load();
    } catch (err) {
      showErrorToast(toBlueyError(err, "storage"));
    }
  };

  return (
    <>
      <SectionHeader
        title="My Context"
        description="Documents Bluey can draw on in every mode — your résumé, the job description, company notes. Files stay on this Mac; only relevant excerpts are sent with a request."
      />

      <div className="mb-3 flex items-center gap-3">
        <label htmlFor="context-kind" className="text-[13px] text-fg-muted">
          Add as
        </label>
        <Select
          id="context-kind"
          aria-label="Document kind"
          value={kind}
          onChange={(e) => setKind(e.target.value as DocumentKind)}
          options={DOCUMENT_KIND_OPTIONS.map((value) => ({ value, label: DOCUMENT_KIND_LABELS[value] }))}
        />
      </div>

      <FilesDropzone
        kind={kind}
        scope="global"
        onAdded={() => void load()}
        headline="Give Bluey the context it needs to answer like you"
      />

      {error ? <ErrorBanner error={error} onRetry={() => void load()} className="mt-4" /> : null}

      <div className="mt-6 mb-2 flex items-center justify-between">
        <h3 className="text-[13px] font-semibold uppercase tracking-wide text-fg-subtle">Documents</h3>
        {documents && documents.length > 0 ? (
          <Button variant="secondary" size="sm" onClick={() => void reindex()} disabled={reindexing !== null}>
            {reindexing === "all" ? <Spinner size={12} /> : <RefreshCw className="size-3.5" aria-hidden />}
            Reindex all
          </Button>
        ) : null}
      </div>

      {!embeddingsEnabled && documents && documents.length > 0 ? (
        <p className="mb-2 text-[12.5px] text-fg-muted">
          Embeddings are off, so documents are matched by keywords. Turn them on in AI settings for semantic
          retrieval.
        </p>
      ) : null}

      {documents === null ? (
        !error ? (
          <div className="flex justify-center py-8">
            <Spinner size={16} />
          </div>
        ) : null
      ) : documents.length === 0 ? (
        <EmptyState
          icon={FolderOpen}
          title="No context documents yet"
          description="Add your résumé or a job description above and Bluey will use it whenever it's relevant."
        />
      ) : (
        <ul className="m-0 flex list-none flex-col gap-1.5 p-0">
          {documents.map((doc) => (
            <DocumentRow
              key={doc.id}
              doc={doc}
              onDelete={() => setConfirmDelete(doc)}
              onReindex={() => void reindex(doc.id)}
              reindexing={reindexing === doc.id || reindexing === "all"}
            />
          ))}
        </ul>
      )}

      <ConfirmDialog
        open={confirmDelete !== null}
        onOpenChange={(open) => {
          if (!open) setConfirmDelete(null);
        }}
        title="Remove this document?"
        description={`“${confirmDelete?.title ?? ""}” and its index will be removed from Bluey. The original file is not touched.`}
        confirmLabel="Remove"
        onConfirm={async () => {
          if (confirmDelete) await remove(confirmDelete);
        }}
      />
    </>
  );
}
