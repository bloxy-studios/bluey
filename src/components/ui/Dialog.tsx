import * as Radix from "@radix-ui/react-dialog";
import { X } from "lucide-react";
import { useState, type ReactNode } from "react";

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
  return (
    <Radix.Root open={open} onOpenChange={onOpenChange}>
      <Radix.Portal>
        <Radix.Overlay className="fixed inset-0 z-40 bg-black/55 motion-safe:animate-fade-in" />
        <Radix.Content
          className={cn(
            "fixed left-1/2 top-1/2 z-50 w-[420px] max-w-[calc(100vw-48px)] -translate-x-1/2 -translate-y-1/2",
            "rounded-card border border-border bg-bg-elevated p-5 shadow-[0_16px_48px_rgba(0,0,0,0.5)]",
            "motion-safe:animate-rise-in",
            className,
          )}
        >
          <div className="flex items-start justify-between gap-4">
            <Radix.Title className="text-[15px] font-semibold text-fg">{title}</Radix.Title>
            <Radix.Close asChild>
              <IconButton aria-label="Close" variant="plain" size="sm">
                <X className="size-4" />
              </IconButton>
            </Radix.Close>
          </div>
          {description ? (
            <Radix.Description className="mt-1.5 text-[13px] leading-relaxed text-fg-muted">
              {description}
            </Radix.Description>
          ) : null}
          {children ? <div className="mt-4">{children}</div> : null}
          {footer ? <div className="mt-5 flex justify-end gap-2">{footer}</div> : null}
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
