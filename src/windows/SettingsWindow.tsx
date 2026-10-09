import { DevOverlayGate } from "@/app/dev/DevOverlay";
import { ErrorBoundary } from "@/components/ui/ErrorBoundary";
import { Toasts } from "@/components/ui/Toast";
import { TooltipProvider } from "@/components/ui/Tooltip";
import { AuthGate } from "@/lib/auth/AuthGate";
import { SettingsShell } from "@/features/settings/SettingsShell";

/** Settings window (930×690, resizable). */
export default function SettingsWindow() {
  return (
    <ErrorBoundary>
      <TooltipProvider>
        <AuthGate>
          <SettingsShell />
        </AuthGate>
        <DevOverlayGate />
        <Toasts />
      </TooltipProvider>
    </ErrorBoundary>
  );
}
