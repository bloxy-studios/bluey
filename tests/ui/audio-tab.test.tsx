import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";

import AudioTab from "@/features/settings/tabs/AudioTab";
import type { SettingsPatch } from "@/lib/types";
import { useSettingsStore } from "@/stores/settingsStore";
import { setupInterceptedApp } from "./helpers";

describe("AudioTab microphone (UX-033)", () => {
  it("offers System default, which unpins the device by sending null", async () => {
    const { transport } = await setupInterceptedApp();
    const patches: SettingsPatch[] = [];
    transport.intercept("settings_update", (args, next) => {
      patches.push(args.patch);
      return next();
    });
    const user = userEvent.setup();
    render(<AudioTab />);

    const picker = await screen.findByLabelText<HTMLSelectElement>("Microphone device");
    const systemDefault = await within(picker).findByRole("option", {
      name: "System default — MacBook Pro Microphone",
    });
    // The fixture pins the built-in microphone.
    expect(picker.value).toBe("mic-builtin");

    await user.selectOptions(picker, systemDefault);

    await waitFor(() => expect(picker.value).toBe(""));
    expect(patches).toEqual([{ audio: { microphoneDeviceId: null } }]);
    expect(useSettingsStore.getState().settings?.audio).not.toHaveProperty("microphoneDeviceId");
  });

  it("still pins a concrete device", async () => {
    await setupInterceptedApp();
    const user = userEvent.setup();
    render(<AudioTab />);

    const picker = await screen.findByLabelText<HTMLSelectElement>("Microphone device");
    await user.selectOptions(picker, await within(picker).findByRole("option", { name: "AirPods Pro" }));

    await waitFor(() => expect(useSettingsStore.getState().settings?.audio.microphoneDeviceId).toBe("mic-airpods"));
  });
});
