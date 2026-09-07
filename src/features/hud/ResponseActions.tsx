import { Check, Copy, Code, RotateCcw, ThumbsDown, ThumbsUp } from "lucide-react";
import { useState } from "react";

import { Tooltip } from "@/components/ui/Tooltip";
import { showToast } from "@/components/ui/toast-store";
import { bluey } from "@/lib/tauri/api";
import type { BlueyResponse, FeedbackCategory, FeedbackRating } from "@/lib/types";
import { cn } from "@/lib/utils/cn";
import { copyText } from "@/lib/utils/clipboard";
import { extractCodeBlocks } from "./markdown";

const FEEDBACK_CHIPS: Array<{ id: FeedbackCategory; label: string }> = [
  { id: "wrong", label: "Wrong" },
  { id: "too_long", label: "Too long" },
  { id: "not_relevant", label: "Not relevant" },
  { id: "missed_context", label: "Missed context" },
  { id: "wrong_tone", label: "Wrong tone" },
];

function ActionButton({
  label,
  onClick,
  children,
  active,
}: {
  label: string;
  onClick: () => void;
  children: React.ReactNode;
  active?: boolean;
}) {
  return (
    <Tooltip label={label}>
      <button
        type="button"
        aria-label={label}
        onClick={onClick}
        className={cn(
          "flex size-7 items-center justify-center rounded-[7px] text-fg-muted transition-colors",
          "hover:bg-white/10 hover:text-fg",
          active && "text-fg bg-white/10",
        )}
      >
        {children}
      </button>
    </Tooltip>
  );
}

export interface ResponseActionsProps {
  response: BlueyResponse;
  onRegenerate: () => void;
}

/** Copy answer / copy code / 👍 👎 (+ category chips) / regenerate. */
export function ResponseActions({ response, onRegenerate }: ResponseActionsProps) {
  const [rating, setRating] = useState<FeedbackRating | null>(response.feedback?.rating ?? null);
  const [showChips, setShowChips] = useState(false);
  const [copiedAnswer, setCopiedAnswer] = useState(false);
  const codeBlocks = extractCodeBlocks(response.content);
  const code = response.code?.code ?? codeBlocks[0]?.code;

  const copyAnswer = async () => {
    if (await copyText(response.content)) {
      setCopiedAnswer(true);
      showToast("Copied");
      setTimeout(() => setCopiedAnswer(false), 1000);
    }
  };

  const copyCode = async () => {
    if (code && (await copyText(code))) showToast("Code copied");
  };

  const sendFeedback = async (nextRating: FeedbackRating, categories?: FeedbackCategory[]) => {
    setRating(nextRating);
    setShowChips(nextRating === "down" && !categories);
    try {
      await bluey.responses.feedback({ responseId: response.id, rating: nextRating, categories });
      if (categories?.length) showToast("Thanks for the feedback");
    } catch (error) {
      console.warn("[feedback] failed", error);
    }
  };

  return (
    <div className="mt-2.5">
      <div className="flex items-center gap-0.5">
        <ActionButton label="Copy answer" onClick={() => void copyAnswer()}>
          {copiedAnswer ? <Check className="size-[15px] text-success" aria-hidden /> : <Copy className="size-[15px]" aria-hidden />}
        </ActionButton>
        {code ? (
          <ActionButton label="Copy code" onClick={() => void copyCode()}>
            <Code className="size-[15px]" aria-hidden />
          </ActionButton>
        ) : null}
        <div className="mx-1 h-4 w-px bg-hud-border" aria-hidden />
        <ActionButton label="Helpful" active={rating === "up"} onClick={() => void sendFeedback("up")}>
          <ThumbsUp className="size-[15px]" aria-hidden />
        </ActionButton>
        <ActionButton label="Not helpful" active={rating === "down"} onClick={() => void sendFeedback("down")}>
          <ThumbsDown className="size-[15px]" aria-hidden />
        </ActionButton>
        <div className="mx-1 h-4 w-px bg-hud-border" aria-hidden />
        <ActionButton label="Regenerate" onClick={onRegenerate}>
          <RotateCcw className="size-[15px]" aria-hidden />
        </ActionButton>
      </div>

      {showChips ? (
        <div className="mt-2 motion-safe:animate-rise-in">
          <div className="mb-1.5 text-[12px] text-fg-muted">Why wasn't this useful?</div>
          <div className="flex flex-wrap gap-1.5">
            {FEEDBACK_CHIPS.map((chip) => (
              <button
                key={chip.id}
                type="button"
                onClick={() => void sendFeedback("down", [chip.id])}
                className="h-[24px] rounded-full border border-hud-border bg-white/4 px-2.5 text-[12px] text-fg-muted transition-colors hover:bg-white/10 hover:text-fg"
              >
                {chip.label}
              </button>
            ))}
          </div>
        </div>
      ) : null}
    </div>
  );
}
