import { History, Pause, Play, Square } from "lucide-react";
import { type ReactElement } from "react";

import { formatDuration } from "@/lib/utils/format";
import { useSessionStore } from "@/stores/sessionStore";
import { HudMenu, type HudMenuEntry } from "./HudMenu";
import { endSession, openSessionHistory, pauseSession, resumeSession, startSession } from "./session-actions";

export interface SessionMenuProps {
  children: ReactElement;
  tooltip?: string;
}

type SessionAction = "start" | "pause" | "resume" | "end" | "history";
const actions: Record<SessionAction, () => Promise<boolean>> = {
  start: startSession,
  pause: pauseSession,
  resume: resumeSession,
  end: endSession,
  history: openSessionHistory,
};

/** Session actions remain owned by session-actions, including their error toasts. */
export function SessionMenu({ children, tooltip }: SessionMenuProps) {
  const active = useSessionStore((s) => s.active);
  const entries: HudMenuEntry<SessionAction>[] = active
    ? [
        {
          kind: "label",
          id: "summary",
          label: `${active.title ?? "Untitled session"} · ${formatDuration(active.startedAt)}${active.status === "paused" ? " · paused" : ""}`,
        },
        active.status === "paused"
          ? {
              kind: "item",
              id: "resume",
              label: "Resume session",
              action: "resume",
              icon: <Play className="size-4" aria-hidden />,
              nativeIcon: "play",
            }
          : {
              kind: "item",
              id: "pause",
              label: "Pause session",
              action: "pause",
              icon: <Pause className="size-4" aria-hidden />,
              nativeIcon: "pause",
            },
        {
          kind: "item",
          id: "end",
          label: "End session",
          action: "end",
          destructive: true,
          icon: <Square className="size-4" aria-hidden />,
          nativeIcon: "stop",
        },
      ]
    : [
        { kind: "label", id: "summary", label: "No active session" },
        {
          kind: "item",
          id: "start",
          label: "Start session",
          action: "start",
          icon: <Play className="size-4" aria-hidden />,
          nativeIcon: "play",
        },
      ];
  entries.push(
    { kind: "separator", id: "history-separator" },
    {
      kind: "item",
      id: "history",
      label: active ? "Open in History" : "Open History",
      action: "history",
      icon: <History className="size-4" aria-hidden />,
      nativeIcon: "history",
    },
  );

  return (
    <HudMenu entries={entries} onSelect={(action) => void actions[action]()} align="end" tooltip={tooltip}>
      {children}
    </HudMenu>
  );
}
