/**
 * Fixture data for the MockTransport (Developer Mode backend).
 * Names / descriptions / icons / groups of the built-in modes follow
 * docs/DESIGN.md and `BUILT_IN_MODE_IDS` in src/lib/types/mode.ts.
 */

import type {
  AudioDevice,
  BlueyDocument,
  BlueyMode,
  BlueyResponse,
  DisplayInfo,
  ModelRoleAssignments,
  Session,
  SessionEvent,
  SessionNote,
  SessionSummary,
  Settings,
  ShortcutBinding,
  AccountIdentity,
  CatalogModel,
  ProviderAccount,
  ProviderModelCatalog,
} from "../../types";
import { applyPresets, GEMINI_PRESET } from "../../ai/provider-presets";

const NOW = () => new Date().toISOString();
const daysAgo = (days: number, offsetMinutes = 0) =>
  new Date(Date.now() - days * 86_400_000 + offsetMinutes * 60_000).toISOString();

/** 1×1 dark PNG (base64, no data: prefix) used for capture fixtures. */
export const FIXTURE_PNG_BASE64 =
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

export const FIXTURE_DISPLAYS: DisplayInfo[] = [
  {
    id: "display-1",
    name: "Built-in Retina Display",
    width: 1512,
    height: 982,
    x: 0,
    y: 0,
    scaleFactor: 2,
    isMain: true,
  },
  {
    id: "display-2",
    name: "Studio Display",
    width: 2560,
    height: 1440,
    x: 1512,
    y: 0,
    scaleFactor: 2,
    isMain: false,
  },
];

export const FIXTURE_AUDIO_DEVICES: AudioDevice[] = [
  { id: "mic-builtin", name: "MacBook Pro Microphone", isDefault: true, kind: "input" },
  { id: "mic-airpods", name: "AirPods Pro", isDefault: false, kind: "input" },
  { id: "out-blackhole", name: "BlackHole 2ch", isDefault: false, kind: "output" },
];

export const CODING_PROBLEM_OCR = `Two Sum

Given an array of integers nums and an integer target, return indices of the
two numbers such that they add up to target.

You may assume that each input would have exactly one solution, and you may
not use the same element twice. You can return the answer in any order.

Example 1:
Input: nums = [2,7,11,15], target = 9
Output: [0,1]
Explanation: Because nums[0] + nums[1] == 9, we return [0, 1].

Example 2:
Input: nums = [3,2,4], target = 6
Output: [1,2]

Constraints:
2 <= nums.length <= 10^4
-10^9 <= nums[i] <= 10^9
Only one valid answer exists.

Follow-up: Can you come up with an algorithm that is less than O(n^2) time complexity?`;

export const CANNED_ANSWER_MARKDOWN = `Use a **hash map** to find the complement of each number in one pass.

Walk the array once. For each value, check whether \`target - value\` was already seen; if it was, you have your pair. Otherwise store the value with its index and continue.

- Time: **O(n)** — each element is visited once
- Space: **O(n)** — the map holds at most every element
- Handles duplicates because the complement is checked *before* inserting the current value

\`\`\`python
def two_sum(nums: list[int], target: int) -> list[int]:
    seen: dict[int, int] = {}
    for i, value in enumerate(nums):
        complement = target - value
        if complement in seen:
            return [seen[complement], i]
        seen[value] = i
    return []
\`\`\`

Mention the brute-force baseline (\`O(n^2)\` nested loops) first, then explain why the hash-map trade of memory for time is worth it. If the interviewer asks about a sorted input, switch to the two-pointer technique instead.`;

export const CANNED_FOLLOW_UP_MARKDOWN = `Yes — when the array is **already sorted** you can drop the extra memory.

Use two pointers, one at each end, and move them inward based on the current sum:

\`\`\`python
def two_sum_sorted(nums: list[int], target: int) -> list[int]:
    lo, hi = 0, len(nums) - 1
    while lo < hi:
        total = nums[lo] + nums[hi]
        if total == target:
            return [lo, hi]
        if total < target:
            lo += 1
        else:
            hi -= 1
    return []
\`\`\`

- Time stays **O(n)**, but space drops to **O(1)**
- Sorting an unsorted input first costs \`O(n log n)\`, so the hash map remains better for the general case`;

/* ── Shortcuts ─────────────────────────────────────────────────────────── */

const shortcut = (
  id: ShortcutBinding["id"],
  label: string,
  group: ShortcutBinding["group"],
  accelerator: string,
): ShortcutBinding => ({ id, label, group, accelerator, defaultAccelerator: accelerator, enabled: true });

