import { Copy, ExternalLink } from "lucide-react";
import { useState } from "react";

import { Button } from "@/components/ui/Button";
import { ErrorBanner } from "@/components/ui/ErrorBanner";
import { Spinner } from "@/components/ui/Spinner";
import { showToast } from "@/components/ui/toast-store";
import { bluey } from "@/lib/tauri/api";
import { toBlueyError, type BlueyError, type SignInStart } from "@/lib/types";
import { copyText } from "@/lib/utils/clipboard";
import { useAuthStatus } from "./useAuthStatus";

export interface BrowserSignInProps {
  title?: string;
  body?: string;
}

/**
 * The browser sign-in card (ADR 0008): one button opens the default browser on
 * Clerk's sign-in page; Bluey waits for the redirect (`bluey://auth/callback`
 * or the loopback listener) and flips to signed-in through `auth.changed`.
 */
export function BrowserSignIn({
  title = "Sign in to Bluey",
  body = "Bluey opens your browser to sign in — come back here when you're done.",
}: BrowserSignInProps) {
  const { signInPending } = useAuthStatus();
  const [start, setStart] = useState<SignInStart | null>(null);
  const [error, setError] = useState<BlueyError | null>(null);
  const [busy, setBusy] = useState(false);

  const begin = async () => {
    setBusy(true);
    setError(null);
    try {
      setStart(await bluey.auth.beginSignIn());
    } catch (err) {
      setError(toBlueyError(err, "authentication"));
    } finally {
      setBusy(false);
    }
  };

  const cancel = async () => {
    try {
      await bluey.auth.cancelSignIn();
    } catch (err) {
      setError(toBlueyError(err, "authentication"));
    }
    setStart(null);
  };

  const copyLink = async () => {
    if (start && (await copyText(start.url))) showToast("Sign-in link copied");
  };

  return (
    <div className="w-[440px] max-w-full rounded-card border border-border bg-bg-elevated p-6 text-center">
      <h1 className="text-[17px] font-semibold text-fg">{title}</h1>
      <p className="mt-1.5 text-[13px] leading-relaxed text-fg-muted">{body}</p>
      {error ? <ErrorBanner error={error} onRetry={() => void begin()} compact className="mt-4 text-left" /> : null}
      {signInPending ? (
        <div className="mt-5 flex flex-col items-center gap-3">
          <div className="flex items-center gap-2 text-[13px] text-fg-muted" role="status">
            <Spinner size={14} /> Waiting for your browser…
          </div>
          <div className="flex flex-wrap items-center justify-center gap-2">
            <Button variant="secondary" size="sm" disabled={busy} onClick={() => void begin()}>
              Open the sign-in page again
            </Button>
            {start ? (
              <Button variant="ghost" size="sm" onClick={() => void copyLink()}>
                <Copy className="size-3.5" aria-hidden /> Copy link
              </Button>
            ) : null}
            <Button variant="ghost" size="sm" onClick={() => void cancel()}>
              Cancel
            </Button>
          </div>
        </div>
      ) : (
        <Button variant="primary" className="mt-5" disabled={busy} onClick={() => void begin()}>
          <ExternalLink className="size-4" aria-hidden /> Sign in with your browser
        </Button>
      )}
    </div>
  );
}
