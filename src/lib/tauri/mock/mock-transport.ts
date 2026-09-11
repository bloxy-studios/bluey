/**
 * MockTransport — the Developer Mode backend. A complete in-memory
 * implementation of every command in `CommandMap`, used for browser
 * development (`bun run dev` without Tauri) and for tests.
 *
 * Strictly isolated under `src/lib/tauri/mock/`: production components never
 * import from here; bootstrap selects it only when the Tauri runtime is absent.
 */

import type {
  AIChunk,
  AIProviderConfig,
  AIRequest,
  AppStatus,
  AudioStatus,
  AuthStatus,
  AuthUser,
  BlueyDocument,
  BlueyError,
  BlueyMode,
  BlueyResponse,
  CaptureProtection,
  ContextSnapshot,
  DetectedEvent,
  DevSimulation,
  LatencyMetrics,
  PermissionState,
  ScreenFrame,
  Session,
  SessionEvent,
  SessionListItem,
  SessionNote,
  SessionSummary,
  Settings,
  SettingsPatch,
  ShortcutConflict,
  TranscribeFileResult,
  TranscriptSegment,
  PanelState,
  AccountStatus,
  ConnectFlowKind,
  ProviderAccount,
  ProviderModelCatalog,
} from "../../types";
import { applyPresets, MODEL_ROLES, presetForKind } from "../../ai/provider-presets";
import { createId } from "../../utils/id";
import type { CommandArgs, CommandName, CommandResult } from "../commands";
import type { EventName, EventPayload } from "../events";
import type { StreamChannel, Transport, Unlisten } from "../transport";
import {
  CANNED_ANSWER_MARKDOWN,
  CANNED_FOLLOW_UP_MARKDOWN,
  CODING_PROBLEM_OCR,
  createBuiltInModes,
  createDefaultSettings,
  createSeedData,
  DEFAULT_SHORTCUTS,
  FIXTURE_AUDIO_DEVICES,
  FIXTURE_DISPLAYS,
  FIXTURE_MODELS_BY_KIND,
  FIXTURE_PNG_BASE64,
  createFixtureCatalog,
  createMockAccounts,
  FIXTURE_ACCOUNT_IDENTITIES,
} from "./fixtures";

const now = () => new Date().toISOString();

/** The identity the simulated browser sign-in returns. */
const MOCK_AUTH_USER: AuthUser = {
  id: "user_mock_jordan",
  email: "jordan@example.com",
  firstName: "Jordan",
  lastName: "Lee",
  imageUrl: undefined,
};

/** What the native picker "returns" in the mock. */
const MOCK_RECORDING_PATH = "/Users/jordan/Recordings/standup.wav";
/** Canned two-speaker recording for `ai_transcribe_file`. */
const MOCK_RECORDING_LINES: ReadonlyArray<readonly [string, string]> = [
  ["spk_1", "Thanks for joining, let's get started."],
  ["spk_2", "Could you walk me through your recent project?"],
  ["spk_1", "We rebuilt the ingestion pipeline around a queue so retries were free."],
  ["spk_2", "What trade-offs did you consider?"],
];

function blueyError(
  partial: Partial<BlueyError> & Pick<BlueyError, "kind" | "code" | "message">,
): BlueyError {
  return { recoverable: false, ...partial };
}

class MockStreamChannel<T> implements StreamChannel<T> {
  private handler: ((message: T) => void) | null = null;
  private buffer: T[] = [];

  get raw(): unknown {
    return this;
  }

  onMessage(handler: (message: T) => void): void {
    this.handler = handler;
    for (const message of this.buffer.splice(0)) handler(message);
  }

  push(message: T): void {
    if (this.handler) this.handler(message);
    else this.buffer.push(message);
  }
}

export interface MockTransportOptions {
  /** Delay between streamed words (default 30ms; 0 = synchronous-ish for tests). */
  streamDelayMs?: number;
  /** Emit `audio.level` ticks while an audio session runs (default true). */
  levelTicks?: boolean;
}

type Handlers = {
  [K in CommandName]: (args: CommandArgs<K>) => CommandResult<K> | Promise<CommandResult<K>>;
};

export class MockTransport implements Transport {
  readonly kind = "mock" as const;

  streamDelayMs: number;
  /**
   * What the next `audio_pick_recording` "returns": `undefined` → the fixture path,
   * `null` → the user cancelled the picker. One-shot; resets to `undefined` once consumed.
   */
  nextPickedRecording: string | null | undefined = undefined;
  private readonly levelTicks: boolean;
  private readonly listeners = new Map<EventName, Set<(payload: never) => void>>();

  // ── state ──
  private settings: Settings = createDefaultSettings();
  private modes: BlueyMode[] = createBuiltInModes();
  private sessions: Session[];
  private events: SessionEvent[];
  private notes: SessionNote[];
  private summaries: SessionSummary[];
  private responses: BlueyResponse[];
  private documents: BlueyDocument[];
  private segments: TranscriptSegment[] = [];
  private readonly secrets = new Map<string, string>();
  private frames = new Map<string, string>();
  private cancelled = new Set<string>();
  private levelTimer: ReturnType<typeof setInterval> | null = null;
  private nextAiFailure: string | null = null;
  /** Simulate a configured Clerk OAuth app (browser sign-in); off by default so tests run as the dev user. */
  authConfigured = false;
  /** How the next simulated browser round-trip ends. */
  nextSignInOutcome: "success" | "denied" | "hang" = "success";
  /** Times the Account Portal was opened (assertable in tests). */
  accountPortalOpens = 0;
  private authUser: AuthUser | null = null;
  private authTokens = false;
  private signInPending = false;

  /* Subscription accounts (ADR 0009): a browser round-trip simulated with a timer. */
  private accounts: ProviderAccount[] = createMockAccounts();
  private accountCatalogs = new Map<string, ProviderModelCatalog>();
  private accountTimers = new Map<string, ReturnType<typeof setTimeout>>();
  private pendingManualCodes = new Set<string>();
  /** How the next `accounts_connect` ends once the simulated browser returns. */
  nextAccountOutcome: "success" | "denied" | "hang" | "rate_limited" | "fingerprint_drift" | "needs_reauth" =
    "success";
  /** Which flow the next `accounts_connect` reports (`manual_code` waits for `accounts_submit_code`). */
  nextAccountFlow: ConnectFlowKind = "browser";
  /** Whether `accounts_import` finds an existing sign-in on this "Mac". */
  nextImportOutcome: "success" | "missing" = "success";
  /** Every provider id `accounts_disconnect` was called for (tests). */
  disconnectedAccounts: string[] = [];

  private permissions: PermissionState = {
    microphone: "granted",
    screenRecording: "not_determined",
    accessibility: "not_determined",
    notifications: "not_determined",
    speechRecognition: "granted",
    checkedAt: now(),
  };

  private protection: CaptureProtection = {
    supported: true,
    enabled: true,
    note: "Bluey excludes its windows from screen recordings and screenshots on macOS 12.3+. Hardware capture cards and cameras pointed at the display can still see it.",
  };

  private status: AppStatus = {
    state: "ready",
    audioActive: false,
    modeId: "general",
    updatedAt: now(),
  };

  private audio: AudioStatus = { state: "stopped", microphoneActive: false, systemAudioActive: false };

  private panel: PanelState = {
    visible: true,
    pinned: false,
    expanded: false,
    x: 0,
    y: 0,
    // Logical native frame (surface + shadow insets), checked against the
    // native launch config by the HUD geometry contract tests.
    width: 754,
    height: 175,
    opacity: 1,
  };

  private metrics: LatencyMetrics = {
    captureMs: 84,
    ocrMs: 128,
    accessibilityMs: 22,
    contextAssemblyMs: 41,
    timeToFirstTokenMs: 412,
    totalResponseMs: 3480,
    inputTokens: 2130,
    outputTokens: 236,
    updatedAt: now(),
  };

  constructor(options: MockTransportOptions = {}) {
    this.streamDelayMs = options.streamDelayMs ?? 30;
    this.levelTicks = options.levelTicks ?? true;
    const seed = createSeedData();
    this.sessions = seed.sessions;
    this.events = seed.events;
    this.notes = seed.notes;
    this.summaries = seed.summaries;
    this.responses = seed.responses;
    this.documents = seed.documents;
    this.status.modeId = this.settings.general.defaultModeId;
  }

  /* ── Transport interface ─────────────────────────────────────────────── */

  async invoke<K extends CommandName>(command: K, args: CommandArgs<K>): Promise<CommandResult<K>> {
    const handler = this.handlers[command] as (
      a: CommandArgs<K>,
    ) => CommandResult<K> | Promise<CommandResult<K>>;
    return await handler(args);
  }

