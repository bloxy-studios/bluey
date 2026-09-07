import { buildRetrievalQuery, inferKinds, retrieveRelevantContext } from "@/context/retrieval";
import type { RetrievalQuery, RetrievedChunk } from "@/lib/types";
import { makeMode, makeSession, makeSettings, makeSnapshot, makeSegment } from "../../fixtures/helpers/builders";

function fakeApi(result: RetrievedChunk[] = []) {
  const queries: RetrievalQuery[] = [];
  return {
    queries,
    api: {
      documents: {
        retrieve: async ({ query }: { query: RetrievalQuery }) => {
          queries.push(query);
          return result;
        },
      },
    },
  };
}

const interviewMode = makeMode({
  id: "interview",
  responseSchema: "suggested-response",
  contextRequirements: ["transcript", "resume", "job_description", "session_memory"],
  group: "Looking for work",
});

describe("retrieveRelevantContext", () => {
  it("skips retrieval when the mode declares no document needs", async () => {
    const { api, queries } = fakeApi();
    const chunks = await retrieveRelevantContext({
      instruction: "why us?",
      mode: makeMode({ contextRequirements: ["transcript"] }),
      settings: makeSettings(),
      api,
    });
    expect(chunks).toEqual([]);
    expect(queries).toHaveLength(0);
  });

  it("queries scopes in session → mode → global priority order", async () => {
    const { api, queries } = fakeApi();
    await retrieveRelevantContext({
      instruction: "Why do you want to work here?",
      mode: interviewMode,
      session: makeSession({ id: "ses_42" }),
      settings: makeSettings(),
      api,
    });
    expect(queries).toHaveLength(1);
    expect(queries[0]?.scopes).toEqual([
      { scope: "session", scopeId: "ses_42" },
      { scope: "mode", scopeId: "interview" },
      { scope: "global" },
    ]);
  });

  it("omits the session scope when no session is active", async () => {
    const { api, queries } = fakeApi();
    await retrieveRelevantContext({
      instruction: "Why do you want to work here?",
      mode: interviewMode,
      settings: makeSettings(),
      api,
    });
    expect(queries[0]?.scopes[0]).toEqual({ scope: "mode", scopeId: "interview" });
  });

  it("infers candidate kinds for candidate modes and JD kinds for role questions", () => {
    const candidateKinds = inferKinds(interviewMode, "tell me about your background");
    expect(candidateKinds).toEqual(expect.arrayContaining(["resume", "cv", "experience", "skills"]));
    expect(candidateKinds).toEqual(
      expect.arrayContaining(["job_description", "role_description", "company_notes"]),
    );

    const salesMode = makeMode({ id: "sales", responseSchema: "sales", contextRequirements: ["transcript", "documents"] });
    const kinds = inferKinds(salesMode, "what does the company need");
    expect(kinds).toEqual(expect.arrayContaining(["job_description", "role_description", "company_notes", "notes", "other"]));
    expect(kinds).not.toContain("resume");
  });

  it("uses keyword strategy when embeddings are disabled and auto when enabled", async () => {
    const { api, queries } = fakeApi();
    const base = {
      instruction: "why us?",
      mode: interviewMode,
      settings: makeSettings({ ai: { embeddingsEnabled: false } }),
      api,
    };
    await retrieveRelevantContext(base);
    await retrieveRelevantContext({ ...base, settings: makeSettings({ ai: { embeddingsEnabled: true } }) });
    expect(queries[0]?.strategy).toBe("keyword");
    expect(queries[1]?.strategy).toBe("auto");
  });

  it("builds the query from instruction + last question heard + OCR headline", () => {
    const snapshot = makeSnapshot({
      transcript: {
        segments: [
          makeSegment({ id: "a", text: "We ship on Fridays." }),
          makeSegment({ id: "b", text: "What experience do you have with Kubernetes?" }),
        ],
        windowSeconds: 180,
      },
      ocr: { blocks: [], text: "Senior Platform Engineer — Acme Corp\nApply now", level: "fast", languages: ["en"], durationMs: 3 },
    });
    const query = buildRetrievalQuery({ instruction: "Help me answer this", snapshot });
    expect(query).toContain("Help me answer this");
    expect(query).toContain("Kubernetes");
    expect(query).toContain("Senior Platform Engineer");
  });

  it("returns [] on backend failure instead of throwing", async () => {
    const api = {
      documents: {
        retrieve: async () => {
          throw new Error("boom");
        },
      },
    };
    const chunks = await retrieveRelevantContext({
      instruction: "why us?",
      mode: interviewMode,
      settings: makeSettings(),
      api,
    });
    expect(chunks).toEqual([]);
  });

  it("sorts returned chunks by score descending", async () => {
    const { api } = fakeApi([
      { chunkId: "lo", documentId: "d", documentTitle: "t", documentKind: "resume", content: "x", score: 0.2, scope: "global" },
      { chunkId: "hi", documentId: "d", documentTitle: "t", documentKind: "resume", content: "y", score: 0.9, scope: "global" },
    ]);
    const chunks = await retrieveRelevantContext({
      instruction: "why us?",
      mode: interviewMode,
      settings: makeSettings(),
      api,
    });
    expect(chunks.map((c) => c.chunkId)).toEqual(["hi", "lo"]);
  });
});
