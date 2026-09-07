import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";

import { bootstrap, resolveWindowLabel } from "@/lib/tauri/bootstrap";
import { eventBus } from "@/lib/tauri/event-bus";
import { getTransport } from "@/lib/tauri/transport";
import { resetStoresForTest } from "@/stores/initStores";
import { useSettingsStore } from "@/stores/settingsStore";
import HudWindow from "@/windows/HudWindow";
import OnboardingWindow from "@/windows/OnboardingWindow";
import SettingsWindow from "@/windows/SettingsWindow";
import { setupMockApp } from "./helpers";

describe("bootstrap", () => {
  beforeEach(async () => {
    await eventBus.dispose();
    resetStoresForTest();
  });

  it("resolves the window label from the query string", () => {
    expect(resolveWindowLabel("?window=settings")).toBe("settings");
    expect(resolveWindowLabel("?window=onboarding")).toBe("onboarding");
    expect(resolveWindowLabel("?window=nope")).toBe("main");
    expect(resolveWindowLabel("")).toBe("main");
  });

  it("selects the MockTransport outside Tauri, loads settings and applies theme attributes", async () => {
    const { windowLabel, transport } = await bootstrap();
    expect(windowLabel).toBe("main");
    expect(transport.kind).toBe("mock");
    expect(getTransport()).toBe(transport);
    expect(useSettingsStore.getState().settings?.general.blueyName).toBe("Bluey");

    const root = document.documentElement;
    expect(root.dataset.window).toBe("main");
    expect(root.dataset.mock).toBe("true");
    expect(root.dataset.theme).toBe("light"); // jsdom matchMedia stub: prefers-color-scheme dark = false
    expect(root.dataset.fontSize).toBe("medium");
    expect(root.dataset.reducedMotion).toBe("false");
  });
});

describe("window roots (MockTransport, dev auth)", () => {
  beforeEach(async () => {
    await setupMockApp();
  });

  it("HudWindow renders the HUD panel", async () => {
    render(<HudWindow />);
    await waitFor(() => expect(screen.getByPlaceholderText("Ask anything about your screen")).toBeInTheDocument());
  });

  it("SettingsWindow renders the shell with tabs and the General page", async () => {
    render(<SettingsWindow />);
    expect(screen.getByText("Bluey Settings")).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Modes" })).toBeInTheDocument();
    await waitFor(() => expect(screen.getByText("Launch Bluey at login")).toBeInTheDocument());
  });

  it("OnboardingWindow renders the welcome step", async () => {
    render(<OnboardingWindow />);
    await waitFor(() => expect(screen.getByText("Welcome to Bluey")).toBeInTheDocument());
  });
});
