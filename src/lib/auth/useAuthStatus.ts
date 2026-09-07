import { useAuthStore, type AuthSnapshot } from "./auth-store";

/** Current auth snapshot (mode + state + user), independent of Clerk context. */
export function useAuthStatus(): AuthSnapshot {
  const mode = useAuthStore((s) => s.mode);
  const state = useAuthStore((s) => s.state);
  const user = useAuthStore((s) => s.user);
  return { mode, state, user };
}
