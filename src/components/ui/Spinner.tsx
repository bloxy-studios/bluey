import { cn } from "@/lib/utils/cn";

export interface SpinnerProps {
  className?: string;
  size?: number;
}

export function Spinner({ className, size = 14 }: SpinnerProps) {
  return (
    <span
      role="status"
      aria-label="Loading"
      style={{ width: size, height: size }}
      className={cn(
        "inline-block shrink-0 rounded-full border-[1.5px] border-fg-muted/40 border-t-fg",
        "motion-safe:animate-spin-slow",
        className,
      )}
    />
  );
}
