import { useCallback, useRef, useState } from "react";

import { useChatStore } from "@/stores/chatStore";
import { useSettingsStore } from "@/stores/settingsStore";
import { cn } from "@/lib/utils/cn";
import { FollowUpHeader, HudIdleRow } from "./HudInputRow";
import { HudToolbar } from "./HudToolbar";
import { ResponseThread } from "./ResponseThread";
import { useAsk } from "./useAsk";
import { useAutoHeight } from "./useAutoHeight";
import { useHudShortcuts } from "./useHudShortcuts";

/**
 * The floating HUD (main window). Idle: input row + toolbar (108px).
 * With a chat: follow-up header + scrolling response body + toolbar.
 */
export function HudPanel() {
  const settings = useSettingsStore((s) => s.settings);
  const turns = useChatStore((s) => s.turns);
  const phase = useChatStore((s) => s.phase);

  const [screenEnabled, setScreenEnabled] = useState(true);
  const panelRef = useRef<HTMLDivElement | null>(null);

  const { ask, stop, generateOrTakePrepared, regenerate, newChat } = useAsk();

  const expanded = turns.length > 0;
  const streaming = phase === "capturing" || phase === "analyzing" || phase === "thinking" || phase === "streaming";

  useAutoHeight(panelRef, expanded);

  const submitTyped = useCallback(
    (text: string) => {
      ask({ trigger: expanded ? "follow_up" : "typed", instruction: text, captureScreen: !expanded && screenEnabled });
    },
    [ask, expanded, screenEnabled],
  );

  const assist = useCallback(() => {
    ask({ trigger: "shortcut_capture", captureScreen: screenEnabled, promptLabel: "Assist" });
  }, [ask, screenEnabled]);

  const onEscape = useCallback(() => {
    if (streaming) void stop();
    else if (expanded) newChat();
  }, [streaming, stop, expanded, newChat]);

  useHudShortcuts({
    onCaptureAnalyze: assist,
    onGenerate: generateOrTakePrepared,
    onNewChat: newChat,
    onEscape,
  });

  const width = settings?.appearance.width ?? 690;
  const opacity = settings?.appearance.opacity ?? 1;
  const blur = settings?.appearance.blur ?? true;

  return (
    <div className="flex h-full items-start justify-center pt-4">
      <div
        ref={panelRef}
        role="dialog"
        aria-label="Bluey"
        style={{ width, opacity }}
        className={cn(
          "flex max-h-[620px] flex-col overflow-hidden rounded-panel border border-hud-border bg-hud-bg",
          "shadow-[0_8px_32px_rgba(0,0,0,0.35)] motion-safe:animate-rise-in",
          blur && "backdrop-blur-[24px] backdrop-saturate-[1.4]",
        )}
      >
        {expanded ? (
          <>
            <FollowUpHeader
              streaming={streaming}
              onBack={newChat}
              onStop={() => void stop()}
              onSubmit={submitTyped}
              onAssist={assist}
            />
            <ResponseThread onRegenerate={regenerate} />
            <div className="shrink-0 border-t border-hud-border">
              <HudToolbar
                screenEnabled={screenEnabled}
                onToggleScreen={() => setScreenEnabled((v) => !v)}
                hasChat
                onNewChat={newChat}
              />
            </div>
          </>
        ) : (
          <>
            <HudIdleRow onSubmit={submitTyped} onAssist={assist} />
            <div className="shrink-0 border-t border-hud-border">
              <HudToolbar
                screenEnabled={screenEnabled}
                onToggleScreen={() => setScreenEnabled((v) => !v)}
                hasChat={false}
                onNewChat={newChat}
              />
            </div>
          </>
        )}
      </div>
    </div>
  );
}