  async listen<K extends EventName>(
    event: K,
    handler: (payload: EventPayload<K>) => void,
  ): Promise<Unlisten> {
    let set = this.listeners.get(event);
    if (!set) {
      set = new Set();
      this.listeners.set(event, set);
    }
    set.add(handler as (payload: never) => void);
    return () => {
      set?.delete(handler as (payload: never) => void);
    };
  }

  createChannel<T>(): StreamChannel<T> {
    return new MockStreamChannel<T>();
  }

  currentWindowLabel(): string {
    if (typeof window !== "undefined") {
      const label = new URLSearchParams(window.location.search).get("window");
      if (label === "settings" || label === "onboarding") return label;
    }
    return "main";
  }

  /** Emit an event to local subscribers (mock stands in for the Rust emitter). */
  emit<K extends EventName>(name: K, payload: EventPayload<K>): void {
    const set = this.listeners.get(name);
    if (!set) return;
    for (const handler of Array.from(set)) {
      (handler as (p: EventPayload<K>) => void)(payload);
    }
  }

  dispose(): void {
    if (this.levelTimer) clearInterval(this.levelTimer);
    this.levelTimer = null;
    this.accountTimers.forEach((timer) => clearTimeout(timer));
    this.accountTimers.clear();
    this.listeners.clear();
  }

  /* ── helpers ─────────────────────────────────────────────────────────── */

  private authStatus(): AuthStatus {
    return {
      state: this.authUser ? "signed_in" : "signed_out",
      user: this.authUser ?? undefined,
      hasStoredSession: this.authTokens,
      configured: this.authConfigured,
      signInPending: this.signInPending,
      checkedAt: now(),
    };
  }

  private account(accountId: string): ProviderAccount {
    const account = this.accounts.find((a) => a.accountId === accountId);
    if (!account) {
      throw blueyError({ kind: "authentication", code: "account.not_found", message: `no account \`${accountId}\`` });
    }
    return account;
  }

  private setAccount(accountId: string, patch: Partial<ProviderAccount>): ProviderAccount {
    const next = { ...this.account(accountId), ...patch };
    this.accounts = this.accounts.map((a) => (a.accountId === accountId ? next : a));
    this.emit("accounts.changed", next);
    return next;
  }

  private ensureAccountsEnabled(): void {
    if (!this.settings.experimental.subscriptionAccounts) {
      throw blueyError({
        kind: "authentication",
        code: "account.disabled",
        message: "subscription accounts are switched off in Settings → AI",
        recoverable: true,
        recovery: { type: "open_settings", tab: "ai" },
      });
    }
  }

  /** The simulated browser came back: apply `outcome` to a connecting account. */
  /**
   * Mirror of `bluey_core::accounts::apply_catalog_presets`: `overwrite` re-points every role the
   * catalog suggests; otherwise only unassigned roles and roles pointing at a model the catalog
   * no longer lists. Emits `settings.changed` when anything moved.
   */
  private applyCatalogPresets(catalog: ProviderModelCatalog, overwrite: boolean): void {
    const models = { ...this.settings.ai.models };
    let changed = false;
    for (const role of MODEL_ROLES) {
      const suggested = catalog.models.find((m) => m.suggestedRoles.includes(role));
      if (!suggested) continue;
      const current = models[role];
      const stale =
        current?.providerId === catalog.providerId && !catalog.models.some((m) => m.id === current.model);
      if (!(overwrite || !current || stale)) continue;
      if (current?.providerId === catalog.providerId && current.model === suggested.id) continue;
      models[role] = { providerId: catalog.providerId, model: suggested.id };
      changed = true;
    }
    if (!changed) return;
    this.settings = { ...this.settings, ai: { ...this.settings.ai, models } };
    this.emitSettings();
  }

  private finishAccountConnect(accountId: string, outcome: MockTransport["nextAccountOutcome"]): void {
    this.accountTimers.delete(accountId);
    this.pendingManualCodes.delete(accountId);
    const account = this.accounts.find((a) => a.accountId === accountId);
    if (!account || account.status.state !== "connecting") return;
    const at = now();
    switch (outcome) {
      case "success": {
        const catalog = createFixtureCatalog(accountId, at);
        this.accountCatalogs.set(accountId, catalog);
        this.setAccount(accountId, {
          status: { state: "connected" },
          identity: FIXTURE_ACCOUNT_IDENTITIES[accountId],
          connectedAt: at,
          expiresAt: new Date(Date.now() + 3_600_000).toISOString(),
          catalogFetchedAt: at,
        });
        this.emit("accounts.catalog", catalog);
        // Mirror of the Rust side: a fresh catalog fills the roles nobody serves yet.
        this.applyCatalogPresets(catalog, false);
        return;
      }
      case "denied":
        this.setAccount(accountId, { status: { state: "disconnected" } });
        this.emit(
          "app.error",
          blueyError({
            kind: "authentication",
            code: "account.denied",
            message: "the sign-in was not completed",
            recoverable: true,
            recovery: { type: "none" },
          }),
        );
        return;
      case "rate_limited":
        this.setAccount(accountId, {
          status: { state: "rate_limited", until: new Date(Date.now() + 2 * 3_600_000).toISOString(), window: "5h" },
          identity: FIXTURE_ACCOUNT_IDENTITIES[accountId],
          connectedAt: at,
        });
        return;
      case "fingerprint_drift":
        this.setAccount(accountId, {
          status: {
            state: "unavailable",
            reason: "fingerprint_drift",
            detail: "Third-party apps now draw from your extra usage, not your plan limits.",
          },
          identity: FIXTURE_ACCOUNT_IDENTITIES[accountId],
        });
        return;
      case "needs_reauth":
        this.setAccount(accountId, { status: { state: "needs_reauth" }, identity: FIXTURE_ACCOUNT_IDENTITIES[accountId] });
        return;
      case "hang":
        return;
    }
  }

  private connectFlow(kind: ConnectFlowKind, providerId: string): AccountStatus {
    const expiresAt = new Date(Date.now() + 600_000).toISOString();
    switch (kind) {
      case "device_code":
        return {
          state: "connecting",
          flow: {
            kind,
            userCode: "BLUEY-4821",
            verificationUrl: "https://auth.openai.com/codex/device",
            expiresAt,
          },
        };
      case "manual_code":
        return {
          state: "connecting",
          flow: { kind, url: `https://mock.${providerId}.example/oauth/authorize?code=true`, expiresAt },
        };
      default:
        return {
          state: "connecting",
          flow: { kind: "browser", url: `https://mock.${providerId}.example/oauth/authorize`, expiresAt },
        };
    }
  }

  private delay(ms: number): Promise<void> {
    if (ms <= 0 || this.streamDelayMs === 0) return Promise.resolve();
    return new Promise((resolve) => setTimeout(resolve, ms));
  }

  private setAppState(patch: Partial<AppStatus>): AppStatus {
    this.status = { ...this.status, ...patch, updatedAt: now() };
    this.emit("app.state", this.status);
    return this.status;
  }

  private emitSettings(): Settings {
    this.emit("settings.changed", this.settings);
    return this.settings;
  }

  private log(level: string, target: string, message: string): void {
    this.emit("dev.log", { level, target, message, at: now() });
  }

  private mode(id: string): BlueyMode {
    const mode = this.modes.find((m) => m.id === id);
    if (!mode)
      throw blueyError({ kind: "storage", code: "modes.not_found", message: `Mode ${id} not found` });
    return mode;
  }

  private makeFrame(inline: boolean): ScreenFrame {
    const id = createId("frame");
    this.frames.set(id, FIXTURE_PNG_BASE64);
    return {
      id,
      image: inline ? FIXTURE_PNG_BASE64 : undefined,
      mimeType: "image/png",
      width: 1512,
      height: 982,
      displayId: "display-1",
      scaleFactor: 2,
      capturedAt: now(),
      hash: "phash-fixture",
      changed: true,
      target: { type: "display", displayId: "display-1" },
      durationMs: 84,
    };
  }

  /**
   * Mirror of `AiCore::default_model_for`: the first role assignment on this provider, else the
   * kind's recommended default (the mock kind answers as `mock-default`).
   */
  private defaultModelFor(provider: AIProviderConfig): string | null {
    for (const role of MODEL_ROLES) {
      const assignment = this.settings.ai.models[role];
      if (assignment?.providerId === provider.id) return assignment.model;
    }
    if (provider.kind === "mock") return "mock-default";
    return presetForKind(provider.kind)?.models.default ?? null;
  }

  /**
   * Rust refreshes `hasApiKey` from the keychain on every settings write, so a provider created
   * after its key was stored (onboarding race) picks the flag up. The seeded flags are kept.
   */
  private withKeyFlags(settings: Settings): Settings {
    const providers = settings.ai.providers.map((p) =>
      !p.hasApiKey && this.secrets.has(`provider:${p.id}:api_key`) ? { ...p, hasApiKey: true } : p,
    );
    return { ...settings, ai: { ...settings.ai, providers } };
  }

