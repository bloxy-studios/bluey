import { useEffect, useRef, useState } from "react";

import { useChatStore } from "@/stores/chatStore";
import { answerSummary, pillAnnouncement } from "./announce";
import { useStatePill } from "./useStatePill";

/** Id of the newest turn once it has finished answering. */
function useFinishedTurnId(): string | null {
  return useChatStore((s) => {
    const last = s.turns.at(-1);
    return last?.status === "done" && last.response ? last.id : null;
  });
}

/**
 * One visually hidden polite live region for the HUD (UX-025): state changes
 * (thinking, listening, errors) and "Answer ready: …" once per finished turn —
 * never per streamed token. Spinners inside the HUD are decorative.
 */
export function HudAnnouncer() {
  const [message, setMessage] = useState("");
  const pillText = pillAnnouncement(useStatePill());
  const finishedTurnId = useFinishedTurnId();
  const announcedTurnRef = useRef<string | null>(null);

  useEffect(() => setMessage(pillText ?? ""), [pillText]);

  // Declared after the pill effect so "Answer ready" wins when both change in one commit.
  useEffect(() => {
    if (!finishedTurnId || announcedTurnRef.current === finishedTurnId) return;
    announcedTurnRef.current = finishedTurnId;
    const response = useChatStore.getState().turns.at(-1)?.response;
    if (response) setMessage(`Answer ready: ${answerSummary(response)}`);
  }, [finishedTurnId]);

  return (
    <div role="status" aria-live="polite" aria-atomic="true" className="sr-only" data-testid="hud-announcer">
      {message}
    </div>
  );
}
