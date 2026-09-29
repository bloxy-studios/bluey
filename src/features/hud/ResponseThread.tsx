import { ArrowDown } from "lucide-react";
import { memo, useCallback, useEffect, useRef, useState } from "react";

import { ErrorBanner } from "@/components/ui/ErrorBanner";
import { ErrorBoundary } from "@/components/ui/ErrorBoundary";
import { Pill } from "@/components/ui/Pill";
import { Spinner } from "@/components/ui/Spinner";
import { truncatedAnswerError } from "@/lib/errors/answers";
import { eventBus } from "@/lib/tauri/event-bus";
import { useChatStore, type ChatTurn, type SuggestionMeta } from "@/stores/chatStore";
import { useResearchStore } from "@/stores/researchStore";
import { ResponseActions } from "./ResponseActions";
import { ResponseView } from "./ResponseView";
import { followScrollTop, offsetInScroller } from "./thread-scroll";

/** Streaming placeholder: what the pipeline is doing right now, incl. deep research. */
function StreamingStatus() {
  const phase = useChatStore((s) => s.phase);
  const research = useResearchStore((s) => s.active);
  const skip = useResearchStore((s) => s.skip);

  if (research) {
    return (
      <div className="flex items-center gap-2 py-1 text-[13px] text-fg-muted motion-safe:animate-fade-in">
        <Spinner size={12} decorative />
        <span className="min-w-0 flex-1 truncate">
          Researching · {research.message}
          {research.toolCalls > 0
            ? ` (${research.toolCalls} ${research.toolCalls === 1 ? "lookup" : "lookups"})`
            : ""}
        </span>
        <button
          type="button"
          onClick={() => void skip()}
          disabled={research.cancelling}
          className="shrink-0 text-[12.5px] font-medium text-accent hover:text-accent-hover disabled:opacity-50"
        >
          Skip research
        </button>
      </div>
    );
  }
  return (
    <div className="flex items-center gap-2 py-1 text-[13px] text-fg-muted motion-safe:animate-fade-in">
      <Spinner size={12} decorative />
      {phase === "capturing" || phase === "analyzing" ? "Reading screen…" : "Thinking…"}
    </div>
  );
}

function PromptPill({ label }: { label: string }) {
  return (
    <div className="flex justify-end">
      <span className="max-w-[80%] rounded-card bg-hud-prompt px-4 py-2.5 text-[14px] leading-snug text-fg">
        {label}
      </span>
    </div>
  );
}

/**
 * Provenance of a suggestion Bluey opened itself — left-aligned, unlike the user's
 * own prompt: who asked, and the question the answer below is for.
 */
function SuggestionHeader({ suggestion }: { suggestion: SuggestionMeta }) {
  return (
    <div className="flex items-start gap-2 text-[13px] leading-snug" data-testid="suggestion-header">
      <Pill size="sm" variant="accent" className="mt-px shrink-0">
        Suggested
      </Pill>
      <p className="m-0 line-clamp-2 min-w-0 text-fg-muted">
        <span className="font-medium text-fg-subtle">
          {suggestion.speaker ? `${suggestion.speaker} asked` : "Question"}
        </span>{" "}
        <span className="italic">“{suggestion.question}”</span>
      </p>
    </div>
  );
}

interface TurnProps {
  turn: ChatTurn;
  isLast: boolean;
  onRetry: (turnId: string) => void;
  onRegenerate: (turnId: string) => void;
}

/** Finished turns keep their identity in the store, so only the streaming turn re-renders (PERF-003). */
const Turn = memo(function Turn({ turn, isLast, onRetry, onRegenerate }: TurnProps) {
  const streaming = turn.status === "streaming";
  // Each turn re-sends its own request, not whatever the last turn asked (UX-011).
  const retry = () => onRetry(turn.id);
  const regenerate = () => onRegenerate(turn.id);

  return (
    <div className="flex flex-col gap-3">
      {turn.suggestion ? (
        <SuggestionHeader suggestion={turn.suggestion} />
      ) : (
        <PromptPill label={turn.promptLabel} />
      )}
      {turn.error ? (
        <ErrorBanner error={turn.error} onRetry={retry} compact />
      ) : turn.response ? (
        <>
          <ResponseView response={turn.response} streaming={streaming} />
          {turn.response.truncated && turn.status === "done" ? (
            <ErrorBanner error={truncatedAnswerError()} onRetry={regenerate} compact />
          ) : null}
          {turn.response.researchNote && turn.status === "done" ? (
            // The answer went without the web it was meant to use (UX-035).
            <p className="m-0 text-[12px] text-fg-subtle">{turn.response.researchNote}</p>
          ) : null}
          {turn.status === "done" && isLast ? (
            <ResponseActions response={turn.response} onRegenerate={regenerate} />
          ) : null}
        </>
      ) : streaming ? (
        <StreamingStatus />
      ) : turn.status === "cancelled" ? (
        <div className="text-[13px] text-fg-subtle">Stopped.</div>
      ) : null}
    </div>
  );
});

