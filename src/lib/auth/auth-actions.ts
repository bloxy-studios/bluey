import { clerkPublishableKey, useAuthStore } from "./auth-store";
import { getClerkInstance } from "./clerk-instance";
import { clearTokenCache } from "./token-cache";

/** Sign out everywhere: Clerk session (when configured) + stored token. */
export async function signOutEverywhere(): Promise<void> {
  const key = clerkPublishableKey();
  if (key) {
    try {
      await getClerkInstance(key).signOut();
    } catch (error) {
      console.warn("[auth] Clerk signOut failed", error);
    }
  }
  await clearTokenCache();
  useAuthStore.getState().set({ state: "signed_out", user: null });
}
