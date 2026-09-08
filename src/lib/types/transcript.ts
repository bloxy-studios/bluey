/** Transcript + audio pipeline types (mirrors `bluey_core::types::transcript`). */

import type { BlueyError } from "./errors";

export type AudioSource = "microphone" | "system";

export interface TranscriptSegment {
  id: string;
  sessionId?: string;
  /** "You", "Interviewer", "Speaker 2"... Never certain — see `speakerConfidence`. */
  speaker?: string;
  speakerConfidence?: number;
  source: AudioSource;
  text: string;
  /** Milliseconds since audio session start. */
  startTime: number;
  endTime: number;
  confidence?: number;
  finalized: boolean;
  language?: string;
  createdAt: string;
}

export interface AudioDevice {
  id: string;
  name: string;
  isDefault: boolean;
  kind: "input" | "output";
}

/** `gemini_live` is the default (Gemini Live API); it falls back to `apple` without a Google key. */
export type TranscriptionProviderKind = "apple" | "gemini_live" | "cloud_realtime" | "mock";

export interface AudioSessionConfig {
  microphone: { enabled: boolean; deviceId?: string };
  systemAudio: { enabled: boolean };
  transcription: {
    provider: TranscriptionProviderKind;
    language: "auto" | string;
    speakerIdentification: boolean;
  };
  vad: { enabled: boolean; sensitivity: "low" | "medium" | "high" };
  /** Keep raw audio according to privacy settings. */
  retainRawAudio: "never" | "until_session_end" | "custom";
}

export type AudioSessionState = "stopped" | "starting" | "running" | "paused" | "error";

export interface AudioStatus {
  state: AudioSessionState;
  microphoneActive: boolean;
  systemAudioActive: boolean;
  provider?: TranscriptionProviderKind;
  currentInputDevice?: AudioDevice;
  startedAt?: string;
  /** Rolling RMS levels 0..1 for UI meters. */
  levels?: { microphone: number; system: number };
  error?: BlueyError;
}

export type DetectedEventType =
  | "question"
  | "behavioral_question"
  | "technical_question"
  | "coding_problem"
  | "objection"
  | "buying_signal"
  | "pricing_concern"
  | "competitor_mention"
  | "decision"
  | "action_item"
  | "topic_change"
  | "important_statement"
  | "follow_up";

export interface DetectedEvent {
  id: string;
  type: DetectedEventType;
  confidence: number;
  requiresResponse: boolean;
  /** The text that triggered the detection. */
  text: string;
  segmentIds: string[];
  speaker?: string;
  detectedAt: string;
}