const TURN_FALLBACK = <div className="text-[13px] text-fg-subtle">This answer couldn’t be displayed.</div>;

export interface ResponseThreadProps {
  /** Re-send a failed turn's original request. */
  onRetry: (turnId: string) => void;
  /** Ask a finished turn's question again (same screen/detected-question context). */
  onRegenerate: (turnId: string) => void;
}

/** Scrollable response body with auto-follow and a floating ↓ button. */
export function ResponseThread({ onRetry, onRegenerate }: ResponseThreadProps) {
  const turns = useChatStore((s) => s.turns);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const contentRef = useRef<HTMLDivElement | null>(null);
  const followedTurnRef = useRef<string | undefined>(undefined);
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

  // A new turn scrolls its top into view; the stream is followed while the
  // user is at the bottom, but never past the turn's first line (UX-027). One
  // layout read per frame, not one per streamed draft (PERF-003).
  const lastTurnId = turns[turns.length - 1]?.id;
  const lastContent = turns[turns.length - 1]?.response?.content;
  useEffect(() => {
    const isNewTurn = followedTurnRef.current !== lastTurnId;
    if (!isNewTurn && !atBottomRef.current) return;
    followedTurnRef.current = lastTurnId;
    const frame = requestAnimationFrame(() => {
      const el = scrollRef.current;
      const turnEl = contentRef.current?.lastElementChild;
      if (!el || !(turnEl instanceof HTMLElement)) return;
      const view = {
        scrollTop: isNewTurn ? 0 : el.scrollTop,
        scrollHeight: el.scrollHeight,
        clientHeight: el.clientHeight,
      };
      el.scrollTo({ top: followScrollTop(view, offsetInScroller(el, turnEl)), behavior: "auto" });
      handleScroll();
    });
    return () => cancelAnimationFrame(frame);
  }, [lastTurnId, lastContent, handleScroll]);

  // Global scroll shortcuts (⌥⌘↑ / ⌥⌘↓ by default, forwarded by the backend).
  useEffect(() => {
    return eventBus.on("panel.scroll", ({ direction }) => {
      const el = scrollRef.current;
      if (!el) return;
      el.scrollBy({ top: direction === "down" ? 160 : -160, behavior: "smooth" });
    });
  }, []);

  return (
    // Natural content height until the SURFACE cap is reached. Both flex
    // ancestors can shrink; only this viewport scrolls, never the chrome.
    <div className="relative flex min-h-0 min-w-0 flex-auto flex-col overflow-hidden">
      <div
        ref={scrollRef}
        onScroll={handleScroll}
        role="region"
        aria-label="Response"
        tabIndex={0}
        className="min-h-0 overflow-y-auto overscroll-contain"
      >
        <div ref={contentRef} className="flex min-h-[120px] flex-col gap-5 px-5 py-4">
          {turns.map((turn, index) => (
            // One turn that fails to render must not blank the HUD (UX-038).
            <ErrorBoundary key={turn.id} resetKey={turn.response} fallback={TURN_FALLBACK}>
              <Turn
                turn={turn}
                isLast={index === turns.length - 1}
                onRetry={onRetry}
                onRegenerate={onRegenerate}
              />
            </ErrorBoundary>
          ))}
        </div>
      </div>
      {!atBottom ? (
        <button
          type="button"
          aria-label="Scroll to bottom"
          onClick={() => scrollToBottom()}
          className="absolute bottom-3 right-4 flex size-9 items-center justify-center rounded-full bg-hud-chip text-fg shadow-lg shadow-black/25 backdrop-blur transition-colors hover:bg-fg/20 motion-safe:animate-fade-in"
        >
          <ArrowDown className="size-4" aria-hidden />
        </button>
      ) : null}
    </div>
  );
}
