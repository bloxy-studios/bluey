import { ExternalLink, KeyRound } from "lucide-react";
import { useEffect, useState } from "react";

import { Button } from "@/components/ui/Button";
import { Input } from "@/components/ui/Input";
import { showErrorToast, showToast } from "@/components/ui/toast-store";
import { bluey } from "@/lib/tauri/api";
import { toBlueyError } from "@/lib/types";
import { cn } from "@/lib/utils/cn";

export interface SecretKeyFieldProps {
  /** Keychain key (see SECRET_KEYS in commands.ts). */
  secretKey: string;
  placeholder?: string;
  className?: string;
  /** Optional "where do I get one" link rendered under the field while editing. */
  help?: { label: string; url: string } | null;
  /** Called after a key was stored (or replaced). */
  onSaved?: () => void;
  "aria-label": string;
}

/**
 * Write-only API-key field: saves through `bluey.secrets.set` and afterwards
 * only ever shows "Key saved ••••" — the key is never re-displayed. Failures
 * surface as error toasts, never as silent console output.
 */
export function SecretKeyField({
  secretKey,
  placeholder = "API key",
  className,
  help,
  onSaved,
  ...aria
}: SecretKeyFieldProps) {
  const [saved, setSaved] = useState<boolean | null>(null);
  const [editing, setEditing] = useState(false);
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let alive = true;
    setSaved(null);
    setEditing(false);
    setValue(""); // never carry a typed key over to another secret
    void bluey.secrets
      .has({ key: secretKey })
      .then((has) => {
        if (alive) setSaved(has);
      })
      .catch((error: unknown) => {
        if (!alive) return;
        setSaved(false);
        showErrorToast(toBlueyError(error, "storage"));
      });
    return () => {
      alive = false;
    };
  }, [secretKey]);

  const save = async () => {
    const trimmed = value.trim();
    if (!trimmed || busy) return;
    setBusy(true);
    try {
      await bluey.secrets.set({ key: secretKey, value: trimmed });
      setSaved(true);
      setEditing(false);
      setValue("");
      showToast("Key saved");
      onSaved?.();
    } catch (error) {
      showErrorToast(toBlueyError(error, "storage"));
    } finally {
      setBusy(false);
    }
  };

  if (saved && !editing) {
    return (
      <div className={cn("flex items-center gap-2", className)}>
        <span className="flex h-9 items-center gap-2 rounded-control border border-border bg-bg-tile px-3 text-[13px] text-fg-muted">
          <KeyRound className="size-3.5" aria-hidden />
          Key saved ••••
        </span>
        <Button variant="ghost" size="sm" onClick={() => setEditing(true)}>
          Replace
        </Button>
      </div>
    );
  }

  return (
    <div className={cn("flex flex-col gap-1.5", className)}>
      <div className="flex items-center gap-2">
        <Input
          type="password"
          autoComplete="off"
          value={value}
          placeholder={placeholder}
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !busy) void save();
          }}
          className="w-[240px]"
          {...aria}
        />
        <Button
          variant="secondary"
          size="md"
          onClick={() => void save()}
          disabled={busy || value.trim().length === 0}
        >
          Save
        </Button>
        {saved && editing ? (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => {
              setEditing(false);
              setValue(""); // drop the plaintext draft with the edit
            }}
          >
            Cancel
          </Button>
        ) : null}
      </div>
      {help ? (
        <a
          href={help.url}
          target="_blank"
          rel="noopener noreferrer"
          className="flex items-center gap-1 text-[12px] text-accent hover:text-accent-hover"
        >
          {help.label} <ExternalLink className="size-3" aria-hidden />
        </a>
      ) : null}
    </div>
  );
}
