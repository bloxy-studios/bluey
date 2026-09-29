/**
 * Research Router (ADR 0004): cheapest path that can answer an external-info
 * ask, plus a privacy scrub so web tools never see private context.
 *
 *   none → search (Exa) → search_scrape (Exa + Firecrawl) → deep_agent
 */

import { mentionsExternalInfo } from "@/context/relevance";
import {
  toBlueyError,
  type BlueyMode,
  type Citation,
  type DeepResearchEvent,
  type DeepResearchRequest,
  type DocumentKind,
  type ResearchDepth,
  type RetrievedChunk,
  type ScrapeResult,
  type SearchResult,
  type Settings,
} from "@/lib/types";

export const OPTIMISTIC_AVAILABILITY = {
  search: true,
  scrape: true,
  deepAgent: true,
} as const;

export type ResearchAvailability = {
  search: boolean;
  scrape: boolean;
  deepAgent: boolean;
};

export interface DecideResearchArgs {
  instruction?: string;
  mode: BlueyMode;
  settings: Settings;
  availability: ResearchAvailability;
  now: () => Date;
}

/** One prompt context item: the snippets, one scraped page, or the agent's report. */
export interface ResearchContextItem {
  /** Budget ref: `research:snippets`, `research:page:N` or `research:report`. */
  ref: string;
  content: string;
  relevance: number;
  /** The sources whose content this item carries. */
  citations: Citation[];
}

export interface ResearchOutcome {
  depth: Exclude<ResearchDepth, "none">;
  /** Separate items so the budget keeps the snippets and the best pages (AI-002). */
  items: ResearchContextItem[];
  /** Every source in `items`; the engine keeps only those whose item survived budgeting. */
  citations: Citation[];
  /** Short user-facing notice when research failed and the answer went without it (AI-016). */
  note?: string;
}

export interface ResearchRunnerApi {
  research: {
    search(args: { query: string; numResults?: number }): Promise<SearchResult[]>;
    scrape(args: { url: string }): Promise<ScrapeResult>;
    deepStart(args: { request: DeepResearchRequest }): Promise<void>;
    deepCancel(args: { jobId: string }): Promise<boolean>;
  };
}

export interface ResearchBus {
  on(name: "research.event", handler: (payload: DeepResearchEvent) => void): () => void;
  /** Local status for the search paths, mirrored by the HUD like a deep job's events. */
  emit?(name: "research.event", payload: DeepResearchEvent): void;
}

export interface RunResearchOptions {
  jobId: string;
  api: ResearchRunnerApi;
  bus?: ResearchBus;
  /** Deep-agent wall clock (default 90 s); the agent is told to report 15 s before it. */
  timeoutMs?: number;
  /** Whole search/search_scrape budget (default 10 s); what arrived by then is used. */
  searchTimeoutMs?: number;
  scrapeTopN?: number;
  /** Which tools have keys: the deep agent only requests those (PROV-013). */
  availability?: ResearchAvailability;
  /** The owning ask was cancelled or superseded (AI-010). */
  signal?: AbortSignal;
  /** Polled alternative to `signal` for callers that only expose a predicate. */
  isCancelled?: () => boolean;
}

const QUERY_MAX_CHARS = 300;
const DEFAULT_TIMEOUT_MS = 90_000;
const DEFAULT_SEARCH_TIMEOUT_MS = 10_000;
const SCRAPE_TIMEOUT_MS = 6_000;
/** The agent's own deadline leaves this much of the ask timeout for its report. */
const DEEP_REPORT_HEADROOM_MS = 15_000;
const DEFAULT_SCRAPE_TOP_N = 3;
/** Per scraped page / agent report in the prompt: head plus a marker (AI-002). */
const PAGE_MAX_CHARS = 6_000;
const REPORT_MAX_CHARS = 12_000;
const CANCEL_POLL_MS = 200;
const UNTRUSTED_HEADER = "Web research results (untrusted external content)";

