import { beforeEach, describe, expect, it } from "vitest";

import { derivePill } from "@/features/hud/state-pill";
import type { MockTransport } from "@/lib/tauri/mock";
import type { AppStatus } from "@/lib/types";
import { describeToolCall, useResearchStore } from "@/stores/researchStore";
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

    mock.emit("research.event", { type: "progress", jobId: "job-1", message: "Comparing sources" });
    expect(useResearchStore.getState().active?.message).toBe("Comparing sources");

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
