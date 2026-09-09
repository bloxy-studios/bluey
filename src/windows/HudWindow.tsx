import { DevOverlayGate } from "@/app/dev/DevOverlay";
import { Toasts } from "@/components/ui/Toast";
import { TooltipProvider } from "@/components/ui/Tooltip";
import { AuthGate } from "@/lib/auth/AuthGate";
import { HudPanel } from "@/features/hud/HudPanel";

/** Main window: the floating HUD over a fully transparent window. */
export default function HudWindow() {
  return (
    <TooltipProvider>
      <AuthGate variant="hud">
        <HudPanel />
      </AuthGate>
      <DevOverlayGate />
      <Toasts />
    </TooltipProvider>
  );
}
