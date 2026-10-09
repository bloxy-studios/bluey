import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";

import { readOnboardingStep, saveOnboardingStep } from "@/features/onboarding/progress";
import PrivacyTab from "@/features/settings/tabs/PrivacyTab";
import { bluey } from "@/lib/tauri/api";
import { useSettingsStore } from "@/stores/settingsStore";
import { setupInterceptedApp, setupMockApp } from "./helpers";

describe("PrivacyTab raw audio and cloud AI", () => {
  beforeEach(async () => {
    await setupMockApp();
  });

  it("shows raw audio as never kept and offers no retention it cannot honour (FEATURE-003)", async () => {
    // A value saved by an older build must not make the tab claim recordings are kept.
    await useSettingsStore.getState().update({ privacy: { storeRawAudio: "custom", rawAudioRetentionMinutes: 45 } });
    render(<PrivacyTab />);

    expect(await screen.findByText("Never kept")).toBeInTheDocument();
    expect(screen.getByText(/never written to disk/)).toBeInTheDocument();
    expect(screen.queryByLabelText("Raw audio retention")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("Raw audio retention minutes")).not.toBeInTheDocument();
    expect(screen.queryByText(/discarded after/)).not.toBeInTheDocument();
  });

  it("does not promise that Privacy mode hides Bluey from modern screen sharing (SEC-004)", async () => {
    // Protection follows the saved display mode (as in Rust), so turn Privacy mode on first.
    await useSettingsStore.getState().update({ privacy: { displayMode: "privacy" } });
    render(<PrivacyTab />);
    await waitFor(() => expect(screen.getByText(/macOS 15 and later may still show Bluey/)).toBeInTheDocument());
    expect(screen.queryByText(/excludes its windows from screen recordings/)).not.toBeInTheDocument();
  });

  it("toggles Cloud AI", async () => {
    const user = userEvent.setup();
    render(<PrivacyTab />);
    await user.click(screen.getByLabelText("Cloud AI"));
    await waitFor(() => expect(useSettingsStore.getState().settings?.privacy.cloudAiEnabled).toBe(false));
  });
});

describe("PrivacyTab display mode (UX-004, SEC-004)", () => {
  it("follows a display mode saved from the HUD eye or the tray", async () => {
    await setupMockApp();
    render(<PrivacyTab />);
    expect(await screen.findByText(/visible in screen shares/)).toBeInTheDocument();

    // The HUD eye and the tray only patch settings; the tab must not keep the old note.
    await act(() => useSettingsStore.getState().update({ privacy: { displayMode: "privacy" } }));
    expect(await screen.findByText(/macOS 15 and later may still show Bluey/)).toBeInTheDocument();
    expect(screen.queryByText(/visible in screen shares/)).not.toBeInTheDocument();
  });

  it("only saves the mode, so a failed save never flips native protection", async () => {
    const { transport } = await setupInterceptedApp();
    const applied: boolean[] = [];
    transport.intercept("capture_set_protection", (args, next) => {
      applied.push(args.enabled);
      return next();
    });
    transport.intercept("settings_update", () => {
      throw { kind: "storage", code: "storage.write", message: "The settings could not be saved.", recoverable: true };
    });
    const user = userEvent.setup();
    render(<PrivacyTab />);

    await user.click(await screen.findByRole("tab", { name: "Privacy" }));
    await waitFor(() => expect(useSettingsStore.getState().lastError).not.toBeNull());
    expect(applied).toEqual([]);
    expect((await bluey.capture.getProtection()).enabled).toBe(false);
    expect(screen.getByRole("tab", { name: "Standard" })).toHaveAttribute("aria-selected", "true");
  });
});

describe("PrivacyTab data actions (DEBT-011)", () => {
  it("offers no Clear AI cache action, since nothing is ever cached", async () => {
    await setupMockApp();
    render(<PrivacyTab />);

    expect(await screen.findByRole("button", { name: "Clear transcripts" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Reset Bluey" })).toBeInTheDocument();
    expect(screen.queryByText(/AI cache/i)).not.toBeInTheDocument();
    expect("clearAiCache" in bluey.data).toBe(false);
  });
});

describe("PrivacyTab reset (DOC-003)", () => {
  it("returns to onboarding with the HUD hidden after a successful reset", async () => {
    const { transport } = await setupInterceptedApp();
    const calls: string[] = [];
    transport.intercept("data_reset_all", (_args, next) => {
      calls.push("reset");
      return next();
    });
    transport.intercept("window_open", async (args) => {
      calls.push(`open:${args.label}`);
    });
    transport.intercept("panel_hide", (_args, next) => {
      calls.push("hide");
      return next();
    });
    saveOnboardingStep("permissions");
    const user = userEvent.setup();
    render(<PrivacyTab />);

    await user.click(await screen.findByRole("button", { name: "Reset Bluey" }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Reset Bluey" }));

    await waitFor(() => expect(calls).toEqual(["reset", "open:onboarding", "hide"]));
    expect(readOnboardingStep()).toBeNull();
    expect((await bluey.panel.getState()).visible).toBe(false);
  });
});
