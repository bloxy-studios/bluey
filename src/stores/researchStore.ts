/**
 * Live view of the deep-research job attached to the current ask. The engine
 * runs research inside the pipeline (best-effort, never fails the ask); this
 * store mirrors `research.event` so the HUD can show what the agent is doing
 * and offer to skip it.
 */

import { create } from "zustand";

import { skipLocalResearch } from "@/ai/research";
import { showErrorToast } from "@/components/ui/toast-store";
import { bluey } from "@/lib/tauri/api";
import { toBlueyError, type DeepResearchEvent } from "@/lib/types";

export interface ResearchActivity {
  jobId: string;
  /** Human progress line ("Searching the web…"). */
  message: string;
  toolCalls: number;
  startedAt: number;
  /** True once `deepCancel` was requested from the UI. */
  cancelling: boolean;
}

interface ResearchStore {
  active: ResearchActivity | null;
  apply(event: DeepResearchEvent): void;
  /** Ask the backend to stop the job; the ask continues without research. */
  skip(): Promise<void>;
  clear(): void;
}

export function describeToolCall(tool: string): string {
  switch (tool) {
    case "exa_search":
      return "Searching the web…";
    case "firecrawl_scrape":
      return "Reading a page…";
    case "document_read":
      return "Reading your documents…";
    default:
      return `Running ${tool}…`;
  }
}

function hostOf(url: string): string {
  try {
    return new URL(url).host.replace(/^www\./, "");
  } catch {
    return "a page";
  }
}

/**
 * Plain words for the agent's developer progress lines (UX-030). `null`
 * keeps the current line: session start, tool failures and anything
 * unrecognised are not worth showing.
 */
export function describeProgress(message: string): string | null {
  const found = /^exa_search: (\d+) result/.exec(message);
  if (found) return found[1] === "1" ? "Found 1 source" : `Found ${found[1]} sources`;
  const fetched = /^firecrawl_scrape: fetched (\S+)/.exec(message);
  if (fetched) return `Read ${hostOf(fetched[1]!)}`;
  if (message.startsWith("document_read: loaded")) return "Read your documents";
  if (/writing the report/i.test(message)) return "Writing up findings…";
  return null;
}

export const useResearchStore = create<ResearchStore>((set, get) => ({
  active: null,
  apply: (event) => {
    const current = get().active;
    switch (event.type) {
      case "started":
        set({
          active: {
            jobId: event.jobId,
            message: "Researching…",
            toolCalls: 0,
            startedAt: Date.now(),
            cancelling: false,
          },
        });
        return;
      case "progress": {
        if (current?.jobId !== event.jobId || current.cancelling) return;
        const message = describeProgress(event.message);
        if (message) set({ active: { ...current, message } });
        return;
      }
      case "tool_call":
        if (current?.jobId !== event.jobId) return;
        set({
          active: { ...current, message: describeToolCall(event.tool), toolCalls: current.toolCalls + 1 },
        });
        return;
      case "text_delta":
        if (current?.jobId !== event.jobId || current.message === "Writing up findings…") return;
        set({ active: { ...current, message: "Writing up findings…" } });
        return;
      case "completed":
      case "failed":
        if (current?.jobId === event.jobId) set({ active: null });
        return;
    }
  },
  skip: async () => {
    const current = get().active;
    if (!current || current.cancelling) return;
    // A web search runs in the engine: skipping it ends it right away (LIVE-006).
    if (skipLocalResearch(current.jobId)) {
      set({ active: null });
      return;
    }
    set({ active: { ...current, cancelling: true, message: "Skipping research…" } });
    try {
      await bluey.research.deepCancel({ jobId: current.jobId });
    } catch (error) {
      showErrorToast(toBlueyError(error, "research"));
      const still = get().active;
      if (still?.jobId === current.jobId) set({ active: { ...still, cancelling: false } });
    }
  },
  clear: () => set({ active: null }),
}));
