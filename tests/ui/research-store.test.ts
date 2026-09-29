import { beforeEach, describe, expect, it } from "vitest";

import { derivePill } from "@/features/hud/state-pill";
import type { MockTransport } from "@/lib/tauri/mock";
import type { AppStatus } from "@/lib/types";
import { runResearch } from "@/ai/research";
import { describeProgress, describeToolCall, useResearchStore } from "@/stores/researchStore";
import { setupMockApp } from "./helpers";

function status(partial: Partial<AppStatus> = {}): AppStatus {
  return {
    state: "thinking",
    audioActive: false,
    modeId: "general",
    updatedAt: new Date().toISOString(),
    ...partial,
  };
}

describe("researchStore", () => {
  let mock: MockTransport;

  beforeEach(async () => {
    mock = await setupMockApp();
  });

  it("mirrors the job lifecycle from research.event", () => {
    mock.emit("research.event", { type: "started", jobId: "job-1" });
    expect(useResearchStore.getState().active).toMatchObject({
      jobId: "job-1",
      message: "Researching…",
      toolCalls: 0,
    });

    mock.emit("research.event", {
      type: "tool_call",
      jobId: "job-1",
      tool: "exa_search",
      input: { query: "x" },
    });
    mock.emit("research.event", {
      type: "tool_call",
      jobId: "job-1",
      tool: "firecrawl_scrape",
      input: { url: "u" },
    });
    expect(useResearchStore.getState().active).toMatchObject({ message: "Reading a page…", toolCalls: 2 });

    mock.emit("research.event", {
      type: "progress",
      jobId: "job-1",
      message: "firecrawl_scrape: fetched https://www.example.com/pricing",
    });
    expect(useResearchStore.getState().active?.message).toBe("Read example.com");

    mock.emit("research.event", { type: "text_delta", jobId: "job-1", text: "The" });
    expect(useResearchStore.getState().active?.message).toBe("Writing up findings…");

    // Events for another job never leak into the active one.
    mock.emit("research.event", {
      type: "completed",
      jobId: "job-other",
      report: "",
      citations: [],
      totalMs: 1,
      turns: 1,
    });
    expect(useResearchStore.getState().active?.jobId).toBe("job-1");

    mock.emit("research.event", {
      type: "completed",
      jobId: "job-1",
      report: "done",
      citations: [],
      totalMs: 900,
      turns: 3,
    });
    expect(useResearchStore.getState().active).toBeNull();
  });

  it("clears on failure and marks a skip request", async () => {
    mock.emit("research.event", { type: "started", jobId: "job-2" });
    await useResearchStore.getState().skip();
    expect(useResearchStore.getState().active).toMatchObject({
      cancelling: true,
      message: "Skipping research…",
    });

    mock.emit("research.event", {
      type: "failed",
      jobId: "job-2",
      error: { kind: "cancelled", code: "cancelled", message: "cancelled", recoverable: false },
    });
    expect(useResearchStore.getState().active).toBeNull();
  });

  it("shows friendly progress instead of the agent's developer strings (UX-030)", () => {
    expect(describeProgress("agent session started (model gemini-3.5-pro, 2 tool(s))")).toBeNull();
    expect(describeProgress('exa_search: 8 result(s) for "vercel pricing"')).toBe("Found 8 sources");
    expect(describeProgress("firecrawl_scrape: fetched https://a.com/x")).toBe("Read a.com");
    expect(describeProgress("writing the report")).toBe("Writing up findings…");

    mock.emit("research.event", { type: "started", jobId: "job-3" });
    mock.emit("research.event", {
      type: "progress",
      jobId: "job-3",
      message: "agent session started (model m, 2 tool(s))",
    });
    expect(useResearchStore.getState().active?.message).toBe("Researching…");
  });

  it("Skip ends an in-engine web search at once, leaving no stale status (LIVE-006)", async () => {
    const pending = runResearch("search_scrape", "vercel pricing", {
      jobId: "res_local",
      api: {
        research: {
          search: () => new Promise(() => {}),
          scrape: () => new Promise(() => {}),
          deepStart: async () => undefined,
          deepCancel: async () => false,
        },
      },
      bus: { on: () => () => undefined, emit: (_name, event) => useResearchStore.getState().apply(event) },
    });
    expect(useResearchStore.getState().active).toMatchObject({
      jobId: "res_local",
      message: "Searching the web…",
    });

    await useResearchStore.getState().skip();

    expect(await pending).toBeNull();
    expect(useResearchStore.getState().active).toBeNull();
  });

  it("describes tool calls in plain words", () => {
    expect(describeToolCall("exa_search")).toBe("Searching the web…");
    expect(describeToolCall("document_read")).toBe("Reading your documents…");
    expect(describeToolCall("custom_tool")).toBe("Running custom_tool…");
  });

  it("wins the pill while the ask is busy, and only then", () => {
    expect(derivePill(status(), "thinking", false, "General", { researching: "Searching the web…" })).toEqual(
      {
        kind: "researching",
        message: "Searching the web…",
      },
    );
    expect(derivePill(status(), "analyzing", false, "General", { researching: "x" }).kind).toBe(
      "researching",
    );
    expect(derivePill(status({ state: "ready" }), "done", false, "General", { researching: "x" }).kind).toBe(
      "idle",
    );
    expect(
      derivePill(status({ state: "error" }), "thinking", false, "General", { researching: "x" }).kind,
    ).toBe("error");
  });
});
