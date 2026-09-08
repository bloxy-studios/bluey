import { Plus } from "lucide-react";
import { useEffect, useState } from "react";

import { Button } from "@/components/ui/Button";
import { SectionHeader } from "@/components/ui/SectionHeader";
import { Select } from "@/components/ui/Select";
import { Slider } from "@/components/ui/Slider";
import { Switch } from "@/components/ui/Switch";
import { showErrorToast, showToast } from "@/components/ui/toast-store";
import { presetForKind } from "@/lib/ai/provider-presets";
import { bluey } from "@/lib/tauri/api";
import { SECRET_KEYS } from "@/lib/tauri/commands";
import {
  toBlueyError,
  type AIProviderConfig,
  type ModelRole,
  type ResearchBackend,
  type ResponseLength,
  type ResponseTone,
} from "@/lib/types";
import { useSettingsStore } from "@/stores/settingsStore";
import { ProviderCard, ProviderDialog } from "../ProviderCard";
import {
  draftToDeployments,
  newProviderFromDraft,
  providerToDraft,
  sortProviders,
  type ProviderDraftValues,
} from "../provider-form";
import { SecretKeyField } from "../SecretKeyField";

const ROLES: Array<{ role: ModelRole; label: string; hint: string }> = [
  { role: "default", label: "Default", hint: "Main answers" },
  { role: "fast", label: "Fast", hint: "Classification, quick replies" },
  { role: "reasoning", label: "Reasoning", hint: "Hard problems" },
  { role: "vision", label: "Vision", hint: "Screenshots" },
  { role: "research", label: "Research", hint: "Deep research agent" },
  { role: "transcription", label: "Transcription", hint: "Batch / live speech-to-text" },
  { role: "embedding", label: "Embedding", hint: "Document retrieval" },
];

const EMBEDDING_DIMENSION_OPTIONS = [
  { value: "768", label: "768 (recommended)" },
  { value: "1536", label: "1536" },
  { value: "3072", label: "3072 (full)" },
];

function ModelRoleRow({
  role,
  label,
  hint,
  providers,
}: {
  role: ModelRole;
  label: string;
  hint: string;
  providers: AIProviderConfig[];
}) {
  const settings = useSettingsStore((s) => s.settings);
  const update = useSettingsStore((s) => s.update);
  const assignment = settings?.ai.models[role] ?? null;
  const [models, setModels] = useState<string[]>([]);
  const providerId = assignment?.providerId ?? providers[0]?.id ?? "";
  const provider = providers.find((p) => p.id === providerId);
  const recommended = provider ? (presetForKind(provider.kind)?.models[role] ?? null) : null;

  useEffect(() => {
    let alive = true;
    if (!providerId) return;
    void bluey.ai
      .listModels({ providerId, role })
      .then((list) => {
        if (alive) setModels(list);
      })
      .catch(() => {
        if (alive) setModels([]); // listing is best-effort; typing a model id always works
      });
    return () => {
      alive = false;
    };
  }, [providerId, role]);

  const save = (nextProviderId: string, model: string) => {
    const models_ = { ...settings?.ai.models } as NonNullable<typeof settings>["ai"]["models"];
    models_[role] = model.trim() ? { providerId: nextProviderId, model: model.trim() } : null;
    void update({ ai: { models: models_ } });
  };

  const datalistId = `models-${role}`;
  return (
    <div className="flex items-center gap-3 py-2">
      <div className="w-[150px] shrink-0">
        <div className="text-[13.5px] font-medium text-fg">{label}</div>
        <div className="text-[12px] text-fg-subtle">{hint}</div>
      </div>
      <Select
        aria-label={`${label} provider`}
        value={providerId}
        onChange={(e) => save(e.target.value, assignment?.model ?? "")}
        options={providers.map((p) => ({ value: p.id, label: p.name }))}
        className="w-[180px] [&>select]:min-w-0 [&>select]:w-full"
      />
      <input
        aria-label={`${label} model`}
        list={datalistId}
        defaultValue={assignment?.model ?? ""}
        key={`${providerId}:${assignment?.model ?? ""}`}
        onBlur={(e) => save(providerId, e.target.value)}
        placeholder={recommended ?? "model name"}
        className="h-9 flex-1 rounded-control border border-border bg-bg-elevated px-3 text-[13px] text-fg outline-none placeholder:text-fg-subtle focus-visible:border-border-strong"
      />
      <datalist id={datalistId}>
        {models.map((m) => (
          <option key={m} value={m} />
        ))}
      </datalist>
      {recommended && assignment?.model !== recommended ? (
        <Button
          variant="link"
          size="sm"
          onClick={() => save(providerId, recommended)}
          className="shrink-0 text-[12px]"
        >
          Use {recommended}
        </Button>
      ) : null}
    </div>
  );
}

