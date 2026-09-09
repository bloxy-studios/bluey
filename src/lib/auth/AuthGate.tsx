import { KeyRound } from "lucide-react";
import { type ReactNode } from "react";

import { BlueyMark } from "@/components/BlueyMark";
import { Spinner } from "@/components/ui/Spinner";
import { BrowserSignIn } from "./BrowserSignIn";
import { useAuthStatus } from "./useAuthStatus";

function CenteredShell({ children }: { children: ReactNode }) {
  return (
    <div className="flex h-full min-h-screen flex-col items-center justify-center gap-6 bg-bg p-8">
      <BlueyMark size={36} className="text-fg opacity-90" />
      {children}
    </div>
  );
}

/** Shown when sign-in is not configured on a real build (never crashes). */
function ConfigurationScreen() {
  return (
    <CenteredShell>
      <div className="w-[440px] max-w-full rounded-card border border-border bg-bg-elevated p-6">
        <div className="flex items-center gap-3">
          <div className="flex size-10 items-center justify-center rounded-card bg-bg-tile">
            <KeyRound className="size-5 text-fg-muted" aria-hidden />
          </div>
          <div>
            <h1 className="text-[15px] font-semibold text-fg">Sign-in isn't configured</h1>
            <p className="text-[13px] text-fg-muted">
              Bluey needs a Clerk instance and a public OAuth app to start.
            </p>
          </div>
        </div>
        <ol className="mt-4 list-decimal space-y-2 pl-5 text-[13px] leading-relaxed text-fg-muted">
          <li>
            Copy <code className="rounded bg-bg-tile px-1.5 py-0.5 font-mono text-[12px]">.env.example</code>{" "}
            to <code className="rounded bg-bg-tile px-1.5 py-0.5 font-mono text-[12px]">.env</code> or{" "}
            <code className="rounded bg-bg-tile px-1.5 py-0.5 font-mono text-[12px]">.env.local</code> in the
            project root
          </li>
          <li>
            Set{" "}
            <code className="rounded bg-bg-tile px-1.5 py-0.5 font-mono text-[12px]">
              VITE_CLERK_PUBLISHABLE_KEY
            </code>{" "}
            and{" "}
            <code className="rounded bg-bg-tile px-1.5 py-0.5 font-mono text-[12px]">
              BLUEY_CLERK_OAUTH_CLIENT_ID
            </code>{" "}
            from your Clerk dashboard (see docs/DEVELOPMENT.md)
          </li>
          <li>Restart Bluey — the files are read at startup; a packaged build needs a rebuild</li>
        </ol>
      </div>
    </CenteredShell>
  );
}

function SignInScreen() {
  return (
    <CenteredShell>
      <BrowserSignIn />
    </CenteredShell>
  );
}

/** Compact prompt for the tiny HUD panel: sign-in happens in onboarding. */
function HudSignInPrompt() {
  return (
    <div className="flex h-full items-start justify-center pt-4">
      <div className="flex w-[420px] items-center gap-3 rounded-panel border border-hud-border bg-hud-bg px-5 py-4 shadow-[0_8px_32px_rgba(0,0,0,0.35)] backdrop-blur-[24px]">
        <BlueyMark size={22} className="text-fg" />
        <div className="flex-1 text-[13px] text-fg-muted">Sign in to use Bluey.</div>
        <button
          type="button"
          onClick={() =>
            void import("@/lib/tauri/api").then(({ bluey }) => bluey.window.open({ label: "onboarding" }))
          }
          className="h-8 rounded-control bg-accent px-3.5 text-[13px] font-medium text-white transition-colors hover:bg-accent-hover"
        >
          Sign in
        </button>
      </div>
    </div>
  );
}

export interface AuthGateProps {
  children: ReactNode;
  /** "hud" keeps the gate compact (the HUD panel cannot host the sign-in card). */
  variant?: "full" | "hud";
}

/**
 * Gates window content on authentication:
 * unconfigured → setup screen · signed out → browser sign-in card (or a compact
 * HUD prompt) · unknown / not loaded → spinner · signed in / dev mode → children.
 */
export function AuthGate({ children, variant = "full" }: AuthGateProps) {
  const { mode, state, loaded } = useAuthStatus();

  if (loaded && mode === "unconfigured") return <ConfigurationScreen />;
  if (loaded && mode === "dev") return <>{children}</>;
  if (!loaded || state === "unknown") {
    return variant === "hud" ? null : (
      <CenteredShell>
        <Spinner size={18} />
      </CenteredShell>
    );
  }
  if (state === "signed_out") return variant === "hud" ? <HudSignInPrompt /> : <SignInScreen />;
  return <>{children}</>;
}
