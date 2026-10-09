import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { TooltipProvider } from "@/components/ui/Tooltip";
import { ModeEditor } from "@/features/settings/ModeEditor";
import { bluey } from "@/lib/tauri/api";
import type { BlueyMode } from "@/lib/types";
import { useModesStore } from "@/stores/modesStore";
import { setupMockApp } from "./helpers";

// A flat stand-in for the Radix menu: opening the real one crawls in jsdom.
vi.mock("@/components/ui/DropdownMenu", () => ({
  DropdownMenu: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  DropdownMenuTrigger: ({ children }: { children: ReactNode }) => <>{children}</>,
  DropdownMenuContent: ({ children }: { children: ReactNode }) => <div role="menu">{children}</div>,
  DropdownMenuSeparator: () => <hr />,
  DropdownMenuItem: ({ children, onSelect }: { children: ReactNode; onSelect?: () => void }) => (
    <button type="button" role="menuitem" onClick={() => onSelect?.()}>
      {children}
    </button>
  ),
}));

function renderEditor(mode: BlueyMode) {
  return render(
    <TooltipProvider>
      <ModeEditor mode={mode} isActive={false} onDeleted={() => {}} />
    </TooltipProvider>,
  );
}

function builtIn(id: string): BlueyMode {
  const mode = useModesStore.getState().modes.find((m) => m.id === id);
  if (!mode) throw new Error(`${id} mode missing`);
  return mode;
}

describe("ModeEditor validation and clearing", () => {
  beforeEach(async () => {
    await setupMockApp();
  });

  it("shows an inline error for over-long instructions and never sends them", async () => {
    const update = vi.spyOn(bluey.modes, "update");
    renderEditor(builtIn("sales"));
    const textarea = screen.getByLabelText("Meeting context");

    fireEvent.change(textarea, { target: { value: "a".repeat(4001) } });

    expect(await screen.findByRole("alert")).toHaveTextContent("4000 characters or fewer");
    expect(textarea).toHaveAttribute("aria-invalid", "true");
    await new Promise((resolve) => setTimeout(resolve, 800));
    expect(update).not.toHaveBeenCalled();

    fireEvent.change(textarea, { target: { value: "a".repeat(4000) } });
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    await waitFor(() => expect(update).toHaveBeenCalledTimes(1), { timeout: 2500 });
    update.mockRestore();
  });

  it("clears the sidebar group and the preferred model", async () => {
    const user = userEvent.setup();
    const created = await bluey.modes.create({
      draft: { name: "Board prep", group: "Work", preferredModelRole: "reasoning" },
    });
    renderEditor(created);

    await user.selectOptions(screen.getByLabelText("Preferred model"), "");
    await user.clear(screen.getByLabelText("Sidebar group"));

    await waitFor(
      async () => {
        const stored = await bluey.modes.get({ id: created.id });
        expect(stored.preferredModelRole).toBeUndefined();
        expect(stored.group).toBeUndefined();
      },
      { timeout: 2500 },
    );
  });

  it("Reset to default shows the shipped instructions again", async () => {
    const shipped = builtIn("sales").systemInstructions;
    renderEditor(builtIn("sales"));
    const textarea = screen.getByLabelText("Meeting context");
    fireEvent.change(textarea, { target: { value: "My own playbook." } });

    fireEvent.click(screen.getByRole("menuitem", { name: "Reset to default" }));

    await waitFor(() => expect(textarea).toHaveValue(shipped));
    // The pending edit was dropped rather than saved over the reset.
    await new Promise((resolve) => setTimeout(resolve, 800));
    expect((await bluey.modes.get({ id: "sales" })).systemInstructions).toBe(shipped);
  });

  it("tells the truth about blank instructions", () => {
    renderEditor(builtIn("sales"));
    expect(screen.getByLabelText("Meeting context")).toHaveAttribute(
      "placeholder",
      expect.stringContaining("Leave it blank to add no instructions") as string,
    );
  });
});
