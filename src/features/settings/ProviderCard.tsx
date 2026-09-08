import { CheckCircle2, Sparkles } from "lucide-react";
import { useState } from "react";

import { Button } from "@/components/ui/Button";
import { Dialog } from "@/components/ui/Dialog";
import { ErrorBanner } from "@/components/ui/ErrorBanner";
import { Input } from "@/components/ui/Input";
import { Select } from "@/components/ui/Select";
import { Spinner } from "@/components/ui/Spinner";
import { Switch } from "@/components/ui/Switch";
import { showErrorToast, showToast } from "@/components/ui/toast-store";
import { presetForKind } from "@/lib/ai/provider-presets";
import { bluey } from "@/lib/tauri/api";
import { SECRET_KEYS } from "@/lib/tauri/commands";
import { toBlueyError, type AIProviderConfig, type ConnectionTestResult } from "@/lib/types";
import { useSettingsStore } from "@/stores/settingsStore";
import {
  draftDefaultsFor,
  PROVIDER_KIND_OPTIONS,
  providerKeyHelp,
  providerKindLabel,
  providerNeedsBaseUrl,
  type ProviderDraftValues,
} from "./provider-form";
import { SecretKeyField } from "./SecretKeyField";

export function ProviderDialog({
  open,
  onOpenChange,
  title,
  initial,
  onSave,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  initial: ProviderDraftValues;
  onSave: (values: ProviderDraftValues) => void;
}) {
  const [values, setValues] = useState(initial);
  const set = (patch: Partial<ProviderDraftValues>) => setValues((v) => ({ ...v, ...patch }));
  const needsBaseUrl = providerNeedsBaseUrl(values.kind);
  const defaults = draftDefaultsFor(values.kind);
  const canSave = (values.name.trim() || defaults.name) && (!needsBaseUrl || values.baseUrl.trim());

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={title}
      footer={
        <>
          <Button variant="secondary" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button variant="primary" disabled={!canSave} onClick={() => onSave(values)}>
            Save
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-3">
        <label className="flex flex-col gap-1 text-[12.5px] text-fg-muted">
          Kind
          <Select
            aria-label="Provider kind"
            value={values.kind}
            onChange={(e) => set({ kind: e.target.value as AIProviderConfig["kind"] })}
            options={PROVIDER_KIND_OPTIONS}
            className="w-full [&>select]:w-full"
          />
        </label>
        <label className="flex flex-col gap-1 text-[12.5px] text-fg-muted">
          Name
          <Input
            value={values.name}
            onChange={(e) => set({ name: e.target.value })}
            placeholder={defaults.name || "My provider"}
          />
        </label>
        <label className="flex flex-col gap-1 text-[12.5px] text-fg-muted">
          {needsBaseUrl ? "Base URL" : "Base URL (optional — the public endpoint is used when empty)"}
          <Input
            value={values.baseUrl}
            onChange={(e) => set({ baseUrl: e.target.value })}
            placeholder={
              values.kind === "google_gemini"
                ? "https://generativelanguage.googleapis.com/v1beta"
                : values.kind === "anthropic"
                  ? "https://api.anthropic.com"
                  : "https://my-resource.openai.azure.com"
            }
          />
        </label>
        {values.kind === "azure_foundry" ? (
          <>
            <label className="flex flex-col gap-1 text-[12.5px] text-fg-muted">
              API version — leave empty for the Foundry v1 endpoint; “preview” for v1 preview features; a date
              (e.g. 2024-10-21) for the legacy endpoint
              <Input
                value={values.apiVersion}
                onChange={(e) => set({ apiVersion: e.target.value })}
                placeholder="(empty = v1)"
              />
            </label>
            <label className="flex flex-col gap-1 text-[12.5px] text-fg-muted">
              Deployments — one `model=deployment` per line
              <textarea
                value={values.deployments}
                onChange={(e) => set({ deployments: e.target.value })}
                rows={3}
                className="w-full rounded-control border border-border bg-bg-tile px-3 py-2 font-mono text-[12px] text-fg outline-none focus-visible:border-border-strong"
                placeholder={"gpt-6-astra=astra-prod"}
              />
            </label>
          </>
        ) : null}
      </div>
    </Dialog>
  );
}

