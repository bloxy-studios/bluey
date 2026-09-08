import { useEffect, useRef } from "react";

import { showErrorToast } from "@/components/ui/toast-store";
import { bluey } from "@/lib/tauri/api";
import { eventBus } from "@/lib/tauri/event-bus";
import { toBlueyError } from "@/lib/types";
import { useAppStore } from "@/stores/appStore";

export interface HudShortcutHandlers {
  onCaptureAnalyze: () => void;
  onGenerate: () => void;
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
    const toggleListening = async () => {
      const audioActive = useAppStore.getState().status?.audioActive ?? false;
      try {
        if (audioActive) await bluey.audio.stop();
        else await bluey.audio.start();
      } catch (error) {
        showErrorToast(toBlueyError(error, "audio"));
      }
    };

    const offShortcut = eventBus.on("shortcut.triggered", ({ id }) => {
      switch (id) {
        case "capture_analyze":
          handlersRef.current.onCaptureAnalyze();
          break;
        case "generate_response":
          handlersRef.current.onGenerate();
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

    const offNewChat = eventBus.on("panel.newChat", () => handlersRef.current.onNewChat());

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        handlersRef.current.onEscape();
        return;
      }
      const meta = event.metaKey || event.ctrlKey;
      if (!meta) return;
      if (event.key === "Enter") {
        // Inputs own plain Enter; ⌘↵ always means capture+analyze / generate.
        event.preventDefault();
        if (event.shiftKey) handlersRef.current.onGenerate();
        else handlersRef.current.onCaptureAnalyze();
        return;
      }
      if ((event.key === "r" || event.key === "R") && !isEditableTarget(event.target)) {
        event.preventDefault();
        handlersRef.current.onNewChat();
      }
    };

    window.addEventListener("keydown", onKeyDown);
    return () => {
      offShortcut();
      offNewChat();
      window.removeEventListener("keydown", onKeyDown);
    };
  }, []);
}
