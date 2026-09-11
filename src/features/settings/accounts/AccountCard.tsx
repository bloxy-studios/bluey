import { Copy, ExternalLink, KeyRound, RefreshCw, ShieldCheck } from "lucide-react";
import { useState } from "react";

import { Button } from "@/components/ui/Button";
import { Card } from "@/components/ui/Card";
import { Input } from "@/components/ui/Input";
import { Pill } from "@/components/ui/Pill";
import { Spinner } from "@/components/ui/Spinner";
import { showToast } from "@/components/ui/toast-store";
import type { ProviderAccount, ProviderModelCatalog } from "@/lib/types";
import { cn } from "@/lib/utils/cn";
import { canConnect, formatAge, formatResetTime, statusLabel, type ProviderCopy, type StatusTone } from "./account-copy";

export interface AccountCardProps {
  account: ProviderAccount;
  copy: ProviderCopy;
  catalog?: ProviderModelCatalog;
  /** Developer mode shows the fingerprint probe. */
  developerMode?: boolean;
  onConnect: () => void;
  onImport: () => void;
  onCancel: () => void;
  onDisconnect: () => void;
  onRefreshCatalog: () => void;
  onSubmitCode: (code: string) => void;
  onProbe: () => void;
}

const TONE_CLASS: Record<StatusTone, string> = {
  neutral: "text-fg-muted",
  pending: "text-fg",
  success: "text-success",
  warning: "text-warning",
  danger: "text-danger",
};

/**
 * One subscription provider (ADR 0009 §3.8): status badge, plan and e-mail,
 * the fingerprint this build ships, the models the plan exposes, and the
 * actions that make sense for the current status. Never shows a token.
 */
