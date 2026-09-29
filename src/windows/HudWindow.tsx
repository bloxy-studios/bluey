import { DevOverlayGate } from "@/app/dev/DevOverlay";
import { ErrorBoundary } from "@/components/ui/ErrorBoundary";
import { Toasts } from "@/components/ui/Toast";
import { TooltipProvider } from "@/components/ui/Tooltip";
import { AuthGate } from "@/lib/auth/AuthGate";
import { HudPanel } from "@/features/hud/HudPanel";

/** Main window: the floating HUD over a fully transparent window. */
export default function HudWindow() {
  return (
    <ErrorBoundary>
      <TooltipProvider>
        <AuthGate variant="hud">
          <HudPanel />
        </AuthGate>
        <DevOverlayGate />
        {/* Errors show in the HUD's notice row; only a confirmation pill overlays. */}
        <Toasts limit={1} />
      </TooltipProvider>
    </ErrorBoundary>
  );
}
