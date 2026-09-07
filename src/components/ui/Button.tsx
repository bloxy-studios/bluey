import { cva, type VariantProps } from "class-variance-authority";
import { type ComponentPropsWithRef } from "react";

import { cn } from "@/lib/utils/cn";

const button = cva(
  [
    "inline-flex items-center justify-center gap-1.5 font-medium select-none whitespace-nowrap",
    "rounded-control transition-colors duration-150 outline-none",
    "disabled:opacity-50 disabled:pointer-events-none",
  ],
  {
    variants: {
      variant: {
        primary: "bg-accent text-white hover:bg-accent-hover",
        secondary: "bg-bg-elevated text-fg border border-border hover:bg-bg-hover",
        ghost: "text-fg-muted hover:text-fg hover:bg-bg-hover",
        danger: "text-danger hover:bg-danger/10",
        link: "text-accent hover:text-accent-hover px-0 h-auto",
      },
      size: {
        sm: "h-7 px-3 text-[12px]",
        md: "h-[34px] px-4 text-[13px]",
        lg: "h-10 px-5 text-[14px]",
      },
    },
    defaultVariants: { variant: "secondary", size: "md" },
  },
);

export interface ButtonProps extends ComponentPropsWithRef<"button">, VariantProps<typeof button> {}

export function Button({ className, variant, size, type = "button", ...props }: ButtonProps) {
  return <button type={type} className={cn(button({ variant, size }), className)} {...props} />;
}
