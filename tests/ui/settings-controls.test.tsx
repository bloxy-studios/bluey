import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Settings } from "lucide-react";
import { createRef, StrictMode, useState } from "react";
import { describe, expect, it, vi } from "vitest";

import { Button } from "@/components/ui/Button";
import { Dialog } from "@/components/ui/Dialog";
import { Input } from "@/components/ui/Input";
import { Select } from "@/components/ui/Select";
import { SettingRow } from "@/components/ui/SettingRow";
import { Switch } from "@/components/ui/Switch";
import { Textarea } from "@/components/ui/Textarea";

const longLabel = "A very long external microphone or custom mode name that should not widen the settings page";

function DialogExample({ description, autoFocus = false }: { description?: string; autoFocus?: boolean }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <Button onClick={() => setOpen(true)}>Edit settings</Button>
      <Dialog
        open={open}
        onOpenChange={setOpen}
        title="Edit settings"
        description={description}
        footer={<Button onClick={() => setOpen(false)}>Save</Button>}
      >
        <label>Display name<Input defaultValue="Bluey" autoFocus={autoFocus} /></label>
        <Textarea aria-label="Notes" defaultValue={"Long notes\n".repeat(100)} />
      </Dialog>
    </>
  );
}

describe("Settings controls", () => {
  it("keeps a native select's ref, change event, disabled options and full selected label", async () => {
    const user = userEvent.setup();
    const ref = createRef<HTMLSelectElement>();
    const onChange = vi.fn();
    const options = [
      { value: "long", label: longLabel },
      { value: "short", label: "System default" },
      { value: "unavailable", label: "Unavailable", disabled: true },
    ];
    const view = render(<Select ref={ref} aria-label="Microphone" value="long" onChange={onChange} options={options} />);
    const select = screen.getByRole("combobox", { name: "Microphone" });
    expect(ref.current).toBe(select);
    expect(select).toHaveAttribute("title", longLabel);
    expect(select).toHaveClass("min-w-0", "w-full", "truncate");
    expect(select.parentElement).toHaveClass("min-w-0", "max-w-full");
    expect(screen.getByRole("option", { name: "Unavailable" })).toBeDisabled();
    await user.selectOptions(select, "short");
    expect(onChange).toHaveBeenCalledOnce();
    view.rerender(<Select aria-label="Microphone" value="short" onChange={onChange} options={options} disabled title="Microphone unavailable" />);
    expect(select).toBeDisabled();
    expect(select).toHaveAttribute("title", "Microphone unavailable");
  });

  it("updates a controlled select's value and full label, ignoring unavailable choices", async () => {
    const user = userEvent.setup();
    function ControlledSelect() {
      const [value, setValue] = useState("long");
      return (
        <Select
          aria-label="Default mode"
          value={value}
          onChange={(event) => setValue(event.currentTarget.value)}
          options={[
            { value: "long", label: longLabel },
            { value: "short", label: "System default" },
            { value: "unavailable", label: "Unavailable", disabled: true },
          ]}
        />
      );
    }
    render(<ControlledSelect />);
    const select = screen.getByRole("combobox", { name: "Default mode" });
    await user.tab();
    expect(select).toHaveFocus();
    await user.selectOptions(select, "short");
    expect(select).toHaveValue("short");
    expect(select).toHaveAttribute("title", "System default");
    await user.selectOptions(select, "unavailable");
    expect(select).toHaveValue("short");
    await user.selectOptions(select, "long");
    expect(select).toHaveValue("long");
    expect(select).toHaveAttribute("title", longLabel);
  });

  it("retains complete row copy while constraining the control column", () => {
    render(
      <SettingRow icon={Settings} title="Microphone" description={longLabel}>
        <Select aria-label="Microphone" value="long" onChange={() => {}} options={[{ value: "long", label: longLabel }]} />
      </SettingRow>,
    );
    const description = screen.getByText(longLabel, { selector: "div" });
    expect(description).toHaveClass("[overflow-wrap:anywhere]");
    expect(screen.getByRole("combobox").parentElement?.parentElement).toHaveClass("min-w-0", "max-w-[45%]", "flex-wrap");
  });

  it("keeps text fields keyboard-editable, theme-aware and visibly focusable", async () => {
    const user = userEvent.setup();
    render(<><Input aria-label="Name" /><Textarea aria-label="Notes" /></>);
    const input = screen.getByRole("textbox", { name: "Name" });
    const textarea = screen.getByRole("textbox", { name: "Notes" });
    await user.tab();
    expect(input).toHaveFocus();
    await user.keyboard("Bluey");
    expect(input).toHaveValue("Bluey");
    await user.tab();
    expect(textarea).toHaveFocus();
    await user.keyboard("Meeting notes");
    expect(textarea).toHaveValue("Meeting notes");
    for (const field of [input, textarea]) {
      expect(field).toHaveClass("min-w-0", "max-w-full", "focus-visible:ring-2", "scheme-dark", "[:root[data-theme=light]_&]:scheme-light");
    }
    expect(textarea).toHaveClass("overscroll-contain");
  });

  it("uses the themed switch track and supports Space without pointer input", async () => {
    const user = userEvent.setup();
    const onCheckedChange = vi.fn();
    render(<Switch aria-label="Launch at login" checked={false} onCheckedChange={onCheckedChange} />);
    const toggle = screen.getByRole("switch", { name: "Launch at login" });
    await user.tab();
    expect(toggle).toHaveFocus();
    await user.keyboard(" ");
    expect(onCheckedChange).toHaveBeenCalledWith(true);
    expect(toggle).toHaveClass("bg-border-strong", "focus-visible:ring-2");
    expect(toggle.className).not.toContain("#2a2a2a");
  });
});

