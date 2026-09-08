import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";

import { Toasts } from "@/components/ui/Toast";
import { showErrorToast, useToastStore } from "@/components/ui/toast-store";
import type { MockTransport } from "@/lib/tauri/mock";
import type { BlueyError } from "@/lib/types";
import { setupMockApp } from "./helpers";

const permissionError: BlueyError = {
  kind: "permission",
  code: "permission.screenRecording_denied",
  message: "screenRecording permission is denied.",
  recoverable: true,
  recovery: { type: "open_system_settings", pane: "screenRecording" },
};

describe("error toasts", () => {
  let mock: MockTransport;

  beforeEach(async () => {
    mock = await setupMockApp();
    useToastStore.setState({ toasts: [] });
  });

  it("surfaces app.error with its recovery button and dismisses after running it", async () => {
    const user = userEvent.setup();
    render(<Toasts />);

    mock.emit("app.error", permissionError);
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("Permission needed");
    expect(alert).toHaveTextContent("Bluey is missing a macOS permission it needs for this.");

    await user.click(screen.getByRole("button", { name: "Open System Settings" }));
    await waitFor(() => expect(screen.queryByRole("alert")).not.toBeInTheDocument());
  });

  it("collapses repeated errors of the same code into one toast", async () => {
    render(<Toasts />);
    mock.emit("audio.error", {
      kind: "audio",
      code: "audio.device_lost",
      message: "Device lost",
      recoverable: true,
    });
    mock.emit("audio.error", {
      kind: "audio",
      code: "audio.device_lost",
      message: "Device lost",
      recoverable: true,
    });
    await screen.findByRole("alert");
    expect(screen.getAllByRole("alert")).toHaveLength(1);
  });

  it("turns a stopped helper into an actionable toast and a restart into a confirmation", async () => {
    render(<Toasts />);
    mock.emit("helper.status", { running: false });
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("Helper not responding");
    expect(screen.getByRole("button", { name: "Restart helper" })).toBeInTheDocument();

    mock.emit("helper.status", { running: true, version: "0.1.0", restarted: true });
    await screen.findByText("Helper restarted");
  });

  it("stays silent for cancellations and can be dismissed manually", async () => {
    const user = userEvent.setup();
    render(<Toasts />);
    showErrorToast({ kind: "cancelled", code: "ai.cancelled", message: "cancelled", recoverable: false });
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();

    showErrorToast({ kind: "ai", code: "ai.provider_unavailable", message: "503", recoverable: true });
    await screen.findByRole("alert");
    await user.click(screen.getByRole("button", { name: "Dismiss" }));
    await waitFor(() => expect(screen.queryByRole("alert")).not.toBeInTheDocument());
  });
});
