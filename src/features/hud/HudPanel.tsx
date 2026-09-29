import { useCallback, useRef, type CSSProperties } from "react";

import { useChatStore } from "@/stores/chatStore";
import { useHudUiStore } from "@/stores/hudUiStore";
import { useSettingsStore } from "@/stores/settingsStore";
import { usePanelStore } from "@/stores/panelStore";
import { hasTauriRuntime } from "@/lib/tauri/transport";
import { cn } from "@/lib/utils/cn";
import { HUD_FRAME_INSETS, hudFrameWidth, hudSurfaceMaxHeight } from "./geometry";
import { HudAnnouncer } from "./HudAnnouncer";
import { HudComposer } from "./HudInputRow";
import { HudNotice } from "./HudNotice";
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
  // Only whether a chat exists: the panel must not re-render per streamed draft (PERF-003).
  const expanded = useChatStore((s) => s.turns.length > 0);
  const phase = useChatStore((s) => s.phase);
  const screenEnabled = useHudUiStore((s) => s.screenEnabled);
  const toggleScreen = useHudUiStore((s) => s.toggleScreen);
  const panelRef = useRef<HTMLDivElement | null>(null);

  const { ask, stop, generateOrTakePrepared, retry: retryTurn, regenerate, newChat } = useAsk();

  const streaming =
    phase === "capturing" || phase === "analyzing" || phase === "thinking" || phase === "streaming";

  useAutoHeight(panelRef, expanded, `${workArea.width}:${workArea.height}`);

  // A thread of only Bluey's own suggestions is not a conversation the user
  // started: a typed question there is a fresh ask with the screen (LIVE-008).
  const followingUp = useChatStore((s) => s.turns.some((turn) => !turn.suggestion));
  const submitTyped = useCallback(
    (text: string) => {
      ask({
        trigger: followingUp ? "follow_up" : "typed",
        instruction: text,
        captureScreen: !followingUp && screenEnabled,
      });
    },
    [ask, followingUp, screenEnabled],
  );

  const stopStreaming = useCallback(() => void stop(), [stop]);

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
    const { draft, setDraft } = useHudUiStore.getState();
    if (streaming) void stop();
    // A half-typed question goes first; only the next Esc clears the thread (UX-012).
    else if (draft.length > 0) setDraft("");
    else if (expanded) newChat();
  }, [streaming, stop, expanded, newChat]);

  const retry = useCallback(() => retryTurn(), [retryTurn]);

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
    <>
      <HudNotice />
      <div className="shrink-0 border-t border-hud-border">
        <HudToolbar
          screenEnabled={screenEnabled}
          onToggleScreen={toggleScreen}
          hasChat={expanded}
          onNewChat={newChat}
          onRetry={retry}
          onTakePrepared={generateOrTakePrepared}
        />
      </div>
    </>
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
        // The opacity preference thins the background, never the text (UX-014).
        style={{ maxHeight: hudSurfaceMaxHeight(workArea.height), "--hud-opacity": opacity } as CSSProperties}
        className={cn(
          "hud-surface flex w-full min-w-0 flex-col overflow-hidden rounded-panel border border-hud-border",
          "shadow-[0_8px_32px_rgba(0,0,0,0.35)] motion-safe:animate-rise-in",
          blur && "backdrop-blur-[24px] backdrop-saturate-[1.4]",
        )}
      >
        {/* One composer and toolbar in both layouts: a turn appearing never remounts them (LIVE-008). */}
        <div className="shrink-0">
          <HudComposer
            expanded={expanded}
            streaming={streaming}
            onBack={newChat}
            onStop={stopStreaming}
            onSubmit={submitTyped}
            onAssist={assist}
          />
        </div>
        {expanded ? <ResponseThread onRetry={retryTurn} onRegenerate={regenerate} /> : null}
        <TranscriptStrip />
        {toolbar}
        <HudAnnouncer />
      </div>
    </div>
  );
}