describe("Settings dialogs", () => {
  it("bounds the panel, scrolls only its body, and keeps its header and footer outside it", async () => {
    const user = userEvent.setup();
    render(<DialogExample description="Provider configuration" />);
    await user.click(screen.getByRole("button", { name: "Edit settings" }));
    const dialog = screen.getByRole("dialog", { name: "Edit settings" });
    expect(dialog).toHaveClass("max-h-[calc(100dvh-48px)]", "max-w-[calc(100vw-48px)]", "overflow-hidden");
    const description = within(dialog).getByText("Provider configuration");
    expect(dialog).toHaveAttribute("aria-describedby", description.id);
    const scrollBody = description.parentElement!;
    expect(scrollBody).toHaveClass("min-h-0", "overflow-y-auto", "overscroll-contain");
    expect(scrollBody).toContainElement(screen.getByRole("textbox", { name: "Notes" }));
    expect(scrollBody).not.toContainElement(screen.getByRole("heading", { name: "Edit settings" }));
    expect(scrollBody).not.toContainElement(screen.getByRole("button", { name: "Save" }));
    expect(screen.getByRole("button", { name: "Save" }).parentElement).toHaveClass("shrink-0", "flex-wrap");
  });

  it("remembers the opener before a native autofocus field takes focus, including on reopen", async () => {
    const user = userEvent.setup();
    render(<DialogExample autoFocus />);
    const opener = screen.getByRole("button", { name: "Edit settings" });
    for (let attempt = 0; attempt < 2; attempt += 1) {
      await user.click(opener);
      expect(screen.getByRole("textbox", { name: "Display name" })).toHaveFocus();
      await user.keyboard("{Escape}");
      await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
      await waitFor(() => expect(opener).toHaveFocus());
    }
  });

  it("restores focus for conditionally mounted dialogs without replacing native autofocus", async () => {
    const user = userEvent.setup();
    function ConditionalDialog() {
      const [open, setOpen] = useState(false);
      return (
        <>
          <Button onClick={() => setOpen(true)}>Add settings</Button>
          {open ? (
            <Dialog open onOpenChange={setOpen} title="Add settings">
              <Input autoFocus aria-label="New setting" />
              <Button onClick={() => setOpen(false)}>Done</Button>
            </Dialog>
          ) : null}
        </>
      );
    }
    render(<StrictMode><ConditionalDialog /></StrictMode>);
    const opener = screen.getByRole("button", { name: "Add settings" });
    await user.click(opener);
    const input = screen.getByRole("textbox", { name: "New setting" });
    expect(input).toHaveFocus();
    await user.keyboard("Example");
    expect(input).toHaveValue("Example");
    await user.tab();
    expect(screen.getByRole("button", { name: "Done" })).toHaveFocus();
    await user.keyboard("{Enter}");
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    await waitFor(() => expect(opener).toHaveFocus());
  });

  it("traps Tab, closes with Escape and restores focus to the external opener", async () => {
    const user = userEvent.setup();
    render(<DialogExample />);
    const opener = screen.getByRole("button", { name: "Edit settings" });
    await user.click(opener);
    const dialog = screen.getByRole("dialog");
    expect(dialog).not.toHaveAttribute("aria-describedby");
    expect(screen.getByRole("button", { name: "Close" })).toHaveFocus();
    await user.tab({ shift: true });
    expect(screen.getByRole("button", { name: "Save" })).toHaveFocus();
    await user.tab();
    expect(screen.getByRole("button", { name: "Close" })).toHaveFocus();
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    await waitFor(() => expect(opener).toHaveFocus());
  });
});
