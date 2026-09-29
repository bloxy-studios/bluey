import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";

import { useToastStore } from "@/components/ui/toast-store";
import { SavedCredentials } from "@/features/settings/SavedCredentials";
import { bluey } from "@/lib/tauri/api";
import { SECRET_KEYS } from "@/lib/tauri/commands";
import type { CredentialHealth } from "@/lib/types";
import { setupInterceptedApp } from "./helpers";

const SIGN_IN: CredentialHealth = {
  key: "auth:clerk_oauth_tokens",
  category: "sign_in",
  label: "Bluey sign-in",
  state: "locked",
  removable: false,
};

describe("SavedCredentials", () => {
  beforeEach(() => {
    useToastStore.setState({ toasts: [] });
  });

  it("lists names and states only, and removes an API key after confirming", async () => {
    const user = userEvent.setup();
    await setupInterceptedApp();
    await bluey.secrets.set({ key: SECRET_KEYS.exaApiKey, value: "exa-secret-value" });
    render(<SavedCredentials />);

    const row = (await screen.findByText("Exa")).closest("li");
    expect(row).not.toBeNull();
    expect(within(row!).getByText(/Research key · Saved/)).toBeInTheDocument();
    expect(document.body.textContent).not.toContain("exa-secret-value");

    await user.click(within(row!).getByRole("button", { name: "Remove" }));
    await user.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Remove key" }));

    await screen.findByText("Nothing saved yet.");
    expect(await bluey.secrets.has({ key: SECRET_KEYS.exaApiKey })).toBe(false);
  });

  it("offers Allow access for a locked item and points sign-in to Sign out instead of Remove", async () => {
    const user = userEvent.setup();
    const { transport } = await setupInterceptedApp();
    let state: CredentialHealth["state"] = "locked";
    const allowed: string[] = [];
    transport.intercept("secrets_health", () => Promise.resolve([{ ...SIGN_IN, state }]));
    transport.intercept("secrets_allow_access", (args) => {
      allowed.push(args.key);
      state = "present";
      return Promise.resolve(state);
    });
    render(<SavedCredentials />);

    const row = (await screen.findByText("Bluey sign-in")).closest("li")!;
    expect(within(row).getByText(/remove it with Sign out · Locked/)).toBeInTheDocument();
    expect(within(row).queryByRole("button", { name: "Remove" })).not.toBeInTheDocument();

    await user.click(within(row).getByRole("button", { name: "Allow access" }));

    await screen.findByText(/remove it with Sign out · Saved/);
    expect(allowed).toEqual([SIGN_IN.key]);
    expect(screen.queryByRole("button", { name: "Allow access" })).not.toBeInTheDocument();
  });
});
