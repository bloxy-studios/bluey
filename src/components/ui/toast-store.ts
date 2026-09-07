import { create } from "zustand";

import { createId } from "@/lib/utils/id";

export interface ToastItem {
  id: string;
  message: string;
}

interface ToastStore {
  toasts: ToastItem[];
  push(message: string, durationMs?: number): void;
  dismiss(id: string): void;
}

export const useToastStore = create<ToastStore>((set) => ({
  toasts: [],
  push: (message, durationMs = 1200) => {
    const id = createId("toast");
    set((state) => ({ toasts: [...state.toasts, { id, message }] }));
    setTimeout(() => {
      set((state) => ({ toasts: state.toasts.filter((t) => t.id !== id) }));
    }, durationMs);
  },
  dismiss: (id) => set((state) => ({ toasts: state.toasts.filter((t) => t.id !== id) })),
}));

/** Fire a transient confirmation toast ("Copied"). */
export function showToast(message: string, durationMs?: number): void {
  useToastStore.getState().push(message, durationMs);
}
