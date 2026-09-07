import { Plus } from "lucide-react";
import { useEffect, useState } from "react";

import { Button } from "@/components/ui/Button";
import { SectionHeader } from "@/components/ui/SectionHeader";
import { Select } from "@/components/ui/Select";
import { Slider } from "@/components/ui/Slider";
import { Switch } from "@/components/ui/Switch";
import { bluey } from "@/lib/tauri/api";
import { SECRET_KEYS } from "@/lib/tauri/commands";
import type { AIProviderConfig, ModelRole, ResponseLength, ResponseTone } from "@/lib/types";
import { createId } from "@/lib/utils/id";
import { useSettingsStore } from "@/stores/settingsStore";
import { ProviderCard, ProviderDialog } from "../ProviderCard";
import { draftToDeployments, providerToDraft, type ProviderDraftValues } from "../provider-form";
import { SecretKeyField } from "../SecretKeyField";

const ROLES: Array<{ role: ModelRole; label: string; hint: string }> = [
  { role: "default", label: "Default", hint: "Main answers" },
  { role: "fast", label: "Fast", hint: "Classification, quick replies" },
  { role: "reasoning", label: "Reasoning", hint: "Hard problems" },
  { role: "vision", label: "Vision", hint: "Screenshots" },
  { role: "research", label: "Research", hint: "Deep research agent" },
  { role: "transcription", label: "Transcription", hint: "Cloud speech-to-text" },
  { role: "embedding", label: "Embedding", hint: "Document retrieval" },
];

function ModelRoleRow({ role, label, hint, providers }: { role: ModelRole; label: string; hint: string; providers: AIProviderConfig[] }) {
  const settings = useSettingsStore((s) => s.settings);
  const update = useSettingsStore((s) => s.update);
  const assignment = settings?.ai.models[role] ?? null;
  const [models, setModels] = useState<string[]>([]);
  const providerId = assignment?.providerId ?? providers[0]?.id ?? "";

  useEffect(() => {
    let alive = true;
    if (!providerId) return;
    void bluey.ai
      .listModels({ providerId })
      .then((list) => {
        if (alive) setModels(list);
      })
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, [providerId]);

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
        onBlur={(e) => save(providerId, e.target.value)}
        placeholder="model name"
        className="h-9 flex-1 rounded-control border border-border bg-bg-elevated px-3 text-[13px] text-fg outline-none placeholder:text-fg-subtle focus-visible:border-border-strong"
      />
      <datalist id={datalistId}>
        {models.map((m) => (
          <option key={m} value={m} />
        ))}
      </datalist>
    </div>
  );
}

export default function AITab() {
  const settings = useSettingsStore((s) => s.settings);
  const update = useSettingsStore((s) => s.update);
  const [dialog, setDialog] = useState<{ mode: "add" } | { mode: "edit"; provider: AIProviderConfig } | null>(null);
  const [budget, setBudget] = useState<number | null>(null);

  if (!settings) return null;
  const { ai } = settings;

  const saveProvider = (values: ProviderDraftValues) => {
    const deployments = draftToDeployments(values.deployments);
    let providers: AIProviderConfig[];
    if (dialog?.mode === "edit") {
      providers = ai.providers.map((p) =>
        p.id === dialog.provider.id
          ? { ...p, name: values.name.trim(), kind: values.kind, baseUrl: values.baseUrl.trim(), apiVersion: values.apiVersion.trim() || undefined, deployments }
          : p,
      );
    } else {
      providers = [
        ...ai.providers,
        {
          id: createId("provider"),
          kind: values.kind,
          name: values.name.trim(),
          baseUrl: values.baseUrl.trim(),
          apiVersion: values.apiVersion.trim() || undefined,
          deployments,
          enabled: true,
          hasApiKey: false,
        },
      ];
    }
    void update({ ai: { providers } });
    setDialog(null);
  };

  return (
    <>
      <div className="flex items-end justify-between">
        <SectionHeader title="Providers" description="Bluey keeps API keys in the macOS Keychain — never in its database" className="mb-0" />
        <Button variant="secondary" size="sm" onClick={() => setDialog({ mode: "add" })}>
          <Plus className="size-3.5" aria-hidden /> Add provider
        </Button>
      </div>

      <div className="mt-3 flex flex-col gap-3">
        {ai.providers.map((provider) => (
          <ProviderCard
            key={provider.id}
            provider={provider}
            onEdit={() => setDialog({ mode: "edit", provider })}
            onToggleEnabled={(enabled) =>
              void update({ ai: { providers: ai.providers.map((p) => (p.id === provider.id ? { ...p, enabled } : p)) } })
            }
          />
        ))}
      </div>

      <SectionHeader title="Models" description="Which model handles each role" />
      <div className="flex flex-col divide-y divide-border/50">
        {ROLES.map(({ role, label, hint }) => (
          <ModelRoleRow key={role} role={role} label={label} hint={hint} providers={ai.providers} />
        ))}
      </div>

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
            <div className="text-[13px] text-fg-muted">Search the web when a question needs fresh facts (Exa).</div>
          </div>
          <Switch aria-label="Web search" checked={ai.researchEnabled} onCheckedChange={(v) => void update({ ai: { researchEnabled: v } })} />
        </div>
        <div className="flex items-center justify-between py-1">
          <div>
            <div className="text-[14px] font-medium text-fg">Deep research agent</div>
            <div className="text-[13px] text-fg-muted">Multi-step research with the Claude agent (public queries only).</div>
          </div>
          <Switch aria-label="Deep research" checked={ai.deepResearchEnabled} onCheckedChange={(v) => void update({ ai: { deepResearchEnabled: v } })} />
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
          <div className="flex items-center justify-between gap-3">
            <span className="text-[13px] text-fg-muted">Anthropic (agent)</span>
            <SecretKeyField secretKey={SECRET_KEYS.anthropicAgentApiKey} aria-label="Anthropic agent API key" />
          </div>
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
        <span className="font-mono text-[13px] text-fg-muted">{((budget ?? ai.contextTokenBudget) / 1000).toFixed(0)}k tokens</span>
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
