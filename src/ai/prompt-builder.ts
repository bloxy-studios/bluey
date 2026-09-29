/**
 * PromptBuilder: composes the final `AIMessage[]` from separate parts —
 * system (identity + safety + response contract), mode (instructions +
 * schema fragment), style, context (rendered `ContextItem`s with provenance
 * labels), task + answer shape, and the output-format instruction. All
 * prompt strings live in `src/ai/prompts/` and `src/modes/prompts/`.
 */

import type { AskTrigger } from "@/lib/engine-contract";
import type {
  AIContentPart,
  AIMessage,
  AnswerShape,
  AnswerVoice,
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
  PREFERENCES_LABEL,
  QUESTION_LABEL,
  RESPONSE_CONTRACT,
  SECTION_LABELS,
  SECTION_ORDER,
  TRUSTED_SOURCES,
  answerShapeLine,
  contextBlock,
  identityBlock,
  newContextNonce,
  outputLanguageLine,
  structuredOutputBlock,
  styleBlock,
  taskLineFor,
  voiceLine,
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
  /** The detected answer shape; rendered as the `Shape:` line under `Task:`. */
  answerShape?: AnswerShape;
  /** Whose words the answer is; rendered as the one `Voice:` line under the shape. */
  voice?: AnswerVoice;
  /** JSON schema spec when structured output is requested. */
  outputSchema?: JsonSchemaSpec;
  /** Base64 screen image when the request needs vision. */
  visionImage?: VisionAttachment;
  /** Note appended when the budget dropped context. */
  omittedNote?: string;
  outputLanguage?: string;
  blueyName?: string;
  /** Fences the untrusted context blocks; drawn fresh per request unless a test pins it. */
  nonce?: string;
}

/**
 * One labelled section. Items that carry a time (`at`: transcript segments,
 * chat turns) read in the order they happened; untimed items (the earlier
 * summary) lead. Budget selection stays relevance-based.
 */
function renderSection(source: ContextSource, bucket: ContextItem[], nonce: string): string {
  const ordered = bucket.some((item) => item.at !== undefined)
    ? bucket.slice().sort((a, b) => {
        const ta = a.at ?? Number.NEGATIVE_INFINITY;
        const tb = b.at ?? Number.NEGATIVE_INFINITY;
        return ta === tb ? 0 : ta < tb ? -1 : 1;
      })
    : bucket;
  return contextBlock(SECTION_LABELS[source], nonce, ordered.map((item) => item.content).join("\n"));
}

/** The trusted items of one source, in order, as one text (empty when none). */
function trustedText(items: readonly ContextItem[], source: ContextSource): string {
  return items
    .filter((item) => item.source === source)
    .map((item) => item.content.trim())
    .filter((content) => content.length > 0)
    .join("\n");
}

export class PromptBuilder {
  private readonly nonce: string;

  constructor(private readonly parts: PromptBuilderParts) {
    this.nonce = parts.nonce ?? newContextNonce();
  }

  /** System message: identity + safety + response contract + mode + style + output format. */
  renderSystem(): string {
    const { mode, style, schemaId, outputSchema, outputLanguage, blueyName } = this.parts;
    const blocks: string[] = [identityBlock(blueyName), RESPONSE_CONTRACT];

    const modeInstructions = mode.systemInstructions.trim();
    const fragment = modePromptFor(schemaId).fragment;
    // A custom mode is the user's own text and outranks the contract (AI-011).
    const label = mode.builtIn ? mode.name : `${mode.name} (the user's custom instructions)`;
    blocks.push(`Mode: ${label}.${modeInstructions ? `\n${modeInstructions}` : ""}\n${fragment}`);

    // Standing personal instructions are the user's own words: trusted, and
    // placed after the mode so they refine it, never above safety (AI-004).
    const preferences = trustedText(this.parts.items, "personal_instructions");
    if (preferences) blocks.push(`${PREFERENCES_LABEL}\n${preferences}`);

    blocks.push(styleBlock(style));

    const language = outputLanguageLine(outputLanguage ?? "");
    if (language) blocks.push(language);

    blocks.push(outputSchema ? structuredOutputBlock(outputSchema) : PLAIN_OUTPUT_BLOCK);
    return blocks.join("\n\n");
  }

  /** Context sections grouped by provenance label, in fixed section order. */
  renderContext(): string {
    const { items, omittedNote } = this.parts;
    if (!items.some((item) => !TRUSTED_SOURCES.has(item.source)) && !omittedNote) return "";

    const grouped = new Map<ContextSource, ContextItem[]>();
    for (const item of items) {
      // The user's own words render outside the untrusted blocks (AI-004).
      if (TRUSTED_SOURCES.has(item.source)) continue;
      const bucket = grouped.get(item.source) ?? [];
      bucket.push(item);
      grouped.set(item.source, bucket);
    }

    const sections: string[] = [CONTEXT_PREAMBLE];
    for (const source of SECTION_ORDER) {
      const bucket = grouped.get(source);
      if (!bucket || bucket.length === 0) continue;
      grouped.delete(source);
      sections.push(renderSection(source, bucket, this.nonce));
    }
    // Any source not in the fixed order still gets rendered (future-proof).
    for (const [source, bucket] of grouped) {
      sections.push(renderSection(source, bucket, this.nonce));
    }
    if (omittedNote) sections.push(`(${omittedNote})`);
    return sections.join("\n\n");
  }

  /** Trigger-specific task instruction, then the answer-shape and voice lines when the intent set them. */
  renderTask(): string {
    const { trigger, items } = this.parts;
    // A follow-up with no earlier turn to build on is just a typed question.
    const followsNothing = trigger === "follow_up" && !items.some((item) => item.source === "conversation");
    const task = taskLineFor(followsNothing ? "typed" : trigger);
    const { answerShape, voice } = this.parts;
    const lines = [task];
    if (answerShape) lines.push(answerShapeLine(answerShape));
    if (voice) lines.push(voiceLine(voice));
    return lines.join("\n");
  }

  /** Full message array for the provider (+ inline image when vision). */
  buildMessages(): AIMessage[] {
    // The typed question is trusted: it follows the context, right before the task (AI-004).
    const question = trustedText(this.parts.items, "user_instruction");
    const userText = [
      this.renderContext(),
      question ? `${QUESTION_LABEL} ${question}` : "",
      this.renderTask(),
    ]
      .filter((part) => part.length > 0)
      .join("\n\n");

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
