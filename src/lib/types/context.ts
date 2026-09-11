/**
 * Context Snapshot: the primary input to the AI system.
 * Mirrors `bluey_core::types::context`. Rust assembles native parts (screen, OCR,
 * accessibility, transcript, active app); TS enriches with session/user/mode context.
 */

import type { SnapshotTrace } from "./latency";
import type { DocumentKind, DocumentScope } from "./documents";
import type { BlueyMode, ResponseStyle } from "./mode";
import type { SessionEvent } from "./session";
import type { TranscriptSegment } from "./transcript";

export interface BoundingBox {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface ApplicationContext {
  name: string;
  bundleId?: string;
  pid?: number;
}

export interface WindowContext {
  title?: string;
  windowId?: number;
  bounds?: BoundingBox;
  /** e.g. "chrome", "vscode", "terminal", "zoom", "google-meet", "teams", "slack", "generic" */
  adapter?: string;
  /** Adapter-derived hints (e.g. { url, meetingDetected: true }). */
  hints?: Record<string, unknown>;
}

export interface DisplayInfo {
  id: string;
  name: string;
  width: number;
  height: number;
  x: number;
  y: number;
  scaleFactor: number;
  isMain: boolean;
}

export interface ScreenFrame {
  /** Stable id for this frame (used to fetch/delete the cached image). */
  id: string;
  /** Base64 JPEG/PNG, present only when the caller asked for inline data. */
  image?: string;
  mimeType: "image/jpeg" | "image/png" | "image/webp";
  /** Absolute path of a temporary file holding the image, if not inlined. */
  path?: string;
  width: number;
  height: number;
  displayId?: string;
  scaleFactor: number;
  capturedAt: string;
  /** Perceptual hash used for change detection. */
  hash?: string;
  /** False when change detection decided the screen did not materially change. */
  changed: boolean;
  target: CaptureTarget;
  durationMs?: number;
}

export type CaptureTarget =
  | { type: "display"; displayId?: string }
  | { type: "window"; windowId?: number }
  | { type: "region"; displayId?: string; rect: BoundingBox }
  | { type: "active_window" };

export interface CaptureOptions {
  target?: CaptureTarget;
  format?: "jpeg" | "png";
  /** 0..1 JPEG quality. */
  quality?: number;
  /** Downscale so the longest side is at most this (default 1600). */
  maxDimension?: number;
  /** Return inline base64 instead of a temp path. */
  inline?: boolean;
  /** Skip work when the hash matches the previous frame. */
  changeDetection?: boolean;
}

export interface OCRBlock {
  text: string;
  confidence: number;
  boundingBox: BoundingBox;
}

export interface OCRContext {
  blocks: OCRBlock[];
  /** Joined text in reading order. */
  text: string;
  level: "fast" | "accurate";
  languages: string[];
  durationMs: number;
  frameId?: string;
}

export interface AccessibilityElement {
  role: string;
  label?: string;
  value?: string;
  title?: string;
  description?: string;
  position?: { x: number; y: number };
  size?: { width: number; height: number };
  actions?: string[];
  focused?: boolean;
  depth: number;
}

export interface AccessibilityContext {
  application: ApplicationContext;
  window?: WindowContext;
  focusedElement?: AccessibilityElement;
  /** Relevant elements only (visible text, inputs, buttons, code editors) — depth/size limited. */
  elements: AccessibilityElement[];
  selectedText?: string;
  /** Flattened visible text, deduplicated. */
  visibleText: string;
  truncated: boolean;
  capturedAt: string;
}

export interface TranscriptContext {
  segments: TranscriptSegment[];
  /** Rolling summary of older transcript, if any. */
  earlierSummary?: string;
  windowSeconds: number;
}

export interface SessionContext {
  sessionId: string;
  modeId: string;
  startedAt: string;
  /** Recent responses (id + title + short content) for continuity. */
  recentResponses: Array<{ id: string; title?: string; content: string; createdAt: string }>;
  recentEvents: SessionEvent[];
  notes: string[];
  /** Session-attached document ids. */
  documentIds: string[];
}

export interface UserContext {
  /** Retrieved chunks from "My Context" and session documents. */
  chunks: RetrievedChunk[];
  /** Personal instructions the user typed (always small). */
  personalInstructions?: string;
  displayName?: string;
}

export interface ModeContext {
  mode: BlueyMode;
  responseStyle: ResponseStyle;
}

export interface RetrievedChunk {
  chunkId: string;
  documentId: string;
  documentTitle: string;
  documentKind: DocumentKind;
  content: string;
  score: number;
  scope: DocumentScope;
}

export interface ContextSnapshot {
  timestamp: string;
  activeApplication?: ApplicationContext;
  activeWindow?: WindowContext;
  screen?: {
    image?: string;
    mimeType?: string;
    width: number;
    height: number;
    displayId?: string;
    frameId?: string;
  };
  ocr?: OCRContext;
  accessibility?: AccessibilityContext;
  transcript?: TranscriptContext;
  session?: SessionContext;
  userContext?: UserContext;
  mode?: ModeContext;
  /** Explicit user question typed into the HUD (highest priority). */
  userInstruction?: string;
  /** Native assembly timings (ms). */
  timings?: Partial<Record<"capture" | "ocr" | "accessibility" | "transcript" | "assembly", number>>;
  /** What the native builder observed on the Rust clock (ADR 0010 §2); the engine anchors on `replyMs`. */
  trace?: SnapshotTrace;
}

/** Options for the Rust fast-path `context_build_snapshot`. */
export interface SnapshotOptions {
  includeScreen: boolean;
  includeOcr: boolean;
  includeAccessibility: boolean;
  includeTranscript: boolean;
  transcriptWindowSeconds?: number;
  capture?: CaptureOptions;
  ocrLevel?: "fast" | "accurate";
  /** Inline the image (base64) so it can be sent to a vision model. */
  inlineImage?: boolean;
}

export type ContextSource =
  | "user_instruction"
  | "screen"
  | "ocr"
  | "accessibility"
  | "transcript"
  | "transcript_old"
  | "resume"
  | "job_description"
  | "document"
  | "session_memory"
  | "personal_instructions";

export interface ContextItem {
  source: ContextSource;
  content: string;
  /** 0..1 relevance to the current task. */
  relevance: number;
  /** Estimated tokens. */
  tokens: number;
  /** Optional origin identifiers for traceability. */
  ref?: string;
}