export const DEFAULT_SHORTCUTS: ShortcutBinding[] = [
  shortcut("toggle_panel", "Toggle visibility of Bluey", "general", "CmdOrCtrl+Backslash"),
  shortcut("capture_analyze", "Ask Bluey about your screen or audio", "general", "CmdOrCtrl+Enter"),
  shortcut("generate_response", "Generate a suggested response", "general", "CmdOrCtrl+Shift+Enter"),
  shortcut("new_chat", "Start a new chat", "general", "CmdOrCtrl+R"),
  shortcut("open_settings", "Open Bluey settings", "general", "CmdOrCtrl+Comma"),
  shortcut("toggle_listening", "Start or stop a Bluey session", "general", "CmdOrCtrl+Shift+Backslash"),
  shortcut("move_up", "Move the window position up", "window", "CmdOrCtrl+Up"),
  shortcut("move_down", "Move the window position down", "window", "CmdOrCtrl+Down"),
  shortcut("move_left", "Move the window position left", "window", "CmdOrCtrl+Left"),
  shortcut("move_right", "Move the window position right", "window", "CmdOrCtrl+Right"),
  shortcut("scroll_up", "Scroll the response window up", "scroll", "CmdOrCtrl+Shift+Up"),
  shortcut("scroll_down", "Scroll the response window down", "scroll", "CmdOrCtrl+Shift+Down"),
];

/* ── Built-in modes ────────────────────────────────────────────────────── */

interface ModeSeed {
  id: string;
  name: string;
  description: string;
  icon: string;
  group?: string;
  schema: BlueyMode["responseSchema"];
  latency: BlueyMode["preferredLatency"];
  context: BlueyMode["contextRequirements"];
  instructions: string;
}