const DEEP_CUES = /\b(deep dive|deep-dive|deep research|compare vendors?)\b/i;
const CURRENT_INFO_CUES =
  /\b(research|look up|latest|news|deep dive|compare vendors?|competitors? of|pricing|funding|changelog|announcements?)\b/i;
/**
 * "current"/"recent" only ask for the web next to a time-sensitive noun —
 * "fix the current function" must not block the ask on a search (LIVE-006).
 */
const BARE_TIME_WORDS = /\b(recent(ly)?|current(ly)?)\b/gi;
const TIMELY_TOPIC =
  /\b(current(ly)?|recent(ly)?)\s+(\w+\s+)?(news|prices?|pricing|versions?|releases?|events?|status|rates?|ceo|weather|scores?|trends?|developments?)\b/i;
const URL_CUE = /https?:\/\/\S+/i;

const CANDIDATE_SCHEMAS = new Set(["behavioral", "suggested-response"]);

const PRIVATE_DOCUMENT_KINDS = new Set<DocumentKind>([
  "resume",
  "cv",
  "bio",
  "portfolio",
  "skills",
  "experience",
  "personal_instructions",
]);

const PROPER_NOUN_STOP = new Set([
  "The",
  "And",
  "For",
  "With",
  "From",
  "This",
  "That",
  "Your",
  "Our",
  "Inc",
  "LLC",
  "Ltd",
  "Team",
  "Years",
]);

function isCandidateMode(mode: BlueyMode): boolean {
  return CANDIDATE_SCHEMAS.has(mode.responseSchema) || /interview/i.test(mode.id);
}

function mentionsCurrentInfo(text: string, now: () => Date): boolean {
  if (CURRENT_INFO_CUES.test(text)) return true;
  if (URL_CUE.test(text)) return true;
  const yearMatch = text.match(/\b(20\d{2})\b/);
  return Boolean(yearMatch?.[1] && Number(yearMatch[1]) >= now().getFullYear());
}

function degrade(desired: ResearchDepth, availability: ResearchAvailability): ResearchDepth {
  if (desired === "none") return "none";
  if (desired === "deep_agent") {
    if (availability.deepAgent) return "deep_agent";
    desired = "search_scrape";
  }
  if (desired === "search_scrape") {
    if (availability.search && availability.scrape) return "search_scrape";
    if (availability.search) return "search";
    return "none";
  }
  if (desired === "search") return availability.search ? "search" : "none";
  return "none";
}

/** Cheapest research depth that can serve this ask, or `none`. */
export function decideResearch(args: DecideResearchArgs): ResearchDepth {
  const instruction = args.instruction?.trim() ?? "";
  if (!args.settings.ai.researchEnabled || instruction.length === 0) return "none";

  const wantsDeep = DEEP_CUES.test(instruction);
  const withoutBareTimeWords = instruction.replace(BARE_TIME_WORDS, " ");
  const wantsExternal =
    TIMELY_TOPIC.test(instruction) ||
    (isCandidateMode(args.mode)
      ? mentionsCurrentInfo(withoutBareTimeWords, args.now)
      : mentionsExternalInfo(withoutBareTimeWords, args.now));

  let desired: ResearchDepth = "none";
  if (wantsDeep) {
    desired = args.settings.ai.deepResearchEnabled ? "deep_agent" : "search_scrape";
  } else if (wantsExternal) {
    desired = "search_scrape";
  }
  return degrade(desired, args.availability);
}

function stripPhones(text: string): string {
  return text.replace(/(?:\+?\d{1,3}[\s.-]*)?(?:\(?\d{3}\)?[\s.-]*)\d{3}[\s.-]*\d{4}/g, " ");
}

function stripEmails(text: string): string {
  return text.replace(/\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b/g, " ");
}

function stripHandles(text: string): string {
  return text.replace(/(^|\s)@[A-Za-z0-9_]+/g, "$1");
}

