import { CheckCircle2, Settings2 } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";

import { Button } from "@/components/ui/Button";
import { ErrorBanner } from "@/components/ui/ErrorBanner";
import { Spinner } from "@/components/ui/Spinner";
import { showErrorToast } from "@/components/ui/toast-store";
import { GEMINI_PRESET } from "@/lib/ai/provider-presets";
import { bluey } from "@/lib/tauri/api";
import { SECRET_KEYS } from "@/lib/tauri/commands";
import { toBlueyError, type AIProviderConfig, type BlueyError } from "@/lib/types";
import { useSettingsStore } from "@/stores/settingsStore";
import { providerKeyHelp } from "@/features/settings/provider-form";
import { SecretKeyField } from "@/features/settings/SecretKeyField";
import type { StepProps } from "../OnboardingFlow";
import { StepShell } from "./basics";

function geminiProviderConfig(hasApiKey = false): AIProviderConfig {
  return {
    id: GEMINI_PRESET.id,
    kind: GEMINI_PRESET.kind,
    name: GEMINI_PRESET.name,
    baseUrl: GEMINI_PRESET.baseUrl,
    enabled: true,
    hasApiKey,
  };
}

type ConnectionStatus = "idle" | "checking" | "connected" | "failed";

/**
 * Onboarding → "Connect Gemini": one Google AI Studio key lights up chat,
 * vision, transcription, embeddings and research. Saving the key stores it in
 * the Keychain (never in settings), ensures the reserved `gemini` provider
 * exists, assigns Gemini's recommended models to every role that has none and
 * then verifies the key with a tiny request — "connected" is only shown once
 * that succeeds. The step can be skipped; Settings → AI offers every other provider.
 */
export function ConnectAIStep({ onReady }: StepProps) {
  const settings = useSettingsStore((s) => s.settings);
  const update = useSettingsStore((s) => s.update);
  const applyRemote = useSettingsStore((s) => s.applyRemote);
  const [skipped, setSkipped] = useState(false);
  const [status, setStatus] = useState<ConnectionStatus>("idle");
  const [testError, setTestError] = useState<BlueyError | null>(null);
  const ensuringProvider = useRef(false);
  const autoVerified = useRef(false);
  const verifyRun = useRef(0);

  const gemini = settings?.ai.providers.find((p) => p.kind === "google_gemini");
  const geminiId = gemini?.id ?? GEMINI_PRESET.id;
  const hasKey = gemini?.hasApiKey ?? false;
  const otherProviderReady =
    settings?.ai.providers.some((p) => p.kind !== "google_gemini" && p.enabled && p.hasApiKey) ?? false;

  useEffect(() => {
    onReady(hasKey || otherProviderReady || skipped);
  }, [hasKey, otherProviderReady, skipped, onReady]);

  // Make sure the reserved Gemini provider exists so the key field has a home.
  useEffect(() => {
    if (!settings || gemini || ensuringProvider.current) return;
    ensuringProvider.current = true;
    void update({ ai: { providers: [geminiProviderConfig(), ...settings.ai.providers] } }).finally(() => {
      ensuringProvider.current = false;
    });
  }, [settings, gemini, update]);

  /** Send a tiny request with Gemini's default model; only the latest run may report. */
  const verify = useCallback(async (providerId: string) => {
    const run = ++verifyRun.current;
    setStatus("checking");
    setTestError(null);
    try {
      const result = await bluey.ai.testConnection({
        providerId,
        model: GEMINI_PRESET.models.default ?? undefined,
      });
      if (run !== verifyRun.current) return;
      if (result.ok) {
        setStatus("connected");
        return;
      }
      setTestError(
        result.error ?? {
          kind: "configuration",
          code: "config.connection_failed",
          message: "The connection test did not succeed.",
          recoverable: true,
          recovery: { type: "retry" },
        },
      );
      setStatus("failed");
    } catch (error) {
      if (run !== verifyRun.current) return;
      setTestError(toBlueyError(error, "configuration"));
      setStatus("failed");
    }
  }, []);

  // A key that is already stored (returning to this step, `.env` import) is verified once too.
  useEffect(() => {
    if (!settings || autoVerified.current) return;
    autoVerified.current = true;
    if (hasKey) void verify(geminiId);
  }, [settings, hasKey, geminiId, verify]);

  const onKeySaved = async () => {
    const current = useSettingsStore.getState().settings;
    const providerId = current?.ai.providers.find((p) => p.kind === "google_gemini")?.id ?? GEMINI_PRESET.id;
    setStatus("checking");
    try {
      if (current && !current.ai.providers.some((p) => p.id === providerId)) {
        // The key landed before the ensure-provider effect resolved: create the provider now so the
        // presets and the connection test have something to address. Failures toast from the store.
        const created = await update({
          ai: { providers: [geminiProviderConfig(true), ...current.ai.providers] },
        });
        if (!created) {
          setStatus("idle");
          return;
        }
      }
      // Fill roles that are still unassigned; a user with an existing setup keeps it.
      applyRemote(await bluey.ai.applyProviderPresets({ providerId, overwrite: false }));
      if (!useSettingsStore.getState().settings?.ai.bootstrapProvider) {
        await update({ ai: { bootstrapProvider: providerId } });
      }
    } catch (error) {
      showErrorToast(toBlueyError(error, "configuration"));
    }
    await verify(providerId);
  };

  return (
    <StepShell
      title="Connect Gemini"
      body="One Google AI Studio key powers answers, screenshot understanding, live transcription, document retrieval and deep research. It stays in your macOS Keychain."
    >
      <div className="flex flex-col items-center gap-4">
        {status === "checking" ? (
          <p className="flex items-center gap-1.5 text-[13px] text-fg-muted">
            <Spinner size={12} /> Checking your key…
          </p>
        ) : status === "connected" ? (
          <p className="flex items-center gap-1.5 text-[13px] text-success">
            <CheckCircle2 className="size-4" aria-hidden /> Gemini is connected
          </p>
        ) : null}
        {status === "failed" && testError ? (
          <ErrorBanner
            error={testError}
            onRetry={() => void verify(geminiId)}
            compact
            className="w-full text-left"
          />
        ) : null}
        <SecretKeyField
          secretKey={SECRET_KEYS.providerApiKey(geminiId)}
          aria-label="Google AI Studio API key"
          placeholder="AIza… (Google AI Studio key)"
          help={providerKeyHelp("google_gemini")}
          onSaved={() => void onKeySaved()}
          className="items-center"
        />
        <div className="flex items-center gap-2">
          <Button
            variant="ghost"
            size="sm"
            onClick={() => void bluey.window.open({ label: "settings", route: "ai" })}
          >
            <Settings2 className="size-3.5" aria-hidden /> Use another provider
          </Button>
          {!hasKey && !otherProviderReady && !skipped ? (
            <Button variant="ghost" size="sm" onClick={() => setSkipped(true)}>
              Skip for now
            </Button>
          ) : null}
        </div>
        {skipped && !hasKey ? (
          <p className="text-[12.5px] text-fg-subtle">You can add a key later in Settings → AI.</p>
        ) : null}
      </div>
    </StepShell>
  );
}
