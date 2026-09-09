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
export function Select({ className, options, title, value, ...props }: SelectProps) {
  const selectedLabel = options.find((option) => option.value === String(value))?.label;
  return (
    <div className={cn("relative inline-flex w-[200px] min-w-0 max-w-full", className)}>
      <select
        {...props}
        value={value}
        title={title ?? selectedLabel}
        className={cn(
          "h-9 w-full min-w-0 max-w-full appearance-none rounded-control border border-border bg-bg-elevated",
          "truncate pl-3 pr-8 text-[13px] text-fg outline-none transition-colors",
          "scheme-dark [:root[data-theme=light]_&]:scheme-light",
          "hover:bg-bg-hover focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40",
          "disabled:opacity-50 disabled:pointer-events-none",
        )}
      >
        {options.map((option) => (
          <option key={option.value} value={option.value} disabled={option.disabled}>
            {option.label}
          </option>
        ))}
      </select>
      <ChevronDown
        className="pointer-events-none absolute right-2.5 top-1/2 size-3.5 -translate-y-1/2 text-fg-muted"
        aria-hidden
      />
    </div>
  );
}