const MODE_SEEDS: ModeSeed[] = [
  {
    id: "general",
    name: "General",
    description:
      "The default mode. No custom prompt, summary template, or attached files. Bluey uses its baseline behavior. Set this active to clear any mode you have selected.",
    icon: "file-text",
    schema: "answer",
    latency: "fast",
    context: ["screen", "transcript"],
    instructions: "",
  },
  {
    id: "interview",
    name: "Interview",
    description:
      "General job-interview support across behavioral, technical, product and role-fit questions.",
    icon: "graduation-cap",
    group: "Looking for work",
    schema: "suggested-response",
    latency: "fast",
    context: ["transcript", "resume", "job_description", "documents"],
    instructions: `I am a candidate in a job interview. Help me perform well across behavioral, technical, product, role-fit, and follow-up questions.

Use the job description, resume, notes, and any attached files as ground truth when available. Do not fabricate experience, credentials, companies, numbers, or project details. If context is missing, give me an answer structure that I can quickly fill in.

For behavioral questions, help me choose a strong example and shape it with situation, task, action, and result. For technical or role-specific questions, explain the concept clearly, state tradeoffs, and answer at the right depth for an interview.`,
  },
  {
    id: "behavioral-interview",
    name: "Behavioral Interview",
    description: "STAR-structured answers grounded in your real experience.",
    icon: "message-square",
    group: "Looking for work",
    schema: "behavioral",
    latency: "fast",
    context: ["transcript", "resume", "documents"],
    instructions: `I am in a behavioral interview. For every question, propose the strongest matching story from my resume and notes and shape it as Situation, Task, Action, Result — ending with what I learned.

Keep answers speakable in under two minutes. Never invent employers, dates, metrics, or outcomes; if my materials lack a good example, say so and give me a structure to fill in live.`,
  },
  {
    id: "coding-interview",
    name: "Coding Interview",
    description: "Restate the problem, find the approach, then write clean, correct code.",
    icon: "code",
    group: "Looking for work",
    schema: "coding",
    latency: "balanced",
    context: ["screen", "accessibility", "transcript"],
    instructions: `I am in a coding interview. Help me solve the problem while explaining my thinking clearly to the interviewer.

First help me restate the problem, identify inputs and outputs, clarify constraints, and surface edge cases. If the prompt is ambiguous, suggest the best clarification questions before jumping into code.

Guide me toward a correct approach, then improve it if there is a more efficient algorithm. Explain the tradeoffs between brute force and optimized solutions, including time and space complexity.`,
  },
  {
    id: "system-design",
    name: "System Design",
    description: "Requirements, high-level architecture, deep dives and tradeoffs.",
    icon: "box",
    group: "Looking for work",
    schema: "system-design",
    latency: "deep",
    context: ["screen", "transcript"],
    instructions: `I am in a system design interview. Help me run a structured discussion: functional and non-functional requirements, capacity estimates, API sketch, high-level architecture, data model, then deep dives.

Always name the tradeoffs (consistency vs availability, latency vs cost, build vs buy) and propose sensible defaults. Where a diagram helps, describe the components and their connections clearly.`,
  },
  {
    id: "case-interview",
    name: "Case Interview",
    description: "Structured frameworks, market sizing and crisp synthesis.",
    icon: "presentation",
    group: "Looking for work",
    schema: "case",
    latency: "balanced",
    context: ["transcript", "documents"],
    instructions: `I am in a consulting case interview. Help me structure the problem with a MECE framework, do market-sizing math out loud with round numbers, and synthesize with a clear recommendation up front, supported by two or three drivers and the key risks.`,
  },
  {
    id: "sales",
    name: "Sales",
    description: "Live objection handling, discovery questions and next steps.",
    icon: "store",
    group: "At work",
    schema: "sales",
    latency: "ultra-fast",
    context: ["transcript", "documents", "session_memory"],
    instructions: `I am on a sales call. Listen for objections, buying signals, pricing concerns, and competitor mentions. Suggest short, natural responses I can say directly — acknowledge, reframe, then advance.

Ground claims in my attached product notes; never invent pricing, features, or customer names. Always keep an eye on securing the concrete next step.`,
  },
  {
    id: "recruiting",
    name: "Recruiting",
    description: "Structured candidate screens with follow-up probes.",
    icon: "briefcase",
    group: "At work",
    schema: "recruiting",
    latency: "fast",
    context: ["transcript", "job_description", "documents"],
    instructions: `I am the recruiter interviewing a candidate. Help me probe the candidate's answers against the role requirements, suggest sharp follow-up questions, flag inconsistencies or gaps to explore, and capture structured notes on strengths, risks, and motivation.`,
  },
  {
    id: "team-meeting",
    name: "Team Meeting",
    description: "Decisions, action items and open questions captured as you go.",
    icon: "video",
    group: "At work",
    schema: "meeting",
    latency: "fast",
    context: ["transcript", "session_memory"],
    instructions: `I am in a team meeting. Track decisions, owners, action items, and open questions as they happen. When I ask for help, answer with the shared context of the meeting so far, and keep suggestions short enough to say out loud.`,
  },
  {
    id: "lecture",
    name: "Lecture",
    description: "Follow along, capture key concepts and build a study guide.",
    icon: "book-open",
    group: "At work",
    schema: "lecture",
    latency: "balanced",
    context: ["transcript", "screen", "session_memory"],
    instructions: `I am attending a lecture. Capture the key concepts, definitions, and examples as they come up. When I ask a question, explain at the level of the course, connect it to what was covered earlier, and note anything the lecturer flagged as exam-relevant.`,
  },
];

export function createBuiltInModes(): BlueyMode[] {
  const createdAt = daysAgo(30);
  return MODE_SEEDS.map((seed) => ({
    id: seed.id,
    name: seed.name,
    description: seed.description,
    icon: seed.icon,
    systemInstructions: seed.instructions,
    responseSchema: seed.schema,
    preferredLatency: seed.latency,
    contextRequirements: seed.context,
    builtIn: true,
    group: seed.group,
    attachedDocumentIds: [],
    createdAt,
    updatedAt: createdAt,
  }));
}

/* ── Settings ──────────────────────────────────────────────────────────── */

const NO_ASSIGNMENTS: ModelRoleAssignments = {
  default: null,
  fast: null,
  reasoning: null,
  vision: null,
  research: null,
  transcription: null,
  embedding: null,
};

