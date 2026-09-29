import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";

import PrivacyTab from "@/features/settings/tabs/PrivacyTab";
import { useSettingsStore } from "@/stores/settingsStore";
import { setupMockApp } from "./helpers";

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

  it("toggles Cloud AI", async () => {
    const user = userEvent.setup();
    render(<PrivacyTab />);
    await user.click(screen.getByLabelText("Cloud AI"));
    await waitFor(() => expect(useSettingsStore.getState().settings?.privacy.cloudAiEnabled).toBe(false));
  });
});
