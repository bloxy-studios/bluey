import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";

import { TooltipProvider } from "@/components/ui/Tooltip";
import { SessionDetail } from "@/features/settings/SessionDetail";
import SessionsTab from "@/features/settings/tabs/SessionsTab";
import { bluey } from "@/lib/tauri/api";
import { setupMockApp } from "./helpers";

function withTooltips(node: React.ReactElement) {
  return render(<TooltipProvider>{node}</TooltipProvider>);
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
  beforeEach(async () => {
    await setupMockApp();
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

  it("shows an error banner instead of spinning forever when the session is gone", async () => {
    withTooltips(<SessionDetail sessionId="session-missing" onBack={() => {}} />);
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("Couldn't save data");
  });
});

describe("Importing recordings", () => {
  beforeEach(async () => {
    await setupMockApp();
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
    expect(await screen.findByRole("list", { name: "Transcript" })).toHaveTextContent("What trade-offs did you consider?");
    const events = await bluey.session.listEvents({ sessionId: "session-coding-1" });
    expect(events.some((event) => event.type === "recording_imported")).toBe(true);
  });
});