export function createDefaultSettings(): Settings {
  return {
    version: 1,
    general: {
      blueyName: "Bluey",
      launchAtLogin: false,
      defaultModeId: "general",
      onboardingCompleted: true,
      developerMode: true,
      outputLanguage: "en", // Rust default (`GeneralSettings::default`)
    },
    appearance: {
      theme: "system",
      opacity: 1,
      width: 690,
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
      microphoneDeviceId: "mic-builtin",
      transcriptionLanguage: "auto",
      speakerIdentification: true,
      transcriptionProvider: "gemini_live",
      vadSensitivity: "medium",
    },
    screen: {
      captureTarget: "display",
      observation: "manual",
      observationIntervalMs: 5000,
      preferredDisplay: "active",
      ocrLevel: "accurate",
      ocrLanguages: ["en-US"],
      maxImageDimension: 1600,
    },
    ai: {
      providers: [
        {
          id: "gemini",
          kind: "google_gemini",
          name: "Google Gemini",
          baseUrl: "",
          enabled: true,
          hasApiKey: true,
        },
        {
          id: "azure-foundry",
          kind: "azure_foundry",
          name: "Azure Foundry",
          baseUrl: "https://bluey-dev.openai.azure.com",
          deployments: { "gpt-6-astra": "astra-prod", "gpt-5.6-luna": "luna-prod" },
          enabled: true,
          hasApiKey: true,
        },
        {
          id: "anthropic",
          kind: "anthropic",
          name: "Claude (Foundry)",
          baseUrl: "https://bluey-dev.services.ai.azure.com/anthropic",
          enabled: true,
          hasApiKey: false,
        },
      ],
      // Gemini is the default provider (ADR 0007): every role runs its recommended model, exactly as
      // `ai_apply_provider_presets` would leave it. Foundry stays a keyed alternate to switch to.
      models: applyPresets(NO_ASSIGNMENTS, GEMINI_PRESET, true).models,
      responseLength: "balanced",
      responseTone: "natural",
      researchEnabled: false,
      deepResearchEnabled: false,
      embeddingsEnabled: true,
      proactivePreparation: true,
      contextTokenBudget: 24_000,
      bootstrapProvider: "gemini",
      embeddingDimensions: 768,
      researchBackend: "gemini",
    },
    privacy: {
      displayMode: "standard",
      storeSessionHistory: true,
      storeScreenshots: false,
      storeTranscripts: true,
      storeRawAudio: "never",
      cloudAiEnabled: true,
      debugLogTranscripts: false,
    },
    shortcuts: DEFAULT_SHORTCUTS.map((s) => ({ ...s })),
    advanced: {
      logLevel: "info",
      showDevOverlay: false,
      helperRestartOnCrash: true,
    },
    experimental: {
      subscriptionAccounts: true,
      acceptedAccountConsents: [],
    },
  };
}

/* ── Seed sessions / documents ─────────────────────────────────────────── */

export interface SeedData {
  sessions: Session[];
  events: SessionEvent[];
  notes: SessionNote[];
  summaries: SessionSummary[];
  responses: BlueyResponse[];
  documents: BlueyDocument[];
}

