import { KeyRound } from "lucide-react";
import { useEffect, useState } from "react";

import { Button } from "@/components/ui/Button";
import { Input } from "@/components/ui/Input";
import { showToast } from "@/components/ui/toast-store";
import { bluey } from "@/lib/tauri/api";
import { cn } from "@/lib/utils/cn";

export interface SecretKeyFieldProps {
  /** Keychain key (see SECRET_KEYS in commands.ts). */
  secretKey: string;
  placeholder?: string;
  className?: string;
  "aria-label": string;
}

/**
 * Write-only API-key field: saves through `bluey.secrets.set` and afterwards
 * only ever shows "Key saved ••••" — the key is never re-displayed.
 */
export function SecretKeyField({ secretKey, placeholder = "API key", className, ...aria }: SecretKeyFieldProps) {
  const [saved, setSaved] = useState<boolean | null>(null);
  const [editing, setEditing] = useState(false);
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let alive = true;
    setSaved(null);
    setEditing(false);
    void bluey.secrets
      .has({ key: secretKey })
      .then((has) => {
        if (alive) setSaved(has);
      })
      .catch(() => {
        if (alive) setSaved(false);
      });
    return () => {
      alive = false;
    };
  }, [secretKey]);

  const save = async () => {
    const trimmed = value.trim();
    if (!trimmed) return;
    setBusy(true);
    try {
      await bluey.secrets.set({ key: secretKey, value: trimmed });
      setSaved(true);
      setEditing(false);
      setValue("");
      showToast("Key saved");
    } catch (error) {
      console.warn("[secrets] save failed", error);
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
    <div className={cn("flex items-center gap-2", className)}>
      <Input
        type="password"
        autoComplete="off"
        value={value}
        placeholder={placeholder}
        onChange={(e) => setValue(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") void save();
        }}
        className="w-[240px]"
        {...aria}
      />
      <Button variant="secondary" size="md" onClick={() => void save()} disabled={busy || value.trim().length === 0}>
        Save
      </Button>
      {saved && editing ? (
        <Button variant="ghost" size="sm" onClick={() => setEditing(false)}>
          Cancel
        </Button>
      ) : null}
    </div>
  );
}
