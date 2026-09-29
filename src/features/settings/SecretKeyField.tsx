import { ExternalLink, KeyRound, Lock } from "lucide-react";
import { useEffect, useState } from "react";

import { Button } from "@/components/ui/Button";
import { ConfirmDialog } from "@/components/ui/Dialog";
import { Input } from "@/components/ui/Input";
import { showErrorToast, showToast } from "@/components/ui/toast-store";
import { bluey } from "@/lib/tauri/api";
import { toBlueyError, type SecretState } from "@/lib/types";
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
 * only ever shows "Key saved ••••" — the key is never re-displayed. A key
 * macOS holds back after an update shows as locked with "Allow access" (the
 * one Keychain prompt, on purpose) rather than as missing. Failures surface as
 * error toasts, never as silent console output.
 */
export function SecretKeyField({
  secretKey,
  placeholder = "API key",
  className,
  help,
  onSaved,
  ...aria
}: SecretKeyFieldProps) {
  const [state, setState] = useState<SecretState | null>(null);
  const [editing, setEditing] = useState(false);
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  const [confirmRemove, setConfirmRemove] = useState(false);

  useEffect(() => {
    let alive = true;
    setState(null);
    setEditing(false);
    setValue(""); // never carry a typed key over to another secret
    void bluey.secrets
      .state({ key: secretKey })
      .then((next) => {
        if (alive) setState(next);
      })
      .catch((error: unknown) => {
        if (!alive) return;
        setState("absent");
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
      setState("present");
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

  const allowAccess = async () => {
    setBusy(true);
    try {
      setState(await bluey.secrets.allowAccess({ key: secretKey }));
    } catch (error) {
      showErrorToast(toBlueyError(error, "storage"));
    } finally {
      setBusy(false);
    }
  };

  const remove = async () => {
    try {
      await bluey.secrets.delete({ key: secretKey });
      setState("absent");
      setEditing(false);
      showToast("Key removed");
    } catch (error) {
      showErrorToast(toBlueyError(error, "storage"));
    }
  };

  const saved = state === "present" || state === "locked";
  if (saved && !editing) {
    const locked = state === "locked";
    return (
      <div className={cn("flex items-center gap-2", className)}>
        <span
          className={cn(
            "flex h-9 items-center gap-2 rounded-control border border-border bg-bg-tile px-3 text-[13px] text-fg-muted",
            locked && "border-border-strong text-fg",
          )}
          title={locked ? "Saved, but macOS wants your OK before this version of Bluey reads it." : undefined}
        >
          {locked ? <Lock className="size-3.5" aria-hidden /> : <KeyRound className="size-3.5" aria-hidden />}
          {locked ? "Key saved · locked" : "Key saved ••••"}
        </span>
        {locked ? (
          <Button variant="secondary" size="sm" onClick={() => void allowAccess()} disabled={busy}>
            Allow access
          </Button>
        ) : null}
        <Button variant="ghost" size="sm" onClick={() => setEditing(true)} disabled={busy}>
          Replace
        </Button>
        <Button variant="ghost" size="sm" onClick={() => setConfirmRemove(true)} disabled={busy}>
          Remove key
        </Button>
        <ConfirmDialog
          open={confirmRemove}
          onOpenChange={setConfirmRemove}
          title="Remove this key?"
          description="Bluey deletes it from the macOS Keychain. You can paste a new key any time."
          confirmLabel="Remove key"
          onConfirm={remove}
        />
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
