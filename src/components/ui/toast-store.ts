import { create } from "zustand";

import { presentError, setRecoveryFailureReporter, type ErrorPresenterOptions } from "@/lib/errors/present";
import type { BlueyError } from "@/lib/types";
import { createId } from "@/lib/utils/id";

export type ToastVariant = "info" | "error";

export interface ToastAction {
  label: string;
  run: () => void | Promise<void>;
}

export interface ToastItem {
  id: string;
  message: string;
  /** Bold first line (error toasts). */
  title?: string;
  variant: ToastVariant;
  action?: ToastAction;
}

export interface ToastInput {
  message: string;
  title?: string;
  variant?: ToastVariant;
  action?: ToastAction;
  durationMs?: number;
  /** Dedupe key: a toast with the same key replaces the one already showing. */
  key?: string;
}

interface ToastStore {
  toasts: ToastItem[];
  /**
   * Surfaces that show error toasts inline (the HUD's notice row) instead of
   * as overlays: while one is mounted, the overlay host renders info toasts only (UX-013).
   */
  inlineErrorHosts: number;
  push(input: string | ToastInput, durationMs?: number): string;
  dismiss(id: string): void;
  /** Mount an inline error host; returns its release. */
  claimInlineErrors(): () => void;
}

const DEFAULT_DURATION_MS = 1200;
const ERROR_DURATION_MS = 6000;
const ACTIONABLE_ERROR_DURATION_MS = 12_000;

const timers = new Map<string, ReturnType<typeof setTimeout>>();
const keys = new Map<string, string>();

export const useToastStore = create<ToastStore>((set, get) => ({
  toasts: [],
  inlineErrorHosts: 0,
  push: (input, durationMs) => {
    const item: ToastInput = typeof input === "string" ? { message: input, durationMs } : input;
    if (item.key) {
      const existing = keys.get(item.key);
      if (existing) get().dismiss(existing);
    }
    const id = createId("toast");
    const variant = item.variant ?? "info";
    set((state) => ({
      toasts: [
        ...state.toasts,
        { id, message: item.message, title: item.title, variant, action: item.action },
      ],
    }));
    if (item.key) keys.set(item.key, id);
    const duration =
      item.durationMs ??
      (variant === "error"
        ? item.action
          ? ACTIONABLE_ERROR_DURATION_MS
          : ERROR_DURATION_MS
        : DEFAULT_DURATION_MS);
    timers.set(
      id,
      setTimeout(() => get().dismiss(id), duration),
    );
    return id;
  },
  dismiss: (id) => {
    const timer = timers.get(id);
    if (timer) clearTimeout(timer);
    timers.delete(id);
    for (const [key, value] of keys) {
      if (value === id) keys.delete(key);
    }
    set((state) => ({ toasts: state.toasts.filter((t) => t.id !== id) }));
  },
  claimInlineErrors: () => {
    set((state) => ({ inlineErrorHosts: state.inlineErrorHosts + 1 }));
    let released = false;
    return () => {
      if (released) return;
      released = true;
      set((state) => ({ inlineErrorHosts: Math.max(0, state.inlineErrorHosts - 1) }));
    };
  },
}));

/** Fire a transient confirmation toast ("Copied"). */
export function showToast(message: string, durationMs?: number): void {
  useToastStore.getState().push(message, durationMs);
}

/**
 * Surface a `BlueyError` as a toast with its single recovery button. Errors of
 * the same code replace each other instead of stacking; cancellations are silent.
 */
export function showErrorToast(error: BlueyError, options: ErrorPresenterOptions = {}): void {
  if (error.kind === "cancelled") return;
  const presented = presentError(error, options);
  const store = useToastStore.getState();
  store.push({
    title: presented.title,
    message: presented.message,
    variant: "error",
    key: `error:${error.code}`,
    action: presented.action
      ? {
          label: presented.actionLabel ?? "Fix",
          run: presented.action,
        }
      : undefined,
  });
}

// A recovery button that fails (Reconnect, Restart helper…) says why, as a toast (UX-036).
setRecoveryFailureReporter((error) => showErrorToast(error));
