import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";

import { TooltipProvider } from "@/components/ui/Tooltip";
import AITab from "@/features/settings/tabs/AITab";
import { useSettingsStore } from "@/stores/settingsStore";
import { setupMockApp } from "./helpers";

function renderTab() {
  return render(
    <TooltipProvider>
      <AITab />
    </TooltipProvider>,
  );
}

const models = () => useSettingsStore.getState().settings?.ai.models;

describe("AITab", () => {
  beforeEach(async () => {
    await setupMockApp();
  });

  it("lists Gemini first, marked as the default provider", async () => {
    renderTab();
    const toggles = await screen.findAllByRole("switch", { name: /^Enable / });
    expect(toggles.map((t) => t.getAttribute("aria-label"))).toEqual([
      "Enable Google Gemini",
      "Enable Azure Foundry",
      "Enable Claude (Foundry)",
    ]);
    expect(screen.getByText("Default", { selector: "span" })).toBeInTheDocument();
    expect(screen.getByLabelText("Default AI provider")).toHaveValue("gemini");
    expect(screen.getByText(/aistudio.google.com\/apikey/)).toBeInTheDocument();
  });

  it("switching the default provider re-points every role at its recommended models", async () => {
    const user = userEvent.setup();
    renderTab();
    await screen.findByLabelText("Default AI provider");

    await user.selectOptions(screen.getByLabelText("Default AI provider"), "azure-foundry");
    await waitFor(() =>
      expect(useSettingsStore.getState().settings?.ai.bootstrapProvider).toBe("azure-foundry"),
    );
    expect(models()?.research).toEqual({ providerId: "azure-foundry", model: "gpt-6-astra" }); // was Anthropic
    expect(models()?.default).toEqual({ providerId: "azure-foundry", model: "gpt-5.6-terra" });

    await user.selectOptions(screen.getByLabelText("Default AI provider"), "gemini");
    await waitFor(() => expect(models()?.default?.providerId).toBe("gemini"));
    expect(models()?.embedding).toEqual({ providerId: "gemini", model: "gemini-embedding-2" });
    expect(useSettingsStore.getState().settings?.ai.bootstrapProvider).toBe("gemini");
  });

  it("'Use recommended models' on a card applies that provider's presets and reveals the embedding size", async () => {
    const user = userEvent.setup();
    renderTab();
    const buttons = await screen.findAllByRole("button", { name: /Use recommended models/ });
    expect(screen.queryByLabelText("Embedding dimensions")).not.toBeInTheDocument();

    await user.click(buttons[0]!); // Gemini card comes first
    await waitFor(() =>
      expect(models()?.default).toEqual({ providerId: "gemini", model: "gemini-3.8-flash" }),
    );
    const dims = await screen.findByLabelText("Embedding dimensions");
    await user.selectOptions(dims, "1536");
    await waitFor(() => expect(useSettingsStore.getState().settings?.ai.embeddingDimensions).toBe(1536));
  });

  it("shows the Anthropic agent key only for the Claude research backend", async () => {
    const user = userEvent.setup();
    renderTab();
    await screen.findByLabelText("Research backend");
    expect(screen.queryByLabelText("Anthropic agent API key")).not.toBeInTheDocument();
    expect(screen.getByText(/Gemini function calling/)).toBeInTheDocument();

    await user.selectOptions(screen.getByLabelText("Research backend"), "claude");
    await waitFor(() => expect(useSettingsStore.getState().settings?.ai.researchBackend).toBe("claude"));
    expect(await screen.findByLabelText("Anthropic agent API key")).toBeInTheDocument();
  });

  it("offers a one-click recommended model per role", async () => {
    const user = userEvent.setup();
    renderTab();
    const [useGemini] = await screen.findAllByRole("button", { name: /Use recommended models/ });
    await user.click(useGemini!);
    await waitFor(() => expect(models()?.fast?.providerId).toBe("gemini"));

    // Moving the Fast role to Foundry keeps the Gemini model id → the row offers Foundry's preset.
    await user.selectOptions(screen.getByLabelText("Fast provider"), "azure-foundry");
    await waitFor(() => expect(models()?.fast?.providerId).toBe("azure-foundry"));
    expect(models()?.fast?.model).toBe("gemini-3.5-flash-lite");

    await user.click(await screen.findByRole("button", { name: "Use gpt-5.6-luna" }));
    await waitFor(() =>
      expect(models()?.fast).toEqual({ providerId: "azure-foundry", model: "gpt-5.6-luna" }),
    );
    expect(screen.queryByRole("button", { name: "Use gpt-5.6-luna" })).not.toBeInTheDocument();
  });
});
