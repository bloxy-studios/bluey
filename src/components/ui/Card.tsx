import { type ComponentPropsWithRef } from "react";

import { cn } from "@/lib/utils/cn";

export type CardProps = ComponentPropsWithRef<"div">;

/** Elevated card: `bg-elevated`, 1px border, radius 12, 20px padding. */
export function Card({ className, ...props }: CardProps) {
  return <div className={cn("rounded-card border border-border bg-bg-elevated p-5", className)} {...props} />;
}
