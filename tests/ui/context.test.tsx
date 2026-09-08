import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";

import { Toasts } from "@/components/ui/Toast";
import { TooltipProvider } from "@/components/ui/Tooltip";
import ContextTab from "@/features/settings/tabs/ContextTab";
import { bluey } from "@/lib/tauri/api";
import { setupMockApp } from "./helpers";

function renderTab() {
  return render(
    <TooltipProvider>
      <ContextTab />
      <Toasts />
    </TooltipProvider>,
  );
}

describe("ContextTab (My Context)", () => {
  beforeEach(async () => {
    await setupMockApp();
  });

  it("lists only global documents", async () => {
    renderTab();
    await screen.findByText("Resume — Jordan Lee.pdf");
    // Kind badge on the row (the picker also offers "Résumé" as an option).
    expect(screen.getAllByText("Résumé")).toHaveLength(2);
    expect(screen.getByRole("option", { name: "Résumé" })).toBeInTheDocument();
    // Mode-scoped documents belong to Modes → Files.
    expect(screen.queryByText("Staff Engineer — Job description.md")).not.toBeInTheDocument();
  });

  it("adds picked files with the selected kind at global scope", async () => {
    const user = userEvent.setup();
    renderTab();
    await screen.findByText("Resume — Jordan Lee.pdf");

    await user.selectOptions(screen.getByLabelText("Document kind"), "job_description");
    await user.click(screen.getByRole("button", { name: "Add files" }));

    await screen.findByText("Portfolio.pdf");
    expect(screen.getByText("Project notes.md")).toBeInTheDocument();

    const docs = await bluey.documents.list({ scope: "global" });
    const added = docs.filter((d) => d.title === "Portfolio.pdf" || d.title === "Project notes.md");
    expect(added).toHaveLength(2);
    expect(added.every((d) => d.kind === "job_description" && d.scope === "global")).toBe(true);
    // Two row badges + the picker option.
    expect(screen.getAllByText("Job description")).toHaveLength(3);
  });

  it("removes a document after confirmation", async () => {
    const user = userEvent.setup();
    renderTab();
    await screen.findByText("Resume — Jordan Lee.pdf");

    await user.click(screen.getByRole("button", { name: "Remove Resume — Jordan Lee.pdf" }));
    await user.click(screen.getByRole("button", { name: "Remove" }));

    await waitFor(() => expect(screen.queryByText("Resume — Jordan Lee.pdf")).not.toBeInTheDocument());
    expect(await screen.findByText("No context documents yet")).toBeInTheDocument();
    expect(await bluey.documents.list({ scope: "global" })).toHaveLength(0);
  });

  it("reindexes everything from the header button", async () => {
    const user = userEvent.setup();
    renderTab();
    await screen.findByText("Resume — Jordan Lee.pdf");
    await user.click(screen.getByRole("button", { name: /Reindex all/ }));
    await screen.findByText(/Reindexed \d+ documents?/);
  });
});
