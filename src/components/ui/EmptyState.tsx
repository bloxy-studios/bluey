import { type LucideIcon } from "lucide-react";
import { type ReactNode } from "react";

import { cn } from "@/lib/utils/cn";

export interface EmptyStateProps {
  icon?: LucideIcon;
  title: string;
  description?: string;
  action?: ReactNode;
  className?: string;
}

export function EmptyState({ icon: Icon, title, description, action, className }: EmptyStateProps) {
  return (
    <div className={cn("flex flex-col items-center justify-center gap-2 px-6 py-12 text-center", className)}>
      {Icon ? (
        <div className="mb-1 flex size-12 items-center justify-center rounded-card bg-bg-tile">
          <Icon className="size-5 text-fg-subtle" strokeWidth={1.8} aria-hidden />
        </div>
      ) : null}
      <div className="text-[14px] font-medium text-fg">{title}</div>
      {description ? <p className="max-w-[360px] text-[13px] leading-relaxed text-fg-muted">{description}</p> : null}
      {action ? <div className="mt-2">{action}</div> : null}
    </div>
  );
}
