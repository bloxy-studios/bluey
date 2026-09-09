import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";

import { useToastStore } from "@/components/ui/toast-store";
import { TooltipProvider } from "@/components/ui/Tooltip";
import { IMPORT_DISABLED_HINT } from "@/features/settings/session-import";
import { SessionDetail } from "@/features/settings/SessionDetail";
import SessionsTab from "@/features/settings/tabs/SessionsTab";
import { bluey } from "@/lib/tauri/api";
import type { MockTransport } from "@/lib/tauri/mock";
import { useSettingsStore } from "@/stores/settingsStore";
import { setupInterceptedApp, setupMockApp } from "./helpers";

function withTooltips(node: React.ReactElement) {
  return render(<TooltipProvider>{node}</TooltipProvider>);
}

/** Point the transcription role at Foundry, whose adapter cannot transcribe files. */
async function moveTranscriptionToFoundry() {
  const current = useSettingsStore.getState().settings;
  if (!current) throw new Error("settings not loaded");
  await useSettingsStore.getState().update({
    ai: {
      models: {
        ...current.ai.models,
        transcription: { providerId: "azure-foundry", model: "MAI-Transcribe-1.5" },
      },
    },
  });
}

describe("SessionsTab", () => {
  beforeEach(async () => {
    await setupMockApp();
  });

  it("lists sessions and deletes one after confirmation", async () => {
    const user = userEvent.setup();
    withTooltips(<SessionsTab />);

    await screen.findByText("Coding interview practice");
    expect(screen.getByText("Weekly team sync")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Delete session Weekly team sync" }));
    await user.click(screen.getByRole("button", { name: "Delete session" }));

    await waitFor(() => expect(screen.queryByText("Weekly team sync")).not.toBeInTheDocument());
    expect(screen.getByText("Coding interview practice")).toBeInTheDocument();
    expect(await bluey.session.search({ query: {} })).toHaveLength(1);
  });

  it("marks the live session and refuses to delete it", async () => {
    await bluey.session.start({ title: "Live one" });
    withTooltips(<SessionsTab />);
    await screen.findByText("Live one");
    expect(screen.getByText("Live")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Delete session Live one" })).toBeDisabled();
  });
});

describe("SessionDetail", () => {
  let mock: MockTransport;

  beforeEach(async () => {
    mock = await setupMockApp();
  });

  it("renames the session inline", async () => {
    const user = userEvent.setup();
    withTooltips(<SessionDetail sessionId="session-coding-1" onBack={() => {}} />);

    await screen.findByRole("heading", { name: "Coding interview practice" });
    await user.click(screen.getByRole("button", { name: "Rename session" }));
    const input = screen.getByLabelText("Session title");
    await user.clear(input);
    await user.type(input, "Mock interview — round 2{Enter}");

    await screen.findByRole("heading", { name: "Mock interview — round 2" });
    const detail = await bluey.session.get({ id: "session-coding-1" });
    expect(detail.session.title).toBe("Mock interview — round 2");
  });

  it("Escape cancels a rename without the blur committing it; leaving the field still commits", async () => {
    const user = userEvent.setup();
    withTooltips(<SessionDetail sessionId="session-coding-1" onBack={() => {}} />);
    await screen.findByRole("heading", { name: "Coding interview practice" });

    await user.click(screen.getByRole("button", { name: "Rename session" }));
    await user.clear(screen.getByLabelText("Session title"));
    await user.type(screen.getByLabelText("Session title"), "Abandoned title{Escape}");
    expect(await screen.findByRole("heading", { name: "Coding interview practice" })).toBeInTheDocument();
    expect((await bluey.session.get({ id: "session-coding-1" })).session.title).toBe("Coding interview practice");

    await user.click(screen.getByRole("button", { name: "Rename session" }));
    await user.clear(screen.getByLabelText("Session title"));
    await user.type(screen.getByLabelText("Session title"), "Committed on blur");
    await user.tab();
    await screen.findByRole("heading", { name: "Committed on blur" });
    expect((await bluey.session.get({ id: "session-coding-1" })).session.title).toBe("Committed on blur");
  });

  it("shows an error banner instead of spinning forever when the session is gone", async () => {
    withTooltips(<SessionDetail sessionId="session-missing" onBack={() => {}} />);
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("Storage problem");
  });

  it("shows the most recent 300 segments and says how many exist", async () => {
    const session = await bluey.session.start({ title: "Long one" });
    await mock.invoke("dev_simulate", {
      simulation: {
        type: "transcript",
        segments: Array.from({ length: 301 }, (_, i) => ({
          text: `Line ${i + 1}`,
          speaker: "spk_1",
          source: "system" as const,
        })),
      },
    });
    await bluey.session.end();

    withTooltips(<SessionDetail sessionId={session.id} onBack={() => {}} />);
    const list = await screen.findByRole("list", { name: "Transcript" });
    expect(list.children).toHaveLength(300);
    expect(screen.getByText("Showing 300 of 301 segments.")).toBeInTheDocument();
    expect(screen.queryByText("Line 1")).not.toBeInTheDocument(); // the oldest one dropped …
    expect(screen.getByText("Line 301")).toBeInTheDocument(); // … the newest kept
  });

  it("keeps the session readable when the transcript lookup fails", async () => {
    const { transport } = await setupInterceptedApp();
    const imported = await bluey.ai.transcribeFile({
      path: "/tmp/standup.wav",
      diarization: true,
      wordTimestamps: true,
    });
    transport.intercept("transcript_list", async () => {
      throw { kind: "storage", code: "storage.locked", message: "database is locked", recoverable: false };
    });

    withTooltips(<SessionDetail sessionId={imported.session.id} onBack={() => {}} />);
    await screen.findByRole("heading", { name: "Imported · standup.wav" });
    expect(await screen.findByText("Transcript unavailable")).toBeInTheDocument();
    expect(screen.queryByRole("list", { name: "Transcript" })).not.toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });
});

describe("Importing recordings", () => {
  let mock: MockTransport;

  beforeEach(async () => {
    mock = await setupMockApp();
    useToastStore.setState({ toasts: [] });
  });

  it("transcribes a picked file into a new session and opens it", async () => {
    const user = userEvent.setup();
    withTooltips(<SessionsTab />);
    await screen.findByText("Coding interview practice");

    await user.click(screen.getByRole("button", { name: "Import recording…" }));

    await screen.findByRole("heading", { name: "Imported · standup.wav" });
    expect(await screen.findByText("Recording imported")).toBeInTheDocument();
    const transcript = await screen.findByRole("list", { name: "Transcript" });
    expect(transcript).toHaveTextContent("Speaker 1");
    expect(transcript).toHaveTextContent("Thanks for joining, let's get started.");
    expect(transcript).toHaveTextContent("Speaker 2");
    expect(useToastStore.getState().toasts.map((t) => t.message)).toEqual([
      "Imported 4 segments from 2 speakers into “Imported · standup.wav”",
    ]);

    const sessions = await bluey.session.search({ query: {} });
    const imported = sessions.find((item) => item.session.title === "Imported · standup.wav");
    expect(imported?.session.status).toBe("completed");
    expect(await bluey.transcript.list({ sessionId: imported!.session.id })).toHaveLength(4);
  });

  it("adds a recording to an existing session from its detail view", async () => {
    const user = userEvent.setup();
    withTooltips(<SessionDetail sessionId="session-coding-1" onBack={() => {}} />);
    await screen.findByRole("heading", { name: "Coding interview practice" });
    expect(screen.queryByRole("list", { name: "Transcript" })).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Add recording" }));

    expect(await screen.findByText("Recording imported")).toBeInTheDocument();
    expect(await screen.findByRole("list", { name: "Transcript" })).toHaveTextContent(
      "What trade-offs did you consider?",
    );
    const events = await bluey.session.listEvents({ sessionId: "session-coding-1" });
    expect(events.some((event) => event.type === "recording_imported")).toBe(true);
  });

  it("does nothing when the picker is cancelled", async () => {
    const user = userEvent.setup();
    mock.nextPickedRecording = null;
    withTooltips(<SessionsTab />);
    await screen.findByText("Coding interview practice");

    await user.click(screen.getByRole("button", { name: "Import recording…" }));

    await waitFor(() => expect(screen.getByRole("button", { name: "Import recording…" })).toBeEnabled());
    expect(screen.queryByRole("heading", { name: /Imported/ })).not.toBeInTheDocument();
    expect(screen.getByText("Coding interview practice")).toBeInTheDocument();
    expect(await bluey.session.search({ query: {} })).toHaveLength(2);
    expect(useToastStore.getState().toasts).toHaveLength(0);
  });

  it("disables importing until the transcription role runs on Gemini", async () => {
    await moveTranscriptionToFoundry();
    withTooltips(<SessionsTab />);
    const button = await screen.findByRole("button", { name: "Import recording…" });
    expect(button).toBeDisabled();
    expect(button).toHaveAttribute("title", IMPORT_DISABLED_HINT);
  });

  it("disables 'Add recording' in the detail view for the same reason", async () => {
    await moveTranscriptionToFoundry();
    withTooltips(<SessionDetail sessionId="session-coding-1" onBack={() => {}} />);
    await screen.findByRole("heading", { name: "Coding interview practice" });
    const button = screen.getByRole("button", { name: "Add recording" });
    expect(button).toBeDisabled();
    expect(button).toHaveAttribute("title", IMPORT_DISABLED_HINT);
  });
});
