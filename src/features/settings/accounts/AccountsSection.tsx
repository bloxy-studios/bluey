import { useState } from "react";

import { SectionHeader } from "@/components/ui/SectionHeader";
import { Switch } from "@/components/ui/Switch";
import { showToast } from "@/components/ui/toast-store";
import { useAccountsStore } from "@/stores/accountsStore";
import { useSettingsStore } from "@/stores/settingsStore";
import { AccountCard } from "./AccountCard";
import { PROVIDER_COPY, SUBSCRIPTION_PROVIDER_ORDER, type ProviderCopy } from "./account-copy";
import { ConsentDialog } from "./ConsentDialog";

/**
 * Settings → AI → Accounts (ADR 0009): one card per subscription provider,
 * above the API-key providers. The section can be switched off without a
 * rebuild; the consent dialog runs once per provider before the browser opens.
 */
export function AccountsSection() {
  const settings = useSettingsStore((s) => s.settings);
  const update = useSettingsStore((s) => s.update);
  const accounts = useAccountsStore((s) => s.accounts);
  const catalogs = useAccountsStore((s) => s.catalogs);
  const connect = useAccountsStore((s) => s.connect);
  const importAccount = useAccountsStore((s) => s.importAccount);
  const cancelConnect = useAccountsStore((s) => s.cancelConnect);
  const submitCode = useAccountsStore((s) => s.submitCode);
  const disconnect = useAccountsStore((s) => s.disconnect);
  const refreshCatalog = useAccountsStore((s) => s.refreshCatalog);
  const probeFingerprint = useAccountsStore((s) => s.probeFingerprint);
  const [consentFor, setConsentFor] = useState<ProviderCopy | null>(null);

  if (!settings) return null;
  const enabled = settings.experimental.subscriptionAccounts;
  const accepted = settings.experimental.acceptedAccountConsents;

  const startConnect = (copy: ProviderCopy) => {
    if (accepted.includes(copy.id)) {
      void connect(copy.id);
    } else {
      setConsentFor(copy);
    }
  };

  const acceptConsent = async (copy: ProviderCopy) => {
    setConsentFor(null);
    const saved = await update({
      experimental: { acceptedAccountConsents: [...accepted.filter((id) => id !== copy.id), copy.id] },
    });
    if (saved) void connect(copy.id);
  };

  return (
    <>
      <div className="mt-6 flex items-end justify-between gap-3">
        <SectionHeader
          title="Accounts"
          description="Sign in with a subscription you already pay for. Unofficial — Bluey speaks the vendors' own client protocols, stops at the first sign of a block and falls back to your API key. Tokens stay in the macOS Keychain."
          className="mb-0"
        />
        <label className="flex shrink-0 items-center gap-2 text-[12.5px] text-fg-muted">
          Subscription accounts
          <Switch
            aria-label="Subscription accounts"
            checked={enabled}
            onCheckedChange={(checked) => void update({ experimental: { subscriptionAccounts: checked } })}
          />
        </label>
      </div>
      {enabled ? (
        <div className="mt-3 flex flex-col gap-3" data-testid="accounts-list">
          {SUBSCRIPTION_PROVIDER_ORDER.map((providerId) => {
            const copy = PROVIDER_COPY[providerId];
            const account = accounts.find((a) => a.providerId === providerId);
            if (!account) return null;
            return (
              <AccountCard
                key={providerId}
                account={account}
                copy={copy}
                catalog={catalogs[account.accountId]}
                developerMode={settings.general.developerMode}
                onConnect={() => startConnect(copy)}
                onImport={() => void importAccount(providerId)}
                onCancel={() => void cancelConnect(account.accountId)}
                onDisconnect={() => void disconnect(account.accountId)}
                onRefreshCatalog={() => void refreshCatalog(account.accountId, true)}
                onSubmitCode={(code) => void submitCode(account.accountId, code)}
                onProbe={() =>
                  void probeFingerprint(account.accountId).then((probe) => {
                    if (probe) {
                      showToast(
                        probe.billedTo === "plan"
                          ? `${copy.name}: billed to your plan (fingerprint ${probe.fingerprintVersion ?? "n/a"})`
                          : `${copy.name}: ${probe.message ?? `billed to ${probe.billedTo}`}`,
                        4000,
                      );
                    }
                  })
                }
              />
            );
          })}
          <p className="text-[12px] text-fg-subtle">
            API keys stay in the Providers list below; a connected account is used first for the roles its models
            cover, and the API key when the account is unavailable.
          </p>
        </div>
      ) : (
        <p className="mt-2 text-[12.5px] text-fg-subtle">Off — Bluey uses API keys only.</p>
      )}
      <ConsentDialog provider={consentFor} onCancel={() => setConsentFor(null)} onAccept={(copy) => void acceptConsent(copy)} />
    </>
  );
}
