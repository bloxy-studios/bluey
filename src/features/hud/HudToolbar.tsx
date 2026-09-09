import { AudioLines, ChevronDown, Eye, EyeOff, Grid2x2, Image, RotateCcw, Timer } from "lucide-react";
import { useEffect, useState } from "react";

import { BlueyMark } from "@/components/BlueyMark";
import { IconButton } from "@/components/ui/IconButton";
import { Keycaps } from "@/components/ui/Keycap";
import { showErrorToast } from "@/components/ui/toast-store";
import { Tooltip } from "@/components/ui/Tooltip";
import { bluey } from "@/lib/tauri/api";
import { toBlueyError } from "@/lib/types";
import { useAppStore } from "@/stores/appStore";
import { modeById, useModesStore } from "@/stores/modesStore";
import { useSessionStore } from "@/stores/sessionStore";
import { preventRepeatedActivation } from "./hud-keyboard";
import { ModeMenu } from "./ModeMenu";
import { SessionMenu } from "./SessionMenu";
import { StatePill } from "./StatePill";

export interface HudToolbarProps {
  screenEnabled: boolean;
  onToggleScreen: () => void;
  hasChat: boolean;
  onNewChat: () => void;
  /** Re-asks the last question when an error offers "Retry". */
  onRetry?: () => void;
}

/**
 * HUD bottom row (52px): logo + state pill · screen / visibility / mode / |
 * / audio + session cluster · "New Chat ⌘R" or "History ↓".
 */
export function HudToolbar({ screenEnabled, onToggleScreen, hasChat, onNewChat, onRetry }: HudToolbarProps) {
  const status = useAppStore((s) => s.status);
  const modes = useModesStore((s) => s.modes);
  const session = useSessionStore((s) => s.active);
  const [protection, setProtection] = useState<boolean | null>(null);

  const audioActive = status?.audioActive ?? false;
  const activeModeName = modeById(modes, status?.modeId)?.name ?? "General";
  const sessionTitle = session ? (session.title ?? "Untitled session") : null;

  useEffect(() => {
    let alive = true;
    void bluey.capture
      .getProtection()
      .then((p) => {
        if (alive) setProtection(p.enabled);
      })
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, []);

  const toggleProtection = async () => {
    if (protection === null) return;
    try {
      const next = await bluey.capture.setProtection({ enabled: !protection });
      setProtection(next.enabled);
    } catch (error) {
      showErrorToast(toBlueyError(error, "capture"));
    }
  };

  const toggleAudio = async () => {
    try {
      if (audioActive) await bluey.audio.stop();
      else await bluey.audio.start();
    } catch (error) {
      showErrorToast(toBlueyError(error, "audio"));
    }
  };

  const audioLabel = audioActive
    ? sessionTitle
      ? `Stop Audio Session · ${sessionTitle}`
      : "Stop Audio Session"
    : "Start Audio Session";

  return (
    <div className="hud-toolbar-container">
      <div
        data-tauri-drag-region
        className={`hud-toolbar flex h-[52px] items-center gap-3 px-3${status?.state === "error" ? " hud-toolbar--error" : ""}`}
      >
        <div data-tauri-drag-region className="hud-status flex min-w-0 flex-1 items-center gap-2.5">
          <BlueyMark size={22} className="ml-1 shrink-0 text-fg" />
          <StatePill onRetry={onRetry} />
        </div>

        <div className="flex items-center gap-1">
          <Tooltip label={screenEnabled ? "Uses Screen" : "Screen off"}>
            <IconButton
              aria-label={screenEnabled ? "Screen context on" : "Screen context off"}
              active={screenEnabled}
              onClick={onToggleScreen}
            >
              <Image className="size-[18px]" strokeWidth={1.8} aria-hidden />
            </IconButton>
          </Tooltip>

          <Tooltip label={protection ? "Content-protected" : "Detectable"}>
            <IconButton
              aria-label={protection ? "Content protection on" : "Content protection off"}
              onClick={() => void toggleProtection()}
              disabled={protection === null}
            >
              {protection ? (
                <EyeOff className="size-[18px]" strokeWidth={1.8} aria-hidden />
              ) : (
                <Eye className="size-[18px]" strokeWidth={1.8} aria-hidden />
              )}
            </IconButton>
          </Tooltip>

          <ModeMenu tooltip={activeModeName}>
            <IconButton aria-label={`Mode: ${activeModeName}`}>
              <Grid2x2 className="size-[18px]" strokeWidth={1.8} aria-hidden />
            </IconButton>
          </ModeMenu>

          <div className="mx-1 h-5 w-px bg-hud-border" aria-hidden />

          <Tooltip label={audioLabel}>
            <IconButton
              aria-label={audioActive ? "Stop audio session" : "Start audio session"}
              onClick={() => void toggleAudio()}
              className="relative"
            >
              <AudioLines className="size-[18px]" strokeWidth={1.8} aria-hidden />
              {audioActive ? (
                <span
                  className="absolute right-1 top-1 size-[6px] rounded-full bg-success motion-safe:animate-pulse-dot"
                  aria-hidden
                />
              ) : null}
            </IconButton>
          </Tooltip>

          <SessionMenu tooltip={sessionTitle ? `Session: ${sessionTitle}` : "Session"}>
            <IconButton
              aria-label={sessionTitle ? `Session: ${sessionTitle}` : "Session menu"}
              className="relative"
            >
              <Timer className="size-[18px]" strokeWidth={1.8} aria-hidden />
              {session ? (
                <span
                  className={
                    session.status === "paused"
                      ? "absolute right-1 top-1 size-[6px] rounded-full bg-fg-muted"
                      : "absolute right-1 top-1 size-[6px] rounded-full bg-accent"
                  }
                  aria-hidden
                />
              ) : null}
            </IconButton>
          </SessionMenu>
        </div>

        <div
          data-tauri-drag-region
          className="hud-trailing flex min-w-0 flex-1 items-center justify-end gap-2"
        >
          {hasChat ? (
            <Tooltip label="New Chat" shortcut="CmdOrCtrl+R">
              <button
                type="button"
                aria-label="New Chat"
                onClick={onNewChat}
                onKeyDown={preventRepeatedActivation}
                className="hud-new-chat-button flex items-center gap-1.5 rounded-[8px] px-2 py-1 text-[13px] text-fg-muted transition-colors hover:bg-hud-chip hover:text-fg"
              >
                <span className="hud-new-chat-label flex items-center gap-1.5">
                  New Chat <Keycaps accelerator="CmdOrCtrl+R" />
                </span>
                <RotateCcw className="hud-new-chat-icon size-4" aria-hidden />
              </button>
            </Tooltip>
          ) : (
            <>
              <span className="hud-trailing-label text-[13px] text-fg-muted">History</span>
              <Tooltip label="Open sessions">
                <IconButton
                  aria-label="Open session history"
                  variant="chip"
                  onClick={() => void bluey.window.open({ label: "settings", route: "sessions" })}
                >
                  <ChevronDown className="size-4" aria-hidden />
                </IconButton>
              </Tooltip>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