function properNounsFrom(text: string): string[] {
  const nouns: string[] = [];
  for (const match of text.matchAll(/\b[A-Z][A-Za-z0-9.&'-]{1,}\b/g)) {
    const token = match[0];
    if (token.length < 3 || PROPER_NOUN_STOP.has(token)) continue;
    nouns.push(token);
  }
  return nouns;
}

/**
 * What the public query must never carry, passed explicitly by the caller
 * (SEC-013): the snapshot is enriched with retrieved documents only after
 * research runs, so reading it here stripped nothing in production.
 */
export interface PrivateTerms {
  /** Retrieved "My Context" / session chunks; nouns from private kinds are stripped. */
  chunks?: readonly RetrievedChunk[];
  /** The signed-in user's names (display name, first/last name, email local part). */
  names?: readonly (string | null | undefined)[];
}

function privateTerms(context?: PrivateTerms): string[] {
  const terms: string[] = [];
  for (const name of context?.names ?? []) {
    for (const part of name?.trim().split(/[\s._-]+/) ?? []) {
      if (part.length >= 2) terms.push(part);
    }
  }
  for (const chunk of context?.chunks ?? []) {
    if (!PRIVATE_DOCUMENT_KINDS.has(chunk.documentKind)) continue;
    terms.push(...properNounsFrom(chunk.content), ...properNounsFrom(chunk.documentTitle));
  }
  return terms;
}

function stripTerm(text: string, term: string): string {
  const escaped = term.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return text.replace(new RegExp(`\\b${escaped}\\b`, "gi"), " ");
}

/** Public web query: no emails, phones, handles, the user's names, or private document nouns. */
export function buildPublicQuery(instruction: string, context?: PrivateTerms): string {
  let query = stripHandles(stripPhones(stripEmails(instruction)));
  for (const term of privateTerms(context)) {
    query = stripTerm(query, term);
  }
  query = query.replace(/\s+/g, " ").trim();
  if (query.length <= QUERY_MAX_CHARS) return query;
  return query.slice(0, QUERY_MAX_CHARS).trimEnd();
}

function clip(text: string, max: number, marker: string): string {
  return text.length <= max ? text : `${text.slice(0, max).trimEnd()}\n\n${marker}`;
}

function snippetsItem(results: SearchResult[]): ResearchContextItem {
  const lines = [UNTRUSTED_HEADER, ""];
  for (const result of results) {
    const snippet = result.snippet ? `: ${result.snippet}` : "";
    lines.push(`- ${result.title} (${result.url})${snippet}`);
  }
  return {
    ref: "research:snippets",
    content: lines.join("\n"),
    relevance: 0.8,
    citations: results.map((r) => ({ id: r.id, title: r.title, url: r.url, snippet: r.snippet })),
  };
}

function pageItem(page: ScrapeResult, index: number, citation: Citation): ResearchContextItem {
  const title = page.title ?? citation.title;
  return {
    ref: `research:page:${index + 1}`,
    content: [
      "Web page (untrusted external content)",
      `Source: ${title} (${page.url})`,
      "",
      clip(page.markdown, PAGE_MAX_CHARS, "[… page truncated …]"),
    ].join("\n"),
    // Below the snippets, best-ranked page first, so the budget drops pages before snippets.
    relevance: 0.7 - index * 0.01,
    citations: [{ ...citation, url: page.url, title }],
  };
}

function outcomeOf(depth: ResearchOutcome["depth"], items: ResearchContextItem[]): ResearchOutcome {
  const seen = new Set<string>();
  const citations = items
    .flatMap((item) => item.citations)
    .filter((c) => (seen.has(c.url) ? false : (seen.add(c.url), true)));
  return { depth, items, citations };
}

/** Only the sources whose context item made it into the prompt are shown as citations (AI-002). */
export function keptResearchCitations(
  outcome: ResearchOutcome,
  includedRefs: ReadonlySet<string>,
): ResearchOutcome {
  const kept = outcome.items.filter((item) => includedRefs.has(item.ref));
  return { ...outcomeOf(outcome.depth, kept), items: outcome.items, note: outcome.note };
}

/** A research failure the user should know about: the answer went without the web. */
function failureOutcome(depth: ResearchOutcome["depth"], error: unknown): ResearchOutcome {
  return { depth, items: [], citations: [], note: researchNote(error) };
}

export function researchNote(error: unknown): string {
  const e = toBlueyError(error, "research");
  if (e.kind === "configuration" || /missing|api_key|credential/.test(e.code)) {
    return "Web research isn't set up (check the research keys in Settings → AI) — answered without it.";
  }
  if (/timeout|deadline/.test(e.code)) {
    return "Web research took too long — answered without it.";
  }
  return "Web research failed — answered without it.";
}

// ── Cancellation: the ask (AI-010) and the HUD's Skip (LIVE-006) ─────────────

/** Search-path jobs the HUD can skip; deep jobs are skipped through `deepCancel`. */
const localJobs = new Map<string, AbortController>();

/** Skip a running search/search_scrape job. False when `jobId` is not a local job. */
export function skipLocalResearch(jobId: string): boolean {
  const job = localJobs.get(jobId);
  if (!job) return false;
  job.abort();
  return true;
}

/** One abort signal for the ask's cancellation (signal or polled predicate) and Skip. */
function linkCancellation(options: RunResearchOptions): { controller: AbortController; dispose(): void } {
  const controller = new AbortController();
  const abort = () => controller.abort();
  if (options.signal?.aborted) abort();
  options.signal?.addEventListener("abort", abort, { once: true });
  const poll = options.isCancelled
    ? setInterval(() => {
        if (options.isCancelled?.()) abort();
      }, CANCEL_POLL_MS)
    : undefined;
  return {
    controller,
    dispose() {
      clearInterval(poll);
      options.signal?.removeEventListener("abort", abort);
    },
  };
}

class ResearchTimeout extends Error {
  readonly kind = "research";
  readonly code = "research.timeout";
}

/** Settle with `promise`, or reject on abort / after `ms` (the IPC call itself cannot be stopped). */
function within<T>(promise: Promise<T>, signal: AbortSignal, ms: number): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const done = () => {
      clearTimeout(timer);
      signal.removeEventListener("abort", onAbort);
    };
    const onAbort = () => {
      done();
      reject(new DOMException("research skipped", "AbortError"));
    };
    const timer = setTimeout(
      () => {
        done();
        reject(new ResearchTimeout("research step timed out"));
      },
      Math.max(0, ms),
    );
    if (signal.aborted) return onAbort();
    signal.addEventListener("abort", onAbort, { once: true });
    promise.then(
      (value) => {
        done();
        resolve(value);
      },
      (error: unknown) => {
        done();
        reject(error);
      },
    );
  });
}

