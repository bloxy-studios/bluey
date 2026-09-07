import { Toasts } from "@/components/ui/Toast";
import { TooltipProvider } from "@/components/ui/Tooltip";
import { ClerkRoot } from "@/lib/auth/ClerkRoot";
import { OnboardingFlow } from "@/features/onboarding/OnboardingFlow";

/** Onboarding window (760×560). Sign-in happens inside the flow — no gate. */
export default function OnboardingWindow() {
  return (
    <ClerkRoot>
      <TooltipProvider>
        <OnboardingFlow />
        <Toasts />
      </TooltipProvider>
    </ClerkRoot>
  );
}
