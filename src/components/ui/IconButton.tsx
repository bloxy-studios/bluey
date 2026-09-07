import { cva, type VariantProps } from "class-variance-authority";
import { type ComponentPropsWithRef } from "react";

import { cn } from "@/lib/utils/cn";

const iconButton = cva(
  [
    "inline-flex items-center justify-center shrink-0 select-none outline-none",
    "transition-colors duration-150 disabled:opacity-50 disabled:pointer-events-none",
  ],
  {
    variants: {
      variant: {
        /** Quiet toolbar button (HUD row 2). */
        ghost: "text-fg-muted hover:text-fg hover:bg-white/10 rounded-[8px]",
        /** Chip on the HUD input row (rgba(255,255,255,.1) rounded square). */
        chip: "bg-hud-chip text-fg hover:bg-white/15 rounded-[10px]",
        /** Circular buttons (back, stop, scroll-to-bottom, "…"). */
        circle: "bg-bg-tile text-fg-muted hover:text-fg hover:bg-bg-hover rounded-full border border-border",
        /** Circular on translucent HUD. */
        hudCircle: "bg-hud-chip text-fg hover:bg-white/15 rounded-full",
        plain: "text-fg-muted hover:text-fg rounded-[8px]",
      },
      size: {
        sm: "size-7",
        md: "size-8",
        lg: "size-9",
      },
      active: {
        true: "text-fg bg-white/10",
        false: "",
      },
    },
    defaultVariants: { variant: "ghost", size: "md", active: false },
  },
);

export interface IconButtonProps extends ComponentPropsWithRef<"button">, VariantProps<typeof iconButton> {
  "aria-label": string;
}

export function IconButton({ className, variant, size, active, type = "button", ...props }: IconButtonProps) {
  return <button type={type} className={cn(iconButton({ variant, size, active }), className)} {...props} />;
}
