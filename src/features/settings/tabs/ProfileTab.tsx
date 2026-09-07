import { UserProfile } from "@clerk/react";

import { Card } from "@/components/ui/Card";
import { useAuthStatus } from "@/lib/auth/useAuthStatus";

/** Profile & security: Clerk `<UserProfile />` (bundled UI, dark theme). */
export default function ProfileTab() {
  const { mode, user } = useAuthStatus();

  if (mode !== "clerk") {
    return (
      <Card className="mt-4">
        <h2 className="text-[15px] font-semibold text-fg">Profile</h2>
        <p className="mt-1 text-[13px] text-fg-muted">
          {mode === "dev"
            ? "Developer session — Clerk is not configured, so you are signed in as a local development user."
            : "Sign-in is not configured. Set VITE_CLERK_PUBLISHABLE_KEY to manage your profile here."}
        </p>
        {user ? (
          <div className="mt-4 flex items-center gap-3">
            <div className="flex size-10 items-center justify-center rounded-full bg-bg-tile text-[14px] font-semibold text-fg">
              {(user.firstName ?? user.email ?? "?").charAt(0).toUpperCase()}
            </div>
            <div>
              <div className="text-[14px] font-medium text-fg">
                {[user.firstName, user.lastName].filter(Boolean).join(" ") || "Developer"}
              </div>
              <div className="text-[12.5px] text-fg-muted">{user.email}</div>
            </div>
          </div>
        ) : null}
      </Card>
    );
  }

  return (
    <div className="mt-4 flex justify-center [&_.cl-rootBox]:w-full">
      <UserProfile routing="hash" />
    </div>
  );
}
