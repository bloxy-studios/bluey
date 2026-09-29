import { ArrowLeft, CornerDownLeft, Square } from "lucide-react";
import { useEffect, useRef, type KeyboardEvent } from "react";

import { IconButton } from "@/components/ui/IconButton";
import { eventBus } from "@/lib/tauri/event-bus";
import { useHudUiStore } from "@/stores/hudUiStore";
import { usePanelStore } from "@/stores/panelStore";
import { isComposingKey, preventRepeatedActivation } from "./hud-keyboard";

interface BaseInputRowProps {
  onSubmit: (text: string) => void;
  /** ⌘↵ with an empty input = context-only "Assist". */
  onAssist?: () => void;
}

function useHudInput({ onSubmit, onAssist }: BaseInputRowProps) {
  const value = useHudUiStore((s) => s.draft);
  const setValue = useHudUiStore((s) => s.setDraft);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const composingRef = useRef(false);
  const visible = usePanelStore((s) => s.state?.visible);

  // Focus when the HUD is shown — never because a (live) turn appeared (LIVE-008).
  useEffect(() => {
    if (visible !== false) inputRef.current?.focus();
  }, [visible]);

  useEffect(() => {
    const offFocus = eventBus.on("panel.focusInput", () => inputRef.current?.focus());
    const onWindowBlur = () => {
      composingRef.current = false;
    };
    window.addEventListener("blur", onWindowBlur);
    return () => {
      offFocus();
      window.removeEventListener("blur", onWindowBlur);
    };
  }, []);

  const submit = () => {
    if (composingRef.current) return;
    const text = value.trim();
    if (text.length > 0) {
      onSubmit(text);
      setValue("");
    } else {
      onAssist?.();
    }
  };

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    // Enter and ⌘↵ submit here; ⌘⇧↵ (generate suggestion) bubbles to the
    // window-level handler in useHudShortcuts.
    if (event.defaultPrevented || composingRef.current || isComposingKey(event.nativeEvent)) return;
    if (event.key !== "Enter" || event.shiftKey) return;
    event.preventDefault();
    event.stopPropagation();
    if (!event.repeat) submit();
  };

  const compositionProps = {
    onCompositionStart: () => {
      composingRef.current = true;
    },
    onCompositionEnd: () => {
      composingRef.current = false;
    },
    onBlur: () => {
      composingRef.current = false;
    },
  };

  return { value, setValue, inputRef, submit, onKeyDown, compositionProps };
}

export interface HudComposerProps extends BaseInputRowProps {
  /** A chat exists: ← back, "Ask follow-up", ■ stop while streaming. */
  expanded: boolean;
  streaming?: boolean;
  onBack?: () => void;
  onStop?: () => void;
}

/**
 * The HUD's one composer. Both layouts render this same component at the same
 * place, so the input element — its text, caret and focus — survives the switch
 * to the expanded layout when a turn (a live suggestion too) appears (LIVE-008).
 * Idle row (56px): "Ask anything about your screen" + ↵ chip; expanded: ← back,
 * "Ask follow-up", ■ stop / ↵ submit.
 */
export function HudComposer({ expanded, streaming = false, onBack, onStop, ...input }: HudComposerProps) {
  const { value, setValue, inputRef, submit, onKeyDown, compositionProps } = useHudInput(input);
  const label = expanded ? "Ask follow-up" : "Ask Bluey";

  return (
    <div
      data-tauri-drag-region
      className={
        expanded
          ? "flex h-14 items-center gap-3 border-b border-hud-border pl-3 pr-3"
          : "flex h-14 items-center gap-3 pl-5 pr-3"
      }
    >
      {expanded ? (
        <IconButton
          aria-label="Back"
          variant="hudCircle"
          size="lg"
          onClick={onBack}
          onKeyDown={preventRepeatedActivation}
        >
          <ArrowLeft className="size-4" aria-hidden />
        </IconButton>
      ) : null}
      <input
        ref={inputRef}
        {...compositionProps}
        value={value}
        onChange={(e) => setValue(e.target.value)}
        onKeyDown={onKeyDown}
        placeholder={expanded ? "Ask follow-up" : "Ask anything about your screen"}
        aria-label={label}
        className="hud-input h-full min-w-0 flex-1 bg-transparent text-[15px] text-fg outline-none placeholder:text-fg-muted"
      />
      {expanded && streaming ? (
        <IconButton
          aria-label="Stop generating"
          variant="hudCircle"
          size="lg"
          onClick={onStop}
          onKeyDown={preventRepeatedActivation}
        >
          <Square className="size-3.5 fill-current" aria-hidden />
        </IconButton>
      ) : (
        <IconButton
          aria-label={expanded ? "Submit follow-up" : "Submit"}
          variant="chip"
          size="lg"
          onClick={submit}
          onKeyDown={preventRepeatedActivation}
        >
          <CornerDownLeft className="size-4" aria-hidden />
        </IconButton>
      )}
    </div>
  );
}

/** The idle composer. */
export function HudIdleRow(props: BaseInputRowProps) {
  return <HudComposer {...props} expanded={false} />;
}

export interface FollowUpHeaderProps extends BaseInputRowProps {
  streaming: boolean;
  onBack: () => void;
  onStop: () => void;
}

/** The expanded composer. */
export function FollowUpHeader(props: FollowUpHeaderProps) {
  return <HudComposer {...props} expanded />;
}
