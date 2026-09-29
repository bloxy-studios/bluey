import type { BlueyResponse } from "@/lib/types";
import { useAccountsStore } from "@/stores/accountsStore";
import { useSettingsStore } from "@/stores/settingsStore";
import { describeProvenance } from "./provenance";

/** The muted provenance line under a finished answer's actions. */
export function ResponseProvenance({ response }: { response: BlueyResponse }) {
  const providers = useSettingsStore((s) => s.settings?.ai.providers);
  const accounts = useAccountsStore((s) => s.accounts);
  const provenance = describeProvenance(
    response,
    providers ?? [],
    accounts.map((a) => a.providerId),
  );
  if (!provenance) return null;
  return (
    <p data-testid="response-provenance" className="m-0 mt-1.5 truncate px-1 text-[11px] text-fg-subtle">
      {provenance.text}
      {provenance.marker ? (
        <>
          {" · "}
          <span title={provenance.detail}>{provenance.marker}</span>
        </>
      ) : null}
    </p>
  );
}
