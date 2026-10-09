import { describe, expect, it } from "vitest";

import { CONTEXT_SOURCE_CHIPS, toggleContextChip } from "@/features/settings/mode-options";

const chip = (source: string) => {
  const found = CONTEXT_SOURCE_CHIPS.find((c) => c.source === source);
  if (!found) throw new Error(`no ${source} chip`);
  return found;
};

describe("toggleContextChip (MODE-011)", () => {
  it("turns the accessibility tree on and off with Screen", () => {
    expect(toggleContextChip(["transcript"], chip("screen"))).toEqual(["transcript", "screen", "accessibility"]);
    expect(toggleContextChip(["screen", "accessibility", "transcript"], chip("screen"))).toEqual(["transcript"]);
  });

  it("keeps requirements that have no chip, such as session memory", () => {
    expect(toggleContextChip(["session_memory", "documents"], chip("documents"))).toEqual(["session_memory"]);
    expect(toggleContextChip(["session_memory"], chip("resume"))).toEqual(["session_memory", "resume"]);
  });
});
