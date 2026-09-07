import { CheckCircle2, XCircle } from "lucide-react";
import { useState } from "react";

import { BlueyMark } from "@/components/BlueyMark";
import { Button } from "@/components/ui/Button";
import { LevelMeter } from "@/components/ui/LevelMeter";
import { Spinner } from "@/components/ui/Spinner";
import { bluey } from "@/lib/tauri/api";
import type { ConnectionTestResult, ScreenFrame } from "@/lib/types";
import { useSettingsStore } from "@/stores/settingsStore";
import { useTranscriptStore } from "@/stores/transcriptStore";
import type { StepProps } from "../OnboardingFlow";
import { StepShell } from "./basics";

export function TestScreenStep(_props: StepProps) {
  const [frame, setFrame] = useState<ScreenFrame | null>(null);
  const [busy, setBusy] = useState(false);

  const capture = async () => {
    setBusy(true);
    try {
      setFrame(await bluey.capture.screen({ inline: true }));
    } catch (error) {
      console.warn("[onboarding] capture failed", error);
    } finally {
      setBusy(false);
    }
  };

  return (
    <StepShell title="Test screen capture" body="Take a capture to make sure Bluey can see your screen.">
      <div className="flex flex-col items-center gap-4">
        {frame?.image ? (
          <img
            src={`data:${frame.mimeType};base64,${frame.image}`}
            alt="Screen capture preview"
            className="max-h-[200px] w-full rounded-card border border-border object-cover"
          />
        ) : (
          <div className="flex h-[140px] w-full items-center justify-center rounded-card border border-dashed border-border text-[13px] text-fg-subtle">
            {busy ? <Spinner /> : "No capture yet"}
          </div>
        )}
        {frame ? (
          <p className="text-[12.5px] text-fg-muted">
            Captured {frame.width}×{frame.height}
            {frame.durationMs !== undefined ? ` in ${frame.durationMs}ms` : ""}.
          </p>
        ) : null}
        <Button variant="secondary" onClick={() => void capture()} disabled={busy}>
          {frame ? "Capture again" : "Capture screen"}
        </Button>
      </div>
    </StepShell>
  );
}

export function TestMicStep(_props: StepProps) {
  const levels = useTranscriptStore((s) => s.levels);
  const [testing, setTesting] = useState(false);
  const [peak, setPeak] = useState<number | null>(null);

  const test = async () => {
    setTesting(true);
    setPeak(null);
    try {
      const result = await bluey.audio.testMicrophone({ durationMs: 2500 });
      setPeak(result.peakLevel);
    } catch (error) {
      console.warn("[onboarding] mic test failed", error);
    } finally {
      setTesting(false);
    }
  };

  return (
    <StepShell title="Test your microphone" body="Say something — the meter should move.">
      <div className="flex flex-col items-center gap-4">
        <LevelMeter level={testing ? levels.microphone : (peak ?? 0)} segments={24} aria-label="Microphone level" className="h-5" />
        {peak !== null ? (
          <p className="flex items-center gap-1.5 text-[13px] text-success">
            <CheckCircle2 className="size-4" aria-hidden /> Heard you — peak {(peak * 100).toFixed(0)}%
          </p>
        ) : null}
        <Button variant="secondary" onClick={() => void test()} disabled={testing}>
          {testing ? "Listening…" : "Test microphone"}
        </Button>
      </div>
    </StepShell>
  );
}

export function TestAIStep(_props: StepProps) {
  const settings = useSettingsStore((s) => s.settings);
  const [result, setResult] = useState<ConnectionTestResult | null>(null);
  const [busy, setBusy] = useState(false);

  const provider =
    settings?.ai.providers.find((p) => p.id === settings.ai.models.default?.providerId) ??
    settings?.ai.providers.find((p) => p.enabled);

  const test = async () => {
    if (!provider) return;
    setBusy(true);
    setResult(null);
    try {
      setResult(await bluey.ai.testConnection({ providerId: provider.id, model: settings?.ai.models.default?.model }));
    } catch (error) {
      console.warn("[onboarding] ai test failed", error);
    } finally {
      setBusy(false);
    }
  };

  if (!provider) {
    return (
      <StepShell
        title="Connect an AI provider"
        body="No provider is configured yet. Add one in Settings → AI (Azure Foundry, Anthropic or any OpenAI-compatible endpoint) and store its API key — it stays in the macOS Keychain."
      >
        <Button variant="secondary" onClick={() => void bluey.window.open({ label: "settings", route: "ai" })}>
          Open AI settings
        </Button>
      </StepShell>
    );
  }

  return (
    <StepShell title="Test your AI provider" body={`Bluey will send a tiny request to ${provider.name} to verify the connection.`}>
      <div className="flex flex-col items-center gap-4">
        {result ? (
          result.ok ? (
            <p className="flex items-center gap-1.5 text-[13px] text-success">
              <CheckCircle2 className="size-4" aria-hidden /> Connected · {result.model} · {result.latencyMs}ms
            </p>
          ) : (
            <p className="flex items-center gap-1.5 text-[13px] text-danger">
              <XCircle className="size-4" aria-hidden /> {result.error?.message ?? "Connection failed"}
            </p>
          )
        ) : null}
        <Button variant="secondary" onClick={() => void test()} disabled={busy}>
          {busy ? "Testing…" : "Test connection"}
        </Button>
      </div>
    </StepShell>
  );
}

export function ReadyStep(_props: StepProps) {
  const blueyName = useSettingsStore((s) => s.settings?.general.blueyName ?? "Bluey");
  return (
    <div className="flex flex-col items-center">
      <BlueyMark size={44} className="mb-6 text-fg" />
      <StepShell
        title={`${blueyName} is ready`}
        body="Press ⌘\ anytime to show or hide the HUD, and ⌘↵ to ask about your screen. Have a great session."
      />
    </div>
  );
}
