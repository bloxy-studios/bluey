/**
 * Subscription accounts (ADR 0009): the WebView's mirror of Rust's
 * `AccountsManager` — every `ProviderAccount` and the catalogs that were
 * fetched. Updated from `accounts.changed` / `accounts.catalog` events; the
 * actions call the backend and never throw (failures are toasted and recorded
 * in `lastError`, like the settings store).
 */

import { create } from "zustand";

import { showErrorToast } from "@/components/ui/toast-store";
import { bluey } from "@/lib/tauri/api";
import {
  toBlueyError,
  type AccountConnectOptions,
  type BlueyError,
  type FingerprintProbe,
  type ProviderAccount,
  type ProviderModelCatalog,
} from "@/lib/types";

interface AccountsStore {
  accounts: ProviderAccount[];
  catalogs: Record<string, ProviderModelCatalog>;
  /** The first `accounts_list` has been applied. */
  loaded: boolean;
  lastError: BlueyError | null;
  /** From `accounts.changed`. */
  applyAccount(account: ProviderAccount): void;
  /** From `accounts.catalog`. */
  applyCatalog(catalog: ProviderModelCatalog): void;
  load(): Promise<void>;
  /** Start a sign-in; the account comes back `connecting`, completion arrives as an event. */
  connect(providerId: string, options?: AccountConnectOptions): Promise<ProviderAccount | null>;
  importAccount(providerId: string): Promise<ProviderAccount | null>;
  cancelConnect(accountId: string): Promise<ProviderAccount | null>;
  submitCode(accountId: string, code: string): Promise<ProviderAccount | null>;
  disconnect(accountId: string): Promise<boolean>;
  refreshCatalog(accountId: string, force?: boolean): Promise<ProviderModelCatalog | null>;
  probeFingerprint(accountId: string): Promise<FingerprintProbe | null>;
}

function upsert(accounts: ProviderAccount[], account: ProviderAccount): ProviderAccount[] {
  return accounts.some((a) => a.accountId === account.accountId)
    ? accounts.map((a) => (a.accountId === account.accountId ? account : a))
    : [...accounts, account];
}

export const useAccountsStore = create<AccountsStore>((set, get) => {
  /** Run a backend call; on failure record + toast and resolve `fallback`. */
  async function attempt<T, F>(run: () => Promise<T>, fallback: F): Promise<T | F> {
    try {
      const result = await run();
      set({ lastError: null });
      return result;
    } catch (error) {
      const failure = toBlueyError(error, "authentication");
      set({ lastError: failure });
      showErrorToast(failure);
      return fallback;
    }
  }

  return {
    accounts: [],
    catalogs: {},
    loaded: false,
    lastError: null,
    applyAccount: (account) => set((state) => ({ accounts: upsert(state.accounts, account) })),
    applyCatalog: (catalog) =>
      set((state) => ({ catalogs: { ...state.catalogs, [catalog.accountId]: catalog } })),
    load: async () => {
      try {
        const accounts = await bluey.accounts.list();
        const catalogs: Record<string, ProviderModelCatalog> = {};
        await Promise.all(
          accounts
            .filter((account) => account.status.state === "connected")
            .map(async (account) => {
              const catalog = await bluey.accounts.catalog({ accountId: account.accountId });
              if (catalog) catalogs[account.accountId] = catalog;
            }),
        );
        set({ accounts, catalogs, loaded: true, lastError: null });
      } catch (error) {
        set({ lastError: toBlueyError(error, "authentication"), loaded: true });
      }
    },
    connect: (providerId, options) =>
      attempt(async () => {
        const account = await bluey.accounts.connect({ providerId, options });
        get().applyAccount(account);
        return account;
      }, null),
    importAccount: (providerId) =>
      attempt(async () => {
        const account = await bluey.accounts.import({ providerId });
        get().applyAccount(account);
        return account;
      }, null),
    cancelConnect: (accountId) =>
      attempt(async () => {
        const account = await bluey.accounts.cancelConnect({ accountId });
        get().applyAccount(account);
        return account;
      }, null),
    submitCode: (accountId, code) =>
      attempt(async () => {
        const account = await bluey.accounts.submitCode({ accountId, code });
        get().applyAccount(account);
        return account;
      }, null),
    disconnect: (accountId) =>
      attempt(async () => {
        await bluey.accounts.disconnect({ accountId });
        set((state) => {
          const catalogs = { ...state.catalogs };
          delete catalogs[accountId];
          return { catalogs };
        });
        return true;
      }, false),
    refreshCatalog: (accountId, force = false) =>
      attempt(async () => {
        const catalog = await bluey.accounts.refreshCatalog({ accountId, force });
        get().applyCatalog(catalog);
        return catalog;
      }, null),
    probeFingerprint: (accountId) => attempt(() => bluey.accounts.probeFingerprint({ accountId }), null),
  };
});

/** The account serving `providerId`, if any (one per provider in this version). */
export function accountForProvider(accounts: ProviderAccount[], providerId: string): ProviderAccount | undefined {
  return accounts.find((account) => account.providerId === providerId);
}
