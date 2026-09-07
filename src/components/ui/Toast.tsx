import { useToastStore } from "./toast-store";

/** Toast host: fixed bottom-center, transient confirmations ("Copied"). */
export function Toasts() {
  const toasts = useToastStore((state) => state.toasts);
  if (toasts.length === 0) return null;
  return (
    <div className="pointer-events-none fixed bottom-6 left-1/2 z-[60] flex -translate-x-1/2 flex-col items-center gap-2">
      {toasts.map((toast) => (
        <div
          key={toast.id}
          role="status"
          className="rounded-full bg-tooltip-bg px-3.5 py-1.5 text-[13px] font-medium text-white shadow-lg shadow-black/30 motion-safe:animate-rise-in"
        >
          {toast.message}
        </div>
      ))}
    </div>
  );
}
