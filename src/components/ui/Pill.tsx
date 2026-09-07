import { cva, type VariantProps } from "class-variance-authority";
import { type ComponentPropsWithRef } from "react";

import { cn } from "@/lib/utils/cn";

const pill = cva(
  "inline-flex items-center gap-1.5 rounded-full font-medium select-none whitespace-nowrap transition-colors",
  {
    variants: {
      variant: {
        /** Blue pill ("Update Available" / mode name in the HUD). */
        accent: "bg-accent text-white",
        /** Quiet state pill on the HUD (`● Listening`, `◌ Thinking`). */
        hud: "bg-hud-chip text-fg",
        muted: "bg-bg-tile text-fg-muted",
        outline: "border border-border text-fg-muted",
      },
      size: {
        sm: "h-[22px] px-2.5 text-[11.5px]",
        md: "h-[26px] px-3 text-[12.5px]",
      },
      interactive: {
        true: "cursor-default hover:brightness-110",
        false: "",
      },
    },
    defaultVariants: { variant: "hud", size: "md", interactive: false },
  },
);

export interface PillProps extends ComponentPropsWithRef<"span">, VariantProps<typeof pill> {}

export function Pill({ className, variant, size, interactive, ...props }: PillProps) {
  return <span className={cn(pill({ variant, size, interactive }), className)} {...props} />;
}
