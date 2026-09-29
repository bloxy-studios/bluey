import { create } from "zustand";

import { bluey } from "@/lib/tauri/api";
import { getTransport } from "@/lib/tauri/transport";
import { toBlueyError, type AuthState, type AuthStatus, type AuthUser, type BlueyError } from "@/lib/types";

export type AuthMode =
  /** Sign-in is configured: the browser flow (Clerk OAuth + deep link / loopback). */
  | "browser"
  /** Not configured, MockTransport: auto-signed-in developer user. */
  | "dev"
  /** Not configured on a real build: show the configuration screen. */
  | "unconfigured";

export interface AuthSnapshot {
  mode: AuthMode;
  state: AuthState;
  user: AuthUser | null;
  /** A browser sign-in is in flight (waiting for the redirect). */
  signInPending: boolean;
}

interface AuthStore extends AuthSnapshot {
  /** The first `auth_get_status` (or `auth.changed`) has been applied. */
  loaded: boolean;
  /**
   * The last `auth_get_status` failed and no status has arrived since. The gate then offers a
   * retry — never the developer configuration screen, which only a real `configured: false` shows.
   */
  statusError: BlueyError | null;
  applyStatus(status: AuthStatus): void;
  load(): Promise<void>;
  set(snapshot: Partial<AuthSnapshot>): void;
}

export const DEV_USER: AuthUser = {
  id: "dev-user",
  email: "dev@bluey.local",
  firstName: "Dev",
  lastName: "Bluey",
};

export function resolveAuthMode(configured: boolean, transportKind: "tauri" | "mock"): AuthMode {
  if (configured) return "browser";
  return transportKind === "mock" ? "dev" : "unconfigured";
}

/** Rust's status → what the UI shows (dev mode is always signed in). */
export function snapshotFromStatus(status: AuthStatus, transportKind: "tauri" | "mock"): AuthSnapshot {
  const mode = resolveAuthMode(status.configured, transportKind);
  if (mode === "dev") return { mode, state: "signed_in", user: status.user ?? DEV_USER, signInPending: false };
  if (mode === "unconfigured") return { mode, state: "signed_out", user: null, signInPending: false };
  return { mode, state: status.state, user: status.user ?? null, signInPending: status.signInPending };
}

export const useAuthStore = create<AuthStore>((set) => ({
  mode: "unconfigured",
  state: "unknown",
  user: null,
  signInPending: false,
  loaded: false,
  statusError: null,
  set: (snapshot) => set(snapshot),
  applyStatus: (status) =>
    set({ ...snapshotFromStatus(status, getTransport().kind), loaded: true, statusError: null }),
  load: async () => {
    try {
      const status = await bluey.auth.getStatus();
      set({ ...snapshotFromStatus(status, getTransport().kind), loaded: true, statusError: null });
    } catch (error) {
      console.warn("[auth] status unavailable", error);
      set({ loaded: true, statusError: toBlueyError(error) });
    }
  },
}));
