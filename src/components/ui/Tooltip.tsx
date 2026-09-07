import * as RadixTooltip from "@radix-ui/react-tooltip";
import { type ReactNode } from "react";

import { cn } from "@/lib/utils/cn";
import { acceleratorToGlyphs } from "@/lib/utils/keyboard";

export function TooltipProvider({ children }: { children: ReactNode }) {
  return (
    <RadixTooltip.Provider delayDuration={350} skipDelayDuration={200}>
      {children}
    </RadixTooltip.Provider>
  );
}

export interface TooltipProps {
  label: string;
  /** Optional accelerator rendered as small keycaps after the label. */
  shortcut?: string;
  side?: "top" | "bottom" | "left" | "right";
  children: ReactNode;
  className?: string;
}

/** Dark macOS-style tooltip: `tooltip-bg`, 13px white, 6px above the trigger. */
export function Tooltip({ label, shortcut, side = "top", children, className }: TooltipProps) {
  return (
    <RadixTooltip.Root>
      <RadixTooltip.Trigger asChild>{children}</RadixTooltip.Trigger>
      <RadixTooltip.Portal>
        <RadixTooltip.Content
          side={side}
          sideOffset={6}
          className={cn(
            "z-50 flex items-center gap-2 rounded-[8px] bg-tooltip-bg px-2.5 py-1.5",
            "text-[13px] font-medium text-white shadow-lg shadow-black/30",
            "motion-safe:animate-tooltip-in",
            className,
          )}
        >
          {label}
          {shortcut ? (
            <span className="flex items-center gap-1">
              {acceleratorToGlyphs(shortcut).map((glyph, i) => (
                <span
                  key={`${glyph}-${i}`}
                  className="flex h-[18px] min-w-[18px] items-center justify-center rounded-[4px] bg-white/12 px-1 text-[11px] text-white/85"
                >
                  {glyph}
                </span>
              ))}
            </span>
          ) : null}
        </RadixTooltip.Content>
      </RadixTooltip.Portal>
    </RadixTooltip.Root>
  );
}
