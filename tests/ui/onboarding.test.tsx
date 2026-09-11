import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { TooltipProvider } from "@/components/ui/Tooltip";
import { OnboardingFlow } from "@/features/onboarding/OnboardingFlow";
import { ConnectAIStep } from "@/features/onboarding/steps/connect";
import type { MockTransport } from "@/lib/tauri/mock";
import { useSettingsStore } from "@/stores/settingsStore";
import { setupMockApp } from "./helpers";

describe("OnboardingFlow (MockTransport)", () => {
  beforeEach(async () => {
    await setupMockApp();
  });

  it("keeps one 44px drag strip and stationary controls outside the step's scroll surface", async () => {
    const user = userEvent.setup();
    const { container } = render(<TooltipProvider><OnboardingFlow /></TooltipProvider>);
    const progress = screen.getByLabelText("Step 1 of 11");
    const scrollBody = screen.getByRole("main", { name: "Onboarding step" });
    const continueButton = screen.getByRole("button", { name: "Continue" });
    expect(container.querySelectorAll("[data-tauri-drag-region]")).toHaveLength(1);
    expect(progress).toHaveAttribute("data-tauri-drag-region");
    expect(progress).toHaveClass("h-11", "shrink-0");
    expect(scrollBody.closest("[data-tauri-drag-region]")).toBeNull();
    expect(continueButton.closest("[data-tauri-drag-region]")).toBeNull();
    expect(scrollBody).toHaveClass("min-h-0", "overflow-y-auto", "overscroll-contain");
    expect(scrollBody).not.toContainElement(progress);
    expect(scrollBody).not.toContainElement(continueButton);
    expect(scrollBody).toContainElement(screen.getByRole("heading", { name: "Welcome to Bluey" }));
    // A natural-height inner column, not an absolutely positioned centered step,
    // lets tall content start at the top while short steps consume spare space.
    expect(scrollBody.firstElementChild).toHaveClass("min-h-full");
    expect(scrollBody.firstElementChild?.firstElementChild).toHaveClass("my-auto", "shrink-0");
    await user.tab();
    expect(continueButton).toHaveFocus();
    await user.keyboard("{Enter}");
    expect(screen.getByRole("heading", { name: "Sign in" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Continue" })).toBe(continueButton);
    expect(continueButton).toHaveFocus();
  });

  it("resets scroll on forward/back navigation but not while editing the same step", async () => {
    const user = userEvent.setup();
    render(<TooltipProvider><OnboardingFlow /></TooltipProvider>);
    const continueButton = screen.getByRole("button", { name: "Continue" });
    const welcome = screen.getByRole("main", { name: "Onboarding step" });
    welcome.scrollTop = 240;
    await user.click(continueButton);
    const signIn = screen.getByRole("main", { name: "Onboarding step" });
    expect(signIn).not.toBe(welcome);
    expect(signIn.scrollTop).toBe(0);
    await user.click(continueButton);
    const nameStep = screen.getByRole("main", { name: "Onboarding step" });
    nameStep.scrollTop = 120;
    await user.type(screen.getByRole("textbox", { name: "Bluey name" }), " for meetings");
    expect(screen.getByRole("textbox", { name: "Bluey name" })).toHaveValue("Bluey for meetings");
    expect(screen.getByRole("main", { name: "Onboarding step" })).toBe(nameStep);
    expect(nameStep.scrollTop).toBe(120);
    await user.click(screen.getByRole("button", { name: "Back" }));
    expect(screen.getByRole("heading", { name: "Sign in" })).toBeInTheDocument();
    expect(screen.getByRole("main", { name: "Onboarding step" }).scrollTop).toBe(0);
    expect(screen.getByRole("button", { name: "Continue" })).toBe(continueButton);
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

describe("ConnectAIStep", () => {
  let mock: MockTransport;

  beforeEach(async () => {
    mock = await setupMockApp();
    // Start without a Gemini key so the step has to wait for one.
    const current = useSettingsStore.getState().settings;
    if (!current) throw new Error("settings not loaded");
    await useSettingsStore.getState().update({
      ai: {
        providers: current.ai.providers.map((p) => (p.id === "gemini" ? { ...p, hasApiKey: false } : p)),
      },
    });
  });

  it("verifies a freshly saved key and shows an error banner instead of 'connected' when it fails", async () => {
    const user = userEvent.setup();
    await mock.invoke("dev_simulate", { simulation: { type: "ai_failure", code: "config.api_key_invalid" } });
    render(
      <TooltipProvider>
        <ConnectAIStep onReady={() => {}} />
      </TooltipProvider>,
    );
    expect(screen.queryByText("Gemini is connected")).not.toBeInTheDocument();

    await user.type(screen.getByLabelText("Google AI Studio API key"), "AIza-not-a-real-key{Enter}");

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("API key rejected");
    expect(screen.queryByText("Gemini is connected")).not.toBeInTheDocument();
    expect(useSettingsStore.getState().settings?.ai.providers.find((p) => p.id === "gemini")?.hasApiKey).toBe(
      true,
    );

    // Retry runs the connection test again — the simulated failure was one-shot.
    await user.click(screen.getByRole("button", { name: "Retry" }));
    await screen.findByText("Gemini is connected");
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("shows 'connected' only after the saved key passes the connection test", async () => {
    const user = userEvent.setup();
    render(
      <TooltipProvider>
        <ConnectAIStep onReady={() => {}} />
      </TooltipProvider>,
    );
    await user.type(screen.getByLabelText("Google AI Studio API key"), "AIza-fine{Enter}");
    await screen.findByText("Gemini is connected");
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });
});

describe("ConnectAIStep — subscription branch (ADR 0009)", () => {
  beforeEach(async () => {
    await setupMockApp();
  });

  it("offers the three subscriptions, asks for consent once and reports the connected plan", async () => {
    const user = userEvent.setup();
    const onReady = vi.fn();
    render(
      <TooltipProvider>
        <ConnectAIStep onReady={onReady} />
      </TooltipProvider>,
    );
    await screen.findByRole("heading", { name: "Connect Gemini" });
    await user.click(screen.getByRole("button", { name: "Use a subscription I already pay for" }));
    const panel = screen.getByTestId("onboarding-subscriptions");
    expect(within(panel).getByRole("button", { name: "ChatGPT" })).toBeInTheDocument();
    expect(within(panel).getByRole("button", { name: "Claude" })).toBeInTheDocument();
    expect(within(panel).getByRole("button", { name: "Google AI" })).toBeInTheDocument();

    await user.click(within(panel).getByRole("button", { name: "Claude" }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("Use your Claude subscription with Bluey?")).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Continue in browser" }));
    await waitFor(() => expect(screen.getByText("Claude connected · Claude Max 5×")).toBeInTheDocument());
    expect(useSettingsStore.getState().settings?.experimental.acceptedAccountConsents).toEqual(["claude"]);
    expect(onReady).toHaveBeenLastCalledWith(true);
  });

  it("hides the branch when subscription accounts are switched off", async () => {
    await useSettingsStore.getState().update({ experimental: { subscriptionAccounts: false } });
    render(
      <TooltipProvider>
        <ConnectAIStep onReady={() => {}} />
      </TooltipProvider>,
    );
    await screen.findByRole("heading", { name: "Connect Gemini" });
    expect(screen.queryByRole("button", { name: "Use a subscription I already pay for" })).not.toBeInTheDocument();
  });
});
