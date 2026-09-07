import * as Radix from "@radix-ui/react-dropdown-menu";
import { Check } from "lucide-react";
import { type ComponentProps, type ReactNode } from "react";

import { cn } from "@/lib/utils/cn";

export const DropdownMenu = Radix.Root;
export const DropdownMenuTrigger = Radix.Trigger;

export function DropdownMenuContent({ className, children, ...props }: ComponentProps<typeof Radix.Content>) {
  return (
    <Radix.Portal>
      <Radix.Content
        sideOffset={6}
        collisionPadding={8}
        {...props}
        className={cn(
          "z-50 min-w-[200px] rounded-card border border-hud-border bg-[#141414] p-1 py-1.5",
          "shadow-[0_8px_32px_rgba(0,0,0,0.45)] motion-safe:animate-rise-in",
          className,
        )}
      >
        {children}
      </Radix.Content>
    </Radix.Portal>
  );
}

export interface DropdownMenuItemProps extends ComponentProps<typeof Radix.Item> {
  icon?: ReactNode;
  /** Right-aligned check for the active item (mode menu). */
  checked?: boolean;
  destructive?: boolean;
}

export function DropdownMenuItem({ className, children, icon, checked, destructive, ...props }: DropdownMenuItemProps) {
  return (
    <Radix.Item
      {...props}
      className={cn(
        "flex cursor-default select-none items-center gap-2.5 rounded-[8px] px-3 py-2 text-[14px] outline-none",
        destructive ? "text-danger data-highlighted:bg-danger/10" : "text-fg data-highlighted:bg-white/8",
        "data-[disabled]:pointer-events-none data-[disabled]:opacity-40",
        className,
      )}
    >
      {icon ? <span className="flex size-4 items-center justify-center text-fg-muted">{icon}</span> : null}
      <span className="flex-1">{children}</span>
      {checked ? <Check className="size-4 text-fg" aria-hidden /> : null}
    </Radix.Item>
  );
}

export function DropdownMenuSeparator({ className, ...props }: ComponentProps<typeof Radix.Separator>) {
  return <Radix.Separator {...props} className={cn("mx-1 my-1 h-px bg-hud-border", className)} />;
}

export function DropdownMenuLabel({ className, ...props }: ComponentProps<typeof Radix.Label>) {
  return <Radix.Label {...props} className={cn("px-3 py-1.5 text-[11px] font-medium text-fg-subtle", className)} />;
}
