import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";

import { clampRetentionMinutes } from "@/features/settings/privacy-retention";
import PrivacyTab from "@/features/settings/tabs/PrivacyTab";
import { useSettingsStore } from "@/stores/settingsStore";
import { setupMockApp } from "./helpers";

describe("PrivacyTab raw audio retention", () => {
  beforeEach(async () => {
    await setupMockApp();
  });

  it("reveals the retention window for a custom raw-audio policy and persists the minutes", async () => {
    const user = userEvent.setup();
    render(<PrivacyTab />);

    expect(screen.queryByLabelText("Raw audio retention minutes")).not.toBeInTheDocument();
    await user.selectOptions(screen.getByLabelText("Raw audio retention"), "custom");

    const input = await screen.findByLabelText("Raw audio retention minutes");
    await waitFor(() => {
      const privacy = useSettingsStore.getState().settings?.privacy;
      expect(privacy?.storeRawAudio).toBe("custom");
      expect(privacy?.rawAudioRetentionMinutes).toBe(30); // sensible default the moment "custom" is chosen
    });

    await user.clear(input);
    await user.type(input, "45");
    await waitFor(
      () => expect(useSettingsStore.getState().settings?.privacy.rawAudioRetentionMinutes).toBe(45),
      {
        timeout: 2000,
      },
    );
    expect(screen.getByText(/discarded after 45 minutes/)).toBeInTheDocument();
  });

  it("clamps the window to the supported range", () => {
    expect(clampRetentionMinutes(0)).toBe(1);
    expect(clampRetentionMinutes(999)).toBe(240);
    expect(clampRetentionMinutes(12.6)).toBe(13);
    expect(clampRetentionMinutes(Number.NaN)).toBe(30);
  });

  it("toggles Cloud AI", async () => {
    const user = userEvent.setup();
    render(<PrivacyTab />);
    await user.click(screen.getByLabelText("Cloud AI"));
    await waitFor(() => expect(useSettingsStore.getState().settings?.privacy.cloudAiEnabled).toBe(false));
  });
});
