import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";

import { TooltipProvider } from "@/components/ui/Tooltip";
import { ResponseActions } from "@/features/hud/ResponseActions";
import { ResponseThread } from "@/features/hud/ResponseThread";
import { useAccountsStore } from "@/stores/accountsStore";
import { useChatStore } from "@/stores/chatStore";
import type { BlueyResponse, ProviderAccount } from "@/lib/types";
import { makeResponse, setupMockApp } from "./helpers";

const answered = (partial: Partial<BlueyResponse> = {}) =>
  makeResponse({
    selection: { role: "default", providerId: "gemini", model: "gemini-2.5-flash" },
    metrics: { provider: "gemini", model: "gemini-2.5-flash", totalMs: 1400 },
    ...partial,
  });

function renderActions(response: BlueyResponse) {
  render(
    <TooltipProvider>
      <ResponseActions response={response} onRegenerate={() => {}} />
    </TooltipProvider>,
  );
}

describe("answer provenance (UX-035)", () => {
  beforeEach(async () => {
    await setupMockApp();
  });

  it("names the provider, model and latency under the actions", () => {
    renderActions(answered());
    expect(screen.getByTestId("response-provenance")).toHaveTextContent("Google Gemini · gemini-2.5-flash · 1.40s");
    expect(screen.queryByText("via API key")).not.toBeInTheDocument();
  });

  it("says 'via API key' when a subscription account was skipped for an API-key provider", () => {
    useAccountsStore.setState({
      accounts: [{ accountId: "chatgpt", providerId: "chatgpt" } as ProviderAccount],
    });
    const fallbackReason = "role fast; provider chatgpt for role fast disabled or missing API key → fallback default";
    renderActions(answered({ selection: { role: "default", providerId: "gemini", model: "gemini-2.5-flash", fallbackReason } }));
    expect(screen.getByText("via API key")).toBeInTheDocument();
  });

  it("marks any other router fallback as a fallback model", () => {
    const fallbackReason = "role reasoning unassigned → fallback default";
    renderActions(answered({ selection: { role: "default", providerId: "gemini", model: "gemini-2.5-flash", fallbackReason } }));
    expect(screen.getByText("fallback model")).toBeInTheDocument();
  });

  it("shows nothing when the answer carries no provenance", () => {
    renderActions(makeResponse());
    expect(screen.queryByTestId("response-provenance")).not.toBeInTheDocument();
  });

  it("shows the research note under a finished answer that went without the web", () => {
    const researchNote = "Web research failed — answered without it.";
    const generation = useChatStore.getState().begin("Latest Bun release?", undefined, { phase: "thinking" });
    useChatStore.getState().complete(generation, answered({ researchNote }));
    render(
      <TooltipProvider>
        <ResponseThread onRetry={() => {}} onRegenerate={() => {}} />
      </TooltipProvider>,
    );
    expect(screen.getByText(researchNote)).toBeInTheDocument();
  });
});
