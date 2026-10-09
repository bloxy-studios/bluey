import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";

import { Toasts } from "@/components/ui/Toast";
import { useToastStore } from "@/components/ui/toast-store";
import { TooltipProvider } from "@/components/ui/Tooltip";
import AITab from "@/features/settings/tabs/AITab";
import { bluey } from "@/lib/tauri/api";
import { useSettingsStore } from "@/stores/settingsStore";
import { setupInterceptedApp, setupMockApp } from "./helpers";

function renderTab() {
  return render(
    <TooltipProvider>
      <AITab />
      <Toasts />
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

  it("says what the Research role drives and where deep research picks its model (PROV-011)", async () => {
    renderTab();

    expect(await screen.findByText("Answers that need research. Deep research: see below")).toBeInTheDocument();
    expect(screen.getByText(/Uses the Research model when it comes from a Gemini provider/)).toBeInTheDocument();
    await useSettingsStore.getState().update({ ai: { researchBackend: "claude" } });
    expect(await screen.findByText(/Uses the Research model when it comes from an Anthropic provider/)).toBeInTheDocument();
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
    await waitFor(() =>
      expect(useSettingsStore.getState().settings?.ai.suggestionDisplay).toBe("on_request"),
    );
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

  it("disables the Claude backend when the installed agent is the lite build", async () => {
    const { transport } = await setupInterceptedApp();
    transport.intercept("research_available", async (_args, next) => ({
      ...(await next()),
      agentBackends: ["gemini"],
    }));
    renderTab();
    const claude = await screen.findByRole("option", { name: "Claude (full build only)" });
    expect(claude).toBeDisabled();
    expect(screen.getByRole("option", { name: "Gemini" })).toBeEnabled();
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

  it("says why an answer can't be routed, and clears once the provider is fixed (ONB-001)", async () => {
    await patchAi((ai) => ({
      providers: ai.providers.map((p) => (p.id === "gemini" ? { ...p, enabled: false } : p)),
    }));
    renderTab();
    expect(await screen.findByRole("alert")).toHaveTextContent("Google Gemini");

    await patchAi((ai) => ({ providers: ai.providers.map((p) => ({ ...p, enabled: true })) }));
    await waitFor(() => expect(screen.queryByRole("alert")).not.toBeInTheDocument());
  });
});

describe("AITab — removing a provider (UX-008)", () => {
  it("confirms with the roles it serves, then drops it, unassigns them and deletes its key", async () => {
    const mock = await setupMockApp();
    await mock.invoke("secrets_set", { key: "provider:gemini:api_key", value: "AIza-test" });
    const user = userEvent.setup();
    renderTab();
    const card = (await screen.findByText("Google Gemini", { selector: "span" })).closest("div.rounded-card");
    if (!(card instanceof HTMLElement)) throw new Error("Gemini card not found");
    expect(within(card).getByText(/^Used by Default, Fast/)).toBeInTheDocument();

    await user.click(within(card).getByRole("button", { name: "Remove" }));
    const dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveTextContent("deleted from the macOS Keychain");
    expect(dialog).toHaveTextContent("Default, Fast, Reasoning, Vision, Research, Transcription, Embedding");
    await user.click(within(dialog).getByRole("button", { name: "Remove provider" }));

    await waitFor(() =>
      expect(useSettingsStore.getState().settings?.ai.providers.map((p) => p.id)).not.toContain("gemini"),
    );
    expect(models()?.default).toBeNull();
    expect(models()?.embedding).toBeNull();
    expect(useSettingsStore.getState().settings?.ai).not.toHaveProperty("bootstrapProvider");
    expect(await mock.invoke("secrets_has", { key: "provider:gemini:api_key" })).toBe(false);
  });

  it("marks keyless providers in the card and in the role pickers", async () => {
    await setupMockApp();
    renderTab();
    const card = (await screen.findByText("Claude (Foundry)", { selector: "span" })).closest(
      "div.rounded-card",
    );
    if (!(card instanceof HTMLElement)) throw new Error("Anthropic card not found");
    expect(within(card).getByText("No key")).toBeInTheDocument();
    const picker = screen.getByLabelText("Default provider");
    expect(within(picker).getByRole("option", { name: "Claude (Foundry) (no key)" })).toBeInTheDocument();
  });
});

describe("AITab — a provider's first key (FEATURE-004)", () => {
  beforeEach(() => useToastStore.setState({ toasts: [] }));
  const saveKey = async (user: ReturnType<typeof userEvent.setup>, name: string) => {
    await user.type(await screen.findByLabelText(`${name} API key`), "sk-test-key");
    const card = screen.getByText(name, { selector: "span" }).closest("div.rounded-card");
    if (!(card instanceof HTMLElement)) throw new Error(`${name} card not found`);
    await user.click(within(card).getByRole("button", { name: "Save" }));
  };
  const undoToast = () =>
    useToastStore.getState().toasts.find((t) => t.message.includes("recommended models"));

  it("fills unassigned roles with its presets, makes it the default, and offers Undo", async () => {
    await setupMockApp();
    await patchAi((ai) => ({
      models: Object.fromEntries(Object.keys(ai.models).map((role) => [role, null])) as typeof ai.models,
      bootstrapProvider: null,
    }));
    const user = userEvent.setup();
    renderTab();
    await saveKey(user, "Claude (Foundry)");

    await waitFor(() => expect(useSettingsStore.getState().settings?.ai.bootstrapProvider).toBe("anthropic"));
    expect(models()?.default).toEqual({ providerId: "anthropic", model: "claude-sonnet-5" });
    const toast = await screen.findByText("Roles now use Claude (Foundry)'s recommended models");

    // Undo as the Settings window renders it, not through the store (the info toast used to drop it).
    await user.click(within(toast.closest("[role=status]") as HTMLElement).getByRole("button", { name: "Undo" }));
    await waitFor(() => expect(models()?.default).toBeNull());
    expect(useSettingsStore.getState().settings?.ai).not.toHaveProperty("bootstrapProvider");
  });

  it("leaves assigned roles and the default provider alone", async () => {
    await setupMockApp();
    const before = models();
    const user = userEvent.setup();
    renderTab();
    await saveKey(user, "Claude (Foundry)");

    await waitFor(async () =>
      expect(await bluey.secrets.has({ key: "provider:anthropic:api_key" })).toBe(true),
    );
    expect(models()).toEqual(before);
    expect(undoToast()).toBeUndefined();
  });
});
