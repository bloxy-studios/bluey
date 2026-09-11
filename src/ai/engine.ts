/**
 * Response engine: the intelligence-layer entry point implementing
 * `ResponseEngine` (src/lib/engine-contract.ts).
 *
 * ask():  capturing → analyzing (retrieval + fusion + budget + intent) →
 *         thinking (request build) → streaming (fence-safe drafts) → done
 *         (structured parse, optimize, persist, events).
 * prepare(): the same pipeline, silent, cached by detected-event id.
 * classify(): heuristics first, fast-model refinement in the 0.4–0.7 band.
 * summarizeSession(): mode-structured post-session summary.
 *
 * Stale protection: a monotonic GenerationGate per scope — a newer ask always
 * wins; stale streams are cancelled and never overwrite newer output.
 */

import * as z from "zod";
import type {
  AskInput,
  ClassifyInput,
  EngineCallbacks,
  EngineHandle,
  EnginePhase,
  ResponseEngine,
  SummarizeInput,
} from "@/lib/engine-contract";
import { bluey } from "@/lib/tauri/api";
import { eventBus } from "@/lib/tauri/event-bus";
import {
  toBlueyError,
  type AIChunk,
  type AIRequest,
  type BlueyResponse,
  type Citation,
  type ContextItem,
  type ContextSnapshot,
  type DeepResearchRequest,
  type DetectedEvent,
  type DetectedEventType,
  type LatencyTrace,
  type ResponseSection,
  type RetrievalQuery,
  type RetrievedChunk,
  type ScrapeResult,
  type SearchResult,
  type SessionEvent,
  type SessionEventType,
  type SessionSummary,
  type SnapshotOptions,
  type StructuredModelOutput,
  type TraceStamps,
} from "@/lib/types";
import { allocateBudget, defaultContextBudget } from "@/context/budget";
import { estimateTokens, fuseContext } from "@/context/fusion";
import { classifyIntent, type Intent } from "@/context/relevance";
import { retrieveRelevantContext } from "@/context/retrieval";
import { buildNativeSnapshot, enrichSnapshot } from "@/context/snapshot";
import { outputSchemaFor, parseJsonLoose, parseStructuredOutput } from "@/modes/schemas";
import { effectiveStyle } from "@/modes/registry";
import { classifySegment } from "@/transcript/classifier";
import { assertCloudAiAllowed, cloudAiAllowed } from "./cloud-gate";
import { GenerationGate } from "./generations";
import { assembleMetrics, emitDevMetrics } from "./metrics";
import { PromptBuilder, type VisionAttachment } from "./prompt-builder";
import { afterNextPaint, perfNow } from "./trace";
import { CLASSIFICATION_SYSTEM, classificationUser } from "./prompts";
import { buildAIRequest, maxOutputTokensFor } from "./request";
import {
  decideResearch,
  buildPublicQuery,
  runResearch,
  OPTIMISTIC_AVAILABILITY,
  type ResearchOutcome,
} from "./research";
import { generateSessionSummary } from "@/sessions/summary";
import { optimizeResponse } from "./optimizer";
import {
  CodeFenceBuffer,
  extractPartialStringField,
  streamRequest,
  visibleWithHeldFences,
  type StreamHandle,
  type StreamOutcome,
} from "./stream";

// ── Injectable dependencies ─────────────────────────────────────────────────

export interface EngineApi {
  context: { buildSnapshot(args: { options: SnapshotOptions }): Promise<ContextSnapshot> };
  documents: { retrieve(args: { query: RetrievalQuery }): Promise<RetrievedChunk[]> };
  ai: {
    stream(request: AIRequest, onChunk: (chunk: AIChunk) => void): Promise<void>;
    cancel(args: { requestId: string }): Promise<boolean>;
    cancelAll(): Promise<number>;
    /** The fast-path trace's late stamps (first paint, done) — optional, best-effort (ADR 0010 §2). */
    reportTrace?(args: { requestId: string; stamps: TraceStamps }): Promise<LatencyTrace | null>;
  };
  responses: { save(args: { response: BlueyResponse }): Promise<BlueyResponse> };
  session: {
    addEvent(args: {
      sessionId: string;
      type: SessionEventType;
      title: string;
      detail?: string;
      refs?: Record<string, string>;
      confidence?: number;
    }): Promise<SessionEvent>;
    saveSummary(args: { summary: Omit<SessionSummary, "id" | "createdAt"> }): Promise<SessionSummary>;
  };
  research: {
    search(args: { query: string; numResults?: number }): Promise<SearchResult[]>;
    scrape(args: { url: string }): Promise<ScrapeResult>;
    deepStart(args: { request: DeepResearchRequest }): Promise<void>;
    deepCancel(args: { jobId: string }): Promise<boolean>;
    available(): Promise<{ search: boolean; scrape: boolean; deepAgent: boolean }>;
  };
}

