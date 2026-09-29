/**
 * Citation collection for the research job.
 *
 * Every URL returned by an exa_search / firecrawl_scrape call during the run
 * is recorded here. The `research.completed` citation list and every link in
 * the report body are checked against these observed sources, so the sidecar
 * never emits a URL the tools did not actually return — even if the model
 * invents one.
 */

import type { WireCitation } from "./protocol";

/**
 * Normalise a URL for dedupe: lowercase scheme/host (done by URL), drop the
 * fragment, and strip trailing slashes from the path. Query strings are kept
 * (they can be significant). Invalid URLs fall back to a trimmed string key.
 */
export function normalizeUrl(raw: string): string {
  const trimmed = raw.trim();
  try {
    const url = new URL(trimmed);
    url.hash = "";
    let out = url.toString();
    while (out.endsWith("/")) out = out.slice(0, -1);
    return out;
  } catch {
    return trimmed.replace(/\/+$/, "");
  }
}

const SNIPPET_MAX = 400;

function clampSnippet(snippet: string | undefined): string | undefined {
  if (!snippet) return undefined;
  const clean = snippet.replace(/\s+/g, " ").trim();
  if (!clean) return undefined;
  return clean.length > SNIPPET_MAX ? `${clean.slice(0, SNIPPET_MAX - 1)}…` : clean;
}

export class CitationStore {
  private readonly byUrl = new Map<string, WireCitation>();
  /** Pages whose full text reached the model (firecrawl_scrape), by normalised URL. */
  private readonly fetched = new Set<string>();

  /**
   * Record a source observed in a tool result. First title wins; a missing
   * snippet can be back-filled. `fetched` marks a page the model read in full.
   */
  add(
    citation: { title?: string; url: string; snippet?: string },
    options: { fetched?: boolean } = {},
  ): void {
    const url = citation.url?.trim();
    if (!url) return;
    const key = normalizeUrl(url);
    if (options.fetched) this.fetched.add(key);
    const existing = this.byUrl.get(key);
    if (existing) {
      if (!existing.snippet) {
        const snippet = clampSnippet(citation.snippet);
        if (snippet) existing.snippet = snippet;
      }
      return;
    }
    const entry: WireCitation = { title: citation.title?.trim() || url, url };
    const snippet = clampSnippet(citation.snippet);
    if (snippet) entry.snippet = snippet;
    this.byUrl.set(key, entry);
  }

  has(url: string): boolean {
    return this.byUrl.has(normalizeUrl(url));
  }

  get size(): number {
    return this.byUrl.size;
  }

  /** All observed sources, insertion-ordered and deduped. */
  list(): WireCitation[] {
    return [...this.byUrl.values()].map((c) => ({ ...c }));
  }

  /**
   * Build the final citation list for `research.completed`: the sources the
   * model says it used, not everything it saw.
   *
   * Model-provided citations (structured output) and the links left in the
   * report body are validated against the observed set — an invented URL is
   * dropped. When neither names a known source, the pages the model read in
   * full are the fallback. Deduped by normalised URL.
   */
  finalize(
    modelCitations?: Array<{ title: string; url: string; snippet?: string }>,
    report = "",
  ): WireCitation[] {
    const out: WireCitation[] = [];
    const seen = new Set<string>();
    const push = (key: string, title?: string, snippet?: string): void => {
      const observed = this.byUrl.get(key);
      if (!observed || seen.has(key)) return; // never emit a URL the tools did not return
      seen.add(key);
      const entry: WireCitation = { title: title?.trim() || observed.title, url: observed.url };
      const clamped = clampSnippet(snippet) ?? observed.snippet;
      if (clamped) entry.snippet = clamped;
      out.push(entry);
    };

    for (const c of modelCitations ?? []) push(normalizeUrl(c.url ?? ""), c.title, c.snippet);
    for (const url of reportUrls(report)) push(normalizeUrl(url));
    if (out.length === 0) for (const key of this.fetched) push(key);
    return out;
  }

  /**
   * De-link every URL in `report` the tools never returned: a Markdown link
   * keeps its text, a bare URL is reduced to its host (no longer a link).
   */
  sanitizeReport(report: string): string {
    const known = (url: string): boolean => this.has(url);
    const withoutLinks = report.replace(MARKDOWN_LINK, (match, text: string, url: string) =>
      known(url) ? match : text,
    );
    return withoutLinks.replace(BARE_URL, (url: string, offset: number, whole: string) => {
      // Skip the URL half of a Markdown link that survived the first pass.
      if (whole.slice(Math.max(0, offset - 2), offset) === "](") return url;
      return known(url) ? url : hostOf(url);
    });
  }
}

/** `[text](url)` / `[text](url "title")` with an http(s) target. */
const MARKDOWN_LINK = /\[([^\]]*)\]\((https?:\/\/[^\s)]+)(?:\s+"[^"]*")?\)/g;
/** A bare http(s) URL; trailing sentence punctuation is not part of it. */
const BARE_URL = /https?:\/\/[^\s<>()[\]"']+[^\s<>()[\]"'.,;:!?]/g;

/** Every http(s) URL in `report` (Markdown link targets and bare URLs), in order. */
export function reportUrls(report: string): string[] {
  return report.match(BARE_URL) ?? [];
}

function hostOf(url: string): string {
  try {
    return new URL(url).host.replace(/^www\./, "");
  } catch {
    return "";
  }
}

/**
 * The report for a job that ran out of turns or time before the model wrote
 * one: the sources gathered so far, labelled as leads (AI-008). Better than
 * throwing the evidence away.
 */
export function evidenceReport(reason: "turns" | "time", sources: WireCitation[]): string {
  return [
    "## Research stopped early",
    "",
    `The research agent ran out of ${reason} before writing its report. These are the sources it ` +
      "found — leads to check, not conclusions.",
    "",
    "## Sources",
    ...sources.map((s) => {
      const title = s.title.replace(/[[\]]/g, "");
      return `- [${title}](${s.url})${s.snippet ? ` — ${s.snippet}` : ""}`;
    }),
  ].join("\n");
}