// ── search / search_scrape ───────────────────────────────────────────────────

async function runSearch(
  query: string,
  scrape: boolean,
  options: RunResearchOptions,
  signal: AbortSignal,
): Promise<ResearchOutcome | null> {
  const depth = scrape ? "search_scrape" : "search";
  const deadline = Date.now() + (options.searchTimeoutMs ?? DEFAULT_SEARCH_TIMEOUT_MS);
  const status = (event: DeepResearchEvent) => options.bus?.emit?.("research.event", event);
  const jobId = options.jobId;
  const startedAt = Date.now();
  status({ type: "started", jobId });
  let items: ResearchContextItem[] = [];
  try {
    status({ type: "tool_call", jobId, tool: "exa_search", input: {} });
    const results = await within(options.api.research.search({ query }), signal, deadline - Date.now());
    items = [snippetsItem(results)];
    if (scrape) {
      status({ type: "tool_call", jobId, tool: "firecrawl_scrape", input: {} });
      // In parallel, each bounded by its own and the overall deadline; a failed
      // or slow page just leaves its snippet (LIVE-006).
      const top = items[0]!.citations.slice(0, options.scrapeTopN ?? DEFAULT_SCRAPE_TOP_N);
      const pages = await Promise.allSettled(
        top.map((c) =>
          within(
            options.api.research.scrape({ url: c.url }),
            signal,
            Math.min(SCRAPE_TIMEOUT_MS, deadline - Date.now()),
          ),
        ),
      );
      if (signal.aborted) throw new DOMException("research skipped", "AbortError");
      pages.forEach((page, index) => {
        if (page.status === "fulfilled") items.push(pageItem(page.value, index, top[index]!));
      });
    }
    const outcome = outcomeOf(depth, items);
    status({
      type: "completed",
      jobId,
      report: "",
      citations: outcome.citations,
      totalMs: Date.now() - startedAt,
      turns: 0,
    });
    return outcome;
  } catch (error) {
    status({ type: "failed", jobId, error: toBlueyError(error, "research") });
    // Skipped or the ask went away: continue without research, no notice.
    if (signal.aborted) return null;
    return failureOutcome(depth, error);
  }
}