  private sessionListItem(session: Session, snippet?: string): SessionListItem {
    return {
      session,
      modeName: this.modes.find((m) => m.id === session.modeId)?.name ?? session.modeId,
      eventCount: this.events.filter((e) => e.sessionId === session.id).length,
      responseCount: this.responses.filter((r) => r.sessionId === session.id).length,
      transcriptSegmentCount: this.segments.filter((s) => s.sessionId === session.id).length,
      hasSummary: this.summaries.some((s) => s.sessionId === session.id),
      snippet,
    };
  }

  private pushSegment(
    text: string,
    speaker: string | undefined,
    source: "microphone" | "system",
  ): TranscriptSegment {
    const startTime = this.segments.length * 4000;
    const segment: TranscriptSegment = {
      id: createId("seg"),
      sessionId: this.status.sessionId,
      speaker,
      speakerConfidence: speaker ? 0.72 : undefined,
      source,
      text,
      startTime,
      endTime: startTime + 3200,
      confidence: 0.94,
      finalized: true,
      language: "en",
      createdAt: now(),
    };
    this.segments.push(segment);
    return segment;
  }

  private buildSnapshot(args: CommandArgs<"context_build_snapshot">): ContextSnapshot {
    const opts = args.options;
    const snapshot: ContextSnapshot = {
      timestamp: now(),
      activeApplication: { name: "Google Chrome", bundleId: "com.google.Chrome", pid: 4021 },
      activeWindow: {
        title: "Two Sum — LeetCode",
        adapter: "chrome",
        hints: { url: "https://leetcode.com/problems/two-sum" },
      },
      timings: { capture: 84, ocr: 128, accessibility: 22, assembly: 41 },
    };
    if (opts.includeScreen) {
      snapshot.screen = {
        image: opts.inlineImage ? FIXTURE_PNG_BASE64 : undefined,
        mimeType: "image/png",
        width: 1512,
        height: 982,
        displayId: "display-1",
        frameId: createId("frame"),
      };
    }
    if (opts.includeOcr) {
      snapshot.ocr = {
        blocks: [
          {
            text: CODING_PROBLEM_OCR,
            confidence: 0.97,
            boundingBox: { x: 220, y: 140, width: 1050, height: 680 },
          },
        ],
        text: CODING_PROBLEM_OCR,
        level: opts.ocrLevel ?? "accurate",
        languages: ["en-US"],
        durationMs: 128,
      };
    }
    if (opts.includeAccessibility) {
      snapshot.accessibility = {
        application: { name: "Google Chrome", bundleId: "com.google.Chrome" },
        window: { title: "Two Sum — LeetCode" },
        elements: [{ role: "AXStaticText", label: "Two Sum", depth: 3 }],
        visibleText: CODING_PROBLEM_OCR,
        truncated: false,
        capturedAt: now(),
      };
    }
    if (opts.includeTranscript) {
      const windowSeconds = opts.transcriptWindowSeconds ?? 120;
      snapshot.transcript = { segments: this.segments.slice(-12), windowSeconds };
    }
    return snapshot;
  }

  private async streamAi(args: CommandArgs<"ai_stream">): Promise<void> {
    const request = args.request as AIRequest;
    const channel = args.onChunk as MockStreamChannel<AIChunk>;
    const startedAt = Date.now();

    this.emit("ai.requested", {
      requestId: request.requestId,
      task: request.task,
      sessionId: request.sessionId,
    });

    if (this.nextAiFailure) {
      const code = this.nextAiFailure;
      this.nextAiFailure = null;
      const error = blueyError({
        kind: "ai",
        code,
        message: "The model request failed (simulated).",
        recoverable: true,
        recovery: { type: "retry" },
      });
      channel.push({ type: "failed", requestId: request.requestId, error });
      this.emit("ai.failed", { requestId: request.requestId, error });
      return;
    }

    const selection = {
      providerId: "azure-foundry",
      providerKind: "azure_foundry" as const,
      model: request.task === "coding" ? "gpt-5.6-terra" : "gpt-5.6-luna",
      role: "default" as const,
      reason: "mock router",
    };

    this.setAppState({ state: "thinking" });
    await this.delay(this.streamDelayMs * 6);
    channel.push({ type: "started", requestId: request.requestId, selection });
    this.emit("ai.started", {
      requestId: request.requestId,
      provider: selection.providerId,
      model: selection.model,
    });

    const hasHistory = request.messages.some((m) => m.role === "assistant");
    const body = hasHistory ? CANNED_FOLLOW_UP_MARKDOWN : CANNED_ANSWER_MARKDOWN;
    const words = body.split(/(\s+)/).filter((w) => w.length > 0);
    let firstToken: number | undefined;

    for (const word of words) {
      if (this.cancelled.has(request.requestId)) {
        this.cancelled.delete(request.requestId);
        channel.push({
          type: "completed",
          requestId: request.requestId,
          finishReason: "cancelled",
          totalMs: Date.now() - startedAt,
          timeToFirstTokenMs: firstToken,
        });
        this.emit("ai.cancelled", { requestId: request.requestId });
        this.setAppState({ state: this.status.audioActive ? "listening" : "ready" });
        return;
      }
      if (firstToken === undefined) firstToken = Date.now() - startedAt;
      channel.push({ type: "delta", requestId: request.requestId, text: word });
      if (!/^\s+$/.test(word)) await this.delay(this.streamDelayMs);
    }

    channel.push({
      type: "usage",
      requestId: request.requestId,
      inputTokens: 2130,
      outputTokens: words.length,
    });
    const totalMs = Date.now() - startedAt;
    channel.push({
      type: "completed",
      requestId: request.requestId,
      finishReason: "stop",
      totalMs,
      timeToFirstTokenMs: firstToken,
    });
    this.emit("ai.completed", { requestId: request.requestId, totalMs, timeToFirstTokenMs: firstToken });
    this.metrics = {
      ...this.metrics,
      modelMs: totalMs,
      totalResponseMs: totalMs + 220,
      timeToFirstTokenMs: firstToken,
      outputTokens: words.length,
      updatedAt: now(),
    };
    this.emit("dev.metrics", this.metrics);
    this.setAppState({ state: "response_ready" });
  }

  private simulate(simulation: DevSimulation): void {
    this.log("info", "dev.simulate", `simulation: ${simulation.type}`);
    switch (simulation.type) {
      case "question": {
        const segment = this.pushSegment(simulation.text, simulation.speaker ?? "Interviewer", "system");
        this.emit("transcript.final", segment);
        const detected: DetectedEvent = {
          id: createId("det"),
          type: "question",
          confidence: 0.9,
          requiresResponse: true,
          text: simulation.text,
          segmentIds: [segment.id],
          speaker: segment.speaker,
          detectedAt: now(),
        };
        this.emit("question.detected", detected);
        break;
      }
      case "coding_problem": {
        const frame = this.makeFrame(false);
        this.emit("screen.captured", frame);
        this.emit("ocr.completed", {
          blocks: [],
          text: simulation.text || CODING_PROBLEM_OCR,
          level: "accurate",
          languages: ["en-US"],
          durationMs: 118,
          frameId: frame.id,
        });
        this.emit("question.detected", {
          id: createId("det"),
          type: "coding_problem",
          confidence: 0.95,
          requiresResponse: true,
          text: simulation.text || "Two Sum — return indices of two numbers adding to target",
          segmentIds: [],
          detectedAt: now(),
        });
        break;
      }
      case "transcript": {
        for (const part of simulation.segments) {
          const segment = this.pushSegment(part.text, part.speaker, part.source ?? "microphone");
          this.emit("transcript.final", segment);
        }
        break;
      }
      case "screen_capture": {
        this.emit("screen.captured", this.makeFrame(false));
        break;
      }
      case "permission_error": {
        this.permissions = { ...this.permissions, [simulation.permission]: "denied", checkedAt: now() };
        this.emit("permissions.changed", this.permissions);
        this.emit(
          "app.error",
          blueyError({
            kind: "permission",
            code: `permission.${simulation.permission}_denied`,
            message: `${simulation.permission} permission is denied.`,
            recoverable: true,
            recovery: { type: "open_system_settings", pane: simulation.permission },
          }),
        );
        break;
      }
      case "ai_latency":
        this.streamDelayMs = Math.max(0, Math.round(simulation.ms / 40));
        break;
      case "ai_failure":
        this.nextAiFailure = simulation.code ?? "ai.provider_unavailable";
        break;
      case "clear":
        this.segments = [];
        this.emit("transcript.cleared", {});
        break;
    }
  }

  /* ── Command handlers (exhaustive over CommandMap) ───────────────────── */

