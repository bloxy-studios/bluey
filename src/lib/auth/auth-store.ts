import { create } from "zustand";

import type { AuthUser } from "@/lib/types";

export type AuthMode =
  /** Clerk is configured (VITE_CLERK_PUBLISHABLE_KEY present). */
  | "clerk"
  /** No key, MockTransport: auto-signed-in developer user. */
  | "dev"
  /** No key on a real build: show the configuration screen. */
  | "unconfigured";

export interface AuthSnapshot {
  mode: AuthMode;
  state: "unknown" | "signed_out" | "signed_in";
  user: AuthUser | null;
}

interface AuthStore extends AuthSnapshot {
  set(snapshot: Partial<AuthSnapshot>): void;
}

export const useAuthStore = create<AuthStore>((set) => ({
  mode: "unconfigured",
  state: "unknown",
  user: null,
  set: (snapshot) => set(snapshot),
}));

export const DEV_USER: AuthUser = {
  id: "dev-user",
  email: "dev@bluey.local",
  firstName: "Dev",
  lastName: "Bluey",
};

export function resolveAuthMode(publishableKey: string | undefined, transportKind: "tauri" | "mock"): AuthMode {
  if (publishableKey) return "clerk";
  return transportKind === "mock" ? "dev" : "unconfigured";
}

export function clerkPublishableKey(): string | undefined {
  const key = import.meta.env.VITE_CLERK_PUBLISHABLE_KEY as string | undefined;
  return key && key.length > 0 ? key : undefined;
}