export function createSeedData(): SeedData {
  const s1: Session = {
    id: "session-coding-1",
    modeId: "coding-interview",
    startedAt: daysAgo(1),
    endedAt: daysAgo(1, 42),
    status: "completed",
    title: "Coding interview practice",
  };
  const s2: Session = {
    id: "session-meeting-1",
    modeId: "team-meeting",
    startedAt: daysAgo(3),
    endedAt: daysAgo(3, 55),
    status: "completed",
    title: "Weekly team sync",
  };

  const ev = (
    id: string,
    sessionId: string,
    type: SessionEvent["type"],
    title: string,
    createdAt: string,
    detail?: string,
    refs?: Record<string, string>,
  ): SessionEvent => ({ id, sessionId, type, title, detail, refs, createdAt });

  const response1: BlueyResponse = {
    id: "response-two-sum",
    requestId: "req-two-sum",
    sessionId: s1.id,
    modeId: "coding-interview",
    type: "code",
    title: "Two Sum — hash map in one pass",
    content: CANNED_ANSWER_MARKDOWN,
    prompt: "Solve the problem on my screen",
    confidence: 0.93,
    metrics: {
      provider: "azure-foundry",
      model: "gpt-5.6-terra",
      timeToFirstTokenMs: 412,
      totalMs: 3480,
      outputTokens: 236,
    },
    createdAt: daysAgo(1, 12),
  };

  return {
    sessions: [s1, s2],
    events: [
      ev("ev-1", s1.id, "session_started", "Session started", s1.startedAt),
      ev(
        "ev-2",
        s1.id,
        "coding_problem_detected",
        "Coding problem detected",
        daysAgo(1, 10),
        "Two Sum — return indices of two numbers adding to target",
        { snapshotId: "frame-fixture" },
      ),
      ev(
        "ev-3",
        s1.id,
        "response_generated",
        "Response generated",
        daysAgo(1, 12),
        "Two Sum — hash map in one pass",
        { responseId: response1.id },
      ),
      ev(
        "ev-4",
        s1.id,
        "note_added",
        "Note added",
        daysAgo(1, 20),
        "Remember to talk through complexity before coding.",
      ),
      ev("ev-5", s1.id, "session_ended", "Session ended", s1.endedAt ?? s1.startedAt),
      ev("ev-6", s2.id, "session_started", "Session started", s2.startedAt),
      ev(
        "ev-7",
        s2.id,
        "decision_detected",
        "Decision",
        daysAgo(3, 18),
        "Ship the beta behind a feature flag on Thursday.",
      ),
      ev(
        "ev-8",
        s2.id,
        "action_item_detected",
        "Action item",
        daysAgo(3, 31),
        "Dana drafts the rollout announcement by Wednesday.",
      ),
      ev("ev-9", s2.id, "session_ended", "Session ended", s2.endedAt ?? s2.startedAt),
    ],
    notes: [
      {
        id: "note-1",
        sessionId: s1.id,
        content: "Remember to talk through complexity before coding.",
        createdAt: daysAgo(1, 20),
        updatedAt: daysAgo(1, 20),
      },
    ],
    summaries: [
      {
        id: "summary-1",
        sessionId: s1.id,
        modeId: "coding-interview",
        overview:
          "45-minute mock coding interview covering the Two Sum problem and a sorted-input follow-up.",
        topics: ["Hash maps", "Two-pointer technique", "Complexity analysis"],
        questions: ["Solve Two Sum", "Optimize for a sorted array"],
        answers: ["One-pass hash map, O(n) time / O(n) space", "Two pointers, O(n) time / O(1) space"],
        decisions: [],
        actionItems: ["Practice explaining space/time tradeoffs out loud"],
        openItems: ["Revisit three-sum variants"],
        improvements: ["State the brute-force baseline before the optimal approach"],
        createdAt: daysAgo(1, 45),
      },
    ],
    responses: [response1],
    documents: [
      {
        id: "doc-resume",
        title: "Resume — Jordan Lee.pdf",
        kind: "resume",
        format: "pdf",
        scope: "global",
        sourcePath: "/Users/jordan/Documents/Resume — Jordan Lee.pdf",
        sizeBytes: 184_320,
        chunkCount: 14,
        indexStatus: "indexed",
        hasEmbeddings: true,
        createdAt: daysAgo(14),
        updatedAt: daysAgo(14),
      },
      {
        id: "doc-jd",
        title: "Staff Engineer — Job description.md",
        kind: "job_description",
        format: "md",
        scope: "mode",
        scopeId: "interview",
        sizeBytes: 6_212,
        chunkCount: 3,
        indexStatus: "indexed",
        hasEmbeddings: true,
        createdAt: daysAgo(7),
        updatedAt: daysAgo(7),
      },
    ],
  };
}

export const FIXTURE_MODELS_BY_KIND: Record<string, string[]> = {
  google_gemini: [
    "gemini-3.8-flash",
    "gemini-3.5-flash-lite",
    "gemini-3.5-flash",
    "gemini-3.5-transcribe",
    "gemini-3.5-transcribe-live",
    "gemini-embedding-2",
    "gemini-embedding-001",
  ],
  azure_foundry: [
    "gpt-6-astra",
    "gpt-5.6-sol",
    "gpt-5.6-terra",
    "gpt-5.6-luna",
    "gpt-4.1-mini",
    "MAI-Transcribe-1.5",
    "text-embedding-3-small",
  ],
  anthropic: ["claude-opus-5", "claude-sonnet-5", "claude-haiku-4-5"],
  openai_compatible: ["llama-3.3-70b-instruct", "qwen2.5-coder-32b"],
  mock: ["mock-fast", "mock-smart"],
};

export const FIXTURE_TIMESTAMP = NOW;

/* ── Subscription accounts (ADR 0009) ───────────────────────────────────── */

/** The three subscription providers as fresh, disconnected account cards. */
export function createMockAccounts(): ProviderAccount[] {
  return [
    {
      accountId: "chatgpt",
      providerId: "chatgpt",
      kind: "chatgpt_codex",
      method: "oauth_subscription",
      status: { state: "disconnected" },
      fingerprintVersion: "codex/0.154.0",
      fingerprintCapturedOn: "2026-09-11",
    },
    {
      accountId: "claude",
      providerId: "claude",
      kind: "claude_subscription",
      method: "oauth_subscription",
      status: { state: "disconnected" },
      fingerprintVersion: "claude_code/2.1.258",
      fingerprintCapturedOn: "2026-09-11",
    },
    {
      accountId: "antigravity",
      providerId: "antigravity",
      kind: "antigravity_google",
      method: "oauth_subscription",
      status: { state: "disconnected" },
      fingerprintVersion: "antigravity/2.12.2",
      fingerprintCapturedOn: "2026-09-11",
    },
  ];
}

