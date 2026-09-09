import * as Radix from "@radix-ui/react-dialog";
import { X } from "lucide-react";
import { useLayoutEffect, useRef, useState, type ReactNode } from "react";

import { cn } from "@/lib/utils/cn";
import { Button } from "./Button";
import { IconButton } from "./IconButton";

export interface DialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  description?: string;
  children?: ReactNode;
  footer?: ReactNode;
  className?: string;
}

export function Dialog({ open, onOpenChange, title, description, children, footer, className }: DialogProps) {
  const opener = useRef<HTMLElement | null>(null);
  useLayoutEffect(() => {
    if (open) {
      // Capture before the portal mounts: a child's native autoFocus can run before
      // (and cause Radix to skip) onOpenAutoFocus.
      opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    }
  }, [open]);

  return (
    <Radix.Root open={open} onOpenChange={onOpenChange}>
      <Radix.Portal>
        <Radix.Overlay className="fixed inset-0 z-40 bg-black/55 motion-safe:animate-fade-in" />
        <Radix.Content
          onCloseAutoFocus={(event) => {
            // These controlled dialogs have no Radix.Trigger to restore focus to.
            event.preventDefault();
            if (opener.current?.isConnected) opener.current.focus({ preventScroll: true });
          }}
          className={cn(
            "fixed left-1/2 top-1/2 z-50 flex max-h-[calc(100dvh-48px)] w-[420px] max-w-[calc(100vw-48px)] -translate-x-1/2 -translate-y-1/2 flex-col",
            "overflow-hidden rounded-card border border-border bg-bg-elevated p-5 text-fg shadow-[0_16px_48px_rgba(0,0,0,0.5)]",
            "motion-safe:animate-rise-in",
            className,
          )}
          {...(!description ? { "aria-describedby": undefined } : {})}
        >
          <div className="flex shrink-0 items-start justify-between gap-4">
            <Radix.Title className="min-w-0 text-[15px] font-semibold text-fg [overflow-wrap:anywhere]">
              {title}
            </Radix.Title>
            <Radix.Close asChild>
              <IconButton aria-label="Close" variant="plain" size="sm">
                <X className="size-4" />
              </IconButton>
            </Radix.Close>
          </div>
          {description || children ? (
            <div className="-mx-1 min-h-0 overflow-y-auto overscroll-contain px-1 [overflow-wrap:anywhere]">
              {description ? (
                <Radix.Description className="mt-1.5 text-[13px] leading-relaxed text-fg-muted">
                  {description}
                </Radix.Description>
              ) : null}
              {children ? <div className="mt-4 pb-1">{children}</div> : null}
            </div>
          ) : null}
          {footer ? <div className="mt-5 flex shrink-0 flex-wrap justify-end gap-2">{footer}</div> : null}
        </Radix.Content>
      </Radix.Portal>
    </Radix.Root>
  );
}

export interface ConfirmDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  description: string;
  confirmLabel: string;
  destructive?: boolean;
  onConfirm: () => void | Promise<void>;
}

/** Confirmation dialog for destructive actions (Privacy → Data, Sessions → Delete…). */
export function ConfirmDialog({ open, onOpenChange, title, description, confirmLabel, destructive = true, onConfirm }: ConfirmDialogProps) {
  const [busy, setBusy] = useState(false);
  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={title}
      description={description}
      footer={
        <>
          <Button variant="secondary" onClick={() => onOpenChange(false)} disabled={busy}>
            Cancel
          </Button>
          <Button
            variant="primary"
            className={cn(destructive && "bg-danger hover:bg-danger/85")}
            disabled={busy}
            onClick={async () => {
              setBusy(true);
              try {
                await onConfirm();
                onOpenChange(false);
              } finally {
                setBusy(false);
              }
            }}
          >
            {confirmLabel}
          </Button>
        </>
      }
    />
  );
}
