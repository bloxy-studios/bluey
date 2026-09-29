import { cn } from "@/lib/utils/cn";

export interface SpinnerProps {
  className?: string;
  size?: number;
  /** Next to a visible label or under a live announcer: hidden from assistive tech (UX-025). */
  decorative?: boolean;
}

export function Spinner({ className, size = 14, decorative = false }: SpinnerProps) {
  return (
    <span
      {...(decorative ? { "aria-hidden": true } : { role: "status", "aria-label": "Loading" })}
      style={{ width: size, height: size }}
      className={cn(
        "inline-block shrink-0 rounded-full border-[1.5px] border-fg-muted/40 border-t-fg",
        "motion-safe:animate-spin-slow",
        className,
      )}
    />
  );
}
