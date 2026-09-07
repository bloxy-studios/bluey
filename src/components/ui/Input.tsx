import { type ComponentPropsWithRef } from "react";

import { cn } from "@/lib/utils/cn";

export type InputProps = ComponentPropsWithRef<"input">;

export function Input({ className, ...props }: InputProps) {
  return (
    <input
      {...props}
      className={cn(
        "h-9 rounded-control border border-border bg-bg-elevated px-3 text-[13px] text-fg",
        "placeholder:text-fg-subtle outline-none transition-colors",
        "focus-visible:border-border-strong",
        "disabled:opacity-50 disabled:pointer-events-none",
        className,
      )}
    />
  );
}
