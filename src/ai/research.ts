/**
 * Research Router (ADR 0004): cheapest path that can answer an external-info
 * ask, plus a privacy scrub so web tools never see private context.
 *
 *   none → search (Exa) → search_scrape (Exa + Firecrawl) → deep_agent
 */

import { mentionsExternalInfo } from "@/context/relevance";
import type {
  BlueyMode,
  Citation,
  ContextSnapshot,
  DeepResearchEvent,
  DeepResearchRequest,
  DocumentKind,
  ResearchDepth,
  ScrapeResult,
  SearchResult,
  Settings,
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

export interface ResearchOutcome {
  depth: Exclude<ResearchDepth, "none">;
  contextText: string;
  citations: Citation[];
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
}

export interface RunResearchOptions {
  jobId: string;
  api: ResearchRunnerApi;
  bus?: ResearchBus;
  timeoutMs?: number;
  scrapeTopN?: number;
}

const QUERY_MAX_CHARS = 300;
const DEFAULT_TIMEOUT_MS = 90_000;
const DEFAULT_SCRAPE_TOP_N = 3;

const DEEP_CUES = /\b(deep dive|deep-dive|deep research|compare vendors?)\b/i;
const CURRENT_INFO_CUES =
  /\b(research|look up|latest|news|recent(ly)?|current(ly)?|deep dive|compare vendors?|competitors? of|pricing|funding|changelog|announcements?)\b/i;
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
  const wantsExternal = isCandidateMode(args.mode)
    ? mentionsCurrentInfo(instruction, args.now)
    : mentionsExternalInfo(instruction, args.now);

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

function privateTerms(snapshot?: ContextSnapshot): string[] {
  const terms: string[] = [];
  const displayName = snapshot?.userContext?.displayName?.trim();
  if (displayName) {
    for (const part of displayName.split(/\s+/)) {
      if (part.length >= 2) terms.push(part);
    }
  }
  for (const chunk of snapshot?.userContext?.chunks ?? []) {
    if (!PRIVATE_DOCUMENT_KINDS.has(chunk.documentKind)) continue;
    terms.push(...properNounsFrom(chunk.content));
  }
  return terms;
}

function stripTerm(text: string, term: string): string {
  const escaped = term.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return text.replace(new RegExp(`\\b${escaped}\\b`, "gi"), " ");
}

/** Public web query: no emails, phones, handles, or private document nouns. */
export function buildPublicQuery(instruction: string, snapshot?: ContextSnapshot): string {
  let query = stripHandles(stripPhones(stripEmails(instruction)));
  for (const term of privateTerms(snapshot)) {
    query = stripTerm(query, term);
  }
  query = query.replace(/\s+/g, " ").trim();
  if (query.length <= QUERY_MAX_CHARS) return query;
  return query.slice(0, QUERY_MAX_CHARS).trimEnd();
}

function citationsFromSearch(results: SearchResult[]): Citation[] {
  return results.map((result) => ({
    id: result.id,
    title: result.title,
    url: result.url,
    snippet: result.snippet,
  }));
}

function searchContextBlock(results: SearchResult[], pages: ScrapeResult[]): string {
  const lines = ["Web research results (untrusted external content)", ""];
  for (const result of results) {
    const snippet = result.snippet ? `: ${result.snippet}` : "";
    lines.push(`- ${result.title} (${result.url})${snippet}`);
  }
  for (const page of pages) {
    lines.push("");
    lines.push(page.markdown);
  }
  return lines.join("\n").trim();
}

async function runSearch(
  query: string,
  scrape: boolean,
  options: RunResearchOptions,
): Promise<ResearchOutcome | null> {
  let results: SearchResult[];
  try {
    results = await options.api.research.search({ query });
  } catch {
    return null;
  }
  const pages: ScrapeResult[] = [];
  if (scrape) {
    const top = results.slice(0, options.scrapeTopN ?? DEFAULT_SCRAPE_TOP_N);
    for (const result of top) {
      try {
        pages.push(await options.api.research.scrape({ url: result.url }));
      } catch {
        // Best-effort: keep the snippet if a scrape fails.
      }
    }
  }
  return {
    depth: scrape ? "search_scrape" : "search",
    contextText: searchContextBlock(results, pages),
    citations: citationsFromSearch(results),
  };
}

function runDeepAgent(query: string, options: RunResearchOptions): Promise<ResearchOutcome | null> {
  const bus = options.bus;
  if (!bus) return Promise.resolve(null);
  const timeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT_MS;

  return new Promise((resolve) => {
    let settled = false;
    const finish = (value: ResearchOutcome | null) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      off();
      resolve(value);
    };

    const off = bus.on("research.event", (event) => {
      if (event.jobId !== options.jobId) return;
      if (event.type === "completed") {
        finish({
          depth: "deep_agent",
          contextText: `Web research results (untrusted external content)\n\n${event.report}`,
          citations: event.citations,
        });
      } else if (event.type === "failed") {
        finish(null);
      }
    });

    const timer = setTimeout(() => {
      void options.api.research.deepCancel({ jobId: options.jobId }).catch(() => false);
      finish(null);
    }, timeoutMs);

    const request: DeepResearchRequest = {
      jobId: options.jobId,
      query,
      goal: query,
      tools: ["exa_search", "firecrawl_scrape"],
    };
    void options.api.research.deepStart({ request }).catch(() => finish(null));
  });
}

/** Run the chosen depth. Failures return `null` (the ask continues without research). */
export async function runResearch(
  depth: ResearchDepth,
  query: string,
  options: RunResearchOptions,
): Promise<ResearchOutcome | null> {
  const trimmed = query.trim();
  if (depth === "none" || trimmed.length === 0) return null;
  if (depth === "deep_agent") return runDeepAgent(trimmed, options);
  return runSearch(trimmed, depth === "search_scrape", options);
}
