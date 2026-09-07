import { SignIn } from "@clerk/react";
import { useEffect, useState } from "react";

import { BlueyMark } from "@/components/BlueyMark";
import { Input } from "@/components/ui/Input";
import { useAuthStatus } from "@/lib/auth/useAuthStatus";
import { useDebouncedCallback } from "@/hooks/useDebouncedCallback";
import { useSettingsStore } from "@/stores/settingsStore";
import type { StepProps } from "../OnboardingFlow";

export function StepShell({ title, body, children }: { title: string; body?: string; children?: React.ReactNode }) {
  return (
    <div className="flex w-full max-w-[460px] flex-col items-center text-center">
      <h1 className="text-[24px] font-semibold leading-tight text-fg">{title}</h1>
      {body ? <p className="mt-2 text-[14px] leading-relaxed text-fg-muted">{body}</p> : null}
      {children ? <div className="mt-6 w-full">{children}</div> : null}
    </div>
  );
}

export function WelcomeStep(_props: StepProps) {
  return (
    <div className="flex flex-col items-center">
      <BlueyMark size={44} className="mb-6 text-fg" />
      <StepShell
        title="Welcome to Bluey"
        body="A real-time copilot that understands your screen and your conversations — and answers before you finish asking."
      />
    </div>
  );
}

export function SignInStep({ onReady }: StepProps) {
  const { mode, state, user } = useAuthStatus();

  useEffect(() => {
    onReady(mode !== "clerk" || state === "signed_in");
  }, [mode, state, onReady]);

  if (mode !== "clerk") {
    return (
      <StepShell
        title="Sign in"
        body={
          mode === "dev"
            ? "Developer mode — you're signed in as a local development user."
            : "Clerk isn't configured (VITE_CLERK_PUBLISHABLE_KEY). You can continue without an account for now."
        }
      />
    );
  }

  if (state === "signed_in") {
    return <StepShell title="You're signed in" body={`Welcome${user?.firstName ? `, ${user.firstName}` : ""}. Let's set up Bluey.`} />;
  }

  return (
    <div className="flex max-h-full flex-col items-center overflow-y-auto">
      <SignIn routing="hash" />
    </div>
  );
}

export function NameStep(_props: StepProps) {
  const settings = useSettingsStore((s) => s.settings);
  const update = useSettingsStore((s) => s.update);
  const [name, setName] = useState(settings?.general.blueyName ?? "Bluey");
  const save = useDebouncedCallback((value: string) => {
    if (value.trim()) void update({ general: { blueyName: value.trim() } });
  }, 400);

  return (
    <StepShell title="Name your Bluey" body="What should your assistant call itself? You can change this anytime.">
      <Input
        value={name}
        aria-label="Bluey name"
        onChange={(e) => {
          setName(e.target.value);
          save(e.target.value);
        }}
        className="h-11 w-full text-center text-[16px]"
      />
    </StepShell>
  );
}
