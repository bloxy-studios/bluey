import { beforeEach, describe, expect, it } from "vitest";

import { MockTransport } from "@/lib/tauri/mock";

/** The mock's `documents_retrieve` is never more permissive than Rust's keyword retrieval. */
describe("MockTransport documents_retrieve", () => {
  let mock: MockTransport;

  beforeEach(async () => {
    mock = new MockTransport({ streamDelayMs: 0, levelTicks: false });
    await mock.invoke("documents_delete_all", {});
    for (const [title, kind] of [
      ["Kubernetes runbook", "notes"],
      ["Quarterly planning", "notes"],
      ["Jordan Lee", "resume"],
    ] as const) {
      await mock.invoke("documents_add", { input: { kind, scope: "global", content: "text", title } });
    }
  });

  const retrieve = (query: string, strategy?: "leading") =>
    mock.invoke("documents_retrieve", { query: { query, scopes: [], limit: 4, strategy } });

  it("returns only documents that share a keyword with the query", async () => {
    const titles = (await retrieve("How do I restart a kubernetes pod?")).map((chunk) => chunk.documentTitle);
    expect(titles).toEqual(["Kubernetes runbook"]);
    expect(await retrieve("What's the capital of Australia?")).toEqual([]);
  });

  it("matches a document by its kind", async () => {
    const titles = (await retrieve("walk me through my resume")).map((chunk) => chunk.documentTitle);
    expect(titles).toEqual(["Jordan Lee"]);
  });

  it("lists the leading documents whatever the query says", async () => {
    expect(await retrieve("", "leading")).toHaveLength(3);
  });
});
