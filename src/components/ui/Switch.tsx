import * as RadixSwitch from "@radix-ui/react-switch";

import { cn } from "@/lib/utils/cn";

export interface SwitchProps {
  checked: boolean;
  onCheckedChange: (checked: boolean) => void;
  disabled?: boolean;
  "aria-label"?: string;
  id?: string;
  className?: string;
}

/** 44×26 macOS-style switch with a theme-aware off track and white knob. */
export function Switch({ className, ...props }: SwitchProps) {
  return (
    <RadixSwitch.Root
      {...props}
      className={cn(
        "relative h-[26px] w-[44px] shrink-0 rounded-full outline-none transition-colors duration-150",
        "bg-border-strong data-[state=checked]:bg-accent",
        "focus-visible:ring-2 focus-visible:ring-accent focus-visible:ring-offset-2 focus-visible:ring-offset-bg",
        "motion-reduce:transition-none disabled:opacity-50 disabled:pointer-events-none",
        className,
      )}
    >
      <RadixSwitch.Thumb
        className={cn(
          "block size-[22px] translate-x-[2px] rounded-full bg-white shadow-[0_1px_3px_rgba(0,0,0,0.35)]",
          "transition-transform duration-150 motion-reduce:transition-none data-[state=checked]:translate-x-[20px]",
        )}
      />
    </RadixSwitch.Root>
  );
}
