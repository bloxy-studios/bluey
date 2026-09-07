import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ProtocolWriter } from "../../sidecars/agent/src/protocol";
import { DocumentBroker } from "../../sidecars/agent/src/tools/documents";
import { ToolError } from "../../sidecars/agent/src/tools/errors";

function makeWriter() {
  const frames: Array<Record<string, unknown>> = [];
  const writer = new ProtocolWriter({
    write(chunk: string) {
      frames.push(JSON.parse(chunk) as Record<string, unknown>);
      return true;
    },
  });
  return { frames, writer };
}

describe("DocumentBroker", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("refuses ids outside the allow-list WITHOUT emitting document.request", async () => {
    const { frames, writer } = makeWriter();
    const broker = new DocumentBroker(writer, ["doc-1"]);

    const err = await broker.read("doc-secret").catch((e: unknown) => e);
    expect(err).toBeInstanceOf(ToolError);
    expect((err as ToolError).code).toBe("document_not_allowed");
    expect(frames).toHaveLength(0);
    expect(broker.pendingCount).toBe(0);
  });

  it("emits document.request and resolves on the matching document.response", async () => {
    const { frames, writer } = makeWriter();
    const broker = new DocumentBroker(writer, ["doc-1"], { makeRequestId: () => "req-1" });

    const pending = broker.read("doc-1");
    expect(frames).toEqual([
      { event: "document.request", data: { requestId: "req-1", documentId: "doc-1" } },
    ]);

    const handled = broker.handleResponse({
      requestId: "req-1",
      documentId: "doc-1",
      text: "document body",
    });
    expect(handled).toBe(true);
    await expect(pending).resolves.toBe("document body");
    expect(broker.pendingCount).toBe(0);
  });

  it("rejects with document_error when Rust reports an error", async () => {
    const { writer } = makeWriter();
    const broker = new DocumentBroker(writer, ["doc-1"], { makeRequestId: () => "req-1" });

    const pending = broker.read("doc-1");
    broker.handleResponse({ requestId: "req-1", documentId: "doc-1", error: "not found" });
    const err = await pending.catch((e: unknown) => e);
    expect((err as ToolError).code).toBe("document_error");
    expect((err as ToolError).message).toContain("not found");
  });

  it("times out after 10 s by default", async () => {
    const { writer } = makeWriter();
    const broker = new DocumentBroker(writer, ["doc-1"], { makeRequestId: () => "req-1" });

    const pending = broker.read("doc-1");
    const caught = pending.catch((e: unknown) => e);
    await vi.advanceTimersByTimeAsync(9_999);
    expect(broker.pendingCount).toBe(1);
    await vi.advanceTimersByTimeAsync(2);
    const err = await caught;
    expect((err as ToolError).code).toBe("document_timeout");
    expect(broker.pendingCount).toBe(0);
  });

  it("ignores unknown request ids", () => {
    const { writer } = makeWriter();
    const broker = new DocumentBroker(writer, ["doc-1"]);
    expect(
      broker.handleResponse({ requestId: "nope", documentId: "doc-1", text: "x" }),
    ).toBe(false);
  });

  it("close() rejects everything in flight and refuses new reads", async () => {
    const { writer } = makeWriter();
    const broker = new DocumentBroker(writer, ["doc-1"]);

    const pending = broker.read("doc-1").catch((e: unknown) => e);
    broker.close("job cancelled");
    expect(((await pending) as ToolError).code).toBe("cancelled");

    const afterClose = await broker.read("doc-1").catch((e: unknown) => e);
    expect((afterClose as ToolError).code).toBe("cancelled");
  });
});