  private readonly handlers: Handlers = {
    // App
    app_get_status: () => this.status,
    app_pause: () => this.setAppState({ state: "paused", resumeState: this.status.state }),
    app_resume: () => this.setAppState({ state: this.status.resumeState ?? "ready", resumeState: undefined }),
    app_recover: () =>
      this.setAppState({ state: this.status.audioActive ? "listening" : "ready", error: undefined }),
    app_dismiss_response: () => this.setAppState({ state: this.status.audioActive ? "listening" : "ready" }),
    app_get_dev_info: () => ({
      version: "0.1.0-dev",
      buildProfile: "debug" as const,
      helperVersion: "0.1.0",
      helperRunning: true,
      agentSidecarAvailable: false,
      dbPath: "~/Library/Application Support/Bluey/bluey.db",
      logPath: "~/Library/Logs/Bluey/bluey.log",
      mockTransport: true,
    }),
    app_run_setup_checks: () => [
      {
        id: "screen" as const,
        label: "Screen recording",
        ok: this.permissions.screenRecording === "granted",
        detail:
          this.permissions.screenRecording === "granted"
            ? "Bluey can capture your screen."
            : "Screen Recording permission is not granted.",
        fix: "Grant Screen Recording in System Settings → Privacy & Security.",
        recovery: { type: "open_system_settings" as const, pane: "screenRecording" as const },
      },
      {
        id: "microphone" as const,
        label: "Microphone",
        ok: this.permissions.microphone === "granted",
        detail:
          this.permissions.microphone === "granted"
            ? "Microphone access is granted."
            : "Microphone permission is not granted.",
        recovery: { type: "open_system_settings" as const, pane: "microphone" as const },
      },
      {
        id: "accessibility" as const,
        label: "Accessibility",
        ok: this.permissions.accessibility === "granted",
        detail:
          this.permissions.accessibility === "granted"
            ? "Bluey can read on-screen structure."
            : "Accessibility permission is not granted.",
        recovery: { type: "open_system_settings" as const, pane: "accessibility" as const },
      },
      {
        id: "ai" as const,
        label: "AI provider",
        ok: this.settings.ai.providers.some((p) => p.enabled && p.hasApiKey),
        detail: this.settings.ai.providers.some((p) => p.enabled && p.hasApiKey)
          ? "A provider with a stored key is configured."
          : "No enabled provider has an API key.",
        recovery: { type: "open_settings" as const, tab: "ai" },
      },
      { id: "helper" as const, label: "Native helper", ok: true, detail: "Helper is running (mock)." },
      {
        id: "systemAudio" as const,
        label: "System audio",
        ok: true,
        detail: "System audio tap available (mock).",
      },
    ],
    app_quit: () => {
      this.log("info", "app", "quit requested (ignored in mock)");
    },

    // Auth (browser sign-in simulated with a timer)
    auth_get_status: () => this.authStatus(),
    auth_begin_sign_in: async () => {
      if (!this.authConfigured) {
        throw blueyError({
          kind: "authentication",
          code: "auth.not_configured",
          message: "sign-in is not configured",
        });
      }
      this.signInPending = true;
      this.emit("auth.changed", this.authStatus());
      const outcome = this.nextSignInOutcome;
      this.nextSignInOutcome = "success";
      if (outcome !== "hang") {
        setTimeout(
          () => {
            if (!this.signInPending) return;
            this.signInPending = false;
            if (outcome === "success") {
              this.authUser = MOCK_AUTH_USER;
              this.authTokens = true;
            }
            this.emit("auth.changed", this.authStatus());
          },
          Math.max(this.streamDelayMs * 4, 10),
        );
      }
      return {
        url: "https://mock.accounts.dev/oauth/authorize?client_id=mock&state=mock",
        redirect: "loopback" as const,
        expiresAt: new Date(Date.now() + 600_000).toISOString(),
      };
    },
    auth_cancel_sign_in: () => {
      this.signInPending = false;
      const status = this.authStatus();
      this.emit("auth.changed", status);
      return status;
    },
    auth_clear_session: () => {
      this.signInPending = false;
      this.authUser = null;
      this.authTokens = false;
      const status = this.authStatus();
      this.emit("auth.changed", status);
      return status;
    },
    auth_open_account_portal: () => {
      this.accountPortalOpens += 1;
    },

    // Subscription accounts (ADR 0009) — the browser round-trip is a timer, like auth above.
    accounts_list: () => this.accounts,
    accounts_status: (args) => this.account(args.accountId),
    accounts_catalog: (args) => {
      this.account(args.accountId);
      return this.accountCatalogs.get(args.accountId) ?? null;
    },
    accounts_connect: (args) => {
      this.ensureAccountsEnabled();
      const account = this.accounts.find((a) => a.providerId === args.providerId);
      if (!account) {
        throw blueyError({
          kind: "authentication",
          code: "account.unknown_provider",
          message: `\`${args.providerId}\` is not a subscription provider`,
        });
      }
      const flow = args.options?.preferDeviceCode ? "device_code" : this.nextAccountFlow;
      this.nextAccountFlow = "browser";
      const outcome = this.nextAccountOutcome;
      this.nextAccountOutcome = "success";
      const existing = this.accountTimers.get(account.accountId);
      if (existing) clearTimeout(existing);
      const connecting = this.setAccount(account.accountId, { status: this.connectFlow(flow, account.providerId) });
      if (flow === "manual_code") {
        // Completes when the user pastes the code (`accounts_submit_code`).
        this.pendingManualCodes.add(account.accountId);
        return connecting;
      }
      if (outcome !== "hang") {
        this.accountTimers.set(
          account.accountId,
          setTimeout(() => this.finishAccountConnect(account.accountId, outcome), Math.max(this.streamDelayMs * 4, 10)),
        );
      }
      return connecting;
    },
    accounts_import: (args) => {
      this.ensureAccountsEnabled();
      const account = this.accounts.find((a) => a.providerId === args.providerId);
      if (!account) {
        throw blueyError({
          kind: "authentication",
          code: "account.unknown_provider",
          message: `\`${args.providerId}\` is not a subscription provider`,
        });
      }
      const outcome = this.nextImportOutcome;
      this.nextImportOutcome = "success";
      if (outcome === "missing") {
        throw blueyError({
          kind: "authentication",
          code: "account.import_not_found",
          message: `no existing ${args.providerId} sign-in was found on this Mac`,
          recoverable: true,
          recovery: { type: "none" },
        });
      }
      this.setAccount(account.accountId, { status: this.connectFlow("browser", account.providerId) });
      this.finishAccountConnect(account.accountId, "success");
      return this.account(account.accountId);
    },
    accounts_cancel_connect: (args) => {
      const timer = this.accountTimers.get(args.accountId);
      if (timer) clearTimeout(timer);
      this.accountTimers.delete(args.accountId);
      this.pendingManualCodes.delete(args.accountId);
      const account = this.account(args.accountId);
      return account.status.state === "connecting"
        ? this.setAccount(args.accountId, { status: { state: "disconnected" } })
        : account;
    },
    accounts_submit_code: (args) => {
      if (!this.pendingManualCodes.has(args.accountId)) {
        throw blueyError({
          kind: "authentication",
          code: "account.no_pending_flow",
          message: "no sign-in is waiting for a code — start again from the account card",
        });
      }
      this.finishAccountConnect(args.accountId, args.code.trim() === "bad" ? "denied" : "success");
      return this.account(args.accountId);
    },
    accounts_disconnect: (args) => {
      const account = this.account(args.accountId);
      const timer = this.accountTimers.get(args.accountId);
      if (timer) clearTimeout(timer);
      this.accountTimers.delete(args.accountId);
      this.pendingManualCodes.delete(args.accountId);
      this.accountCatalogs.delete(args.accountId);
      this.disconnectedAccounts.push(account.providerId);
      // Roles that pointed at this provider go back to unassigned.
      const models = { ...this.settings.ai.models };
      let changed = false;
      for (const role of MODEL_ROLES) {
        if (models[role]?.providerId === account.providerId) {
          models[role] = null;
          changed = true;
        }
      }
      if (changed) {
        this.settings = { ...this.settings, ai: { ...this.settings.ai, models } };
        this.emitSettings();
      }
      this.setAccount(args.accountId, {
        status: { state: "disconnected" },
        identity: undefined,
        connectedAt: undefined,
        expiresAt: undefined,
        catalogFetchedAt: undefined,
      });
    },
    accounts_refresh_catalog: (args) => {
      this.ensureAccountsEnabled();
      const account = this.account(args.accountId);
      if (account.status.state !== "connected") {
        throw blueyError({
          kind: "authentication",
          code: "account.not_connected",
          message: "connect the account before fetching its models",
        });
      }
      const cached = this.accountCatalogs.get(args.accountId);
      if (cached && !args.force) return cached;
      const catalog = createFixtureCatalog(args.accountId, now());
      this.accountCatalogs.set(args.accountId, catalog);
      this.setAccount(args.accountId, { catalogFetchedAt: catalog.fetchedAt });
      this.emit("accounts.catalog", catalog);
      return catalog;
    },
    accounts_probe_fingerprint: (args) => {
      this.ensureAccountsEnabled();
      const account = this.account(args.accountId);
      if (account.status.state !== "connected") {
        throw blueyError({
          kind: "authentication",
          code: "account.not_connected",
          message: "connect the account before probing it",
        });
      }
      return {
        accountId: args.accountId,
        ok: true,
        billedTo: "plan" as const,
        fingerprintVersion: account.fingerprintVersion,
        message: "1-token probe billed to the plan",
        checkedAt: now(),
      };
    },

    // Permissions
    permissions_get: () => this.permissions,
    permissions_request: (args) => {
      this.permissions = { ...this.permissions, [args.kind]: "granted", checkedAt: now() };
      this.emit("permissions.changed", this.permissions);
      return this.permissions;
    },
    permissions_open_settings: (args) => {
      this.log("info", "permissions", `open System Settings pane: ${args.kind}`);
    },

    // Screen capture
    capture_list_displays: () => FIXTURE_DISPLAYS,
    capture_list_windows: () => [
      {
        windowId: 118,
        title: "Two Sum — LeetCode",
        ownerName: "Google Chrome",
        bundleId: "com.google.Chrome",
        pid: 4021,
        bounds: { x: 0, y: 38, width: 1512, height: 944 },
        onScreen: true,
      },
    ],
    capture_screen: async (args) => {
      const frame = this.makeFrame(args?.options?.inline ?? false);
      this.emit("screen.captured", frame);
      return frame;
    },
    capture_read_frame: (args) => {
      const data = this.frames.get(args.frameId);
      if (!data)
        throw blueyError({
          kind: "capture",
          code: "capture.frame_not_found",
          message: `Frame ${args.frameId} not found`,
        });
      return data;
    },
    capture_discard_frame: (args) => {
      this.frames.delete(args.frameId);
    },
    capture_observe_start: () => {
      this.log("info", "capture", "observation started");
    },
    capture_observe_stop: () => {
      this.log("info", "capture", "observation stopped");
    },
    capture_get_protection: () => this.protection,
    capture_set_protection: (args) => {
      this.protection = { ...this.protection, enabled: args.enabled };
      return this.protection;
    },

    // OCR / accessibility
    ocr_recognize: (args) => ({
      blocks: [
        {
          text: CODING_PROBLEM_OCR,
          confidence: 0.97,
          boundingBox: { x: 220, y: 140, width: 1050, height: 680 },
        },
      ],
      text: CODING_PROBLEM_OCR,
      level: args.level ?? "accurate",
      languages: args.languages ?? ["en-US"],
      durationMs: 128,
      frameId: args.frameId,
    }),
    accessibility_snapshot: () => ({
      application: { name: "Google Chrome", bundleId: "com.google.Chrome" },
      window: { title: "Two Sum — LeetCode" },
      elements: [{ role: "AXStaticText", label: "Two Sum", depth: 3 }],
      visibleText: CODING_PROBLEM_OCR,
      truncated: false,
      capturedAt: now(),
    }),
    accessibility_frontmost_app: () => ({
      application: { name: "Google Chrome", bundleId: "com.google.Chrome", pid: 4021 },
      window: { title: "Two Sum — LeetCode" },
    }),

    // Audio / transcript
    audio_list_devices: () => FIXTURE_AUDIO_DEVICES,
    audio_start: (args) => {
      const device =
        FIXTURE_AUDIO_DEVICES.find(
          (d) => d.id === (args?.config?.microphone?.deviceId ?? this.settings.audio.microphoneDeviceId),
        ) ?? FIXTURE_AUDIO_DEVICES[0];
      this.audio = {
        state: "running",
        microphoneActive: true,
        systemAudioActive: this.settings.audio.source !== "microphone",
        provider: this.settings.audio.transcriptionProvider,
        currentInputDevice: device,
        startedAt: now(),
        levels: { microphone: 0.1, system: 0.05 },
      };
      this.emit("audio.started", this.audio);
      this.setAppState({
        audioActive: true,
        state: this.status.state === "ready" ? "listening" : this.status.state,
      });
      if (this.levelTicks && this.streamDelayMs > 0 && !this.levelTimer) {
        this.levelTimer = setInterval(() => {
          const mic = Math.max(0, Math.min(1, 0.25 + Math.random() * 0.5));
          const system = Math.max(0, Math.min(1, 0.1 + Math.random() * 0.3));
          this.emit("audio.level", { microphone: mic, system });
        }, 120);
      }
      return this.audio;
    },
    audio_stop: () => {
      if (this.levelTimer) {
        clearInterval(this.levelTimer);
        this.levelTimer = null;
      }
      this.audio = { state: "stopped", microphoneActive: false, systemAudioActive: false };
      this.emit("audio.stopped", this.audio);
      this.setAppState({
        audioActive: false,
        state: this.status.state === "listening" ? "ready" : this.status.state,
      });
      return this.audio;
    },
    audio_pause: () => {
      this.audio = { ...this.audio, state: "paused", microphoneActive: false, systemAudioActive: false };
      this.emit("audio.paused", this.audio);
      return this.audio;
    },
    audio_resume: () => {
      this.audio = { ...this.audio, state: "running", microphoneActive: true };
      this.emit("audio.resumed", this.audio);
      return this.audio;
    },
    audio_get_status: () => this.audio,
    audio_test_microphone: async (args) => {
      const duration = args.durationMs ?? 1200;
      const ticks = this.streamDelayMs === 0 ? 1 : Math.max(1, Math.floor(duration / 100));
      let peak = 0;
      for (let i = 0; i < ticks; i += 1) {
        const level = 0.2 + Math.random() * 0.6;
        peak = Math.max(peak, level);
        this.emit("audio.level", { microphone: level, system: 0 });
        await this.delay(100);
      }
      return { peakLevel: Number(peak.toFixed(2)), ok: true };
    },
    audio_pick_recording: () => {
      const next = this.nextPickedRecording;
      this.nextPickedRecording = undefined;
      return next === undefined ? MOCK_RECORDING_PATH : next;
    },
    transcript_list: (args) => {
      let list = this.segments;
      if (args.sessionId) list = list.filter((s) => s.sessionId === args.sessionId);
      if (args.sinceMs !== undefined) list = list.filter((s) => s.startTime >= (args.sinceMs ?? 0));
      if (args.limit !== undefined) list = list.slice(-args.limit);
      return list;
    },
    transcript_recent: (args) => {
      const cutoff =
        this.segments.length > 0
          ? (this.segments[this.segments.length - 1]?.endTime ?? 0) - args.windowSeconds * 1000
          : 0;
      return this.segments.filter((s) => s.endTime >= cutoff);
    },
    transcript_clear: (args) => {
      this.segments = args.sessionId ? this.segments.filter((s) => s.sessionId !== args.sessionId) : [];
      this.emit("transcript.cleared", { sessionId: args.sessionId });
    },

    // Context
    context_build_snapshot: async (args) => {
      this.setAppState({ state: "capturing" });
      await this.delay(this.streamDelayMs * 4);
      const snapshot = this.buildSnapshot(args);
      this.setAppState({ state: "analyzing" });
      this.emit("context.updated", { snapshot, reason: "manual" });
      return snapshot;
    },

    // AI
    ai_stream: (args) => this.streamAi(args),
    ai_cancel: (args) => {
      this.cancelled.add(args.requestId);
      return true;
    },
    ai_cancel_all: () => {
      const count = this.cancelled.size;
      return count;
    },
    ai_embed: (args) =>
      args.texts.map((text) => Array.from({ length: 8 }, (_, i) => ((text.length * (i + 3)) % 97) / 97)),
    // Same contract as `AiCore::test_connection`: unknown provider / no model THROW; a provider
    // without a key answers `ok: false` with `config.missing_key`.
    ai_test_connection: async (args) => {
      const provider = this.settings.ai.providers.find((p) => p.id === args.providerId);
      await this.delay(this.streamDelayMs * 8);
      if (!provider) {
        throw blueyError({
          kind: "configuration",
          code: "config.unknown_provider",
          message: "the provider is not configured",
          recoverable: true,
          recovery: { type: "configure_provider" },
        });
      }
      const model = args.model ?? this.defaultModelFor(provider);
      if (!model) {
        throw blueyError({
          kind: "configuration",
          code: "config.no_model",
          message: "pass a model or assign one to this provider first",
          recoverable: true,
          recovery: { type: "configure_provider" },
        });
      }
      if (!provider.hasApiKey) {
        return {
          ok: false,
          providerId: provider.id,
          model,
          error: blueyError({
            kind: "configuration",
            code: "config.missing_key",
            message: "the provider has no API key configured",
            recoverable: true,
            recovery: { type: "configure_provider" },
          }),
        };
      }
      if (this.nextAiFailure) {
        // `dev_simulate { type: "ai_failure" }` fails the next model request — this one included.
        const code = this.nextAiFailure;
        this.nextAiFailure = null;
        return {
          ok: false,
          providerId: provider.id,
          model,
          error: blueyError({
            kind: "ai",
            code,
            message: "The model request failed (simulated).",
            recoverable: true,
            recovery: { type: "retry" },
          }),
        };
      }
      return { ok: true, providerId: provider.id, model, latencyMs: 132 };
    },
    ai_transcribe_file: async (args) => {
      await this.delay(this.streamDelayMs * 4);
      const fileName = args.path.split("/").pop() || "recording";
      let session = args.sessionId ? this.sessions.find((s) => s.id === args.sessionId) : undefined;
      if (args.sessionId && !session) {
        throw blueyError({
          kind: "storage",
          code: "storage.not_found",
          message: `Session ${args.sessionId} not found`,
        });
      }
      if (!session) {
        session = {
          id: createId("session"),
          modeId: this.status.modeId,
          startedAt: now(),
          endedAt: now(),
          status: "completed",
          title: `Imported · ${fileName}`,
        };
        this.sessions = [session, ...this.sessions];
      }
      const sessionId = session.id;
      const segments: TranscriptSegment[] = MOCK_RECORDING_LINES.map(([speaker, text], index) => ({
        id: createId("seg"),
        sessionId,
        speaker: args.diarization ? speaker : undefined,
        source: "system",
        text,
        startTime: index * 4_000,
        endTime: index * 4_000 + 3_500,
        finalized: true,
        language: args.language,
        createdAt: now(),
      }));
      // Privacy → store transcripts off: the segments are returned to the caller but never persisted.
      const stored = this.settings.privacy.storeTranscripts;
      if (stored) this.segments = [...this.segments, ...segments];
      const speakers = args.diarization ? new Set(MOCK_RECORDING_LINES.map(([speaker]) => speaker)).size : 0;
      const durationMs = segments[segments.length - 1]?.endTime ?? 0;
      const event: SessionEvent = {
        id: createId("ev"),
        sessionId,
        type: "recording_imported",
        title: "Recording imported",
        detail: `${fileName} · ${segments.length} segments · ${Math.round(durationMs / 1000)} s${
          speakers ? ` · ${speakers} speakers` : ""
        }`,
        refs: { file: fileName, segments: String(segments.length) },
        createdAt: now(),
      };
      this.events = [...this.events, event];
      this.emit("session.event", event);
      const result: TranscribeFileResult = {
        session,
        segments,
        speakers,
        durationMs,
        language: args.language,
        stored,
      };
      return result;
    },
    ai_list_models: (args) => {
      // A subscription account lists what its plan's catalog exposes (mirror of the ChatGPT adapter).
      const account = this.accounts.find((a) => a.providerId === args.providerId);
      if (account) {
        const catalog = this.accountCatalogs.get(account.accountId);
        if (!catalog)
          throw blueyError({
            kind: "authentication",
            code: "account.not_connected",
            message: "connect the account and refresh its models first",
          });
        return args.role === "embedding" || args.role === "transcription" ? [] : catalog.models.map((m) => m.id);
      }
      const provider = this.settings.ai.providers.find((p) => p.id === args.providerId);
      const models = FIXTURE_MODELS_BY_KIND[provider?.kind ?? "mock"] ?? [];
      // Mirror of the Rust per-role filter: embeddings / transcription / text generation.
      const isEmbedding = (id: string) => /embedding/i.test(id);
      const isTranscription = (id: string) => /transcribe/i.test(id);
      switch (args.role) {
        case "embedding":
          return models.filter(isEmbedding);
        case "transcription":
          return models.filter(isTranscription);
        case undefined:
          return models;
        default:
          return models.filter((id) => !isEmbedding(id) && !isTranscription(id) && !/-live$/i.test(id));
      }
    },
    ai_apply_provider_presets: (args) => {
      const account = this.accounts.find((a) => a.providerId === args.providerId);
      if (account) {
        const catalog = this.accountCatalogs.get(account.accountId);
        if (!catalog || account.status.state !== "connected")
          throw blueyError({
            kind: "authentication",
            code: "account.not_connected",
            message: "connect the account before applying its models",
          });
        this.applyCatalogPresets(catalog, args.overwrite);
        return this.settings;
      }
      const provider = this.settings.ai.providers.find((p) => p.id === args.providerId);
      if (!provider)
        throw blueyError({
          kind: "configuration",
          code: "config.unknown_provider",
          message: "the provider is not configured",
        });
      let result: ReturnType<typeof applyPresets>;
      try {
        result = applyPresets(this.settings.ai.models, provider, args.overwrite);
      } catch {
        throw blueyError({
          kind: "configuration",
          code: "config.no_preset",
          message: "this provider kind has no recommended models",
        });
      }
      this.settings = { ...this.settings, ai: { ...this.settings.ai, models: result.models } };
      this.emitSettings();
      return this.settings;
    },

    // Research
    research_search: (args) => [
      {
        id: "sr-1",
        title: `Result for "${args.query}"`,
        url: "https://example.com/1",
        snippet: "Fixture search result.",
        source: "mock" as const,
      },
      {
        id: "sr-2",
        title: "Second fixture result",
        url: "https://example.com/2",
        snippet: "More fixture context.",
        source: "mock" as const,
      },
    ],
    research_scrape: (args) => ({
      url: args.url,
      title: "Fixture page",
      markdown: "# Fixture page\n\nScraped content (mock).",
      source: "mock" as const,
    }),
    research_deep_start: async (args) => {
      const jobId = args.request.jobId;
      this.emit("research.event", { type: "started", jobId });
      await this.delay(this.streamDelayMs * 10);
      this.emit("research.event", { type: "progress", jobId, message: "Searching sources…" });
      await this.delay(this.streamDelayMs * 10);
      this.emit("research.event", {
        type: "completed",
        jobId,
        report: "## Research report\n\nFixture findings (mock).",
        citations: [{ id: "c1", title: "Example source", url: "https://example.com" }],
        totalMs: 1200,
        turns: 2,
      });
    },
    research_deep_cancel: () => true,
    research_available: () => ({ search: true, scrape: true, deepAgent: false }),

    // Modes
    modes_list: () => this.modes,
    modes_get: (args) => this.mode(args.id),
    modes_create: (args) => {
      const created: BlueyMode = {
        id: createId("mode"),
        name: args.draft.name || "Untitled Mode",
        description: args.draft.description ?? "",
        icon: args.draft.icon ?? "sparkles",
        systemInstructions: args.draft.systemInstructions ?? "",
        responseSchema: args.draft.responseSchema ?? "answer",
        preferredLatency: args.draft.preferredLatency ?? "fast",
        contextRequirements: args.draft.contextRequirements ?? ["screen", "transcript"],
        builtIn: false,
        group: args.draft.group,
        responseStyle: args.draft.responseStyle,
        preferredModelRole: args.draft.preferredModelRole,
        attachedDocumentIds: [],
        createdAt: now(),
        updatedAt: now(),
      };
      this.modes = [...this.modes, created];
      this.emit("modes.changed", this.modes);
      return created;
    },
    modes_update: (args) => {
      const current = this.mode(args.id);
      const updated: BlueyMode = { ...current, ...args.patch, updatedAt: now() } as BlueyMode;
      this.modes = this.modes.map((m) => (m.id === args.id ? updated : m));
      this.emit("modes.changed", this.modes);
      return updated;
    },
    modes_delete: (args) => {
      const mode = this.mode(args.id);
      if (mode.builtIn)
        throw blueyError({
          kind: "configuration",
          code: "modes.built_in",
          message: "Built-in modes cannot be deleted.",
        });
      this.modes = this.modes.filter((m) => m.id !== args.id);
      if (this.status.modeId === args.id) this.setAppState({ modeId: this.settings.general.defaultModeId });
      this.emit("modes.changed", this.modes);
    },
    modes_duplicate: (args) => {
      const source = this.mode(args.id);
      const copy: BlueyMode = {
        ...source,
        id: createId("mode"),
        name: `${source.name} copy`,
        builtIn: false,
        createdAt: now(),
        updatedAt: now(),
      };
      this.modes = [...this.modes, copy];
      this.emit("modes.changed", this.modes);
      return copy;
    },
    modes_set_default: (args) => {
      this.mode(args.id);
      this.settings = { ...this.settings, general: { ...this.settings.general, defaultModeId: args.id } };
      return this.emitSettings();
    },
    modes_set_active: (args) => {
      const mode = this.mode(args.id);
      const status = this.setAppState({ modeId: mode.id });
      this.emit("mode.changed", { mode, sessionId: this.status.sessionId });
      return status;
    },
    modes_reset_built_in: (args) => {
      const original = createBuiltInModes().find((m) => m.id === args.id);
      if (!original)
        throw blueyError({
          kind: "configuration",
          code: "modes.not_built_in",
          message: "Not a built-in mode.",
        });
      const reset: BlueyMode = { ...original, updatedAt: now() };
      this.modes = this.modes.map((m) => (m.id === args.id ? reset : m));
      this.emit("modes.changed", this.modes);
      return reset;
    },

    // Sessions
    sessions_start: (args) => {
      const session: Session = {
        id: createId("session"),
        modeId: args.modeId ?? this.status.modeId,
        startedAt: now(),
        status: "active",
        title: args.title,
      };
      this.sessions = [session, ...this.sessions];
      this.setAppState({ sessionId: session.id });
      this.emit("session.started", session);
      return session;
    },
    sessions_pause: () => {
      const session = this.requireActiveSession();
      const updated = { ...session, status: "paused" as const };
      this.replaceSession(updated);
      this.emit("session.paused", updated);
      return updated;
    },
    sessions_resume: () => {
      const session = this.requireActiveSession();
      const updated = { ...session, status: "active" as const };
      this.replaceSession(updated);
      this.emit("session.resumed", updated);
      return updated;
    },
    sessions_end: () => {
      const session = this.requireActiveSession();
      const updated = { ...session, status: "completed" as const, endedAt: now() };
      this.replaceSession(updated);
      this.setAppState({ sessionId: undefined });
      this.emit("session.ended", updated);
      return updated;
    },
    sessions_get_active: () => this.sessions.find((s) => s.id === this.status.sessionId) ?? null,
    sessions_list: (args) => {
      let list = [...this.sessions].sort((a, b) => b.startedAt.localeCompare(a.startedAt));
      const query = args.query;
      if (query?.modeId) list = list.filter((s) => s.modeId === query.modeId);
      if (query?.limit) list = list.slice(0, query.limit);
      return list.map((s) => this.sessionListItem(s));
    },
    sessions_get: (args) => {
      const session = this.sessions.find((s) => s.id === args.id);
      if (!session)
        throw blueyError({
          kind: "storage",
          code: "sessions.not_found",
          message: `Session ${args.id} not found`,
        });
      return {
        session,
        events: this.events
          .filter((e) => e.sessionId === session.id)
          .sort((a, b) => a.createdAt.localeCompare(b.createdAt)),
        notes: this.notes.filter((n) => n.sessionId === session.id),
        summary: this.summaries.find((s) => s.sessionId === session.id),
        responses: this.responses.filter((r) => r.sessionId === session.id),
        transcriptSegmentCount: this.segments.filter((s) => s.sessionId === session.id).length,
      };
    },
    sessions_search: (args) => {
      const text = args.query.text?.toLowerCase() ?? "";
      return this.sessions
        .filter((s) => {
          if (args.query.modeId && s.modeId !== args.query.modeId) return false;
          if (args.query.from && s.startedAt < args.query.from) return false;
          if (args.query.to && s.startedAt > args.query.to) return false;
          if (!text) return true;
          const inTitle = (s.title ?? "").toLowerCase().includes(text);
          const inEvents = this.events.some(
            (e) => e.sessionId === s.id && `${e.title} ${e.detail ?? ""}`.toLowerCase().includes(text),
          );
          return inTitle || inEvents;
        })
        .map((s) => this.sessionListItem(s, text ? `…${text}…` : undefined));
    },
    sessions_delete: (args) => {
      this.sessions = this.sessions.filter((s) => s.id !== args.id);
      this.events = this.events.filter((e) => e.sessionId !== args.id);
      this.notes = this.notes.filter((n) => n.sessionId !== args.id);
      this.summaries = this.summaries.filter((s) => s.sessionId !== args.id);
      this.responses = this.responses.filter((r) => r.sessionId !== args.id);
      this.segments = this.segments.filter((s) => s.sessionId !== args.id);
    },
    sessions_delete_all: () => {
      const count = this.sessions.length;
      this.sessions = [];
      this.events = [];
      this.notes = [];
      this.summaries = [];
      this.responses = this.responses.filter((r) => !r.sessionId);
      this.segments = this.segments.filter((s) => !s.sessionId);
      return count;
    },
    sessions_rename: (args) => {
      const session = this.sessions.find((s) => s.id === args.id);
      if (!session)
        throw blueyError({
          kind: "storage",
          code: "sessions.not_found",
          message: `Session ${args.id} not found`,
        });
      const updated = { ...session, title: args.title };
      this.replaceSession(updated);
      return updated;
    },
    sessions_add_event: (args) => {
      const event: SessionEvent = {
        id: createId("ev"),
        sessionId: args.sessionId,
        type: args.type,
        title: args.title,
        detail: args.detail,
        refs: args.refs,
        confidence: args.confidence,
        createdAt: now(),
      };
      this.events = [...this.events, event];
      this.emit("session.event", event);
      return event;
    },
    sessions_list_events: (args) => this.events.filter((e) => e.sessionId === args.sessionId),
    sessions_add_note: (args) => {
      const note: SessionNote = {
        id: createId("note"),
        sessionId: args.sessionId,
        content: args.content,
        createdAt: now(),
        updatedAt: now(),
      };
      this.notes = [...this.notes, note];
      return note;
    },
    sessions_delete_note: (args) => {
      this.notes = this.notes.filter((n) => n.id !== args.noteId);
    },
    sessions_save_summary: (args) => {
      const summary: SessionSummary = { ...args.summary, id: createId("summary"), createdAt: now() };
      this.summaries = [...this.summaries.filter((s) => s.sessionId !== summary.sessionId), summary];
      return summary;
    },
    sessions_get_summary: (args) => this.summaries.find((s) => s.sessionId === args.sessionId) ?? null,

    // Responses
    responses_save: (args) => {
      this.responses = [...this.responses.filter((r) => r.id !== args.response.id), args.response];
      return args.response;
    },
    responses_list: (args) => {
      let list = this.responses.filter((r) => r.sessionId === args.sessionId);
      if (args.limit) list = list.slice(-args.limit);
      return list;
    },
    responses_get: (args) => {
      const response = this.responses.find((r) => r.id === args.id);
      if (!response)
        throw blueyError({
          kind: "storage",
          code: "responses.not_found",
          message: `Response ${args.id} not found`,
        });
      return response;
    },
    responses_feedback: (args) => {
      let response = this.responses.find((r) => r.id === args.responseId);
      if (!response) {
        response = {
          id: args.responseId,
          requestId: args.responseId,
          modeId: this.status.modeId,
          type: "answer",
          content: "",
          createdAt: now(),
        };
        this.responses = [...this.responses, response];
      }
      const updated: BlueyResponse = {
        ...response,
        feedback: {
          responseId: args.responseId,
          rating: args.rating,
          categories: args.categories,
          comment: args.comment,
          createdAt: now(),
        },
      };
      this.responses = this.responses.map((r) => (r.id === args.responseId ? updated : r));
      return updated;
    },
    responses_delete: (args) => {
      this.responses = this.responses.filter((r) => r.id !== args.id);
    },

    // Documents
    documents_add: (args) => {
      const input = args.input;
      const fromPath = input.path?.split("/").pop();
      const format =
        input.format ?? (fromPath?.split(".").pop() as BlueyDocument["format"] | undefined) ?? "text";
      const doc: BlueyDocument = {
        id: createId("doc"),
        title: input.title ?? fromPath ?? "Untitled document",
        kind: input.kind,
        format: ["pdf", "docx", "txt", "md", "text"].includes(format) ? format : "text",
        scope: input.scope,
        scopeId: input.scopeId,
        sourcePath: input.path,
        sizeBytes: input.content?.length ?? 24_576,
        chunkCount: 4,
        indexStatus: "indexed",
        hasEmbeddings: this.settings.ai.embeddingsEnabled,
        createdAt: now(),
        updatedAt: now(),
      };
      this.documents = [...this.documents, doc];
      if (input.scope === "mode" && input.scopeId) {
        this.modes = this.modes.map((m) =>
          m.id === input.scopeId ? { ...m, attachedDocumentIds: [...m.attachedDocumentIds, doc.id] } : m,
        );
        this.emit("modes.changed", this.modes);
      }
      return doc;
    },
    documents_list: (args) => {
      let list = this.documents;
      if (args.scope) list = list.filter((d) => d.scope === args.scope);
      if (args.scopeId) list = list.filter((d) => d.scopeId === args.scopeId);
      return list;
    },
    documents_get: (args) => {
      const doc = this.documents.find((d) => d.id === args.id);
      if (!doc)
        throw blueyError({
          kind: "storage",
          code: "documents.not_found",
          message: `Document ${args.id} not found`,
        });
      return doc;
    },
    documents_get_text: () => "Fixture document text (mock).",
    documents_delete: (args) => {
      this.documents = this.documents.filter((d) => d.id !== args.id);
      this.modes = this.modes.map((m) =>
        m.attachedDocumentIds.includes(args.id)
          ? { ...m, attachedDocumentIds: m.attachedDocumentIds.filter((id) => id !== args.id) }
          : m,
      );
      this.emit("modes.changed", this.modes);
    },
    documents_delete_all: (args) => {
      const before = this.documents.length;
      this.documents = args.scope ? this.documents.filter((d) => d.scope !== args.scope) : [];
      return before - this.documents.length;
    },
    documents_retrieve: (args) =>
      this.documents.slice(0, args.query.limit ?? 4).map((doc, index) => ({
        chunkId: `${doc.id}-chunk-${index}`,
        documentId: doc.id,
        documentTitle: doc.title,
        documentKind: doc.kind,
        content: `Relevant excerpt from ${doc.title} (mock).`,
        score: 0.9 - index * 0.1,
        scope: doc.scope,
      })),
    documents_reindex: () => this.documents.length,
    documents_pick_files: () => [
      "/Users/jordan/Documents/Portfolio.pdf",
      "/Users/jordan/Documents/Project notes.md",
    ],

    // Settings & secrets
    settings_get: () => this.settings,
    settings_update: (args) => {
      this.settings = this.withKeyFlags(mergeSettings(this.settings, args.patch));
      return this.emitSettings();
    },
    settings_reset: () => {
      this.settings = createDefaultSettings();
      return this.emitSettings();
    },
    secrets_set: (args) => {
      this.secrets.set(args.key, args.value);
      const match = /^provider:(.+):api_key$/.exec(args.key);
      if (match) {
        this.settings = {
          ...this.settings,
          ai: {
            ...this.settings.ai,
            providers: this.settings.ai.providers.map((p) =>
              p.id === match[1] ? { ...p, hasApiKey: true } : p,
            ),
          },
        };
        this.emitSettings();
      }
    },
    secrets_has: (args) => this.secrets.has(args.key),
    secrets_delete: (args) => {
      this.secrets.delete(args.key);
    },

    // Shortcuts
    shortcuts_list: () => this.settings.shortcuts,
    shortcuts_update: (args) => {
      this.settings = {
        ...this.settings,
        shortcuts: this.settings.shortcuts.map((s) =>
          s.id === args.id ? { ...s, accelerator: args.accelerator, enabled: args.enabled ?? s.enabled } : s,
        ),
      };
      this.emitSettings();
      return this.settings.shortcuts;
    },
    shortcuts_reset: () => {
      this.settings = { ...this.settings, shortcuts: DEFAULT_SHORTCUTS.map((s) => ({ ...s })) };
      this.emitSettings();
      return this.settings.shortcuts;
    },
    shortcuts_check_conflict: (args): ShortcutConflict | null => {
      const system = ["CmdOrCtrl+Q", "CmdOrCtrl+W", "CmdOrCtrl+Space", "CmdOrCtrl+Tab"];
      if (system.includes(args.accelerator)) {
        return { accelerator: args.accelerator, conflictsWith: "system", detail: "Reserved by macOS." };
      }
      const clash = this.settings.shortcuts.find(
        (s) => s.accelerator === args.accelerator && s.id !== args.ignoreId,
      );
      if (clash) {
        return {
          accelerator: args.accelerator,
          conflictsWith: "bluey",
          detail: `Already used by “${clash.label}”.`,
        };
      }
      return null;
    },

    // Panel / windows
    panel_show: () => this.setPanel({ visible: true }),
    panel_hide: () => this.setPanel({ visible: false }),
    panel_toggle: () => this.setPanel({ visible: !this.panel.visible }),
    panel_move: (args) => {
      const step = args.stepPx ?? 24;
      const delta = {
        up: { x: 0, y: -step },
        down: { x: 0, y: step },
        left: { x: -step, y: 0 },
        right: { x: step, y: 0 },
      }[args.direction];
      return this.setPanel({ x: this.panel.x + delta.x, y: this.panel.y + delta.y });
    },
    panel_set_position: (args) => this.setPanel({ x: args.x, y: args.y }),
    panel_resize: (args) => this.setPanel({ width: args.width, height: args.height }),
    panel_set_expanded: (args) => {
      // `height` already includes the borders/insets, even for idle transcripts.
      const measured = args.height;
      const height =
        measured !== undefined && Number.isFinite(measured) && measured > 0
          ? Math.ceil(measured)
          : args.expanded
            ? 544
            : 175;
      return this.setPanel({ expanded: args.expanded, height });
    },
    panel_set_opacity: (args) => this.setPanel({ opacity: args.opacity }),
    panel_set_pinned: (args) => this.setPanel({ pinned: args.pinned }),
    panel_get_state: () => this.panel,
    panel_start_drag: () => {
      this.log("debug", "panel", "drag started");
    },
    // Browser HudMenu uses Radix. A direct mock invocation behaves like cancellation.
    hud_menu_popup: () => null,
    window_open: (args) => {
      if (typeof window !== "undefined" && typeof window.open === "function") {
        const url = new URL(window.location.href);
        url.searchParams.set("window", args.label);
        url.searchParams.delete("tab");
        if (args.route) url.searchParams.set("tab", args.route);
        try {
          window.open(url.toString(), `bluey-${args.label}`);
        } catch {
          // jsdom: window.open is not implemented — fine in tests.
        }
      }
    },
    window_close: (args) => {
      this.log("info", "window", `close requested: ${args.label}`);
    },

    // Data management
    data_usage_stats: () => ({
      sessions: this.sessions.length,
      responses: this.responses.length,
      transcriptSegments: this.segments.length,
      screenshots: this.frames.size,
      documents: this.documents.length,
      dbSizeBytes: 6_815_744,
      screenshotCacheBytes: this.frames.size * 480_000,
    }),
    data_delete_screenshots: () => {
      const count = this.frames.size;
      this.frames = new Map();
      return count;
    },
    data_clear_transcripts: () => {
      const count = this.segments.length;
      this.segments = [];
      this.emit("transcript.cleared", {});
      return count;
    },
    data_clear_ai_cache: () => 12,
    data_reset_all: () => {
      this.settings = createDefaultSettings();
      this.modes = createBuiltInModes();
      this.sessions = [];
      this.events = [];
      this.notes = [];
      this.summaries = [];
      this.responses = [];
      this.documents = [];
      this.segments = [];
      this.secrets.clear();
      this.emitSettings();
      this.emit("modes.changed", this.modes);
    },
    data_export_session: (args) => {
      const detail = this.sessions.find((s) => s.id === args.sessionId);
      if (!detail)
        throw blueyError({
          kind: "storage",
          code: "sessions.not_found",
          message: `Session ${args.sessionId} not found`,
        });
      if (args.format === "json") {
        return JSON.stringify(
          { session: detail, events: this.events.filter((e) => e.sessionId === detail.id) },
          null,
          2,
        );
      }
      const lines = [
        `# ${detail.title ?? "Bluey session"}`,
        "",
        `Mode: ${this.modes.find((m) => m.id === detail.modeId)?.name ?? detail.modeId}`,
        `Started: ${detail.startedAt}`,
        "",
        "## Timeline",
        ...this.events
          .filter((e) => e.sessionId === detail.id)
          .map((e) => `- **${e.title}**${e.detail ? ` — ${e.detail}` : ""}`),
        "",
        "## Responses",
        ...this.responses
          .filter((r) => r.sessionId === detail.id)
          .flatMap((r) => [`### ${r.title ?? "Response"}`, "", r.content, ""]),
      ];
      return lines.join("\n");
    },

    // Developer mode
    dev_simulate: (args) => this.simulate(args.simulation),
    dev_get_metrics: () => this.metrics,
    dev_restart_helper: async () => {
      this.emit("helper.status", { running: false });
      await this.delay(this.streamDelayMs * 6);
      this.emit("helper.status", { running: true, version: "0.1.0", restarted: true });
    },
  };

