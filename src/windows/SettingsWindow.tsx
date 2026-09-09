import { DevOverlayGate } from "@/app/dev/DevOverlay";
import { Toasts } from "@/components/ui/Toast";
import { TooltipProvider } from "@/components/ui/Tooltip";
import { AuthGate } from "@/lib/auth/AuthGate";
import { SettingsShell } from "@/features/settings/SettingsShell";

/** Settings window (930×690, resizable). */
export default function SettingsWindow() {
  return (
    <TooltipProvider>
      <AuthGate>
        <SettingsShell />
      </AuthGate>
      <DevOverlayGate />
      <Toasts />
    </TooltipProvider>
  );
}
