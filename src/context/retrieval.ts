/**
 * Document retrieval: pull only the relevant chunks (never whole documents)
 * from the user's context library via `bluey.documents.retrieve`.
 *
 * Every ask runs a few small queries in parallel:
 * - personal instructions: the leading chunks of every `personal_instructions`
 *   document in scope, whatever the wording (they apply to every answer);
 * - the active mode's own files, with no kind filter (files dropped on a mode
 *   are stored as `notes`);
 * - the session + global library: kinds inferred from the mode when it
 *   declares document needs, plus a small relevance-floored pass in every mode;
 * - candidate modes pin the résumé's leading chunks when the question matched
 *   none of it or is a canonical intro/behavioral one.
 *
 * Scopes are listed in priority order: session → mode → global.
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
const RESUME_KINDS: readonly DocumentKind[] = ["resume", "cv"];
const ROLE_KINDS: readonly DocumentKind[] = ["job_description", "role_description", "company_notes"];

const ROLE_QUESTION_CUES =
  /\b(role|position|job|company|team|responsibilit|requirement|the org|their stack|about (us|them)|why (us|this company|acme)|culture|benefits|salary|comp)\b/i;

/** Questions answered from the whole résumé rather than a matching line. */
const INTRO_QUESTION_CUES =
  /\b(about yourself|introduce yourself|walk (me|us) through your|your (background|experience|resume|cv|career)|(biggest|greatest) (strength|weakness)|strengths?|weakness(es)?|why should we hire you|a time (when )?you|describe a (time|situation)|proudest|accomplishments?|where do you see yourself)\b/i;

const MATCH_LIMIT = 8;
/** Chunks pulled from outside the mode's declared document needs. */
const OPPORTUNISTIC_LIMIT = 3;
/**
 * Minimum score for those chunks. Rust scores a non-discriminating keyword
 * match 0.25, times at most 1.15 (session scope) × 1.25 (kind intent) ≈ 0.36.
 */
const OPPORTUNISTIC_MIN_SCORE = 0.4;
/** Leading chunks of personal instructions (snapshot enrichment caps the text). */
const PERSONAL_INSTRUCTION_CHUNKS = 6;
/** Leading résumé chunks pinned in candidate modes (~350 tokens each). */
const RESUME_PIN_CHUNKS = 2;

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

/** Library kinds the mode declares a need for. Empty = only the relevance-floored pass. */
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
  return Array.from(kinds);
}

/** Whether the ask should see the résumé whatever its wording. */
export function isIntroQuestion(queryText: string): boolean {
  return INTRO_QUESTION_CUES.test(queryText);
}

async function run(api: RetrievalApi, query: RetrievalQuery): Promise<RetrievedChunk[]> {
  try {
    return await api.documents.retrieve({ query });
  } catch {
    return [];
  }
}

const byScore = (a: RetrievedChunk, b: RetrievedChunk) => b.score - a.score;

/**
 * Retrieve relevant chunks: matched content chunks sorted by score, then the
 * personal-instruction chunks in document order. Never throws for backend
 * failures — retrieval is best-effort context.
 */
export async function retrieveRelevantContext(args: RetrieveArgs): Promise<RetrievedChunk[]> {
  const { mode, session, settings, api } = args;

  const queryText = args.query?.trim() ?? buildRetrievalQuery(args);
  const strategy = settings.ai.embeddingsEnabled ? "auto" : "keyword";
  const sessionScope: RetrievalQuery["scopes"] = session ? [{ scope: "session", scopeId: session.id }] : [];
  const modeScope: RetrievalQuery["scopes"] = [{ scope: "mode", scopeId: mode.id }];
  const library: RetrievalQuery["scopes"] = [...sessionScope, { scope: "global" }];
  const allScopes: RetrievalQuery["scopes"] = [...sessionScope, ...modeScope, { scope: "global" }];
  const hasQuery = queryText.length > 0;
  const kinds = inferKinds(mode, queryText);
  const limit = args.limit ?? MATCH_LIMIT;
  const candidate = isCandidateMode(mode) || requires(mode, "resume");

  const none = Promise.resolve<RetrievedChunk[]>([]);
  const [personal, modeFiles, declared, opportunistic, resumePin] = await Promise.all([
    run(api, {
      query: queryText,
      scopes: allScopes,
      kinds: ["personal_instructions"],
      limit: PERSONAL_INSTRUCTION_CHUNKS,
      strategy: "leading",
    }),
    hasQuery && mode.attachedDocumentIds.length > 0
      ? run(api, { query: queryText, scopes: modeScope, limit, strategy })
      : none,
    hasQuery && kinds.length > 0 ? run(api, { query: queryText, scopes: library, kinds, limit, strategy }) : none,
    hasQuery
      ? run(api, { query: queryText, scopes: library, limit: OPPORTUNISTIC_LIMIT, strategy: "keyword" })
      : none,
    candidate
      ? run(api, {
          query: queryText,
          scopes: allScopes,
          kinds: [...RESUME_KINDS],
          limit: RESUME_PIN_CHUNKS,
          strategy: "leading",
        })
      : none,
  ]);

  const matched = new Map<string, RetrievedChunk>();
  const keep = (chunk: RetrievedChunk) => {
    // Personal instructions come from their own deterministic query.
    if (chunk.documentKind === "personal_instructions") return;
    const existing = matched.get(chunk.chunkId);
    if (!existing || existing.score < chunk.score) matched.set(chunk.chunkId, chunk);
  };
  modeFiles.forEach(keep);
  declared.forEach(keep);
  opportunistic.filter((chunk) => chunk.score >= OPPORTUNISTIC_MIN_SCORE).forEach(keep);

  const resumeMatched = Array.from(matched.values()).some((chunk) =>
    RESUME_KINDS.includes(chunk.documentKind),
  );
  if (!resumeMatched || isIntroQuestion(queryText)) resumePin.forEach(keep);

  const personalChunks = personal.filter((chunk) => chunk.documentKind === "personal_instructions");
  return [...Array.from(matched.values()).sort(byScore), ...personalChunks];
}
