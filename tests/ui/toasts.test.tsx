import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";

import { Toasts } from "@/components/ui/Toast";
import { showErrorToast, useToastStore } from "@/components/ui/toast-store";
import type { MockTransport } from "@/lib/tauri/mock";
import type { BlueyError } from "@/lib/types";
import { ErrorBanner } from "@/components/ui/ErrorBanner";
import { describeError } from "@/lib/errors/present";
import { setupInterceptedApp, setupMockApp } from "./helpers";

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

  it("announces an automatic helper restart as a notice, not a failure", async () => {
    render(<Toasts />);
    // The supervisor is already spawning a replacement (MAC-004 / UX-041).
    mock.emit("helper.status", { running: false, restarted: true });
    await screen.findByText("Restarting the native helper…");
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("shows the on-device fallback once as a notice, not an error (UX-023)", async () => {
    render(<Toasts />);
    const fallback = {
      kind: "audio" as const,
      code: "audio.stt_fallback",
      message: "no key configured for gemini",
      recoverable: false,
    };
    mock.emit("audio.error", fallback);
    await screen.findByText(/so Bluey is transcribing on-device/);
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();

    useToastStore.setState({ toasts: [] });
    mock.emit("audio.error", fallback);
    expect(useToastStore.getState().toasts).toHaveLength(0);
  });

  it("says when Apple Speech transcribes on Apple's servers, as a notice (MAC-007)", async () => {
    render(<Toasts />);
    mock.emit("audio.error", {
      kind: "audio",
      code: "audio.speech_server",
      message: "Apple Speech has no on-device model for de-DE, so it transcribes on Apple's servers",
      recoverable: false,
    });
    await screen.findByText(/sends audio to Apple to transcribe it/);
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
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

describe("recovery runner (UX-036)", () => {
  const expired: BlueyError = {
    kind: "authentication",
    code: "auth.account_expired",
    message: "The OpenAI sign-in expired.",
    recoverable: true,
    recovery: { type: "reconnect_account", accountId: "acct-openai", providerId: "openai" },
  };
  const connectFailed: BlueyError = {
    kind: "network",
    code: "network.offline",
    message: "The browser sign-in could not reach OpenAI.",
    recoverable: true,
  };

  beforeEach(async () => {
    const { transport } = await setupInterceptedApp();
    useToastStore.setState({ toasts: [] });
    transport.intercept("accounts_connect", async () => {
      throw connectFailed;
    });
  });

  it("shows a failing toast recovery instead of swallowing it", async () => {
    const user = userEvent.setup();
    render(<Toasts />);
    showErrorToast(expired);
    await user.click(await screen.findByRole("button", { name: "Reconnect" }));

    // The reconnect's own failure replaces the dismissed toast.
    const { title, message } = describeError(connectFailed);
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(title);
    expect(alert).toHaveTextContent(message);
  });

  it("shows a failing banner recovery as a toast", async () => {
    const user = userEvent.setup();
    render(
      <>
        <ErrorBanner error={expired} />
        <Toasts />
      </>,
    );
    await user.click(screen.getByRole("button", { name: "Reconnect" }));
    await waitFor(() => expect(useToastStore.getState().toasts).toHaveLength(1));
    expect(useToastStore.getState().toasts[0]?.variant).toBe("error");
  });
});