  private requireActiveSession(): Session {
    const session = this.sessions.find((s) => s.id === this.status.sessionId);
    if (!session)
      throw blueyError({ kind: "storage", code: "sessions.no_active", message: "No active session" });
    return session;
  }

  private replaceSession(session: Session): void {
    this.sessions = this.sessions.map((s) => (s.id === session.id ? session : s));
  }

  private setPanel(patch: Partial<PanelState>): PanelState {
    if (Object.entries(patch).every(([key, value]) => this.panel[key as keyof PanelState] === value)) {
      return this.panel;
    }
    this.panel = { ...this.panel, ...patch };
    this.emit("panel.state", this.panel);
    return this.panel;
  }
}

/** Deep-merge a SettingsPatch: objects merge one level, arrays/scalars replace. */
function mergeSettings(current: Settings, patch: SettingsPatch): Settings {
  const next = { ...current } as unknown as Record<string, unknown>;
  const base = current as unknown as Record<string, unknown>;
  for (const key of Object.keys(patch) as Array<keyof Settings>) {
    const value = patch[key];
    if (value === undefined) continue;
    if (Array.isArray(value) || typeof value !== "object" || value === null) {
      next[key] = value;
    } else {
      next[key] = { ...(base[key] as object), ...value };
    }
  }
  return next as unknown as Settings;
}
