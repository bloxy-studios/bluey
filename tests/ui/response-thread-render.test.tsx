import { act, render, screen } from "@testing-library/react";
import type { ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { TooltipProvider } from "@/components/ui/Tooltip";
import { ResponseThread } from "@/features/hud/ResponseThread";
import { useChatStore } from "@/stores/chatStore";
import { makeResponse, setupMockApp } from "./helpers";

/** How often each markdown text was parsed (the expensive part of a render). */
const parses = new Map<string, number>();

vi.mock("react-markdown", () => ({
  default: ({ children }: { children?: ReactNode }) => {
    const text = String(children ?? "");
    parses.set(text, (parses.get(text) ?? 0) + 1);
    return <div>{text}</div>;
  },
}));

function seedFinishedTurns(count: number): void {
  for (let i = 0; i < count; i += 1) {
    const generation = useChatStore.getState().begin(`question ${i}`);
    useChatStore.getState().complete(generation, makeResponse({ id: `r${i}`, content: `answer ${i}` }));
  }
}

describe("ResponseThread rendering (PERF-003)", () => {
  beforeEach(async () => {
    await setupMockApp();
    parses.clear();
  });

  it("does not re-render finished turns while the last turn streams", () => {
    seedFinishedTurns(10);
    render(
      <TooltipProvider>
        <ResponseThread onRetry={() => undefined} onRegenerate={() => undefined} />
      </TooltipProvider>,
    );
    const generation = useChatStore.getState().begin("streaming question");

    let text = "";
    for (let i = 0; i < 50; i += 1) {
      text += `token${i} `;
      act(() => {
        useChatStore.getState().applyDraft(generation, makeResponse({ id: "live", content: text }));
        useChatStore.getState().flushDraft();
      });
    }

    expect(screen.getByText(text.trim())).toBeInTheDocument();
    for (let i = 0; i < 10; i += 1) expect(parses.get(`answer ${i}`)).toBe(1);
  });
});