// ── deep_agent ───────────────────────────────────────────────────────────────

function runDeepAgent(
  query: string,
  options: RunResearchOptions,
  signal: AbortSignal,
): Promise<ResearchOutcome | null> {
  const bus = options.bus;
  if (!bus) return Promise.resolve(null);
  const timeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT_MS;
  const cancel = () => void options.api.research.deepCancel({ jobId: options.jobId }).catch(() => false);

  return new Promise((resolve) => {
    let settled = false;
    const finish = (value: ResearchOutcome | null) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      signal.removeEventListener("abort", onAbort);
      off();
      resolve(value);
    };
    // The ask was cancelled or superseded: stop paying for the job (AI-010).
    const onAbort = () => {
      cancel();
      finish(null);
    };

    const off = bus.on("research.event", (event) => {
      if (event.jobId !== options.jobId) return;
      if (event.type === "completed") {
        const report = clip(event.report, REPORT_MAX_CHARS, "[… report truncated …]");
        const item = {
          ref: "research:report",
          content: `${UNTRUSTED_HEADER}\n\n${report}`,
          relevance: 0.8,
          citations: event.citations,
        };
        finish(outcomeOf("deep_agent", [item]));
      } else if (event.type === "failed") {
        finish(event.error.kind === "cancelled" ? null : failureOutcome("deep_agent", event.error));
      }
    });

    const timer = setTimeout(() => {
      cancel();
      finish(failureOutcome("deep_agent", new ResearchTimeout("deep research timed out")));
    }, timeoutMs);
    if (signal.aborted) return onAbort();
    signal.addEventListener("abort", onAbort, { once: true });

    const request: DeepResearchRequest = {
      jobId: options.jobId,
      query,
      goal: query,
      // Only tools with keys: the sidecar fails a job whose tools lack one (PROV-013).
      tools: options.availability?.scrape === false ? ["exa_search"] : ["exa_search", "firecrawl_scrape"],
      deadlineMs: Math.max(timeoutMs - DEEP_REPORT_HEADROOM_MS, timeoutMs / 2),
    };
    options.api.research.deepStart({ request }).catch((error: unknown) => {
      finish(failureOutcome("deep_agent", error));
    });
  });
}

/**
 * Run the chosen depth. Never fails the ask: a failure comes back as an
 * outcome with no items and a `note`; skipping or cancelling returns `null`.
 */
export async function runResearch(
  depth: ResearchDepth,
  query: string,
  options: RunResearchOptions,
): Promise<ResearchOutcome | null> {
  const trimmed = query.trim();
  if (depth === "none" || trimmed.length === 0) return null;
  const { controller, dispose } = linkCancellation(options);
  try {
    if (depth === "deep_agent") return await runDeepAgent(trimmed, options, controller.signal);
    localJobs.set(options.jobId, controller);
    return await runSearch(trimmed, depth === "search_scrape", options, controller.signal);
  } finally {
    localJobs.delete(options.jobId);
    dispose();
  }
}
