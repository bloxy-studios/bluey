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
  it("runs only the personal-instructions and relevance-floored passes when the mode declares no document needs", async () => {
    const { api, queries } = fakeApi();
    const chunks = await retrieveRelevantContext({
      instruction: "why us?",
      mode: makeMode({ contextRequirements: ["transcript"] }),
      settings: makeSettings(),
      api,
    });
    expect(chunks).toEqual([]);
    expect(queries.map((q) => [q.strategy, q.kinds, q.limit])).toEqual([
      ["leading", ["personal_instructions"], 6],
      ["keyword", undefined, 3],
    ]);
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
    const personal = queries.find((q) => q.kinds?.includes("personal_instructions"));
    expect(personal?.scopes).toEqual([
      { scope: "session", scopeId: "ses_42" },
      { scope: "mode", scopeId: "interview" },
      { scope: "global" },
    ]);
    const declared = queries.find((q) => q.kinds?.includes("job_description"));
    expect(declared?.scopes).toEqual([{ scope: "session", scopeId: "ses_42" }, { scope: "global" }]);
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
    const declared = queries.filter((q) => q.kinds?.includes("job_description"));
    expect(declared.map((q) => q.strategy)).toEqual(["keyword", "auto"]);
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

function chunk(id: string, kind: RetrievedChunk["documentKind"], score: number, scope: RetrievedChunk["scope"] = "global"): RetrievedChunk {
  return { chunkId: id, documentId: `d_${id}`, documentTitle: id, documentKind: kind, content: `${id} text`, score, scope };
}

/** A backend that answers each query from its own table (by strategy/kinds/scope). */
function routedApi(route: (query: RetrievalQuery) => RetrievedChunk[]) {
  const queries: RetrievalQuery[] = [];
  const api = {
    documents: {
      retrieve: async ({ query }: { query: RetrievalQuery }) => {
        queries.push(query);
        return route(query);
      },
    },
  };
  return { api, queries };
}

const generalMode = makeMode({ id: "general", contextRequirements: ["screen", "transcript"] });

describe("retrieval beyond the declared kinds (CTX-002/003/008)", () => {
  it("searches a mode's own files with no kind filter when files are attached", async () => {
    const notes = chunk("mode_notes", "notes", 0.7, "mode");
    const { api, queries } = routedApi((q) => (q.scopes.length === 1 && q.scopes[0]?.scope === "mode" ? [notes] : []));
    const mode = makeMode({ id: "coding", contextRequirements: ["screen"], attachedDocumentIds: ["d_mode_notes"] });
    const chunks = await retrieveRelevantContext({ instruction: "use my notes on graphs", mode, settings: makeSettings(), api });
    expect(chunks.map((c) => c.chunkId)).toEqual(["mode_notes"]);
    expect(queries.find((q) => q.scopes.length === 1)?.kinds).toBeUndefined();
  });

  it("keeps library chunks above the relevance floor in a mode with no document needs", async () => {
    const { api } = routedApi((q) =>
      q.strategy === "keyword" && !q.kinds ? [chunk("strong", "resume", 0.8), chunk("weak", "resume", 0.36, "session")] : [],
    );
    const chunks = await retrieveRelevantContext({ instruction: "my Kafka project", mode: generalMode, settings: makeSettings(), api });
    expect(chunks.map((c) => c.chunkId)).toEqual(["strong"]);
  });

  it("loads personal instructions in every mode, after the matched chunks, in document order", async () => {
    const { api } = routedApi((q) =>
      q.strategy === "leading" && q.kinds?.includes("personal_instructions")
        ? [chunk("pi_1", "personal_instructions", 1), chunk("pi_2", "personal_instructions", 1, "mode")]
        : [],
    );
    const chunks = await retrieveRelevantContext({ instruction: "", mode: generalMode, settings: makeSettings(), api });
    expect(chunks.map((c) => c.chunkId)).toEqual(["pi_1", "pi_2"]);
  });

  it("pins the résumé's leading chunks for an intro question that matched none of it", async () => {
    const { api, queries } = routedApi((q) =>
      q.strategy === "leading" && q.kinds?.includes("resume") ? [chunk("res_0", "resume", 1), chunk("res_1", "resume", 1)] : [],
    );
    const chunks = await retrieveRelevantContext({
      instruction: "Tell me about yourself",
      mode: interviewMode,
      settings: makeSettings(),
      api,
    });
    expect(chunks.map((c) => c.chunkId)).toEqual(["res_0", "res_1"]);
    expect(queries.find((q) => q.kinds?.includes("resume") && q.strategy === "leading")?.limit).toBe(2);
  });

  it("does not pin the résumé when a specific question already matched it", async () => {
    const { api } = routedApi((q) => {
      if (q.strategy === "leading" && q.kinds?.includes("resume")) return [chunk("res_0", "resume", 1)];
      if (q.kinds?.includes("job_description")) return [chunk("res_kafka", "resume", 0.6)];
      return [];
    });
    const chunks = await retrieveRelevantContext({
      instruction: "How did you scale Kafka consumers?",
      mode: interviewMode,
      settings: makeSettings(),
      api,
    });
    expect(chunks.map((c) => c.chunkId)).toEqual(["res_kafka"]);
  });

  it("never pins the résumé outside candidate modes", async () => {
    const { queries, api } = routedApi(() => []);
    await retrieveRelevantContext({ instruction: "Tell me about yourself", mode: generalMode, settings: makeSettings(), api });
    expect(queries.some((q) => q.kinds?.includes("resume"))).toBe(false);
  });
});
