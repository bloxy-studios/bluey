import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";

import { useToastStore } from "@/components/ui/toast-store";
import { TooltipProvider } from "@/components/ui/Tooltip";
import AITab from "@/features/settings/tabs/AITab";
import type { MockTransport } from "@/lib/tauri/mock";
import { useAccountsStore } from "@/stores/accountsStore";
import { useSettingsStore } from "@/stores/settingsStore";
import { setupMockApp } from "./helpers";

function renderTab() {
  return render(
    <TooltipProvider>
      <AITab />
    </TooltipProvider>,
  );
}

const card = (providerId: string) => screen.getByTestId(`account-card-${providerId}`);

/** Skip the consent dialog for the given providers (it is tested on its own). */
async function acceptConsents(...ids: string[]) {
  await useSettingsStore.getState().update({ experimental: { acceptedAccountConsents: ids } });
}

describe("Settings → AI → Accounts (MockTransport)", () => {
  let mock: MockTransport;

  beforeEach(async () => {
    mock = await setupMockApp();
  });

  it("lists the three subscription providers as disconnected cards, above the API-key providers", async () => {
    renderTab();
    await screen.findByTestId("accounts-list");
    for (const id of ["chatgpt", "claude", "antigravity"]) {
      expect(within(card(id)).getByText("Not connected")).toBeInTheDocument();
    }
    expect(within(card("chatgpt")).getByText(/Fingerprint codex\/0\.154\.0 · captured 2026-09-11/)).toBeInTheDocument();
    expect(within(card("claude")).getByRole("button", { name: "Import Claude Code sign-in" })).toBeInTheDocument();
    // The API-key providers are untouched.
    expect(screen.getAllByRole("switch", { name: /^Enable / })).toHaveLength(3);
    expect(screen.getByTestId("accounts-list").compareDocumentPosition(screen.getByText("Providers"))).toBe(
      Node.DOCUMENT_POSITION_FOLLOWING,
    );
  });

  it("asks for consent once per provider, then connects through the simulated browser", async () => {
    const user = userEvent.setup();
    renderTab();
    await screen.findByTestId("accounts-list");

    await user.click(within(card("claude")).getByRole("button", { name: "Connect Claude" }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("Use your Claude subscription with Bluey?")).toBeInTheDocument();
    expect(within(dialog).getByText(/Extra usage/)).toBeInTheDocument();
    expect(within(dialog).getByText(/falls back to your API key/)).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Continue in browser" }));

    await waitFor(() =>
      expect(useSettingsStore.getState().settings?.experimental.acceptedAccountConsents).toEqual(["claude"]),
    );
    await waitFor(() => expect(within(card("claude")).getByText("Claude Max 5× · connected")).toBeInTheDocument());
    expect(within(card("claude")).getByText("jordan@example.com")).toBeInTheDocument();
    await waitFor(() => expect(within(card("claude")).getByText(/4 models · fetched/)).toBeInTheDocument());
    expect(useAccountsStore.getState().catalogs.claude?.models.map((m) => m.id)).toContain("claude-sonnet-5");

    // Disconnect, then reconnect: consent is remembered.
    await user.click(within(card("claude")).getByRole("button", { name: "Disconnect" }));
    await waitFor(() => expect(within(card("claude")).getByText("Not connected")).toBeInTheDocument());
    expect(mock.disconnectedAccounts).toEqual(["claude"]);
    expect(useAccountsStore.getState().catalogs.claude).toBeUndefined();
    await user.click(within(card("claude")).getByRole("button", { name: "Connect Claude" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    await waitFor(() => expect(within(card("claude")).getByText("Claude Max 5× · connected")).toBeInTheDocument());
  });

  it("cancelling the consent dialog connects nothing", async () => {
    const user = userEvent.setup();
    renderTab();
    await screen.findByTestId("accounts-list");
    await user.click(within(card("chatgpt")).getByRole("button", { name: "Connect ChatGPT" }));
    await user.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(within(card("chatgpt")).getByText("Not connected")).toBeInTheDocument();
    expect(useSettingsStore.getState().settings?.experimental.acceptedAccountConsents).toEqual([]);
  });

  it("shows the device code, lets the flow be cancelled, and takes a pasted code", async () => {
    const user = userEvent.setup();
    renderTab();
    await screen.findByTestId("accounts-list");
    await acceptConsents("chatgpt", "antigravity");

    mock.nextAccountFlow = "device_code";
    mock.nextAccountOutcome = "hang";
    await user.click(within(card("chatgpt")).getByRole("button", { name: "Connect ChatGPT" }));
    expect(await within(card("chatgpt")).findByLabelText("Device code")).toHaveTextContent("BLUEY-4821");
    expect(within(card("chatgpt")).getByText(/auth\.openai\.com\/codex\/device/)).toBeInTheDocument();
    await user.click(within(card("chatgpt")).getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(within(card("chatgpt")).getByText("Not connected")).toBeInTheDocument());

    mock.nextAccountFlow = "manual_code";
    await user.click(within(card("antigravity")).getByRole("button", { name: "Connect Google AI" }));
    const input = await within(card("antigravity")).findByLabelText("Google AI sign-in code");
    await user.type(input, "abc123#state-1");
    await user.click(within(card("antigravity")).getByRole("button", { name: "Submit code" }));
    await waitFor(() => expect(within(card("antigravity")).getByText("Google AI Pro · connected")).toBeInTheDocument());
    expect(within(card("antigravity")).getByText(/project bluey-owner-4f2a/)).toBeInTheDocument();
  });

  it("renders rate limits, expired sign-ins and fingerprint drift with the matching actions", async () => {
    const user = userEvent.setup();
    renderTab();
    await screen.findByTestId("accounts-list");
    await acceptConsents("chatgpt", "claude", "antigravity");

    mock.nextAccountOutcome = "rate_limited";
    await user.click(within(card("chatgpt")).getByRole("button", { name: "Connect ChatGPT" }));
    await waitFor(() => expect(within(card("chatgpt")).getByText(/5h limit · resets/)).toBeInTheDocument());
    expect(within(card("chatgpt")).getByText(/plan window is used up/)).toBeInTheDocument();
    expect(within(card("chatgpt")).getByRole("button", { name: "Disconnect" })).toBeInTheDocument();

    mock.nextAccountOutcome = "needs_reauth";
    await user.click(within(card("claude")).getByRole("button", { name: "Connect Claude" }));
    expect(await within(card("claude")).findByRole("button", { name: "Reconnect" })).toBeInTheDocument();
    expect(within(card("claude")).getByText("Sign in again")).toBeInTheDocument();

    mock.nextAccountOutcome = "fingerprint_drift";
    await user.click(within(card("antigravity")).getByRole("button", { name: "Connect Google AI" }));
    await waitFor(() =>
      expect(within(card("antigravity")).getByText("Provider stopped recognising Bluey")).toBeInTheDocument(),
    );
    expect(within(card("antigravity")).getByText(/Third-party apps now draw from your extra usage/)).toBeInTheDocument();
    expect(within(card("antigravity")).getByText(/Your API key is used instead/)).toBeInTheDocument();
    expect(within(card("antigravity")).getByRole("button", { name: "Connect Google AI" })).toBeInTheDocument();
  });

  it("a denied sign-in returns to disconnected and surfaces the reason", async () => {
    const user = userEvent.setup();
    renderTab();
    await screen.findByTestId("accounts-list");
    await acceptConsents("chatgpt");
    mock.nextAccountOutcome = "denied";
    await user.click(within(card("chatgpt")).getByRole("button", { name: "Connect ChatGPT" }));
    await waitFor(() =>
      expect(useToastStore.getState().toasts.some((toast) => toast.title === "Sign-in not completed")).toBe(true),
    );
    expect(within(card("chatgpt")).getByText("Not connected")).toBeInTheDocument();
  });

  it("disconnecting unassigns the roles that pointed at the provider and refreshes models on demand", async () => {
    const user = userEvent.setup();
    renderTab();
    await screen.findByTestId("accounts-list");
    await acceptConsents("chatgpt");
    await user.click(within(card("chatgpt")).getByRole("button", { name: "Connect ChatGPT" }));
    await waitFor(() => expect(within(card("chatgpt")).getByText("ChatGPT Plus · connected")).toBeInTheDocument());

    const before = useAccountsStore.getState().catalogs.chatgpt?.fetchedAt;
    await user.click(within(card("chatgpt")).getByRole("button", { name: "Refresh models" }));
    await waitFor(() => expect(useAccountsStore.getState().catalogs.chatgpt?.models).toHaveLength(5));
    expect(before).toBeDefined();

    const settings = useSettingsStore.getState().settings!;
    await useSettingsStore.getState().update({
      ai: { models: { ...settings.ai.models, default: { providerId: "chatgpt", model: "gpt-6-astra" } } },
    });
    await user.click(within(card("chatgpt")).getByRole("button", { name: "Disconnect" }));
    await waitFor(() => expect(within(card("chatgpt")).getByText("Not connected")).toBeInTheDocument());
    await waitFor(() => expect(useSettingsStore.getState().settings?.ai.models.default).toBeNull());
    expect(useSettingsStore.getState().settings?.ai.models.fast).toEqual({
      providerId: "gemini",
      model: "gemini-3.5-flash-lite",
    });
  });

  it("the switch hides the cards and blocks new sign-ins without a rebuild", async () => {
    const user = userEvent.setup();
    renderTab();
    await screen.findByTestId("accounts-list");
    await user.click(screen.getByRole("switch", { name: "Subscription accounts" }));
    await waitFor(() => expect(screen.queryByTestId("accounts-list")).not.toBeInTheDocument());
    expect(screen.getByText("Off — Bluey uses API keys only.")).toBeInTheDocument();
    expect(await useAccountsStore.getState().connect("claude")).toBeNull();
    expect(useAccountsStore.getState().lastError?.code).toBe("account.disabled");
  });
});
