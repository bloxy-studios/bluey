/** Document / context library types (mirrors `bluey_core::types::documents`). */

export type DocumentKind =
  | "resume"
  | "cv"
  | "bio"
  | "portfolio"
  | "skills"
  | "experience"
  | "personal_instructions"
  | "job_description"
  | "company_notes"
  | "role_description"
  | "notes"
  | "other";

/** Where the document is attached. Priority: session > mode > global. */
export type DocumentScope = "global" | "session" | "mode";

export type DocumentFormat = "pdf" | "docx" | "txt" | "md" | "text";

export type DocumentIndexStatus = "pending" | "indexed" | "failed";

export interface BlueyDocument {
  id: string;
  title: string;
  kind: DocumentKind;
  format: DocumentFormat;
  scope: DocumentScope;
  /** sessionId or modeId when scope != global. */
  scopeId?: string;
  /** Original file path (if imported from disk). */
  sourcePath?: string;
  sizeBytes: number;
  chunkCount: number;
  indexStatus: DocumentIndexStatus;
  hasEmbeddings: boolean;
  /** `providerId/model` that produced the chunk vectors (absent until embedded). */
  embeddingModel?: string;
  /** Vector length the chunks were embedded with. */
  embeddingDimensions?: number;
  metadata?: Record<string, unknown>;
  createdAt: string;
  updatedAt: string;
}

export interface DocumentChunk {
  id: string;
  documentId: string;
  index: number;
  content: string;
  tokens: number;
  /** Section heading path, if detected. */
  heading?: string;
}

export interface AddDocumentInput {
  title?: string;
  kind: DocumentKind;
  scope: DocumentScope;
  scopeId?: string;
  /** Either a file path on disk... */
  path?: string;
  /** ...or inline text content. */
  content?: string;
  format?: DocumentFormat;
}

export interface RetrievalQuery {
  query: string;
  scopes: Array<{ scope: DocumentScope; scopeId?: string }>;
  kinds?: DocumentKind[];
  limit?: number;
  /** Use embeddings if available, otherwise keyword ranking. */
  strategy?: "auto" | "keyword" | "semantic";
}
