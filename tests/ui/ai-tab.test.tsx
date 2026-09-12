import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
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

/** Patch the loaded settings through the store (arrays / role maps replace wholesale). */
async function patchAi(
  patch: (
    current: NonNullable<ReturnType<typeof useSettingsStore.getState>["settings"]>["ai"],
  ) => Partial<NonNullable<ReturnType<typeof useSettingsStore.getState>["settings"]>["ai"]>,
) {
  const current = useSettingsStore.getState().settings;
  if (!current) throw new Error("settings not loaded");
  await useSettingsStore.getState().update({ ai: patch(current.ai) });
}

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
    expect(models()?.research).toEqual({ providerId: "azure-foundry", model: "gpt-6-astra" }); // was Gemini
    expect(models()?.default).toEqual({ providerId: "azure-foundry", model: "gpt-5.6-terra" });

    await user.selectOptions(screen.getByLabelText("Default AI provider"), "gemini");
    await waitFor(() => expect(models()?.default?.providerId).toBe("gemini"));
    expect(models()?.embedding).toEqual({ providerId: "gemini", model: "gemini-embedding-2" });
    expect(useSettingsStore.getState().settings?.ai.bootstrapProvider).toBe("gemini");
  });

  it("keeps providers without a key out of reach as the default", async () => {
    renderTab();
    const select = await screen.findByLabelText("Default AI provider");
    expect(within(select).getByRole("option", { name: "Claude (Foundry) (no key)" })).toBeDisabled();
    expect(within(select).getByRole("option", { name: "Azure Foundry" })).toBeEnabled();
    expect(
      screen.getByText("Switching applies the provider's recommended models to every role."),
    ).toBeInTheDocument();

    // Even a programmatic change to the keyless provider is refused: nothing is re-pointed.
    fireEvent.change(select, { target: { value: "anthropic" } });
    await waitFor(() => expect(select).toHaveValue("gemini"));
    expect(useSettingsStore.getState().settings?.ai.bootstrapProvider).toBe("gemini");
    expect(models()?.default?.providerId).toBe("gemini");
  });

  it("always lists the current default, even once it is keyless and disabled", async () => {
    await patchAi((ai) => ({
      providers: ai.providers.map((p) =>
        p.id === "gemini" ? { ...p, hasApiKey: false, enabled: false } : p,
      ),
    }));
    renderTab();
    const select = await screen.findByLabelText("Default AI provider");
    expect(select).toHaveValue("gemini");
    expect(within(select).getByRole("option", { name: "Google Gemini (no key)" })).toBeDisabled();
  });

  it("an unassigned role remembers the picked provider and saves once a model is committed with Enter", async () => {
    const user = userEvent.setup();
    await patchAi((ai) => ({
      models: { ...ai.models, research: null },
      providers: ai.providers.map((p) => (p.id === "anthropic" ? { ...p, enabled: false } : p)),
    }));
    renderTab();
    const providerSelect = await screen.findByLabelText("Research provider");
    expect(providerSelect).toHaveValue("");
    expect(within(providerSelect).getByRole("option", { name: "Choose provider" })).toBeInTheDocument();
    expect(
      within(providerSelect).getByRole("option", { name: "Claude (Foundry) (disabled)" }),
    ).toBeDisabled();
    // Assigned rows don't carry the placeholder.
    expect(
      within(screen.getByLabelText("Fast provider")).queryByRole("option", { name: "Choose provider" }),
    ).toBeNull();

    await user.selectOptions(providerSelect, "azure-foundry");
    expect(providerSelect).toHaveValue("azure-foundry");
    expect(models()?.research).toBeNull(); // a provider alone assigns nothing (and nothing wrong)

    await user.type(screen.getByLabelText("Research model"), "gpt-5.6-sol{Enter}");
    await waitFor(() =>
      expect(models()?.research).toEqual({ providerId: "azure-foundry", model: "gpt-5.6-sol" }),
    );
    expect(screen.getByLabelText("Research provider")).toHaveValue("azure-foundry");
  });

  it("a model typed before the provider is picked is saved with that provider", async () => {
    const user = userEvent.setup();
    await patchAi((ai) => ({ models: { ...ai.models, research: null } }));
    renderTab();
    await screen.findByLabelText("Research provider");

    await user.type(screen.getByLabelText("Research model"), "gpt-5.6-sol");
    await user.tab(); // blur without a provider saves nothing …
    expect(models()?.research).toBeNull();
    expect(screen.getByLabelText("Research model")).toHaveValue("gpt-5.6-sol");

    await user.selectOptions(screen.getByLabelText("Research provider"), "azure-foundry"); // … the pick does
    await waitFor(() =>
      expect(models()?.research).toEqual({ providerId: "azure-foundry", model: "gpt-5.6-sol" }),
    );
  });

  it("offers the picked provider's recommended model to an unassigned role", async () => {
    const user = userEvent.setup();
    await patchAi((ai) => ({ models: { ...ai.models, research: null } }));
    renderTab();
    await user.selectOptions(await screen.findByLabelText("Research provider"), "gemini");
    await user.click(await screen.findByRole("button", { name: "Use gemini-3.8-flash" }));
    await waitFor(() =>
      expect(models()?.research).toEqual({ providerId: "gemini", model: "gemini-3.8-flash" }),
    );
    expect(screen.queryByRole("button", { name: "Use gemini-3.8-flash" })).not.toBeInTheDocument();
  });

  it("'Use recommended models' on a card applies that provider's presets and toggles the embedding size", async () => {
    const user = userEvent.setup();
    renderTab();
    const buttons = await screen.findAllByRole("button", { name: /Use recommended models/ });
    // Seeded on Gemini → the MRL size select is there; Foundry embeddings hide it …
    expect(screen.getByLabelText("Embedding dimensions")).toBeInTheDocument();
    await user.click(buttons[1]!); // Foundry card comes second
    await waitFor(() =>
      expect(models()?.embedding).toEqual({ providerId: "azure-foundry", model: "text-embedding-3-small" }),
    );
    await waitFor(() => expect(screen.queryByLabelText("Embedding dimensions")).not.toBeInTheDocument());

    // … and Gemini's recommended models bring it back.
    await user.click(buttons[0]!); // Gemini card comes first
    await waitFor(() =>
      expect(models()?.default).toEqual({ providerId: "gemini", model: "gemini-3.8-flash" }),
    );
    const dims = await screen.findByLabelText("Embedding dimensions");
    await user.selectOptions(dims, "1536");
    await waitFor(() => expect(useSettingsStore.getState().settings?.ai.embeddingDimensions).toBe(1536));
  });

  it("lets the user choose how live suggestions surface", async () => {
    const user = userEvent.setup();
    renderTab();
    const display = await screen.findByLabelText("Show suggestions");
    expect(display).toHaveValue("live");
    expect(screen.getByText(/streams into the HUD/)).toBeInTheDocument();

    await user.selectOptions(display, "on_request");
    await waitFor(() => expect(useSettingsStore.getState().settings?.ai.suggestionDisplay).toBe("on_request"));
    expect(screen.getByText(/⌘⇧↵ shows it/)).toBeInTheDocument();

    await user.click(screen.getByRole("switch", { name: "Prepare answers while listening" }));
    await waitFor(() => expect(useSettingsStore.getState().settings?.ai.proactivePreparation).toBe(false));
    expect(screen.getByLabelText("Show suggestions")).toBeDisabled();
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
