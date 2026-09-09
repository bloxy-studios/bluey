import { Toasts } from "@/components/ui/Toast";
import { TooltipProvider } from "@/components/ui/Tooltip";
import { OnboardingFlow } from "@/features/onboarding/OnboardingFlow";

/** Onboarding window (760×560). Sign-in happens inside the flow — no gate. */
export default function OnboardingWindow() {
  return (
    <TooltipProvider>
      <OnboardingFlow />
      <Toasts />
    </TooltipProvider>
  );
}
