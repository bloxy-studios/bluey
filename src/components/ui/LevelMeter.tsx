import { cn } from "@/lib/utils/cn";

export interface LevelMeterProps {
  /** 0..1 */
  level: number;
  segments?: number;
  className?: string;
  "aria-label"?: string;
}

/** Segmented audio level meter (Test Microphone, onboarding mic step). */
export function LevelMeter({ level, segments = 18, className, ...aria }: LevelMeterProps) {
  const active = Math.round(Math.max(0, Math.min(1, level)) * segments);
  return (
    <div
      role="meter"
      aria-valuemin={0}
      aria-valuemax={1}
      aria-valuenow={Number(level.toFixed(2))}
      {...aria}
      className={cn("flex h-4 items-center gap-[3px]", className)}
    >
      {Array.from({ length: segments }, (_, i) => (
        <span
          key={i}
          className={cn(
            "h-full w-[5px] rounded-[2px] transition-colors duration-75",
            i < active ? (i > segments * 0.8 ? "bg-danger" : "bg-success") : "bg-bg-tile",
          )}
        />
      ))}
    </div>
  );
}
