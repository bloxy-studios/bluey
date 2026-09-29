import { AlertCircle, X } from "lucide-react";
import { useEffect } from "react";

import { useToastStore } from "@/components/ui/toast-store";
import { presentError, runRecovery } from "@/lib/errors/present";
import { eventBus } from "@/lib/tauri/event-bus";
import type { SnapshotWarning } from "@/lib/types";
import { useHudUiStore } from "@/stores/hudUiStore";

interface NoticeProps {
  /** `alert` for errors; `status` for the quieter "screen not included" warning. */
  role?: "alert" | "status";
  title: string;
  message: string;
  actionLabel?: string;
  onAction?: () => void;
  onDismiss: () => void;
}

function NoticeRow({ role = "alert", title, message, actionLabel, onAction, onDismiss }: NoticeProps) {
  return (
    <div
      role={role}
      className="flex shrink-0 items-center gap-2 border-t border-hud-border px-4 py-2 text-[12.5px] motion-safe:animate-fade-in"
    >
      <AlertCircle
        className={`size-3.5 shrink-0 ${role === "alert" ? "text-danger" : "text-fg-muted"}`}
        aria-hidden
      />
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

/** The fix a snapshot warning offers (Open System Settings → Screen Recording), worded like any error's. */
function warningRecovery(warning: SnapshotWarning) {
  return presentError({
    kind: "permission",
    code: warning.code,
    message: warning.message,
    recoverable: true,
    ...(warning.recovery ? { recovery: warning.recovery } : {}),
  });
}

/** Mirrors the latest snapshot's `screen_unavailable` warning into the HUD (UX-002). */
function useScreenWarning(): void {
  useEffect(
    () =>
      eventBus.on("context.updated", ({ snapshot }) => {
        const warning = snapshot.warnings?.find((w) => w.kind === "screen_unavailable") ?? null;
        useHudUiStore.getState().setScreenWarning(warning);
      }),
    [],
  );
}

/**
 * The HUD's one inline notice row, above the toolbar and inside the measured
 * frame: the newest error with its recovery, else why the last ask went
 * without the screen. An overlay toast in the auto-sized HUD window covered the
 * toolbar and clipped when stacked (UX-013).
 */
export function HudNotice() {
  const claimInlineErrors = useToastStore((s) => s.claimInlineErrors);
  const error = useToastStore((s) => s.toasts.findLast((toast) => toast.variant === "error"));
  const dismiss = useToastStore((s) => s.dismiss);

  const screenWarning = useHudUiStore((s) => s.screenWarning);

  useEffect(() => claimInlineErrors(), [claimInlineErrors]);
  useScreenWarning();

  if (!error && screenWarning) {
    const { actionLabel, action: fix } = warningRecovery(screenWarning);
    const clear = () => useHudUiStore.getState().setScreenWarning(null);
    return (
      <NoticeRow
        role="status"
        title="Screen not included"
        message={screenWarning.message}
        actionLabel={actionLabel}
        onAction={
          fix
            ? () => {
                clear();
                void runRecovery(fix);
              }
            : undefined
        }
        onDismiss={clear}
      />
    );
  }
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
