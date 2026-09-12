import { MODE_PROMPTS } from "@/modes/prompts";
import { SECTION_TITLES } from "@/modes/schemas";
import type { ResponseSchemaId } from "@/lib/types";

/** Every double-quoted string in a fragment is a section title (module docs). */
const QUOTED = /"([^"]+)"/g;

describe("mode prompt fragments", () => {
  it("only name section titles the provider schema's closed enum accepts", () => {
    for (const [schemaId, prompt] of Object.entries(MODE_PROMPTS) as Array<
      [ResponseSchemaId, { fragment: string }]
    >) {
      const titles = SECTION_TITLES[schemaId];
      const quoted = [...prompt.fragment.matchAll(QUOTED)].map((match) => match[1]);
      if (!titles) {
        expect(quoted, `${schemaId} has no section enum and must quote no titles`).toEqual([]);
        continue;
      }
      for (const title of quoted) {
        expect(titles, `${schemaId}: "${title}" is not in SECTION_TITLES`).toContain(title);
      }
    }
  });

  it("describe fields, not voice — voice belongs to the response contract", () => {
    for (const [schemaId, prompt] of Object.entries(MODE_PROMPTS)) {
      expect(prompt.fragment.startsWith("Fields:"), schemaId).toBe(true);
      expect(prompt.fragment, schemaId).not.toMatch(/\b(be concise|be brief|tone|you should|the user should)\b/i);
    }
  });

  it("keeps the STAR guidance for behavioral answers unlabelled", () => {
    expect(MODE_PROMPTS.behavioral.fragment).toContain("never label the STAR parts out loud");
  });
});
