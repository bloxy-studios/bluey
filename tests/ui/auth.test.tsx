import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";

import { TooltipProvider } from "@/components/ui/Tooltip";
import { SignInStep } from "@/features/onboarding/steps/basics";
import ProfileTab from "@/features/settings/tabs/ProfileTab";
import { AuthGate } from "@/lib/auth/AuthGate";
import { resolveAuthMode, snapshotFromStatus, useAuthStore } from "@/lib/auth/auth-store";
import type { MockTransport } from "@/lib/tauri/mock";
import { setupMockApp } from "./helpers";

/** A mock app whose Clerk OAuth client is configured (browser sign-in enabled). */
async function configuredApp(): Promise<MockTransport> {
  const mock = await setupMockApp();
  mock.authConfigured = true;
  await useAuthStore.getState().load();
  return mock;
}

describe("auth store", () => {
  it("maps Rust's status onto the UI mode", () => {
    expect(resolveAuthMode(true, "tauri")).toBe("browser");
    expect(resolveAuthMode(false, "mock")).toBe("dev");
    expect(resolveAuthMode(false, "tauri")).toBe("unconfigured");
    const base = { hasStoredSession: false, checkedAt: "now" };
    expect(snapshotFromStatus({ ...base, state: "signed_out", configured: false, signInPending: false }, "mock")).toMatchObject({
      mode: "dev",
      state: "signed_in",
    });
    expect(snapshotFromStatus({ ...base, state: "signed_out", configured: false, signInPending: false }, "tauri")).toMatchObject({
      mode: "unconfigured",
      state: "signed_out",
    });
    expect(snapshotFromStatus({ ...base, state: "signed_out", configured: true, signInPending: true }, "tauri")).toMatchObject({
      mode: "browser",
      state: "signed_out",
      signInPending: true,
    });
  });
});

describe("auth (developer mode)", () => {
  beforeEach(async () => {
    await setupMockApp();
  });

  it("signs in automatically as the development user and lets content through", () => {
    const snapshot = useAuthStore.getState();
    expect(snapshot.mode).toBe("dev");
    expect(snapshot.state).toBe("signed_in");
    render(
      <AuthGate>
        <div>gated content</div>
      </AuthGate>,
    );
    expect(screen.getByText("gated content")).toBeInTheDocument();
  });
});

describe("browser sign-in", () => {
  it("opens the browser flow and signs in when the redirect comes back", async () => {
    await configuredApp();
    const user = userEvent.setup();
    render(
      <TooltipProvider>
        <AuthGate>
          <div>gated content</div>
        </AuthGate>
      </TooltipProvider>,
    );
    expect(screen.queryByText("gated content")).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /Sign in with your browser/ }));
    // The simulated browser round-trip completes on a timer; the gate flips to the content.
    await waitFor(() => expect(screen.getByText("gated content")).toBeInTheDocument());
    expect(useAuthStore.getState().user?.email).toBe("jordan@example.com");
    expect(useAuthStore.getState().signInPending).toBe(false);
  });

  it("can cancel while waiting and stays signed out after a denied redirect", async () => {
    const mock = await configuredApp();
    mock.nextSignInOutcome = "hang";
    const user = userEvent.setup();
    render(
      <AuthGate>
        <div>gated content</div>
      </AuthGate>,
    );

    await user.click(screen.getByRole("button", { name: /Sign in with your browser/ }));
    await screen.findByText(/Waiting for your browser/);
    expect(screen.getByRole("button", { name: "Copy link" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    await screen.findByRole("button", { name: /Sign in with your browser/ });

    mock.nextSignInOutcome = "denied";
    await user.click(screen.getByRole("button", { name: /Sign in with your browser/ }));
    await waitFor(() => expect(useAuthStore.getState().signInPending).toBe(false));
    await screen.findByRole("button", { name: /Sign in with your browser/ });
    expect(useAuthStore.getState().state).toBe("signed_out");
    expect(screen.queryByText("gated content")).not.toBeInTheDocument();
  });

  it("is the onboarding sign-in step and feeds the profile tab", async () => {
    const mock = await configuredApp();
    const user = userEvent.setup();
    const { unmount } = render(<SignInStep onReady={() => {}} />);
    await user.click(screen.getByRole("button", { name: /Sign in with your browser/ }));
    await screen.findByText("You're signed in");
    unmount();

    render(
      <TooltipProvider>
        <ProfileTab />
      </TooltipProvider>,
    );
    expect(await screen.findByText("jordan@example.com")).toBeInTheDocument();
    expect(screen.getByText("Jordan Lee")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Manage account" }));
    expect(mock.accountPortalOpens).toBe(1);
    await user.click(screen.getByRole("button", { name: "Sign out" }));
    await waitFor(() => expect(useAuthStore.getState().state).toBe("signed_out"));
  });
});
