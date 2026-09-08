/** Settings contract (mirrors `bluey_core::types::settings`). Persisted in SQLite `settings`. */

import type { AIProviderConfig, ModelRoleAssignments } from "./ai";
import type { ResponseLength, ResponseTone } from "./mode";
import type { TranscriptionProviderKind } from "./transcript";

export type Theme = "system" | "dark" | "light";
export type PanelDensity = "compact" | "comfortable";
export type FontSize = "small" | "medium" | "large";
export type PanelPositionPreference = "remember" | "center" | "top" | "bottom" | "left" | "right";
export type DisplayMode = "standard" | "privacy";
export type RawAudioRetention = "never" | "until_session_end" | "custom";
export type ObservationMode = "manual" | "smart";
export type CaptureTargetPreference = "display" | "active_window" | "region";
export type LogLevel = "error" | "warn" | "info" | "debug" | "trace";

export interface GeneralSettings {
  blueyName: string;
  launchAtLogin: boolean;
  defaultModeId: string;
  onboardingCompleted: boolean;
  developerMode: boolean;
  outputLanguage: string;
}

export interface AppearanceSettings {
  theme: Theme;
  /** 0.4 .. 1 */
  opacity: number;
  width: number;
  blur: boolean;
  fontSize: FontSize;
  alwaysOnTop: boolean;
  density: PanelDensity;
  position: PanelPositionPreference;
  followActiveDisplay: boolean;
  reducedMotion: "system" | "on" | "off";
}

export interface AudioSettings {
  source: "microphone" | "system" | "both";
  microphoneDeviceId?: string;
  transcriptionLanguage: "auto" | string;
  speakerIdentification: boolean;
  transcriptionProvider: TranscriptionProviderKind;
  vadSensitivity: "low" | "medium" | "high";
}

export interface ScreenSettings {
  captureTarget: CaptureTargetPreference;
  observation: ObservationMode;
  observationIntervalMs: number;
  preferredDisplay: "active" | string;
  ocrLevel: "fast" | "accurate";
  ocrLanguages: string[];
  maxImageDimension: number;
}

/** Which backend the research sidecar runs (`RESEARCH_BACKEND`). */
export type ResearchBackend = "gemini" | "claude";

export interface AISettings {
  providers: AIProviderConfig[];
  models: ModelRoleAssignments;
  responseLength: ResponseLength;
  responseTone: ResponseTone;
  researchEnabled: boolean;
  deepResearchEnabled: boolean;
  embeddingsEnabled: boolean;
  proactivePreparation: boolean;
  /** Max input tokens per request (token budget). */
  contextTokenBudget: number;
  /** Provider id the `.env` import nominated at boot (`BLUEY_AI_PROVIDER`). */
  bootstrapProvider?: string;
  /** MRL-truncated embedding size for gemini-embedding-2 (768 · 1536 · 3072). */
  embeddingDimensions: number;
  researchBackend: ResearchBackend;
}

export interface PrivacySettings {
  displayMode: DisplayMode;
  storeSessionHistory: boolean;
  storeScreenshots: boolean;
  storeTranscripts: boolean;
  storeRawAudio: RawAudioRetention;
  rawAudioRetentionMinutes?: number;
  cloudAiEnabled: boolean;
  /** Debug-only switches, default false. */
  debugLogTranscripts: boolean;
}

export interface ShortcutBinding {
  id: ShortcutId;
  label: string;
  group: "general" | "window" | "scroll";
  /** Tauri accelerator string, e.g. "CmdOrCtrl+Backslash". */
  accelerator: string;
  defaultAccelerator: string;
  enabled: boolean;
}

export type ShortcutId =
  | "toggle_panel"
  | "capture_analyze"
  | "generate_response"
  | "toggle_listening"
  | "new_chat"
  | "open_settings"
  | "move_up"
  | "move_down"
  | "move_left"
  | "move_right"
  | "scroll_up"
  | "scroll_down";

export interface AdvancedSettings {
  logLevel: LogLevel;
  showDevOverlay: boolean;
  helperRestartOnCrash: boolean;
}

export interface Settings {
  version: number;
  general: GeneralSettings;
  appearance: AppearanceSettings;
  audio: AudioSettings;
  screen: ScreenSettings;
  ai: AISettings;
  privacy: PrivacySettings;
  shortcuts: ShortcutBinding[];
  advanced: AdvancedSettings;
}

/** Deep partial patch applied by `settings_update`. */
export type SettingsPatch = {
  [K in keyof Settings]?: Settings[K] extends object ? Partial<Settings[K]> : Settings[K];
};

export interface PanelState {
  visible: boolean;
  pinned: boolean;
  expanded: boolean;
  x: number;
  y: number;
  width: number;
  height: number;
  displayId?: string;
  opacity: number;
}

export interface ShortcutConflict {
  accelerator: string;
  conflictsWith: "system" | "bluey" | "registration_failed";
  detail: string;
}
