import { useEffect, useRef } from "react";

import { showErrorToast } from "@/components/ui/toast-store";
import { bluey } from "@/lib/tauri/api";
import { eventBus } from "@/lib/tauri/event-bus";
import { toBlueyError } from "@/lib/types";
import { useAppStore } from "@/stores/appStore";
import { hasActiveHudOverlay, isComposingKey } from "./hud-keyboard";

export interface HudShortcutHandlers {
  /** `triggeredAtMs`: the global shortcut's keydown on Bluey's monotonic clock (the trace's `tShortcut`); absent for local keys. */
  onCaptureAnalyze: (triggeredAtMs?: number) => void;
  onGenerate: (triggeredAtMs?: number) => void;
  onNewChat: () => void;
  onEscape: () => void;
}

function isEditableTarget(target: EventTarget | null): boolean {
  return (
    target instanceof HTMLElement &&
    (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable)
  );
}

/**
 * Wires global shortcut events (`shortcut.triggered` from the backend),
 * panel events, and local key handling (Esc, ⌘↵, ⌘⇧↵, ⌘R when the HUD
 * window itself has focus).
 */
export function useHudShortcuts(handlers: HudShortcutHandlers): void {
  const handlersRef = useRef(handlers);
  handlersRef.current = handlers;

  useEffect(() => {
    let composing = false;
    const overlayKeys = new WeakSet<KeyboardEvent>();
    const onCompositionStart = () => {
      composing = true;
    };
    const onCompositionEnd = () => {
      composing = false;
    };
    // Snapshot before a menu/dialog dismisses itself (and disappears) at target
    // or document phase. The original event still belongs to that overlay.
    const onKeyDownCapture = (event: KeyboardEvent) => {
      if (hasActiveHudOverlay()) overlayKeys.add(event);
    };

    const toggleListening = async () => {
      const audioActive = useAppStore.getState().status?.audioActive ?? false;
      try {
        if (audioActive) await bluey.audio.stop();
        else await bluey.audio.start();
      } catch (error) {
        showErrorToast(toBlueyError(error, "audio"));
      }
    };

    const offShortcut = eventBus.on("shortcut.triggered", ({ id, monoMs }) => {
      // Keep backend bindings/dispatch intact; only protect local IME work from
      // ask/new-chat notifications received while the HUD is composing.
      if (composing && (id === "capture_analyze" || id === "generate_response" || id === "new_chat")) return;
      const triggeredAtMs = typeof monoMs === "number" ? monoMs : undefined;
      switch (id) {
        case "capture_analyze":
          handlersRef.current.onCaptureAnalyze(triggeredAtMs);
          break;
        case "generate_response":
          handlersRef.current.onGenerate(triggeredAtMs);
          break;
        case "new_chat":
          handlersRef.current.onNewChat();
          break;
        case "toggle_listening":
          void toggleListening();
          break;
        default:
          break;
      }
    });

    const offNewChat = eventBus.on("panel.newChat", () => {
      if (!composing) handlersRef.current.onNewChat();
    });

    const onKeyDown = (event: KeyboardEvent) => {
      if (
        event.defaultPrevented ||
        composing ||
        isComposingKey(event) ||
        overlayKeys.has(event) ||
        hasActiveHudOverlay()
      )
        return;
      if (event.key === "Escape") {
        event.preventDefault();
        if (!event.repeat) handlersRef.current.onEscape();
        return;
      }
      const meta = event.metaKey || event.ctrlKey;
      if (!meta) return;
      if (event.key === "Enter") {
        // Inputs already own Enter/⌘↵; unhandled ⌘↵ captures, ⌘⇧↵ generates.
        event.preventDefault();
        if (event.repeat) return;
        if (event.shiftKey) handlersRef.current.onGenerate();
        else handlersRef.current.onCaptureAnalyze();
        return;
      }
      if ((event.key === "r" || event.key === "R") && !isEditableTarget(event.target)) {
        event.preventDefault();
        if (!event.repeat) handlersRef.current.onNewChat();
      }
    };

    window.addEventListener("compositionstart", onCompositionStart, true);
    window.addEventListener("compositionend", onCompositionEnd, true);
    window.addEventListener("focusout", onCompositionEnd);
    window.addEventListener("blur", onCompositionEnd);
    window.addEventListener("keydown", onKeyDownCapture, true);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      offShortcut();
      offNewChat();
      window.removeEventListener("compositionstart", onCompositionStart, true);
      window.removeEventListener("compositionend", onCompositionEnd, true);
      window.removeEventListener("focusout", onCompositionEnd);
      window.removeEventListener("blur", onCompositionEnd);
      window.removeEventListener("keydown", onKeyDownCapture, true);
      window.removeEventListener("keydown", onKeyDown);
    };
  }, []);
}
