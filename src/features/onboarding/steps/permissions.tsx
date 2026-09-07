import { Accessibility, AudioWaveform, Mic, MonitorUp, type LucideIcon } from "lucide-react";
import { useEffect, useState } from "react";

import { Button } from "@/components/ui/Button";
import { cn } from "@/lib/utils/cn";
import type { PermissionKind } from "@/lib/types";
import { usePermissionsStore } from "@/stores/permissionsStore";
import type { StepProps } from "../OnboardingFlow";

interface PermissionCopy {
  kind: PermissionKind;
  label: string;
  icon: LucideIcon;
  what: string;
  why: string;
  access: string;
}

const PERMISSIONS: PermissionCopy[] = [
  {
    kind: "screenRecording",
    label: "Screen Recording",
    icon: MonitorUp,
    what: "Bluey needs permission to capture your screen.",
    why: "So it can answer questions about what you're looking at — code, documents, slides.",
    access: "Frames are processed on this Mac and only leave it when you send a question to your AI provider.",
  },
  {
    kind: "microphone",
    label: "Microphone",
    icon: Mic,
    what: "Bluey needs your microphone during audio sessions.",
    why: "To transcribe your side of the conversation in interviews and meetings.",
    access: "Raw audio is never stored by default — only the transcript, and only if you keep transcripts on.",
  },
  {
    kind: "accessibility",
    label: "Accessibility",
    icon: Accessibility,
    what: "Bluey reads on-screen structure through Accessibility.",
    why: "Window titles and focused text give faster, cheaper context than full screenshots.",
    access: "Only the frontmost app's visible structure — never keystrokes or passwords fields.",
  },
  {
    kind: "speechRecognition",
    label: "Speech Recognition",
    icon: AudioWaveform,
    what: "Bluey uses Apple's on-device speech recognition.",
    why: "For private, fast transcription without sending audio to the cloud.",
    access: "Transcription runs on this Mac when the Apple provider is selected.",
  },
];

export function PermissionsStep({ onReady }: StepProps) {
  const permissions = usePermissionsStore((s) => s.permissions);
  const request = usePermissionsStore((s) => s.request);
  const openSystemSettings = usePermissionsStore((s) => s.openSystemSettings);
  const [sub, setSub] = useState(0);

  const current = PERMISSIONS[Math.min(sub, PERMISSIONS.length - 1)] ?? PERMISSIONS[0]!;
  const status = permissions?.[current.kind] ?? "unknown";
  const granted = status === "granted";

  // The step's Continue only unlocks on the last permission screen.
  useEffect(() => {
    onReady(sub >= PERMISSIONS.length - 1);
  }, [sub, onReady]);

  const Icon = current.icon;

  return (
    <div className="flex w-full max-w-[480px] flex-col items-center text-center">
      <div className="mb-5 flex size-14 items-center justify-center rounded-card bg-bg-tile">
        <Icon className="size-6 text-fg" strokeWidth={1.7} aria-hidden />
      </div>
      <h1 className="text-[24px] font-semibold text-fg">{current.label}</h1>

      <dl className="mt-5 flex w-full flex-col gap-3 text-left">
        {(
          [
            ["What Bluey needs", current.what],
            ["Why", current.why],
            ["What it can access", current.access],
          ] as Array<[string, string]>
        ).map(([label, text]) => (
          <div key={label} className="rounded-card border border-border bg-bg-elevated px-4 py-3">
            <dt className="text-[11.5px] font-medium uppercase tracking-wide text-fg-subtle">{label}</dt>
            <dd className="m-0 mt-0.5 text-[13.5px] leading-relaxed text-fg-muted">{text}</dd>
          </div>
        ))}
      </dl>

      <div className="mt-5 flex items-center gap-2.5">
        <span
          className={cn(
            "rounded-full px-2.5 py-1 text-[11.5px] font-medium",
            granted ? "bg-success/15 text-success" : "bg-bg-tile text-fg-muted",
          )}
        >
          {granted ? "Granted" : status === "denied" ? "Denied" : "Not granted yet"}
        </span>
        {!granted ? (
          <>
            <Button variant="primary" size="sm" onClick={() => void request(current.kind)}>
              Grant access
            </Button>
            <Button variant="secondary" size="sm" onClick={() => void openSystemSettings(current.kind)}>
              Open System Settings
            </Button>
          </>
        ) : null}
        {sub < PERMISSIONS.length - 1 ? (
          <Button variant="ghost" size="sm" onClick={() => setSub((s) => s + 1)}>
            {granted ? "Next permission" : "Skip for now"}
          </Button>
        ) : null}
      </div>

      <div className="mt-4 flex items-center gap-1.5" aria-hidden>
        {PERMISSIONS.map((p, i) => (
          <span key={p.kind} className={cn("h-1 w-5 rounded-full", i <= sub ? "bg-fg-muted" : "bg-bg-tile")} />
        ))}
      </div>
    </div>
  );
}
