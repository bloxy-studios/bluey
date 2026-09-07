import { cn } from "@/lib/utils/cn";

export interface BlueyMarkProps {
  className?: string;
  size?: number;
}

/** The Bluey logo mark: monochrome circle with a diagonal slash (public/bluey-mark.svg). */
export function BlueyMark({ className, size = 28 }: BlueyMarkProps) {
  return (
    <svg
      viewBox="0 0 28 28"
      width={size}
      height={size}
      fill="none"
      aria-hidden
      className={cn("shrink-0", className)}
    >
      <circle cx="14" cy="14" r="10.5" stroke="currentColor" strokeWidth="2.4" />
      <line x1="7.2" y1="20.8" x2="20.8" y2="7.2" stroke="currentColor" strokeWidth="2.4" strokeLinecap="round" />
    </svg>
  );
}
