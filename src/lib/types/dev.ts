/** Developer-mode simulation + observability contract. Never shown to normal users. */

import type { PermissionKind } from "./errors";

export type DevSimulation =
  | { type: "question"; text: string; speaker?: string }
  | { type: "coding_problem"; text: string }
  | { type: "transcript"; segments: Array<{ text: string; speaker?: string; source?: "microphone" | "system" }> }
  | { type: "screen_capture"; fixture?: string }
  | { type: "permission_error"; permission: PermissionKind }
  | { type: "ai_latency"; ms: number }
  | { type: "ai_failure"; code?: string }
  | { type: "clear" };

export interface LatencyMetrics {
  captureMs?: number;
  ocrMs?: number;
  accessibilityMs?: number;
  transcriptMs?: number;
  contextAssemblyMs?: number;
  modelMs?: number;
  timeToFirstTokenMs?: number;
  totalResponseMs?: number;
  inputTokens?: number;
  outputTokens?: number;
  updatedAt: string;
}

export interface DevInfo {
  version: string;
  buildProfile: "debug" | "release";
  helperVersion?: string;
  helperRunning: boolean;
  agentSidecarAvailable: boolean;
  dbPath: string;
  logPath: string;
  mockTransport: boolean;
}
