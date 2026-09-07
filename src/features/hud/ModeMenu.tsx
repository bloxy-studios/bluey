import { LayoutGrid } from "lucide-react";
import { type ReactNode } from "react";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/DropdownMenu";
import { bluey } from "@/lib/tauri/api";
import { useAppStore } from "@/stores/appStore";
import { useModesStore } from "@/stores/modesStore";

export interface ModeMenuProps {
  children: ReactNode;
}

/** The HUD mode menu: modes with a check on the active one + "Manage". */
export function ModeMenu({ children }: ModeMenuProps) {
  const modes = useModesStore((s) => s.modes);
  const activeModeId = useAppStore((s) => s.status?.modeId);

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>{children}</DropdownMenuTrigger>
      <DropdownMenuContent align="start">
        {modes.map((mode) => (
          <DropdownMenuItem
            key={mode.id}
            checked={mode.id === activeModeId}
            onSelect={() => void bluey.modes.setActive({ id: mode.id })}
          >
            {mode.name}
          </DropdownMenuItem>
        ))}
        <DropdownMenuSeparator />
        <DropdownMenuItem
          icon={<LayoutGrid className="size-4" aria-hidden />}
          onSelect={() => void bluey.window.open({ label: "settings", route: "modes" })}
        >
          Manage
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
