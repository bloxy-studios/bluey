/**
 * Firecrawl v2 scrape client (https://api.firecrawl.dev/v2/scrape).
 *
 * fetch-based, hard timeout, typed errors. The API key comes from the
 * environment (FIRECRAWL_API_KEY) injected by the Rust backend.
 */

import { postJson, ToolError, type FetchLike } from "./errors";

export const FIRECRAWL_SCRAPE_URL = "https://api.firecrawl.dev/v2/scrape";
const DEFAULT_TIMEOUT_MS = 45_000;
/** Keep scraped pages bounded so a single page cannot blow the agent context. */
export const DEFAULT_MAX_MARKDOWN_CHARS = 40_000;

export interface FirecrawlScrapeResult {
  url: string;
  title?: string;
  markdown: string;
  truncated: boolean;
}

export interface FirecrawlClient {
  scrape(url: string): Promise<FirecrawlScrapeResult>;
}

export interface FirecrawlClientOptions {
  apiKey: string;
  fetchImpl?: FetchLike;
  timeoutMs?: number;
  baseUrl?: string;
  maxMarkdownChars?: number;
}

/** Map a raw Firecrawl v2 `/scrape` response onto our result shape. */
export function mapFirecrawlResponse(
  json: unknown,
  requestedUrl: string,
  maxMarkdownChars: number = DEFAULT_MAX_MARKDOWN_CHARS,
): FirecrawlScrapeResult {
  if (typeof json !== "object" || json === null) {
    throw new ToolError("invalid_response", "firecrawl scrape returned an unexpected body");
  }
  const top = json as Record<string, unknown>;
  if (top["success"] === false) {
    const detail = typeof top["error"] === "string" ? `: ${top["error"].slice(0, 200)}` : "";
    throw new ToolError("http_error", `firecrawl scrape reported failure${detail}`);
  }
  const data = top["data"];
  if (typeof data !== "object" || data === null) {
    throw new ToolError("invalid_response", "firecrawl scrape response has no data object");
  }
  const d = data as Record<string, unknown>;
  const markdownRaw = typeof d["markdown"] === "string" ? d["markdown"] : "";
  if (!markdownRaw.trim()) {
    throw new ToolError("invalid_response", "firecrawl scrape returned no markdown content");
  }
  const metadata =
    typeof d["metadata"] === "object" && d["metadata"] !== null
      ? (d["metadata"] as Record<string, unknown>)
      : {};
  const sourceUrl = typeof metadata["sourceURL"] === "string" ? metadata["sourceURL"] : requestedUrl;
  const title =
    typeof metadata["title"] === "string" && metadata["title"].trim()
      ? metadata["title"].trim()
      : undefined;

  const truncated = markdownRaw.length > maxMarkdownChars;
  const markdown = truncated
    ? `${markdownRaw.slice(0, maxMarkdownChars)}\n\n[… content truncated by bluey-agent …]`
    : markdownRaw;

  const result: FirecrawlScrapeResult = { url: sourceUrl, markdown, truncated };
  if (title) result.title = title;
  return result;
}

export function createFirecrawlClient(options: FirecrawlClientOptions): FirecrawlClient {
  const { apiKey, fetchImpl, baseUrl } = options;
  const timeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT_MS;
  const maxMarkdownChars = options.maxMarkdownChars ?? DEFAULT_MAX_MARKDOWN_CHARS;
  if (!apiKey) {
    throw new ToolError("missing_api_key", "FIRECRAWL_API_KEY is not set");
  }
  return {
    async scrape(url: string): Promise<FirecrawlScrapeResult> {
      const json = await postJson({
        url: baseUrl ?? FIRECRAWL_SCRAPE_URL,
        headers: { authorization: `Bearer ${apiKey}` },
        body: { url, formats: ["markdown"], onlyMainContent: true },
        timeoutMs,
        fetchImpl,
        label: "firecrawl scrape",
      });
      return mapFirecrawlResponse(json, url, maxMarkdownChars);
    },
  };
}
