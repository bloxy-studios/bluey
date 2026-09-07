/**
 * Exa search client (https://api.exa.ai/search).
 *
 * fetch-based, hard timeout, typed errors. The API key comes from the
 * environment (EXA_API_KEY) injected by the Rust backend.
 */

import { postJson, ToolError, type FetchLike } from "./errors";

export const EXA_SEARCH_URL = "https://api.exa.ai/search";
const DEFAULT_TIMEOUT_MS = 20_000;
const DEFAULT_NUM_RESULTS = 8;
const MAX_NUM_RESULTS = 10;
const HIGHLIGHT_MAX_CHARACTERS = 600;

export interface ExaSearchParams {
  query: string;
  numResults?: number;
  /** ISO date — only return results published after this date. */
  startPublishedDate?: string;
}

export interface ExaResultItem {
  title: string;
  url: string;
  snippet?: string;
  publishedDate?: string;
  author?: string;
}

export interface ExaClient {
  search(params: ExaSearchParams): Promise<ExaResultItem[]>;
}

export interface ExaClientOptions {
  apiKey: string;
  fetchImpl?: FetchLike;
  timeoutMs?: number;
  baseUrl?: string;
}

/** Map a raw Exa `/search` response body onto our result items. */
export function mapExaResponse(json: unknown): ExaResultItem[] {
  if (typeof json !== "object" || json === null) {
    throw new ToolError("invalid_response", "exa search returned an unexpected body");
  }
  const results = (json as Record<string, unknown>)["results"];
  if (!Array.isArray(results)) {
    throw new ToolError("invalid_response", "exa search response has no results array");
  }
  const items: ExaResultItem[] = [];
  for (const raw of results) {
    if (typeof raw !== "object" || raw === null) continue;
    const r = raw as Record<string, unknown>;
    const url = typeof r["url"] === "string" ? r["url"] : undefined;
    if (!url) continue;
    const title = typeof r["title"] === "string" && r["title"].trim() ? r["title"] : url;
    const summary = typeof r["summary"] === "string" ? r["summary"].trim() : "";
    const highlights = Array.isArray(r["highlights"])
      ? (r["highlights"] as unknown[]).filter((h): h is string => typeof h === "string")
      : [];
    const snippet = summary || highlights.join(" … ").trim();
    const item: ExaResultItem = { title, url };
    if (snippet) item.snippet = snippet;
    if (typeof r["publishedDate"] === "string") item.publishedDate = r["publishedDate"];
    if (typeof r["author"] === "string" && r["author"].trim()) item.author = r["author"];
    items.push(item);
  }
  return items;
}

export function createExaClient(options: ExaClientOptions): ExaClient {
  const { apiKey, fetchImpl, baseUrl } = options;
  const timeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT_MS;
  if (!apiKey) {
    throw new ToolError("missing_api_key", "EXA_API_KEY is not set");
  }
  return {
    async search(params: ExaSearchParams): Promise<ExaResultItem[]> {
      const numResults = Math.max(
        1,
        Math.min(Math.trunc(params.numResults ?? DEFAULT_NUM_RESULTS), MAX_NUM_RESULTS),
      );
      const body: Record<string, unknown> = {
        query: params.query,
        type: "auto",
        numResults,
        contents: {
          highlights: { maxCharacters: HIGHLIGHT_MAX_CHARACTERS },
          summary: true,
        },
      };
      if (params.startPublishedDate) body["startPublishedDate"] = params.startPublishedDate;

      const json = await postJson({
        url: baseUrl ?? EXA_SEARCH_URL,
        headers: { "x-api-key": apiKey },
        body,
        timeoutMs,
        fetchImpl,
        label: "exa search",
      });
      return mapExaResponse(json);
    },
  };
}
