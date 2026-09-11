import { useCallback, useRef } from "react";

import { useChatStore } from "@/stores/chatStore";
import { useHudUiStore } from "@/stores/hudUiStore";
import { useSettingsStore } from "@/stores/settingsStore";
import { usePanelStore } from "@/stores/panelStore";
import { hasTauriRuntime } from "@/lib/tauri/transport";
import { cn } from "@/lib/utils/cn";
import { HUD_FRAME_INSETS, hudFrameWidth, hudSurfaceMaxHeight } from "./geometry";
import { FollowUpHeader, HudIdleRow } from "./HudInputRow";
import { HudToolbar } from "./HudToolbar";
import { ResponseThread } from "./ResponseThread";
import { TranscriptStrip } from "./TranscriptStrip";
import { useAsk } from "./useAsk";
import { useAutoHeight } from "./useAutoHeight";
import { useHudShortcuts } from "./useHudShortcuts";
import { useHudWorkArea } from "./useHudWorkArea";

/**
 * The bordered HUD surface lives inside a measured, transparent shadow frame.
 * Only the reading region shrinks/scrolls; composer and toolbar stay visible.
 */
export function HudPanel() {
  const settings = useSettingsStore((s) => s.settings);
  const nativeOpacity = usePanelStore((s) => s.state?.opacity);
  const native = hasTauriRuntime();
  const workArea = useHudWorkArea();
  const turns = useChatStore((s) => s.turns);
  const phase = useChatStore((s) => s.phase);
  const screenEnabled = useHudUiStore((s) => s.screenEnabled);
  const toggleScreen = useHudUiStore((s) => s.toggleScreen);
  const panelRef = useRef<HTMLDivElement | null>(null);

  const { ask, stop, generateOrTakePrepared, regenerate, newChat } = useAsk();

  const expanded = turns.length > 0;
  const streaming =
    phase === "capturing" || phase === "analyzing" || phase === "thinking" || phase === "streaming";

  useAutoHeight(panelRef, expanded, `${workArea.width}:${workArea.height}`);

  const submitTyped = useCallback(
    (text: string) => {
      ask({
        trigger: expanded ? "follow_up" : "typed",
        instruction: text,
        captureScreen: !expanded && screenEnabled,
      });
    },
    [ask, expanded, screenEnabled],
  );

  const assist = useCallback(
    (triggeredAtMs?: number) => {
      ask({
        trigger: "shortcut_capture",
        captureScreen: screenEnabled,
        promptLabel: "Assist",
        triggeredAtMs: typeof triggeredAtMs === "number" ? triggeredAtMs : undefined,
      });
    },
    [ask, screenEnabled],
  );

  const onEscape = useCallback(() => {
    if (streaming) void stop();
    else if (expanded) newChat();
  }, [streaming, stop, expanded, newChat]);

  const retry = useCallback(() => {
    if (useChatStore.getState().turns.length > 0) regenerate();
  }, [regenerate]);

  useHudShortcuts({
    onCaptureAnalyze: assist,
    onGenerate: generateOrTakePrepared,
    onNewChat: newChat,
    onEscape,
  });

  const width = settings?.appearance.width ?? 690;
  const opacity = (native ? nativeOpacity : undefined) ?? settings?.appearance.opacity ?? 1;
  const blur = settings?.appearance.blur ?? true;

  const toolbar = (
    <div className="shrink-0 border-t border-hud-border">
      <HudToolbar
        screenEnabled={screenEnabled}
        onToggleScreen={toggleScreen}
        hasChat={expanded}
        onNewChat={newChat}
        onRetry={retry}
      />
    </div>
  );

  return (
    <div
      ref={panelRef}
      data-hud-frame
      className="mx-auto box-border max-w-full"
      style={{
        // The actual WebView width is authoritative after native clamping or
        // resizing. Browser preview still uses the appearance surface width.
        width: native ? "100%" : hudFrameWidth(width),
        padding: `${HUD_FRAME_INSETS.top}px ${HUD_FRAME_INSETS.right}px ${HUD_FRAME_INSETS.bottom}px ${HUD_FRAME_INSETS.left}px`,
      }}
    >
      <div
        role="dialog"
        aria-label="Bluey"
        style={{ maxHeight: hudSurfaceMaxHeight(workArea.height), opacity }}
        className={cn(
          "flex w-full min-w-0 flex-col overflow-hidden rounded-panel border border-hud-border bg-hud-bg",
          "shadow-[0_8px_32px_rgba(0,0,0,0.35)] motion-safe:animate-rise-in",
          blur && "backdrop-blur-[24px] backdrop-saturate-[1.4]",
        )}
      >
        {expanded ? (
          <>
            <div className="shrink-0">
              <FollowUpHeader
                streaming={streaming}
                onBack={newChat}
                onStop={() => void stop()}
                onSubmit={submitTyped}
                onAssist={assist}
              />
            </div>
            <ResponseThread onRegenerate={regenerate} />
            <TranscriptStrip />
            {toolbar}
          </>
        ) : (
          <>
            <div className="shrink-0">
              <HudIdleRow onSubmit={submitTyped} onAssist={assist} />
            </div>
            <TranscriptStrip />
            {toolbar}
          </>
        )}
      </div>
    </div>
  );
}
