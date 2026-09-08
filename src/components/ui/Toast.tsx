import { AlertCircle, X } from "lucide-react";

import { cn } from "@/lib/utils/cn";
import { useToastStore, type ToastItem } from "./toast-store";

function ErrorToast({ toast }: { toast: ToastItem }) {
  const dismiss = useToastStore((state) => state.dismiss);
  return (
    <div
      role="alert"
      className={cn(
        "pointer-events-auto flex w-[360px] max-w-[calc(100vw-32px)] items-start gap-2.5 rounded-card border border-danger/30",
        "bg-tooltip-bg px-3.5 py-2.5 text-white shadow-lg shadow-black/30 motion-safe:animate-rise-in",
      )}
    >
      <AlertCircle className="mt-0.5 size-4 shrink-0 text-danger" aria-hidden />
      <div className="min-w-0 flex-1">
        {toast.title ? <div className="text-[13px] font-medium">{toast.title}</div> : null}
        <p className="m-0 mt-0.5 text-[12.5px] leading-snug text-white/70">{toast.message}</p>
        {toast.action ? (
          <button
            type="button"
            className="mt-1.5 text-[12.5px] font-medium text-accent hover:text-accent-hover"
            onClick={() => {
              dismiss(toast.id);
              void toast.action?.run();
            }}
          >
            {toast.action.label}
          </button>
        ) : null}
      </div>
      <button
        type="button"
        aria-label="Dismiss"
        onClick={() => dismiss(toast.id)}
        className="-mr-1 -mt-0.5 flex size-6 shrink-0 items-center justify-center rounded-[6px] text-white/60 hover:bg-white/10 hover:text-white"
      >
        <X className="size-3.5" aria-hidden />
      </button>
    </div>
  );
}

/** Toast host: fixed bottom-center. Transient confirmations ("Copied") and error toasts with a recovery link. */
export function Toasts() {
  const toasts = useToastStore((state) => state.toasts);
  if (toasts.length === 0) return null;
  return (
    <div className="pointer-events-none fixed bottom-6 left-1/2 z-[60] flex -translate-x-1/2 flex-col items-center gap-2">
      {toasts.map((toast) =>
        toast.variant === "error" ? (
          <ErrorToast key={toast.id} toast={toast} />
        ) : (
          <div
            key={toast.id}
            role="status"
            className="rounded-full bg-tooltip-bg px-3.5 py-1.5 text-[13px] font-medium text-white shadow-lg shadow-black/30 motion-safe:animate-rise-in"
          >
            {toast.message}
          </div>
        ),
      )}
    </div>
  );
}
