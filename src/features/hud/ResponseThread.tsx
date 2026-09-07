import { ArrowDown } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";

import { ErrorBanner } from "@/components/ui/ErrorBanner";
import { Spinner } from "@/components/ui/Spinner";
import { eventBus } from "@/lib/tauri/event-bus";
import { useChatStore, type ChatTurn } from "@/stores/chatStore";
import { ResponseActions } from "./ResponseActions";
import { ResponseView } from "./ResponseView";

function PromptPill({ label }: { label: string }) {
  return (
    <div className="flex justify-end">
      <span className="max-w-[80%] rounded-card bg-hud-prompt px-4 py-2.5 text-[14px] leading-snug text-fg">
        {label}
      </span>
    </div>
  );
}

function Turn({ turn, isLast, onRegenerate }: { turn: ChatTurn; isLast: boolean; onRegenerate: () => void }) {
  const phase = useChatStore((s) => s.phase);
  const streaming = turn.status === "streaming";

  return (
    <div className="flex flex-col gap-3">
      <PromptPill label={turn.promptLabel} />
      {turn.error ? (
        <ErrorBanner error={turn.error} onRetry={onRegenerate} compact />
      ) : turn.response ? (
        <>
          <ResponseView response={turn.response} streaming={streaming} />
          {turn.status === "done" && isLast ? <ResponseActions response={turn.response} onRegenerate={onRegenerate} /> : null}
        </>
      ) : streaming ? (
        <div className="flex items-center gap-2 py-1 text-[13px] text-fg-muted motion-safe:animate-fade-in">
          <Spinner size={12} />
          {phase === "capturing" || phase === "analyzing" ? "Reading screen…" : "Thinking…"}
        </div>
      ) : turn.status === "cancelled" ? (
        <div className="text-[13px] text-fg-subtle">Stopped.</div>
      ) : null}
    </div>
  );
}

export interface ResponseThreadProps {
  onRegenerate: () => void;
}

/** Scrollable response body with auto-follow and a floating ↓ button. */
export function ResponseThread({ onRegenerate }: ResponseThreadProps) {
  const turns = useChatStore((s) => s.turns);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const [atBottom, setAtBottom] = useState(true);
  const atBottomRef = useRef(true);

  const scrollToBottom = useCallback((smooth = true) => {
    const el = scrollRef.current;
    if (!el) return;
    el.scrollTo({ top: el.scrollHeight, behavior: smooth ? "smooth" : "auto" });
  }, []);

  const handleScroll = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    const nearBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 48;
    atBottomRef.current = nearBottom;
    setAtBottom(nearBottom);
  }, []);

  // Follow the stream while the user is at the bottom.
  const lastContent = turns[turns.length - 1]?.response?.content;
  useEffect(() => {
    if (atBottomRef.current) scrollToBottom(false);
  }, [lastContent, turns.length, scrollToBottom]);

  // Global scroll shortcuts (⇧⌘↑ / ⇧⌘↓ forwarded by the backend).
  useEffect(() => {
    return eventBus.on("panel.scroll", ({ direction }) => {
      const el = scrollRef.current;
      if (!el) return;
      el.scrollBy({ top: direction === "down" ? 160 : -160, behavior: "smooth" });
    });
  }, []);

  return (
    // Content-driven height: the panel grows with the response (DESIGN.md
    // 380–620px) and the body scrolls past the cap.
    <div className="relative">
      <div
        ref={scrollRef}
        onScroll={handleScroll}
        className="max-h-[460px] min-h-[120px] overflow-y-auto overscroll-contain px-5 py-4"
      >
        <div className="flex flex-col gap-5">
          {turns.map((turn, index) => (
            <Turn key={turn.id} turn={turn} isLast={index === turns.length - 1} onRegenerate={onRegenerate} />
          ))}
        </div>
      </div>
      {!atBottom ? (
        <button
          type="button"
          aria-label="Scroll to bottom"
          onClick={() => scrollToBottom()}
          className="absolute bottom-3 right-4 flex size-9 items-center justify-center rounded-full bg-hud-chip text-fg shadow-lg shadow-black/25 backdrop-blur transition-colors hover:bg-white/20 motion-safe:animate-fade-in"
        >
          <ArrowDown className="size-4" aria-hidden />
        </button>
      ) : null}
    </div>
  );
}
