import { AlertCircle } from "lucide-react";

import { useErrorPresenter, type ErrorPresenterOptions } from "@/hooks/useErrorPresenter";
import type { BlueyError } from "@/lib/types";
import { cn } from "@/lib/utils/cn";
import { Button } from "./Button";

export interface ErrorBannerProps extends ErrorPresenterOptions {
  error: BlueyError;
  className?: string;
  compact?: boolean;
}

/** Friendly error surface with a single recovery button. */
export function ErrorBanner({ error, className, compact, ...presenterOptions }: ErrorBannerProps) {
  const present = useErrorPresenter(presenterOptions);
  const presented = present(error);

  return (
    <div
      role="alert"
      className={cn(
        "flex items-start gap-3 rounded-card border border-danger/25 bg-danger/8 p-3.5",
        compact && "p-2.5",
        className,
      )}
    >
      <AlertCircle className="mt-0.5 size-4 shrink-0 text-danger" aria-hidden />
      <div className="min-w-0 flex-1">
        <div className="text-[13px] font-medium text-fg">{presented.title}</div>
        <p className="mt-0.5 text-[12.5px] leading-snug text-fg-muted">{presented.message}</p>
      </div>
      {presented.action ? (
        <Button size="sm" variant="secondary" onClick={() => void presented.action?.()}>
          {presented.actionLabel}
        </Button>
      ) : null}
    </div>
  );
}
