import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";

import { useToastStore } from "@/components/ui/toast-store";
import { SecretKeyField } from "@/features/settings/SecretKeyField";
import { bluey } from "@/lib/tauri/api";
import { SECRET_KEYS } from "@/lib/tauri/commands";
import { deferred } from "../fixtures/helpers/fake-transport";
import { setupInterceptedApp, setupMockApp } from "./helpers";

const KEY = SECRET_KEYS.providerApiKey("anthropic");

describe("SecretKeyField", () => {
  beforeEach(async () => {
    await setupMockApp();
    useToastStore.setState({ toasts: [] });
  });

  it("saves on Enter and afterwards only shows that a key exists", async () => {
    const user = userEvent.setup();
    render(<SecretKeyField secretKey={KEY} aria-label="Anthropic API key" />);
    const input = await screen.findByLabelText("Anthropic API key");
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();

    await user.type(input, "sk-ant-test{Enter}");

    await screen.findByText("Key saved ••••");
    expect(await bluey.secrets.has({ key: KEY })).toBe(true);
    expect(useToastStore.getState().toasts.map((t) => t.message)).toContain("Key saved");
    // The WebView never re-displays a stored key: no field holds the value any more.
    expect(screen.queryByLabelText("Anthropic API key")).not.toBeInTheDocument();
  });

  it("Cancel drops the typed replacement key", async () => {
    const user = userEvent.setup();
    await bluey.secrets.set({ key: KEY, value: "sk-ant-existing" });
    render(<SecretKeyField secretKey={KEY} aria-label="Anthropic API key" />);
    await screen.findByText("Key saved ••••");

    await user.click(screen.getByRole("button", { name: "Replace" }));
    await user.type(screen.getByLabelText("Anthropic API key"), "sk-ant-draft");
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.getByText("Key saved ••••")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Replace" }));
    expect(screen.getByLabelText("Anthropic API key")).toHaveValue("");
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    expect(await bluey.secrets.has({ key: KEY })).toBe(true);
  });

  it("ignores Enter while a save is in flight", async () => {
    const user = userEvent.setup();
    const { transport } = await setupInterceptedApp();
    const gate = deferred<void>();
    let writes = 0;
    transport.intercept("secrets_set", async (_args, next) => {
      writes += 1;
      await gate.promise;
      return next();
    });
    render(<SecretKeyField secretKey={KEY} aria-label="Anthropic API key" />);
    const input = await screen.findByLabelText("Anthropic API key");

    await user.type(input, "sk-ant-slow{Enter}");
    await waitFor(() => expect(writes).toBe(1));
    await user.keyboard("{Enter}{Enter}");
    expect(writes).toBe(1);
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();

    gate.resolve();
    await screen.findByText("Key saved ••••");
    expect(writes).toBe(1);
  });
});
