import { cn } from "@/lib/utils/cn";

export interface BlueyMarkProps {
  className?: string;
  size?: number;
}

/** The "b" of the app icon as a monochrome mark (`currentColor`). Same path as `public/bluey-mark.svg`. */
const MARK_PATH =
  "M6.32 2L6.89 1.98L7.71 2.25L10.59 3.99L11.39 4.74L11.85 5.52L12.08 6.39L12.1 9.57L13.11 9.57L14.07 9.25L14.98 9.11L16.45 9.11L17.86 9.38L19.14 9.89L20.24 10.57L21.54 11.78L22.23 12.74L22.82 13.93L23.23 15.39L23.37 16.95L23.23 18.59L22.78 20.24L22.27 21.34L21.36 22.66L20.06 23.92L19.28 24.47L18 25.15L16.4 25.7L14.66 25.98L13.11 25.98L12.29 25.89L11.05 25.61L10.23 25.34L8.4 24.42L7.26 23.55L6.41 22.66L5.77 21.79L5.41 21.15L4.95 20.06L4.72 19.14L4.58 17.86L4.58 3.78L4.72 3.28L4.99 2.78L5.66 2.21L6.07 2.02Z";

/** The Bluey logo mark: the "b" of the app icon, filled with the current text colour. */
export function BlueyMark({ className, size = 28 }: BlueyMarkProps) {
  return (
    <svg
      viewBox="0 0 28 28"
      width={size}
      height={size}
      fill="currentColor"
      aria-hidden
      className={cn("shrink-0", className)}
    >
      <path d={MARK_PATH} />
    </svg>
  );
}
