import { Bug, FileCode2, RefreshCcw, ScrollText } from "lucide-react";
import { useEffect, useState } from "react";

import { formatStageMs, summarize } from "@/ai/trace";
import { Button } from "@/components/ui/Button";
import { SectionHeader } from "@/components/ui/SectionHeader";
import { Select } from "@/components/ui/Select";
import { SettingRow } from "@/components/ui/SettingRow";
import { Switch } from "@/components/ui/Switch";
import { showToast } from "@/components/ui/toast-store";
import { bluey } from "@/lib/tauri/api";
import { toBlueyError, type DevSimulation, type LogLevel } from "@/lib/types";
import { formatMs } from "@/lib/utils/format";
import { useDevStore } from "@/stores/devStore";
import { useSettingsStore } from "@/stores/settingsStore";

const SIMULATIONS: Array<{ label: string; simulation: DevSimulation }> = [
  { label: "Question detected", simulation: { type: "question", text: "Can you walk me through your approach?", speaker: "Interviewer" } },
  { label: "Coding problem", simulation: { type: "coding_problem", text: "Two Sum — return indices of two numbers adding to target" } },
  { label: "Transcript burst", simulation: { type: "transcript", segments: [{ text: "Let's look at the numbers for Q3.", speaker: "Speaker 1", source: "system" }] } },
  { label: "Screen capture", simulation: { type: "screen_capture" } },
  { label: "Permission error", simulation: { type: "permission_error", permission: "screenRecording" } },
  { label: "AI latency 2s", simulation: { type: "ai_latency", ms: 2000 } },
  { label: "AI failure", simulation: { type: "ai_failure" } },
  { label: "Clear transcript", simulation: { type: "clear" } },
];

