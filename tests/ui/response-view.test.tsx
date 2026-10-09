import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { Toasts } from "@/components/ui/Toast";
import { TooltipProvider } from "@/components/ui/Tooltip";
import { ResponseActions } from "@/features/hud/ResponseActions";
import { ResponseView } from "@/features/hud/ResponseView";
import { splitStreamingMarkdown } from "@/features/hud/markdown";
import { makeResponse, setupInterceptedApp, setupMockApp } from "./helpers";

describe("ResponseView", () => {
  beforeEach(async () => {
    await setupMockApp();
  });

  it("renders markdown: bold, lists, inline code", () => {
    render(
      <ResponseView
        response={makeResponse({
          content: "Use a **hash map**.\n\n- one pass\n- `O(n)` time",
        })}
      />,
    );
    expect(screen.getByText("hash map")).toBeInTheDocument();
    expect(screen.getByText("one pass")).toBeInTheDocument();
    expect(screen.getByText("O(n)")).toBeInTheDocument();
  });

  it("renders a fenced code block with language header and copy", async () => {
    // Fake timers: the "Copied" label lasts 1 s, which a slow run could miss.
    userEvent.setup(); // installs the clipboard stub
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    try {
      const clipboardSpy = vi.spyOn(navigator.clipboard, "writeText").mockResolvedValue(undefined);
      render(
        <ResponseView response={makeResponse({ content: "Before\n\n```python\nprint('hi')\n```\n\nAfter" })} />,
      );
      expect(screen.getByText("python")).toBeInTheDocument();
      expect(screen.getByText("print('hi')")).toBeInTheDocument();

      fireEvent.click(screen.getByLabelText("Copy code"));
      await act(async () => {});
      expect(screen.getByText("Copied")).toBeInTheDocument();
      expect(clipboardSpy).toHaveBeenCalledWith("print('hi')");

      act(() => vi.advanceTimersByTime(1000));
      expect(screen.getByText("Copy")).toBeInTheDocument();
      clipboardSpy.mockRestore();
    } finally {
      vi.useRealTimers();
    }
  });

  it("buffers an unclosed code fence while streaming", () => {
    const { renderable, pendingCode } = splitStreamingMarkdown("Intro\n\n```python\nprint('par");
    expect(pendingCode).toBe(true);
    expect(renderable).not.toContain("```");

    render(<ResponseView streaming response={makeResponse({ content: "Intro\n\n```python\nprint('par" })} />);
    expect(screen.getByText("Writing code…")).toBeInTheDocument();
    expect(screen.queryByText(/print\(/)).not.toBeInTheDocument();
  });

  it("renders citations as sources", () => {
    render(
      <ResponseView
        response={makeResponse({
          citations: [{ id: "c1", title: "MDN — Array.prototype.map", url: "https://mdn.example" }],
        })}
      />,
    );
    expect(screen.getByText("Sources")).toBeInTheDocument();
    expect(screen.getByText("MDN — Array.prototype.map")).toBeInTheDocument();
  });
});

describe("ResponseActions", () => {
  beforeEach(async () => {
    await setupMockApp();
  });

  it("copies the answer and shows feedback chips on thumbs-down", async () => {
    const user = userEvent.setup();
    const clipboardSpy = vi.spyOn(navigator.clipboard, "writeText").mockResolvedValue(undefined);
    render(
      <TooltipProvider>
        <ResponseActions response={makeResponse({ content: "The answer" })} onRegenerate={() => {}} />
      </TooltipProvider>,
    );

    await user.click(screen.getByLabelText("Copy answer"));
    expect(clipboardSpy).toHaveBeenCalledWith("The answer");

    await user.click(screen.getByLabelText("Not helpful"));
    await waitFor(() => expect(screen.getByText("Why wasn't this useful?")).toBeInTheDocument());
    expect(screen.getByText("Missed context")).toBeInTheDocument();
    await user.click(screen.getByText("Too long"));
    await waitFor(() => expect(screen.queryByText("Why wasn't this useful?")).not.toBeInTheDocument());
    clipboardSpy.mockRestore();
  });

  it("regenerate calls the callback", async () => {
    const user = userEvent.setup();
    const onRegenerate = vi.fn();
    render(
      <TooltipProvider>
        <ResponseActions response={makeResponse()} onRegenerate={onRegenerate} />
      </TooltipProvider>,
    );
    await user.click(screen.getByLabelText("Regenerate"));
    expect(onRegenerate).toHaveBeenCalledOnce();
  });
});

describe("ResponseView never renders JSON", () => {
  beforeEach(async () => {
    await setupMockApp();
  });

  it("shows the unreadable-answer state instead of a JSON envelope", () => {
    render(
      <ResponseView
        response={makeResponse({ title: "Pick", content: '{"responseType":"answer","title":"Pick","content":"B"}' })}
      />,
    );
    expect(screen.queryByText(/responseType/)).not.toBeInTheDocument();
    expect(screen.queryByRole("heading", { level: 2 })).not.toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent("Couldn't read the answer");
  });

  it("renders answer sections as headed blocks and keeps code sections collapsible", () => {
    render(
      <ResponseView
        response={makeResponse({
          type: "answer",
          content: "B.",
          sections: [
            { id: "s1", title: "Why it wins", content: "The index covers the predicate." },
            { id: "s2", title: "Solution", kind: "code", language: "sql", content: "CREATE INDEX idx ON users(email);" },
          ],
        })}
      />,
    );
    expect(screen.getByRole("heading", { level: 3, name: "Why it wins" })).toBeInTheDocument();
    expect(screen.getByText("The index covers the predicate.")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Why it wins/ })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Solution/ })).toHaveAttribute("aria-expanded", "true");
  });

  it("keeps code responses' sections collapsible", () => {
    render(
      <ResponseView
        response={makeResponse({
          type: "code",
          content: "Use a hash map.",
          sections: [{ id: "s1", title: "Complexity", content: "O(n) time, O(n) space." }],
        })}
      />,
    );
    expect(screen.getByRole("button", { name: /Complexity/ })).toBeInTheDocument();
  });
});