export type EngineBus = Pick<typeof eventBus, "emit" | "on">;

export interface EngineDeps {
  api?: EngineApi;
  bus?: EngineBus;
  now?: () => Date;
  idGen?: () => string;
  /** Deep-research wall clock cap in ms (default 90s). */
  researchTimeoutMs?: number;
}

const SCOPE_ASK = "ask";
const SCOPE_PREPARE = "prepare";

export const PREPARED_CACHE_MAX = 5;
export const PREPARED_TTL_MS = 3 * 60 * 1000;

interface PreparedEntry {
  response: BlueyResponse;
  at: number;
}

class CancelledError extends Error {
  constructor() {
    super("cancelled");
    this.name = "CancelledError";
  }
}

const classifyRefinement = z.object({
  type: z.string(),
  requiresResponse: z.boolean().optional(),
  confidence: z.number().min(0).max(1).optional(),
});

const DETECTED_TYPES: readonly DetectedEventType[] = [
  "question",
  "behavioral_question",
  "technical_question",
  "coding_problem",
  "objection",
  "buying_signal",
  "pricing_concern",
  "competitor_mention",
  "decision",
  "action_item",
  "topic_change",
  "important_statement",
  "follow_up",
];

// ── Pipeline result plumbing ────────────────────────────────────────────────

interface PipelineOptions {
  requestId: string;
  generation: number;
  scope: string;
  silent: boolean;
  callbacks: EngineCallbacks;
  isCancelled(): boolean;
  onStreamHandle(handle: StreamHandle): void;
}

