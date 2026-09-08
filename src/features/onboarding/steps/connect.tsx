import { CheckCircle2, Settings2 } from "lucide-react";
import { useEffect, useState } from "react";

import { Button } from "@/components/ui/Button";
import { showErrorToast } from "@/components/ui/toast-store";
import { GEMINI_PRESET } from "@/lib/ai/provider-presets";
import { bluey } from "@/lib/tauri/api";
import { SECRET_KEYS } from "@/lib/tauri/commands";
import { toBlueyError, type AIProviderConfig } from "@/lib/types";
import { useSettingsStore } from "@/stores/settingsStore";
import { providerKeyHelp } from "@/features/settings/provider-form";
import { SecretKeyField } from "@/features/settings/SecretKeyField";
import type { StepProps } from "../OnboardingFlow";
import { StepShell } from "./basics";

function geminiProviderConfig(): AIProviderConfig {
  return {
    id: GEMINI_PRESET.id,
    kind: GEMINI_PRESET.kind,
    name: GEMINI_PRESET.name,
    baseUrl: GEMINI_PRESET.baseUrl,
    enabled: true,
    hasApiKey: false,
  };
}

/**
 * Onboarding → "Connect Gemini": one Google AI Studio key lights up chat,
 * vision, transcription, embeddings and research. Saving the key stores it in
 * the Keychain (never in settings), ensures the reserved `gemini` provider
 * exists and assigns Gemini's recommended models to every role that has none.
 * The step can be skipped; Settings → AI offers every other provider.
 */
export function ConnectAIStep({ onReady }: StepProps) {
  const settings = useSettingsStore((s) => s.settings);
  const update = useSettingsStore((s) => s.update);
  const applyRemote = useSettingsStore((s) => s.applyRemote);
  const [skipped, setSkipped] = useState(false);

  const gemini = settings?.ai.providers.find((p) => p.kind === "google_gemini");
  const hasKey = gemini?.hasApiKey ?? false;
  const otherProviderReady =
    settings?.ai.providers.some((p) => p.kind !== "google_gemini" && p.enabled && p.hasApiKey) ?? false;

  useEffect(() => {
    onReady(hasKey || otherProviderReady || skipped);
  }, [hasKey, otherProviderReady, skipped, onReady]);

  // Make sure the reserved Gemini provider exists so the key field has a home.
  useEffect(() => {
    if (!settings || gemini) return;
    void update({ ai: { providers: [geminiProviderConfig(), ...settings.ai.providers] } }).catch(
      (error: unknown) => showErrorToast(toBlueyError(error, "storage")),
    );
  }, [settings, gemini, update]);

  const onKeySaved = async () => {
    const providerId = gemini?.id ?? GEMINI_PRESET.id;
    try {
      // Fill roles that are still unassigned; a user with an existing setup keeps it.
      applyRemote(await bluey.ai.applyProviderPresets({ providerId, overwrite: false }));
      if (!settings?.ai.bootstrapProvider) await update({ ai: { bootstrapProvider: providerId } });
    } catch (error) {
      showErrorToast(toBlueyError(error, "configuration"));
    }
  };

  return (
    <StepShell
      title="Connect Gemini"
      body="One Google AI Studio key powers answers, screenshot understanding, live transcription, document retrieval and deep research. It stays in your macOS Keychain."
    >
      <div className="flex flex-col items-center gap-4">
        {hasKey ? (
          <p className="flex items-center gap-1.5 text-[13px] text-success">
            <CheckCircle2 className="size-4" aria-hidden /> Gemini is connected
          </p>
        ) : null}
        <SecretKeyField
          secretKey={SECRET_KEYS.providerApiKey(gemini?.id ?? GEMINI_PRESET.id)}
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
