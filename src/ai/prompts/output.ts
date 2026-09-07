/** Output-format instructions (structured JSON schema guidance). */

import type { JsonSchemaSpec } from "@/lib/types";

export function structuredOutputBlock(spec: JsonSchemaSpec): string {
  return [
    `Output format: respond with a single JSON object matching the "${spec.name}" schema you were given.`,
    "Put the main markdown answer in `content`. Use `sections` for the named parts the mode asks for.",
    "Escape newlines correctly inside JSON strings. No text before or after the JSON object.",
  ].join(" ");
}

export const PLAIN_OUTPUT_BLOCK =
  "Output format: well-formed markdown. Use fenced code blocks with a language tag for any code.";
