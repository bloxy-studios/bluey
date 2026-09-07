import { DevOverlayGate } from "@/app/dev/DevOverlay";
import { Toasts } from "@/components/ui/Toast";
import { TooltipProvider } from "@/components/ui/Tooltip";
import { AuthGate } from "@/lib/auth/AuthGate";
import { ClerkRoot } from "@/lib/auth/ClerkRoot";
import { SettingsShell } from "@/features/settings/SettingsShell";

/** Settings window (930×690, resizable). */
export default function SettingsWindow() {
  return (
    <ClerkRoot>
      <TooltipProvider>
        <AuthGate>
          <SettingsShell />
        </AuthGate>
        <DevOverlayGate />
        <Toasts />
      </TooltipProvider>
    </ClerkRoot>
  );
}
