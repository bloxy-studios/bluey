import { type ComponentPropsWithRef } from "react";

import { cn } from "@/lib/utils/cn";

export type TextareaProps = ComponentPropsWithRef<"textarea">;

export function Textarea({ className, ...props }: TextareaProps) {
  return (
    <textarea
      {...props}
      className={cn(
        "w-full min-w-0 max-w-full resize-y overscroll-contain rounded-[10px] border border-border bg-bg-elevated px-3.5 py-3",
        "text-[15px] leading-relaxed text-fg placeholder:text-fg-subtle outline-none",
        "scheme-dark [:root[data-theme=light]_&]:scheme-light",
        "transition-colors focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40",
        "disabled:opacity-50 disabled:pointer-events-none",
        className,
      )}
    />
  );
}
