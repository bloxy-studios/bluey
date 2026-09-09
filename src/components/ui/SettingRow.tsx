import { type LucideIcon } from "lucide-react";
import { type ReactNode } from "react";

import { cn } from "@/lib/utils/cn";

export interface SettingRowProps {
  icon: LucideIcon;
  title: string;
  description?: string;
  /** Right-aligned control (Switch, Select, Button…). */
  children?: ReactNode;
  className?: string;
}

/**
 * Settings row: 48×48 icon tile, 12px gap, 14px title + 13px muted
 * description, control right-aligned. 68px tall, no borders.
 */
export function SettingRow({ icon: Icon, title, description, children, className }: SettingRowProps) {
  return (
    <div className={cn("flex min-h-[68px] items-center gap-3 py-2", className)}>
      <div className="flex size-12 shrink-0 items-center justify-center rounded-card bg-bg-tile">
        <Icon className="size-5 text-fg-muted" strokeWidth={1.8} aria-hidden />
      </div>
      <div className="min-w-0 flex-1">
        <div className="text-[14px] font-medium text-fg [overflow-wrap:anywhere]">{title}</div>
        {description ? (
          <div className="mt-0.5 text-[13px] leading-snug text-fg-muted [overflow-wrap:anywhere]">
            {description}
          </div>
        ) : null}
      </div>
      {children ? (
        <div className="flex min-w-0 max-w-[45%] shrink-0 flex-wrap items-center justify-end gap-2 [&>*]:max-w-full">
          {children}
        </div>
      ) : null}
    </div>
  );
}
