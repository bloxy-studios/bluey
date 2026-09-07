import * as RadixSlider from "@radix-ui/react-slider";

import { cn } from "@/lib/utils/cn";

export interface SliderProps {
  value: number;
  onValueChange: (value: number) => void;
  onValueCommit?: (value: number) => void;
  min: number;
  max: number;
  step?: number;
  disabled?: boolean;
  className?: string;
  "aria-label": string;
}

export function Slider({ value, onValueChange, onValueCommit, min, max, step = 1, disabled, className, ...aria }: SliderProps) {
  return (
    <RadixSlider.Root
      className={cn("relative flex h-5 w-[160px] touch-none select-none items-center", className)}
      value={[value]}
      onValueChange={(values) => onValueChange(values[0] ?? value)}
      onValueCommit={(values) => onValueCommit?.(values[0] ?? value)}
      min={min}
      max={max}
      step={step}
      disabled={disabled}
      {...aria}
    >
      <RadixSlider.Track className="relative h-[4px] grow rounded-full bg-bg-tile">
        <RadixSlider.Range className="absolute h-full rounded-full bg-accent" />
      </RadixSlider.Track>
      <RadixSlider.Thumb
        aria-label={aria["aria-label"]}
        className="block size-[16px] rounded-full bg-white shadow-[0_1px_4px_rgba(0,0,0,0.4)] outline-none focus-visible:ring-2 focus-visible:ring-accent"
      />
    </RadixSlider.Root>
  );
}
