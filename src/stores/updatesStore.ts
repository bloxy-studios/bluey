/**
 * In-app updates (docs/UPDATES.md). Rust owns the cycle and publishes every
 * transition as `update.status`; this store mirrors it for the HUD pill and
 * Settings → General → Updates and exposes the three user actions.
 */

import { create } from "zustand";

import { showErrorToast } from "@/components/ui/toast-store";
import { bluey } from "@/lib/tauri/api";
import { toBlueyError, type UpdateStatus } from "@/lib/types";

interface UpdatesStore {
  status: UpdateStatus | null;
  /** A check or install command is in flight from this window. */
  busy: boolean;
  /** From `update.status` events (single source of truth: the backend). */
  applyRemote(status: UpdateStatus): void;
  load(): Promise<void>;
  /** Check the current channel now (automatic mode also downloads + installs). */
  check(): Promise<void>;
  /** Download, verify and install the update the last check found. */
  install(): Promise<void>;
  /** Restart into an installed update. */
  relaunch(): Promise<void>;
}

export const useUpdatesStore = create<UpdatesStore>((set, get) => ({
  status: null,
  busy: false,
  applyRemote: (status) => set({ status }),
  load: async () => {
    try {
      set({ status: await bluey.updates.getStatus() });
    } catch (error) {
      console.warn("[updatesStore] failed to load status", toBlueyError(error));
    }
  },
  check: async () => {
    if (get().busy) return;
    set({ busy: true });
    try {
      set({ status: await bluey.updates.check() });
    } catch (error) {
      showErrorToast(toBlueyError(error));
    } finally {
      set({ busy: false });
    }
  },
  install: async () => {
    if (get().busy) return;
    set({ busy: true });
    try {
      set({ status: await bluey.updates.install() });
    } catch (error) {
      showErrorToast(toBlueyError(error));
    } finally {
      set({ busy: false });
    }
  },
  relaunch: async () => {
    try {
      await bluey.updates.relaunch();
    } catch (error) {
      showErrorToast(toBlueyError(error));
    }
  },
}));
