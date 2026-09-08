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
