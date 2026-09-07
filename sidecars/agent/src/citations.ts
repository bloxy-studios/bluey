/**
 * Citation collection for the research job.
 *
 * Every URL returned by an exa_search / firecrawl_scrape call during the run
 * is recorded here. The final `research.completed` citation list is built from
 * these observed sources, so the sidecar can never emit a URL the tools did
 * not actually return — even if the model invents one.
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

  /** Record a source observed in a tool result. First title wins; a missing snippet can be back-filled. */
  add(citation: { title?: string; url: string; snippet?: string }): void {
    const url = citation.url?.trim();
    if (!url) return;
    const key = normalizeUrl(url);
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
   * Build the final citation list for `research.completed`.
   *
   * Model-provided citations (structured output) are validated against the
   * observed set — an invented URL is dropped. Validated model citations come
   * first (the model picked their titles/snippets deliberately), then every
   * remaining observed source is appended so the list covers every exa /
   * firecrawl result actually used. Deduped by normalised URL.
   */
  finalize(modelCitations?: Array<{ title: string; url: string; snippet?: string }>): WireCitation[] {
    const out: WireCitation[] = [];
    const seen = new Set<string>();

    for (const c of modelCitations ?? []) {
      const key = normalizeUrl(c.url ?? "");
      if (!key || seen.has(key)) continue;
      const observed = this.byUrl.get(key);
      if (!observed) continue; // never emit a URL the tools did not return
      seen.add(key);
      const entry: WireCitation = {
        title: c.title?.trim() || observed.title,
        url: observed.url,
      };
      const snippet = clampSnippet(c.snippet) ?? observed.snippet;
      if (snippet) entry.snippet = snippet;
      out.push(entry);
    }

    for (const [key, citation] of this.byUrl) {
      if (seen.has(key)) continue;
      seen.add(key);
      out.push({ ...citation });
    }

    return out;
  }
}
