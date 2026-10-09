import { KeyRound, Lock, UserRound } from "lucide-react";
import { useCallback, useEffect, useState } from "react";

import { Button } from "@/components/ui/Button";
import { ConfirmDialog } from "@/components/ui/Dialog";
import { SectionHeader } from "@/components/ui/SectionHeader";
import { SettingRow } from "@/components/ui/SettingRow";
import { showErrorToast, showToast } from "@/components/ui/toast-store";
import { bluey } from "@/lib/tauri/api";
import { toBlueyError, type CredentialHealth } from "@/lib/types";

const STATE_COPY: Record<CredentialHealth["state"], string> = {
  present: "Saved",
  locked: "Locked — macOS wants your OK before this version of Bluey reads it",
  absent: "Not set",
};

const CATEGORY_COPY: Record<CredentialHealth["category"], string> = {
  provider_key: "API key",
  research_key: "Research key",
  agent_key: "Agent key",
  account_tokens: "Subscription sign-in · remove it with Disconnect",
  sign_in: "Bluey account · remove it with Sign out",
  unknown: "Saved item",
};

/**
 * Settings → Privacy → Saved credentials: every Bluey-owned Keychain item by
 * name and state — never a value. "Allow access" is the one deliberate
 * Keychain prompt; API keys can be removed here, sign-in and subscription
 * tokens go through Sign out and Disconnect.
 */
export function SavedCredentials() {
  const [items, setItems] = useState<CredentialHealth[] | null>(null);
  const [busyKey, setBusyKey] = useState<string | null>(null);
  const [removing, setRemoving] = useState<CredentialHealth | null>(null);

  const load = useCallback(async () => {
    try {
      setItems(await bluey.secrets.health());
    } catch (error) {
      setItems([]);
      showErrorToast(toBlueyError(error, "storage"));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const allowAccess = async (item: CredentialHealth) => {
    setBusyKey(item.key);
    try {
      await bluey.secrets.allowAccess({ key: item.key });
    } catch (error) {
      showErrorToast(toBlueyError(error, "storage"));
    } finally {
      setBusyKey(null);
      await load();
    }
  };

  const remove = async (item: CredentialHealth) => {
    try {
      await bluey.secrets.delete({ key: item.key });
      showToast("Key removed");
    } catch (error) {
      showErrorToast(toBlueyError(error, "storage"));
    } finally {
      await load();
    }
  };

  return (
    <>
      <SectionHeader
        title="Saved credentials"
        description="Stored in the macOS Keychain — Bluey never shows them"
      />
      {items && items.length === 0 ? (
        <p className="mb-4 text-[13px] text-fg-muted">Nothing saved yet.</p>
      ) : null}
      <ul className="mb-4 flex flex-col" aria-label="Saved credentials">
        {items?.map((item) => (
          <li key={item.key}>
            <SettingRow
              icon={item.state === "locked" ? Lock : item.category === "sign_in" ? UserRound : KeyRound}
              title={item.label}
              description={`${CATEGORY_COPY[item.category]} · ${STATE_COPY[item.state]}`}
            >
              <div className="flex items-center gap-2">
                {item.state === "locked" ? (
                  <Button size="sm" onClick={() => void allowAccess(item)} disabled={busyKey === item.key}>
                    Allow access
                  </Button>
                ) : null}
                {item.removable ? (
                  <Button variant="ghost" size="sm" onClick={() => setRemoving(item)}>
                    Remove
                  </Button>
                ) : null}
              </div>
            </SettingRow>
          </li>
        ))}
      </ul>
      <ConfirmDialog
        open={removing !== null}
        onOpenChange={(open) => {
          if (!open) setRemoving(null);
        }}
        title={`Remove the ${removing?.label ?? ""} key?`}
        description="Bluey deletes it from the macOS Keychain. You can paste a new key any time."
        confirmLabel="Remove key"
        onConfirm={async () => {
          if (removing) await remove(removing);
        }}
      />
    </>
  );
}