/** Who the mock says is signed in, per provider. */
export const FIXTURE_ACCOUNT_IDENTITIES: Record<string, AccountIdentity> = {
  chatgpt: {
    email: "jordan@example.com",
    planTier: "plus",
    planLabel: "ChatGPT Plus",
    accountId: "1f2e3d4c-5b6a-4789-9abc-def012345678",
  },
  claude: {
    email: "jordan@example.com",
    displayName: "Jordan Lee",
    planTier: "default_claude_max_5x",
    planLabel: "Claude Max 5×",
    accountId: "org_9a8b7c6d",
  },
  antigravity: {
    email: "jordan@example.com",
    planTier: "g1-pro",
    planLabel: "Google AI Pro",
    projectId: "bluey-owner-4f2a",
  },
};

const codexModel = (id: string, label: string, suggestedRoles: CatalogModel["suggestedRoles"]): CatalogModel => ({
  id,
  label,
  capabilities: {
    vision: true,
    tools: true,
    reasoningLevels: ["low", "medium", "high", "xhigh"],
    streaming: true,
    contextWindow: 272_000,
  },
  suggestedRoles,
});

const claudeModel = (
  id: string,
  label: string,
  contextWindow: number,
  suggestedRoles: CatalogModel["suggestedRoles"],
): CatalogModel => ({
  id,
  label,
  capabilities: { vision: true, tools: true, reasoningLevels: ["low", "medium", "high", "xhigh", "max"], streaming: true, contextWindow },
  suggestedRoles,
});

const antigravityModel = (
  id: string,
  label: string,
  reasoningLevels: string[],
  suggestedRoles: CatalogModel["suggestedRoles"],
): CatalogModel => ({
  id,
  label,
  capabilities: { vision: true, tools: true, reasoningLevels, streaming: true, contextWindow: 1_048_576 },
  quotaPool: "antigravity",
  suggestedRoles,
});

/**
 * The models each subscription exposes in the mock — the catalogs
 * `docs/PROVIDER_ACCOUNTS.md` records for 2026-09-11.
 */
export function createFixtureCatalog(accountId: string, fetchedAt: string): ProviderModelCatalog {
  const models: CatalogModel[] =
    accountId === "chatgpt"
      ? [
          codexModel("gpt-6-astra", "GPT-6 Astra", ["default", "vision", "reasoning", "research"]),
          codexModel("gpt-5.6-terra", "GPT-5.6 Terra", ["default", "vision"]),
          codexModel("gpt-5.6-sol", "GPT-5.6 Sol", []),
          codexModel("gpt-5.6-luna", "GPT-5.6 Luna", ["fast"]),
          codexModel("gpt-5.5", "GPT-5.5", []),
        ]
      : accountId === "claude"
        ? [
            claudeModel("claude-sonnet-5", "Claude Sonnet 5", 1_000_000, ["default", "vision"]),
            claudeModel("claude-opus-5", "Claude Opus 5", 1_000_000, ["reasoning", "research"]),
            claudeModel("claude-haiku-4-5-20251001", "Claude Haiku 4.5", 200_000, ["fast"]),
            claudeModel("claude-fable-5-1", "Claude Fable 5.1", 1_000_000, []),
          ]
        : [
            antigravityModel("gemini-3.8-flash-high", "Gemini 3.8 Flash (High)", ["low", "medium", "high"], [
              "default",
              "vision",
              "research",
            ]),
            antigravityModel("gemini-3.1-flash-lite", "Gemini 3.1 Flash-Lite", ["minimal", "low", "medium", "high"], ["fast"]),
            antigravityModel("gemini-pro-agent", "Gemini 3.1 Pro (High)", ["low", "medium", "high"], []),
            antigravityModel("claude-sonnet-4-6", "Claude Sonnet 4.6", ["low", "medium", "high"], []),
            antigravityModel("claude-opus-4-6-thinking", "Claude Opus 4.6 (Thinking)", ["low", "medium", "high"], ["reasoning"]),
            antigravityModel("gpt-oss-120b-medium", "GPT-OSS 120B (Medium)", ["medium"], []),
          ];
  return {
    accountId,
    providerId: accountId,
    fetchedAt,
    source: { type: "fixture" },
    models,
  };
}
