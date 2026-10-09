import { beforeEach, describe, expect, it } from "vitest";

import { useTranscriptStore } from "@/stores/transcriptStore";

import { makeSegment } from "./helpers";

describe("transcriptStore partials (UX-010)", () => {
  beforeEach(() => useTranscriptStore.getState().clear());

  it("keeps the microphone and system partials side by side", () => {
    const store = useTranscriptStore.getState();
    store.applyPartial(makeSegment({ id: "mic-1", source: "microphone", text: "I think", finalized: false }));
    store.applyPartial(makeSegment({ id: "sys-1", source: "system", text: "So tell", finalized: false }));

    const { partials } = useTranscriptStore.getState();
    expect(partials.microphone?.text).toBe("I think");
    expect(partials.system?.text).toBe("So tell");
  });

  it("clears only the finalized source's partial, even when an older helper keyed it differently", () => {
    const store = useTranscriptStore.getState();
    store.applyPartial(makeSegment({ id: "mic-1", source: "microphone", text: "I think", finalized: false }));
    store.applyPartial(makeSegment({ id: "sys-1", source: "system", text: "So tell", finalized: false }));
    store.applyFinal(makeSegment({ id: "mic-2", source: "microphone", text: "I think so." }));

    const { partials, segments } = useTranscriptStore.getState();
    expect(partials.microphone).toBeUndefined();
    expect(partials.system?.id).toBe("sys-1");
    expect(segments.map((s) => s.text)).toEqual(["I think so."]);
  });
});
