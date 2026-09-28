import { useAuthStore, type AuthSnapshot } from "./auth-store";

/** Current auth snapshot (mode + state + user + pending flag). */
export function useAuthStatus(): AuthSnapshot & { loaded: boolean; statusFailed: boolean } {
  const mode = useAuthStore((s) => s.mode);
  const state = useAuthStore((s) => s.state);
  const user = useAuthStore((s) => s.user);
  const signInPending = useAuthStore((s) => s.signInPending);
  const loaded = useAuthStore((s) => s.loaded);
  const statusFailed = useAuthStore((s) => s.statusError !== null);
  return { mode, state, user, signInPending, loaded, statusFailed };
}