export default function AdvancedTab() {
  const settings = useSettingsStore((s) => s.settings);
  const update = useSettingsStore((s) => s.update);
  const metrics = useDevStore((s) => s.metrics);
  const setMetrics = useDevStore((s) => s.setMetrics);
  const traces = useDevStore((s) => s.traces);
  const clearTraces = useDevStore((s) => s.clearTraces);
  const bench = useDevStore((s) => s.bench);
  const setBench = useDevStore((s) => s.setBench);
  const [benchRunning, setBenchRunning] = useState(false);

  useEffect(() => {
    void bluey.dev.getMetrics().then(setMetrics).catch(() => undefined);
  }, [setMetrics]);

  const runBench = async () => {
    setBenchRunning(true);
    try {
      setBench(await bluey.dev.benchFastPath({ options: { iterations: 10, provider: "mock" } }));
    } catch (error) {
      showToast(toBlueyError(error, "ai").message);
    } finally {
      setBenchRunning(false);
    }
  };

  if (!settings) return null;
  const stageRows = summarize(traces);

  return (
    <>
      <SectionHeader title="Advanced" description="Logging, diagnostics and the native helper" />

      <SettingRow icon={ScrollText} title="Log level" description="How much Bluey writes to its log file.">
        <Select
          aria-label="Log level"
          value={settings.advanced.logLevel}
          onChange={(e) => void update({ advanced: { logLevel: e.target.value as LogLevel } })}
          options={["error", "warn", "info", "debug", "trace"].map((l) => ({ value: l, label: l }))}
        />
      </SettingRow>

      <SettingRow icon={Bug} title="Developer overlay" description="Latency metrics + simulation buttons over every window.">
        <Switch
          aria-label="Developer overlay"
          checked={settings.advanced.showDevOverlay}
          onCheckedChange={(v) => void update({ advanced: { showDevOverlay: v } })}
        />
      </SettingRow>

      <SettingRow icon={RefreshCcw} title="Native helper" description="Restart the capture/audio helper process.">
        <Button
          variant="secondary"
          onClick={() => void bluey.dev.restartHelper().then(() => showToast("Helper restarting"))}
        >
          Restart helper
        </Button>
      </SettingRow>

      <SettingRow icon={FileCode2} title="Restart helper on crash" description="Automatically relaunch the helper if it stops.">
        <Switch
          aria-label="Restart helper on crash"
          checked={settings.advanced.helperRestartOnCrash}
          onCheckedChange={(v) => void update({ advanced: { helperRestartOnCrash: v } })}
        />
      </SettingRow>

      {settings.general.developerMode ? (
        <>
          <SectionHeader title="Simulate" description="Feed fixture events through the whole pipeline (Developer Mode)" />
          <div className="flex flex-wrap gap-2">
            {SIMULATIONS.map((item) => (
              <Button key={item.label} variant="secondary" size="sm" onClick={() => void bluey.dev.simulate({ simulation: item.simulation })}>
                {item.label}
              </Button>
            ))}
          </div>

          <SectionHeader title="Latency metrics" description="From the last response cycle" />
          <table className="w-full max-w-[420px] text-[13px]">
            <tbody>
              {(
                [
                  ["Capture", metrics?.captureMs],
                  ["OCR", metrics?.ocrMs],
                  ["Accessibility", metrics?.accessibilityMs],
                  ["Context assembly", metrics?.contextAssemblyMs],
                  ["Model", metrics?.modelMs],
                  ["First token", metrics?.timeToFirstTokenMs],
                  ["Total response", metrics?.totalResponseMs],
                ] as Array<[string, number | undefined]>
              ).map(([label, value]) => (
                <tr key={label} className="border-b border-border/40 last:border-0">
                  <td className="py-1.5 text-fg-muted">{label}</td>
                  <td className="py-1.5 text-right font-mono text-fg">{formatMs(value)}</td>
                </tr>
              ))}
              <tr>
                <td className="py-1.5 text-fg-muted">Tokens in / out</td>
                <td className="py-1.5 text-right font-mono text-fg">
                  {metrics?.inputTokens ?? "–"} / {metrics?.outputTokens ?? "–"}
                </td>
              </tr>
            </tbody>
          </table>

          <SectionHeader
            title="Fast path"
            description={
              traces.length > 0
                ? `p50 / p95 over the last ${traces.length} traced requests — ms since ⌘↵ (ADR 0010)`
                : "p50 / p95 per stage appear here after the first traced request (ms since ⌘↵, ADR 0010)"
            }
          />
          {stageRows.length > 0 ? (
            <table className="w-full max-w-[520px] text-[13px]" data-testid="fast-path-table">
              <thead>
                <tr className="text-left text-fg-muted">
                  <th className="py-1 font-normal">Stage</th>
                  <th className="py-1 text-right font-normal">p50</th>
                  <th className="py-1 text-right font-normal">p95</th>
                  <th className="py-1 text-right font-normal">n</th>
                </tr>
              </thead>
              <tbody>
                {stageRows.map((row) => (
                  <tr key={row.stage} className="border-b border-border/40 last:border-0">
                    <td className="py-1.5 text-fg-muted">{row.label}</td>
                    <td className="py-1.5 text-right font-mono text-fg">{formatStageMs(row.p50Ms)}</td>
                    <td className="py-1.5 text-right font-mono text-fg">{formatStageMs(row.p95Ms)}</td>
                    <td className="py-1.5 text-right font-mono text-fg-muted">{row.samples}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          ) : null}
          <div className="flex flex-wrap items-center gap-2">
            <Button variant="secondary" size="sm" disabled={benchRunning} onClick={() => void runBench()}>
              {benchRunning ? "Running bench…" : "Run bench (mock, 10 runs)"}
            </Button>
            {traces.length > 0 ? (
              <Button variant="secondary" size="sm" onClick={clearTraces}>
                Clear traces
              </Button>
            ) : null}
            {bench ? (
              <span className="text-[12px] text-fg-muted">
                local total p50 {formatStageMs(bench.localTotalP50Ms)} · p95 {formatStageMs(bench.localTotalP95Ms)}
              </span>
            ) : null}
          </div>
          {bench ? (
            <pre className="max-w-[640px] whitespace-pre-wrap rounded-md border border-border/40 p-3 font-mono text-[12px] text-fg">
              {bench.markdown}
            </pre>
          ) : null}
        </>
      ) : null}
    </>
  );
}
