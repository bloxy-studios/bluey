import { showErrorToast } from "@/components/ui/toast-store";
import { bluey } from "@/lib/tauri/api";
import { toBlueyError } from "@/lib/types";
import { useAuthStore } from "./auth-store";

/** Sign out: Rust revokes and forgets the tokens; the store follows the returned status. */
export async function signOutEverywhere(): Promise<void> {
  try {
    const status = await bluey.auth.clearSession();
    useAuthStore.getState().applyStatus(status);
  } catch (error) {
    showErrorToast(toBlueyError(error, "authentication"));
  }
}
