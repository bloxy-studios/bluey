import { GenerationGate } from "@/ai/generations";

describe("GenerationGate", () => {
  it("issues monotonic generations per scope", () => {
    const gate = new GenerationGate();
    expect(gate.next("ask")).toBe(1);
    expect(gate.next("ask")).toBe(2);
    expect(gate.next("prepare")).toBe(1); // independent scope
    expect(gate.current("ask")).toBe(2);
  });

  it("marks older generations stale once a newer one starts", () => {
    const gate = new GenerationGate();
    const g1 = gate.next("ask");
    expect(gate.isStale("ask", g1)).toBe(false);
    const g2 = gate.next("ask");
    expect(gate.isStale("ask", g1)).toBe(true);
    expect(gate.isStale("ask", g2)).toBe(false);
  });

  it("scopes do not interfere", () => {
    const gate = new GenerationGate();
    const ask = gate.next("ask");
    gate.next("prepare");
    gate.next("prepare");
    expect(gate.isStale("ask", ask)).toBe(false);
  });

  it("invalidate bumps a scope; invalidateAll bumps every known scope", () => {
    const gate = new GenerationGate();
    const a = gate.next("a");
    const b = gate.next("b");
    gate.invalidate("a");
    expect(gate.isStale("a", a)).toBe(true);
    expect(gate.isStale("b", b)).toBe(false);
    gate.invalidateAll();
    expect(gate.isStale("b", b)).toBe(true);
  });

  it("tracks in-flight request ids per scope", () => {
    const gate = new GenerationGate();
    gate.setInflight("ask", "req_1");
    expect(gate.takeInflight("ask")).toBe("req_1");
    expect(gate.takeInflight("ask")).toBeUndefined();

    gate.setInflight("ask", "req_2");
    gate.clearInflight("ask", "req_other"); // wrong id: no-op
    expect(gate.inflightIds()).toEqual(["req_2"]);
    gate.clearInflight("ask", "req_2");
    expect(gate.inflightIds()).toEqual([]);
  });
});
