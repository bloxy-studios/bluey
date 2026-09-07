import { ChevronDown } from "lucide-react";
import { type ComponentPropsWithRef } from "react";

import { cn } from "@/lib/utils/cn";

export interface SelectOption {
  value: string;
  label: string;
  disabled?: boolean;
}

export interface SelectProps extends Omit<ComponentPropsWithRef<"select">, "children"> {
  options: SelectOption[];
}

/** Native select styled to the design system (reliable inside the WebView). */
export function Select({ className, options, ...props }: SelectProps) {
  return (
    <div className={cn("relative inline-flex", className)}>
      <select
        {...props}
        className={cn(
          "h-9 min-w-[140px] appearance-none rounded-control border border-border bg-bg-elevated",
          "pl-3 pr-8 text-[13px] text-fg outline-none transition-colors",
          "hover:bg-bg-hover focus-visible:border-border-strong",
          "disabled:opacity-50 disabled:pointer-events-none",
        )}
      >
        {options.map((option) => (
          <option key={option.value} value={option.value} disabled={option.disabled}>
            {option.label}
          </option>
        ))}
      </select>
      <ChevronDown className="pointer-events-none absolute right-2.5 top-1/2 size-3.5 -translate-y-1/2 text-fg-muted" />
    </div>
  );
}
