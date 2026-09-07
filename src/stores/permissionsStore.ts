import { create } from "zustand";

import { bluey } from "@/lib/tauri/api";
import { toBlueyError, type PermissionKind, type PermissionState } from "@/lib/types";

interface PermissionsStore {
  permissions: PermissionState | null;
  applyRemote(permissions: PermissionState): void;
  load(): Promise<void>;
  request(kind: PermissionKind): Promise<void>;
  openSystemSettings(kind: PermissionKind): Promise<void>;
}

export const usePermissionsStore = create<PermissionsStore>((set) => ({
  permissions: null,
  applyRemote: (permissions) => set({ permissions }),
  load: async () => {
    try {
      set({ permissions: await bluey.permissions.get() });
    } catch (error) {
      console.warn("[permissionsStore] failed to load", toBlueyError(error));
    }
  },
  request: async (kind) => {
    try {
      set({ permissions: await bluey.permissions.request({ kind }) });
    } catch (error) {
      console.warn("[permissionsStore] request failed", toBlueyError(error));
    }
  },
  openSystemSettings: async (kind) => {
    try {
      await bluey.permissions.openSettings({ kind });
    } catch (error) {
      console.warn("[permissionsStore] openSettings failed", toBlueyError(error));
    }
  },
}));
