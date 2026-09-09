import { Slot } from "@radix-ui/react-slot";
import { type ReactElement, type ReactNode, useRef, useState } from "react";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/DropdownMenu";
import { Tooltip } from "@/components/ui/Tooltip";
import { showErrorToast } from "@/components/ui/toast-store";
import { menuLabel, type MenuEntry } from "@/lib/tauri/menu-model";
import { openNativeHudMenu } from "@/lib/tauri/native-hud-menu";
import { hasTauriRuntime } from "@/lib/tauri/transport";
import { toBlueyError } from "@/lib/types";
import { isComposingKey, preventRepeatedActivation } from "./hud-keyboard";

export type HudMenuEntry<Action> = MenuEntry<Action> & { icon?: ReactNode };

export interface HudMenuProps<Action> {
  /** A single ref-forwarding button; neither backend inserts a wrapping span. */
  children: ReactElement;
  entries: readonly HudMenuEntry<Action>[];
  onSelect: (action: Action) => void;
  align?: "start" | "end";
  tooltip?: string;
}

/** Native tracking in the desktop runtime; unchanged Radix menus in browser previews. */
export function HudMenu<Action>(props: HudMenuProps<Action>) {
  return hasTauriRuntime() ? <NativeHudMenu {...props} /> : <WebHudMenu {...props} />;
}

function NativeHudMenu<Action>({
  children,
  entries,
  onSelect,
  align = "start",
  tooltip,
}: HudMenuProps<Action>) {
  const triggerRef = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);

  const activate = () => {
    const button = triggerRef.current;
    if (!button) return;
    const popup = openNativeHudMenu(entries, button, align);
    if (!popup) return;
    setOpen(true);
    void popup
      .then((selection) => {
        // Unmount does not dispose a tracking native menu. Rust finishes cleanup; a stale
        // UI instance must neither run actions nor refocus a newly mounted toolbar.
        if (triggerRef.current !== button || !button.isConnected) return;
        if (selection) onSelect(selection.action);
      })
      .catch((error: unknown) => {
        if (triggerRef.current === button && button.isConnected) showErrorToast(toBlueyError(error));
      })
      .finally(() => {
        if (triggerRef.current === button && button.isConnected) setOpen(false);
      });
  };

  // Slot composes the child's ref and event handlers onto the actual IconButton, just
  // like Radix Trigger asChild. No additional button/span, tabindex or native focus API.
  const trigger = (
    <Slot
      ref={triggerRef}
      aria-haspopup="menu"
      aria-expanded={open}
      data-state={open ? "open" : "closed"}
      data-native-hud-menu={open ? "open" : "closed"}
      onClick={(event) => {
        if (!event.defaultPrevented) activate();
      }}
      onKeyDown={(event) => {
        if (event.defaultPrevented) return;
        if (["Enter", " ", "ArrowDown"].includes(event.key)) {
          event.preventDefault(); // no additional browser-synthesized click or page scroll
          event.stopPropagation();
          if (
            !isComposingKey(event.nativeEvent) &&
            !event.repeat &&
            !event.altKey &&
            !event.ctrlKey &&
            !event.metaKey
          ) {
            activate();
          }
        }
      }}
    >
      {children}
    </Slot>
  );
  return tooltip ? <Tooltip label={tooltip}>{trigger}</Tooltip> : trigger;
}

function WebHudMenu<Action>({ children, entries, onSelect, align = "start", tooltip }: HudMenuProps<Action>) {
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
