import { act, createEvent, fireEvent, render, renderHook, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { FollowUpHeader, HudIdleRow } from "@/features/hud/HudInputRow";
import { useHudShortcuts, type HudShortcutHandlers } from "@/features/hud/useHudShortcuts";
import { eventBus } from "@/lib/tauri/event-bus";
import { setupMockApp } from "./helpers";

const handlers = () => ({
  onCaptureAnalyze: vi.fn(),
  onGenerate: vi.fn(),
  onNewChat: vi.fn(),
  onEscape: vi.fn(),
});

function InputHarness({
  followUp,
  submit,
  assist,
  shortcuts,
}: {
  followUp: boolean;
  submit: (text: string) => void;
  assist: () => void;
  shortcuts: HudShortcutHandlers;
}) {
  useHudShortcuts(shortcuts);
  return followUp ? (
    <FollowUpHeader
      streaming={false}
      onSubmit={submit}
      onAssist={assist}
      onBack={shortcuts.onNewChat}
      onStop={shortcuts.onEscape}
    />
  ) : (
    <HudIdleRow onSubmit={submit} onAssist={assist} />
  );
}

beforeEach(async () => {
  await setupMockApp();
});
afterEach(() => {
  vi.restoreAllMocks();
});

for (const followUp of [false, true]) {
  describe(followUp ? "follow-up input keyboard" : "idle input keyboard", () => {
    const setup = () => {
      const submit = vi.fn();
      const assist = vi.fn();
      const shortcuts = handlers();
      render(<InputHarness followUp={followUp} submit={submit} assist={assist} shortcuts={shortcuts} />);
      return { input: screen.getByRole("textbox"), submit, assist, shortcuts };
    };

    it.each([{}, { metaKey: true }, { ctrlKey: true }])(
      "preserves typed Enter semantics (%j)",
      (modifiers) => {
        const { input, submit, assist, shortcuts } = setup();
        fireEvent.change(input, { target: { value: "  What is this?  " } });
        expect(fireEvent.keyDown(input, { key: "Enter", ...modifiers })).toBe(false);
        expect(submit).toHaveBeenCalledExactlyOnceWith("What is this?");
        expect(input).toHaveValue("");
        expect(assist).not.toHaveBeenCalled();
        expect(shortcuts.onCaptureAnalyze).not.toHaveBeenCalled();
        expect(shortcuts.onGenerate).not.toHaveBeenCalled();
      },
    );

    it.each([{}, { metaKey: true }, { ctrlKey: true }])(
      "preserves empty Assist semantics (%j)",
      (modifiers) => {
        const { input, submit, assist, shortcuts } = setup();
        fireEvent.keyDown(input, { key: "Enter", ...modifiers });
        expect(assist).toHaveBeenCalledOnce();
        expect(submit).not.toHaveBeenCalled();
        expect(shortcuts.onCaptureAnalyze).not.toHaveBeenCalled();
      },
    );

    it.each([{ isComposing: true }, { isComposing: false, keyCode: 229 }])(
      "leaves IME keys and draft untouched (%j)",
      (ime) => {
        const { input, submit, assist, shortcuts } = setup();
        fireEvent.change(input, { target: { value: "こんにちは" } });
        for (const key of [
          { key: "Enter" },
          { key: "Enter", metaKey: true },
          { key: "Enter", metaKey: true, shiftKey: true },
          { key: "Escape" },
          { key: "r", metaKey: true },
        ]) {
          expect(fireEvent.keyDown(input, { ...key, ...ime })).toBe(true);
        }
        expect(input).toHaveValue("こんにちは");
        expect(submit).not.toHaveBeenCalled();
        expect(assist).not.toHaveBeenCalled();
        for (const handler of Object.values(shortcuts)) expect(handler).not.toHaveBeenCalled();
      },
    );

    it("tracks composition even when WebKit omits the event flag, including local backend notifications", () => {
      const { input, submit, assist, shortcuts } = setup();
      fireEvent.change(input, { target: { value: "候補" } });
      fireEvent.compositionStart(input);
      fireEvent.keyDown(input, { key: "Enter" });
      fireEvent.keyDown(input, { key: "Escape" });
      fireEvent.keyDown(input, { key: "Enter", metaKey: true, shiftKey: true });
      fireEvent.keyDown(document.body, { key: "r", metaKey: true });
      fireEvent.click(screen.getByRole("button", { name: followUp ? "Submit follow-up" : "Submit" }));
      act(() => {
        eventBus.emit("shortcut.triggered", { id: "capture_analyze", at: new Date().toISOString() });
        eventBus.emit("shortcut.triggered", { id: "generate_response", at: new Date().toISOString() });
        eventBus.emit("shortcut.triggered", { id: "new_chat", at: new Date().toISOString() });
        eventBus.emit("panel.newChat", {});
      });
      expect(input).toHaveValue("候補");
      expect(submit).not.toHaveBeenCalled();
      expect(assist).not.toHaveBeenCalled();
      for (const handler of Object.values(shortcuts)) expect(handler).not.toHaveBeenCalled();

      // WebKit can send compositionend BEFORE the confirming Enter, with keyCode=229.
      fireEvent.compositionEnd(input, { data: "候補" });
      fireEvent.keyDown(input, { key: "Enter", keyCode: 229, isComposing: false });
      expect(submit).not.toHaveBeenCalled();
      fireEvent.keyDown(input, { key: "Enter" });
      expect(submit).toHaveBeenCalledExactlyOnceWith("候補");
    });

    it("does not turn held Enter into an empty Assist or held Escape into a second clear", () => {
      const { input, submit, assist, shortcuts } = setup();
      fireEvent.change(input, { target: { value: "question" } });
      fireEvent.keyDown(input, { key: "Enter" });
      expect(fireEvent.keyDown(input, { key: "Enter", repeat: true })).toBe(false);
      expect(submit).toHaveBeenCalledOnce();
      expect(assist).not.toHaveBeenCalled();
      fireEvent.keyDown(input, { key: "Escape" });
      fireEvent.keyDown(input, { key: "Escape", repeat: true });
      expect(shortcuts.onEscape).toHaveBeenCalledOnce();
    });

    it("keeps Shift+Enter untouched, and Command+Shift+Enter generates just once", () => {
      const { input, submit, assist, shortcuts } = setup();
      expect(fireEvent.keyDown(input, { key: "Enter", shiftKey: true })).toBe(true);
      fireEvent.keyDown(input, { key: "Enter", metaKey: true, shiftKey: true });
      fireEvent.keyDown(input, { key: "Enter", metaKey: true, shiftKey: true, repeat: true });
      expect(shortcuts.onGenerate).toHaveBeenCalledOnce();
      expect(submit).not.toHaveBeenCalled();
      expect(assist).not.toHaveBeenCalled();
    });

    it("respects already-consumed Enter", () => {
      const { input, submit, assist } = setup();
      fireEvent.change(input, { target: { value: "draft" } });
      const event = createEvent.keyDown(input, { key: "Enter" });
      event.preventDefault();
      fireEvent(input, event);
      expect(input).toHaveValue("draft");
      expect(submit).not.toHaveBeenCalled();
      expect(assist).not.toHaveBeenCalled();
    });

    it("keeps native text editing shortcuts unconsumed and leaves editable Command+R alone", () => {
      const { input, submit, assist, shortcuts } = setup();
      for (const modifier of [{ metaKey: true }, { ctrlKey: true }]) {
        for (const key of ["a", "c", "v", "x", "z", "Z", "y", "r"]) {
          expect(fireEvent.keyDown(input, { key, ...modifier, shiftKey: key === "Z" })).toBe(true);
        }
      }
      expect(submit).not.toHaveBeenCalled();
      expect(assist).not.toHaveBeenCalled();
      for (const handler of Object.values(shortcuts)) expect(handler).not.toHaveBeenCalled();
    });

    it("prevents repeated button activation without disabling a fresh click", () => {
      const { input, submit } = setup();
      fireEvent.change(input, { target: { value: "question" } });
      const button = screen.getByRole("button", { name: followUp ? "Submit follow-up" : "Submit" });
      expect(fireEvent.keyDown(button, { key: "Enter", repeat: true })).toBe(false);
      expect(fireEvent.keyDown(button, { key: " ", repeat: true })).toBe(false);
      expect(submit).not.toHaveBeenCalled();
      fireEvent.click(button);
      expect(submit).toHaveBeenCalledExactlyOnceWith("question");
    });
  });
}

describe("HUD shortcut scope", () => {
  it.each(["menu", "dialog", "alertdialog", "listbox"])(
    "does not run local HUD actions while a %s owns the interaction",
    (role) => {
      const shortcuts = handlers();
      renderHook(() => useHudShortcuts(shortcuts));
      const { unmount } = render(
        <div role={role} data-state="open">
          <input aria-label="Overlay input" />
        </div>,
      );
      const target = screen.getByLabelText("Overlay input");
      for (const key of [{ key: "Escape" }, { key: "Enter", metaKey: true }, { key: "r", metaKey: true }]) {
        expect(fireEvent.keyDown(target, key)).toBe(true);
        expect(fireEvent.keyDown(document.body, key)).toBe(true);
      }
      for (const handler of Object.values(shortcuts)) expect(handler).not.toHaveBeenCalled();
      unmount();
      fireEvent.keyDown(document.body, { key: "Escape" });
      expect(shortcuts.onEscape).toHaveBeenCalledOnce();
    },
  );

  it("remembers the overlay scope even if Escape removes it before the window bubble phase", () => {
    const shortcuts = handlers();
    renderHook(() => useHudShortcuts(shortcuts));
    const dialog = document.createElement("div");
    dialog.setAttribute("role", "dialog");
    dialog.setAttribute("data-state", "open");
    dialog.addEventListener("keydown", () => dialog.remove());
    document.body.append(dialog);
    fireEvent.keyDown(dialog, { key: "Escape" });
    expect(dialog.isConnected).toBe(false);
    expect(shortcuts.onEscape).not.toHaveBeenCalled();
    fireEvent.keyDown(document.body, { key: "Escape" });
    expect(shortcuts.onEscape).toHaveBeenCalledOnce();
  });

  it("does not let a tooltip or closed/hidden menu disable HUD shortcuts", () => {
    const shortcuts = handlers();
    renderHook(() => useHudShortcuts(shortcuts));
    render(
      <>
        <div role="menu" data-state="closed" />
        <div role="dialog" hidden />
        <div role="listbox" aria-hidden="true" />
        <div role="tooltip">Tip</div>
      </>,
    );
    fireEvent.keyDown(document.body, { key: "Escape" });
    expect(shortcuts.onEscape).toHaveBeenCalledOnce();
  });

  it.each([{ key: "Escape" }, { key: "Enter", metaKey: true }, { key: "r", metaKey: true }])(
    "respects defaultPrevented (%j)",
    (key) => {
      const shortcuts = handlers();
      renderHook(() => useHudShortcuts(shortcuts));
      const event = createEvent.keyDown(document.body, key);
      event.preventDefault();
      fireEvent(document.body, event);
      for (const handler of Object.values(shortcuts)) expect(handler).not.toHaveBeenCalled();
    },
  );

  it("ignores repeated local asks/new-chat, while preserving existing outside-input bindings", () => {
    const shortcuts = handlers();
    renderHook(() => useHudShortcuts(shortcuts));
    for (const key of [
      { key: "Enter", metaKey: true },
      { key: "r", ctrlKey: true },
    ]) {
      expect(fireEvent.keyDown(document.body, key)).toBe(false);
      expect(fireEvent.keyDown(document.body, { ...key, repeat: true })).toBe(false);
    }
    expect(shortcuts.onCaptureAnalyze).toHaveBeenCalledOnce();
    expect(shortcuts.onNewChat).toHaveBeenCalledOnce();
  });

  it("preserves panel/configured backend events outside composition and releases the gate on blur", () => {
    const shortcuts = handlers();
    renderHook(() => useHudShortcuts(shortcuts));
    fireEvent.compositionStart(document.body);
    fireEvent.blur(window);
    act(() => {
      eventBus.emit("shortcut.triggered", { id: "capture_analyze", at: new Date().toISOString() });
      eventBus.emit("shortcut.triggered", { id: "generate_response", at: new Date().toISOString() });
      eventBus.emit("shortcut.triggered", { id: "new_chat", at: new Date().toISOString() });
      eventBus.emit("panel.newChat", {});
    });
    expect(shortcuts.onCaptureAnalyze).toHaveBeenCalledOnce();
    expect(shortcuts.onGenerate).toHaveBeenCalledOnce();
    expect(shortcuts.onNewChat).toHaveBeenCalledTimes(2);
  });

  it("releases a composition gate when focus moves inside the HUD without a compositionend", () => {
    const shortcuts = handlers();
    renderHook(() => useHudShortcuts(shortcuts));
    render(<input aria-label="Composing input" />);
    const input = screen.getByLabelText("Composing input");
    fireEvent.compositionStart(input);
    fireEvent.keyDown(document.body, { key: "r", metaKey: true });
    expect(shortcuts.onNewChat).not.toHaveBeenCalled();
    fireEvent.focusOut(input);
    fireEvent.keyDown(document.body, { key: "r", metaKey: true });
    expect(shortcuts.onNewChat).toHaveBeenCalledOnce();
  });

  it("uses the latest handlers and cleans up keyboard/composition/event subscriptions on unmount", () => {
    const first = handlers();
    const latest = handlers();
    const { rerender, unmount } = renderHook(({ shortcuts }) => useHudShortcuts(shortcuts), {
      initialProps: { shortcuts: first },
    });
    rerender({ shortcuts: latest });
    fireEvent.keyDown(document.body, { key: "Escape" });
    expect(first.onEscape).not.toHaveBeenCalled();
    expect(latest.onEscape).toHaveBeenCalledOnce();
    unmount();
    fireEvent.compositionStart(document.body);
    fireEvent.keyDown(document.body, { key: "Escape" });
    eventBus.emit("panel.newChat", {});
    eventBus.emit("shortcut.triggered", { id: "capture_analyze", at: new Date().toISOString() });
    expect(latest.onEscape).toHaveBeenCalledOnce();
    expect(latest.onNewChat).not.toHaveBeenCalled();
    expect(latest.onCaptureAnalyze).not.toHaveBeenCalled();
  });
});
