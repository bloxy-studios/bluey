import { ArrowLeft, CornerDownLeft, Square } from "lucide-react";
import { useEffect, useRef, useState, type KeyboardEvent } from "react";

import { IconButton } from "@/components/ui/IconButton";
import { eventBus } from "@/lib/tauri/event-bus";
import { isComposingKey, preventRepeatedActivation } from "./hud-keyboard";

interface BaseInputRowProps {
  onSubmit: (text: string) => void;
  /** ⌘↵ with an empty input = context-only "Assist". */
  onAssist?: () => void;
}

function useHudInput({ onSubmit, onAssist }: BaseInputRowProps) {
  const [value, setValue] = useState("");
  const inputRef = useRef<HTMLInputElement | null>(null);
  const composingRef = useRef(false);

  useEffect(() => {
    inputRef.current?.focus();
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

export interface HudIdleRowProps extends BaseInputRowProps {
  placeholder?: string;
}

/** Idle first row (56px): "Ask anything about your screen" + ↵ chip. */
export function HudIdleRow({
  onSubmit,
  onAssist,
  placeholder = "Ask anything about your screen",
}: HudIdleRowProps) {
  const { value, setValue, inputRef, submit, onKeyDown, compositionProps } = useHudInput({
    onSubmit,
    onAssist,
  });

  return (
    <div data-tauri-drag-region className="flex h-14 items-center gap-3 pl-5 pr-3">
      <input
        ref={inputRef}
        {...compositionProps}
        value={value}
        onChange={(e) => setValue(e.target.value)}
        onKeyDown={onKeyDown}
        placeholder={placeholder}
        aria-label="Ask Bluey"
        className="hud-input h-full min-w-0 flex-1 bg-transparent text-[15px] text-fg outline-none placeholder:text-fg-muted"
      />
      <IconButton
        aria-label="Submit"
        variant="chip"
        size="lg"
        onClick={submit}
        onKeyDown={preventRepeatedActivation}
      >
        <CornerDownLeft className="size-4" aria-hidden />
      </IconButton>
    </div>
  );
}

export interface FollowUpHeaderProps extends BaseInputRowProps {
  streaming: boolean;
  onBack: () => void;
  onStop: () => void;
}

/** Expanded header: ← back, "Ask follow-up" input, ■ stop / ↵ submit. */
export function FollowUpHeader({ onSubmit, onAssist, streaming, onBack, onStop }: FollowUpHeaderProps) {
  const { value, setValue, inputRef, submit, onKeyDown, compositionProps } = useHudInput({
    onSubmit,
    onAssist,
  });

  return (
    <div data-tauri-drag-region className="flex h-14 items-center gap-3 border-b border-hud-border pl-3 pr-3">
      <IconButton
        aria-label="Back"
        variant="hudCircle"
        size="lg"
        onClick={onBack}
        onKeyDown={preventRepeatedActivation}
      >
        <ArrowLeft className="size-4" aria-hidden />
      </IconButton>
      <input
        ref={inputRef}
        {...compositionProps}
        value={value}
        onChange={(e) => setValue(e.target.value)}
        onKeyDown={onKeyDown}
        placeholder="Ask follow-up"
        aria-label="Ask follow-up"
        className="hud-input h-full min-w-0 flex-1 bg-transparent text-[15px] text-fg outline-none placeholder:text-fg-muted"
      />
      {streaming ? (
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
          aria-label="Submit follow-up"
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
