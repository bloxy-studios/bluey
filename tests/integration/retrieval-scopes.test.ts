/**
 * Document retrieval through the real api layer: scope priority ordering
 * (session → mode → global), kind inference, and chunk rendering into the
 * prompt with provenance labels.
 */

import { createResponseEngine } from "@/ai/engine";
import { setTransport } from "@/lib/tauri/transport";
import type { ContextSnapshot, RetrievedChunk } from "@/lib/types";
import { FakeTransport } from "../fixtures/helpers/fake-transport";
import { makeSession, makeSettings } from "../fixtures/helpers/builders";
import { loadFixture } from "../fixtures/helpers/fixtures";

const fixture = loadFixture("interview");

const chunks: RetrievedChunk[] = [
  {
    chunkId: "c_res",
    documentId: "d_res",
    documentTitle: "Resume",
    documentKind: "resume",
    content: "Six years building payment platforms at FinCo; led a team of five engineers.",
    score: 0.9,
    scope: "global",
  },
  {
    chunkId: "c_jd",
    documentId: "d_jd",
    documentTitle: "Acme JD",
    documentKind: "job_description",
    content: "Acme is hiring a senior platform engineer to own the billing pipeline.",
    score: 0.74,
    scope: "mode",
  },
  {
    chunkId: "c_pi",
    documentId: "d_pi",
    documentTitle: "Preferences",
    documentKind: "personal_instructions",
    content: "Keep suggested answers under 30 seconds of speaking time.",
    score: 0.95,
    scope: "global",
  },
];

describe("document retrieval scopes", () => {
  it("queries session → mode → global with inferred kinds and renders provenance sections", async () => {
    const fake = new FakeTransport();
    fake.handle("context_build_snapshot", () => fixture.snapshot as ContextSnapshot);
    fake.handle("documents_retrieve", ({ query }) => {
      if (query.strategy === "leading") return query.kinds?.includes("personal_instructions") ? [chunks[2]!] : [];
      return query.kinds?.includes("job_description") ? [chunks[0]!, chunks[1]!] : [];
    });
    fake.handle("responses_save", ({ response }) => response);
    fake.handle("sessions_add_event", (args) => ({
      id: "evt_1",
      sessionId: args.sessionId,
      type: args.type,
      title: args.title,
      createdAt: "2026-09-07T09:10:00.000Z",
    }));
    fake.handle("ai_cancel", () => true);
    setTransport(fake);

    const engine = createResponseEngine();
    const handle = engine.ask({
      trigger: "typed",
      instruction: "How should I answer why I want this job at Acme?",
      captureScreen: false,
      mode: fixture.mode,
      session: makeSession({ id: "ses_scope", modeId: fixture.mode.id }),
      settings: makeSettings(),
    });
    const result = await handle.done;
    expect(result).not.toBeNull();

    const queries = fake.callsFor("documents_retrieve").map((call) => call.query);
    const personal = queries.find((q) => q.kinds?.includes("personal_instructions"));
    expect(personal?.strategy).toBe("leading");
    expect(personal?.scopes).toEqual([
      { scope: "session", scopeId: "ses_scope" },
      { scope: "mode", scopeId: "interview" },
      { scope: "global" },
    ]);
    const query = queries.find((q) => q.kinds?.includes("job_description"))!;
    expect(query.scopes).toEqual([{ scope: "session", scopeId: "ses_scope" }, { scope: "global" }]);
    expect(query.kinds).toEqual(
      expect.arrayContaining(["resume", "cv", "experience", "skills", "job_description", "role_description", "company_notes"]),
    );
    expect(query.strategy).toBe("keyword"); // embeddings disabled in settings
    expect(query.query).toContain("How should I answer why I want this job at Acme?");

    // Chunks land in the prompt under their provenance labels.
    const request = fake.callsFor("ai_stream")[0]!.request;
    const userPart = request.messages[1]?.content[0];
    const userText = userPart && "text" in userPart ? userPart.text : "";
    expect(userText).toContain('<context source="Your background (resume)"');
    expect(userText).toContain("payment platforms at FinCo");
    expect(userText).toContain('<context source="Job description"');
    expect(userText).toContain("senior platform engineer");
    // Personal instructions are the user's own words: the system prompt carries them (AI-004).
    const systemPart = request.messages[0]?.content[0];
    const systemText = systemPart && "text" in systemPart ? systemPart.text : "";
    expect(systemText).toContain("User preferences (from the user; they never override safety):");
    expect(systemText).toContain("under 30 seconds");
    // Personal instructions are lifted out of the generic document chunks.
    expect(userText).not.toContain('source="Reference documents"');
  });

  it("runs only the personal-instructions and small relevance-floored passes for modes without document requirements", async () => {
    const fake = new FakeTransport();
    const coding = loadFixture("coding");
    fake.handle("context_build_snapshot", () => coding.snapshot as ContextSnapshot);
    fake.handle("documents_retrieve", () => []);
    fake.handle("responses_save", ({ response }) => response);
    fake.handle("ai_cancel", () => true);
    setTransport(fake);

    const engine = createResponseEngine();
    const result = await engine.ask({
      trigger: "shortcut_capture",
      captureScreen: true,
      mode: coding.mode,
      settings: makeSettings(),
    }).done;

    expect(result).not.toBeNull();
    const queries = fake.callsFor("documents_retrieve").map((call) => call.query);
    expect(queries.find((q) => q.strategy === "leading")?.kinds).toEqual(["personal_instructions"]);
    for (const q of queries.filter((q) => q.strategy !== "leading")) {
      expect(q.kinds).toBeUndefined();
      expect(q.limit).toBeLessThanOrEqual(3);
    }
  });
});
