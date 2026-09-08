/** Test data builders for settings, modes, snapshots and segments. */

import type {
  BlueyMode,
  BlueyResponse,
  ContextSnapshot,
  Session,
  Settings,
  TranscriptSegment,
} from "@/lib/types";

export function makeSettings(
  overrides: {
    ai?: Partial<Settings["ai"]>;
    general?: Partial<Settings["general"]>;
    screen?: Partial<Settings["screen"]>;
    privacy?: Partial<Settings["privacy"]>;
  } = {},
): Settings {
  return {
    version: 1,
    general: {
      blueyName: "Bluey",
      launchAtLogin: false,
      defaultModeId: "general",
      onboardingCompleted: true,
      developerMode: true,
      outputLanguage: "auto",
      ...overrides.general,
    },
    appearance: {
      theme: "system",
      opacity: 0.95,
      width: 460,
      blur: true,
      fontSize: "medium",
      alwaysOnTop: true,
      density: "comfortable",
      position: "remember",
      followActiveDisplay: true,
      reducedMotion: "system",
    },
    audio: {
      source: "both",
      transcriptionLanguage: "auto",
      speakerIdentification: true,
      transcriptionProvider: "mock",
      vadSensitivity: "medium",
    },
    screen: {
      captureTarget: "display",
      observation: "manual",
      observationIntervalMs: 2000,
      preferredDisplay: "active",
      ocrLevel: "accurate",
      ocrLanguages: ["en"],
      maxImageDimension: 1600,
      ...overrides.screen,
    },
    ai: {
      providers: [],
      models: {
        default: { providerId: "mock", model: "mock-default" },
        fast: null,
        reasoning: null,
        vision: null,
        research: null,
        transcription: null,
        embedding: null,
      },
      responseLength: "balanced",
      responseTone: "natural",
      researchEnabled: false,
      deepResearchEnabled: false,
      embeddingsEnabled: false,
      proactivePreparation: true,
      contextTokenBudget: 8000,
      embeddingDimensions: 768,
      researchBackend: "gemini",
      ...overrides.ai,
    },
    privacy: {
      displayMode: "standard",
      storeSessionHistory: true,
      storeScreenshots: false,
      storeTranscripts: true,
      storeRawAudio: "never",
      cloudAiEnabled: true,
      debugLogTranscripts: false,
      ...overrides.privacy,
    },
    shortcuts: [],
    advanced: { logLevel: "info", showDevOverlay: false, helperRestartOnCrash: true },
  };
}

export function makeMode(overrides: Partial<BlueyMode> = {}): BlueyMode {
  return {
    id: "general",
    name: "General",
    description: "General assistant mode",
    icon: "sparkles",
    systemInstructions: "Help the user with whatever is in front of them.",
    responseSchema: "answer",
    preferredLatency: "fast",
    contextRequirements: ["transcript", "session_memory"],
    builtIn: true,
    attachedDocumentIds: [],
    createdAt: "2026-09-07T08:00:00.000Z",
    updatedAt: "2026-09-07T08:00:00.000Z",
    ...overrides,
  };
}

export function makeSnapshot(overrides: Partial<ContextSnapshot> = {}): ContextSnapshot {
  return {
    timestamp: "2026-09-07T09:00:00.000Z",
    ...overrides,
  };
}

export function makeSegment(overrides: Partial<TranscriptSegment> = {}): TranscriptSegment {
  return {
    id: "seg_1",
    source: "system",
    text: "Hello there.",
    startTime: 0,
    endTime: 1500,
    finalized: true,
    createdAt: "2026-09-07T09:00:01.500Z",
    ...overrides,
  };
}

export function makeSession(overrides: Partial<Session> = {}): Session {
  return {
    id: "ses_1",
    modeId: "general",
    startedAt: "2026-09-07T08:55:00.000Z",
    status: "active",
    ...overrides,
  };
}

export function makeResponse(overrides: Partial<BlueyResponse> = {}): BlueyResponse {
  return {
    id: "resp_prev_1",
    requestId: "req_prev_1",
    modeId: "general",
    type: "answer",
    content: "Earlier answer content.",
    createdAt: "2026-09-07T08:58:00.000Z",
    ...overrides,
  };
}
