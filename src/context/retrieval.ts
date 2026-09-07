/**
 * Document retrieval: pull only the relevant chunks (never whole documents)
 * from the user's context library via `bluey.documents.retrieve`.
 *
 * Scopes are searched in priority order: session → mode → global.
 * Kinds are inferred from the mode (candidate-side modes want resume-ish
 * kinds; role/company questions want JD-ish kinds).
 */

import type {
  BlueyMode,
  ContextSnapshot,
  DocumentKind,
  RetrievalQuery,
  RetrievedChunk,
  Session,
  Settings,
} from "@/lib/types";
import { isCandidateMode, requires } from "@/modes/registry";
import { currentQuestionText, looksLikeQuestion } from "./fusion";

export interface RetrievalApi {
  documents: {
    retrieve(args: { query: RetrievalQuery }): Promise<RetrievedChunk[]>;
  };
}

export interface RetrieveArgs {
  /** Explicit query override; otherwise built from instruction + transcript + OCR. */
  query?: string;
  instruction?: string;
  snapshot?: ContextSnapshot;
  mode: BlueyMode;
  session?: Session | null;
  settings: Settings;
  limit?: number;
  api: RetrievalApi;
}

const CANDIDATE_KINDS: readonly DocumentKind[] = ["resume", "cv", "experience", "skills"];
const ROLE_KINDS: readonly DocumentKind[] = ["job_description", "role_description", "company_notes"];

const ROLE_QUESTION_CUES =
  /\b(role|position|job|company|team|responsibilit|requirement|the org|their stack|about (us|them)|why (us|this company|acme)|culture|benefits|salary|comp)\b/i;

/** First OCR line that looks like a heading/headline (short, non-empty). */
export function ocrHeadline(snapshot: ContextSnapshot | undefined): string {
  const text = snapshot?.ocr?.text ?? "";
  for (const rawLine of text.split("\n")) {
    const line = rawLine.trim();
    if (line.length >= 8 && line.length <= 120) return line;
  }
  return "";
}

/** Query = instruction + last question heard + OCR headline (deduped, capped). */
export function buildRetrievalQuery(args: Pick<RetrieveArgs, "instruction" | "snapshot">): string {
  const parts: string[] = [];
  const instruction = args.instruction?.trim();
  if (instruction) parts.push(instruction);

  if (args.snapshot) {
    const question = currentQuestionText(args.snapshot, {});
    if (question && question !== instruction && looksLikeQuestion(question)) parts.push(question);
    const headline = ocrHeadline(args.snapshot);
    if (headline) parts.push(headline);
  }

  return parts.join(" \n").slice(0, 480).trim();
}

/** Document kinds worth retrieving for this mode + ask. Empty = skip retrieval. */
export function inferKinds(mode: BlueyMode, queryText: string): DocumentKind[] {
  const kinds = new Set<DocumentKind>();
  const wantsDocs =
    requires(mode, "documents") || requires(mode, "resume") || requires(mode, "job_description");
  if (!wantsDocs) return [];

  if (requires(mode, "resume") || isCandidateMode(mode)) {
    for (const kind of CANDIDATE_KINDS) kinds.add(kind);
  }
  if (requires(mode, "job_description") || ROLE_QUESTION_CUES.test(queryText)) {
    for (const kind of ROLE_KINDS) kinds.add(kind);
  }
  if (requires(mode, "documents")) {
    kinds.add("notes");
    kinds.add("other");
  }
  // Personal instructions ride along whenever we retrieve at all.
  if (kinds.size > 0) kinds.add("personal_instructions");
  return Array.from(kinds);
}

/**
 * Retrieve relevant chunks. Returns [] when the mode declares no document
 * needs or when there is nothing to query with. Never throws for backend
 * failures — retrieval is best-effort context.
 */
export async function retrieveRelevantContext(args: RetrieveArgs): Promise<RetrievedChunk[]> {
  const { mode, session, settings, api } = args;

  const queryText = args.query?.trim() ?? buildRetrievalQuery(args);
  const kinds = inferKinds(mode, queryText);
  if (kinds.length === 0) return [];
  if (queryText.length === 0) return [];

  const scopes: RetrievalQuery["scopes"] = [];
  if (session) scopes.push({ scope: "session", scopeId: session.id });
  scopes.push({ scope: "mode", scopeId: mode.id });
  scopes.push({ scope: "global" });

  const query: RetrievalQuery = {
    query: queryText,
    scopes,
    kinds,
    limit: args.limit ?? 8,
    strategy: settings.ai.embeddingsEnabled ? "auto" : "keyword",
  };

  try {
    const chunks = await api.documents.retrieve({ query });
    return chunks.slice().sort((a, b) => b.score - a.score);
  } catch {
    return [];
  }
}