export function AccountCard({
  account,
  copy,
  catalog,
  developerMode = false,
  onConnect,
  onImport,
  onCancel,
  onDisconnect,
  onRefreshCatalog,
  onSubmitCode,
  onProbe,
}: AccountCardProps) {
  const [code, setCode] = useState("");
  const { status, identity } = account;
  const badge = statusLabel(account);
  const connected = status.state === "connected";

  return (
    <Card className="flex flex-col gap-3 p-4" data-testid={`account-card-${account.providerId}`}>
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <span className="text-[14px] font-medium text-fg">{copy.name}</span>
            <span className="text-[12px] text-fg-subtle">{copy.plans}</span>
          </div>
          <div className="mt-0.5 text-[12.5px] text-fg-muted">
            {identity?.email ?? `Sign in with your ${copy.vendor} subscription`}
            {identity?.projectId ? <span className="text-fg-subtle"> · project {identity.projectId}</span> : null}
          </div>
        </div>
        <Pill variant="outline" size="sm" className={cn("shrink-0", TONE_CLASS[badge.tone])} title={badge.label}>
          {status.state === "connecting" ? <Spinner size={10} /> : null}
          <span className="truncate">{badge.label}</span>
        </Pill>
      </div>

      {status.state === "connecting" ? (
        <div className="flex flex-col gap-2 rounded-card bg-bg-tile p-3 text-[12.5px] text-fg-muted">
          {status.flow.kind === "device_code" ? (
            <>
              <p>
                Enter this code at{" "}
                <button
                  type="button"
                  className="inline-flex items-center gap-1 text-accent hover:underline"
                  onClick={() => {
                    if (status.flow.verificationUrl) window.open(status.flow.verificationUrl, "_blank", "noopener");
                  }}
                >
                  {status.flow.verificationUrl?.replace(/^https?:\/\//, "") ?? "the verification page"}
                  <ExternalLink className="size-3" aria-hidden />
                </button>
              </p>
              <div className="flex items-center gap-2">
                <code className="rounded-control bg-bg-elevated px-2 py-1 text-[15px] tracking-[0.15em] text-fg" aria-label="Device code">
                  {status.flow.userCode}
                </code>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => {
                    void navigator.clipboard?.writeText(status.flow.userCode ?? "");
                    showToast("Code copied", 1500);
                  }}
                >
                  <Copy className="size-3.5" aria-hidden /> Copy code
                </Button>
              </div>
            </>
          ) : status.flow.kind === "manual_code" ? (
            <>
              <p>Sign in in your browser, then paste the code the page shows you (it looks like `code#state`).</p>
              <form
                className="flex items-center gap-2"
                onSubmit={(event) => {
                  event.preventDefault();
                  if (code.trim()) {
                    onSubmitCode(code.trim());
                    setCode("");
                  }
                }}
              >
                <Input
                  aria-label={`${copy.name} sign-in code`}
                  value={code}
                  onChange={(event) => setCode(event.target.value)}
                  placeholder="code#state"
                  className="flex-1"
                />
                <Button type="submit" variant="secondary" size="sm" disabled={!code.trim()}>
                  Submit code
                </Button>
              </form>
            </>
          ) : (
            <p className="flex items-center gap-1.5">
              <Spinner size={11} /> Waiting for your browser… finish signing in to {copy.vendor}, then come back.
            </p>
          )}
          <p className="text-[12px] text-fg-subtle">Expires {formatResetTime(status.flow.expiresAt)}.</p>
        </div>
      ) : null}

      {status.state === "rate_limited" ? (
        <p className="text-[12.5px] text-fg-muted">
          Your {status.window ? `${status.window} ` : ""}plan window is used up — it resets at {formatResetTime(status.until)}.
          Bluey routes these roles to your API key until then.
        </p>
      ) : null}

      {status.state === "unavailable" ? (
        <p className="text-[12.5px] text-fg-muted">
          {status.detail ?? "The provider is not accepting requests from Bluey."} Your API key is used instead.
        </p>
      ) : null}

      {status.state === "needs_reauth" ? (
        <p className="text-[12.5px] text-fg-muted">The sign-in expired. Reconnect to keep using your plan; your API key is used meanwhile.</p>
      ) : null}

      <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-[12px] text-fg-subtle">
        <span className="inline-flex items-center gap-1" title="The request fingerprint this build reproduces">
          <ShieldCheck className="size-3.5" aria-hidden />
          {account.fingerprintVersion
            ? `Fingerprint ${account.fingerprintVersion} · captured ${account.fingerprintCapturedOn ?? "—"}`
            : "No fingerprint in this build"}
        </span>
        {connected ? (
          <span>
            {catalog ? `${catalog.models.length} models · fetched ${formatAge(catalog.fetchedAt)}` : "Models not fetched yet"}
          </span>
        ) : null}
      </div>

      <div className="flex flex-wrap items-center gap-2">
        {status.state === "connecting" ? (
          <Button variant="secondary" size="sm" onClick={onCancel}>
            Cancel
          </Button>
        ) : null}
        {canConnect(status) ? (
          <>
            <Button variant="primary" size="sm" onClick={onConnect}>
              <KeyRound className="size-3.5" aria-hidden />
              {status.state === "needs_reauth" ? "Reconnect" : `Connect ${copy.name}`}
            </Button>
            <Button variant="ghost" size="sm" onClick={onImport}>
              {copy.importLabel}
            </Button>
          </>
        ) : null}
        {connected ? (
          <>
            <Button variant="secondary" size="sm" onClick={onRefreshCatalog}>
              <RefreshCw className="size-3.5" aria-hidden /> Refresh models
            </Button>
            {developerMode ? (
              <Button variant="ghost" size="sm" onClick={onProbe}>
                Probe fingerprint
              </Button>
            ) : null}
          </>
        ) : null}
        {status.state !== "disconnected" && status.state !== "connecting" ? (
          <Button variant="ghost" size="sm" className="text-danger" onClick={onDisconnect}>
            Disconnect
          </Button>
        ) : null}
      </div>
    </Card>
  );
}
