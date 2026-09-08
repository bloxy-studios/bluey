import { useCallback, useMemo, useState } from "react";

import { Button } from "@/components/ui/Button";
import { bluey } from "@/lib/tauri/api";
import { cn } from "@/lib/utils/cn";
import { useSettingsStore } from "@/stores/settingsStore";
import { NameStep, SignInStep, WelcomeStep } from "./steps/basics";
import { ConnectAIStep } from "./steps/connect";
import { PermissionsStep } from "./steps/permissions";
import { DefaultModeStep, ShortcutsStep } from "./steps/setup";
import { ReadyStep, TestAIStep, TestMicStep, TestScreenStep } from "./steps/tests";

export interface StepProps {
  onReady: (ready: boolean) => void;
}

const STEPS = [
  { id: "welcome", component: WelcomeStep },
  { id: "sign-in", component: SignInStep },
  { id: "name", component: NameStep },
  { id: "connect-ai", component: ConnectAIStep },
  { id: "permissions", component: PermissionsStep },
  { id: "default-mode", component: DefaultModeStep },
  { id: "shortcuts", component: ShortcutsStep },
  { id: "test-screen", component: TestScreenStep },
  { id: "test-microphone", component: TestMicStep },
  { id: "test-ai", component: TestAIStep },
  { id: "ready", component: ReadyStep },
] as const;

/** Onboarding window (760×560): centered steps with progress dots. */
export function OnboardingFlow() {
  const [index, setIndex] = useState(0);
  const [ready, setReady] = useState(true);
  const update = useSettingsStore((s) => s.update);

  const step = STEPS[Math.min(index, STEPS.length - 1)] ?? STEPS[0];
  const Step = step.component;
  const isLast = index === STEPS.length - 1;

  const next = useCallback(async () => {
    if (isLast) {
      await update({ general: { onboardingCompleted: true } });
      await bluey.window.open({ label: "main" });
      await bluey.window.close({ label: "onboarding" });
      return;
    }
    setReady(true);
    setIndex((i) => Math.min(i + 1, STEPS.length - 1));
  }, [isLast, update]);

  const back = useCallback(() => {
    setReady(true);
    setIndex((i) => Math.max(i - 1, 0));
  }, []);

  const onReady = useCallback((value: boolean) => setReady(value), []);
  const stepKey = useMemo(() => step.id, [step.id]);

  return (
    <div data-tauri-drag-region className="flex h-screen flex-col bg-bg text-fg">
      <div
        data-tauri-drag-region
        className="flex shrink-0 items-center justify-center pt-6"
        aria-label={`Step ${index + 1} of ${STEPS.length}`}
      >
        <div className="flex items-center gap-1.5">
          {STEPS.map((s, i) => (
            <span
              key={s.id}
              className={cn(
                "size-1.5 rounded-full transition-colors",
                i === index ? "bg-fg" : i < index ? "bg-fg-muted" : "bg-bg-tile",
              )}
              aria-hidden
            />
          ))}
        </div>
      </div>

      <div
        key={stepKey}
        className="flex min-h-0 flex-1 flex-col items-center justify-center px-12 motion-safe:animate-rise-in"
      >
        <Step onReady={onReady} />
      </div>

      <div className="flex shrink-0 items-center justify-between px-8 pb-7">
        {index > 0 ? (
          <Button variant="ghost" onClick={back}>
            Back
          </Button>
        ) : (
          <span />
        )}
        <Button variant="primary" onClick={() => void next()} disabled={!ready}>
          {isLast ? "Open Bluey" : "Continue"}
        </Button>
      </div>
    </div>
  );
}
