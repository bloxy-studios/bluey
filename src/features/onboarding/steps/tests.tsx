import { CheckCircle2 } from "lucide-react";
import { useState } from "react";

import { BlueyMark } from "@/components/BlueyMark";
import { Button } from "@/components/ui/Button";
import { ErrorBanner } from "@/components/ui/ErrorBanner";
import { LevelMeter } from "@/components/ui/LevelMeter";
import { Spinner } from "@/components/ui/Spinner";
import { showErrorToast } from "@/components/ui/toast-store";
import { bluey } from "@/lib/tauri/api";
import { toBlueyError, type ConnectionTestResult, type ScreenFrame } from "@/lib/types";
import { useAiReadiness } from "@/hooks/useAiReadiness";
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
      showErrorToast(toBlueyError(error, "capture"));
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
      showErrorToast(toBlueyError(error, "audio"));
    } finally {
      setTesting(false);
    }
  };

  return (
    <StepShell title="Test your microphone" body="Say something — the meter should move.">
      <div className="flex flex-col items-center gap-4">
        <LevelMeter
          level={testing ? levels.microphone : (peak ?? 0)}
          segments={24}
          aria-label="Microphone level"
          className="h-5"
        />
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
  const { readiness } = useAiReadiness();
  const [result, setResult] = useState<ConnectionTestResult | null>(null);
  const [busy, setBusy] = useState(false);

  // Test exactly what an answer would use (the router's choice), not a guess from the settings.
  const provider = readiness?.ok
    ? settings?.ai.providers.find((p) => p.id === readiness.providerId)
    : undefined;

  const test = async () => {
    if (!provider) return;
    setBusy(true);
    setResult(null);
    try {
      setResult(
        await bluey.ai.testConnection({ providerId: provider.id, model: readiness?.model }),
      );
    } catch (error) {
      showErrorToast(toBlueyError(error, "ai"));
    } finally {
      setBusy(false);
    }
  };

  if (!readiness) {
    return (
      <StepShell title="Test your AI provider" body="Checking your AI setup…">
        <Spinner className="mx-auto" />
      </StepShell>
    );
  }

  if (!provider) {
    return (
      <StepShell
        title="Connect an AI provider"
        body="Bluey can't answer yet. Paste a Google AI Studio key in the previous step, or add Microsoft Foundry, Anthropic or an OpenAI-compatible endpoint in Settings → AI — keys stay in the macOS Keychain."
      >
        {readiness.error ? (
          <ErrorBanner error={readiness.error} compact className="w-full text-left" />
        ) : (
          <Button
            variant="secondary"
            onClick={() => void bluey.window.open({ label: "settings", route: "ai" })}
          >
            Open AI settings
          </Button>
        )}
      </StepShell>
    );
  }

  return (
    <StepShell
      title="Test your AI provider"
      body={`Bluey will send a tiny request to ${provider.name} to verify the connection.`}
    >
      <div className="flex flex-col items-center gap-4">
        {result ? (
          result.ok ? (
            <p className="flex items-center gap-1.5 text-[13px] text-success">
              <CheckCircle2 className="size-4" aria-hidden /> Connected · {result.model} · {result.latencyMs}
              ms
            </p>
          ) : result.error ? (
            <ErrorBanner
              error={result.error}
              onRetry={() => void test()}
              compact
              className="w-full text-left"
            />
          ) : null
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
  const { readiness } = useAiReadiness();
  // Only the router's answer makes the claim: "ready" while it cannot route would be a lie.
  const unready = readiness !== null && !readiness.ok;
  return (
    <div className="flex flex-col items-center">
      <BlueyMark size={44} className="mb-6 text-fg" />
      <StepShell
        title={unready ? `${blueyName} can't answer yet` : `${blueyName} is ready`}
        body={
          unready
            ? "Everything else is set up. Fix the AI provider below, or open Bluey and do it later in Settings → AI."
            : "Press ⌘\\ anytime to show or hide the HUD, and ⌘↵ to ask about your screen. Have a great session."
        }
      >
        {unready && readiness.error ? (
          <ErrorBanner error={readiness.error} compact className="w-full text-left" />
        ) : null}
      </StepShell>
    </div>
  );
}
