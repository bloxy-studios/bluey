import { History, Pause, Play, Square } from "lucide-react";
import { type ReactNode } from "react";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/DropdownMenu";
import { formatDuration } from "@/lib/utils/format";
import { useSessionStore } from "@/stores/sessionStore";
import { endSession, openSessionHistory, pauseSession, resumeSession, startSession } from "./session-actions";

export interface SessionMenuProps {
  children: ReactNode;
}

/**
 * HUD session menu: the active session (title · duration · state) with
 * pause / resume / end, or "Start session" when none is running.
 */
export function SessionMenu({ children }: SessionMenuProps) {
  const active = useSessionStore((s) => s.active);

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>{children}</DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        {active ? (
          <>
            <DropdownMenuLabel className="max-w-[260px] truncate">
              {active.title ?? "Untitled session"} · {formatDuration(active.startedAt)}
              {active.status === "paused" ? " · paused" : ""}
            </DropdownMenuLabel>
            {active.status === "paused" ? (
              <DropdownMenuItem
                icon={<Play className="size-4" aria-hidden />}
                onSelect={() => void resumeSession()}
              >
                Resume session
              </DropdownMenuItem>
            ) : (
              <DropdownMenuItem
                icon={<Pause className="size-4" aria-hidden />}
                onSelect={() => void pauseSession()}
              >
                Pause session
              </DropdownMenuItem>
            )}
            <DropdownMenuItem
              icon={<Square className="size-4" aria-hidden />}
              destructive
              onSelect={() => void endSession()}
            >
              End session
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            <DropdownMenuItem
              icon={<History className="size-4" aria-hidden />}
              onSelect={() => void openSessionHistory()}
            >
              Open in History
            </DropdownMenuItem>
          </>
        ) : (
          <>
            <DropdownMenuLabel>No active session</DropdownMenuLabel>
            <DropdownMenuItem
              icon={<Play className="size-4" aria-hidden />}
              onSelect={() => void startSession()}
            >
              Start session
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            <DropdownMenuItem
              icon={<History className="size-4" aria-hidden />}
              onSelect={() => void openSessionHistory()}
            >
              Open History
            </DropdownMenuItem>
          </>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
