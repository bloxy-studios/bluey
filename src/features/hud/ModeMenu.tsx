import { LayoutGrid } from "lucide-react";
import { type ReactElement } from "react";

import { showErrorToast } from "@/components/ui/toast-store";
import { bluey } from "@/lib/tauri/api";
import { toBlueyError } from "@/lib/types";
import { useAppStore } from "@/stores/appStore";
import { useModesStore } from "@/stores/modesStore";
import { HudMenu, type HudMenuEntry } from "./HudMenu";

export interface ModeMenuProps {
  children: ReactElement;
  tooltip?: string;
}

type ModeAction = { type: "mode"; id: string } | { type: "manage" };

/** The HUD mode menu: modes with a check on the active one + "Manage". */
export function ModeMenu({ children, tooltip }: ModeMenuProps) {
  const modes = useModesStore((s) => s.modes);
  const activeModeId = useAppStore((s) => s.status?.modeId);
  const entries: HudMenuEntry<ModeAction>[] = [
    ...modes.map((mode): HudMenuEntry<ModeAction> => ({
      kind: "item",
      id: `mode-${mode.id}`,
      label: mode.name,
      action: { type: "mode", id: mode.id },
      checked: mode.id === activeModeId,
    })),
    { kind: "separator", id: "manage-separator" },
    {
      kind: "item",
      id: "manage",
      label: "Manage",
      action: { type: "manage" },
      icon: <LayoutGrid className="size-4" aria-hidden />,
      nativeIcon: "manage",
    },
  ];

  const onSelect = (action: ModeAction) => {
    const result =
      action.type === "mode"
        ? bluey.modes.setActive({ id: action.id })
        : bluey.window.open({ label: "settings", route: "modes" });
    void result.catch((error: unknown) => showErrorToast(toBlueyError(error)));
  };

  return (
    <HudMenu entries={entries} onSelect={onSelect} tooltip={tooltip}>
      {children}
    </HudMenu>
  );
}
