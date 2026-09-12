import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";

import { TooltipProvider } from "@/components/ui/Tooltip";
import { HudPanel } from "@/features/hud/HudPanel";
import { derivePill } from "@/features/hud/state-pill";
import GeneralTab from "@/features/settings/tabs/GeneralTab";
import type { AppStatus, UpdateStatus } from "@/lib/types";
import { describeUpdateStatus, primaryUpdateAction, updatePillLabel } from "@/lib/updates/describe";
import { setEngine } from "@/stores/engine";
import { useSettingsStore } from "@/stores/settingsStore";
import { useUpdatesStore } from "@/stores/updatesStore";
import { FakeEngine, setupMockApp } from "./helpers";

function updateStatus(partial: Partial<UpdateStatus> = {}): UpdateStatus {
  return {
    phase: "idle",
    currentVersion: "0.1.0-dev",
    channel: "latest",
    automatic: true,
    supported: true,
    ...partial,
  };
}

function appStatus(partial: Partial<AppStatus> = {}): AppStatus {
  return { state: "ready", audioActive: false, modeId: "general", updatedAt: new Date().toISOString(), ...partial };
}

describe("updatesStore", () => {
  beforeEach(async () => {
    await setupMockApp();
  });

  it("loads the status from the backend and mirrors update.status events", async () => {
    const mock = await setupMockApp();
    expect(useUpdatesStore.getState().status).toMatchObject({ phase: "idle", channel: "latest", automatic: true });
    mock.emit("update.status", updateStatus({ phase: "available", available: { version: "0.2.0", channel: "latest" } }));
    expect(useUpdatesStore.getState().status?.available?.version).toBe("0.2.0");
  });

  it("with automatic updates on, a check ends installed and waiting for a relaunch", async () => {
    await useUpdatesStore.getState().check();
    const status = useUpdatesStore.getState().status;
    expect(status?.phase).toBe("ready");
    expect(status?.available?.version).toBe("0.2.0");
    expect(status?.lastCheckedAt).toBeTruthy();
  });

  it("notify-only stops at available; the nightly channel offers a nightly build", async () => {
    await useSettingsStore.getState().update({ updates: { channel: "nightly", automatic: false } });
    await useUpdatesStore.getState().check();
    const status = useUpdatesStore.getState().status;
    expect(status?.phase).toBe("available");
    expect(status?.channel).toBe("nightly");
    expect(status?.available?.version).toContain("nightly");
    await useUpdatesStore.getState().install();
    expect(useUpdatesStore.getState().status?.phase).toBe("ready");
    await useUpdatesStore.getState().relaunch();
    expect(useUpdatesStore.getState().status?.currentVersion).toContain("nightly");
    expect(useUpdatesStore.getState().status?.phase).toBe("idle");
  });
});

describe("derivePill with the updater", () => {
  const available = updateStatus({ phase: "available", available: { version: "0.2.0", channel: "latest" } });

  it("shows the update pill only when the HUD is otherwise idle", () => {
    expect(derivePill(appStatus(), null, false, "General", { update: available })).toEqual({
      kind: "update",
      phase: "available",
      label: "Update available · 0.2.0",
    });
    expect(derivePill(appStatus({ audioActive: true }), null, false, "General", { update: available }).kind).toBe("listening");
    expect(derivePill(appStatus(), "streaming", false, "General", { update: available }).kind).toBe("thinking");
    expect(derivePill(appStatus(), null, true, "General", { update: available }).kind).toBe("prepared");
    expect(derivePill(appStatus(), null, false, "General", { update: updateStatus({ phase: "up_to_date" }) })).toEqual({
      kind: "idle",
      modeName: "General",
    });
  });

  it("labels downloading with progress and ready as a relaunch", () => {
    expect(updatePillLabel(updateStatus({ phase: "downloading", progress: { downloaded: 21, total: 100 } }))).toBe(
      "Updating… 21%",
    );
    expect(updatePillLabel(updateStatus({ phase: "downloading" }))).toBe("Updating…");
    expect(updatePillLabel(updateStatus({ phase: "ready", available: { version: "0.2.0", channel: "latest" } }))).toBe(
      "Restart to update",
    );
    expect(updatePillLabel(updateStatus({ phase: "error" }))).toBeNull();
  });
});

