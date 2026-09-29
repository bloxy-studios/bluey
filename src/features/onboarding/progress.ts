/**
 * The onboarding step the user reached, kept across relaunches (ONB-004): granting Screen
 * Recording often ends with macOS's "Quit & Reopen", and the wizard should resume where it
 * stopped instead of at Welcome. Only a step id is stored — never anything the user typed.
 */
export const ONBOARDING_STEP_KEY = "bluey.onboarding.step";

export function readOnboardingStep(): string | null {
  try {
    return globalThis.localStorage?.getItem(ONBOARDING_STEP_KEY) ?? null;
  } catch {
    return null;
  }
}

export function saveOnboardingStep(stepId: string): void {
  try {
    globalThis.localStorage?.setItem(ONBOARDING_STEP_KEY, stepId);
  } catch {
    // Private mode / quota — the wizard simply restarts at Welcome after a relaunch.
  }
}

/** Onboarding finished or reset: the next run starts at Welcome. */
export function clearOnboardingStep(): void {
  try {
    globalThis.localStorage?.removeItem(ONBOARDING_STEP_KEY);
  } catch {
    // Nothing stored or storage unavailable — nothing to clear.
  }
}
