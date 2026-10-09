import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { TooltipProvider } from "@/components/ui/Tooltip";
import { ReadyStep, TestAIStep } from "@/features/onboarding/steps/tests";
import type { MockTransport } from "@/lib/tauri/mock";
import type { AIProviderConfig } from "@/lib/types";
import { useSettingsStore } from "@/stores/settingsStore";
import { setupInterceptedApp, setupMockApp } from "./helpers";

/** Rewrite the providers (and optionally the roles) the way Settings → AI would. */
async function patchAi(
  providers: (p: AIProviderConfig) => AIProviderConfig,
  models: Record<string, { providerId: string; model: string } | null> = {},
): Promise<void> {
  const current = useSettingsStore.getState().settings;
  if (!current) throw new Error("settings not loaded");
  await useSettingsStore.getState().update({
    ai: { providers: current.ai.providers.map(providers), models: { ...current.ai.models, ...models } },
  });
}

const noop = () => {};

function renderStep(Step: typeof ReadyStep) {
  return render(
    <TooltipProvider>
      <Step onReady={noop} />
    </TooltipProvider>,
  );
}

describe("onboarding readiness (ONB-001)", () => {
  let mock: MockTransport;

  beforeEach(async () => {
    mock = await setupMockApp();
  });

  it("says ready only when the router can route an answer", async () => {
    renderStep(ReadyStep);
    await screen.findByText("Bluey is ready");
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("names why Bluey can't answer when the assigned provider has no key", async () => {
    await patchAi((p) => (p.id === "gemini" ? { ...p, hasApiKey: false } : p));
    renderStep(ReadyStep);

    await screen.findByText("Bluey can't answer yet");
    expect(screen.queryByText("Bluey is ready")).not.toBeInTheDocument();
    const alert = screen.getByRole("alert");
    expect(alert).toHaveTextContent("Google Gemini has no API key yet");
  });

  it("says not ready when Cloud AI is off, whatever the keys", async () => {
    await useSettingsStore.getState().update({ privacy: { cloudAiEnabled: false } });
    renderStep(ReadyStep);
    await screen.findByText("Bluey can't answer yet");
  });

  it("tests the provider and model the router picks, not the first enabled provider", async () => {
    const user = userEvent.setup();
    const foundry = { providerId: "azure-foundry", model: "gpt-6-astra" };
    await patchAi((p) => (p.id === "gemini" ? { ...p, hasApiKey: false } : p), {
      fast: foundry,
      default: foundry,
    });
    const invoke = vi.spyOn(mock, "invoke");
    renderStep(TestAIStep);

    await user.click(await screen.findByRole("button", { name: "Test connection" }));
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("ai_test_connection", {
        providerId: "azure-foundry",
        model: "gpt-6-astra",
      }),
    );
  });

  it("shows the cause instead of a test button when nothing routes", async () => {
    await patchAi((p) => ({ ...p, hasApiKey: false }));
    renderStep(TestAIStep);

    await screen.findByText("Connect an AI provider");
    expect(screen.getByRole("alert")).toHaveTextContent("has no API key yet");
    expect(screen.queryByRole("button", { name: "Test connection" })).not.toBeInTheDocument();
  });

  // Rust's router also routes to connected subscription accounts (ai/mod.rs `providers()`),
  // which are not in settings.ai.providers; the mock does not route them, so the test answers
  // as Rust would.
  it("tests a connected account the router picks, with no API key anywhere", async () => {
    await patchAi((p) => ({ ...p, hasApiKey: false }));
    const { transport } = await setupInterceptedApp();
    transport.intercept("ai_readiness", async () => ({ ok: true, providerId: "chatgpt", model: "gpt-5.5", vision: true }));
    const tested: unknown[] = [];
    transport.intercept("ai_test_connection", async (args) => {
      tested.push(args);
      return { ok: true, providerId: args.providerId, model: args.model, latencyMs: 120 };
    });
    const user = userEvent.setup();
    renderStep(TestAIStep);

    await user.click(await screen.findByRole("button", { name: "Test connection" }));
    expect(screen.getByText(/tiny request to ChatGPT/)).toBeInTheDocument();
    expect(screen.queryByText("Connect an AI provider")).not.toBeInTheDocument();
    await screen.findByText(/Connected · gpt-5.5/);
    expect(tested).toEqual([{ providerId: "chatgpt", model: "gpt-5.5" }]);
  });
});
