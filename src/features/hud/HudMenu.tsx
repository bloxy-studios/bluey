import { type ReactElement, type ReactNode } from "react";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/DropdownMenu";
import { Tooltip } from "@/components/ui/Tooltip";
import { menuLabel, type MenuEntry } from "@/lib/tauri/menu-model";
import { preventRepeatedActivation } from "./hud-keyboard";

export type HudMenuEntry<Action> = MenuEntry<Action> & { icon?: ReactNode };

export interface HudMenuProps<Action> {
  /** A single button; no wrapping span between the Radix trigger and button. */
  children: ReactElement;
  entries: readonly HudMenuEntry<Action>[];
  onSelect: (action: Action) => void;
  align?: "start" | "end";
  tooltip?: string;
}

/**
 * Accessible web menu. Native popup is deliberately NOT enabled on the locked
 * Tauri version: it leaks action channels even after close (docs/NATIVE_HUD_MENUS.md).
 */
export function HudMenu<Action>({
  children,
  entries,
  onSelect,
  align = "start",
  tooltip,
}: HudMenuProps<Action>) {
  const trigger = (
    <DropdownMenuTrigger asChild onKeyDown={preventRepeatedActivation}>
      {children}
    </DropdownMenuTrigger>
  );

  return (
    <DropdownMenu>
      {tooltip ? <Tooltip label={tooltip}>{trigger}</Tooltip> : trigger}
      <DropdownMenuContent
        align={align}
        onCloseAutoFocus={(event) => {
          // Do not refocus a webview after the user has moved to another app/window.
          if (!document.hasFocus()) event.preventDefault();
        }}
      >
        {entries.map((entry) => {
          if (entry.kind === "separator") return <DropdownMenuSeparator key={entry.id} />;
          if (entry.kind === "label") {
            return (
              <DropdownMenuLabel key={entry.id} title={entry.label}>
                {menuLabel(entry.label)}
              </DropdownMenuLabel>
            );
          }
          return (
            <DropdownMenuItem
              key={entry.id}
              icon={entry.icon}
              checked={entry.checked}
              disabled={entry.disabled}
              destructive={entry.destructive}
              textValue={entry.label}
              aria-label={entry.label}
              onSelect={() => onSelect(entry.action)}
            >
              {menuLabel(entry.label)}
            </DropdownMenuItem>
          );
        })}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
