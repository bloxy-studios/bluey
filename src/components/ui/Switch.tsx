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

/** 44×26 macOS-style switch (off `#2a2a2a`, on accent, white knob). */
export function Switch({ className, ...props }: SwitchProps) {
  return (
    <RadixSwitch.Root
      {...props}
      className={cn(
        "relative h-[26px] w-[44px] shrink-0 rounded-full outline-none transition-colors duration-150",
        "bg-[#2a2a2a] data-[state=checked]:bg-accent",
        "disabled:opacity-50 disabled:pointer-events-none",
        className,
      )}
    >
      <RadixSwitch.Thumb
        className={cn(
          "block size-[22px] translate-x-[2px] rounded-full bg-white shadow-[0_1px_3px_rgba(0,0,0,0.35)]",
          "transition-transform duration-150 data-[state=checked]:translate-x-[20px]",
        )}
      />
    </RadixSwitch.Root>
  );
}
