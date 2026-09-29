import { useEffect } from "react";

import { usePermissionsStore } from "@/stores/permissionsStore";

/** How often an open permissions screen re-reads macOS permission state (ONB-002). */
export const PERMISSIONS_POLL_MS = 2000;

/**
 * Keep permission badges live while the calling screen is mounted. macOS sends nothing when the
 * user grants access in System Settings, so `permissions_get` (which re-checks every permission)
 * runs on mount, whenever the window regains focus, and every {@link PERMISSIONS_POLL_MS} while
 * the window is visible.
 */
export function useLivePermissions(): void {
  useEffect(() => {
    const load = () => void usePermissionsStore.getState().load();
    const loadIfVisible = () => {
      if (document.visibilityState === "visible") load();
    };
    load();
    const timer = setInterval(loadIfVisible, PERMISSIONS_POLL_MS);
    window.addEventListener("focus", load);
    document.addEventListener("visibilitychange", loadIfVisible);
    return () => {
      clearInterval(timer);
      window.removeEventListener("focus", load);
      document.removeEventListener("visibilitychange", loadIfVisible);
    };
  }, []);
}
