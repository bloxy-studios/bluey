import { ErrorBoundary } from "@/components/ui/ErrorBoundary";
import { Toasts } from "@/components/ui/Toast";
import { TooltipProvider } from "@/components/ui/Tooltip";
import { OnboardingFlow } from "@/features/onboarding/OnboardingFlow";

/** Onboarding window (760×560). Sign-in happens inside the flow — no gate. */
export default function OnboardingWindow() {
  return (
    <ErrorBoundary>
      <TooltipProvider>
        <OnboardingFlow />
        <Toasts />
      </TooltipProvider>
    </ErrorBoundary>
  );
}
