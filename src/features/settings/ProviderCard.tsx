import { CheckCircle2, XCircle } from "lucide-react";
import { useState } from "react";

import { Button } from "@/components/ui/Button";
import { Dialog } from "@/components/ui/Dialog";
import { Input } from "@/components/ui/Input";
import { Select } from "@/components/ui/Select";
import { Spinner } from "@/components/ui/Spinner";
import { Switch } from "@/components/ui/Switch";
import { bluey } from "@/lib/tauri/api";
import { SECRET_KEYS } from "@/lib/tauri/commands";
import type { AIProviderConfig, ConnectionTestResult } from "@/lib/types";
import { SecretKeyField } from "./SecretKeyField";
import type { ProviderDraftValues } from "./provider-form";

const KIND_OPTIONS = [
  { value: "azure_foundry", label: "Azure Foundry / OpenAI" },
  { value: "anthropic", label: "Anthropic" },
  { value: "openai_compatible", label: "OpenAI-compatible" },
];

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
          <Button variant="primary" disabled={!values.name.trim() || !values.baseUrl.trim()} onClick={() => onSave(values)}>
            Save
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-3">
        <label className="flex flex-col gap-1 text-[12.5px] text-fg-muted">
          Name
          <Input value={values.name} onChange={(e) => set({ name: e.target.value })} placeholder="My provider" />
        </label>
        <label className="flex flex-col gap-1 text-[12.5px] text-fg-muted">
          Kind
          <Select
            value={values.kind}
            onChange={(e) => set({ kind: e.target.value as AIProviderConfig["kind"] })}
            options={KIND_OPTIONS}
            className="w-full [&>select]:w-full"
          />
        </label>
        <label className="flex flex-col gap-1 text-[12.5px] text-fg-muted">
          Base URL
          <Input value={values.baseUrl} onChange={(e) => set({ baseUrl: e.target.value })} placeholder="https://my-resource.openai.azure.com" />
        </label>
        <label className="flex flex-col gap-1 text-[12.5px] text-fg-muted">
          API version (Azure legacy endpoint)
          <Input value={values.apiVersion} onChange={(e) => set({ apiVersion: e.target.value })} placeholder="2024-10-21" />
        </label>
        <label className="flex flex-col gap-1 text-[12.5px] text-fg-muted">
          Deployments — one `model=deployment` per line
          <textarea
            value={values.deployments}
            onChange={(e) => set({ deployments: e.target.value })}
            rows={3}
            className="w-full rounded-control border border-border bg-bg-tile px-3 py-2 font-mono text-[12px] text-fg outline-none focus-visible:border-border-strong"
            placeholder={"gpt-4.1=gpt-41-prod"}
          />
        </label>
      </div>
    </Dialog>
  );
}

export interface ProviderCardProps {
  provider: AIProviderConfig;
  onEdit: () => void;
  onToggleEnabled: (enabled: boolean) => void;
}

export function ProviderCard({ provider, onEdit, onToggleEnabled }: ProviderCardProps) {
  const [testing, setTesting] = useState(false);
  const [result, setResult] = useState<ConnectionTestResult | null>(null);

  const test = async () => {
    setTesting(true);
    setResult(null);
    try {
      setResult(await bluey.ai.testConnection({ providerId: provider.id }));
    } catch (error) {
      console.warn("[ai] test failed", error);
    } finally {
      setTesting(false);
    }
  };

  return (
    <div className="rounded-card border border-border bg-bg-elevated p-4">
      <div className="flex items-center gap-3">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span className="text-[14px] font-medium text-fg">{provider.name}</span>
            <span className="rounded-full bg-bg-tile px-2 py-0.5 text-[11px] text-fg-subtle">
              {KIND_OPTIONS.find((k) => k.value === provider.kind)?.label ?? provider.kind}
            </span>
          </div>
          <div className="mt-0.5 truncate text-[12.5px] text-fg-muted">{provider.baseUrl}</div>
        </div>
        <Button variant="ghost" size="sm" onClick={onEdit}>
          Edit
        </Button>
        <Switch aria-label={`Enable ${provider.name}`} checked={provider.enabled} onCheckedChange={onToggleEnabled} />
      </div>

      <div className="mt-3 flex flex-wrap items-center justify-between gap-3 border-t border-border pt-3">
        <SecretKeyField secretKey={SECRET_KEYS.providerApiKey(provider.id)} aria-label={`${provider.name} API key`} />
        <div className="flex items-center gap-2">
          {result ? (
            result.ok ? (
              <span className="flex items-center gap-1.5 text-[12.5px] text-success">
                <CheckCircle2 className="size-4" aria-hidden /> Connected · {result.latencyMs}ms
              </span>
            ) : (
              <span className="flex items-center gap-1.5 text-[12.5px] text-danger">
                <XCircle className="size-4" aria-hidden /> {result.error?.message ?? "Failed"}
              </span>
            )
          ) : null}
          <Button variant="secondary" size="sm" onClick={() => void test()} disabled={testing}>
            {testing ? <Spinner size={12} /> : null} Test connection
          </Button>
        </div>
      </div>
    </div>
  );
}