export function createResponseEngine(deps: EngineDeps = {}): ResponseEngine {
  const api: EngineApi = deps.api ?? bluey;
  const bus: EngineBus = deps.bus ?? eventBus;
  const now = deps.now ?? (() => new Date());
  const idGen = deps.idGen ?? (() => crypto.randomUUID());

  const gate = new GenerationGate();
  const prepared = new Map<string, PreparedEntry>();

  function isStale(scope: string, generation: number): boolean {
    return gate.isStale(scope, generation);
  }

  function checkAlive(opts: PipelineOptions): void {
    if (opts.isCancelled() || isStale(opts.scope, opts.generation)) throw new CancelledError();
  }

  function phase(opts: PipelineOptions, value: EnginePhase): void {
    if (!opts.silent) opts.callbacks.onPhase?.(value, opts.requestId);
  }

  // ── Research (best-effort, never fails the ask) ───────────────────────────

  async function maybeResearch(input: AskInput, snapshot: ContextSnapshot): Promise<ResearchOutcome | null> {
    const instruction = input.instruction?.trim();
    if (!instruction || !input.settings.ai.researchEnabled) return null;
    const policyDepth = decideResearch({
      instruction,
      mode: input.mode,
      settings: input.settings,
      availability: OPTIMISTIC_AVAILABILITY,
      now,
    });
    if (policyDepth === "none") return null;
    const availability = await api.research
      .available()
      .catch(() => ({ search: false, scrape: false, deepAgent: false }));
    const depth = decideResearch({
      instruction,
      mode: input.mode,
      settings: input.settings,
      availability,
      now,
    });
    if (depth === "none") return null;
    const query = buildPublicQuery(instruction, snapshot);
    if (query.length === 0) return null;
    return runResearch(depth, query, {
      jobId: `res_${idGen()}`,
      api,
      bus,
      timeoutMs: deps.researchTimeoutMs,
    });
  }

  // ── Final response assembly ───────────────────────────────────────────────

  function toSections(parsed: StructuredModelOutput): ResponseSection[] | undefined {
    if (!parsed.sections || parsed.sections.length === 0) return undefined;
    return parsed.sections.map((section, index) => ({ id: `sec_${index + 1}`, ...section }));
  }

  function mergeCitations(
    parsed: StructuredModelOutput,
    research: ResearchOutcome | null,
  ): Citation[] | undefined {
    const merged: Citation[] = [];
    const seen = new Set<string>();
    for (const citation of research?.citations ?? []) {
      if (seen.has(citation.url)) continue;
      seen.add(citation.url);
      merged.push(citation);
    }
    (parsed.citations ?? []).forEach((citation, index) => {
      if (seen.has(citation.url)) return;
      seen.add(citation.url);
      merged.push({ id: `cit_${index + 1}`, ...citation });
    });
    return merged.length > 0 ? merged : undefined;
  }

  // ── The core pipeline ─────────────────────────────────────────────────────

  async function runPipeline(input: AskInput, opts: PipelineOptions): Promise<BlueyResponse> {
    // Privacy master switch: nothing is captured or sent while Cloud AI is off.
    assertCloudAiAllowed(input.settings);
    const startedAt = now().getTime();

    // The fast-path trace (ADR 0010 §2): WebView stamps are offsets from the
    // moment the native snapshot reply arrived; Rust merges them by request id.
    const askStartedTs = perfNow();
    let anchorTs = askStartedTs;
    let anchorRust: number | undefined;
    let ipcMs: number | undefined;
    let snapshotReadyMs: number | undefined;
    let captureDoneMs: number | undefined;
    let imageBytes: number | undefined;
    let imagePx: number | undefined;

    // Phase: capturing ──────────────────────────────────────────────────────
    if (input.captureScreen && !input.snapshot) phase(opts, "capturing");
    let snapshot: ContextSnapshot;
    if (input.snapshot) {
      snapshot = input.snapshot;
    } else {
      const invokeTs = perfNow();
      snapshot = await buildNativeSnapshot({
        mode: input.mode,
        settings: input.settings,
        trigger: input.trigger,
        captureScreen: input.captureScreen,
        transcriptWindowSeconds: input.transcriptWindowSeconds,
        api,
      });
      const replyTs = perfNow();
      anchorTs = replyTs;
      snapshotReadyMs = 0;
      const nativeTrace = snapshot.trace;
      if (nativeTrace) {
        anchorRust = nativeTrace.replyMs;
        const nativeMs = nativeTrace.replyMs - nativeTrace.startedMs;
        ipcMs = Math.max(0, replyTs - invokeTs - nativeMs);
        captureDoneMs = nativeTrace.captureDoneMs;
        imageBytes = nativeTrace.imageBytes;
        imagePx = nativeTrace.imagePx;
      }
    }
    checkAlive(opts);

    // Phase: analyzing ──────────────────────────────────────────────────────
    phase(opts, "analyzing");
    const retrieved = await retrieveRelevantContext({
      instruction: input.instruction,
      snapshot,
      mode: input.mode,
      session: input.session,
      settings: input.settings,
      api,
    });
    const retrievalDoneMs = perfNow() - anchorTs;
    checkAlive(opts);

    const research = await maybeResearch(input, snapshot);
    checkAlive(opts);

    snapshot = enrichSnapshot(snapshot, {
      mode: input.mode,
      settings: input.settings,
      session: input.session,
      instruction: input.instruction,
      previousResponses: input.previousResponses,
      retrieved,
      sessionEvents: input.sessionEvents,
      sessionNotes: input.sessionNotes,
      sessionDocumentIds: input.sessionDocumentIds,
    });

    const items: ContextItem[] = fuseContext(snapshot, {
      instruction: input.instruction,
      detectedEvent: input.detectedEvent,
    });
    if (research) {
      items.push({
        source: "document",
        content: research.contextText,
        relevance: 0.8,
        tokens: estimateTokens(research.contextText),
        ref: "research",
      });
    }

    const intent: Intent = classifyIntent({
      instruction: input.instruction,
      snapshot,
      mode: input.mode,
      detectedEvent: input.detectedEvent,
      now,
    });

    const style = effectiveStyle(input.mode, input.settings);
    const headroom = maxOutputTokensFor(style.length, intent.task);
    const budget = allocateBudget(items, defaultContextBudget(input.settings, headroom));
    const contextAssemblyMs = now().getTime() - startedAt;
    checkAlive(opts);

    // Phase: thinking ───────────────────────────────────────────────────────
    phase(opts, "thinking");
    const outputSchema = outputSchemaFor(intent.schemaId);
    let visionImage: VisionAttachment | undefined;
    if (intent.visionRequired && snapshot.screen?.image) {
      visionImage = {
        mediaType: (snapshot.screen.mimeType ?? "image/jpeg") as VisionAttachment["mediaType"],
        data: snapshot.screen.image,
      };
    }

    const builder = new PromptBuilder({
      mode: input.mode,
      style,
      schemaId: intent.schemaId,
      trigger: input.trigger,
      items: budget.included,
      instruction: input.instruction,
      detectedEvent: input.detectedEvent,
      outputSchema,
      visionImage,
      omittedNote: budget.omittedNote,
      outputLanguage: input.settings.general.outputLanguage,
      blueyName: input.settings.general.blueyName,
    });

    const request = buildAIRequest({
      requestId: opts.requestId,
      generation: opts.generation,
      intent,
      messages: builder.buildMessages(),
      contextTokens: budget.totalTokens,
      responseLength: style.length,
      session: input.session,
      outputSchema,
      now,
    });
    const promptBuiltMs = perfNow() - anchorTs;
    let firstPaintMs: number | undefined;
    let firstPaintScheduled = false;

    // Phase: streaming ──────────────────────────────────────────────────────
    const responseId = `resp_${idGen()}`;
    const prompt = input.instruction ?? input.detectedEvent?.text;
    const baseResponse: BlueyResponse = {
      id: responseId,
      requestId: opts.requestId,
      sessionId: input.session?.id,
      modeId: input.mode.id,
      type: intent.responseType,
      content: "",
      prompt,
      createdAt: now().toISOString(),
    };

    const fenceBuffer = new CodeFenceBuffer();
    let lastDraftLength = -1;
    let streamingAnnounced = false;

    const streamInvokedMs = perfNow() - anchorTs;
    request.trace = {
      trigger: opts.silent ? "prepare" : input.trigger,
      anchorMs: anchorRust,
      ipcMs,
      shortcutMs: input.triggeredAtMs,
      captureDoneMs,
      imageBytes,
      imagePx,
      askStartedMs: askStartedTs - anchorTs,
      snapshotReadyMs,
      retrievalDoneMs,
      promptBuiltMs,
      streamInvokedMs,
    };

    const handle = streamRequest(
      request,
      {
        onDelta: (delta, accumulated) => {
          if (opts.silent || opts.isCancelled() || isStale(opts.scope, opts.generation)) return;
          if (!streamingAnnounced) {
            streamingAnnounced = true;
            phase(opts, "streaming");
          }
          fenceBuffer.push(delta);
          const draftContent = request.outputSchema
            ? visibleWithHeldFences(extractPartialStringField(accumulated, "content") ?? "")
            : fenceBuffer.visible();
          if (draftContent.length > lastDraftLength) {
            lastDraftLength = draftContent.length;
            opts.callbacks.onDraft?.({ ...baseResponse, content: draftContent });
            if (!firstPaintScheduled) {
              firstPaintScheduled = true;
              afterNextPaint(() => {
                if (firstPaintMs === undefined) firstPaintMs = perfNow() - anchorTs;
              });
            }
          }
        },
      },
      api,
      () => now().getTime(),
    );
    opts.onStreamHandle(handle);

    const outcome: StreamOutcome = await handle.done;
    const doneMs = perfNow() - anchorTs;
    if (api.ai.reportTrace) {
      // Best-effort: the late half of the trace; never delays the answer.
      void api.ai
        .reportTrace({
          requestId: opts.requestId,
          stamps: { anchorMs: anchorRust, ipcMs, streamInvokedMs, firstPaintMs, doneMs },
        })
        .catch(() => null);
    }
    checkAlive(opts);
    if (outcome.finishReason === "cancelled") throw new CancelledError();
    if (outcome.finishReason === "error") {
      throw (
        outcome.error ?? {
          kind: "ai" as const,
          code: "ai.stream_failed",
          message: "The model stream failed",
          recoverable: true,
        }
      );
    }

    // Phase: done ───────────────────────────────────────────────────────────
    phase(opts, "done");
    const parsed =
      parseStructuredOutput(intent.schemaId, outcome.text) ??
      ({ responseType: intent.responseType, content: "" } as StructuredModelOutput);
    const parserFellBack =
      parsed.responseType === "answer" &&
      parsed.sections === undefined &&
      parsed.content === outcome.text.trim();

    const metrics = assembleMetrics({
      snapshot,
      contextAssemblyMs,
      contextTokens: budget.totalTokens,
      outcome,
    });

    let response: BlueyResponse = {
      ...baseResponse,
      type: parserFellBack ? intent.responseType : parsed.responseType,
      title: parsed.title,
      content: parsed.content,
      sections: toSections(parsed),
      code: parsed.code,
      diagram: parsed.diagram,
      confidence: parsed.confidence,
      citations: mergeCitations(parsed, research),
      metrics,
      createdAt: now().toISOString(),
    };
    response = optimizeResponse(response, { style, mode: input.mode });
    if (opts.silent) response.prepared = true;

    // Persist + events (best-effort; the response is already usable).
    if (!opts.silent) {
      try {
        await api.responses.save({ response });
      } catch {
        // Storage failures must not lose the answer.
      }
      if (input.session) {
        try {
          await api.session.addEvent({
            sessionId: input.session.id,
            type: "response_generated",
            title: "Response generated",
            detail: response.title,
            refs: { responseId: response.id, requestId: opts.requestId },
          });
        } catch {
          // Best-effort.
        }
      }
      bus.emit("context.updated", { snapshot, reason: "response_generated" });
      emitDevMetrics(metrics, bus, now);
    }

    return response;
  }

  // ── ask ───────────────────────────────────────────────────────────────────

  function ask(input: AskInput, callbacks: EngineCallbacks = {}): EngineHandle {
    const requestId = `req_${idGen()}`;
    const generation = gate.next(SCOPE_ASK);

    // Supersede: cancel whatever was in flight for this window.
    const previous = gate.takeInflight(SCOPE_ASK);
    if (previous) void api.ai.cancel({ requestId: previous }).catch(() => false);
    gate.setInflight(SCOPE_ASK, requestId);

    let cancelled = false;
    let streamHandle: StreamHandle | null = null;

    const opts: PipelineOptions = {
      requestId,
      generation,
      scope: SCOPE_ASK,
      silent: false,
      callbacks,
      isCancelled: () => cancelled,
      onStreamHandle: (handle) => {
        streamHandle = handle;
      },
    };

    const done: Promise<BlueyResponse | null> = (async () => {
      try {
        const response = await runPipeline(input, opts);
        callbacks.onComplete?.(response);
        return response;
      } catch (error) {
        if (error instanceof CancelledError) {
          phase(opts, "cancelled");
          return null;
        }
        const blueyError = toBlueyError(error, "ai");
        callbacks.onError?.(blueyError, requestId);
        phase(opts, "error");
        return null;
      } finally {
        gate.clearInflight(SCOPE_ASK, requestId);
      }
    })();

    return {
      requestId,
      generation,
      cancel: async () => {
        cancelled = true;
        if (streamHandle) await streamHandle.cancel();
        else await api.ai.cancel({ requestId }).catch(() => false);
      },
      done,
    };
  }

  // ── prepare / takePrepared ────────────────────────────────────────────────

  function purgeExpired(): void {
    const cutoff = now().getTime() - PREPARED_TTL_MS;
    for (const [key, entry] of prepared) {
      if (entry.at < cutoff) prepared.delete(key);
    }
  }

  async function prepare(input: AskInput): Promise<BlueyResponse | null> {
    if (!input.settings.ai.proactivePreparation) return null;
    purgeExpired();
    const key = input.detectedEvent?.id ?? "generic";
    const cached = prepared.get(key);
    if (cached) return cached.response;

    const requestId = `req_${idGen()}`;
    const generation = gate.next(SCOPE_PREPARE);
    const opts: PipelineOptions = {
      requestId,
      generation,
      scope: SCOPE_PREPARE,
      silent: true,
      callbacks: {},
      isCancelled: () => false,
      onStreamHandle: () => {},
    };
    try {
      const response = await runPipeline(input, opts);
      purgeExpired();
      prepared.set(key, { response, at: now().getTime() });
      while (prepared.size > PREPARED_CACHE_MAX) {
        let oldestKey: string | undefined;
        let oldestAt = Number.POSITIVE_INFINITY;
        for (const [entryKey, entry] of prepared) {
          if (entry.at < oldestAt) {
            oldestAt = entry.at;
            oldestKey = entryKey;
          }
        }
        if (oldestKey === undefined) break;
        prepared.delete(oldestKey);
      }
      bus.emit("response.prepared", response);
      return response;
    } catch {
      return null;
    }
  }

  function takePrepared(eventId?: string): BlueyResponse | null {
    purgeExpired();
    if (eventId) {
      const entry = prepared.get(eventId);
      if (!entry) return null;
      prepared.delete(eventId);
      return entry.response;
    }
    let newestKey: string | undefined;
    let newestAt = -1;
    for (const [key, entry] of prepared) {
      if (entry.at > newestAt) {
        newestAt = entry.at;
        newestKey = key;
      }
    }
    if (newestKey === undefined) return null;
    const entry = prepared.get(newestKey);
    prepared.delete(newestKey);
    return entry?.response ?? null;
  }

  // ── classify ──────────────────────────────────────────────────────────────

  async function refineWithFastModel(
    input: ClassifyInput,
    event: DetectedEvent,
  ): Promise<DetectedEvent | null> {
    const request: AIRequest = {
      requestId: `req_${idGen()}`,
      generation: 1,
      task: "classification",
      latencyBudget: "ultra-fast",
      reasoning: "none",
      visionRequired: false,
      contextTokens: estimateTokens(input.segment.text),
      messages: [
        { role: "system", content: [{ type: "text", text: CLASSIFICATION_SYSTEM }] },
        {
          role: "user",
          content: [
            {
              type: "text",
              text: classificationUser(input.segment.text, event.speaker ?? "Speaker", input.mode.name),
            },
          ],
        },
      ],
      outputSchema: {
        name: "bluey_classification",
        schema: z.toJSONSchema(classifyRefinement) as Record<string, unknown>,
        strict: true,
      },
      maxOutputTokens: 100,
      temperature: 0,
      createdAt: now().toISOString(),
    };

    const outcome = await streamRequest(request, {}, api, () => now().getTime()).done;
    if (outcome.finishReason !== "stop") return event;
    const json = parseJsonLoose(outcome.text);
    const refined = classifyRefinement.safeParse(json);
    if (!refined.success) return event;

    if (refined.data.type === "none") {
      return (refined.data.confidence ?? 0.7) > 0.6 ? null : event;
    }
    const type = DETECTED_TYPES.includes(refined.data.type as DetectedEventType)
      ? (refined.data.type as DetectedEventType)
      : event.type;
    return {
      ...event,
      type,
      confidence: refined.data.confidence ?? event.confidence,
      requiresResponse: (refined.data.requiresResponse ?? event.requiresResponse) && event.speaker !== "You",
    };
  }

  async function classify(input: ClassifyInput): Promise<DetectedEvent | null> {
    const heuristic = classifySegment({
      segment: input.segment,
      recent: input.recent,
      mode: input.mode,
      now,
      idGen,
    });
    if (!heuristic) return null;

    let event: DetectedEvent | null = heuristic;
    const fastModelConfigured = input.settings.ai.models.fast != null && cloudAiAllowed(input.settings);
    if (fastModelConfigured && heuristic.confidence >= 0.4 && heuristic.confidence <= 0.7) {
      try {
        event = await refineWithFastModel(input, heuristic);
      } catch {
        event = heuristic; // model assist is strictly optional
      }
    }
    if (event?.requiresResponse) bus.emit("question.detected", event);
    return event;
  }

  // ── summarizeSession ──────────────────────────────────────────────────────

  async function summarizeSession(input: SummarizeInput): Promise<SessionSummary> {
    assertCloudAiAllowed(input.settings);
    const summary = await generateSessionSummary(input, { api, now, idGen });
    try {
      const { id: _id, createdAt: _createdAt, ...rest } = summary;
      return await api.session.saveSummary({ summary: rest });
    } catch {
      return summary; // persistence failure must not lose the summary
    }
  }

  // ── cancelAll ─────────────────────────────────────────────────────────────

  async function cancelAll(): Promise<void> {
    gate.invalidateAll();
    gate.next(SCOPE_ASK);
    gate.next(SCOPE_PREPARE);
    try {
      await api.ai.cancelAll();
    } catch {
      // Best-effort.
    }
  }

  return { ask, prepare, takePrepared, classify, summarizeSession, cancelAll };
}
