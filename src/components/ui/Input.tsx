import { type ComponentPropsWithRef } from "react";

import { cn } from "@/lib/utils/cn";

export type InputProps = ComponentPropsWithRef<"input">;

export function Input({ className, ...props }: InputProps) {
  return (
    <input
      {...props}
      className={cn(
        "h-9 min-w-0 max-w-full rounded-control border border-border bg-bg-elevated px-3 text-[13px] text-fg",
        "placeholder:text-fg-subtle outline-none transition-colors",
        "scheme-dark [:root[data-theme=light]_&]:scheme-light",
        "focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40",
        "disabled:opacity-50 disabled:pointer-events-none",
        className,
      )}
    />
  );
}
