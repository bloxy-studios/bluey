import { AlertCircle, X } from "lucide-react";
import { useEffect } from "react";

import { useToastStore } from "@/components/ui/toast-store";
import { runRecovery } from "@/lib/errors/present";

interface NoticeProps {
  title: string;
  message: string;
  actionLabel?: string;
  onAction?: () => void;
  onDismiss: () => void;
}

function NoticeRow({ title, message, actionLabel, onAction, onDismiss }: NoticeProps) {
  return (
    <div
      role="alert"
      className="flex shrink-0 items-center gap-2 border-t border-hud-border px-4 py-2 text-[12.5px] motion-safe:animate-fade-in"
    >
      <AlertCircle className="size-3.5 shrink-0 text-danger" aria-hidden />
      <p className="m-0 min-w-0 flex-1 truncate text-fg-muted" title={`${title} — ${message}`}>
        <span className="font-medium text-fg">{title}</span> · {message}
      </p>
      {actionLabel && onAction ? (
        <button
          type="button"
          onClick={onAction}
          className="shrink-0 rounded-full bg-hud-chip px-2.5 py-0.5 text-[12px] font-medium text-fg transition-colors hover:bg-fg/15"
        >
          {actionLabel}
        </button>
      ) : null}
      <button
        type="button"
        aria-label="Dismiss"
        onClick={onDismiss}
        className="flex size-6 shrink-0 items-center justify-center rounded-full text-fg-muted transition-colors hover:bg-fg/10 hover:text-fg"
      >
        <X className="size-3.5" aria-hidden />
      </button>
    </div>
  );
}

/**
 * The HUD's one inline notice row, above the toolbar and inside the measured
 * frame: the newest error with its recovery. An overlay toast in the auto-sized
 * HUD window covered the toolbar and clipped when stacked (UX-013).
 */
export function HudNotice() {
  const claimInlineErrors = useToastStore((s) => s.claimInlineErrors);
  const error = useToastStore((s) => s.toasts.findLast((toast) => toast.variant === "error"));
  const dismiss = useToastStore((s) => s.dismiss);

  useEffect(() => claimInlineErrors(), [claimInlineErrors]);

  if (!error) return null;
  const action = error.action;
  return (
    <NoticeRow
      title={error.title ?? "Something went wrong"}
      message={error.message}
      actionLabel={action?.label}
      onAction={
        action
          ? () => {
              dismiss(error.id);
              void runRecovery(action.run);
            }
          : undefined
      }
      onDismiss={() => dismiss(error.id)}
    />
  );
}