export interface ProviderCardProps {
  provider: AIProviderConfig;
  /** True when this provider is the nominated default (Settings → AI → Default provider). */
  isDefault?: boolean;
  onEdit: () => void;
  onToggleEnabled: (enabled: boolean) => void;
}

export function ProviderCard({ provider, isDefault = false, onEdit, onToggleEnabled }: ProviderCardProps) {
  const applyRemote = useSettingsStore((s) => s.applyRemote);
  const [testing, setTesting] = useState(false);
  const [applying, setApplying] = useState(false);
  const [result, setResult] = useState<ConnectionTestResult | null>(null);
  const preset = presetForKind(provider.kind);

  const test = async () => {
    setTesting(true);
    setResult(null);
    try {
      setResult(await bluey.ai.testConnection({ providerId: provider.id }));
    } catch (error) {
      showErrorToast(toBlueyError(error, "ai"));
    } finally {
      setTesting(false);
    }
  };

  const useRecommended = async () => {
    setApplying(true);
    try {
      const settings = await bluey.ai.applyProviderPresets({ providerId: provider.id, overwrite: true });
      applyRemote(settings);
      showToast(`Roles now use ${provider.name}'s recommended models`, 2000);
    } catch (error) {
      showErrorToast(toBlueyError(error, "configuration"));
    } finally {
      setApplying(false);
    }
  };

  return (
    <div className="rounded-card border border-border bg-bg-elevated p-4">
      <div className="flex items-center gap-3">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span className="text-[14px] font-medium text-fg">{provider.name}</span>
            <span className="rounded-full bg-bg-tile px-2 py-0.5 text-[11px] text-fg-subtle">
              {providerKindLabel(provider.kind)}
            </span>
            {isDefault ? (
              <span className="rounded-full bg-accent-soft px-2 py-0.5 text-[11px] font-medium text-accent">
                Default
              </span>
            ) : null}
          </div>
          <div className="mt-0.5 truncate text-[12.5px] text-fg-muted">
            {provider.baseUrl ||
              (provider.kind === "google_gemini" ? "generativelanguage.googleapis.com" : "—")}
          </div>
        </div>
        <Button variant="ghost" size="sm" onClick={onEdit}>
          Edit
        </Button>
        <Switch
          aria-label={`Enable ${provider.name}`}
          checked={provider.enabled}
          onCheckedChange={onToggleEnabled}
        />
      </div>

      <div className="mt-3 flex flex-wrap items-start justify-between gap-3 border-t border-border pt-3">
        <SecretKeyField
          secretKey={SECRET_KEYS.providerApiKey(provider.id)}
          aria-label={`${provider.name} API key`}
          placeholder={provider.kind === "google_gemini" ? "AIza… (Google AI Studio key)" : "API key"}
          help={providerKeyHelp(provider.kind)}
        />
        <div className="flex items-center gap-2">
          {preset ? (
            <Button
              variant="ghost"
              size="sm"
              onClick={() => void useRecommended()}
              disabled={applying || !provider.hasApiKey}
              title={provider.hasApiKey ? undefined : "Save an API key first"}
            >
              {applying ? <Spinner size={12} /> : <Sparkles className="size-3.5" aria-hidden />} Use
              recommended models
            </Button>
          ) : null}
          <Button variant="secondary" size="sm" onClick={() => void test()} disabled={testing}>
            {testing ? <Spinner size={12} /> : null} Test connection
          </Button>
        </div>
      </div>

      {result ? (
        result.ok ? (
          <p className="mt-3 flex items-center gap-1.5 text-[12.5px] text-success">
            <CheckCircle2 className="size-4" aria-hidden /> Connected · {result.model} · {result.latencyMs}ms
          </p>
        ) : result.error ? (
          <ErrorBanner error={result.error} onRetry={() => void test()} compact className="mt-3" />
        ) : null
      ) : null}
    </div>
  );
}