describe("update copy", () => {
  const now = () => new Date("2026-09-12T16:30:00.000Z");

  it("describes every phase for the Settings row", () => {
    expect(describeUpdateStatus(null)).toBe("Checking the updater…");
    expect(describeUpdateStatus(updateStatus())).toContain("every 6 hours");
    expect(describeUpdateStatus(updateStatus({ phase: "up_to_date", lastCheckedAt: "2026-09-12T16:25:00.000Z" }), now)).toBe(
      "Up to date · checked 5 min ago.",
    );
    expect(
      describeUpdateStatus(updateStatus({ phase: "available", available: { version: "0.2.0", channel: "latest" } })),
    ).toBe("Version 0.2.0 is available — downloading shortly.");
    expect(
      describeUpdateStatus(updateStatus({ phase: "ready", available: { version: "0.2.0", channel: "latest" } })),
    ).toBe("0.2.0 is installed — restart Bluey to finish.");
    expect(describeUpdateStatus(updateStatus({ supported: false }))).toContain("Development builds");
  });

  it("picks the one button per phase", () => {
    expect(primaryUpdateAction(updateStatus())).toEqual({ label: "Check now", action: "check", disabled: false });
    expect(primaryUpdateAction(updateStatus({ phase: "checking" }))).toMatchObject({ action: null, disabled: true });
    expect(primaryUpdateAction(updateStatus({ phase: "available" }))).toMatchObject({ label: "Install", action: "install" });
    expect(primaryUpdateAction(updateStatus({ phase: "ready" }))).toMatchObject({ label: "Restart to update", action: "relaunch" });
    expect(primaryUpdateAction(updateStatus(), true)).toMatchObject({ disabled: true });
  });
});

describe("HUD update pill", () => {
  it("shows Update available, installs on click, then asks to restart", async () => {
    const mock = await setupMockApp();
    setEngine(new FakeEngine());
    await useSettingsStore.getState().update({ updates: { channel: "latest", automatic: false } });
    render(
      <TooltipProvider>
        <HudPanel />
      </TooltipProvider>,
    );
    await mock.invoke("updates_check", undefined);
    await waitFor(() => expect(screen.getByText("Update available · 0.2.0")).toBeInTheDocument());

    await userEvent.setup().click(screen.getByRole("button", { name: "Update available · 0.2.0" }));
    await waitFor(() => expect(screen.getByText("Restart to update")).toBeInTheDocument());
    expect(useUpdatesStore.getState().status?.phase).toBe("ready");
  });
});

describe("Settings → General → Updates", () => {
  it("renders the version row with its action and persists channel and automatic", async () => {
    await setupMockApp();
    const user = userEvent.setup();
    render(
      <TooltipProvider>
        <GeneralTab />
      </TooltipProvider>,
    );
    expect(await screen.findByText("Bluey 0.1.0-dev")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Check now" })).toBeInTheDocument();

    await user.selectOptions(screen.getByRole("combobox", { name: "Update channel" }), "nightly");
    await waitFor(() => expect(useSettingsStore.getState().settings?.updates.channel).toBe("nightly"));

    await user.click(screen.getByRole("switch", { name: "Automatic updates" }));
    await waitFor(() => expect(useSettingsStore.getState().settings?.updates.automatic).toBe(false));

    await user.click(screen.getByRole("button", { name: "Check now" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Install" })).toBeInTheDocument());
    expect(screen.getByText(/Version 0\.2\.0-nightly\.\d+ is available\./)).toBeInTheDocument();
  });
});
