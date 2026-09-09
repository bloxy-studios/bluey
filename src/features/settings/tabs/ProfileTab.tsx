import { ExternalLink, LogOut } from "lucide-react";
import { useState } from "react";

import { Button } from "@/components/ui/Button";
import { Card } from "@/components/ui/Card";
import { showErrorToast } from "@/components/ui/toast-store";
import { signOutEverywhere } from "@/lib/auth/auth-actions";
import { useAuthStatus } from "@/lib/auth/useAuthStatus";
import { bluey } from "@/lib/tauri/api";
import { toBlueyError } from "@/lib/types";

/**
 * Profile & security. The account itself lives on Clerk's hosted Account
 * Portal (opened in the browser); Bluey shows the identity it holds and offers
 * sign-out.
 */
export default function ProfileTab() {
  const { mode, user } = useAuthStatus();
  const [busy, setBusy] = useState(false);

  const manage = async () => {
    try {
      await bluey.auth.openAccountPortal();
    } catch (error) {
      showErrorToast(toBlueyError(error, "authentication"));
    }
  };

  const signOut = async () => {
    setBusy(true);
    try {
      await signOutEverywhere();
    } finally {
      setBusy(false);
    }
  };

  const displayName = [user?.firstName, user?.lastName].filter(Boolean).join(" ") || user?.email || "Signed in";
  const initial = (user?.firstName ?? user?.email ?? "?").charAt(0).toUpperCase();

  return (
    <Card className="mt-4">
      <h2 className="text-[15px] font-semibold text-fg">Profile</h2>
      <p className="mt-1 text-[13px] text-fg-muted">
        {mode === "browser"
          ? "You signed in through your browser. Email, password and connected accounts are managed on your Clerk account page."
          : mode === "dev"
            ? "Developer session — sign-in is not configured, so you are signed in as a local development user."
            : "Sign-in is not configured. Set VITE_CLERK_PUBLISHABLE_KEY and BLUEY_CLERK_OAUTH_CLIENT_ID to sign in through your browser."}
      </p>
      {user ? (
        <div className="mt-4 flex items-center gap-3">
          {user.imageUrl ? (
            <img src={user.imageUrl} alt="" className="size-10 rounded-full object-cover" />
          ) : (
            <div className="flex size-10 items-center justify-center rounded-full bg-bg-tile text-[14px] font-semibold text-fg">
              {initial}
            </div>
          )}
          <div className="min-w-0">
            <div className="truncate text-[14px] font-medium text-fg">{displayName}</div>
            {user.email ? <div className="truncate text-[12.5px] text-fg-muted">{user.email}</div> : null}
          </div>
        </div>
      ) : null}
      {mode === "browser" ? (
        <div className="mt-5 flex flex-wrap gap-2">
          <Button variant="secondary" size="sm" onClick={() => void manage()}>
            <ExternalLink className="size-3.5" aria-hidden /> Manage account
          </Button>
          <Button variant="danger" size="sm" disabled={busy} onClick={() => void signOut()}>
            <LogOut className="size-3.5" aria-hidden /> Sign out
          </Button>
        </div>
      ) : null}
    </Card>
  );
}
