import { type ComponentPropsWithRef } from "react";

import { cn } from "@/lib/utils/cn";

export type ScrollAreaProps = ComponentPropsWithRef<"div">;

/** Native overflow scroll with the app's quiet scrollbar styling. */
export function ScrollArea({ className, ...props }: ScrollAreaProps) {
  return <div {...props} className={cn("min-h-0 overflow-y-auto overscroll-contain", className)} />;
}