export default function AITab() {
  const settings = useSettingsStore((s) => s.settings);
  const update = useSettingsStore((s) => s.update);
  const applyRemote = useSettingsStore((s) => s.applyRemote);
  const [dialog, setDialog] = useState<{ mode: "add" } | { mode: "edit"; provider: AIProviderConfig } | null>(
    null,
  );
  const [budget, setBudget] = useState<number | null>(null);
  const [switching, setSwitching] = useState(false);

  if (!settings) return null;
  const { ai } = settings;
  const providers = sortProviders(ai.providers);
  const enabledProviders = providers.filter((p) => p.enabled);
  const defaultProviderId =
    ai.bootstrapProvider && providers.some((p) => p.id === ai.bootstrapProvider)
      ? ai.bootstrapProvider
      : (ai.models.default?.providerId ?? "");
  const embeddingProvider = providers.find((p) => p.id === ai.models.embedding?.providerId);

  const saveProvider = (values: ProviderDraftValues) => {
    let next: AIProviderConfig[];
    if (dialog?.mode === "edit") {
      const deployments =
        values.kind === "azure_foundry" ? draftToDeployments(values.deployments) : undefined;
      next = ai.providers.map((p) =>
        p.id === dialog.provider.id
          ? {
              ...p,
              name: values.name.trim() || p.name,
              kind: values.kind,
              baseUrl: values.baseUrl.trim(),
              apiVersion: values.apiVersion.trim() || undefined,
              deployments,
            }
          : p,
      );
    } else {
      next = [...ai.providers, newProviderFromDraft(values, ai.providers)];
    }
    void update({ ai: { providers: next } }).catch((error: unknown) =>
      showErrorToast(toBlueyError(error, "storage")),
    );
    setDialog(null);
  };

  const switchDefaultProvider = async (providerId: string) => {
    if (!providerId || providerId === defaultProviderId) return;
    const provider = providers.find((p) => p.id === providerId);
    if (!provider) return;
    setSwitching(true);
    try {
      if (presetForKind(provider.kind)) {
        applyRemote(await bluey.ai.applyProviderPresets({ providerId, overwrite: true }));
      }
      await update({ ai: { bootstrapProvider: providerId } });
      showToast(`${provider.name} is now the default provider`, 2000);
    } catch (error) {
      showErrorToast(toBlueyError(error, "configuration"));
    } finally {
      setSwitching(false);
    }
  };

  return (
    <>
      <SectionHeader
        title="Default provider"
        description="One switch for every role — the provider's recommended models are assigned to chat, vision, transcription, research and embeddings. Roles it doesn't serve keep their current model."
      />
      <div className="flex items-center gap-3 py-1">
        <Select
          aria-label="Default AI provider"
          value={defaultProviderId}
          disabled={switching || enabledProviders.length === 0}
          onChange={(e) => void switchDefaultProvider(e.target.value)}
          options={[
            ...(defaultProviderId ? [] : [{ value: "", label: "Choose a provider" }]),
            ...enabledProviders.map((p) => ({ value: p.id, label: p.name })),
          ]}
          className="w-[260px] [&>select]:w-full"
        />
        {ai.bootstrapProvider ? (
          <span className="text-[12.5px] text-fg-subtle">Configured from your environment / onboarding.</span>
        ) : null}
      </div>

      <div className="mt-6 flex items-end justify-between">
        <SectionHeader
          title="Providers"
          description="Bluey keeps API keys in the macOS Keychain — never in its database"
          className="mb-0"
        />
        <Button variant="secondary" size="sm" onClick={() => setDialog({ mode: "add" })}>
          <Plus className="size-3.5" aria-hidden /> Add provider
        </Button>
      </div>

      <div className="mt-3 flex flex-col gap-3">
        {providers.map((provider) => (
          <ProviderCard
            key={provider.id}
            provider={provider}
            isDefault={provider.id === defaultProviderId}
            onEdit={() => setDialog({ mode: "edit", provider })}
            onToggleEnabled={(enabled) =>
              void update({
                ai: { providers: ai.providers.map((p) => (p.id === provider.id ? { ...p, enabled } : p)) },
              })
            }
          />
        ))}
      </div>

      <SectionHeader
        title="Models"
        description="Which model handles each role — pick from the provider's catalogue or type an id"
      />
      <div className="flex flex-col divide-y divide-border/50">
        {ROLES.map(({ role, label, hint }) => (
          <ModelRoleRow key={role} role={role} label={label} hint={hint} providers={providers} />
        ))}
      </div>

      {embeddingProvider?.kind === "google_gemini" ? (
        <div className="mt-3 flex items-center gap-3 py-1">
          <div className="w-[150px] shrink-0">
            <div className="text-[13.5px] font-medium text-fg">Embedding size</div>
            <div className="text-[12px] text-fg-subtle">gemini-embedding-2 (MRL)</div>
          </div>
          <Select
            aria-label="Embedding dimensions"
            value={String(ai.embeddingDimensions)}
            onChange={(e) => void update({ ai: { embeddingDimensions: Number(e.target.value) } })}
            options={EMBEDDING_DIMENSION_OPTIONS}
          />
          <span className="text-[12px] text-fg-subtle">Changing it re-indexes your documents.</span>
        </div>
      ) : null}

      <SectionHeader title="Responses" description="Global style — modes can override" />
      <div className="flex items-center gap-3 py-1">
        <Select
          aria-label="Response length"
          value={ai.responseLength}
          onChange={(e) => void update({ ai: { responseLength: e.target.value as ResponseLength } })}
          options={[
            { value: "concise", label: "Concise" },
            { value: "balanced", label: "Balanced" },
            { value: "detailed", label: "Detailed" },
          ]}
        />
        <Select
          aria-label="Response tone"
          value={ai.responseTone}
          onChange={(e) => void update({ ai: { responseTone: e.target.value as ResponseTone } })}
          options={[
            { value: "natural", label: "Natural" },
            { value: "professional", label: "Professional" },
            { value: "technical", label: "Technical" },
            { value: "conversational", label: "Conversational" },
            { value: "direct", label: "Direct" },
          ]}
        />
      </div>

      <SectionHeader title="Research" description="Optional web research during answers" />
      <div className="flex flex-col gap-3">
        <div className="flex items-center justify-between py-1">
          <div>
            <div className="text-[14px] font-medium text-fg">Web search</div>
            <div className="text-[13px] text-fg-muted">
              Search the web when a question needs fresh facts (Exa).
            </div>
          </div>
          <Switch
            aria-label="Web search"
            checked={ai.researchEnabled}
            onCheckedChange={(v) => void update({ ai: { researchEnabled: v } })}
          />
        </div>
        <div className="flex items-center justify-between py-1">
          <div>
            <div className="text-[14px] font-medium text-fg">Deep research agent</div>
            <div className="text-[13px] text-fg-muted">
              Multi-step research in a sandboxed sidecar (public queries only).
            </div>
          </div>
          <Switch
            aria-label="Deep research"
            checked={ai.deepResearchEnabled}
            onCheckedChange={(v) => void update({ ai: { deepResearchEnabled: v } })}
          />
        </div>
        <div className="flex items-center justify-between py-1">
          <div>
            <div className="text-[14px] font-medium text-fg">Research backend</div>
            <div className="text-[13px] text-fg-muted">
              {ai.researchBackend === "gemini"
                ? "Gemini function calling with your Google AI Studio key — nothing else to configure."
                : "Claude Agent SDK — needs an Anthropic key (or Claude in Foundry) and the full sidecar build."}
            </div>
          </div>
          <Select
            aria-label="Research backend"
            value={ai.researchBackend}
            onChange={(e) => void update({ ai: { researchBackend: e.target.value as ResearchBackend } })}
            options={[
              { value: "gemini", label: "Gemini" },
              { value: "claude", label: "Claude" },
            ]}
          />
        </div>
        <div className="flex flex-col gap-2.5 rounded-card border border-border bg-bg-elevated p-4">
          <div className="flex items-center justify-between gap-3">
            <span className="text-[13px] text-fg-muted">Exa</span>
            <SecretKeyField secretKey={SECRET_KEYS.exaApiKey} aria-label="Exa API key" />
          </div>
          <div className="flex items-center justify-between gap-3">
            <span className="text-[13px] text-fg-muted">Firecrawl</span>
            <SecretKeyField secretKey={SECRET_KEYS.firecrawlApiKey} aria-label="Firecrawl API key" />
          </div>
          {ai.researchBackend === "claude" ? (
            <div className="flex items-center justify-between gap-3">
              <span className="text-[13px] text-fg-muted">Anthropic (agent)</span>
              <SecretKeyField
                secretKey={SECRET_KEYS.anthropicAgentApiKey}
                aria-label="Anthropic agent API key"
              />
            </div>
          ) : null}
        </div>
      </div>

      <SectionHeader title="Context budget" description="Maximum input tokens per request" />
      <div className="flex items-center gap-4 py-1">
        <Slider
          aria-label="Context token budget"
          min={4000}
          max={128000}
          step={4000}
          value={budget ?? ai.contextTokenBudget}
          onValueChange={setBudget}
          onValueCommit={(value) => {
            setBudget(null);
            void update({ ai: { contextTokenBudget: value } });
          }}
          className="w-[240px]"
        />
        <span className="font-mono text-[13px] text-fg-muted">
          {((budget ?? ai.contextTokenBudget) / 1000).toFixed(0)}k tokens
        </span>
      </div>

      {dialog ? (
        <ProviderDialog
          open
          onOpenChange={(open) => {
            if (!open) setDialog(null);
          }}
          title={dialog.mode === "edit" ? `Edit ${dialog.provider.name}` : "Add provider"}
          initial={providerToDraft(dialog.mode === "edit" ? dialog.provider : undefined)}
          onSave={saveProvider}
        />
      ) : null}
    </>
  );
}
