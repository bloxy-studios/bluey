import { type ComponentPropsWithRef } from "react";

import { cn } from "@/lib/utils/cn";

export type TextareaProps = ComponentPropsWithRef<"textarea">;

export function Textarea({ className, ...props }: TextareaProps) {
  return (
    <textarea
      {...props}
      className={cn(
        "w-full resize-y rounded-[10px] border border-border bg-bg-elevated px-3.5 py-3",
        "text-[15px] leading-relaxed text-fg placeholder:text-fg-subtle outline-none",
        "transition-colors focus-visible:border-border-strong",
        "disabled:opacity-50 disabled:pointer-events-none",
        className,
      )}
    />
  );
}
