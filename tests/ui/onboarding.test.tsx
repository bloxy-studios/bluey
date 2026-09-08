import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";

import { TooltipProvider } from "@/components/ui/Tooltip";
import { OnboardingFlow } from "@/features/onboarding/OnboardingFlow";
import { useSettingsStore } from "@/stores/settingsStore";
import { setupMockApp } from "./helpers";

describe("OnboardingFlow (MockTransport)", () => {
  beforeEach(async () => {
    await setupMockApp();
  });

  it("walks every step and completes onboarding", async () => {
    const user = userEvent.setup();
    render(
      <TooltipProvider>
        <OnboardingFlow />
      </TooltipProvider>,
    );

    // 1 — Welcome
    expect(screen.getByText("Welcome to Bluey")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Continue" }));

    // 2 — Sign in (no Clerk key in tests → informational step)
    expect(screen.getByText("Sign in")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Continue" }));

    // 3 — Name your Bluey
    expect(screen.getByText("Name your Bluey")).toBeInTheDocument();
    expect(screen.getByLabelText("Bluey name")).toHaveValue("Bluey");
    await user.click(screen.getByRole("button", { name: "Continue" }));

    // 3b — Connect Gemini (the fixture already holds a key → connected, Continue enabled)
    expect(screen.getByText("Connect Gemini")).toBeInTheDocument();
    await waitFor(() => expect(screen.getByText("Gemini is connected")).toBeInTheDocument());
    expect(screen.getByText(/aistudio.google.com\/apikey/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Continue" })).toBeEnabled();
    await user.click(screen.getByRole("button", { name: "Continue" }));

    // 4 — Permissions: four sub-screens with the three explanations
    expect(screen.getByText("Screen Recording")).toBeInTheDocument();
    expect(screen.getByText("What Bluey needs")).toBeInTheDocument();
    expect(screen.getByText("Why")).toBeInTheDocument();
    expect(screen.getByText("What it can access")).toBeInTheDocument();

    // Request the first permission — live status flips to Granted.
    await user.click(screen.getByRole("button", { name: "Grant access" }));
    await waitFor(() => expect(screen.getByText("Granted")).toBeInTheDocument());
    await user.click(screen.getByRole("button", { name: "Next permission" }));

    expect(screen.getByText("Microphone")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /Next permission|Skip for now/ }));
    expect(screen.getByText("Accessibility")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /Next permission|Skip for now/ }));
    expect(screen.getByText("Speech Recognition")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Continue" }));

    // 5 — Default mode
    expect(screen.getByText("Choose your default mode")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /Coding Interview/ }));
    await waitFor(() =>
      expect(useSettingsStore.getState().settings?.general.defaultModeId).toBe("coding-interview"),
    );
    await user.click(screen.getByRole("button", { name: "Continue" }));

    // 6 — Shortcuts
    expect(screen.getByText("Your shortcuts")).toBeInTheDocument();
    expect(screen.getByText("Ask Bluey about your screen or audio")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Continue" }));

    // 7 — Test screen: capture shows a thumbnail
    expect(screen.getByText("Test screen capture")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Capture screen" }));
    await waitFor(() => expect(screen.getByAltText("Screen capture preview")).toBeInTheDocument());
    await user.click(screen.getByRole("button", { name: "Continue" }));

    // 8 — Test microphone
    expect(screen.getByText("Test your microphone")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Test microphone" }));
    await waitFor(() => expect(screen.getByText(/Heard you/)).toBeInTheDocument());
    await user.click(screen.getByRole("button", { name: "Continue" }));

    // 9 — Test AI (mock provider has a stored key)
    expect(screen.getByText("Test your AI provider")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Test connection" }));
    await waitFor(() => expect(screen.getByText(/Connected/)).toBeInTheDocument());
    await user.click(screen.getByRole("button", { name: "Continue" }));

    // 10 — Ready → completes onboarding and opens the HUD
    expect(screen.getByText(/is ready/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Open Bluey" }));
    await waitFor(() => expect(useSettingsStore.getState().settings?.general.onboardingCompleted).toBe(true));
  });
});
