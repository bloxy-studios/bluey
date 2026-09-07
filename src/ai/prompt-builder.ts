/**
 * PromptBuilder: composes the final `AIMessage[]` from separate parts —
 * system (identity + safety), mode (instructions + schema fragment), style,
 * context (rendered `ContextItem`s with provenance labels), task, and the
 * output-format instruction. All prompt strings live in `src/ai/prompts/`.
 */

import type { AskTrigger } from "@/lib/engine-contract";
import type {
  AIContentPart,
  AIMessage,
  BlueyMode,
  ContextItem,
  ContextSource,
  DetectedEvent,
  JsonSchemaSpec,
  ResponseSchemaId,
  ResponseStyle,
} from "@/lib/types";
import { modePromptFor } from "@/modes/prompts";
import {
  CONTEXT_PREAMBLE,
  PLAIN_OUTPUT_BLOCK,
  SECTION_LABELS,
  SECTION_ORDER,
  identityBlock,
  outputLanguageLine,
  structuredOutputBlock,
  styleBlock,
  taskLineFor,
} from "./prompts";

export interface VisionAttachment {
  mediaType: "image/jpeg" | "image/png" | "image/webp";
  data: string;
}

export interface PromptBuilderParts {
  mode: BlueyMode;
  style: ResponseStyle;
  schemaId: ResponseSchemaId;
  trigger: AskTrigger;
  items: ContextItem[];
  instruction?: string;
  detectedEvent?: DetectedEvent;
  /** JSON schema spec when structured output is requested. */
  outputSchema?: JsonSchemaSpec;
  /** Base64 screen image when the request needs vision. */
  visionImage?: VisionAttachment;
  /** Note appended when the budget dropped context. */
  omittedNote?: string;
  outputLanguage?: string;
  blueyName?: string;
}

export class PromptBuilder {
  constructor(private readonly parts: PromptBuilderParts) {}

  /** System message: identity + safety + mode + style + output format. */
  renderSystem(): string {
    const { mode, style, schemaId, outputSchema, outputLanguage, blueyName } = this.parts;
    const blocks: string[] = [identityBlock(blueyName)];

    const modeInstructions = mode.systemInstructions.trim();
    const fragment = modePromptFor(schemaId).fragment;
    blocks.push(`Mode: ${mode.name}.${modeInstructions ? `\n${modeInstructions}` : ""}\n${fragment}`);

    blocks.push(styleBlock(style));

    const language = outputLanguageLine(outputLanguage ?? "");
    if (language) blocks.push(language);

    blocks.push(outputSchema ? structuredOutputBlock(outputSchema) : PLAIN_OUTPUT_BLOCK);
    return blocks.join("\n\n");
  }

  /** Context sections grouped by provenance label, in fixed section order. */
  renderContext(): string {
    const { items, omittedNote } = this.parts;
    if (items.length === 0 && !omittedNote) return "";

    const grouped = new Map<ContextSource, string[]>();
    for (const item of items) {
      const bucket = grouped.get(item.source) ?? [];
      bucket.push(item.content);
      grouped.set(item.source, bucket);
    }

    const sections: string[] = [CONTEXT_PREAMBLE];
    for (const source of SECTION_ORDER) {
      const contents = grouped.get(source);
      if (!contents || contents.length === 0) continue;
      grouped.delete(source);
      sections.push(`### ${SECTION_LABELS[source]}\n${contents.join("\n")}`);
    }
    // Any source not in the fixed order still gets rendered (future-proof).
    for (const [source, contents] of grouped) {
      sections.push(`### ${SECTION_LABELS[source]}\n${contents.join("\n")}`);
    }
    if (omittedNote) sections.push(`(${omittedNote})`);
    return sections.join("\n\n");
  }

  /** Trigger-specific task instruction. */
  renderTask(): string {
    return taskLineFor(this.parts.trigger);
  }

  /** Full message array for the provider (+ inline image when vision). */
  buildMessages(): AIMessage[] {
    const context = this.renderContext();
    const userText = [context, this.renderTask()].filter((part) => part.length > 0).join("\n\n");

    const userContent: AIContentPart[] = [{ type: "text", text: userText }];
    if (this.parts.visionImage) {
      userContent.push({
        type: "image",
        mediaType: this.parts.visionImage.mediaType,
        data: this.parts.visionImage.data,
      });
    }

    return [
      { role: "system", content: [{ type: "text", text: this.renderSystem() }] },
      { role: "user", content: userContent },
    ];
  }
}
