import { Accessibility, AudioWaveform, Mic, MonitorUp, type LucideIcon } from "lucide-react";
import { useState } from "react";

import { Button } from "@/components/ui/Button";
import { SectionHeader } from "@/components/ui/SectionHeader";
import { Spinner } from "@/components/ui/Spinner";
import { bluey } from "@/lib/tauri/api";
import type { PermissionKind, PermissionStatus, SetupCheck } from "@/lib/types";
import { cn } from "@/lib/utils/cn";
import { usePermissionsStore } from "@/stores/permissionsStore";

const CARDS: Array<{ kind: PermissionKind; label: string; icon: LucideIcon; why: string }> = [
  { kind: "screenRecording", label: "Screen Recording", icon: MonitorUp, why: "Lets Bluey see your screen when you ask about it. Frames stay on this Mac unless you send a question." },
  { kind: "microphone", label: "Microphone", icon: Mic, why: "Needed to transcribe your voice during audio sessions." },
  { kind: "accessibility", label: "Accessibility", icon: Accessibility, why: "Reads on-screen structure (window titles, focused text) for faster, cheaper context than screenshots." },
  { kind: "speechRecognition", label: "Speech Recognition", icon: AudioWaveform, why: "Enables on-device Apple transcription." },
];

const STATUS_STYLE: Record<PermissionStatus, { label: string; className: string }> = {
  granted: { label: "Granted", className: "bg-success/15 text-success" },
  denied: { label: "Denied", className: "bg-danger/15 text-danger" },
  not_determined: { label: "Not requested", className: "bg-bg-tile text-fg-muted" },
  restricted: { label: "Restricted", className: "bg-danger/15 text-danger" },
  unknown: { label: "Unknown", className: "bg-bg-tile text-fg-muted" },
};

export default function PermissionsTab() {
  const permissions = usePermissionsStore((s) => s.permissions);
  const request = usePermissionsStore((s) => s.request);
  const openSystemSettings = usePermissionsStore((s) => s.openSystemSettings);
  const [checks, setChecks] = useState<SetupCheck[] | null>(null);
  const [running, setRunning] = useState(false);

  const runChecks = async () => {
    setRunning(true);
    try {
      setChecks(await bluey.app.runSetupChecks());
    } catch (error) {
      console.warn("[permissions] setup checks failed", error);
    } finally {
      setRunning(false);
    }
  };

  return (
    <>
      <div className="flex items-end justify-between">
        <SectionHeader title="Permissions" description="macOS permissions Bluey needs — status updates live" className="mb-0" />
        <Button variant="secondary" size="sm" onClick={() => void runChecks()} disabled={running}>
          {running ? <Spinner size={12} /> : null} Run setup checks
        </Button>
      </div>

      <div className="mt-4 grid grid-cols-2 gap-3">
        {CARDS.map(({ kind, label, icon: Icon, why }) => {
          const status = permissions?.[kind] ?? "unknown";
          const style = STATUS_STYLE[status];
          return (
            <div key={kind} className="flex flex-col gap-2.5 rounded-card border border-border bg-bg-elevated p-4">
              <div className="flex items-center gap-3">
                <div className="flex size-10 items-center justify-center rounded-[10px] bg-bg-tile">
                  <Icon className="size-5 text-fg-muted" strokeWidth={1.8} aria-hidden />
                </div>
                <div className="flex-1 text-[14px] font-medium text-fg">{label}</div>
                <span className={cn("rounded-full px-2.5 py-1 text-[11.5px] font-medium", style.className)}>{style.label}</span>
              </div>
              <p className="text-[12.5px] leading-relaxed text-fg-muted">{why}</p>
              <div className="mt-auto flex gap-2 pt-1">
                {status !== "granted" ? (
                  <Button size="sm" variant="primary" onClick={() => void request(kind)}>
                    Request
                  </Button>
                ) : null}
                <Button size="sm" variant="ghost" onClick={() => void openSystemSettings(kind)}>
                  Open System Settings
                </Button>
              </div>
            </div>
          );
        })}
      </div>

      {checks ? (
        <div className="mt-6">
          <SectionHeader title="Setup checks" />
          <ul className="flex flex-col gap-1.5">
            {checks.map((check) => (
              <li key={check.id} className="flex items-center gap-3 rounded-[10px] border border-border bg-bg-elevated px-3.5 py-2.5">
                <span className={cn("size-2 shrink-0 rounded-full", check.ok ? "bg-success" : "bg-danger")} aria-hidden />
                <span className="w-[140px] shrink-0 text-[13px] font-medium text-fg">{check.label}</span>
                <span className="flex-1 text-[12.5px] text-fg-muted">{check.detail}</span>
                {!check.ok && check.fix ? <span className="text-[12px] text-fg-subtle">{check.fix}</span> : null}
              </li>
            ))}
          </ul>
        </div>
      ) : null}
    </>
  );
}
