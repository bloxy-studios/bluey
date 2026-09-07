/**
 * document_read: local document access via the Rust backend.
 *
 * The sidecar never touches SQLite. It emits a `document.request` event and
 * waits for the matching `document.response` method call from Rust (10 s
 * timeout). Only document ids present in the job's `allowedDocumentIds`
 * allow-list may be requested — a non-allowed id is refused WITHOUT emitting
 * any `document.request`.
 */

import type { DocumentResponseParams, ProtocolWriter } from "../protocol";
import { ToolError } from "./errors";

export const DOCUMENT_REQUEST_TIMEOUT_MS = 10_000;

interface PendingDocumentRequest {
  documentId: string;
  resolve: (text: string) => void;
  reject: (err: ToolError) => void;
  timer: ReturnType<typeof setTimeout>;
}

export interface DocumentBrokerOptions {
  timeoutMs?: number;
  makeRequestId?: () => string;
}

export class DocumentBroker {
  private readonly pending = new Map<string, PendingDocumentRequest>();
  private readonly allowed: ReadonlySet<string>;
  private readonly timeoutMs: number;
  private readonly makeRequestId: () => string;
  private counter = 0;
  private closed = false;

  constructor(
    private readonly writer: ProtocolWriter,
    allowedDocumentIds: readonly string[],
    options: DocumentBrokerOptions = {},
  ) {
    this.allowed = new Set(allowedDocumentIds);
    this.timeoutMs = options.timeoutMs ?? DOCUMENT_REQUEST_TIMEOUT_MS;
    this.makeRequestId = options.makeRequestId ?? (() => `docreq-${++this.counter}`);
  }

  get pendingCount(): number {
    return this.pending.size;
  }

  isAllowed(documentId: string): boolean {
    return this.allowed.has(documentId);
  }

  /** Request a document's text from Rust. Enforces the allow-list. */
  read(documentId: string): Promise<string> {
    if (this.closed) {
      return Promise.reject(new ToolError("cancelled", "document broker is closed"));
    }
    if (!this.isAllowed(documentId)) {
      return Promise.reject(
        new ToolError(
          "document_not_allowed",
          `document "${documentId}" is not in the allowed document list for this job`,
        ),
      );
    }
    const requestId = this.makeRequestId();
    return new Promise<string>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(requestId);
        reject(
          new ToolError(
            "document_timeout",
            `document.request ${requestId} timed out after ${this.timeoutMs}ms`,
          ),
        );
      }, this.timeoutMs);
      // Don't let a pending document request keep the process alive on its own.
      (timer as { unref?: () => void }).unref?.();
      this.pending.set(requestId, { documentId, resolve, reject, timer });
      this.writer.event("document.request", { requestId, documentId });
    });
  }

  /** Handle a `document.response` method from Rust. Returns false for unknown request ids. */
  handleResponse(params: DocumentResponseParams): boolean {
    const entry = this.pending.get(params.requestId);
    if (!entry) return false;
    this.pending.delete(params.requestId);
    clearTimeout(entry.timer);
    if (typeof params.text === "string") {
      entry.resolve(params.text);
    } else {
      const detail = params.error ? `: ${params.error}` : "";
      entry.reject(
        new ToolError("document_error", `backend could not provide document${detail}`),
      );
    }
    return true;
  }

  /** Reject everything in flight (job cancelled / process shutting down). */
  close(reason = "job ended"): void {
    this.closed = true;
    for (const [requestId, entry] of this.pending) {
      clearTimeout(entry.timer);
      entry.reject(new ToolError("cancelled", `document request ${requestId} aborted: ${reason}`));
    }
    this.pending.clear();
  }
}