describe("ResponseActions copy and feedback (UX-028)", () => {
  it("copies the title and sections as plain text", async () => {
    await setupMockApp();
    const user = userEvent.setup();
    const clipboardSpy = vi.spyOn(navigator.clipboard, "writeText").mockResolvedValue(undefined);
    const response = makeResponse({
      title: "Behavioral",
      content: "Lead with **ownership**.",
      sections: [{ id: "s1", title: "Story used", content: "The **Acme** migration" }],
    });
    render(
      <TooltipProvider>
        <ResponseActions response={response} onRegenerate={() => {}} />
      </TooltipProvider>,
    );

    await user.click(screen.getByLabelText("Copy answer"));
    expect(clipboardSpy).toHaveBeenCalledWith("Behavioral\n\nLead with ownership.\n\nStory used\nThe Acme migration");
    clipboardSpy.mockRestore();
  });

  it("rolls the rating back and says so when the feedback cannot be saved", async () => {
    const { transport } = await setupInterceptedApp();
    transport.intercept("responses_feedback", () => {
      throw { kind: "storage", code: "storage.write", message: "The feedback could not be saved.", recoverable: true };
    });
    const user = userEvent.setup();
    render(
      <TooltipProvider>
        <ResponseActions response={makeResponse({ content: "The answer" })} onRegenerate={() => {}} />
        <Toasts />
      </TooltipProvider>,
    );

    await user.click(screen.getByLabelText("Helpful"));

    expect(await screen.findByRole("alert")).toBeInTheDocument();
    await waitFor(() => expect(screen.getByLabelText("Helpful")).toHaveAttribute("aria-pressed", "false"));
  });
});
