import { Bug, ChevronDown, ChevronUp } from "lucide-react";
import { useState } from "react";

import { bluey } from "@/lib/tauri/api";
import type { DevSimulation } from "@/lib/types";
import { formatMs } from "@/lib/utils/format";
import { useDevStore } from "@/stores/devStore";
import { useSettingsStore } from "@/stores/settingsStore";

const SIMULATIONS: Array<{ label: string; simulation: DevSimulation }> = [
  { label: "Question", simulation: { type: "question", text: "Can you walk me through how you'd design a rate limiter?", speaker: "Interviewer" } },
  { label: "Coding problem", simulation: { type: "coding_problem", text: "Two Sum — return indices of two numbers adding to target" } },
  {
    label: "Transcript",
    simulation: {
      type: "transcript",
      segments: [
        { text: "Thanks for joining — let's get started.", speaker: "Interviewer", source: "system" },
        { text: "Great to be here.", speaker: "You", source: "microphone" },
      ],
    },
  },
  { label: "Screen capture", simulation: { type: "screen_capture" } },
  { label: "Permission error", simulation: { type: "permission_error", permission: "screenRecording" } },
  { label: "AI slow (2s)", simulation: { type: "ai_latency", ms: 2000 } },
  { label: "AI failure", simulation: { type: "ai_failure" } },
  { label: "Clear", simulation: { type: "clear" } },
];

const METRIC_ROWS: Array<{ label: string; key: "captureMs" | "ocrMs" | "contextAssemblyMs" | "timeToFirstTokenMs" | "totalResponseMs" }> = [
  { label: "Capture", key: "captureMs" },
  { label: "OCR", key: "ocrMs" },
  { label: "Context", key: "contextAssemblyMs" },
  { label: "First token", key: "timeToFirstTokenMs" },
  { label: "Total", key: "totalResponseMs" },
];

/** Floating developer overlay: latency metrics, simulations, log tail. */
export function DevOverlay() {
  const metrics = useDevStore((s) => s.metrics);
  const logs = useDevStore((s) => s.logs);
  const [open, setOpen] = useState(false);

  return (
    <div className="fixed bottom-3 right-3 z-50 w-[260px] select-none text-[12px]">
      <div className="overflow-hidden rounded-card border border-border bg-bg-elevated/95 shadow-lg shadow-black/40 backdrop-blur">
        <button
          type="button"
          onClick={() => setOpen((v) => !v)}
          className="flex w-full items-center gap-2 px-3 py-2 text-fg-muted transition-colors hover:text-fg"
        >
          <Bug className="size-3.5" aria-hidden />
          <span className="flex-1 text-left font-medium">Developer</span>
          {open ? <ChevronDown className="size-3.5" aria-hidden /> : <ChevronUp className="size-3.5" aria-hidden />}
        </button>
        {open ? (
          <div className="border-t border-border px-3 py-2.5">
            <table className="w-full">
              <tbody>
                {METRIC_ROWS.map((row) => (
                  <tr key={row.key}>
                    <td className="py-0.5 text-fg-subtle">{row.label}</td>
                    <td className="py-0.5 text-right font-mono text-fg-muted">{formatMs(metrics?.[row.key])}</td>
                  </tr>
                ))}
              </tbody>
            </table>
            <div className="mt-2.5 flex flex-wrap gap-1">
              {SIMULATIONS.map((item) => (
                <button
                  key={item.label}
                  type="button"
                  onClick={() => void bluey.dev.simulate({ simulation: item.simulation })}
                  className="rounded-[6px] border border-border bg-bg-tile px-2 py-1 text-[11px] text-fg-muted transition-colors hover:text-fg"
                >
                  {item.label}
                </button>
              ))}
            </div>
            {logs.length > 0 ? (
              <div className="mt-2.5 max-h-24 overflow-y-auto rounded-[6px] bg-bg p-1.5 font-mono text-[10.5px] leading-relaxed text-fg-subtle">
                {logs.slice(-8).map((log, i) => (
                  <div key={i} className="truncate">
                    <span className="text-fg-muted">{log.level}</span> {log.target}: {log.message}
                  </div>
                ))}
              </div>
            ) : null}
          </div>
        ) : null}
      </div>
    </div>
  );
}

/** Renders the overlay when `advanced.showDevOverlay` or `?dev=1`. */
export function DevOverlayGate() {
  const show = useSettingsStore((s) => s.settings?.advanced.showDevOverlay ?? false);
  const forced = typeof window !== "undefined" && new URLSearchParams(window.location.search).get("dev") === "1";
  if (!show && !forced) return null;
  return <DevOverlay />;
}
